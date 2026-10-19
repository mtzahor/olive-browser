//! Block, inline-block, flex and grid box geometry for the display list.
use super::bidi::layout_bidi;
use super::*;

pub(super) struct ImagePlacement {
    pub(super) outer: egui::Rect,
    pub(super) content: egui::Rect,
    pub(super) border: f32,
    pub(super) radius: u8,
    pub(super) margin_bottom: f32,
}

pub(super) fn image_placement(
    image: &RenderedImage,
    parent: &BoxLayout,
    root: f32,
) -> ImagePlacement {
    let style = image.style;
    let resolve = |value: Length| value.resolve(parent.width, style.font_size, root);
    let margin = style.margin.map(resolve);
    let padding = style.padding.map(resolve);
    let border = if style.border_solid {
        resolve(style.border_width).max(0.0)
    } else {
        0.0
    };
    let horizontal_sides = padding[1] + padding[3] + border * 2.0;
    let vertical_sides = padding[0] + padding[2] + border * 2.0;
    let available = (parent.width - margin[1] - margin[3] - horizontal_sides).max(1.0);
    let intrinsic = egui::vec2(image.image.size[0] as f32, image.image.size[1] as f32);
    let width_value = if style.width == Length::Auto {
        image.html_width
    } else {
        Some(
            (resolve(style.width)
                - if style.border_box {
                    horizontal_sides
                } else {
                    0.0
                })
            .max(1.0),
        )
    };
    let height_value = if style.height == Length::Auto {
        image.html_height
    } else {
        Some(
            (resolve(style.height)
                - if style.border_box {
                    vertical_sides
                } else {
                    0.0
                })
            .max(1.0),
        )
    };
    let mut content_width = width_value.unwrap_or(intrinsic.x).max(1.0);
    let mut content_height = height_value.unwrap_or(intrinsic.y).max(1.0);
    if width_value.is_some() && height_value.is_none() {
        content_height = (content_width * intrinsic.y / intrinsic.x).max(1.0);
    } else if width_value.is_none() && height_value.is_some() {
        content_width = (content_height * intrinsic.x / intrinsic.y).max(1.0);
    }
    let max_width = if style.max_width == Length::Auto {
        available
    } else {
        available.min(
            (resolve(style.max_width)
                - if style.border_box {
                    horizontal_sides
                } else {
                    0.0
                })
            .max(1.0),
        )
    };
    if content_width > max_width {
        let scale = max_width / content_width;
        content_width = max_width;
        if height_value.is_none() {
            content_height *= scale;
        }
    }
    let min_height = resolve(style.min_height)
        - if style.border_box {
            vertical_sides
        } else {
            0.0
        };
    content_height = content_height.max(min_height);
    let outer_width = content_width + horizontal_sides;
    let auto_left = style.margin[3] == Length::Auto;
    let auto_right = style.margin[1] == Length::Auto;
    let spare = (parent.width - outer_width - margin[1] - margin[3]).max(0.0);
    let alignment = if auto_left {
        spare / if auto_right { 2.0 } else { 1.0 }
    } else if !auto_right && parent.style.text_align == TextAlign::Center {
        spare / 2.0
    } else if !auto_right && parent.style.text_align == TextAlign::Right {
        spare
    } else {
        0.0
    };
    let left = parent.content_left + margin[3] + alignment;
    let top = parent.cursor + margin[0];
    let content_top = top + border + padding[0];
    let content = egui::Rect::from_min_size(
        egui::pos2(left + border + padding[3], content_top),
        egui::vec2(content_width, content_height),
    );
    let outer = egui::Rect::from_min_size(
        egui::pos2(left, top),
        egui::vec2(outer_width, content_height + vertical_sides),
    );
    let radius = style
        .border_radius
        .resolve(0.0, style.font_size, root)
        .clamp(0.0, 255.0) as u8;
    ImagePlacement {
        outer,
        content,
        border,
        radius,
        margin_bottom: margin[2],
    }
}

pub(super) struct BoxLayout {
    pub(super) index: usize,
    pub(super) style: ComputedStyle,
    pub(super) left: f32,
    pub(super) top: f32,
    pub(super) outer_width: f32,
    pub(super) content_left: f32,
    pub(super) width: f32,
    pub(super) cursor: f32,
    pub(super) flow_top: f32,
    pub(super) content_top: f32,
    pub(super) padding_bottom: f32,
    pub(super) border: f32,
    pub(super) margin_bottom: f32,
    pub(super) flex_row: bool,
    pub(super) flex_column: bool,
    pub(super) flex_items: Vec<FlexPlacement>,
    pub(super) flex_child_index: usize,
    pub(super) flex_y: f32,
    pub(super) flex_row_height: f32,
    pub(super) flex_height: f32,
    pub(super) flex_row_gap: f32,
    pub(super) grid: bool,
    pub(super) grid_columns: usize,
    pub(super) grid_child_index: usize,
    pub(super) grid_y: f32,
    pub(super) grid_row_height: f32,
    pub(super) grid_height: f32,
    pub(super) grid_row_gap: f32,
    pub(super) grid_column_gap: f32,
    pub(super) out_of_flow: bool,
    pub(super) shadow_paint: Option<egui::layers::ShapeIdx>,
    pub(super) paint: Option<egui::layers::ShapeIdx>,
}

#[derive(Clone, Copy, Default)]
pub(super) struct FlexPlacement {
    pub(super) width: f32,
    pub(super) x: f32,
    pub(super) break_before: bool,
}

#[derive(Clone, Copy)]
pub(super) enum ChildPlacement {
    Flex(FlexPlacement),
    Grid { width: f32, x: f32, y: f32 },
}

pub(super) struct FlexLayout {
    pub(super) items: Vec<FlexPlacement>,
    pub(super) has_anonymous_text: bool,
}

/// Measure a block flex item's contents without wrapping so auto-sized list
/// items keep their intrinsic width instead of stretching across the row.
pub(super) fn intrinsic_width(
    commands: &[Command],
    open_index: usize,
    blocks: &mut [Block],
    images: &[RenderedImage],
    ui: &egui::Ui,
    remaining_steps: &mut usize,
) -> f32 {
    let mut depth = 0usize;
    let mut width: f32 = 0.0;
    for command in commands.iter().skip(open_index + 1) {
        if *remaining_steps == 0 {
            break;
        }
        *remaining_steps -= 1;
        match *command {
            Command::Open(_) => depth += 1,
            Command::Close if depth == 0 => break,
            Command::Close => depth -= 1,
            Command::Text(index) => {
                let cost = blocks[index].job.text.len();
                if cost > *remaining_steps {
                    break;
                }
                *remaining_steps -= cost;
                let galley = layout_bidi(&mut blocks[index], ui, 10_000.0);
                width = width.max(galley.size().x);
            }
            Command::Image(index) => {
                let image = &images[index];
                let natural = image.html_width.unwrap_or(image.image.size[0] as f32);
                width = width.max(natural);
            }
            Command::Control(_, _) => width = width.max(120.0),
            Command::Anchor(_) => {}
        }
    }
    width.max(1.0)
}

pub(super) fn has_direct_text(
    commands: &[Command],
    open_index: usize,
    remaining_steps: &mut usize,
) -> bool {
    let mut depth = 0usize;
    let mut completed = false;
    for command in commands.iter().skip(open_index + 1) {
        if *remaining_steps == 0 {
            return true;
        }
        *remaining_steps -= 1;
        match *command {
            Command::Text(_) if depth == 0 => return true,
            Command::Open(_) => depth += 1,
            Command::Close if depth == 0 => {
                completed = true;
                break;
            }
            Command::Close => depth -= 1,
            _ => {}
        }
    }
    !completed
}

// These borrowed inputs are one render pass's shared measurement context.
#[allow(clippy::too_many_arguments)]
pub(super) fn flex_layouts(
    commands: &[Command],
    boxes: &[ComputedStyle],
    blocks: &mut [Block],
    images: &[RenderedImage],
    ui: &egui::Ui,
    open_index: usize,
    style: ComputedStyle,
    available_width: f32,
    root: f32,
    remaining_steps: &mut usize,
) -> FlexLayout {
    #[derive(Clone, Copy)]
    struct Item {
        base: f32,
        extra: f32,
        margin_left: f32,
        grow: f32,
        shrink: f32,
    }

    let gap = style
        .column_gap
        .resolve(0.0, style.font_size, root)
        .max(0.0);
    let mut rows: Vec<Vec<Item>> = Vec::new();
    let mut row = Vec::new();
    let mut row_used = 0.0;
    let mut depth = 0usize;
    let mut has_anonymous_text = false;
    let mut completed = false;
    for (command_index, command) in commands.iter().enumerate().skip(open_index + 1) {
        if *remaining_steps == 0 {
            has_anonymous_text = true;
            break;
        }
        *remaining_steps -= 1;
        match *command {
            Command::Text(_) if depth == 0 => has_anonymous_text = true,
            Command::Open(index) if depth == 0 => {
                let child = boxes[index];
                if child.position != Position::Absolute {
                    let resolve =
                        |length: Length| length.resolve(available_width, child.font_size, root);
                    let margin_left = if child.margin[3] == Length::Auto {
                        0.0
                    } else {
                        resolve(child.margin[3])
                    };
                    let margin_right = if child.margin[1] == Length::Auto {
                        0.0
                    } else {
                        resolve(child.margin[1])
                    };
                    let padding = resolve(child.padding[1]) + resolve(child.padding[3]);
                    let border = if child.border_solid {
                        resolve(child.border_width).max(0.0) * 2.0
                    } else {
                        0.0
                    };
                    let explicit_width = if child.width == Length::Auto {
                        None
                    } else {
                        Some(
                            (resolve(child.width)
                                - if child.border_box {
                                    padding + border
                                } else {
                                    0.0
                                })
                            .max(1.0),
                        )
                    };
                    let base = if child.flex_basis != Length::Auto {
                        (resolve(child.flex_basis)
                            - if child.border_box {
                                padding + border
                            } else {
                                0.0
                            })
                        .max(1.0)
                    } else {
                        explicit_width.unwrap_or_else(|| {
                            intrinsic_width(
                                commands,
                                command_index,
                                blocks,
                                images,
                                ui,
                                remaining_steps,
                            )
                        })
                    };
                    let min_width = (resolve(child.min_width)
                        - if child.border_box {
                            padding + border
                        } else {
                            0.0
                        })
                    .max(1.0);
                    let base = base.max(min_width);
                    let extra = margin_left + margin_right + padding + border;
                    let projected = base + extra + if row.is_empty() { 0.0 } else { gap };
                    if style.flex_wrap == FlexWrap::Wrap
                        && !row.is_empty()
                        && row_used + projected > available_width
                    {
                        rows.push(std::mem::take(&mut row));
                        row_used = 0.0;
                    }
                    if !row.is_empty() {
                        row_used += gap;
                    }
                    row_used += base + extra;
                    row.push(Item {
                        base,
                        extra,
                        margin_left,
                        grow: child.flex_grow,
                        shrink: child.flex_shrink,
                    });
                }
                depth += 1;
            }
            Command::Open(_) => depth += 1,
            Command::Close if depth == 0 => {
                completed = true;
                break;
            }
            Command::Close => depth -= 1,
            _ => {}
        }
    }
    if !row.is_empty() {
        rows.push(row);
    }
    has_anonymous_text |= !completed;

    let mut placements = Vec::new();
    for (row_index, row) in rows.into_iter().enumerate() {
        let mut widths: Vec<f32> = row.iter().map(|item| item.base).collect();
        let base_used = row.iter().map(|item| item.base + item.extra).sum::<f32>()
            + gap * row.len().saturating_sub(1) as f32;
        let free = available_width - base_used;
        if free > 0.0 {
            let grow = row.iter().map(|item| item.grow).sum::<f32>();
            if grow > 0.0 {
                for (width, item) in widths.iter_mut().zip(&row) {
                    *width += free * item.grow / grow;
                }
            }
        } else if free < 0.0 {
            let shrink = row
                .iter()
                .map(|item| item.shrink * item.base.max(1.0))
                .sum::<f32>();
            if shrink > 0.0 {
                for (width, item) in widths.iter_mut().zip(&row) {
                    *width = (*width + free * item.shrink * item.base.max(1.0) / shrink).max(1.0);
                }
            }
        }
        let used = widths
            .iter()
            .zip(&row)
            .map(|(width, item)| width + item.extra)
            .sum::<f32>()
            + gap * row.len().saturating_sub(1) as f32;
        let remaining = (available_width - used).max(0.0);
        let mut start = 0.0;
        let mut distributed_gap = gap;
        match style.justify_content {
            JustifyContent::Start => {}
            JustifyContent::End => start = remaining,
            JustifyContent::Center => start = remaining / 2.0,
            JustifyContent::SpaceBetween if row.len() > 1 => {
                distributed_gap += remaining / (row.len() - 1) as f32;
            }
            JustifyContent::SpaceAround if !row.is_empty() => {
                distributed_gap += remaining / row.len() as f32;
                start = remaining / (2.0 * row.len() as f32);
            }
            JustifyContent::SpaceEvenly if !row.is_empty() => {
                distributed_gap += remaining / (row.len() + 1) as f32;
                start = remaining / (row.len() + 1) as f32;
            }
            _ => {}
        }
        let mut x = start;
        for ((item, width), item_index) in row.iter().zip(widths).zip(0..) {
            placements.push(FlexPlacement {
                width,
                x: x + item.margin_left,
                break_before: row_index > 0 && item_index == 0,
            });
            x += width + item.extra + distributed_gap;
        }
    }
    FlexLayout {
        items: placements,
        has_anonymous_text,
    }
}

impl BoxLayout {
    pub(super) fn root(start: egui::Pos2, width: f32) -> Self {
        Self {
            index: 0,
            style: ComputedStyle::default(),
            left: start.x,
            top: start.y,
            outer_width: width,
            content_left: start.x,
            width,
            cursor: start.y,
            flow_top: start.y,
            content_top: start.y,
            padding_bottom: 0.0,
            border: 0.0,
            margin_bottom: 0.0,
            flex_row: false,
            flex_column: false,
            flex_items: Vec::new(),
            flex_child_index: 0,
            flex_y: 0.0,
            flex_row_height: 0.0,
            flex_height: 0.0,
            flex_row_gap: 0.0,
            grid: false,
            grid_columns: 1,
            grid_child_index: 0,
            grid_y: 0.0,
            grid_row_height: 0.0,
            grid_height: 0.0,
            grid_row_gap: 0.0,
            grid_column_gap: 0.0,
            out_of_flow: false,
            shadow_paint: None,
            paint: None,
        }
    }

    pub(super) fn child_placement(&mut self, child: ComputedStyle) -> Option<ChildPlacement> {
        if child.position == Position::Absolute {
            return None;
        }
        if self.flex_row {
            let placement = self
                .flex_items
                .get(self.flex_child_index)
                .copied()
                .unwrap_or(FlexPlacement {
                    width: self.width,
                    ..Default::default()
                });
            self.flex_child_index = self.flex_child_index.saturating_add(1);
            if placement.break_before {
                self.flex_y += self.flex_row_height + self.flex_row_gap;
                self.flex_row_height = 0.0;
            }
            return Some(ChildPlacement::Flex(placement));
        }
        if self.grid {
            let index = self.grid_child_index;
            self.grid_child_index = self.grid_child_index.saturating_add(1);
            if index > 0 && index % self.grid_columns == 0 {
                self.grid_y += self.grid_row_height + self.grid_row_gap;
                self.grid_row_height = 0.0;
            }
            let gap_total = self.grid_column_gap * self.grid_columns.saturating_sub(1) as f32;
            let track_width = ((self.width - gap_total) / self.grid_columns as f32).max(1.0);
            let column = index % self.grid_columns;
            return Some(ChildPlacement::Grid {
                width: track_width,
                x: column as f32 * (track_width + self.grid_column_gap),
                y: self.grid_y,
            });
        }
        None
    }

    // Keep the layout pass inputs borrowed rather than cloning page structures.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn open(
        index: usize,
        style: ComputedStyle,
        parent: &mut Self,
        root: f32,
        ui: &egui::Ui,
        placement: Option<ChildPlacement>,
        commands: &[Command],
        boxes: &[ComputedStyle],
        blocks: &mut [Block],
        images: &[RenderedImage],
        command_index: usize,
        remaining_steps: &mut usize,
    ) -> Self {
        let resolve = |v: Length| v.resolve(parent.width, style.font_size, root);
        let margin = style.margin.map(resolve);
        let padding = style.padding.map(resolve);
        let border = if style.border_solid {
            resolve(style.border_width)
        } else {
            0.0
        };
        let sides = padding[1] + padding[3] + border * 2.0;
        let available = (parent.width - margin[1] - margin[3] - sides).max(1.0);
        let assigned = match placement {
            Some(ChildPlacement::Flex(item)) => Some(item.width),
            Some(ChildPlacement::Grid { width, .. }) if style.width == Length::Auto => {
                Some((width - margin[1] - margin[3] - sides).max(1.0))
            }
            Some(ChildPlacement::Grid { .. }) | None => None,
        };
        let width = if let Some(assigned) = assigned {
            assigned.min(available)
        } else if style.width == Length::Auto {
            if style.position == Position::Absolute {
                intrinsic_width(commands, command_index, blocks, images, ui, remaining_steps)
                    .min(available)
            } else {
                available
            }
        } else {
            (resolve(style.width) - if style.border_box { sides } else { 0.0 }).max(1.0)
        };
        let width = if style.max_width == Length::Auto {
            width
        } else {
            width.min(
                (resolve(style.max_width) - if style.border_box { sides } else { 0.0 }).max(1.0),
            )
        };
        let width = width.max(resolve(style.min_width).max(1.0));
        let spare = (available - width).max(0.0);
        let left_auto = style.margin[3] == Length::Auto;
        let right_auto = style.margin[1] == Length::Auto;
        let left_margin = margin[3]
            + if left_auto {
                spare / if right_auto { 2.0 } else { 1.0 }
            } else {
                0.0
            };
        let flow_top = match placement {
            Some(ChildPlacement::Flex(_)) => parent.content_top + parent.flex_y + margin[0],
            Some(ChildPlacement::Grid { y, .. }) => parent.content_top + y + margin[0],
            None => parent.cursor + margin[0],
        };
        let (normal_left, normal_top) = match placement {
            Some(ChildPlacement::Flex(item)) => (parent.content_left + item.x, flow_top),
            Some(ChildPlacement::Grid { x, .. }) => (parent.content_left + x + margin[3], flow_top),
            None => (parent.content_left + left_margin, flow_top),
        };
        let absolute = style.position == Position::Absolute;
        let left = if absolute {
            if style.inset[3] != Length::Auto {
                parent.content_left + resolve(style.inset[3])
            } else if style.inset[1] != Length::Auto {
                parent.content_left + parent.width - resolve(style.inset[1]) - width - sides
            } else {
                parent.content_left
            }
        } else if style.position == Position::Relative {
            normal_left
                + if style.inset[3] != Length::Auto {
                    resolve(style.inset[3])
                } else if style.inset[1] != Length::Auto {
                    -resolve(style.inset[1])
                } else {
                    0.0
                }
        } else {
            normal_left
        };
        let top = if absolute {
            if style.inset[0] != Length::Auto {
                parent.content_top + resolve(style.inset[0])
            } else if style.inset[2] != Length::Auto {
                parent.cursor - resolve(style.inset[2])
            } else {
                parent.cursor
            }
        } else if style.position == Position::Relative {
            normal_top
                + if style.inset[0] != Length::Auto {
                    resolve(style.inset[0])
                } else if style.inset[2] != Length::Auto {
                    -resolve(style.inset[2])
                } else {
                    0.0
                }
        } else {
            normal_top
        };
        let content_top = top + border + padding[0];
        let grid = style.display == Display::Grid
            && !has_direct_text(commands, command_index, remaining_steps);
        let row_flex = style.display == Display::Flex
            && style.flex_direction == crate::css::FlexDirection::Row;
        let mut has_anonymous_text = false;
        let flex_items = if row_flex {
            let layout = flex_layouts(
                commands,
                boxes,
                blocks,
                images,
                ui,
                command_index,
                style,
                width,
                root,
                remaining_steps,
            );
            has_anonymous_text = layout.has_anonymous_text;
            layout.items
        } else {
            Vec::new()
        };
        Self {
            index,
            style,
            left,
            top,
            outer_width: width + sides,
            content_left: left + border + padding[3],
            width,
            cursor: content_top,
            flow_top,
            content_top,
            padding_bottom: padding[2],
            border,
            margin_bottom: margin[2],
            flex_row: row_flex && !has_anonymous_text,
            flex_column: style.display == Display::Flex
                && (style.flex_direction == crate::css::FlexDirection::Column
                    || has_anonymous_text),
            flex_items,
            flex_child_index: 0,
            flex_y: 0.0,
            flex_row_height: 0.0,
            flex_height: 0.0,
            flex_row_gap: style.row_gap.resolve(0.0, style.font_size, root).max(0.0),
            grid,
            grid_columns: style.grid_columns.clamp(1, 12) as usize,
            grid_child_index: 0,
            grid_y: 0.0,
            grid_row_height: 0.0,
            grid_height: 0.0,
            grid_row_gap: style.row_gap.resolve(0.0, style.font_size, root).max(0.0),
            grid_column_gap: style
                .column_gap
                .resolve(0.0, style.font_size, root)
                .max(0.0),
            out_of_flow: absolute,
            shadow_paint: style
                .box_shadow
                .map(|_| ui.painter().add(egui::Shape::Noop)),
            paint: Some(ui.painter().add(egui::Shape::Noop)),
        }
    }
    pub(super) fn finish(&self, ui: &egui::Ui, root: f32) -> egui::Rect {
        let min_height = self
            .style
            .height
            .resolve(0.0, self.style.font_size, root)
            .max(
                self.style
                    .min_height
                    .resolve(0.0, self.style.font_size, root),
            );
        let min_height = if self.style.border_box {
            (min_height - (self.content_top - self.top) - self.padding_bottom - self.border)
                .max(0.0)
        } else {
            min_height
        };
        let bottom = self
            .cursor
            .max(self.content_top + min_height)
            .max(self.content_top + self.flex_height)
            .max(self.content_top + self.grid_height)
            + self.padding_bottom
            + self.border;
        let rect = egui::Rect::from_min_max(
            egui::pos2(self.left, self.top),
            egui::pos2(self.left + self.outer_width, bottom),
        );
        if let (Some(paint), Some(shadow)) = (self.shadow_paint, self.style.box_shadow) {
            let resolve = |value: Length| value.resolve(0.0, self.style.font_size, root);
            let offset = egui::vec2(resolve(shadow.offset_x), resolve(shadow.offset_y));
            let blur = resolve(shadow.blur).clamp(0.0, 64.0);
            let spread = resolve(shadow.spread).clamp(-32.0, 32.0);
            let expansion =
                (spread + blur * 0.7).clamp(-rect.width().min(rect.height()) / 2.0, 96.0);
            let shadow_rect = rect.expand(expansion).translate(offset);
            let radius = self
                .style
                .border_radius
                .resolve(0.0, self.style.font_size, root)
                .clamp(0.0, 255.0) as u8;
            let alpha_scale = if blur == 0.0 { 1.0 } else { 0.28 };
            let fill = paint_color(shadow.color, self.style.opacity * alpha_scale);
            let shape = egui::epaint::RectShape::new(
                shadow_rect,
                radius.saturating_add(expansion.clamp(0.0, 255.0) as u8),
                fill,
                Stroke::NONE,
                egui::StrokeKind::Inside,
            );
            ui.painter().set(paint, shape);
        }
        if let Some(paint) = self.paint {
            let radius = self
                .style
                .border_radius
                .resolve(0.0, self.style.font_size, root)
                .clamp(0.0, 255.0) as u8;
            ui.painter().set(
                paint,
                egui::epaint::RectShape::new(
                    rect,
                    radius,
                    paint_color(self.style.background, self.style.opacity),
                    Stroke::new(
                        self.border,
                        paint_color(self.style.border_color, self.style.opacity),
                    ),
                    egui::StrokeKind::Inside,
                ),
            );
        }
        rect
    }
}
