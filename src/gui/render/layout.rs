//! Block, inline-block, flex and grid box geometry for the display list.
use super::*;

pub(super) struct ImagePlacement {
    pub(super) outer: egui::Rect,
    pub(super) content: egui::Rect,
    pub(super) radius: egui::CornerRadius,
    pub(super) margin_bottom: f32,
}

/// Circular corner radii for a box. Percentages use the shorter side, and
/// radii shrink together when adjacent corners would overlap, as in CSS.
pub(super) fn corner_radii(
    style: &ComputedStyle,
    rect: egui::Rect,
    root: f32,
) -> egui::epaint::CornerRadiusF32 {
    let (width, height) = (rect.width().max(0.0), rect.height().max(0.0));
    let [nw, ne, se, sw] = style.border_radius.map(|radius| {
        radius
            .resolve(width.min(height), style.font_size, root)
            .max(0.0)
    });
    let scale = [
        (nw + ne, width),
        (sw + se, width),
        (nw + sw, height),
        (ne + se, height),
    ]
    .into_iter()
    .filter(|&(sum, side)| sum > side)
    .map(|(sum, side)| side / sum)
    .fold(1.0, f32::min);
    egui::epaint::CornerRadiusF32 { nw, ne, sw, se } * scale
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
    let border = style.border_widths(root);
    let horizontal_sides = padding[1] + padding[3] + border[1] + border[3];
    let vertical_sides = padding[0] + padding[2] + border[0] + border[2];
    // Pictures beside floats take the room left between them.
    let (inset_left, inset_right) = parent.float_insets(parent.cursor);
    let room = (parent.width - inset_left - inset_right).max(1.0);
    let available = (room - margin[1] - margin[3] - horizontal_sides).max(1.0);
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
    // Percentage heights follow the parent's definite height, if it has one.
    let used_height = |length: Length| match length {
        Length::Auto => None,
        length if length.has_percent() => parent
            .definite_height
            .map(|basis| length.resolve(basis, style.font_size, root)),
        length => Some(length.resolve(0.0, style.font_size, root)),
    };
    let height_value = used_height(style.height)
        .map(|height| {
            (height
                - if style.border_box {
                    vertical_sides
                } else {
                    0.0
                })
            .max(1.0)
        })
        .or(image.html_height);
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
    let min_height = used_height(style.min_height).unwrap_or(0.0)
        - if style.border_box {
            vertical_sides
        } else {
            0.0
        };
    content_height = content_height.max(min_height);
    let outer_width = content_width + horizontal_sides;
    let auto_left = style.margin[3] == Length::Auto;
    let auto_right = style.margin[1] == Length::Auto;
    let spare = (room - outer_width - margin[1] - margin[3]).max(0.0);
    let alignment = if auto_left {
        spare / if auto_right { 2.0 } else { 1.0 }
    } else if auto_right {
        0.0
    } else {
        // Pictures follow the line's alignment, including start and end in
        // right-to-left text.
        let rtl = parent.style.direction == Direction::Rtl;
        match super::bidi::text_alignment(parent.style.text_align, rtl) {
            egui::Align::Min => 0.0,
            egui::Align::Center => spare / 2.0,
            egui::Align::Max => spare,
        }
    };
    let left = parent.content_left + inset_left + margin[3] + alignment;
    let top = parent.cursor + margin[0];
    let content_top = top + border[0] + padding[0];
    let content = egui::Rect::from_min_size(
        egui::pos2(left + border[3] + padding[3], content_top),
        egui::vec2(content_width, content_height),
    );
    let outer = egui::Rect::from_min_size(
        egui::pos2(left, top),
        egui::vec2(outer_width, content_height + vertical_sides),
    );
    ImagePlacement {
        outer,
        content,
        radius: corner_radii(&style, outer, root).into(),
        margin_bottom: margin[2],
    }
}

/// The stroke of a border whose sides all match, or `None` for uneven sides.
fn uniform_border(style: &ComputedStyle, root: f32) -> Option<Stroke> {
    let widths = style.border_widths(root);
    let colors = style
        .border_color
        .map(|ink| paint_color(ink, style.opacity));
    if widths.iter().all(|&width| width == 0.0) {
        Some(Stroke::NONE)
    } else if widths.iter().all(|&width| width == widths[0])
        && colors.iter().all(|&ink| ink == colors[0])
    {
        Some(Stroke::new(widths[0], colors[0]))
    } else {
        None
    }
}

/// A border inside `rect`: one rounded stroke when every side matches, or a
/// band per side with mitred corners. Corners of uneven borders stay square.
pub(super) fn border_shape(
    style: &ComputedStyle,
    rect: egui::Rect,
    radius: egui::epaint::CornerRadiusF32,
    root: f32,
) -> Option<egui::Shape> {
    if let Some(stroke) = uniform_border(style, root) {
        return (!stroke.is_empty()).then(|| {
            egui::epaint::RectShape::stroke(rect, radius, stroke, egui::StrokeKind::Inside).into()
        });
    }
    let widths = style.border_widths(root);
    let colors = style
        .border_color
        .map(|ink| paint_color(ink, style.opacity));
    let [top, right, bottom, left] = widths;
    let inner = egui::Rect::from_min_max(
        egui::pos2(
            (rect.left() + left).min(rect.right()),
            (rect.top() + top).min(rect.bottom()),
        ),
        egui::pos2(
            (rect.right() - right)
                .max(rect.left() + left)
                .min(rect.right()),
            (rect.bottom() - bottom)
                .max(rect.top() + top)
                .min(rect.bottom()),
        ),
    );
    let bands = [
        [
            rect.left_top(),
            rect.right_top(),
            inner.right_top(),
            inner.left_top(),
        ],
        [
            rect.right_top(),
            rect.right_bottom(),
            inner.right_bottom(),
            inner.right_top(),
        ],
        [
            rect.right_bottom(),
            rect.left_bottom(),
            inner.left_bottom(),
            inner.right_bottom(),
        ],
        [
            rect.left_bottom(),
            rect.left_top(),
            inner.left_top(),
            inner.left_bottom(),
        ],
    ];
    Some(egui::Shape::Vec(
        bands
            .into_iter()
            .zip(widths)
            .zip(colors)
            .filter(|((_, width), _)| *width > 0.0)
            .map(|((points, _), ink)| {
                egui::Shape::convex_polygon(points.to_vec(), ink, Stroke::NONE)
            })
            .collect(),
    ))
}

/// The plane around `rect` in four non-overlapping bands. CSS never paints a
/// shadow under its own box, which shows whenever the box is not opaque.
pub(super) fn outside(rect: egui::Rect) -> [egui::Rect; 4] {
    [
        egui::Rect::everything_above(rect.top()),
        egui::Rect::everything_below(rect.bottom()),
        egui::Rect::from_x_y_ranges(f32::NEG_INFINITY..=rect.left(), rect.y_range()),
        egui::Rect::from_x_y_ranges(rect.right()..=f32::INFINITY, rect.y_range()),
    ]
}

/// The soft `box-shadow` silhouette painted behind a box or image.
pub(super) fn shadow_shape(
    style: &ComputedStyle,
    rect: egui::Rect,
    root: f32,
) -> Option<egui::epaint::RectShape> {
    let shadow = style.box_shadow?;
    let resolve = |value: Length| value.resolve(0.0, style.font_size, root);
    let offset = egui::vec2(resolve(shadow.offset_x), resolve(shadow.offset_y));
    let blur = resolve(shadow.blur).clamp(0.0, 64.0);
    let spread = resolve(shadow.spread)
        .clamp(-32.0, 32.0)
        .max(-rect.width().min(rect.height()) / 2.0);
    let radius = corner_radii(style, rect, root);
    // Spread grows rounded corners but keeps square corners sharp.
    let spread_corner = |r: f32| {
        let ratio = if spread > 0.0 && r < spread {
            1.0 + (r / spread - 1.0).powi(3)
        } else {
            1.0
        };
        (r + spread * ratio).max(0.0)
    };
    let shadow_radius = egui::epaint::CornerRadiusF32 {
        nw: spread_corner(radius.nw),
        ne: spread_corner(radius.ne),
        sw: spread_corner(radius.sw),
        se: spread_corner(radius.se),
    };
    // A CSS blur radius softens about that far on each side of the edge;
    // epaint feathers linearly across a band centred on it.
    Some(
        egui::epaint::RectShape::new(
            rect.expand(spread).translate(offset),
            shadow_radius,
            paint_color(shadow.color, style.opacity),
            Stroke::NONE,
            egui::StrokeKind::Inside,
        )
        .with_blur_width(blur * 2.0),
    )
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
    /// Top, right, bottom, left.
    pub(super) border: [f32; 4],
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
    pub(super) shadow_paint: Vec<egui::layers::ShapeIdx>,
    pub(super) paint: Option<egui::layers::ShapeIdx>,
    /// A table's cell placements, taken by each row as it opens.
    pub(super) table_rows: Vec<Vec<FlexPlacement>>,
    pub(super) table_row_index: usize,
    pub(super) table_row_gap: f32,
    pub(super) table_row_edge: f32,
    /// Cells spanning more rows, with the rows they still cover, and the bottom
    /// of the latest row; a table stretches these cells as rows close.
    pub(super) spanning: Vec<(BoxLayout, egui::Rect, u8)>,
    pub(super) last_row_bottom: f32,
    /// A table row, and the cells it stretches to its height when it closes.
    pub(super) table_row: bool,
    pub(super) row_cells: Vec<(BoxLayout, egui::Rect)>,
    /// This box floats in its parent; `margins` are top, right, bottom, left.
    pub(super) floating: FloatSide,
    pub(super) margins: [f32; 4],
    /// Margin boxes of child floats that later content flows beside, the
    /// lowest float edge (the box grows to contain its floats), and the top of
    /// the latest float, which later floats may not rise above.
    pub(super) floats: Vec<(egui::Rect, FloatSide)>,
    pub(super) float_bottom: f32,
    pub(super) float_top: f32,
    /// Whether a picture was painted inside this box, and the bottom of any
    /// absolutely positioned child holding one.
    pub(super) contains_image: bool,
    pub(super) absolute_image_bottom: f32,
    /// This box narrowed the paint clip for its contents (`overflow: hidden`).
    pub(super) clipped: bool,
    /// The content height when `height` makes it definite; children's
    /// percentage heights resolve against it.
    pub(super) definite_height: Option<f32>,
    /// Where absolutely positioned descendants are placed.
    pub(super) containing: Containing,
    /// This box is placed by its bottom edge, using its height from the
    /// previous frame.
    pub(super) anchored_bottom: bool,
    /// The least content height that `height` and `min-height` ask for.
    pub(super) min_height: f32,
}

/// The padding box that absolutely positioned descendants are placed in: the
/// nearest positioned ancestor's, or the page's first screen.
#[derive(Clone, Copy)]
pub(super) struct Containing {
    /// The box, whose previous size stands in for a height not yet known.
    pub(super) index: Option<usize>,
    pub(super) left: f32,
    pub(super) top: f32,
    pub(super) width: f32,
    /// The height when known before the box's content is laid out.
    pub(super) height: Option<f32>,
}

/// Floats a container remembers at once; older ones stop shaping later content.
const MAX_ACTIVE_FLOATS: usize = 64;

#[derive(Clone, Copy, Default)]
pub(super) struct FlexPlacement {
    pub(super) width: f32,
    pub(super) x: f32,
    /// Offset below the top of the item's line; only wrapped columns use it.
    pub(super) y: f32,
    pub(super) break_before: bool,
}

#[derive(Clone, Copy)]
pub(super) enum ChildPlacement {
    Flex(FlexPlacement),
    Grid { width: f32, x: f32, y: f32 },
    TableRow(usize),
}

pub(super) struct FlexLayout {
    pub(super) items: Vec<FlexPlacement>,
    pub(super) has_anonymous_text: bool,
}

/// Most columns a table lays out; cells past the limit share the last column.
const MAX_TABLE_COLUMNS: usize = 64;

/// The widest unbreakable run of glyphs on any row of `galley`.
fn widest_word(galley: &egui::Galley) -> f32 {
    let mut widest: f32 = 0.0;
    for row in &galley.rows {
        let mut start = None;
        for glyph in &row.glyphs {
            if glyph.chr.is_whitespace() {
                start = None;
            } else {
                let left = *start.get_or_insert(glyph.pos.x);
                widest = widest.max(glyph.max_x() - left);
            }
        }
    }
    widest
}

/// The narrowest and natural widths of a box's contents, as CSS min- and
/// max-content sizes: the widest word, picture or fixed-width box, and the
/// widest unwrapped line. Flex rows, table rows and runs of floats sum their
/// items, nested boxes add their padding, borders and margins, and absolutely
/// positioned boxes do not count. Also returns the box's `Close` index, or
/// `None` when the work budget ran out first. Measuring lays out copies, so it
/// never disturbs a block's cached lines.
pub(super) fn content_widths(
    commands: &[Command],
    boxes: &[ComputedStyle],
    open_index: usize,
    blocks: &[Block],
    images: &[RenderedImage],
    ui: &egui::Ui,
    remaining_steps: &mut usize,
) -> (f32, f32, Option<usize>) {
    /// One open box: its style, whether it lays items side by side, and the
    /// sizes gathered so far. Floats sum separately from in-flow content.
    struct Frame {
        style: ComputedStyle,
        row: bool,
        skip: bool,
        items: usize,
        min: f32,
        max: f32,
        floats: f32,
    }
    impl Frame {
        fn new(style: ComputedStyle, skip: bool) -> Self {
            Self {
                style,
                row: (style.display == Display::Flex && style.flex_direction.is_row())
                    || style.display == Display::TableRow,
                skip,
                items: 0,
                min: 0.0,
                max: 0.0,
                floats: 0.0,
            }
        }
        /// Add one item's outer sizes.
        fn add(&mut self, min: f32, max: f32, floating: bool) {
            if self.row {
                let gap = if self.items > 0 {
                    self.style
                        .column_gap
                        .resolve(0.0, self.style.font_size, 0.0)
                        .max(0.0)
                } else {
                    0.0
                };
                self.max += max + gap;
                self.min = if self.style.flex_wrap == FlexWrap::Wrap {
                    self.min.max(min)
                } else {
                    self.min + min + gap
                };
            } else {
                if floating {
                    self.floats += max;
                } else {
                    self.max = self.max.max(max);
                }
                self.min = self.min.max(min);
            }
            self.items += 1;
        }
        /// The content sizes, with grid tracks repeating the widest item.
        fn content(&self) -> (f32, f32) {
            let max = self.max.max(self.floats);
            if self.style.display == Display::Grid {
                let columns = self.style.grid_columns.clamp(1, 12) as f32;
                let gap = self
                    .style
                    .column_gap
                    .resolve(0.0, self.style.font_size, 0.0)
                    .max(0.0)
                    * (columns - 1.0);
                (self.min * columns + gap, max * columns + gap)
            } else {
                (self.min, max)
            }
        }
    }
    /// A box's outer sizes from its content sizes.
    fn outer(frame: &Frame) -> (f32, f32) {
        let style = frame.style;
        let resolve = |value: Length| match value {
            Length::Auto => 0.0,
            value => value.resolve(0.0, style.font_size, 0.0),
        };
        let border = style.border_widths(0.0);
        let sides = resolve(style.padding[1]) + resolve(style.padding[3]) + border[1] + border[3];
        let (mut min, mut max) = frame.content();
        let fixed = |length: Length| match length {
            Length::Px(width) => Some(width - if style.border_box { sides } else { 0.0 }),
            _ => None,
        };
        if let Some(width) = fixed(style.width) {
            min = width.max(0.0);
            max = min;
        }
        if let Some(limit) = fixed(style.max_width) {
            max = max.min(limit.max(0.0));
            min = min.min(max);
        }
        if let Some(least) = fixed(style.min_width) {
            min = min.max(least);
            max = max.max(least);
        }
        let extra = sides + resolve(style.margin[1]) + resolve(style.margin[3]);
        ((min + extra).max(0.0), (max + extra).max(0.0))
    }
    let root = match commands.get(open_index) {
        Some(&Command::Open(index)) => boxes[index],
        _ => ComputedStyle::default(),
    };
    let mut stack = vec![Frame::new(root, false)];
    for (command_index, command) in commands.iter().enumerate().skip(open_index + 1) {
        if *remaining_steps == 0 {
            break;
        }
        *remaining_steps -= 1;
        let skip = stack.last().is_some_and(|frame| frame.skip);
        match *command {
            Command::Open(index) => {
                let child = boxes[index];
                stack.push(Frame::new(child, skip || child.position.out_of_flow()));
            }
            Command::Close => {
                let frame = stack.pop().unwrap();
                let Some(parent) = stack.last_mut() else {
                    let (min, max) = frame.content();
                    return (min, max, Some(command_index));
                };
                if !frame.skip {
                    let (min, max) = outer(&frame);
                    parent.add(min, max, frame.style.float_side != FloatSide::None);
                }
            }
            Command::Text(index) => {
                let block = &blocks[index];
                let cost = block.job.text.len();
                if cost > *remaining_steps {
                    break;
                }
                *remaining_steps -= cost;
                if skip || block.rule || block.marker {
                    continue;
                }
                let mut job = block.job.clone();
                job.wrap.max_width = f32::INFINITY;
                job.halign = egui::Align::Min;
                let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
                // Round up so that text laid out at exactly this width, at any
                // display scale, does not wrap its last word.
                let line = galley.size().x.ceil();
                let word = if matches!(
                    block.style.white_space,
                    WhiteSpace::Pre | WhiteSpace::NoWrap
                ) {
                    line
                } else {
                    widest_word(&galley).ceil()
                };
                // Text in a row container still forms one line of its own.
                let frame = stack.last_mut().unwrap();
                frame.max = frame.max.max(line);
                frame.min = frame.min.max(word);
            }
            Command::Image(index) if !skip => {
                let image = &images[index];
                let natural = image.html_width.unwrap_or(image.image.size[0] as f32);
                stack.last_mut().unwrap().add(natural, natural, false);
            }
            Command::Control(_, _) if !skip => stack.last_mut().unwrap().add(120.0, 120.0, false),
            _ => {}
        }
    }
    let (min, max) = stack[0].content();
    (min, max, None)
}

/// Measure a block flex item's contents without wrapping so auto-sized list
/// items keep their intrinsic width instead of stretching across the row.
pub(super) fn intrinsic_width(
    commands: &[Command],
    boxes: &[ComputedStyle],
    open_index: usize,
    blocks: &[Block],
    images: &[RenderedImage],
    ui: &egui::Ui,
    remaining_steps: &mut usize,
) -> f32 {
    content_widths(
        commands,
        boxes,
        open_index,
        blocks,
        images,
        ui,
        remaining_steps,
    )
    .1
    .max(1.0)
}

/// Where each row's cells go, in row and document order, the content width
/// the columns and their spacing take, and the vertical spacing between rows
/// and at the table's edges. Collapsed borders make the spacing negative so
/// that neighboring borders overlap and draw once.
pub(super) struct TableLayout {
    pub(super) rows: Vec<Vec<FlexPlacement>>,
    pub(super) width: f32,
    pub(super) row_gap: f32,
    pub(super) row_edge: f32,
}

/// Size a table's columns from its cells, following the CSS automatic table
/// layout: every column gets at least its widest word, and the room left is
/// shared in proportion to how much more each column's unwrapped text needs.
/// `room` is the content width available, or the table's own width when
/// `fixed`. Spans, `rowspan` and right-to-left column order are honored.
// These borrowed inputs are one render pass's shared measurement context.
#[allow(clippy::too_many_arguments)]
pub(super) fn table_layout(
    commands: &[Command],
    boxes: &[ComputedStyle],
    blocks: &[Block],
    images: &[RenderedImage],
    ui: &egui::Ui,
    open_index: usize,
    style: ComputedStyle,
    room: f32,
    fixed: bool,
    root: f32,
    remaining_steps: &mut usize,
) -> TableLayout {
    struct Cell {
        row: usize,
        column: usize,
        span: usize,
        min: f32,
        max: f32,
        sides: f32,
        specified: bool,
    }
    let mut cells = Vec::new();
    // The widest horizontal and vertical cell borders, which collapse into
    // the space between columns and rows.
    let mut cell_border = [0.0f32; 2];
    let mut rows = 0;
    let mut row_open = false;
    // Rows each column stays covered by an earlier cell's `rowspan`.
    let mut covered: Vec<u8> = Vec::new();
    let mut next_column = 0;
    let mut columns = 0;
    let mut depth = 0usize;
    let mut index = open_index + 1;
    while index < commands.len() && *remaining_steps > 0 {
        *remaining_steps -= 1;
        match commands[index] {
            Command::Open(box_index) => {
                let child = boxes[box_index];
                let in_flow = !child.position.out_of_flow();
                if depth == 0 && in_flow && child.display == Display::TableRow {
                    if rows > 0 {
                        for count in &mut covered {
                            *count = count.saturating_sub(1);
                        }
                    }
                    rows += 1;
                    row_open = true;
                    next_column = 0;
                } else if depth == 1 && row_open && in_flow && child.display == Display::TableCell {
                    let resolve = |value: Length| value.resolve(room, child.font_size, root);
                    let border = child.border_widths(root);
                    cell_border[0] = cell_border[0].max(border[1]).max(border[3]);
                    cell_border[1] = cell_border[1].max(border[0]).max(border[2]);
                    let sides = resolve(child.padding[1])
                        + resolve(child.padding[3])
                        + border[1]
                        + border[3];
                    let (min, mut max, end) =
                        content_widths(commands, boxes, index, blocks, images, ui, remaining_steps);
                    let Some(end) = end else {
                        break;
                    };
                    let specified = child.width != Length::Auto;
                    if specified {
                        // A cell's width is a preference that its content can exceed.
                        let width =
                            resolve(child.width) - if child.border_box { sides } else { 0.0 };
                        max = width.max(min);
                    }
                    while covered.get(next_column).is_some_and(|&count| count > 0) {
                        next_column += 1;
                    }
                    let column = next_column.min(MAX_TABLE_COLUMNS - 1);
                    let span =
                        usize::from(child.column_span.max(1)).min(MAX_TABLE_COLUMNS - column);
                    if covered.len() < column + span {
                        covered.resize(column + span, 0);
                    }
                    covered[column..column + span].fill(child.row_span.max(1));
                    next_column = column + span;
                    columns = columns.max(column + span);
                    cells.push(Cell {
                        row: rows - 1,
                        column,
                        span,
                        min: min + sides,
                        max: max.max(min) + sides,
                        sides,
                        specified,
                    });
                    // The cell's contents were measured; continue after it.
                    index = end + 1;
                    continue;
                }
                depth += 1;
            }
            Command::Close if depth == 0 => break,
            Command::Close => {
                depth -= 1;
                if depth == 0 {
                    row_open = false;
                }
            }
            _ => {}
        }
        index += 1;
    }

    let spacing = |gap: Length| gap.resolve(0.0, style.font_size, root).max(0.0);
    let table_border = style.border_widths(root);
    let (gap, edge, row_gap, row_edge) = if style.border_collapse {
        (
            -cell_border[0],
            -table_border[1].max(table_border[3]),
            -cell_border[1],
            -table_border[0].max(table_border[2]),
        )
    } else {
        let (column, row) = (spacing(style.column_gap), spacing(style.row_gap));
        (column, column, row, row)
    };
    let mut min = vec![0.0f32; columns];
    let mut max = vec![0.0f32; columns];
    let mut specified = vec![false; columns];
    for cell in cells.iter().filter(|cell| cell.span == 1) {
        min[cell.column] = min[cell.column].max(cell.min);
        max[cell.column] = max[cell.column].max(cell.max);
        specified[cell.column] |= cell.specified;
    }
    // Spanning cells share any extra width they need across their columns.
    for cell in cells.iter().filter(|cell| cell.span > 1) {
        let range = cell.column..cell.column + cell.span;
        let inner = gap * (cell.span - 1) as f32;
        for (widths, wanted) in [(&mut min, cell.min), (&mut max, cell.max)] {
            let current = widths[range.clone()].iter().sum::<f32>() + inner;
            if wanted > current {
                let extra = (wanted - current) / cell.span as f32;
                for width in &mut widths[range.clone()] {
                    *width += extra;
                }
            }
        }
    }
    for (low, high) in min.iter().zip(&mut max) {
        *high = high.max(*low);
    }
    let edges = 2.0 * edge + gap * columns.saturating_sub(1) as f32;
    let min_total = min.iter().sum::<f32>();
    let max_total = max.iter().sum::<f32>();
    let room = (room - edges).max(0.0);
    let target = if fixed {
        room.max(min_total)
    } else {
        max_total.min(room).max(min_total)
    };
    let widths: Vec<f32> = if target >= max_total {
        // Extra room goes to columns without a specified width when possible.
        let flexible = |column: usize| !specified[column] || specified.iter().all(|&s| s);
        let share = (0..columns)
            .filter(|&column| flexible(column))
            .map(|column| max[column])
            .sum::<f32>();
        let count = (0..columns)
            .filter(|&column| flexible(column))
            .count()
            .max(1);
        (0..columns)
            .map(|column| {
                let extra = target - max_total;
                max[column]
                    + match (flexible(column), share > 0.0) {
                        (false, _) => 0.0,
                        (true, true) => extra * max[column] / share,
                        (true, false) => extra / count as f32,
                    }
            })
            .collect()
    } else if max_total > min_total {
        let fraction = (target - min_total) / (max_total - min_total);
        min.iter()
            .zip(&max)
            .map(|(low, high)| low + (high - low) * fraction)
            .collect()
    } else {
        min.clone()
    };
    let mut lefts = Vec::with_capacity(columns + 1);
    let mut x = edge;
    for width in &widths {
        lefts.push(x);
        x += width + gap;
    }
    lefts.push(x);
    let width = if columns == 0 { 0.0 } else { x - gap + edge };
    let rtl = style.direction == Direction::Rtl;
    let mut placements = vec![Vec::new(); rows];
    for cell in &cells {
        let left = lefts[cell.column];
        let outer = lefts[cell.column + cell.span] - gap - left;
        placements[cell.row].push(FlexPlacement {
            width: (outer - cell.sides).max(1.0),
            x: if rtl { width - left - outer } else { left },
            ..Default::default()
        });
    }
    TableLayout {
        rows: placements,
        width,
        row_gap,
        row_edge,
    }
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
    blocks: &[Block],
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
                if !child.position.out_of_flow() {
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
                    let border = child.border_widths(root);
                    let border = border[1] + border[3];
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
                                boxes,
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
        let reverse = style.flex_direction == crate::css::FlexDirection::RowReverse;
        let mut x = start;
        for ((item, width), item_index) in row.iter().zip(widths).zip(0..) {
            // `row-reverse` runs the same line from the right edge.
            let outer_left = if reverse {
                available_width - x - width - item.extra
            } else {
                x
            };
            placements.push(FlexPlacement {
                width,
                x: outer_left + item.margin_left,
                y: 0.0,
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

/// Item placements for `flex-flow: column wrap` in a container of fixed
/// height whose items all have fixed heights, so columns can be filled before
/// the items are laid out. Other column containers stack their items.
// These borrowed inputs are one render pass's shared measurement context.
#[allow(clippy::too_many_arguments)]
fn flex_column_wrap(
    commands: &[Command],
    boxes: &[ComputedStyle],
    blocks: &[Block],
    images: &[RenderedImage],
    ui: &egui::Ui,
    open_index: usize,
    style: ComputedStyle,
    available_width: f32,
    root: f32,
    remaining_steps: &mut usize,
) -> Option<Vec<FlexPlacement>> {
    let Length::Px(height) = style.height else {
        return None;
    };
    if style.flex_wrap != FlexWrap::Wrap {
        return None;
    }
    let border = style.border_widths(root);
    let resolve = |length: Length| length.resolve(available_width, style.font_size, root);
    let height = if style.border_box {
        height - resolve(style.padding[0]) - resolve(style.padding[2]) - border[0] - border[2]
    } else {
        height
    }
    .max(0.0);
    // Each item's width, outer width and outer height.
    let mut items = Vec::new();
    let mut depth = 0usize;
    for (command_index, command) in commands.iter().enumerate().skip(open_index + 1) {
        *remaining_steps = remaining_steps.checked_sub(1)?;
        match *command {
            Command::Text(_) | Command::Image(_) | Command::Control(..) if depth == 0 => {
                return None;
            }
            Command::Open(index) if depth == 0 => {
                depth += 1;
                let child = boxes[index];
                if child.position.out_of_flow() {
                    continue;
                }
                let Length::Px(child_height) = child.height else {
                    return None;
                };
                let resolve = |length: Length| match length {
                    Length::Auto => 0.0,
                    length => length.resolve(available_width, child.font_size, root),
                };
                let border = child.border_widths(root);
                let horizontal =
                    resolve(child.padding[1]) + resolve(child.padding[3]) + border[1] + border[3];
                let vertical =
                    resolve(child.padding[0]) + resolve(child.padding[2]) + border[0] + border[2];
                let width = match child.width {
                    Length::Auto => intrinsic_width(
                        commands,
                        boxes,
                        command_index,
                        blocks,
                        images,
                        ui,
                        remaining_steps,
                    ),
                    width => resolve(width) - if child.border_box { horizontal } else { 0.0 },
                }
                .max(1.0);
                let outer_height = if child.border_box {
                    child_height
                } else {
                    child_height + vertical
                } + resolve(child.margin[0])
                    + resolve(child.margin[2]);
                let outer_width =
                    width + horizontal + resolve(child.margin[1]) + resolve(child.margin[3]);
                items.push((
                    width,
                    outer_width,
                    outer_height.max(0.0),
                    resolve(child.margin[3]),
                ));
            }
            Command::Open(_) => depth += 1,
            Command::Close if depth == 0 => break,
            Command::Close => depth -= 1,
            _ => {}
        }
    }
    // Fill each column down to the container's height, then start another.
    let gap = style
        .column_gap
        .resolve(0.0, style.font_size, root)
        .max(0.0);
    let row_gap = style.row_gap.resolve(0.0, style.font_size, root).max(0.0);
    let mut columns: Vec<Vec<usize>> = Vec::new();
    let mut used = 0.0;
    for (index, &(_, _, outer_height, _)) in items.iter().enumerate() {
        match columns.last_mut() {
            Some(column) if used + row_gap + outer_height <= height + 0.5 => {
                column.push(index);
                used += row_gap + outer_height;
            }
            _ => {
                columns.push(vec![index]);
                used = outer_height;
            }
        }
    }
    let mut placements = vec![FlexPlacement::default(); items.len()];
    let mut x = 0.0;
    for column in columns {
        let column_width = column
            .iter()
            .map(|&index| items[index].1)
            .fold(0.0, f32::max);
        let column_height = column.iter().map(|&index| items[index].2).sum::<f32>()
            + row_gap * column.len().saturating_sub(1) as f32;
        let mut y = match style.justify_content {
            JustifyContent::Center => (height - column_height) / 2.0,
            JustifyContent::End => height - column_height,
            _ => 0.0,
        }
        .max(0.0);
        for index in column {
            let (width, _, outer_height, margin_left) = items[index];
            placements[index] = FlexPlacement {
                width,
                x: x + margin_left,
                y,
                break_before: false,
            };
            y += outer_height + row_gap;
        }
        x += column_width + gap;
    }
    Some(placements)
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
            border: [0.0; 4],
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
            shadow_paint: Vec::new(),
            paint: None,
            table_rows: Vec::new(),
            table_row_index: 0,
            table_row_gap: 0.0,
            table_row_edge: 0.0,
            spanning: Vec::new(),
            last_row_bottom: 0.0,
            table_row: false,
            row_cells: Vec::new(),
            floating: FloatSide::None,
            margins: [0.0; 4],
            floats: Vec::new(),
            float_bottom: f32::NEG_INFINITY,
            float_top: f32::NEG_INFINITY,
            contains_image: false,
            absolute_image_bottom: f32::NEG_INFINITY,
            clipped: false,
            definite_height: None,
            containing: Containing {
                index: None,
                left: start.x,
                top: start.y,
                width,
                height: None,
            },
            anchored_bottom: false,
            min_height: 0.0,
        }
    }

    /// Where `overflow: hidden` clips this box's contents: its border box
    /// horizontally, and vertically when its height is fixed.
    pub(super) fn overflow_clip(&self, root: f32) -> Option<egui::Rect> {
        let [horizontal, vertical] = self.style.overflow_hidden;
        if !horizontal && !vertical {
            return None;
        }
        let x = if horizontal {
            egui::Rangef::new(self.left, self.left + self.outer_width)
        } else {
            egui::Rangef::EVERYTHING
        };
        let y = match self.style.height {
            Length::Auto | Length::Percent(_) => egui::Rangef::EVERYTHING,
            height if vertical => {
                let height = height.resolve(0.0, self.style.font_size, root).max(0.0);
                let outer = if self.style.border_box {
                    height
                } else {
                    height + (self.content_top - self.top) + self.padding_bottom + self.border[2]
                };
                egui::Rangef::new(self.top, self.top + outer)
            }
            _ => egui::Rangef::EVERYTHING,
        };
        Some(egui::Rect::from_x_y_ranges(x, y))
    }

    /// Whether children of this box are in normal block flow, where floats apply.
    pub(super) fn flows_floats(&self) -> bool {
        !self.flex_row
            && !self.flex_column
            && !self.grid
            && !self.table_row
            && self.style.display != Display::Table
    }

    /// Room taken by floats at the start and end of the line at `y`.
    pub(super) fn float_insets(&self, y: f32) -> (f32, f32) {
        let (mut left, mut right): (f32, f32) = (0.0, 0.0);
        for (rect, side) in &self.floats {
            if rect.top() <= y && y < rect.bottom() {
                match side {
                    FloatSide::Left => left = left.max(rect.right() - self.content_left),
                    FloatSide::Right => {
                        right = right.max(self.content_left + self.width - rect.left())
                    }
                    FloatSide::None => {}
                }
            }
        }
        (left.max(0.0), right.max(0.0))
    }

    /// Move the flow below earlier floats on the cleared sides.
    pub(super) fn clear(&mut self, clear: Clear) {
        for (rect, side) in &self.floats {
            if matches!(
                (clear, side),
                (Clear::Both, _)
                    | (Clear::Left, FloatSide::Left)
                    | (Clear::Right, FloatSide::Right)
            ) {
                self.cursor = self.cursor.max(rect.bottom());
            }
        }
    }

    /// Where a float `outer` wide (with margins) goes: beside earlier floats at
    /// the current line, or below them when the line is too full.
    pub(super) fn place_float(&self, side: FloatSide, outer: f32) -> (f32, f32) {
        let mut y = self.cursor.max(self.float_top);
        let (mut left, mut right) = self.float_insets(y);
        for _ in 0..=MAX_ACTIVE_FLOATS {
            // Allow for rounding when floats sized to their content line up.
            if outer <= self.width - left - right + 0.5 || (left == 0.0 && right == 0.0) {
                break;
            }
            // Try again where the soonest-ending float at this height ends.
            y = self
                .floats
                .iter()
                .filter(|(rect, _)| rect.top() <= y && y < rect.bottom())
                .map(|(rect, _)| rect.bottom())
                .fold(f32::INFINITY, f32::min);
            (left, right) = self.float_insets(y);
        }
        let x = if side == FloatSide::Right {
            self.content_left + self.width - right - outer
        } else {
            self.content_left + left
        };
        (x, y)
    }

    /// Remember a finished child float so later content flows beside it.
    pub(super) fn add_float(&mut self, rect: egui::Rect, side: FloatSide) {
        let cursor = self.cursor;
        self.floats
            .retain(|(previous, _)| previous.bottom() > cursor);
        if self.floats.len() == MAX_ACTIVE_FLOATS {
            self.floats.remove(0);
        }
        self.floats.push((rect, side));
        self.float_bottom = self.float_bottom.max(rect.bottom());
        self.float_top = rect.top();
    }

    pub(super) fn child_placement(&mut self, child: ComputedStyle) -> Option<ChildPlacement> {
        if child.position.out_of_flow() {
            return None;
        }
        if self.style.display == Display::Table && child.display == Display::TableRow {
            // Rows past the measured ones (when the work budget ran out) stack their cells.
            let index = self.table_row_index;
            self.table_row_index += 1;
            return (index < self.table_rows.len()).then_some(ChildPlacement::TableRow(index));
        }
        if self.table_row && child.display != Display::TableCell {
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
        viewport: egui::Rect,
        previous: &[egui::Rect],
        commands: &[Command],
        boxes: &[ComputedStyle],
        blocks: &[Block],
        images: &[RenderedImage],
        command_index: usize,
        remaining_steps: &mut usize,
    ) -> Self {
        // Fixed boxes are placed against the page's first screen instead of
        // their parent. Olive does not keep them in view while scrolling.
        let fixed = style.position == Position::Fixed;
        let (container_left, container_top, container_width) = if fixed {
            (viewport.left(), viewport.top(), viewport.width())
        } else if style.position == Position::Absolute {
            let containing = parent.containing;
            (containing.left, containing.top, containing.width)
        } else {
            (parent.content_left, parent.content_top, parent.width)
        };
        let basis = container_width;
        let resolve = |v: Length| v.resolve(basis, style.font_size, root);
        let margin = style.margin.map(resolve);
        let padding = style.padding.map(resolve);
        let border = style.border_widths(root);
        let sides = padding[1] + padding[3] + border[1] + border[3];
        // Normal-flow boxes clear earlier floats when asked, and otherwise take
        // the room beside any floats at their top.
        let in_flow = placement.is_none() && !style.position.out_of_flow() && parent.flows_floats();
        if in_flow && style.clear != Clear::None {
            parent.clear(style.clear);
        }
        let floating = if in_flow {
            style.float_side
        } else {
            FloatSide::None
        };
        let (inset_left, inset_right) = if in_flow && floating == FloatSide::None {
            parent.float_insets(parent.cursor)
        } else {
            (0.0, 0.0)
        };
        // An absolutely positioned box has the room its insets leave.
        let (inset_left, inset_right) = if style.position.out_of_flow() {
            let inset = |side: usize| match style.inset[side] {
                Length::Auto => 0.0,
                length => resolve(length),
            };
            (inset(3), inset(1))
        } else {
            (inset_left, inset_right)
        };
        let available =
            (container_width - inset_left - inset_right - margin[1] - margin[3] - sides).max(1.0);
        let assigned = match placement {
            Some(ChildPlacement::Flex(item)) => Some(item.width),
            Some(ChildPlacement::Grid { width, .. }) if style.width == Length::Auto => {
                Some((width - margin[1] - margin[3] - sides).max(1.0))
            }
            Some(ChildPlacement::Grid { .. } | ChildPlacement::TableRow(_)) | None => None,
        };
        let width = if let Some(assigned) = assigned {
            assigned.min(available)
        } else if style.width == Length::Auto {
            if style.position.out_of_flow()
                && style.inset[1] != Length::Auto
                && style.inset[3] != Length::Auto
            {
                // Pinned to both sides, it spans the room between them.
                available
            } else if style.position.out_of_flow() {
                intrinsic_width(
                    commands,
                    boxes,
                    command_index,
                    blocks,
                    images,
                    ui,
                    remaining_steps,
                )
                .min(available)
            } else if floating != FloatSide::None {
                // Floats shrink to fit their content.
                let (min, max, _) = content_widths(
                    commands,
                    boxes,
                    command_index,
                    blocks,
                    images,
                    ui,
                    remaining_steps,
                );
                max.min(available).max(min).max(1.0)
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
        let table = (style.display == Display::Table).then(|| {
            table_layout(
                commands,
                boxes,
                blocks,
                images,
                ui,
                command_index,
                style,
                width,
                style.width != Length::Auto,
                root,
                remaining_steps,
            )
        });
        // Tables shrink to their columns unless sized, and grow to fit content.
        let width = match &table {
            Some(table) if style.width == Length::Auto => table.width.max(1.0),
            Some(table) => width.max(table.width),
            None => width,
        };
        let spare = (available - width).max(0.0);
        let left_auto = style.margin[3] == Length::Auto;
        let right_auto = style.margin[1] == Length::Auto;
        let left_margin = margin[3]
            + if left_auto {
                spare / if right_auto { 2.0 } else { 1.0 }
            } else if !right_auto && parent.style.direction == Direction::Rtl {
                // A narrower box starts at the right edge of right-to-left flow.
                spare
            } else {
                0.0
            };
        let flow_top = match placement {
            Some(ChildPlacement::Flex(item)) => {
                parent.content_top + parent.flex_y + item.y + margin[0]
            }
            Some(ChildPlacement::Grid { y, .. }) => parent.content_top + y + margin[0],
            Some(ChildPlacement::TableRow(row)) => {
                parent.cursor
                    + if row == 0 {
                        parent.table_row_edge
                    } else {
                        parent.table_row_gap
                    }
                    + margin[0]
            }
            None => parent.cursor + margin[0],
        };
        let (normal_left, normal_top) = match placement {
            Some(ChildPlacement::Flex(item)) => (parent.content_left + item.x, flow_top),
            Some(ChildPlacement::Grid { x, .. }) => (parent.content_left + x + margin[3], flow_top),
            Some(ChildPlacement::TableRow(_)) => (parent.content_left + left_margin, flow_top),
            None if floating != FloatSide::None => {
                let (x, y) = parent.place_float(floating, width + sides + margin[1] + margin[3]);
                (x + margin[3], y + margin[0])
            }
            None => (parent.content_left + inset_left + left_margin, flow_top),
        };
        let flow_top = if floating == FloatSide::None {
            flow_top
        } else {
            normal_top
        };
        let row_cells = match placement {
            Some(ChildPlacement::TableRow(row)) => {
                Some(std::mem::take(&mut parent.table_rows[row]))
            }
            _ => None,
        };
        let absolute = style.position.out_of_flow();
        let left = if absolute {
            if style.inset[3] != Length::Auto {
                container_left + resolve(style.inset[3])
            } else if style.inset[1] != Length::Auto {
                container_left + container_width - resolve(style.inset[1]) - width - sides
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
        // A box placed by its bottom edge needs its own height and that of its
        // containing block. Heights not known in advance come from the
        // previous frame, so the box settles on the frame after it first shows.
        let containing_height = if fixed {
            Some(viewport.height())
        } else {
            parent.containing.height.or_else(|| {
                let index = parent.containing.index?;
                let rect = previous.get(index).filter(|rect| rect.is_finite())?;
                let border = boxes[index].border_widths(root);
                Some(rect.height() - border[0] - border[2])
            })
        };
        let anchored_bottom =
            absolute && style.inset[2] != Length::Auto && containing_height.is_some();
        let top = if absolute {
            if style.inset[0] != Length::Auto {
                container_top + resolve(style.inset[0])
            } else if let (true, Some(containing_height)) = (anchored_bottom, containing_height) {
                let height = previous
                    .get(index)
                    .filter(|rect| rect.is_finite())
                    .map_or(0.0, |rect| rect.height());
                container_top + containing_height - resolve(style.inset[2]) - margin[2] - height
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
        let content_top = top + border[0] + padding[0];
        // Percentage heights need a containing block whose height is known
        // before its content is laid out.
        let height_basis = if fixed {
            Some(viewport.height())
        } else if absolute {
            parent.containing.height
        } else {
            parent.definite_height
        };
        let vertical_sides = padding[0] + padding[2] + border[0] + border[2];
        let used_height = |length: Length| {
            let height = match length {
                Length::Auto => None,
                length if length.has_percent() => {
                    height_basis.map(|basis| length.resolve(basis, style.font_size, root))
                }
                length => Some(length.resolve(0.0, style.font_size, root)),
            }?;
            Some(
                if style.border_box {
                    height - vertical_sides
                } else {
                    height
                }
                .max(0.0),
            )
        };
        // Pinned to the top and bottom of a containing block of known
        // height, a box spans the height between them.
        let definite_height = used_height(style.height).or_else(|| {
            if !absolute || style.inset[0] == Length::Auto || style.inset[2] == Length::Auto {
                return None;
            }
            let room = containing_height?
                - resolve(style.inset[0])
                - resolve(style.inset[2])
                - margin[0]
                - margin[2]
                - vertical_sides;
            Some(room.max(0.0))
        });
        let grid = style.display == Display::Grid
            && !has_direct_text(commands, command_index, remaining_steps);
        let row_flex = style.display == Display::Flex && style.flex_direction.is_row();
        let mut has_anonymous_text = false;
        let column_wrap = (style.display == Display::Flex && !row_flex)
            .then(|| {
                flex_column_wrap(
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
                )
            })
            .flatten();
        let row_flex = row_flex || column_wrap.is_some();
        let flex_items = if let Some(items) = column_wrap {
            items
        } else if row_flex {
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
            content_left: left + border[3] + padding[3],
            width,
            cursor: content_top,
            flow_top,
            content_top,
            padding_bottom: padding[2],
            border,
            margin_bottom: margin[2],
            // A table row places its cells side by side like a flex row.
            flex_row: (row_flex && !has_anonymous_text)
                || row_cells.as_ref().is_some_and(|cells| !cells.is_empty()),
            flex_column: style.display == Display::Flex
                && (!style.flex_direction.is_row() || has_anonymous_text),
            table_row: row_cells.is_some(),
            flex_items: row_cells.unwrap_or(flex_items),
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
            // A see-through box needs its shadow clipped to the four bands around it.
            shadow_paint: match style.box_shadow {
                None => Vec::new(),
                Some(_) if style.background.3 == 255 && style.opacity >= 1.0 => {
                    vec![ui.painter().add(egui::Shape::Noop)]
                }
                Some(_) => (0..4)
                    .map(|_| ui.painter().add(egui::Shape::Noop))
                    .collect(),
            },
            paint: Some(ui.painter().add(egui::Shape::Noop)),
            table_row_gap: table.as_ref().map_or(0.0, |table| table.row_gap),
            table_row_edge: table.as_ref().map_or(0.0, |table| table.row_edge),
            table_rows: table.map(|table| table.rows).unwrap_or_default(),
            table_row_index: 0,
            spanning: Vec::new(),
            last_row_bottom: 0.0,
            row_cells: Vec::new(),
            floating,
            margins: margin,
            floats: Vec::new(),
            float_bottom: f32::NEG_INFINITY,
            float_top: f32::NEG_INFINITY,
            contains_image: false,
            absolute_image_bottom: f32::NEG_INFINITY,
            clipped: false,
            definite_height,
            containing: if style.position == Position::Static {
                parent.containing
            } else {
                Containing {
                    index: Some(index),
                    left: left + border[3],
                    top: top + border[0],
                    width: width + padding[1] + padding[3],
                    height: definite_height.map(|height| height + padding[0] + padding[2]),
                }
            },
            anchored_bottom,
            min_height: definite_height
                .unwrap_or(0.0)
                .max(used_height(style.min_height).unwrap_or(0.0)),
        }
    }
    pub(super) fn finish(&self, ui: &egui::Ui, root: f32) -> egui::Rect {
        let min_height = self.min_height;
        // Table spacing also separates the last row from the table's edge.
        let cursor = self.cursor
            + if self.table_row_index > 0 {
                self.table_row_edge
            } else {
                0.0
            };
        // An empty positioned frame around an absolutely placed picture is
        // usually sized by generated content (the padding aspect-ratio idiom),
        // which Olive does not render; let the picture give it a height.
        let empty = self.cursor <= self.content_top
            && self.flex_height == 0.0
            && self.grid_height == 0.0
            && self.definite_height.is_none()
            && self.style.position == Position::Relative;
        let bottom = cursor
            .max(self.float_bottom)
            .max(if empty {
                self.absolute_image_bottom
            } else {
                f32::NEG_INFINITY
            })
            .max(self.content_top + min_height)
            .max(self.content_top + self.flex_height)
            .max(self.content_top + self.grid_height)
            + self.padding_bottom
            + self.border[2];
        // A fixed height normally grows to fit, so differing fonts never make
        // text overlap; with clipped overflow it keeps its specified height.
        let bottom = match self.overflow_clip(root) {
            Some(clip) if clip.bottom().is_finite() => clip.bottom(),
            _ => bottom,
        };
        let rect = egui::Rect::from_min_max(
            egui::pos2(self.left, self.top),
            egui::pos2(self.left + self.outer_width, bottom),
        );
        self.paint(ui, rect, root);
        rect
    }

    /// Extend a table cell down to `bottom`, the end of the last row it covers.
    pub(super) fn stretch_cell(
        &self,
        ui: &egui::Ui,
        rect: egui::Rect,
        bottom: f32,
        root: f32,
        box_rects: &mut [egui::Rect],
    ) {
        if rect.bottom() < bottom {
            let stretched = rect.with_max_y(bottom);
            self.paint(ui, stretched, root);
            box_rects[self.index] = stretched;
        }
    }

    /// Paint the box's shadow, background and border into its reserved slots.
    pub(super) fn paint(&self, ui: &egui::Ui, rect: egui::Rect, root: f32) {
        let radius = corner_radii(&self.style, rect, root);
        if let Some(shape) = shadow_shape(&self.style, rect, root) {
            match self.shadow_paint[..] {
                [paint] => ui.painter().set(paint, shape),
                ref slots => {
                    for (&paint, clip) in slots.iter().zip(outside(rect)) {
                        ui.painter().with_clip_rect(clip).set(paint, shape.clone());
                    }
                }
            }
        }
        if let Some(paint) = self.paint {
            let fill = paint_color(self.style.background, self.style.opacity);
            // Matching sides draw as the background's own stroke.
            let shape = match uniform_border(&self.style, root) {
                Some(stroke) => egui::epaint::RectShape::new(
                    rect,
                    radius,
                    fill,
                    stroke,
                    egui::StrokeKind::Inside,
                )
                .into(),
                None => egui::Shape::Vec(
                    std::iter::once(egui::epaint::RectShape::filled(rect, radius, fill).into())
                        .chain(border_shape(&self.style, rect, radius, root))
                        .collect(),
                ),
            };
            ui.painter().set(paint, shape);
        }
    }
}
