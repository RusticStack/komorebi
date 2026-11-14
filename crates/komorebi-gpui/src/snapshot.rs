//! `KOMOREBI_SNAPSHOT` for the GPUI app (docs/GPUI-TESTING.md "Pixel
//! snapshots"): the switches the 0.7 app's snapshot had. The window opens as
//! usual (at `KOMOREBI_SIZE`, in
//! `KOMOREBI_THEME`, in `KOMOREBI_LANG`), and once its first frame is on
//! screen the client area is captured with `PrintWindow(PW_CLIENTONLY |
//! PW_RENDERFULLCONTENT)` (DWM renders the DirectComposition content, as
//! Skua's skua-shot does), written to the file (PNG when it ends in
//! `.png`, else a BMP as the Slint app writes), checked under
//! `KOMOREBI_SNAPSHOT_CHECK`, and the app quits. On a machine without a
//! GPU, `KOMOREBI_GPUI_FORCE_WARP` draws the same frame with WARP.
//!
//! Safety, enforced here before anything opens: a snapshot runs only with
//! `KOMOREBI_OFFLINE` set and `KOMOREBI_DB` naming a throwaway cache outside
//! the app's own cache folder; otherwise the app refuses and exits 2.

use anyhow::{Context as _, Result, bail};
use komorebi::db::ThemeChoice;
use std::path::{Path, PathBuf};

use crate::theme::{Palette, layout, space};

/// A snapshot that was taken but failed its check, or could not be taken.
pub const EXIT_FAILED: u8 = 1;
/// A snapshot request that is unsafe or malformed: nothing was opened.
pub const EXIT_REFUSED: u8 = 2;

/// The snapshot actions the GPUI app knows (`KOMOREBI_SNAPSHOT_ACTION`).
/// 0.7's other actions (docs/GPUI-TESTING.md) come with the views they
/// show.
pub const ACTIONS: &[&str] = &[
    "shell",
    "compose",
    "settings-accounts",
    "settings-account",
    "settings-signatures",
    "settings-rules",
    "settings-general",
    "settings-privacy",
    "settings-about",
    "rule-edit",
    "contacts",
    "contact-edit",
    "contacts-sync",
    "ob-welcome",
    "ob-email",
    "ob-sign-in",
    "ob-manual",
];

/// What to snapshot.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub path: PathBuf,
    /// `KOMOREBI_THEME=light|dark` over the saved theme.
    pub theme: Option<ThemeChoice>,
    /// `KOMOREBI_SIZE=WxH`, logical px; else the default size. A snapshot
    /// never uses the saved place.
    pub size: Option<(f32, f32)>,
    pub action: &'static str,
    /// `KOMOREBI_SNAPSHOT_CHECK`: fail (exit 1) on a blank window.
    pub check: bool,
    /// The check also names a blank list or reading pane
    /// ([`blank_panes`]): the mail view with one message opened
    /// (`KOMOREBI_SNAPSHOT_SELECT=<row>`), as the Slint app's CI boot check
    /// runs. Off for the other views; on whenever the SELECT switch holds a
    /// row number, so it is for a seeded cache only (an empty cache with a
    /// SELECT fails with "list, reader": the welcome flow has no list).
    pub panes: bool,
    /// `KOMOREBI_SNAPSHOT_TITLEBAR=system`: the window keeps the system
    /// caption (the client area has no title bar to check).
    pub system_title_bar: bool,
    /// What happens in the window between its first frame and the capture
    /// (`KOMOREBI_SNAPSHOT_FOCUS`, `_WALK`, `_SCROLL`, `_HOVER`).
    pub drive: Drive,
}

/// What a snapshot does to the window before the capture, in this order:
/// the keyboard focus moves ([`Focus`]), the next messages open (`walk`),
/// the reading pane scrolls (`scroll`), the pointer points ([`Hover`]). The
/// order and the waits are the 0.7 app's (`snapshot_drive`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Drive {
    pub focus: Option<Focus>,
    /// `KOMOREBI_SNAPSHOT_WALK=n`: then open the next n messages (at most
    /// 1000), 250 ms apart, to measure memory after a reading session.
    pub walk: u32,
    /// `KOMOREBI_SNAPSHOT_SCROLL=px`: scroll the reading pane down this
    /// many logical px once the message is up.
    pub scroll: Option<f32>,
    pub hover: Option<Hover>,
}

impl Drive {
    /// Whether anything is to be done (otherwise the capture is taken at
    /// the first frame, as before).
    pub fn is_empty(&self) -> bool {
        *self == Drive::default()
    }
}

/// `KOMOREBI_SNAPSHOT_FOCUS`: the keyboard focus, moved with real key
/// presses from the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Focus {
    /// `list`: the list's current row.
    List,
    /// `nav`, `nav:n`, `nav:n:keys`: Shift+F6 to the folders, then Down n
    /// times, or the keys.
    Nav { downs: usize, keys: Vec<Key> },
    /// `toolbar`, `toolbar:n`: one Tab, then Right n times. The toolbar has
    /// no roving stop yet (docs/GPUI-TEST-PORT.md rows 154 and 155), so
    /// where the Tab lands is not asserted and Right does not move along
    /// the buttons: `tab:n` reaches the nth button.
    Toolbar { rights: usize },
    /// `tab:n`: n Tabs.
    Tab(usize),
    /// `press:keys`: the keys, where the focus already is (a dialog, a
    /// Settings page).
    Press(Vec<Key>),
}

/// A key a snapshot presses (`l` Left, `r` Right, `u` Up, `d` Down, `e`
/// Enter, `n` PageDown, `p` PageUp, `h` Home, `z` End, `f` F6, `t` Tab).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Left,
    Right,
    Up,
    Down,
    Enter,
    PageDown,
    PageUp,
    Home,
    End,
    F6,
    Tab,
}

impl Key {
    fn from_letter(c: char) -> Option<Key> {
        Some(match c {
            'l' => Key::Left,
            'r' => Key::Right,
            'u' => Key::Up,
            'd' => Key::Down,
            'e' => Key::Enter,
            'n' => Key::PageDown,
            'p' => Key::PageUp,
            'h' => Key::Home,
            'z' => Key::End,
            'f' => Key::F6,
            't' => Key::Tab,
            _ => return None,
        })
    }

    /// The keystroke, in GPUI's notation.
    pub fn keystroke(self) -> &'static str {
        match self {
            Key::Left => "left",
            Key::Right => "right",
            Key::Up => "up",
            Key::Down => "down",
            Key::Enter => "enter",
            Key::PageDown => "pagedown",
            Key::PageUp => "pageup",
            Key::Home => "home",
            Key::End => "end",
            Key::F6 => "f6",
            Key::Tab => "tab",
        }
    }
}

impl Focus {
    /// The keystrokes that bring the focus there, starting from the list
    /// (empty for `list`; the list has the keys at start).
    pub fn keystrokes(&self) -> Vec<&'static str> {
        let repeat = |key: Key, n: usize| std::iter::repeat_n(key.keystroke(), n);
        match self {
            Focus::List => Vec::new(),
            Focus::Nav { downs, keys } if keys.is_empty() => std::iter::once("shift-f6")
                .chain(repeat(Key::Down, *downs))
                .collect(),
            Focus::Nav { keys, .. } => std::iter::once("shift-f6")
                .chain(keys.iter().map(|k| k.keystroke()))
                .collect(),
            Focus::Toolbar { rights } => std::iter::once(Key::Tab.keystroke())
                .chain(repeat(Key::Right, *rights))
                .collect(),
            Focus::Tab(n) => repeat(Key::Tab, *n).collect(),
            Focus::Press(keys) => keys.iter().map(|k| k.keystroke()).collect(),
        }
    }
}

/// `KOMOREBI_SNAPSHOT_HOVER`: where the pointer is when the snapshot is
/// taken (700 ms after it got there, so that a tooltip shows).
#[derive(Clone, Debug, PartialEq)]
pub enum Hover {
    /// `x,y`, `x,y:press` (the left button held down there) or `x,y:click`
    /// (clicked there); logical px in the client area.
    At { x: f32, y: f32, how: Pointer },
    /// `caption-min`, `caption-max` or `caption-close`, with `:press` for
    /// the button held down.
    Caption {
        which: crate::titlebar::Caption,
        press: bool,
    },
}

/// What the pointer does where `x,y` is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pointer {
    Move,
    Press,
    Click,
}

/// The snapshot this process was asked for, from its environment: `None`
/// when no `KOMOREBI_SNAPSHOT*` switch is set; an error (the app exits
/// [`EXIT_REFUSED`]) when the request is unsafe or malformed.
pub fn from_env() -> Result<Option<Request>, String> {
    let get = |name: &str| komorebi::dev_var(name).and_then(|v| v.into_string().ok());
    let any = std::env::vars_os().any(|(k, v)| {
        !v.is_empty()
            && k.to_str()
                .is_some_and(|k| k.starts_with("KOMOREBI_SNAPSHOT"))
    });
    parse(get, any, komorebi::seed::in_real_cache_dir)
}

/// [`from_env`] over `get` (a switch's value, `None` when unset or empty)
/// and `any` (some `KOMOREBI_SNAPSHOT*` switch is set).
pub fn parse(
    get: impl Fn(&str) -> Option<String>,
    any: bool,
    is_real_cache: impl Fn(&Path) -> bool,
) -> Result<Option<Request>, String> {
    if !any {
        return Ok(None);
    }
    let Some(path) = get("KOMOREBI_SNAPSHOT") else {
        return Err("a KOMOREBI_SNAPSHOT_* switch is set without KOMOREBI_SNAPSHOT=<file>".into());
    };
    if get("KOMOREBI_OFFLINE").is_none() {
        return Err("refusing a snapshot without KOMOREBI_OFFLINE=1".into());
    }
    let Some(db) = get("KOMOREBI_DB") else {
        return Err("refusing a snapshot without KOMOREBI_DB=<throwaway cache>".into());
    };
    if is_real_cache(Path::new(&db)) {
        return Err(format!(
            "refusing a snapshot of {db}: KOMOREBI_DB must be a throwaway cache, not in the app's cache folder"
        ));
    }
    let theme = match get("KOMOREBI_THEME").as_deref() {
        None => None,
        Some("light") => Some(ThemeChoice::Light),
        Some("dark") => Some(ThemeChoice::Dark),
        Some(other) => {
            return Err(format!(
                "KOMOREBI_THEME={other}: the GPUI app knows light and dark"
            ));
        }
    };
    let size = match get("KOMOREBI_SIZE") {
        None => None,
        Some(s) => Some(parse_size(&s).ok_or_else(|| {
            format!("KOMOREBI_SIZE={s}: expected WxH in logical pixels, such as 1280x800")
        })?),
    };
    let action = match get("KOMOREBI_SNAPSHOT_ACTION").as_deref() {
        None => ACTIONS[0],
        Some(a) => ACTIONS.iter().copied().find(|k| *k == a).ok_or_else(|| {
            format!(
                "KOMOREBI_SNAPSHOT_ACTION={a} is not in the GPUI app yet (it knows: {})",
                ACTIONS.join(", ")
            )
        })?,
    };
    let system_title_bar = match get("KOMOREBI_SNAPSHOT_TITLEBAR").as_deref() {
        None | Some("custom") => false,
        Some("system") => true,
        Some(other) => {
            return Err(format!(
                "KOMOREBI_SNAPSHOT_TITLEBAR={other}: expected custom or system"
            ));
        }
    };
    let drive = Drive {
        focus: get("KOMOREBI_SNAPSHOT_FOCUS")
            .map(|s| parse_focus(&s))
            .transpose()?,
        walk: match get("KOMOREBI_SNAPSHOT_WALK") {
            None => 0,
            Some(s) => s
                .trim()
                .parse::<u32>()
                .map_err(|_| format!("KOMOREBI_SNAPSHOT_WALK={s}: expected a whole number"))?
                .min(MAX_WALK),
        },
        scroll: match get("KOMOREBI_SNAPSHOT_SCROLL") {
            None => None,
            Some(s) => Some(
                s.trim()
                    .parse::<f32>()
                    .ok()
                    .filter(|y| y.is_finite())
                    .ok_or_else(|| {
                        format!("KOMOREBI_SNAPSHOT_SCROLL={s}: expected logical px, such as 600")
                    })?,
            ),
        },
        hover: get("KOMOREBI_SNAPSHOT_HOVER")
            .map(|s| parse_hover(&s))
            .transpose()?,
    };
    // One row opened: the list and the reading pane both have content.
    let panes = action == "shell"
        && get("KOMOREBI_SNAPSHOT_SELECT").is_some_and(|rows| rows.trim().parse::<usize>().is_ok());
    Ok(Some(Request {
        path: PathBuf::from(path),
        theme,
        size,
        action,
        check: get("KOMOREBI_SNAPSHOT_CHECK").is_some(),
        panes,
        system_title_bar,
        drive,
    }))
}

/// The most messages `KOMOREBI_SNAPSHOT_WALK` opens.
const MAX_WALK: u32 = 1000;
/// The most keys or steps one focus switch takes.
const MAX_STEPS: usize = 100;

/// `KOMOREBI_SNAPSHOT_FOCUS`: `list`, `nav[:n[:keys]]`, `toolbar[:n]`,
/// `tab:n` or `press:keys`.
fn parse_focus(s: &str) -> Result<Focus, String> {
    let bad = || {
        format!(
            "KOMOREBI_SNAPSHOT_FOCUS={s}: expected list, nav[:n[:keys]], toolbar[:n], tab:n or press:keys (keys: l r u d e n p h z f t)"
        )
    };
    let count = |n: &str| {
        n.trim()
            .parse::<usize>()
            .ok()
            .filter(|n| *n <= MAX_STEPS)
            .ok_or_else(bad)
    };
    let keys = |k: &str| -> Result<Vec<Key>, String> {
        let keys: Option<Vec<Key>> = k.chars().map(Key::from_letter).collect();
        keys.filter(|k| !k.is_empty() && k.len() <= MAX_STEPS)
            .ok_or_else(bad)
    };
    let parts: Vec<&str> = s.trim().split(':').collect();
    match parts.as_slice() {
        ["list"] => Ok(Focus::List),
        ["nav"] => Ok(Focus::Nav {
            downs: 0,
            keys: Vec::new(),
        }),
        ["nav", n] => Ok(Focus::Nav {
            downs: count(n)?,
            keys: Vec::new(),
        }),
        ["nav", n, k] => {
            count(n)?;
            Ok(Focus::Nav {
                downs: 0,
                keys: keys(k)?,
            })
        }
        ["toolbar"] => Ok(Focus::Toolbar { rights: 0 }),
        ["toolbar", n] => Ok(Focus::Toolbar { rights: count(n)? }),
        ["tab", n] => Ok(Focus::Tab(count(n)?)),
        ["press", k] => Ok(Focus::Press(keys(k)?)),
        _ => Err(bad()),
    }
}

/// `KOMOREBI_SNAPSHOT_HOVER`: `x,y[:press|:click]` or
/// `caption-min|caption-max|caption-close[:press]`.
fn parse_hover(s: &str) -> Result<Hover, String> {
    use crate::titlebar::Caption;
    let bad = || {
        format!(
            "KOMOREBI_SNAPSHOT_HOVER={s}: expected x,y[:press|:click] in logical px, or caption-min, caption-max or caption-close[:press]"
        )
    };
    let (at, how) = match s.trim().split_once(':') {
        Some((at, how)) => (at, Some(how)),
        None => (s.trim(), None),
    };
    let caption = match at {
        "caption-min" => Some(Caption::Minimize),
        "caption-max" => Some(Caption::Maximize),
        "caption-close" => Some(Caption::Close),
        _ => None,
    };
    if let Some(which) = caption {
        return match how {
            None => Ok(Hover::Caption {
                which,
                press: false,
            }),
            Some("press") => Ok(Hover::Caption { which, press: true }),
            Some(_) => Err(bad()),
        };
    }
    let (x, y) = at.split_once(',').ok_or_else(bad)?;
    let coordinate = |v: &str| {
        v.trim()
            .parse::<f32>()
            .ok()
            .filter(|v| v.is_finite() && *v >= 0.)
            .ok_or_else(bad)
    };
    let how = match how {
        None => Pointer::Move,
        Some("press") => Pointer::Press,
        Some("click") => Pointer::Click,
        Some(_) => return Err(bad()),
    };
    Ok(Hover::At {
        x: coordinate(x)?,
        y: coordinate(y)?,
        how,
    })
}

/// `WxH`, both whole and at least the window's minimum.
fn parse_size(s: &str) -> Option<(f32, f32)> {
    let (w, h) = s.split_once('x')?;
    let (w, h): (u32, u32) = (w.trim().parse().ok()?, h.trim().parse().ok()?);
    let (min_w, min_h) = crate::window::MIN_SIZE;
    (w as f32 >= min_w && h as f32 >= min_h && w <= 8192 && h <= 8192)
        .then_some((w as f32, h as f32))
}

/// What the capture needs from the window, taken on the UI thread.
#[derive(Clone, Copy, Debug)]
pub struct Shot {
    pub hwnd: Option<isize>,
    /// Physical px per logical px.
    pub scale: f32,
    pub palette: Palette,
}

/// An RGBA image, rows top to bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Image {
    fn pixel(&self, x: u32, y: u32) -> [u8; 3] {
        let i = ((y * self.width + x) * 4) as usize;
        [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2]]
    }
}

/// How many DWM compositions a capture waits at most for the window's
/// first frame to reach the screen (WARP presents later than a GPU).
const COMPOSITIONS: usize = 30;

/// Captures, writes and checks; runs off the UI thread. The app's "first
/// frame" is its first present; DWM composes it on a later pass, so the
/// capture waits for DWM's next composition (`DwmFlush`, no timer) and
/// takes the frame again while it is still blank, up to [`COMPOSITIONS`].
pub fn take(request: &Request, shot: &Shot) -> Result<()> {
    let hwnd = shot.hwnd.context("the window has no HWND")?;
    let mut image = capture(hwnd)?;
    let mut waited = 0;
    while waited < COMPOSITIONS && !blank(request, &image, shot).is_empty() {
        wait_for_composition();
        image = capture(hwnd)?;
        waited += 1;
    }
    write(&image, &request.path).with_context(|| format!("writing {}", request.path.display()))?;
    eprintln!(
        "snapshot: {} ({}x{} px, after {waited} more DWM compositions)",
        request.path.display(),
        image.width,
        image.height
    );
    if request.check {
        let blank = blank(request, &image, shot);
        if !blank.is_empty() {
            bail!("snapshot check failed: blank {}", blank.join(", "));
        }
        eprintln!("snapshot check passed");
    }
    Ok(())
}

/// What the request's check finds blank in `image`: the shell's parts, and
/// the panes when the request asks for them.
fn blank(request: &Request, image: &Image, shot: &Shot) -> Vec<&'static str> {
    let mut blank = blank_parts(image, shot.scale, &shot.palette, request.system_title_bar);
    if request.panes {
        blank.extend(blank_panes(image, shot.scale));
    }
    blank
}

/// Blocks this (background) thread until DWM's next composition pass.
#[cfg(windows)]
fn wait_for_composition() {
    // SAFETY: no arguments; fails only when composition is off, and then
    // the next capture simply comes sooner.
    let _ = unsafe { windows::Win32::Graphics::Dwm::DwmFlush() };
}

#[cfg(not(windows))]
fn wait_for_composition() {}

/// The window's HWND (raw-window-handle, as for the window mark).
#[cfg(windows)]
pub fn window_handle(window: &gpui::Window) -> Option<isize> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let handle = HasWindowHandle::window_handle(window).ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
        _ => None,
    }
}

#[cfg(not(windows))]
pub fn window_handle(_: &gpui::Window) -> Option<isize> {
    None
}

/// The client area of `hwnd` as DWM composes it (Skua's skua-shot
/// capture: a top-down 32-bit DIB, alpha set opaque).
#[cfg(windows)]
pub fn capture(hwnd: isize) -> Result<Image> {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleBitmap, CreateCompatibleDC,
        DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, HGDIOBJ, ReleaseDC, SelectObject,
    };
    use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
    use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
    /// `PW_CLIENTONLY | PW_RENDERFULLCONTENT`.
    const PW_CLIENT_FULL: PRINT_WINDOW_FLAGS = PRINT_WINDOW_FLAGS(1 | 2);

    let hwnd = HWND(hwnd as *mut core::ffi::c_void);
    // SAFETY: plain Win32 calls on a live window this process owns; every
    // GDI object made here is released before returning. The UI thread is
    // free meanwhile to answer PrintWindow's message.
    unsafe {
        let mut rect = RECT::default();
        GetClientRect(hwnd, &mut rect).context("GetClientRect")?;
        let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
        if w <= 0 || h <= 0 {
            bail!("the window has an empty client area ({w}x{h}); is it minimized?");
        }
        let screen = GetDC(Some(hwnd));
        let mem = CreateCompatibleDC(Some(screen));
        let bitmap = CreateCompatibleBitmap(screen, w, h);
        let previous = SelectObject(mem, HGDIOBJ(bitmap.0));
        let printed = PrintWindow(hwnd, mem, PW_CLIENT_FULL).as_bool();
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        // The bitmap must not be selected into a DC while GetDIBits reads it.
        SelectObject(mem, previous);
        let lines = GetDIBits(
            mem,
            bitmap,
            0,
            h as u32,
            Some(pixels.as_mut_ptr().cast()),
            &mut info,
            DIB_RGB_COLORS,
        );
        let freed = DeleteObject(HGDIOBJ(bitmap.0)).as_bool() & DeleteDC(mem).as_bool();
        ReleaseDC(Some(hwnd), screen);
        if !printed {
            bail!("PrintWindow failed");
        }
        if lines != h {
            bail!("GetDIBits copied {lines} of {h} rows");
        }
        if !freed {
            eprintln!("snapshot: a GDI object was not freed");
        }
        for px in pixels.chunks_exact_mut(4) {
            px.swap(0, 2);
            px[3] = 255;
        }
        Ok(Image {
            width: w as u32,
            height: h as u32,
            rgba: pixels,
        })
    }
}

#[cfg(not(windows))]
pub fn capture(_: isize) -> Result<Image> {
    bail!("snapshots need Windows")
}

/// A PNG when `path` ends in `.png`, else a 32-bit BMP (the Slint app's
/// format).
pub fn write(image: &Image, path: &Path) -> Result<()> {
    let png = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("png"));
    let bytes = if png {
        encode_png(image)?
    } else {
        encode_bmp(image)
    };
    std::fs::write(path, bytes)?;
    Ok(())
}

fn encode_png(image: &Image) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&image.rgba)?;
    writer.finish()?;
    Ok(out)
}

fn encode_bmp(image: &Image) -> Vec<u8> {
    let (w, h) = (image.width, image.height);
    let size = 54 + w * h * 4;
    let mut bmp = Vec::with_capacity(size as usize);
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&size.to_le_bytes());
    bmp.extend_from_slice(&[0; 4]);
    bmp.extend_from_slice(&54u32.to_le_bytes());
    bmp.extend_from_slice(&40u32.to_le_bytes());
    bmp.extend_from_slice(&(w as i32).to_le_bytes());
    bmp.extend_from_slice(&(-(h as i32)).to_le_bytes());
    bmp.extend_from_slice(&1u16.to_le_bytes());
    bmp.extend_from_slice(&32u16.to_le_bytes());
    bmp.extend_from_slice(&[0; 24]);
    for p in image.rgba.chunks_exact(4) {
        bmp.extend_from_slice(&[p[2], p[1], p[0], 255]);
    }
    bmp
}

/// The parts of the shell a snapshot (at `scale` physical px per logical
/// px) shows wrong: "title bar" when its left part (where the title is)
/// has no ink on the canvas colour, "pane" when most of the middle of the
/// window below the title bar is not the paper colour. Empty when the shell
/// drew.
pub fn blank_parts(
    image: &Image,
    scale: f32,
    palette: &Palette,
    system_title_bar: bool,
) -> Vec<&'static str> {
    let phys = |logical: f32| (logical * scale).round() as u32;
    let mut blank = Vec::new();
    let title = (0..phys(200.), 0..phys(48.));
    let canvas = rgb8(palette.canvas);
    let background = most_common(image, title.0.clone(), title.1.clone());
    if !system_title_bar
        && (background.is_none_or(|c| !near(c, canvas))
            || ink(image, title.0.clone(), title.1.clone()) < 0.005)
    {
        blank.push("title bar");
    }
    // The paper below the title bar, over the middle half of the width (a
    // card or a field under one pixel would not make it blank): the colour
    // most of that band has must be the paper's.
    let band = most_common(
        image,
        image.width / 4..image.width * 3 / 4,
        phys(48.)..image.height,
    );
    if band.is_none_or(|c| !near(c, rgb8(palette.paper))) {
        blank.push("pane");
    }
    blank
}

/// The panes of a mail snapshot (taken at `scale` physical pixels per
/// logical one) that show nothing: "list" or "reader", the inner parts of
/// the panes at the window's width (the folder pane or rail, then the list,
/// then the reading pane, 8 px apart, as `views::mail` lays them out) with
/// less than 1% of their pixels unlike their background. The Slint app's
/// `blank_panes`. Empty below the one-pane width, where which pane shows
/// depends on the message.
pub fn blank_panes(image: &Image, scale: f32) -> Vec<&'static str> {
    let phys = |logical: f32| (logical * scale).round() as u32;
    let width = image.width as f32 / scale;
    if width < crate::views::mail::ONE_PANE_BELOW_W {
        return Vec::new();
    }
    let (nav, list) = if width < layout::COMPACT_BELOW_W {
        (layout::RAIL_W, layout::LIST_W_COMPACT)
    } else {
        (layout::NAV_W, layout::LIST_W)
    };
    let gap = space::PANE_GAP;
    let (h, w) = (image.height, image.width);
    let rows = h / 5..h * 9 / 10;
    let first = nav + gap;
    let panes = [
        ("list", phys(first + gap)..phys(first + list - gap)),
        (
            "reader",
            phys(first + list + gap + space::SP_4)..w.saturating_sub(phys(space::SP_4)),
        ),
    ];
    panes
        .into_iter()
        .filter(|(_, cols)| ink(image, cols.clone(), rows.clone()) < 0.01)
        .map(|(name, _)| name)
        .collect()
}

/// A token colour as 8-bit RGB (alpha ignored: the shell's colours are
/// opaque).
fn rgb8(c: gpui::Hsla) -> [u8; 3] {
    let c = gpui::Rgba::from(c);
    let byte = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
    [byte(c.r), byte(c.g), byte(c.b)]
}

/// Within 3 per channel (WARP and GPUs blend a little differently).
fn near(a: [u8; 3], b: [u8; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 3)
}

fn clip(r: std::ops::Range<u32>, max: u32) -> std::ops::Range<u32> {
    r.start.min(r.end).min(max)..r.end.min(max)
}

fn counts(
    image: &Image,
    cols: std::ops::Range<u32>,
    rows: std::ops::Range<u32>,
) -> std::collections::HashMap<[u8; 3], usize> {
    let (cols, rows) = (clip(cols, image.width), clip(rows, image.height));
    let mut counts = std::collections::HashMap::new();
    for y in rows {
        for x in cols.clone() {
            *counts.entry(image.pixel(x, y)).or_insert(0usize) += 1;
        }
    }
    counts
}

fn most_common(
    image: &Image,
    cols: std::ops::Range<u32>,
    rows: std::ops::Range<u32>,
) -> Option<[u8; 3]> {
    counts(image, cols, rows)
        .into_iter()
        .max_by_key(|(_, n)| *n)
        .map(|(c, _)| c)
}

/// The share of the pixels in a region (clipped to the image) that differ
/// from its most common colour; 0 for an empty region (the Slint app's
/// `ink`).
pub fn ink(image: &Image, cols: std::ops::Range<u32>, rows: std::ops::Range<u32>) -> f64 {
    let counts = counts(image, cols, rows);
    let total: usize = counts.values().sum();
    let background = counts.values().copied().max().unwrap_or(0);
    if total == 0 {
        return 0.0;
    }
    (total - background) as f64 / total as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn parse_env(pairs: &[(&str, &str)]) -> Result<Option<Request>, String> {
        let vars = env(pairs);
        let any = vars.keys().any(|k| k.starts_with("KOMOREBI_SNAPSHOT"));
        parse(
            |k| vars.get(k).cloned(),
            any,
            |p| p.to_string_lossy().contains("AppData\\Local\\komorebi"),
        )
    }

    const SAFE: [(&str, &str); 3] = [
        ("KOMOREBI_SNAPSHOT", "out.png"),
        ("KOMOREBI_OFFLINE", "1"),
        ("KOMOREBI_DB", "F:\\scratch\\seed.sqlite3"),
    ];

    #[test]
    fn no_switch_is_no_snapshot() {
        assert_eq!(parse_env(&[("KOMOREBI_OFFLINE", "1")]), Ok(None));
    }

    #[test]
    fn a_snapshot_needs_offline_and_a_throwaway_cache() {
        assert!(parse_env(&[("KOMOREBI_SNAPSHOT", "out.png")]).is_err());
        assert!(
            parse_env(&[("KOMOREBI_SNAPSHOT", "a.png"), ("KOMOREBI_OFFLINE", "1")])
                .unwrap_err()
                .contains("KOMOREBI_DB")
        );
        assert!(
            parse_env(&[
                ("KOMOREBI_SNAPSHOT", "a.png"),
                ("KOMOREBI_DB", "F:\\scratch\\x.sqlite3")
            ])
            .unwrap_err()
            .contains("KOMOREBI_OFFLINE")
        );
        let real = parse_env(&[
            ("KOMOREBI_SNAPSHOT", "a.png"),
            ("KOMOREBI_OFFLINE", "1"),
            (
                "KOMOREBI_DB",
                "C:\\Users\\x\\AppData\\Local\\komorebi\\cache.sqlite3",
            ),
        ]);
        assert!(real.unwrap_err().contains("throwaway"));
        // Any snapshot switch alone is refused too: nothing is opened.
        assert!(parse_env(&[("KOMOREBI_SNAPSHOT_CHECK", "1")]).is_err());
    }

    #[test]
    fn the_switches_of_0_7_carry_over() {
        let mut pairs = SAFE.to_vec();
        pairs.extend([
            ("KOMOREBI_THEME", "dark"),
            ("KOMOREBI_SIZE", "1280x800"),
            ("KOMOREBI_SNAPSHOT_CHECK", "1"),
            ("KOMOREBI_SNAPSHOT_SELECT", "0"),
            ("KOMOREBI_SNAPSHOT_FOCUS", "list"),
        ]);
        let r = parse_env(&pairs).unwrap().unwrap();
        assert_eq!(r.path, PathBuf::from("out.png"));
        assert_eq!(r.theme, Some(ThemeChoice::Dark));
        assert_eq!(r.size, Some((1280., 800.)));
        assert_eq!(r.action, "shell");
        assert!(r.check);
        assert_eq!(r.drive.focus, Some(Focus::List));
        assert!(!r.system_title_bar);
        for (k, v) in [
            ("KOMOREBI_THEME", "contrast"),
            ("KOMOREBI_SIZE", "12x8"),
            ("KOMOREBI_SIZE", "wide"),
            ("KOMOREBI_SNAPSHOT_ACTION", "reply"),
        ] {
            let mut bad = SAFE.to_vec();
            bad.push((k, v));
            assert!(parse_env(&bad).is_err(), "{k}={v}");
        }
    }

    /// The compose action is known (its view is the composer in the Mail
    /// tab's reading pane, `window::places`).
    #[test]
    fn the_compose_action_is_known() {
        let mut pairs = SAFE.to_vec();
        pairs.push(("KOMOREBI_SNAPSHOT_ACTION", "compose"));
        assert_eq!(parse_env(&pairs).unwrap().unwrap().action, "compose");
        assert!(ACTIONS.contains(&"compose"));
        // Not the mail view's one-row pane check: the composer is open.
        pairs.push(("KOMOREBI_SNAPSHOT_SELECT", "0"));
        assert!(!parse_env(&pairs).unwrap().unwrap().panes);
    }

    fn drive(extra: &[(&str, &str)]) -> Result<Drive, String> {
        let mut pairs = SAFE.to_vec();
        pairs.extend_from_slice(extra);
        parse_env(&pairs).map(|r| r.unwrap().drive)
    }

    /// 0.7's focus switch: the same words, the keys the focus takes.
    #[test]
    fn the_focus_switch_names_the_keys_it_presses() {
        let focus = |v: &str| {
            drive(&[("KOMOREBI_SNAPSHOT_FOCUS", v)])
                .unwrap()
                .focus
                .unwrap()
        };
        assert_eq!(focus("list").keystrokes(), Vec::<&str>::new());
        assert_eq!(focus("nav").keystrokes(), ["shift-f6"]);
        assert_eq!(focus("nav:2").keystrokes(), ["shift-f6", "down", "down"]);
        assert_eq!(
            focus("nav:0:udle").keystrokes(),
            ["shift-f6", "up", "down", "left", "enter"]
        );
        assert_eq!(focus("toolbar").keystrokes(), ["tab"]);
        assert_eq!(focus("toolbar:2").keystrokes(), ["tab", "right", "right"]);
        assert_eq!(focus("tab:3").keystrokes(), ["tab", "tab", "tab"]);
        assert_eq!(
            focus("press:ttnpzhf").keystrokes(),
            ["tab", "tab", "pagedown", "pageup", "end", "home", "f6"]
        );
        for bad in [
            "", "nav:x", "nav:1:q", "tab", "tab:1000", "press:", "press:x", "side",
        ] {
            let r = drive(&[("KOMOREBI_SNAPSHOT_FOCUS", bad)]);
            assert!(r.unwrap_err().contains("KOMOREBI_SNAPSHOT_FOCUS"), "{bad}");
        }
    }

    #[test]
    fn walk_scroll_and_hover_are_read() {
        let d = drive(&[
            ("KOMOREBI_SNAPSHOT_WALK", "40"),
            ("KOMOREBI_SNAPSHOT_SCROLL", "600"),
            ("KOMOREBI_SNAPSHOT_HOVER", "120, 80:click"),
        ])
        .unwrap();
        assert_eq!(d.walk, 40);
        assert_eq!(d.scroll, Some(600.));
        assert_eq!(
            d.hover,
            Some(Hover::At {
                x: 120.,
                y: 80.,
                how: Pointer::Click
            })
        );
        assert!(!d.is_empty());
        // The walk is capped as the 0.7 app capped it.
        assert_eq!(
            drive(&[("KOMOREBI_SNAPSHOT_WALK", "5000")]).unwrap().walk,
            1000
        );
        assert!(drive(&[]).unwrap().is_empty());
        use crate::titlebar::Caption;
        let caption = |v: &str| drive(&[("KOMOREBI_SNAPSHOT_HOVER", v)]).unwrap().hover;
        assert_eq!(
            caption("caption-close:press"),
            Some(Hover::Caption {
                which: Caption::Close,
                press: true
            })
        );
        assert_eq!(
            caption("caption-min"),
            Some(Hover::Caption {
                which: Caption::Minimize,
                press: false
            })
        );
        assert_eq!(
            caption("5,6:press"),
            Some(Hover::At {
                x: 5.,
                y: 6.,
                how: Pointer::Press
            })
        );
        for (k, v) in [
            ("KOMOREBI_SNAPSHOT_WALK", "many"),
            ("KOMOREBI_SNAPSHOT_SCROLL", "far"),
            ("KOMOREBI_SNAPSHOT_SCROLL", "inf"),
            ("KOMOREBI_SNAPSHOT_HOVER", "10"),
            ("KOMOREBI_SNAPSHOT_HOVER", "-1,4"),
            ("KOMOREBI_SNAPSHOT_HOVER", "1,4:drag"),
            ("KOMOREBI_SNAPSHOT_HOVER", "caption-close:click"),
        ] {
            assert!(drive(&[(k, v)]).unwrap_err().contains(k), "{k}={v}");
        }
    }

    #[test]
    fn the_title_bar_switch_is_custom_or_system() {
        let with = |v: &str| {
            let mut pairs = SAFE.to_vec();
            pairs.push(("KOMOREBI_SNAPSHOT_TITLEBAR", v));
            parse_env(&pairs)
        };
        assert!(!with("custom").unwrap().unwrap().system_title_bar);
        assert!(with("system").unwrap().unwrap().system_title_bar);
        assert!(with("native").unwrap_err().contains("custom or system"));
    }

    fn filled(w: u32, h: u32, c: [u8; 3]) -> Image {
        Image {
            width: w,
            height: h,
            rgba: (0..w * h).flat_map(|_| [c[0], c[1], c[2], 255]).collect(),
        }
    }

    fn paint(
        image: &mut Image,
        cols: std::ops::Range<u32>,
        rows: std::ops::Range<u32>,
        c: [u8; 3],
    ) {
        for y in rows {
            for x in cols.clone() {
                let i = ((y * image.width + x) * 4) as usize;
                image.rgba[i..i + 3].copy_from_slice(&c);
            }
        }
    }

    /// Port of the Slint app's `ink_counts_pixels_unlike_the_background`
    /// (`src/main.rs`) to the GPUI snapshot's image.
    #[test]
    fn ink_counts_pixels_unlike_the_background() {
        let mut image = filled(100, 50, [255, 255, 255]);
        assert_eq!(ink(&image, 0..100, 0..50), 0.0);
        // A 10x5 "word" at (20, 10).
        paint(&mut image, 20..30, 10..15, [0, 0, 0]);
        assert_eq!(ink(&image, 0..100, 0..50), 50.0 / 5000.0);
        assert_eq!(ink(&image, 20..30, 10..15), 0.0, "all black: no background");
        assert_eq!(ink(&image, 0..40, 0..20), 50.0 / 800.0);
        assert_eq!(ink(&image, 50..100, 0..50), 0.0);
        // Clipped to the image; empty regions count as blank.
        assert_eq!(ink(&image, 90..500, 40..500), 0.0);
        assert_eq!(ink(&image, 200..300, 0..50), 0.0);
        let reversed = std::ops::Range { start: 30, end: 20 };
        assert_eq!(ink(&image, reversed, 0..50), 0.0);
    }

    #[test]
    fn the_shell_check_names_what_is_missing() {
        let p = crate::theme::color::palette(crate::theme::Scheme::Light);
        let (w, h) = (1280, 800);
        let mut shell = filled(w, h, rgb8(p.canvas));
        // The pane, and the title as a few strokes of ink.
        paint(&mut shell, 8..w - 8, 48..h - 8, rgb8(p.paper));
        for x in (16..80).step_by(4) {
            paint(&mut shell, x..x + 2, 18..30, rgb8(p.ink_2));
        }
        assert!(blank_parts(&shell, 1.0, &p, false).is_empty());
        assert_eq!(
            blank_parts(&filled(w, h, rgb8(p.canvas)), 1.0, &p, false),
            ["title bar", "pane"]
        );
        // A card (canvas-coloured, as the settings cards are) under the
        // middle of the window does not make the pane blank.
        paint(&mut shell, 480..800, 380..470, rgb8(p.canvas));
        assert!(blank_parts(&shell, 1.0, &p, false).is_empty());
        // Black, as an undrawn swap chain captures.
        assert_eq!(
            blank_parts(&filled(w, h, [0, 0, 0]), 1.0, &p, false),
            ["title bar", "pane"]
        );
        // Under the system caption the client area has no title bar to
        // check (`KOMOREBI_SNAPSHOT_TITLEBAR=system`): the pane still is.
        assert_eq!(
            blank_parts(&filled(w, h, [0, 0, 0]), 1.0, &p, true),
            ["pane"]
        );
    }

    /// Port of the Slint app's `blank_panes_names_the_empty_ones`
    /// (`src/main.rs`) to the GPUI layout: the folder pane is 240 px (the
    /// rail 56 px below 1000 px), then the list (384 px, 320 compact), the
    /// panes 8 px apart.
    #[test]
    fn blank_panes_names_the_empty_ones() {
        let white = [250, 250, 250];
        let grey = [40, 40, 40];
        // Ink in the middle of each pane, as stripes of text.
        let draw = |w: u32, h: u32, cols: &[(u32, u32)]| {
            let mut image = filled(w, h, white);
            for y in (h / 4..h * 3 / 4).step_by(8) {
                for &(a, b) in cols {
                    paint(&mut image, a..b, y..y + 1, grey);
                }
            }
            image
        };
        let wide = draw(1280, 800, &[(300, 600), (700, 1200)]);
        assert!(blank_panes(&wide, 1.0).is_empty());
        assert_eq!(
            blank_panes(&draw(1280, 800, &[(300, 600)]), 1.0),
            ["reader"]
        );
        assert_eq!(blank_panes(&draw(1280, 800, &[]), 1.0), ["list", "reader"]);
        // At 150%, the panes are 1.5 times as far in.
        let scaled = draw(1920, 1200, &[(450, 900), (1050, 1800)]);
        assert!(blank_panes(&scaled, 1.5).is_empty());
        // Below 1000 px the rail is narrow: 900 px wide, the list is at 72-376.
        assert!(blank_panes(&draw(900, 700, &[(100, 300), (450, 850)]), 1.0).is_empty());
        assert_eq!(blank_panes(&draw(900, 700, &[(450, 850)]), 1.0), ["list"]);
        // One pane at a time below 720 px: which one shows depends on the
        // message, so nothing is named.
        assert!(blank_panes(&draw(700, 700, &[]), 1.0).is_empty());
    }

    /// The pane check runs for the mail view with one row opened only: the
    /// other views and an empty cache have no list to find.
    #[test]
    fn the_pane_check_is_for_one_opened_row() {
        let with = |extra: &[(&str, &str)]| {
            let mut pairs = SAFE.to_vec();
            pairs.extend_from_slice(extra);
            parse_env(&pairs).unwrap().unwrap()
        };
        assert!(!with(&[]).panes);
        assert!(with(&[("KOMOREBI_SNAPSHOT_SELECT", "0")]).panes);
        assert!(!with(&[("KOMOREBI_SNAPSHOT_SELECT", "0,2")]).panes);
        assert!(
            !with(&[
                ("KOMOREBI_SNAPSHOT_SELECT", "0"),
                ("KOMOREBI_SNAPSHOT_ACTION", "contacts")
            ])
            .panes
        );
    }

    #[test]
    fn images_are_written_as_bmp_or_png() {
        let image = filled(3, 2, [1, 2, 3]);
        let bmp = encode_bmp(&image);
        assert_eq!(&bmp[..2], b"BM");
        assert_eq!(bmp.len(), 54 + 3 * 2 * 4);
        assert_eq!(&bmp[54..58], &[3, 2, 1, 255]);
        let png = encode_png(&image).unwrap();
        assert_eq!(&png[1..4], b"PNG");
    }
}
