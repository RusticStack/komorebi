//! The cascade for the email subset: UA defaults, presentational
//! attributes and `<font>`, `<style>` rules (css.rs) and inline styles.
//! Also the hidden-preheader heuristics and the mapping of common font
//! stacks to fonts installed on Windows. In the dark theme, mail without
//! dark rules of its own has its resolved colours dimmed (see [`dim`]).

use super::Deadline;
use super::css::{self, Ctx, Decl, Sheet};
use super::dom::{Dom, Id, Kind};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

pub type Color = [u8; 4];

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Len {
    Auto,
    Px(f32),
    Pct(f32),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Display {
    None,
    Inline,
    Block,
    InlineBlock,
    ListItem,
    Table,
    RowGroup,
    Row,
    Cell,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Align {
    Left,
    Center,
    Right,
    Justify,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum VAlign {
    Top,
    Middle,
    Bottom,
    Baseline,
}

/// `list-style-type`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Marker {
    Disc,
    Circle,
    Square,
    Decimal,
    LowerAlpha,
    UpperAlpha,
    LowerRoman,
    UpperRoman,
}

impl Marker {
    fn parse(v: &str) -> Option<Marker> {
        Some(match v {
            "disc" => Marker::Disc,
            "circle" => Marker::Circle,
            "square" => Marker::Square,
            "decimal" | "decimal-leading-zero" => Marker::Decimal,
            "lower-alpha" | "lower-latin" => Marker::LowerAlpha,
            "upper-alpha" | "upper-latin" => Marker::UpperAlpha,
            "lower-roman" => Marker::LowerRoman,
            "upper-roman" => Marker::UpperRoman,
            _ => return None,
        })
    }

    /// The `type` attribute of a list or item: case tells `a` from `A`.
    fn from_attr(v: &str) -> Option<Marker> {
        match v.trim() {
            "1" => Some(Marker::Decimal),
            "a" => Some(Marker::LowerAlpha),
            "A" => Some(Marker::UpperAlpha),
            "i" => Some(Marker::LowerRoman),
            "I" => Some(Marker::UpperRoman),
            v => Marker::parse(&v.to_ascii_lowercase()),
        }
    }

    /// What the item numbered `n` shows.
    pub fn text(self, n: i64) -> String {
        let alpha = |base: u8| {
            let mut left = n;
            let mut out = Vec::new();
            while left > 0 {
                let d = left.saturating_sub(1).rem_euclid(26);
                out.push(char::from(base.saturating_add(d as u8)));
                left = left.saturating_sub(1) / 26;
            }
            out.iter().rev().collect::<String>()
        };
        let roman = || {
            const STEPS: [(i64, &str); 13] = [
                (1000, "m"),
                (900, "cm"),
                (500, "d"),
                (400, "cd"),
                (100, "c"),
                (90, "xc"),
                (50, "l"),
                (40, "xl"),
                (10, "x"),
                (9, "ix"),
                (5, "v"),
                (4, "iv"),
                (1, "i"),
            ];
            let mut left = n;
            let mut out = String::new();
            for (v, t) in STEPS {
                while left >= v {
                    out.push_str(t);
                    left = left.saturating_sub(v);
                }
            }
            out
        };
        // As a browser falls back: what has no form of its own is a number.
        let num = |s: String| {
            if s.is_empty() || n >= 4000 {
                format!("{n}.")
            } else {
                format!("{s}.")
            }
        };
        match self {
            Marker::Disc => "\u{2022}".into(),
            Marker::Circle => "\u{25E6}".into(),
            Marker::Square => "\u{25AA}".into(),
            Marker::Decimal => format!("{n}."),
            Marker::LowerAlpha => num(alpha(b'a')),
            Marker::UpperAlpha => num(alpha(b'A')),
            Marker::LowerRoman => num(roman()),
            Marker::UpperRoman => num(roman().to_ascii_uppercase()),
        }
    }
}

/// `border-style`; `groove`, `ridge`, `inset` and `outset` draw solid.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum LineStyle {
    None,
    Solid,
    Dashed,
    Dotted,
    Double,
}

impl LineStyle {
    fn parse(v: &str) -> Option<LineStyle> {
        Some(match v {
            "none" | "hidden" => LineStyle::None,
            "solid" | "groove" | "ridge" | "inset" | "outset" => LineStyle::Solid,
            "dashed" => LineStyle::Dashed,
            "dotted" => LineStyle::Dotted,
            "double" => LineStyle::Double,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum LineH {
    Normal,
    Mult(f32),
    Px(f32),
}

/// `text-transform`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Transform {
    None,
    Upper,
    Lower,
    Capitalize,
}

impl Transform {
    /// `t` transformed. `Capitalize` upper-cases the first letter of each
    /// word in this text node (a word split across elements is not seen).
    pub fn apply(self, t: &str) -> String {
        match self {
            Transform::None => t.to_string(),
            Transform::Upper => t.to_uppercase(),
            Transform::Lower => t.to_lowercase(),
            Transform::Capitalize => {
                let mut out = String::with_capacity(t.len());
                let mut start = true;
                for c in t.chars() {
                    if start && c.is_alphabetic() {
                        out.extend(c.to_uppercase());
                    } else {
                        out.push(c);
                    }
                    start = !(c.is_alphanumeric() || c == '\'' || c == '\u{2019}');
                }
                out
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Float {
    None,
    Left,
    Right,
}

/// `background-size`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum BgSize {
    Cover,
    Contain,
    /// Width and height; `Auto` keeps the image's own or its aspect ratio.
    Is(Len, Len),
}

/// How a background image is placed in its box.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct BgPlace {
    pub repeat_x: bool,
    pub repeat_y: bool,
    pub size: BgSize,
    pub x: Len,
    pub y: Len,
}

impl BgPlace {
    const INITIAL: BgPlace = BgPlace {
        repeat_x: true,
        repeat_y: true,
        size: BgSize::Is(Len::Auto, Len::Auto),
        x: Len::Pct(0.0),
        y: Len::Pct(0.0),
    };
}

/// `linear-gradient()`: its direction and colour stops.
#[derive(Clone, PartialEq, Debug)]
pub struct Gradient {
    /// Clockwise from "to top".
    pub degrees: f32,
    /// Position along the line, 0 to 1, in order.
    pub stops: Vec<(f32, Color)>,
}

/// Rounded corners: one radius and the corners that have it, as bits from the
/// top left clockwise. Unequal corners of one box take the largest.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Radius {
    pub px: f32,
    pub corners: u8,
}

impl Radius {
    pub const NONE: Radius = Radius {
        px: 0.0,
        corners: 0,
    };

    pub fn all(px: f32) -> Radius {
        Radius { px, corners: 15 }
    }

    pub fn scaled(self, scale: f32) -> Radius {
        Radius {
            px: self.px * scale,
            ..self
        }
    }
}

#[derive(Clone, Debug)]
pub struct Style {
    pub display: Display,
    pub color: Color,
    pub bg: Color,
    pub bg_image: Option<Rc<str>>,
    pub bg_place: BgPlace,
    pub gradient: Option<Arc<Gradient>>,
    pub font_family: Rc<str>,
    pub font_size: f32,
    pub bold: bool,
    /// `font-weight` as a number, when it is not plain 400 or 700.
    pub weight: Option<u16>,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub line_height: LineH,
    pub align: Align,
    pub nowrap: bool,
    pub pre: bool,
    pub transform: Transform,
    pub width: Len,
    pub height: Len,
    pub min_width: Len,
    pub max_width: Len,
    pub min_height: Len,
    /// `box-sizing: border-box`.
    pub border_box: bool,
    /// Top, right, bottom, left.
    pub margin: [Len; 4],
    pub padding: [f32; 4],
    pub border: [f32; 4],
    pub border_color: [Color; 4],
    pub border_style: [LineStyle; 4],
    pub radius: f32,
    /// `border-radius` when it is not one length in px for all corners: top
    /// left, top right, bottom right, bottom left.
    pub corners: Option<[Len; 4]>,
    pub valign: VAlign,
    pub collapse: bool,
    /// `table-layout: fixed`.
    pub fixed_layout: bool,
    pub spacing: f32,
    /// `visibility: hidden`: takes space, never painted.
    pub hidden: bool,
    pub letter_spacing: f32,
    pub word_spacing: f32,
    /// `text-indent`; negative for Word's hanging lists.
    pub indent: Len,
    pub list_none: bool,
    pub marker: Marker,
    pub float: Float,
    /// `clear`: left, right.
    pub clear: (bool, bool),
    pub overflow_hidden: bool,
    pub max_height: Len,
    pub opacity_zero: bool,
    /// `opacity`, 0 to 1: this element's own until [`apply_opacity`] has
    /// multiplied it down the tree.
    pub opacity: f32,
    /// `object-fit`: fill, or else `cover`.
    pub cover: bool,
    pub link_color: Option<Color>,
    /// Legacy `align=` / `<center>` (`-webkit-center` and `-webkit-right` in
    /// a browser): besides the text it also moves child tables and
    /// fixed-width blocks. A table does not pass it on; `Left` is none.
    pub block_align: Align,
    /// The parent's [`Style::block_align`]: what moves this box.
    pub moved_by: Align,
    /// `dir=rtl` / `direction: rtl`: table columns and inline boxes run from
    /// the right (the trick that swaps two columns of a newsletter).
    pub rtl: bool,
    /// `word-break: break-all`.
    pub break_all: bool,
    /// `overflow-wrap` / `word-wrap`: anywhere | break-word.
    pub wrap_anywhere: bool,
    pub fixed: bool,
    pub absolute: bool,
    pub negative_offset: bool,
}

impl Style {
    /// The rounded corners of a box `w` wide.
    pub fn radius_on(&self, w: f32) -> Radius {
        let Some(c) = self.corners else {
            return Radius::all(self.radius);
        };
        let mut r = Radius::NONE;
        for (l, bit) in c.iter().zip([1u8, 2, 4, 8]) {
            let v = match *l {
                Len::Px(p) => p,
                Len::Pct(p) => w * p / 100.0,
                Len::Auto => 0.0,
            };
            if v > 0.5 {
                r.px = r.px.max(v);
                r.corners |= bit;
            }
        }
        r
    }

    /// Whether, as an inline element, it paints a box behind its text.
    pub fn has_inline_box(&self) -> bool {
        self.bg[3] > 0 || self.border.iter().any(|&b| b > 0.0)
    }

    pub fn root() -> Style {
        Style {
            display: Display::Block,
            color: [0, 0, 0, 255],
            bg: [0; 4],
            bg_image: None,
            bg_place: BgPlace::INITIAL,
            gradient: None,
            font_family: "Segoe UI, Arial, sans-serif".into(),
            font_size: 16.0,
            bold: false,
            weight: None,
            italic: false,
            underline: false,
            strike: false,
            line_height: LineH::Normal,
            align: Align::Left,
            nowrap: false,
            pre: false,
            transform: Transform::None,
            width: Len::Auto,
            height: Len::Auto,
            min_width: Len::Auto,
            max_width: Len::Auto,
            min_height: Len::Auto,
            border_box: false,
            margin: [Len::Px(0.0); 4],
            padding: [0.0; 4],
            border: [0.0; 4],
            border_color: [[0, 0, 0, 255]; 4],
            border_style: [LineStyle::None; 4],
            radius: 0.0,
            corners: None,
            valign: VAlign::Baseline,
            collapse: false,
            fixed_layout: false,
            spacing: 2.0,
            hidden: false,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            indent: Len::Px(0.0),
            list_none: false,
            marker: Marker::Disc,
            float: Float::None,
            clear: (false, false),
            overflow_hidden: false,
            max_height: Len::Auto,
            opacity_zero: false,
            opacity: 1.0,
            cover: false,
            link_color: None,
            block_align: Align::Left,
            moved_by: Align::Left,
            rtl: false,
            break_all: false,
            wrap_anywhere: false,
            fixed: false,
            absolute: false,
            negative_offset: false,
        }
    }

    fn inherit(p: &Style) -> Style {
        Style {
            display: Display::Inline,
            color: p.color,
            font_family: p.font_family.clone(),
            font_size: p.font_size,
            bold: p.bold,
            weight: p.weight,
            italic: p.italic,
            underline: p.underline,
            strike: p.strike,
            line_height: p.line_height,
            align: p.align,
            nowrap: p.nowrap,
            pre: p.pre,
            transform: p.transform,
            hidden: p.hidden,
            letter_spacing: p.letter_spacing,
            word_spacing: p.word_spacing,
            indent: p.indent,
            list_none: p.list_none,
            marker: p.marker,
            link_color: p.link_color,
            block_align: p.block_align,
            moved_by: p.block_align,
            rtl: p.rtl,
            break_all: p.break_all,
            wrap_anywhere: p.wrap_anywhere,
            ..Style::root()
        }
    }
}

// ---------------------------------------------------------------- cascade

pub struct Styles {
    /// One per DOM node.
    pub v: Vec<Style>,
    pub remote_urls: Vec<String>,
    /// `prefers-color-scheme: dark` rules or a dark `color-scheme` meta.
    pub has_dark_rules: bool,
    /// The colours were dimmed for the dark theme.
    pub dimmed: bool,
}

pub const MAX_FONT_PX: f32 = 200.0;
/// `repeat`, `no-repeat`, `repeat-x`, `repeat-y` (and the rest as `repeat`);
/// returns whether `v` is one of them.
fn set_bg_repeat(p: &mut BgPlace, v: &str) -> bool {
    let (x, y) = match v {
        "no-repeat" => (false, false),
        "repeat-x" => (true, false),
        "repeat-y" => (false, true),
        "repeat" | "space" | "round" => (true, true),
        _ => return false,
    };
    p.repeat_x = x;
    p.repeat_y = y;
    true
}

fn parse_bg_size(toks: &[&str], fs: f32) -> Option<BgSize> {
    Some(match *toks {
        ["cover"] => BgSize::Cover,
        ["contain"] => BgSize::Contain,
        [w] => BgSize::Is(parse_len(w, fs)?, Len::Auto),
        [w, h, ..] => BgSize::Is(parse_len(w, fs)?, parse_len(h, fs)?),
        _ => return None,
    })
}

/// `background-position` with one or two values, keywords in either order.
fn set_bg_position(p: &mut BgPlace, toks: &[&str], fs: f32) {
    let (mut x, mut y) = (None, None);
    let mut rest = vec![];
    for &t in toks {
        match t {
            "left" => x = Some(Len::Pct(0.0)),
            "right" => x = Some(Len::Pct(100.0)),
            "top" => y = Some(Len::Pct(0.0)),
            "bottom" => y = Some(Len::Pct(100.0)),
            "center" => rest.push(Len::Pct(50.0)),
            t => rest.extend(parse_len(t, fs)),
        }
    }
    let mut rest = rest.into_iter();
    let x = x.or_else(|| rest.next());
    let y = y.or_else(|| rest.next());
    if x.is_none() && y.is_none() {
        return;
    }
    p.x = x.unwrap_or(Len::Pct(50.0));
    p.y = y.unwrap_or(Len::Pct(50.0));
}

/// What is left of the `background` shorthand once the colour and the image
/// are out: repeat, position, `/` and size.
fn parse_bg_shorthand(p: &mut BgPlace, toks: &[&str], fs: f32) {
    let mut pos: Vec<&str> = vec![];
    let mut size: Vec<&str> = vec![];
    let mut after_slash = false;
    for tok in toks {
        // `center/cover`, `center / cover`, `center /cover`.
        for (k, part) in tok.split('/').enumerate() {
            if k > 0 {
                after_slash = true;
            }
            if part.is_empty() || matches!(part, "scroll" | "fixed" | "local") {
                continue;
            }
            if part.ends_with("-box") || set_bg_repeat(p, part) {
                continue;
            }
            if after_slash {
                size.push(part);
            } else {
                pos.push(part);
            }
        }
    }
    set_bg_position(p, &pos, fs);
    if let Some(sz) = parse_bg_size(&size, fs) {
        p.size = sz;
    }
}

/// Split on commas outside parentheses.
fn split_commas(v: &str) -> Vec<&str> {
    let mut out = vec![];
    let mut depth = 0u32;
    let mut start = 0;
    for (i, ch) in v.char_indices() {
        match ch {
            '(' => depth = depth.saturating_add(1),
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                out.push(v.get(start..i).unwrap_or(""));
                start = i.saturating_add(1);
            }
            _ => {}
        }
    }
    out.push(v.get(start..).unwrap_or(""));
    out
}

/// The inside of the parentheses of the gradient function in `value`.
fn gradient_args(value: &str) -> Option<&str> {
    let (_, inner) = value.split_once("gradient(")?;
    let mut depth = 1u32;
    for (i, ch) in inner.char_indices() {
        match ch {
            '(' => depth = depth.saturating_add(1),
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return inner.get(..i);
                }
            }
            _ => {}
        }
    }
    Some(inner)
}

/// The first colour of any gradient in `value`.
fn first_stop(value: &str) -> Option<Color> {
    split_commas(gradient_args(value)?)
        .iter()
        .find_map(|p| split_top(p.trim()).first().and_then(|t| parse_color(t)))
}

/// `linear-gradient(...)` with an angle or `to` a side or corner, and stops
/// at percentages or evenly spaced. The older `-webkit-` form counts angles
/// the other way and names the side it starts from.
fn parse_gradient(value: &str) -> Option<Gradient> {
    if !value.contains("linear-gradient(") {
        return None;
    }
    let legacy = value.contains("-linear-gradient(");
    let args = split_commas(gradient_args(value)?);
    let mut iter = args.iter().map(|a| a.trim()).peekable();
    let head = *iter.peek()?;
    let side = |w: &str| match w {
        "top" => Some(0.0f32),
        "right" => Some(90.0),
        "bottom" => Some(180.0),
        "left" => Some(270.0),
        _ => None,
    };
    let direction: Option<f32> = if let Some(a) = head.strip_suffix("deg").and_then(num) {
        Some(if legacy { 90.0 - a } else { a })
    } else if let Some(a) = head.strip_suffix("turn").and_then(num) {
        Some(a * 360.0)
    } else if let Some(to) = head.strip_prefix("to ") {
        match *split_top(to).as_slice() {
            [w] => side(w),
            [a, b] => {
                let (a, b) = (side(a)?, side(b)?);
                // A corner: near enough, as boxes in mail are wide bands.
                let (lo, hi) = (a.min(b), a.max(b));
                Some(if hi - lo > 180.0 {
                    (lo + 360.0 + hi) / 2.0
                } else {
                    (lo + hi) / 2.0
                })
            }
            _ => None,
        }
    } else if legacy {
        match *split_top(head).as_slice() {
            [w] => side(w).map(|d| d + 180.0),
            [a, b] => {
                let (a, b) = (side(a)? + 180.0, side(b)? + 180.0);
                Some((a + b) / 2.0)
            }
            _ => None,
        }
    } else {
        None
    };
    if direction.is_some() {
        iter.next();
    }
    let mut stops: Vec<(Option<f32>, Color)> = vec![];
    for part in iter {
        let toks = split_top(part);
        let c = parse_color(toks.first()?)?;
        let at = toks
            .get(1)
            .and_then(|t| t.strip_suffix('%'))
            .and_then(num)
            .map(|p| p / 100.0);
        stops.push((at, c));
    }
    if stops.len() < 2 || stops.len() > 32 {
        return None;
    }
    // The ends default to 0 and 100%, what lies between shares the room, and
    // positions never go back.
    let last = stops.len().saturating_sub(1);
    if let Some(s) = stops.first_mut() {
        s.0 = s.0.or(Some(0.0));
    }
    if let Some(s) = stops.last_mut() {
        s.0 = s.0.or(Some(1.0));
    }
    let mut out: Vec<(f32, Color)> = Vec::with_capacity(stops.len());
    let mut i = 0;
    let mut prev = 0.0f32;
    while let Some(&(at, c)) = stops.get(i) {
        if let Some(at) = at {
            prev = at.clamp(prev, 1.0);
            out.push((prev, c));
            i = i.saturating_add(1);
            continue;
        }
        let next = (i..=last).find(|&k| stops.get(k).is_some_and(|s| s.0.is_some()));
        let Some(next) = next else {
            break;
        };
        let end = stops
            .get(next)
            .and_then(|s| s.0)
            .unwrap_or(1.0)
            .clamp(prev, 1.0);
        let count = next.saturating_sub(i).saturating_add(1) as f32;
        for (k, stop) in stops.get(i..next).unwrap_or(&[]).iter().enumerate() {
            out.push((prev + (end - prev) * (k as f32 + 1.0) / count, stop.1));
        }
        i = next;
    }
    Some(Gradient {
        degrees: direction.unwrap_or(180.0).rem_euclid(360.0),
        stops: out,
    })
}

/// `font-weight`. A number other than 400 or 700 asks for that weight of the
/// family (Segoe UI Semibold at 600, Arial Black at 900), so far as the family
/// has it; `bold` stays the question of whether it counts as bold.
fn set_weight(s: &mut Style, v: &str) {
    let (bold, weight) = match v {
        "bold" | "bolder" => (true, None),
        "normal" | "lighter" => (false, None),
        v => match v.parse::<u16>() {
            Ok(n @ 1..=1000) => (n >= 600, Some(n).filter(|n| !matches!(n, 400 | 700))),
            _ => return,
        },
    };
    s.bold = bold;
    s.weight = weight;
}

/// A border width nothing has declared.
const UNSET_BORDER: f32 = -1.0;
/// `medium`, the initial border width.
const MEDIUM_BORDER: f32 = 3.0;

/// Compute every node's style; `None` when the deadline passes.
pub fn compute(dom: &Dom, ctx: &Ctx, deadline: &Deadline) -> Option<Styles> {
    let mut sheet = Sheet::default();
    for t in &dom.styles {
        sheet.parse_more(t, ctx);
    }
    let mut remote = std::mem::take(&mut sheet.remote_urls);
    let mut v: Vec<Style> = Vec::with_capacity(dom.nodes.len());
    let root = Style::root();
    v.push(root.clone());
    let mut normal = Vec::new();
    let mut imp = Vec::new();
    let mut inline = Vec::new();
    let mut dark_meta = false;
    for (i, node) in dom.nodes.iter().enumerate().skip(1) {
        // Selector matching can cost milliseconds per element.
        if (i % 256 == 0 || !sheet.is_empty()) && deadline.expired() {
            return None;
        }
        let ps = v.get(node.parent as usize).unwrap_or(&root);
        let st = match &node.kind {
            Kind::Text(_) => ps.clone(),
            Kind::Elem { tag, .. } => {
                let Ok(id) = Id::try_from(i) else {
                    break;
                };
                normal.clear();
                imp.clear();
                inline.clear();
                sheet.matched(dom, id, &mut normal, &mut imp);
                if let Some(st) = dom.attr(id, "style") {
                    css::parse_decls(st, &mut inline);
                    for d in &inline {
                        let mut u = vec![];
                        if d.value.contains("url(") {
                            css::urls_in(&d.value, &mut u);
                        }
                        remote.extend(u.into_iter().filter(|x| css::is_remote(x)));
                    }
                }
                if &**tag == "meta"
                    && dom
                        .attr(id, "name")
                        .is_some_and(|n| n.eq_ignore_ascii_case("color-scheme"))
                    && dom
                        .attr(id, "content")
                        .is_some_and(|c| c.to_ascii_lowercase().contains("dark"))
                {
                    dark_meta = true;
                }
                style_for(dom, id, ps, &normal, &imp, &inline)
            }
        };
        v.push(st);
    }
    apply_opacity(dom, &mut v);
    let has_dark_rules = sheet.has_dark_rules || dark_meta;
    let dimmed = ctx.dim && !has_dark_rules;
    if dimmed {
        dim(dom, &mut v, deadline)?;
    }
    Some(Styles {
        v,
        remote_urls: remote,
        has_dark_rules,
        dimmed,
    })
}

/// Fade every colour by the `opacity` of its element and its ancestors. An
/// approximation of a group's opacity: where an element's own boxes overlap,
/// they show through each other.
fn apply_opacity(dom: &Dom, v: &mut [Style]) {
    if v.iter().all(|s| s.opacity >= 1.0) {
        return;
    }
    let fade = |c: &mut Color, by: f32| c[3] = (f32::from(c[3]) * by).round() as u8;
    for i in 0..v.len() {
        let Some(node) = dom.nodes.get(i).filter(|_| i > 0) else {
            continue;
        };
        let parent = v.get(node.parent as usize).map_or(1.0, |p| p.opacity);
        let Some(s) = v.get_mut(i) else {
            continue;
        };
        // A text node's style is a copy of its parent's, not a further step.
        s.opacity = if matches!(node.kind, Kind::Text(_)) {
            parent
        } else {
            parent * s.opacity
        };
        let by = s.opacity;
        if by < 1.0 {
            fade(&mut s.color, by);
            fade(&mut s.bg, by);
            s.border_color.iter_mut().for_each(|c| fade(c, by));
            if let Some(c) = &mut s.link_color {
                fade(c, by);
            }
        }
    }
}

// ---------------------------------------------------------------- dark dimming

/// The canvas colour of a page with no background of its own.
pub const WHITE: Color = [255; 4];

/// Dim every style's colours for a dark theme. Light backgrounds (Oklab
/// lightness over 0.6) are darkened and dark text (under 0.5) lightened:
/// a lightness flip in Oklab, so white paper becomes a dark grey and black
/// text a soft white; mid tones such as a brand-coloured button keep
/// their colour. Text is then kept at least as legible over its (dimmed)
/// background as it was over the original, up to 4.5:1: low-contrast
/// text is never made harder to read, and text hidden in its background
/// colour stays hidden. Images are not touched. `None` once the deadline
/// passes.
fn dim(dom: &Dom, v: &mut [Style], deadline: &Deadline) -> Option<()> {
    // (original, dimmed) background each node's text sits on, and whether
    // it is an image (text over an image keeps its colour, as the image
    // does).
    let canvas = (WHITE, dim_bg(WHITE), false);
    let mut under: Vec<(Color, Color, bool)> = Vec::with_capacity(v.len());
    let mut memo: HashMap<(Color, Color), Color> = HashMap::new();
    let mut text = |fg: Color, bg: (Color, Color, bool)| {
        if bg.2 {
            return fg;
        }
        *memo
            .entry((fg, bg.0))
            .or_insert_with(|| dim_text(fg, bg.0, bg.1))
    };
    for (i, s) in v.iter_mut().enumerate() {
        // A miss in the contrast search costs microseconds.
        if i % 256 == 0 && deadline.expired() {
            return None;
        }
        // Parents come before their children.
        let inherited = dom
            .nodes
            .get(i)
            .filter(|_| i > 0)
            .and_then(|n| under.get(n.parent as usize))
            .copied()
            .unwrap_or(canvas);
        let orig = s.bg;
        s.bg = dim_bg(orig);
        if let Some(g) = &mut s.gradient {
            let stops = g.stops.iter().map(|&(at, c)| (at, dim_bg(c))).collect();
            *g = Arc::new(Gradient {
                degrees: g.degrees,
                stops,
            });
        }
        let here = if s.bg_image.is_some() {
            (orig, s.bg, true)
        } else if orig[3] > 0 {
            (orig, s.bg, false)
        } else {
            inherited
        };
        s.color = text(s.color, here);
        s.link_color = s.link_color.map(|c| text(c, here));
        for b in s.border_color.iter_mut() {
            *b = dim_line(*b);
        }
        under.push(here);
    }
    Some(())
}

/// sRGB (8-bit) to Oklab.
fn oklab(c: Color) -> [f32; 3] {
    let lin = |v: u8| {
        let v = f32::from(v) / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    let [r, g, b, _] = c;
    let (r, g, b) = (lin(r), lin(g), lin(b));
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

/// Oklab back to 8-bit sRGB (clipped to the gamut), with alpha `a`.
fn from_oklab([l, a, b]: [f32; 3], alpha: u8) -> Color {
    let l_ = l + 0.396_337_78 * a + 0.215_803_76 * b;
    let m_ = l - 0.105_561_346 * a - 0.063_854_17 * b;
    let s_ = l - 0.089_484_18 * a - 1.291_485_5 * b;
    let (l3, m3, s3) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    let r = 4.076_741_7 * l3 - 3.307_711_6 * m3 + 0.230_969_94 * s3;
    let g = -1.268_438 * l3 + 2.609_757_4 * m3 - 0.341_319_38 * s3;
    let bl = -0.004_196_086_3 * l3 - 0.703_418_6 * m3 + 1.707_614_7 * s3;
    let enc = |v: f32| {
        let v = if v.is_finite() {
            v.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let v = if v <= 0.003_130_8 {
            v * 12.92
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        };
        (v * 255.0).round().clamp(0.0, 255.0) as u8
    };
    [enc(r), enc(g), enc(bl), alpha]
}

/// WCAG relative luminance.
fn luminance(c: Color) -> f32 {
    let lin = |v: u8| {
        let v = f32::from(v) / 255.0;
        if v <= 0.039_28 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    let [r, g, b, _] = c;
    0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
}

/// The WCAG contrast ratio of two colours (alpha ignored), 1 to 21.
pub fn contrast(a: Color, b: Color) -> f32 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// A background for the dark theme: anything light is darkened
/// (white becomes Oklab L 0.22, the dark theme's surface grey), and slightly desaturated.
pub fn dim_bg(c: Color) -> Color {
    let [l, a, b] = oklab(c);
    if c[3] == 0 || l <= 0.6 {
        return c;
    }
    from_oklab([1.22 - l, a * 0.8, b * 0.8], c[3])
}

/// A foreground for the dark theme: darker than mid-grey is lightened
/// (black becomes Oklab L 0.92, a soft white; dark grey text 0.82).
fn dim_fg(c: Color) -> Color {
    let [l, a, b] = oklab(c);
    if c[3] == 0 || l >= 0.5 {
        return c;
    }
    from_oklab([0.92 - l * 0.5, a, b], c[3])
}

/// A border or rule: light ones darken like backgrounds, dark ones
/// lighten like text, so a divider stays a divider.
fn dim_line(c: Color) -> Color {
    let l = oklab(c)[0];
    if l > 0.6 { dim_bg(c) } else { dim_fg(c) }
}

/// Text colour `fg`, written for background `orig`, now drawn over the
/// dimmed background `bg`: dimmed, then moved in lightness until it reads
/// at least as well as before, up to 4.5:1.
pub fn dim_text(fg: Color, orig: Color, bg: Color) -> Color {
    let want = contrast(fg, orig).min(4.5);
    // Text all but invisible on its background (a hidden preheader) is
    // dimmed like the background, so it stays out of sight.
    if want < 1.5 {
        return dim_bg(fg);
    }
    let out = dim_fg(fg);
    if fg[3] == 0 || contrast(out, bg) >= want {
        return out;
    }
    let [l, a, b] = oklab(out);
    let away = if l >= oklab(bg)[0] { 1.0 } else { -1.0 };
    for dir in [away, -away] {
        for step in 1..=50u8 {
            let c = from_oklab([l + dir * f32::from(step) * 0.02, a, b], fg[3]);
            if contrast(c, bg) >= want {
                return c;
            }
        }
    }
    // Out of gamut either way: plain white or black, whichever reads.
    let (w, k) = ([255, 255, 255, fg[3]], [0, 0, 0, fg[3]]);
    if contrast(w, bg) >= contrast(k, bg) {
        w
    } else {
        k
    }
}

fn style_for(
    dom: &Dom,
    id: Id,
    ps: &Style,
    normal: &[&Decl],
    imp: &[&Decl],
    inline: &[Decl],
) -> Style {
    let tag = dom.tag(id);
    let mut s = Style::inherit(ps);
    // Negative until something sets it: a style alone means `medium`.
    s.border = [UNSET_BORDER; 4];
    ua_defaults(dom, id, tag, &mut s, ps);
    presentational(dom, id, tag, &mut s);
    let order = normal
        .iter()
        .copied()
        .chain(inline.iter().filter(|d| !d.important))
        .chain(imp.iter().copied())
        .chain(inline.iter().filter(|d| d.important));
    // Two passes: font-size first so em units resolve.
    for d in order.clone() {
        if matches!(&*d.name, "font-size" | "font") {
            apply(&mut s, &d.name, &d.value, ps);
        }
    }
    for d in order {
        if &*d.name != "font-size" {
            apply(&mut s, &d.name, &d.value, ps);
        }
    }
    s.font_size = s.font_size.clamp(0.0, MAX_FONT_PX);
    // Adversarial sizes (border=9999, height=99999, cellpadding=-5). A side
    // with no style has no width, whatever `border-width` says.
    for (b, st) in s.border.iter_mut().zip(s.border_style) {
        *b = match st {
            LineStyle::None => 0.0,
            _ if *b == UNSET_BORDER => MEDIUM_BORDER,
            _ => b.clamp(0.0, 50.0),
        };
    }
    for p in s.padding.iter_mut() {
        *p = p.clamp(0.0, 500.0);
    }
    // Images keep their declared aspect ratio and are clamped in layout.
    if let Len::Px(h) = s.height
        && tag != "img"
    {
        s.height = Len::Px(h.clamp(0.0, 5000.0));
    }
    if let Len::Px(h) = s.min_height {
        s.min_height = Len::Px(h.clamp(0.0, 5000.0));
    }
    s.radius = s.radius.clamp(0.0, 500.0);
    s.letter_spacing = s.letter_spacing.clamp(-10.0, 100.0);
    s.spacing = s.spacing.clamp(0.0, 100.0);
    // Classic hidden-preheader patterns without display:none.
    let zero = |l: Len| matches!(l, Len::Px(p) if p <= 0.5);
    if s.overflow_hidden && (zero(s.max_height) || zero(s.height)) && s.display != Display::Cell {
        s.display = Display::None;
    }
    if s.opacity_zero && s.font_size <= 1.0 {
        s.display = Display::None;
    }
    // Spoofing defence: fixed overlays and absolute boxes pushed out of
    // place are dropped (everything else positioned stays in flow).
    if s.fixed || (s.absolute && s.negative_offset) {
        s.display = Display::None;
    }
    if s.float != Float::None && s.display == Display::Inline {
        s.display = Display::Block;
    }
    s
}

fn em(v: f32, fs: f32) -> Len {
    Len::Px(v * fs)
}

/// A finite number (`"NaN"` and `"inf"` parse as floats too).
pub fn num(v: &str) -> Option<f32> {
    v.trim().parse::<f32>().ok().filter(|n| n.is_finite())
}

fn ua_defaults(dom: &Dom, id: Id, tag: &str, s: &mut Style, ps: &Style) {
    use Display::*;
    let fs = s.font_size;
    match tag {
        "head" | "title" | "script" | "style" | "meta" | "link" | "template" | "area" | "base"
        | "noscript" | "select" | "datalist" | "xml" | "iframe" | "object" | "embed" | "applet"
        | "video" | "audio" | "svg" | "math" | "map" | "frameset" | "frame" | "param"
        | "source" | "input" | "button" | "textarea" | "canvas" => s.display = None,
        // VML / Office namespaces: Outlook-only content.
        t if t != "o:p"
            && (t.starts_with("v:")
                || t.starts_with("o:")
                || t.starts_with("w:")
                || t.starts_with("m:")) =>
        {
            s.display = None
        }
        "html" | "body" | "div" | "section" | "article" | "header" | "footer" | "main" | "nav"
        | "aside" | "form" | "fieldset" | "figure" | "figcaption" | "address" | "details"
        | "summary" | "legend" | "caption" => {
            s.display = Block;
            if tag == "caption" {
                s.align = Align::Center;
            }
            if tag == "body" {
                s.margin = [Len::Px(8.0); 4];
            }
            if tag == "address" {
                s.italic = true;
            }
        }
        "center" => {
            s.display = Block;
            s.block_align = Align::Center;
            s.align = Align::Center;
        }
        "p" => {
            s.display = Block;
            s.margin[0] = em(1.0, fs);
            s.margin[2] = em(1.0, fs);
        }
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            s.display = Block;
            let (sz, m) = match tag {
                "h1" => (2.0, 0.67),
                "h2" => (1.5, 0.83),
                "h3" => (1.17, 1.0),
                "h4" => (1.0, 1.33),
                "h5" => (0.83, 1.67),
                _ => (0.67, 2.33),
            };
            s.font_size = ps.font_size * sz;
            s.bold = true;
            s.weight = Option::None;
            s.margin[0] = em(m, s.font_size);
            s.margin[2] = em(m, s.font_size);
        }
        "ul" | "ol" | "dl" | "menu" | "dir" => {
            s.display = Block;
            s.margin[0] = em(1.0, fs);
            s.margin[2] = em(1.0, fs);
            if tag != "dl" {
                s.padding[3] = 40.0;
            }
            s.marker = if tag == "ol" {
                Marker::Decimal
            } else {
                // Inside other lists: circles, then squares.
                let mut up = dom.parent(id);
                let mut depth = 0u32;
                while up != 0 && depth < 2 {
                    if matches!(dom.tag(up), "ul" | "ol" | "menu" | "dir") {
                        depth = depth.saturating_add(1);
                    }
                    up = dom.parent(up);
                }
                [Marker::Disc, Marker::Circle, Marker::Square]
                    .get(depth as usize)
                    .copied()
                    .unwrap_or(Marker::Square)
            };
            // Nested lists have no vertical margin.
            if matches!(dom.tag(dom.parent(id)), "ul" | "ol" | "li") {
                s.margin[0] = Len::Px(0.0);
                s.margin[2] = Len::Px(0.0);
            }
        }
        "li" => s.display = ListItem,
        "dt" => s.display = Block,
        "dd" => {
            s.display = Block;
            s.margin[3] = Len::Px(40.0);
        }
        "blockquote" => {
            s.display = Block;
            s.margin = [em(1.0, fs), Len::Px(40.0), em(1.0, fs), Len::Px(40.0)];
        }
        "pre" | "listing" | "xmp" | "plaintext" => {
            s.display = Block;
            s.pre = true;
            s.font_family = MONO.into();
            s.font_size = ps.font_size * 13.0 / 16.0;
            s.margin[0] = Len::Px(13.0);
            s.margin[2] = Len::Px(13.0);
        }
        "code" | "kbd" | "samp" | "tt" => {
            s.font_family = MONO.into();
            s.font_size = ps.font_size * 13.0 / 16.0;
        }
        "hr" => {
            s.display = Block;
            s.margin[0] = Len::Px(8.0);
            s.margin[2] = Len::Px(8.0);
            s.margin[1] = Len::Auto;
            s.margin[3] = Len::Auto;
            s.border = [1.0; 4];
            s.border_style = [LineStyle::Solid; 4];
            s.border_color = [[154, 154, 154, 255]; 4];
        }
        "table" => {
            s.display = Table;
            s.spacing = 2.0;
            // A browser's tables drop the legacy `-webkit-` alignments, also
            // in standards mode (webmail's), while a `text-align` from CSS
            // goes on into the cells.
            if s.block_align != Align::Left {
                s.align = Align::Left;
                s.block_align = Align::Left;
            }
        }
        "col" | "colgroup" => s.display = None,
        "thead" | "tbody" | "tfoot" => s.display = RowGroup,
        "tr" => s.display = Row,
        "td" | "th" => {
            s.display = Cell;
            s.padding = [1.0; 4];
            s.valign = VAlign::Middle;
            if tag == "th" {
                s.bold = true;
                s.weight = Option::None;
                s.align = Align::Center;
            }
            // Cell defaults from the table's attributes.
            if let Some(t) = ancestor(dom, id, "table") {
                if let Some(cp) = dom
                    .attr(t, "cellpadding")
                    .and_then(|v| num(v.trim().trim_end_matches("px")))
                {
                    s.padding = [cp; 4];
                }
                if dom.attr(t, "border").and_then(num).is_some_and(|b| b > 0.0) {
                    s.border = [1.0; 4];
                    s.border_style = [LineStyle::Solid; 4];
                    s.border_color = [[128, 128, 128, 255]; 4];
                }
            }
        }
        "img" => s.display = InlineBlock,
        "mark" => {
            s.bg = [255, 255, 0, 255];
            s.color = [0, 0, 0, 255];
        }
        "b" | "strong" => {
            s.bold = true;
            s.weight = Option::None;
        }
        "i" | "em" | "cite" | "var" | "dfn" => s.italic = true,
        "u" | "ins" => s.underline = true,
        "s" | "strike" | "del" => s.strike = true,
        "small" => s.font_size = ps.font_size * 0.8333,
        "big" => s.font_size = ps.font_size * 1.2,
        "sub" | "sup" => s.font_size = ps.font_size * 0.83,
        "a" if dom.attr(id, "href").is_some() => {
            s.underline = true;
            s.color = s.link_color.unwrap_or([0, 0, 238, 255]);
        }
        _ => {}
    }
    if dom.attr(id, "hidden").is_some() {
        s.display = None;
    }
}

const MONO: &str = "Consolas, Courier New, monospace";

pub fn ancestor(dom: &Dom, mut id: Id, tag: &str) -> Option<Id> {
    while id != 0 {
        id = dom.parent(id);
        if id != 0 && dom.tag(id) == tag {
            return Some(id);
        }
    }
    None
}

fn presentational(dom: &Dom, id: Id, tag: &str, s: &mut Style) {
    if let Some(d) = dom.attr(id, "dir") {
        set_direction(s, &d.trim().to_ascii_lowercase());
    }
    if let Some(c) = dom.attr(id, "bgcolor").and_then(parse_color_legacy) {
        s.bg = c;
    }
    if let Some(c) = dom.attr(id, "background") {
        s.bg_image = Some(c.trim().into());
    }
    if tag == "body" {
        if let Some(c) = dom.attr(id, "text").and_then(parse_color_legacy) {
            s.color = c;
        }
        if let Some(c) = dom.attr(id, "link").and_then(parse_color_legacy) {
            s.link_color = Some(c);
        }
    }
    if let Some(a) = dom.attr(id, "align") {
        let a = a.trim().to_ascii_lowercase();
        if tag == "table" || tag == "img" {
            match a.as_str() {
                "center" => {
                    s.margin[1] = Len::Auto;
                    s.margin[3] = Len::Auto;
                }
                "right" => s.float = Float::Right,
                "left" => s.float = Float::Left,
                _ => {}
            }
        } else {
            match a.as_str() {
                "left" => {
                    s.align = Align::Left;
                    s.block_align = Align::Left;
                }
                "right" => {
                    s.align = Align::Right;
                    s.block_align = Align::Right;
                }
                "center" | "middle" => {
                    s.align = Align::Center;
                    s.block_align = Align::Center;
                }
                "justify" => s.align = Align::Justify,
                _ => {}
            }
        }
    }
    if let Some(v) = dom.attr(id, "valign") {
        s.valign = match v.trim().to_ascii_lowercase().as_str() {
            "top" => VAlign::Top,
            "bottom" => VAlign::Bottom,
            "middle" => VAlign::Middle,
            _ => VAlign::Baseline,
        };
    }
    if let Some(w) = dom.attr(id, "width").and_then(parse_dim) {
        s.width = w;
    }
    if let Some(h) = dom.attr(id, "height").and_then(parse_dim) {
        s.height = h;
    }
    if tag == "img" {
        let gap = |name| dom.attr(id, name).and_then(num).map(Len::Px);
        if let Some(h) = gap("hspace") {
            s.margin[1] = h;
            s.margin[3] = h;
        }
        if let Some(v) = gap("vspace") {
            s.margin[0] = v;
            s.margin[2] = v;
        }
        if let Some(b) = dom.attr(id, "border").and_then(num).filter(|&b| b > 0.0) {
            s.border = [b; 4];
            s.border_style = [LineStyle::Solid; 4];
            // Inherited: inside a link, the link's colour.
            s.border_color = [s.color; 4];
        }
    }
    if tag == "br"
        && let Some(c) = dom.attr(id, "clear")
    {
        s.clear = parse_clear(&c.trim().to_ascii_lowercase());
    }
    if matches!(tag, "ul" | "ol" | "li")
        && let Some(m) = dom.attr(id, "type").and_then(Marker::from_attr)
    {
        s.marker = m;
    }
    if matches!(tag, "td" | "th") && dom.attr(id, "nowrap").is_some() {
        s.nowrap = true;
    }
    if tag == "hr" {
        let size = dom.attr(id, "size").and_then(num).filter(|&v| v >= 1.0);
        let color = dom.attr(id, "color").and_then(parse_color_legacy);
        if dom.attr(id, "noshade").is_some() || color.is_some() {
            let c = color.unwrap_or([128, 128, 128, 255]);
            s.border_color = [c; 4];
            s.bg = c;
        }
        if let Some(h) = size {
            // A box of this height in all, as a browser draws it.
            s.height = Len::Px((h - 2.0).max(0.0));
        }
        match dom.attr(id, "align").map(str::trim) {
            Some(a) if a.eq_ignore_ascii_case("left") => {
                s.margin[3] = Len::Px(0.0);
                s.margin[1] = Len::Auto;
            }
            Some(a) if a.eq_ignore_ascii_case("right") => {
                s.margin[3] = Len::Auto;
                s.margin[1] = Len::Px(0.0);
            }
            _ => {}
        }
    }
    if tag == "table" {
        if let Some(b) = dom.attr(id, "border").and_then(num).filter(|&b| b > 0.0) {
            s.border = [b; 4];
            s.border_style = [LineStyle::Solid; 4];
            s.border_color = [[128, 128, 128, 255]; 4];
        }
        if let Some(cs) = dom.attr(id, "cellspacing").and_then(num) {
            s.spacing = cs;
        }
        if let Some(c) = dom.attr(id, "bordercolor").and_then(parse_color_legacy) {
            s.border_color = [c; 4];
        }
    }
    if tag == "font" {
        if let Some(c) = dom.attr(id, "color").and_then(parse_color_legacy) {
            s.color = c;
        }
        if let Some(f) = dom.attr(id, "face") {
            s.font_family = clean_family(f).into();
        }
        if let Some(sz) = dom.attr(id, "size") {
            let sz = sz.trim();
            let n: Option<i32> = if let Some(r) = sz.strip_prefix('+') {
                r.parse::<i32>().ok().map(|d| d.saturating_add(3))
            } else if let Some(r) = sz.strip_prefix('-') {
                r.parse::<i32>().ok().map(|d| 3i32.saturating_sub(d))
            } else {
                sz.parse().ok()
            };
            const SIZES: [f32; 7] = [10.0, 13.0, 16.0, 18.0, 24.0, 32.0, 48.0];
            if let Some(px) = n.and_then(|n| SIZES.get(n.clamp(1, 7).saturating_sub(1) as usize)) {
                s.font_size = *px;
            }
        }
    }
}

/// `clear` and `<br clear>`: left, right.
fn parse_clear(v: &str) -> (bool, bool) {
    match v {
        "left" => (true, false),
        "right" => (false, true),
        "both" | "all" => (true, true),
        _ => (false, false),
    }
}

fn parse_dim(v: &str) -> Option<Len> {
    let v = v.trim();
    if let Some(p) = v.strip_suffix('%') {
        return num(p).map(Len::Pct);
    }
    num(v.trim_end_matches("px")).map(Len::Px)
}

fn parse_color_legacy(v: &str) -> Option<Color> {
    let v = v.trim().to_ascii_lowercase();
    if let Some(h) = v.strip_prefix('#') {
        return parse_hex(h);
    }
    if v.len() == 6 && v.chars().all(|c| c.is_ascii_hexdigit()) {
        return parse_hex(&v);
    }
    parse_color(&v)
}

// ---------------------------------------------------------------- declarations

fn apply(s: &mut Style, name: &str, value: &str, ps: &Style) {
    let value = value.trim();
    let fs = s.font_size;
    match name {
        "display" => {
            s.display = match value {
                "none" => Display::None,
                "block" | "flex" | "grid" => Display::Block,
                "inline" | "contents" => Display::Inline,
                "inline-block" | "inline-flex" | "inline-grid" | "inline-table" => {
                    Display::InlineBlock
                }
                "list-item" => Display::ListItem,
                "table" => Display::Table,
                "table-row" => Display::Row,
                "table-cell" => Display::Cell,
                "table-row-group" | "table-header-group" | "table-footer-group" => {
                    Display::RowGroup
                }
                _ => s.display,
            }
        }
        "visibility" => s.hidden = value == "hidden" || value == "collapse",
        "color" => {
            if let Some(c) = parse_color(value) {
                s.color = c;
            }
        }
        "background-color" => {
            if let Some(c) = parse_color(value) {
                s.bg = c;
            }
        }
        "background" | "background-image" => {
            if name == "background" {
                s.bg = [0; 4];
                s.bg_image = None;
                s.gradient = None;
                s.bg_place = BgPlace::INITIAL;
            } else if value == "none" {
                s.bg_image = None;
                s.gradient = None;
            }
            if let Some(u) = url_of(value) {
                s.bg_image = Some(u.into());
            }
            if let Some(g) = parse_gradient(value) {
                // Under it, for what looks at the background's colour.
                if let Some(&(_, c)) = g.stops.first() {
                    s.bg = c;
                }
                s.gradient = Some(Arc::new(g));
            } else if let Some(c) = first_stop(value) {
                // Radial and conic ones: their first colour, as for a client
                // with no gradients (a `bgcolor` usually says the same).
                s.bg = c;
            }
            if name == "background" {
                let mut place = Vec::new();
                for tok in split_top(value) {
                    if let Some(c) = parse_color(tok) {
                        s.bg = c;
                    } else if !tok.contains('(') {
                        place.push(tok);
                    }
                }
                parse_bg_shorthand(&mut s.bg_place, &place, fs);
            }
        }
        "background-repeat" => {
            set_bg_repeat(&mut s.bg_place, value);
        }
        "background-size" => {
            if let Some(size) = parse_bg_size(&split_top(value), fs) {
                s.bg_place.size = size;
            }
        }
        "background-position" => set_bg_position(&mut s.bg_place, &split_top(value), fs),
        "font-family" => s.font_family = clean_family(value).into(),
        "font-size" => {
            s.font_size = match value {
                "xx-small" => 9.0,
                "x-small" => 10.0,
                "small" => 13.0,
                "medium" => 16.0,
                "large" => 18.0,
                "x-large" => 24.0,
                "xx-large" => 32.0,
                "smaller" => ps.font_size / 1.2,
                "larger" => ps.font_size * 1.2,
                v => match parse_len(v, ps.font_size) {
                    Some(Len::Px(p)) => p,
                    Some(Len::Pct(p)) => ps.font_size * p / 100.0,
                    _ => s.font_size,
                },
            }
        }
        "font-weight" => set_weight(s, value),
        "font-style" => s.italic = value == "italic" || value == "oblique",
        "font" => {
            // [style] [weight] size[/line-height] family
            let toks = split_top(value);
            let mut family_from = None;
            for (i, t) in toks.iter().enumerate() {
                match *t {
                    "italic" | "oblique" => s.italic = true,
                    "bold" | "bolder" => s.bold = true,
                    "normal" | "small-caps" | "lighter" => {}
                    t if t.len() == 3 && t.parse::<u16>().is_ok() => set_weight(s, t),
                    t => {
                        let (sz, lh) = t.split_once('/').unwrap_or((t, ""));
                        if let Some(l) = parse_len(sz, ps.font_size) {
                            s.font_size = match l {
                                Len::Px(p) => p,
                                Len::Pct(p) => ps.font_size * p / 100.0,
                                Len::Auto => s.font_size,
                            };
                            if !lh.is_empty() {
                                set_line_height(s, lh);
                            }
                            family_from = Some(i.saturating_add(1));
                            break;
                        }
                    }
                }
            }
            if let Some(fam) = family_from
                .and_then(|i| toks.get(i..))
                .filter(|f| !f.is_empty())
            {
                s.font_family = clean_family(&fam.join(" ")).into();
            }
        }
        "line-height" => set_line_height(s, value),
        "text-align" => {
            s.align = match value {
                "center" | "-webkit-center" => Align::Center,
                "right" | "end" | "-webkit-right" => Align::Right,
                "justify" => Align::Justify,
                "start" if s.rtl => Align::Right,
                _ => Align::Left,
            };
            s.block_align = match value {
                "-webkit-center" => Align::Center,
                "-webkit-right" => Align::Right,
                _ => Align::Left,
            };
        }
        "direction" => set_direction(s, value),
        "text-decoration" | "text-decoration-line" => {
            s.underline = value.contains("underline");
            s.strike = value.contains("line-through");
        }
        "text-transform" => {
            s.transform = match value {
                "uppercase" => Transform::Upper,
                "lowercase" => Transform::Lower,
                "capitalize" => Transform::Capitalize,
                _ => Transform::None,
            }
        }
        "letter-spacing" => {
            if let Some(Len::Px(p)) = parse_len(value, fs) {
                s.letter_spacing = p;
            }
        }
        "word-spacing" => {
            if let Some(Len::Px(p)) = parse_len(value, fs) {
                s.word_spacing = p.clamp(-10.0, 100.0);
            }
        }
        "text-indent" => {
            if let Some(l) = parse_len(value, fs) {
                s.indent = l;
            }
        }
        "white-space" => {
            s.pre = matches!(value, "pre" | "pre-wrap" | "pre-line");
            s.nowrap = matches!(value, "nowrap" | "pre");
        }
        "width" => s.width = parse_len(value, fs).unwrap_or(Len::Auto),
        "height" => s.height = parse_len(value, fs).unwrap_or(Len::Auto),
        "min-width" => s.min_width = parse_len(value, fs).unwrap_or(Len::Auto),
        "max-width" => s.max_width = parse_len(value, fs).unwrap_or(Len::Auto),
        "min-height" => s.min_height = parse_len(value, fs).unwrap_or(Len::Auto),
        "box-sizing" => s.border_box = value == "border-box",
        "margin" => {
            if let Some(v) = four(value, |x| parse_len(x, fs)) {
                s.margin = v;
            }
        }
        "margin-top" => set_len(&mut s.margin[0], value, fs),
        "margin-right" => set_len(&mut s.margin[1], value, fs),
        "margin-bottom" => set_len(&mut s.margin[2], value, fs),
        "margin-left" => set_len(&mut s.margin[3], value, fs),
        "padding" => {
            if let Some(v) = four(value, |x| px(x, fs)) {
                s.padding = v;
            }
        }
        "padding-top" => set_px(&mut s.padding[0], value, fs),
        "padding-right" => set_px(&mut s.padding[1], value, fs),
        "padding-bottom" => set_px(&mut s.padding[2], value, fs),
        "padding-left" => set_px(&mut s.padding[3], value, fs),
        "border" | "border-top" | "border-right" | "border-bottom" | "border-left" => {
            let (w, c, st) = parse_border(value, fs);
            let sides: &[usize] = match name {
                "border" => &[0, 1, 2, 3],
                "border-top" => &[0],
                "border-right" => &[1],
                "border-bottom" => &[2],
                _ => &[3],
            };
            for &i in sides {
                if let Some(b) = s.border.get_mut(i) {
                    *b = w;
                }
                if let Some(bs) = s.border_style.get_mut(i) {
                    *bs = st;
                }
                if let Some(bc) = s.border_color.get_mut(i) {
                    *bc = c.unwrap_or(s.color);
                }
            }
        }
        "border-width" => {
            if let Some(v) = four(value, |x| border_width(x, fs)) {
                s.border = v;
            }
        }
        "border-color" => {
            let cs: Vec<Color> = split_top(value)
                .iter()
                .filter_map(|t| parse_color(t))
                .collect();
            match *cs.as_slice() {
                [a] => s.border_color = [a; 4],
                [a, b] => s.border_color = [a, b, a, b],
                [a, b, c] => s.border_color = [a, b, c, b],
                [a, b, c, d] => s.border_color = [a, b, c, d],
                _ => {}
            }
        }
        "border-style" => {
            if let Some(v) = four(value, LineStyle::parse) {
                s.border_style = v;
            }
        }
        "border-top-style" | "border-right-style" | "border-bottom-style" | "border-left-style" => {
            let i = match name {
                "border-top-style" => 0,
                "border-right-style" => 1,
                "border-bottom-style" => 2,
                _ => 3,
            };
            if let (Some(st), Some(bs)) = (LineStyle::parse(value), s.border_style.get_mut(i)) {
                *bs = st;
            }
        }
        "border-top-width" | "border-right-width" | "border-bottom-width" | "border-left-width" => {
            let i = match name {
                "border-top-width" => 0,
                "border-right-width" => 1,
                "border-bottom-width" => 2,
                _ => 3,
            };
            if let (Some(w), Some(b)) = (border_width(value, fs), s.border.get_mut(i)) {
                *b = w;
            }
        }
        "border-top-color" | "border-right-color" | "border-bottom-color" | "border-left-color" => {
            let i = match name {
                "border-top-color" => 0,
                "border-right-color" => 1,
                "border-bottom-color" => 2,
                _ => 3,
            };
            if let (Some(c), Some(bc)) = (parse_color(value), s.border_color.get_mut(i)) {
                *bc = c;
            }
        }
        "border-radius" => {
            // Elliptical corners (`a / b`) keep their horizontal radii.
            let round = value.split('/').next().unwrap_or(value);
            if let Some(v) = four(round, |x| parse_len(x, fs)) {
                match v[0] {
                    Len::Px(p) if v.iter().all(|l| *l == v[0]) => {
                        s.radius = p;
                        s.corners = None;
                    }
                    _ => s.corners = Some(v),
                }
            }
        }
        "border-top-left-radius"
        | "border-top-right-radius"
        | "border-bottom-right-radius"
        | "border-bottom-left-radius" => {
            let i = match name {
                "border-top-left-radius" => 0,
                "border-top-right-radius" => 1,
                "border-bottom-right-radius" => 2,
                _ => 3,
            };
            if let Some(l) = split_top(value).first().and_then(|v| parse_len(v, fs)) {
                let mut c = s.corners.unwrap_or([Len::Px(s.radius); 4]);
                if let Some(x) = c.get_mut(i) {
                    *x = l;
                }
                s.corners = Some(c);
            }
        }
        "border-collapse" => s.collapse = value == "collapse",
        "table-layout" => s.fixed_layout = value == "fixed",
        "border-spacing" => {
            if let Some(Len::Px(p)) = split_top(value).first().and_then(|v| parse_len(v, fs)) {
                s.spacing = p;
            }
        }
        "vertical-align" => {
            s.valign = match value {
                "top" | "text-top" => VAlign::Top,
                "middle" => VAlign::Middle,
                "bottom" | "text-bottom" => VAlign::Bottom,
                _ => VAlign::Baseline,
            }
        }
        "float" => {
            s.float = match value {
                "left" => Float::Left,
                "right" => Float::Right,
                _ => Float::None,
            }
        }
        "clear" => s.clear = parse_clear(value),
        "position" => {
            s.fixed = value == "fixed";
            s.absolute = value == "absolute";
        }
        "top" | "left" | "right" | "bottom" => {
            if matches!(parse_len(value, fs), Some(Len::Px(p) | Len::Pct(p)) if p < 0.0) {
                s.negative_offset = true;
            }
        }
        "word-break" => {
            // Legacy `word-break: break-word` means overflow-wrap:anywhere.
            s.break_all = value == "break-all";
            if value == "break-word" {
                s.wrap_anywhere = true;
            }
        }
        "overflow-wrap" | "word-wrap" => {
            s.wrap_anywhere = value == "anywhere" || value == "break-word"
        }
        "overflow" | "overflow-y" => s.overflow_hidden = value == "hidden",
        "max-height" => s.max_height = parse_len(value, fs).unwrap_or(Len::Auto),
        "opacity" => {
            let o = value
                .strip_suffix('%')
                .and_then(num)
                .map(|p| p / 100.0)
                .or_else(|| num(value));
            if let Some(o) = o {
                s.opacity = o.clamp(0.0, 1.0);
                s.opacity_zero = o <= 0.01;
            }
        }
        "object-fit" => s.cover = value == "cover",
        "list-style" | "list-style-type" => {
            let toks = split_top(value);
            // `list-style: none` and `list-style: inside none` alike; an
            // image (never fetched) leaves the type alone.
            s.list_none = toks.contains(&"none");
            if let Some(m) = toks.iter().find_map(|t| Marker::parse(t)) {
                s.marker = m;
            }
        }
        _ => {}
    }
}

/// `direction`, which also starts the text from that side (`text-align:
/// start`) until something says otherwise.
fn set_direction(s: &mut Style, v: &str) {
    match v {
        "rtl" => {
            s.rtl = true;
            if s.align == Align::Left {
                s.align = Align::Right;
            }
        }
        "ltr" => {
            s.rtl = false;
            if s.align == Align::Right && s.block_align != Align::Right {
                s.align = Align::Left;
            }
        }
        _ => {}
    }
}

fn set_line_height(s: &mut Style, v: &str) {
    s.line_height = if v == "normal" {
        LineH::Normal
    } else if let Some(m) = num(v) {
        LineH::Mult(m.clamp(0.0, 10.0))
    } else {
        match parse_len(v, s.font_size) {
            Some(Len::Px(p)) => LineH::Px(p.clamp(0.0, 1000.0)),
            Some(Len::Pct(p)) => LineH::Mult((p / 100.0).clamp(0.0, 10.0)),
            _ => LineH::Normal,
        }
    };
}

fn set_len(d: &mut Len, v: &str, fs: f32) {
    if let Some(l) = parse_len(v, fs) {
        *d = l;
    }
}

fn set_px(d: &mut f32, v: &str, fs: f32) {
    if let Some(l) = px(v, fs) {
        *d = l;
    }
}

fn px(v: &str, fs: f32) -> Option<f32> {
    match parse_len(v, fs)? {
        Len::Px(p) => Some(p),
        _ => None,
    }
}

fn four<T: Copy>(v: &str, f: impl Fn(&str) -> Option<T>) -> Option<[T; 4]> {
    let t: Vec<T> = split_top(v).iter().filter_map(|x| f(x)).collect();
    Some(match *t.as_slice() {
        [a] => [a; 4],
        [a, b] => [a, b, a, b],
        [a, b, c] => [a, b, c, b],
        [a, b, c, d] => [a, b, c, d],
        _ => return None,
    })
}

/// A `border-width` value.
fn border_width(v: &str, fs: f32) -> Option<f32> {
    match v {
        "thin" => Some(1.0),
        "medium" => Some(MEDIUM_BORDER),
        "thick" => Some(5.0),
        v => px(v, fs),
    }
}

/// The `border` shorthand: width, colour and style, each with its initial
/// value when left out (so `border: 1px #ccc` draws nothing).
fn parse_border(v: &str, fs: f32) -> (f32, Option<Color>, LineStyle) {
    let mut w = MEDIUM_BORDER;
    let mut c = None;
    let mut st = LineStyle::None;
    for t in split_top(v) {
        if let Some(l) = LineStyle::parse(t) {
            st = l;
        } else if let Some(b) = border_width(t, fs) {
            w = b;
        } else if let Some(col) = parse_color(t) {
            c = Some(col);
        }
    }
    (w, c, st)
}

/// Split on whitespace outside parentheses.
fn split_top(v: &str) -> Vec<&str> {
    let mut out = vec![];
    let mut depth = 0u32;
    let mut start = None;
    for (i, ch) in v.char_indices() {
        match ch {
            '(' => depth = depth.saturating_add(1),
            ')' => depth = depth.saturating_sub(1),
            _ => {}
        }
        if ch.is_whitespace() && depth == 0 {
            if let Some(s) = start.take() {
                out.push(v.get(s..i).unwrap_or(""));
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(s) = start {
        out.push(v.get(s..).unwrap_or(""));
    }
    out
}

/// A CSS font stack with web and Apple fonts mapped to fonts that ship
/// with Windows, so the intended look survives instead of the fallback.
fn clean_family(v: &str) -> String {
    let mut out: Vec<&str> = vec![];
    for f in v.split(',') {
        let f = f.trim().trim_matches(['\'', '"']).trim();
        let mapped = match f.to_ascii_lowercase().as_str() {
            "" => continue,
            "-apple-system" | "blinkmacsystemfont" | "system-ui" | "sf pro text"
            | "sf pro display" | "roboto" | "inter" | "ubuntu" | "open sans" | "lato"
            | "helvetica neue" => "Segoe UI",
            "helvetica" => "Arial",
            "menlo" | "monaco" | "sf mono" | "sfmono-regular" | "ui-monospace" | "courier" => {
                "Consolas"
            }
            "merriweather" | "charter" | "iowan old style" | "palatino" => "Georgia",
            _ => f,
        };
        if !out.contains(&mapped) {
            out.push(mapped);
        }
    }
    out.join(", ")
}

pub fn url_of(v: &str) -> Option<String> {
    let (_, rest) = v.split_once("url(")?;
    let (u, _) = rest.split_once(')')?;
    Some(u.trim().trim_matches(['\'', '"']).to_string())
}

pub fn parse_len(v: &str, fs: f32) -> Option<Len> {
    let v = v.trim();
    if v == "auto" {
        return Some(Len::Auto);
    }
    let split = v
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+'))
        .unwrap_or(v.len());
    let (n, unit) = v.split_at_checked(split)?;
    let n = num(n)?;
    let l = match unit {
        "" | "px" => Len::Px(n),
        "pt" => Len::Px(n * 4.0 / 3.0),
        "em" => Len::Px(n * fs),
        "rem" => Len::Px(n * 16.0),
        "ex" => Len::Px(n * fs * 0.5),
        "%" => Len::Pct(n),
        "in" => Len::Px(n * 96.0),
        "cm" => Len::Px(n * 96.0 / 2.54),
        "mm" => Len::Px(n * 96.0 / 25.4),
        "pc" => Len::Px(n * 16.0),
        "vw" | "vh" => Len::Pct(n),
        _ => return None,
    };
    match l {
        Len::Px(p) | Len::Pct(p) if !p.is_finite() => None,
        l => Some(l),
    }
}

fn parse_hex(h: &str) -> Option<Color> {
    let d: Vec<u8> = h
        .chars()
        .map(|c| c.to_digit(16).and_then(|v| u8::try_from(v).ok()))
        .collect::<Option<_>>()?;
    let x1 = |a: u8| a.wrapping_mul(17);
    let x2 = |a: u8, b: u8| a.wrapping_mul(16).wrapping_add(b);
    Some(match *d.as_slice() {
        [r, g, b] => [x1(r), x1(g), x1(b), 255],
        [r, g, b, a] => [x1(r), x1(g), x1(b), x1(a)],
        [r1, r2, g1, g2, b1, b2] => [x2(r1, r2), x2(g1, g2), x2(b1, b2), 255],
        [r1, r2, g1, g2, b1, b2, a1, a2] => [x2(r1, r2), x2(g1, g2), x2(b1, b2), x2(a1, a2)],
        _ => return None,
    })
}

/// A float in 0..=255 as a channel; NaN becomes 0.
fn channel(x: f32) -> u8 {
    x.round().clamp(0.0, 255.0) as u8
}

pub fn parse_color(v: &str) -> Option<Color> {
    let v = v.trim();
    if let Some(h) = v.strip_prefix('#') {
        return parse_hex(h);
    }
    let args = |inner: &str| -> Vec<String> {
        inner
            .trim_end_matches(')')
            .split(|c: char| c == ',' || c == '/' || c.is_whitespace())
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect()
    };
    let alpha = |p: Option<&String>| -> Option<u8> {
        Some(match p {
            Some(p) => match p.strip_suffix('%') {
                Some(pc) => channel(num(pc)? * 2.55),
                None => channel(num(p)? * 255.0),
            },
            None => 255,
        })
    };
    if let Some(inner) = v.strip_prefix("rgba(").or_else(|| v.strip_prefix("rgb(")) {
        let parts = args(inner);
        let ch = |p: Option<&String>| -> Option<u8> {
            let p = p?;
            Some(match p.strip_suffix('%') {
                Some(pc) => channel(num(pc)? * 2.55),
                None => channel(num(p)?),
            })
        };
        return Some([
            ch(parts.first())?,
            ch(parts.get(1))?,
            ch(parts.get(2))?,
            alpha(parts.get(3))?,
        ]);
    }
    if let Some(inner) = v.strip_prefix("hsla(").or_else(|| v.strip_prefix("hsl(")) {
        let parts = args(inner);
        let n = |i: usize| -> Option<f32> {
            num(parts
                .get(i)?
                .trim_end_matches(['%', '°'])
                .trim_end_matches("deg"))
        };
        let (h, sat, l) = (n(0)?, n(1)? / 100.0, n(2)? / 100.0);
        let k = |x: f32| (x + h / 30.0).rem_euclid(12.0);
        let a = sat * l.min(1.0 - l);
        let f = |x: f32| channel((l - a * (k(x) - 3.0).min(9.0 - k(x)).clamp(-1.0, 1.0)) * 255.0);
        return Some([f(0.0), f(8.0), f(4.0), alpha(parts.get(3))?]);
    }
    named_color(v)
}

fn named_color(n: &str) -> Option<Color> {
    let (r, g, b) = match n {
        "transparent" => return Some([0; 4]),
        "aliceblue" => (240, 248, 255),
        "antiquewhite" => (250, 235, 215),
        "aqua" => (0, 255, 255),
        "aquamarine" => (127, 255, 212),
        "azure" => (240, 255, 255),
        "beige" => (245, 245, 220),
        "bisque" => (255, 228, 196),
        "black" => (0, 0, 0),
        "blanchedalmond" => (255, 235, 205),
        "blue" => (0, 0, 255),
        "blueviolet" => (138, 43, 226),
        "brown" => (165, 42, 42),
        "burlywood" => (222, 184, 135),
        "cadetblue" => (95, 158, 160),
        "chartreuse" => (127, 255, 0),
        "chocolate" => (210, 105, 30),
        "coral" => (255, 127, 80),
        "cornflowerblue" => (100, 149, 237),
        "cornsilk" => (255, 248, 220),
        "crimson" => (220, 20, 60),
        "cyan" => (0, 255, 255),
        "darkblue" => (0, 0, 139),
        "darkcyan" => (0, 139, 139),
        "darkgoldenrod" => (184, 134, 11),
        "darkgray" => (169, 169, 169),
        "darkgreen" => (0, 100, 0),
        "darkgrey" => (169, 169, 169),
        "darkkhaki" => (189, 183, 107),
        "darkmagenta" => (139, 0, 139),
        "darkolivegreen" => (85, 107, 47),
        "darkorange" => (255, 140, 0),
        "darkorchid" => (153, 50, 204),
        "darkred" => (139, 0, 0),
        "darksalmon" => (233, 150, 122),
        "darkseagreen" => (143, 188, 143),
        "darkslateblue" => (72, 61, 139),
        "darkslategray" => (47, 79, 79),
        "darkslategrey" => (47, 79, 79),
        "darkturquoise" => (0, 206, 209),
        "darkviolet" => (148, 0, 211),
        "deeppink" => (255, 20, 147),
        "deepskyblue" => (0, 191, 255),
        "dimgray" => (105, 105, 105),
        "dimgrey" => (105, 105, 105),
        "dodgerblue" => (30, 144, 255),
        "firebrick" => (178, 34, 34),
        "floralwhite" => (255, 250, 240),
        "forestgreen" => (34, 139, 34),
        "fuchsia" => (255, 0, 255),
        "gainsboro" => (220, 220, 220),
        "ghostwhite" => (248, 248, 255),
        "gold" => (255, 215, 0),
        "goldenrod" => (218, 165, 32),
        "gray" => (128, 128, 128),
        "green" => (0, 128, 0),
        "greenyellow" => (173, 255, 47),
        "grey" => (128, 128, 128),
        "honeydew" => (240, 255, 240),
        "hotpink" => (255, 105, 180),
        "indianred" => (205, 92, 92),
        "indigo" => (75, 0, 130),
        "ivory" => (255, 255, 240),
        "khaki" => (240, 230, 140),
        "lavender" => (230, 230, 250),
        "lavenderblush" => (255, 240, 245),
        "lawngreen" => (124, 252, 0),
        "lemonchiffon" => (255, 250, 205),
        "lightblue" => (173, 216, 230),
        "lightcoral" => (240, 128, 128),
        "lightcyan" => (224, 255, 255),
        "lightgoldenrodyellow" => (250, 250, 210),
        "lightgray" => (211, 211, 211),
        "lightgreen" => (144, 238, 144),
        "lightgrey" => (211, 211, 211),
        "lightpink" => (255, 182, 193),
        "lightsalmon" => (255, 160, 122),
        "lightseagreen" => (32, 178, 170),
        "lightskyblue" => (135, 206, 250),
        "lightslategray" => (119, 136, 153),
        "lightslategrey" => (119, 136, 153),
        "lightsteelblue" => (176, 196, 222),
        "lightyellow" => (255, 255, 224),
        "lime" => (0, 255, 0),
        "limegreen" => (50, 205, 50),
        "linen" => (250, 240, 230),
        "magenta" => (255, 0, 255),
        "maroon" => (128, 0, 0),
        "mediumaquamarine" => (102, 205, 170),
        "mediumblue" => (0, 0, 205),
        "mediumorchid" => (186, 85, 211),
        "mediumpurple" => (147, 112, 219),
        "mediumseagreen" => (60, 179, 113),
        "mediumslateblue" => (123, 104, 238),
        "mediumspringgreen" => (0, 250, 154),
        "mediumturquoise" => (72, 209, 204),
        "mediumvioletred" => (199, 21, 133),
        "midnightblue" => (25, 25, 112),
        "mintcream" => (245, 255, 250),
        "mistyrose" => (255, 228, 225),
        "moccasin" => (255, 228, 181),
        "navajowhite" => (255, 222, 173),
        "navy" => (0, 0, 128),
        "oldlace" => (253, 245, 230),
        "olive" => (128, 128, 0),
        "olivedrab" => (107, 142, 35),
        "orange" => (255, 165, 0),
        "orangered" => (255, 69, 0),
        "orchid" => (218, 112, 214),
        "palegoldenrod" => (238, 232, 170),
        "palegreen" => (152, 251, 152),
        "paleturquoise" => (175, 238, 238),
        "palevioletred" => (219, 112, 147),
        "papayawhip" => (255, 239, 213),
        "peachpuff" => (255, 218, 185),
        "peru" => (205, 133, 63),
        "pink" => (255, 192, 203),
        "plum" => (221, 160, 221),
        "powderblue" => (176, 224, 230),
        "purple" => (128, 0, 128),
        "rebeccapurple" => (102, 51, 153),
        "red" => (255, 0, 0),
        "rosybrown" => (188, 143, 143),
        "royalblue" => (65, 105, 225),
        "saddlebrown" => (139, 69, 19),
        "salmon" => (250, 128, 114),
        "sandybrown" => (244, 164, 96),
        "seagreen" => (46, 139, 87),
        "seashell" => (255, 245, 238),
        "sienna" => (160, 82, 45),
        "silver" => (192, 192, 192),
        "skyblue" => (135, 206, 235),
        "slateblue" => (106, 90, 205),
        "slategray" => (112, 128, 144),
        "slategrey" => (112, 128, 144),
        "snow" => (255, 250, 250),
        "springgreen" => (0, 255, 127),
        "steelblue" => (70, 130, 180),
        "tan" => (210, 180, 140),
        "teal" => (0, 128, 128),
        "thistle" => (216, 191, 216),
        "tomato" => (255, 99, 71),
        "turquoise" => (64, 224, 208),
        "violet" => (238, 130, 238),
        "wheat" => (245, 222, 179),
        "white" => (255, 255, 255),
        "whitesmoke" => (245, 245, 245),
        "yellow" => (255, 255, 0),
        "yellowgreen" => (154, 205, 50),
        _ => return None,
    };
    Some([r, g, b, 255])
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn colors_and_lengths() {
        assert_eq!(parse_color("#232f3e"), Some([0x23, 0x2f, 0x3e, 255]));
        assert_eq!(parse_color("#fff"), Some([255; 4]));
        assert_eq!(parse_color("rgba(0,0,0,.5)"), Some([0, 0, 0, 128]));
        assert_eq!(parse_color("rgb(nan, 0, 0)"), None);
        assert_eq!(parse_len("12pt", 16.0), Some(Len::Px(16.0)));
        assert_eq!(parse_len("inf", 16.0), None);
        assert_eq!(parse_len("NaNpx", 16.0), None);
    }

    #[test]
    fn text_transforms() {
        assert_eq!(
            Transform::Capitalize.apply("olá mário-vitó o'neil, 2nd"),
            "Olá Mário-Vitó O'neil, 2nd"
        );
        assert_eq!(Transform::Upper.apply("straße"), "STRASSE");
        assert_eq!(Transform::Lower.apply("ÁB"), "áb");
        assert_eq!(Transform::None.apply("aB"), "aB");
    }

    #[test]
    fn dimming_keeps_body_text_legible() {
        let page = dim_bg(WHITE);
        assert!(oklab(page)[0] < 0.25, "white paper turns dark: {page:?}");
        assert!(oklab(dim_text([0, 0, 0, 255], WHITE, page))[0] > 0.85);
        let texts: [Color; 8] = [
            [0, 0, 0, 255],
            [34, 34, 34, 255],
            [89, 89, 89, 255],
            [0, 0, 238, 255],
            [15, 108, 189, 255],
            [176, 0, 32, 255],
            [27, 122, 27, 255],
            [102, 51, 153, 255],
        ];
        let backgrounds: [Color; 7] = [
            WHITE,
            [244, 246, 248, 255],
            [255, 248, 220, 255],
            [232, 240, 251, 255],
            [255, 235, 238, 255],
            [238, 241, 245, 255],
            [250, 250, 250, 255],
        ];
        for bg in backgrounds {
            for fg in texts {
                let before = contrast(fg, bg);
                let after = contrast(dim_text(fg, bg, dim_bg(bg)), dim_bg(bg));
                assert!(
                    after >= before.min(4.5) - 0.01,
                    "{fg:?} on {bg:?}: {before:.2} -> {after:.2}"
                );
                if before >= 4.5 {
                    assert!(after >= 4.5, "{fg:?} on {bg:?}: {after:.2}");
                }
            }
        }
        // White text on a brand-coloured button stays as it is.
        let button = [15, 108, 189, 255];
        assert_eq!(dim_bg(button), button);
        assert_eq!(dim_text(WHITE, button, button), WHITE);
        // Text hidden in its background stays hidden.
        let hidden = dim_text(WHITE, WHITE, page);
        assert!(contrast(hidden, page) < 1.5);
        // Light dividers stay subtle, dark ones show.
        assert!(contrast(dim_line([221, 221, 221, 255]), page) < 2.0);
        assert!(contrast(dim_line([0, 0, 0, 255]), page) > 4.5);
    }

    #[test]
    fn oklab_round_trips() {
        for c in [
            [0, 0, 0, 255],
            WHITE,
            [15, 108, 189, 255],
            [200, 30, 90, 128],
        ] {
            let back = from_oklab(oklab(c), c[3]);
            for (x, y) in back.iter().zip(&c) {
                assert!(x.abs_diff(*y) <= 1, "{c:?} -> {back:?}");
            }
        }
    }

    #[test]
    fn font_stacks_map_to_windows_fonts() {
        assert_eq!(
            clean_family("-apple-system, BlinkMacSystemFont, 'Helvetica Neue', Helvetica, Arial"),
            "Segoe UI, Arial"
        );
    }
}
