use eframe::egui::IconData;

pub fn data() -> IconData {
    eframe::icon_data::from_png_bytes(include_bytes!("../../assets/olive-browser.png"))
        .expect("Olive Browser icon must be a valid PNG")
}
