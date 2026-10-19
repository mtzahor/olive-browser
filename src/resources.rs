//! Explicit page resource collection. Parsing and the standalone engines stay inert.
use crate::{
    Document, ExternalSource, NodeId,
    css::applicable_style,
    js::document::classic_type,
    net::{DocumentLoader, Location, MAX_RESOURCE_BYTES, ResourceKind},
};
use std::{
    collections::HashMap,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

pub const MAX_RESOURCES: usize = 256;
pub const MAX_PAGE_CSS_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_PAGE_SCRIPT_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_PAGE_IMAGE_BYTES: usize = 32 * 1024 * 1024;
pub const RESOURCE_TIMEOUT: Duration = Duration::from_secs(30);
// Page resources are opportunistic: a stalled image or stylesheet must not
// hold the whole document for several seconds before it can be displayed.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
const RESOURCE_WORKERS: usize = 8;

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
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ResourceReport {
    pub attempted: usize,
    pub loaded: usize,
    pub diagnostics: Vec<String>,
    pub limited: bool,
    #[cfg_attr(feature = "serde", serde(default))]
    pub pending: bool,
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
        // Fetch visual resources before optional scripts. Bound each queue while
        // scanning so a large page cannot allocate an unbounded request list.
        let mut queues = [Vec::new(), Vec::new(), Vec::new()];
        let mut eligible = vec![false; document.node_count()];
        eligible[document.root().index()] = true;
        for id in document.descendants(document.root()) {
            let Some(node) = document.node(id) else {
                continue;
            };
            if !node.parent().is_some_and(|parent| eligible[parent.index()]) {
                continue;
            }
            let Some(element) = node.as_element() else {
                continue;
            };
            eligible[id.index()] = !element.is_inert_container();
            let (kind, reference) =
                if element.name.local.as_str() == "link" && applicable_style(element) {
                    (ResourceKind::Stylesheet, element.attribute("href"))
                } else if scripting
                    && element.is_html_tag("script")
                    && classic_type(element.attribute("type"), element.attribute("language"))
                {
                    (ResourceKind::Script, element.attribute("src"))
                } else if element.is_html_tag("img") {
                    (ResourceKind::Image, element.attribute("src"))
                } else {
                    continue;
                };
            let Some(reference) = reference else { continue };
            let priority = match kind {
                ResourceKind::Stylesheet => 0,
                ResourceKind::Image => 1,
                ResourceKind::Script => 2,
            };
            if queues[priority].len() <= MAX_RESOURCES {
                queues[priority].push((id, kind, reference));
            }
        }
        let mut candidates = Vec::new();
        for queue in queues {
            let remaining_slots = MAX_RESOURCES.saturating_sub(candidates.len());
            let queue_len = queue.len();
            candidates.extend(queue.into_iter().take(remaining_slots));
            if queue_len > remaining_slots {
                resources.report.limited = true;
                resources
                    .report
                    .diagnostics
                    .push("Page resource count or time limit reached.".into());
            }
        }
        let outcomes = load_batch(
            loader,
            location,
            document,
            &base,
            &candidates,
            started,
            timeout,
        );
        for ((id, kind, reference), result) in candidates.iter().copied().zip(outcomes) {
            let element = document.node(id).unwrap().as_element().unwrap();
            let remaining = match kind {
                ResourceKind::Stylesheet => &mut css_remaining,
                ResourceKind::Script => &mut js_remaining,
                ResourceKind::Image => &mut image_remaining,
            };
            if resources.report.attempted == MAX_RESOURCES {
                resources.report.limited = true;
                resources
                    .report
                    .diagnostics
                    .push("Page resource count or time limit reached.".into());
                break;
            }
            if started.elapsed() >= timeout {
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
                result.and_then(|loaded| {
                    if loaded.bytes.len() > *remaining {
                        resources.report.limited = true;
                        Err(format!(
                            "Page {:?} budget exhausted: resource needs {} bytes; {} bytes remain.",
                            kind,
                            loaded.bytes.len(),
                            *remaining
                        ))
                    } else {
                        Ok(loaded)
                    }
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
                    resources
                        .report
                        .diagnostics
                        .push(crate::text::truncate(&message, 1024).to_owned());
                }
            }
        }
        resources
    }
}

/// Fetch page resources with one bounded pool. Fetching resource classes
/// sequentially creates avoidable delays; CSS and scripts are still applied in
/// document order after their sources have been collected.
fn load_batch(
    loader: &DocumentLoader,
    location: &Location,
    document: &Document,
    base: &Location,
    candidates: &[(NodeId, ResourceKind, &str)],
    started: Instant,
    timeout: Duration,
) -> Vec<Result<crate::net::LoadedResource, String>> {
    if started.elapsed() >= timeout {
        return candidates
            .iter()
            .map(|_| Err("Resource loading deadline reached.".into()))
            .collect();
    }
    let targets: Vec<_> = candidates
        .iter()
        .map(|(_, _, reference)| {
            if reference.trim().is_empty() {
                Err("Empty resource address.".into())
            } else {
                base.resolve(reference)
            }
        })
        .collect();
    let requires_integrity: Vec<_> = candidates
        .iter()
        .map(|(id, _, _)| {
            document
                .node(*id)
                .and_then(|node| node.as_element())
                .is_some_and(|element| {
                    element
                        .attribute("integrity")
                        .is_some_and(|value| !value.trim().is_empty())
                })
        })
        .collect();
    let results = Mutex::new(
        std::iter::repeat_with(|| None)
            .take(candidates.len())
            .collect::<Vec<Option<Result<crate::net::LoadedResource, String>>>>(),
    );
    let next = AtomicUsize::new(0);
    let worker_count = RESOURCE_WORKERS.min(candidates.len());
    thread::scope(|scope| {
        for _ in 0..worker_count {
            let targets = &targets;
            let requires_integrity = &requires_integrity;
            let results = &results;
            let next = &next;
            scope.spawn(move || {
                loop {
                    if started.elapsed() >= timeout {
                        break;
                    }
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some((_, kind, _)) = candidates.get(index) else {
                        break;
                    };
                    if requires_integrity[index] {
                        if let Ok(mut slots) = results.lock() {
                            slots[index] = Some(Err(
                                "Resources requiring integrity verification are not supported."
                                    .into(),
                            ));
                        }
                        continue;
                    }
                    let result = targets[index].as_ref().map_or_else(
                        |error| Err(error.clone()),
                        |target| {
                            loader.load_resource(
                                location,
                                target.clone(),
                                *kind,
                                timeout
                                    .saturating_sub(started.elapsed())
                                    .min(REQUEST_TIMEOUT),
                                MAX_RESOURCE_BYTES,
                            )
                        },
                    );
                    if let Ok(mut slots) = results.lock() {
                        slots[index] = Some(result);
                    }
                }
            });
        }
    });
    results
        .into_inner()
        .unwrap_or_default()
        .into_iter()
        .map(|result| result.unwrap_or_else(|| Err("Resource loading worker stopped.".into())))
        .collect()
}
