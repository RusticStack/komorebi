# Branding assets

Komorebi 1.0's brand is the hanko K seal: a soft square in persimmon #B3503A with the K carved
through it, the only square and the only colour. The rules (colour, shape, type, voice) are in
the approved brand book (komorebi-brand `final/BRAND.md`); this page lists every asset file in
this repository, where it comes from and where it is used.

The seal replaced 0.7's icon (the sun over an envelope) at the cut-over: the former
`assets/brand-1.0/` files moved up into `assets/` (`komorebi.ico`, `komorebi.rc`, `tray/`, and
the master SVGs as `assets/brand-src/`), and the installer has only the 1.0 branch
([GPUI-MIGRATION.md](GPUI-MIGRATION.md) "Cut-over").

## Sources and how to rebuild

| Path | What |
|---|---|
| `assets/brand-src/*.svg` | The brand's master SVGs, copied unchanged from komorebi-brand `final/logo`: `app-icon.svg` (the icon seal, 224 of 256), `app-icon-16.svg` and `app-icon-24.svg` (drawn by hand on the pixel grid), `tray-16-light.svg`, `tray-16-dark.svg`, `tray-16-unread.svg` (the 16 px tray glyph in ink #1A1A1A, white and persimmon), `mark.svg`, `mark-mono.svg`, `fall.svg`, `wordmark.svg`, `wordmark-dark.svg`, `logo.svg`, `logo-dark.svg`. |
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
| `komorebi.ico` | 16, 20, 24, 32, 40, 48, 64, 128 (32-bit BMP) and 256 (PNG) | 16 and 24 from the hand-drawn pixel files; the others from `app-icon.svg`, with the seal's outer edge snapped to whole pixels (20 px has a 1 px inset, 40 px 2 px) | Resource 1 of `komorebi.exe`: Explorer, the window and the taskbar (GPUI's window class loads icon 1), Start and the desktop shortcut, Settings > Apps (`UninstallDisplayIcon` is the exe), the mail client and `mailto:` registrations (`komorebi.exe,0`), the large image of notifications; the setup's own icon. |
| `tray/tray-light.ico` | 16, 20, 24, 32 | 16 from `tray-16-light.svg`; 24 from `app-icon-24.svg` and 20, 32 from `app-icon.svg`, recoloured to #1A1A1A | Resource 101: the tray seal on a light taskbar. |
| `tray/tray-dark.ico` | 16, 20, 24, 32 | the same in white #FFFFFF | Resource 102: the tray seal on a dark taskbar. |
| `tray/tray-unread.ico` | 16, 20, 24, 32 | the same in persimmon | Resource 103: the tray seal while unread mail waits, on either taskbar. |
| `komorebi.rc` | | | The exe's resources: icon 1, icons 101 to 103, the manifest (`komorebi.manifest`) and the version information. Compiled by `crates/komorebi-gpui/build.rs`. |

The notification area asks for the system small icon size at the system DPI: 16 px at 100
percent, 20 at 125, 24 at 150, 32 at 200. The shell code loads the resource at exactly that
size (`LoadImageW` with `SM_CXSMICON`), so Windows picks the matching image instead of scaling.

**How the tray picks its icon** (`src/win_shell.rs`, shared library code):

- An exe that carries resource 101 (`komorebi.exe`) uses the brand tray icons. An exe
  without it (the library's tests and examples) shows its own icon 1 in every state.
- Unread mail waiting (`Shell::set_unread(true)`): persimmon (103), on both taskbars (4.6:1 on the
  light taskbar, 3.2:1 on the dark one).
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
image is the persimmon seal whatever the tray glyph is at that moment. (0.7 used
`NIIF_NONE`.)

## Installer (`installer/`)

| File | Size | Used by |
|---|---|---|
| `wizard-image.bmp` | 164 x 314, 24-bit | `WizardImageFile` (welcome and finish pages) at 100 percent |
| `wizard-image-2x.bmp` | 328 x 628 | the same at 200 percent; Setup picks the closer one for the DPI |
| `wizard-small.bmp` | 55 x 58 | `WizardSmallImageFile` (the other pages' header) at 100 percent |
| `wizard-small-2x.bmp` | 110 x 116 | the same at 200 percent |

Paper ground (#FFFFFF), the seal, and on the large image the ink wordmark under it. No gradient,
no shadow. `installer/komorebi.iss` uses them, with `assets/komorebi.ico` as the setup's icon.

## GPUI view assets (`assets/brand/`)

Small files the GPUI views load through their `AssetSource` (phases 1, 6 and 7). `index.json`
lists each one with its use. Total about 55 KB (the test keeps it under 150 KB).

| File | Use |
|---|---|
| `mark.svg` | The seal in persimmon: welcome (40 px), About (72 px), the 2 s "Sent" notice (18 px). Never in text. |
| `mark-mono.svg` | The seal in `currentColor`: the ink mark in the title bar (GPUI's `svg()` recolours it with the text colour). |
| `fall.svg` | The K's light, the one large decorative shape: welcome screen only, in the `fall` tone (#F1F1EE light, #202020 dark); never persimmon or ink, never behind reading text. `currentColor`, cropped to the K. |
| `wordmark.svg`, `wordmark-dark.svg` | The drawn lowercase wordmark in ink (#171717) and for dark grounds (#F2F2F0). |
| `logo.svg`, `logo-dark.svg` | Seal and wordmark lockup, light and dark (About). |
| `png/*.png`, `png/*@2x.png` | The same at 1x and 2x for `img()`: `mark-18`, `mark-40`, `mark-72`, `wordmark-24`, `wordmark-dark-24` (24 px high), `logo-40`, `logo-dark-40` (40 px high), `fall-light`, `fall-dark` (325 x 360). |

The seal never sits on dark wash; in dark mode it goes on paper, canvas or the ink Sent pill.
At most one persimmon element is visible in the app at a time.

## Repository images (`.github/assets/`)

| File | What |
|---|---|
| `logo.png` | The lockup (seal and ink wordmark), 640 x 135, transparent: the README header on a light page. |
| `logo-light.png` | The lockup with the light wordmark (#F2F2F0): the README header on a dark page. |
| `social-preview.webp`, `social-preview-dark.webp` | 1280 x 640 social card, light (paper) and dark (#141414): the seal, the wordmark, "Read. Reply. Rest." and "Free for Windows 10 and 11", as the film's end card. Upload in the repository's Settings > Social preview. |
| `screenshots/` | Not brand assets: screenshots of the app, replaced when the GPUI app is the shipping one. |
