use crate::{
    daily_ui::DailyWindows,
    document::{ClickRequest, LoadedPage},
    downloads::Downloads,
    find::FindBar,
    focus::{MAX_FONT_SIZE, MIN_FONT_SIZE, Settings as FocusSettings, Theme as FocusTheme},
    forms::{Activation, Kind},
    history::BrowsingHistory,
    history_ui::HistoryWindow,
    icon,
    navigation::{History, Navigation},
    profile::{MAX_ZOOM, MIN_ZOOM, Profile, SavedTab, Session},
    render::ImageTextureCache,
    theme::{DOCUMENT_PAPER, Theme},
    ui_icons::{self, Icon, IconButton},
    worker::{Command, Event, Worker},
    zoom,
};
use eframe::egui::{self, RichText};
use olive_html::net::{CookieJar, Location};
use std::{ffi::OsString, path::PathBuf};

struct PendingPage {
    worker: Worker,
    navigation: Navigation,
    requested: Location,
}

struct Shared {
    icon: egui::TextureHandle,
    browsing_history: BrowsingHistory,
    history_window: HistoryWindow,
    cookies: CookieJar,
    profile: Profile,
    downloads: Downloads,
    daily_windows: DailyWindows,
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
                match Worker::spawn_form_with_cookies(
                    request,
                    shared.cookies.clone(),
                    Some(loaded.location.clone()),
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
        match Worker::spawn_with_cookies(
            location,
            scripting,
            shared.cookies.clone(),
            initiator,
            ctx,
        ) {
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
                shared.cookies.apply_updates(&update.cookie_updates);
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
        let palette = shared.profile.data.settings.browser_theme.palette();
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
        let mut toggle_bookmark = false;
        let mut show_bookmarks = false;
        let mut show_settings = false;
        let mut show_downloads = false;
        let mut download_current = false;
        let mut zoom_in = shortcut(egui::Modifiers::COMMAND, egui::Key::Plus)
            || shortcut(egui::Modifiers::COMMAND, egui::Key::Equals);
        let mut zoom_out = shortcut(egui::Modifiers::COMMAND, egui::Key::Minus);
        let mut zoom_reset = shortcut(egui::Modifiers::COMMAND, egui::Key::Num0);
        let mut link = None;
        let mut form_activation = None;
        let mut link_new_tab = false;
        let mut stop = false;
        let busy = self.pending.is_some();
        if !self.focus_mode {
            egui::Panel::top("toolbar")
                .exact_size(58.0)
                .frame(
                    egui::Frame::new()
                        .fill(palette.chrome)
                        .inner_margin(egui::Margin::symmetric(12, 10)),
                )
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        ui.spacing_mut().button_padding = egui::vec2(8.0, 8.0);
                        let compact = ui.available_width() < 800.0;
                        back |= ui
                            .add_enabled(
                                self.history.back().is_some(),
                                IconButton::icon_only(Icon::Back, "Back").quiet(),
                            )
                            .on_hover_text("Back (Alt+Left)")
                            .clicked();
                        forward |= ui
                            .add_enabled(
                                self.history.forward().is_some(),
                                IconButton::icon_only(Icon::Forward, "Forward").quiet(),
                            )
                            .on_hover_text("Forward (Alt+Right)")
                            .clicked();
                        let working = busy || self.worker.as_ref().is_some_and(Worker::busy);
                        if working {
                            stop |= ui
                                .add(IconButton::icon_only(Icon::Close, "Stop tab").quiet())
                                .on_hover_text("Stop this tab (Escape)")
                                .clicked();
                        } else {
                            reload |= ui
                                .add_enabled(
                                    self.loaded.is_some() || !self.address.is_empty(),
                                    IconButton::icon_only(Icon::Reload, "Reload").quiet(),
                                )
                                .on_hover_text("Reload (Cmd/Ctrl+R or F5)")
                                .clicked();
                        }
                        // Five trailing actions and their gaps. The bookmark slot stays
                        // present on blank tabs to keep the address field stable.
                        let focus_extra = if compact {
                            0.0
                        } else {
                            ui.painter()
                                .layout_no_wrap(
                                    "Focus".into(),
                                    egui::TextStyle::Button.resolve(ui.style()),
                                    palette.ink,
                                )
                                .size()
                                .x
                                + 5.0
                        };
                        let address_width =
                            (ui.available_width() - 5.0 * 42.0 - focus_extra).max(60.0);
                        let mut output = egui::TextEdit::singleline(&mut self.address)
                            .id(egui::Id::new(("address", self.id)))
                            .hint_text("Enter a URL or file path")
                            .char_limit(olive_html::net::MAX_URL_BYTES)
                            .desired_width(address_width - 20.0)
                            .margin(egui::vec2(10.0, 9.0))
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
                        {
                            let marked = self.loaded.as_ref().is_some_and(|loaded| {
                                shared.profile.is_bookmarked(loaded.location.as_str())
                            });
                            let mut bookmark = IconButton::icon_only(
                                Icon::Star,
                                if marked {
                                    "Remove bookmark"
                                } else {
                                    "Add bookmark"
                                },
                            )
                            .quiet();
                            bookmark.button = bookmark.button.selected(marked);
                            toggle_bookmark |= ui
                                .add_enabled(self.loaded.is_some(), bookmark)
                                .on_hover_text(if marked {
                                    "Remove bookmark"
                                } else {
                                    "Add bookmark"
                                })
                                .clicked();
                        }
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

                        let dark = shared.profile.data.settings.browser_theme == Theme::Dark;
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
                            .on_hover_text("Browser menu · files, history, bookmarks and settings");
                        egui::Popup::menu(&menu).show(|ui| {
                            ui.set_min_width(210.0);
                            ui.label(RichText::new("Olive Browser").strong());
                            ui.separator();
                            if ui
                                .add(IconButton::new(Icon::Folder, "Open HTML…").quiet())
                                .on_hover_text("Cmd/Ctrl+O")
                                .clicked()
                            {
                                choose = true;
                                ui.close();
                            }
                            if ui
                                .add(
                                    IconButton::new(Icon::History, "History")
                                        .quiet()
                                        .warning(shared.browsing_history.error().is_some()),
                                )
                                .on_hover_text("Cmd/Ctrl+Shift+H")
                                .clicked()
                            {
                                show_history = true;
                                ui.close();
                            }
                            if ui
                                .add(IconButton::new(Icon::Bookmark, "Bookmarks").quiet())
                                .clicked()
                            {
                                show_bookmarks = true;
                                ui.close();
                            }
                            if ui
                                .add(IconButton::new(Icon::Download, "Downloads").quiet())
                                .clicked()
                            {
                                show_downloads = true;
                                ui.close();
                            }
                            if ui
                                .add_enabled(
                                    self.loaded.is_some(),
                                    IconButton::new(Icon::Download, "Save page…").quiet(),
                                )
                                .clicked()
                            {
                                download_current = true;
                                ui.close();
                            }
                            ui.separator();
                            if ui
                                .add(IconButton::new(Icon::Settings, "Settings").quiet())
                                .clicked()
                            {
                                show_settings = true;
                                ui.close();
                            }
                            ui.label(
                                RichText::new(concat!(
                                    "Version ",
                                    env!("CARGO_PKG_VERSION"),
                                    " · RC 2"
                                ))
                                .small()
                                .weak(),
                            );
                        });
                    });
                });
            egui::Panel::bottom("status")
            .frame(
                egui::Frame::new()
                    .fill(palette.chrome)
                    .inner_margin(egui::Margin::symmetric(12, 6)),
            )
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    ui.spacing_mut().button_padding = egui::vec2(6.0, 3.0);
                    ui.spacing_mut().interact_size.y = 24.0;
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
                    zoom_out |= ui.add(IconButton::icon_only(Icon::Minus, "Zoom out").small()).on_hover_text("Zoom out (Cmd/Ctrl−)").clicked();
                    zoom_reset |= ui.button(format!("{}%", self.zoom)).on_hover_text("Reset page zoom (Cmd/Ctrl+0)").clicked();
                    zoom_in |= ui.add(IconButton::icon_only(Icon::Plus, "Zoom in").small()).on_hover_text("Zoom in (Cmd/Ctrl+)").clicked();
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
                            RichText::new(concat!("v", env!("CARGO_PKG_VERSION"), " · RC 2"))
                                .size(12.0)
                                .weak(),
                        );
                    });
                });
            });
        } else {
            self.focus_toolbar(ui, busy);
        }

        // Apply keyboard shortcuts and status-bar button clicks together, after
        // the controls have had a chance to update their local flags.
        if zoom_in {
            self.zoom = zoom::step(self.zoom, true);
        }
        if zoom_out {
            self.zoom = zoom::step(self.zoom, false);
        }
        if zoom_reset {
            self.zoom = 100;
        }

        if self.find.open {
            ui.push_id(self.id, |ui| self.find.show(ui, find_count));
        }
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new().fill(
                    if self.focus_mode { self.focus_settings.theme.paper() } else {
                        self.loaded.as_ref().map(|loaded| loaded.page.background.unwrap_or(DOCUMENT_PAPER)).unwrap_or(palette.canvas)
                    },
                ),
            )
            .show(ui, |ui| {
                if self.focus_mode {
                    ui.style_mut().visuals = self.focus_settings.theme.visuals();
                }
                if let Some(error) = &self.error {
                    egui::Frame::new()
                        .fill(palette.error_bg)
                        .inner_margin(16)
                        .show(ui, |ui| {
                            ui.visuals_mut().override_text_color = Some(palette.error);
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
                            if !focus {
                                // Native form controls belong to the document, not browser chrome.
                                ui.style_mut().visuals = Theme::Light.visuals();
                            }
                            let column = if focus { 720.0 } else { 820.0 };
                            let margin = ((ui.available_width() - column) / 2.0).max(24.0);
                            // Keep the column centered beyond the integer frame-margin limit.
                            let inset = if focus { (margin - 100.0).max(0.0) } else { 0.0 };
                            let mut rect = ui.available_rect_before_wrap();
                            rect.min.x += inset;
                            rect.max.x -= inset;
                            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                                let mut overflow = egui::Vec2::ZERO;
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
                                    let rendered = ui
                                        .push_id((self.id, self.generation), |ui| {
                                            ui.add_enabled_ui(
                                                !busy
                                                    && !self.crashed
                                                    && !self.worker.as_ref().is_some_and(Worker::busy),
                                                |ui| {
                                                    let page_zoom = self.zoom;
                                                    zoom::show(ui, self.id, page_zoom, |ui| {
                                                        page.show_with_textures(
                                                            ui,
                                                            &mut self.image_textures,
                                                        )
                                                    })
                                                },
                                            )
                                            .inner
                                        })
                                        .inner;
                                    overflow = rendered.overflow;
                                    if let Some(click) = rendered.inner {
                                        link = click.href;
                                        link_new_tab = click.new_tab;
                                        form_activation = click.form;
                                        if !click.new_tab && !busy && !self.crashed {
                                            if let (Some(target), Some(worker)) = (click.target, self.worker.as_mut()) {
                                                if !worker.busy() {
                                                    self.pending_form = form_activation.take();
                                                    if let Err(error) = worker.send(Command::ClickWithCookies { click: ClickRequest { target, href: link.take() }, cookies: shared.cookies.clone() }) {
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
                                // Reserve the portion of the transformed document that extends
                                // beyond the viewport after the frame has painted.
                                ui.allocate_space(overflow);
                            });
                        });
                } else {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(((ui.available_height() - 320.0) * 0.4).max(16.0));
                        ui.add(egui::Image::new(&shared.icon).fit_to_exact_size(egui::vec2(64.0, 64.0)));
                        ui.add_space(16.0);
                        ui.label(
                            RichText::new("A fresh page.")
                                .size(38.0)
                                .color(palette.accent)
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
        if show_bookmarks {
            shared.daily_windows.bookmarks = true;
        }
        if show_settings {
            shared.daily_windows.settings = true;
        }
        if show_downloads {
            shared.downloads.open = true;
        }
        if toggle_bookmark {
            if let Some(loaded) = &self.loaded {
                if let Err(error) = shared
                    .profile
                    .toggle_bookmark(&loaded.location, &loaded.page.title)
                {
                    self.error = Some(error);
                }
            }
        }
        if download_current {
            if let Some(loaded) = &self.loaded {
                shared.downloads.start(
                    loaded.location.clone(),
                    shared.profile.data.settings.download_directory.clone(),
                );
                shared.downloads.open = true;
            }
        }
        let daily_location =
            shared
                .daily_windows
                .show(&ctx, &mut shared.profile, &mut shared.downloads);
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
            if let Some(location) = history_location
                .or_else(|| daily_location.and_then(|url| Location::from_input(&url).ok()))
            {
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
                self.activate_form(activation, &ctx, shared);
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
    recovery: Option<Session>,
    window_title: String,
}
impl OliveApp {
    pub fn new(cc: &eframe::CreationContext<'_>, source: Option<OsString>) -> Self {
        let ctx = &cc.egui_ctx;
        ctx.set_fonts(crate::fonts::definitions());
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
            },
            tabs: vec![Tab::new(0)],
            active: 0,
            next_id: 1,
            focus_address: source.is_none(),
            reveal_tab: false,
            recovery: None,
            window_title: String::new(),
        };
        let defaults = app.shared.profile.data.settings.clone();
        app.tabs[0].zoom = defaults.default_zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        app.tabs[0].focus_settings = defaults.focus;
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
            let mut tab = Tab::new(self.next_id);
            self.next_id += 1;
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

    fn recovery_bar(&mut self, ui: &mut egui::Ui) {
        if self.recovery.is_none() {
            return;
        }
        let mut restore = false;
        let mut discard = false;
        egui::Panel::top("session-recovery").show(ui, |ui| {
            ui.label("Olive did not close normally. Restore your saved tabs or start fresh.");
            ui.small("Restored pages start with JavaScript disabled. Background tabs load when selected.");
            ui.horizontal_wrapped(|ui| {
                restore = ui.button("Restore tabs").clicked();
                discard = ui.button("Start fresh").clicked();
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
            self.tabs[self.active].error =
                Some("Close a tab before opening another (32 tab limit).".into());
            return;
        }
        let mut tab = Tab::new(self.next_id);
        self.next_id += 1;
        tab.zoom = self
            .shared
            .profile
            .data
            .settings
            .default_zoom
            .clamp(MIN_ZOOM, MAX_ZOOM);
        tab.focus_settings = self.shared.profile.data.settings.focus;
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
        egui::Panel::top("tabs")
            .exact_size(48.0)
            .frame(
                egui::Frame::new()
                    .fill(palette.chrome)
                    .inner_margin(egui::Margin::symmetric(10, 6)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.add(
                        egui::Image::new(&self.shared.icon)
                            .fit_to_exact_size(egui::vec2(26.0, 26.0)),
                    )
                    .on_hover_text("Olive Browser");
                    let width = (ui.available_width() - 44.0).max(100.0);
                    egui::ScrollArea::horizontal()
                        .id_salt("tab-strip")
                        .max_width(width)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                for (index, tab) in self.tabs.iter().enumerate() {
                                    ui.push_id(tab.id, |ui| {
                                        let active = index == self.active;
                                        let card = egui::Frame::new()
                                            .fill(if active {
                                                palette.surface
                                            } else {
                                                egui::Color32::TRANSPARENT
                                            })
                                            .corner_radius(8)
                                            .inner_margin(egui::Margin::symmetric(4, 0))
                                            .show(ui, |ui| {
                                                ui.horizontal(|ui| {
                                                    ui.spacing_mut().item_spacing.x = 0.0;
                                                    let busy = tab.pending.is_some()
                                                        || tab
                                                            .worker
                                                            .as_ref()
                                                            .is_some_and(Worker::busy);
                                                    let title: String =
                                                        tab.title().chars().take(80).collect();
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
                                                        .frame(false)
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
                                                        response.scroll_to_me(Some(
                                                            egui::Align::Center,
                                                        ));
                                                    }
                                                    if response.clicked() {
                                                        selected = Some(index);
                                                    }
                                                    if response
                                                        .clicked_by(egui::PointerButton::Middle)
                                                    {
                                                        close = Some(index);
                                                    }
                                                    if ui
                                                        .add(
                                                            IconButton::icon_only(
                                                                Icon::Close,
                                                                "Close tab",
                                                            )
                                                            .small()
                                                            .quiet(),
                                                        )
                                                        .on_hover_text("Close tab (Cmd/Ctrl+W)")
                                                        .clicked()
                                                    {
                                                        close = Some(index);
                                                    }
                                                });
                                            });
                                        if active {
                                            let rect = card.response.rect;
                                            ui.painter().line_segment(
                                                [
                                                    egui::pos2(rect.left() + 10.0, rect.bottom()),
                                                    egui::pos2(rect.right() - 10.0, rect.bottom()),
                                                ],
                                                egui::Stroke::new(2.0, palette.accent),
                                            );
                                        }
                                    });
                                }
                            });
                        });
                    add = ui
                        .add_enabled(
                            self.tabs.len() < MAX_TABS,
                            IconButton::icon_only(Icon::Plus, "New tab").quiet(),
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
        if self
            .tabs
            .iter()
            .any(|tab| tab.worker.as_ref().is_some_and(Worker::busy) || tab.pending.is_some())
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let previous_theme = self.shared.profile.data.settings.browser_theme;
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
        self.recovery_bar(ui);
        self.tab_bar(ui);
        if let Some(location) = self.tabs[self.active].deferred.take() {
            let scripting = self.tabs[self.active].deferred_scripts;
            self.tabs[self.active].open_with_scripts(
                location,
                Navigation::New,
                &ctx,
                scripting,
                &mut self.shared,
            );
        }
        let autofocus = std::mem::take(&mut self.focus_address);
        if let Some(location) = self.tabs[self.active].ui(ui, &mut self.shared, autofocus) {
            self.new_tab(Some(location), &ctx);
        }
        if self.shared.profile.data.settings.browser_theme != previous_theme {
            self.shared.profile.data.settings.browser_theme.apply(&ctx);
        }
        self.checkpoint();
        let title: String = self.tabs[self.active].title().chars().take(200).collect();
        if self.window_title != title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
                "{title} — Olive Browser"
            )));
            self.window_title = title;
        }
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
            cookies: CookieJar::default(),
            profile: Profile::default(),
            downloads: Downloads::default(),
            daily_windows: DailyWindows::default(),
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
                tab.ui(ui, shared, false);
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
                    ctx.set_fonts(crate::fonts::definitions());
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
            recovery: None,
            window_title: String::new(),
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
    fn clean_session_restore_enables_scripts_but_crash_restore_does_not() {
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
        assert!(browser.tabs[0].deferred_scripts);
        browser.restore_session(session, false);
        assert!(!browser.tabs[0].deferred_scripts);
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
            recovery: None,
            window_title: String::new(),
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
