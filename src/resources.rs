//! Explicit page resource collection. Parsing and the standalone engines stay inert.
use crate::{
    Document, ExternalSource, NodeId,
    css::applicable_style,
    js::document::{classic_type, eligible_ancestry},
    net::{DocumentLoader, Location, MAX_RESOURCE_BYTES, ResourceKind},
};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

pub const MAX_RESOURCES: usize = 64;
pub const MAX_PAGE_CSS_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_PAGE_SCRIPT_BYTES: usize = 32 * 1024 * 1024;
pub const RESOURCE_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Default)]
pub struct PageResources {
    pub styles: HashMap<NodeId, ExternalSource>,
    pub scripts: HashMap<NodeId, ExternalSource>,
    pub report: ResourceReport,
}

#[derive(Clone, Debug, Default)]
pub struct ResourceReport {
    pub attempted: usize,
    pub loaded: usize,
    pub diagnostics: Vec<String>,
    pub limited: bool,
}

impl PageResources {
    /// Snapshot supported links and scripts using the final document URL and first
    /// base href. No imports, dynamic resources, modules, or recursive loads.
    /// Disabled scripting never requests script sources.
    pub fn load(
        loader: &DocumentLoader,
        location: &Location,
        document: &Document,
        scripting: bool,
    ) -> Self {
        Self::load_with_timeout(loader, location, document, scripting, RESOURCE_TIMEOUT)
    }

    pub fn load_with_timeout(
        loader: &DocumentLoader,
        location: &Location,
        document: &Document,
        scripting: bool,
        timeout: Duration,
    ) -> Self {
        let mut resources = Self::default();
        let base = location.document_base(document);
        let started = Instant::now();
        let mut css_remaining = MAX_PAGE_CSS_BYTES;
        let mut js_remaining = MAX_PAGE_SCRIPT_BYTES;
        for id in document.descendants(document.root()) {
            let Some(element) = document.node(id).and_then(|n| n.as_element()) else {
                continue;
            };
            if !eligible_ancestry(document, id) {
                continue;
            }
            let (kind, reference, remaining) =
                if element.name.local.as_ref() == "link" && applicable_style(element) {
                    (
                        ResourceKind::Stylesheet,
                        element.attribute("href"),
                        &mut css_remaining,
                    )
                } else if scripting
                    && element.name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
                    && element.name.local.as_ref() == "script"
                    && classic_type(element.attribute("type"), element.attribute("language"))
                {
                    (
                        ResourceKind::Script,
                        element.attribute("src"),
                        &mut js_remaining,
                    )
                } else {
                    continue;
                };
            let Some(reference) = reference else { continue };
            if resources.report.attempted == MAX_RESOURCES || started.elapsed() >= timeout {
                resources.report.limited = true;
                resources
                    .report
                    .diagnostics
                    .push("Page resource count or time limit reached.".into());
                break;
            }
            resources.report.attempted += 1;
            let result = if *remaining == 0 {
                resources.report.limited = true;
                Err(format!(
                    "Page {:?} budget exhausted ({} bytes total).",
                    kind,
                    match kind {
                        ResourceKind::Stylesheet => MAX_PAGE_CSS_BYTES,
                        ResourceKind::Script => MAX_PAGE_SCRIPT_BYTES,
                    }
                ))
            } else if reference.trim().is_empty() {
                Err("Empty resource address.".into())
            } else if element
                .attribute("integrity")
                .is_some_and(|value| !value.trim().is_empty())
            {
                Err("Resources requiring integrity verification are not supported.".into())
            } else {
                base.resolve(reference).and_then(|target| {
                    loader.load_resource(
                        location,
                        target,
                        kind,
                        timeout.saturating_sub(started.elapsed()),
                        MAX_RESOURCE_BYTES,
                    ).and_then(|loaded| {
                        if loaded.source.len() > *remaining {
                            resources.report.limited = true;
                            Err(format!("Page {:?} budget exhausted: resource needs {} bytes; {} bytes remain.", kind, loaded.source.len(), *remaining))
                        } else { Ok(loaded) }
                    })
                })
            };
            match result {
                Ok(loaded) => {
                    *remaining -= loaded.source.len();
                    let source = ExternalSource {
                        reference: reference.to_owned(),
                        source: loaded.source,
                    };
                    match kind {
                        ResourceKind::Stylesheet => resources.styles.insert(id, source),
                        ResourceKind::Script => resources.scripts.insert(id, source),
                    };
                    resources.report.loaded += 1;
                }
                Err(error) => {
                    // Bound both retained URLs and transport error chains.
                    let message = format!("{reference}: {error}");
                    let mut end = message.len().min(1024);
                    while !message.is_char_boundary(end) {
                        end -= 1;
                    }
                    resources.report.diagnostics.push(message[..end].to_owned());
                }
            }
        }
        resources
    }
}
