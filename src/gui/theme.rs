//! Browser chrome only. Documents and Focus keep their own presentation.
use eframe::egui::{self, Color32, Stroke};

pub const DOCUMENT_PAPER: Color32 = Color32::from_rgb(250, 250, 246);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Theme {
    #[default]
    Light,
    Dark,
}

#[derive(Clone, Copy)]
pub struct Palette {
    pub canvas: Color32,
    pub chrome: Color32,
    pub surface: Color32,
    pub hover: Color32,
    pub selection: Color32,
    pub border: Color32,
    pub ink: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub on_accent: Color32,
    pub error: Color32,
    pub error_bg: Color32,
    pub warning: Color32,
}

impl Theme {
    pub fn from_visuals(visuals: &egui::Visuals) -> Self {
        if visuals.dark_mode {
            Self::Dark
        } else {
            Self::Light
        }
    }

    pub fn toggle(&mut self) {
        *self = match self {
            Self::Light => Self::Dark,
            Self::Dark => Self::Light,
        };
    }

    pub fn palette(self) -> Palette {
        match self {
            Self::Light => Palette {
                canvas: Color32::from_rgb(248, 249, 244),
                chrome: Color32::from_rgb(234, 238, 226),
                surface: Color32::from_rgb(255, 255, 252),
                hover: Color32::from_rgb(224, 232, 213),
                selection: Color32::from_rgb(212, 226, 193),
                border: Color32::from_rgb(202, 210, 190),
                ink: Color32::from_rgb(38, 44, 32),
                muted: Color32::from_rgb(99, 110, 86),
                accent: Color32::from_rgb(78, 101, 43),
                on_accent: Color32::WHITE,
                error: Color32::from_rgb(155, 49, 35),
                error_bg: Color32::from_rgb(255, 236, 229),
                warning: Color32::from_rgb(137, 90, 16),
            },
            Self::Dark => Palette {
                canvas: Color32::from_rgb(24, 28, 23),
                chrome: Color32::from_rgb(31, 37, 29),
                surface: Color32::from_rgb(40, 47, 36),
                hover: Color32::from_rgb(51, 62, 44),
                selection: Color32::from_rgb(63, 81, 46),
                border: Color32::from_rgb(68, 79, 58),
                ink: Color32::from_rgb(233, 238, 224),
                muted: Color32::from_rgb(166, 180, 151),
                accent: Color32::from_rgb(180, 207, 139),
                on_accent: Color32::from_rgb(27, 39, 16),
                error: Color32::from_rgb(255, 173, 153),
                error_bg: Color32::from_rgb(65, 36, 29),
                warning: Color32::from_rgb(238, 193, 113),
            },
        }
    }

    pub fn visuals(self) -> egui::Visuals {
        let p = self.palette();
        let mut v = match self {
            Self::Light => egui::Visuals::light(),
            Self::Dark => egui::Visuals::dark(),
        };
        v.override_text_color = None;
        v.weak_text_color = Some(p.muted);
        v.panel_fill = p.canvas;
        v.window_fill = p.surface;
        v.window_stroke = Stroke::new(1.0, p.border);
        v.window_corner_radius = 12.into();
        v.menu_corner_radius = 10.into();
        v.extreme_bg_color = p.canvas;
        v.text_edit_bg_color = Some(p.surface);
        v.faint_bg_color = p.chrome;
        v.code_bg_color = p.chrome;
        v.hyperlink_color = p.accent;
        v.selection.bg_fill = p.selection;
        v.selection.stroke = Stroke::new(1.0, p.accent);
        v.text_cursor.stroke = Stroke::new(2.0, p.accent);
        v.warn_fg_color = p.warning;
        v.error_fg_color = p.error;
        for (widget, fill, border) in [
            (&mut v.widgets.noninteractive, p.surface, p.border),
            (&mut v.widgets.inactive, p.surface, p.border),
            (&mut v.widgets.hovered, p.hover, p.accent),
            (&mut v.widgets.active, p.selection, p.accent),
            (&mut v.widgets.open, p.selection, p.accent),
        ] {
            widget.bg_fill = fill;
            widget.weak_bg_fill = fill;
            widget.bg_stroke = Stroke::new(1.0, border);
            widget.fg_stroke = Stroke::new(1.0, p.ink);
            widget.corner_radius = 8.into();
            widget.expansion = 0.0;
        }
        // Sliders and checkboxes need a solid track against the window surface.
        v.widgets.inactive.bg_fill = p.chrome;
        v
    }

    pub fn apply(self, ctx: &egui::Context) {
        ctx.set_theme(match self {
            Self::Light => egui::Theme::Light,
            Self::Dark => egui::Theme::Dark,
        });
        let mut style = egui::Style {
            visuals: self.visuals(),
            ..Default::default()
        };
        style.spacing.button_padding = egui::vec2(10.0, 7.0);
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.window_margin = egui::Margin::same(16);
        style.spacing.interact_size.y = 32.0;
        ctx.set_global_style(style);
        ctx.request_repaint();
    }
}
