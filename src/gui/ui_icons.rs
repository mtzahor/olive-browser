//! Browser controls use a single, font-independent set of vector icons.
//! Drawn on a 24-unit grid with rounded 1.8-unit strokes; normally shown at 18 pt.

use eframe::egui::{
    self, Atom, Button, Color32, IdSalt, Painter, Rect, Response, RichText, Role, Shape, Stroke,
    Ui, Vec2, Widget, WidgetInfo,
};

#[derive(Clone, Copy)]
pub enum Icon {
    Plus,
    Minus,
    Search,
    Back,
    Forward,
    Reload,
    Go,
    Folder,
    History,
    Code,
    CodeOff,
    Resources,
    Warning,
    Trash,
    Close,
    Check,
    Page,
    Focus,
    Appearance,
    Bookmark,
    Star,
    Download,
    Settings,
    Menu,
    Sun,
    Moon,
    Leaf,
    Globe,
    Lock,
    ChevronUp,
    ChevronDown,
    StarFilled,
}

impl Icon {
    pub fn paint(self, painter: &Painter, rect: Rect, color: Color32) {
        let scale = rect.width().min(rect.height()) / 24.0;
        let origin = rect.center() - Vec2::splat(12.0 * scale);
        let point = |x, y| origin + egui::vec2(x, y) * scale;
        let stroke = Stroke::new(1.8 * scale, color);
        let path = |points: &[[f32; 2]]| {
            let points: Vec<_> = points.iter().map(|p| point(p[0], p[1])).collect();
            painter.add(Shape::line(points.clone(), stroke));
            // Round caps and joins stay consistent across platforms and scale factors.
            for p in points {
                painter.circle_filled(p, stroke.width / 2.0, color);
            }
        };
        let arc = |cx: f32, cy: f32, radius: f32, start: f32, end: f32| {
            let points: Vec<_> = (0..=32)
                .map(|i| {
                    let angle = (start + (end - start) * i as f32 / 32.0).to_radians();
                    [cx + radius * angle.cos(), cy + radius * angle.sin()]
                })
                .collect();
            path(&points);
        };
        let quad = |from: [f32; 2], control: [f32; 2], to: [f32; 2]| {
            let points: Vec<_> = (0..=16)
                .map(|i| {
                    let t = i as f32 / 16.0;
                    let u = 1.0 - t;
                    [
                        u * u * from[0] + 2.0 * u * t * control[0] + t * t * to[0],
                        u * u * from[1] + 2.0 * u * t * control[1] + t * t * to[1],
                    ]
                })
                .collect();
            path(&points);
        };
        match self {
            Self::Menu => {
                for y in [6.0, 12.0, 18.0] {
                    path(&[[4.0, y], [20.0, y]]);
                }
            }
            Self::Sun => {
                painter.circle_stroke(point(12.0, 12.0), 4.0 * scale, stroke);
                for index in 0..8 {
                    let angle = index as f32 * std::f32::consts::FRAC_PI_4;
                    path(&[
                        [12.0 + 7.0 * angle.cos(), 12.0 + 7.0 * angle.sin()],
                        [12.0 + 10.0 * angle.cos(), 12.0 + 10.0 * angle.sin()],
                    ]);
                }
            }
            Self::Moon => {
                arc(12.0, 12.0, 9.0, 0.0, 270.0);
                arc(19.0, 5.0, 7.0, 90.0, 180.0);
            }
            Self::Search => {
                painter.circle_stroke(point(10.0, 10.0), 6.0 * scale, stroke);
                path(&[[14.5, 14.5], [21.0, 21.0]]);
            }
            Self::Plus => {
                path(&[[5.0, 12.0], [19.0, 12.0]]);
                path(&[[12.0, 5.0], [12.0, 19.0]]);
            }
            Self::Minus => path(&[[5.0, 12.0], [19.0, 12.0]]),
            Self::Focus => {
                path(&[[8.0, 3.0], [3.0, 3.0], [3.0, 8.0]]);
                path(&[[16.0, 3.0], [21.0, 3.0], [21.0, 8.0]]);
                path(&[[3.0, 16.0], [3.0, 21.0], [8.0, 21.0]]);
                path(&[[21.0, 16.0], [21.0, 21.0], [16.0, 21.0]]);
                path(&[[8.0, 9.0], [16.0, 9.0]]);
                path(&[[8.0, 13.0], [16.0, 13.0]]);
                path(&[[8.0, 17.0], [13.0, 17.0]]);
            }
            Self::Appearance => {
                path(&[[3.0, 19.0], [9.0, 5.0], [15.0, 19.0]]);
                path(&[[5.0, 14.0], [13.0, 14.0]]);
                path(&[[17.0, 10.0], [21.0, 10.0], [21.0, 19.0]]);
                path(&[[21.0, 14.0], [17.0, 14.0], [17.0, 19.0], [21.0, 19.0]]);
            }
            Self::Bookmark => {
                path(&[
                    [5.0, 4.0],
                    [19.0, 4.0],
                    [19.0, 21.0],
                    [12.0, 17.0],
                    [5.0, 21.0],
                    [5.0, 4.0],
                ]);
            }
            Self::Star | Self::StarFilled => {
                let points: Vec<[f32; 2]> = (0..10)
                    .map(|index| {
                        let angle = -std::f32::consts::FRAC_PI_2
                            + index as f32 * std::f32::consts::PI / 5.0;
                        let radius = if index % 2 == 0 { 9.0 } else { 4.0 };
                        [12.0 + radius * angle.cos(), 12.0 + radius * angle.sin()]
                    })
                    .collect();
                if matches!(self, Self::StarFilled) {
                    // A star is not convex: fill its inner pentagon and five points.
                    let at = |index: usize| point(points[index % 10][0], points[index % 10][1]);
                    let fill = |points| Shape::convex_polygon(points, color, Stroke::NONE);
                    painter.add(fill((0..5).map(|i| at(2 * i + 1)).collect()));
                    for i in 0..5 {
                        painter.add(fill(vec![at(2 * i + 9), at(2 * i), at(2 * i + 1)]));
                    }
                }
                path(&points);
                path(&[[points[9][0], points[9][1]], [points[0][0], points[0][1]]]);
            }
            Self::Leaf => {
                // An olive leaf: a slender blade with a midrib and a short stem.
                quad([5.0, 19.0], [5.0, 7.0], [20.0, 4.0]);
                quad([5.0, 19.0], [17.0, 18.0], [20.0, 4.0]);
                path(&[[3.0, 21.0], [5.0, 19.0]]);
                quad([5.0, 19.0], [11.5, 12.5], [16.5, 7.5]);
            }
            Self::Globe => {
                painter.circle_stroke(point(12.0, 12.0), 9.0 * scale, stroke);
                path(&[[3.0, 12.0], [21.0, 12.0]]);
                quad([12.0, 3.0], [6.5, 12.0], [12.0, 21.0]);
                quad([12.0, 3.0], [17.5, 12.0], [12.0, 21.0]);
            }
            Self::Lock => {
                path(&[
                    [5.0, 11.0],
                    [19.0, 11.0],
                    [19.0, 21.0],
                    [5.0, 21.0],
                    [5.0, 11.0],
                ]);
                path(&[[8.0, 11.0], [8.0, 8.0]]);
                path(&[[16.0, 11.0], [16.0, 8.0]]);
                arc(12.0, 8.0, 4.0, 180.0, 360.0);
                painter.circle_filled(point(12.0, 16.0), 1.4 * scale, color);
            }
            Self::ChevronUp => path(&[[6.0, 15.0], [12.0, 9.0], [18.0, 15.0]]),
            Self::ChevronDown => path(&[[6.0, 9.0], [12.0, 15.0], [18.0, 9.0]]),
            Self::Download => {
                path(&[[12.0, 3.0], [12.0, 16.0]]);
                path(&[[7.0, 11.0], [12.0, 16.0], [17.0, 11.0]]);
                path(&[[4.0, 21.0], [20.0, 21.0]]);
            }
            Self::Settings => {
                arc(12.0, 12.0, 7.0, 0.0, 360.0);
                painter.circle_filled(point(12.0, 12.0), 2.0 * scale, color);
                for angle in [0.0_f32, 60.0, 120.0, 180.0, 240.0, 300.0] {
                    let a = angle.to_radians();
                    path(&[
                        [12.0 + 8.0 * a.cos(), 12.0 + 8.0 * a.sin()],
                        [12.0 + 10.0 * a.cos(), 12.0 + 10.0 * a.sin()],
                    ]);
                }
            }
            Self::Back => {
                path(&[[19.0, 12.0], [5.0, 12.0]]);
                path(&[[11.0, 6.0], [5.0, 12.0], [11.0, 18.0]]);
            }
            Self::Forward | Self::Go => {
                path(&[[5.0, 12.0], [19.0, 12.0]]);
                path(&[[13.0, 6.0], [19.0, 12.0], [13.0, 18.0]]);
            }
            Self::Reload => {
                arc(12.0, 12.0, 8.0, 35.0, 315.0);
                path(&[[13.5, 6.5], [18.0, 6.5], [18.0, 2.0]]);
            }
            Self::Folder => {
                path(&[
                    [3.0, 18.0],
                    [3.0, 5.0],
                    [9.0, 5.0],
                    [11.0, 8.0],
                    [19.0, 8.0],
                    [19.0, 11.0],
                ]);
                path(&[
                    [3.0, 19.0],
                    [6.0, 11.0],
                    [22.0, 11.0],
                    [19.0, 19.0],
                    [3.0, 19.0],
                ]);
            }
            Self::History => {
                arc(12.0, 12.0, 8.0, 215.0, 495.0);
                path(&[[3.0, 3.0], [3.0, 8.0], [8.0, 8.0]]);
                path(&[[12.0, 7.0], [12.0, 12.0], [16.0, 14.0]]);
            }
            Self::Code | Self::CodeOff => {
                path(&[[7.0, 7.0], [2.0, 12.0], [7.0, 17.0]]);
                path(&[[17.0, 7.0], [22.0, 12.0], [17.0, 17.0]]);
                if matches!(self, Self::CodeOff) {
                    path(&[[3.0, 3.0], [21.0, 21.0]]);
                } else {
                    path(&[[14.0, 4.0], [10.0, 20.0]]);
                }
            }
            Self::Resources => {
                path(&[
                    [12.0, 3.0],
                    [22.0, 8.0],
                    [12.0, 13.0],
                    [2.0, 8.0],
                    [12.0, 3.0],
                ]);
                path(&[[3.0, 13.0], [12.0, 17.5], [21.0, 13.0]]);
                path(&[[3.0, 17.0], [12.0, 21.5], [21.0, 17.0]]);
            }
            Self::Warning => {
                path(&[[12.0, 3.0], [22.0, 20.0], [2.0, 20.0], [12.0, 3.0]]);
                path(&[[12.0, 9.0], [12.0, 13.0]]);
                painter.circle_filled(point(12.0, 16.5), 1.0 * scale, color);
            }
            Self::Trash => {
                path(&[[3.0, 6.0], [21.0, 6.0]]);
                path(&[[9.0, 6.0], [9.0, 3.0], [15.0, 3.0], [15.0, 6.0]]);
                path(&[[5.0, 6.0], [6.0, 21.0], [18.0, 21.0], [19.0, 6.0]]);
                path(&[[10.0, 10.0], [10.0, 17.0]]);
                path(&[[14.0, 10.0], [14.0, 17.0]]);
            }
            Self::Close => {
                path(&[[6.0, 6.0], [18.0, 18.0]]);
                path(&[[18.0, 6.0], [6.0, 18.0]]);
            }
            Self::Check => path(&[[4.0, 12.0], [9.0, 17.0], [20.0, 6.0]]),
            Self::Page => {
                path(&[
                    [14.0, 3.0],
                    [5.0, 3.0],
                    [5.0, 21.0],
                    [19.0, 21.0],
                    [19.0, 8.0],
                    [14.0, 3.0],
                    [14.0, 8.0],
                    [19.0, 8.0],
                ]);
                path(&[[9.0, 12.0], [15.0, 12.0]]);
                path(&[[9.0, 16.0], [15.0, 16.0]]);
            }
        }
    }
}

/// A three-quarter ring turning once a second, in the icon grid's stroke.
pub fn paint_spinner(ui: &Ui, rect: Rect, color: Color32) {
    let scale = rect.width().min(rect.height()) / 24.0;
    let start = (ui.input(|input| input.time) as f32 * 360.0) % 360.0;
    let points: Vec<_> = (0..=24)
        .map(|i| {
            let angle = (start + 270.0 * i as f32 / 24.0).to_radians();
            rect.center() + egui::vec2(angle.cos(), angle.sin()) * 8.0 * scale
        })
        .collect();
    ui.painter()
        .add(Shape::line(points, Stroke::new(2.0 * scale, color)));
    ui.ctx().request_repaint();
}

/// A normal egui button with a custom-painted icon atom. Keeping egui's button
/// preserves keyboard activation, focus rings, disabled states and accessibility.
pub struct IconButton<'a> {
    button: Button<'a>,
    icon: Icon,
    label: String,
    icon_only: bool,
    /// Keep the accessible name to the label when a shortcut is shown beside it.
    has_shortcut: bool,
    color: Option<Color32>,
    warning: bool,
    primary: bool,
    quiet: bool,
    spinning: bool,
}

impl<'a> IconButton<'a> {
    const ICON_ID: &'static str = "button-icon";

    pub fn new(icon: Icon, label: impl Into<RichText>) -> Self {
        let label = label.into();
        Self {
            label: label.text().to_owned(),
            button: Button::new((Self::atom(), label)).gap(7.0).corner_radius(7),
            icon,
            icon_only: false,
            has_shortcut: false,
            color: None,
            warning: false,
            primary: false,
            quiet: false,
            spinning: false,
        }
    }

    fn atom() -> Atom<'a> {
        Atom::custom(IdSalt::new(Self::ICON_ID), Vec2::splat(18.0))
    }

    pub fn icon_only(icon: Icon, label: &str) -> Self {
        Self {
            button: Button::new(Self::atom())
                .min_size(Vec2::splat(36.0))
                .corner_radius(7),
            icon_only: true,
            ..Self::new(icon, label)
        }
    }

    pub fn primary(mut self) -> Self {
        self.primary = true;
        self
    }

    /// Show the button's background only while it is hovered or pressed.
    pub fn quiet(mut self) -> Self {
        self.button = self.button.frame(true).frame_when_inactive(false);
        self.quiet = true;
        self
    }

    /// Keep the icon and label at the start when the button is wider than they are.
    pub fn align_start(mut self) -> Self {
        self.button = self.button.right_text("");
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.button = self.button.selected(selected);
        self
    }

    pub fn truncate(mut self) -> Self {
        self.button = self.button.truncate();
        self
    }

    pub fn min_size(mut self, size: Vec2) -> Self {
        self.button = self.button.min_size(size);
        self
    }

    pub fn warning(mut self, warning: bool) -> Self {
        self.warning = warning;
        self
    }

    /// Paint the icon in this color instead of the text color.
    pub fn icon_color(mut self, color: Color32) -> Self {
        self.color = Some(color);
        self
    }

    /// Replace the icon with a loading spinner.
    pub fn spinning(mut self, spinning: bool) -> Self {
        self.spinning = spinning;
        self
    }

    /// Weak right-aligned text, such as a menu item's keyboard shortcut.
    pub fn shortcut(mut self, text: impl Into<String>) -> Self {
        self.button = self.button.shortcut_text(text.into());
        self.has_shortcut = true;
        self
    }

    pub fn small(mut self) -> Self {
        self.button = self.button.small();
        if self.icon_only {
            self.button = self.button.min_size(Vec2::splat(24.0));
        }
        self
    }
}

impl Widget for IconButton<'_> {
    fn ui(mut self, ui: &mut Ui) -> Response {
        if self.primary {
            let palette = crate::gui::theme::Theme::from_visuals(ui.visuals()).palette();
            self.color = Some(palette.on_accent);
            // Set the text via a local style so callers keep their size and other options.
            return ui
                .scope(|ui| {
                    ui.visuals_mut().override_text_color = Some(palette.on_accent);
                    self.button = self.button.fill(palette.accent);
                    self.primary = false;
                    self.ui(ui)
                })
                .inner;
        }
        if self.quiet {
            // A soft fill is enough feedback for toolbar and menu buttons.
            return ui
                .scope(|ui| {
                    let widgets = &mut ui.visuals_mut().widgets;
                    widgets.hovered.bg_stroke = Stroke::NONE;
                    widgets.active.bg_stroke = Stroke::NONE;
                    self.quiet = false;
                    self.ui(ui)
                })
                .inner;
        }
        let output = self.button.atom_ui(ui);
        if let Some(rect) = output.rect(IdSalt::new(Self::ICON_ID)) {
            let color = self.color.unwrap_or_else(|| {
                ui.style()
                    .visuals
                    .widgets
                    .style(&output.response)
                    .fg_stroke
                    .color
            });
            if self.spinning {
                paint_spinner(ui, rect, color);
            } else {
                self.icon.paint(ui.painter(), rect, color);
            }
            if self.warning {
                let center = rect.right_top() + egui::vec2(-1.0, 2.0);
                ui.painter()
                    .circle_filled(center, 3.5, ui.visuals().window_fill);
                ui.painter()
                    .circle_filled(center, 2.0, Color32::from_rgb(245, 184, 73));
            }
        }
        if self.icon_only || self.has_shortcut {
            output
                .response
                .widget_info(|| WidgetInfo::labeled(Role::Button, ui.is_enabled(), &self.label));
        }
        if self.icon_only {
            output.response.on_disabled_hover_text(self.label)
        } else {
            output.response
        }
    }
}

/// A text button with the same hover-only frame as a quiet [`IconButton`],
/// its text at the start of `width`.
pub fn quiet_text_button(ui: &mut Ui, text: impl Into<RichText>, width: f32) -> Response {
    ui.scope(|ui| {
        let widgets = &mut ui.visuals_mut().widgets;
        widgets.hovered.bg_stroke = Stroke::NONE;
        widgets.active.bg_stroke = Stroke::NONE;
        ui.add(
            Button::new(text.into())
                .frame(true)
                .frame_when_inactive(false)
                .right_text("")
                .truncate()
                .min_size(Vec2::new(width, 30.0)),
        )
    })
    .inner
}

pub fn menu(ui: &mut Ui, icon: Icon, label: &str, content: impl FnOnce(&mut Ui)) {
    let response = ui.add(IconButton::new(icon, label).small().quiet());
    egui::Popup::menu(&response).show(content);
}

/// A rounded text field led by a magnifying glass, for find and history search.
pub fn search_field(ui: &mut Ui, edit: egui::TextEdit<'_>) -> Response {
    const ICON: &str = "search-field-icon";
    let palette = crate::gui::theme::Theme::from_visuals(ui.visuals()).palette();
    let output = ui
        .scope(|ui| {
            crate::gui::theme::Theme::round_fields(ui.visuals_mut(), 16);
            edit.prefix(Atom::custom(IdSalt::new(ICON), Vec2::splat(16.0)))
                .margin(egui::vec2(10.0, 6.0))
                .show(ui)
        })
        .inner;
    if let Some(rect) = output.response.rect(IdSalt::new(ICON)) {
        Icon::Search.paint(ui.painter(), rect, palette.muted);
    }
    output.response.response
}
