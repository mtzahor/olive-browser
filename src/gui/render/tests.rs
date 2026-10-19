use super::bidi::layout_bidi;
use super::paint::{action_at, visual_match_ranges};
use super::*;
use crate::{css::DEFAULT_VIEWPORT, parse};

/// The normal view with the document's own styles and no images.
fn styled(doc: &Document, scripting_enabled: bool) -> Page {
    Page::new(
        doc,
        scripting_enabled,
        &Stylesheet::from_document(doc),
        &HashMap::new(),
        DEFAULT_VIEWPORT,
    )
}
fn page(html: &str) -> Page {
    styled(&parse(html).unwrap().document, true)
}
fn texts(page: &Page) -> Vec<&str> {
    page.blocks.iter().map(|b| b.job.text.as_str()).collect()
}

fn reading(html: &str) -> Page {
    let doc = parse(html).unwrap().document;
    Page::reading(
        &doc,
        false,
        &Stylesheet::from_document(&doc),
        &HashMap::new(),
        None,
        DEFAULT_VIEWPORT,
    )
}

#[test]
fn unicode_bidi_reorders_visual_text_but_keeps_logical_search_text() {
    let mut page = page("<html lang=he><body><p>שלום world 123</p></body></html>");
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(Default::default(), |ui| {
        page.show(ui);
    });
    output.textures_delta.clear();
    assert_eq!(texts(&page), ["שלום world 123"]);
    assert_eq!(page.blocks[0].visual_job.text, "world 123 שלום");
    let mut rendered: Vec<_> = page.blocks[0]
        .layout
        .as_ref()
        .unwrap()
        .2
        .rows
        .iter()
        .flat_map(|row| &row.glyphs)
        .filter(|glyph| glyph.advance_width > 0.0)
        .map(|glyph| glyph.chr)
        .collect();
    rendered.sort_unstable();
    let mut expected: Vec<_> = "world 123 םולש".chars().collect();
    expected.sort_unstable();
    // Galley glyph storage order is an egui implementation detail; the shaped
    // result should preserve every visible character.
    assert_eq!(rendered, expected);
    assert_eq!(page.blocks[0].visual_to_logical.len(), 14);
    assert_eq!(page.find_query("שלום"), 1);
    assert_eq!(
        visual_match_ranges(&page.blocks[0], &page.find.blocks[0]),
        [(10..14, 0)]
    );
    let reading =
        reading("<html dir=rtl><body><article><p>שלום world 123</p></article></body></html>");
    assert_eq!(reading.blocks[0].style.direction, Direction::Rtl);
}

#[test]
fn bidi_handles_brackets_combining_marks_overrides_and_wrapped_links() {
    let mut page = page(
        "<p dir=rtl><a href='/hebrew'>שָׁלוֹם</a> (123) abc</p><p dir=auto>אבג דהו זחט יכל מנס עפר צקת</p><p dir=ltr>abc &#x202e;DEF&#x202c;</p>",
    );
    let ctx = egui::Context::default();
    ctx.set_fonts(crate::gui::fonts::definitions());
    let mut wide = Vec::new();
    let mut output = ctx.run_ui(Default::default(), |ui| {
        for block in &mut page.blocks {
            layout_bidi(block, ui, 800.0);
            wide.push(block.visual_job.text.clone());
        }
    });
    output.textures_delta.clear();
    assert_eq!(wide[0], "abc (123) שָׁלוֹם");
    assert!(wide[2].contains("FED"));
    let block = &page.blocks[0];
    assert_eq!(block.visual_actions.len(), 1);
    let (range, action) = &block.visual_actions[0];
    assert_eq!(
        block
            .visual_job
            .text
            .chars()
            .skip(range.start)
            .take(range.len())
            .collect::<String>(),
        "שָׁלוֹם"
    );
    assert!(action.link.is_some());
    let mut output = ctx.run_ui(Default::default(), |ui| {
        let block = &mut page.blocks[1];
        let galley = layout_bidi(block, ui, 80.0);
        assert!(galley.rows.len() > 1);
        let first_line: String = galley.rows[0]
            .glyphs
            .iter()
            .map(|glyph| glyph.chr)
            .collect();
        assert!(first_line.contains("גבא"), "{first_line}");
        let mut offsets = block.visual_to_logical.clone();
        offsets.sort_unstable();
        assert_eq!(
            offsets,
            (0..block.job.text.chars().count()).collect::<Vec<_>>()
        );
    });
    output.textures_delta.clear();
}

#[test]
fn normal_command_and_middle_clicks_keep_the_same_link_and_target() {
    let ctx = egui::Context::default();
    let doc = parse("<p dir=rtl><a href='/next' onclick='alert(1)'>שלום Open another page</a></p>")
        .unwrap()
        .document;
    let mut page = styled(&doc, true);
    let mut output = ctx.run_ui(Default::default(), |ui| {
        page.show(ui);
    });
    let pos = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text().contains("Open another page") => {
                Some(text.pos + text.galley.rows[0].rect().center().to_vec2())
            }
            _ => None,
        })
        .unwrap();
    output.textures_delta.clear();
    for (button, modifiers, new_tab) in [
        (egui::PointerButton::Primary, egui::Modifiers::NONE, false),
        (egui::PointerButton::Primary, egui::Modifiers::COMMAND, true),
        (egui::PointerButton::Middle, egui::Modifiers::NONE, true),
    ] {
        let mut click = None;
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![
                    egui::Event::ModifiersChanged(modifiers),
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button,
                        pressed: true,
                        modifiers,
                    },
                    egui::Event::PointerButton {
                        pos,
                        button,
                        pressed: false,
                        modifiers,
                    },
                ],
                ..Default::default()
            },
            |ui| {
                click = page.show(ui).or(click.take());
            },
        );
        output.textures_delta.clear();
        let click = click.expect("link click must be delivered");
        assert_eq!(click.href.as_deref(), Some("/next"));
        assert!(click.target.is_some());
        assert_eq!(click.new_tab, new_tab);
    }
}

#[test]
fn presentation_validation_rejects_unbalanced_boxes_and_invalid_text_ranges() {
    let doc = parse("<p>Hello</p>").unwrap().document;
    let mut page = styled(&doc, true);
    assert!(page.validate().is_ok());
    page.commands.push(Command::Text(page.blocks.len()));
    assert!(page.validate().is_err());
    page.commands.pop();
    page.commands.insert(0, Command::Close);
    assert!(page.validate().is_err());
    page.commands.remove(0);
    page.blocks[0].job.sections[0].byte_range.end.0 += 1;
    assert!(page.validate().is_err());
}

#[test]
fn focus_extracts_article_and_removes_distractions_and_hidden_subtrees() {
    let page = reading(
        r#"
        <title>Article title</title><style>.invisible { display: none }</style>
        <header>Site masthead</header><nav>Site links</nav><main>
        <article style="color: red; background: blue; width: 4000px; font-size: 2px">
        <header><h1>A quieter page</h1><p>By Olive</p></header>
        <p>The story <strong>continues</strong>.</p><div class="ad-slot">Buy now</div>
        <div class="newsletter-signup">Subscribe</div><aside>Related stories</aside>
        <form>Search here <input><button>Search</button></form>
        <div hidden>Hidden attribute</div><p aria-hidden="true">Hidden ARIA</p>
        <div class="invisible"><p>Hidden by CSS</p></div>
        <div class="social-share">Share this</div><div id="comments">Discussion</div>
        <p id="end">The end.</p></article><p>Outside the article</p></main>
        <footer>Site footer</footer>"#,
    );
    assert_eq!(
        texts(&page),
        [
            "A quieter page",
            "By Olive",
            "The story continues.",
            "The end."
        ]
    );
    assert!(page.anchors.contains(&"end".to_owned()));
    assert!(page.boxes.iter().all(|style| style.width == Length::Auto));
    let body = &page.blocks[2];
    assert_eq!(body.job.sections[0].format.font_id.size, 20.0);
    assert_eq!(
        body.job.sections[0].format.color,
        color(Theme::Light.reading_ink())
    );
}

#[test]
fn focus_keeps_plain_pages_lists_code_unicode_and_links_but_no_click_handlers() {
    let html = r#"<title>Reading notes</title><p>שלום café &lt;script&gt;</p>
        <ol start="3"><li>First</li><li value="7">Next</li></ol>
        <pre><code>one\n  two</code></pre><p onclick="alert('no')">
        <a href="../next#part" onclick="return false">Read more</a></p>"#;
    let doc = parse(html).unwrap().document;
    let page = Page::reading(
        &doc,
        true,
        &Stylesheet::from_document(&doc),
        &HashMap::new(),
        None,
        DEFAULT_VIEWPORT,
    );
    assert_eq!(texts(&page)[0], "Reading notes");
    assert!(texts(&page).contains(&"שלום café <script>"));
    assert!(texts(&page).contains(&"3. First"));
    assert!(texts(&page).contains(&"7. Next"));
    assert!(page.links.values().any(|href| href == "../next#part"));
    assert!(
        page.blocks
            .iter()
            .flat_map(|block| &block.actions)
            .all(|(_, action)| action.target.is_none())
    );
    assert!(
        page.blocks
            .iter()
            .any(|block| block.style.white_space == WhiteSpace::PreWrap)
    );
    assert!(!reading("<p>A short, untitled plain page.</p>").is_empty());
    assert!(
        reading("<title>Empty</title><nav>Navigation only</nav><script>ignored</script>")
            .is_empty()
    );
}

#[test]
fn focus_finds_unmarked_prose_and_preserves_noscript_fallback() {
    let prose = "A long paragraph with enough readable text to identify the main story. ".repeat(8);
    let page = reading(&format!(
        "<div><a href='/one'>{}</a></div><div><h1>Story</h1><p>{prose}</p><noscript>Fallback text</noscript></div>",
        "Menu link ".repeat(30)
    ));
    assert!(!texts(&page).join(" ").contains("Menu link"));
    assert!(texts(&page).join(" ").contains("Fallback text"));
    assert_eq!(texts(&page)[0], "Story");
}

#[test]
fn focus_appearance_reflows_and_recolors_all_text_without_changing_content() {
    let mut page = reading(
        "<h1>Title</h1><p>A paragraph with <a href='/next'><strong>a link</strong></a> and <code>code</code>.</p><pre>more code</pre>",
    );
    let original_text: Vec<_> = texts(&page).into_iter().map(str::to_owned).collect();
    draw(&mut page, 320.0).textures_delta.clear();
    let original = page.blocks[1].layout.as_ref().unwrap().2.size();
    page.set_reading_style(Settings {
        font_size: 28.0,
        theme: Theme::Dark,
    });
    assert!(page.blocks.iter().all(|block| block.layout.is_none()));
    draw(&mut page, 320.0).textures_delta.clear();
    assert!(page.blocks[1].layout.as_ref().unwrap().2.size().y > original.y);
    assert_eq!(page.blocks[0].job.sections[0].format.font_id.size, 53.2);
    let formats: Vec<_> = page
        .blocks
        .iter()
        .flat_map(|block| block.job.sections.iter().map(|section| &section.format))
        .collect();
    assert!(
        formats
            .iter()
            .any(|format| format.color == color(Theme::Dark.reading_link()))
    );
    assert!(
        formats
            .iter()
            .any(|format| format.background == color(Theme::Dark.reading_code()))
    );
    assert!(
        page.boxes
            .iter()
            .any(|style| style.background == Theme::Dark.reading_code())
    );
    assert_eq!(texts(&page), original_text);
    page.set_reading_style(Settings::default());
    assert!((page.blocks[0].job.sections[0].format.font_id.size - 38.0).abs() < 0.001);
    page.set_reading_style(Settings {
        font_size: f32::INFINITY,
        ..Settings::default()
    });
    assert_eq!(page.root_font, focus::DEFAULT_FONT_SIZE);
    page.set_reading_style(Settings {
        font_size: 999.0,
        ..Settings::default()
    });
    assert_eq!(page.root_font, focus::MAX_FONT_SIZE);
}

#[test]
fn focus_retains_preloaded_article_images_and_original_presentation() {
    let doc = parse("<nav>Navigation</nav><article><h1>Story</h1><a href='/photo'><img src='photo.png' width='100' alt='Olive'></a><p>Caption</p></article>").unwrap().document;
    let id = doc
        .descendants(doc.root())
        .find(|&id| {
            doc.node(id)
                .unwrap()
                .as_element()
                .is_some_and(|e| e.name.local.as_str() == "img")
        })
        .unwrap();
    let images = HashMap::from([(
        id,
        ImageAsset {
            reference: "photo.png".into(),
            image: Arc::new(ColorImage::filled([2, 2], Color32::GREEN)),
        },
    )]);
    let sheet = Stylesheet::from_document(&doc);
    let mut focus = Page::reading(&doc, true, &sheet, &images, None, DEFAULT_VIEWPORT);
    let original = Page::new(&doc, true, &sheet, &images, DEFAULT_VIEWPORT);
    assert_eq!(focus.image_count(), 1);
    assert_eq!(focus.images[0].html_width, Some(100.0));
    assert!(focus.images[0].action.link.is_some());
    assert!(focus.images[0].action.target.is_none());
    focus.set_reading_style(Settings {
        theme: Theme::Dark,
        ..Settings::default()
    });
    assert!(texts(&original).contains(&"Navigation"));
    assert_eq!(original.blocks[0].job.sections[0].format.font_id.size, 17.0);
}

#[test]
fn focus_deep_and_large_documents_respect_preview_limits() {
    let html = format!(
        "{}<p>{}</p>{}",
        "<div>".repeat(2000),
        "word ".repeat(50_000),
        "</div>".repeat(2000)
    );
    let page = reading(&html);
    assert!(page.truncated);
    assert!(
        page.blocks
            .iter()
            .map(|block| block.job.text.chars().count())
            .sum::<usize>()
            <= MAX_TEXT
    );
    assert!(
        page.commands
            .iter()
            .filter(|cmd| matches!(cmd, Command::Open(_)))
            .count()
            == page
                .commands
                .iter()
                .filter(|cmd| matches!(cmd, Command::Close))
                .count()
    );
}

#[test]
fn links_have_independent_hit_targets_inside_wrapped_unicode_text() {
    let mut page = page(
        "<p>Plain café <a href='/one'>first <strong>link</strong></a> between <a href='/two'>second link wraps across lines</a> end</p>",
    );
    let mut output = draw(&mut page, 180.0);
    let block = &page.blocks[0];
    let galley = &block.layout.as_ref().unwrap().2;
    assert!(galley.rows.len() > 1);
    let text = block.job.text.chars().collect::<Vec<_>>();
    let mut offset = 0;
    for row in &galley.rows {
        for (column, glyph) in row.glyphs.iter().enumerate() {
            let position = egui::vec2(
                row.pos.x + glyph.pos.x + glyph.advance_width / 2.0,
                row.rect().center().y,
            );
            let action = action_at(block, galley, position);
            let prefix: String = text[..offset + column].iter().collect();
            let expected = if prefix.len() >= block.job.text.find("first").unwrap()
                && prefix.len() < block.job.text.find(" between").unwrap()
            {
                Some("/one")
            } else if prefix.len() >= block.job.text.find("second").unwrap()
                && prefix.len() < block.job.text.find(" end").unwrap()
            {
                Some("/two")
            } else {
                None
            };
            // Collapsed separator whitespace may inherit either adjacent style.
            if !glyph.chr.is_whitespace() {
                assert_eq!(
                    action
                        .and_then(|a| a.link)
                        .and_then(|id| page.links.get(&id))
                        .map(String::as_str),
                    expected,
                    "{prefix}"
                );
            }
        }
        offset += row.glyphs.len() + usize::from(row.ends_with_newline);
    }
    assert!(action_at(block, galley, egui::vec2(-10.0, -10.0)).is_none());
    output.textures_delta.clear();
}

#[test]
fn remote_noscript_is_visible_and_onclick_is_inert() {
    let document = parse("<body><noscript><p>Fallback</p></noscript><p onclick='alert(1)'>Inert</p><a href='/next' onclick='alert(2)'>Next</a><h2 id='café'>Anchor</h2><a name='legacy'></a>").unwrap().document;
    let page = styled(&document, false);
    assert!(texts(&page).contains(&"Fallback"));
    assert!(
        page.blocks
            .iter()
            .flat_map(|b| &b.actions)
            .all(|(_, action)| action.target.is_none())
    );
    assert_eq!(page.links.len(), 1);
    assert_eq!(page.anchors, ["café", "legacy"]);
}

#[test]
fn javascript_mutations_feed_text_and_css_rendering() {
    let doc = parse("<!doctype html><style>.done {color:red}</style><p id=p>before</p><noscript>fallback</noscript><script>const p=document.getElementById('p'); p.textContent='after'; p.className='done'; document.title='Changed';</script>").unwrap().document;
    let (doc, report) = crate::js::run_document(doc, Default::default());
    assert!(report.diagnostics.is_empty());
    let page = styled(&doc, true);
    assert_eq!(texts(&page), ["after"]);
    assert_eq!(page.title, "Changed");
    assert_eq!(page.blocks[0].job.sections[0].format.color, Color32::RED);
}

#[test]
fn renders_dom_text_with_inline_styles_and_html_whitespace() {
    let page = page(
        "<!doctype html><title>Title</title><h1>Heading</h1><p>A <strong>bold</strong> and <em>italic</em>&nbsp;x<br>end</p><pre>  a\n b</pre>",
    );
    assert_eq!(page.title, "Title");
    assert_eq!(
        texts(&page),
        ["Heading", "A bold and italic\u{a0}x\nend", "  a\n b"]
    );
    assert_eq!(page.blocks[0].job.sections[0].format.font_id.size, 34.0);
    let sections = &page.blocks[1].job.sections;
    assert!(sections.iter().any(|s| s.format.italics));
    assert!(
        sections
            .iter()
            .any(|s| s.format.coords == VariationCoords::new([("wght", 700.0)]))
    );
}

#[test]
fn escaped_markup_stays_text_while_real_elements_are_styled() {
    let page = page(
        "<!doctype html><p>&lt;h1&gt;Literal&lt;/h1&gt; &amp;lt;em&amp;gt;</p><h1>Heading</h1>",
    );
    assert_eq!(texts(&page), ["<h1>Literal</h1> &lt;em&gt;", "Heading"]);
    assert_eq!(page.blocks[0].job.sections[0].format.font_id.size, 17.0);
    assert_eq!(page.blocks[1].job.sections[0].format.font_id.size, 34.0);
}

#[test]
fn skips_inert_and_hidden_subtrees_and_keeps_image_alt_text() {
    let page = page(
        "<!doctype html><style>hidden css</style><body><script>hidden js</script><template>hidden template</template><div hidden><p>secret</p></div><p>visible <img src='file:///secret' alt='an olive'></p><iframe src='https://example.com'>frame</iframe>",
    );
    assert_eq!(texts(&page), ["visible [Image: an olive]"]);
}

#[test]
fn renders_predecoded_images_with_intrinsic_and_html_dimensions() {
    let document = parse(
        "<p>Before</p><p><a href='/image'><img src='olive.png' alt='Olive' width='120'></a></p><p>After</p>",
    )
    .unwrap()
    .document;
    let image_id = document
        .descendants(document.root())
        .find(|&id| {
            document
                .node(id)
                .and_then(|node| node.as_element())
                .is_some_and(|element| element.name.local.as_str() == "img")
        })
        .unwrap();
    let mut images = HashMap::new();
    images.insert(
        image_id,
        ImageAsset {
            reference: "olive.png".into(),
            image: Arc::new(ColorImage::filled([4, 2], Color32::WHITE)),
        },
    );
    let mut page = Page::new(
        &document,
        false,
        &Stylesheet::from_document(&document),
        &images,
        DEFAULT_VIEWPORT,
    );
    assert!(!page.is_empty());
    assert_eq!(page.images.len(), 1);
    assert_eq!(page.images[0].image.size, [4, 2]);
    assert!(
        page.commands
            .iter()
            .any(|command| matches!(command, Command::Image(_)))
    );
    let mut output = draw(&mut page, 400.0);
    assert!(!output.textures_delta.set.is_empty());
    output.textures_delta.clear();
}

#[test]
fn lists_keep_numbering_and_nested_order() {
    let page = page(
        "<!doctype html><ol start=3><li>  three<ul><li> nested<ul><li>deeper</li></ul></li></ul></li><li value=8>eight</li><li>nine</li></ol>",
    );
    assert_eq!(
        texts(&page),
        ["3. three", "• nested", "• deeper", "8. eight", "9. nine"]
    );
    assert_eq!(
        page.blocks
            .iter()
            .map(|block| block.indent)
            .collect::<Vec<_>>(),
        [0, 1, 2, 0, 0]
    );
}

#[test]
fn reflows_on_resize_and_reuses_layout_when_width_is_unchanged() {
    let mut page = page(&format!(
        "<p>{}",
        "A paragraph that wraps at the window edge. ".repeat(12)
    ));
    let ctx = egui::Context::default();
    let mut layout = |width| {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, 600.0),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            page.show(ui);
        });
        output.textures_delta.clear();
        page.blocks[0].layout.as_ref().unwrap().2.clone()
    };
    let wide = layout(800.0);
    let narrow = layout(300.0);
    assert!(narrow.size().y > wide.size().y);
    assert!(Arc::ptr_eq(&narrow, &layout(300.0)));
}

#[test]
fn preview_limits_and_deep_trees_are_bounded() {
    let text = page(&format!("<p>{}", "🫒".repeat(MAX_TEXT + 1)));
    assert!(text.truncated);
    assert_eq!(text.blocks[0].job.text.chars().count(), MAX_TEXT);
    let blocks = page(&"<p>x".repeat(MAX_BLOCKS + 1));
    assert!(blocks.truncated);
    assert_eq!(blocks.blocks.len(), MAX_BLOCKS);
    let deep = page(&format!("{}end", "<div>".repeat(8_000)));
    assert_eq!(texts(&deep), ["end"]);
}

fn draw(page: &mut Page, width: f32) -> egui::FullOutput {
    let ctx = egui::Context::default();
    ctx.set_fonts(crate::gui::fonts::definitions());
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, 900.0),
            )),
            ..Default::default()
        },
        |ui| {
            page.show(ui);
        },
    )
}

#[test]
fn css_styles_text_hides_subtrees_and_changes_block_flow() {
    let page = page(
        r#"<!doctype html><style>
        .gone { display:none } span.block {display:block; color:#f00}
        p {display:inline; margin:0} strong {font-weight:400; color:blue}
        </style><div class=gone>secret<p>hidden</p></div>
        <p>A <strong>normal</strong></p><span class=block>B</span><p>C</p>"#,
    );
    assert_eq!(texts(&page), ["A normal", "B", "C"]);
    let normal = &page.blocks[0].job.sections[1].format;
    assert_eq!(normal.color, Color32::BLUE);
    assert_eq!(normal.coords, VariationCoords::new([("wght", 400.0)]));
    assert_eq!(page.blocks[1].job.sections[0].format.color, Color32::RED);
}

#[test]
fn border_box_sizes_include_padding_and_borders_at_multiple_viewports() {
    let mut page = page(
        r#"<!doctype html><style>
        * {box-sizing:border-box}
        body {margin:0}
        section {width:100%; max-width:400px; margin:0 auto; padding:20px; border:2px solid red; min-height:120px}
        div {width:100%; height:30px; padding:4px; border:1px solid blue}
        </style><section><div></div></section>"#,
    );
    for viewport in [320.0, 800.0] {
        let mut output = draw(&mut page, viewport);
        output.textures_delta.clear();
        let section = page
            .boxes
            .iter()
            .position(|s| s.max_width == Length::Px(400.0))
            .unwrap();
        let child = page
            .boxes
            .iter()
            .position(|s| s.height == Length::Px(30.0))
            .unwrap();
        let outer = page.box_rects[section];
        let inner = page.box_rects[child];
        assert!((outer.width() - page.box_rects[1].width().min(400.0)).abs() < 0.1);
        assert_eq!(outer.height(), 120.0);
        assert_eq!(inner.width(), outer.width() - 44.0);
        assert_eq!(inner.height(), 30.0);
        assert_eq!(inner.left() - outer.left(), 22.0);
    }
    page.validate().unwrap();
}

#[test]
fn nested_box_geometry_accounts_for_padding_borders_and_auto_margins() {
    let mut page = page(
        r#"<!doctype html><body><div style="width:200px; margin:0 auto; padding:10px; border:2px solid red; background:#def">
        <p style="margin:0; padding:5px; background:blue">child</p></div>"#,
    );
    let mut output = draw(&mut page, 800.0);
    let parent = page
        .boxes
        .iter()
        .position(|s| s.width == Length::Px(200.0))
        .unwrap();
    let child = page
        .boxes
        .iter()
        .position(|s| s.background == Color(0, 0, 255, 255))
        .unwrap();
    let a = page.box_rects[parent];
    let b = page.box_rects[child];
    assert_eq!(a.width(), 224.0);
    assert_eq!(b.width(), 200.0);
    assert_eq!(b.left() - a.left(), 12.0);
    assert_eq!(b.top() - a.top(), 12.0);
    assert_eq!(a.bottom() - b.bottom(), 12.0);
    let body = page.box_rects[1];
    assert!((a.center().x - body.center().x).abs() < 0.1);
    let fills: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Rect(rect) => Some(rect.fill),
            _ => None,
        })
        .collect();
    let outer_fill = fills
        .iter()
        .position(|c| *c == Color32::from_rgb(221, 238, 255))
        .unwrap();
    let inner_fill = fills.iter().position(|c| *c == Color32::BLUE).unwrap();
    assert!(
        outer_fill < inner_fill,
        "parent background must paint behind children"
    );
    output.textures_delta.clear();
}

#[test]
fn whitespace_modes_wrap_and_preserve_as_requested() {
    let text = "A long line with words ".repeat(10);
    let mut page = page(&format!(
        "<p style='white-space:pre-wrap'>  {text}\n  end</p><p style='white-space:nowrap'>{text}</p>"
    ));
    let mut output = draw(&mut page, 260.0);
    assert!(page.blocks[0].job.text.starts_with("  "));
    assert!(page.blocks[0].job.text.contains("\n  end"));
    let wrap = &page.blocks[0].layout.as_ref().unwrap().2;
    let nowrap = &page.blocks[1].layout.as_ref().unwrap().2;
    assert!(wrap.rows.len() > 2);
    assert_eq!(nowrap.rows.len(), 1);
    assert!(nowrap.size().x > 260.0);
    output.textures_delta.clear();
}

#[test]
fn empty_boxes_are_visible_and_limits_leave_balanced_commands() {
    let mut empty = page("<div style='height:40px; background:red; padding:5px'></div>");
    assert!(!empty.is_empty());
    let mut output = draw(&mut empty, 400.0);
    assert_eq!(empty.box_rects.last().unwrap().height(), 50.0);
    output.textures_delta.clear();
    let deep = page(&format!("{}end", "<div>".repeat(MAX_BOXES + 1)));
    assert!(deep.truncated);
    assert_eq!(deep.boxes.len(), MAX_BOXES);
    assert_eq!(
        deep.commands
            .iter()
            .filter(|c| matches!(c, Command::Open(_)))
            .count(),
        deep.commands
            .iter()
            .filter(|c| matches!(c, Command::Close))
            .count()
    );
}
#[test]
fn native_input_edit_enter_and_hidden_controls() {
    let ctx = egui::Context::default();
    let mut page = page(
        "<form><input name=q placeholder='Search olives'><input type=hidden name=secret value=hidden><button>Search</button></form>",
    );
    assert!(!page.is_empty());
    page.validate().unwrap();
    let mut output = ctx.run_ui(Default::default(), |ui| {
        page.show(ui);
    });
    let pos = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == "Search olives" => {
                Some(text.pos + egui::vec2(6.0, 6.0))
            }
            _ => None,
        })
        .unwrap();
    output.textures_delta.clear();
    let events = vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        },
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        },
    ];
    let mut output = ctx.run_ui(
        egui::RawInput {
            events,
            ..Default::default()
        },
        |ui| {
            page.show(ui);
        },
    );
    output.textures_delta.clear();
    let mut output = ctx.run_ui(
        egui::RawInput {
            events: vec![egui::Event::Text("Olive & tea".into())],
            ..Default::default()
        },
        |ui| {
            page.show(ui);
        },
    );
    output.textures_delta.clear();
    assert_eq!(page.forms.controls[0].value, "Olive & tea");
    let mut activation = None;
    let mut output = ctx.run_ui(
        egui::RawInput {
            events: vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        },
        |ui| {
            activation = page.show(ui).or(activation.take());
        },
    );
    output.textures_delta.clear();
    assert_eq!(
        activation.unwrap().form.unwrap().control,
        page.forms.controls[2].node
    );
    assert_eq!(
        page.commands
            .iter()
            .filter(|c| matches!(c, Command::Control(..)))
            .count(),
        2
    );
}

#[test]
fn find_uses_rendered_text_and_highlights_wrapped_unicode_matches() {
    let mut page = page(
        "<p>שלום <strong>Olive</strong> olive</p><p style='display:none'>olive</p><input type=password value=olive><script>olive</script>",
    );
    assert_eq!(page.find_query("olive"), 2);
    assert_eq!(page.find.blocks[0], vec![5..10, 11..16]);
    page.find_current = 1;
    page.find_scroll = true;
    let mut output = draw(&mut page, 100.0);
    output.textures_delta.clear();
    assert!(!page.find_scroll);
    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Rect(rect) if rect.fill == Color32::from_rgba_unmultiplied(255, 145, 20, 115))));
    assert_eq!(page.find_query("missing"), 0);
    assert_eq!(page.find_query(""), 0);
    assert_eq!(page.find_query("שלום"), 1);
}
