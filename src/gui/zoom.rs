use eframe::egui;

pub struct ZoomOutput<R> {
    pub inner: R,
    pub overflow: egui::Vec2,
}

/// Give the document its own coordinate system. Layout sees the narrower CSS
/// viewport at higher zoom; egui transforms painting and native control input.
pub fn show<R>(
    ui: &mut egui::Ui,
    id: u64,
    percent: u16,
    contents: impl FnOnce(&mut egui::Ui) -> R,
) -> ZoomOutput<R> {
    let rect = ui.available_rect_before_wrap();
    let scale = f32::from(percent) / 100.0;
    let transform = egui::emath::TSTransform::from_translation(rect.min.to_vec2())
        * egui::emath::TSTransform::from_scaling(scale);
    let layer = egui::LayerId::new(ui.layer_id().order, ui.id().with(("page-zoom", id)));
    ui.ctx().set_sublayer(ui.layer_id(), layer);
    ui.ctx().set_transform_layer(layer, transform);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(("zoom", id))
            .layer_id(layer)
            .max_rect(transform.inverse() * rect),
    );
    // The parent clip is in screen coordinates while the document child is in
    // transformed coordinates. Convert the complete viewport clip into the
    // child's coordinate system; intersecting it with `rect` would shorten the
    // bottom of the clip as the outer scroll offset grows, making lower page
    // content disappear while scrolling.
    child.set_clip_rect(transform.inverse() * ui.clip_rect());
    let result = contents(&mut child);
    // Keep the transformed layer at the viewport size. The caller reserves the
    // returned overflow after its frame has painted, so the frame itself does
    // not expand over the transformed page.
    let content_size = child.min_size() * scale;
    ui.allocate_space(rect.size());
    ZoomOutput {
        inner: result,
        overflow: egui::vec2(
            (content_size.x - rect.width()).max(0.0),
            (content_size.y - rect.height()).max(0.0),
        ),
    }
}

pub fn step(percent: u16, increase: bool) -> u16 {
    const STEPS: [u16; 10] = [50, 67, 75, 90, 100, 110, 125, 150, 175, 200];
    if increase {
        STEPS
            .into_iter()
            .find(|&value| value > percent)
            .unwrap_or(200)
    } else {
        STEPS
            .into_iter()
            .rev()
            .find(|&value| value < percent)
            .unwrap_or(50)
    }
}

#[cfg(test)]
mod tests {
    use super::show;
    use eframe::egui;

    #[test]
    fn propagates_scaled_document_size_to_the_parent() {
        let ctx = eframe::egui::Context::default();
        let mut parent_height = 0.0;
        let mut overflow = egui::Vec2::ZERO;
        let mut output = ctx.run_ui(
            eframe::egui::RawInput {
                screen_rect: Some(eframe::egui::Rect::from_min_size(
                    eframe::egui::Pos2::ZERO,
                    eframe::egui::vec2(800.0, 600.0),
                )),
                ..Default::default()
            },
            |ui| {
                let output = show(ui, 1, 200, |ui| {
                    ui.allocate_space(eframe::egui::vec2(300.0, 1_200.0));
                });
                overflow = output.overflow;
                ui.allocate_space(overflow);
                parent_height = ui.min_rect().height();
            },
        );
        output.textures_delta.clear();
        assert!(parent_height >= 2_400.0);
        assert!(overflow.y >= 1_800.0);
    }

    #[test]
    fn keeps_transformed_page_paint_visible_when_overflow_is_reserved_after_frame() {
        let ctx = eframe::egui::Context::default();
        let mut output = ctx.run_ui(
            eframe::egui::RawInput {
                screen_rect: Some(eframe::egui::Rect::from_min_size(
                    eframe::egui::Pos2::ZERO,
                    eframe::egui::vec2(800.0, 600.0),
                )),
                ..Default::default()
            },
            |ui| {
                eframe::egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let mut overflow = egui::Vec2::ZERO;
                        eframe::egui::Frame::new().inner_margin(16).show(ui, |ui| {
                            let rendered = show(ui, 1, 100, |ui| {
                                ui.label("page body");
                                ui.allocate_space(egui::vec2(300.0, 1_200.0));
                            });
                            overflow = rendered.overflow;
                        });
                        ui.allocate_space(overflow);
                    });
            },
        );
        let text_shapes = output
            .shapes
            .iter()
            .filter(|shape| matches!(shape.shape, eframe::egui::Shape::Text(_)))
            .count();
        output.textures_delta.clear();
        assert!(text_shapes > 0);
    }

    #[test]
    fn keeps_lower_page_content_visible_at_a_scrolled_offset() {
        let ctx = eframe::egui::Context::default();
        let mut output = ctx.run_ui(
            eframe::egui::RawInput {
                screen_rect: Some(eframe::egui::Rect::from_min_size(
                    eframe::egui::Pos2::ZERO,
                    eframe::egui::vec2(800.0, 600.0),
                )),
                ..Default::default()
            },
            |ui| {
                eframe::egui::ScrollArea::vertical()
                    .vertical_scroll_offset(600.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let mut overflow = egui::Vec2::ZERO;
                        eframe::egui::Frame::new().show(ui, |ui| {
                            let rendered = show(ui, 1, 100, |ui| {
                                ui.allocate_space(egui::vec2(300.0, 800.0));
                                ui.label("lower page content");
                                ui.allocate_space(egui::vec2(300.0, 400.0));
                            });
                            overflow = rendered.overflow;
                        });
                        ui.allocate_space(overflow);
                    });
            },
        );
        let visible_lower_text = output.shapes.iter().any(|shape| {
            matches!(&shape.shape, eframe::egui::Shape::Text(text) if text.galley.text() == "lower page content")
        });
        output.textures_delta.clear();
        assert!(visible_lower_text, "scrolled page content was clipped away");
    }
}
