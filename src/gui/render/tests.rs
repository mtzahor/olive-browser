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
    let joined = texts(&page).join("|");
    assert!(joined.contains("3.|First|7.|Next"), "{joined}");
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
    let mut page = page(
        "<!doctype html><ol start=3><li>  three<ul><li> nested<ul><li>deeper</li></ul></li></ul></li><li value=8>eight</li><li>nine</li></ol>",
    );
    assert_eq!(
        texts(&page),
        [
            "3.", "three", "◦", "nested", "▪", "deeper", "8.", "eight", "9.", "nine"
        ]
    );
    assert_eq!(
        page.blocks
            .iter()
            .map(|block| block.marker)
            .collect::<Vec<_>>(),
        [
            true, false, true, false, true, false, true, false, true, false
        ]
    );
    // Markers are decoration: search skips them, as in other browsers.
    assert_eq!(page.find_query("3."), 0);
    assert_eq!(page.find_query("nested"), 1);

    let mut output = draw(&mut page, 600.0);
    let text_rect = |wanted: &str| {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == wanted => {
                    Some(text.galley.rect.translate(text.pos.to_vec2()))
                }
                _ => None,
            })
            .unwrap()
    };
    let (three, nested, deeper) = (text_rect("three"), text_rect("nested"), text_rect("deeper"));
    // Each nested list indents its items by the list's start padding.
    assert_eq!(three.left(), 48.0);
    assert_eq!(nested.left() - three.left(), 40.0);
    assert_eq!(deeper.left() - nested.left(), 40.0);
    // Markers hang in that padding, level with their item's first line.
    for (marker, item) in [("3.", three), ("◦", nested), ("▪", deeper)] {
        let marker = text_rect(marker);
        assert!(marker.right() < item.left(), "{marker:?} overlaps {item:?}");
        assert!(marker.left() > item.left() - 40.0);
        assert_eq!(marker.top(), item.top());
    }
    // Items sit on consecutive lines instead of paragraph spacing.
    assert_eq!(text_rect("nine").top() - text_rect("eight").bottom(), 0.0);
    output.textures_delta.clear();
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

/// The laid-out rectangle of the box whose background is `color`.
fn box_with_background(page: &Page, color: Color) -> egui::Rect {
    let index = page
        .boxes
        .iter()
        .position(|style| style.background == color)
        .unwrap_or_else(|| panic!("no box with background {color:?}"));
    page.box_rects[index]
}

#[test]
fn tables_share_column_widths_spans_and_row_heights() {
    let mut page = page(
        r#"<!doctype html><style>td { padding: 0 }</style>
        <table cellspacing=4>
          <tr><td style="background:rgb(1,0,0)">A</td><td style="background:rgb(2,0,0)">Short</td>
              <td rowspan=2 style="background:rgb(3,0,0)">Spans two rows</td></tr>
          <tr><td style="background:rgb(4,0,0)">A much longer first column</td>
              <td style="background:rgb(5,0,0)">One<br>two<br>three</td></tr>
          <tr><td colspan=2 style="background:rgb(6,0,0)">Both columns</td>
              <td style="background:rgb(7,0,0)">C</td></tr>
        </table>"#,
    );
    let mut output = draw(&mut page, 800.0);
    let cell = |red| box_with_background(&page, Color(red, 0, 0, 255));
    let (a, short, spanning) = (cell(1), cell(2), cell(3));
    let (long, tall, both, c) = (cell(4), cell(5), cell(6), cell(7));
    // Columns line up across rows and fit the widest content.
    assert_eq!(a.left(), long.left());
    assert_eq!(a.width(), long.width());
    assert_eq!(short.left(), tall.left());
    assert!(long.width() > 100.0);
    assert_eq!(short.left() - long.right(), 4.0);
    // A spanning cell covers both columns and the spacing between them.
    assert_eq!(both.left(), a.left());
    assert_eq!(both.right(), short.right());
    assert_eq!(spanning.left(), c.left());
    // Cells fill their row's height; the rowspan cell reaches the second row's end.
    assert_eq!(long.height(), tall.height());
    assert_eq!(a.height(), short.height());
    assert_eq!(spanning.top(), a.top());
    assert_eq!(spanning.bottom(), tall.bottom());
    assert_eq!(both.top() - tall.bottom(), 4.0);
    // Without a width, the table shrinks to its columns.
    assert!(c.right() < 700.0);
    output.textures_delta.clear();
}

#[test]
fn collapsed_tables_overlap_borders_and_rtl_tables_mirror_columns() {
    let mut page = page(
        r#"<!doctype html><style>
        table { border-collapse: collapse; border: 2px solid black }
        td { border: 1px solid black; padding: 0 }
        </style>
        <table><tr><td style="background:rgb(1,0,0)">one</td><td style="background:rgb(2,0,0)">two</td></tr>
        <tr><td style="background:rgb(3,0,0)">three</td><td>four</td></tr></table>
        <div dir=rtl><table style="width:300px"><tr><td style="background:rgb(4,0,0)">א</td>
        <td style="background:rgb(5,0,0)">ב</td></tr></table></div>"#,
    );
    let mut output = draw(&mut page, 800.0);
    let cell = |red| box_with_background(&page, Color(red, 0, 0, 255));
    let table = page.box_rects[page
        .boxes
        .iter()
        .position(|style| style.display == Display::Table)
        .unwrap()];
    // Neighboring borders share one line, and outer cells sit on the table's border.
    assert_eq!(cell(2).left(), cell(1).right() - 1.0);
    assert_eq!(cell(3).top(), cell(1).bottom() - 1.0);
    assert_eq!(cell(1).left(), table.left());
    assert_eq!(cell(1).top(), table.top());
    // Right-to-left tables start at the right edge and run leftward.
    let (first, second) = (cell(4), cell(5));
    assert!(first.left() > second.left());
    assert_eq!(first.right(), 800.0 - 8.0);
    output.textures_delta.clear();
}

#[test]
fn soft_shadows_and_rounded_corners_follow_the_box_shape() {
    let mut page = page(
        r#"<!doctype html>
        <div style="width:100px; height:50px; background:#fff; border-radius:50%;
            box-shadow:0 4px 10px 2px rgba(0,0,0,.5)"></div>
        <div style="width:80px; height:40px; border-radius:6px 6px 0 0; background:rgb(9,9,9)"></div>
        <div style="width:60px; height:30px; box-shadow:0 0 8px black"></div>"#,
    );
    let mut output = draw(&mut page, 400.0);
    let rects: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Rect(rect) => Some((shape.clip_rect, rect.clone())),
            _ => None,
        })
        .collect();
    // The opaque pill casts one blurred, offset shadow that follows its radius.
    let pill = box_with_background(&page, Color(255, 255, 255, 255));
    let shadow = &rects
        .iter()
        .find(|(_, rect)| rect.blur_width > 0.0 && rect.fill.a() > 100)
        .unwrap()
        .1;
    assert_eq!(shadow.blur_width, 20.0);
    assert_eq!(
        shadow.rect,
        pill.expand(2.0).translate(egui::vec2(0.0, 4.0))
    );
    assert_eq!(shadow.corner_radius, egui::CornerRadius::same(27));
    let fill = |color| {
        rects
            .iter()
            .find(|(_, rect)| rect.fill == color)
            .unwrap()
            .1
            .corner_radius
    };
    assert_eq!(fill(Color32::WHITE), egui::CornerRadius::same(25));
    assert_eq!(
        fill(Color32::from_rgb(9, 9, 9)),
        egui::CornerRadius {
            nw: 6,
            ne: 6,
            sw: 0,
            se: 0
        }
    );
    // A see-through box keeps its shadow outside itself, in four clipped bands.
    let clear = page.box_rects[page
        .boxes
        .iter()
        .position(|style| style.width == Length::Px(60.0))
        .unwrap()];
    let bands: Vec<_> = rects
        .iter()
        .filter(|(_, rect)| rect.blur_width == 16.0)
        .map(|(clip, _)| *clip)
        .collect();
    assert_eq!(bands.len(), 4);
    assert!(bands.iter().all(|clip| !clip.contains(clear.center())));
    output.textures_delta.clear();
}

#[test]
fn rounded_images_clip_the_picture_inside_their_frame() {
    let document = parse(
        "<!doctype html><img src='face.png' style='width:80px; height:80px; border-radius:50%; border:4px solid red'>",
    )
    .unwrap()
    .document;
    let image_id = document
        .descendants(document.root())
        .find(|&id| {
            document
                .node(id)
                .and_then(|node| node.as_element())
                .is_some_and(|element| element.is_html_tag("img"))
        })
        .unwrap();
    let mut images = HashMap::new();
    images.insert(
        image_id,
        ImageAsset {
            reference: "face.png".into(),
            image: Arc::new(ColorImage::filled([8, 8], Color32::WHITE)),
        },
    );
    let mut page = Page::new(
        &document,
        false,
        &Stylesheet::from_document(&document),
        &images,
        DEFAULT_VIEWPORT,
    );
    let mut output = draw(&mut page, 400.0);
    let picture = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Rect(rect) if rect.brush.is_some() => Some(rect.clone()),
            _ => None,
        })
        .expect("the image paints as a textured rectangle");
    assert_eq!(picture.rect.size(), egui::vec2(80.0, 80.0));
    // The frame is 88px across, so its 44px radius leaves 40px inside the border.
    assert_eq!(picture.corner_radius, egui::CornerRadius::same(40));
    output.textures_delta.clear();
}

#[test]
fn inline_runs_stay_together_and_rtl_flow_aligns_boxes_right() {
    let mut page = page(
        r#"<!doctype html><p>a <span style="position:relative; top:-2px">b</span>
        <sup><a href='#n'>1</a></sup> c</p>
        <div dir=rtl><div style="width:100px; background:rgb(7,7,7)">x</div></div>"#,
    );
    // Relative offsets do not split a paragraph into separate blocks.
    assert_eq!(texts(&page), ["a b 1 c", "x"]);
    let superscript = page.blocks[0]
        .job
        .sections
        .iter()
        .find(|section| {
            page.blocks[0].job.text[section.byte_range.start.0..section.byte_range.end.0].trim()
                == "1"
        })
        .unwrap();
    assert_eq!(superscript.format.valign, egui::Align::TOP);
    let mut output = draw(&mut page, 600.0);
    let narrow = box_with_background(&page, Color(7, 7, 7, 255));
    assert_eq!(narrow.right(), 600.0 - 8.0);
    output.textures_delta.clear();
}

#[test]
fn default_rules_are_quiet_and_authored_rules_keep_their_own_paint() {
    let mut page = page("<!doctype html><hr><hr style='border:1px solid rgb(200,0,0); height:0'>");
    let mut output = draw(&mut page, 400.0);
    let lines: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::LineSegment { stroke, .. } => Some(stroke.color),
            _ => None,
        })
        .collect();
    assert_eq!(lines, [Color32::from_rgb(150, 156, 140)]);
    assert!(output.shapes.iter().any(|shape| matches!(
        &shape.shape,
        egui::Shape::Rect(rect) if rect.stroke.color == Color32::from_rgb(200, 0, 0)
    )));
    output.textures_delta.clear();
}

#[test]
fn large_tables_fall_back_to_inline_rows_before_the_display_limit() {
    let rows: String = (0..1_200)
        .map(|row| {
            format!("<tr><td>r{row}a</td><td>r{row}b</td><td>r{row}c</td><td>r{row}d</td></tr>")
        })
        .collect();
    let mut page = page(&format!("<!doctype html><table>{rows}</table><p>After</p>"));
    assert!(!page.truncated);
    assert!(page.boxes.len() <= MAX_BOXES && page.blocks.len() <= MAX_BLOCKS);
    // Early rows keep real cells; later ones become one line per row.
    assert!(texts(&page).contains(&"r0a"));
    assert!(texts(&page).contains(&"r1199a r1199b r1199c r1199d"));
    assert_eq!(texts(&page).last(), Some(&"After"));
    let mut output = draw(&mut page, 800.0);
    output.textures_delta.clear();
    page.validate().unwrap();
}

#[test]
fn floats_line_up_beside_each_other_and_shape_following_content() {
    let mut page = page(
        r#"<!doctype html><style>
        body { margin: 0 } .bar { width: 600px } p { margin: 0 }
        .logo { float: left; width: 100px; height: 40px; background: rgb(1,0,0) }
        .a { float: right; background: rgb(2,0,0) } .b { float: right; background: rgb(3,0,0) }
        .menu { float: left; background: rgb(4,0,0) } .clear { clear: both; background: rgb(5,0,0) }
        .tile { float: left; width: 50%; padding-right: 20px; box-sizing: border-box; height: 30px }
        </style>
        <div class=bar><div class=logo></div>
          <nav style="background:rgb(6,0,0)"><span class=a>Login</span><span class=b>Search</span>
          <span class=menu>News Features</span></nav>
          <p>Text beside the logo</p>
          <div class=clear>Below</div>
          <div class=tile style="background:rgb(7,0,0)"></div><div class=tile style="background:rgb(8,0,0)"></div>
          <div class=tile style="background:rgb(9,0,0)"></div>
        </div>"#,
    );
    let mut output = draw(&mut page, 800.0);
    let fill = |red| box_with_background(&page, Color(red, 0, 0, 255));
    let (logo, login, search, menu, nav) = (fill(1), fill(2), fill(3), fill(4), fill(6));
    // A block after a float takes the room beside it.
    assert_eq!(nav.left(), logo.right());
    assert_eq!(nav.top(), logo.top());
    // Right floats stack from the right edge in source order; left floats from the left.
    assert_eq!(login.right(), 600.0);
    assert_eq!(search.right(), login.left());
    assert_eq!(menu.left(), nav.left());
    assert_eq!(login.top(), nav.top());
    // Floats shrink to fit their text.
    assert!(menu.width() < 200.0 && login.width() < 80.0);
    // Text beside the logo starts after it; `clear` moves below every float.
    let beside = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == "Text beside the logo" => {
                Some(text.pos)
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(beside.x, logo.right());
    assert!(fill(5).top() >= logo.bottom());
    // Border-box halves sit side by side and the third wraps beneath them.
    let (one, two, three) = (fill(7), fill(8), fill(9));
    assert_eq!(two.left(), one.right());
    assert_eq!(two.top(), one.top());
    assert_eq!(three.left(), one.left());
    assert_eq!(three.top(), one.bottom());
    // The container grows to hold its floats.
    let bar = page.box_rects[page
        .boxes
        .iter()
        .position(|style| style.width == Length::Px(600.0))
        .unwrap()];
    assert!(bar.bottom() >= three.bottom());
    output.textures_delta.clear();
}

#[test]
fn hidden_overflow_clips_contents_but_not_the_box_itself() {
    let mut page = page(
        r#"<!doctype html><a href='#main' style="position:absolute; width:1px; height:1px; overflow:hidden">Skip to content</a>
        <div style="width:200px; height:40px; overflow:hidden; background:rgb(1,2,3)"><p>Clipped</p><p>Clipped</p><p>Clipped</p></div>
        <div style="width:200px; overflow:auto"><p>Scrollable</p></div>"#,
    );
    let mut output = draw(&mut page, 600.0);
    let clip_of = |wanted: &str| {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == wanted => Some(shape.clip_rect),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let skip = clip_of("Skip to content");
    assert!(
        skip.iter()
            .all(|clip| clip.width() <= 1.0 && clip.height() <= 1.0)
    );
    let frame = box_with_background(&page, Color(1, 2, 3, 255));
    assert!(clip_of("Clipped").iter().all(|clip| *clip == frame));
    // `overflow: auto` keeps content visible, as Olive has no inner scrolling.
    assert!(
        clip_of("Scrollable")
            .iter()
            .all(|clip| clip.width() > 200.0)
    );
    // The clipping box's own background still paints with the page clip.
    assert!(output.shapes.iter().any(|shape| matches!(
        &shape.shape,
        egui::Shape::Rect(rect) if rect.fill == Color32::from_rgb(1, 2, 3) && shape.clip_rect.contains_rect(frame)
    )));
    output.textures_delta.clear();
}

#[test]
fn empty_relative_frames_take_the_height_of_an_absolute_picture() {
    let document = parse(
        "<!doctype html><figure style='position:relative; width:200px; margin:0'>         <a href='/story' style='position:absolute; top:0; left:0; width:100%'><img src='thumb.png' style='width:100%'></a></figure>         <p>Headline</p>",
    )
    .unwrap()
    .document;
    let image_id = document
        .descendants(document.root())
        .find(|&id| {
            document
                .node(id)
                .and_then(|node| node.as_element())
                .is_some_and(|element| element.is_html_tag("img"))
        })
        .unwrap();
    let images = HashMap::from([(
        image_id,
        ImageAsset {
            reference: "thumb.png".into(),
            image: Arc::new(ColorImage::filled([40, 20], Color32::WHITE)),
        },
    )]);
    let mut page = Page::new(
        &document,
        false,
        &Stylesheet::from_document(&document),
        &images,
        DEFAULT_VIEWPORT,
    );
    let mut output = draw(&mut page, 600.0);
    let figure = page.box_rects[page
        .boxes
        .iter()
        .position(|style| style.width == Length::Px(200.0))
        .unwrap()];
    assert_eq!(figure.height(), 100.0);
    let headline = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == "Headline" => Some(text.pos.y),
            _ => None,
        })
        .unwrap();
    assert!(headline >= figure.bottom());
    output.textures_delta.clear();
}

/// The paint order of the box filled with each color, among `output`'s shapes.
fn paint_order(output: &egui::FullOutput, colors: &[Color32]) -> Vec<usize> {
    colors
        .iter()
        .map(|&color| {
            output
                .shapes
                .iter()
                .position(
                    |shape| matches!(&shape.shape, egui::Shape::Rect(rect) if rect.fill == color),
                )
                .unwrap_or_else(|| panic!("no shape filled with {color:?}"))
        })
        .collect()
}

#[test]
fn fixed_headers_stay_out_of_flow_and_paint_above_the_page() {
    // GSMArena's header: fixed, floated and full width, above a padded wrapper.
    let mut page = page(
        r#"<!doctype html><body style="margin:0">
        <header style="position:fixed; float:left; top:0; left:0; min-width:100%; height:50px;
            z-index:1; background:rgb(1,0,0)">Header</header>
        <div style="position:relative; z-index:0; padding-top:40px; background:rgb(2,0,0)">
            <p style="margin:0">Content</p></div>"#,
    );
    let mut output = draw(&mut page, 600.0);
    let header = box_with_background(&page, Color(1, 0, 0, 255));
    let wrapper = box_with_background(&page, Color(2, 0, 0, 255));
    assert_eq!(header.min, egui::Pos2::ZERO);
    assert_eq!(header.width(), 600.0);
    // The float does not push the page aside, and the header paints over it.
    assert_eq!(wrapper.min, egui::Pos2::ZERO);
    assert_eq!(wrapper.width(), 600.0);
    let order = paint_order(
        &output,
        &[Color32::from_rgb(2, 0, 0), Color32::from_rgb(1, 0, 0)],
    );
    assert!(order[0] < order[1]);
    output.textures_delta.clear();
}

#[test]
fn reversed_rows_start_at_the_right_and_wrapped_columns_fill_fixed_heights() {
    let mut page = page(
        r#"<!doctype html><body style="margin:0">
        <div style="display:flex; flex-direction:row-reverse; width:300px">
            <div style="width:100px; background:rgb(1,0,0)">a</div>
            <div style="width:50px; background:rgb(2,0,0)">b</div></div>
        <ul style="display:flex; flex-flow:column wrap; height:60px; width:200px; margin:0; padding:0">
            <li style="height:20px; width:50px; background:rgb(3,0,0)"></li>
            <li style="height:20px; width:50px; background:rgb(4,0,0)"></li>
            <li style="height:20px; width:50px; background:rgb(5,0,0)"></li>
            <li style="height:20px; width:50px; background:rgb(6,0,0)"></li></ul>"#,
    );
    let mut output = draw(&mut page, 600.0);
    let first = box_with_background(&page, Color(1, 0, 0, 255));
    let second = box_with_background(&page, Color(2, 0, 0, 255));
    assert_eq!(first.x_range(), egui::Rangef::new(200.0, 300.0));
    assert_eq!(second.x_range(), egui::Rangef::new(150.0, 200.0));
    let column_top = box_with_background(&page, Color(3, 0, 0, 255));
    let third = box_with_background(&page, Color(5, 0, 0, 255));
    let fourth = box_with_background(&page, Color(6, 0, 0, 255));
    assert_eq!(third.left(), column_top.left());
    assert_eq!(third.top(), column_top.top() + 40.0);
    assert_eq!(fourth.left(), column_top.left() + 50.0);
    assert_eq!(fourth.top(), column_top.top());
    output.textures_delta.clear();
}

#[test]
fn absolute_boxes_use_their_positioned_ancestor_and_follow_z_index() {
    // A review card: a picture and a caption placed in the card, not in the
    // static link that wraps them, with the caption painted above the picture.
    let mut page = page(
        r#"<!doctype html><body style="margin:0">
        <article style="position:relative; width:200px; height:100px; overflow:hidden">
          <a href="/review" style="display:block">
            <header style="position:absolute; bottom:0; left:0; width:100%; z-index:1;
                background:rgb(1,0,0)">Caption</header>
            <div style="position:absolute; top:0; left:-50%; height:100%; width:300px;
                z-index:0; background:rgb(2,0,0)"></div>
          </a></article>"#,
    );
    let mut output = draw(&mut page, 600.0);
    output.textures_delta.clear();
    // Bottom edges settle on the next frame, once the caption's height is known.
    let mut output = draw(&mut page, 600.0);
    let caption = box_with_background(&page, Color(1, 0, 0, 255));
    let picture = box_with_background(&page, Color(2, 0, 0, 255));
    assert_eq!(picture.left(), -100.0);
    assert_eq!(picture.height(), 100.0);
    assert_eq!(picture.width(), 300.0);
    assert_eq!(caption.bottom(), 100.0);
    assert_eq!(caption.width(), 200.0);
    let order = paint_order(
        &output,
        &[Color32::from_rgb(2, 0, 0), Color32::from_rgb(1, 0, 0)],
    );
    assert!(order[0] < order[1]);
    output.textures_delta.clear();
}

#[test]
fn percentage_heights_need_a_definite_container() {
    let mut page = page(
        r#"<!doctype html><body style="margin:0">
        <div style="height:200px"><div style="height:50%; background:rgb(1,0,0)"></div></div>
        <div><div style="height:50%; background:rgb(2,0,0)">auto</div></div>"#,
    );
    let mut output = draw(&mut page, 600.0);
    assert_eq!(
        box_with_background(&page, Color(1, 0, 0, 255)).height(),
        100.0
    );
    assert!(box_with_background(&page, Color(2, 0, 0, 255)).height() < 40.0);
    output.textures_delta.clear();
}

#[test]
fn uneven_borders_paint_each_side_and_take_their_own_space() {
    let mut page = page(
        r#"<!doctype html><body style="margin:0">
        <div style="width:100px; height:20px; border-bottom:3px solid rgb(200,0,0);
            border-left:1px solid rgb(0,0,200); background:rgb(1,0,0)"></div>
        <p style="margin:0">After</p>"#,
    );
    let mut output = draw(&mut page, 600.0);
    let rect = box_with_background(&page, Color(1, 0, 0, 255));
    assert_eq!(rect.size(), egui::vec2(101.0, 23.0));
    fn fills(shape: &egui::Shape, colors: &mut Vec<Color32>) {
        match shape {
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| fills(shape, colors)),
            egui::Shape::Path(path) => colors.push(path.fill),
            _ => {}
        }
    }
    let mut sides = Vec::new();
    output
        .shapes
        .iter()
        .for_each(|shape| fills(&shape.shape, &mut sides));
    assert!(sides.contains(&Color32::from_rgb(200, 0, 0)));
    assert!(sides.contains(&Color32::from_rgb(0, 0, 200)));
    output.textures_delta.clear();
}

#[test]
fn object_fallbacks_text_transforms_and_icon_font_glyphs() {
    let page = page(
        "<!doctype html><object data='logo.svg' type='image/svg+xml'><p>Fallback logo</p></object>\
         <p style='text-transform:uppercase'>mixed Case</p>\
         <p style='text-transform:capitalize'>hello <b>wide</b> world</p>\
         <p style='text-transform:lowercase'>QUIET</p>\
         <p>Search\u{e962} here</p>",
    );
    assert_eq!(
        texts(&page),
        [
            "Fallback logo",
            "MIXED CASE",
            "Hello Wide World",
            "quiet",
            "Search here"
        ]
    );
}

#[test]
fn text_below_the_visible_page_is_shaped_over_later_frames() {
    // Enough text that shaping all of it takes longer than one frame's share,
    // even in optimized builds.
    let words = "Long paragraphs wrap across many lines in a narrow column. ".repeat(10);
    let html: String = (0..80)
        .map(|i| format!("<p>Paragraph {i}. {words}</p>"))
        .collect();
    let mut deferred = page(&html);
    let mut exact = page(&html);
    // Find scrolling shapes the whole page in one frame.
    exact.find_scroll = true;
    // Separate contexts, so egui's layout cache cannot shape for the other page.
    let contexts = [egui::Context::default(), egui::Context::default()];
    let input = || egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(300.0, 100.0),
        )),
        ..Default::default()
    };
    let frame = |page: &mut Page| {
        let ctx = &contexts[usize::from(page.find_scroll)];
        let mut output = ctx.run_ui(input(), |ui| {
            page.show(ui);
        });
        output.textures_delta.clear();
        output.viewport_output[&egui::ViewportId::ROOT]
            .repaint_delay
            .is_zero()
    };
    frame(&mut exact);
    assert!(exact.blocks.iter().all(|block| block.layout.is_some()));
    assert!(
        frame(&mut deferred),
        "estimated text asks for another frame"
    );
    assert!(deferred.blocks.first().unwrap().layout.is_some());
    assert!(deferred.blocks.last().unwrap().layout.is_none());
    let mut frames = 1;
    while frame(&mut deferred) {
        frames += 1;
        assert!(frames < 1000, "shaping must finish");
    }
    frame(&mut exact);
    assert!(deferred.blocks.iter().all(|block| block.layout.is_some()));
    for (shaped, expected) in deferred.box_rects.iter().zip(&exact.box_rects) {
        assert!(
            (shaped.min - expected.min).length() < 0.01,
            "{shaped:?} {expected:?}"
        );
        assert!(
            (shaped.max - expected.max).length() < 0.01,
            "{shaped:?} {expected:?}"
        );
    }
}

#[test]
fn updates_keep_the_shaped_lines_of_unchanged_text() {
    let ctx = egui::Context::default();
    let mut previous = page("<p>Unchanged text</p><p>Old ending</p>");
    let mut output = ctx.run_ui(Default::default(), |ui| {
        previous.show(ui);
    });
    output.textures_delta.clear();
    let shaped = previous.blocks[0].layout.as_ref().unwrap().2.clone();
    let mut updated = page("<p>Unchanged text</p><p>New ending</p>");
    updated.reuse_text_layout(&mut previous);
    assert!(Arc::ptr_eq(
        &updated.blocks[0].layout.as_ref().unwrap().2,
        &shaped
    ));
    assert_eq!(updated.blocks[0].visual_to_logical.len(), 14);
    assert!(updated.blocks[1].layout.is_none());
    // Text in another direction is shaped again.
    let mut previous = page("<p>Unchanged text</p>");
    let mut output = ctx.run_ui(Default::default(), |ui| {
        previous.show(ui);
    });
    output.textures_delta.clear();
    let mut rtl = page("<p dir=rtl>Unchanged text</p>");
    rtl.reuse_text_layout(&mut previous);
    assert!(rtl.blocks[0].layout.is_none());
}
