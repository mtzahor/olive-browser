//! Inspect one public page without executing JavaScript. Used by the live audit.
use olive_html::{
    css::{ComputedStyle, StyleBudget, Stylesheet},
    net::{DocumentLoader, Location},
    resources::PageResources,
};
use serde_json::{Value, json};
use std::{collections::HashMap, time::Instant};

fn inspect(address: &str) -> Result<Value, String> {
    let loader = DocumentLoader::new()?;
    let loaded = loader.load(Location::from_input(address)?)?;
    let document = loaded.parse(false)?.document;
    let resources = PageResources::load(&loader, &loaded.location, &document, false);
    let sheet = Stylesheet::from_document_with_sources(&document, &resources.styles);
    let mut budget = StyleBudget::default();
    let mut styles = HashMap::new();
    let mut root_font = 17.0;
    let started = Instant::now();
    for id in document.descendants(document.root()) {
        let node = document.node(id).unwrap();
        let Some(element) = node.as_element() else {
            continue;
        };
        let parent = node
            .parent()
            .and_then(|id| styles.get(&id))
            .copied()
            .unwrap_or_else(ComputedStyle::default);
        let style = sheet.compute(&document, id, parent, root_font, &mut budget);
        if element.name.local.as_ref() == "html" {
            root_font = style.font_size;
        }
        styles.insert(id, style);
    }
    Ok(json!({
        "final_url": loaded.location.as_str(),
        "http_status": loaded.status,
        "html_bytes": loaded.bytes.len(),
        "elements": styles.len(),
        "resources": resources.report,
        "stylesheets": resources.styles.len(),
        "images": resources.images.len(),
        "css_bytes": resources.styles.values().map(|s| s.source.len()).sum::<usize>(),
        "image_bytes": resources.images.values().map(|s| s.bytes.len()).sum::<usize>(),
        "css_rules": sheet.rule_count(),
        "css_ignored": sheet.diagnostics.ignored + budget.ignored,
        "css_parse_limited": sheet.diagnostics.limited,
        "css_match_limited": budget.limited,
        "style_ms": started.elapsed().as_millis(),
    }))
}

fn main() {
    let address = std::env::args().nth(1).expect("Supply a URL or file path");
    let started = Instant::now();
    let mut report = match inspect(&address) {
        Ok(report) => report,
        Err(error) => json!({"error": error}),
    };
    report["url"] = json!(address);
    report["elapsed_ms"] = json!(started.elapsed().as_millis());
    report["version"] = json!(env!("CARGO_PKG_VERSION"));
    println!("{report}");
}
