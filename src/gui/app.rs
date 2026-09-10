use crate::{
    document::{ClickRequest, LoadedPage},
    find::FindBar,
    focus::{MAX_FONT_SIZE, MIN_FONT_SIZE, Settings as FocusSettings, Theme as FocusTheme},
    forms::{Activation, Kind},
    history::BrowsingHistory,
    history_ui::HistoryWindow,
    icon,
    navigation::{History, Navigation},
    render::{INK, ImageTextureCache, OLIVE},
    ui_icons::{self, Icon, IconButton},
    worker::{Command, Event, Worker},
};
use eframe::egui::{self, Color32, RichText};
use olive_html::net::Location;
use std::{ffi::OsString, path::PathBuf};

const PAPER: Color32 = Color32::from_rgb(250, 250, 246);
const CHROME: Color32 = Color32::from_rgb(239, 242, 231);
struct PendingPage {
    worker: Worker,
    navigation: Navigation,
    requested: Location,
}

struct Shared {
    icon: egui::TextureHandle,
    browsing_history: BrowsingHistory,
    history_window: HistoryWindow,
}

pub struct Tab {
    id: u64,
    worker: Option<Worker>,
    crashed: bool,
    loaded: Option<LoadedPage>,
    pending: Option<PendingPage>,
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
        ui.style_mut().visuals = self.focus_settings.theme.visuals();
        egui::Panel::top("focus_toolbar")
            .exact_size(64.0)
            .frame(
                egui::Frame::new()
                    .fill(self.focus_settings.theme.paper())
                    .inner_margin(12),
            )
            .show(ui, |ui| {
                ui.spacing_mut().button_padding = egui::vec2(12.0, 9.0);
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
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let mut appearance = IconButton::new(Icon::Appearance, "Appearance");
                        appearance.button = appearance.button.selected(self.focus_panel);
                        if ui
                            .add(appearance)
                            .on_hover_text("Show or hide reading settings")
                            .clicked()
                        {
                            self.focus_panel = !self.focus_panel;
                        }
                    });
                });
            });
        if self.focus_panel {
            egui::Panel::top("focus_appearance")
                .frame(
                    egui::Frame::new()
                        .fill(self.focus_settings.theme.paper())
                        .inner_margin(egui::Margin::symmetric(20, 12)),
                )
                .show(ui, |ui| {
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
                            FocusTheme::Light,
                            "Light",
                        );
                        ui.selectable_value(
                            &mut self.focus_settings.theme,
                            FocusTheme::Dark,
                            "Dark",
                        );
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
        }
    }

    fn activate_form(&mut self, activation: Activation, ctx: &egui::Context) {
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
                match Worker::spawn_form(request, ctx) {
                    Ok(worker) => {
                        self.address = requested.as_str().into();
                        self.error = None;
                        self.alert = None;
                        // Forms always navigate, even if the action is the current address.
                        self.pending = Some(PendingPage {
                            worker,
                            navigation: Navigation::New,
                            requested,
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
        let scripting = !location.is_remote()
            || (matches!(navigation, Navigation::Reload)
                && self.loaded.as_ref().is_some_and(|loaded| {
                    loaded.scripting_enabled && loaded.location.same_document(&location)
                }));
        self.open_with_scripts(location, navigation, ctx, scripting, shared);
    }

    fn open_with_scripts(
        &mut self,
        location: Location,
        navigation: Navigation,
        ctx: &egui::Context,
        scripting: bool,
        shared: &mut Shared,
    ) {
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
        match Worker::spawn(location, scripting, ctx) {
            Ok(worker) => {
                self.pending = Some(PendingPage {
                    worker,
                    navigation,
                    requested,
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
                    self.worker = Some(pending.worker);
                    self.crashed = false;
                    self.error = None;
                    self.focus_mode = false;
                    self.pending_form = None;
                    self.find.current = 0;
                    self.find.scroll = self.find.open;
                    self.image_textures.clear();
                    self.generation = self.generation.wrapping_add(1);
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
            Some(Ok(Event::Updated(mut update))) => {
                let activation = self.pending_form.take().filter(|_| update.default_allowed);
                if let Some(loaded) = &mut self.loaded {
                    update.page.forms.preserve_edits(&loaded.page.forms);
                    loaded.page = update.page;
                    self.find.scroll = self.find.open;
                    loaded.reading = update.reading;
                    loaded.base = update.base;
                    loaded.scripts = update.scripts;
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
                    self.activate_form(activation, ctx);
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
        } else if self.worker.as_ref().is_some_and(Worker::busy) {
            self.worker = None;
            self.crashed = true;
            self.error = Some("This tab was stopped. Reload to restart it.".into());
        }
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

fn open_button(ui: &mut egui::Ui, enabled: bool) -> bool {
    let mut button = IconButton::new(Icon::Folder, "Open HTML…").primary();
    button.button = button.button.min_size(egui::vec2(152.0, 40.0));
    ui.add_enabled(enabled, button).clicked()
}

impl Tab {
    fn ui(&mut self, ui: &mut egui::Ui, shared: &mut Shared, autofocus: bool) -> Option<Location> {
        // Panels share the root UI; leave no unpainted gap between them.
        ui.spacing_mut().item_spacing.y = 0.0;
        let ctx = ui.ctx().clone();
        if ctx.input_mut(|input| {
            input.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND,
                egui::Key::O,
            ))
        }) {
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
        let mut choose = false;
        let shortcut = |modifiers, key| {
            ctx.input_mut(|input| {
                input.consume_shortcut(&egui::KeyboardShortcut::new(modifiers, key))
            })
        };
        let focus_address = autofocus | shortcut(egui::Modifiers::COMMAND, egui::Key::L);
        let find_escape = self.find.open && shortcut(egui::Modifiers::NONE, egui::Key::Escape);
        if find_escape {
            self.find.open = false;
        }
        let previous_focus_mode = self.focus_mode;
        if self.focus_mode && (shortcut(egui::Modifiers::NONE, egui::Key::Escape) || focus_address)
        {
            self.focus_mode = false;
        }
        if shortcut(
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            egui::Key::F,
        ) {
            self.toggle_focus(shared);
        }
        if shortcut(egui::Modifiers::COMMAND, egui::Key::F) {
            self.find.open = true;
            self.find.focus = true;
            self.find.scroll = true;
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
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::G,
            ) || shortcut(egui::Modifiers::SHIFT, egui::Key::F3)
            {
                self.find.step(find_count, true);
            } else if shortcut(egui::Modifiers::COMMAND, egui::Key::G)
                || shortcut(egui::Modifiers::NONE, egui::Key::F3)
            {
                self.find.step(find_count, false);
            }
        }
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
        let mut form_activation = None;
        let mut link_new_tab = false;
        let mut stop = false;
        let busy = self.pending.is_some();
        if !self.focus_mode {
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
                        let compact = ui.available_width() < 760.0;
                        ui.add(
                            egui::Image::new(&shared.icon)
                                .fit_to_exact_size(egui::vec2(30.0, 30.0)),
                        )
                        .on_hover_text("Olive Browser");
                        back |= ui
                            .add_enabled(
                                self.history.back().is_some(),
                                IconButton::icon_only(Icon::Back, "Back"),
                            )
                            .on_hover_text("Back (Alt+Left)")
                            .clicked();
                        forward |= ui
                            .add_enabled(
                                self.history.forward().is_some(),
                                IconButton::icon_only(Icon::Forward, "Forward"),
                            )
                            .on_hover_text("Forward (Alt+Right)")
                            .clicked();
                        let working = busy || self.worker.as_ref().is_some_and(Worker::busy);
                        if working {
                            stop |= ui
                                .add(IconButton::icon_only(Icon::Close, "Stop tab"))
                                .on_hover_text("Stop this tab (Escape)")
                                .clicked();
                        } else {
                            reload |= ui
                                .add_enabled(
                                    self.loaded.is_some() || !self.address.is_empty(),
                                    IconButton::icon_only(Icon::Reload, "Reload"),
                                )
                                .on_hover_text("Reload (Cmd/Ctrl+R or F5)")
                                .clicked();
                        }
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
                        let trailing_width = 4.0 * 36.0
                            + 4.0 * 6.0
                            + if compact {
                                0.0
                            } else {
                                label_width("Open…")
                                    + label_width("History")
                                    + label_width("Focus")
                                    + 3.0 * 7.0
                            };
                        let address_width = (ui.available_width() - trailing_width).max(60.0);
                        let mut output = egui::TextEdit::singleline(&mut self.address)
                            .id(egui::Id::new(("address", self.id)))
                            .hint_text("Enter a URL or file path")
                            .char_limit(olive_html::net::MAX_URL_BYTES)
                            .desired_width(address_width)
                            .margin(egui::vec2(10.0, 10.0))
                            .show(ui);
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
                        go |= output.response.lost_focus()
                            && ui.input(|input| input.key_pressed(egui::Key::Enter));
                        go |= ui
                            .add(IconButton::icon_only(Icon::Go, "Go").primary())
                            .on_hover_text("Open address")
                            .clicked();
                        let readable = self
                            .loaded
                            .as_ref()
                            .is_some_and(|loaded| !loaded.reading.is_empty());
                        if ui
                            .add_enabled(
                                !busy && readable,
                                if compact {
                                    IconButton::icon_only(Icon::Focus, "Focus mode")
                                } else {
                                    IconButton::new(Icon::Focus, "Focus")
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

                        choose |= ui
                            .add_enabled(
                                true,
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
                                .warning(shared.browsing_history.error().is_some()),
                            )
                            .on_hover_text(if shared.browsing_history.error().is_some() {
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
                        if ui.add(IconButton::new(Icon::Search, "Find").small()).on_hover_text("Find in page (Cmd/Ctrl+F)").clicked() {
                            self.find.open = true; self.find.focus = true; self.find.scroll = true;
                        }
                        if loaded.location.is_remote() {
                            toggle_scripts = ui.add_enabled(!busy, IconButton::new(
                                if loaded.scripting_enabled { Icon::CodeOff } else { Icon::Code },
                                if loaded.scripting_enabled { "Disable JavaScript" } else { "Enable JavaScript" }
                            ).small()).on_hover_text("Reload this page with JavaScript enabled or disabled. Enable only for pages you trust: scripts run in this tab's process without an OS security sandbox. New addresses start with web JavaScript disabled.").clicked();
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
        } else {
            self.focus_toolbar(ui, busy);
        }
        if self.find.open {
            ui.push_id(self.id, |ui| self.find.show(ui, find_count));
        }
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new().fill(
                    if self.focus_mode { self.focus_settings.theme.paper() } else {
                        self.loaded.as_ref().and_then(|loaded| loaded.page.background).unwrap_or(PAPER)
                    },
                ),
            )
            .show(ui, |ui| {
                if self.focus_mode {
                    ui.style_mut().visuals = self.focus_settings.theme.visuals();
                }
                if let Some(error) = &self.error {
                    egui::Frame::new()
                        .fill(Color32::from_rgb(255, 235, 228))
                        .inner_margin(16)
                        .show(ui, |ui| {
                            ui.visuals_mut().override_text_color = Some(INK);
                            ui.label(
                                RichText::new(if self.crashed { "This tab stopped" } else { "Could not open this page" })
                                    .variations([("wght", 650.0)]),
                            );
                            ui.label(error);
                            if ui.add(IconButton::new(Icon::Reload, "Reload tab")).clicked() { reload = true; }
                        });
                }
                if let Some(loaded) = &mut self.loaded {
                    let focus = self.focus_mode;
                    let page = if focus { &mut loaded.reading } else { &mut loaded.page };
                    if focus { page.set_reading_style(self.focus_settings); }
                    let count = page.find_query(if self.find.open { &self.find.query } else { "" });
                    self.find.current = self.find.current.min(count.saturating_sub(1));
                    page.find_current = self.find.current;
                    page.find_scroll |= std::mem::take(&mut self.find.scroll) && count > 0;
                    egui::ScrollArea::both()
                        .id_salt(("document", self.id, self.generation, focus))
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            let column = if focus { 720.0 } else { 820.0 };
                            let margin = ((ui.available_width() - column) / 2.0).max(24.0);
                            // Keep the column centered beyond the integer frame-margin limit.
                            let inset = if focus { (margin - 100.0).max(0.0) } else { 0.0 };
                            let mut rect = ui.available_rect_before_wrap();
                            rect.min.x += inset;
                            rect.max.x -= inset;
                            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                            egui::Frame::new()
                                .inner_margin(egui::Margin {
                                    left: margin.min(100.0) as i8,
                                    right: margin.min(100.0) as i8,
                                    top: 34,
                                    bottom: 40,
                                })
                                .show(ui, |ui| {
                                    if page.is_empty() {
                                        ui.label("This document has no visible content.");
                                    }
                                    if let Some(click) =
                                        ui.push_id((self.id, self.generation), |ui| {
                                            ui.add_enabled_ui(!busy && !self.crashed && !self.worker.as_ref().is_some_and(Worker::busy), |ui| page.show_with_textures(ui, &mut self.image_textures)).inner
                                        }).inner
                                    {
                                        link = click.href;
                                        link_new_tab = click.new_tab;
                                        form_activation = click.form;
                                        if !click.new_tab && !busy && !self.crashed {
                                            if let (Some(target), Some(worker)) = (click.target, self.worker.as_mut()) {
                                                if !worker.busy() {
                                                    self.pending_form = form_activation.take();
                                                    if let Err(error) = worker.send(Command::Click(ClickRequest { target, href: link.take() })) {
                                                        self.error = Some(error);
                                                        self.pending_form = None;
                                                    }
                                                }
                                                link = None;
                                            }
                                        }

                                    }
                                    if focus && page.truncated {
                                        ui.label("This reading view was shortened to keep the page responsive.");
                                    }
                                });
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
                        choose |= open_button(ui, true);
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
                .id(egui::Id::new(("alert", self.id)))
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
            shared.history_window.toggle();
        }
        let history_location =
            shared
                .history_window
                .show(&ctx, &mut shared.browsing_history, false);
        stop |=
            !find_escape && !self.focus_mode && shortcut(egui::Modifiers::NONE, egui::Key::Escape);
        if stop {
            self.stop();
        }
        if choose {
            self.choose_file(&ctx, shared);
        } else {
            if let Some(location) = history_location {
                self.open(location, Navigation::New, &ctx, shared);
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
                self.open_result(location, navigation, &ctx, shared);
            } else if back {
                if let Some((index, location)) = self.history.back() {
                    self.open(location, Navigation::Traverse(index), &ctx, shared);
                }
            } else if forward {
                if let Some((index, location)) = self.history.forward() {
                    self.open(location, Navigation::Traverse(index), &ctx, shared);
                }
            } else if toggle_scripts {
                if let Some(loaded) = &self.loaded {
                    self.open_with_scripts(
                        loaded.location.clone(),
                        Navigation::Reload,
                        &ctx,
                        !loaded.scripting_enabled,
                        shared,
                    );
                }
            } else if reload {
                if let Some(pending) = &self.pending {
                    self.open(pending.requested.clone(), pending.navigation, &ctx, shared);
                } else if let Some(loaded) = &self.loaded {
                    self.open(loaded.location.clone(), Navigation::Reload, &ctx, shared);
                } else {
                    self.open_result(
                        Location::from_input(&self.address),
                        Navigation::New,
                        &ctx,
                        shared,
                    );
                }
            } else if let Some(activation) = form_activation {
                self.activate_form(activation, &ctx);
            } else if let (Some(href), Some(loaded)) = (link, &self.loaded) {
                let location = loaded.base.resolve(&href);
                if link_new_tab {
                    match location {
                        Ok(location) => return Some(location),
                        Err(error) => self.error = Some(error),
                    }
                } else {
                    self.open_result(location, Navigation::New, &ctx, shared);
                }
            }
        }
        None
    }
}

const MAX_TABS: usize = 32;

pub struct OliveApp {
    shared: Shared,
    tabs: Vec<Tab>,
    active: usize,
    next_id: u64,
    focus_address: bool,
    reveal_tab: bool,
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
            shared: Shared {
                icon,
                browsing_history: BrowsingHistory::load_default(),
                history_window: HistoryWindow::default(),
            },
            tabs: vec![Tab::new(0)],
            active: 0,
            next_id: 1,
            focus_address: source.is_none(),
            reveal_tab: false,
        };
        if let Some(source) = source {
            let location = match source.to_str() {
                Some(input) => Location::from_input(input),
                None => Location::from_path(PathBuf::from(source)),
            };
            app.tabs[0].open_result(location, Navigation::New, ctx, &mut app.shared);
        }
        app
    }

    fn new_tab(&mut self, location: Option<Location>, ctx: &egui::Context) {
        if self.tabs.len() >= MAX_TABS {
            self.tabs[self.active].error =
                Some("Close a tab before opening another (32 tab limit).".into());
            return;
        }
        let mut tab = Tab::new(self.next_id);
        self.next_id += 1;
        self.focus_address = location.is_none();
        if let Some(location) = location {
            tab.open(location, Navigation::New, ctx, &mut self.shared);
        }
        self.tabs.push(tab);
        self.select(self.tabs.len() - 1);
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
        let mut selected = None;
        let mut close = None;
        let mut add = false;
        egui::Panel::top("tabs")
            .exact_size(48.0)
            .frame(
                egui::Frame::new()
                    .fill(CHROME)
                    .inner_margin(egui::Margin::symmetric(10, 6)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let width = (ui.available_width() - 44.0).max(100.0);
                    egui::ScrollArea::horizontal()
                        .id_salt("tab-strip")
                        .max_width(width)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                for (index, tab) in self.tabs.iter().enumerate() {
                                    ui.push_id(tab.id, |ui| {
                                        ui.spacing_mut().item_spacing.x = 0.0;
                                        let busy = tab.pending.is_some()
                                            || tab.worker.as_ref().is_some_and(Worker::busy);
                                        let title: String = tab.title().chars().take(80).collect();
                                        let label = if busy {
                                            format!("Opening… {title}")
                                        } else {
                                            title
                                        };
                                        let mut button = IconButton::new(
                                            if tab.crashed || tab.error.is_some() {
                                                Icon::Warning
                                            } else {
                                                Icon::Page
                                            },
                                            label,
                                        );
                                        button.button = button
                                            .button
                                            .selected(index == self.active)
                                            .truncate()
                                            .min_size(egui::vec2(140.0, 32.0));
                                        let response = ui
                                            .add_sized([180.0, 32.0], button)
                                            .on_hover_text(format!(
                                                "{}\n{}",
                                                tab.title(),
                                                tab.worker
                                                    .as_ref()
                                                    .or_else(|| tab
                                                        .pending
                                                        .as_ref()
                                                        .map(|p| &p.worker))
                                                    .and_then(Worker::pid)
                                                    .map(|pid| format!("Tab process {pid}"))
                                                    .unwrap_or_else(|| "New tab".into())
                                            ));
                                        if self.reveal_tab && index == self.active {
                                            response.scroll_to_me(Some(egui::Align::Center));
                                        }
                                        if response.clicked() {
                                            selected = Some(index);
                                        }
                                        if response.clicked_by(egui::PointerButton::Middle) {
                                            close = Some(index);
                                        }
                                        if ui
                                            .add(
                                                IconButton::icon_only(Icon::Close, "Close tab")
                                                    .small(),
                                            )
                                            .on_hover_text("Close tab (Cmd/Ctrl+W)")
                                            .clicked()
                                        {
                                            close = Some(index);
                                        }
                                    });
                                }
                            });
                        });
                    add = ui
                        .add_enabled(
                            self.tabs.len() < MAX_TABS,
                            IconButton::icon_only(Icon::Plus, "New tab"),
                        )
                        .on_hover_text("New tab (Cmd/Ctrl+T)")
                        .on_disabled_hover_text("Close a tab to open another (32 tab limit)")
                        .clicked();
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
impl eframe::App for OliveApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        PAPER.to_normalized_gamma_f32()
    }
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        for tab in &mut self.tabs {
            tab.receive(ctx, &mut self.shared);
        }
        if self
            .tabs
            .iter()
            .any(|tab| tab.worker.is_some() || tab.pending.is_some())
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let shortcut = |modifiers, key| {
            ctx.input_mut(|input| {
                input.consume_shortcut(&egui::KeyboardShortcut::new(modifiers, key))
            })
        };
        if shortcut(egui::Modifiers::COMMAND, egui::Key::T) {
            self.new_tab(None, &ctx);
        }
        if shortcut(egui::Modifiers::COMMAND, egui::Key::W) {
            self.close_tab(self.active, &ctx);
        }
        if shortcut(
            egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
            egui::Key::Tab,
        ) {
            self.select((self.active + self.tabs.len() - 1) % self.tabs.len());
        } else if shortcut(egui::Modifiers::CTRL, egui::Key::Tab) {
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
            if shortcut(egui::Modifiers::COMMAND, key) {
                self.select(if index == 8 {
                    self.tabs.len() - 1
                } else {
                    index
                });
            }
        }
        ui.spacing_mut().item_spacing.y = 0.0;
        self.tab_bar(ui);
        let autofocus = std::mem::take(&mut self.focus_address);
        if let Some(location) = self.tabs[self.active].ui(ui, &mut self.shared, autofocus) {
            self.new_tab(Some(location), &ctx);
        }
        let title: String = self.tabs[self.active].title().chars().take(200).collect();
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
            "{title} — Olive Browser"
        )));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::prepare_page_with_loader;
    use olive_html::net::{DocumentLoader, LoadedDocument};

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

    #[test]
    fn focus_toggles_without_reloading_or_recording_visits_and_keeps_preferences() {
        let ctx = egui::Context::default();
        let (mut app, mut shared) = app_for_test(
            &ctx,
            prepare_page(remote_source("https://example.com/story")).unwrap(),
        );
        app.focus_settings = FocusSettings {
            font_size: 25.0,
            theme: FocusTheme::Dark,
        };
        app.toggle_focus(&mut shared);
        assert!(app.focus_mode && app.focus_panel);
        app.focus_panel = false;
        app.toggle_focus(&mut shared);
        assert!(!app.focus_mode);
        app.toggle_focus(&mut shared);
        assert!(app.focus_mode && app.focus_panel);
        assert_eq!(app.focus_settings.font_size, 25.0);
        assert_eq!(app.focus_settings.theme, FocusTheme::Dark);
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
        ctx.set_fonts(crate::fonts::definitions());
        let (mut app, _shared) = app_for_test(
            &ctx,
            prepare_page(remote_source("https://example.com/story")).unwrap(),
        );
        for theme in [FocusTheme::Light, FocusTheme::Dark] {
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
        let (worker, sender) = Worker::mock();
        assert!(
            sender
                .send(Ok(Event::Loaded(Box::new(prepare_page(source).unwrap()))))
                .is_ok()
        );
        app.pending = Some(PendingPage {
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
            let (worker, sender) = Worker::mock();
            assert!(
                sender
                    .send(Ok(Event::Loaded(Box::new(
                        prepare_page(remote_source(url)).unwrap()
                    ))))
                    .is_ok()
            );
            app.pending = Some(PendingPage {
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
    fn background_completion_and_failure_are_routed_to_their_own_tab() {
        let ctx = egui::Context::default();
        let (mut first, mut shared) = app_for_test(
            &ctx,
            prepare_page(remote_source("https://example.com/first")).unwrap(),
        );
        let mut second = Tab::new(1);
        let (worker, sender) = Worker::mock();
        assert!(
            sender
                .send(Ok(Event::Loaded(Box::new(
                    prepare_page(remote_source("https://example.com/second")).unwrap()
                ))))
                .is_ok()
        );
        second.pending = Some(PendingPage {
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
        assert!(second.worker.is_some());
        assert!(!second.crashed);
        assert!(second.error.is_none());
        second.receive(&ctx, &mut shared);
        assert!(second.error.is_none());
    }

    #[test]
    fn tab_strip_and_page_fit_a_small_window_and_scroll_state_is_tab_local() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::fonts::definitions());
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
                let _ = browser.tabs[0].ui(ui, &mut browser.shared, false);
            },
        );
        assert!(!output.shapes.is_empty());
        output.textures_delta.clear();
        let scroll_id = |tab: &Tab| egui::Id::new(("document", tab.id, tab.generation, false));
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
                    tab.ui(ui, &mut shared, false);
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
