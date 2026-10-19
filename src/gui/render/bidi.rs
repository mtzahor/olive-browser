//! Bidirectional text layout with logical-to-visual character maps.
use super::*;

/// Resolve Unicode levels in logical order, wrap that logical text, then apply
/// the visual ordering separately to each displayed line. Formats, links and
/// search offsets follow the same character map. Only logical text crosses IPC.
pub(super) fn layout_bidi(block: &mut Block, ui: &egui::Ui, width: f32) -> Arc<egui::Galley> {
    use unicode_bidi::{BidiInfo, Level};
    let base = match block.style.direction {
        Direction::Ltr => Some(Level::ltr()),
        Direction::Rtl => Some(Level::rtl()),
        Direction::Auto => None,
    };
    let bidi = BidiInfo::new(&block.job.text, base);
    let mut logical_job = block.job.clone();
    logical_job.halign = text_alignment(
        block.style.text_align,
        bidi.paragraphs
            .first()
            .is_some_and(|paragraph| paragraph.level.is_rtl()),
    );
    logical_job.wrap.max_width = if matches!(
        block.style.white_space,
        WhiteSpace::Pre | WhiteSpace::NoWrap
    ) {
        f32::INFINITY
    } else {
        width
    };
    let logical = ui.fonts_mut(|fonts| fonts.layout_job(logical_job));
    let chars: Vec<_> = block.job.text.char_indices().collect();
    block.visual_to_logical.clear();
    block.visual_actions.clear();
    if !bidi.has_rtl() {
        block.visual_job = (*logical.job).clone();
        block.visual_to_logical.extend(0..chars.len());
        block.visual_actions.clone_from(&block.actions);
        return logical;
    }
    let mut format_index = 0;
    let formats: Vec<_> = chars
        .iter()
        .map(|&(byte, _)| {
            while block.job.sections[format_index].byte_range.end.0 <= byte {
                format_index += 1;
            }
            &block.job.sections[format_index].format
        })
        .collect();
    let mut actions = vec![None; chars.len()];
    for (range, action) in &block.actions {
        actions[range.clone()].fill(Some(*action));
    }
    let mut visual_job = LayoutJob {
        halign: logical.job.halign,
        ..Default::default()
    };
    let mut lines = Vec::with_capacity(logical.rows.len());
    // Record each character's paragraph end once. Searching from every wrapped
    // line to the next newline rescans the same suffix and becomes quadratic
    // for long RTL paragraphs.
    let mut paragraph_ends = vec![chars.len(); chars.len() + 1];
    let mut paragraph_end = chars.len();
    for index in (0..chars.len()).rev() {
        if chars[index].1 == '\n' {
            paragraph_end = index;
        }
        paragraph_ends[index] = paragraph_end;
    }
    let mut first = 0;
    for (row_index, row) in logical.rows.iter().enumerate() {
        let row_start = first.min(chars.len());
        let paragraph_end = paragraph_ends[row_start];
        let is_last_row = row_index + 1 == logical.rows.len();
        // Hard paragraph breaks and the final row have exact source bounds.
        // This matters for RTL shaping, which can add continuation glyphs at
        // the end of a paragraph. Only soft-wrapped rows use glyph counts.
        let end = if row.ends_with_newline || is_last_row {
            paragraph_end
        } else {
            row_start
                .saturating_add(row.glyphs.len())
                .min(paragraph_end)
        };
        let start_byte = chars
            .get(row_start)
            .map_or(block.job.text.len(), |&(byte, _)| byte);
        let end_byte = chars
            .get(end)
            .map_or(block.job.text.len(), |&(byte, _)| byte);
        let paragraph_index = bidi
            .paragraphs
            .partition_point(|p| p.range.end <= start_byte);
        let paragraph = bidi
            .paragraphs
            .get(paragraph_index)
            .or_else(|| bidi.paragraphs.last());
        let para_level = paragraph.map_or(Level::ltr(), |p| p.level);
        let order = bidi_line_order(&bidi, start_byte..end_byte, row_start, para_level);
        let mut line = LayoutJob {
            halign: logical.job.halign,
            ..Default::default()
        };
        for (source, mirrored) in order {
            let mut character = chars[source].1;
            if mirrored {
                character = unicode_bidi_mirroring::get_mirrored(character).unwrap_or(character);
            }
            append_character(&mut line, character, formats[source]);
            append_character(&mut visual_job, character, formats[source]);
            let visual = block.visual_to_logical.len();
            block.visual_to_logical.push(source);
            if let Some(action) = actions[source] {
                if let Some((range, previous)) = block
                    .visual_actions
                    .last_mut()
                    .filter(|(range, previous)| range.end == visual && *previous == action)
                {
                    let _ = previous;
                    range.end += 1;
                } else {
                    block.visual_actions.push((visual..visual + 1, action));
                }
            }
        }
        // A hard break remains a logical character; soft wraps do not insert
        // synthetic characters into selection or find offsets.
        let has_source_newline = row.ends_with_newline
            && chars
                .get(end)
                .is_some_and(|&(_, character)| character == '\n');
        if has_source_newline {
            if let Some(format) = formats.get(end) {
                append_character(&mut visual_job, '\n', format);
                block.visual_to_logical.push(end);
            }
        }
        let galley = ui.fonts_mut(|fonts| fonts.layout_job(line));
        lines.push(galley);
        first = end
            .saturating_add(usize::from(has_source_newline))
            .min(chars.len());
    }
    block.visual_job = visual_job;
    let mut galley = egui::Galley::concat(
        Arc::new(block.visual_job.clone()),
        &lines,
        logical.pixels_per_point,
    );
    for (visual, original) in galley.rows.iter_mut().zip(&logical.rows) {
        visual.ends_with_newline = original.ends_with_newline;
    }
    Arc::new(galley)
}

pub(super) fn append_character(job: &mut LayoutJob, character: char, format: &TextFormat) {
    if let Some(last) = job
        .sections
        .last_mut()
        .filter(|last| last.format == *format)
    {
        job.text.push(character);
        last.byte_range.end.0 = job.text.len();
    } else {
        job.append(character.encode_utf8(&mut [0; 4]), 0.0, format.clone());
    }
}

/// Apply line-specific L1 whitespace resets to a small slice, avoiding a copy
/// of the whole paragraph for every wrapped row. L2 comes from unicode-bidi;
/// grapheme clusters retain base/mark order (L3), and glyph mirroring follows L4.
pub(super) fn bidi_line_order(
    bidi: &unicode_bidi::BidiInfo<'_>,
    line: Range<usize>,
    first_character: usize,
    base: unicode_bidi::Level,
) -> Vec<(usize, bool)> {
    use unicode_bidi::{BidiClass::*, BidiInfo};
    use unicode_segmentation::UnicodeSegmentation;
    let text = &bidi.text[line.clone()];
    let mut levels = bidi.levels[line.clone()].to_vec();
    let mut whitespace_start = Some(0);
    let mut previous_level = base;
    for (byte, character) in text.char_indices() {
        let end = byte + character.len_utf8();
        match bidi.original_classes[line.start + byte] {
            B | S => {
                let start = whitespace_start.take().unwrap_or(byte);
                levels[start..end].fill(base);
            }
            WS | FSI | LRI | RLI | PDI => {
                whitespace_start.get_or_insert(byte);
            }
            RLE | LRE | RLO | LRO | PDF | BN => {
                whitespace_start.get_or_insert(byte);
                levels[byte..end].fill(previous_level);
            }
            _ => {
                whitespace_start = None;
            }
        }
        previous_level = levels[byte];
    }
    if let Some(start) = whitespace_start {
        levels[start..].fill(base);
    }
    let mut next_character = first_character;
    let clusters: Vec<_> = text
        .grapheme_indices(true)
        .map(|(byte, cluster)| {
            let start = next_character;
            next_character += cluster.chars().count();
            let bidi_class = bidi.original_classes[line.start + byte];
            (
                start..next_character,
                levels[byte],
                levels[byte].is_rtl() && matches!(bidi_class, R | AL),
            )
        })
        .collect();
    let cluster_levels: Vec<_> = clusters.iter().map(|(_, level, _)| *level).collect();
    // epaint delegates each script run to HarfRust. HarfRust already emits
    // RTL glyphs in visual glyph order, so reverse only the order of visual
    // runs here and keep the characters inside an RTL run logical. Reversing
    // both would make Hebrew/Arabic words read backwards a second time.
    let visual_clusters = BidiInfo::reorder_visual(&cluster_levels);
    let mut result = Vec::with_capacity(next_character - first_character);
    let mut visual_index = 0;
    while visual_index < visual_clusters.len() {
        let index = visual_clusters[visual_index];
        let rtl_text = clusters[index].2;
        let mut end = visual_index + 1;
        if rtl_text {
            while end < visual_clusters.len() && clusters[visual_clusters[end]].2 {
                end += 1;
            }
        }
        let indices = &visual_clusters[visual_index..end];
        if rtl_text {
            for &index in indices.iter().rev() {
                let (range, level, _) = &clusters[index];
                result.extend(range.clone().map(|source| (source, level.is_rtl())));
            }
        } else {
            for &index in indices {
                let (range, level, _) = &clusters[index];
                result.extend(range.clone().map(|source| (source, level.is_rtl())));
            }
        }
        visual_index = end;
    }
    result
}

pub(super) fn text_alignment(align: TextAlign, rtl: bool) -> egui::Align {
    match align {
        TextAlign::Left => egui::Align::Min,
        TextAlign::Right => egui::Align::Max,
        TextAlign::Center => egui::Align::Center,
        TextAlign::Start => {
            if rtl {
                egui::Align::Max
            } else {
                egui::Align::Min
            }
        }
        TextAlign::End => {
            if rtl {
                egui::Align::Min
            } else {
                egui::Align::Max
            }
        }
    }
}
