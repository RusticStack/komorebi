//! SKUA PATCH (G3-F27): in-place re-render of notified cached views.
//!
//! Upstream marks every ancestor of a notified view dirty (`Window::mark_view_dirty`), so a
//! cursor blink inside a cached view still re-renders, lays out and prepaints every uncached
//! ancestor up to the window root. When every notified view lives inside a *top-level* cached
//! view (one whose ancestors are all uncached) and nothing else asks for a redraw (no refresh,
//! same viewport, no a11y tree, inspector, prompt or drag), the ancestors would render exactly
//! their last frame. This module re-renders only those cached views ("holes"), at their last
//! bounds and with their last context, and replays the root's last frame around them, the same
//! way a reused cached view replays its own range.
//!
//! Frame layout: every per-frame vector (dispatch nodes, hitboxes, scene operations, ...) is
//! appended in tree pre-order, so the root's range is `[prefix][hole 1][middle][hole 2]...`.
//! Prefix and middle segments are copied with their indices shifted; dispatch nodes keep their
//! parents (a node's parent is either in its own segment or in an earlier one).

use super::*;
use crate::StyleRefinement;

/// A top-level cached view of a frame and the context it was drawn in.
pub(crate) struct ViewSlot {
    view: AnyView,
    style: StyleRefinement,
    global_id: GlobalElementId,
    context: ViewSlotContext,
    bounds: Bounds<Pixels>,
    pub(crate) prepaint_range: Range<PrepaintStateIndex>,
    paint_range: Option<Range<PaintIndex>>,
    paint_content_mask: ContentMask<Pixels>,
    paint_opacity: f32,
}

/// The window state a cached view's `ViewElement::prepaint` ran in.
#[derive(Clone)]
pub(crate) struct ViewSlotContext {
    element_id_stack: ElementIdStack,
    text_style_stack: Vec<TextStyleRefinement>,
    content_mask: ContentMask<Pixels>,
    rem_size: Option<Pixels>,
    opacity: f32,
    parent_view: EntityId,
    /// The view element's own dispatch node (its range starts right after it).
    node: DispatchNodeId,
}

/// A hole being re-rendered: old slot index, new slot index, its prepainted element.
pub(crate) struct SplicedRoot {
    holes: Vec<(usize, usize, Option<AnyElement>)>,
    /// Old prepaint segment start -> new start, one per copied segment, in order.
    prepaint_segments: Vec<(Range<PrepaintStateIndex>, PrepaintStateIndex)>,
    /// Dispatch node segments: (old start, old end, new start).
    node_segments: Vec<(usize, usize, usize)>,
}

fn shift_prepaint(
    index: &PrepaintStateIndex,
    from: &PrepaintStateIndex,
    to: &PrepaintStateIndex,
) -> PrepaintStateIndex {
    PrepaintStateIndex {
        hitboxes_index: index.hitboxes_index - from.hitboxes_index + to.hitboxes_index,
        tooltips_index: index.tooltips_index - from.tooltips_index + to.tooltips_index,
        deferred_draws_index: index.deferred_draws_index - from.deferred_draws_index
            + to.deferred_draws_index,
        dispatch_tree_index: index.dispatch_tree_index - from.dispatch_tree_index
            + to.dispatch_tree_index,
        accessed_element_states_index: index.accessed_element_states_index
            - from.accessed_element_states_index
            + to.accessed_element_states_index,
        line_layout_index: index
            .line_layout_index
            .shifted(&from.line_layout_index, &to.line_layout_index),
    }
}

fn shift_paint(index: &PaintIndex, from: &PaintIndex, to: &PaintIndex) -> PaintIndex {
    PaintIndex {
        scene_index: index.scene_index - from.scene_index + to.scene_index,
        #[cfg(any(test, feature = "test-support"))]
        debug_bounds_index: index.debug_bounds_index - from.debug_bounds_index
            + to.debug_bounds_index,
        mouse_listeners_index: index.mouse_listeners_index - from.mouse_listeners_index
            + to.mouse_listeners_index,
        input_handlers_index: index.input_handlers_index - from.input_handlers_index
            + to.input_handlers_index,
        cursor_styles_index: index.cursor_styles_index - from.cursor_styles_index
            + to.cursor_styles_index,
        accessed_element_states_index: index.accessed_element_states_index
            - from.accessed_element_states_index
            + to.accessed_element_states_index,
        tab_handle_index: index.tab_handle_index - from.tab_handle_index + to.tab_handle_index,
        line_layout_index: index
            .line_layout_index
            .shifted(&from.line_layout_index, &to.line_layout_index),
        window_control_hitboxes_index: index.window_control_hitboxes_index
            - from.window_control_hitboxes_index
            + to.window_control_hitboxes_index,
    }
}

fn map_node(segments: &[(usize, usize, usize)], old: usize) -> Option<usize> {
    segments
        .iter()
        .find(|(start, end, _)| (*start..*end).contains(&old))
        .map(|(start, _, new_start)| old - start + new_start)
}

impl Window {
    /// The window control area under the mouse, as the platform's non-client hit test sees it
    /// (`on_hit_test_window_control`). For tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn window_control_area_under_mouse(&self) -> Option<WindowControlArea> {
        self.rendered_frame
            .window_control_hitboxes
            .iter()
            .find(|(_, hitbox)| self.mouse_hit_test.ids.contains(&hitbox.id))
            .map(|(area, _)| *area)
    }

    /// The context of a cached view about to prepaint, if it is a top-level cached view of a
    /// root drawn in full (nested cached views are covered by their top-level ancestor).
    pub(crate) fn view_slot_context(&self) -> Option<ViewSlotContext> {
        if !self.recording_view_slots
            || self.cached_view_depth > 0
            || !self.image_cache_stack.is_empty()
        {
            return None;
        }
        Some(ViewSlotContext {
            element_id_stack: self.element_id_stack.clone(),
            text_style_stack: self.text_style_stack.clone(),
            content_mask: self.content_mask(),
            rem_size: self.rem_size_override_stack.last().copied(),
            opacity: self.element_opacity,
            parent_view: *self.rendered_entity_stack.last()?,
            node: self.next_frame.dispatch_tree.active_node_id()?,
        })
    }

    pub(crate) fn push_view_slot(
        &mut self,
        context: ViewSlotContext,
        view: AnyView,
        style: StyleRefinement,
        global_id: GlobalElementId,
        bounds: Bounds<Pixels>,
        prepaint_range: Range<PrepaintStateIndex>,
    ) -> usize {
        let opacity = context.opacity;
        let content_mask = context.content_mask;
        self.next_frame.view_slots.push(ViewSlot {
            view,
            style,
            global_id,
            context,
            bounds,
            prepaint_range,
            paint_range: None,
            paint_content_mask: content_mask,
            paint_opacity: opacity,
        });
        self.next_frame.view_slots.len() - 1
    }

    pub(crate) fn finish_view_slot(
        &mut self,
        ix: usize,
        content_mask: ContentMask<Pixels>,
        opacity: f32,
        paint_range: Range<PaintIndex>,
    ) {
        if let Some(slot) = self.next_frame.view_slots.get_mut(ix) {
            slot.paint_range = Some(paint_range);
            slot.paint_content_mask = content_mask;
            slot.paint_opacity = opacity;
        }
    }

    /// The slots to re-render in place this frame, in tree order, or `None` for a full draw.
    pub(crate) fn plan_splice(&mut self, cx: &App) -> Option<SmallVec<[usize; 4]>> {
        if self.refreshing || self.prompt.is_some() || cx.active_drag.is_some() {
            return None;
        }
        #[cfg(any(feature = "inspector", debug_assertions))]
        if self.inspector.is_some() {
            return None;
        }
        let frame = &self.rendered_frame;
        frame.root_prepaint.as_ref()?;
        frame.root_paint.as_ref()?;
        if frame.root_size != Some((self.viewport_size, self.scale_factor)) {
            return None;
        }
        // Every slot must be complete and the slots must not overlap, in prepaint and paint
        // order alike (they are recorded in tree order).
        let mut prev: Option<&ViewSlot> = None;
        for slot in &frame.view_slots {
            let paint = slot.paint_range.as_ref()?;
            if let Some(prev) = prev {
                let prev_paint = prev.paint_range.as_ref()?;
                if prev.prepaint_range.end.dispatch_tree_index
                    > slot.context.node.index()
                    || prev_paint.end.scene_index > paint.start.scene_index
                {
                    return None;
                }
            }
            prev = Some(slot);
        }
        let views = self.invalidator.take_views();
        let mut holes = SmallVec::<[usize; 4]>::new();
        let mut ok = !views.is_empty();
        for view in &views {
            if !ok {
                break;
            }
            let slot = frame.dispatch_tree.view_path_reversed(*view).find_map(|ancestor| {
                let mut matches = frame
                    .view_slots
                    .iter()
                    .enumerate()
                    .filter(|(_, slot)| slot.view.entity_id() == ancestor);
                match (matches.next(), matches.next()) {
                    (Some((ix, _)), None) => Some(Some(ix)),
                    (Some(_), Some(_)) => Some(None),
                    _ => None,
                }
            });
            match slot {
                Some(Some(ix)) => {
                    if !holes.contains(&ix) {
                        holes.push(ix);
                    }
                }
                _ => ok = false,
            }
        }
        self.invalidator.replace_views(views);
        if !ok {
            return None;
        }
        holes.sort_unstable();
        Some(holes)
    }

    /// Prepaint the root as its last frame with the `holes` re-rendered.
    pub(crate) fn prepaint_spliced(
        &mut self,
        holes: SmallVec<[usize; 4]>,
        cx: &mut App,
    ) -> SplicedRoot {
        let old_root = self.rendered_frame.root_prepaint.clone().unwrap_or_default();
        let mut spliced = SplicedRoot {
            holes: Vec::new(),
            prepaint_segments: Vec::new(),
            node_segments: Vec::new(),
        };
        let mut cursor = old_root.start.clone();
        for old_ix in holes {
            let slot_start = self.rendered_frame.view_slots[old_ix].prepaint_range.start.clone();
            self.reuse_prepaint_segment(cursor.clone()..slot_start, &mut spliced);

            let slot = &self.rendered_frame.view_slots[old_ix];
            let (view, style, global_id, bounds) = (
                slot.view.clone(),
                slot.style.clone(),
                slot.global_id.clone(),
                slot.bounds,
            );
            let mut context = slot.context.clone();
            let old_end = slot.prepaint_range.end.clone();
            let node = map_node(&spliced.node_segments, context.node.index())
                .map(DispatchNodeId::from_index)
                .expect("a cached view's node precedes its range");
            context.node = node;

            self.next_frame.dispatch_tree.reset_stacks();
            self.next_frame.dispatch_tree.set_active_node(node);
            self.element_id_stack.clone_from(&context.element_id_stack);
            self.text_style_stack.clone_from(&context.text_style_stack);
            let opacity = mem::replace(&mut self.element_opacity, context.opacity);
            let (element, range) = self.with_rendered_view(context.parent_view, |window| {
                window.with_rem_size(context.rem_size, |window| {
                    window.with_content_mask(Some(context.content_mask), |window| {
                        crate::view::prepaint_view_slot(&view, &global_id, bounds, window, cx)
                    })
                })
            });
            self.element_opacity = opacity;
            self.element_id_stack.clear();
            self.text_style_stack.clear();
            self.next_frame.dispatch_tree.reset_stacks();

            let new_ix =
                self.push_view_slot(context, view, style, global_id, bounds, range.unwrap_or_default());
            spliced.holes.push((old_ix, new_ix, element));
            cursor = old_end;
        }
        self.reuse_prepaint_segment(cursor..old_root.end, &mut spliced);
        // The replayed ancestors accessed what they accessed last frame. Added after the holes
        // rendered, so each hole's cache records everything it read itself.
        if let Some(tracked) = cx.tracked_entities.get(&self.handle.window_id()).cloned() {
            cx.entities.extend_accessed(&tracked);
        }
        spliced
    }

    /// `reuse_prepaint` for a span of the root that need not be a whole subtree.
    fn reuse_prepaint_segment(
        &mut self,
        range: Range<PrepaintStateIndex>,
        spliced: &mut SplicedRoot,
    ) {
        let new_start = self.prepaint_index();
        self.next_frame.hitboxes.extend(
            self.rendered_frame.hitboxes[range.start.hitboxes_index..range.end.hitboxes_index]
                .iter()
                .cloned(),
        );
        self.next_frame.tooltip_requests.extend(
            self.rendered_frame.tooltip_requests
                [range.start.tooltips_index..range.end.tooltips_index]
                .iter_mut()
                .map(|request| request.take()),
        );
        self.next_frame.accessed_element_states.extend(
            self.rendered_frame.accessed_element_states[range.start.accessed_element_states_index
                ..range.end.accessed_element_states_index]
                .iter()
                .map(|(id, type_id)| (id.clone(), *type_id)),
        );
        self.text_system.reuse_layouts(
            range.start.line_layout_index.clone()..range.end.line_layout_index.clone(),
        );

        let old_nodes = range.start.dispatch_tree_index..range.end.dispatch_tree_index;
        let new_node_start = self.next_frame.dispatch_tree.len();
        let segments = spliced.node_segments.clone();
        let contains_focus = self.next_frame.dispatch_tree.reuse_span(
            old_nodes.clone(),
            &mut self.rendered_frame.dispatch_tree,
            |old| map_node(&segments, old),
            self.focus,
        );
        if contains_focus {
            self.next_frame.focus = self.focus;
        }
        spliced
            .node_segments
            .push((old_nodes.start, old_nodes.end, new_node_start));

        let segments = &spliced.node_segments;
        self.next_frame.deferred_draws.extend(
            self.rendered_frame.deferred_draws
                [range.start.deferred_draws_index..range.end.deferred_draws_index]
                .iter()
                .map(|deferred_draw| DeferredDraw {
                    current_view: deferred_draw.current_view,
                    parent_node: map_node(segments, deferred_draw.parent_node.index())
                        .map(DispatchNodeId::from_index)
                        .unwrap_or(deferred_draw.parent_node),
                    element_id_stack: deferred_draw.element_id_stack.clone(),
                    text_style_stack: deferred_draw.text_style_stack.clone(),
                    content_mask: deferred_draw.content_mask,
                    rem_size: deferred_draw.rem_size,
                    priority: deferred_draw.priority,
                    element: None,
                    absolute_offset: deferred_draw.absolute_offset,
                    prepaint_range: deferred_draw.prepaint_range.clone(),
                    paint_range: deferred_draw.paint_range.clone(),
                }),
        );
        spliced.prepaint_segments.push((range, new_start));
    }

    /// Paint the root as its last frame with the holes painted fresh, then carry the other
    /// slots (and their cached views' element states) over at their new indices.
    pub(crate) fn paint_spliced(&mut self, spliced: SplicedRoot, cx: &mut App) {
        let old_root = self.rendered_frame.root_paint.clone().unwrap_or_default();
        let mut paint_segments: Vec<(Range<PaintIndex>, PaintIndex)> = Vec::new();
        let mut cursor = old_root.start.clone();
        let mut hole_old = Vec::new();
        for (old_ix, new_ix, mut element) in spliced.holes {
            hole_old.push(old_ix);
            let old_paint = self.rendered_frame.view_slots[old_ix]
                .paint_range
                .clone()
                .unwrap_or_default();
            let new_start = self.paint_index();
            self.reuse_paint(cursor.clone()..old_paint.start.clone());
            paint_segments.push((cursor..old_paint.start, new_start));

            let slot = &self.next_frame.view_slots[new_ix];
            let (view, global_id, context, mask, opacity) = (
                slot.view.clone(),
                slot.global_id.clone(),
                slot.context.clone(),
                self.rendered_frame.view_slots[old_ix].paint_content_mask,
                self.rendered_frame.view_slots[old_ix].paint_opacity,
            );
            self.next_frame.dispatch_tree.reset_stacks();
            self.next_frame.dispatch_tree.set_active_node(context.node);
            self.element_id_stack.clone_from(&context.element_id_stack);
            self.text_style_stack.clone_from(&context.text_style_stack);
            let saved_opacity = mem::replace(&mut self.element_opacity, opacity);
            let range = self.with_rendered_view(context.parent_view, |window| {
                window.with_rem_size(context.rem_size, |window| {
                    window.with_content_mask(Some(mask), |window| {
                        crate::view::paint_view_slot(&view, &global_id, &mut element, window, cx)
                    })
                })
            });
            self.element_opacity = saved_opacity;
            self.element_id_stack.clear();
            self.text_style_stack.clear();
            self.next_frame.dispatch_tree.reset_stacks();
            if let Some(range) = range {
                self.finish_view_slot(new_ix, mask, opacity, range);
            }
            cursor = old_paint.end;
        }
        let new_start = self.paint_index();
        self.reuse_paint(cursor.clone()..old_root.end.clone());
        paint_segments.push((cursor..old_root.end, new_start));

        // Carry the other slots over. A slot lies in exactly one copied segment; the k-th
        // prepaint segment and the k-th paint segment are the same span of the tree.
        let old_slots = mem::take(&mut self.rendered_frame.view_slots);
        for (old_ix, slot) in old_slots.into_iter().enumerate() {
            if hole_old.contains(&old_ix) {
                continue;
            }
            let Some(old_paint) = slot.paint_range.clone() else {
                continue;
            };
            let Some(k) = spliced.prepaint_segments.iter().position(|(segment, _)| {
                segment.start.dispatch_tree_index <= slot.context.node.index()
                    && slot.prepaint_range.end.dispatch_tree_index
                        <= segment.end.dispatch_tree_index
            }) else {
                continue;
            };
            let (prepaint_from, prepaint_to) = {
                let (segment, to) = &spliced.prepaint_segments[k];
                (segment.start.clone(), to.clone())
            };
            let Some((paint_segment, paint_to)) = paint_segments.get(k).cloned() else {
                continue;
            };
            let Some(node) = map_node(&spliced.node_segments, slot.context.node.index()) else {
                continue;
            };
            let prepaint_range = shift_prepaint(&slot.prepaint_range.start, &prepaint_from, &prepaint_to)
                ..shift_prepaint(&slot.prepaint_range.end, &prepaint_from, &prepaint_to);
            let paint_range = shift_paint(&old_paint.start, &paint_segment.start, &paint_to)
                ..shift_paint(&old_paint.end, &paint_segment.start, &paint_to);
            crate::view::set_view_slot_ranges(
                &mut self.rendered_frame.element_states,
                &slot.global_id,
                prepaint_range.clone(),
                paint_range.clone(),
            );
            let mut context = slot.context;
            context.node = DispatchNodeId::from_index(node);
            self.next_frame.view_slots.push(ViewSlot {
                view: slot.view,
                style: slot.style,
                global_id: slot.global_id,
                context,
                bounds: slot.bounds,
                prepaint_range,
                paint_range: Some(paint_range),
                paint_content_mask: slot.paint_content_mask,
                paint_opacity: slot.paint_opacity,
            });
        }
        self.next_frame
            .view_slots
            .sort_by_key(|slot| slot.context.node.index());
    }
}
