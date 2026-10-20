#![cfg(feature = "css")]
use olive_html::{
    NodeId,
    css::{
        Color, ComputedStyle, Direction, Display, FlexDirection, FlexWrap, Length, LineHeight,
        Position, StyleBudget, Stylesheet, TextAlign, TextTransform, VerticalAlign, WhiteSpace,
    },
    parse,
};
use std::collections::HashMap;

fn styles(html: &str) -> (HashMap<String, ComputedStyle>, Stylesheet, StyleBudget) {
    let parsed = parse(html).unwrap();
    let doc = &parsed.document;
    let sheet = Stylesheet::from_document(doc);
    let mut budget = StyleBudget::default();
    let mut computed: HashMap<NodeId, ComputedStyle> = HashMap::new();
    let mut result = HashMap::new();
    let mut root_font = 17.0;
    for id in doc.descendants(doc.root()) {
        let node = doc.node(id).unwrap();
        let Some(element) = node.as_element() else {
            continue;
        };
        let parent = node
            .parent()
            .and_then(|id| computed.get(&id))
            .copied()
            .unwrap_or_default();
        let style = sheet.compute(doc, id, parent, root_font, &mut budget);
        if element.name.local.as_str() == "html" {
            root_font = style.font_size;
        }
        computed.insert(id, style);
        if let Some(id) = element.attribute("id") {
            result.insert(id.to_owned(), style);
        }
    }
    (result, sheet, budget)
}
#[test]
fn cascade_uses_specificity_importance_inline_and_source_order() {
    let (s, _, _) = styles(
        r#"<!doctype html><style>
        p { color: red; } .note { color: blue; } #a { color: green; }
        #a { color: purple; } #b { color: red !important; }
        p { background: red !important; background: blue; }
        </style><p id=a class=note>one</p>
        <p id=b style="color: blue">two</p>
        <p id=c style="color: blue !important; color: red">three</p>"#,
    );
    assert_eq!(s["a"].color, Color(128, 0, 128, 255));
    assert_eq!(s["b"].color, Color(255, 0, 0, 255));
    assert_eq!(s["c"].color, Color(0, 0, 255, 255));
    assert_eq!(s["a"].background, Color(255, 0, 0, 255));
}
#[test]
fn selector_groups_use_specificity_of_matching_selector_only() {
    let (s, _, _) = styles(
        "<!doctype html><style>#absent, p { color: red } .note { color: blue }</style><p class=note id=a>x",
    );
    assert_eq!(s["a"].color, Color(0, 0, 255, 255));
}
#[test]
fn selectors_support_compounds_escapes_child_descendant_and_backtracking() {
    let (s, _, _) = styles(
        r#"<!doctype html><style>
        .outer > .middle .leaf { color: red; }
        #\74 arget.a.b { background: #1234; }
        div/**/.a { font-style: italic; }
        </style><div class=outer><div class=middle><div><div class=middle>
        <div id=target class="leaf a b">x</div></div></div></div></div>"#,
    );
    assert_eq!(s["target"].color, Color(255, 0, 0, 255));
    assert_eq!(s["target"].background, Color(17, 34, 51, 68));
    assert!(s["target"].italic);
}
#[test]
fn unsupported_selector_invalidates_whole_group_and_at_rules_stay_inert() {
    let (s, sheet, _) = styles(
        r#"<!doctype html><style>
        @import "file:///private.css";
        @media screen { p { color: red; } }
        p:has(a), p { color: green; }
        p { font-style: italic; background: url("https://example.com/x;}.png"); color: purple; }
        </style><p id=a>x"#,
    );
    assert_eq!(s["a"].color, Color(128, 0, 128, 255));
    assert!(s["a"].italic);
    assert_eq!(s["a"].background.3, 0);
    assert!(sheet.diagnostics.ignored >= 3);
}
#[test]
fn renderable_button_css_has_no_unsupported_declarations() {
    let (s, sheet, _) = styles(
        r#"<!doctype html><style>
        body { font-family: Arial, sans-serif; background-color: #f0f0f0; margin: 0; padding: 20px; }
        .container { max-width: 600px; margin: 0 auto; background: white; padding: 20px;
          border-radius: 8px; box-shadow: 0 2px 4px rgba(0,0,0,0.1); }
        .button { display: inline-block; background: #0066cc; color: white; padding: 10px 20px;
          text-decoration: none; border-radius: 5px; }
        .button:hover { background: #004499; }
        </style><body><div id=button class=button onclick="alert('ok')">Click</div>"#,
    );
    assert_eq!(sheet.diagnostics.ignored, 0);
    assert_eq!(s["button"].display, Display::InlineBlock);
    assert_eq!(s["button"].background, Color(0, 102, 204, 255));
    assert_eq!(
        s["button"].padding,
        [
            Length::Px(10.0),
            Length::Px(20.0),
            Length::Px(10.0),
            Length::Px(20.0)
        ]
    );
}
#[test]
fn malformed_values_do_not_replace_valid_declarations() {
    let (s, sheet, _) = styles(
        r#"<!doctype html><style>
        p { color: red; color: blue garbage; broken; font-size: 22px;
            font-size: -4px; padding: 4px; padding: -2px; width: 10qu;
            background: rgb(10 20 30 / 50%); }
        </style><p id=a style="font-size: 14px nope; color: !important">x"#,
    );
    assert_eq!(s["a"].color, Color(255, 0, 0, 255));
    assert_eq!(s["a"].font_size, 22.0);
    assert_eq!(s["a"].padding, [Length::Px(4.0); 4]);
    assert_eq!(s["a"].background, Color(10, 20, 30, 128));
    assert!(sheet.diagnostics.ignored > 0);
}
#[test]
fn inherited_text_noninherited_boxes_and_css_wide_keywords() {
    let (s, _, _) = styles(
        r#"<!doctype html><div id=parent style="color: red; font-size: 20px; padding: 3px; background: blue; line-height: 2">
        <span id=a>one</span><span id=b style="color: initial; padding: inherit; background: unset">two</span>
        <span id=c style="color: unset; font-size: 150%; line-height: inherit">three</span></div>"#,
    );
    assert_eq!(s["a"].color, Color(255, 0, 0, 255));
    assert_eq!(s["a"].font_size, 20.0);
    assert_eq!(s["a"].padding, [Length::Px(0.0); 4]);
    assert_eq!(s["a"].background.3, 0);
    assert_eq!(s["b"].color, ComputedStyle::default().color);
    assert_eq!(s["b"].padding, [Length::Px(3.0); 4]);
    assert_eq!(s["b"].background.3, 0);
    assert_eq!(s["c"].color, Color(255, 0, 0, 255));
    assert_eq!(s["c"].font_size, 30.0);
    assert_eq!(s["c"].line_height_px(), 60.0);
}
#[test]
fn shorthand_expands_before_cascade_and_resolves_lengths_after_font_size() {
    let (s, _, _) = styles(
        r#"<!doctype html><html style="font-size: 20px"><style>
        p { margin: 1px 2px 3px 4px; margin-left: 8px; padding: 1em 2rem;
            border: red solid 2px; border-color: currentColor; color: blue; font-size: 24px;
            width: 60%; max-width: 500px; border-radius: 6px; }
        </style><p id=a>x"#,
    );
    assert_eq!(
        s["a"].margin,
        [
            Length::Px(1.0),
            Length::Px(2.0),
            Length::Px(3.0),
            Length::Px(8.0)
        ]
    );
    assert_eq!(
        s["a"].padding,
        [
            Length::Px(24.0),
            Length::Px(40.0),
            Length::Px(24.0),
            Length::Px(40.0)
        ]
    );
    assert_eq!(s["a"].border_width, [Length::Px(2.0); 4]);
    assert_eq!(s["a"].border_color, [Color(0, 0, 255, 255); 4]);
    assert_eq!(s["a"].border_solid, [true; 4]);
    assert_eq!(s["a"].width, Length::Percent(0.6));
    assert_eq!(s["a"].max_width, Length::Px(500.0));
}
#[test]
fn stylesheet_order_and_media_type_filtering() {
    let (s, sheet, _) = styles(
        r#"<!doctype html><style>p { color: red }</style>
        <style media=print>p { color: blue }</style><style type=text/plain>p { color: blue }</style>
        <style media="screen, print">p { color: green }</style>
        <template><style>p { color: blue }</style></template><p id=a>x"#,
    );
    assert_eq!(sheet.rule_count(), 2);
    assert_eq!(s["a"].color, Color(0, 128, 0, 255));
}
#[test]
fn font_whitespace_alignment_and_display_override_html_defaults() {
    let (s, _, _) = styles(
        r#"<!doctype html><h1 id=a style="font-size: 18px; font-weight: 450; font-style: normal; font-family: 'Courier New', monospace !important; text-align: center; white-space: pre-wrap; display: inline; line-height: 150%; text-decoration: underline line-through">x</h1>
        <p id=b style="display:none">hide</p><p id=c hidden style="display:block">hide</p>"#,
    );
    assert_eq!(s["a"].font_size, 18.0);
    assert_eq!(s["a"].font_weight, 450.0);
    assert!(s["a"].monospace);
    assert!(s["a"].underline && s["a"].strike);
    assert_eq!(s["a"].text_align, TextAlign::Center);
    assert_eq!(s["a"].white_space, WhiteSpace::PreWrap);
    assert_eq!(s["a"].line_height, LineHeight::Px(27.0));
    assert_eq!(s["a"].display, Display::Inline);
    assert_eq!(s["b"].display, Display::None);
    assert_eq!(s["c"].display, Display::None);
}
#[test]
fn direction_is_inherited_and_html_dir_is_used_as_the_initial_value() {
    let (s, _, _) = styles(
        r#"<html dir=rtl><body><p id=a><span id=b>שלום</span></p>
        <p id=c style="direction:ltr"><span id=d>hello</span></p>"#,
    );
    assert_eq!(s["a"].direction, Direction::Rtl);
    assert_eq!(s["b"].direction, Direction::Rtl);
    assert_eq!(s["c"].direction, Direction::Ltr);
    assert_eq!(s["d"].direction, Direction::Ltr);
    let (s, _, _) = styles("<p id=a dir=auto>x");
    assert_eq!(s["a"].direction, Direction::Auto);
}
#[test]
fn language_direction_fills_in_missing_dir_without_beating_explicit_css() {
    let (s, _, _) = styles(
        r#"<html lang=he><body><p id=he>שלום</p><p id=en lang=en>Hello</p>
        <p id=css lang=he style="direction:ltr">Hello</p><p id=dir lang=he dir=ltr>Hello</p>"#,
    );
    assert_eq!(s["he"].direction, Direction::Rtl);
    assert_eq!(s["en"].direction, Direction::Ltr);
    assert_eq!(s["css"].direction, Direction::Ltr);
    assert_eq!(s["dir"].direction, Direction::Ltr);
}
#[test]
fn colors_support_named_hex_alpha_and_rgb_notation() {
    for (value, expected) in [
        ("rebeccapurple", Color(102, 51, 153, 255)),
        ("#aBc", Color(170, 187, 204, 255)),
        ("#12345678", Color(18, 52, 86, 120)),
        ("rgba(255, 0, 0, 0.5)", Color(255, 0, 0, 128)),
        ("rgb(100% 0% 0% / 25%)", Color(255, 0, 0, 64)),
        ("transparent", Color(0, 0, 0, 0)),
    ] {
        let (s, _, _) = styles(&format!("<!doctype html><p id=a style='color:{value}'>x"));
        assert_eq!(s["a"].color, expected, "{value}");
    }
}
#[test]
fn root_rem_and_explicit_length_inheritance_preserve_computed_values() {
    let (s, _, _) = styles(
        r#"<!doctype html><html id=root style="font-size:20px; padding:1rem"><body>
        <div style="font-size:10px; padding:2em; line-height:2em">
        <p id=a style="font-size:30px; padding:inherit">x</p></div>"#,
    );
    assert_eq!(s["root"].padding, [Length::Px(20.0); 4]);
    assert_eq!(s["a"].padding, [Length::Px(20.0); 4]);
    assert_eq!(s["a"].line_height, LineHeight::Px(20.0));
}
#[test]
fn case_sensitive_ids_and_classes_follow_document_mode() {
    let html = "<style>.Note {color:red}</style><p id=a class=note>x";
    assert_eq!(
        styles(&format!("<!doctype html>{html}")).0["a"].color,
        ComputedStyle::default().color
    );
    assert_eq!(styles(html).0["a"].color, Color(255, 0, 0, 255));
}
#[test]
fn limits_bound_css_storage_and_matching_work() {
    let large = Stylesheet::parse(&" ".repeat(olive_html::css::MAX_CSS_BYTES + 1));
    assert!(large.diagnostics.limited);
    assert_eq!(large.rule_count(), 0);
    let rules = Stylesheet::parse(&"p {color:red}".repeat(olive_html::css::MAX_RULES + 1));
    assert!(rules.diagnostics.limited);
    assert_eq!(rules.rule_count(), olive_html::css::MAX_RULES);
    let (_, sheet, _) = styles(&format!(
        "<style>p {{{}}}</style><p>x",
        "color:red;".repeat(513)
    ));
    assert!(sheet.diagnostics.limited);
    let (_, _, budget) = styles(&format!(
        "<style>{}</style>{}",
        ".missing {color:red}".repeat(2048),
        "<p>x</p>".repeat(1000)
    ));
    assert!(
        !budget.limited,
        "unrelated indexed rules must not exhaust matching"
    );
    let (_, _, budget) = styles(&format!(
        "<style>{}</style>{}",
        "[data-missing] {color:red}".repeat(4096),
        "<p>x</p>".repeat(1000)
    ));
    assert!(
        !budget.limited,
        "attribute indexing must exclude unrelated elements"
    );
    let (_, _, budget) = styles(&format!(
        "<style>{}</style>{}",
        "* {color:red}".repeat(4096),
        "<p>x</p>".repeat(1000)
    ));
    assert!(budget.limited, "universal matching remains bounded");
}
#[test]
fn deterministic_malformed_css_recovers_without_panics() {
    let tokens = [
        "p",
        "{",
        "}",
        ";",
        "color",
        ":",
        "red",
        "/*",
        "*/",
        "\"",
        "(",
        ")",
        "@media",
        "!important",
        ".x",
        "\\",
        "🫒",
    ];
    let mut state = 13u64;
    for _ in 0..200 {
        let mut source = String::new();
        for _ in 0..100 {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            source.push_str(tokens[(state >> 32) as usize % tokens.len()]);
            source.push(' ');
        }
        let _ = Stylesheet::parse(&source);
    }
}

#[test]
fn megabyte_stylesheets_keep_rules_after_large_literal_data() {
    let source = format!("/* {} */ p {{color:red}}", "x".repeat(3 * 1024 * 1024));
    let sheet = Stylesheet::parse(&source);
    assert!(!sheet.diagnostics.limited);
    assert_eq!(sheet.rule_count(), 1);
}

#[test]
fn attributes_structural_selectors_and_siblings_match_real_elements() {
    let (s, sheet, _) = styles(
        r#"<!doctype html><style>
        :root {font-size:20px}
        [data-kind="NEWS" i] {color:red}
        [data-kind="NEWS" s] {color:blue}
        [data-tags~=lead][lang|=he][data-url^="https:"][data-url$=".il"][data-url*="news"] {font-style:italic}
        [data-kind^=""] {color:green}
        main > p:first-child + p {background:yellow}
        main > p:first-child ~ p:last-child {color:purple}
        p:nth-child(2n + 1) {font-weight:700}
        p:nth-last-child(2) {text-decoration:underline}
        p:nth-of-type(2) {padding:4px}
        p:nth-last-of-type(1) {margin:3px}
        p:empty {display:none}
        section > p:only-child {font-size:30px}
        a:any-link {color:blue}
        a:hover, a:visited, a:focus {display:none}
        </style><main><p id=a data-kind=news data-tags="top lead" lang=he-IL data-url="https://news.il">A</p>
        <!--not an element--><p id=b>B</p> text <p id=c>C</p></main>
        <p id=empty><!--empty--></p><section><p id=only>Only</p></section><a id=link href='/'>Link</a>"#,
    );
    assert_eq!(sheet.diagnostics.ignored, 0);
    assert_eq!(s["a"].color, Color(255, 0, 0, 255));
    assert!(s["a"].italic);
    assert_eq!(s["a"].font_weight, 700.0);
    assert_eq!(s["b"].background, Color(255, 255, 0, 255));
    assert!(s["b"].underline);
    assert_eq!(s["b"].padding, [Length::Px(4.0); 4]);
    assert_eq!(s["c"].color, Color(128, 0, 128, 255));
    assert_eq!(s["c"].margin, [Length::Px(3.0); 4]);
    assert_eq!(s["empty"].display, Display::None);
    assert_eq!(s["only"].font_size, 30.0);
    assert_eq!(s["link"].display, Display::Inline);
}

#[test]
fn functional_selectors_preserve_specificity_and_negation() {
    let (s, sheet, _) = styles(
        r#"<!doctype html><style>
        :where(#a) {color:red}
        p {color:green}
        p:is(.note, #absent) {color:blue}
        #a {color:purple}
        :not(.note, .skip) {font-style:italic}
        p:not(:is(.skip, .missing)) {background:yellow}
        </style><p id=a class=note>A</p><p id=b>B</p><p id=c class=skip>C</p>"#,
    );
    assert_eq!(sheet.diagnostics.ignored, 0);
    assert_eq!(s["a"].color, Color(0, 0, 255, 255));
    assert_eq!(s["b"].color, Color(0, 128, 0, 255));
    // Reset inherited italic before inspecting a negation's own effect.
    let (s, _, _) = styles(
        "<style>p:not(.skip) {font-style:italic}</style><p id=a>A</p><p id=b class=skip>B</p>",
    );
    assert!(s["a"].italic);
    assert!(!s["b"].italic);
}

#[test]
fn screen_media_preserves_order_and_skips_unknown_conditions() {
    let (s, sheet, _) = styles(
        r#"<!doctype html><style>
        p {color:red}
        @media print {p {color:blue}}
        @media screen {p {color:green} @media all {p {font-size:22px}}}
        @media only screen {p {color:purple}}
        @media screen and (max-width:1px) {p {display:none}}
        @media not print {p {font-style:italic}}
        </style><style media="only screen">p {padding:2px}</style><p id=a>A</p>"#,
    );
    assert_eq!(sheet.diagnostics.ignored, 0);
    assert_eq!(s["a"].color, Color(128, 0, 128, 255));
    assert_eq!(s["a"].font_size, 22.0);
    assert_eq!(s["a"].display, Display::Block);
    assert_eq!(s["a"].padding, [Length::Px(2.0); 4]);
    assert!(s["a"].italic);
    let nested = format!(
        "{}p {{color:red}}{}",
        "@media screen {".repeat(100),
        "}".repeat(100)
    );
    assert!(Stylesheet::parse(&nested).diagnostics.limited);
    let grouped = format!(
        "@media screen {{{}}}",
        "p {color:red}".repeat(olive_html::css::MAX_RULES + 1)
    );
    let sheet = Stylesheet::parse(&grouped);
    assert!(sheet.diagnostics.limited);
    assert_eq!(sheet.rule_count(), olive_html::css::MAX_RULES);
}

#[test]
fn indexed_groups_keep_rule_order_and_matching_specificity() {
    let (s, _, budget) = styles(
        r#"<!doctype html><style>
        #absent, .note, p {color:red}
        .note {color:blue}
        </style><p id=a class="note note note">A</p>"#,
    );
    assert_eq!(s["a"].color, Color(0, 0, 255, 255));
    assert!(!budget.limited);
    let nested = format!("{}p{}", ":not(".repeat(100), ")".repeat(100));
    assert_eq!(
        Stylesheet::parse(&format!("{nested} {{color:red}}")).rule_count(),
        0
    );
}

#[test]
fn border_box_and_min_height_compute_and_inherit_explicitly() {
    let (s, _, _) = styles(
        r#"<div style="box-sizing:border-box; min-height:2em; font-size:20px">
        <p id=a style="box-sizing:inherit; min-height:inherit">A</p><p id=b>B</p></div>"#,
    );
    assert!(s["a"].border_box);
    assert_eq!(s["a"].min_height, Length::Px(40.0));
    assert!(!s["b"].border_box);
    assert_eq!(s["b"].min_height, Length::Px(0.0));
}

fn px(length: Length) -> f32 {
    match length {
        Length::Px(value) => value,
        other => panic!("expected a pixel length, got {other:?}"),
    }
}

#[test]
fn html_defaults_space_headings_lists_and_code_relative_to_their_text() {
    let (s, _, _) = styles(
        r#"<!doctype html><body id=body style="font-size:20px">
        <h1 id=h1>T</h1><h2 id=h2>T</h2>
        <p id=p><code id=code>x</code> <sup id=sup>1</sup> <sub id=sub>2</sub> <mark id=mark>m</mark></p>
        <pre id=pre><code id=inner>x</code></pre>
        <ul id=ul><li id=li>x<ul id=nested><li>y</li></ul></li></ul>
        <ol id=rtl dir=rtl><li>x</li></ol>
        <blockquote id=quote>q</blockquote>
        <center id=center>c</center><dialog id=dialog>d</dialog><dialog id=open open>e</dialog>"#,
    );
    assert_eq!(s["body"].margin, [Length::Px(8.0); 4]);
    // Headings scale with the surrounding text and keep more space above.
    assert_eq!(s["h1"].font_size, 40.0);
    assert_eq!(s["h2"].font_size, 32.0);
    assert!((px(s["h1"].margin[0]) - 24.0).abs() < 0.01);
    assert!((px(s["h1"].margin[2]) - 16.0).abs() < 0.01);
    assert_eq!(s["code"].font_size, 20.0 * 15.0 / 17.0);
    assert_eq!(s["inner"].font_size, s["pre"].font_size);
    assert_eq!(s["pre"].border_radius, [Length::Px(6.0); 4]);
    assert!(px(s["pre"].padding[3]) > 0.0);
    assert_eq!(s["sup"].vertical_align, VerticalAlign::Super);
    assert_eq!(s["sub"].vertical_align, VerticalAlign::Sub);
    assert_eq!(s["sup"].font_size, 15.0);
    assert_eq!(s["p"].vertical_align, VerticalAlign::Baseline);
    assert!(s["mark"].background.3 > 0);
    // Lists indent at the start side; only outer lists are spaced from what follows.
    assert_eq!(s["ul"].padding[3], Length::Px(40.0));
    assert_eq!(s["ul"].margin[2], Length::Px(12.0));
    assert_eq!(s["li"].margin, [Length::Px(0.0); 4]);
    assert_eq!(s["nested"].margin[2], Length::Px(0.0));
    assert_eq!(s["rtl"].padding[1], Length::Px(40.0));
    assert_eq!(s["rtl"].padding[3], Length::Px(0.0));
    assert_eq!(s["quote"].margin[1], Length::Px(40.0));
    assert_eq!(s["quote"].margin[3], Length::Px(40.0));
    assert_eq!(s["center"].display, Display::Block);
    assert_eq!(s["center"].text_align, TextAlign::Center);
    assert_eq!(s["dialog"].display, Display::None);
    assert_eq!(s["open"].display, Display::Inline);
}

#[test]
fn tables_take_display_spans_and_legacy_presentational_attributes() {
    let (s, _, _) = styles(
        r##"<!doctype html><center><table id=t width="85%" bgcolor="f6f6ef" cellspacing=0 cellpadding=3 border=1>
        <tr id=r align=right><td id=a colspan=3 rowspan=0 bgcolor="#ff6600" nowrap width=18>x</td>
        <th id=h>y</th><td id=deep><table id=inner><tr><td>n</td></tr></table></td></tr></table></center>
        <table id=plain><tr><td id=b>z</td></tr><col id=col></table>
        <font id=font color=green>f</font>"##,
    );
    let t = s["t"];
    assert_eq!(t.display, Display::Table);
    assert_eq!(t.width, Length::Percent(0.85));
    assert_eq!(t.background, Color(246, 246, 239, 255));
    assert_eq!(
        (t.row_gap, t.column_gap),
        (Length::Px(0.0), Length::Px(0.0))
    );
    assert_eq!(t.border_solid, [true; 4]);
    assert_eq!(t.border_width, [Length::Px(1.0); 4]);
    assert_eq!(t.border_color, [Color(128, 128, 128, 255); 4]);
    assert_eq!((t.margin[1], t.margin[3]), (Length::Auto, Length::Auto));
    assert_eq!(t.margin[2], Length::Px(12.0));
    assert_eq!(s["r"].display, Display::TableRow);
    let a = s["a"];
    assert_eq!(a.display, Display::TableCell);
    assert_eq!((a.column_span, a.row_span), (3, 1));
    assert_eq!(a.background, Color(255, 102, 0, 255));
    assert_eq!(a.white_space, WhiteSpace::NoWrap);
    assert_eq!(a.width, Length::Px(18.0));
    assert_eq!(a.padding, [Length::Px(3.0); 4]);
    assert_eq!(a.border_solid, [true; 4]);
    assert_eq!(a.border_width, [Length::Px(1.0); 4]);
    assert_eq!(a.text_align, TextAlign::Right);
    assert_eq!(s["h"].font_weight, 700.0);
    assert_eq!(s["h"].text_align, TextAlign::Center);
    // Layout tables nested in cells add no space below themselves.
    assert_eq!(s["inner"].margin[2], Length::Px(0.0));
    let plain = s["plain"];
    assert_eq!(plain.row_gap, Length::Px(2.0));
    assert_eq!(plain.margin[1], Length::Px(0.0));
    assert!(!plain.has_border());
    assert!(px(s["b"].padding[1]) > px(s["b"].padding[0]));
    assert_eq!(s["col"].display, Display::None);
    assert_eq!(s["font"].color, Color(0, 128, 0, 255));
}

#[test]
fn quirks_mode_tables_do_not_inherit_alignment_or_fonts() {
    let markup =
        r#"<center style="font-size:30px"><table id=t><tr><td id=c>x</td></tr></table></center>"#;
    let (quirks, _, _) = styles(markup);
    assert_eq!(quirks["t"].text_align, TextAlign::Start);
    assert_eq!(quirks["c"].font_size, 17.0);
    let (standard, _, _) = styles(&format!("<!doctype html>{markup}"));
    assert_eq!(standard["t"].text_align, TextAlign::Center);
    assert_eq!(standard["c"].font_size, 30.0);
}

#[test]
fn table_border_model_corner_radii_and_absolute_units_parse() {
    let (s, sheet, budget) = styles(
        r#"<!doctype html><style>
        .c { border-collapse: collapse; border-spacing: 4px 6px; font-size: 12pt }
        .r { border-radius: 50% 4px / 2px }
        .r3 { border-radius: 1px 2px 3px; border-top-left-radius: 9px 1px }
        .u { width: 1in; height: 2.54cm; padding: 1pc 10mm 0 0 }
        </style><table id=c class=c><tr><td id=d>w</td></tr></table>
        <div id=r class=r></div><div id=r3 class=r3></div><div id=u class=u></div>"#,
    );
    assert!(s["c"].border_collapse);
    assert!(s["d"].border_collapse);
    assert_eq!(s["c"].column_gap, Length::Px(4.0));
    assert_eq!(s["c"].row_gap, Length::Px(6.0));
    assert_eq!(s["c"].font_size, 16.0);
    assert_eq!(
        s["r"].border_radius,
        [
            Length::Percent(0.5),
            Length::Px(4.0),
            Length::Percent(0.5),
            Length::Px(4.0)
        ]
    );
    assert_eq!(
        s["r3"].border_radius,
        [
            Length::Px(9.0),
            Length::Px(2.0),
            Length::Px(3.0),
            Length::Px(2.0)
        ]
    );
    assert_eq!(s["u"].width, Length::Px(96.0));
    assert!((px(s["u"].height) - 96.0).abs() < 0.01);
    assert_eq!(s["u"].padding[0], Length::Px(16.0));
    assert!((px(s["u"].padding[1]) - 37.795).abs() < 0.01);
    assert_eq!(sheet.diagnostics.ignored + budget.ignored, 0);
    assert!(sheet.diagnostics.unsupported_properties.is_empty());
}

#[test]
fn pseudo_element_selectors_never_match_but_keep_their_list_valid() {
    let (s, sheet, _) = styles(
        r#"<!doctype html><style>
        *, *:before, *::after { box-sizing: border-box }
        p::selection, .x::-webkit-scrollbar { color: red }
        .only::before { content: "x"; float: left }
        p::-moz-selection, p { font-weight: 700 }
        :is(::before, p) { font-style: italic }
        </style><p id=a class=only>x</p>"#,
    );
    let a = s["a"];
    assert!(a.border_box);
    assert_eq!(a.color, ComputedStyle::default().color);
    assert_eq!(a.float_side, olive_html::css::FloatSide::None);
    // Another engine's prefixed pseudo-element still invalidates its list.
    assert_eq!(a.font_weight, 400.0);
    // Pseudo-elements are not valid inside functional pseudo-classes.
    assert!(a.italic);
    assert_eq!(sheet.rule_count(), 2);
}

#[test]
fn overflow_and_clear_parse_their_supported_keywords() {
    let (s, sheet, budget) = styles(
        r#"<!doctype html><style>
        #a { overflow: hidden auto; clear: both } #b { overflow-y: clip; clear: inline-start }
        #c { overflow: scroll; clear: right } #d { overflow: sideways; clear: top }
        </style><div id=a></div><div id=b></div><div id=c></div><div id=d></div>"#,
    );
    use olive_html::css::Clear;
    assert_eq!(s["a"].overflow_hidden, [true, false]);
    assert_eq!(s["a"].clear, Clear::Both);
    assert_eq!(s["b"].overflow_hidden, [false, true]);
    assert_eq!(s["b"].clear, Clear::Left);
    // Scrolling overflow stays visible: Olive has no inner scrolling.
    assert_eq!(s["c"].overflow_hidden, [false, false]);
    assert_eq!(s["c"].clear, Clear::Right);
    assert_eq!(s["d"].overflow_hidden, [false, false]);
    assert_eq!(s["d"].clear, Clear::None);
    assert_eq!(sheet.diagnostics.ignored + budget.ignored, 2);
}

#[test]
fn font_shorthand_sets_and_resets_its_longhands() {
    let (s, _, _) = styles(
        r#"<!doctype html><style>
        div { font-weight: 700; font-style: italic; line-height: 3 }
        #a { font: 400 14px/35px Google-Oswald, Arimo, Arial }
        #b { font: italic bold 2em/1.2 "Courier New", monospace }
        #c { font: 16px Arial }
        #d { font: 0/0 a }
        #e { font: 12px }
        </style><div id=a>x</div><div id=b>x</div><div id=c>x</div><div id=d>x</div><div id=e>x</div>"#,
    );
    assert_eq!(s["a"].font_size, 14.0);
    assert_eq!(s["a"].font_weight, 400.0);
    assert!(!s["a"].italic);
    assert_eq!(s["a"].line_height, LineHeight::Px(35.0));
    assert_eq!(s["b"].font_size, 34.0);
    assert!(s["b"].italic && s["b"].monospace);
    assert_eq!(s["b"].font_weight, 700.0);
    assert_eq!(s["b"].line_height, LineHeight::Number(1.2));
    // Omitted parts return to their initial values.
    assert_eq!(s["c"].font_weight, 400.0);
    assert!(!s["c"].italic);
    assert_eq!(s["c"].line_height, LineHeight::Number(1.45));
    assert_eq!(s["d"].font_size, 1.0);
    // A family is required.
    assert_eq!(s["e"].font_weight, 700.0);
    assert_eq!(s["e"].line_height, LineHeight::Number(3.0));
}

#[test]
fn border_sides_parse_separately_and_combine() {
    let (s, _, _) = styles(
        r#"<!doctype html><style>
        #a { color: blue; border-bottom: 2px solid red; border-left: thick solid }
        #b { border: 1px solid #000; border-top-width: 4px; border-right-style: none; border-left-color: rgb(0, 128, 0) }
        #c { border-width: 1px 2px 3px; border-style: solid none; border-color: red green }
        </style><p id=a>x</p><p id=b>x</p><p id=c>x</p>"#,
    );
    let a = s["a"];
    assert_eq!(a.border_solid, [false, false, true, true]);
    assert_eq!(a.border_width[2], Length::Px(2.0));
    assert_eq!(a.border_width[3], Length::Px(5.0));
    assert_eq!(a.border_color[2], Color(255, 0, 0, 255));
    assert_eq!(a.border_color[3], Color(0, 0, 255, 255));
    let b = s["b"];
    assert_eq!(b.border_solid, [true, false, true, true]);
    assert_eq!(b.border_width[0], Length::Px(4.0));
    assert_eq!(b.border_color[3], Color(0, 128, 0, 255));
    assert_eq!(b.border_color[0], Color(0, 0, 0, 255));
    let c = s["c"];
    assert_eq!(
        c.border_width,
        [
            Length::Px(1.0),
            Length::Px(2.0),
            Length::Px(3.0),
            Length::Px(2.0)
        ]
    );
    assert_eq!(c.border_solid, [true, false, true, false]);
    assert_eq!(c.border_color[1], Color(0, 128, 0, 255));
    assert!(c.has_border());
}

#[test]
fn positioning_layout_and_text_keywords_from_common_sites() {
    let (s, _, budget) = styles(
        r#"<!doctype html><style>
        #a { position: fixed; z-index: 3; text-transform: uppercase; flex-flow: column wrap }
        #b { position: -webkit-sticky; z-index: auto; flex-direction: row-reverse; height: 100%; min-height: 50% }
        #c { position: sticky; flex-flow: wrap-reverse; text-transform: capitalize; z-index: -1 }
        </style><div id=a><span id=inner>x</span></div><div id=b>x</div><div id=c>x</div>"#,
    );
    assert_eq!(s["a"].position, Position::Fixed);
    assert_eq!(s["a"].z_index, Some(3));
    assert_eq!(s["a"].flex_direction, FlexDirection::Column);
    assert_eq!(s["a"].flex_wrap, FlexWrap::Wrap);
    // Text transforms inherit.
    assert_eq!(s["inner"].text_transform, TextTransform::Uppercase);
    assert_eq!(s["b"].position, Position::Sticky);
    assert_eq!(s["b"].z_index, None);
    assert_eq!(s["b"].flex_direction, FlexDirection::RowReverse);
    assert_eq!(s["b"].height, Length::Percent(1.0));
    assert_eq!(s["b"].min_height, Length::Percent(0.5));
    assert_eq!(s["c"].flex_direction, FlexDirection::Row);
    assert_eq!(s["c"].flex_wrap, FlexWrap::Wrap);
    assert_eq!(s["c"].text_transform, TextTransform::Capitalize);
    assert_eq!(s["c"].z_index, Some(-1));
    assert!(Position::Fixed.out_of_flow() && !Position::Sticky.out_of_flow());
    assert_eq!(budget.ignored, 0);
}
