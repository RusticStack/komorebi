//! The appearance the user chose (Settings > Appearance): a preset or
//! their own copy of one, saved as one JSON value under the settings key
//! [`KEY`], read at start before the first frame and applied live through
//! `theme::set_appearance` (the one theme path: tokens, then Kit).
//!
//! A theme file ([`export`], [`import`]) is small, versioned JSON holding
//! colours only:
//!
//! ```json
//! { "format": "komorebi-theme", "version": 1, "name": "Harbour",
//!   "light": { "accent": "#0f6cbd", "canvas": "#f5f5f5", ... },
//!   "dark":  { "accent": "#479ef5", ... } }
//! ```
//!
//! Each variant must name the main four (accent, canvas, paper, ink); a
//! token it leaves out is derived from them (`derive.rs`). Anything else is
//! refused with a plain message ([`ImportError`]), never half-read.

use serde_json::{Map, Value, json};

use super::backdrop::Background;
use super::color::Scheme;
use super::derive;
use super::presets::{ColorSet, Preset, Rgb, TokenId};

/// The settings key.
pub const KEY: &str = "appearance";
/// A theme file's `format`.
pub const FORMAT: &str = "komorebi-theme";
/// The settings value's `format`.
const SETTINGS_FORMAT: &str = "komorebi-appearance";
/// The newest version written and read.
pub const VERSION: u64 = 1;
/// The largest theme file read (a full one is about 1.5 KB).
pub const MAX_FILE: usize = 64 * 1024;
/// The longest name kept.
const MAX_NAME: usize = 64;

/// Which theme draws the app.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ThemeRef {
    Preset(Preset),
    #[default]
    Default,
    Custom,
}

/// One variant of a custom theme: its colours and the tokens the user set
/// by hand (kept when a main colour changes; the others follow it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Variant {
    pub colors: ColorSet,
    pub edited: Vec<TokenId>,
}

impl Variant {
    pub fn of(colors: ColorSet) -> Self {
        Self {
            colors,
            edited: Vec::new(),
        }
    }

    /// Sets one token. A main colour carries the tokens derived from it
    /// along; any other token is marked as set by hand.
    pub fn set(&mut self, id: TokenId, value: Rgb) {
        self.colors.set(id, value);
        if id.is_main() {
            self.colors = derive::rederive(&self.colors, id, &self.edited);
        } else if !self.edited.contains(&id) {
            self.edited.push(id);
        }
    }
}

/// The user's own theme: a copy of a preset, edited.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomTheme {
    pub name: String,
    pub base: Preset,
    pub light: Variant,
    pub dark: Variant,
}

impl CustomTheme {
    /// A copy of `base`, unedited.
    pub fn from_preset(base: Preset) -> Self {
        Self {
            name: base.name().to_string(),
            base,
            light: Variant::of(base.colors(Scheme::Light)),
            dark: Variant::of(base.colors(Scheme::Dark)),
        }
    }

    pub fn variant(&self, scheme: Scheme) -> &Variant {
        match scheme {
            Scheme::Light => &self.light,
            Scheme::Dark => &self.dark,
        }
    }

    pub fn variant_mut(&mut self, scheme: Scheme) -> &mut Variant {
        match scheme {
            Scheme::Light => &mut self.light,
            Scheme::Dark => &mut self.dark,
        }
    }

    /// "Reset to preset": one variant back to its base preset's colours.
    pub fn reset(&mut self, scheme: Scheme) {
        *self.variant_mut(scheme) = Variant::of(self.base.colors(scheme));
    }
}

/// Everything Settings > Appearance chooses.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Appearance {
    pub theme: ThemeRef,
    /// The custom theme, kept while a preset is chosen so it is not lost.
    pub custom: Option<CustomTheme>,
    /// The picture, translucency and window material (all off by default).
    pub background: Background,
}

impl Appearance {
    /// The preset in use, or the custom theme's base.
    pub fn preset(&self) -> Preset {
        match self.theme {
            ThemeRef::Preset(p) => p,
            ThemeRef::Default => Preset::Komorebi,
            ThemeRef::Custom => self.custom.as_ref().map_or(Preset::Komorebi, |c| c.base),
        }
    }

    /// Whether the custom theme draws the app.
    pub fn is_custom(&self) -> bool {
        self.theme == ThemeRef::Custom && self.custom.is_some()
    }

    /// The colours of a scheme.
    pub fn colors(&self, scheme: Scheme) -> ColorSet {
        match (&self.theme, &self.custom) {
            (ThemeRef::Custom, Some(c)) => c.variant(scheme).colors,
            _ => self.preset().colors(scheme),
        }
    }

    /// Whether this is the default look (nothing chosen, or Komorebi with
    /// nothing else).
    pub fn is_default(&self) -> bool {
        matches!(
            self.theme,
            ThemeRef::Default | ThemeRef::Preset(Preset::Komorebi)
        ) && self.background == Background::default()
    }

    /// The settings value.
    pub fn to_setting(&self) -> String {
        let theme = match self.theme {
            ThemeRef::Preset(p) => p.key(),
            ThemeRef::Default => Preset::Komorebi.key(),
            ThemeRef::Custom => "custom",
        };
        let mut v = json!({
            "format": SETTINGS_FORMAT,
            "version": VERSION,
            "theme": theme,
        });
        if let (Some(custom), Some(map)) = (&self.custom, v.as_object_mut()) {
            map.insert("custom".into(), custom_json(custom, true));
        }
        if self.background != Background::default()
            && let Some(map) = v.as_object_mut()
        {
            map.insert("background".into(), self.background.to_json());
        }
        v.to_string()
    }

    /// The settings value back. Anything unreadable is the default look
    /// (a damaged value never keeps the app from starting).
    pub fn from_setting(value: &str) -> Self {
        let Ok(v) = serde_json::from_str::<Value>(value) else {
            return Self::default();
        };
        if v.get("format").and_then(Value::as_str) != Some(SETTINGS_FORMAT)
            || v.get("version")
                .and_then(Value::as_u64)
                .is_none_or(|n| n > VERSION)
        {
            return Self::default();
        }
        let custom = v.get("custom").and_then(|c| read_custom(c, true).ok());
        let theme = match v.get("theme").and_then(Value::as_str) {
            Some("custom") if custom.is_some() => ThemeRef::Custom,
            Some(key) => Preset::from_key(key).map_or(ThemeRef::Default, ThemeRef::Preset),
            None => ThemeRef::Default,
        };
        let background = read_background(v.get("background"));
        Self {
            theme,
            custom,
            background,
        }
    }
}

fn variant_json(v: &Variant, with_edits: bool) -> Value {
    let mut map = Map::new();
    for (id, value) in v.colors.iter() {
        map.insert(id.key().into(), Value::String(value.hex()));
    }
    if with_edits && !v.edited.is_empty() {
        map.insert(
            "edited".into(),
            Value::Array(
                v.edited
                    .iter()
                    .map(|t| Value::String(t.key().into()))
                    .collect(),
            ),
        );
    }
    Value::Object(map)
}

fn custom_json(c: &CustomTheme, settings: bool) -> Value {
    let mut v = json!({
        "name": c.name,
        "base": c.base.key(),
        "light": variant_json(&c.light, settings),
        "dark": variant_json(&c.dark, settings),
    });
    if !settings && let Some(map) = v.as_object_mut() {
        map.insert("format".into(), Value::String(FORMAT.into()));
        map.insert("version".into(), json!(VERSION));
    }
    v
}

/// Why a theme file was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportError {
    /// Larger than [`MAX_FILE`].
    TooLarge,
    /// Not JSON, or not a JSON object.
    NotJson,
    /// JSON, but not a Komorebi theme (no or another `format`).
    NotATheme,
    /// A theme from a newer Komorebi.
    NewerVersion(u64),
    /// A variant is missing, or misses one of the main four.
    Missing { variant: &'static str, key: String },
    /// A colour that is not `#rrggbb`.
    BadColour { variant: &'static str, key: String },
}

fn read_variant(
    v: Option<&Value>,
    name: &'static str,
    base: Preset,
    scheme: Scheme,
    with_edits: bool,
) -> Result<Variant, ImportError> {
    let missing = |key: &str| ImportError::Missing {
        variant: name,
        key: key.to_string(),
    };
    let map = v.and_then(Value::as_object).ok_or_else(|| missing(name))?;
    let mut colors = base.colors(scheme);
    let mut named = Vec::new();
    for (key, value) in map {
        if key == "edited" {
            continue;
        }
        // Unknown keys are left for a newer Komorebi's tokens.
        let Some(id) = TokenId::from_key(key) else {
            continue;
        };
        let bad = || ImportError::BadColour {
            variant: name,
            key: key.clone(),
        };
        let rgb = value.as_str().and_then(Rgb::parse).ok_or_else(bad)?;
        colors.set(id, rgb);
        named.push(id);
    }
    for main in TokenId::MAIN {
        if !named.contains(&main) {
            return Err(missing(main.key()));
        }
    }
    // A token the file leaves out follows its main colours.
    for id in TokenId::ALL {
        if !named.contains(&id) {
            colors.set(id, derive::derived(id, &colors));
        }
    }
    let edited = if with_edits {
        map.get("edited")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|k| k.as_str().and_then(TokenId::from_key))
                    .filter(|t| !t.is_main())
                    .collect()
            })
            .unwrap_or_default()
    } else {
        // Every token a file names is the author's choice.
        named.into_iter().filter(|t| !t.is_main()).collect()
    };
    Ok(Variant { colors, edited })
}

fn read_custom(v: &Value, settings: bool) -> Result<CustomTheme, ImportError> {
    let base = v
        .get("base")
        .and_then(Value::as_str)
        .and_then(Preset::from_key)
        .unwrap_or_default();
    let name: String = v
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or(base.name())
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME)
        .collect();
    Ok(CustomTheme {
        name,
        base,
        light: read_variant(v.get("light"), "light", base, Scheme::Light, settings)?,
        dark: read_variant(v.get("dark"), "dark", base, Scheme::Dark, settings)?,
    })
}

/// Background settings from a settings value or a theme file: lenient
/// (what is unreadable is the default) and in range.
fn read_background(v: Option<&Value>) -> Background {
    v.map(Background::from_json).unwrap_or_default()
}

/// A theme file of `theme` (colours only).
pub fn export(theme: &CustomTheme) -> String {
    export_with(theme, None)
}

/// A theme file of `theme`, with the background settings when asked (never
/// the picture itself: a file that names one is only told there was one).
pub fn export_with(theme: &CustomTheme, background: Option<&Background>) -> String {
    let mut v = custom_json(theme, false);
    if let (Some(bg), Some(map)) = (background, v.as_object_mut()) {
        let bg = Background {
            picture: false,
            ..bg.clone()
        };
        map.insert("background".into(), bg.to_json());
    }
    serde_json::to_string_pretty(&v).unwrap_or_default()
}

/// The background settings a theme file carries, if any (never a
/// picture: the one chosen on this PC is kept).
pub fn import_background(text: &str) -> Option<Background> {
    if text.len() > MAX_FILE {
        return None;
    }
    let v: Value = serde_json::from_str(text).ok()?;
    let bg = v.get("background")?;
    Some(Background {
        picture: false,
        ..read_background(Some(bg))
    })
}

/// A theme file read back, or why it was refused.
pub fn import(text: &str) -> Result<CustomTheme, ImportError> {
    if text.len() > MAX_FILE {
        return Err(ImportError::TooLarge);
    }
    let v: Value = serde_json::from_str(text).map_err(|_| ImportError::NotJson)?;
    if !v.is_object() {
        return Err(ImportError::NotJson);
    }
    if v.get("format").and_then(Value::as_str) != Some(FORMAT) {
        return Err(ImportError::NotATheme);
    }
    match v.get("version").and_then(Value::as_u64) {
        Some(n) if n > VERSION => return Err(ImportError::NewerVersion(n)),
        Some(_) => {}
        None => return Err(ImportError::NotATheme),
    }
    read_custom(&v, false)
}

/// A snapshot's appearance (`KOMOREBI_SNAPSHOT_APPEARANCE`): a preset's
/// key, or a theme file made the custom theme.
pub fn for_snapshot(asked: &str) -> Result<Appearance, String> {
    if let Some(p) = Preset::from_key(asked) {
        return Ok(Appearance {
            theme: ThemeRef::Preset(p),
            ..Appearance::default()
        });
    }
    let text =
        std::fs::read_to_string(asked).map_err(|e| format!("reading the theme file: {e}"))?;
    let custom = import(&text).map_err(|e| format!("not a theme file: {e:?}"))?;
    Ok(Appearance {
        theme: ThemeRef::Custom,
        custom: Some(custom),
        background: Background::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edited() -> CustomTheme {
        let mut c = CustomTheme::from_preset(Preset::Outlook);
        c.name = "Harbour".into();
        c.light.set(TokenId::Accent, Rgb(0x8A3FFC));
        c.light.set(TokenId::Ink3, Rgb(0x505050));
        c.dark.set(TokenId::Canvas, Rgb(0x101820));
        c
    }

    #[test]
    fn the_settings_value_round_trips() {
        let a = Appearance::default();
        assert_eq!(
            Appearance::from_setting(&a.to_setting()).colors(Scheme::Light),
            a.colors(Scheme::Light)
        );
        for theme in [
            ThemeRef::Preset(Preset::Outlook),
            ThemeRef::Preset(Preset::GitHub),
            ThemeRef::Custom,
        ] {
            let a = Appearance {
                theme,
                custom: Some(edited()),
                background: Background::default(),
            };
            let back = Appearance::from_setting(&a.to_setting());
            assert_eq!(back, a, "{theme:?}");
        }
    }

    #[test]
    fn a_damaged_setting_is_the_default_look() {
        for bad in [
            "",
            "{",
            "[]",
            r#"{"format":"x","version":1}"#,
            r#"{"format":"komorebi-appearance","version":99,"theme":"github"}"#,
            r#"{"format":"komorebi-appearance","version":1,"theme":"custom"}"#,
            r#"{"format":"komorebi-appearance","version":1,"theme":"purple"}"#,
        ] {
            let a = Appearance::from_setting(bad);
            assert!(a.is_default(), "{bad}");
            assert_eq!(
                a.colors(Scheme::Dark),
                Preset::Komorebi.colors(Scheme::Dark)
            );
        }
    }

    #[test]
    fn a_theme_file_round_trips() {
        let theme = edited();
        let file = export(&theme);
        assert!(file.len() < 2048, "{} bytes", file.len());
        let back = import(&file).expect("its own file");
        assert_eq!(back.name, "Harbour");
        assert_eq!(back.light.colors, theme.light.colors);
        assert_eq!(back.dark.colors, theme.dark.colors);
        assert!(file.contains("\"format\": \"komorebi-theme\""));
    }

    #[test]
    fn a_file_with_only_the_main_four_derives_the_rest() {
        let file = r##"{"format":"komorebi-theme","version":1,"name":"Moss",
            "light":{"accent":"#2e7d32","canvas":"#f3f5f0","paper":"#ffffff","ink":"#1b1f1a"},
            "dark":{"accent":"#81c784","canvas":"#0f130e","paper":"#171c16","ink":"#eef2ec"}}"##;
        let t = import(file).expect("valid");
        for scheme in [Scheme::Light, Scheme::Dark] {
            let c = t.variant(scheme).colors;
            assert!(
                derive::check(&c).is_empty(),
                "{scheme:?}: {:?}",
                derive::check(&c)
            );
        }
        assert_eq!(t.light.colors.get(TokenId::Ring), Rgb(0x2E7D32));
    }

    #[test]
    fn malformed_files_are_refused_plainly() {
        let ok_variant =
            r##"{"accent":"#000001","canvas":"#eeeeee","paper":"#ffffff","ink":"#111111"}"##;
        let file = |body: &str| format!(r#"{{"format":"komorebi-theme","version":1,{body}}}"#);
        assert_eq!(import("not json"), Err(ImportError::NotJson));
        assert_eq!(import("[1,2]"), Err(ImportError::NotJson));
        assert_eq!(import(r#"{"light":{}}"#), Err(ImportError::NotATheme));
        assert_eq!(
            import(r#"{"format":"komorebi-theme","version":7}"#),
            Err(ImportError::NewerVersion(7))
        );
        assert_eq!(
            import(&file(&format!(r#""light":{ok_variant}"#))),
            Err(ImportError::Missing {
                variant: "dark",
                key: "dark".into()
            })
        );
        assert_eq!(
            import(&file(&format!(
                r##""light":{ok_variant},"dark":{{"accent":"#000001","canvas":"#eeeeee","paper":"#ffffff"}}"##
            ))),
            Err(ImportError::Missing {
                variant: "dark",
                key: "ink".into()
            })
        );
        assert_eq!(
            import(&file(&format!(
                r##""light":{ok_variant},"dark":{{"accent":"blue","canvas":"#eeeeee","paper":"#ffffff","ink":"#111111"}}"##
            ))),
            Err(ImportError::BadColour {
                variant: "dark",
                key: "accent".into()
            })
        );
        assert_eq!(
            import(&" ".repeat(MAX_FILE + 1)),
            Err(ImportError::TooLarge)
        );
    }

    #[test]
    fn setting_a_main_colour_carries_its_derived_tokens() {
        let mut v = Variant::of(Preset::Komorebi.colors(Scheme::Light));
        v.set(TokenId::Accent, Rgb(0x0F6CBD));
        assert_eq!(v.colors.get(TokenId::Ring), Rgb(0x0F6CBD));
        assert!(v.edited.is_empty());
        v.set(TokenId::Ring, Rgb(0x222222));
        v.set(TokenId::Accent, Rgb(0xC50F1F));
        assert_eq!(
            v.colors.get(TokenId::Ring),
            Rgb(0x222222),
            "set by hand: kept"
        );
        assert_eq!(v.edited, vec![TokenId::Ring]);
    }

    #[test]
    fn background_settings_are_saved_and_optionally_exported() {
        use super::super::backdrop::{Fit, Material};
        let bg = Background {
            picture: true,
            fit: Fit::Centre,
            dim: 25,
            blur: 20,
            panes: 70,
            sidebar: 60,
            pane_blur: 20,
            material: Material::Mica,
            tint: 50,
            ..Background::default()
        };
        let a = Appearance {
            theme: ThemeRef::Preset(Preset::Komorebi),
            background: bg.clone(),
            ..Appearance::default()
        };
        assert!(!a.is_default());
        assert_eq!(Appearance::from_setting(&a.to_setting()), a);
        // Out of range in a settings value: brought into range.
        let wild = a
            .to_setting()
            .replace("\"panes\":70", "\"panes\":3")
            .replace("\"blur\":20", "\"blur\":200");
        let back = Appearance::from_setting(&wild).background;
        assert_eq!((back.panes, back.blur), (40, 40));
        // A theme file has no background unless asked, and never the
        // picture.
        let theme = edited();
        assert_eq!(import_background(&export(&theme)), None);
        let file = export_with(&theme, Some(&bg));
        assert_eq!(import(&file).unwrap(), import(&export(&theme)).unwrap());
        let got = import_background(&file).unwrap();
        assert!(!got.picture);
        assert_eq!(
            got,
            Background {
                picture: false,
                ..bg
            }
        );
    }
}
