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
pub const MAX_PAGE_IMAGE_BYTES: usize = 32 * 1024 * 1024;
pub const RESOURCE_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Default)]
pub struct PageResources {
    pub styles: HashMap<NodeId, ExternalSource>,
    pub scripts: HashMap<NodeId, ExternalSource>,
    pub images: HashMap<NodeId, ExternalImage>,
    pub report: ResourceReport,
}

/// A fetched image kept in its original encoded form until the GUI decodes it.
#[derive(Clone, Debug)]
pub struct ExternalImage {
    pub reference: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "gui", derive(serde::Serialize, serde::Deserialize))]
pub struct ResourceReport {
    pub attempted: usize,
    pub loaded: usize,
    pub diagnostics: Vec<String>,
    pub limited: bool,
}

impl PageResources {
    /// Snapshot supported links, scripts, and images using the final document URL and first
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
        let mut image_remaining = MAX_PAGE_IMAGE_BYTES;
        for id in document.descendants(document.root()) {
            let Some(element) = document.node(id).and_then(|n| n.as_element()) else {
                continue;
            };
            if !eligible_ancestry(document, id) {
                continue;
            }
            let (kind, reference) =
                if element.name.local.as_ref() == "link" && applicable_style(element) {
                    (ResourceKind::Stylesheet, element.attribute("href"))
                } else if scripting
                    && element.name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
                    && element.name.local.as_ref() == "script"
                    && classic_type(element.attribute("type"), element.attribute("language"))
                {
                    (ResourceKind::Script, element.attribute("src"))
                } else if element.name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
                    && element.name.local.as_ref() == "img"
                {
                    (ResourceKind::Image, element.attribute("src"))
                } else {
                    continue;
                };
            let Some(reference) = reference else { continue };
            let remaining = match kind {
                ResourceKind::Stylesheet => &mut css_remaining,
                ResourceKind::Script => &mut js_remaining,
                ResourceKind::Image => &mut image_remaining,
            };
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
                        ResourceKind::Image => MAX_PAGE_IMAGE_BYTES,
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
                        if loaded.bytes.len() > *remaining {
                            resources.report.limited = true;
                            Err(format!("Page {:?} budget exhausted: resource needs {} bytes; {} bytes remain.", kind, loaded.bytes.len(), *remaining))
                        } else { Ok(loaded) }
                    })
                })
            };
            match result {
                Ok(loaded) => {
                    *remaining -= loaded.bytes.len();
                    match kind {
                        ResourceKind::Stylesheet => {
                            resources.styles.insert(
                                id,
                                ExternalSource {
                                    reference: reference.to_owned(),
                                    source: loaded.source,
                                },
                            );
                        }
                        ResourceKind::Script => {
                            resources.scripts.insert(
                                id,
                                ExternalSource {
                                    reference: reference.to_owned(),
                                    source: loaded.source,
                                },
                            );
                        }
                        ResourceKind::Image => {
                            resources.images.insert(
                                id,
                                ExternalImage {
                                    reference: reference.to_owned(),
                                    bytes: loaded.bytes,
                                },
                            );
                        }
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
