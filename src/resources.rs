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
        Condvar, Mutex,
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
/// The largest site icon file `PageResources::load_favicon` accepts.
pub const MAX_FAVICON_BYTES: usize = 1024 * 1024;
// Page resources are opportunistic: a stalled image or stylesheet must not
// hold the whole document for several seconds before it can be displayed.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
// Most page resources come from a few hosts; this many requests in flight
// roughly halves image-heavy page loads compared with eight.
const RESOURCE_WORKERS: usize = 16;

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
    /// Where to look for the page's icon, in order: the best raster
    /// `<link rel="icon">` (or Apple touch icon), then `/favicon.ico` for
    /// websites. SVG icons are skipped.
    pub fn favicon_addresses(location: &Location, document: &Document) -> Vec<Location> {
        let base = location.document_base(document);
        let declared = favicon_link(document).and_then(|href| base.resolve(href).ok());
        let fallback = location
            .is_remote()
            .then(|| location.resolve("/favicon.ico").ok())
            .flatten();
        let mut addresses: Vec<Location> = declared.into_iter().collect();
        if let Some(fallback) = fallback.filter(|fallback| !addresses.contains(fallback)) {
            addresses.push(fallback);
        }
        addresses
    }

    /// Fetch the first of `addresses` that answers with an image. Failures are
    /// silent and not part of the page's resource report.
    pub fn load_favicon(
        loader: &DocumentLoader,
        location: &Location,
        addresses: &[Location],
        timeout: Duration,
    ) -> Option<ExternalImage> {
        let started = Instant::now();
        for target in addresses {
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                break;
            }
            if let Ok(loaded) = loader.load_resource(
                location,
                target.clone(),
                ResourceKind::Image,
                remaining.min(REQUEST_TIMEOUT),
                MAX_FAVICON_BYTES,
            ) {
                return Some(ExternalImage {
                    reference: loaded.location.as_str().to_owned(),
                    bytes: loaded.bytes,
                });
            }
        }
        None
    }

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
        Self::load_with_progress(loader, location, document, scripting, timeout, |_| {})
    }

    /// `load_with_timeout` that hands `styles_loaded` the page's stylesheets as
    /// soon as they are in, while images and scripts are still loading, so a
    /// styled page can show before its pictures. It runs at most once, on the
    /// calling thread, and only when a stylesheet loaded before the rest.
    pub fn load_with_progress(
        loader: &DocumentLoader,
        location: &Location,
        document: &Document,
        scripting: bool,
        timeout: Duration,
        styles_loaded: impl FnOnce(&HashMap<NodeId, ExternalSource>),
    ) -> Self {
        let mut intake = Intake {
            document,
            started: Instant::now(),
            timeout,
            resources: Self::default(),
            css_remaining: MAX_PAGE_CSS_BYTES,
            js_remaining: MAX_PAGE_SCRIPT_BYTES,
            image_remaining: MAX_PAGE_IMAGE_BYTES,
            stopped: false,
        };
        let base = location.document_base(document);
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
            // Without scripting, `noscript` holds the page's real fallback
            // content, such as stylesheets that scripts would otherwise insert.
            // Olive never shows `object` data, so its fallback content shows instead.
            eligible[id.index()] = !element.is_inert_container()
                || (!scripting && element.is_html_tag("noscript"))
                || element.is_html_tag("object");
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
                intake.limit_reached();
            }
        }
        // Stylesheets lead the queue; the page can be styled once they are in.
        let styles = candidates
            .iter()
            .take_while(|(_, kind, _)| *kind == ResourceKind::Stylesheet)
            .count();
        let outcomes = load_batch(
            loader,
            location,
            document,
            &base,
            &candidates,
            intake.started,
            timeout,
            styles,
            |outcomes, still_loading| {
                for (&candidate, result) in candidates.iter().zip(outcomes) {
                    intake.accept(candidate, result);
                }
                if still_loading && !intake.resources.styles.is_empty() {
                    styles_loaded(&intake.resources.styles);
                }
            },
        );
        for (&candidate, result) in candidates[styles..].iter().zip(outcomes) {
            intake.accept(candidate, result);
        }
        intake.resources
    }
}

/// Fetched resources accepted in document order against the page's budgets.
struct Intake<'a> {
    document: &'a Document,
    started: Instant,
    timeout: Duration,
    resources: PageResources,
    css_remaining: usize,
    js_remaining: usize,
    image_remaining: usize,
    /// The resource count or time limit stopped intake.
    stopped: bool,
}

impl Intake<'_> {
    fn limit_reached(&mut self) {
        self.resources.report.limited = true;
        self.resources
            .report
            .diagnostics
            .push("Page resource count or time limit reached.".into());
    }

    fn accept(
        &mut self,
        (id, kind, reference): (NodeId, ResourceKind, &str),
        result: Result<crate::net::LoadedResource, String>,
    ) {
        if self.stopped {
            return;
        }
        if self.resources.report.attempted == MAX_RESOURCES
            || self.started.elapsed() >= self.timeout
        {
            self.limit_reached();
            self.stopped = true;
            return;
        }
        let element = self.document.node(id).unwrap().as_element().unwrap();
        let resources = &mut self.resources;
        let remaining = match kind {
            ResourceKind::Stylesheet => &mut self.css_remaining,
            ResourceKind::Script => &mut self.js_remaining,
            ResourceKind::Image => &mut self.image_remaining,
        };
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
}

/// The `href` of the icon link that suits a tab best: a raster icon nearest
/// 32 pixels, or an Apple touch icon when the page declares nothing else.
fn favicon_link(document: &Document) -> Option<&str> {
    let mut best: Option<(i32, &str)> = None;
    for id in document.descendants(document.root()) {
        let Some(element) = document.node(id).and_then(|node| node.as_element()) else {
            continue;
        };
        if !element.is_html_tag("link") {
            continue;
        }
        let (Some(rel), Some(href)) = (element.attribute("rel"), element.attribute("href")) else {
            continue;
        };
        let href = href.trim();
        let rel = rel.to_ascii_lowercase();
        let icon = rel.split_ascii_whitespace().any(|token| token == "icon");
        let touch = rel
            .split_ascii_whitespace()
            .any(|token| token.starts_with("apple-touch-icon"));
        let path = href.split(['?', '#']).next().unwrap_or_default();
        let svg = element
            .attribute("type")
            .is_some_and(|kind| kind.to_ascii_lowercase().contains("svg"))
            || path.to_ascii_lowercase().ends_with(".svg");
        if href.is_empty() || svg || !(icon || touch) {
            continue;
        }
        // How far the declared size nearest 32 pixels is from it, as in
        // sizes="16x16 32x32".
        let distance = element
            .attribute("sizes")
            .into_iter()
            .flat_map(str::split_ascii_whitespace)
            .filter_map(|size| {
                let (width, _) = size.split_once(['x', 'X'])?;
                width.parse::<i32>().ok()
            })
            .map(|size| (size - 32).abs().min(90))
            .min();
        let score = match (icon, distance) {
            (true, Some(distance)) => 100 - distance,
            (true, None) => 60,
            (false, _) => 5,
        };
        if best.is_none_or(|(previous, _)| score > previous) {
            best = Some((score, href));
        }
    }
    best.map(|(_, href)| href)
}

/// Fetch page resources with one bounded pool. Fetching resource classes
/// sequentially creates avoidable delays; CSS and scripts are still applied in
/// document order after their sources have been collected. Repeated addresses
/// are fetched once. `first_loaded` receives the outcomes of the first `first`
/// candidates on the calling thread as soon as they are in, and whether others
/// are still loading; the outcomes of the rest are returned.
// These borrowed inputs are one page's shared loading context.
#[allow(clippy::too_many_arguments)]
fn load_batch(
    loader: &DocumentLoader,
    location: &Location,
    document: &Document,
    base: &Location,
    candidates: &[(NodeId, ResourceKind, &str)],
    started: Instant,
    timeout: Duration,
    first: usize,
    first_loaded: impl FnOnce(Vec<LoadOutcome>, bool),
) -> Vec<LoadOutcome> {
    if started.elapsed() >= timeout {
        let mut outcomes: Vec<LoadOutcome> = candidates
            .iter()
            .map(|_| Err("Resource loading deadline reached.".into()))
            .collect();
        let rest = outcomes.split_off(first);
        first_loaded(outcomes, false);
        return rest;
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
    // Each candidate takes its outcome from the first candidate fetching the
    // same address as the same kind of resource.
    let mut first_fetch = HashMap::new();
    let source: Vec<usize> = candidates
        .iter()
        .enumerate()
        .map(|(index, (_, kind, _))| match &targets[index] {
            Ok(target) if !requires_integrity[index] => {
                *first_fetch.entry((*kind, target.as_str())).or_insert(index)
            }
            _ => index,
        })
        .collect();
    let fetches: Vec<usize> = (0..candidates.len())
        .filter(|&index| source[index] == index && !requires_integrity[index])
        .filter(|&index| targets[index].is_ok())
        .collect();
    let slots: Vec<Option<LoadOutcome>> = (0..candidates.len())
        .map(|index| match &targets[index] {
            _ if requires_integrity[index] => Some(Err(
                "Resources requiring integrity verification are not supported.".into(),
            )),
            Err(error) => Some(Err(error.clone())),
            Ok(_) => None,
        })
        .collect();
    let finished = |slots: &[Option<LoadOutcome>], range: std::ops::Range<usize>| {
        range
            .into_iter()
            .all(|index| slots[source[index]].is_some())
    };
    // Duplicates copy the outcome of their source, which comes earlier, so
    // outcomes are taken from the end.
    let take = |slots: &mut [Option<LoadOutcome>], range: std::ops::Range<usize>| {
        let mut outcomes: Vec<_> = range
            .rev()
            .map(|index| {
                if source[index] == index {
                    slots[index].take()
                } else {
                    slots[source[index]].clone()
                }
                .unwrap_or_else(|| Err("Resource loading worker stopped.".into()))
            })
            .collect();
        outcomes.reverse();
        outcomes
    };
    let first_done = finished(&slots, 0..first);
    let slots = Mutex::new(slots);
    let filled = Condvar::new();
    let next = AtomicUsize::new(0);
    let worker_count = RESOURCE_WORKERS.min(fetches.len());
    let mut first_loaded = Some(first_loaded);
    thread::scope(|scope| {
        for _ in 0..worker_count {
            let (fetches, targets, slots, filled, next) =
                (&fetches, &targets, &slots, &filled, &next);
            scope.spawn(move || {
                loop {
                    if started.elapsed() >= timeout {
                        break;
                    }
                    let Some(&index) = fetches.get(next.fetch_add(1, Ordering::Relaxed)) else {
                        break;
                    };
                    let (_, kind, _) = candidates[index];
                    let result = targets[index].as_ref().map_or_else(
                        |error| Err(error.clone()),
                        |target| {
                            loader.load_resource(
                                location,
                                target.clone(),
                                kind,
                                timeout
                                    .saturating_sub(started.elapsed())
                                    .min(REQUEST_TIMEOUT),
                                MAX_RESOURCE_BYTES,
                            )
                        },
                    );
                    if let Ok(mut slots) = slots.lock() {
                        slots[index] = Some(result);
                    }
                    filled.notify_all();
                }
                // Wake the caller when the pool stops at the deadline.
                filled.notify_all();
            });
        }
        if first == 0 || first_done || first == candidates.len() {
            return;
        }
        let Ok(mut guard) = slots.lock() else { return };
        while !finished(&guard, 0..first) {
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                break;
            }
            match filled.wait_timeout(guard, remaining) {
                Ok((next_guard, _)) => guard = next_guard,
                Err(_) => return,
            }
        }
        let still_loading = !finished(&guard, first..candidates.len());
        let outcomes = take(&mut guard, 0..first);
        drop(guard);
        if let Some(first_loaded) = first_loaded.take() {
            first_loaded(outcomes, still_loading);
        }
    });
    let mut slots = slots.into_inner().unwrap_or_default();
    slots.resize_with(candidates.len(), || None);
    let rest = take(&mut slots, first..candidates.len());
    if let Some(first_loaded) = first_loaded {
        first_loaded(take(&mut slots, 0..first), false);
    }
    rest
}

type LoadOutcome = Result<crate::net::LoadedResource, String>;
