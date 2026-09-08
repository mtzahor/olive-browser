//! Fetch and inspect page resources without executing downloaded JavaScript.
use olive_html::{
    css::Stylesheet,
    net::{DocumentLoader, Location},
    resources::PageResources,
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let address = std::env::args()
        .nth(1)
        .ok_or("Supply a URL or HTML file path")?;
    let loader = DocumentLoader::new()?;
    let loaded = loader.load(Location::from_input(&address)?)?;
    let document = loaded.parse(false)?.document;
    let resources = PageResources::load(&loader, &loaded.location, &document, true);
    println!(
        "{} of {} resources loaded (JavaScript is not executed)",
        resources.report.loaded, resources.report.attempted
    );
    for kind in [&resources.styles, &resources.scripts] {
        let mut entries: Vec<_> = kind.values().collect();
        entries.sort_by_key(|source| &source.reference);
        for source in entries {
            println!("{} bytes: {}", source.source.len(), source.reference);
        }
    }
    for diagnostic in &resources.report.diagnostics {
        eprintln!("{diagnostic}");
    }
    let css = Stylesheet::from_document_with_sources(&document, &resources.styles);
    println!(
        "{} CSS rules retained; {} unsupported items; limited={}",
        css.rule_count(),
        css.diagnostics.ignored,
        css.diagnostics.limited
    );
    Ok(())
}
