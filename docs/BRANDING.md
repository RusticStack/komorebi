# Branding assets

Komorebi's brand is the hanko K seal: a soft square with the K in it, the only square. Komorebi
has **no accent colour**: the seal is neutral everywhere, in the app and outside it, and the
one hue the app shows is Windows' own red on the Close button's hover. The rules for shape, type
and voice are in the approved brand book (komorebi-brand `final/BRAND.md`); this page lists every
asset file in this repository, where it comes from and where it is used.

## Neutral brand (after 1.0, a deviation from the brand book)

The brand book draws the seal in persimmon #B3503A, the brand's one colour. After 1.0.0 the
owner decided that Komorebi is neutral instead: first the Windows icons (1.0.1's app icon and
tray), then every other place the persimmon seal appeared. The deviation is complete: nothing
this repository builds or shows is persimmon. The brand folder (komorebi-brand `final/`) is the
source of record and is not changed; the master SVGs in `assets/brand-src/` are still its
unchanged copies (so they are still persimmon), and `scripts/brand-assets.js` applies the
neutral treatment to every output (`neutral()`, `sealAt()`), failing if any persimmon is left.

There are two neutral forms of the seal, one for pictures and one for the app's views:

- **The neutral seal**, for pictures of the brand (the app icon, the toast image, the installer,
  `assets/brand/`, the README logo and the social preview): the seal in **ink-3 #6A6A66** with
  the **K in white #FFFFFF**, drawn on it instead of carved through. It reads the same on light
  and dark grounds.
- **The ink mark**, inside the app (`crates/komorebi-gpui/assets/brand/seal.svg`, the K carved
  through, drawn in the text colour): the title bar, the welcome screen and the onboarding's
  last step in **ink** (#171717 light, #F2F2F0 dark), and the 2 s "Sent" notice in **on-ink**
  on its ink pill (white on the light scheme's ink pill, #141414 on the dark one's). The app's
  palette has no `seal` token any more; a test (`the_palette_has_no_accent_colour`) keeps every
  colour but Close's red a grey. The About page shows no seal.

The details of each form:

- **The app icon** (`komorebi.ico`: Explorer, the window, the taskbar, Start, the toast's large
  image) and **the installer images**: the seal in **ink-3 #6A6A66** (BRAND.md's neutral
  ink-3) with the **K in white #FFFFFF** (on-ink), drawn on the seal instead of carved through.
  One icon has to read on a light and a dark taskbar: ink-3 is 4.7 to 4.9:1 on the light
  taskbars (#EEEEEE, #F3F3F3) and 3.0 to 3.1:1 on the dark ones (#202020, #1C1C1C), so the
  seal's outline passes the 3:1 non-text rule on both, and the white K is 5.4:1 on it. Ink
  #171717 was rejected (1.1:1 on a dark taskbar: only a floating K would show), and so was
  ink-2 #555552 (2.2:1 on a dark taskbar). The 16 and 24 px pixel files keep their hand-drawn
  pixels; their carved pixels are filled with white (`fillCarving`).
- **The tray**: unchanged in ink #1A1A1A on a light taskbar and white on a dark one. Unread mail
  no longer turns it persimmon: it shows BRAND.md's unread spot, a dot in the glyph's own
  colour in the top right corner with a gap cut round it (`withSpot`), so the unread state is
  two icons, one per taskbar (`tray-unread-light.ico`, `tray-unread-dark.ico`, resources 103
  and 104).
- **In the app**: the welcome screen and the onboarding's last step draw the ink mark (40 px)
  where they had the persimmon seal, and the "Sent" notice the on-ink mark (18 px); the title
  bar already had the ink mark.
- **Outside the app**: `assets/brand/` (`mark.svg`, `logo.svg`, `logo-dark.svg` and their
  PNGs), the README logo (`.github/assets/logo.png`, `logo-light.png`) and the social previews
  use the neutral seal, and the welcome screenshots (`.github/assets/screenshots/welcome-*.webp`)
  were taken again. `tray-16-unread.svg` stays in `assets/brand-src/` as the brand's copy but is
  no longer built.

The seal replaced 0.7's icon (the sun over an envelope) at the cut-over: the former
`assets/brand-1.0/` files moved up into `assets/` (`komorebi.ico`, `komorebi.rc`, `tray/`, and
the master SVGs as `assets/brand-src/`), and the installer has only the 1.0 branch
([GPUI-MIGRATION.md](GPUI-MIGRATION.md) "Cut-over").

## Sources and how to rebuild

| Path | What |
|---|---|
| `assets/brand-src/*.svg` | The brand's master SVGs, copied unchanged from komorebi-brand `final/logo` (in the brand's persimmon; never shipped as they are): `app-icon.svg` (the icon seal, 224 of 256), `app-icon-16.svg` and `app-icon-24.svg` (drawn by hand on the pixel grid), `tray-16-light.svg`, `tray-16-dark.svg`, `tray-16-unread.svg` (the 16 px tray glyph in ink #1A1A1A, white and persimmon), `mark.svg`, `mark-mono.svg`, `fall.svg`, `wordmark.svg`, `wordmark-dark.svg`, `logo.svg`, `logo-dark.svg`. |
| `scripts/brand-assets.js` | Builds every file below from those sources. Rendering is resvg; the ICO and BMP containers are written by the script itself. |

```powershell
npm install --prefix C:\dev-overflow\komorebi-brand-tools @resvg/resvg-js@2.6.2 sharp@0.33.5
$env:NODE_PATH = 'C:\dev-overflow\komorebi-brand-tools\node_modules'
node scripts/brand-assets.js
cargo test -p komorebi-gpui --test brand_assets
```

`crates/komorebi-gpui/tests/brand_assets.rs` checks the outputs: every ICO's sizes and formats,
each image's own header, a load of every size through `LoadImageW`, the installer image sizes,
and that `assets/brand` lists only files that exist and stays under 150 KB.

## Windows icons (`assets/`)

| File | Sizes | Made from | Used by |
|---|---|---|---|
| `komorebi.ico` | 16, 20, 24, 32, 40, 48, 64, 128 (32-bit BMP) and 256 (PNG) | The neutral seal (ink-3, white K; "Neutral icon" above). 16 and 24 from the hand-drawn pixel files; the others from `app-icon.svg`, with the seal's outer edge snapped to whole pixels (20 px has a 1 px inset, 40 px 2 px) | Resource 1 of `komorebi.exe`: Explorer, the window and the taskbar (GPUI's window class loads icon 1), Start and the desktop shortcut, Settings > Apps (`UninstallDisplayIcon` is the exe), the mail client and `mailto:` registrations (`komorebi.exe,0`), the large image of notifications; the setup's own icon. |
| `tray/tray-light.ico` | 16, 20, 24, 32 | 16 from `tray-16-light.svg`; 24 from `app-icon-24.svg` and 20, 32 from `app-icon.svg`, recoloured to #1A1A1A | Resource 101: the tray seal on a light taskbar. |
| `tray/tray-dark.ico` | 16, 20, 24, 32 | the same in white #FFFFFF | Resource 102: the tray seal on a dark taskbar. |
| `tray/tray-unread-light.ico` | 16, 20, 24, 32 | `tray-light.ico`'s glyphs with the unread spot | Resource 103: unread mail waits, light taskbar. |
| `tray/tray-unread-dark.ico` | 16, 20, 24, 32 | `tray-dark.ico`'s glyphs with the unread spot | Resource 104: unread mail waits, dark taskbar. |
| `komorebi.rc` | | | The exe's resources: icon 1, icons 101 to 104, the manifest (`komorebi.manifest`) and the version information. Compiled by `crates/komorebi-gpui/build.rs`. |

The notification area asks for the system small icon size at the system DPI: 16 px at 100
percent, 20 at 125, 24 at 150, 32 at 200. The shell code loads the resource at exactly that
size (`LoadImageW` with `SM_CXSMICON`), so Windows picks the matching image instead of scaling.

**How the tray picks its icon** (`src/win_shell.rs`, shared library code):

- An exe that carries resource 101 (`komorebi.exe`) uses the brand tray icons. An exe
  without it (the library's tests and examples) shows its own icon 1 in every state.
- Unread mail waiting (`Shell::set_unread(true)`): the same glyph with the unread spot, 103 on a
  light taskbar and 104 on a dark one (until 1.0.0 it was one persimmon icon, 103, on both).
- Otherwise the taskbar theme decides, not the app's theme: `HKCU\Software\Microsoft\Windows\
  CurrentVersion\Themes\Personalize\SystemUsesLightTheme` = 1 is a light taskbar (ink seal, 101);
  0 or missing is a dark one (white seal, 102). Windows 10 before 1903 has no value and a dark
  taskbar. The crossed pairs are 1.1:1, which is why the theme must decide.
- The shell window hears `WM_SETTINGCHANGE` with the area `ImmersiveColorSet` when the mode
  changes, re-reads the value and swaps the icon. After an Explorer restart (`TaskbarCreated`)
  the icon is added again in the current state.

**Notifications.** Komorebi's notifications are the tray icon's `NIF_INFO` notifications, which
Windows 10 and 11 show as native toasts. Windows attributes them to the process that owns the
tray icon, by its exe (FileDescription "Komorebi" and icon 1, the seal in `komorebi.exe`).
No AppUserModelID or shortcut is involved, so the installer needs nothing for it. With the brand icons, `show_toast` also passes
icon 1 at the large icon size as `hBalloonIcon` with `NIIF_USER | NIIF_LARGE_ICON`, so the toast's
image is the neutral seal whatever the tray glyph is at that moment. (0.7 used
`NIIF_NONE`.)

## Installer (`installer/`)

| File | Size | Used by |
|---|---|---|
| `wizard-image.bmp` | 164 x 314, 24-bit | `WizardImageFile` (welcome and finish pages) at 100 percent |
| `wizard-image-2x.bmp` | 328 x 628 | the same at 200 percent; Setup picks the closer one for the DPI |
| `wizard-small.bmp` | 55 x 58 | `WizardSmallImageFile` (the other pages' header) at 100 percent |
| `wizard-small-2x.bmp` | 110 x 116 | the same at 200 percent |

Paper ground (#FFFFFF), the neutral seal (ink-3, white K), and on the large image the ink wordmark under it. No gradient,
no shadow. `installer/komorebi.iss` uses them, with `assets/komorebi.ico` as the setup's icon.

## GPUI view assets (`assets/brand/`)

Small files the GPUI views load through their `AssetSource` (phases 1, 6 and 7). `index.json`
lists each one with its use. Total about 55 KB (the test keeps it under 150 KB).

| File | Use |
|---|---|
| `mark.svg` | The neutral seal as a picture (ink-3, white K). Never in text. |
| `mark-mono.svg` | The seal in `currentColor`: the ink mark (GPUI's `svg()` recolours it with the text colour). The app draws its own copy, `crates/komorebi-gpui/assets/brand/seal.svg`, in the title bar, on the welcome screen, on the onboarding's last step and in the "Sent" notice. |
| `fall.svg` | The K's light, the one large decorative shape: welcome screen only, in the `fall` tone (#F1F1EE light, #202020 dark); never ink, never behind reading text. `currentColor`, cropped to the K. |
| `wordmark.svg`, `wordmark-dark.svg` | The drawn lowercase wordmark in ink (#171717) and for dark grounds (#F2F2F0). |
| `logo.svg`, `logo-dark.svg` | The neutral seal and wordmark lockup, light and dark. |
| `png/*.png`, `png/*@2x.png` | The same at 1x and 2x for `img()`: `mark-18`, `mark-40`, `mark-72`, `wordmark-24`, `wordmark-dark-24` (24 px high), `logo-40`, `logo-dark-40` (40 px high), `fall-light`, `fall-dark` (325 x 360). |

The seal never sits on dark wash; in dark mode it goes on paper, canvas or the ink Sent pill.
No element in the app is persimmon, or any other accent colour.

## Repository images (`.github/assets/`)

| File | What |
|---|---|
| `logo.png` | The lockup (the neutral seal and the ink wordmark), 640 x 135, transparent: the README header on a light page. |
| `logo-light.png` | The lockup with the light wordmark (#F2F2F0): the README header on a dark page. |
| `social-preview.webp`, `social-preview-dark.webp` | 1280 x 640 social card, light (paper) and dark (#141414): the neutral seal, the wordmark, "Read. Reply. Rest." and "Free for Windows 10 and 11", as the film's end card. Upload in the repository's Settings > Social preview. |
| `screenshots/` | Not brand assets: screenshots of the app, replaced when the GPUI app is the shipping one. |
