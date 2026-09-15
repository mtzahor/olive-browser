use eframe::egui;

/// Give the document its own coordinate system. Layout sees the narrower CSS
/// viewport at higher zoom; egui transforms painting and native control input.
pub fn show<R>(
    ui: &mut egui::Ui,
    id: u64,
    percent: u16,
    contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let rect = ui.available_rect_before_wrap();
    let transform = egui::emath::TSTransform::from_translation(rect.min.to_vec2())
        * egui::emath::TSTransform::from_scaling(f32::from(percent) / 100.0);
    let layer = egui::LayerId::new(ui.layer_id().order, ui.id().with(("page-zoom", id)));
    ui.ctx().set_sublayer(ui.layer_id(), layer);
    ui.ctx().set_transform_layer(layer, transform);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(("zoom", id))
            .layer_id(layer)
            .max_rect(transform.inverse() * rect),
    );
    child.set_clip_rect(transform.inverse() * ui.clip_rect().intersect(rect));
    let result = contents(&mut child);
    ui.allocate_space(rect.size());
    result
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
