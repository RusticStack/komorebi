//! The background picture and the window material at run time (Settings >
//! Appearance > Background; the settings are `theme::backdrop`).
//!
//! The picture is chosen once, decoded and scaled down to the largest
//! monitor off the UI thread, and kept as `background.png` in the app's data
//! folder ([`dir`]: beside the cache file, never inside it). From it, also
//! off the UI thread, two pictures are made ([`bake`]), in a fixed order:
//!
//! 1. decode the kept picture;
//! 2. fit or crop it to the window (with a style or a vignette; otherwise
//!    the GPU places it when drawing, and the result is the same);
//! 3. blur it (Blur image);
//! 4. redraw it in its style (pixel art or ASCII, `style`), so the cells
//!    stay crisp over a blurred picture;
//! 5. dim it;
//! 6. fade it toward the window's edges (vignette, in the style's terms);
//! 7. that is the stored picture; for Blur behind panes a quarter-size copy
//!    of it is blurred again (the pane copy).
//!
//! Those two are all that is held ([`Backdrop`]); the decoded original is
//! dropped once they are made, and a new pair replaces the old one in every
//! window's atlas. Moving a slider makes the pair again after it rests
//! ([`DEBOUNCE`]); so does resizing the window or changing the theme when
//! the picture is made at the window's size (a style or a vignette).
//!
//! Drawing: the window root draws the picture and the veil over it
//! ([`layer`]); a pane draws its copy clipped to its own rounded rectangle
//! under its translucent fill ([`pane_layer`]). With nothing chosen there is
//! no global, no element and no extra work.

pub mod pixels;
pub mod style;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{
    AnyElement, App, Bounds, Corners, Global, Hsla, IntoElement as _, Pixels, Point, RenderImage,
    Size, Styled as _, Window, WindowBackgroundAppearance, canvas, fill, size,
};

use crate::theme::ActiveTokens as _;
use crate::theme::backdrop::{
    AsciiInk, Background, DEBOUNCE, Fit, MICA_BUILD, Material, SIGMA_PER_PX, Style, ToneGrid,
    Tones, place, safe_opacity,
};
use crate::theme::layout;
use crate::theme::presets::Rgb;
use pixels::Refusal;

/// The picture's file in [`dir`].
pub const FILE: &str = "background.png";

/// The app's data folder for the appearance: beside the cache file, as
/// the drafts' folder is (`cache.appearance`).
pub fn dir(db_file: &Path) -> PathBuf {
    db_file.with_extension("appearance")
}

/// The kept picture.
pub fn picture(db_file: &Path) -> PathBuf {
    dir(db_file).join(FILE)
}

/// The largest monitor in physical px (what a picture is scaled down to),
/// at least 1920 x 1080 when no monitor answers.
pub fn monitor_cap(window: Option<&Window>, cx: &App) -> (u32, u32) {
    let scale = window.map_or(1., Window::scale_factor);
    let fallback = pixels::FALLBACK_CAP;
    cx.displays()
        .iter()
        .map(|d| {
            let b = d.bounds();
            (
                (f32::from(b.size.width) * scale).round() as u32,
                (f32::from(b.size.height) * scale).round() as u32,
            )
        })
        .fold(None, |acc: Option<(u32, u32)>, (w, h)| {
            Some(acc.map_or((w, h), |(aw, ah)| (aw.max(w), ah.max(h))))
        })
        .filter(|(w, h)| *w > 0 && *h > 0)
        .unwrap_or(fallback)
}

/// What a pair of pictures is made from: the settings, and what they
/// need from the window (its scale, its size when the picture is made at
/// it, and the theme's colours for ASCII).
#[derive(Debug, Clone, PartialEq)]
pub struct Recipe {
    pub source: PathBuf,
    pub cap: (u32, u32),
    /// The window's scale (blur and cells are in logical px).
    pub scale: f32,
    pub fit: Fit,
    /// The window's size in physical px when the picture is made at it
    /// (a style or a vignette); None: placed by the GPU.
    pub frame: Option<(u32, u32)>,
    pub blur: u8,
    pub dim: u8,
    pub pane_blur: u8,
    pub style: Style,
    pub cell: u8,
    pub colours: u8,
    pub dither: bool,
    /// ASCII: one ink for every character (None: the picture's colours).
    pub ink: Option<[u8; 3]>,
    /// ASCII: the characters' intensity, 0 to 100.
    pub intensity: u8,
    /// A style on a light theme: its canvas, which the style's cells are
    /// drawn toward (and ASCII's dense characters then mean dark).
    pub ground: Option<[u8; 3]>,
    pub vignette: u8,
    pub vignette_size: u8,
}

/// Whether `bg`'s picture is made at the window's size.
pub fn framed(bg: &Background) -> bool {
    bg.style != Style::None || bg.vignette > 0
}

fn bytes(c: gpui::Hsla) -> [u8; 3] {
    let c = c.to_rgb(); // token-exempt: a theme token's own value, read as bytes for the ASCII ink
    [c.r, c.g, c.b].map(|v| (v * 255.).round().clamp(0., 255.) as u8)
}

impl Recipe {
    /// The recipe for `bg` in `window`.
    pub fn new(source: PathBuf, bg: &Background, window: &Window, cx: &App) -> Self {
        let scale = window.scale_factor();
        let frame = framed(bg).then(|| {
            let v = window.viewport_size();
            (
                (f32::from(v.width) * scale).round().max(1.) as u32,
                (f32::from(v.height) * scale).round().max(1.) as u32,
            )
        });
        let ascii = bg.style == Style::Ascii;
        let c = cx.tokens().color;
        let ink = match bg.ascii_ink {
            _ if !ascii => None,
            AsciiInk::Picture => None,
            AsciiInk::Text => Some(bytes(c.ink)),
            AsciiInk::Accent => Some(bytes(c.accent)),
        };
        Recipe {
            source,
            cap: monitor_cap(Some(window), cx),
            scale,
            fit: bg.fit,
            frame,
            blur: bg.blur,
            dim: bg.dim,
            pane_blur: if bg.picture { bg.pane_blur } else { 0 },
            style: bg.style,
            cell: bg.cell,
            colours: bg.colours,
            dither: bg.dither,
            ink,
            intensity: if ascii { bg.intensity } else { 0 },
            ground: (bg.style != Style::None && c.canvas.l > 0.5).then(|| bytes(c.canvas)),
            vignette: bg.vignette,
            vignette_size: bg.vignette_size,
        }
    }

    /// How the stored picture is placed: at the window's size it covers it.
    fn placement(&self) -> Fit {
        if self.frame.is_some() {
            Fit::Fill
        } else {
            self.fit
        }
    }
}

/// The pictures made from a [`Recipe`].
pub struct Baked {
    pub sharp: Arc<RenderImage>,
    pub soft: Option<Arc<RenderImage>>,
    /// The picture's size in physical px.
    pub size: (u32, u32),
    /// What is behind a pane, for the legibility warning.
    pub tones: Tones,
    /// The stored picture's tones over a grid (what the sidebar shows), and
    /// the pane copy's (what a blurred pane shows), for the floor.
    pub grid: ToneGrid,
    pub soft_grid: Option<ToneGrid>,
    /// A style on a light theme: panes and the sidebar keep muted text
    /// legible over the part of the picture behind them. (Dark themes keep
    /// the chosen opacity, and the page warns as without a style.)
    pub styled: bool,
}

/// The tone grid's columns and rows.
const GRID: (usize, usize) = (16, 9);

/// The stored picture, the pane copy, and the size it is drawn at.
pub type Made = (image::RgbaImage, Option<image::RgbaImage>, (u32, u32));

/// The stored picture and the pane copy as pixels (the pipeline, in the
/// order the module's documentation gives).
pub fn bake_pixels(r: &Recipe) -> Result<Made, Refusal> {
    // 1. Decode.
    let mut img = pixels::load(&r.source, r.cap)?;
    style::premultiply(&mut img);
    // 2. Fit or crop to the window.
    if let Some(size) = r.frame {
        img = style::frame(&img, r.fit, size);
    }
    // Drawn at this size whatever is kept.
    let size = img.dimensions();
    // 3. Blur. Without a style a blurred picture has no detail finer than
    // its blur: kept smaller (up to a quarter a side) it costs a sixteenth
    // of the memory and of the blur, and the GPU's scaling up is invisible
    // under the blur. A style needs every device pixel for its cells.
    let scale = r.scale.max(1.);
    let sigma = f32::from(r.blur) * SIGMA_PER_PX * scale;
    let shrink = if r.style == Style::None {
        pixels::shrink_for(sigma)
    } else {
        1
    };
    if shrink > 1 {
        img = pixels::shrink(&img, shrink);
    }
    pixels::blur(&mut img, sigma / shrink as f32);
    // 4. Style, with the vignette in its own terms.
    let fade = (
        f32::from(r.vignette) / 100.,
        f32::from(r.vignette_size) / 100.,
    );
    let cell = (f32::from(r.cell) * scale).round().max(1.) as u32;
    img = match r.style {
        Style::None => img,
        Style::PixelArt => {
            style::pixel_art(&img, cell, usize::from(r.colours), r.dither, fade, r.ground)
        }
        Style::Ascii => style::ascii(
            &img,
            cell,
            r.ink,
            f32::from(r.intensity) / 100.,
            fade,
            r.ground,
        ),
    };
    // 5. Dim.
    pixels::dim(&mut img, f32::from(r.dim) / 100.);
    // 6. The smooth vignette (a style faded already).
    if r.style == Style::None {
        style::vignette(&mut img, fade.0, fade.1);
    }
    // 7. The pane copy, from the stored picture.
    let soft = (r.pane_blur > 0)
        .then(|| pixels::soften(&img, f32::from(r.pane_blur) * SIGMA_PER_PX * scale));
    Ok((img, soft, size))
}

/// Makes the pictures (off the UI thread).
pub fn bake(r: &Recipe) -> Result<Baked, Refusal> {
    let (img, soft, size) = bake_pixels(r)?;
    // From the stored picture: what the sidebar shows, and the extremes a
    // pane's blur only softens.
    let tones = pixels::tones(&img);
    let grid = pixels::tone_grid(&img, GRID.0, GRID.1);
    let soft_grid = soft.as_ref().map(|s| pixels::tone_grid(s, GRID.0, GRID.1));
    let image = |img| {
        let (w, h, bytes) = pixels::to_bgra(img);
        RenderImage::from_shared_bgra(w, h, bytes).map(Arc::new)
    };
    let sharp = image(img).ok_or(Refusal::Unreadable)?;
    let soft = match soft {
        Some(s) => Some(image(s).ok_or(Refusal::Unreadable)?),
        None => None,
    };
    Ok(Baked {
        sharp,
        soft,
        size,
        tones,
        grid,
        soft_grid,
        styled: r.style != Style::None && r.ground.is_some(),
    })
}

/// The pictures in use (absent: nothing is drawn).
#[derive(Default)]
pub struct Backdrop {
    baked: Option<Baked>,
    fit: Fit,
    /// The newest request; an older one finishing late is dropped.
    generation: u64,
    recipe: Option<Recipe>,
    /// The kept picture and the settings, so a picture made at the
    /// window's size can follow the window and the theme.
    wanted: Option<(PathBuf, Background)>,
    /// How many times pictures were made (tests: a slider makes them once).
    pub bakes: u64,
}

impl Global for Backdrop {}

impl Backdrop {
    /// The pictures' tones, once made.
    pub fn tones(cx: &App) -> Option<Tones> {
        cx.try_global::<Backdrop>()
            .and_then(|b| b.baked.as_ref().map(|b| b.tones))
    }

    /// The held picture's size in physical px.
    pub fn held_size(cx: &App) -> Option<(u32, u32)> {
        cx.try_global::<Backdrop>()
            .and_then(|b| b.baked.as_ref().map(|b| b.size))
    }

    /// How many times pictures were made.
    pub fn bakes(cx: &App) -> u64 {
        cx.try_global::<Backdrop>().map_or(0, |b| b.bakes)
    }

    /// Whether a picture is held (tests: the defaults hold none).
    pub fn holds_picture(cx: &App) -> bool {
        cx.try_global::<Backdrop>()
            .is_some_and(|b| b.baked.is_some())
    }
}

/// Puts `baked` in use, placed for `fit`: the old pictures leave every
/// window's atlas.
fn install(baked: Option<Baked>, fit: Fit, cx: &mut App) {
    let old = cx
        .try_global::<Backdrop>()
        .is_some()
        .then(|| cx.global_mut::<Backdrop>().baked.take())
        .flatten();
    if let Some(old) = old {
        cx.drop_image(old.sharp, None);
        if let Some(soft) = old.soft {
            cx.drop_image(soft, None);
        }
    }
    if baked.is_some() || cx.has_global::<Backdrop>() {
        let state = cx.default_global::<Backdrop>();
        state.baked = baked;
        state.fit = fit;
    }
    cx.refresh_windows();
}

/// The pictures for `bg` with the kept picture `source` (none: no
/// picture), made off the UI thread; after [`DEBOUNCE`] when `debounce` (a
/// slider moving), and only if no newer request came meanwhile. The same
/// recipe again does nothing.
pub fn want(
    source: Option<PathBuf>,
    bg: &Background,
    debounce: bool,
    window: &Window,
    cx: &mut App,
) {
    let source = source.filter(|_| bg.picture);
    let recipe = source.clone().map(|s| Recipe::new(s, bg, window, cx));
    if recipe.is_none() && !cx.has_global::<Backdrop>() {
        return;
    }
    cx.default_global::<Backdrop>().wanted = source.map(|s| (s, bg.clone()));
    request(recipe, debounce, cx);
}

fn request(recipe: Option<Recipe>, debounce: bool, cx: &mut App) {
    let state = cx.default_global::<Backdrop>();
    let settled = match &recipe {
        None => state.baked.is_none(),
        Some(_) => state.baked.is_some(),
    };
    if state.recipe == recipe && settled {
        cx.refresh_windows();
        return;
    }
    state.generation += 1;
    state.recipe = recipe.clone();
    let generation = state.generation;
    let Some(recipe) = recipe else {
        install(None, Fit::Fill, cx);
        return;
    };
    let fit = recipe.placement();
    let current = move |cx: &App| {
        cx.try_global::<Backdrop>()
            .is_some_and(|b| b.generation == generation)
    };
    cx.spawn(async move |cx| {
        if debounce {
            cx.background_executor().timer(DEBOUNCE).await; // timer-exempt: a slider's rest; tests advance the executor clock
            if !cx.update(|cx| current(cx)) {
                return;
            }
        }
        let baked = cx
            .background_executor()
            .spawn(async move { bake(&recipe) })
            .await;
        cx.update(|cx| {
            if !current(cx) {
                return;
            }
            cx.global_mut::<Backdrop>().bakes += 1;
            match baked {
                Ok(b) => install(Some(b), fit, cx),
                Err(e) => {
                    eprintln!("komorebi-gpui: background picture not made: {e:?}");
                    install(None, fit, cx);
                }
            }
        });
    })
    .detach();
}

/// A picture made at the window's size follows it: when the window's size
/// or the theme no longer match what it was made for, it is made again
/// once they rest. Called as the picture is drawn; nothing is made while
/// they match.
fn follow(window: &Window, cx: &mut App) {
    let Some(state) = cx.try_global::<Backdrop>() else {
        return;
    };
    let Some((source, bg)) = state.wanted.as_ref().filter(|(_, bg)| framed(bg)) else {
        return;
    };
    let next = Recipe::new(source.clone(), bg, window, cx);
    if state.recipe.as_ref() != Some(&next) {
        request(Some(next), true, cx);
    }
}

/// The kept picture changed on disk: the next request makes the pictures
/// again even with the same recipe (the pictures in use stay until then).
pub fn invalidate(cx: &mut App) {
    if let Some(state) = cx
        .try_global::<Backdrop>()
        .is_some()
        .then(|| cx.global_mut::<Backdrop>())
    {
        state.recipe = None;
    }
}

/// Makes and installs the pictures now, on this thread (a snapshot's
/// first frame must have them).
pub fn install_now(
    source: PathBuf,
    bg: &Background,
    window: &Window,
    cx: &mut App,
) -> Result<(), Refusal> {
    let recipe = Recipe::new(source.clone(), bg, window, cx);
    let baked = bake(&recipe)?;
    let fit = recipe.placement();
    let state = cx.default_global::<Backdrop>();
    state.generation += 1;
    state.bakes += 1;
    state.recipe = Some(recipe);
    state.wanted = Some((source, bg.clone()));
    install(Some(baked), fit, cx);
    Ok(())
}

/// The window's own background for `bg`: opaque unless a material is
/// chosen (Mica only where Windows has it).
pub fn window_background(bg: &Background) -> WindowBackgroundAppearance {
    match bg.material {
        Material::None => WindowBackgroundAppearance::Opaque,
        Material::Mica if mica_available() => WindowBackgroundAppearance::MicaBackdrop,
        Material::Mica => WindowBackgroundAppearance::Opaque,
        Material::Acrylic => WindowBackgroundAppearance::Blurred,
    }
}

/// Whether this Windows has Mica as a system backdrop.
pub fn mica_available() -> bool {
    komorebi::theme::windows_build() >= MICA_BUILD
}

/// Every window's material follows `bg` (on the next turn: a window being
/// updated is not reachable from inside its own update).
pub fn sync_material(bg: &Background, cx: &mut App) {
    let wanted = window_background(bg);
    cx.spawn(async move |cx| {
        cx.update(|cx| {
            for handle in cx.windows() {
                let _ = handle.update(cx, |_, window, _| {
                    window.set_background_appearance(wanted);
                });
            }
        });
    })
    .detach();
}

/// Draws `image` (of `px_size` physical px) placed for `fit` in `area`,
/// only where it meets `clip`, with `radii` when the clip is whole.
fn paint_picture(
    window: &mut Window,
    area: Bounds<Pixels>,
    clip: Bounds<Pixels>,
    image: &Arc<RenderImage>,
    (w, h): (u32, u32),
    fit: Fit,
    radii: Corners<Pixels>,
) {
    let scale = window.scale_factor().max(f32::EPSILON);
    let logical = size(w as f32 / scale, h as f32 / scale);
    for rect in place(fit, logical, area) {
        let seen = rect.intersect(&clip);
        if seen.size.width <= Pixels::ZERO || seen.size.height <= Pixels::ZERO {
            continue;
        }
        let corners = if seen == clip {
            radii
        } else {
            Corners::default()
        };
        let _ = window.paint_image(seen, rect, corners, image.clone(), 0, false);
    }
}

/// The window's whole client area.
fn viewport(window: &Window) -> Bounds<Pixels> {
    Bounds {
        origin: Point::default(),
        size: window.viewport_size(),
    }
}

/// The root's layer: the picture, then the veil over it. None (no element
/// at all) when no picture is held.
pub fn layer(cx: &App) -> Option<AnyElement> {
    let state = cx.try_global::<Backdrop>()?;
    let baked = state.baked.as_ref()?;
    let (image, dims, fit) = (baked.sharp.clone(), baked.size, state.fit);
    let c = cx.tokens().color;
    let veil = c.veil;
    // Under a style the sidebar's column and the title bar keep muted text
    // legible over what is behind them: each gets as much more veil as its
    // own part of the picture needs, and the rest of the picture keeps the
    // chosen veil.
    let floor = baked.styled.then(|| {
        let grid = baked.grid.clone();
        let (canvas, muted) = (Rgb::from_colour(c.canvas), Rgb::from_colour(c.ink_3));
        move |view: Size<Pixels>| {
            let (w, h) = (
                f32::from(view.width).max(1.),
                f32::from(view.height).max(1.),
            );
            [
                ((0., 0.), (layout::NAV_W / w, 1.)),
                ((0., 0.), (1., layout::TITLEBAR_H / h)),
            ]
            .into_iter()
            .filter_map(|(a, b)| {
                let need = f32::from(safe_opacity(canvas, muted, &grid.under(a, b)?)) / 100.;
                let region = Bounds::from_corners(
                    Point::new(view.width * a.0, view.height * a.1),
                    Point::new(view.width * b.0, view.height * b.1),
                );
                Some((region, need))
            })
            .collect::<Vec<_>>()
        }
    });
    Some(
        canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                paint_picture(
                    window,
                    viewport(window),
                    bounds,
                    &image,
                    dims,
                    fit,
                    Corners::default(),
                );
                window.paint_quad(fill(bounds, veil));
                if let Some(needs) = &floor {
                    for (region, need) in needs(window.viewport_size()) {
                        // Over the veil already painted: together they
                        // cover `need`.
                        let more = (need - veil.a) / (1. - veil.a).max(f32::EPSILON);
                        if more > 0. {
                            let region = Bounds {
                                origin: bounds.origin + region.origin,
                                size: region.size,
                            };
                            window.paint_quad(fill(region, Hsla { a: more, ..veil }));
                        }
                    }
                }
                follow(window, cx);
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .into_any_element(),
    )
}

/// A pane's layer under its content: its part of the softer copy, then the
/// pane's translucent fill, both inside its rounded rectangle. None (the
/// pane fills itself as usual) unless Blur behind panes is above 0 or a
/// style is on (then the fill keeps muted text legible over what is
/// behind this pane).
pub fn pane_layer(radius: Pixels, cx: &App) -> Option<AnyElement> {
    let state = cx.try_global::<Backdrop>()?;
    let baked = state.baked.as_ref()?;
    if baked.soft.is_none() && !baked.styled {
        return None;
    }
    let soft = baked.soft.clone();
    let (w, h) = baked.size;
    let fit = state.fit;
    let c = cx.tokens().color;
    let (pane, ground) = (c.pane, c.backdrop);
    let floor = baked.styled.then(|| {
        (
            baked
                .soft_grid
                .clone()
                .unwrap_or_else(|| baked.grid.clone()),
            Rgb::from_colour(c.paper),
            Rgb::from_colour(c.ink_3),
        )
    });
    Some(
        canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let radii = Corners::all(radius);
                // The soft copy stands for the whole picture: placed as the
                // picture is, at the picture's size.
                if let Some(soft) = &soft {
                    // The window's own background first: where the copy is
                    // clear (a style's blanks, the vignette, Fit's bands)
                    // the sharp picture under the pane must not show.
                    window.paint_quad(fill(bounds, ground).corner_radii(radii));
                    paint_picture(window, viewport(window), bounds, soft, (w, h), fit, radii);
                }
                let mut pane = pane;
                if let Some((grid, paper, muted)) = &floor {
                    let v = window.viewport_size();
                    let (vw, vh) = (f32::from(v.width).max(1.), f32::from(v.height).max(1.));
                    let (x, y) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
                    let (bw, bh) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
                    if let Some(g) = grid.under((x / vw, y / vh), ((x + bw) / vw, (y + bh) / vh)) {
                        pane.a = pane
                            .a
                            .max(f32::from(safe_opacity(*paper, *muted, &g)) / 100.);
                    }
                }
                window.paint_quad(fill(bounds, pane).corner_radii(radii));
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recipe(source: PathBuf) -> Recipe {
        Recipe {
            source,
            cap: (4000, 4000),
            scale: 1.,
            fit: Fit::Fill,
            frame: Some((320, 200)),
            blur: 20,
            dim: 40,
            pane_blur: 20,
            style: Style::PixelArt,
            cell: 8,
            colours: 16,
            dither: true,
            ink: None,
            intensity: 50,
            ground: None,
            vignette: 100,
            vignette_size: 60,
        }
    }

    /// decode -> fit to the window -> blur -> style -> dim -> vignette ->
    /// stored copy -> pane copy from it.
    #[test]
    fn the_pipeline_runs_in_its_order() {
        let dir = std::env::temp_dir().join(format!("komorebi-bake-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("ramp.png");
        image::RgbaImage::from_fn(640, 200, |x, y| {
            image::Rgba([(x * 255 / 639) as u8, (y * 255 / 199) as u8, 90, 255])
        })
        .save(&source)
        .unwrap();
        let (img, soft, size) = bake_pixels(&recipe(source.clone())).unwrap();
        // Fitted to the window, at its size.
        assert_eq!((size, img.dimensions()), ((320, 200), (320, 200)));
        // The style came after the blur: every cell is still one colour.
        for (x, y, p) in img.enumerate_pixels() {
            assert_eq!(p, img.get_pixel(x / 8 * 8, y / 8 * 8), "{x},{y}");
        }
        // The dim came after the style: no cell is brighter than 60 %.
        assert!(img.pixels().all(|p| p.0[..3].iter().all(|&c| c <= 154)));
        // The vignette came last, in whole cells: clear corners, no partial
        // cells, a full middle.
        assert_eq!(img.get_pixel(0, 0).0[3], 0);
        assert!(img.pixels().all(|p| p.0[3] == 0 || p.0[3] == 255));
        assert_eq!(img.get_pixel(160, 100).0[3], 255);
        // The pane copy is made from the stored picture: its corner is
        // clear too, and it is a quarter the size.
        let soft = soft.unwrap();
        assert_eq!(soft.dimensions(), (80, 50));
        assert!(soft.get_pixel(0, 0).0[3] < 64);
        // No style and no vignette: placed by the GPU, kept at its own
        // size (or smaller under a strong blur).
        let plain = Recipe {
            frame: None,
            style: Style::None,
            vignette: 0,
            blur: 0,
            pane_blur: 0,
            ..recipe(source)
        };
        let (img, soft, size) = bake_pixels(&plain).unwrap();
        assert_eq!((size, img.dimensions()), ((640, 200), (640, 200)));
        assert!(soft.is_none());
        std::fs::remove_dir_all(&dir).ok();
    }
}
