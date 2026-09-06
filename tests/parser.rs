use olive_html::{
    Document, NodeId, NodeKind, ParseError, ParseOptions, QuirksMode, parse, parse_reader,
    parse_utf8,
};
use std::{
    collections::HashSet,
    io::{self, Read},
};

fn find(doc: &Document, name: &str) -> NodeId {
    doc.descendants(doc.root())
        .find(|&id| {
            doc.node(id)
                .unwrap()
                .as_element()
                .is_some_and(|e| e.name.local.as_ref() == name)
        })
        .unwrap_or_else(|| panic!("missing element {name}"))
}

fn text(doc: &Document, root: NodeId) -> String {
    doc.descendants(root)
        .filter_map(|id| match &doc.node(id).unwrap().kind {
            NodeKind::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn element_names(doc: &Document, root: NodeId) -> Vec<String> {
    doc.children(root)
        .filter_map(|id| {
            doc.node(id)
                .unwrap()
                .as_element()
                .map(|e| e.name.local.to_string())
        })
        .collect()
}

fn assert_tree_integrity(doc: &Document) {
    let mut visited = HashSet::new();
    let mut pending = vec![doc.root()];
    while let Some(parent) = pending.pop() {
        assert!(visited.insert(parent), "cycle or multiply-owned node");
        assert!(visited.len() <= doc.node_count());
        let children: Vec<_> = doc.children(parent).take(doc.node_count() + 1).collect();
        assert!(children.len() <= doc.node_count(), "sibling cycle");
        for child in children {
            assert_eq!(doc.node(child).unwrap().parent(), Some(parent));
            pending.push(child);
        }
        if let Some(fragment) = doc
            .node(parent)
            .unwrap()
            .as_element()
            .and_then(|e| e.template_contents)
        {
            assert!(doc.node(fragment).unwrap().parent().is_none());
            pending.push(fragment);
        }
    }
}

#[test]
fn inserts_document_structure_and_recovers_omitted_end_tags() {
    let parsed = parse("<!doctype html><title>Olive</title><p>one<p>two<ul><li>a<li>b").unwrap();
    let doc = &parsed.document;
    assert_eq!(doc.quirks_mode(), QuirksMode::NoQuirks);
    assert_eq!(element_names(doc, find(doc, "html")), ["head", "body"]);
    assert_eq!(element_names(doc, find(doc, "body")), ["p", "p", "ul"]);
    assert_eq!(element_names(doc, find(doc, "ul")), ["li", "li"]);
    assert_eq!(text(doc, find(doc, "title")), "Olive");
    assert_tree_integrity(doc);
}

#[test]
fn empty_and_legacy_documents_have_correct_quirks_modes() {
    assert_eq!(
        parse("").unwrap().document.quirks_mode(),
        QuirksMode::Quirks
    );
    let doc = parse("<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.0 Transitional//EN\" \"http://www.w3.org/TR/xhtml1/DTD/xhtml1-transitional.dtd\">").unwrap().document;
    assert_eq!(doc.quirks_mode(), QuirksMode::LimitedQuirks);
    assert_eq!(element_names(&doc, find(&doc, "html")), ["head", "body"]);
}

#[test]
fn decodes_full_character_references_and_normalizes_attributes() {
    let doc = parse("<!doctype html><P ID='first' id=second disabled title='&notit; &notin; &amp=1'>&CounterClockwiseContourIntegral; &#x1FAD2; &#0; &#x80; &NotEqualTilde;</P>").unwrap().document;
    let p = find(&doc, "p");
    let element = doc.node(p).unwrap().as_element().unwrap();
    assert_eq!(element.attribute("id"), Some("first"));
    assert_eq!(element.attribute("disabled"), Some(""));
    assert_eq!(element.attribute("title"), Some("&notit; ∉ &amp=1"));
    assert!(element.had_duplicate_attributes);
    assert_eq!(text(&doc, p), "∳ 🫒 � € ≂̸");
}

#[test]
fn raw_text_rcdata_void_elements_and_scripting_disabled() {
    let doc = parse("<!doctype html><title>A &amp; B</title><style>a>b &amp;</style><body><script>if (a<b) x='&amp;';</script><noscript><p>fallback</p></noscript><textarea>\n&lt;b&gt;&amp;</textarea><br><img src=x><p>end").unwrap().document;
    assert_eq!(text(&doc, find(&doc, "title")), "A & B");
    assert_eq!(text(&doc, find(&doc, "style")), "a>b &amp;");
    assert_eq!(text(&doc, find(&doc, "script")), "if (a<b) x='&amp;';");
    assert_eq!(element_names(&doc, find(&doc, "noscript")), ["p"]);
    assert_eq!(text(&doc, find(&doc, "textarea")), "<b>&");
    assert_eq!(doc.children(find(&doc, "img")).count(), 0);
    assert_tree_integrity(&doc);
}

#[test]
fn foster_parents_table_text_and_inserts_tbody() {
    let doc = parse("<!doctype html><table>before<tr><td>cell</td></tr>after</table>")
        .unwrap()
        .document;
    let body = find(&doc, "body");
    let children: Vec<_> = doc.children(body).collect();
    assert!(
        matches!(&doc.node(children[0]).unwrap().kind, NodeKind::Text(t) if t == "beforeafter")
    );
    assert_eq!(element_names(&doc, find(&doc, "table")), ["tbody"]);
    assert_eq!(text(&doc, find(&doc, "td")), "cell");
    assert_tree_integrity(&doc);
}

#[test]
fn repairs_misnested_formatting_with_adoption_agency_algorithm() {
    let doc = parse("<!doctype html><p><b>one<i>two</b>three</i>four")
        .unwrap()
        .document;
    let p = find(&doc, "p");
    assert_eq!(element_names(&doc, p), ["b", "i"]);
    let elements: Vec<_> = doc
        .children(p)
        .filter(|&id| doc.node(id).unwrap().as_element().is_some())
        .collect();
    assert_eq!(text(&doc, elements[0]), "onetwo");
    assert_eq!(text(&doc, elements[1]), "three");
    assert_eq!(text(&doc, p), "onetwothreefour");
    assert_tree_integrity(&doc);
}

#[test]
fn template_contents_are_separate_and_nest() {
    let doc = parse("<!doctype html><template><p>A<template>B</template>C</p></template><p>D")
        .unwrap()
        .document;
    let template = find(&doc, "template");
    assert_eq!(doc.children(template).count(), 0);
    let fragment = doc
        .node(template)
        .unwrap()
        .as_element()
        .unwrap()
        .template_contents
        .unwrap();
    assert!(matches!(
        doc.node(fragment).unwrap().kind,
        NodeKind::DocumentFragment
    ));
    assert_eq!(text(&doc, fragment), "AC");
    assert_eq!(text(&doc, doc.root()), "D");
    assert_tree_integrity(&doc);
}

#[test]
fn preserves_foreign_namespaces_attributes_and_integration_points() {
    let doc = parse("<!doctype html><svg viewbox='0 0 10 10'><a xlink:href='#x'/><foreignObject><p>HTML</p></foreignObject></svg><math><annotation-xml encoding='text/html'><div>more HTML</div></annotation-xml></math>").unwrap().document;
    let svg = doc.node(find(&doc, "svg")).unwrap().as_element().unwrap();
    assert_eq!(svg.name.ns.as_ref(), "http://www.w3.org/2000/svg");
    assert_eq!(svg.attribute("viewBox"), Some("0 0 10 10"));
    let a = doc.node(find(&doc, "a")).unwrap().as_element().unwrap();
    assert_eq!(
        a.attributes[0].name.ns.as_ref(),
        "http://www.w3.org/1999/xlink"
    );
    for name in ["p", "div"] {
        assert_eq!(
            doc.node(find(&doc, name))
                .unwrap()
                .as_element()
                .unwrap()
                .name
                .ns
                .as_ref(),
            "http://www.w3.org/1999/xhtml"
        );
    }
    assert_tree_integrity(&doc);
}

#[test]
fn repeated_html_body_tags_merge_only_missing_attributes() {
    let doc = parse("<!doctype html><html lang=en><body id=first><html lang=fr dir=rtl><body id=second class=merged>hello").unwrap().document;
    let html = doc.node(find(&doc, "html")).unwrap().as_element().unwrap();
    assert_eq!(html.attribute("lang"), Some("en"));
    assert_eq!(html.attribute("dir"), Some("rtl"));
    let body = doc.node(find(&doc, "body")).unwrap().as_element().unwrap();
    assert_eq!(body.attribute("id"), Some("first"));
    assert_eq!(body.attribute("class"), Some("merged"));
}

#[test]
fn normalizes_newlines_bom_and_invalid_utf8() {
    let parsed = parse_utf8(
        b"\xef\xbb\xbf<!doctype html><p>A\r\nB\rC\xffD\0E",
        ParseOptions::default(),
    )
    .unwrap();
    assert_eq!(
        text(&parsed.document, find(&parsed.document, "p")),
        "A\nB\nC�DE"
    );
    assert!(!parsed.diagnostics.is_empty());
    assert_eq!(parsed.document.quirks_mode(), QuirksMode::NoQuirks);
}

#[test]
fn comments_bogus_comments_and_eof_recovery() {
    let doc = parse("<!--before--><!doctype html><p>a<!bogus><!--unfinished")
        .unwrap()
        .document;
    let comments: Vec<_> = doc
        .descendants(doc.root())
        .filter_map(|id| match &doc.node(id).unwrap().kind {
            NodeKind::Comment(t) => Some(t.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(comments, ["before", "bogus", "unfinished"]);
    assert_tree_integrity(&doc);
}

#[test]
fn limits_input_before_parsing_and_counts_bytes() {
    let options = ParseOptions {
        max_input_bytes: 4,
        ..Default::default()
    };
    assert!(parse_utf8("🫒".as_bytes(), options).is_ok());
    assert!(matches!(
        parse_utf8("🫒x".as_bytes(), options),
        Err(ParseError::InputTooLarge { limit: 4 })
    ));
    assert!(
        parse_utf8(
            b"",
            ParseOptions {
                max_input_bytes: 0,
                ..options
            }
        )
        .is_ok()
    );
    let mut infinite = io::repeat(b'x');
    assert!(matches!(
        parse_reader(&mut infinite, options),
        Err(ParseError::InputTooLarge { .. })
    ));
}

#[test]
fn propagates_reader_failures() {
    struct Broken;
    impl Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("read failed"))
        }
    }
    assert!(matches!(
        parse_reader(Broken, ParseOptions::default()),
        Err(ParseError::Io(_))
    ));
}

#[test]
fn caps_diagnostics_without_stopping_recovery() {
    for max_diagnostics in [0, 2] {
        let parsed = parse_utf8(
            b"<p a=1 a=2>\0</bad></bad></bad>",
            ParseOptions {
                max_diagnostics,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(parsed.diagnostics.len(), max_diagnostics);
        assert!(parsed.omitted_diagnostics > 0);
        assert_tree_integrity(&parsed.document);
    }
}

#[test]
fn deep_trees_can_be_walked_printed_and_dropped_on_a_small_stack() {
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(|| {
            let depth = 8_000;
            let doc = parse(&format!("<!doctype html>{}end", "<div>".repeat(depth)))
                .unwrap()
                .document;
            assert_eq!(doc.descendants(doc.root()).count(), depth + 5);
            doc.write_tree(io::sink()).unwrap();
            drop(doc);
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn diagnostic_tree_escapes_terminal_control_characters() {
    let doc = parse("<!doctype html><p title='\x1b[31m'>\x1b[2J<!--\x07-->")
        .unwrap()
        .document;
    let mut output = Vec::new();
    doc.write_tree(&mut output).unwrap();
    assert!(!output.contains(&0x1b));
    assert!(!output.contains(&7));
}

#[test]
fn deterministic_malformed_corpus_preserves_tree_invariants() {
    let tokens = [
        "<p>",
        "<table>",
        "<tr>",
        "<td>",
        "<b>",
        "<i>",
        "<a>",
        "<template>",
        "</template>",
        "<svg>",
        "<math>",
        "</table>",
        "</p>",
        "</b>",
        "</i>",
        "text",
        "&amp;",
        "<!--x-->",
        "<select>",
        "<option>",
        "</select>",
        "<form>",
        "</form>",
        "<div>",
        "</div>",
    ];
    let mut state = 0x0001_101e_u64;
    for _ in 0..200 {
        let mut input = String::from("<!doctype html>");
        for _ in 0..80 {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            input.push_str(tokens[(state >> 32) as usize % tokens.len()]);
        }
        let doc = parse(&input).unwrap().document;
        assert_tree_integrity(&doc);
    }
}
