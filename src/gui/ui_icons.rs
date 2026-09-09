//! Browser controls use a single, font-independent set of vector icons.
//! Drawn on a 24-unit grid with rounded 1.8-unit strokes; normally shown at 18 pt.

use eframe::egui::{
    self, Atom, Atoms, Button, Color32, Id, Painter, Rect, Response, RichText, Shape, Stroke, Ui,
    Vec2, Widget, WidgetInfo, WidgetType,
};

#[derive(Clone, Copy)]
pub enum Icon {
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
        match self {
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

/// A normal egui button with a custom-painted icon atom. Keeping egui's button
/// preserves keyboard activation, focus rings, disabled states and accessibility.
pub struct IconButton<'a> {
    pub button: Button<'a>,
    icon: Icon,
    label: String,
    icon_only: bool,
    color: Option<Color32>,
    warning: bool,
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
            color: None,
            warning: false,
        }
    }

    fn atom() -> Atom<'a> {
        Atom::custom(Id::new(Self::ICON_ID), Vec2::splat(18.0))
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
        self.color = Some(Color32::WHITE);
        let mut atoms = Atoms::new(Self::atom());
        if !self.icon_only {
            atoms.push_right(RichText::new(&self.label).color(Color32::WHITE));
        }
        self.button = Button::new(atoms)
            .gap(7.0)
            .fill(crate::render::OLIVE)
            .corner_radius(7)
            .min_size(Vec2::splat(36.0));
        self
    }

    pub fn warning(mut self, warning: bool) -> Self {
        self.warning = warning;
        self
    }

    pub fn small(mut self) -> Self {
        self.button = self.button.small();
        self
    }
}

impl Widget for IconButton<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let output = self.button.atom_ui(ui);
        if let Some(rect) = output.rect(Id::new(Self::ICON_ID)) {
            let color = self.color.unwrap_or_else(|| {
                ui.style()
                    .button_style(&Default::default(), output.response.widget_state())
                    .text_style
                    .color
            });
            self.icon.paint(ui.painter(), rect, color);
            if self.warning {
                let center = rect.right_top() + egui::vec2(-1.0, 2.0);
                ui.painter().circle_filled(center, 3.5, crate::render::INK);
                ui.painter()
                    .circle_filled(center, 2.0, Color32::from_rgb(245, 184, 73));
            }
        }
        if self.icon_only {
            output.response.widget_info(|| {
                WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), &self.label)
            });
            output.response.on_disabled_hover_text(self.label)
        } else {
            output.response
        }
    }
}

pub fn menu(ui: &mut Ui, icon: Icon, label: &str, content: impl FnOnce(&mut Ui)) {
    let response = ui.add(IconButton::new(icon, label).small());
    egui::Popup::menu(&response).show(content);
}
