//! The A/B harness's in-app driver (`scripts/opt-ab.ps1`, docs/PERF.md
//! section 8). Development only and off by default: nothing here runs, and
//! no state is kept, unless `KOMOREBI_BENCH=<steps>` is set.
//!
//! The steps run once, after the first frame, on the live window (no
//! snapshot, no capture), and each reports a `[bench] ...` line on stderr
//! for the script to read:
//!
//! | Step | Does | Prints |
//! |---|---|---|
//! | `wait:<ms>` | waits | |
//! | `mark:<name>` | nothing | `[bench] mark <name>` (the script samples the process) |
//! | `open:<row>` | opens that row of the list, waits until the reader has laid out and drawn the tiles in view, that shown in a frame, and the tiles a scroll reaches next | `[bench] open parsed|page|first_tile|settled|complete_view|complete_ahead <ms>` (from the call that opens the message; complete_view: every visible tile painted, in a rendered frame, images decoded and the whole page laid out; complete_ahead: also every tile of the read-ahead window) |
//! | `read:<frames>:<px>` | scrolls the open message `px` per frame, to the end and back | `[bench] read frames=.. blank_frames=.. blank_tiles=.. worst_blank_ms=.. moved=..` (frames that showed a visible tile not painted yet, how many such tiles over all frames, the longest such stretch) |
//! | `scroll:<frames>:<px>` | one wheel step of `px` over the list per frame, down, up, down... in blocks of 50 | `[bench] scroll frames=.. median=.. p95=.. max=.. moved=..` (ms from the wheel event to the frame being rendered) |
//! | `compose` | a new message in the reading pane, the body focused | |
//! | `type:<n>` | `n` keys into the focused editor, one per frame | `[bench] type keys=.. median=.. p95=.. max=..` (ms from the key to the frame being rendered; the first 10 keys are warm-up and not counted) |
//! | `sample:<name>`, `sample-end` | start and stop the in-app sampling profiler (feature `sampler`, sampler.rs) | `[prof] <name> samples=.. report=<file>` |
//! | `quit` | quits the app | |
//!
//! Every ms figure is wall time on the UI thread's frame clock, so it
//! includes the wait for the display's next frame (up to one refresh
//! interval) as well as the work; the same on A and B.
//!
//! Safety, in code (`from_env`, before the cache or any window opens): the
//! app refuses (exit 2, nothing opened) unless `KOMOREBI_OFFLINE` is set and
//! `KOMOREBI_DB` names a cache outside the app's own folder, as the
//! snapshot does.

use std::cell::Cell;
use std::path::Path;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use futures::FutureExt as _;
use futures::channel::oneshot;
use gpui::{
    AnyWindowHandle, AsyncApp, Context, Entity, Keystroke, Modifiers, MouseMoveEvent,
    PlatformInput, ScrollDelta, ScrollWheelEvent, TouchPhase, Window, point, px,
};

use crate::views::mail::MailView;
use crate::window::MainWindow;

static STEPS: OnceLock<Vec<Step>> = OnceLock::new();

/// One step of a script.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Wait(u64),
    Mark(String),
    Open(usize),
    /// Scroll the open message this far per frame, for this many frames.
    Read {
        frames: u32,
        px: f32,
    },
    Scroll {
        frames: u32,
        px: f32,
    },
    Compose,
    Type(u32),
    /// Start the sampling profiler (feature `sampler`) under this name.
    Sample(String),
    /// Stop it and write the report.
    SampleEnd,
    Quit,
}

/// The longest any single wait for the window may take.
const STEP_TIMEOUT: Duration = Duration::from_secs(20);

/// `KOMOREBI_BENCH`, parsed.
pub fn parse(spec: &str) -> Result<Vec<Step>, String> {
    let mut steps = Vec::new();
    for raw in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let (name, rest) = raw.split_once(':').unwrap_or((raw, ""));
        let num = |s: &str| {
            s.parse::<u64>()
                .map_err(|_| format!("KOMOREBI_BENCH: {raw}: expected a number"))
        };
        steps.push(match name {
            "wait" => Step::Wait(num(rest)?),
            "mark" if !rest.is_empty() => Step::Mark(rest.to_owned()),
            "open" => Step::Open(num(rest)? as usize),
            "read" => {
                let (frames, dy) = rest.split_once(':').unwrap_or((rest, "60"));
                Step::Read {
                    frames: num(frames)? as u32,
                    px: num(dy)? as f32,
                }
            }
            "scroll" => {
                let (frames, dy) = rest.split_once(':').unwrap_or((rest, "60"));
                Step::Scroll {
                    frames: num(frames)? as u32,
                    px: num(dy)? as f32,
                }
            }
            "compose" => Step::Compose,
            "sample" if !rest.is_empty() => Step::Sample(rest.to_owned()),
            "sample-end" => Step::SampleEnd,
            "type" => Step::Type(num(rest)? as u32),
            "quit" => Step::Quit,
            _ => {
                return Err(format!(
                    "KOMOREBI_BENCH: unknown step {raw} (wait:ms, mark:name, open:row, read:frames:px, scroll:frames:px, compose, type:n, quit)"
                ));
            }
        });
    }
    Ok(steps)
}

/// The script this process was asked for, from its environment: `None` when
/// `KOMOREBI_BENCH` is not set; an error (the app exits
/// `snapshot::EXIT_REFUSED`) when it is unsafe or malformed.
pub fn from_env() -> Result<Option<Vec<Step>>, String> {
    let get = |name: &str| komorebi::dev_var(name).and_then(|v| v.into_string().ok());
    let Some(spec) = get("KOMOREBI_BENCH") else {
        return Ok(None);
    };
    if get("KOMOREBI_OFFLINE").is_none() {
        return Err("refusing KOMOREBI_BENCH without KOMOREBI_OFFLINE=1".into());
    }
    let Some(db) = get("KOMOREBI_DB") else {
        return Err("refusing KOMOREBI_BENCH without KOMOREBI_DB=<throwaway cache>".into());
    };
    if komorebi::seed::in_real_cache_dir(Path::new(&db)) {
        return Err(format!(
            "refusing KOMOREBI_BENCH on {db}: KOMOREBI_DB must be a throwaway cache, not in the app's cache folder"
        ));
    }
    let steps = parse(&spec)?;
    let _ = STEPS.set(steps.clone());
    Ok(Some(steps))
}

/// The steps this process was asked for (set by [`from_env`]).
pub fn script() -> Option<Vec<Step>> {
    STEPS.get().cloned()
}

/// Whether the reader should report how long a message takes to open.
pub fn enabled() -> bool {
    STEPS.get().is_some()
}

// ------------------------------------------------------------ open clock

thread_local! {
    /// When the open message was asked for, and which events have printed
    /// (bit 0 parsed, 1 first tile, 2 settled, 3 page).
    static OPEN: Cell<Option<(Instant, u8)>> = const { Cell::new(None) };
}

/// A message is being opened (`ReaderView::open`).
pub fn open_begin() {
    if enabled() {
        OPEN.set(Some((Instant::now(), 0)));
    }
}

/// One of `parsed`, `page`, `first_tile`, `settled` happened for the message being
/// opened; printed once per open.
pub fn open_event(what: &str) {
    let bit = match what {
        "parsed" => 1,
        "first_tile" => 2,
        "page" => 8,
        _ => 4,
    };
    if let Some((t0, seen)) = OPEN.get()
        && seen & bit == 0
        // Settled counts only once a tile has been shown.
        && (bit != 4 || seen & 2 != 0)
    {
        OPEN.set(Some((t0, seen | bit)));
        eprintln!("[bench] open {what} {:.2}", ms(t0.elapsed()));
    }
}

/// A completion mark of the message being opened; printed each time.
fn open_mark(what: &str) {
    if let Some((t0, _)) = OPEN.get() {
        eprintln!("[bench] open {what} {:.2}", ms(t0.elapsed()));
    }
}

/// The next frame rendered, or `None` after a quarter second.
async fn next_frame(handle: AnyWindowHandle, cx: &mut AsyncApp) -> Option<Instant> {
    let (tx, rx) = oneshot::channel();
    handle
        .update(cx, move |_, window, _| {
            window.on_next_frame(move |_, _| {
                let _ = tx.send(Instant::now());
            });
            window.refresh();
        })
        .ok()?;
    frame(rx, cx).await
}

/// The allocator's counters at `mark` (the `alloc-count` build only).
fn alloc_mark(mark: &str) {
    #[cfg(feature = "alloc-count")]
    eprintln!("{}", crate::alloc_count::line(mark));
    #[cfg(not(feature = "alloc-count"))]
    let _ = mark;
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

// ---------------------------------------------------------------- script

/// Starts the script on the window (called once, after the first frame).
pub fn start(
    window: &Window,
    mail: Option<Entity<MailView>>,
    steps: Vec<Step>,
    cx: &mut Context<MainWindow>,
) {
    let handle = window.window_handle();
    let t0 = Instant::now();
    alloc_mark("first_frame");
    cx.spawn(async move |_, cx| {
        for step in steps {
            run_step(handle, mail.as_ref(), &step, cx).await;
        }
        eprintln!("[bench] done {:.1}", ms(t0.elapsed()));
    })
    .detach();
}

async fn run_step(
    handle: AnyWindowHandle,
    mail: Option<&Entity<MailView>>,
    step: &Step,
    cx: &mut AsyncApp,
) {
    match step {
        Step::Wait(n) => sleep(cx, Duration::from_millis(*n)).await,
        Step::Mark(name) => {
            alloc_mark(name);
            eprintln!("[bench] mark {name}");
        }
        Step::Sample(name) => {
            #[cfg(feature = "sampler")]
            crate::sampler::start(name);
            #[cfg(not(feature = "sampler"))]
            let _ = name;
        }
        Step::SampleEnd =>
        {
            #[cfg(feature = "sampler")]
            if let Some(line) = crate::sampler::stop_and_report() {
                eprintln!("{line}");
            }
        }
        Step::Quit => {
            cx.update(|cx| cx.quit());
        }
        Step::Open(row) => {
            let Some(mail) = mail else { return };
            alloc_mark("open0");
            let row = *row;
            let m = mail.clone();
            let _ = handle.update(cx, move |_, window, cx| {
                m.update(cx, |m, cx| m.snapshot_select(&[row], window, cx));
            });
            // Poll until the reader has drawn what is in view.
            let began = Instant::now();
            while began.elapsed() < STEP_TIMEOUT {
                sleep(cx, Duration::from_millis(2)).await; // token-exempt: harness pacing
                let m = mail.clone();
                let done = handle
                    .update(cx, move |_, _, cx| m.read(cx).reader().read(cx).settled())
                    .unwrap_or(true);
                if done {
                    break;
                }
            }
            alloc_mark("open1");
            // The frame that shows them, then the tiles a scroll reaches next.
            next_frame(handle, cx).await;
            open_mark("complete_view");
            let m = mail.clone();
            let page = handle
                .update(cx, move |_, _, cx| m.read(cx).reader().read(cx).page_view())
                .unwrap_or(false);
            eprintln!("[bench] open view {}", if page { "page" } else { "text" });
            while began.elapsed() < STEP_TIMEOUT {
                let m = mail.clone();
                let done = handle
                    .update(cx, move |_, _, cx| {
                        m.read(cx).reader().read(cx).ahead_ready()
                    })
                    .unwrap_or(true);
                if done {
                    break;
                }
                sleep(cx, Duration::from_millis(2)).await; // token-exempt: harness pacing
            }
            next_frame(handle, cx).await;
            open_mark("complete_ahead");
            alloc_mark("open2");
        }
        Step::Read { frames, px: dy } => {
            let Some(mail) = mail else { return };
            let (mut blank_frames, mut blank_tiles, mut sign) = (0u32, 0u64, 1.0_f32);
            let (mut worst, mut since, mut moved) = (0.0_f64, None::<Instant>, 0.0_f32);
            for _ in 0..*frames {
                let m = mail.clone();
                let (dy, s) = (*dy, sign);
                let step = handle.update(cx, move |_, _, cx| {
                    let reader = m.read(cx).reader().clone();
                    reader.update(cx, |r, cx| r.bench_read_step(dy * s, cx))
                });
                let Ok(step) = step else { break };
                moved += step.abs();
                if step.abs() < 0.5 {
                    sign = -sign; // the end of the page: back the other way
                }
                let Some(t) = next_frame(handle, cx).await else {
                    continue;
                };
                let m = mail.clone();
                let blank = handle
                    .update(cx, move |_, _, cx| {
                        m.read(cx).reader().read(cx).blank_tiles()
                    })
                    .unwrap_or(0);
                if blank > 0 {
                    blank_frames += 1;
                    blank_tiles += u64::from(blank);
                    let start = *since.get_or_insert(t);
                    worst = worst.max(ms(t - start));
                } else {
                    since = None;
                }
            }
            eprintln!(
                "[bench] read frames={frames} blank_frames={blank_frames} blank_tiles={blank_tiles} worst_blank_ms={worst:.1} moved={moved:.0}"
            );
        }
        Step::Scroll { frames, px: dy } => {
            let Some(mail) = mail else { return };
            let mut times = Vec::new();
            // The pointer first, and a frame, so that the wheel is hit-tested
            // over the list.
            let (tx, rx) = oneshot::channel();
            let _ = handle.update(cx, move |_, window, cx| {
                point_at_list(window, cx);
                window.on_next_frame(move |_, _| {
                    let _ = tx.send(Instant::now());
                });
            });
            let _ = frame(rx, cx).await;
            let before = offset(handle, mail, cx);
            let mut moved = 0.0_f32;
            alloc_mark("scroll0");
            for i in 0..*frames {
                let sign = if (i / 50) % 2 == 0 { -1.0 } else { 1.0 };
                let dy = *dy * sign;
                let (tx, rx) = oneshot::channel();
                let t0 = Instant::now();
                let sent = handle.update(cx, move |_, window, cx| {
                    wheel(window, dy, cx);
                    window.on_next_frame(move |_, _| {
                        let _ = tx.send(Instant::now());
                    });
                });
                if sent.is_err() {
                    break;
                }
                if let Some(t1) = frame(rx, cx).await {
                    times.push(ms(t1 - t0));
                }
                // The first block of 50 steps goes down: how far it got.
                if i + 1 == (*frames).min(50) {
                    moved = (offset(handle, mail, cx) - before).abs();
                }
            }
            alloc_mark("scroll1");
            report(
                "scroll",
                "frames",
                &mut times,
                &format!(" moved={moved:.0}"),
            );
        }
        Step::Compose => {
            let Some(mail) = mail else { return };
            let m = mail.clone();
            let _ = handle.update(cx, move |_, window, cx| {
                m.update(cx, |m, cx| {
                    m.new_mail(window, cx);
                    let composer = m.composer().clone();
                    composer.update(cx, |c, cx| c.focus_body(window, cx));
                });
            });
            sleep(cx, Duration::from_millis(400)).await; // token-exempt: harness pacing
        }
        Step::Type(n) => {
            let mail = mail.cloned();
            let mut times = Vec::new();
            for k in 0..*n + WARM_UP {
                if k == WARM_UP {
                    alloc_mark("type0");
                }
                let key = if k % 10 == 9 { "space" } else { "x" };
                let (tx, rx) = oneshot::channel();
                let t0 = Instant::now();
                let sent = handle.update(cx, move |_, window, cx| {
                    if let Ok(stroke) = Keystroke::parse(key) {
                        window.dispatch_keystroke(stroke, cx);
                    }
                    window.on_next_frame(move |_, _| {
                        let _ = tx.send(Instant::now());
                    });
                });
                if sent.is_err() {
                    break;
                }
                if let Some(t1) = frame(rx, cx).await
                    && k >= WARM_UP
                {
                    times.push(ms(t1 - t0));
                }
            }
            alloc_mark("type1");
            let typed = body_len(handle, mail.as_ref(), cx);
            report("type", "keys", &mut times, &format!(" typed={typed}"));
        }
    }
}

/// Keys typed first and not counted (glyph caches, the first layout).
const WARM_UP: u32 = 10;

async fn sleep(cx: &mut AsyncApp, d: Duration) {
    cx.background_executor().timer(d).await; // timer-exempt: the development driver waits for the app's own work, as the snapshot driver does
}

/// The frame callback, or `None` after a quarter second.
async fn frame(rx: oneshot::Receiver<Instant>, cx: &mut AsyncApp) -> Option<Instant> {
    let mut rx = rx.fuse();
    let mut limit = cx
        .background_executor()
        .timer(Duration::from_millis(250)) // token-exempt: harness pacing // timer-exempt: a frame that never comes ends the step
        .fuse();
    futures::select_biased! {
        got = rx => got.ok(),
        () = limit => None,
    }
}

fn offset(handle: AnyWindowHandle, mail: &Entity<MailView>, cx: &mut AsyncApp) -> f32 {
    let mail = mail.clone();
    handle
        .update(cx, move |_, _, cx| mail.read(cx).bench_list_offset())
        .unwrap_or(0.0)
}

fn body_len(handle: AnyWindowHandle, mail: Option<&Entity<MailView>>, cx: &mut AsyncApp) -> usize {
    let Some(mail) = mail else { return 0 };
    let mail = mail.clone();
    handle
        .update(cx, move |_, _, cx| {
            let composer = mail.read(cx).composer().clone();
            composer.read(cx).bench_body_len(cx)
        })
        .unwrap_or(0)
}

fn report(step: &str, unit: &str, times: &mut [f64], extra: &str) {
    if times.is_empty() {
        eprintln!("[bench] {step} {unit}=0");
        return;
    }
    times.sort_by(f64::total_cmp);
    let at = |q: usize| times[(times.len() * q / 100).min(times.len() - 1)];
    eprintln!(
        "[bench] {step} {unit}={} median={:.2} p95={:.2} max={:.2} min={:.2}{extra}",
        times.len(),
        at(50),
        at(95),
        times[times.len() - 1],
        times[0],
    );
}

/// The pointer over the list.
fn point_at_list(window: &mut Window, cx: &mut gpui::App) {
    let size = window.viewport_size();
    let (x, y) = list_centre(size.width.as_f32(), size.height.as_f32());
    window.dispatch_event(
        PlatformInput::MouseMove(MouseMoveEvent {
            position: point(px(x), px(y)),
            pressed_button: None,
            modifiers: Modifiers::default(),
        }),
        cx,
    );
}

/// A mouse wheel step over the list (a negative `dy` scrolls down).
fn wheel(window: &mut Window, dy: f32, cx: &mut gpui::App) {
    let size = window.viewport_size();
    let (x, y) = list_centre(size.width.as_f32(), size.height.as_f32());
    let position = point(px(x), px(y));
    window.dispatch_event(
        PlatformInput::ScrollWheel(ScrollWheelEvent {
            position,
            delta: ScrollDelta::Pixels(point(px(0.), px(dy))), // token-exempt: the requested scroll, not a size
            modifiers: Modifiers::default(),
            touch_phase: TouchPhase::Moved,
        }),
        cx,
    );
}

/// The middle of the message list (logical px) in a window `width` by
/// `height`, the layout of `views::mail` (as `snapshot_drive`'s reading
/// pane point).
pub fn list_centre(width: f32, height: f32) -> (f32, f32) {
    use crate::theme::layout;
    if width < crate::views::mail::ONE_PANE_BELOW_W {
        return (width / 2., height / 2.);
    }
    let (nav, list) = if width < layout::COMPACT_BELOW_W {
        (layout::RAIL_W, layout::LIST_W_COMPACT)
    } else {
        (layout::NAV_W, layout::LIST_W)
    };
    (nav + crate::theme::space::PANE_GAP + list / 2., height / 2.)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_script_parses_and_a_bad_step_is_refused() {
        let steps =
            parse("wait:2600, open:0,mark:opened,read:9:40,scroll:300:60,compose,type:200,quit")
                .expect("a script");
        assert_eq!(
            steps,
            vec![
                Step::Wait(2600),
                Step::Open(0),
                Step::Mark("opened".into()),
                Step::Read {
                    frames: 9,
                    px: 40.0
                },
                Step::Scroll {
                    frames: 300,
                    px: 60.0
                },
                Step::Compose,
                Step::Type(200),
                Step::Quit,
            ]
        );
        for bad in ["wait", "wait:x", "jump:1", "mark", "open:-1"] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        assert_eq!(parse("").expect("empty"), Vec::new());
    }

    #[test]
    fn the_list_point_is_inside_the_list_pane() {
        let (x, y) = list_centre(1280., 800.);
        assert!(x > crate::theme::layout::NAV_W && x < crate::theme::layout::NAV_W + 384. + 16.);
        assert_eq!(y, 400.);
    }
}
