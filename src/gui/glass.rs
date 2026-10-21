//! Liquid glass: the browser frame is made of glass panes floating over the
//! page, or over an ambient backdrop when there is none. The page fills the
//! window and scrolls beneath the frame, which paints in two bands above it.
//! Panes in the bands show the page through them, blurred and bent at their
//! rims (see `lens`); every pane is a tinted fill lit by a specular sheen
//! along its top, with light caught on its rim, a faint outline and a soft
//! shadow that lifts it.
use crate::gui::theme::Palette;
use eframe::egui::{
    self, Color32, ColorImage, CornerRadius, Id, LayerId, Mesh, Order, Pos2, Rect, Shape, Stroke,
    StrokeKind, TextureId, TextureOptions,
    epaint::{RectShape, Shadow},
    pos2,
};
use lens::Lens;

#[allow(unsafe_code)]
#[path = "glass/lens.rs"]
mod lens;

/// The layer the page paints in, above the browser's root layer.
pub fn page_layer() -> LayerId {
    LayerId::new(Order::Background, Id::unique("olive-page"))
}

/// A band of the browser frame, floating above the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Band {
    /// The tab strip, toolbar and find bar.
    Top,
    /// The status bar.
    Bottom,
}

impl Band {
    pub fn layer(self) -> LayerId {
        LayerId::new(
            Order::Background,
            Id::unique(match self {
                Self::Top => "olive-frame-top",
                Self::Bottom => "olive-frame-bottom",
            }),
        )
    }
}

/// Stack the page and then the frame's bands above `root` for this pass.
/// Each band takes the pointer within its rect, as known from the previous
/// pass, so the page beneath does not react there.
pub fn stack_layers(ctx: &egui::Context, root: LayerId, bands: [Rect; 2]) {
    ctx.set_sublayer(root, page_layer());
    for (band, rect) in [Band::Top, Band::Bottom].into_iter().zip(bands) {
        ctx.set_sublayer(root, band.layer());
        if rect.is_positive() {
            ctx.memory_mut(|memory| {
                memory.areas_mut().set_state(
                    band.layer(),
                    egui::AreaState {
                        pivot_pos: Some(rect.min),
                        size: Some(rect.size()),
                        interactable: true,
                        ..Default::default()
                    },
                )
            });
        }
    }
}

/// Run `add` in `band`, so that it paints above the page.
pub fn in_band<R>(ui: &mut egui::Ui, band: Band, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.scope_builder(egui::UiBuilder::new().layer_id(band.layer()), add)
        .inner
}

/// Copy the screen for the panes that show the page. Paint it in the top
/// band before anything else in the bands.
pub fn capture(ctx: &egui::Context) -> Shape {
    lens::capture(ctx, ctx.content_rect())
}

/// Soften the page under `band`, which covers `rect`, so that the frame
/// reads over any page: the page blurs, and a veil deepens toward the window
/// edge, both fading out toward the page.
pub fn band_backdrop(ctx: &egui::Context, rect: Rect, band: Band, palette: &Palette) -> Shape {
    let (edge, inner) = match band {
        Band::Top => (rect.top(), rect.bottom()),
        Band::Bottom => (rect.bottom(), rect.top()),
    };
    // Whole across the frame's controls, then softening into the page.
    let whole = edge + (inner - edge) * 0.7;
    let mut veil = Mesh::default();
    for (y, color) in [
        (edge, palette.veil),
        (whole, palette.veil.gamma_multiply(0.8)),
        (inner, Color32::TRANSPARENT),
    ] {
        veil.colored_vertex(pos2(rect.left(), y), color);
        veil.colored_vertex(pos2(rect.right(), y), color);
    }
    for row in 0..2 {
        let top = row * 2;
        veil.add_triangle(top, top + 1, top + 2);
        veil.add_triangle(top + 1, top + 3, top + 2);
    }
    Shape::Vec(vec![
        lens::pane(
            ctx,
            rect,
            CornerRadius::ZERO,
            Lens {
                blur: 12.0,
                refraction: 0.0,
                fade: Some((whole, inner)),
            },
        ),
        Shape::mesh(veil),
    ])
}

/// How far a pane stands out from the backdrop.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    /// Toolbar groups, tabs under the pointer and status chips.
    Regular,
    /// The active tab and other panes nearest the viewer.
    Prominent,
}

/// Grid cells across each side of the backdrop gradient.
const BACKDROP_STEPS: usize = 12;

/// The backdrop color at `pos` on a window covering `screen`: the palette's
/// four corner colors blended across the window, brightened by a glow of
/// light falling from above the tab strip.
pub fn backdrop_color(palette: &Palette, screen: Rect, pos: Pos2) -> Color32 {
    let u = ((pos.x - screen.left()) / screen.width().max(1.0)).clamp(0.0, 1.0);
    let v = ((pos.y - screen.top()) / screen.height().max(1.0)).clamp(0.0, 1.0);
    let [top_left, top_right, bottom_right, bottom_left] = palette.backdrop;
    let top = mix(top_left, top_right, u);
    let bottom = mix(bottom_left, bottom_right, u);
    let base = mix(top, bottom, v);
    let glow = (-((u - 0.68).powi(2) / 0.09 + v.powi(2) / 0.06)).exp() * 0.75;
    mix(base, palette.glow, glow)
}

fn mix(from: Color32, to: Color32, amount: f32) -> Color32 {
    let blend = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * amount).round() as u8;
    Color32::from_rgba_premultiplied(
        blend(from.r(), to.r()),
        blend(from.g(), to.g()),
        blend(from.b(), to.b()),
        blend(from.a(), to.a()),
    )
}

/// Paint the backdrop across the whole window, beneath everything else.
pub fn backdrop(painter: &egui::Painter, screen: Rect, palette: &Palette) {
    let mut mesh = Mesh::default();
    for row in 0..=BACKDROP_STEPS {
        for column in 0..=BACKDROP_STEPS {
            let pos = pos2(
                screen.left() + screen.width() * column as f32 / BACKDROP_STEPS as f32,
                screen.top() + screen.height() * row as f32 / BACKDROP_STEPS as f32,
            );
            mesh.colored_vertex(pos, backdrop_color(palette, screen, pos));
        }
    }
    let width = BACKDROP_STEPS as u32 + 1;
    for row in 0..BACKDROP_STEPS as u32 {
        for column in 0..BACKDROP_STEPS as u32 {
            let top_left = row * width + column;
            mesh.add_triangle(top_left, top_left + 1, top_left + width);
            mesh.add_triangle(top_left + 1, top_left + width + 1, top_left + width);
        }
    }
    painter.add(Shape::mesh(mesh));
}

/// A glass pane filling `rect` over the backdrop, ready to paint or to place
/// in a reserved slot.
pub fn pane(
    ctx: &egui::Context,
    rect: Rect,
    radius: impl Into<CornerRadius>,
    palette: &Palette,
    kind: Pane,
) -> Shape {
    pane_shape(ctx, rect, radius.into(), palette, kind, false)
}

/// Whether `ui` paints in a band of the frame, over the page.
fn over_page(ui: &egui::Ui) -> bool {
    [Band::Top, Band::Bottom]
        .into_iter()
        .any(|band| ui.layer_id() == band.layer())
}

/// A pane's shadow, glass, sheen and rims. Glass over the page shows it
/// through a lens beneath a tint that keeps the pane readable on any page.
fn pane_shape(
    ctx: &egui::Context,
    rect: Rect,
    radius: CornerRadius,
    palette: &Palette,
    kind: Pane,
    over_page: bool,
) -> Shape {
    let prominent = kind == Pane::Prominent;
    let shadow = Shadow {
        offset: [0, if prominent { 5 } else { 3 }],
        blur: if prominent { 20 } else { 12 },
        spread: 0,
        color: if prominent {
            palette.glass_shadow
        } else {
            palette.glass_shadow.gamma_multiply(0.6)
        },
    };
    let tint = match (over_page, prominent) {
        (false, false) => palette.glass,
        (false, true) => palette.glass_strong,
        (true, false) => palette.page_glass,
        (true, true) => palette.page_glass_strong,
    };
    let mut shapes = vec![shadow.as_shape(rect, radius).into()];
    if over_page {
        shapes.push(lens::pane(
            ctx,
            rect,
            radius,
            Lens {
                blur: 14.0,
                refraction: 5.0,
                fade: None,
            },
        ));
    }
    shapes.extend([
        RectShape::filled(rect, radius, tint).into(),
        RectShape::filled(rect, radius, palette.sheen)
            .with_texture(sheen(ctx), Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)))
            .into(),
        RectShape::stroke(
            rect,
            radius,
            Stroke::new(1.0, palette.rim),
            StrokeKind::Inside,
        )
        .into(),
        RectShape::stroke(
            rect,
            radius,
            Stroke::new(1.0, palette.hairline),
            StrokeKind::Outside,
        )
        .into(),
    ]);
    Shape::Vec(shapes)
}

/// Reserve a paint slot for a pane behind content that is not laid out yet.
pub fn reserve(ui: &egui::Ui) -> egui::layers::ShapeIdx {
    ui.painter().add(Shape::Noop)
}

/// Fill a reserved slot with a pane covering `rect`.
pub fn fill(
    ui: &egui::Ui,
    slot: egui::layers::ShapeIdx,
    rect: Rect,
    radius: impl Into<CornerRadius>,
    palette: &Palette,
    kind: Pane,
) {
    ui.painter().set(
        slot,
        pane_shape(ui.ctx(), rect, radius.into(), palette, kind, over_page(ui)),
    );
}

/// Lay out `content` on a regular glass pane, `margin` inside its edge.
pub fn capsule<R>(
    ui: &mut egui::Ui,
    margin: impl Into<egui::Margin>,
    radius: impl Into<CornerRadius>,
    palette: &Palette,
    content: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    let slot = reserve(ui);
    let inner = egui::Frame::new().inner_margin(margin).show(ui, content);
    fill(
        ui,
        slot,
        inner.response.rect,
        radius,
        palette,
        Pane::Regular,
    );
    inner
}

/// A white gradient from opaque at the top to clear below the middle, which
/// a pane's sheen color tints.
fn sheen(ctx: &egui::Context) -> TextureId {
    let id = Id::unique("olive-glass-sheen");
    if let Some(texture) = ctx.data(|data| data.get_temp::<egui::TextureHandle>(id)) {
        return texture.id();
    }
    const HEIGHT: usize = 64;
    let pixels = (0..HEIGHT)
        .map(|row| {
            let depth = row as f32 / (HEIGHT - 1) as f32;
            let light = (1.0 - depth / 0.55).clamp(0.0, 1.0).powf(1.7);
            Color32::from_white_alpha((light * 255.0).round() as u8)
        })
        .collect();
    let texture = ctx.load_texture(
        "olive-glass-sheen",
        ColorImage::new([1, HEIGHT], pixels),
        TextureOptions::LINEAR,
    );
    let texture_id = texture.id();
    ctx.data_mut(|data| data.insert_temp(id, texture));
    texture_id
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::theme::Theme;

    fn frame(ctx: &egui::Context, add: impl FnMut(&mut egui::Ui)) -> egui::FullOutput {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 300.0))),
                ..Default::default()
            },
            add,
        );
        output.textures_delta.clear();
        output
    }

    #[test]
    fn bands_paint_above_the_page_and_take_the_pointer_over_it() {
        let ctx = egui::Context::default();
        let window = Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 300.0));
        let bands = [
            Rect::from_min_max(window.min, pos2(400.0, 60.0)),
            Rect::from_min_max(pos2(0.0, 260.0), window.max),
        ];
        let mut root = None;
        let mut fills = Vec::new();
        for _ in 0..2 {
            let output = frame(&ctx, |ui| {
                root = Some(ui.layer_id());
                stack_layers(ui.ctx(), ui.layer_id(), bands);
                // The frame lays out before the page, yet paints above it.
                in_band(ui, Band::Top, |ui| {
                    ui.painter().rect_filled(bands[0], 0.0, Color32::RED)
                });
                ui.scope_builder(egui::UiBuilder::new().layer_id(page_layer()), |ui| {
                    ui.painter().rect_filled(window, 0.0, Color32::BLUE)
                });
            });
            fills = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    Shape::Rect(rect) => Some(rect.fill),
                    _ => None,
                })
                .collect();
        }
        assert_eq!(fills, [Color32::BLUE, Color32::RED]);
        assert_eq!(ctx.layer_id_at(pos2(200.0, 30.0)), Some(Band::Top.layer()));
        assert_eq!(ctx.layer_id_at(pos2(200.0, 150.0)), root);
        assert_eq!(
            ctx.layer_id_at(pos2(200.0, 280.0)),
            Some(Band::Bottom.layer())
        );
    }

    #[test]
    fn only_panes_in_the_bands_show_the_page_through_them() {
        let palette = Theme::Light.palette();
        let ctx = egui::Context::default();
        let rect = Rect::from_min_size(pos2(10.0, 10.0), egui::vec2(80.0, 30.0));
        let lenses = |band: Option<Band>| {
            let output = frame(&ctx, |ui| {
                let paint = |ui: &mut egui::Ui| {
                    let slot = reserve(ui);
                    fill(ui, slot, rect, 15, &palette, Pane::Regular);
                };
                match band {
                    Some(band) => in_band(ui, band, paint),
                    None => paint(ui),
                }
            });
            fn callbacks(shape: &Shape) -> usize {
                match shape {
                    Shape::Callback(_) => 1,
                    Shape::Vec(shapes) => shapes.iter().map(callbacks).sum(),
                    _ => 0,
                }
            }
            output
                .shapes
                .iter()
                .map(|shape| callbacks(&shape.shape))
                .sum::<usize>()
        };
        assert_eq!(lenses(None), 0);
        assert_eq!(lenses(Some(Band::Top)), 1);
        assert_eq!(lenses(Some(Band::Bottom)), 1);
    }

    #[test]
    fn backdrop_blends_corners_and_glows_beneath_the_tab_strip() {
        let palette = Theme::Light.palette();
        let screen = Rect::from_min_size(Pos2::ZERO, egui::vec2(1000.0, 800.0));
        assert_eq!(
            backdrop_color(&palette, screen, screen.left_bottom()),
            palette.backdrop[3]
        );
        // The glow brightens the top edge without reaching the lower corners.
        let lit = backdrop_color(&palette, screen, pos2(680.0, 0.0));
        let plain = mix(palette.backdrop[0], palette.backdrop[1], 0.68);
        assert!(lit.r() > plain.r() && lit.b() > plain.b());
        assert_eq!(
            backdrop_color(&palette, screen, screen.right_bottom()),
            palette.backdrop[2]
        );
    }
}
