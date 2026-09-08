//! Font fallback shared by browser chrome, documents and rendering tests.
//! Fonts are embedded, so missing system/web fonts cannot make Hebrew disappear.

use eframe::egui::{FontData, FontDefinitions, FontFamily};

pub fn definitions() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "Inter".into(),
        FontData::from_static(include_bytes!("../../assets/fonts/InterVariable.ttf")).into(),
    );
    fonts
        .families
        .get_mut(&FontFamily::Proportional)
        .unwrap()
        .insert(0, "Inter".into());

    fonts.font_data.insert(
        "Noto Sans Hebrew".into(),
        FontData::from_static(include_bytes!("../../assets/fonts/NotoSansHebrew.ttf")).into(),
    );
    // Include code blocks, address fields and dialogs as well as normal page
    // text. Keep the existing Latin and emoji faces ahead of the new fallback.
    for family in fonts.families.values_mut() {
        family.push("Noto Sans Hebrew".into());
    }
    fonts
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{
        self, FontId,
        epaint::text::{Fonts, VariationCoords},
        text::{LayoutJob, TextFormat},
    };

    const HEBREW: &str = "אבגדהוזחטיךכלםמןנסעףפץצקרשת שָׁלוֹם העדפות הפרטיות שלך";

    #[test]
    fn bundled_fallback_fixes_the_yahoo_hebrew_missing_glyphs() {
        let mut old_fonts = definitions();
        old_fonts.font_data.remove("Noto Sans Hebrew");
        for family in old_fonts.families.values_mut() {
            family.retain(|name| name != "Noto Sans Hebrew");
        }
        // Inspect the actual face charmaps: egui 0.36's has_glyph reports false
        // for ordinary characters owned by its replacement-character face.
        let mut before = Fonts::new(Default::default(), old_fonts);
        assert!(
            !before
                .fonts
                .font(&FontFamily::Proportional)
                .characters()
                .contains_key(&'ש')
        );

        let mut after = Fonts::new(Default::default(), definitions());
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            let mut font = after.fonts.font(&family);
            let characters = font.characters();
            for text in [HEBREW, "Yahoo Privacy / Cookie 123 — café Ελληνικά Русский"]
            {
                let missing: Vec<_> = text
                    .chars()
                    .filter(|c| !characters.contains_key(c))
                    .collect();
                assert!(missing.is_empty(), "{family:?}: missing {missing:?}");
            }
        }
    }

    #[test]
    fn hebrew_regular_and_bold_rasterize_using_the_shared_fonts() {
        let ctx = egui::Context::default();
        ctx.set_fonts(definitions());
        let mut output = ctx.run_ui(Default::default(), |ui| {
            for weight in [400.0, 700.0] {
                let text = "העדפות הפרטיות שלך";
                let job = LayoutJob::single_section(
                    text.into(),
                    TextFormat {
                        font_id: FontId::proportional(24.0),
                        coords: VariationCoords::new([("wght", weight)]),
                        ..Default::default()
                    },
                );
                let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
                assert!(galley.size().x > 0.0);
                assert!(galley.num_vertices > 0);
                // Shaping can include zero-width continuation glyphs. Count
                // visible allocations, which must include every unpointed letter.
                let visible = galley
                    .rows
                    .iter()
                    .flat_map(|row| &row.glyphs)
                    .filter(|glyph| {
                        glyph.uv_rect.max[0] > glyph.uv_rect.min[0]
                            && glyph.uv_rect.max[1] > glyph.uv_rect.min[1]
                    })
                    .count();
                assert_eq!(
                    visible,
                    text.chars().filter(|c| !c.is_whitespace()).count(),
                    "weight {weight}"
                );
            }
        });
        output.textures_delta.clear();
    }
}
