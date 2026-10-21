//! Light and dark appearance. The chrome palette lives here; Focus reading
//! colors are in `focus`, and documents keep their own presentation.
//!
//! The browser frame is liquid glass over an olive grove: translucent panes
//! float on a soft backdrop of sage and sunlight (see `glass`), with
//! leaf-green actions and an olive-oil gold for saved pages.
use eframe::egui::{self, Color32, Shadow, Stroke};

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
    /// The tab strip, one step darker than the toolbar the active tab joins.
    pub strip: Color32,
    pub chrome: Color32,
    pub surface: Color32,
    pub hover: Color32,
    pub selection: Color32,
    pub border: Color32,
    pub ink: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub on_accent: Color32,
    /// Olive-oil gold for bookmarked pages.
    pub oil: Color32,
    pub error: Color32,
    pub error_bg: Color32,
    pub warning: Color32,
    pub warning_bg: Color32,
    /// The ambient backdrop behind the glass: top left, top right, bottom
    /// right and bottom left.
    pub backdrop: [Color32; 4],
    /// Light falling on the backdrop from above the tab strip.
    pub glow: Color32,
    /// Glass panes, and the brighter glass of the active tab and pressed controls.
    pub glass: Color32,
    pub glass_strong: Color32,
    /// Light caught along a pane's edge, and the faint outline outside it.
    pub rim: Color32,
    pub hairline: Color32,
    pub glass_shadow: Color32,
    /// The specular sheen across the top of a pane.
    pub sheen: Color32,
    /// Recessed glass for fields, such as the address field.
    pub well: Color32,
    /// Glass panes over a page, and the brighter glass of the active tab
    /// there: tinted enough for the frame's text to read over any page.
    pub page_glass: Color32,
    pub page_glass_strong: Color32,
    /// The veil that softens the page under the frame toward the window edge.
    pub veil: Color32,
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
                canvas: Color32::from_rgb(247, 248, 241),
                strip: Color32::from_rgb(223, 229, 211),
                chrome: Color32::from_rgb(241, 244, 233),
                surface: Color32::from_rgb(255, 255, 251),
                hover: Color32::from_rgb(226, 233, 214),
                selection: Color32::from_rgb(210, 225, 188),
                border: Color32::from_rgb(203, 211, 188),
                ink: Color32::from_rgb(36, 42, 30),
                muted: Color32::from_rgb(96, 107, 83),
                accent: Color32::from_rgb(78, 101, 43),
                on_accent: Color32::WHITE,
                oil: Color32::from_rgb(178, 124, 24),
                error: Color32::from_rgb(155, 49, 35),
                error_bg: Color32::from_rgb(255, 236, 229),
                warning: Color32::from_rgb(137, 90, 16),
                warning_bg: Color32::from_rgb(250, 239, 212),
                backdrop: [
                    Color32::from_rgb(214, 226, 196),
                    Color32::from_rgb(239, 234, 210),
                    Color32::from_rgb(229, 226, 202),
                    Color32::from_rgb(203, 219, 190),
                ],
                glow: Color32::from_rgb(255, 253, 238),
                glass: Color32::from_white_alpha(150),
                glass_strong: Color32::from_white_alpha(222),
                rim: Color32::from_white_alpha(235),
                hairline: Color32::from_rgba_unmultiplied(48, 62, 28, 30),
                glass_shadow: Color32::from_rgba_unmultiplied(40, 52, 22, 38),
                sheen: Color32::from_white_alpha(150),
                well: Color32::from_white_alpha(196),
                page_glass: Color32::from_rgba_unmultiplied(250, 251, 244, 115),
                page_glass_strong: Color32::from_rgba_unmultiplied(252, 253, 247, 200),
                veil: Color32::from_rgba_unmultiplied(236, 240, 226, 150),
            },
            Self::Dark => Palette {
                canvas: Color32::from_rgb(24, 28, 23),
                strip: Color32::from_rgb(17, 20, 16),
                chrome: Color32::from_rgb(32, 38, 30),
                surface: Color32::from_rgb(42, 49, 38),
                hover: Color32::from_rgb(52, 63, 45),
                selection: Color32::from_rgb(64, 82, 47),
                border: Color32::from_rgb(62, 73, 54),
                ink: Color32::from_rgb(233, 238, 224),
                muted: Color32::from_rgb(166, 180, 151),
                accent: Color32::from_rgb(180, 207, 139),
                on_accent: Color32::from_rgb(27, 39, 16),
                oil: Color32::from_rgb(230, 186, 96),
                error: Color32::from_rgb(255, 173, 153),
                error_bg: Color32::from_rgb(65, 36, 29),
                warning: Color32::from_rgb(238, 193, 113),
                warning_bg: Color32::from_rgb(58, 47, 26),
                backdrop: [
                    Color32::from_rgb(29, 36, 26),
                    Color32::from_rgb(21, 25, 20),
                    Color32::from_rgb(27, 30, 22),
                    Color32::from_rgb(17, 22, 17),
                ],
                glow: Color32::from_rgb(50, 64, 37),
                glass: Color32::from_white_alpha(18),
                glass_strong: Color32::from_white_alpha(38),
                rim: Color32::from_white_alpha(46),
                hairline: Color32::from_black_alpha(110),
                glass_shadow: Color32::from_black_alpha(120),
                sheen: Color32::from_white_alpha(22),
                well: Color32::from_black_alpha(70),
                page_glass: Color32::from_rgba_unmultiplied(30, 36, 27, 175),
                page_glass_strong: Color32::from_rgba_unmultiplied(46, 54, 40, 210),
                veil: Color32::from_rgba_unmultiplied(20, 24, 18, 175),
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
        // Windows and menus are frosted glass: the page shows faintly through.
        v.window_fill = match self {
            Self::Light => Color32::from_rgba_unmultiplied(252, 252, 246, 244),
            Self::Dark => Color32::from_rgba_unmultiplied(34, 40, 31, 242),
        };
        v.window_stroke = Stroke::new(
            1.0,
            match self {
                Self::Light => Color32::from_rgba_unmultiplied(48, 62, 28, 44),
                Self::Dark => Color32::from_white_alpha(34),
            },
        );
        v.window_corner_radius = 18.into();
        v.menu_corner_radius = 14.into();
        // Shadows take the grove's green-brown rather than neutral gray.
        let shade = |alpha| match self {
            Self::Light => Color32::from_rgba_unmultiplied(40, 52, 22, alpha),
            Self::Dark => Color32::from_black_alpha(alpha.saturating_mul(3)),
        };
        v.window_shadow = Shadow {
            offset: [0, 16],
            blur: 44,
            spread: 0,
            color: shade(40),
        };
        v.popup_shadow = Shadow {
            offset: [0, 8],
            blur: 26,
            spread: 0,
            color: shade(34),
        };
        v.extreme_bg_color = p.canvas;
        v.text_edit_bg_color = Some(p.well);
        v.faint_bg_color = p.chrome;
        v.code_bg_color = p.chrome;
        v.hyperlink_color = p.accent;
        v.selection.bg_fill = p.selection;
        v.selection.stroke = Stroke::new(1.0, p.accent);
        v.text_cursor.stroke = Stroke::new(2.0, p.accent);
        v.warn_fg_color = p.warning;
        v.error_fg_color = p.error;
        // Controls are glass too: brighter as the pointer reaches them, tinted
        // with leaf green while pressed.
        let hover = match self {
            Self::Light => Color32::from_white_alpha(210),
            Self::Dark => Color32::from_white_alpha(28),
        };
        for (widget, fill, border) in [
            (&mut v.widgets.noninteractive, p.surface, p.border),
            (&mut v.widgets.inactive, p.glass_strong, p.hairline),
            (&mut v.widgets.hovered, hover, p.hairline),
            (&mut v.widgets.active, p.selection, p.accent),
            // Also the title bar of the front window, which should stay quiet.
            (&mut v.widgets.open, hover, p.hairline),
        ] {
            widget.bg_fill = fill;
            widget.weak_bg_fill = fill;
            widget.bg_stroke = Stroke::new(1.0, border);
            widget.fg_stroke = Stroke::new(1.0, p.ink);
            widget.corner_radius = 10.into();
            widget.expansion = 0.0;
        }
        // Sliders and checkboxes need a solid track against the window surface.
        v.widgets.inactive.bg_fill = p.strip;
        v
    }

    /// Round text fields into pills, such as the address and find fields.
    pub fn round_fields(visuals: &mut egui::Visuals, radius: u8) {
        for widget in [
            &mut visuals.widgets.inactive,
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
        ] {
            widget.corner_radius = radius.into();
        }
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
