use crate::gui::{
    daily_ui::DailyWindows,
    devtools::DevTools,
    document::{ClickRequest, LoadedPage},
    downloads::Downloads,
    favicons::Favicons,
    find::FindBar,
    focus::{MAX_FONT_SIZE, MIN_FONT_SIZE, Settings as FocusSettings},
    forms::{Activation, Kind},
    glass::{self, Band, Pane},
    history::BrowsingHistory,
    history_ui::HistoryWindow,
    icon,
    navigation::{History, Navigation},
    new_tab,
    profile::{MAX_TABS, MAX_ZOOM, MIN_ZOOM, Profile, SavedTab, Session},
    render::{ImageTextureCache, Page},
    theme::{DOCUMENT_PAPER, Palette, Theme},
    ui_icons::{self, Icon, IconButton},
    worker::{Command, Event, Request, ScriptPolicy, Target, Worker},
    zoom,
};
use crate::net::{CookieJar, Location};
use eframe::egui::{self, RichText};
use std::{ffi::OsString, path::PathBuf};

struct PendingPage {
    worker: Worker,
    navigation: Navigation,
    requested: Location,
    /// The CSS viewport the page is laid out for.
    viewport: Option<(u32, u32)>,
}

struct Shared {
    icon: egui::TextureHandle,
    browsing_history: BrowsingHistory,
    history_window: HistoryWindow,
    cookies: CookieJar,
    profile: Profile,
    downloads: Downloads,
    daily_windows: DailyWindows,
    favicons: Favicons,
    /// The size of the page area in the latest frame that showed a page.
    page_area: std::cell::Cell<Option<egui::Vec2>>,
    /// The part of the window that the frame's bands leave uncovered, in the
    /// latest frame.
    page_view: std::cell::Cell<Option<egui::Rect>>,
}

impl Shared {
    /// The CSS viewport a page loaded at `zoom` percent will have, when known.
    fn viewport(&self, zoom: u16) -> Option<(u32, u32)> {
        self.page_area.get().map(|area| css_viewport(area, zoom))
    }

    /// The frame's top and bottom bands across `window`, around the page.
    fn bands(&self, window: egui::Rect) -> [egui::Rect; 2] {
        self.page_view
            .get()
            .map_or([egui::Rect::NOTHING; 2], |view| {
                [
                    egui::Rect::from_min_max(window.min, egui::pos2(window.max.x, view.top())),
                    egui::Rect::from_min_max(egui::pos2(window.min.x, view.bottom()), window.max),
                ]
            })
    }
}

/// The CSS viewport, in whole CSS pixels, of a page area `area` points large
/// at `zoom` percent.
fn css_viewport(area: egui::Vec2, zoom: u16) -> (u32, u32) {
    let scale = (zoom as f32 / 100.0).max(0.25);
    let side = |points: f32| (points / scale).round().clamp(1.0, 10_000.0) as u32;
    (side(area.x), side(area.y))
}

pub struct Tab {
    id: u64,
    worker: Option<Worker>,
    crashed: bool,
    loaded: Option<LoadedPage>,
    pending: Option<PendingPage>,
    opener: Option<Location>,
    deferred: Option<Location>,
    deferred_scripts: bool,
    last_requested: Option<Location>,
    address: String,
    history: History,
    error: Option<String>,
    alert: Option<String>,
    image_textures: ImageTextureCache,
    // Each successful open gets a new scroll ID, including re-opening the same file.
    generation: u64,
    find: FindBar,
    pending_form: Option<Activation>,
    focus_mode: bool,
    focus_panel: bool,
    focus_settings: FocusSettings,
    zoom: u16,
    css_viewport: (u32, u32),
    /// The current page's site icon.
    favicon: Option<egui::TextureHandle>,
}

impl Tab {
    fn toggle_focus(&mut self, shared: &mut Shared) {
        if self.focus_mode {
            self.focus_mode = false;
        } else if self.pending.is_none()
            && self
                .loaded
                .as_ref()
                .is_some_and(|loaded| !loaded.reading.is_empty())
        {
            self.focus_mode = true;
            self.focus_panel = true;
            shared.history_window = HistoryWindow::default();
            self.alert = None;
        }
    }

    fn focus_toolbar(&mut self, ui: &mut egui::Ui, busy: bool) {
        let previous_style = ui.style().clone();
        // Theme just the reading surface; the browser returns to its normal appearance on exit.
        ui.style_mut().visuals = self.focus_settings.theme.reading_visuals();
        // Focus keeps the glass frame in the reading theme's colors.
        let palette = self.focus_settings.theme.palette();
        egui::Panel::top("focus_toolbar")
            .exact_size(60.0)
            .show_separator_line(false)
            .frame(egui::Frame::new().inner_margin(egui::Margin {
                left: 12,
                right: 12,
                top: 4,
                bottom: 8,
            }))
            .show(ui, |ui| {
                glass::in_band(ui, Band::Top, |ui| {
                    ui.spacing_mut().button_padding = egui::vec2(12.0, 9.0);
                    glass::capsule(ui, egui::Margin::symmetric(6, 2), CAPSULE, &palette, |ui| {
                        ui.horizontal(|ui| {
                            if ui
                                .add(IconButton::new(Icon::Back, "Exit Focus"))
                                .on_hover_text("Return to the page (Esc)")
                                .clicked()
                            {
                                self.focus_mode = false;
                            }
                            if busy {
                                ui.spinner();
                                ui.label("Opening…");
                            } else {
                                ui.label(RichText::new("Focus mode").strong());
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let html = self
                                        .loaded
                                        .as_ref()
                                        .and_then(|loaded| loaded.reading.export_html.as_deref());
                                    let reason = self
                                        .loaded
                                        .as_ref()
                                        .and_then(|loaded| loaded.reading.export_error.as_deref())
                                        .unwrap_or("Export the article for offline reading");
                                    if ui
                                        .add_enabled(
                                            !busy && html.is_some(),
                                            IconButton::icon_only(
                                                Icon::Download,
                                                "Export Focus view as HTML",
                                            ),
                                        )
                                        .on_hover_text(reason)
                                        .clicked()
                                    {
                                        if let Some(path) = rfd::FileDialog::new()
                                            .add_filter("HTML documents", &["html"])
                                            .set_file_name("olive-reading.html")
                                            .save_file()
                                        {
                                            let saved = self
                                                .loaded
                                                .as_ref()
                                                .and_then(|loaded| loaded.reading.export_document())
                                                .unwrap_or_else(|| Err("Nothing to export".into()))
                                                .and_then(|html| {
                                                    crate::gui::focus_export::save(&path, &html)
                                                        .map_err(|error| error.to_string())
                                                });
                                            match saved {
                                                Ok(()) => {
                                                    self.alert = Some(format!(
                                                        "Focus view exported to {}",
                                                        path.display()
                                                    ))
                                                }
                                                Err(error) => {
                                                    self.error = Some(format!(
                                                        "Could not export Focus view: {error}"
                                                    ))
                                                }
                                            }
                                        }
                                    }
                                    let appearance =
                                        IconButton::new(Icon::Appearance, "Appearance")
                                            .selected(self.focus_panel);
                                    if ui
                                        .add(appearance)
                                        .on_hover_text("Show or hide reading settings")
                                        .clicked()
                                    {
                                        self.focus_panel = !self.focus_panel;
                                    }
                                },
                            );
                        });
                    });
                });
            });
        if self.focus_panel {
            egui::Panel::top("focus_appearance")
                .show_separator_line(false)
                .frame(egui::Frame::new().inner_margin(egui::Margin {
                    left: 12,
                    right: 12,
                    top: 0,
                    bottom: 8,
                }))
                .show(ui, |ui| {
                    glass::in_band(ui, Band::Top, |ui| {
                        glass::capsule(ui, egui::Margin::symmetric(14, 8), 20, &palette, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing.y = 8.0;
                                ui.spacing_mut().button_padding = egui::vec2(12.0, 8.0);
                                ui.add(
                                    egui::Slider::new(
                                        &mut self.focus_settings.font_size,
                                        MIN_FONT_SIZE..=MAX_FONT_SIZE,
                                    )
                                    .integer()
                                    .suffix(" px")
                                    .text("Font size")
                                    .show_value(true),
                                );
                                ui.selectable_value(
                                    &mut self.focus_settings.theme,
                                    Theme::Light,
                                    "Light",
                                );
                                ui.selectable_value(
                                    &mut self.focus_settings.theme,
                                    Theme::Dark,
                                    "Dark",
                                );
                            });
                        });
                    });
                });
        }
        ui.set_style(previous_style);
    }

    fn new(id: u64) -> Self {
        Self {
            id,
            worker: None,
            crashed: false,
            loaded: None,
            pending: None,
            opener: None,
            deferred: None,
            deferred_scripts: true,
            last_requested: None,
            address: String::new(),
            history: History::default(),
            error: None,
            alert: None,
            image_textures: ImageTextureCache::default(),
            generation: 0,
            find: FindBar::default(),
            pending_form: None,
            focus_mode: false,
            focus_panel: true,
            focus_settings: FocusSettings::default(),
            zoom: 100,
            css_viewport: (0, 0),
            favicon: None,
        }
    }

    fn activate_form(&mut self, activation: Activation, ctx: &egui::Context, shared: &Shared) {
        if self.pending.is_some() || self.crashed {
            return;
        }
        let Some(loaded) = &mut self.loaded else {
            return;
        };
        let Some(control) = loaded
            .page
            .forms
            .controls
            .iter()
            .find(|c| c.node == activation.control)
        else {
            return;
        };
        if control.disabled || control.owner.is_none() {
            return;
        }
        match control.kind {
            Kind::Reset => {
                loaded.page.forms.reset(activation.control);
                return;
            }
            Kind::Button => return,
            Kind::Submit | Kind::Text | Kind::Password => {}
            _ => return,
        }
        match loaded
            .page
            .forms
            .submit(activation.control, &loaded.location, &loaded.base)
        {
            Ok(request) => {
                let requested = request.location.clone();
                let viewport = shared.viewport(self.zoom);
                match Worker::spawn(
                    Request {
                        target: Target::Submit(request),
                        scripting: ScriptPolicy::for_load(
                            shared.profile.data.settings.javascript_by_default,
                            true,
                        ),
                        cookies: shared.cookies.clone(),
                        initiator: Some(loaded.location.clone()),
                        viewport: viewport.map(|(width, height)| (width as f32, height as f32)),
                    },
                    ctx,
                ) {
                    Ok(worker) => {
                        self.address = requested.as_str().into();
                        self.error = None;
                        self.alert = None;
                        // Forms always navigate, even if the action is the current address.
                        self.pending = Some(PendingPage {
                            worker,
                            navigation: Navigation::New,
                            requested,
                            viewport,
                        });
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn choose_file(&mut self, ctx: &egui::Context, shared: &mut Shared) {
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
            self.open_result(Location::from_path(path), Navigation::New, ctx, shared);
        }
    }

    fn open_result(
        &mut self,
        location: Result<Location, String>,
        navigation: Navigation,
        ctx: &egui::Context,
        shared: &mut Shared,
    ) {
        match location {
            Ok(location) => self.open(location, navigation, ctx, shared),
            Err(error) => self.error = Some(error),
        }
    }

    fn open(
        &mut self,
        location: Location,
        navigation: Navigation,
        ctx: &egui::Context,
        shared: &mut Shared,
    ) {
        let scripting = self.scripting_for_load(&location, navigation, shared);
        self.open_with_scripts(location, navigation, ctx, scripting, shared);
    }

    fn scripting_for_load(
        &self,
        location: &Location,
        navigation: Navigation,
        shared: &Shared,
    ) -> bool {
        if !location.is_remote() || shared.profile.data.settings.javascript_by_default {
            return true;
        }
        if matches!(navigation, Navigation::Reload) {
            if let Some(loaded) = self
                .loaded
                .as_ref()
                .filter(|page| page.location.same_document(location))
            {
                return loaded.scripting_enabled;
            }
        }
        false
    }

    fn open_with_scripts(
        &mut self,
        location: Location,
        navigation: Navigation,
        ctx: &egui::Context,
        scripting: bool,
        shared: &mut Shared,
    ) {
        self.deferred = None;
        self.last_requested = Some(location.clone());
        self.pending_form = None;
        self.pending = None; // Cancels and reaps a replaced navigation.
        self.error = None;
        self.alert = None;
        self.address = location.as_str().to_owned();
        if !self.crashed && !matches!(navigation, Navigation::Reload) {
            if let Some(loaded) = self.loaded.as_mut().filter(|loaded| {
                loaded.location.same_document(&location)
                    && (loaded.location != location
                        || matches!(navigation, Navigation::Traverse(_)))
            }) {
                loaded.page.scroll_to_fragment(location.fragment());
                loaded.reading.scroll_to_fragment(location.fragment());
                // A base href can be independent of the document URL.
                if loaded.base.same_document(&loaded.location) {
                    loaded.base = location.clone();
                }
                loaded.location = location.clone();
                shared
                    .browsing_history
                    .record(&location, &loaded.page.title);
                self.history.commit(location, navigation);
                return;
            }
        }
        let requested = location.clone();
        let initiator = self
            .loaded
            .as_ref()
            .map(|loaded| loaded.location.clone())
            .or_else(|| self.opener.take());
        let viewport = shared.viewport(self.zoom);
        match Worker::spawn(
            Request {
                target: Target::Load(location),
                scripting: ScriptPolicy::for_load(
                    scripting,
                    shared.profile.data.settings.javascript_by_default,
                ),
                cookies: shared.cookies.clone(),
                initiator,
                viewport: viewport.map(|(width, height)| (width as f32, height as f32)),
            },
            ctx,
        ) {
            Ok(worker) => {
                self.pending = Some(PendingPage {
                    worker,
                    navigation,
                    requested,
                    viewport,
                })
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn receive(&mut self, ctx: &egui::Context, shared: &mut Shared) {
        if let Some(event) = self
            .pending
            .as_mut()
            .and_then(|pending| pending.worker.poll())
        {
            let pending = self.pending.take().unwrap();
            match event {
                Ok(Event::Loaded(mut loaded)) => {
                    let keep_worker = loaded.scripting_enabled || loaded.resources.pending;
                    let resources_pending = loaded.resources.pending;
                    shared.cookies.apply_updates(&loaded.cookie_updates);
                    self.history
                        .commit(loaded.location.clone(), pending.navigation);
                    shared
                        .browsing_history
                        .record(&loaded.location, &loaded.page.title);
                    self.address = loaded.location.as_str().to_owned();
                    loaded.page.scroll_to_fragment(loaded.location.fragment());
                    loaded
                        .reading
                        .scroll_to_fragment(loaded.location.fragment());
                    self.loaded = Some(*loaded);
                    self.worker = if keep_worker {
                        let mut worker = pending.worker;
                        if resources_pending {
                            worker.await_resource_update();
                        }
                        Some(worker)
                    } else {
                        None
                    };
                    self.crashed = false;
                    self.error = None;
                    self.focus_mode = false;
                    self.pending_form = None;
                    self.find.current = 0;
                    self.find.scroll = self.find.open;
                    self.image_textures.clear();
                    self.generation = self.generation.wrapping_add(1);
                    self.css_viewport = pending.viewport.unwrap_or((0, 0));
                    self.favicon = None;
                }
                other => {
                    let error = match other {
                        Ok(Event::Error(error)) | Err(error) => error,
                        _ => "The tab process sent an unexpected reply.".into(),
                    };
                    self.error = Some(format!("{}\n{error}", pending.requested.as_str()));
                    if let Some(loaded) = &self.loaded {
                        self.address = loaded.location.as_str().into();
                    }
                }
            }
        }
        let event = self.worker.as_mut().and_then(Worker::poll);
        match event {
            Some(Ok(Event::ResourcesUpdated(mut update))) => {
                shared.cookies.apply_updates(&update.cookie_updates);
                // A styled page arrives first while images are still loading.
                let keep_worker = update.resources.pending || update.presentation_available;
                if let (Some(icon), Some(loaded)) = (&update.favicon, &self.loaded) {
                    shared.favicons.store(ctx, loaded.location.as_str(), icon);
                    self.favicon = Some(ctx.load_texture(
                        format!("tab-favicon-{}", self.id),
                        icon.0.clone(),
                        egui::TextureOptions::LINEAR,
                    ));
                }
                if let Some(loaded) = &mut self.loaded {
                    loaded.apply_update(&mut update);
                    loaded.resources = update.resources;
                    shared
                        .browsing_history
                        .update_title(&loaded.location, &loaded.page.title);
                }
                self.find.scroll = self.find.open;
                self.image_textures.clear();
                self.pending_form = None;
                if !keep_worker {
                    self.worker = None;
                }
            }
            Some(Ok(Event::Updated(mut update))) => {
                shared.cookies.apply_updates(&update.cookie_updates);
                let activation = self.pending_form.take().filter(|_| update.default_allowed);
                if let Some(loaded) = &mut self.loaded {
                    loaded.apply_update(&mut update);
                    self.find.scroll = self.find.open;
                    shared
                        .browsing_history
                        .update_title(&loaded.location, &loaded.page.title);
                    self.alert = update.alert;
                    if self.pending.is_none() {
                        if let Some(href) = update.link {
                            let location = loaded.base.resolve(&href);
                            self.open_result(location, Navigation::New, ctx, shared);
                        }
                    }
                }
                if let Some(activation) = activation {
                    self.activate_form(activation, ctx, shared);
                }
            }
            Some(event) => {
                self.pending_form = None;
                self.worker = None;
                self.crashed = true;
                self.alert = None;
                self.error = Some(match event {
                    Err(error) | Ok(Event::Error(error)) => error,
                    _ => "The tab process sent an unexpected reply. Reload this tab.".into(),
                });
            }
            None => {}
        }
    }

    fn stop(&mut self) {
        self.pending_form = None;
        if self.pending.take().is_some() {
            if let Some(loaded) = &self.loaded {
                self.address = loaded.location.as_str().into();
            }
        } else if self
            .loaded
            .as_ref()
            .is_some_and(|loaded| loaded.resources.pending)
        {
            self.worker = None;
            if let Some(loaded) = &mut self.loaded {
                loaded.resources.pending = false;
                loaded
                    .resources
                    .diagnostics
                    .push("Resource loading stopped.".into());
            }
        } else if self.worker.as_ref().is_some_and(Worker::busy) {
            self.worker = None;
            self.crashed = true;
            self.error = Some("This tab was stopped. Reload to restart it.".into());
        }
    }

    fn saved_tab(&self) -> SavedTab {
        let location = self
            .pending
            .as_ref()
            .map(|pending| &pending.requested)
            .or_else(|| self.loaded.as_ref().map(|page| &page.location))
            .or(self.deferred.as_ref())
            .or(self.last_requested.as_ref());
        SavedTab {
            url: location.map(|location| location.as_str().to_owned()),
            zoom: self.zoom,
            focus: self.focus_settings,
        }
    }

    /// Whether a navigation or a worker command is still in flight.
    fn working(&self) -> bool {
        self.pending.is_some() || self.worker.as_ref().is_some_and(Worker::busy)
    }

    fn restored_scripting(&self, shared: &Shared) -> bool {
        self.deferred_scripts && shared.profile.data.settings.javascript_by_default
    }

    fn site(&self) -> Site {
        match self
            .loaded
            .as_ref()
            .map(|loaded| loaded.location.url().scheme())
        {
            None => Site::Blank,
            Some("https") => Site::Secure,
            Some("http") => Site::Insecure,
            Some(_) => Site::Local,
        }
    }

    /// The address whose site icon the tab shows, including a restored tab
    /// that has not loaded yet.
    fn page_address(&self) -> Option<&str> {
        self.loaded
            .as_ref()
            .map(|loaded| loaded.location.as_str())
            .or_else(|| self.deferred.as_ref().map(Location::as_str))
    }

    fn title(&self) -> &str {
        self.loaded
            .as_ref()
            .map(|page| {
                if page.page.title.trim().is_empty() {
                    page.location.as_str()
                } else {
                    &page.page.title
                }
            })
            .unwrap_or(if self.address.is_empty() {
                "New tab"
            } else {
                &self.address
            })
    }
}

/// The width of an icon-only toolbar button.
const ICON_BUTTON: f32 = 36.0;
/// Space between a toolbar capsule's edge and its controls, and between capsules.
const GROUP_PAD: i8 = 4;
const GROUP_GAP: f32 = 8.0;
/// The corner radius of toolbar capsules: half their height.
const CAPSULE: u8 = 22;
const SITE_ICON: &str = "site-icon";

/// How the current page was reached, for the address field and status bar.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Site {
    Blank,
    Secure,
    Insecure,
    Local,
}

impl Site {
    fn icon(self) -> Icon {
        match self {
            Self::Blank => Icon::Globe,
            Self::Secure => Icon::Lock,
            Self::Insecure => Icon::Warning,
            Self::Local => Icon::Page,
        }
    }

    fn color(self, palette: Palette) -> egui::Color32 {
        match self {
            Self::Secure => palette.accent,
            Self::Insecure => palette.warning,
            Self::Blank | Self::Local => palette.muted,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Blank => "Ready",
            Self::Secure => "HTTPS",
            Self::Insecure => "HTTP · Not encrypted",
            Self::Local => "Local file",
        }
    }

    fn description(self) -> Option<&'static str> {
        match self {
            Self::Blank => None,
            Self::Secure => Some("Connection is encrypted (HTTPS)"),
            Self::Insecure => {
                Some("Connection is not encrypted (HTTP). Avoid entering private information.")
            }
            Self::Local => Some("A file on this computer"),
        }
    }
}

/// Requests gathered from shortcuts and controls while a tab's panels draw.
/// `Tab::apply` acts on them once every panel has had a chance to set them.
#[derive(Default)]
struct TabActions {
    open_file: bool,
    go: bool,
    back: bool,
    forward: bool,
    reload: bool,
    stop: bool,
    toggle_scripts: bool,
    toggle_bookmark: bool,
    show_history: bool,
    show_bookmarks: bool,
    show_settings: bool,
    show_downloads: bool,
    download_current: bool,
    zoom_in: bool,
    zoom_out: bool,
    zoom_reset: bool,
    link: Option<String>,
    link_new_tab: bool,
    /// An address chosen on the new-tab page.
    open_url: Option<String>,
    form: Option<Activation>,
}

const SCRIPT_TOGGLE_HINT: &str = "Reload this page with JavaScript enabled or disabled. \
    Use only for pages you trust: workers can still read local files and use the network. \
    New addresses use the JavaScript default in Settings.";

fn shortcut(ctx: &egui::Context, modifiers: egui::Modifiers, key: egui::Key) -> bool {
    ctx.input_mut(|input| input.consume_shortcut(&egui::KeyboardShortcut::new(modifiers, key)))
}

impl Tab {
    fn ui(
        &mut self,
        ui: &mut egui::Ui,
        shared: &mut Shared,
        autofocus: bool,
        devtools_open: &mut bool,
    ) -> Option<Location> {
        let palette = shared.profile.data.settings.browser_theme.palette();
        // Panels share the root UI; leave no unpainted gap between them.
        ui.spacing_mut().item_spacing.y = 0.0;
        let ctx = ui.ctx().clone();
        if shortcut(&ctx, egui::Modifiers::COMMAND, egui::Key::O) {
            self.choose_file(&ctx, shared);
        }
        if let Some(path) = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .first()
                .map(|file| file.path().to_path_buf())
        }) {
            self.open_result(Location::from_path(path), Navigation::New, &ctx, shared);
        }
        let mut actions = TabActions::default();
        let (focus_address, find_escape, find_count) =
            self.shortcuts(&ctx, shared, autofocus, &mut actions);
        let busy = self.pending.is_some();
        if !self.focus_mode {
            self.toolbar(ui, shared, &mut actions, focus_address, busy, devtools_open);
            self.status_bar(ui, palette, &mut actions, busy);
        } else {
            self.focus_toolbar(ui, busy);
        }

        // Apply keyboard shortcuts and status-bar button clicks together, after
        // the controls have had a chance to update their flags.
        if actions.zoom_in {
            self.zoom = zoom::step(self.zoom, true);
        }
        if actions.zoom_out {
            self.zoom = zoom::step(self.zoom, false);
        }
        if actions.zoom_reset {
            self.zoom = 100;
        }

        if self.find.open {
            ui.push_id(self.id, |ui| self.find.show(ui, find_count));
        }
        // A page fills the window beneath the glass frame; the new-tab page
        // sits on the backdrop itself.
        let paper = self.loaded.as_ref().map(|loaded| {
            if self.focus_mode {
                self.focus_settings.theme.reading_paper()
            } else {
                loaded.page.background.unwrap_or(DOCUMENT_PAPER)
            }
        });
        egui::CentralPanel::default()
            .frame(egui::Frame::new())
            .show(ui, |ui| {
                if let Some(paper) = paper {
                    let window = ctx.content_rect();
                    ui.painter()
                        .with_clip_rect(window)
                        .rect_filled(window, 0.0, paper);
                }
                if self.focus_mode {
                    ui.style_mut().visuals = self.focus_settings.theme.reading_visuals();
                }
                // Errors float with the frame; the page scrolls beneath them.
                glass::in_band(ui, Band::Top, |ui| {
                    self.error_banner(ui, palette, &mut actions)
                });
                if self.loaded.is_some() {
                    self.document_view(ui, shared, &mut actions, busy);
                } else {
                    let area = ui.available_rect_before_wrap();
                    shared.page_view.set(Some(area));
                    // A page here would be as wide as predicted; its status bar
                    // is a little taller.
                    if shared.page_area.get().is_none() {
                        shared.page_area.set(Some(area.size()));
                    }
                    let choice = new_tab::show(
                        ui,
                        shared.profile.data.settings.browser_theme,
                        &shared.profile.data.bookmarks,
                        &shared.browsing_history,
                        &mut shared.favicons,
                    );
                    actions.open_file |= choice.open_file;
                    actions.open_url = choice.url;
                    actions.link_new_tab |= choice.new_tab;
                }
            });
        self.alert_window(&ctx);
        self.apply(actions, find_escape, &ctx, shared)
    }

    /// Consume this tab's shortcuts. Overlapping shortcuts are checked longest
    /// first (Cmd+Shift+F before Cmd+F), so the order here matters. Returns
    /// whether to focus the address field, whether Escape closed the find bar,
    /// and the number of find matches.
    fn shortcuts(
        &mut self,
        ctx: &egui::Context,
        shared: &mut Shared,
        autofocus: bool,
        actions: &mut TabActions,
    ) -> (bool, bool, usize) {
        let focus_address = autofocus | shortcut(ctx, egui::Modifiers::COMMAND, egui::Key::L);
        let find_escape = self.find.open && shortcut(ctx, egui::Modifiers::NONE, egui::Key::Escape);
        if find_escape {
            self.find.open = false;
        }
        let previous_focus_mode = self.focus_mode;
        if self.focus_mode
            && (shortcut(ctx, egui::Modifiers::NONE, egui::Key::Escape) || focus_address)
        {
            self.focus_mode = false;
        }
        if shortcut(
            ctx,
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            egui::Key::F,
        ) {
            self.toggle_focus(shared);
        }
        if shortcut(ctx, egui::Modifiers::COMMAND, egui::Key::F) {
            self.find.activate();
        }
        if previous_focus_mode != self.focus_mode {
            self.find.current = 0;
            self.find.scroll = true;
        }
        let find_count = self.loaded.as_mut().map_or(0, |loaded| {
            let page = if self.focus_mode {
                &mut loaded.reading
            } else {
                &mut loaded.page
            };
            page.find_query(if self.find.open { &self.find.query } else { "" })
        });
        if self.find.open {
            if shortcut(
                ctx,
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::G,
            ) || shortcut(ctx, egui::Modifiers::SHIFT, egui::Key::F3)
            {
                self.find.step(find_count, true);
            } else if shortcut(ctx, egui::Modifiers::COMMAND, egui::Key::G)
                || shortcut(ctx, egui::Modifiers::NONE, egui::Key::F3)
            {
                self.find.step(find_count, false);
            }
        }
        actions.show_history = shortcut(
            ctx,
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            egui::Key::H,
        );
        actions.back = shortcut(ctx, egui::Modifiers::ALT, egui::Key::ArrowLeft);
        actions.forward = shortcut(ctx, egui::Modifiers::ALT, egui::Key::ArrowRight);
        actions.reload = shortcut(ctx, egui::Modifiers::COMMAND, egui::Key::R)
            || shortcut(ctx, egui::Modifiers::NONE, egui::Key::F5);
        actions.zoom_in = shortcut(ctx, egui::Modifiers::COMMAND, egui::Key::Plus)
            || shortcut(ctx, egui::Modifiers::COMMAND, egui::Key::Equals);
        actions.zoom_out = shortcut(ctx, egui::Modifiers::COMMAND, egui::Key::Minus);
        actions.zoom_reset = shortcut(ctx, egui::Modifiers::COMMAND, egui::Key::Num0);
        (focus_address, find_escape, find_count)
    }

    fn toolbar(
        &mut self,
        ui: &mut egui::Ui,
        shared: &mut Shared,
        actions: &mut TabActions,
        focus_address: bool,
        busy: bool,
        devtools_open: &mut bool,
    ) {
        let palette = shared.profile.data.settings.browser_theme.palette();
        let working = self.working();
        let mut address_pane = egui::Rect::NOTHING;
        // Three glass capsules float above the page: navigation, the address,
        // and the page and browser actions.
        egui::Panel::top("toolbar")
            .exact_size(56.0)
            .show_separator_line(false)
            .frame(egui::Frame::new().inner_margin(egui::Margin {
                left: 12,
                right: 12,
                top: 4,
                bottom: 6,
            }))
            .show(ui, |ui| {
                glass::in_band(ui, Band::Top, |ui| {
                    ui.horizontal_centered(|ui| {
                        ui.spacing_mut().item_spacing.x = GROUP_GAP;
                        ui.spacing_mut().button_padding = egui::vec2(8.0, 8.0);
                        let compact = ui.available_width() < 800.0;
                        glass::capsule(ui, GROUP_PAD, CAPSULE, &palette, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 0.0;
                                actions.back |= ui
                                    .add_enabled(
                                        self.history.back().is_some(),
                                        IconButton::icon_only(Icon::Back, "Back").quiet(),
                                    )
                                    .on_hover_text("Back (Alt+Left)")
                                    .clicked();
                                actions.forward |= ui
                                    .add_enabled(
                                        self.history.forward().is_some(),
                                        IconButton::icon_only(Icon::Forward, "Forward").quiet(),
                                    )
                                    .on_hover_text("Forward (Alt+Right)")
                                    .clicked();
                                if working {
                                    actions.stop |= ui
                                        .add(IconButton::icon_only(Icon::Close, "Stop tab").quiet())
                                        .on_hover_text("Stop this tab (Escape)")
                                        .clicked();
                                } else {
                                    actions.reload |= ui
                                        .add_enabled(
                                            self.loaded.is_some() || !self.address.is_empty(),
                                            IconButton::icon_only(Icon::Reload, "Reload").quiet(),
                                        )
                                        .on_hover_text("Reload (Cmd/Ctrl+R or F5)")
                                        .clicked();
                                }
                            });
                        });
                        // Go, bookmark, Focus, theme and menu follow the address field,
                        // which takes the rest of the row. The bookmark slot stays present
                        // on blank tabs to keep the address field stable.
                        let focus_width = if compact {
                            ICON_BUTTON
                        } else {
                            let label = ui.painter().layout_no_wrap(
                                "Focus".into(),
                                egui::TextStyle::Button.resolve(ui.style()),
                                palette.ink,
                            );
                            2.0 * ui.spacing().button_padding.x + 18.0 + 7.0 + label.size().x
                        };
                        let pad = 2.0 * f32::from(GROUP_PAD);
                        let trailing = pad
                            + 4.0
                            + ICON_BUTTON
                            + GROUP_GAP
                            + pad
                            + 3.0 * ICON_BUTTON
                            + focus_width;
                        let address_width = (ui.available_width() - trailing).max(60.0);
                        let site = self.site();
                        address_pane = glass::capsule(ui, GROUP_PAD, CAPSULE, &palette, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                let mut output = ui
                                    .scope(|ui| {
                                        // The field is clear glass inside its capsule;
                                        // focus draws a leaf-green ring.
                                        let visuals = ui.visuals_mut();
                                        Theme::round_fields(visuals, 18);
                                        visuals.text_edit_bg_color =
                                            Some(egui::Color32::TRANSPARENT);
                                        for state in [
                                            &mut visuals.widgets.inactive,
                                            &mut visuals.widgets.hovered,
                                        ] {
                                            state.bg_stroke = egui::Stroke::NONE;
                                        }
                                        visuals.selection.stroke.width = 2.0;
                                        egui::TextEdit::singleline(&mut self.address)
                                            .id(egui::Id::unique(("address", self.id)))
                                            .hint_text("Enter a URL or file path")
                                            .char_limit(crate::net::MAX_URL_BYTES)
                                            .prefix(egui::Atom::custom(
                                                egui::IdSalt::new(SITE_ICON),
                                                egui::Vec2::splat(16.0),
                                            ))
                                            .desired_width(address_width)
                                            .margin(egui::vec2(12.0, 9.0))
                                            .show(ui)
                                    })
                                    .inner;
                                if let Some(rect) =
                                    output.response.rect(egui::IdSalt::new(SITE_ICON))
                                {
                                    site.icon().paint(ui.painter(), rect, site.color(palette));
                                    if let Some(description) = site.description() {
                                        ui.interact(
                                            rect,
                                            egui::Id::unique((SITE_ICON, self.id)),
                                            egui::Sense::hover(),
                                        )
                                        .on_hover_text(description);
                                    }
                                }
                                if focus_address {
                                    output.response.request_focus();
                                    output.state.cursor.set_char_range(Some(
                                        egui::text::CCursorRange::two(
                                            egui::text::CCursor::new(0),
                                            egui::text::CCursor::new(self.address.chars().count()),
                                        ),
                                    ));
                                    output.state.store(ui.ctx(), output.response.id);
                                }
                                actions.go |= output.response.lost_focus()
                                    && ui.input(|input| input.key_pressed(egui::Key::Enter));
                                actions.go |= ui
                                    .add(IconButton::icon_only(Icon::Go, "Go").primary())
                                    .on_hover_text("Open address")
                                    .clicked();
                            });
                        })
                        .response
                        .rect;
                        glass::capsule(ui, GROUP_PAD, CAPSULE, &palette, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 0.0;
                                let marked = self.loaded.as_ref().is_some_and(|loaded| {
                                    shared.profile.is_bookmarked(loaded.location.as_str())
                                });
                                let label = if marked {
                                    "Remove bookmark"
                                } else {
                                    "Add bookmark"
                                };
                                let bookmark = if marked {
                                    IconButton::icon_only(Icon::StarFilled, label)
                                        .icon_color(palette.oil)
                                } else {
                                    IconButton::icon_only(Icon::Star, label)
                                };
                                actions.toggle_bookmark |= ui
                                    .add_enabled(self.loaded.is_some(), bookmark.quiet())
                                    .on_hover_text(label)
                                    .clicked();
                                let readable = self
                                    .loaded
                                    .as_ref()
                                    .is_some_and(|loaded| !loaded.reading.is_empty());
                                if ui
                                    .add_enabled(
                                        !busy && readable,
                                        if compact {
                                            IconButton::icon_only(Icon::Focus, "Focus mode").quiet()
                                        } else {
                                            IconButton::new(Icon::Focus, "Focus").quiet()
                                        },
                                    )
                                    .on_disabled_hover_text(if readable {
                                        "Wait for the page to finish loading"
                                    } else {
                                        "Open a page with readable text to use Focus mode"
                                    })
                                    .on_hover_text("Focus mode (Cmd/Ctrl+Shift+F)")
                                    .clicked()
                                {
                                    self.toggle_focus(shared);
                                }
                                let dark =
                                    shared.profile.data.settings.browser_theme == Theme::Dark;
                                let theme_label = if dark {
                                    "Switch to light mode"
                                } else {
                                    "Switch to dark mode"
                                };
                                if ui
                                    .add(
                                        IconButton::icon_only(
                                            if dark { Icon::Sun } else { Icon::Moon },
                                            theme_label,
                                        )
                                        .quiet(),
                                    )
                                    .on_hover_text(theme_label)
                                    .clicked()
                                {
                                    shared.profile.data.settings.browser_theme.toggle();
                                }
                                let menu = ui
                                    .add(
                                        IconButton::icon_only(Icon::Menu, "Browser menu")
                                            .quiet()
                                            .warning(
                                                shared.browsing_history.error().is_some()
                                                    || shared.profile.error.is_some(),
                                            ),
                                    )
                                    .on_hover_text(
                                        "Browser menu · files, history, bookmarks and settings",
                                    );
                                self.browser_menu(&menu, shared, actions, devtools_open);
                            });
                        });
                    });
                });
            });
        if working && address_pane.is_finite() {
            // An indeterminate glint slides along the foot of the address capsule.
            let track = address_pane.shrink2(egui::vec2(CAPSULE as f32, 0.0));
            let phase = (ui.input(|input| input.time) as f32 / 1.4).fract();
            let width = track.width() * 0.35;
            let left = track.left() - width + phase * (track.width() + width);
            let bar = egui::Rect::from_min_max(
                egui::pos2(left.max(track.left()), track.bottom() - 3.0),
                egui::pos2((left + width).min(track.right()), track.bottom() - 1.0),
            );
            ui.painter().rect_filled(bar, 1, palette.accent);
            ui.ctx().request_repaint();
        }
    }

    fn browser_menu(
        &self,
        menu: &egui::Response,
        shared: &Shared,
        actions: &mut TabActions,
        devtools_open: &mut bool,
    ) {
        let keys = |modifiers, key| {
            menu.ctx
                .format_shortcut(&egui::KeyboardShortcut::new(modifiers, key))
        };
        egui::Popup::menu(menu).show(|ui| {
            ui.set_min_width(250.0);
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.spacing_mut().button_padding = egui::vec2(8.0, 6.0);
            ui.horizontal(|ui| {
                ui.add(egui::Image::new(&shared.icon).fit_to_exact_size(egui::vec2(20.0, 20.0)));
                ui.label(RichText::new("Olive Browser").strong());
            });
            ui.separator();
            if ui
                .add(
                    IconButton::new(Icon::Folder, "Open HTML…")
                        .quiet()
                        .shortcut(keys(egui::Modifiers::COMMAND, egui::Key::O)),
                )
                .clicked()
            {
                actions.open_file = true;
                ui.close();
            }
            if ui
                .add(
                    IconButton::new(Icon::History, "History")
                        .quiet()
                        .warning(shared.browsing_history.error().is_some())
                        .shortcut(keys(
                            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                            egui::Key::H,
                        )),
                )
                .clicked()
            {
                actions.show_history = true;
                ui.close();
            }
            if ui
                .add(IconButton::new(Icon::Bookmark, "Bookmarks").quiet())
                .clicked()
            {
                actions.show_bookmarks = true;
                ui.close();
            }
            if ui
                .add(IconButton::new(Icon::Download, "Downloads").quiet())
                .clicked()
            {
                actions.show_downloads = true;
                ui.close();
            }
            if ui
                .add_enabled(
                    self.loaded.is_some(),
                    IconButton::new(Icon::Page, "Save page…").quiet(),
                )
                .clicked()
            {
                actions.download_current = true;
                ui.close();
            }
            ui.separator();
            if ui
                .add(IconButton::new(Icon::Settings, "Settings").quiet())
                .clicked()
            {
                actions.show_settings = true;
                ui.close();
            }
            if ui
                .add(
                    IconButton::new(Icon::Code, "Developer tools")
                        .quiet()
                        .shortcut(keys(egui::Modifiers::NONE, egui::Key::F12)),
                )
                .clicked()
            {
                *devtools_open = true;
                ui.close();
            }
            ui.separator();
            ui.label(
                RichText::new(concat!("Version ", env!("CARGO_PKG_VERSION")))
                    .small()
                    .weak(),
            );
        });
    }

    fn status_bar(
        &mut self,
        ui: &mut egui::Ui,
        palette: Palette,
        actions: &mut TabActions,
        busy: bool,
    ) {
        // One glass capsule floats along the foot of the window.
        egui::Panel::bottom("status")
            .show_separator_line(false)
            .frame(egui::Frame::new().inner_margin(egui::Margin {
                left: 12,
                right: 12,
                top: 6,
                bottom: 8,
            }))
            .show(ui, |ui| {
                glass::in_band(ui, Band::Bottom, |ui| {
                    glass::capsule(ui, egui::Margin::symmetric(8, 4), 18, &palette, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                            ui.spacing_mut().button_padding = egui::vec2(6.0, 3.0);
                            ui.spacing_mut().interact_size.y = 24.0;
                            let site = self.site();
                            let (icon, label) = if busy {
                                (None, "Opening…".to_owned())
                            } else {
                                let label =
                                    match self.loaded.as_ref().and_then(|loaded| loaded.status) {
                                        Some(status) if status >= 400 => {
                                            format!("{} · HTTP {status}", site.label())
                                        }
                                        _ => site.label().to_owned(),
                                    };
                                (Some(site.icon()), label)
                            };
                            let (fill, ink) = match site {
                                Site::Secure if !busy => (palette.selection, palette.accent),
                                Site::Insecure if !busy => (palette.warning_bg, palette.warning),
                                _ => (egui::Color32::TRANSPARENT, palette.muted),
                            };
                            let chip = egui::Frame::new()
                                .fill(fill)
                                .corner_radius(6)
                                .inner_margin(egui::Margin::symmetric(7, 3))
                                .show(ui, |ui| {
                                    ui.spacing_mut().item_spacing.x = 5.0;
                                    let (rect, _) = ui.allocate_exact_size(
                                        egui::Vec2::splat(14.0),
                                        egui::Sense::hover(),
                                    );
                                    match icon {
                                        Some(icon) => icon.paint(ui.painter(), rect, ink),
                                        None => ui_icons::paint_spinner(ui, rect, ink),
                                    }
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(label).size(12.0).color(ink),
                                        )
                                        .truncate(),
                                    );
                                })
                                .response;
                            if let Some(description) = site.description().filter(|_| !busy) {
                                chip.on_hover_text(description);
                            }
                            // Zoom out, reset and in share one outlined group.
                            egui::Frame::new()
                                .stroke(egui::Stroke::new(1.0, palette.border))
                                .corner_radius(7)
                                .show(ui, |ui| {
                                    ui.spacing_mut().item_spacing.x = 0.0;
                                    actions.zoom_out |= ui
                                        .add(
                                            IconButton::icon_only(Icon::Minus, "Zoom out")
                                                .small()
                                                .quiet(),
                                        )
                                        .on_hover_text("Zoom out (Cmd/Ctrl−)")
                                        .clicked();
                                    actions.zoom_reset |= ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new(format!("{}%", self.zoom)).size(12.0),
                                            )
                                            .frame(false)
                                            .min_size(egui::vec2(46.0, 24.0)),
                                        )
                                        .on_hover_text("Reset page zoom (Cmd/Ctrl+0)")
                                        .clicked();
                                    actions.zoom_in |= ui
                                        .add(
                                            IconButton::icon_only(Icon::Plus, "Zoom in")
                                                .small()
                                                .quiet(),
                                        )
                                        .on_hover_text("Zoom in (Cmd/Ctrl+)")
                                        .clicked();
                                });
                            if let Some(loaded) = &self.loaded {
                                if ui
                                    .add(IconButton::new(Icon::Search, "Find").small().quiet())
                                    .on_hover_text("Find in page (Cmd/Ctrl+F)")
                                    .clicked()
                                {
                                    self.find.activate();
                                }
                                if loaded.location.is_remote() {
                                    let enabled = loaded.scripting_enabled;
                                    actions.toggle_scripts = ui
                                        .add_enabled(
                                            !busy,
                                            IconButton::new(
                                                if enabled { Icon::CodeOff } else { Icon::Code },
                                                if enabled {
                                                    "Disable JavaScript"
                                                } else {
                                                    "Enable JavaScript"
                                                },
                                            )
                                            .small()
                                            .quiet(),
                                        )
                                        .on_hover_text(SCRIPT_TOGGLE_HINT)
                                        .clicked();
                                }
                                if loaded.resources.attempted > 0 || loaded.resources.limited {
                                    ui_icons::menu(
                                        ui,
                                        if loaded.resources.diagnostics.is_empty() {
                                            Icon::Resources
                                        } else {
                                            Icon::Warning
                                        },
                                        if loaded.resources.diagnostics.is_empty() {
                                            "Resources"
                                        } else {
                                            "Resource errors"
                                        },
                                        |ui| {
                                            ui.label(format!(
                                                "{} of {} resources loaded",
                                                loaded.resources.loaded, loaded.resources.attempted
                                            ));
                                            egui::ScrollArea::vertical().max_height(240.0).show(
                                                ui,
                                                |ui| {
                                                    for message in &loaded.resources.diagnostics {
                                                        ui.label(message);
                                                    }
                                                },
                                            );
                                        },
                                    );
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
                                    ui_icons::menu(
                                        ui,
                                        if loaded.scripts.limited
                                            || !loaded.scripts.diagnostics.is_empty()
                                        {
                                            Icon::Warning
                                        } else {
                                            Icon::Code
                                        },
                                        label,
                                        |ui| {
                                            ui.label(format!(
                                                "{} scripts completed; {} skipped",
                                                loaded.scripts.executed, loaded.scripts.skipped
                                            ));
                                            egui::ScrollArea::vertical().max_height(240.0).show(
                                                ui,
                                                |ui| {
                                                    for error in &loaded.scripts.diagnostics {
                                                        if let Some(source) = &error.source {
                                                            ui.strong(source);
                                                        }
                                                        ui.label(&error.message);
                                                    }
                                                    for message in &loaded.scripts.console {
                                                        ui.monospace(message);
                                                    }
                                                    let omitted = loaded
                                                        .scripts
                                                        .omitted_diagnostics
                                                        .saturating_add(
                                                            loaded.scripts.omitted_console_messages,
                                                        );
                                                    if omitted > 0 {
                                                        ui.label(format!(
                                                            "{omitted} messages omitted"
                                                        ));
                                                    }
                                                },
                                            );
                                        },
                                    );
                                }
                                if loaded.page.css_ignored > 0 {
                                    ui.label(
                                        RichText::new("Some CSS is unsupported").size(11.0).weak(),
                                    )
                                    .on_hover_text(unsupported_css_summary(&loaded.page));
                                }
                                if loaded.corrections > 0 {
                                    ui.label(
                                        RichText::new("Opened with HTML corrections")
                                            .size(11.0)
                                            .weak(),
                                    );
                                }
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.spacing_mut().item_spacing.x = 4.0;
                                    ui.label(
                                        RichText::new(concat!("v", env!("CARGO_PKG_VERSION")))
                                            .size(12.0)
                                            .weak(),
                                    );
                                    let (rect, _) = ui.allocate_exact_size(
                                        egui::Vec2::splat(14.0),
                                        egui::Sense::hover(),
                                    );
                                    Icon::Leaf.paint(ui.painter(), rect, palette.accent);
                                },
                            );
                        });
                    });
                });
            });
    }

    fn error_banner(&self, ui: &mut egui::Ui, palette: Palette, actions: &mut TabActions) {
        if let Some(error) = &self.error {
            egui::Frame::new()
                .fill(palette.error_bg)
                .stroke(egui::Stroke::new(1.0, palette.error.gamma_multiply(0.35)))
                .corner_radius(12)
                .inner_margin(16)
                .outer_margin(12)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_top(|ui| {
                        let (rect, _) =
                            ui.allocate_exact_size(egui::Vec2::splat(24.0), egui::Sense::hover());
                        Icon::Warning.paint(ui.painter(), rect, palette.error);
                        ui.add_space(4.0);
                        ui.vertical(|ui| {
                            ui.visuals_mut().override_text_color = Some(palette.error);
                            ui.label(
                                RichText::new(if self.crashed {
                                    "This tab stopped"
                                } else {
                                    "Could not open this page"
                                })
                                .size(16.0)
                                .variations([("wght", 650.0)]),
                            );
                            ui.label(error);
                            ui.add_space(4.0);
                            ui.visuals_mut().override_text_color = None;
                            if ui
                                .add(IconButton::new(Icon::Reload, "Reload tab"))
                                .clicked()
                            {
                                actions.reload = true;
                            }
                        });
                    });
                });
        }
    }

    /// Show the page. It scrolls in the area between the frame's bands and
    /// paints across the whole window, beneath them.
    fn document_view(
        &mut self,
        ui: &mut egui::Ui,
        shared: &Shared,
        actions: &mut TabActions,
        busy: bool,
    ) {
        let Some(loaded) = &mut self.loaded else {
            return;
        };
        let focus = self.focus_mode;
        let resource_pending = loaded.resources.pending;
        let scripting_enabled = loaded.scripting_enabled;
        let page = if focus {
            &mut loaded.reading
        } else {
            &mut loaded.page
        };
        if focus {
            page.set_reading_style(self.focus_settings);
        }
        let count = page.find_query(if self.find.open { &self.find.query } else { "" });
        self.find.current = self.find.current.min(count.saturating_sub(1));
        page.find_current = self.find.current;
        page.find_scroll |= std::mem::take(&mut self.find.scroll) && count > 0;
        egui::ScrollArea::both()
            .id_salt(("document", self.id, self.generation, focus))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let view = ui.clip_rect();
                shared.page_view.set(Some(view));
                ui.set_clip_rect(ui.ctx().content_rect());
                if !focus {
                    // Native form controls belong to the document, not browser chrome.
                    ui.style_mut().visuals = Theme::Light.visuals();
                }
                let column = if focus { 720.0 } else { ui.available_width() };
                let margin = if focus {
                    ((ui.available_width() - column) / 2.0).max(24.0)
                } else {
                    0.0
                };
                // Keep the column centered beyond the integer frame-margin limit.
                let inset = if focus {
                    (margin - 100.0).max(0.0)
                } else {
                    0.0
                };
                let mut rect = ui.available_rect_before_wrap();
                rect.min.x += inset;
                rect.max.x -= inset;
                ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                    let mut overflow = egui::Vec2::ZERO;
                    // Pages bring their own spacing through the `body` margin, as in
                    // other browsers; the reading column keeps a generous frame.
                    egui::Frame::new()
                        .inner_margin(egui::Margin {
                            left: margin.min(100.0) as i8,
                            right: margin.min(100.0) as i8,
                            top: if focus { 34 } else { 0 },
                            bottom: if focus { 40 } else { 0 },
                        })
                        .show(ui, |ui| {
                            if page.is_empty() {
                                egui::Frame::new().inner_margin(8).show(ui, |ui| {
                                    ui.label(if resource_pending {
                                        "Loading page resources…"
                                    } else {
                                        "This document has no visible content."
                                    });
                                });
                            }
                            let css_viewport = css_viewport(view.size(), self.zoom);
                            if !focus {
                                shared.page_area.set(Some(view.size()));
                            }
                            let rendered = ui
                                .push_id((self.id, self.generation), |ui| {
                                    ui.add_enabled_ui(
                                        !busy
                                            && !self.crashed
                                            && (!self.worker.as_ref().is_some_and(Worker::busy)
                                                || (resource_pending && !scripting_enabled)),
                                        |ui| {
                                            let page_zoom = self.zoom;
                                            zoom::show(
                                                ui,
                                                self.id,
                                                glass::page_layer(),
                                                page_zoom,
                                                view,
                                                |ui, view| {
                                                    page.show_in_view(
                                                        ui,
                                                        &mut self.image_textures,
                                                        view,
                                                    )
                                                },
                                            )
                                        },
                                    )
                                    .inner
                                })
                                .inner;
                            overflow = rendered.overflow;
                            if let Some(click) = rendered.inner {
                                actions.link = click.href;
                                actions.link_new_tab = click.new_tab;
                                actions.form = click.form;
                                if !click.new_tab && !busy && !self.crashed && scripting_enabled {
                                    if let (Some(target), Some(worker)) =
                                        (click.target, self.worker.as_mut())
                                    {
                                        if !worker.busy() {
                                            self.pending_form = actions.form.take();
                                            if let Err(error) = worker.send(Command::Click {
                                                click: ClickRequest {
                                                    target,
                                                    href: actions.link.take(),
                                                },
                                                cookies: shared.cookies.clone(),
                                            }) {
                                                self.error = Some(error);
                                                self.pending_form = None;
                                            }
                                        }
                                        actions.link = None;
                                    }
                                }
                            }
                            if !focus && css_viewport != self.css_viewport {
                                if let Some(worker) = self.worker.as_mut() {
                                    if !worker.busy()
                                        && worker
                                            .send(Command::Resize {
                                                width: css_viewport.0 as f32,
                                                height: css_viewport.1 as f32,
                                            })
                                            .is_ok()
                                    {
                                        self.css_viewport = css_viewport;
                                    }
                                }
                            }
                            if focus && page.truncated {
                                ui.label(
                                    "This reading view was shortened to keep the page responsive.",
                                );
                            }
                        });
                    // Reserve the portion of the transformed document that extends
                    // beyond the viewport after the frame has painted.
                    ui.allocate_space(overflow);
                });
            });
    }

    fn alert_window(&mut self, ctx: &egui::Context) {
        if let Some(message) = &self.alert {
            let mut close = false;
            egui::Window::new("JavaScript alert")
                .id(egui::Id::unique(("alert", self.id)))
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
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
    }

    /// Act on the frame's requests after every panel has drawn. Navigation
    /// requests are exclusive and follow this priority order.
    fn apply(
        &mut self,
        mut actions: TabActions,
        find_escape: bool,
        ctx: &egui::Context,
        shared: &mut Shared,
    ) -> Option<Location> {
        if actions.show_history {
            shared.history_window.toggle();
        }
        if actions.show_bookmarks {
            shared.daily_windows.bookmarks = true;
        }
        if actions.show_settings {
            shared.daily_windows.settings = true;
        }
        if actions.show_downloads {
            shared.downloads.open = true;
        }
        if actions.toggle_bookmark {
            if let Some(loaded) = &self.loaded {
                if let Err(error) = shared
                    .profile
                    .toggle_bookmark(&loaded.location, &loaded.page.title)
                {
                    self.error = Some(error);
                }
            }
        }
        if actions.download_current {
            if let Some(loaded) = &self.loaded {
                shared.downloads.start(
                    loaded.location.clone(),
                    shared.profile.data.settings.download_directory.clone(),
                );
                shared.downloads.open = true;
            }
        }
        let daily_location = shared.daily_windows.show(
            ctx,
            &mut shared.profile,
            &mut shared.downloads,
            &mut shared.favicons,
        );
        let history_location = shared.history_window.show(
            ctx,
            &mut shared.browsing_history,
            &mut shared.favicons,
            false,
        );
        actions.stop |= !find_escape
            && !self.focus_mode
            && shortcut(ctx, egui::Modifiers::NONE, egui::Key::Escape);
        if actions.stop {
            self.stop();
        }
        if actions.open_file {
            self.choose_file(ctx, shared);
        } else {
            if let Some(location) = history_location
                .or_else(|| daily_location.and_then(|url| Location::from_input(&url).ok()))
            {
                self.open(location, Navigation::New, ctx, shared);
            } else if let Some(url) = actions.open_url {
                let location = Location::from_input(&url);
                if actions.link_new_tab {
                    match location {
                        Ok(location) => return Some(location),
                        Err(error) => self.error = Some(error),
                    }
                } else {
                    self.open_result(location, Navigation::New, ctx, shared);
                }
            } else if actions.go {
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
                self.open_result(location, navigation, ctx, shared);
            } else if actions.back {
                if let Some((index, location)) = self.history.back() {
                    self.open(location, Navigation::Traverse(index), ctx, shared);
                }
            } else if actions.forward {
                if let Some((index, location)) = self.history.forward() {
                    self.open(location, Navigation::Traverse(index), ctx, shared);
                }
            } else if actions.toggle_scripts {
                if let Some(loaded) = &self.loaded {
                    self.open_with_scripts(
                        loaded.location.clone(),
                        Navigation::Reload,
                        ctx,
                        !loaded.scripting_enabled,
                        shared,
                    );
                }
            } else if actions.reload {
                if let Some(pending) = &self.pending {
                    self.open(pending.requested.clone(), pending.navigation, ctx, shared);
                } else if let Some(loaded) = &self.loaded {
                    self.open(loaded.location.clone(), Navigation::Reload, ctx, shared);
                } else {
                    self.open_result(
                        Location::from_input(&self.address),
                        Navigation::New,
                        ctx,
                        shared,
                    );
                }
            } else if let Some(activation) = actions.form {
                self.activate_form(activation, ctx, shared);
            } else if let (Some(href), Some(loaded)) = (actions.link, &self.loaded) {
                let location = loaded.base.resolve(&href);
                if actions.link_new_tab {
                    match location {
                        Ok(location) => return Some(location),
                        Err(error) => self.error = Some(error),
                    }
                } else {
                    self.open_result(location, Navigation::New, ctx, shared);
                }
            }
        }
        None
    }
}

/// Hover text for the status bar's unsupported-CSS notice.
fn unsupported_css_summary(page: &Page) -> String {
    use std::fmt::Write;
    let mut summary = format!(
        "{} CSS rules or declarations were ignored.",
        page.css_ignored
    );
    if !page.css_unsupported.is_empty() {
        summary.push_str("\n\nUnsupported properties:");
        for (name, count) in page.css_unsupported.iter().take(16) {
            let _ = write!(summary, "\n{name}: {count}");
        }
    }
    summary
}

pub struct OliveApp {
    shared: Shared,
    tabs: Vec<Tab>,
    active: usize,
    next_id: u64,
    focus_address: bool,
    reveal_tab: bool,
    recovery: Option<Session>,
    window_title: String,
    devtools: DevTools,
}

impl OliveApp {
    pub fn new(cc: &eframe::CreationContext<'_>, source: Option<OsString>) -> Self {
        let ctx = &cc.egui_ctx;
        ctx.set_fonts(crate::gui::fonts::definitions());
        let icon = ctx.load_texture(
            "olive-browser-icon",
            egui::ColorImage::from(icon::data()),
            egui::TextureOptions::LINEAR,
        );

        let mut profile = Profile::load_default();
        profile.data.settings.browser_theme.apply(ctx);
        let interrupted = profile.begin_session();
        let mut app = Self {
            shared: Shared {
                icon,
                browsing_history: BrowsingHistory::load_default(),
                history_window: HistoryWindow::default(),
                cookies: CookieJar::default(),
                profile,
                downloads: Downloads::default(),
                daily_windows: DailyWindows::default(),
                favicons: Favicons::load_default(),
                page_area: Default::default(),
                page_view: Default::default(),
            },
            tabs: Vec::new(),
            active: 0,
            next_id: 0,
            focus_address: source.is_none(),
            reveal_tab: false,
            recovery: None,
            window_title: String::new(),
            devtools: DevTools::default(),
        };
        // Keep only the icons that bookmarks and history can still show.
        app.shared.favicons.prune(
            app.shared
                .browsing_history
                .entries()
                .iter()
                .map(|entry| entry.url.as_str())
                .chain(
                    app.shared
                        .profile
                        .data
                        .bookmarks
                        .iter()
                        .map(|bookmark| bookmark.url.as_str()),
                ),
        );
        let tab = app.blank_tab();
        app.tabs.push(tab);
        if let Some(source) = source {
            let location = match source.to_str() {
                Some(input) => Location::from_input(input),
                None => Location::from_path(PathBuf::from(source)),
            };
            app.tabs[0].open_result(location, Navigation::New, ctx, &mut app.shared);
        } else if app.shared.profile.data.settings.restore_session {
            let session = app.shared.profile.data.session.clone();
            if interrupted && !session.tabs.is_empty() {
                app.recovery = Some(session);
            } else {
                app.restore_session(session, true);
            }
        }
        app
    }

    fn restore_session(&mut self, session: Session, scripting: bool) {
        if session.tabs.is_empty() {
            return;
        }
        self.tabs.clear();
        for saved in session.tabs.into_iter().take(MAX_TABS) {
            let mut tab = self.blank_tab();
            tab.zoom = saved.zoom;
            tab.focus_settings = saved.focus;
            tab.deferred_scripts = scripting;
            tab.deferred = saved.url.and_then(|url| Location::from_input(&url).ok());
            if let Some(location) = &tab.deferred {
                tab.address = location.as_str().into();
            }
            self.tabs.push(tab);
        }
        self.active = session.active.min(self.tabs.len() - 1);
        self.reveal_tab = true;
        self.focus_address = false;
    }

    fn checkpoint(&mut self) {
        // Preserve the recovery snapshot until the user chooses what to do with it.
        if self.recovery.is_none() {
            self.shared.profile.data.session = Session {
                tabs: self.tabs.iter().map(Tab::saved_tab).collect(),
                active: self.active,
            };
        }
        self.shared.profile.save_if_changed();
    }

    fn apply_browser_theme(&mut self, ctx: &egui::Context) {
        self.shared.profile.data.settings.browser_theme.apply(ctx);
        // Both active and hidden views retain galleys referencing the old font atlas.
        // Keep document state and scroll IDs; only the text needs laying out again.
        for tab in &mut self.tabs {
            if let Some(loaded) = &mut tab.loaded {
                loaded.page.invalidate_text_layout();
                loaded.reading.invalidate_text_layout();
            }
        }
    }

    fn recovery_bar(&mut self, ui: &mut egui::Ui) {
        if self.recovery.is_none() {
            return;
        }
        let palette = self.shared.profile.data.settings.browser_theme.palette();
        let mut restore = false;
        let mut discard = false;
        egui::Panel::top("session-recovery")
            .show_separator_line(false)
            .frame(egui::Frame::new().inner_margin(egui::Margin {
                left: 12,
                right: 12,
                top: 8,
                bottom: 0,
            }))
            .show(ui, |ui| {
                glass::in_band(ui, Band::Top, |ui| {
                    egui::Frame::new()
                        .fill(palette.warning_bg.gamma_multiply(0.92))
                        .stroke(egui::Stroke::new(1.0, palette.warning.gamma_multiply(0.3)))
                        .corner_radius(18)
                        .inner_margin(egui::Margin::symmetric(14, 10))
                        .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::Vec2::splat(20.0), egui::Sense::hover());
                    Icon::History.paint(ui.painter(), rect, palette.warning);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(
                            RichText::new(
                                "Olive did not close normally. Restore your saved tabs or start fresh.",
                            )
                            .strong(),
                        );
                        ui.small("Restored pages start with JavaScript disabled. Background tabs load when selected.");
                    });
                    restore = ui
                        .add(IconButton::new(Icon::Reload, "Restore tabs").primary())
                        .clicked();
                    discard = ui
                        .add(IconButton::new(Icon::Leaf, "Start fresh").quiet())
                        .clicked();
                });
                        });
                });
            });
        if restore {
            let session = self.recovery.take().unwrap();
            self.restore_session(session, false);
        } else if discard {
            self.recovery = None;
        }
    }

    fn new_tab(&mut self, location: Option<Location>, ctx: &egui::Context) {
        if self.tabs.len() >= MAX_TABS {
            self.tabs[self.active].error = Some(format!(
                "Close a tab before opening another ({MAX_TABS} tab limit)."
            ));
            return;
        }
        let mut tab = self.blank_tab();
        self.focus_address = location.is_none();
        if let Some(location) = location {
            tab.opener = self.tabs[self.active]
                .loaded
                .as_ref()
                .map(|loaded| loaded.location.clone());
            tab.open(location, Navigation::New, ctx, &mut self.shared);
        }
        self.tabs.push(tab);
        self.select(self.tabs.len() - 1);
    }
    /// A new tab with the saved default zoom and Focus preferences.
    fn blank_tab(&mut self) -> Tab {
        let mut tab = Tab::new(self.next_id);
        self.next_id += 1;
        let settings = &self.shared.profile.data.settings;
        tab.zoom = settings.default_zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        tab.focus_settings = settings.focus;
        tab
    }
    fn select(&mut self, index: usize) {
        self.active = index.min(self.tabs.len() - 1);
        self.reveal_tab = true;
    }
    fn close_tab(&mut self, index: usize, ctx: &egui::Context) {
        self.tabs.remove(index); // Drop terminates this tab's current and pending children.
        if self.tabs.is_empty() {
            self.active = 0;
            self.new_tab(None, ctx);
        } else {
            if index < self.active {
                self.active -= 1;
            }
            self.select(self.active);
        }
    }

    fn tab_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let palette = self.shared.profile.data.settings.browser_theme.palette();
        let mut selected = None;
        let mut close = None;
        let mut add = false;
        // Tabs float on the backdrop; the active one is the brightest glass.
        egui::Panel::top("tabs")
            .exact_size(50.0)
            .show_separator_line(false)
            .frame(egui::Frame::new().inner_margin(egui::Margin {
                left: 12,
                right: 10,
                top: 4,
                bottom: 0,
            }))
            .show(ui, |ui| {
                glass::in_band(ui, Band::Top, |ui| {
                    ui.horizontal_centered(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.add(
                            egui::Image::new(&self.shared.icon)
                                .fit_to_exact_size(egui::vec2(24.0, 24.0)),
                        )
                        .on_hover_text("Olive Browser");
                        ui.add_space(6.0);
                        let width = (ui.available_width() - 44.0).max(100.0);
                        egui::ScrollArea::horizontal()
                            .id_salt("tab-strip")
                            .max_width(width)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                // Room for the panes' shadows inside the strip's clip.
                                egui::Frame::new()
                                    .inner_margin(egui::Margin::symmetric(4, 6))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.spacing_mut().item_spacing.x = 4.0;
                                            for (index, tab) in self.tabs.iter().enumerate() {
                                                let active = index == self.active;
                                                let favicon = tab.favicon.clone().or_else(|| {
                                                    self.shared.favicons.get(
                                                        &ctx,
                                                        tab.page_address().unwrap_or_default(),
                                                    )
                                                });
                                                let card = ui.push_id(tab.id, |ui| {
                                                    tab_card(
                                                        ui,
                                                        tab,
                                                        favicon
                                                            .as_ref()
                                                            .map(egui::TextureHandle::id),
                                                        active,
                                                        &palette,
                                                    )
                                                });
                                                let (response, closed) = card.inner;
                                                if self.reveal_tab && active {
                                                    response
                                                        .scroll_to_me(Some(egui::Align::Center));
                                                }
                                                if response.clicked() {
                                                    selected = Some(index);
                                                }
                                                if closed
                                                    || response
                                                        .clicked_by(egui::PointerButton::Middle)
                                                {
                                                    close = Some(index);
                                                }
                                            }
                                        });
                                    });
                            });
                        add = ui
                            .add_enabled(
                                self.tabs.len() < MAX_TABS,
                                IconButton::icon_only(Icon::Plus, "New tab").quiet(),
                            )
                            .on_hover_text("New tab (Cmd/Ctrl+T)")
                            .on_disabled_hover_text(format!(
                                "Close a tab to open another ({MAX_TABS} tab limit)"
                            ))
                            .clicked();
                    });
                });
            });
        self.reveal_tab = false;
        if let Some(index) = selected {
            self.select(index);
        }
        if let Some(index) = close {
            self.close_tab(index, &ctx);
        }
        if add {
            self.new_tab(None, &ctx);
        }
    }
}

/// One tab in the strip: its site icon (or a drawn one), title and close
/// button, on a glass pane while it is active or under the pointer. Returns
/// the title button's response and whether close was clicked.
fn tab_card(
    ui: &mut egui::Ui,
    tab: &Tab,
    favicon: Option<egui::TextureId>,
    active: bool,
    palette: &Palette,
) -> (egui::Response, bool) {
    let mut closed = false;
    let pane = glass::reserve(ui);
    let card = egui::Frame::new().inner_margin(4).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let busy = tab.working();
            let title: String = tab.title().chars().take(80).collect();
            let failed = tab.crashed || tab.error.is_some();
            let icon = if failed {
                Icon::Warning
            } else {
                match tab.site() {
                    Site::Blank if tab.address.is_empty() => Icon::Leaf,
                    Site::Blank | Site::Secure | Site::Insecure => Icon::Globe,
                    Site::Local => Icon::Page,
                }
            };
            let ink = if active { palette.ink } else { palette.muted };
            let button = IconButton::new(icon, RichText::new(title).color(ink))
                .icon_color(if failed {
                    palette.warning
                } else if active {
                    palette.accent
                } else {
                    palette.muted
                })
                .image(favicon.filter(|_| !failed))
                .spinning(busy)
                .bare()
                .align_start()
                .truncate()
                .min_size(egui::vec2(140.0, 30.0));
            let process = tab
                .worker
                .as_ref()
                .or_else(|| tab.pending.as_ref().map(|p| &p.worker))
                .and_then(Worker::pid)
                .map(|pid| format!("Tab process {pid}"))
                .unwrap_or_else(|| "New tab".into());
            let response = ui.add_sized([176.0, 30.0], button).on_hover_text(format!(
                "{}\n{}",
                tab.title(),
                if busy {
                    format!("Loading… · {process}")
                } else {
                    process
                }
            ));
            closed = ui
                .add(
                    IconButton::icon_only(Icon::Close, "Close tab")
                        .icon_color(ink)
                        .small()
                        .quiet(),
                )
                .on_hover_text("Close tab (Cmd/Ctrl+W)")
                .clicked();
            response
        })
        .inner
    });
    let rect = card.response.rect;
    if active {
        glass::fill(ui, pane, rect, 15, palette, Pane::Prominent);
    } else if ui.rect_contains_pointer(rect) {
        glass::fill(ui, pane, rect, 15, palette, Pane::Regular);
    }
    (card.inner, closed)
}

impl eframe::App for OliveApp {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.checkpoint();
        if self.recovery.is_none() {
            self.shared.profile.finish_session();
        }
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        visuals.panel_fill.to_normalized_gamma_f32()
    }
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.shared.downloads.poll(ctx);
        for tab in &mut self.tabs {
            tab.receive(ctx, &mut self.shared);
        }
        if self.tabs.iter().any(Tab::working) {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let previous_theme = self.shared.profile.data.settings.browser_theme;
        if shortcut(&ctx, egui::Modifiers::NONE, egui::Key::F12) {
            self.devtools.toggle();
        }
        if shortcut(&ctx, egui::Modifiers::COMMAND, egui::Key::T) {
            self.new_tab(None, &ctx);
        }
        if shortcut(&ctx, egui::Modifiers::COMMAND, egui::Key::W) {
            self.close_tab(self.active, &ctx);
        }
        if shortcut(
            &ctx,
            egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
            egui::Key::Tab,
        ) {
            self.select((self.active + self.tabs.len() - 1) % self.tabs.len());
        } else if shortcut(&ctx, egui::Modifiers::CTRL, egui::Key::Tab) {
            self.select((self.active + 1) % self.tabs.len());
        }
        for (index, key) in [
            egui::Key::Num1,
            egui::Key::Num2,
            egui::Key::Num3,
            egui::Key::Num4,
            egui::Key::Num5,
            egui::Key::Num6,
            egui::Key::Num7,
            egui::Key::Num8,
            egui::Key::Num9,
        ]
        .into_iter()
        .enumerate()
        {
            if shortcut(&ctx, egui::Modifiers::COMMAND, key) {
                self.select(if index == 8 {
                    self.tabs.len() - 1
                } else {
                    index
                });
            }
        }
        ui.spacing_mut().item_spacing.y = 0.0;
        let palette = self.shared.profile.data.settings.browser_theme.palette();
        let window = ctx.content_rect();
        glass::backdrop(ui.painter(), window, &palette);
        // The page fills the window; the frame floats above it in two bands,
        // placed as they were last frame until this frame lays them out.
        let bands = self.shared.bands(window);
        glass::stack_layers(&ctx, ui.layer_id(), bands);
        let top = ctx.layer_painter(Band::Top.layer());
        top.add(glass::capture(&ctx));
        let band_backdrops = [
            top.add(egui::Shape::Noop),
            ctx.layer_painter(Band::Bottom.layer())
                .add(egui::Shape::Noop),
        ];
        self.shared.page_view.set(None);
        self.recovery_bar(ui);
        self.tab_bar(ui);
        if let Some(location) = self.tabs[self.active].deferred.take() {
            let scripting = self.tabs[self.active].restored_scripting(&self.shared);
            self.tabs[self.active].open_with_scripts(
                location,
                Navigation::New,
                &ctx,
                scripting,
                &mut self.shared,
            );
        }
        let autofocus = std::mem::take(&mut self.focus_address);
        if let Some(location) =
            self.tabs[self.active].ui(ui, &mut self.shared, autofocus, &mut self.devtools.open)
        {
            self.new_tab(Some(location), &ctx);
        }
        let laid_out = self.shared.bands(window);
        if self.tabs[self.active].loaded.is_some() {
            for ((band, rect), slot) in [Band::Top, Band::Bottom]
                .into_iter()
                .zip(laid_out)
                .zip(band_backdrops)
            {
                if rect.is_positive() {
                    let shape = glass::band_backdrop(&ctx, rect, band, &palette);
                    ctx.layer_painter(band.layer()).set(slot, shape);
                }
            }
        }
        if laid_out != bands {
            ctx.request_repaint();
        }
        let tab = &self.tabs[self.active];
        self.devtools.show(
            &ctx,
            tab.loaded.as_ref(),
            tab.worker.as_ref().and_then(Worker::pid),
        );
        if self.shared.profile.data.settings.browser_theme != previous_theme {
            self.apply_browser_theme(&ctx);
        }
        self.checkpoint();
        let title: String = self.tabs[self.active].title().chars().take(200).collect();
        if self.window_title != title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(
                if title.ends_with("Olive Browser") {
                    title.clone()
                } else {
                    format!("{title} — Olive Browser")
                },
            ));
            self.window_title = title;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::document::prepare_page_with_loader;
    use crate::net::{DocumentLoader, LoadedDocument};

    fn prepare_page(source: LoadedDocument) -> Result<LoadedPage, String> {
        let scripting = !source.location.is_remote();
        Ok(prepare_page_with_loader(source, &DocumentLoader::new()?, scripting)?.loaded)
    }

    fn load_file(path: PathBuf) -> Result<LoadedPage, String> {
        DocumentLoader::new()?
            .load(Location::from_path(path)?)
            .and_then(prepare_page)
    }

    fn app_for_test(ctx: &egui::Context, loaded: LoadedPage) -> (Tab, Shared) {
        let mut tab = Tab::new(0);
        tab.history.commit(loaded.location.clone(), Navigation::New);
        let mut shared = Shared {
            icon: ctx.load_texture(
                "test-icon",
                egui::ColorImage::from(icon::data()),
                Default::default(),
            ),
            browsing_history: BrowsingHistory::default(),
            history_window: HistoryWindow::default(),
            cookies: CookieJar::default(),
            profile: Profile::default(),
            downloads: Downloads::default(),
            daily_windows: DailyWindows::default(),
            favicons: Favicons::default(),
            page_area: Default::default(),
            page_view: Default::default(),
        };
        shared
            .browsing_history
            .record(&loaded.location, &loaded.page.title);
        tab.address = loaded.location.as_str().into();
        tab.loaded = Some(loaded);
        tab.generation = 1;
        (tab, shared)
    }

    fn remote_source(url: &str) -> LoadedDocument {
        LoadedDocument {
            location: Location::from_input(url).unwrap(),
            bytes: b"<!doctype html><p>Remote</p>".to_vec(),
            status: Some(200),
            plain_text: false,
        }
    }

    fn control_rect(output: &egui::FullOutput, label: &str) -> egui::Rect {
        let node = output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some(label))
            .unwrap_or_else(|| panic!("missing control: {label}"));
        let bounds = node.1.bounds().unwrap();
        egui::Rect::from_min_max(
            egui::pos2(bounds.x0 as f32, bounds.y0 as f32),
            egui::pos2(bounds.x1 as f32, bounds.y1 as f32),
        )
    }

    /// Show one tab with the frame's layers stacked as the browser stacks them.
    fn show_tab(ui: &mut egui::Ui, tab: &mut Tab, shared: &mut Shared) -> Option<Location> {
        let bands = shared.bands(ui.ctx().content_rect());
        glass::stack_layers(ui.ctx(), ui.layer_id(), bands);
        tab.ui(ui, shared, false, &mut false)
    }

    fn chrome_frame(
        ctx: &egui::Context,
        tab: &mut Tab,
        shared: &mut Shared,
        width: f32,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 600.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                show_tab(ui, tab, shared);
            },
        );
        output.textures_delta.clear();
        output
    }

    fn click_chrome(
        ctx: &egui::Context,
        tab: &mut Tab,
        shared: &mut Shared,
        width: f32,
        pos: egui::Pos2,
    ) -> egui::FullOutput {
        for pressed in [true, false] {
            let output = chrome_frame(
                ctx,
                tab,
                shared,
                width,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            if !pressed {
                return output;
            }
        }
        unreachable!()
    }

    #[test]
    fn theme_toggle_and_menu_fit_blank_and_loaded_tabs_at_all_window_sizes() {
        for theme in [Theme::Light, Theme::Dark] {
            for width in [480.0, 800.0, 1000.0] {
                for blank in [true, false] {
                    let ctx = egui::Context::default();
                    ctx.set_fonts(crate::gui::fonts::definitions());
                    ctx.enable_accesskit();
                    theme.apply(&ctx);
                    let (mut tab, mut shared) = app_for_test(
                        &ctx,
                        prepare_page(remote_source("https://example.com/story")).unwrap(),
                    );
                    shared.profile.data.settings.browser_theme = theme;
                    if blank {
                        tab = Tab::new(1);
                    }
                    chrome_frame(&ctx, &mut tab, &mut shared, width, vec![]);
                    let output = chrome_frame(&ctx, &mut tab, &mut shared, width, vec![]);
                    let viewport =
                        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 600.0));
                    let theme_label = if theme == Theme::Light {
                        "Switch to dark mode"
                    } else {
                        "Switch to light mode"
                    };
                    for label in [
                        "Back",
                        "Forward",
                        "Reload",
                        "Go",
                        "Add bookmark",
                        theme_label,
                        "Browser menu",
                    ] {
                        let rect = control_rect(&output, label);
                        assert!(
                            viewport.contains_rect(rect),
                            "{label} clipped at {width}: {rect:?}"
                        );
                    }
                    let menu = control_rect(&output, "Browser menu");
                    click_chrome(&ctx, &mut tab, &mut shared, width, menu.center());
                    let output = chrome_frame(&ctx, &mut tab, &mut shared, width, vec![]);
                    for label in ["History", "Bookmarks", "Downloads", "Settings"] {
                        assert!(
                            viewport.contains_rect(control_rect(&output, label)),
                            "menu item {label} clipped at {width}"
                        );
                    }
                    let settings = control_rect(&output, "Settings");
                    click_chrome(&ctx, &mut tab, &mut shared, width, settings.center());
                    assert!(shared.daily_windows.settings);
                }
            }
        }
    }

    #[test]
    fn new_tab_page_offers_bookmarks_and_recent_pages_and_opens_them_in_new_tabs() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::gui::fonts::definitions());
        ctx.enable_accesskit();
        let (_, mut shared) = app_for_test(
            &ctx,
            prepare_page(remote_source("https://example.com/story")).unwrap(),
        );
        shared
            .profile
            .data
            .bookmarks
            .push(crate::gui::profile::Bookmark {
                url: "https://www.rust-lang.org/".into(),
                title: "Rust".into(),
            });
        let mut tab = Tab::new(1);
        chrome_frame(&ctx, &mut tab, &mut shared, 480.0, vec![]);
        let output = chrome_frame(&ctx, &mut tab, &mut shared, 480.0, vec![]);
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(480.0, 600.0));
        let tile = control_rect(&output, "Rust");
        assert!(
            viewport.contains_rect(tile),
            "bookmark tile clipped: {tile:?}"
        );
        // The recorded page has no title, so its address names it.
        assert!(viewport.contains_rect(control_rect(&output, "https://example.com/story")));

        let mut opened = None;
        for pressed in [true, false] {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(viewport),
                    events: vec![
                        egui::Event::PointerMoved(tile.center()),
                        egui::Event::PointerButton {
                            pos: tile.center(),
                            button: egui::PointerButton::Middle,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    ..Default::default()
                },
                |ui| {
                    if let Some(location) = show_tab(ui, &mut tab, &mut shared) {
                        opened = Some(location);
                    }
                },
            );
            output.textures_delta.clear();
        }
        assert_eq!(opened.unwrap().as_str(), "https://www.rust-lang.org/");
        assert!(tab.loaded.is_none() && tab.pending.is_none() && tab.error.is_none());
    }

    #[test]
    fn browser_theme_click_preserves_document_forms_history_and_focus_preferences() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        Theme::Light.apply(&ctx);
        let mut source = remote_source("https://example.com/story");
        source.bytes = b"<style>body { color: #123456; background: #ffffff; }</style><p>Story</p><input value=edited>".to_vec();
        let (mut tab, mut shared) = app_for_test(&ctx, prepare_page(source).unwrap());
        chrome_frame(&ctx, &mut tab, &mut shared, 480.0, vec![]);
        let output = chrome_frame(&ctx, &mut tab, &mut shared, 480.0, vec![]);
        let page = serde_json::to_value(&tab.loaded.as_ref().unwrap().page).unwrap();
        let saved = tab.saved_tab();
        let toggle = control_rect(&output, "Switch to dark mode");
        click_chrome(&ctx, &mut tab, &mut shared, 480.0, toggle.center());
        assert_eq!(shared.profile.data.settings.browser_theme, Theme::Dark);
        Theme::Dark.apply(&ctx);
        chrome_frame(&ctx, &mut tab, &mut shared, 480.0, vec![]);
        assert_eq!(
            serde_json::to_value(&tab.loaded.as_ref().unwrap().page).unwrap(),
            page
        );
        assert_eq!(tab.saved_tab(), saved);
        assert_eq!(tab.generation, 1);
        assert!(tab.pending.is_none());
        assert_eq!(shared.browsing_history.entries()[0].visits, 1);
        assert!(ctx.global_style().visuals.dark_mode);
    }

    #[test]
    fn focus_toggles_without_reloading_or_recording_visits_and_keeps_preferences() {
        let ctx = egui::Context::default();
        let (mut app, mut shared) = app_for_test(
            &ctx,
            prepare_page(remote_source("https://example.com/story")).unwrap(),
        );
        app.focus_settings = FocusSettings {
            font_size: 25.0,
            theme: Theme::Dark,
        };
        app.toggle_focus(&mut shared);
        assert!(app.focus_mode && app.focus_panel);
        app.focus_panel = false;
        app.toggle_focus(&mut shared);
        assert!(!app.focus_mode);
        app.toggle_focus(&mut shared);
        assert!(app.focus_mode && app.focus_panel);
        assert_eq!(app.focus_settings.font_size, 25.0);
        assert_eq!(app.focus_settings.theme, Theme::Dark);
        assert!(app.pending.is_none());
        assert_eq!(app.generation, 1);
        assert_eq!(shared.browsing_history.entries()[0].visits, 1);
        let mut source = remote_source("https://example.com/empty");
        source.bytes = b"<title>No article</title><nav>Site links</nav>".to_vec();
        app.loaded = Some(prepare_page(source).unwrap());
        app.focus_mode = false;
        app.toggle_focus(&mut shared);
        assert!(!app.focus_mode);
    }

    #[test]
    fn focus_controls_fit_the_minimum_window_width_in_both_themes() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::gui::fonts::definitions());
        let (mut app, _shared) = app_for_test(
            &ctx,
            prepare_page(remote_source("https://example.com/story")).unwrap(),
        );
        for theme in [Theme::Light, Theme::Dark] {
            app.focus_settings.theme = theme;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(480.0, 360.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    app.focus_toolbar(ui, false);
                    assert!(ui.available_height() >= 200.0);
                },
            );
            let labels: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| {
                    if let egui::Shape::Text(text) = &shape.shape {
                        Some(text)
                    } else {
                        None
                    }
                })
                .collect();
            assert!(labels.iter().any(|text| text.galley.text() == "Exit Focus"));
            assert!(labels.iter().any(|text| text.galley.text() == "Dark"));
            for text in labels {
                assert!(
                    text.pos.x >= 0.0 && text.pos.x + text.galley.size().x <= 480.0,
                    "control clipped: {}",
                    text.galley.text()
                );
            }
            output.textures_delta.clear();
        }
    }

    #[test]
    fn failed_navigation_keeps_the_page_address_and_forward_history() {
        let ctx = egui::Context::default();
        let original = Location::from_input("https://example.com/first").unwrap();
        let (mut app, mut shared) = app_for_test(
            &ctx,
            prepare_page(remote_source(original.as_str())).unwrap(),
        );
        let next = Location::from_input("https://example.com/second").unwrap();
        app.toggle_focus(&mut shared);
        app.history.commit(next.clone(), Navigation::New);
        app.history
            .commit(original.clone(), Navigation::Traverse(0));
        let requested = Location::from_input("https://missing.example/").unwrap();
        app.address = requested.as_str().into();
        let (worker, sender) = Worker::mock();
        assert!(sender.send(Err("Test connection failure".into())).is_ok());
        app.pending = Some(PendingPage {
            viewport: None,
            worker,
            requested,
            navigation: Navigation::New,
        });
        app.receive(&ctx, &mut shared);
        assert_eq!(app.loaded.as_ref().unwrap().location, original);
        assert_eq!(app.address, original.as_str());
        assert_eq!(app.history.forward().unwrap().1, next);
        assert!(app.pending.is_none());
        assert!(app.error.as_ref().unwrap().contains("missing.example"));
        assert_eq!(shared.browsing_history.entries().len(), 1);
        assert_eq!(shared.browsing_history.entries()[0].url, original.as_str());
        assert!(app.focus_mode);
    }

    #[test]
    fn saved_history_records_final_urls_and_successful_reload_and_traversal() {
        let ctx = egui::Context::default();
        let (mut app, mut shared) = app_for_test(
            &ctx,
            prepare_page(remote_source("https://example.com/first")).unwrap(),
        );
        let requested = Location::from_input("https://example.com/redirect").unwrap();
        app.toggle_focus(&mut shared);
        app.focus_settings.font_size = 24.0;
        let final_url = "https://example.com/final";
        let mut source = remote_source(final_url);
        // Readable HTTP error pages are successful navigations too.
        source.status = Some(404);
        source.bytes = b"<!doctype html><title>Not found</title><p>404</p>".to_vec();
        let page = prepare_page(source).unwrap();
        let (worker, sender) = Worker::mock_rendering(&page);
        assert!(sender.send(Ok(Event::Loaded(Box::new(page)))).is_ok());
        app.pending = Some(PendingPage {
            viewport: None,
            worker,
            requested: requested.clone(),
            navigation: Navigation::New,
        });
        app.receive(&ctx, &mut shared);
        assert!(!app.focus_mode);
        assert_eq!(app.focus_settings.font_size, 24.0);
        assert_eq!(shared.browsing_history.entries().len(), 2);
        let entry = &shared.browsing_history.entries()[0];
        assert_eq!(entry.url, final_url);
        assert_eq!(entry.title, "Not found");
        assert!(
            shared
                .browsing_history
                .search(requested.as_str())
                .is_empty()
        );

        for (url, navigation) in [
            (final_url, Navigation::Reload),
            ("https://example.com/first", Navigation::Traverse(0)),
        ] {
            let page = prepare_page(remote_source(url)).unwrap();
            let (worker, sender) = Worker::mock_rendering(&page);
            assert!(sender.send(Ok(Event::Loaded(Box::new(page)))).is_ok());
            app.pending = Some(PendingPage {
                viewport: None,
                worker,
                requested: Location::from_input(url).unwrap(),
                navigation,
            });
            app.receive(&ctx, &mut shared);
            assert_eq!(shared.browsing_history.entries().len(), 2);
            assert_eq!(shared.browsing_history.entries()[0].url, url);
            assert_eq!(shared.browsing_history.entries()[0].visits, 2);
        }
        assert_eq!(app.history.forward().unwrap().1.as_str(), final_url);
        shared.browsing_history.clear();
        assert!(shared.browsing_history.entries().is_empty());
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
        let (mut app, mut shared) = app_for_test(&ctx, prepare_page(remote_source(url)).unwrap());
        app.toggle_focus(&mut shared);
        let anchor = Location::from_input(&format!("{url}#section")).unwrap();
        app.open(anchor.clone(), Navigation::New, &ctx, &mut shared);
        assert!(app.pending.is_none());
        assert!(app.focus_mode);
        assert_eq!(shared.browsing_history.entries()[0].url, anchor.as_str());
        let (index, location) = app.history.back().unwrap();
        app.open(location, Navigation::Traverse(index), &ctx, &mut shared);
        assert!(app.pending.is_none());
        assert_eq!(shared.browsing_history.entries().len(), 2);
        assert_eq!(shared.browsing_history.entries()[0].url, url);
        assert_eq!(shared.browsing_history.entries()[0].visits, 2);
        shared.browsing_history.remove(anchor.as_str());
        assert_eq!(app.history.forward().unwrap().1, anchor);
    }

    #[test]
    fn disconnected_load_does_not_record_a_visit() {
        let ctx = egui::Context::default();
        let (mut app, mut shared) = app_for_test(
            &ctx,
            prepare_page(remote_source("https://example.com/first")).unwrap(),
        );
        let (worker, sender) = Worker::mock();
        drop(sender);
        app.pending = Some(PendingPage {
            viewport: None,
            worker,
            requested: Location::from_input("https://example.com/disconnected").unwrap(),
            navigation: Navigation::New,
        });
        app.receive(&ctx, &mut shared);
        assert!(app.pending.is_none());
        assert!(app.error.is_some());
        assert_eq!(shared.browsing_history.entries().len(), 1);
        assert!(shared.browsing_history.search("disconnected").is_empty());
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
        assert!(!loaded.scripting_enabled);
        assert_eq!(loaded.base.as_str(), "https://example.com/docs/");
        assert!(!loaded.page.is_empty());
    }

    #[test]
    fn javascript_default_applies_to_navigation_and_reload() {
        let ctx = egui::Context::default();
        let (mut tab, mut shared) = app_for_test(
            &ctx,
            prepare_page(remote_source("https://example.com/current")).unwrap(),
        );
        let next = Location::from_input("https://example.com/next").unwrap();
        let current = tab.loaded.as_ref().unwrap().location.clone();
        let local = Location::from_path(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/scripted.html"),
        )
        .unwrap();
        for by_default in [false, true] {
            shared.profile.data.settings.javascript_by_default = by_default;
            for navigation in [Navigation::New, Navigation::Traverse(0)] {
                assert_eq!(
                    tab.scripting_for_load(&next, navigation, &shared),
                    by_default
                );
                assert_eq!(
                    Tab::new(1).scripting_for_load(&next, navigation, &shared),
                    by_default
                );
                assert!(tab.scripting_for_load(&local, navigation, &shared));
            }
            for enabled in [false, true] {
                tab.loaded.as_mut().unwrap().scripting_enabled = enabled;
                assert_eq!(
                    tab.scripting_for_load(&current, Navigation::Reload, &shared),
                    by_default || enabled
                );
            }
        }
    }

    #[test]
    fn theme_changes_refresh_text_in_all_tabs_and_reading_views_without_reload() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::gui::fonts::definitions());
        Theme::Light.apply(&ctx);
        let source = || {
            let mut source = remote_source("https://example.com/story");
            source.bytes =
                b"<article><p>Page glyphs: Zebra 123</p></article><input value=edited>".to_vec();
            source
        };
        let (first, shared) = app_for_test(&ctx, prepare_page(source()).unwrap());
        let (mut second, _) = app_for_test(&ctx, prepare_page(source()).unwrap());
        second.id = 1;
        let mut browser = OliveApp {
            shared,
            tabs: vec![first, second],
            active: 0,
            next_id: 2,
            focus_address: false,
            reveal_tab: false,
            recovery: None,
            window_title: String::new(),
            devtools: DevTools::default(),
        };
        // Warm every cache, including the views that will be hidden at the toggle.
        for tab in &mut browser.tabs {
            for focus in [false, true] {
                tab.focus_mode = focus;
                chrome_frame(&ctx, tab, &mut browser.shared, 800.0, vec![]);
            }
        }
        let before: Vec<_> = browser
            .tabs
            .iter()
            .map(|tab| (tab.saved_tab(), serde_json::to_value(&tab.loaded).unwrap()))
            .collect();
        for theme in [Theme::Dark, Theme::Light, Theme::Dark] {
            browser.shared.profile.data.settings.browser_theme = theme;
            browser.apply_browser_theme(&ctx);
            for tab in &mut browser.tabs {
                for focus in [false, true] {
                    tab.focus_mode = focus;
                    let output = chrome_frame(&ctx, tab, &mut browser.shared, 800.0, vec![]);
                    let text = output
                        .shapes
                        .iter()
                        .find_map(|shape| match &shape.shape {
                            egui::Shape::Text(text)
                                if text.galley.text() == "Page glyphs: Zebra 123" =>
                            {
                                Some(text)
                            }
                            _ => None,
                        })
                        .expect("page text remains painted");
                    let fresh = ctx.fonts_mut(|fonts| fonts.layout_job((*text.galley.job).clone()));
                    assert_eq!(
                        text.galley.as_ref(),
                        fresh.as_ref(),
                        "glyphs must reference the current font atlas"
                    );
                }
            }
        }
        for (tab, (saved, loaded)) in browser.tabs.iter().zip(before) {
            assert_eq!(tab.saved_tab(), saved);
            assert_eq!(serde_json::to_value(&tab.loaded).unwrap(), loaded);
            assert_eq!(tab.generation, 1);
            assert!(tab.pending.is_none());
        }
        assert_eq!(browser.shared.browsing_history.entries()[0].visits, 1);
    }

    #[test]
    fn loader_accepts_an_html_file_and_reports_read_errors() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let loaded = load_file(root.join("examples/hello.html")).unwrap();
        assert_eq!(loaded.page.title, "Olive Browser");
        assert!(!loaded.page.is_empty());
        let image_page = load_file(root.join("examples/reading.html")).unwrap();
        assert_eq!(image_page.page.image_count(), 1);
        assert_eq!(image_page.reading.image_count(), 1);
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
    fn tabs_keep_independent_state_and_closing_preserves_selection() {
        let ctx = egui::Context::default();
        let (mut first, shared) = app_for_test(
            &ctx,
            prepare_page(remote_source("https://example.com/first")).unwrap(),
        );
        first.address = "unfinished address".into();
        first.focus_mode = true;
        first.focus_settings.font_size = 28.0;
        let mut browser = OliveApp {
            shared,
            tabs: vec![first],
            active: 0,
            next_id: 1,
            focus_address: false,
            reveal_tab: false,
            recovery: None,
            window_title: String::new(),
            devtools: DevTools::default(),
        };
        browser.new_tab(None, &ctx);
        let second_id = browser.tabs[1].id;
        assert_eq!(browser.active, 1);
        assert!(browser.tabs[1].address.is_empty());
        assert!(browser.tabs[1].history.back().is_none());
        assert!(!browser.tabs[1].focus_mode);
        browser.tabs[1].address = "second draft".into();
        browser.select(0);
        assert_eq!(browser.tabs[0].address, "unfinished address");
        assert!(browser.tabs[0].focus_mode);
        assert_eq!(browser.tabs[0].focus_settings.font_size, 28.0);
        browser.new_tab(None, &ctx);
        browser.select(1);
        browser.close_tab(0, &ctx);
        assert_eq!(browser.active, 0);
        assert_eq!(browser.tabs[0].id, second_id);
        assert_eq!(browser.tabs[0].address, "second draft");
        browser.close_tab(1, &ctx);
        browser.close_tab(0, &ctx);
        assert_eq!(browser.tabs.len(), 1);
        assert_eq!(browser.active, 0);
        assert!(browser.tabs[0].loaded.is_none());
        assert!(browser.tabs[0].id > second_id);
    }

    #[test]
    fn session_checkpoint_keeps_a_navigation_that_has_not_loaded_yet() {
        let mut tab = Tab::new(7);
        let location = Location::from_input("https://example.com/pending").unwrap();
        tab.last_requested = Some(location.clone());
        assert_eq!(tab.saved_tab().url.as_deref(), Some(location.as_str()));

        let loaded = prepare_page(remote_source("https://example.com/old")).unwrap();
        let destination = Location::from_input("https://example.com/new").unwrap();
        let (worker, _events) = Worker::mock();
        tab.loaded = Some(loaded);
        tab.pending = Some(PendingPage {
            viewport: None,
            worker,
            navigation: Navigation::New,
            requested: destination.clone(),
        });
        assert_eq!(tab.saved_tab().url.as_deref(), Some(destination.as_str()));

        let deferred = Location::from_input("https://example.com/background").unwrap();
        tab.pending = None;
        tab.loaded = None;
        tab.last_requested = None;
        tab.deferred = Some(deferred.clone());
        assert_eq!(tab.saved_tab().url.as_deref(), Some(deferred.as_str()));
    }

    #[test]
    fn clean_session_restore_uses_current_setting_but_crash_restore_disables_scripts() {
        let ctx = egui::Context::default();
        let (tab, shared) = app_for_test(
            &ctx,
            prepare_page(remote_source("https://example.com/story")).unwrap(),
        );
        let mut browser = OliveApp {
            shared,
            tabs: vec![tab],
            active: 0,
            next_id: 1,
            focus_address: false,
            reveal_tab: false,
            recovery: None,
            window_title: String::new(),
            devtools: DevTools::default(),
        };
        let session = Session {
            tabs: vec![SavedTab {
                url: Some("https://example.com/restored".into()),
                zoom: 100,
                focus: FocusSettings::default(),
            }],
            active: 0,
        };
        browser.restore_session(session.clone(), true);
        assert!(!browser.tabs[0].restored_scripting(&browser.shared));
        browser.shared.profile.data.settings.javascript_by_default = true;
        assert!(browser.tabs[0].restored_scripting(&browser.shared));
        browser.shared.profile.data.settings.javascript_by_default = false;
        assert!(!browser.tabs[0].restored_scripting(&browser.shared));
        browser.shared.profile.data.settings.javascript_by_default = true;
        browser.restore_session(session, false);
        assert!(!browser.tabs[0].restored_scripting(&browser.shared));
    }

    #[test]
    fn pages_fill_the_window_beneath_the_glass_frame() {
        let ctx = egui::Context::default();
        let mut source = remote_source("https://example.com/story");
        source.bytes = b"<p>Beneath the glass</p>".to_vec();
        let (mut tab, mut shared) = app_for_test(&ctx, prepare_page(source).unwrap());
        chrome_frame(&ctx, &mut tab, &mut shared, 800.0, vec![]);
        let output = chrome_frame(&ctx, &mut tab, &mut shared, 800.0, vec![]);
        let text = |wanted: &str| {
            output.shapes.iter().position(
                |shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == wanted),
            )
        };
        let page = text("Beneath the glass").unwrap();
        let view = shared.page_view.get().unwrap();
        // The page scrolls between the bands and paints beneath both.
        let clip = output.shapes[page].clip_rect;
        assert!(clip.top() < view.top() && clip.bottom() > view.bottom());
        assert!(view.top() > 50.0 && view.bottom() < 600.0);
        assert_eq!(
            shared.bands(ctx.content_rect()),
            [
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(800.0, view.top())),
                egui::Rect::from_min_max(egui::pos2(0.0, view.bottom()), egui::pos2(800.0, 600.0)),
            ]
        );
        // The frame paints above it.
        assert!(text("https://example.com/story").unwrap() > page);
    }

    #[test]
    fn loads_carry_the_page_area_as_a_css_viewport() {
        let ctx = egui::Context::default();
        let (mut tab, mut shared) = app_for_test(
            &ctx,
            prepare_page(remote_source("https://example.com/first")).unwrap(),
        );
        assert_eq!(shared.viewport(100), None);
        // The new-tab page predicts the width that a page then measures.
        let mut blank = Tab::new(1);
        chrome_frame(&ctx, &mut blank, &mut shared, 1200.0, vec![]);
        let predicted = shared.page_area.get().unwrap();
        shared.page_area.set(None);
        chrome_frame(&ctx, &mut tab, &mut shared, 1200.0, vec![]);
        assert_eq!(shared.page_area.get().unwrap().x, predicted.x);
        shared.page_area.set(Some(egui::vec2(1000.0, 700.4)));
        assert_eq!(shared.viewport(100), Some((1000, 700)));
        assert_eq!(shared.viewport(125), Some((800, 560)));
        // The page arrives laid out for that viewport, so no resize follows.
        let page = prepare_page(remote_source("https://example.com/second")).unwrap();
        let (worker, sender) = Worker::mock_rendering(&page);
        assert!(sender.send(Ok(Event::Loaded(Box::new(page)))).is_ok());
        tab.pending = Some(PendingPage {
            viewport: shared.viewport(125),
            worker,
            requested: Location::from_input("https://example.com/second").unwrap(),
            navigation: Navigation::New,
        });
        tab.receive(&ctx, &mut shared);
        assert_eq!(tab.css_viewport, (800, 560));
    }

    #[test]
    fn styled_pages_show_while_images_load_and_the_tab_keeps_loading() {
        let ctx = egui::Context::default();
        let (mut tab, mut shared) = app_for_test(
            &ctx,
            prepare_page(remote_source("https://example.com/story")).unwrap(),
        );
        tab.loaded.as_mut().unwrap().resources.pending = true;
        let (mut worker, sender) = Worker::mock_rendering(tab.loaded.as_ref().unwrap());
        worker.await_resource_update();
        tab.worker = Some(worker);
        let update = |html: &[u8], pending: bool| {
            let mut source = remote_source("https://example.com/story");
            source.bytes = html.to_vec();
            let mut update = prepare_page(source).unwrap().into_update();
            update.resources.pending = pending;
            Ok(Event::ResourcesUpdated(Box::new(update)))
        };
        assert!(sender.send(update(b"<title>Styled</title>", true)).is_ok());
        tab.receive(&ctx, &mut shared);
        let loaded = tab.loaded.as_ref().unwrap();
        assert_eq!(loaded.page.title, "Styled");
        assert!(loaded.resources.pending);
        assert!(tab.working(), "the load's deadline still applies");
        assert!(
            sender
                .send(update(b"<title>Complete</title>", false))
                .is_ok()
        );
        tab.receive(&ctx, &mut shared);
        let loaded = tab.loaded.as_ref().unwrap();
        assert_eq!(loaded.page.title, "Complete");
        assert!(!loaded.resources.pending);
        assert!(tab.worker.is_none());
        assert!(!tab.working());
    }

    #[test]
    fn background_completion_and_failure_are_routed_to_their_own_tab() {
        let ctx = egui::Context::default();
        let (mut first, mut shared) = app_for_test(
            &ctx,
            prepare_page(remote_source("https://example.com/first")).unwrap(),
        );
        let mut second = Tab::new(1);
        let page = prepare_page(remote_source("https://example.com/second")).unwrap();
        let (worker, sender) = Worker::mock_rendering(&page);
        assert!(sender.send(Ok(Event::Loaded(Box::new(page)))).is_ok());
        second.pending = Some(PendingPage {
            viewport: None,
            worker,
            requested: Location::from_input("https://example.com/second").unwrap(),
            navigation: Navigation::New,
        });
        second.receive(&ctx, &mut shared);
        assert_eq!(first.address, "https://example.com/first");
        assert_eq!(second.address, "https://example.com/second");
        assert_eq!(shared.browsing_history.entries().len(), 2);
        let (worker, failed) = Worker::mock();
        first.worker = Some(worker);
        assert!(failed.send(Err("Simulated process crash".into())).is_ok());
        first.receive(&ctx, &mut shared);
        assert!(first.crashed);
        assert!(first.worker.is_none());
        assert!(
            second
                .loaded
                .as_ref()
                .is_some_and(|page| !page.scripting_enabled)
        );
        assert!(second.worker.is_none());
        assert!(!second.crashed);
        assert!(second.error.is_none());
        second.receive(&ctx, &mut shared);
        assert!(second.error.is_none());
    }

    #[test]
    fn tab_strip_and_page_fit_a_small_window_and_scroll_state_is_tab_local() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::gui::fonts::definitions());
        let (first, shared) = app_for_test(
            &ctx,
            prepare_page(remote_source("https://example.com/first")).unwrap(),
        );
        let mut browser = OliveApp {
            shared,
            tabs: vec![first],
            active: 0,
            next_id: 1,
            focus_address: false,
            reveal_tab: false,
            recovery: None,
            window_title: String::new(),
            devtools: DevTools::default(),
        };
        for _ in 0..8 {
            browser.new_tab(None, &ctx);
        }
        browser.select(0);
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(480.0, 360.0),
                )),
                ..Default::default()
            },
            |ui| {
                browser.tab_bar(ui);
                assert!(ui.available_height() >= 300.0);
                let _ = browser.tabs[0].ui(ui, &mut browser.shared, false, &mut false);
            },
        );
        assert!(!output.shapes.is_empty());
        output.textures_delta.clear();
        let scroll_id = |tab: &Tab| egui::Id::unique(("document", tab.id, tab.generation, false));
        assert_ne!(scroll_id(&browser.tabs[0]), scroll_id(&browser.tabs[1]));
    }
    #[test]
    fn find_shortcuts_keep_focus_mode_and_tab_state_independent() {
        let ctx = egui::Context::default();
        let mut source = remote_source("https://example.com/story");
        source.bytes =
            b"<p>Olive trees. Olive oil.</p><form><input name=q value=original></form>".to_vec();
        let (mut tab, mut shared) = app_for_test(&ctx, prepare_page(source).unwrap());
        tab.focus_mode = true;
        tab.find.query = "olive".into();
        let mut press = |tab: &mut Tab, key, modifiers| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(480.0, 600.0),
                    )),
                    events: vec![
                        egui::Event::ModifiersChanged(modifiers),
                        egui::Event::Key {
                            key,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers,
                        },
                    ],
                    ..Default::default()
                },
                |ui| {
                    show_tab(ui, tab, &mut shared);
                },
            );
            output.textures_delta.clear();
        };
        press(&mut tab, egui::Key::F, egui::Modifiers::COMMAND);
        assert!(tab.find.open && tab.focus_mode);
        assert_eq!(tab.loaded.as_ref().unwrap().reading.find.count, 2);
        press(&mut tab, egui::Key::G, egui::Modifiers::COMMAND);
        assert_eq!(tab.find.current, 1);
        press(&mut tab, egui::Key::Escape, egui::Modifiers::NONE);
        assert!(!tab.find.open && tab.focus_mode);
        let other = Tab::new(9);
        assert!(!other.find.open && other.find.query.is_empty());
        assert_eq!(
            tab.loaded.as_ref().unwrap().page.forms.controls[0].value,
            "original"
        );
        assert_eq!(shared.browsing_history.entries()[0].visits, 1);
    }
}
