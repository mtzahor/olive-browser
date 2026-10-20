//! CSS painting order for positioned boxes. Boxes paint in document order as
//! they are laid out; afterwards their shapes are reordered so positioned boxes
//! paint above in-flow content, grouped into stacking contexts by `z-index`.
use super::*;

/// Stacking contexts nested deeper than this paint with their parent context.
const MAX_CONTEXT_DEPTH: usize = 64;

/// One positioned box and the shapes it painted.
struct Layer {
    z_index: i32,
    /// Its document position, which orders boxes with equal `z-index`.
    order: usize,
    /// It starts a stacking context: a `z-index` other than `auto`, or fixed.
    context: bool,
    /// The stacking context it paints in; `None` is the page's root context.
    parent_context: Option<usize>,
    start: usize,
    /// The end of the box's own background, border and shadow shapes.
    background_end: usize,
    end: usize,
}

pub(super) struct Stacking {
    layer: egui::LayerId,
    page_start: usize,
    /// The end of the root element's background shapes.
    backdrop: Option<usize>,
    layers: Vec<Layer>,
    open: Vec<usize>,
}

impl Stacking {
    pub(super) fn new(ui: &egui::Ui) -> Self {
        let layer = ui.layer_id();
        Self {
            layer,
            page_start: shape_count(ui.ctx(), layer),
            backdrop: None,
            layers: Vec::new(),
            open: Vec::new(),
        }
    }

    /// Call before a box reserves its shapes; `order` is its document position.
    pub(super) fn open(&mut self, ui: &egui::Ui, style: &ComputedStyle, order: usize) {
        if style.position == Position::Static {
            return;
        }
        let contexts = self
            .open
            .iter()
            .filter(|&&index| self.layers[index].context)
            .count();
        let start = shape_count(ui.ctx(), self.layer);
        self.layers.push(Layer {
            z_index: style.z_index.unwrap_or(0),
            order,
            context: (style.z_index.is_some() || style.position == Position::Fixed)
                && contexts < MAX_CONTEXT_DEPTH,
            parent_context: self
                .open
                .iter()
                .rev()
                .copied()
                .find(|&index| self.layers[index].context),
            start,
            background_end: start,
            end: start,
        });
        self.open.push(self.layers.len() - 1);
    }

    /// Call once a box has reserved its background shapes.
    pub(super) fn opened(&mut self, ui: &egui::Ui, style: &ComputedStyle) {
        let painted = shape_count(ui.ctx(), self.layer);
        self.backdrop.get_or_insert(painted);
        if style.position != Position::Static {
            if let Some(&index) = self.open.last() {
                self.layers[index].background_end = painted;
            }
        }
    }

    /// Call after a box and its contents have painted.
    pub(super) fn close(&mut self, ui: &egui::Ui, style: &ComputedStyle) {
        if style.position != Position::Static {
            if let Some(index) = self.open.pop() {
                self.layers[index].end = shape_count(ui.ctx(), self.layer);
            }
        }
    }

    /// Reorder the page's shapes into CSS painting order.
    pub(super) fn finish(self, ui: &egui::Ui) {
        if self.layers.is_empty() {
            return;
        }
        let page_end = shape_count(ui.ctx(), self.layer);
        let start = self.page_start;
        let root = self.layers.len();
        // Each shape belongs to the innermost positioned box that painted it.
        let mut own = vec![Vec::new(); root + 1];
        let mut active: Vec<usize> = Vec::new();
        let mut next = 0;
        for index in start..page_end {
            while active
                .last()
                .is_some_and(|&layer| self.layers[layer].end <= index)
            {
                active.pop();
            }
            while next < root && self.layers[next].start <= index {
                if self.layers[next].end > index {
                    active.push(next);
                }
                next += 1;
            }
            own[active.last().copied().unwrap_or(root)].push(index);
        }
        let mut members = vec![Vec::new(); root + 1];
        for (index, layer) in self.layers.iter().enumerate() {
            members[layer.parent_context.unwrap_or(root)].push(index);
        }
        for list in &mut members {
            list.sort_by_key(|&index| (self.layers[index].z_index, self.layers[index].order));
        }
        let mut order = Vec::with_capacity(page_end - start);
        self.emit(None, &own, &members, &mut order);
        let Some(first) = order
            .iter()
            .enumerate()
            .find(|&(offset, &source)| source != start + offset)
            .map(|(offset, _)| start + offset)
        else {
            return;
        };
        ui.ctx().graphics_mut(|layers| {
            let list = layers.entry(self.layer);
            let shapes: Vec<egui::epaint::ClippedShape> = list
                .all_entries()
                .skip(first)
                .take(page_end - first)
                .cloned()
                .collect();
            for (offset, &source) in order[first - start..].iter().enumerate() {
                let shape = shapes[source - first].clone();
                list.mutate_shape(egui::layers::ShapeIdx(first + offset), |slot| {
                    *slot = shape;
                });
            }
        });
    }

    /// A stacking context paints its background, then members with a negative
    /// `z-index`, its in-flow content, and the remaining members.
    fn emit(
        &self,
        context: Option<usize>,
        own: &[Vec<usize>],
        members: &[Vec<usize>],
        order: &mut Vec<usize>,
    ) {
        let root = self.layers.len();
        let shapes = &own[context.unwrap_or(root)];
        let background_end = match context {
            Some(index) => self.layers[index].background_end,
            None => self.backdrop.unwrap_or(self.page_start),
        };
        let split = shapes.partition_point(|&index| index < background_end);
        order.extend_from_slice(&shapes[..split]);
        let list = &members[context.unwrap_or(root)];
        let below = list.partition_point(|&index| self.layers[index].z_index < 0);
        for &member in &list[..below] {
            self.emit_member(member, own, members, order);
        }
        order.extend_from_slice(&shapes[split..]);
        for &member in &list[below..] {
            self.emit_member(member, own, members, order);
        }
    }

    /// A box that starts no stacking context paints only its own shapes here;
    /// its positioned descendants take their places in the enclosing context.
    fn emit_member(
        &self,
        member: usize,
        own: &[Vec<usize>],
        members: &[Vec<usize>],
        order: &mut Vec<usize>,
    ) {
        if self.layers[member].context {
            self.emit(Some(member), own, members, order);
        } else {
            order.extend_from_slice(&own[member]);
        }
    }
}

fn shape_count(ctx: &egui::Context, layer: egui::LayerId) -> usize {
    ctx.graphics(|layers| layers.get(layer).map_or(0, |list| list.next_idx().0))
}
