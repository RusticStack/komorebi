//! The window side of the app-drawn title bar (docs/WINUI3.md "Title bar"
//! and "Title bar and frame"): which mode the bar is in, the hit test that
//! tells Windows where the caption, the caption buttons and the resize
//! band are, and (on Windows) the window subclass that gives the caption
//! area to the client and drives the caption buttons.
//!
//! Everything the window procedure reads is plain data in a thread-local
//! ([`set_bar`], [`set_custom`]): it never touches Slint or the app state,
//! since it can run under a native dialog's nested message loop. Caption
//! button hover and pressed states go out through one function pointer
//! ([`set_notify`]), which the app turns into a coalesced
//! `invoke_from_event_loop`.

/// `WM_NCHITTEST` results (winuser.h), here so the hit test is plain,
/// testable code on any platform.
pub const HTCLIENT: u32 = 1;
pub const HTCAPTION: u32 = 2;
pub const HTMINBUTTON: u32 = 8;
pub const HTMAXBUTTON: u32 = 9;
pub const HTLEFT: u32 = 10;
pub const HTRIGHT: u32 = 11;
pub const HTTOP: u32 = 12;
pub const HTTOPLEFT: u32 = 13;
pub const HTTOPRIGHT: u32 = 14;
pub const HTBOTTOM: u32 = 15;
pub const HTCLOSE: u32 = 20;

/// A caption button's width and height in logical px (WinUI's 46 × 48 for
/// the expanded bar); they never scale with the text size.
pub const BUTTON_W: f32 = 46.0;
pub const BUTTON_H: f32 = 48.0;

/// The Windows build that brought Windows 11 (and its custom-frame top
/// border, rounded corners and Snap Layouts on HTMAXBUTTON).
pub const WINDOWS_11: u32 = 22000;

/// A rectangle in physical client px (`w`, `h` exclusive edges).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PxRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl PxRect {
    /// A logical rectangle at `scale` physical px per logical px, its edges
    /// rounded on their own (so neighbours share an edge: no pixel is lost
    /// or counted twice).
    pub fn from_logical(x: f32, y: f32, w: f32, h: f32, scale: f32) -> PxRect {
        let px = |v: f32| (v * scale).round() as i32;
        let (left, top) = (px(x), px(y));
        PxRect {
            x: left,
            y: top,
            w: px(x + w) - left,
            h: px(y + h) - top,
        }
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

/// What the Slint bar pushes: its drawn height (physical px; taller than
/// 48 logical px at a large text size), Slint's scale, the interactive
/// rectangles in it (the search box, its scope control), and whether an
/// in-window popup is open.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Bar {
    pub height: i32,
    pub scale: f32,
    pub passthrough: Vec<PxRect>,
    pub popup: bool,
}

/// One of the three caption buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Minimize,
    Maximize,
    Close,
}

impl Button {
    /// Its hit-test code.
    pub fn code(self) -> u32 {
        match self {
            Button::Minimize => HTMINBUTTON,
            Button::Maximize => HTMAXBUTTON,
            Button::Close => HTCLOSE,
        }
    }

    pub fn from_code(code: u32) -> Option<Button> {
        match code {
            HTMINBUTTON => Some(Button::Minimize),
            HTMAXBUTTON => Some(Button::Maximize),
            HTCLOSE => Some(Button::Close),
            _ => None,
        }
    }

    /// Its index for the Slint side (1 minimize, 2 maximize, 3 close; 0 is
    /// none).
    pub fn index(self) -> i32 {
        match self {
            Button::Minimize => 1,
            Button::Maximize => 2,
            Button::Close => 3,
        }
    }

    pub fn from_index(index: i32) -> Option<Button> {
        match index {
            1 => Some(Button::Minimize),
            2 => Some(Button::Maximize),
            3 => Some(Button::Close),
            _ => None,
        }
    }
}

/// The caption button under client point `x, y` (physical px) of a client
/// area `client_w` wide: three 46 × 48 columns at the top right, their
/// edges rounded from the right edge.
pub fn button_at(x: i32, y: i32, client_w: i32, scale: f32) -> Option<Button> {
    let px = |v: f32| (v * scale).round() as i32;
    if y < 0 || y >= px(BUTTON_H) || x >= client_w {
        return None;
    }
    let edge = |k: f32| client_w - px(k * BUTTON_W);
    if x >= edge(1.0) {
        Some(Button::Close)
    } else if x >= edge(2.0) {
        Some(Button::Maximize)
    } else if x >= edge(3.0) {
        Some(Button::Minimize)
    } else {
        None
    }
}

/// Where client point `x, y` (physical px) is, for `WM_NCHITTEST`, in a
/// client area `client_w` wide whose top resize band is `frame_y` tall:
/// the resize band along the top (not while maximized), the caption
/// buttons, the bar's interactive parts (client), the rest of the bar
/// (caption: drag, double-click, system menu) and the content (client).
/// While an in-window popup is open everything but the buttons is client,
/// so a click there closes the popup instead of dragging the window.
pub fn hit_test(x: i32, y: i32, client_w: i32, maximized: bool, frame_y: i32, bar: &Bar) -> u32 {
    if !maximized && y < frame_y {
        // The corners resize diagonally over a band as wide as two frames,
        // as the native frame's corners do.
        let corner = 2 * frame_y;
        return if x < corner {
            HTTOPLEFT
        } else if x >= client_w - corner {
            HTTOPRIGHT
        } else {
            HTTOP
        };
    }
    if let Some(b) = button_at(x, y, client_w, bar.scale) {
        return b.code();
    }
    if y >= 0 && y < bar.height {
        if bar.popup || bar.passthrough.iter().any(|r| r.contains(x, y)) {
            return HTCLIENT;
        }
        return HTCAPTION;
    }
    HTCLIENT
}

/// DefWindowProc's answer for the native side borders, upgraded: a side
/// border point level with the top resize band (client `y < frame_y`, not
/// maximized) resizes the corner, as on every native window.
pub fn upgrade_border(def: u32, y: i32, maximized: bool, frame_y: i32) -> u32 {
    if maximized || y >= frame_y {
        return def;
    }
    match def {
        HTLEFT => HTTOPLEFT,
        HTRIGHT => HTTOPRIGHT,
        other => other,
    }
}

/// The screen point in a mouse message's lParam: two **signed** 16-bit
/// values (monitors left of or above the primary have negative ones).
pub fn point_from_lparam(lparam: isize) -> (i32, i32) {
    let x = (lparam & 0xffff) as u16 as i16 as i32;
    let y = ((lparam >> 16) & 0xffff) as u16 as i16 as i32;
    (x, y)
}

/// The client top after `WM_NCCALCSIZE`: the window top (the caption is
/// the app's), or `frame_y` below it while maximized, where the window
/// reaches that far above the screen.
pub fn nccalc_top(window_top: i32, maximized: bool, frame_y: i32) -> i32 {
    window_top + if maximized { frame_y } else { 0 }
}

/// The screen edges holding an auto-hide taskbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Edges {
    pub left: bool,
    pub top: bool,
    pub right: bool,
    pub bottom: bool,
}

/// How far a maximized client area keeps off each edge (left, top, right,
/// bottom): 2 px off an edge with an auto-hide taskbar, so the pointer can
/// still reach the screen edge and show it.
pub const AUTOHIDE_INSET: i32 = 2;
pub fn autohide_insets(edges: Edges) -> [i32; 4] {
    let inset = |on: bool| if on { AUTOHIDE_INSET } else { 0 };
    [
        inset(edges.left),
        inset(edges.top),
        inset(edges.right),
        inset(edges.bottom),
    ]
}

/// Whether a `WM_WINDOWPOSCHANGING` with these `SWP_*` flags is one of
/// winit's client-size requests (which add a system caption the custom
/// frame gives back to the client), whose height must lose that caption:
/// a size change with no move outside a user's size/move loop, or any
/// size change while winit applies a DPI change. Never one that changes
/// the frame (the app's own install and mode switches).
pub fn corrects_size(flags: u32, in_size_move: bool, dpi_change: bool) -> bool {
    const SWP_NOSIZE: u32 = 0x1;
    const SWP_NOMOVE: u32 = 0x2;
    const SWP_FRAMECHANGED: u32 = 0x20;
    if flags & SWP_NOSIZE != 0 || flags & SWP_FRAMECHANGED != 0 {
        return false;
    }
    dpi_change || (!in_size_move && flags & SWP_NOMOVE != 0)
}

/// Whether the bar is app-drawn ("custom") rather than the system caption:
/// on Windows 11 (`build`), not under high contrast, and not switched off
/// with `KOMOREBI_SYSTEM_TITLEBAR` (`system_switch`). A snapshot draws the
/// custom bar unless under high contrast; `forced` (`custom` or `system`,
/// from `KOMOREBI_SNAPSHOT_TITLEBAR`) wins over everything.
pub fn custom_wanted(
    build: u32,
    high_contrast: bool,
    system_switch: bool,
    snapshot: bool,
    forced: Option<&str>,
) -> bool {
    match forced {
        Some("custom") => return true,
        Some("system") => return false,
        _ => {}
    }
    if high_contrast {
        return false;
    }
    snapshot || (build >= WINDOWS_11 && !system_switch)
}

#[cfg(windows)]
pub use win::*;

#[cfg(windows)]
mod win {
    use super::{Bar, Button, Edges};
    use std::cell::{Cell, RefCell};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow, ScreenToClient,
    };
    use windows_sys::Win32::UI::HiDpi::{
        AdjustWindowRectExForDpi, GetDpiForWindow, GetSystemMetricsForDpi,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, TrackMouseEvent,
    };
    use windows_sys::Win32::UI::Shell::{
        ABE_BOTTOM, ABE_LEFT, ABE_RIGHT, ABE_TOP, ABM_GETAUTOHIDEBAREX, APPBARDATA,
        DefSubclassProc, RemoveWindowSubclass, SHAppBarMessage, SetWindowSubclass,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CBT_CREATEWNDW, CallNextHookEx, GWL_EXSTYLE, GWL_STYLE, GetClassNameW, GetClientRect,
        GetWindowLongW, GetWindowRect, HCBT_CREATEWND, HHOOK, IsIconic, IsZoomed, MINMAXINFO,
        NCCALCSIZE_PARAMS, PostMessageW, SC_CLOSE, SC_MAXIMIZE, SC_MINIMIZE, SC_RESTORE,
        SM_CXPADDEDBORDER, SM_CYFRAME, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE,
        SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos, SetWindowsHookExW,
        UnhookWindowsHookEx, WH_CBT, WINDOWPOS, WM_DPICHANGED, WM_ENTERSIZEMOVE, WM_EXITSIZEMOVE,
        WM_GETMINMAXINFO, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCACTIVATE, WM_NCCALCSIZE, WM_NCDESTROY,
        WM_NCHITTEST, WM_NCLBUTTONDBLCLK, WM_NCLBUTTONDOWN, WM_NCLBUTTONUP, WM_NCMOUSELEAVE,
        WM_NCMOUSEMOVE, WM_SYSCOMMAND, WM_WINDOWPOSCHANGING, WS_CAPTION, WS_CHILD,
    };

    /// The subclass id of [`chrome_proc`] (the appearance listener is 1).
    pub const SUBCLASS_ID: usize = 2;

    #[derive(Default)]
    struct State {
        custom: bool,
        bar: Bar,
        hover: Option<Button>,
        pressed: Option<Button>,
        tracking: bool,
        in_size_move: bool,
        dpi_change: bool,
        hook: HHOOK,
        installed: HWND,
    }

    thread_local! {
        static STATE: RefCell<State> = RefCell::new(State {
            hook: std::ptr::null_mut(),
            installed: std::ptr::null_mut(),
            ..Default::default()
        });
        /// Told when the hover or pressed caption button changes.
        static NOTIFY: Cell<Option<fn()>> = const { Cell::new(None) };
    }

    /// Whether the bar is app-drawn now.
    pub fn custom() -> bool {
        STATE.with(|s| s.borrow().custom)
    }

    /// Set the mode (before the window exists, or with [`frame_changed`]
    /// after). Returns whether it changed.
    pub fn set_custom(custom: bool) -> bool {
        STATE.with(|s| std::mem::replace(&mut s.borrow_mut().custom, custom) != custom)
    }

    /// The bar's height, scale and interactive rectangles, as drawn.
    pub fn set_bar(bar: Bar) {
        STATE.with(|s| s.borrow_mut().bar = bar);
    }

    pub fn bar() -> Bar {
        STATE.with(|s| s.borrow().bar.clone())
    }

    /// The caption button under the pointer and the one held down.
    pub fn caption_state() -> (Option<Button>, Option<Button>) {
        STATE.with(|s| {
            let s = s.borrow();
            (s.hover, s.pressed)
        })
    }

    /// Call `f` (on this thread, from the window procedure) when the
    /// caption state changes. It must not touch Slint directly.
    pub fn set_notify(f: fn()) {
        NOTIFY.set(Some(f));
    }

    fn update(change: impl FnOnce(&mut State)) {
        let changed = STATE.with(|s| {
            let mut s = s.borrow_mut();
            let before = (s.hover, s.pressed);
            change(&mut s);
            before != (s.hover, s.pressed)
        });
        if changed && let Some(f) = NOTIFY.get() {
            f();
        }
    }

    /// The window `chrome_proc` was installed on (null: none yet).
    pub fn installed() -> HWND {
        STATE.with(|s| s.borrow().installed)
    }

    /// Subclass `hwnd` with [`chrome_proc`].
    ///
    /// # Safety
    /// `hwnd` must be a live window of this thread.
    pub unsafe fn install(hwnd: HWND) -> bool {
        // SAFETY: the caller's window, and a procedure that removes itself
        // when the window is destroyed.
        let ok = unsafe { SetWindowSubclass(hwnd, Some(chrome_proc), SUBCLASS_ID, 0) } != 0;
        if ok {
            STATE.with(|s| s.borrow_mut().installed = hwnd);
        }
        ok
    }

    /// Install [`chrome_proc`] on the next captioned top-level window this
    /// thread creates, as it is created: its first `WM_NCCALCSIZE` is then
    /// already the custom one, so no system caption is ever drawn.
    pub fn hook_creation() {
        // SAFETY: a thread hook (no module handle) on this thread, removed
        // by its own procedure once it has found the window.
        let hook = unsafe {
            SetWindowsHookExW(
                WH_CBT,
                Some(cbt_proc),
                std::ptr::null_mut(),
                windows_sys::Win32::System::Threading::GetCurrentThreadId(),
            )
        };
        STATE.with(|s| s.borrow_mut().hook = hook);
    }

    /// Remove the creation hook if it is still waiting.
    pub fn unhook_creation() {
        let hook =
            STATE.with(|s| std::mem::replace(&mut s.borrow_mut().hook, std::ptr::null_mut()));
        if !hook.is_null() {
            // SAFETY: the hook set by `hook_creation`.
            unsafe { UnhookWindowsHookEx(hook) };
        }
    }

    unsafe extern "system" fn cbt_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code == HCBT_CREATEWND as i32 {
            let hwnd = wparam as HWND;
            // SAFETY: for HCBT_CREATEWND, lParam is a CBT_CREATEWNDW whose
            // CREATESTRUCTW lives for the call.
            let cs = unsafe { &*(*(lparam as *const CBT_CREATEWNDW)).lpcs };
            let style = cs.style as u32;
            if cs.hwndParent.is_null()
                && style & WS_CAPTION == WS_CAPTION
                && style & WS_CHILD == 0
                && !is_dialog(hwnd)
            {
                // SAFETY: the window being created, on this thread.
                unsafe { install(hwnd) };
                unhook_creation();
            }
        }
        // SAFETY: passing the hook chain on, as every hook must.
        unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
    }

    /// Whether `hwnd` is of the dialog class (`#32770`: message boxes and
    /// common dialogs, which the creation hook must pass over).
    fn is_dialog(hwnd: HWND) -> bool {
        let mut name = [0u16; 16];
        // SAFETY: a window handle and a buffer with its length in chars.
        let n = unsafe { GetClassNameW(hwnd, name.as_mut_ptr(), name.len() as i32) };
        let dialog: Vec<u16> = "#32770".encode_utf16().collect();
        usize::try_from(n).is_ok_and(|n| name[..n.min(name.len())] == dialog[..])
    }

    /// The top resize band's height in physical px at `hwnd`'s DPI.
    ///
    /// # Safety
    /// `hwnd` must be a live window.
    pub unsafe fn frame_y(hwnd: HWND) -> i32 {
        // SAFETY: plain queries on the caller's window.
        unsafe {
            let dpi = GetDpiForWindow(hwnd);
            GetSystemMetricsForDpi(SM_CYFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi)
        }
    }

    /// The height the system caption and top frame take (physical px at
    /// `hwnd`'s DPI): what winit adds to a client size, and the custom
    /// frame gives back to the client.
    ///
    /// # Safety
    /// `hwnd` must be a live window.
    pub unsafe fn caption_extra(hwnd: HWND) -> i32 {
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        // SAFETY: plain queries on the caller's window and a local rect.
        unsafe {
            let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
            let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
            AdjustWindowRectExForDpi(&mut rect, style, 0, ex, GetDpiForWindow(hwnd));
        }
        -rect.top
    }

    /// Have Windows ask for the frame again after a mode change (the window
    /// keeps its outer size).
    ///
    /// # Safety
    /// `hwnd` must be a live window.
    pub unsafe fn frame_changed(hwnd: HWND) {
        // SAFETY: the caller's window.
        unsafe {
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED
                    | SWP_NOMOVE
                    | SWP_NOSIZE
                    | SWP_NOZORDER
                    | SWP_NOOWNERZORDER
                    | SWP_NOACTIVATE,
            )
        };
    }

    /// Install on a window that already has its system frame (the creation
    /// hook missed it), keeping its client size: the window loses the
    /// caption height the client gains.
    ///
    /// # Safety
    /// `hwnd` must be a live window of this thread.
    pub unsafe fn install_late(hwnd: HWND) {
        // SAFETY: the caller's window.
        unsafe {
            install(hwnd);
            if !custom() || IsZoomed(hwnd) != 0 || IsIconic(hwnd) != 0 {
                frame_changed(hwnd);
                return;
            }
            let extra = caption_extra(hwnd);
            let mut r = RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            GetWindowRect(hwnd, &mut r);
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                r.right - r.left,
                r.bottom - r.top - extra,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_NOACTIVATE,
            );
        }
    }

    /// The system command a caption button sends (maximize restores a
    /// maximized window).
    pub fn command(button: Button, maximized: bool) -> u32 {
        match button {
            Button::Minimize => SC_MINIMIZE,
            Button::Maximize if maximized => SC_RESTORE,
            Button::Maximize => SC_MAXIMIZE,
            Button::Close => SC_CLOSE,
        }
    }

    /// Send a caption button's system command to `hwnd` (posted: winit
    /// tracks minimizing there, and close takes the normal path).
    ///
    /// # Safety
    /// `hwnd` must be a window handle (a stale one fails harmlessly).
    pub unsafe fn press(hwnd: HWND, button: Button) {
        // SAFETY: plain calls on a window handle.
        unsafe {
            let sc = command(button, IsZoomed(hwnd) != 0);
            PostMessageW(hwnd, WM_SYSCOMMAND, sc as WPARAM, 0);
        }
    }

    /// The edges of `hwnd`'s monitor that hold an auto-hide taskbar.
    unsafe fn autohide_edges(hwnd: HWND) -> Edges {
        // SAFETY: plain queries; the structs are sized and zeroed.
        unsafe {
            let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(monitor, &mut info) == 0 {
                return Edges::default();
            }
            let on = |edge: u32| {
                let mut data: APPBARDATA = std::mem::zeroed();
                data.cbSize = std::mem::size_of::<APPBARDATA>() as u32;
                data.uEdge = edge;
                data.rc = info.rcMonitor;
                SHAppBarMessage(ABM_GETAUTOHIDEBAREX, &mut data) != 0
            };
            Edges {
                left: on(ABE_LEFT),
                top: on(ABE_TOP),
                right: on(ABE_RIGHT),
                bottom: on(ABE_BOTTOM),
            }
        }
    }

    fn client_point(hwnd: HWND, lparam: LPARAM) -> POINT {
        let (x, y) = super::point_from_lparam(lparam);
        let mut pt = POINT { x, y };
        // SAFETY: the window this procedure runs for, and a local point.
        unsafe { ScreenToClient(hwnd, &mut pt) };
        pt
    }

    /// The custom frame's window procedure (subclass [`SUBCLASS_ID`]): the
    /// caption area goes to the client, the hit test answers for the bar,
    /// the caption buttons work through the non-client mouse messages, and
    /// winit's client-size requests lose the caption height it adds. In
    /// the system mode everything passes through.
    unsafe extern "system" fn chrome_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        id: usize,
        _: usize,
    ) -> LRESULT {
        // SAFETY: the arguments this subclass procedure was called with.
        let def = |lparam: LPARAM| unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
        if msg == WM_NCDESTROY {
            // SAFETY: the subclass installed by `install`.
            unsafe { RemoveWindowSubclass(hwnd, Some(chrome_proc), id) };
            STATE.with(|s| {
                let mut s = s.borrow_mut();
                if s.installed == hwnd {
                    s.installed = std::ptr::null_mut();
                }
            });
            return def(lparam);
        }
        match msg {
            WM_ENTERSIZEMOVE => STATE.with(|s| s.borrow_mut().in_size_move = true),
            WM_EXITSIZEMOVE => STATE.with(|s| s.borrow_mut().in_size_move = false),
            WM_DPICHANGED => {
                // winit rebuilds the window from the client size through a
                // captioned frame: its SetWindowPos is corrected below.
                STATE.with(|s| s.borrow_mut().dpi_change = true);
                let r = def(lparam);
                STATE.with(|s| s.borrow_mut().dpi_change = false);
                return r;
            }
            _ => {}
        }
        if !custom() {
            return def(lparam);
        }
        // SAFETY: plain queries on the window this procedure runs for.
        let zoomed = || unsafe { IsZoomed(hwnd) != 0 };
        match msg {
            WM_NCCALCSIZE if wparam != 0 => {
                // SAFETY: with wParam TRUE, lParam is an NCCALCSIZE_PARAMS
                // for the call.
                let params = unsafe { &mut *(lparam as *mut NCCALCSIZE_PARAMS) };
                let top = params.rgrc[0].top;
                let r = def(lparam);
                if r != 0 {
                    return r;
                }
                let maximized = zoomed();
                // SAFETY: the window this procedure runs for.
                let fy = unsafe { frame_y(hwnd) };
                let rect = &mut params.rgrc[0];
                rect.top = super::nccalc_top(top, maximized, fy);
                if maximized {
                    // SAFETY: as above.
                    let [l, t, r, b] = super::autohide_insets(unsafe { autohide_edges(hwnd) });
                    rect.left += l;
                    rect.top += t;
                    rect.right -= r;
                    rect.bottom -= b;
                }
                0
            }
            WM_NCCALCSIZE => {
                // wParam FALSE: lParam is the window rect, which becomes the
                // client rect; the caption stays the app's here too.
                // SAFETY: with wParam FALSE, lParam is a RECT for the call.
                let rect = unsafe { &mut *(lparam as *mut RECT) };
                let top = rect.top;
                let r = def(lparam);
                // SAFETY: the window this procedure runs for.
                rect.top = super::nccalc_top(top, zoomed(), unsafe { frame_y(hwnd) });
                r
            }
            WM_NCHITTEST => {
                let def_code = def(lparam) as u32;
                let pt = client_point(hwnd, lparam);
                let maximized = zoomed();
                // SAFETY: the window this procedure runs for.
                let fy = unsafe { frame_y(hwnd) };
                if def_code != super::HTCLIENT {
                    return super::upgrade_border(def_code, pt.y, maximized, fy) as LRESULT;
                }
                let mut rc = RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                };
                // SAFETY: as above, and a local rect.
                unsafe { GetClientRect(hwnd, &mut rc) };
                STATE
                    .with(|s| super::hit_test(pt.x, pt.y, rc.right, maximized, fy, &s.borrow().bar))
                    as LRESULT
            }
            WM_NCMOUSEMOVE => {
                let button = Button::from_code(wparam as u32);
                let track = STATE.with(|s| {
                    let mut s = s.borrow_mut();
                    !std::mem::replace(&mut s.tracking, true)
                });
                if track {
                    let mut t = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE | TME_NONCLIENT,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    // SAFETY: a sized struct naming this window.
                    unsafe { TrackMouseEvent(&mut t) };
                }
                update(|s| s.hover = button);
                def(lparam)
            }
            WM_NCMOUSELEAVE => {
                update(|s| {
                    s.tracking = false;
                    s.hover = None;
                    s.pressed = None;
                });
                def(lparam)
            }
            WM_MOUSEMOVE => {
                update(|s| s.hover = None);
                def(lparam)
            }
            WM_LBUTTONUP => {
                update(|s| s.pressed = None);
                def(lparam)
            }
            WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK => match Button::from_code(wparam as u32) {
                // Never DefWindowProc: it would run the classic caption
                // button loop and paint old buttons over the bar.
                Some(b) => {
                    update(|s| s.pressed = Some(b));
                    0
                }
                None => def(lparam),
            },
            WM_NCLBUTTONUP => match Button::from_code(wparam as u32) {
                Some(b) => {
                    let mut was = None;
                    update(|s| was = s.pressed.take());
                    if was == Some(b) {
                        // SAFETY: the window this procedure runs for.
                        unsafe { press(hwnd, b) };
                    }
                    0
                }
                // Released over the bar, not a button: nothing stays pressed.
                None => {
                    update(|s| s.pressed = None);
                    def(lparam)
                }
            },
            WM_NCACTIVATE => {
                // -1: Windows must not repaint a caption over the bar.
                def(-1)
            }
            WM_WINDOWPOSCHANGING => {
                // SAFETY: lParam is a WINDOWPOS for the call.
                let pos = unsafe { &mut *(lparam as *mut WINDOWPOS) };
                let (in_size_move, dpi_change) =
                    STATE.with(|s| (s.borrow().in_size_move, s.borrow().dpi_change));
                // SAFETY: plain queries on this window.
                let minimized = unsafe { IsIconic(hwnd) != 0 };
                if super::corrects_size(pos.flags, in_size_move, dpi_change)
                    && !zoomed()
                    && !minimized
                {
                    // SAFETY: as above.
                    pos.cy -= unsafe { caption_extra(hwnd) };
                }
                def(lparam)
            }
            WM_GETMINMAXINFO => {
                let r = def(lparam);
                // SAFETY: lParam is a MINMAXINFO for the call.
                let mmi = unsafe { &mut *(lparam as *mut MINMAXINFO) };
                if mmi.ptMinTrackSize.y > 0 {
                    // SAFETY: plain query on this window.
                    let extra = unsafe { caption_extra(hwnd) };
                    mmi.ptMinTrackSize.y = (mmi.ptMinTrackSize.y - extra).max(1);
                }
                r
            }
            _ => def(lparam),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(scale: f32, height: f32) -> Bar {
        Bar {
            height: (height * scale).round() as i32,
            scale,
            passthrough: vec![PxRect::from_logical(340.0, 8.0, 600.0, 32.0, scale)],
            popup: false,
        }
    }

    #[test]
    fn the_bar_is_caption_buttons_and_search_are_not() {
        let b = bar(1.0, 48.0);
        let (w, fy) = (1280, 8);
        // The middle of the bar drags; its search box is client.
        assert_eq!(hit_test(200, 24, w, false, fy, &b), HTCAPTION);
        assert_eq!(hit_test(500, 24, w, false, fy, &b), HTCLIENT);
        // The three 46 px columns, and their boundary pixels.
        assert_eq!(hit_test(1279, 24, w, false, fy, &b), HTCLOSE);
        assert_eq!(hit_test(1234, 24, w, false, fy, &b), HTCLOSE);
        assert_eq!(hit_test(1233, 24, w, false, fy, &b), HTMAXBUTTON);
        assert_eq!(hit_test(1188, 24, w, false, fy, &b), HTMAXBUTTON);
        assert_eq!(hit_test(1187, 24, w, false, fy, &b), HTMINBUTTON);
        assert_eq!(hit_test(1142, 24, w, false, fy, &b), HTMINBUTTON);
        assert_eq!(hit_test(1141, 24, w, false, fy, &b), HTCAPTION);
        // The last row of the bar, and the row below it.
        assert_eq!(hit_test(200, 47, w, false, fy, &b), HTCAPTION);
        assert_eq!(hit_test(1250, 47, w, false, fy, &b), HTCLOSE);
        assert_eq!(hit_test(200, 48, w, false, fy, &b), HTCLIENT);
        assert_eq!(hit_test(1250, 48, w, false, fy, &b), HTCLIENT);
    }

    #[test]
    fn with_no_search_the_whole_bar_drags() {
        // No account yet: the bar pushes no passthrough rectangle, so where
        // the search box would be is caption (drag, double-click, Snap).
        let b = Bar {
            passthrough: Vec::new(),
            ..bar(1.25, 48.0)
        };
        let (w, fy) = (1600, 10);
        assert_eq!(hit_test(500 * 5 / 4, 30, w, false, fy, &b), HTCAPTION);
        assert_eq!(hit_test(800, 30, w, false, fy, &b), HTCAPTION);
        // The caption buttons stay where they are.
        assert_eq!(hit_test(1599, 30, w, false, fy, &b), HTCLOSE);
        // Below the bar is the client (the welcome).
        assert_eq!(hit_test(800, 60, w, false, fy, &b), HTCLIENT);
    }

    #[test]
    fn the_top_band_resizes_unless_maximized() {
        let b = bar(1.0, 48.0);
        let (w, fy) = (1280, 8);
        assert_eq!(hit_test(600, 0, w, false, fy, &b), HTTOP);
        assert_eq!(hit_test(600, 7, w, false, fy, &b), HTTOP);
        // Over the search box and the close button too, as natively.
        assert_eq!(hit_test(500, 3, w, false, fy, &b), HTTOP);
        assert_eq!(hit_test(1250, 3, w, false, fy, &b), HTTOP);
        assert_eq!(hit_test(0, 0, w, false, fy, &b), HTTOPLEFT);
        assert_eq!(hit_test(15, 5, w, false, fy, &b), HTTOPLEFT);
        assert_eq!(hit_test(16, 5, w, false, fy, &b), HTTOP);
        assert_eq!(hit_test(1279, 0, w, false, fy, &b), HTTOPRIGHT);
        assert_eq!(hit_test(1264, 5, w, false, fy, &b), HTTOPRIGHT);
        assert_eq!(hit_test(200, 8, w, false, fy, &b), HTCAPTION);
        // Maximized: no band; the close button reaches the screen corner.
        assert_eq!(hit_test(600, 0, w, true, fy, &b), HTCAPTION);
        assert_eq!(hit_test(1279, 0, w, true, fy, &b), HTCLOSE);
        assert_eq!(hit_test(0, 0, w, true, fy, &b), HTCAPTION);
    }

    #[test]
    fn side_borders_resize_the_corner_level_with_the_top_band() {
        assert_eq!(upgrade_border(HTLEFT, 3, false, 8), HTTOPLEFT);
        assert_eq!(upgrade_border(HTRIGHT, 7, false, 8), HTTOPRIGHT);
        assert_eq!(upgrade_border(HTLEFT, 8, false, 8), HTLEFT);
        assert_eq!(upgrade_border(HTRIGHT, 400, false, 8), HTRIGHT);
        assert_eq!(upgrade_border(HTLEFT, 3, true, 8), HTLEFT);
        assert_eq!(upgrade_border(HTBOTTOM, 3, false, 8), HTBOTTOM);
    }

    #[test]
    fn scaled_hit_tests_lose_and_double_no_pixel() {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let b = bar(scale, 48.0);
            let w = (1280.0 * scale) as i32;
            let fy = (8.0 * scale) as i32;
            let mut counts = [0; 4];
            for x in 0..w {
                match hit_test(x, fy + 1, w, false, fy, &b) {
                    HTMINBUTTON => counts[0] += 1,
                    HTMAXBUTTON => counts[1] += 1,
                    HTCLOSE => counts[2] += 1,
                    _ => counts[3] += 1,
                }
            }
            let px = |v: f32| (v * scale).round() as i32;
            assert_eq!(
                counts[0] + counts[1] + counts[2],
                px(3.0 * BUTTON_W),
                "{scale}"
            );
            assert_eq!(counts.iter().sum::<i32>(), w, "{scale}");
            for c in &counts[..3] {
                assert!(
                    (c - (BUTTON_W * scale) as i32).abs() <= 1,
                    "{scale}: {counts:?}"
                );
            }
            // The buttons are 48 tall, then the content starts.
            assert_eq!(hit_test(w - 1, px(BUTTON_H) - 1, w, false, fy, &b), HTCLOSE);
            assert_eq!(hit_test(w - 1, px(BUTTON_H), w, false, fy, &b), HTCLIENT);
            // The passthrough rect's edges.
            let r = b.passthrough[0];
            assert_eq!(hit_test(r.x, r.y + 1, w, false, fy, &b), HTCLIENT);
            assert_eq!(hit_test(r.x - 1, r.y + 1, w, false, fy, &b), HTCAPTION);
            assert_eq!(hit_test(r.x + r.w - 1, r.y + 1, w, false, fy, &b), HTCLIENT);
            assert_eq!(hit_test(r.x + r.w, r.y + 1, w, false, fy, &b), HTCAPTION);
        }
    }

    #[test]
    fn a_taller_bar_bounds_the_caption_by_its_pushed_height() {
        // Text scale 2.25: the search box is 48 tall, so the bar is 64.
        let b = bar(1.5, 64.0);
        let w = 1920;
        assert_eq!(b.height, 96);
        assert_eq!(hit_test(300, 80, w, false, 12, &b), HTCAPTION);
        assert_eq!(hit_test(300, 95, w, false, 12, &b), HTCAPTION);
        assert_eq!(hit_test(300, 96, w, false, 12, &b), HTCLIENT);
        // The caption buttons stay 48 tall at the top.
        assert_eq!(hit_test(w - 1, 71, w, false, 12, &b), HTCLOSE);
        assert_eq!(hit_test(w - 1, 72, w, false, 12, &b), HTCAPTION);
    }

    #[test]
    fn an_open_popup_makes_the_bar_client_but_the_buttons() {
        let mut b = bar(1.0, 48.0);
        b.popup = true;
        assert_eq!(hit_test(200, 24, 1280, false, 8, &b), HTCLIENT);
        assert_eq!(hit_test(1250, 24, 1280, false, 8, &b), HTCLOSE);
        assert_eq!(hit_test(600, 2, 1280, false, 8, &b), HTTOP);
    }

    #[test]
    fn lparam_points_are_signed() {
        let pack = |x: i32, y: i32| ((y as u16 as isize) << 16) | (x as u16 as isize);
        assert_eq!(point_from_lparam(pack(-1500, -20)), (-1500, -20));
        assert_eq!(point_from_lparam(pack(100, 200)), (100, 200));
        assert_eq!(point_from_lparam(pack(-1, 32767)), (-1, 32767));
        // High bits beyond the 32 used are ignored.
        assert_eq!(point_from_lparam(pack(5, -7) | (1 << 40)), (5, -7));
    }

    #[test]
    fn the_client_top_is_the_window_top_or_the_screen_top_when_maximized() {
        assert_eq!(nccalc_top(100, false, 8), 100);
        assert_eq!(nccalc_top(-8, true, 8), 0);
        assert_eq!(nccalc_top(-11, true, 11), 0);
        assert_eq!(autohide_insets(Edges::default()), [0, 0, 0, 0]);
        let bottom = Edges {
            bottom: true,
            ..Edges::default()
        };
        assert_eq!(autohide_insets(bottom), [0, 0, 0, 2]);
        let left_top = Edges {
            left: true,
            top: true,
            ..Edges::default()
        };
        assert_eq!(autohide_insets(left_top), [2, 2, 0, 0]);
    }

    #[test]
    fn only_winit_size_requests_lose_the_caption() {
        // winit's set_size: no move, no z-order, no activation.
        let winit = 0x2 | 0x4 | 0x200 | 0x10;
        assert!(corrects_size(winit, false, false));
        // Not during a user's resize, nor a move or a frame change.
        assert!(!corrects_size(winit, true, false));
        assert!(!corrects_size(0x4 | 0x10, false, false));
        assert!(!corrects_size(winit | 0x20, false, false));
        assert!(!corrects_size(winit | 0x1, false, false));
        // A DPI change moves and resizes: corrected even while dragged.
        assert!(corrects_size(0x4 | 0x10, true, true));
        assert!(!corrects_size(0x1 | 0x4, false, true));
    }

    #[test]
    fn saved_geometry_round_trips_through_winit() {
        // winit turns a client height into a window height through a
        // captioned frame (top = caption + frame, bottom = frame); the
        // correction takes the top back, the custom frame keeps only the
        // bottom border: the client comes back as saved, every time.
        let (top, bottom) = (39, 9);
        let mut saved = 820;
        for _ in 0..3 {
            let outer = saved + top + bottom;
            let corrected = outer - top;
            let client = corrected - bottom;
            assert_eq!(client, saved);
            saved = client;
        }
    }

    #[test]
    fn custom_on_windows_11_without_high_contrast() {
        assert!(custom_wanted(22631, false, false, false, None));
        assert!(!custom_wanted(19045, false, false, false, None));
        assert!(!custom_wanted(22631, true, false, false, None));
        assert!(!custom_wanted(22631, false, true, false, None));
        // Snapshots draw it on any build, but not under high contrast.
        assert!(custom_wanted(19045, false, false, true, None));
        assert!(!custom_wanted(22631, true, false, true, None));
        assert!(custom_wanted(19045, true, true, true, Some("custom")));
        assert!(!custom_wanted(22631, false, false, true, Some("system")));
        assert!(custom_wanted(22631, false, false, false, Some("other")));
    }

    #[test]
    fn buttons_map_to_codes_and_indexes() {
        for b in [Button::Minimize, Button::Maximize, Button::Close] {
            assert_eq!(Button::from_code(b.code()), Some(b));
            assert_eq!(Button::from_index(b.index()), Some(b));
        }
        assert_eq!(Button::from_code(HTCAPTION), None);
        assert_eq!(Button::from_index(0), None);
    }

    /// Win32 integration: a hidden overlapped window far off screen with
    /// the chrome subclass, driven by synchronous messages (no Slint event
    /// loop).
    #[cfg(windows)]
    mod win32 {
        use super::super::*;
        // The crate's own codes (u32), over the glob's.
        use super::super::{
            HTBOTTOM, HTCAPTION, HTCLIENT, HTCLOSE, HTLEFT, HTMAXBUTTON, HTMINBUTTON, HTTOP,
            HTTOPLEFT, HTTOPRIGHT,
        };
        use windows_sys::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
        use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
        use windows_sys::Win32::UI::HiDpi::AdjustWindowRectExForDpi;
        use windows_sys::Win32::UI::WindowsAndMessaging::*;

        fn wide(s: &str) -> Vec<u16> {
            s.encode_utf16().chain([0]).collect()
        }

        /// A window of a class with DefWindowProcW, its client `w` × `h`
        /// as a captioned frame would hold it, subclassed and in custom
        /// mode.
        fn window(w: i32, h: i32) -> HWND {
            let class = wide("KomorebiChromeTest");
            // SAFETY: plain Win32 calls with sized structs and NUL-terminated
            // strings; registering twice fails harmlessly.
            unsafe {
                let wc = WNDCLASSW {
                    lpfnWndProc: Some(DefWindowProcW),
                    lpszClassName: class.as_ptr(),
                    ..std::mem::zeroed()
                };
                RegisterClassW(&wc);
                let mut r = RECT {
                    left: 0,
                    top: 0,
                    right: w,
                    bottom: h,
                };
                AdjustWindowRectExForDpi(&mut r, WS_OVERLAPPEDWINDOW, 0, 0, 96);
                let hwnd = CreateWindowExW(
                    0,
                    class.as_ptr(),
                    wide("Komorebi").as_ptr(),
                    WS_OVERLAPPEDWINDOW,
                    -3000,
                    100,
                    r.right - r.left,
                    r.bottom - r.top,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                );
                assert!(!hwnd.is_null());
                set_custom(true);
                set_bar(Bar {
                    height: 48,
                    scale: 1.0,
                    passthrough: vec![PxRect {
                        x: 300,
                        y: 8,
                        w: 400,
                        h: 32,
                    }],
                    popup: false,
                });
                hwnd
            }
        }

        fn rects(hwnd: HWND) -> (RECT, RECT, POINT) {
            // SAFETY: a live window and local structs.
            unsafe {
                let mut wr = std::mem::zeroed();
                let mut cr = std::mem::zeroed();
                GetWindowRect(hwnd, &mut wr);
                GetClientRect(hwnd, &mut cr);
                let mut origin = POINT { x: 0, y: 0 };
                ClientToScreen(hwnd, &mut origin);
                (wr, cr, origin)
            }
        }

        fn hit(hwnd: HWND, x: i32, y: i32) -> u32 {
            let (_, _, origin) = rects(hwnd);
            let (sx, sy) = (origin.x + x, origin.y + y);
            let lp = ((sy as u16 as isize) << 16) | (sx as u16 as isize);
            // SAFETY: a live window of this thread; a synchronous message.
            unsafe { SendMessageW(hwnd, WM_NCHITTEST, 0, lp as LPARAM) as u32 }
        }

        fn destroy(hwnd: HWND) {
            // SAFETY: a window created on this thread.
            unsafe { DestroyWindow(hwnd) };
        }

        #[test]
        fn the_frame_gives_the_caption_to_the_client_at_the_same_size() {
            let hwnd = window(800, 600);
            // SAFETY: the window above.
            unsafe { install_late(hwnd) };
            let (wr, cr, origin) = rects(hwnd);
            assert_eq!(origin.y, wr.top, "the client starts at the window top");
            assert!(origin.x > wr.left, "the left border stays");
            assert!(wr.bottom > origin.y + cr.bottom, "the bottom border stays");
            assert!(wr.right > origin.x + cr.right, "the right border stays");
            // Installed on a framed window, it kept the client size.
            assert_eq!((cr.right, cr.bottom), (800, 600));
            destroy(hwnd);
        }

        #[test]
        fn winit_size_requests_keep_the_client_size() {
            let hwnd = window(800, 600);
            // SAFETY: the window above.
            unsafe { install_late(hwnd) };
            for want in [640, 700, 700] {
                // As winit's set_size does: the client height through a
                // captioned frame, without moving.
                let mut r = RECT {
                    left: 0,
                    top: 0,
                    right: 900,
                    bottom: want,
                };
                // SAFETY: plain calls on the window above.
                unsafe {
                    AdjustWindowRectExForDpi(&mut r, WS_OVERLAPPEDWINDOW, 0, 0, 96);
                    SetWindowPos(
                        hwnd,
                        std::ptr::null_mut(),
                        0,
                        0,
                        r.right - r.left,
                        r.bottom - r.top,
                        SWP_ASYNCWINDOWPOS
                            | SWP_NOZORDER
                            | SWP_NOOWNERZORDER
                            | SWP_NOMOVE
                            | SWP_NOACTIVATE,
                    );
                }
                let (_, cr, _) = rects(hwnd);
                assert_eq!((cr.right, cr.bottom), (900, want));
            }
            destroy(hwnd);
        }

        #[test]
        fn hit_tests_answer_for_the_bar_and_the_borders() {
            let hwnd = window(1000, 600);
            // SAFETY: the window above.
            unsafe { install_late(hwnd) };
            // SAFETY: as above.
            let fy = unsafe { frame_y(hwnd) };
            assert_eq!(hit(hwnd, 150, 24), HTCAPTION);
            assert_eq!(hit(hwnd, 400, 24), HTCLIENT, "the search box");
            assert_eq!(hit(hwnd, 999, 24), HTCLOSE);
            assert_eq!(hit(hwnd, 950, 24), HTMAXBUTTON);
            assert_eq!(hit(hwnd, 880, 24), HTMINBUTTON);
            assert_eq!(hit(hwnd, 500, 0), HTTOP);
            assert_eq!(hit(hwnd, 2, 1), HTTOPLEFT);
            assert_eq!(hit(hwnd, 500, 300), HTCLIENT);
            // The native borders outside the client, the left one upgraded
            // level with the top band.
            assert_eq!(hit(hwnd, -2, 300), HTLEFT);
            assert_eq!(hit(hwnd, -2, fy - 2), HTTOPLEFT);
            assert_eq!(hit(hwnd, 1001, fy - 2), HTTOPRIGHT);
            assert_eq!(hit(hwnd, 500, 601), HTBOTTOM);
            destroy(hwnd);
        }

        #[test]
        fn a_maximized_client_starts_a_frame_below_the_window_top() {
            let hwnd = window(800, 600);
            // SAFETY: the window above; WS_MAXIMIZE is what IsZoomed reads,
            // so the hidden window counts as maximized without showing.
            unsafe {
                install(hwnd);
                let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
                SetWindowLongW(hwnd, GWL_STYLE, (style | WS_MAXIMIZE) as i32);
                assert!(IsZoomed(hwnd) != 0);
                let fy = frame_y(hwnd);
                let mut params: NCCALCSIZE_PARAMS = std::mem::zeroed();
                params.rgrc[0] = RECT {
                    left: -8,
                    top: -8,
                    right: 1928,
                    bottom: 1048,
                };
                SendMessageW(hwnd, WM_NCCALCSIZE, 1, &raw mut params as LPARAM);
                let top = params.rgrc[0].top;
                // An auto-hide taskbar on the top edge pulls it 2 px more.
                assert!(
                    top == -8 + fy || top == -8 + fy + 2,
                    "top {top}, frame {fy}"
                );
                // Not maximized: the window top.
                SetWindowLongW(hwnd, GWL_STYLE, style as i32);
                params.rgrc[0] = RECT {
                    left: 100,
                    top: 100,
                    right: 900,
                    bottom: 700,
                };
                SendMessageW(hwnd, WM_NCCALCSIZE, 1, &raw mut params as LPARAM);
                assert_eq!(params.rgrc[0].top, 100);
                // wParam FALSE (a bare rect): the window top too.
                let mut rect = RECT {
                    left: 100,
                    top: 100,
                    right: 900,
                    bottom: 700,
                };
                SendMessageW(hwnd, WM_NCCALCSIZE, 0, &raw mut rect as LPARAM);
                assert_eq!(rect.top, 100);
                assert!(rect.bottom < 700, "the bottom border stays");
            }
            destroy(hwnd);
        }

        /// The WM_SYSCOMMANDs posted to `hwnd` so far (taken off the queue).
        fn posted_commands(hwnd: HWND) -> Vec<u32> {
            let mut out = Vec::new();
            // SAFETY: a live window of this thread and a local message.
            unsafe {
                let mut msg: MSG = std::mem::zeroed();
                while PeekMessageW(&mut msg, hwnd, WM_SYSCOMMAND, WM_SYSCOMMAND, PM_REMOVE) != 0 {
                    out.push(msg.wParam as u32);
                }
            }
            out
        }

        #[test]
        fn caption_buttons_act_on_release_over_the_same_button() {
            let hwnd = window(800, 600);
            // SAFETY: the window above; synchronous messages.
            unsafe {
                install_late(hwnd);
                let send = |msg: u32, code: u32| SendMessageW(hwnd, msg, code as usize, 0);
                assert_eq!(send(WM_NCLBUTTONDOWN, HTMAXBUTTON), 0);
                assert_eq!(caption_state().1, Some(Button::Maximize));
                assert_eq!(send(WM_NCLBUTTONUP, HTMAXBUTTON), 0);
                assert_eq!(posted_commands(hwnd), vec![SC_MAXIMIZE]);
                assert_eq!(caption_state().1, None);
                // Down on minimize, up on close: nothing.
                send(WM_NCLBUTTONDOWN, HTMINBUTTON);
                send(WM_NCLBUTTONUP, HTCLOSE);
                assert!(posted_commands(hwnd).is_empty());
                // Down on close, released over the drag area: nothing
                // stays pressed and nothing is sent.
                send(WM_NCLBUTTONDOWN, HTCLOSE);
                send(WM_NCLBUTTONUP, HTCAPTION);
                assert_eq!(caption_state(), (None, None));
                assert!(posted_commands(hwnd).is_empty());
                // A double click on a button is a press too.
                send(WM_NCLBUTTONDBLCLK, HTCLOSE);
                send(WM_NCLBUTTONUP, HTCLOSE);
                assert_eq!(posted_commands(hwnd), vec![SC_CLOSE]);
                // Hover follows the pointer and leaves with it.
                send(WM_NCMOUSEMOVE, HTMINBUTTON);
                assert_eq!(caption_state().0, Some(Button::Minimize));
                send(WM_NCMOUSELEAVE, 0);
                assert_eq!(caption_state(), (None, None));
            }
            destroy(hwnd);
        }

        #[test]
        fn the_system_mode_passes_everything_through() {
            let hwnd = window(800, 600);
            // SAFETY: the window above.
            unsafe {
                install(hwnd);
                set_custom(false);
                frame_changed(hwnd);
                let (wr, _, origin) = rects(hwnd);
                assert!(origin.y > wr.top + 20, "a system caption above the client");
                assert_eq!(SendMessageW(hwnd, WM_NCLBUTTONUP, HTCLOSE as usize, 0), 0);
            }
            assert!(posted_commands(hwnd).is_empty());
            destroy(hwnd);
        }
    }
}
