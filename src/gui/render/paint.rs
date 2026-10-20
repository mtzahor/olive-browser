//! Paint a laid-out `Page` and turn pointer input into page actions.
use super::bidi::layout_bidi;
use super::layout::{BoxLayout, border_shape, image_placement, outside, shadow_shape};
use super::stacking::Stacking;
use super::*;

impl Page {
    pub fn show(&mut self, ui: &mut egui::Ui) -> Option<PageClick> {
        let mut textures = ImageTextureCache::default();
        self.show_with_textures(ui, &mut textures)
    }
    pub fn show_with_textures(
        &mut self,
        ui: &mut egui::Ui,
        textures: &mut ImageTextureCache,
    ) -> Option<PageClick> {
        let mut clicked = None;
        // Find numbers matches in document order, which deferred fixed boxes
        // leave, so each block's first match number is counted up front.
        let match_offsets: Vec<usize> = self
            .find
            .blocks
            .iter()
            .scan(0, |total, matches| {
                let first = *total;
                *total += matches.len();
                Some(first)
            })
            .collect();
        let mut scroll_to = self.scroll_to.take();
        let start = ui.cursor().min;
        if scroll_to.as_deref() == Some("") || scroll_to.as_deref() == Some("top") {
            ui.scroll_to_rect(
                egui::Rect::from_min_size(start, egui::vec2(1.0, 1.0)),
                Some(egui::Align::Min),
            );
            if scroll_to.as_deref() == Some("") {
                scroll_to = None;
            }
        }
        let width = ui.available_width().max(1.0);
        let viewport = egui::Rect::from_min_size(start, egui::vec2(width, ui.clip_rect().height()));
        // The page's first screen is the initial containing block.
        let mut stack = vec![BoxLayout::root(start, width)];
        stack[0].definite_height = Some(viewport.height());
        stack[0].containing.height = Some(viewport.height());
        self.box_rects.resize(self.boxes.len(), egui::Rect::NOTHING);
        let mut right = start.x + width;
        let mut layout_measure_steps = MAX_LAYOUT_MEASURE_STEPS;
        // This UI shares the page's painter and clip, but text placement must not
        // advance the caller's layout until the complete page size is known.
        let mut text_ui = ui.new_child(egui::UiBuilder::new().max_rect(ui.max_rect()));
        // Paint clips of the open `overflow: hidden` boxes, outermost first.
        let mut clips = vec![ui.clip_rect()];
        // Positioned boxes are reordered to paint above the rest once the
        // page is drawn.
        let mut stacking = Stacking::new(ui);
        let mut unsettled = false;
        // Fixed boxes are laid out after the rest of the page, from the spot
        // where they occur, so ancestors' overflow clips do not apply.
        let mut deferred: Vec<(usize, usize, egui::Pos2, f32)> = Vec::new();
        let mut next_deferred = 0;
        let mut in_deferred = false;
        let mut position = 0;
        let mut end = self.commands.len();
        loop {
            if position >= end {
                if in_deferred {
                    stack.pop();
                }
                let Some(&(first, last, origin, width)) = deferred.get(next_deferred) else {
                    break;
                };
                next_deferred += 1;
                stack.push(BoxLayout::root(origin, width));
                in_deferred = true;
                position = first;
                end = last + 1;
                continue;
            }
            let command_index = position;
            position += 1;
            let command = self.commands[command_index];
            if let Command::Open(index) = command {
                if !in_deferred && self.boxes[index].position == Position::Fixed {
                    let close = matching_close(&self.commands, command_index);
                    let parent = stack.last().unwrap();
                    deferred.push((
                        command_index,
                        close,
                        egui::pos2(parent.content_left, parent.cursor),
                        parent.width,
                    ));
                    position = close + 1;
                    continue;
                }
            }
            match command {
                Command::Anchor(index) => {
                    if scroll_to.as_deref() == Some(self.anchors[index].as_str()) {
                        let top = egui::pos2(start.x, stack.last().unwrap().cursor);
                        ui.scroll_to_rect(
                            egui::Rect::from_min_size(top, egui::vec2(1.0, 1.0)),
                            Some(egui::Align::Min),
                        );
                        scroll_to = None;
                    }
                }
                Command::Open(index) => {
                    stacking.open(ui, &self.boxes[index], command_index);
                    let parent = stack.last_mut().unwrap();
                    let placement = parent.child_placement(self.boxes[index]);
                    let layout = BoxLayout::open(
                        index,
                        self.boxes[index],
                        parent,
                        self.root_font,
                        ui,
                        placement,
                        viewport,
                        &self.box_rects,
                        &self.commands,
                        &self.boxes,
                        &self.blocks,
                        &self.images,
                        command_index,
                        &mut layout_measure_steps,
                    );
                    right = right.max(layout.left + layout.outer_width);
                    stacking.opened(ui, &self.boxes[index]);
                    let mut layout = layout;
                    if let Some(clip) = layout.overflow_clip(self.root_font) {
                        let clip = clip.intersect(*clips.last().unwrap());
                        clips.push(clip);
                        ui.set_clip_rect(clip);
                        text_ui.set_clip_rect(clip);
                        layout.clipped = true;
                    }
                    stack.push(layout);
                }
                Command::Close => {
                    let mut layout = stack.pop().unwrap();
                    // A box's own background and border are not clipped by it.
                    if layout.clipped {
                        clips.pop();
                        let clip = *clips.last().unwrap();
                        ui.set_clip_rect(clip);
                        text_ui.set_clip_rect(clip);
                    }
                    let root = self.root_font;
                    let rect = layout.finish(ui, root);
                    let previous = std::mem::replace(&mut self.box_rects[layout.index], rect);
                    if layout.anchored_bottom && (previous.height() - rect.height()).abs() > 0.5 {
                        unsettled = true;
                    }
                    stacking.close(ui, &layout.style);
                    // Table cells fill the height of their row; cells spanning
                    // rows wait for the last one, or for the end of the table.
                    let row_bottom = layout.content_top + layout.flex_height;
                    let mut spanning = Vec::new();
                    for (cell, cell_rect) in std::mem::take(&mut layout.row_cells) {
                        if cell.style.row_span > 1 {
                            let rows = cell.style.row_span - 1;
                            spanning.push((cell, cell_rect, rows));
                        } else {
                            cell.stretch_cell(ui, cell_rect, row_bottom, root, &mut self.box_rects);
                        }
                    }
                    let table_bottom = layout.last_row_bottom;
                    for (cell, cell_rect, _) in std::mem::take(&mut layout.spanning) {
                        cell.stretch_cell(ui, cell_rect, table_bottom, root, &mut self.box_rects);
                    }
                    let parent = stack.last_mut().unwrap();
                    parent.contains_image |= layout.contains_image;
                    if layout.out_of_flow && layout.contains_image {
                        parent.absolute_image_bottom =
                            parent.absolute_image_bottom.max(rect.bottom());
                    }
                    if layout.table_row {
                        let mut covering = Vec::new();
                        for (cell, cell_rect, rows) in std::mem::take(&mut parent.spanning) {
                            if rows > 1 {
                                covering.push((cell, cell_rect, rows - 1));
                            } else {
                                cell.stretch_cell(
                                    ui,
                                    cell_rect,
                                    row_bottom,
                                    root,
                                    &mut self.box_rects,
                                );
                            }
                        }
                        covering.extend(spanning);
                        parent.spanning = covering;
                        parent.last_row_bottom = row_bottom;
                    }
                    if layout.out_of_flow {
                        continue;
                    }
                    if layout.floating != FloatSide::None {
                        let [top, right, bottom, left] = layout.margins;
                        let margin_box = egui::Rect::from_min_max(
                            rect.min - egui::vec2(left, top),
                            rect.max + egui::vec2(right, bottom),
                        );
                        parent.add_float(margin_box, layout.floating);
                        continue;
                    }
                    if parent.flex_row {
                        let row_height = (rect.height() + layout.margin_bottom).max(0.0);
                        parent.flex_row_height = parent.flex_row_height.max(row_height);
                        parent.flex_height = parent
                            .flex_height
                            .max(parent.flex_y + parent.flex_row_height);
                    } else if parent.flex_column {
                        parent.cursor = layout.flow_top
                            + rect.height()
                            + layout.margin_bottom
                            + parent.flex_row_gap;
                    } else if parent.grid {
                        let row_height = (rect.height() + layout.margin_bottom).max(0.0);
                        parent.grid_row_height = parent.grid_row_height.max(row_height);
                        parent.grid_height = parent.grid_height.max(parent.grid_y + row_height);
                    } else {
                        parent.cursor = layout.flow_top + rect.height() + layout.margin_bottom;
                    }
                    if parent.table_row {
                        parent.row_cells.push((layout, rect));
                    }
                }
                Command::Text(index) => {
                    let parent = stack.last_mut().unwrap();
                    let block = &mut self.blocks[index];
                    // Lines beside floats take the room left between them.
                    let (inset_left, inset_right) = parent.float_insets(parent.cursor);
                    let width = (parent.width - inset_left - inset_right).max(1.0);
                    let x = parent.content_left + inset_left;
                    if block.rule {
                        // An authored border or background already paints the rule's box.
                        if !block.style.has_border() && block.style.background.3 == 0 {
                            ui.painter().hline(
                                x..=x + width,
                                parent.cursor + 0.5,
                                Stroke::new(
                                    1.0,
                                    paint_color(block.style.color, block.style.opacity),
                                ),
                            );
                            parent.cursor += 1.0;
                        }
                        continue;
                    }
                    let scale = ui.ctx().pixels_per_point();
                    if block
                        .layout
                        .as_ref()
                        .is_none_or(|(previous, previous_scale, _)| {
                            (previous - width).abs() > 0.5 || *previous_scale != scale
                        })
                    {
                        let galley = layout_bidi(block, ui, width);
                        block.layout = Some((width, scale, galley));
                    }
                    let galley = block.layout.as_ref().unwrap().2.clone();
                    if block.marker {
                        // Hang the marker outside the item's start edge, level with
                        // its first line, without taking space from the item text.
                        let gap = block.style.font_size * 0.45;
                        let left = if block.style.direction == Direction::Rtl {
                            x + width + gap
                        } else {
                            x - gap - galley.size().x
                        };
                        let rect = egui::Rect::from_min_size(
                            egui::pos2(left, parent.cursor),
                            galley.size(),
                        );
                        let origin = match galley.job.halign {
                            egui::Align::Min => rect.left_top(),
                            egui::Align::Center => rect.center_top(),
                            egui::Align::Max => rect.right_top(),
                        };
                        ui.painter()
                            .galley(origin, galley, color(block.style.color));
                        right = right.max(rect.right());
                        continue;
                    }
                    let offset = match galley.job.halign {
                        egui::Align::Min => 0.0,
                        egui::Align::Center => (width - galley.size().x).max(0.0) / 2.0,
                        egui::Align::Max => (width - galley.size().x).max(0.0),
                    };
                    let rect = egui::Rect::from_min_size(
                        egui::pos2(x + offset, parent.cursor),
                        galley.size(),
                    );
                    let galley_origin = match galley.job.halign {
                        egui::Align::Min => rect.left_top(),
                        egui::Align::Center => rect.center_top(),
                        egui::Align::Max => rect.right_top(),
                    };
                    let response = text_ui.put(
                        rect,
                        egui::Label::new(galley.clone()).selectable(true).sense(
                            if !block.visual_actions.is_empty() {
                                egui::Sense::click()
                            } else {
                                egui::Sense::hover()
                            },
                        ),
                    );
                    if !block.visual_actions.is_empty() {
                        response.widget_info(|| {
                            WidgetInfo::labeled(Role::Link, ui.is_enabled(), &block.job.text)
                        });
                    }
                    if response.hovered() {
                        if let Some(action) = response
                            .hover_pos()
                            .and_then(|pos| action_at(block, &galley, pos - galley_origin))
                        {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            let href = action.link.and_then(|id| self.links.get(&id)).cloned();
                            if response.clicked()
                                || response.clicked_by(egui::PointerButton::Middle)
                            {
                                clicked = Some(PageClick {
                                    form: None,
                                    new_tab: response.clicked_by(egui::PointerButton::Middle)
                                        || ui.input(|input| input.modifiers.command),
                                    target: action.target,
                                    href: href.clone(),
                                });
                            }
                            if let Some(href) = href {
                                response.on_hover_text(href);
                            }
                        }
                    }
                    if let Some(matches) = self.find.blocks.get(index) {
                        let active = self.find_current.checked_sub(match_offsets[index]);
                        let visual_matches = visual_match_ranges(block, matches);
                        if let Some(target) =
                            paint_matches(ui, &galley, galley_origin, &visual_matches, active)
                        {
                            if self.find_scroll {
                                ui.scroll_to_rect(target, Some(egui::Align::Center));
                                self.find_scroll = false;
                            }
                        }
                    }
                    parent.cursor += rect.height();
                    right = right.max(rect.right());
                }
                Command::Control(index, target) => {
                    let parent = stack.last_mut().unwrap();
                    let rect = egui::Rect::from_min_size(
                        egui::pos2(parent.content_left, parent.cursor + 4.0),
                        egui::vec2(parent.width, 0.0),
                    );
                    let mut control_ui = text_ui.new_child(
                        egui::UiBuilder::new()
                            .id_salt(("control", index))
                            .max_rect(rect),
                    );
                    control_ui.style_mut().override_font_id =
                        Some(FontId::proportional(self.root_font));
                    control_ui.spacing_mut().button_padding = egui::vec2(10.0, 6.0);
                    control_ui.spacing_mut().interact_size.y = 30.0;
                    if let Some(activation) = self.forms.show(index, &mut control_ui) {
                        let target = if activation.control == self.forms.controls[index].node {
                            target
                        } else {
                            self.commands.iter().find_map(|command| match command {
                                Command::Control(i, target)
                                    if self.forms.controls[*i].node == activation.control =>
                                {
                                    *target
                                }
                                _ => None,
                            })
                        };
                        clicked = Some(PageClick {
                            form: Some(activation),
                            target,
                            href: None,
                            new_tab: false,
                        });
                    }
                    parent.cursor = control_ui.min_rect().bottom() + 6.0;
                    right = right.max(control_ui.min_rect().right());
                }
                Command::Image(index) => {
                    let parent = stack.last_mut().unwrap();
                    let image = &self.images[index];
                    let placement = image_placement(image, parent, self.root_font);
                    let outer = placement.outer;
                    let content = placement.content;
                    let texture = textures.texture(ui.ctx(), image);
                    // Pictures may be transparent, so keep the shadow outside them.
                    if let Some(shadow) = shadow_shape(&image.style, outer, self.root_font) {
                        for clip in outside(outer) {
                            ui.painter().with_clip_rect(clip).add(shadow.clone());
                        }
                    }
                    if image.style.background.3 > 0 {
                        ui.painter().rect_filled(
                            outer,
                            placement.radius,
                            paint_color(image.style.background, image.style.opacity),
                        );
                    }
                    // Rounded corners clip the picture itself, inset by any border
                    // and padding so the frame keeps an even width.
                    let inset = (content.left() - outer.left()).clamp(0.0, 127.0) as u8;
                    ui.painter().add(
                        egui::epaint::RectShape::filled(
                            content,
                            egui::CornerRadius {
                                nw: placement.radius.nw.saturating_sub(inset),
                                ne: placement.radius.ne.saturating_sub(inset),
                                sw: placement.radius.sw.saturating_sub(inset),
                                se: placement.radius.se.saturating_sub(inset),
                            },
                            Color32::WHITE.gamma_multiply(image.style.opacity),
                        )
                        .with_texture(
                            texture.id(),
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        ),
                    );
                    if let Some(border) =
                        border_shape(&image.style, outer, placement.radius.into(), self.root_font)
                    {
                        ui.painter().add(border);
                    }
                    let response = ui.interact(
                        outer,
                        ui.make_persistent_id(("olive-image", image.node, &image.reference)),
                        if image.action != Action::default() {
                            egui::Sense::click()
                        } else {
                            egui::Sense::hover()
                        },
                    );
                    if image.action != Action::default() {
                        let label = image.alt.as_deref().unwrap_or("Image link");
                        response.widget_info(|| {
                            WidgetInfo::labeled(Role::Link, ui.is_enabled(), label)
                        });
                    }
                    if response.hovered() {
                        if image.action != Action::default() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        let href = image
                            .action
                            .link
                            .and_then(|id| self.links.get(&id))
                            .cloned();
                        if response.clicked() || response.clicked_by(egui::PointerButton::Middle) {
                            clicked = Some(PageClick {
                                form: None,
                                new_tab: response.clicked_by(egui::PointerButton::Middle)
                                    || ui.input(|input| input.modifiers.command),
                                target: image.action.target,
                                href: href.clone(),
                            });
                        }
                        if let Some(href) = href {
                            response.on_hover_text(href);
                        } else if let Some(alt) = &image.alt {
                            response.on_hover_text(alt);
                        }
                    }
                    parent.cursor = outer.bottom() + placement.margin_bottom;
                    parent.contains_image = true;
                    right = right.max(outer.right());
                }
            }
        }
        stacking.finish(ui);
        // Boxes placed by their bottom edge used heights from the previous
        // frame; when those changed, lay out once more. A few frames suffice,
        // so oscillating content cannot keep repainting.
        if unsettled && self.settle_frames < 3 {
            self.settle_frames += 1;
            ui.ctx().request_repaint();
        } else if !unsettled {
            self.settle_frames = 0;
        }
        ui.set_clip_rect(clips[0]);
        let bottom = stack[0].cursor.max(start.y);
        ui.allocate_space(egui::vec2(right - start.x, bottom - start.y));
        if self.forms.limited {
            ui.label("This page reached the form control limit. Form submission is unavailable.");
        }
        if self.truncated || self.css_limited {
            ui.add_space(12.0);
            ui.separator();
            ui.label(if self.truncated {
                "This preview has reached its display limit. The rest of the file is not shown."
            } else {
                "This document reached the CSS processing limit. Some styles are not shown."
            });
        }
        clicked
    }
}

// Highlight glyph ranges after text painting; translucent fills preserve authored colors.
/// The index of the `Close` that ends the box opened at `open`.
fn matching_close(commands: &[Command], open: usize) -> usize {
    let mut depth = 0usize;
    for (index, command) in commands.iter().enumerate().skip(open) {
        match command {
            Command::Open(_) => depth += 1,
            Command::Close => {
                depth -= 1;
                if depth == 0 {
                    return index;
                }
            }
            _ => {}
        }
    }
    commands.len() - 1
}

fn paint_matches(
    ui: &egui::Ui,
    galley: &egui::Galley,
    origin: egui::Pos2,
    matches: &[(Range<usize>, usize)],
    active: Option<usize>,
) -> Option<egui::Rect> {
    let mut offset = 0;
    let mut index = 0;
    let mut target = None;
    for row in &galley.rows {
        let end = offset + row.glyphs.len();
        while index < matches.len() && matches[index].0.start < end {
            let (range, logical_index) = &matches[index];
            let start = range.start.max(offset) - offset;
            let stop = range.end.min(end) - offset;
            if start < stop {
                let rect = egui::Rect::from_min_max(
                    origin + egui::vec2(row.pos.x + row.glyphs[start].pos.x, row.pos.y),
                    origin
                        + egui::vec2(
                            row.pos.x + row.glyphs[stop - 1].max_x(),
                            row.pos.y + row.size.y,
                        ),
                );
                let current = active == Some(*logical_index);
                ui.painter().rect_filled(
                    rect,
                    2.0,
                    if current {
                        Color32::from_rgba_unmultiplied(255, 145, 20, 115)
                    } else {
                        Color32::from_rgba_unmultiplied(255, 225, 30, 85)
                    },
                );
                if current && target.is_none() {
                    target = Some(rect);
                }
            }
            if range.end > end {
                break;
            }
            index += 1;
        }
        offset = end + usize::from(row.ends_with_newline);
        while index < matches.len() && matches[index].0.end <= offset {
            index += 1;
        }
    }
    target
}

pub(super) fn visual_match_ranges(
    block: &Block,
    matches: &[Range<usize>],
) -> Vec<(Range<usize>, usize)> {
    let mut logical_matches = vec![None; block.job.text.chars().count()];
    for (index, range) in matches.iter().enumerate() {
        logical_matches[range.clone()].fill(Some(index));
    }
    let mut result: Vec<(Range<usize>, usize)> = Vec::new();
    for (visual, &logical) in block.visual_to_logical.iter().enumerate() {
        if let Some(index) = logical_matches[logical] {
            if let Some((range, _)) = result
                .last_mut()
                .filter(|(range, previous)| range.end == visual && *previous == index)
            {
                range.end += 1;
            } else {
                result.push((visual..visual + 1, index));
            }
        }
    }
    result
}

// Hit-test actual glyphs, so adjacent links and surrounding text retain their
// individual actions even when they share a wrapped, selectable paragraph.
pub(super) fn action_at(
    block: &Block,
    galley: &egui::Galley,
    position: egui::Vec2,
) -> Option<Action> {
    let mut offset = 0;
    for row in &galley.rows {
        if row.rect().contains(position.to_pos2()) {
            let column = row.glyphs.iter().position(|glyph| {
                position.x >= row.pos.x + glyph.pos.x && position.x < row.pos.x + glyph.max_x()
            })?;
            return block
                .visual_actions
                .iter()
                .find(|(range, _)| range.contains(&(offset + column)))
                .map(|(_, action)| *action);
        }
        offset += row.glyphs.len() + usize::from(row.ends_with_newline);
    }
    None
}
