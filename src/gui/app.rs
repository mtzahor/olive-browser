use crate::{
    history::BrowsingHistory,
    history_ui::HistoryWindow,
    icon,
    navigation::{History, Navigation},
    render::{INK, ImageAsset, ImageTextureCache, OLIVE, Page},
    ui_icons::{self, Icon, IconButton},
};
use eframe::egui::{self, Color32, RichText};
use image::{ImageReader, Limits};
use olive_html::js::{DocumentSession, ScriptOptions, ScriptReport};
use olive_html::{
    ExternalSource, NodeId,
    css::Stylesheet,
    net::{DocumentLoader, LoadedDocument, Location},
    resources::{PageResources, ResourceReport},
};
use std::{
    collections::HashMap,
    ffi::OsString,
    io::Cursor,
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
};

const PAPER: Color32 = Color32::from_rgb(250, 250, 246);
const CHROME: Color32 = Color32::from_rgb(239, 242, 231);
const MAX_IMAGE_DIMENSION: u32 = 4_096;
const MAX_IMAGE_DECODE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_PAGE_IMAGE_PIXELS: u64 = 8 * 1024 * 1024;

struct LoadedPage {
    location: Location,
    base: Location,
    status: Option<u16>,
    page: Page,
    corrections: usize,
    scripts: ScriptReport,
    scripting_enabled: bool,
    resources: ResourceReport,
    session: Option<SessionHandle>,
}

struct ClickRequest {
    target: NodeId,
    href: Option<String>,
}
struct PageUpdate {
    page: Page,
    base: Location,
    scripts: ScriptReport,
    alert: Option<String>,
    link: Option<String>,
}
struct SessionHandle {
    sender: Sender<ClickRequest>,
    receiver: Receiver<PageUpdate>,
    busy: bool,
}
struct PreparedPage {
    loaded: LoadedPage,
    session: Option<DocumentSession>,
    styles: HashMap<NodeId, ExternalSource>,
    images: HashMap<NodeId, ImageAsset>,
}

struct PendingPage {
    receiver: Receiver<Result<LoadedPage, String>>,
    navigation: Navigation,
    requested: Location,
}

pub struct OliveApp {
    icon: egui::TextureHandle,
    loaded: Option<LoadedPage>,
    pending: Option<PendingPage>,
    address: String,
    history: History,
    browsing_history: BrowsingHistory,
    history_window: HistoryWindow,
    error: Option<String>,
    alert: Option<String>,
    image_textures: ImageTextureCache,
    // Each successful open gets a new scroll ID, including re-opening the same file.
    generation: u64,
}

impl OliveApp {
    pub fn new(cc: &eframe::CreationContext<'_>, source: Option<OsString>) -> Self {
        let ctx = &cc.egui_ctx;
        ctx.set_fonts(crate::fonts::definitions());
        let mut style = egui::Style {
            visuals: egui::Visuals::light(),
            ..Default::default()
        };
        style.visuals.override_text_color = Some(INK);
        style.visuals.panel_fill = PAPER;
        style.visuals.selection.bg_fill = Color32::from_rgb(209, 223, 182);
        style.spacing.button_padding = egui::vec2(16.0, 10.0);
        style.spacing.item_spacing = egui::vec2(12.0, 8.0);
        ctx.set_global_style(style);
        ctx.set_theme(egui::Theme::Light);
        let icon = ctx.load_texture(
            "olive-browser-icon",
            egui::ColorImage::from(icon::data()),
            egui::TextureOptions::LINEAR,
        );
        let mut app = Self {
            icon,
            loaded: None,
            pending: None,
            address: String::new(),
            history: History::default(),
            browsing_history: BrowsingHistory::load_default(),
            history_window: HistoryWindow::default(),
            error: None,
            alert: None,
            image_textures: ImageTextureCache::default(),
            generation: 0,
        };
        if let Some(source) = source {
            let location = match source.to_str() {
                Some(input) => Location::from_input(input),
                None => Location::from_path(PathBuf::from(source)),
            };
            app.open_result(location, Navigation::New, ctx);
        }
        app
    }

    fn choose_file(&mut self, ctx: &egui::Context) {
        if self.pending.is_some() {
            return;
        }
        let mut dialog = rfd::FileDialog::new().add_filter("HTML documents", &["html", "htm"]);
        if let Some(path) = self
            .loaded
            .as_ref()
            .and_then(|loaded| loaded.location.file_path())
        {
            if let Some(parent) = path.parent() {
                dialog = dialog.set_directory(parent);
            }
        }
        if let Some(path) = dialog.pick_file() {
            self.open_result(Location::from_path(path), Navigation::New, ctx);
        }
    }

    fn open_result(
        &mut self,
        location: Result<Location, String>,
        navigation: Navigation,
        ctx: &egui::Context,
    ) {
        match location {
            Ok(location) => self.open(location, navigation, ctx),
            Err(error) => self.error = Some(error),
        }
    }

    fn open(&mut self, location: Location, navigation: Navigation, ctx: &egui::Context) {
        let scripting = !location.is_remote()
            || (matches!(navigation, Navigation::Reload)
                && self.loaded.as_ref().is_some_and(|loaded| {
                    loaded.scripting_enabled && loaded.location.same_document(&location)
                }));
        self.open_with_scripts(location, navigation, ctx, scripting);
    }

    fn open_with_scripts(
        &mut self,
        location: Location,
        navigation: Navigation,
        ctx: &egui::Context,
        scripting: bool,
    ) {
        if self.pending.is_some() {
            return;
        }
        self.error = None;
        self.alert = None;
        self.address = location.as_str().to_owned();
        if !matches!(navigation, Navigation::Reload) {
            if let Some(loaded) = self.loaded.as_mut().filter(|loaded| {
                loaded.location.same_document(&location)
                    && (loaded.location != location
                        || matches!(navigation, Navigation::Traverse(_)))
            }) {
                loaded.page.scroll_to_fragment(location.fragment());
                // A base href can be independent of the document URL.
                if loaded.base.same_document(&loaded.location) {
                    loaded.base = location.clone();
                }
                loaded.location = location.clone();
                self.browsing_history.record(&location, &loaded.page.title);
                self.history.commit(location, navigation);
                return;
            }
        }
        let (sender, receiver) = mpsc::channel();
        let requested = location.clone();
        let ctx = ctx.clone();
        // The parser's DOM stays on this worker; only the owned presentation crosses threads.
        match std::thread::Builder::new()
            .name("olive-document-loader".into())
            .spawn(move || {
                let result = (|| {
                    let loader = DocumentLoader::new()?;
                    let source = loader.load(location.clone())?;
                    // Opt-in belongs to this address, never a redirected destination.
                    let scripting = !source.location.is_remote()
                        || (scripting && source.location.same_document(&location));
                    prepare_page_with_loader(source, &loader, scripting)
                })();
                match result {
                    Ok(prepared) => serve_page(prepared, sender, &ctx),
                    Err(error) => {
                        let _ = sender.send(Err(error));
                        ctx.request_repaint();
                    }
                }
            }) {
            Ok(_) => {
                self.pending = Some(PendingPage {
                    receiver,
                    navigation,
                    requested,
                })
            }
            Err(error) => {
                self.error = Some(format!("Could not start the document loader: {error}"))
            }
        }
    }

    fn receive(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.pending else {
            return;
        };
        match pending.receiver.try_recv() {
            Ok(Ok(mut loaded)) => {
                self.history
                    .commit(loaded.location.clone(), pending.navigation);
                self.browsing_history
                    .record(&loaded.location, &loaded.page.title);
                self.address = loaded.location.as_str().to_owned();
                loaded.page.scroll_to_fragment(loaded.location.fragment());
                let title = if loaded.page.title.trim().is_empty() {
                    loaded.location.as_str().to_owned()
                } else {
                    loaded.page.title.clone()
                };
                ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
                    "{title} — Olive Browser"
                )));
                self.loaded = Some(loaded);
                self.image_textures.clear();
                self.generation = self.generation.wrapping_add(1);
                self.pending = None;
            }
            Ok(Err(error)) => {
                self.error = Some(format!("{}\n{error}", pending.requested.as_str()));
                if let Some(loaded) = &self.loaded {
                    self.address = loaded.location.as_str().to_owned();
                }
                self.pending = None;
            }
            Err(TryRecvError::Disconnected) => {
                self.error = Some(
                    "The document loader stopped unexpectedly. You can try another address.".into(),
                );
                if let Some(loaded) = &self.loaded {
                    self.address = loaded.location.as_str().to_owned();
                }
                self.pending = None;
            }
            Err(TryRecvError::Empty) => {}
        }
    }
}

fn prepare_page_with_loader(
    source: LoadedDocument,
    loader: &DocumentLoader,
    scripting_enabled: bool,
) -> Result<PreparedPage, String> {
    let parsed = source.parse(scripting_enabled)?;
    let corrections = parsed
        .diagnostics
        .len()
        .saturating_add(parsed.omitted_diagnostics);
    let resources = PageResources::load(
        loader,
        &source.location,
        &parsed.document,
        scripting_enabled,
    );
    let mut resource_report = resources.report;
    let images = decode_images(&resources.images, &mut resource_report);
    let (page, base, scripts, session) = if scripting_enabled {
        let session = DocumentSession::with_sources(
            parsed.document,
            ScriptOptions::browser(),
            &resources.scripts,
        );
        let (page, base) = session.with_document(|document| {
            (
                Page::with_stylesheet_and_images(
                    document,
                    true,
                    Stylesheet::from_document_with_sources(document, &resources.styles),
                    &images,
                ),
                source.location.document_base(document),
            )
        });
        (page, base, session.report().clone(), Some(session))
    } else {
        let document = parsed.document;
        let page = Page::with_stylesheet_and_images(
            &document,
            false,
            Stylesheet::from_document_with_sources(&document, &resources.styles),
            &images,
        );
        (
            page,
            source.location.document_base(&document),
            ScriptReport::default(),
            None,
        )
    };
    Ok(PreparedPage {
        loaded: LoadedPage {
            location: source.location,
            base,
            status: source.status,
            page,
            corrections,
            scripts,
            scripting_enabled,
            resources: resource_report,
            session: None,
        },
        session,
        styles: resources.styles,
        images,
    })
}

fn decode_images(
    sources: &HashMap<NodeId, olive_html::resources::ExternalImage>,
    report: &mut ResourceReport,
) -> HashMap<NodeId, ImageAsset> {
    let mut decoded = HashMap::new();
    let mut pixels = 0_u64;
    for (&id, source) in sources {
        let result = decode_image(&source.bytes).and_then(|(image, image_pixels)| {
            if pixels.saturating_add(image_pixels) > MAX_PAGE_IMAGE_PIXELS {
                Err(format!(
                    "Page image pixel budget exhausted ({} pixels total).",
                    MAX_PAGE_IMAGE_PIXELS
                ))
            } else {
                pixels += image_pixels;
                Ok(image)
            }
        });
        match result {
            Ok(image) => {
                decoded.insert(
                    id,
                    ImageAsset {
                        reference: source.reference.clone(),
                        image,
                    },
                );
            }
            Err(error) => {
                let message = format!("{}: {error}", source.reference);
                let end = message
                    .char_indices()
                    .nth(1024)
                    .map_or(message.len(), |(index, _)| index);
                report.diagnostics.push(message[..end].to_owned());
            }
        }
    }
    decoded
}

fn decode_image(bytes: &[u8]) -> Result<(std::sync::Arc<egui::ColorImage>, u64), String> {
    if bytes.starts_with(&[0xff, 0xd8]) {
        return decode_jpeg(bytes);
    }
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| format!("could not identify image format: {error}"))?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_DIMENSION);
    limits.max_image_height = Some(MAX_IMAGE_DIMENSION);
    limits.max_alloc = Some(MAX_IMAGE_DECODE_BYTES);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|error| format!("could not decode image: {error}"))?;
    let width = image.width();
    let height = image.height();
    let pixels = checked_image_pixels(width, height)?;
    let rgba = image.to_rgba8();
    Ok((color_image(width, height, rgba.as_raw()), pixels))
}

fn decode_jpeg(bytes: &[u8]) -> Result<(std::sync::Arc<egui::ColorImage>, u64), String> {
    use zune_jpeg::{JpegDecoder, zune_core::colorspace::ColorSpace};

    let options = zune_jpeg::zune_core::options::DecoderOptions::default()
        .jpeg_set_out_colorspace(ColorSpace::RGBA)
        .set_use_unsafe(false);
    let mut decoder = JpegDecoder::new_with_options(bytes, options);
    decoder
        .decode_headers()
        .map_err(|error| format!("could not read image dimensions: {error}"))?;
    let info = decoder
        .info()
        .ok_or_else(|| "could not read image dimensions".to_owned())?;
    let width = u32::from(info.width);
    let height = u32::from(info.height);
    let pixels = checked_image_pixels(width, height)?;
    let rgba = decoder
        .decode()
        .map_err(|error| format!("could not decode image: {error}"))?;
    let expected = usize::try_from(pixels)
        .ok()
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "decoded image is too large".to_owned())?;
    if rgba.len() != expected {
        return Err("JPEG decoder returned an unexpected pixel buffer".into());
    }
    Ok((color_image(width, height, &rgba), pixels))
}

fn checked_image_pixels(width: u32, height: u32) -> Result<u64, String> {
    if width > MAX_IMAGE_DIMENSION || height > MAX_IMAGE_DIMENSION {
        return Err(format!(
            "image dimensions exceed the {MAX_IMAGE_DIMENSION}px limit"
        ));
    }
    let pixels = u64::from(width).saturating_mul(u64::from(height));
    if pixels > MAX_PAGE_IMAGE_PIXELS {
        return Err(format!(
            "image has too many pixels (maximum {MAX_PAGE_IMAGE_PIXELS})"
        ));
    }
    Ok(pixels)
}

fn color_image(width: u32, height: u32, rgba: &[u8]) -> std::sync::Arc<egui::ColorImage> {
    std::sync::Arc::new(egui::ColorImage::from_rgba_unmultiplied(
        [width as usize, height as usize],
        rgba,
    ))
}

// The DOM and Boa realm stay on their original worker. Only presentation data
// crosses threads, so scripts run once and click handlers retain their globals.
fn serve_page(
    mut prepared: PreparedPage,
    sender: Sender<Result<LoadedPage, String>>,
    ctx: &egui::Context,
) {
    let Some(mut session) = prepared.session else {
        let _ = sender.send(Ok(prepared.loaded));
        ctx.request_repaint();
        return;
    };
    // Page-load alerts have no dialog lifecycle in this preview.
    session.take_alerts();
    let location = prepared.loaded.location.clone();
    let (click_sender, click_receiver) = mpsc::channel::<ClickRequest>();
    let (update_sender, update_receiver) = mpsc::channel();
    prepared.loaded.session = Some(SessionHandle {
        sender: click_sender,
        receiver: update_receiver,
        busy: false,
    });
    if sender.send(Ok(prepared.loaded)).is_err() {
        return;
    }
    ctx.request_repaint();
    for click in click_receiver {
        let allowed = session.click(click.target);
        let (page, base) = session.with_document(|document| {
            (
                Page::with_stylesheet_and_images(
                    document,
                    true,
                    Stylesheet::from_document_with_sources(document, &prepared.styles),
                    &prepared.images,
                ),
                location.document_base(document),
            )
        });
        let update = PageUpdate {
            page,
            base,
            scripts: session.report().clone(),
            alert: session.take_alerts().into_iter().next(),
            link: if allowed { click.href } else { None },
        };
        if update_sender.send(update).is_err() {
            break;
        }
        ctx.request_repaint();
    }
}

#[cfg(test)]
fn prepare_page(source: LoadedDocument) -> Result<LoadedPage, String> {
    let scripting = !source.location.is_remote();
    Ok(prepare_page_with_loader(source, &DocumentLoader::new()?, scripting)?.loaded)
}

fn open_button(ui: &mut egui::Ui, enabled: bool) -> bool {
    let mut button = IconButton::new(Icon::Folder, "Open HTML…").primary();
    button.button = button.button.min_size(egui::vec2(152.0, 40.0));
    ui.add_enabled(enabled, button).clicked()
}

impl eframe::App for OliveApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        PAPER.to_normalized_gamma_f32()
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.receive(ctx);
        let mut link = None;
        if let Some(loaded) = &mut self.loaded {
            if let Some(session) = &mut loaded.session {
                match session.receiver.try_recv() {
                    Ok(update) => {
                        session.busy = false;
                        loaded.page = update.page;
                        loaded.base = update.base;
                        loaded.scripts = update.scripts;
                        self.browsing_history
                            .update_title(&loaded.location, &loaded.page.title);
                        self.alert = update.alert;
                        if self.pending.is_none() {
                            link = update.link.map(|href| loaded.base.resolve(&href));
                            ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
                                "{} — Olive Browser",
                                loaded.page.title
                            )));
                        }
                    }
                    Err(TryRecvError::Disconnected) => {
                        loaded.session = None;
                        self.error =
                            Some("The JavaScript worker stopped. Reload to try again.".into());
                    }
                    Err(TryRecvError::Empty) => {}
                }
            }
        }
        if let Some(location) = link {
            self.open_result(location, Navigation::New, ctx);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Panels share the root UI; leave no unpainted gap between them.
        ui.spacing_mut().item_spacing.y = 0.0;
        let ctx = ui.ctx().clone();
        if ctx.input_mut(|input| {
            input.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND,
                egui::Key::O,
            ))
        }) {
            self.choose_file(&ctx);
        }
        if let Some(path) = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .first()
                .map(|file| file.path().to_path_buf())
        }) {
            self.open_result(Location::from_path(path), Navigation::New, &ctx);
        }
        let mut choose = false;
        let shortcut = |modifiers, key| {
            ctx.input_mut(|input| {
                input.consume_shortcut(&egui::KeyboardShortcut::new(modifiers, key))
            })
        };
        let focus_address = shortcut(egui::Modifiers::COMMAND, egui::Key::L);
        let mut show_history = shortcut(
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            egui::Key::H,
        );
        let mut back = shortcut(egui::Modifiers::ALT, egui::Key::ArrowLeft);
        let mut forward = shortcut(egui::Modifiers::ALT, egui::Key::ArrowRight);
        let mut reload = shortcut(egui::Modifiers::COMMAND, egui::Key::R)
            || shortcut(egui::Modifiers::NONE, egui::Key::F5);
        let mut go = false;
        let mut toggle_scripts = false;
        let mut link = None;
        let busy = self.pending.is_some();
        egui::Panel::top("toolbar")
            .exact_size(64.0)
            .frame(
                egui::Frame::new()
                    .fill(CHROME)
                    .inner_margin(egui::Margin::symmetric(12, 12)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.spacing_mut().button_padding = egui::vec2(9.0, 10.0);
                    let compact = ui.available_width() < 640.0;
                    ui.add(egui::Image::new(&self.icon).fit_to_exact_size(egui::vec2(30.0, 30.0)))
                        .on_hover_text("Olive Browser");
                    back |= ui
                        .add_enabled(
                            !busy && self.history.back().is_some(),
                            IconButton::icon_only(Icon::Back, "Back"),
                        )
                        .on_hover_text("Back (Alt+Left)")
                        .clicked();
                    forward |= ui
                        .add_enabled(
                            !busy && self.history.forward().is_some(),
                            IconButton::icon_only(Icon::Forward, "Forward"),
                        )
                        .on_hover_text("Forward (Alt+Right)")
                        .clicked();
                    reload |= ui
                        .add_enabled(
                            !busy && self.loaded.is_some(),
                            IconButton::icon_only(Icon::Reload, "Reload"),
                        )
                        .on_hover_text("Reload (Cmd/Ctrl+R or F5)")
                        .clicked();
                    // Reserve actual control widths, including icon gaps and text,
                    // so the address field never pushes actions out of the window.
                    let label_width = |label: &str| {
                        ui.painter()
                            .layout_no_wrap(
                                label.into(),
                                egui::TextStyle::Button.resolve(ui.style()),
                                INK,
                            )
                            .size()
                            .x
                    };
                    let trailing_width = 3.0 * 36.0
                        + 3.0 * 6.0
                        + if compact {
                            0.0
                        } else {
                            label_width("Open…") + label_width("History") + 2.0 * 7.0
                        };
                    let address_width = (ui.available_width() - trailing_width).max(60.0);
                    let mut output = egui::TextEdit::singleline(&mut self.address)
                        .id(egui::Id::new("address"))
                        .hint_text("Enter a URL or file path")
                        .char_limit(olive_html::net::MAX_URL_BYTES)
                        .desired_width(address_width)
                        .margin(egui::vec2(10.0, 10.0))
                        .show(ui);
                    if focus_address {
                        output.response.request_focus();
                        output
                            .state
                            .cursor
                            .set_char_range(Some(egui::text::CCursorRange::two(
                                egui::text::CCursor::new(0),
                                egui::text::CCursor::new(self.address.chars().count()),
                            )));
                        output.state.store(ui.ctx(), output.response.id);
                    }
                    go |= output.response.lost_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter));
                    go |= ui
                        .add_enabled(!busy, IconButton::icon_only(Icon::Go, "Go").primary())
                        .on_hover_text("Open address")
                        .clicked();
                    choose |= ui
                        .add_enabled(
                            !busy,
                            if compact {
                                IconButton::icon_only(Icon::Folder, "Open HTML file")
                            } else {
                                IconButton::new(Icon::Folder, "Open…")
                            },
                        )
                        .on_hover_text("Open HTML file (Cmd/Ctrl+O)")
                        .clicked();
                    show_history |= ui
                        .add(
                            (if compact {
                                IconButton::icon_only(Icon::History, "Browsing history")
                            } else {
                                IconButton::new(Icon::History, "History")
                            })
                            .warning(self.browsing_history.error().is_some()),
                        )
                        .on_hover_text(if self.browsing_history.error().is_some() {
                            "Browsing history — could not save history (Cmd/Ctrl+Shift+H)"
                        } else {
                            "Browsing history (Cmd/Ctrl+Shift+H)"
                        })
                        .clicked();
                });
            });
        egui::Panel::bottom("status")
            .frame(
                egui::Frame::new()
                    .fill(CHROME)
                    .inner_margin(egui::Margin::symmetric(20, 9)),
            )
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    let label = if busy { "Opening…".to_owned() } else {
                        self.loaded.as_ref().map(|loaded| {
                            let transport = match loaded.location.url().scheme() {
                                "https" => "HTTPS",
                                "http" => "HTTP · Not encrypted",
                                _ => "Local file",
                            };
                            match loaded.status {
                                Some(status) if status >= 400 => format!("{transport} · HTTP {status}"),
                                _ => transport.to_owned(),
                            }
                        }).unwrap_or_else(|| "Ready".into())
                    };
                    ui.add(egui::Label::new(RichText::new(label).size(12.0)).truncate());
                    if let Some(loaded) = &self.loaded {
                        if loaded.location.is_remote() {
                            toggle_scripts = ui.add_enabled(!busy, IconButton::new(
                                if loaded.scripting_enabled { Icon::CodeOff } else { Icon::Code },
                                if loaded.scripting_enabled { "Disable JavaScript" } else { "Enable JavaScript" }
                            ).small()).on_hover_text("Reload this page with JavaScript enabled or disabled. Enable only for pages you trust: scripts run inside Olive's process. New addresses start with web JavaScript disabled.").clicked();
                        }
                        if loaded.resources.attempted > 0 || loaded.resources.limited {
                            ui_icons::menu(ui, if loaded.resources.diagnostics.is_empty() { Icon::Resources } else { Icon::Warning }, if loaded.resources.diagnostics.is_empty() { "Resources" } else { "Resource errors" }, |ui| {
                                ui.label(format!("{} of {} resources loaded", loaded.resources.loaded, loaded.resources.attempted));
                                egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                                    for message in &loaded.resources.diagnostics { ui.label(message); }
                                });
                            });
                        }
                        if loaded.scripts.attempted > 0
                            || loaded.scripts.skipped > 0
                            || loaded.scripts.limited
                        {
                            let label = if loaded.scripts.limited {
                                "JavaScript limit reached"
                            } else if !loaded.scripts.diagnostics.is_empty() {
                                "JavaScript errors"
                            } else {
                                "JavaScript"
                            };
                            ui_icons::menu(ui, if loaded.scripts.limited || !loaded.scripts.diagnostics.is_empty() { Icon::Warning } else { Icon::Code }, label, |ui| {
                                ui.label(format!(
                                    "{} scripts completed; {} skipped",
                                    loaded.scripts.executed, loaded.scripts.skipped
                                ));
                                egui::ScrollArea::vertical()
                                    .max_height(240.0)
                                    .show(ui, |ui| {
                                        for error in &loaded.scripts.diagnostics {
                                            if let Some(source) = &error.source { ui.strong(source); }
                                            ui.label(&error.message);
                                        }
                                        for message in &loaded.scripts.console {
                                            ui.monospace(message);
                                        }
                                        let omitted =
                                            loaded.scripts.omitted_diagnostics.saturating_add(
                                                loaded.scripts.omitted_console_messages,
                                            );
                                        if omitted > 0 {
                                            ui.label(format!("{omitted} messages omitted"));
                                        }
                                    });
                            });
                        }
                        if loaded.page.css_ignored > 0 {
                            ui.label(RichText::new("Some CSS is unsupported").size(11.0).weak())
                                .on_hover_text(format!(
                                    "{} CSS rules or declarations were ignored.",
                                    loaded.page.css_ignored
                                ));
                        }
                        if loaded.corrections > 0 {
                            ui.label(
                                RichText::new("Opened with HTML corrections")
                                    .size(11.0)
                                    .weak(),
                            );
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(concat!("v", env!("CARGO_PKG_VERSION")))
                                .size(12.0)
                                .weak(),
                        );
                    });
                });
            });
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new().fill(
                    self.loaded
                        .as_ref()
                        .and_then(|loaded| loaded.page.background)
                        .unwrap_or(PAPER),
                ),
            )
            .show(ui, |ui| {
                if let Some(error) = &self.error {
                    egui::Frame::new()
                        .fill(Color32::from_rgb(255, 235, 228))
                        .inner_margin(16)
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new("Could not open this page")
                                    .variations([("wght", 650.0)]),
                            );
                            ui.label(error);
                        });
                }
                if let Some(loaded) = &mut self.loaded {
                    egui::ScrollArea::both()
                        .id_salt(("document", self.generation))
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            let margin = ((ui.available_width() - 820.0) / 2.0).max(24.0);
                            egui::Frame::new()
                                .inner_margin(egui::Margin {
                                    left: margin.min(100.0) as i8,
                                    right: margin.min(100.0) as i8,
                                    top: 34,
                                    bottom: 40,
                                })
                                .show(ui, |ui| {
                                    if loaded.page.is_empty() {
                                        ui.label("This document has no visible content.");
                                    }
                                    if let Some(click) =
                                        loaded.page.show_with_textures(ui, &mut self.image_textures)
                                    {
                                        link = click.href;
                                        if let (Some(target), Some(session)) =
                                            (click.target, loaded.session.as_mut())
                                        {
                                            if !session.busy
                                                && session
                                                    .sender
                                                    .send(ClickRequest {
                                                        target,
                                                        href: link.take(),
                                                    })
                                                    .is_ok()
                                            {
                                                session.busy = true;
                                            }
                                            link = None;
                                        }
                                    }
                                });
                        });
                } else {
                    ui.vertical_centered(|ui| {
                        ui.add_space(((ui.available_height() - 240.0) * 0.4).max(24.0));
                        ui.label(
                            RichText::new("A fresh page.")
                                .size(38.0)
                                .color(OLIVE)
                                .variations([("wght", 600.0)]),
                        );
                        ui.add_space(10.0);
                        ui.label(RichText::new("A small browser for the open web.").size(18.0));
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new("Enter a website above, or open a local HTML file.")
                                .size(14.0)
                                .weak(),
                        );
                        ui.add_space(22.0);
                        choose |= open_button(ui, self.pending.is_none());
                        ui.add_space(10.0);
                        ui.label(
                            RichText::new(if cfg!(target_os = "macos") {
                                "⌘L to enter an address · ⌘O to open a file"
                            } else {
                                "Ctrl+L to enter an address · Ctrl+O to open a file"
                            })
                            .size(12.0)
                            .weak(),
                        );
                    });
                }
            });
        if let Some(message) = &self.alert {
            let mut close = false;
            egui::Window::new("JavaScript alert")
                .collapsible(false)
                .resizable(false)
                .show(ui.ctx(), |ui| {
                    ui.label(message);
                    if ui
                        .add(IconButton::new(Icon::Check, "OK").primary())
                        .clicked()
                    {
                        close = true;
                    }
                });
            if close {
                self.alert = None;
            }
        }
        if show_history {
            self.history_window.toggle();
        }
        let history_location = self
            .history_window
            .show(&ctx, &mut self.browsing_history, busy);
        if choose {
            self.choose_file(&ctx);
        } else if !busy {
            if let Some(location) = history_location {
                self.open(location, Navigation::New, &ctx);
            } else if go {
                let location = Location::from_input(&self.address);
                let navigation = if location.as_ref().ok().is_some_and(|location| {
                    self.loaded
                        .as_ref()
                        .is_some_and(|loaded| &loaded.location == location)
                }) {
                    Navigation::Reload
                } else {
                    Navigation::New
                };
                self.open_result(location, navigation, &ctx);
            } else if back {
                if let Some((index, location)) = self.history.back() {
                    self.open(location, Navigation::Traverse(index), &ctx);
                }
            } else if forward {
                if let Some((index, location)) = self.history.forward() {
                    self.open(location, Navigation::Traverse(index), &ctx);
                }
            } else if toggle_scripts {
                if let Some(loaded) = &self.loaded {
                    self.open_with_scripts(
                        loaded.location.clone(),
                        Navigation::Reload,
                        &ctx,
                        !loaded.scripting_enabled,
                    );
                }
            } else if reload {
                if let Some(loaded) = &self.loaded {
                    self.open(loaded.location.clone(), Navigation::Reload, &ctx);
                }
            } else if let (Some(href), Some(loaded)) = (link, &self.loaded) {
                let location = loaded.base.resolve(&href);
                self.open_result(location, Navigation::New, &ctx);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_file(path: PathBuf) -> Result<LoadedPage, String> {
        DocumentLoader::new()?
            .load(Location::from_path(path)?)
            .and_then(prepare_page)
    }

    fn app_for_test(ctx: &egui::Context, loaded: LoadedPage) -> OliveApp {
        let mut history = History::default();
        history.commit(loaded.location.clone(), Navigation::New);
        let mut browsing_history = BrowsingHistory::default();
        browsing_history.record(&loaded.location, &loaded.page.title);
        OliveApp {
            icon: ctx.load_texture(
                "test-icon",
                egui::ColorImage::from(icon::data()),
                Default::default(),
            ),
            address: loaded.location.as_str().into(),
            history,
            browsing_history,
            history_window: HistoryWindow::default(),
            loaded: Some(loaded),
            pending: None,
            error: None,
            alert: None,
            image_textures: ImageTextureCache::default(),
            generation: 1,
        }
    }

    fn remote_source(url: &str) -> LoadedDocument {
        LoadedDocument {
            location: Location::from_input(url).unwrap(),
            bytes: b"<!doctype html><p>Remote</p>".to_vec(),
            status: Some(200),
            plain_text: false,
        }
    }

    #[test]
    fn decodes_png_images_with_bounded_dimensions() {
        let (image, pixels) =
            decode_image(include_bytes!("../../assets/olive-browser.png")).unwrap();
        assert_eq!(image.size, [1024, 1024]);
        assert_eq!(pixels, 1024 * 1024);
        assert!(decode_image(b"not an image").is_err());
    }

    #[test]
    fn failed_navigation_keeps_the_page_address_and_forward_history() {
        let ctx = egui::Context::default();
        let original = Location::from_input("https://example.com/first").unwrap();
        let mut app = app_for_test(
            &ctx,
            prepare_page(remote_source(original.as_str())).unwrap(),
        );
        let next = Location::from_input("https://example.com/second").unwrap();
        app.history.commit(next.clone(), Navigation::New);
        app.history
            .commit(original.clone(), Navigation::Traverse(0));
        let requested = Location::from_input("https://missing.example/").unwrap();
        app.address = requested.as_str().into();
        let (sender, receiver) = mpsc::channel();
        sender.send(Err("Test connection failure".into())).unwrap();
        app.pending = Some(PendingPage {
            receiver,
            requested,
            navigation: Navigation::New,
        });
        app.receive(&ctx);
        assert_eq!(app.loaded.as_ref().unwrap().location, original);
        assert_eq!(app.address, original.as_str());
        assert_eq!(app.history.forward().unwrap().1, next);
        assert!(app.pending.is_none());
        assert!(app.error.as_ref().unwrap().contains("missing.example"));
        assert_eq!(app.browsing_history.entries().len(), 1);
        assert_eq!(app.browsing_history.entries()[0].url, original.as_str());
    }

    #[test]
    fn saved_history_records_final_urls_and_successful_reload_and_traversal() {
        let ctx = egui::Context::default();
        let mut app = app_for_test(
            &ctx,
            prepare_page(remote_source("https://example.com/first")).unwrap(),
        );
        let requested = Location::from_input("https://example.com/redirect").unwrap();
        let final_url = "https://example.com/final";
        let mut source = remote_source(final_url);
        // Readable HTTP error pages are successful navigations too.
        source.status = Some(404);
        source.bytes = b"<!doctype html><title>Not found</title><p>404</p>".to_vec();
        let (sender, receiver) = mpsc::channel();
        sender.send(Ok(prepare_page(source).unwrap())).unwrap();
        app.pending = Some(PendingPage {
            receiver,
            requested: requested.clone(),
            navigation: Navigation::New,
        });
        app.receive(&ctx);
        assert_eq!(app.browsing_history.entries().len(), 2);
        let entry = &app.browsing_history.entries()[0];
        assert_eq!(entry.url, final_url);
        assert_eq!(entry.title, "Not found");
        assert!(app.browsing_history.search(requested.as_str()).is_empty());

        for (url, navigation) in [
            (final_url, Navigation::Reload),
            ("https://example.com/first", Navigation::Traverse(0)),
        ] {
            let (sender, receiver) = mpsc::channel();
            sender
                .send(Ok(prepare_page(remote_source(url)).unwrap()))
                .unwrap();
            app.pending = Some(PendingPage {
                receiver,
                requested: Location::from_input(url).unwrap(),
                navigation,
            });
            app.receive(&ctx);
            assert_eq!(app.browsing_history.entries().len(), 2);
            assert_eq!(app.browsing_history.entries()[0].url, url);
            assert_eq!(app.browsing_history.entries()[0].visits, 2);
        }
        assert_eq!(app.history.forward().unwrap().1.as_str(), final_url);
        app.browsing_history.clear();
        assert!(app.browsing_history.entries().is_empty());
        assert_eq!(app.history.forward().unwrap().1.as_str(), final_url);
        assert_eq!(
            app.loaded.as_ref().unwrap().location.as_str(),
            "https://example.com/first"
        );
    }

    #[test]
    fn same_document_navigation_records_visits_without_a_loader() {
        let ctx = egui::Context::default();
        let url = "https://example.com/page";
        let mut app = app_for_test(&ctx, prepare_page(remote_source(url)).unwrap());
        let anchor = Location::from_input(&format!("{url}#section")).unwrap();
        app.open(anchor.clone(), Navigation::New, &ctx);
        assert!(app.pending.is_none());
        assert_eq!(app.browsing_history.entries()[0].url, anchor.as_str());
        let (index, location) = app.history.back().unwrap();
        app.open(location, Navigation::Traverse(index), &ctx);
        assert!(app.pending.is_none());
        assert_eq!(app.browsing_history.entries().len(), 2);
        assert_eq!(app.browsing_history.entries()[0].url, url);
        assert_eq!(app.browsing_history.entries()[0].visits, 2);
        app.browsing_history.remove(anchor.as_str());
        assert_eq!(app.history.forward().unwrap().1, anchor);
    }

    #[test]
    fn disconnected_load_does_not_record_a_visit() {
        let ctx = egui::Context::default();
        let mut app = app_for_test(
            &ctx,
            prepare_page(remote_source("https://example.com/first")).unwrap(),
        );
        let (sender, receiver) = mpsc::channel();
        drop(sender);
        app.pending = Some(PendingPage {
            receiver,
            requested: Location::from_input("https://example.com/disconnected").unwrap(),
            navigation: Navigation::New,
        });
        app.receive(&ctx);
        assert!(app.pending.is_none());
        assert!(app.error.is_some());
        assert_eq!(app.browsing_history.entries().len(), 1);
        assert!(app.browsing_history.search("disconnected").is_empty());
    }

    #[test]
    fn remote_pages_default_to_disabled_scripts() {
        let source = LoadedDocument {
            location: Location::from_input("https://example.com/page").unwrap(),
            bytes: b"<!doctype html><title>Original</title><base href='/docs/'><noscript>Readable fallback</noscript><script>document.title='Executed';</script><a href='next'>Next</a>".to_vec(),
            status: Some(200), plain_text: false,
        };
        let loaded = prepare_page(source).unwrap();
        assert_eq!(loaded.page.title, "Original");
        assert_eq!(loaded.scripts.attempted, 0);
        assert!(loaded.session.is_none());
        assert_eq!(loaded.base.as_str(), "https://example.com/docs/");
        assert!(!loaded.page.is_empty());
    }

    #[test]
    fn loader_accepts_an_html_file_and_reports_read_errors() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let loaded = load_file(root.join("examples/hello.html")).unwrap();
        assert_eq!(loaded.page.title, "Olive Browser");
        assert!(!loaded.page.is_empty());
        let image_page = load_file(root.join("examples/reading.html")).unwrap();
        assert_eq!(image_page.page.image_count(), 1);
        assert!(load_file(root.clone()).is_err());
        assert!(load_file(root.join("missing-olive-example.html")).is_err());
    }

    #[test]
    fn loader_executes_demo_scripts_before_building_presentation() {
        let loaded =
            load_file(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/scripted.html"))
                .unwrap();
        assert_eq!(loaded.page.title, "Olive — 39 olives harvested");
        assert_eq!(loaded.scripts.executed, 2);
        assert!(
            loaded.scripts.diagnostics.is_empty(),
            "{:?}",
            loaded.scripts.diagnostics
        );
        assert!(!loaded.scripts.limited);
        assert_eq!(loaded.scripts.console[0], "Harvest calculated: 39");
    }
    #[test]
    fn enabled_web_session_executes_once_and_retains_globals_for_worker_clicks() {
        let ctx = egui::Context::default();
        let source = LoadedDocument {
            location: Location::from_input("https://example.com/demo").unwrap(),
            bytes: br#"<!doctype html><title>Before</title><script>
                let count=1; document.title='Loaded '+count;
                function increment(){count++; document.title='Clicked '+count; alert(count)}
                </script><button id=increment onclick='increment(); return false'>Increment</button>"#.to_vec(),
            status: Some(200), plain_text: false,
        };
        let (sender, receiver) = mpsc::channel();
        let (target_sender, target_receiver) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let prepared =
                prepare_page_with_loader(source, &DocumentLoader::new().unwrap(), true).unwrap();
            let target = prepared.session.as_ref().unwrap().with_document(|doc| {
                doc.descendants(doc.root())
                    .find(|&id| {
                        doc.node(id)
                            .unwrap()
                            .as_element()
                            .is_some_and(|e| e.attribute("id") == Some("increment"))
                    })
                    .unwrap()
            });
            target_sender.send(target).unwrap();
            serve_page(prepared, sender, &ctx);
        });
        let mut loaded = receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
            .unwrap();
        assert!(loaded.scripting_enabled);
        assert_eq!(loaded.page.title, "Loaded 1");
        let target = target_receiver.recv().unwrap();
        let session = loaded.session.take().unwrap();
        for count in [2, 3] {
            session
                .sender
                .send(ClickRequest {
                    target,
                    href: Some("/must-not-navigate".into()),
                })
                .unwrap();
            let update = session
                .receiver
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            assert_eq!(update.page.title, format!("Clicked {count}"));
            assert_eq!(update.scripts.executed, 1);
            assert!(update.scripts.diagnostics.is_empty());
            assert_eq!(update.alert, Some(count.to_string()));
            assert!(update.link.is_none());
        }
        drop(session);
        worker.join().unwrap();
    }
}
