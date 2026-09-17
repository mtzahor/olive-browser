#![cfg(feature = "css")]
use olive_html::{
    NodeId,
    css::{
        Color, ComputedStyle, Direction, Display, Length, LineHeight, StyleBudget, Stylesheet,
        TextAlign, WhiteSpace,
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
        if element.name.local.as_ref() == "html" {
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
        p::before, p { color: blue; } p:has(a), p { color: green; }
        p { font-style: italic; background: url("https://example.com/x;}.png"); color: purple; }
        </style><p id=a>x"#,
    );
    assert_eq!(s["a"].color, Color(128, 0, 128, 255));
    assert!(s["a"].italic);
    assert_eq!(s["a"].background.3, 0);
    assert!(sheet.diagnostics.ignored >= 4);
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
    assert_eq!(s["button"].display, Display::Block);
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
    assert_eq!(s["a"].border_width, Length::Px(2.0));
    assert_eq!(s["a"].border_color, Color(0, 0, 255, 255));
    assert!(s["a"].border_solid);
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
