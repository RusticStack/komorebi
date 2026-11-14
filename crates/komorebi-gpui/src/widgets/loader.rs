//! Busy marks.
//!
//! [`Loader`]: the brand's three spots (7, 4 and 5 px, 5 px apart) that
//! breathe in opacity from 0.3 to 1 over 2.4 s, a third of a cycle apart;
//! they never move (BRAND.md "Motion"). The critic's rule
//! (IMPLEMENTATION.md section 5): no `with_animation`, which asks for a frame
//! every vsync; the loader's own entity steps on a 125 ms timer (8 frames a
//! second) only while it is active and motion is allowed, and the task
//! ends with an epoch bump (Skua's cursor blink pattern). Under reduce
//! motion, or when idle, the spots hold at fixed opacities and no task runs.
//!
//! [`ProgressRing`]: Kit's progress circle in ink for a known fraction
//! (an attachment saving, an import).

use std::f32::consts::TAU;
use std::time::Duration;

use gpui::{
    App, Context, ElementId, IntoElement, ParentElement, Render, RenderOnce, Role, SharedString,
    Styled, Window, div, prelude::*, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::progress::ProgressCircle;
use gpui_kit::component::{Sizable as _, Size};

use crate::theme::{ActiveTokens as _, layout, motion, radius};

/// The three spots' opacities `step` ticks into the cycle.
pub fn opacities(step: u64) -> [f32; 3] {
    let at = step * motion::LOADER_STEP_MS;
    motion::LOADER_OFFSETS_MS.map(|offset| {
        let phase =
            ((at + offset) % motion::LOADER_PERIOD_MS) as f32 / motion::LOADER_PERIOD_MS as f32;
        // 0 at the start of the cycle, 1 half way, back to 0.
        let breath = (1. - (phase * TAU).cos()) / 2.;
        motion::LOADER_MIN + (motion::LOADER_MAX - motion::LOADER_MIN) * breath
    })
}

/// The loader's state, held in its own entity so its steps redraw only it.
pub struct Loader {
    name: SharedString,
    active: bool,
    /// Bumped to end a running step task.
    epoch: u64,
    running: Option<u64>,
    step: u64,
}

impl Loader {
    /// `name` is what a screen reader says ("Syncing").
    pub fn new(name: impl Into<SharedString>) -> Self {
        Self {
            name: name.into(),
            active: false,
            epoch: 0,
            running: None,
            step: 0,
        }
    }

    pub fn set_active(&mut self, active: bool, cx: &mut Context<Self>) {
        if self.active != active {
            self.active = active;
            cx.notify();
        }
    }

    pub fn is_stepping(&self) -> bool {
        self.running.is_some()
    }

    /// Starts or stops the step task to match the state (called from
    /// `render`, so reduce motion and activity are re-read every time).
    fn sync(&mut self, cx: &mut Context<Self>) {
        let wanted = self.active && !cx.reduce_motion();
        if !wanted {
            if self.running.take().is_some() {
                self.epoch += 1;
                self.step = 0;
            }
            return;
        }
        if self.running.is_some() {
            return;
        }
        self.epoch += 1;
        let epoch = self.epoch;
        self.running = Some(epoch);
        let period = Duration::from_millis(motion::LOADER_STEP_MS);
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(period).await; // timer-exempt: the loader's 8 fps step, only while active (brand motion)
                let alive = this.update(cx, |loader, cx| {
                    if loader.epoch != epoch {
                        return false;
                    }
                    loader.step += 1;
                    cx.notify();
                    true
                });
                if !matches!(alive, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
    }
}

impl Render for Loader {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync(cx);
        let ink = cx.tokens().color.ink;
        let alphas = if self.running.is_some() {
            opacities(self.step)
        } else {
            motion::LOADER_REST
        };
        div()
            .id("loader")
            .role(Role::ProgressIndicator)
            .aria_label(self.name.clone())
            .flex()
            .flex_row()
            .items_center()
            .gap(px(layout::LOADER_GAP))
            .children(
                layout::LOADER_SPOTS
                    .iter()
                    .zip(alphas)
                    .map(|(size, alpha)| {
                        div()
                            .size(px(*size))
                            .flex_none()
                            .rounded(px(radius::FULL))
                            .bg(ink)
                            .opacity(alpha)
                    }),
            )
            .test_support()
    }
}

/// A determinate ring in ink.
#[derive(IntoElement)]
pub struct ProgressRing {
    id: ElementId,
    name: SharedString,
    /// 0 to 1.
    fraction: f32,
}

impl ProgressRing {
    pub fn new(id: impl Into<ElementId>, name: impl Into<SharedString>, fraction: f32) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            fraction: fraction.clamp(0., 1.),
        }
    }
}

impl RenderOnce for ProgressRing {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        ProgressCircle::new(self.id)
            .value(self.fraction * 100.)
            .color(cx.tokens().color.ink)
            .accessibility_label(self.name)
            .with_size(Size::Size(px(layout::PROGRESS_RING)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::test_support;
    use gpui::TestAppContext;

    #[test]
    fn the_spots_breathe_a_third_apart_within_the_range() {
        let start = opacities(0);
        assert!((start[0] - motion::LOADER_MIN).abs() < 1e-4, "{start:?}");
        let half = motion::LOADER_PERIOD_MS / 2 / motion::LOADER_STEP_MS;
        // Half a cycle in (the step nearest it): nearly full.
        assert!(opacities(half)[0] > 0.99, "{:?}", opacities(half));
        for step in 0..40 {
            for a in opacities(step) {
                assert!((motion::LOADER_MIN..=motion::LOADER_MAX).contains(&a));
            }
        }
        // The steps and the cycle line up every 12 s (96 steps).
        assert_eq!(opacities(3), opacities(3 + 96));
    }

    /// Steps only while active, at 8 a second, and stops cleanly.
    #[gpui::test]
    fn the_loader_steps_only_while_active(cx: &mut TestAppContext) {
        test_support::init(cx, false);
        cx.update(|cx| cx.set_reduce_motion(false));
        let (loader, cx) = cx.add_window_view(|_, _| Loader::new("Syncing"));
        let step = |cx: &mut gpui::VisualTestContext| loader.read_with(cx, |l, _| l.step);
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        assert_eq!(step(cx), 0, "idle: no task");
        loader.update(cx, |l, cx| l.set_active(true, cx));
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(1000));
        cx.run_until_parked();
        let ran = step(cx);
        assert!((7..=8).contains(&ran), "about 8 steps a second, got {ran}");
        loader.update(cx, |l, cx| l.set_active(false, cx));
        cx.run_until_parked();
        assert!(!loader.read_with(cx, |l, _| l.is_stepping()));
        cx.executor().advance_clock(Duration::from_secs(2));
        cx.run_until_parked();
        assert_eq!(step(cx), 0, "stopped and reset");
    }

    #[gpui::test]
    fn reduce_motion_holds_the_spots(cx: &mut TestAppContext) {
        test_support::init(cx, false);
        let (loader, cx) = cx.add_window_view(|_, _| Loader::new("Syncing"));
        loader.update(cx, |l, cx| l.set_active(true, cx));
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        assert!(!loader.read_with(cx, |l, _| l.is_stepping()));
    }
}
