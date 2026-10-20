//! Liquid glass: the browser frame is made of translucent panes floating over
//! an ambient backdrop. egui cannot blur what lies beneath a pane, so each one
//! is a tinted, translucent fill lit by a specular sheen along its top, with
//! light caught on its rim, a faint outline and a soft shadow that lifts it.
use crate::gui::theme::Palette;
use eframe::egui::{
    self, Color32, ColorImage, CornerRadius, Id, Mesh, Pos2, Rect, Shape, Stroke, StrokeKind,
    TextureId, TextureOptions,
    epaint::{RectShape, Shadow},
    pos2,
};

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

/// A glass pane filling `rect`, ready to paint or to place in a reserved slot.
pub fn pane(
    ctx: &egui::Context,
    rect: Rect,
    radius: impl Into<CornerRadius>,
    palette: &Palette,
    kind: Pane,
) -> Shape {
    let radius = radius.into();
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
    Shape::Vec(vec![
        shadow.as_shape(rect, radius).into(),
        RectShape::filled(
            rect,
            radius,
            if prominent {
                palette.glass_strong
            } else {
                palette.glass
            },
        )
        .into(),
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
    ])
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
    ui.painter()
        .set(slot, pane(ui.ctx(), rect, radius, palette, kind));
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

/// The shadow that lifts a page card off the backdrop.
pub fn card_shadow(painter: &egui::Painter, card: Rect, radius: f32, palette: &Palette) {
    painter.add(
        Shadow {
            offset: [0, 4],
            blur: 22,
            spread: 0,
            color: palette.glass_shadow,
        }
        .as_shape(card, CornerRadius::from(radius)),
    );
}

/// Round the corners of a page card drawn over the backdrop: the backdrop is
/// repainted over each square corner outside the rounded shape, and a fine
/// outline finishes the edge. `layer` must paint above the page.
pub fn round_card(
    ctx: &egui::Context,
    layer: egui::LayerId,
    card: Rect,
    radius: f32,
    palette: &Palette,
) {
    const ARC_STEPS: usize = 10;
    let screen = ctx.content_rect();
    let painter = ctx.layer_painter(layer);
    let radius = radius.min(card.width() / 2.0).min(card.height() / 2.0);
    let mut mesh = Mesh::default();
    for (corner, center, start) in [
        (
            card.left_top(),
            card.left_top() + egui::vec2(radius, radius),
            180.0f32,
        ),
        (
            card.right_top(),
            card.right_top() + egui::vec2(-radius, radius),
            270.0,
        ),
        (
            card.right_bottom(),
            card.right_bottom() + egui::vec2(-radius, -radius),
            0.0,
        ),
        (
            card.left_bottom(),
            card.left_bottom() + egui::vec2(radius, -radius),
            90.0,
        ),
    ] {
        let first = mesh.vertices.len() as u32;
        mesh.colored_vertex(corner, backdrop_color(palette, screen, corner));
        for step in 0..=ARC_STEPS {
            let angle = (start + 90.0 * step as f32 / ARC_STEPS as f32).to_radians();
            let pos = center + radius * egui::vec2(angle.cos(), angle.sin());
            mesh.colored_vertex(pos, backdrop_color(palette, screen, pos));
            if step > 0 {
                let arc = first + 1 + step as u32;
                mesh.add_triangle(first, arc - 1, arc);
            }
        }
    }
    painter.add(Shape::mesh(mesh));
    painter.add(RectShape::stroke(
        card,
        CornerRadius::from(radius),
        Stroke::new(1.0, palette.hairline),
        StrokeKind::Middle,
    ));
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
