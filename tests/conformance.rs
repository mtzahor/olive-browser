//! Pinned html5lib tree-construction expectations, independent of our DOM sink.
use olive_html::{Document, NodeId, NodeKind, parse};
use std::fmt::Write;

fn canonical_tree(doc: &Document) -> String {
    let mut output = String::new();
    let mut pending: Vec<(NodeId, usize)> = doc.children(doc.root()).map(|id| (id, 0)).collect();
    pending.reverse();
    while let Some((id, depth)) = pending.pop() {
        let indent = format!("| {}", "  ".repeat(depth));
        let node = doc.node(id).unwrap();
        match &node.kind {
            NodeKind::Doctype {
                name,
                public_id,
                system_id,
            } => {
                if public_id.is_empty() && system_id.is_empty() {
                    writeln!(output, "{indent}<!DOCTYPE {name}>").unwrap();
                } else {
                    writeln!(
                        output,
                        "{indent}<!DOCTYPE {name} \"{public_id}\" \"{system_id}\">"
                    )
                    .unwrap();
                }
            }
            NodeKind::Element(element) => {
                let namespace = match element.name.ns.as_ref() {
                    "http://www.w3.org/2000/svg" => "svg ",
                    "http://www.w3.org/1998/Math/MathML" => "math ",
                    _ => "",
                };
                writeln!(output, "{indent}<{namespace}{}>", element.name.local).unwrap();
                let mut attrs: Vec<_> = element.attributes.iter().collect();
                attrs.sort_by(|a, b| a.name.local.cmp(&b.name.local));
                for attr in attrs {
                    let namespace = match attr.name.ns.as_ref() {
                        "http://www.w3.org/1999/xlink" => "xlink ",
                        "http://www.w3.org/XML/1998/namespace" => "xml ",
                        "http://www.w3.org/2000/xmlns/" => "xmlns ",
                        _ => "",
                    };
                    writeln!(
                        output,
                        "{indent}  {namespace}{}=\"{}\"",
                        attr.name.local, attr.value
                    )
                    .unwrap();
                }
                if let Some(fragment) = element.template_contents {
                    pending.push((fragment, depth + 1));
                }
            }
            NodeKind::Text(text) => writeln!(output, "{indent}\"{text}\"").unwrap(),
            NodeKind::Comment(text) => writeln!(output, "{indent}<!-- {text} -->").unwrap(),
            NodeKind::DocumentFragment => writeln!(output, "{indent}content").unwrap(),
            NodeKind::ProcessingInstruction { target, data } => {
                writeln!(output, "{indent}<?{target} {data}>").unwrap()
            }
            NodeKind::Document => unreachable!(),
        }
        let children: Vec<_> = doc.children(id).collect();
        pending.extend(children.into_iter().rev().map(|child| (child, depth + 1)));
    }
    output
}

#[test]
fn html5lib_tests1_document_trees() {
    let fixture = include_str!("fixtures/html5lib/tests1.dat");
    let mut count = 0;
    for (index, case) in fixture.split("#data\n").skip(1).enumerate() {
        let (input, rest) = case
            .split_once("\n#errors\n")
            .expect("fixture errors section");
        assert!(
            !rest.contains("#script-on"),
            "fixture requires unsupported scripting mode"
        );
        assert!(
            !rest.contains("#document-fragment"),
            "fixture requires fragment API"
        );
        let (_, expected) = rest
            .split_once("#document\n")
            .expect("fixture document section");
        let doc = parse(input).unwrap().document;
        assert_eq!(
            canonical_tree(&doc).trim_end_matches('\n'),
            expected.trim_end_matches('\n'),
            "html5lib tests1 case {}: {input:?}",
            index + 1
        );
        count += 1;
    }
    assert_eq!(count, 112, "ensure all vendored cases were run");
}
