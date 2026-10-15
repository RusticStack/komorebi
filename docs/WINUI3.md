# WinUI 3 / Windows 11 reference for Komorebi

The binding reference for making Komorebi look and behave like an app that
ships with Windows 11: WinUI 3's theme resources, control metrics, and how a
WinUI 3 app follows the user's Windows theme. It complements
`docs/FLUENT2.md` (principles, Fluent 2 tokens, motion, wait UX); it does not
repeat it. **Where the two disagree, this file wins on Windows.** Compiled
on 2026-10-09 from:

- microsoft/microsoft-ui-xaml @ `4f685bc779bfbbaf7c01d9ec7e114565c4e2c768`:
  `controls/dev/CommonStyles/Common_themeresources_any.xaml` (all colours),
  `CornerRadius_`, `TextBlock_` and the per-control `*_themeresources.xaml`,
  `controls/dev/{ComboBox,NavigationView,InfoBar,ProgressRing,ProgressBar,PersonPicture,TitleBar,InfoBadge}/`,
  `Materials/Acrylic/AcrylicBrush_themeresources.xaml`,
  `dxaml/xcp/dxaml/themes/generic.xaml`,
  `dxaml/xcp/core/core/elements/ListViewBaseItemChrome.cpp`,
  `dxaml/xcp/components/theminginterop/SystemThemingInterop.cpp`,
  `dxaml/xcp/components/theming/FrameworkTheming.cpp`,
  `dxaml/xcp/core/text/common/TextFormatting.cpp`,
  `dxaml/xcp/dxaml/lib/JupiterWindow.cpp`.
- MicrosoftDocs/windows-dev-docs @ `aabd22a9113af71e56bd6f5af8fde06df68162e0`
  (learn.microsoft.com): `hub/apps/design/signature-experiences/*`,
  `style/{mica,acrylic}.md`, `iconography/*`,
  `accessibility/high-contrast-themes.md`, `develop/ui/theming.md`,
  `develop/platform/xaml/xaml-theme-resources.md`, `design/input/text-scaling`,
  `design/style/spacing`.
- Slint 1.18.1, winit 0.30.13, softbuffer 0.4.8 and windows-sys 0.61.2
  sources, and a windows-sys probe run on Windows 11 build 26300.

Tags: **[source]** read in Microsoft or crate source; **[verified]** measured
on a machine; **[inferred]** follows from source but the final value was not
read; **[unverified]** still to be checked. Untagged values are [source].
Sizes are effective pixels (epx), which Slint calls logical `px`. Colours are
`#AARRGGBB` as in XAML; six digits are opaque. "Dark" is XAML's `Default`
dictionary.

## Theme resources

### High contrast

Every `<Color>` in WinUI's HighContrast dictionary is a `#FF0000`
placeholder: never copy it. In HC each brush points at a `SystemColor*`,
which is Win32 `GetSysColor(index)` (COLORREF `0x00BBGGRR`):

| HC name | Index | Use (high-contrast-themes.md) |
|---|---|---|
| Window | `COLOR_WINDOW` 5 | Page, pane and card backgrounds |
| WindowText | `COLOR_WINDOWTEXT` 8 | Body text, icons, borders, dividers |
| Hotlight | `COLOR_HOTLIGHT` 26 | Hyperlinks only, on Window |
| GrayText | `COLOR_GRAYTEXT` 17 | Disabled only, never secondary text |
| Highlight / HighlightText | 13 / 14 | Selected, hovered, pressed, in-progress UI (text in HighlightText on Highlight) |
| ButtonFace / ButtonText | 15 / 18 | Interactive controls (text and border ButtonText on ButtonFace) |

HC has no translucency and no shadows; fills are equal, so every control
keeps its 1 px stroke.

### Text

| Resource | Light | Dark | HC |
|---|---|---|---|
| TextFillColorPrimary | #E4000000 | #FFFFFF | WindowText |
| TextFillColorSecondary | #9E000000 | #C5FFFFFF | WindowText |
| TextFillColorTertiary | #72000000 | #87FFFFFF | WindowText |
| TextFillColorDisabled | #5C000000 | #5DFFFFFF | GrayText |
| TextFillColorInverse | #FFFFFF | #E4000000 | WindowText |
| AccentTextFillColorPrimary / Secondary / Tertiary | Dark2 / Dark3 / Dark1 | Light3 / Light3 / Light2 | WindowText |
| AccentTextFillColorDisabled | #5C000000 | #5DFFFFFF | GrayText |
| TextOnAccentFillColorPrimary | #FFFFFF | **#000000** | WindowText |
| TextOnAccentFillColorSecondary | #B3FFFFFF | #80000000 | WindowText |
| TextOnAccentFillColorDisabled | #FFFFFF | #87FFFFFF | GrayText |
| TextOnAccentFillColorSelectedText | #FFFFFF | #FFFFFF | WindowText |

### Fills

| Resource | Light | Dark | HC |
|---|---|---|---|
| ControlFillColorDefault | #B3FFFFFF | #0FFFFFFF | ButtonFace |
| ControlFillColorSecondary (hover) | #80F9F9F9 | #15FFFFFF | ButtonFace |
| ControlFillColorTertiary (pressed) | #4DF9F9F9 | #08FFFFFF | ButtonFace |
| ControlFillColorDisabled | #4DF9F9F9 | #0BFFFFFF | ButtonFace |
| ControlFillColorInputActive (focused input) | #FFFFFF | #B31E1E1E | ButtonFace |
| ControlStrongFillColorDefault | #72000000 | #8BFFFFFF | ButtonFace |
| ControlStrongFillColorDisabled | #51000000 | #3FFFFFFF | ButtonFace |
| ControlSolidFillColorDefault | #FFFFFF | #454545 | ButtonFace |
| SubtleFillColorTransparent | #00FFFFFF | #00FFFFFF | Transparent |
| SubtleFillColorSecondary (hover, selected) | #09000000 | #0FFFFFFF | ButtonFace |
| SubtleFillColorTertiary (pressed) | #06000000 | #0AFFFFFF | ButtonFace |
| ControlAltFillColorSecondary / Tertiary / Quarternary | #06000000 / #0F000000 / #18000000 | #19000000 / #0BFFFFFF / #12FFFFFF | ButtonFace |
| AccentFillColorDefault | Dark1 | Light2 | Window (accent controls use Highlight [inferred]) |
| AccentFillColorSecondary (hover) / Tertiary (pressed) | Dark1 at 0.9 / 0.8 opacity | Light2 at 0.9 / 0.8 | Window |
| AccentFillColorDisabled | #37000000 | #28FFFFFF | Window |
| AccentFillColorSelectedTextBackground | SystemAccentColor | SystemAccentColor | Window (text selection uses Highlight [inferred]) |
| CardBackgroundFillColorDefault / Secondary | #B3FFFFFF / #80F6F6F6 | #0DFFFFFF / #08FFFFFF | Window |
| LayerFillColorDefault | #80FFFFFF | #4C3A3A3A | Window |
| LayerFillColorAlt | #FFFFFF | #0DFFFFFF | Window |
| SolidBackgroundFillColorBase (Mica fallback) | #F3F3F3 | #202020 | Window |
| SolidBackgroundFillColorBaseAlt (Mica Alt fallback) | #DADADA | #0A0A0A | Window |
| SolidBackgroundFillColorSecondary / Tertiary / Quarternary | #EEEEEE / #F9F9F9 / #FFFFFF | #1C1C1C / #282828 / #2C2C2C | Window |
| AcrylicInAppFillColorDefault / AcrylicBackgroundFillColorDefault, solid fallback | #F9F9F9 | #2C2C2C | Window |
| SmokeFillColorDefault | #4D000000 | #4D000000 | Window at 0.8 |

### Strokes

| Resource | Light | Dark | HC |
|---|---|---|---|
| ControlStrokeColorDefault | #0F000000 | #12FFFFFF | ButtonText |
| ControlStrokeColorSecondary | #29000000 | #18FFFFFF | ButtonText |
| ControlStrokeColorOnAccentDefault / Secondary | #14FFFFFF / #66000000 | #14FFFFFF / #23000000 | ButtonText |
| ControlStrongStrokeColorDefault (input bottom edge, checkbox) | #72000000 | #8BFFFFFF | ButtonText |
| ControlStrongStrokeColorDisabled | #37000000 | #28FFFFFF | ButtonText |
| CardStrokeColorDefault | #0F000000 | #19000000 | WindowText |
| CardStrokeColorDefaultSolid | #EBEBEB | #1C1C1C | WindowText |
| SurfaceStrokeColorDefault (dialog) | #66757575 | #66757575 | WindowText |
| SurfaceStrokeColorFlyout | #0F000000 | #33000000 | WindowText |
| DividerStrokeColorDefault | #0F000000 | #15FFFFFF | WindowText |
| FocusStrokeColorOuter (2 px) | #E4000000 | #FFFFFF | WindowText |
| FocusStrokeColorInner (1 px) | #B3FFFFFF | #B3000000 | Window |

### System (status) colours

Always paired with an icon or text (FLUENT2 "Colour").

| Resource | Light | Dark | HC |
|---|---|---|---|
| SystemFillColorSuccess | #0F7B0F | #6CCB5F | WindowText |
| SystemFillColorCaution | #9D5D00 | #FCE100 | WindowText |
| SystemFillColorCritical | #C42B1C | #FF99A4 | WindowText |
| SystemFillColorAttention | SystemAccentColor | Light2 | WindowText |
| SystemFillColorSuccessBackground | #DFF6DD | #393D1B | Window |
| SystemFillColorCautionBackground | #FFF4CE | #433519 | Window |
| SystemFillColorCriticalBackground | #FDE7E9 | #442726 | Window |
| SystemFillColorAttentionBackground | #80F6F6F6 | #08FFFFFF | Window |

### Accent ramp

`SystemAccentColor` and `Light1..3` / `Dark1..3` are runtime values computed
by the shell; the algorithm is not public and no hex appears in any XAML
file. How WinUI uses them:

| Use | Light | Dark |
|---|---|---|
| Accent fill: primary button, checkbox, toggle on, selection pill, progress, focused input underline | Dark1 | Light2 |
| Accent text and links: rest / hover / pressed | Dark2 / Dark3 / Dark1 | Light3 / Light3 / Light2 |
| Text selection background (text #FFFFFF) | SystemAccentColor | SystemAccentColor |
| Text on accent fill | #FFFFFF | #000000 |

Fallback ramp, WinUI's own built-in blue (`SystemThemingInterop.cpp`):
Dark3 #002642, Dark2 #004275, Dark1 #005A9E, Accent #0078D7, Light1
#429CE3, Light2 #76B9ED, Light3 #A6D8FF. (#0078D4 is often quoted as the
Windows default base [unverified].)

### Elevation borders

All 1 px. WinUI draws them as absolute-mapped gradients, which produce
these pixels:

| Brush | Light | Dark |
|---|---|---|
| ControlElevationBorderBrush (Button, ComboBox) | bottom edge #29000000, other sides #0F000000 | top edge #18FFFFFF, other sides #12FFFFFF |
| AccentControlElevationBorderBrush | bottom #66000000, rest #14FFFFFF | bottom #23000000, rest #14FFFFFF |
| TextControlElevationBorderBrush (TextBox) | bottom #72000000, rest #0F000000 | bottom #8BFFFFFF, rest #12FFFFFF |
| TextControlElevationBorderFocusedBrush | bottom **2 px** Dark1, rest #0F000000 | bottom 2 px Light2, rest #12FFFFFF |
| Any of these in HC | solid WindowText | |

Pressed and disabled controls drop the gradient for plain
ControlStrokeColorDefault (accent: transparent). Shadows (ThemeShadow,
elevation 2-128) are not in the XAML; Komorebi uses the strokes alone
(FLUENT2 "Elevation").

### Materials and precomposited values

The software renderer cannot draw Mica or acrylic. Komorebi uses the solid
fallbacks Windows itself shows when transparency is off, on battery saver,
when the window is inactive, or before build 22000 (mica.md): base
#F3F3F3 / #202020, flyouts #F9F9F9 / #2C2C2C, smoke #4D000000.

Precomposited (`round(top·a + bottom·(1−a))`) on Komorebi's two surfaces,
**layer** = SolidBackgroundFillColorBase and **card** = LayerFillColorAlt
over the base (#FFFFFF / #2B2B2B). Contrast is against that surface.

| Resource | Light on layer | Light on card | Dark on layer | Dark on card |
|---|---|---|---|---|
| TextFillColorPrimary | #1A1A1A | #1B1B1B (17.2:1) | #FFFFFF | #FFFFFF (14.2:1) |
| TextFillColorSecondary | #5C5C5C | #616161 (6.2:1) | #CCCCCC | #CFCFCF (9.1:1) |
| TextFillColorTertiary | #868686 (3.3:1) | #8D8D8D (3.3:1) | #969696 (5.5:1) | #9B9B9B (5.1:1) |
| TextFillColorDisabled | #9B9B9B | #A3A3A3 | #717171 | #787878 |
| SubtleFillColorSecondary | #EAEAEA | #F6F6F6 | #2D2D2D | #373737 |
| SubtleFillColorTertiary | #EDEDED | #F9F9F9 | #292929 | #333333 |
| ControlFillColorDefault | #FBFBFB | #FFFFFF | #2D2D2D | #373737 |
| ControlFillColorSecondary | #F6F6F6 | #FCFCFC | #323232 | #3C3C3C |
| ControlStrokeColorDefault | #E5E5E5 | #F0F0F0 | #303030 | #3A3A3A |
| ControlStrokeColorSecondary | #CCCCCC | #D6D6D6 | #353535 | #3F3F3F |
| DividerStrokeColorDefault | #E5E5E5 | #F0F0F0 | #323232 | #3C3C3C |
| CardStrokeColorDefault | #E5E5E5 | #F0F0F0 | #1D1D1D | #272727 |
| ControlStrongStrokeColorDefault | #868686 | #8D8D8D | #9A9A9A | #9F9F9F |
| AccentFillColorDisabled | #BFBFBF | #C8C8C8 | #434343 | #4C4C4C |
| SystemFillColorAttentionBackground | #F5F5F5 | #FAFAFA | #272727 | #323232 |

Light TextFillColorTertiary fails 4.5:1; see "Mapping to Komorebi".

## Control metrics

Shared: `ControlCornerRadius` 4 (controls, list backplates, tooltips),
`OverlayCornerRadius` 8 (dialogs, flyouts, menus, window). Body 14/20 in
Segoe UI Variable; icons Segoe Fluent Icons (MDL2 on Windows 10) at 16, 12
inside inputs. Keyboard focus only: 2 px FocusStrokeColorOuter + 1 px
FocusStrokeColorInner, margin −3 (outside) for buttons and menu items, +1
(inside) for list items. State changes (hover, pressed, colour) are discrete;
only geometry animates (knob, pill, scrollbar), 83/167 ms, spline 0,0,0,1.
Spacing: 8 between buttons and control-to-header, 12 between a control and
its label or between content blocks, 16 from a surface edge to its text.

### Button
Padding `11,5,11,6`, border 1, radius 4, height 32.

| State | Standard bg / fg / border | Accent bg / fg / border | Subtle bg / fg |
|---|---|---|---|
| Rest | ControlFillDefault / TextPrimary / ControlElevationBorder | AccentFillDefault / TextOnAccentPrimary / AccentControlElevationBorder | transparent / TextPrimary |
| Hover | ControlFillSecondary / TextPrimary / same | AccentFillSecondary / TextOnAccentPrimary / same | SubtleSecondary / TextPrimary |
| Pressed | ControlFillTertiary / **TextSecondary** / ControlStrokeDefault | AccentFillTertiary / TextOnAccentSecondary / transparent | SubtleTertiary / TextSecondary |
| Disabled | ControlFillDisabled / TextDisabled / ControlStrokeDefault | AccentFillDisabled / TextOnAccentDisabled / transparent | transparent / TextDisabled |

HC standard: ButtonFace / ButtonText, border transparent; hover and pressed
bg HighlightText, fg and border Highlight; disabled GrayText. HC accent:
Highlight / HighlightText; hover ButtonText / ButtonFace. Komorebi keeps
the 1 px ButtonText border on a standard button at rest under HC ("Mapping to
Komorebi": every control keeps its stroke).

### ToggleButton
Button metrics and states while unchecked. Checked: AccentFill Default /
Secondary (hover) / Tertiary (pressed) / Disabled, text TextOnAccent Primary
/ Primary / Secondary / Disabled, border AccentControlElevationBorder
(transparent pressed and disabled). HC checked: Highlight with
HighlightText, no border; checked hover ButtonText with ButtonFace text and
border (`ToggleButton_themeresources.xaml`).

### HyperlinkButton
Button padding and radius, border 1 transparent. Foreground AccentText
Primary / Secondary (hover) / Tertiary (pressed) / Disabled over
SubtleFill Transparent / Secondary / Tertiary. HC: Hotlight text on a
transparent background in every state, disabled GrayText
(`HyperlinkButton_themeresources.xaml`: `SystemControlHyperlinkTextBrush`).

### TextBox
MinHeight 32, MinWidth 64, padding `10,5,6,6`, border 1 (focused
`1,1,1,2`), radius 4, header margin `0,0,0,8`, clear button margin
`0,4,4,4` with a 12 px glyph.

| State | Background | Border | Text / placeholder |
|---|---|---|---|
| Rest | ControlFillDefault | TextControlElevationBorder | TextPrimary / **TextSecondary** |
| Hover | ControlFillSecondary | same | same |
| Focused | ControlFillInputActive | Focused: bottom 2 px accent | same |
| Disabled | ControlFillDisabled | ControlStrokeDefault | TextDisabled |

Selection: SystemAccentColor with #FFFFFF text. HC: border ButtonText,
hover border Highlight, focused bg ButtonFace with a 2 px border.

**Error state** (WinUI has no built-in one; this follows Fluent 2's field
validation): the bottom edge turns SystemFillColorCritical (1 px at rest,
2 px focused), the sides keep ControlStroke, and the message sits 4 under
the field in Caption SystemFillColorCritical with no icon, in place of the
field's caption while it shows. The message is the field's accessible
description, so a screen reader reads it with the field. HC: the whole
border is 2 px. Komorebi uses it in the onboarding flow only, and only
after a press of Continue or Sign in (never while typing): a missing or
wrong address ("Enter an email address like name@example.com."), an
empty Password ("Enter your password.") and, in manual setup, an empty
server name. A rejected password puts the Password box in this state
too, focuses it and selects its text.

### ComboBox
As Button (fills, ControlElevationBorder), padding `12,5,0,7`, chevron
E70D 12 px in TextSecondary. Drop-down: acrylic fallback #F9F9F9 / #2C2C2C,
1 px SurfaceStrokeColorFlyout, radius 8, margin `0,4`, MinWidth 80. Item:
margin `5,2`, padding `11,5,11,7`, radius 3; hover and **selected**
SubtleSecondary, pressed SubtleTertiary; selected pill 3×16, radius 1.5,
AccentFillDefault.

### AutoSuggestBox
A TextBox with buttons inside: query glyph 12 px (`AutoSuggestBoxIconFontSize`),
query button padding `3,2`, inner button margin `1,3`, delete button margin
`0,4`, left / right button margins 3 / 4, header margin `0,0,0,8`.
Suggestion list: acrylic fallback with 1 px SurfaceStrokeColorFlyout, as the
ComboBox drop-down (`AutoSuggestBox_themeresources.xaml`).
Komorebi's `SearchBox` (no suggestion list: search is live) lives in the
title bar (below), centred, 220-600 px wide, aligned left in the compact
layout. It is a TextField with the 12 px query button (E721, 28×24, 4 from
the end; it hands the keyboard to the results) after the clear button.
- Placeholder "Search {view}" ("Search All inboxes", "Search Newsletters";
  pt-PT "Pesquisar em {0}"), name "Search mail", tooltip
  "Search (Ctrl+E)".
- The clear button (E894) shows **whenever there is text**, focused or
  not, named "Clear search (Esc)".
- The scope control beside it looks like a ComboBox (Button fill, the
  current word and the 12 px chevron E70D) and opens a two-item menu of
  **scope words**, never a view name: "This folder" / "All folders"
  (pt-PT "Esta pasta" / "Todas as pastas"; in All inboxes the first is
  "These inboxes" / "Estas caixas"). The placeholder carries the view
  name, so the pair reads "Search Newsletters · This folder". Its slot is
  always reserved (as wide as the longer word), so typing never reflows
  the box; it is disabled in Drafts and the Outbox. The menu is a native
  menu (a separate window), so it never overlaps the bar's drag area.
- Ctrl+E, Ctrl+F and F3 focus it; so does a click. From Settings, this
  closes Settings first and returns to the mail view the search applies
  to. It is the first stop of the F6 cycle (search → folder pane → list →
  reader) and the first Tab stop of the window; the scope control is the
  next.
- Switching folders keeps the query and runs it in the new view; only the
  scope goes back to "This folder". While a query runs the list title is
  "Search results", a results line under the SelectorBar says
  "{n} results in {scope} · Search all folders · Clear search" (a polite
  live region where the platform has one), the pane pill stays on the view
  the search started from, and rows of an all-folders search carry an
  "Account · Folder" tag. The row menu then has **Show in folder**, which
  clears the query and opens the message in its folder.

### SelectorBar
Bar padding `0,4`, items side by side (no gap; an item's icon sits 8 from
its text), transparent. Item padding `12,10,12,7`, Body 14 regular (selected is not bold), focus visual margin −2. Text:
TextPrimary at rest and selected, TextSecondary on hover (also selected
hover and selected pressed), TextTertiary pressed, TextDisabled. Selection
pill at the item's bottom centre: 4×3 scaled ×4 = 16×3, radius 1.5,
AccentFillDefault (disabled AccentFillDisabled), fading and scaling in over
the ComboBox item duration with spline 0,0,0,1. HC: text WindowText, hover
and pressed GrayText, pill Highlight (`SelectorBar.xaml`,
`SelectorBar_themeresources.xaml`).

### CheckBox / RadioButton / ToggleSwitch
- CheckBox: box 20×20, stroke 1, radius 4, row MinHeight 32, label
  padding `8,5,0,0`, glyph E73E 12 px. Unchecked: ControlAltFillSecondary
  with ControlStrongStrokeDefault (hover ControlAltFillTertiary, pressed
  ControlAltFillQuarternary). Checked: AccentFill Default / Secondary /
  Tertiary, glyph TextOnAccentPrimary. Disabled: AccentFillDisabled.
- RadioButton: ring 20, dot 12 (14 hover, 10 pressed). Unchecked ring
  ControlStrongStrokeDefault on ControlAltFillSecondary (hover
  ControlAltFillTertiary, pressed ControlAltFillQuarternary); checked:
  AccentFill Default / Secondary / Tertiary with the dot in
  TextOnAccentPrimary; label 8 from the ring, row MinHeight 32
  (`RadioButton_themeresources.xaml`). A group (`RadioButtons`) is one Tab
  stop: Up/Down (and Left/Right) move the selection and check it, Space
  checks the focused item; with nothing selected yet the focus lands on
  the first item without checking it. HC: ring ButtonText, checked
  Highlight with the dot HighlightText.
- Komorebi's `RadioButtons`: the group header above it (a FieldHeader,
  8 below), then each item with an optional **one-line description** in
  Caption TextSecondary under its label (Windows Settings explains a
  choice this way), items 4 apart. The onboarding chip's provider choice
(after a lookup found nothing):
  "Purelymail" / "Any domain hosted at Purelymail", "Postale.io" / "Any
  domain hosted at Postale", "Other server" / "Any other IMAP and SMTP
  server". Accessible: the group is a `radio-group` named by its header,
  each item a `radio-button` described by its line.
- ToggleSwitch: track 40×20, radius 10. Off: 1 px ControlStrongStroke on
  ControlAltFillSecondary, knob TextSecondary. On: AccentFillDefault, knob
  TextOnAccentPrimary. Knob 12, 14 on hover, 17×14 pressed (83 ms).
  On/Off content 10 px from the track (`ToggleSwitchPreContentMargin`),
  header margin `0,0,0,4`. Off hover ControlAltFillTertiary, pressed
  ControlAltFillQuarternary; on hover / pressed AccentFill Secondary /
  Tertiary.

### ListView item (the message list)
MinHeight 40, padding `16,0,12,0`. Backplate = item inset `4,2,4,2`,
radius 4. Backgrounds: rest transparent, hover SubtleSecondary, pressed
SubtleTertiary, **selected SubtleSecondary**, selected+hover SubtleTertiary.
Text stays TextPrimary in every state; there is no accent tint and no
accent border.
**Selection pill**: 3 px wide, radius 1.5, x = 4, AccentFillDefault,
vertically centred, height `max(16, rowHeight − 40)` (pressed −6): a
76 px row gives 36 px. Content moves 7 px right while the pill shows.
HC: hover, pressed and selected are Highlight with HighlightText; pill
HighlightText.

Komorebi's message row (`MailItem`): 76 px (`Theme.row-height`, taller
with the text size), padding `16,9,12,9`, the plate and pill as above
(selected + pressed SubtleSecondary, as `ListViewItemBackgroundSelectedPressed`),
the pill's height animated over 83 ms; no dividers and no unread bar (a
left bar would meet the selection pill).
**The content does not move 7 px while the pill shows**: the row's three
lines would re-elide and jump on every selection, against FLUENT2 "Built
for focus"; the pill sits in the 16 px padding instead. Sender and subject
Body, date and preview Caption, status icons 12 px TextSecondary. Each
colour holds in every plate state (rest, hover, pressed, selected).

Cues (new Outlook, Windows Mail): an **unread** row has its sender,
subject and date in TextPrimary, semibold (the preview stays Caption
TextSecondary, regular), and an **8 px AccentFillDefault dot in a leading
8 px slot** before the sender, centred on line 1 (HighlightText on a lit
row under HC). The slot (8 px, then 4 px) is kept on all three lines and
on read rows too, so the columns never shift; it follows the 16 px padding
(and the check box while it shows), 9 px clear of the selection pill. A
**read** row is regular and **dimmed**: sender and subject `row-read-text`,
date and preview `row-read-meta`, both TextSecondary. TextTertiary measures
3.3:1 on the light card, under the 4.5:1 floor, so it is never used for row
text. Only weights 400 and 600; every row colour keeps 4.5:1 on the card
and on the hover, pressed and selected plates, and under a contrast theme
all of them are WindowText (`hc-text`, HighlightText on a lit row), so HC
keeps the dot and the weight as its cues. An optional subtle unread tint
(an `unread-tint` fill behind unread rows) is allowed if it stays under the
plates' strength and drops out under HC; Komorebi does not ship one, as at
the plates' 4 to 6 percent it competes with hover and selection. The
**account** (All inboxes with
more than one account) and a search of all folders ("Account · Folder")
are a **text tag** at the start of line 3, Caption TextSecondary in a
radius-4 box with a ControlStrokeColorSecondary edge, never a coloured
dot. No other coloured dots in rows.

**Important** (IMAP `\Flagged`, worded "Mark as important" / "Remove
importance" and the "Important" filter): a 14 px filled flag in
`Theme.important` at the end of line 2, in a 20 px slot after the
subject (kept on every row of messages, so a hover never re-elides it). `important` is Fluent 2 marigold (shade30 #835b00 on light,
primary #eaa300 on dark: 6.1:1 and 6.6:1 on the card, 5.6:1 and 5.4:1 on
a hovered or selected row), kept apart from SystemFillColorCritical,
which stays for drafts and failed sends; WindowText under HC
(HighlightText on a lit row). It sits on line 2, not before the date,
because the quick actions cover the end of line 1 on hover. On a hovered
row of messages that is not important the slot shows an outline flag in
TextSecondary. The slot is a subtle button of its own (hover and pressed
fills, a HighlightText outline instead under HC, a tooltip, an accessible name, no Tab stop): a click toggles that
row only, like the quick action, and never selects or opens the row. On
a draft or an Outbox row (no quick actions) the flag is a plain icon,
outside the accessibility tree: the row's name already says Important.

**High importance** (set by the sender: `Importance: high`, `X-Priority`
1 or 2, or `X-MSMail-Priority: High`) is a different thing from the
user's Important and never looks like it: a bold "!" (Body size, weight
700, in the text font, as the icon fonts' Important glyph is an outline
too thin at 12 to 14 px) in TextPrimary, first among line 1's status
icons before the date; HighlightText on a lit row and WindowText under HC
(through `on-fill`). Never amber (the user's marker) and never
SystemFillColorCritical (drafts and errors). It is passive (no pointer
target) and, like the date, sits under the quick actions while they show;
the row's accessible name says "High importance". **Low importance** is
quieter: the composer's Down glyph (Segoe Fluent Icons E74B) at 12 px in
TextSecondary, among line 1's status icons after the "!" slot, like the
reply and attachment icons; the row's accessible name says "Low
importance". It only takes room on the (few) low rows; at 900x700 in
pt-PT the sender elides a little more there and nothing else moves.

**Reader importance**: under the To line, a Caption TextSecondary line
"Sent with high importance", led by the list's bold "!" (Body, weight
700, TextPrimary, hidden from screen readers, which read the sentence),
or "Sent with low importance" alone. Plain text, not an InfoBar (it is
information about the message, not a state to act on); passive, and
absent for normal mail. It elides rather than wraps.

**Composer importance**: two ToggleButtons at the end of the formatting
bar, after a divider: High importance (the Important glyph) and Low
importance (Down), 32 px, tooltip only. At most one is on; turning it off
again sends the message as normal. They are part of the bar's one Tab
stop (Left/Right move along it), and checked uses the bar's
checked-button style (Preview's).

**Quick actions** (new Outlook's hover actions): while the pointer is over
a row, or the row has the keyboard ring, four 28 px subtle buttons (Archive,
Delete, Mark as important / Remove importance, Mark as read / Mark as unread) take the date's
place at the end of line 1, on the row's own plate. They act on that row
only: they never open it, never mark it read (only Mark as read sets
`\Seen`) and never change the selection. They are **not Tab stops** and
take no focus: focus stays on the row, and the keyboard reaches the same
commands with Backspace, Delete, Insert (or Ctrl+Shift+G), Ctrl+Q, Ctrl+U
and Shift+F10.
Each has a tooltip with its shortcut ("Archive (Backspace)") and the same
accessible name. Only the hovered or ringed row builds them (`if`), so the
list's first frame does not pay for them.

**Multi-select** (ListView `SelectionMode.Multiple` with check boxes):
"Select messages" in the list header's "..." menu turns check boxes on
(the menu item shows a check mark while it is on); they also show while
two or more rows are selected. The 20 px check box sits in the leading
slot (the text moves 28 px right while the boxes show); clicking it adds or
removes that row as Ctrl+click does, and never opens the row. Multi-
selection uses the same plate and pill on every selected row. The reader
pane then shows "N messages selected" with the commands for all of them,
"Select all" and "Clear selection" (Esc), and when Move is off its reason
as Caption text under the buttons ("Select messages of one account to move
them").

### Layout breakpoints
WinUI's NavigationView switches at 641 and 1008 px (Minimal / Compact /
Expanded), and its list-details pattern shows one pane at a time on a
narrow window, with a back button on the details. Komorebi,
in logical px:

| Width | Navigation | List and reader |
|---|---|---|
| ≥ 1040 | the folder pane (264) | list 380 + reader |
| 720 - 1039 | the 64 px rail | list 300 + reader while the reader holds something (a message, the "N selected" panel, the composer, an Outbox entry); otherwise the list alone, stretched over the reader's place |
| 500 - 719 (one pane) | the 64 px rail | the list alone, or the reader alone: an open message, the composer or an Outbox entry takes the content area, with a back IconButton (E72B, "Back to list (Esc)") first in its command bar; a selection without an open message keeps the list, so Shift+arrows and Ctrl+click can still extend it (the bulk commands stay on the row menu, the quick actions and the keys) |

- The window's minimum is 500 × 480, so Snap Layouts' halves and thirds
  fit common screens (1366 / 2 = 683, 1920 / 3 = 640 physical px; at
  125 % a 500 px window is 625 px). At 150 % and more a third of a
  1920 px screen is under 500 logical px, so Windows keeps the window at
  its minimum there (hand check).
- One pane: while the list shows, Up / Down, Home / End and PageUp /
  PageDown only move its cursor (opening would hide the list under the
  keys, as the Mail app's narrow layout does); Enter or a click opens the
  row, and with a message or Outbox entry open the arrows step to the
  next one, as everywhere. The hidden list is no Tab or F6 stop; the back
  button alone takes the focus when the list goes from under it, also on
  F6 and after a closed prompt (the command bar's buttons stand back
  then, so the key after Enter never lands on Reply; "the button the
  arrows last reached" holds in the wide and compact layouts). Esc keeps
  the one Esc
  stack (search, then the open message); its last step is the back
  button's: the message or Outbox entry closes and the list returns with
  the focus. The composer keeps its own Close.
- One pane: the list's status line and the Undo offer would be hidden
  with it, so they float over the reader's foot (12 px from its edges,
  the Undo card's style) and never move its content.
- The list's width comes from min / max widths and stretch, never from
  the window's width (which would feed back into the window's layout
  constraints, a binding loop).
- The title bar's search: 220 - 600 px down to 720 px, down to 120 px in
  one pane, so the 48 px drag space beside the caption buttons stays at
  500 px. The Search teaching tip sits beside the list over the reader
  column, or under the search box when there is no reader column.

### NavigationView, left (folder pane)
Compact pane 48, open pane 320 default; compact below 641, expanded from
1008 (Komorebi: compact rail below 1040). Item: MinHeight 36, margin `4,2`
(40 per row), radius 4, 40-wide icon box with a 16 px icon, Body 14.
Content (label) margin `4,-1,8,-1` after the icon box, so the label starts
44 from the item's edge (48 from the pane's). Backgrounds: rest
transparent, hover SubtleSecondary, pressed SubtleTertiary, selected
SubtleSecondary, **selected + hover SubtleTertiary**, selected + pressed
SubtleSecondary; text TextPrimary, **pressed TextSecondary**, disabled
TextDisabled. Selection indicator 3×16, radius 2, AccentFillDefault,
at the item's left edge, animated between items (Komorebi: it jumps).
Expand chevron E70D at 8 px
(`NavigationViewItemExpandedGlyphFontSize`), turned over while expanded,
at the item's end. Group headers TextSecondary, margin `16,0`. Separator
1 px DividerStroke, margin `0,3,0,4` (8 px a row). PaneToggleButton 40×36
(E700), subtle, named and tooltipped "Open Navigation" / "Close Navigation"
(`NavigationView/Strings/en-us/Resources.resw`; Komorebi in sentence case). Pane background transparent (on the base); the pane
opened over the content (overlay, light-dismiss) is
AcrylicInAppFillColorDefault (Komorebi: `flyout`, its 1 px `surface-stroke`
edge for the 16 px shadow). **Content area** on LayerFill with a
`1,1,0,0` CardStrokeColorDefault border and radius `8,0,0,0`. HC:
indicator HighlightText, hover and selected Highlight
(`NavigationView_themeresources.xaml`).

Komorebi's folder pane (`NavRow`): pane 264 (`Theme.pane-width`: WinUI's
248 plus the 16 px expander column; a documented deviation), rail 48 below
1040 px; items `Theme.nav-row` tall (36, taller with the text size), 4
apart and 4 from the pane's sides. Folders read as a tree, TreeView's
pattern inside the NavigationView pane (NavigationView's trailing chevron
is made for 2-level menus, not mail trees; a documented deviation): a 16 px
expander slot at 4 + 12 px per level (`Theme.nav-indent`; levels capped at
5), E76C while folded and E70D while open at 8 px, `text-2` (`text` on a
lit row or under the pointer), shown only on parents but reserved on every
row; the 16 px icon at 20, the label at 48 (plus the indent; elided at
its end, with the full name as a tooltip), and a count column ending 8 px
before the item's end that never elides. Folded accounts and parent
folders are remembered (the `nav_folded` setting); a folded parent shows
its subtree's unread, and a selected folder inside a folded line puts the
pill on that line. The account line heads its group (Outlook's,
Thunderbird's and eM Client's strong account line): 8 px of space above
it (`Theme.nav-gap`; space, not a divider, and not part of its plate, hit
area or focus ring), its chevron at 4 in `text` (the accordion's control,
so it reads at rest), a 20 px initial tile at 20 (`Theme.tile`: the
account's colour under its initial in `on-avatar`, Caption semibold, even
with one account; hollow with a 1 px `text-disabled` ring while switched
off; under HC the Window colour in a WindowText ring, HighlightText on a
lit row, never the palette), a rejected sign-in as an 8 px `danger` badge
on the tile's bottom-right, cut out by a ring of the pane's colour, and
its name (its address when it has none) in Body Strong `text`. Its
tooltip always gives the name and the address; folded, it shows its
Inbox's unread. While hovered or the keyboard's row, a subtle 28 px "..."
button (E712, "More actions") 4 px from the item's end takes the count's
place and opens the row menu under it. Counts are neutral, so no accent
reads as an error: unread Caption
semibold `text`, items (local Drafts and Outbox, and the server's Drafts
total) Caption `text-2`, capped at 999+ (the accessible label keeps the
number); `text-disabled` for a switched-off account. Row menu (one shared
`ContextMenuArea`, never one per row; a right-click, the "...", or
Shift+F10 / the Menu key under the current row), only commands the app
has, built by the Rust side (`App::nav_menu`; `menu:pane` prints it): a
folder or All inboxes Mark all as read (that folder, whatever view is
open, or every Inbox) and Collapse all (every account's parent folders);
an account line Mark all as read (its Inbox), Collapse all folders (its
parent folders), then Refresh (that account) and Account settings;
Drafts and Outbox none. Mark all as read is the list "..." menu's
function (disabled there while a search is showing, since it covers the
view, not the results): no prompt and no Undo (as in Windows Mail and new Outlook);
never "Empty folder" (it would expunge on the server). The pane scrolls with
the list's thin bar (below, "ScrollBar"), its thumb in the pane's right
4 px (an 8 px strip, not the list's 12, so it stays clear of the "...");
a right-click on any part of a row, chevron and "..." included, asks
for its menu and makes it the keyboard's row; while rows are scrolled away a 1 px `stroke` edge shows under New
mail (scrolled down) and a footer separator above Settings (rows below,
WinUI's footer separator), each in the middle of the 8 px between.
The rail (below 1040 px; `Theme.rail-width`, 64 px, wider only when the
longest word of its fixed names needs it, measured from `rail-words`, so
pt-PT "Definições" and "Rascunhos" are never cut) has its own short list,
not the folder tree, each item an icon with its name under it (new
Outlook's labelled rail; UX plan "Labels vs icons"): the pane toggle
("Folders", tooltip "Folders (Open navigation)"), New mail (the 40x32
accent button, its name under it), All inboxes (with more than one
account; its short rail name, "rail" context, pt-PT "Caixas de
entrada"), Drafts, Outbox (while shown), 8 px of space, then one 20 px
initial tile per account (the same tile rules and warn badge,
`text-disabled` while the account is switched off) that opens the
account's Inbox, its pill on the open folder's account, named "{account}
- {Inbox} ({n} unread)" as a tooltip and to screen readers. A rail item
is `Theme.rail-row` tall (64 px at 100% text size: a 24 px icon zone at
4, then the name in Caption, centred, wrapped on at most two lines and
elided after them); its pill sits beside the icon. Counts are InfoBadges
(16 px tall, radius 8, Caption semibold, 999+) at the icon's top-right:
unread `accent` with `text-inverse` (`text-disabled` for a switched-off
account), item counts (Drafts, Outbox) `text-2`; under HC Highlight with
HighlightText, reversed on a lit item. A rail tile's menu goes to its
account's Inbox, Sent or Archive (greyed where it has none). The
toggle opens the 264 px pane over the content on the flyout surface (its
right edge `surface-stroke`, then a 16 px shadow fading from #00000040
over the content, as the software renderer draws no drop-shadow), the
rest of the window under smoke (`scrim`, #4D000000; a click on it closes
the pane), scrolled to the open folder, until Esc, a click outside it, a
folder, Settings or Account settings is chosen, or the window widens;
folding in it keeps it open; no animation. On the flyout surface the
selected plate is SubtleTertiary in dark (the layer's opaque
`nav-selected` would vanish on #2C2C2C). Settings is the footer item; it
and Ctrl+, toggle Settings, whose header starts with a back arrow ("Back
to mail (Esc)"). Text and icons stay `text` (`text-disabled` for a
switched-off account).

Folding the pane at any width ("Tabs and windows", P1): the pane toggle
(E700, "Folders", the PaneToggleButton) sits at the top of the full pane
too, as in WinUI's NavigationView, where it switches Expanded and Compact.
From 1040 px it folds the 264 px pane to the rail and back (setting
`nav_collapsed`, read with the other settings before the first frame);
below 1040 px it keeps opening the pane over the content. Ctrl+Shift+E and
the pane's row menu ("Hide folders" / "Show folders") do the same. The
width change animates over `Theme.fast` (167 ms, ease-out; 0 with
animation effects off). The Folders tip shows whenever the rail does.

Navigation structure: with one account there is no All
inboxes (the start opens on that account's Inbox); with more, accounts
after the first start folded. The view shown is saved (`last_view`) and
opened at the next start while it exists, else the Inbox. When an
account's folders first arrive, its user parent folders without unread
mail of their own start folded (`tree::default_folds`, once per account:
the `nav_folds_init` setting); a folded parent shows its subtree's
unread. **Drafts** is always the second unified entry (its count; "No
drafts. Select New mail to start one." when empty): the local drafts.
The account's server Drafts folder stays in the tree, its list and count
leaving out the copies of local drafts (`Db::draft_copies`: same folder,
uid and UIDVALIDITY); it has no SelectorBar and no unread bold, and its
messages open without Reply / Reply all, with an Informational InfoBar
("This draft was saved on another device or in webmail. To keep writing
it here, use Forward as new draft.") and Forward labelled "Forward as new
draft". Draft and Outbox rows take Up/Down/Home/End (a draft is only
moved to: the composer would take the keys; an Outbox entry shows),
Enter, Delete (asks first) and a row menu (Open, Discard). The list
header shows a folder's account as a Caption line under its name; its
"..." ("More actions") holds text commands only (Mark all as read): no
bulk action hides behind an icon.

### CommandBar / AppBarButton
AppBarButton 68 wide, bar 64 tall (compact 48), icon 16, label Caption 12;
inner border margin `2,6`, radius 4; Subtle fills, pressed text
TextSecondary. Overflow on the flyout fallback with a 1 px
CardStrokeColorDefaultSolid border.

Komorebi's reader bar is a CommandBar with labels on the right (as Windows
Mail and Outlook): `IconButton`s 32 tall, 4 px apart (so the −3 outside
focus ring of one stays clear of its neighbours and the separators), padded
8 in a bar 48 tall at 100 % text (taller with the text size), separators
1×20 DividerStroke centred in the row, every button as tall as Reply (the
first to get a label) so the icons stay in line at a larger text size. Primary commands: Reply, Reply all,
Forward | Archive, Delete, Move (a 12 px E70D chevron 8 px from its end,
its flyout 4 below the button) | Mark as read / Mark as unread and Mark as
important / Remove importance (plain buttons whose label and icon swap with the message's
state, not toggles: a toggle's "checked" fill did not say which way it
would go) and More.

**Label priority, per button.** Each label is measured on its own at the
current text size, and labels are dropped one at a time, first dropped
first: Forward, Reply all, Move, Mark as important, Mark as read, then Reply, Delete,
and Archive last. Archive, Delete and Reply keep theirs strictly in that
order; the others each show while they still fit after the ones before
them (so a short Move label can show when Mark as read's long one does
not, but never before Archive, Delete and Reply have theirs). The room is
the bar less the icon-only bar (padding 16, eight 32 px buttons and Move
52, separators 2, gaps 40); a label takes its text and 15 px (Move 8 px).
So the commands people use most keep their words longest, whatever the
language: pt-PT at 1280 x 800 shows Responder, Arquivar, Eliminar and
Mover; English shows Reply, Archive and Delete and then Mark as read
or Move, whichever fits for that message (Mark as important / Remove
importance is too long to keep its label); at 900 both keep Archive and Delete
(English also Reply). The bar publishes which labels show (`cmd-labels`, a bit per button
in drop order) for the tests. Every tooltip carries its shortcut ("Reply
(Ctrl+R)", "Archive (Backspace)", "Mark as read (Ctrl+Q)", "Mark as
important (Ctrl+Shift+G)"), as do the quick actions.

More is a native menu of the secondary commands: Forward as attachment,
Move to (the folders), Report junk / Not junk, View as plain text
(checkable), Show original colours (checkable; on only for a dimmed page
in the dark theme), Load images / Stop loading images from this sender,
Save as .eml, Show original, Copy Message-ID, and Show in folder while
searching. WinUI moves primary commands into the overflow when they do not
fit; Komorebi drops their labels instead (the icons always fit).

**Move flyout** (Ctrl+Shift+V, the Move button, the selection's Move): a
filter box on top (focused as it opens; typing narrows the list), then the
last three folders used for this account ("Recent"), then the account's
folders in the pane's order, indented by depth (Theme.nav-indent per
level). Drafts folders and the Outbox are left out (a message is never
filed there). Enter in the filter moves to the first entry.

### MenuFlyout / ToolTip / Flyout
- MenuFlyout: radius 8, 1 px SurfaceStrokeColorFlyout, padding `0,2`,
  #F9F9F9 / #2C2C2C. Item margin `4,2`, radius 4, padding `11,4,11,5`
  (mouse; touch `11,8,11,9`), slot 32; hover SubtleSecondary, pressed
  SubtleTertiary; accelerator text TextSecondary. Separator 1 px Divider.
  Komorebi's menus are native Win32 popups (Slint's `ContextMenuArea`
  through muda), so they follow Windows' own menu look. Slint 1.18 allows
  `MenuItem.shortcut` only in a `MenuBar`, so the accelerator text goes in
  the item's title after a tab (`"Reply\tCtrl+R"`), which Win32 draws
  right-aligned as accelerator text. The row and More menus are built in
  Rust (`MenuCommand`), so `menu:row` and `menu:more` print exactly what
  they show.
- ToolTip: #F9F9F9 / #2C2C2C, 1 px SurfaceStrokeColorFlyout, padding
  `9,6,9,8`, 12 px TextPrimary, MaxWidth 320, radius **4**. HC Window /
  WindowText.
- Flyout: padding `16,15,16,17`, radius 8, border 1.

### ContentDialog
Width 320-548, height 184-756, radius 8, 1 px SurfaceStrokeColorDefault.
Top part (title, content): padding 24, LayerFillColorAlt, 1 px
CardStrokeColorDefault below; title Subtitle 20 semibold, margin-bottom 12.
Button bar: padding 24, SolidBackgroundFillColorBase, equal-width buttons
8 apart, default button accent. Smoke #4D000000 behind. HC: Window /
WindowText, smoke Window at 0.8.
Komorebi: one smoke layer at a time (Settings is a page, not a dialog,
so a prompt over it has the only smoke). A destructive question (remove
an account, remove all data, discard, send again) has no default button:
both standard, Cancel focused, Enter does nothing; any other (an
invitation's reply, Download again) has the accent default, focused,
which Enter presses. Closing a prompt gives the focus back to its opener
(FLUENT2 "Accessibility"): Settings' button that opened it, the
composer's body, the reader (the link prompt to the page and its focused
link; Show original from More, an invitation's answer or the Outbox's
Retry / Discard to the command bar's last-reached button or the Outbox's
first button), the folders, else the list. Menus and the Move flyout
need nothing: Slint gives the focus back to the item that had it when a
popup closes. The buttons share the bar equally, but a label
that needs more (a large text size) keeps its full width; a three-button
prompt (an invitation's reply) uses the full 548 px. Show original is a
tall dialog with the same bar (Copy Message-ID, Copy headers, Copy all as
the accent, Close).
Keyboard shortcuts (F1, Ctrl+/ and Settings › About) is a tall dialog of the
same kind: a scrolling two-column list (the keys in Body semibold, at
most 200 px wide, the action in Body; each group under a BodyStrong
heading), one **Close** button (accent, focused; Esc and Enter close;
Up / Down, PageUp / PageDown, Home and End scroll the list, and F6 stays
inside the dialog). The card has the groupbox role and the title as its
name, as ContentDialog does.
Its rows are the README "Keyboard" table's keys, kept equal by a test
(`tests/shortcuts_table.rs`), with short actions.

### TeachingTip
From WinUI `TeachingTip_themeresources.xaml` and `TeachingTip.xaml`
(values recalled, not re-read for this note: MinWidth 320, MaxWidth 336,
content margin 12, `OverlayCornerRadius` 8, 1 px border) [recalled].
Komorebi's tip is the **tail-less** variant (`PreferredPlacement` with
`TailVisibility="Collapsed"`), as Windows 11 Mail and new Outlook show
theirs:
- Card: max width 320 (narrower when the window leaves less), radius 8,
  the flyout surface (`flyout`, #F9F9F9 / #2C2C2C) with a 1 px
  SurfaceStrokeColorFlyout edge (`flyout-stroke`); padding 16 left and
  right, 12 top and bottom. HC: Window fill, 1 px WindowText edge.
- Text: the title in BodyStrong (14 semibold, TextPrimary), the body in
  Body (14) TextSecondary, wrapping; then the place in the series,
  "1 of 2", in Caption TextSecondary. The count is computed from the tips
  still due in the current layout, never written into the text.
- Buttons, 12 below the text, right-aligned, 8 apart: **Got it** (accent)
  and **Skip tips** (standard). Got it closes this tip (the next one in
  the series follows); Skip tips closes it and marks every tip seen.
- Placement: 8 px beside its target, inside the window and below the
  title bar (the bar's passthrough rectangles would take its clicks), on
  the side where it covers neither the current row nor the status line:
  the search tip under the title-bar search over the reader column, the
  Settings tip above the Settings footer inside the wide pane (to its
  right beside the rail), the Folders tip to the right of the rail's
  pane toggle.
- Behaviour: one at a time, **non-modal** (no smoke, the window stays
  usable), never takes the keyboard focus (it is opened by a timer, never
  from the keyboard), and **Esc is Got it**: the innermost step of the
  window's Esc stack (`keys`' capture handler takes it before a focused
  text field or the list does). While a tip shows, tooltips are
  suppressed (`Tip.blocked`), and a tip never shows over a dialog,
  Settings, the composer or Show original. Screen readers hear it once,
  politely (`accessible-live-region: polite` on the card, a groupbox named
  by its title).
- Komorebi's series: **Folders** (the narrow
  rail only: "Accounts and folders are behind this button when the window
  is narrow."), **Search** and **Help** (Settings › About and F1). A tip
  must teach what is not visible on screen. They start 1.5 s after the
  list first shows a row with an account, never on the welcome and never
  together with the What's new bar; the setting `tips_seen`
  (`folders,search,help`) remembers each, so a tip seen in one layout is
  not shown again in the other, and Folders shows once, as "1 of 1", the
  first time a desktop user's window folds the pane into the rail.
  Settings › About › Show tips again clears it.

### InfoBar
MinHeight 48, radius 4, 1 px CardStrokeColorDefault (HC 2), content left
padding 16, icon 16 with margin `0,16,14,16`, title 14 semibold, message
14 regular, close button 38. Severity: background
SystemFillColor*Background, icon a filled circle F136 in the severity
colour with the glyph (F13F info, F13E success, F13C warning, F13D error)
in TextFillColorInverse; informational uses the accent (Attention). HC:
background Window, 2 px border, title and message WindowText, icon circle
Highlight with the glyph in HighlightText (`InfoBar_themeresources.xaml`).
Custom content (`InfoBar.Content`, `InfoBar.xaml` row 1) sits under the
title and message, in the text column.
Komorebi's `InfoBar` takes children for it (`@children`, under the
message and above the content buttons, 12 below the message): the
account page's error bar (and the onboarding error's "Show details")
holds the check's step list (a 16 px state
glyph and the step name in Body) with its actions ("Set username",
"Review certificate") as the content buttons under the steps. Only that
bar has children; the others stay property-only.

Komorebi's reader bars: a sender notice (Caution, not closable): the
title "This message may not be from who it says", the plain reason on the
second line (and an encrypted message's note after it), and three content
buttons, Report junk (Not junk in the Junk folder), Delete and "Why?" (a
ContentDialog that explains the check with one Close button);
Informational for an encrypted message alone; remote images blocked (Informational, the sender's full
address in the message, buttons "Load images" and "Always load from this
sender" as its content, 12 below the message, side by side or one under the
other when the reader is too narrow for both) and images that could not
be loaded (Caution); both image bars close for the message shown.
"Always load from this sender" is followed by an Informational bar
"Images from {sender} will always load." with Undo and Privacy settings;
later messages from an allowed sender show the Caption "Images load from
this sender · Change" (Change opens Settings › Privacy) above the page.

**Undo, at the bottom of the list** (new Outlook's toast, in the InfoBar's
look): Undo for an archive, delete, move or junk report, and the
undo-send countdown, show in their own slot over the bottom of the list
column, inset 12, min height 48, radius 8, the flyout fill with a 1 px
CardStrokeColorDefault edge: the message and an "Undo" button (tooltip
"Undo (Ctrl+Z)"). It never moves the rows (it floats over the last ones)
and has its own slot, so a status at the top of the list cannot replace
it. A move's offer stays until the next archive, delete or move, or 10 s,
whichever is later, and its time stops while the pointer is over it or
it has the focus (WCAG 2.2.1); it then gets at least 3 s more. The
undo-send offer lasts the window set in Settings. Esc in the list never
takes it away. Screen readers hear it politely as it appears.

**What's new** (after an update): the status line, Informational, "Komorebi
was updated to {0}." with the button What's new (the changelog, through the
link prompt), 2 s after the start, never while a teaching tip shows. It does
not time out; once a run; the version is saved when it is closed or its
button pressed, so a status that takes its place lets it come back at the
next start.

**A message not downloaded yet** (offline, or its body still on the
server): after a 2 s wait (a body arriving sooner shows without it) the
reader says "This message hasn't been downloaded yet. It downloads when
Komorebi can reach the server." with Retry; never a blank page.

### SettingsCard (Windows Community Toolkit)
Not in WinUI itself; the card Windows 11 Settings is built from
(CommunityToolkit/Windows `SettingsControls`). MinHeight
68, padding 16, background CardBackgroundFillColorDefault, 1 px
CardStrokeColorDefault border, radius 4 (ControlCornerRadius); header
icon 20 px with margin `2,0,20,0`; header Body 14 TextPrimary,
description Caption 12 TextSecondary under it; the control at the end,
vertically centred, 16 after the text. Cards in a group 4 apart; a group
header Body Strong, 24 above and 8 below. Below a wrap threshold (about
476 px) the toolkit moves the control under the text [unverified];
Komorebi does not: its narrowest page (900 px window) leaves the text
column at least 400 px.

Komorebi's Settings page: on the layer (no content card under it, as in
Settings), title "Settings" in Title 28 semibold `display-font`, the
SelectorBar Accounts / General / Privacy / About under it, the column 16
from the page's edges and at most 1000 px wide. `SettingsCard` carries a
Button (`action`, accent while there is no account, `danger` label for
Remove all data) or a ToggleSwitch (`toggle`) as plain hidden elements;
the General ComboBoxes (200 px) and the repository HyperlinkButton are
drawn over the card's `trailing` space (a card with children would be a
copy of the card's code at every use). An account's card is its own
`SettingsSurface` and one button (Windows Settings' "Your accounts"
cards): icon and colour dot, the account's name, its address, its status
(the full sentence of a problem, with "Update password" or "Review
certificate"), a 3 px ProgressBar while it downloads, and a chevron at
its end; a click, Enter or Space opens the **account page**. The account
page and the Edit page are one page ("Accounts > {name}", the parent in
TextSecondary, Back before it; a "…" IconButton "More options for this
account" after the title opens a menu with Move up / Move down, only with
two accounts or more). Its column, at most 560 px wide, has groups (Body
Strong, 24 above, 8 below): **Account** (the "Sync this account"
SettingsCard with its ToggleSwitch, which says what Off does, then Name,
Email (disabled, with the Caption "The address can't be changed. To use
another one, add another account.") and Password), **Signature** (two ComboBoxes, "New messages" and "Replies
and forwards", each listing None and the signature names, with a
"Manage signatures" link to the Signatures page; the Add-account form has
no signature field), **Folders** (Save a copy of sent mail, then each role's header *above*
its ComboBox, so no label is cut), **Server settings** (an `Expander`,
collapsed, that opens by itself on a failed check or while a certificate
is saved; Find settings inside it, and only for a server that isn't a
preset), then a **Downloaded mail** card (Download again) and the
**Remove account** card (danger label) at the bottom. Save, Test
connection and Cancel are pinned under the scroll view, after a 1 px
divider, so they never scroll away; an error scrolls into view and
focuses its first button. There is no add form in Settings any more:
the Accounts page's "Add account" opens the onboarding flow ("Onboarding"
below) inside the content area, without the welcome; its server fields
and port (96 px) side by side are the flow's manual setup.

The certificate prompt ("Trust this certificate?") shows the SHA-256
fingerprint in the code font as 4 lines of 8 bytes (never broken inside
a byte), a Copy IconButton beside it ("Copy fingerprint"), and a
HyperlinkButton "How do I check this?" that reveals the openssl hint. A
certificate that replaces a saved one shows both fingerprints, "New" and
"Saved before", with a caution line. A changed certificate on the status
line is one short sentence with **Review certificate** (it opens the
account page and reads the new certificate into this prompt); the full
sentence is on the account's card.

A server out of reach puts a caution badge (a 12 px Caution dot cut out
of the tile, as the rejected sign-in's danger dot) on the account's pane
row and rail tile until a sync goes through, with "Can't reach {server}"
in its tooltip and accessible name; the account row in the wide pane also
shows a caution IconButton that opens the account page. The status line
says so (Caution) with **Try now**.

Privacy: Remote images (the senders whose images always load), Diagnostics
(the log toggle in one sentence, and Open logs folder), and a **Reset**
group holding Remove all data. About: the version card, a Help group
(What's new, Show tips again, Keyboard shortcuts, Get help, Report a
problem) and an About group (Privacy statement, Data folder with the path
cut from its start and the whole path in the tooltip, Source code, Made
with Slint), every action at the cards' end. A language change redraws
the window in the new language at once and puts an Informational InfoBar
under the Language card: "Some text changes after Komorebi restarts."

### Onboarding (first run and Add account)
Fluent 2 "Take action" (docs/FLUENT2.md "Onboarding"), built from the
components above in `ui/onboarding.slint`; one page for the first run
(the whole window under the title bar, no pane or rail; the bar keeps no
search and drags as a whole) and for Settings > Accounts > Add account
(the same page in the content area, its step indicator and "Make it
yours" kept, the welcome hero not shown). The Settings "Add an account"
sub-page is gone; the account page (Edit) is unchanged.

- **Frame**: on `layer`; the column centred, its top at 40 % of the free
  height (at least `s10`), scrolling when taller than the window. The
  `StepIndicator` sits above the card, `s6` under the title bar.
- **`OnboardCard`**: at most 480 px wide (560 px for manual setup),
  padding `s8` wide / `s6` narrow, `card` fill, 1 px `card-stroke` (2 px
  under a contrast theme), radius `radius-xl` (12, this card only). Its
  content enters with opacity 0→1 and y +12→0 over `normal` (200 ms,
  `ease-out`, decelerateMax); leaving fades over `faster` (`ease-in`);
  Back reverses the offset.
- **`StepIndicator`**: three 24 px circles joined by 1 px `stroke-strong`
  lines, the label in Caption under each: done is an accent fill with a
  check, current a 2 px accent ring with an accent dot, future a
  `stroke-strong` outline with a `text-3` label (HC: Highlight /
  HighlightText). Below 440 px of column or above 1.5× text it is just
  "Step {0} of {1}" in Caption `text-2`. `accessible-role: list`, each
  item "{label}, step {n} of 3, completed / current / not started"; the
  current one is a polite live region.
- **`BrandMark`**: vector only (a circle in `mark-a`, three leaves in
  `mark-b` and `mark-c` (white, Light3 on dark) at 0.85, a sun with four rays), 96 px on the welcome and
  64 px on the finish; `celebrate` sways the leaves ±6° and scales
  0.92→1 over `slow` once (still with Animation effects off). HC: 2 px
  `hc-text` outlines on `hc-window`. Accessible as an image named
  "Komorebi".
- **`ProviderChip`**: min height 48, radius 4, `card-fill-2`, 1 px
  `card-stroke`: looking (a 16 px ring after 1 s), found (a 16 px success
  circle with a check, the title in Body Strong, the detail in Caption
  `text-2`), not found (Caution background, with the provider
  RadioButtons inline under it), and a trailing subtle "Change" that opens
  manual setup.
- **Welcome** (only while there is no account): wide (window ≥ 1040 px),
  two columns `s12` apart: the 96 px mark, the headline in `display` (40,
  `display-font`, 600), the subhead in Subtitle 16 `text-2` (max 440 px),
  the accent "Add your email account" (min 200 × 40) and the time line in
  Caption; on the right three value rows (a 20 px glyph in a 36 px
  accent-tinted rounded square, Body Strong title, Caption `text-2`
  line, `s4` apart). Narrow: one centred column. Footer `s6` from the
  bottom: "Need help?" · "Privacy" · a Language ComboBox (live; 160 px, or
  wider for a language's own name, never cut) and a Settings IconButton.
  The entrance (the mark over `gentle`, then the text and the rows each a
  `stagger` later; opacity only for now) starts from a Timer after the
  first frame; with Animation effects off everything is drawn at once.
- **Your email**: Title 20 / 600, a Body `text-2` line, the TextBox
  (`name@example.com`), then Back (standard) and Continue (accent),
  stacked when they do not fit. Errors are the TextBox error state.
- **Sign in**: the title "Sign in to {0}" (elided, the whole address in
  the tooltip), the chip, the Password box (lock glyph `E72E` leading in
  `text-3`, the reveal button), the Credential Manager caption, Back and
  Sign in. The wait disables the Password box and Sign in only; the card's
  lower half shows three stages (16 px state glyph and Body text) at once,
  the ring on the running stage after 1 s, a Caption after 3 s and
  another after 10 s. A failure is a Critical InfoBar inside the card,
  above the buttons: title, one sentence, the next step as its buttons, and
  a subtle "Show details" that unfolds the 7-step list and the long
  sentence with the host names.
- **Manual setup** (560 px): "Server settings", one Body `text-2` line,
  the IMAP and SMTP host, port (96 px) and security ("Encrypted (SSL/TLS)"
  / "Encrypted after connecting (STARTTLS)"; the port follows), "Same
  sign-in as incoming", Username, Find settings with its note, the
  certificate lines; Back returns to the chip.
- **Make it yours**: "Your name" (prefilled from the address's local
  part), the 4-line signature `ComposeBody`, a From preview strip on
  `card-fill-2` in Caption, the sync line (3 px ProgressBar) under the
  card, Skip (subtle) and Done (accent); no Back, Esc is Skip.
- **Finish**: a centred column at most 440 px: the 64 px mark with
  `celebrate`, "You're all set" in Title Large, a Body `text-2` line, the
  sync line, the accent "Open your inbox" (focused) and a subtle "Add
  another account".
- **Keys**: Enter is the primary action, Esc goes back (the welcome from
  the email step), Alt+Left is Back; the focus lands on the step's first
  field, and on the primary button on the welcome and finish.
- **Motion tokens**: `normal` 200, `gentle` 250, `slow` 300, `stagger`
  50 (each 0 ms with Animation effects off), `ease-out`
  cubic-bezier(0.1, 0.9, 0.2, 1), `ease-in` cubic-bezier(0.9, 0.1, 1,
  0.2). Only opacity and a ≤16 px offset on the column, never the window.

### Composer toolbar

A CommandBar (the pattern of "CommandBar / AppBarButton" above) in the
composer, one Tab stop with arrow-key roving focus, never clipped. Groups,
in order: font ComboBox (140) and size ComboBox (64); Bold, Italic,
Underline, Strikethrough (ToggleButtons exposing `accessible-checked`);
font colour (a SplitButton whose flyout is a 4x3 grid of named swatches,
Automatic first); bulleted and numbered list, decrease and increase indent;
alignment (a DropDownButton of four options); link, quote, table (a flyout
with a 10x10 hover grid labelled "{} x {} table" plus Rows and Columns
NumberBoxes for the keyboard); clear formatting. On the right: Signature (a
DropDownButton), High and Low importance, and More (...), which holds
Preview and "Switch to plain text" / "Switch to formatted text". Below
1040 px (or a composer under 940 px, which covers Portuguese) the font
group, the indents and the alignment move into More; under 700 px the
importance toggles follow, and under 560 px Quote and Clear formatting,
so the bar never clips. In plain mode only Signature, importance and More
remain. Komorebi's font and size boxes are the std ComboBox, each its own
Tab stop before the buttons' one; the Rows and Columns boxes are
ComboBoxes of 1 to 10. With the caret in a table the Table button's menu
offers insert row above and below, insert column left and right, and
delete row, column and table. Swapping to plain text asks first in a
ContentDialog and can be undone (Ctrl+Z, or switching back before
typing). Colours follow the system text colour in high contrast.

### RichEditBox mapping

Komorebi has no RichEditBox: `RichBody` is a TextBox-framed control whose
text, caret, IME and clipboard belong to a `TextInput` proxy (one line per
paragraph or table cell, U+FFFC for an atomic quote or logo), while the
document model (`src/mail/rich/`) owns the formatting. The page is painted
by Komorebi's own layout engine (`src/mail/rich/view.rs`, the reader's
parley text and tiny-skia painter) in pieces per block, and only the block
an edit changed is laid out again; the proxy is clipped to 1 px under the
painted caret (kept inside the part in view: a focused item scrolled out of
its clip stops taking keys), so the IME's candidate window opens there, and
what the IME is composing shows in a chip at the caret (the proxy's
`preedit-text`). The TextInput lays out all its text on every key, so a
message over 16 KB puts only the lines around the caret and the whole
selection in the proxy (Narrator reads that part); Ctrl+A, Ctrl+Home and
Ctrl+End go through the controller, and the part moves as the caret nears
its edge.
The caret is 1 px of the text colour, steady; the selection is the
SelectionHighlight colour under the text. Up, Down, Page Up and Down, Home
and End, clicks, drags and double clicks are worked out from the page's
layout. Frame, focus (2 px accent bottom edge), disabled and high-contrast
states are the TextBox's. Narrator reads the proxy text as "Message
body"; formatting is not announced, so the toolbar state is how to check
it.

### Signatures page

Settings > Signatures is a list of `SettingsCard` rows (name, a first-line
preview, Edit and Delete), an "Add signature" button and, when empty, a
sentence saying signatures start new mail and replies. Deleting asks in a
ContentDialog naming how many accounts use it as their default. The edit
page has a required, unique Name `TextBox`, a `RichBody` with a reduced
toolbar (no table, no quote), and "Add logo..." / "Remove logo" (PNG or JPEG
up to 200 KB, shown at most 400 x 150 px), then Save (accent) and Cancel;
Esc cancels. Komorebi's rows are `SettingsSurface` cards with the name in
Body Strong and the preview in Caption TextSecondary, Edit a standard
Button and Delete a subtle IconButton.

### Expander
MinHeight 48, header padding `16,0,0,0`, header CardBackgroundFillColorDefault
with a 1 px CardStrokeColorDefault border, radius 4 (ControlCornerRadius);
chevron button 32×32 with a 12 px glyph (E70D collapsed, E70E expanded),
margin `20,0,8,0`, transparent with SubtleSecondary / SubtleTertiary fills
on hover / press, foreground TextPrimary; content CardBackgroundFillColorSecondary
with a CardStrokeColorDefault border; HC chevron border 2
(`Expander_themeresources.xaml`).
Komorebi's invitation card uses only the chevron: Details is a subtle
button with the 12 px E70D / E70E glyph among the card's tools, the
description shown under them.
The account page's **Server settings** is the second
use, so it is a full `Expander` (`ui/widgets.slint`): a 48 px header row
on the card fill with a 1 px CardStroke edge, the header in Body
TextPrimary 16 from the left with an optional Caption TextSecondary line
under it, and the 32 x 32 chevron area at the end with the 12 px glyph;
the whole row is one Tab stop (Space or Enter toggles;
`accessible-expandable`, `accessible-expanded`). Expanded, the header's
bottom corners square off and the content shows under it on
CardBackgroundFillColorSecondary with the same edge, padding 16. The
widget is the header; the page lays the content out under it (a widget
with children is a second copy of its code). It opens by itself after
"Find settings" filled it and whenever a check fails on one of its
fields.

### ScrollBar
Thumb 2 px idle, 6 px expanded (12 px hit area), radius 3, min length 30,
ControlStrongFillColorDefault. Expands after 400 ms hover over 167 ms,
contracts after 2 s.
Komorebi's message list: Slint's own bar is off (`vertical-scrollbar-policy:
always-off`) and the thumb is drawn over the rows, 3 px from the edge,
only while the rows overflow; its length and place come from the rows'
total height (`rows × row-height`, not the ListView's estimate). It widens
to 6 px (167 ms, `Theme.fast`) while the pointer is on its 12 px strip, at
once rather than after 400 ms, and narrows when the pointer leaves; drag
moves it, a click beside it pages, the wheel scrolls. The thumb is 3.3:1
on the light card (#8d8d8d) and 5.7:1 on the dark one. The folder pane
uses the same bar (`ScrollThumb`), from its rows' laid-out height. The
reader keeps Slint's bar.

### Progress, PersonPicture, InfoBadge
- ProgressRing 32 (min 16), stroke 4, AccentFillDefault (HC Highlight).
  ProgressBar 3 px, radius 1.5, track 1 px ControlStrongStroke; paused
  Caution, error Critical. Both appear only after a wait passes 1 s
  (FLUENT2 "Wait UX"); with Animation effects off the ring is a still
  three-quarter arc.
- PersonPicture: ControlAltFillColorQuarternary, 1 px CardStroke,
  semibold initials in TextPrimary.
- InfoBadge: AccentFillDefault, TextOnAccentPrimary, height 16, 11 px text.

### Title bar
App-drawn on Windows 11 (WinUI `ExtendsContentIntoTitleBar` with the
expanded 48 px `TitleBarExpandedHeight`, as new Outlook and Mail draw it,
because it holds the search box). Windows 10 and high contrast keep the
system caption (32 px) plus the same 48 px in-client strip holding only the
search. "Title bar and frame" below says when each is used and how the frame
works.
- Background `layer`, so the bar and the folder pane read as one surface.
- Left to right: margin 16, the 16 px app icon, 12 px gap, "Komorebi" in
  Caption (TextPrimary), a `*` drag gap, the search box and its scope
  control centred (4*, 220-600 px), at least 48 px of drag gap, the three
  caption buttons. Below 1040 px the title collapses to make room for the
  search, which aligns left after the icon; with no search (no account
  yet) the title stays at every width.
- Height `max(48, search box height + 16)` at the current text scale
  (1.0-2.25); the icon and title stay centred in the first 48 px. The
  actual height is pushed to the hit test with the passthrough rects.
- Caption buttons: **46 × 48, top-aligned**, never scaled with text, 10 px
  glyphs E921 minimize, E922 maximize, E923 restore, E8BB close in
  `Theme.icon-font`; transparent at rest; hover SubtleSecondary, pressed
  SubtleTertiary with the glyph in TextSecondary; close hover #C42B1C with
  a white glyph, close pressed #C42B1C at 90 % with the glyph at 70 %
  white.
- Inactive window: glyphs TextDisabled, title TextTertiary, icon at 0.5
  opacity.
- No account yet, or the first-run flow still open (the welcome, its
  steps, and Settings opened from it):
  the bar keeps the icon, title and caption buttons but hides the search
  box and its scope control (there is nothing to search) and pushes no
  passthrough rectangle, so the whole bar between the title and the
  caption buttons drags the window. The search comes back when the
  first-run flow closes on the first account's Inbox.
- Accessibility: the caption buttons are `button`s named Minimize /
  Maximize or Restore / Close (pt-PT Minimizar / Maximizar / Restaurar /
  Fechar) whose default action sends the same system command. They are not
  Tab stops (as in Windows) but stay in the accessibility tree. The HWND
  title stays the window title (Alt+Tab, taskbar, Narrator).

### TabView (message and composer tabs)
From `controls/dev/TabView/TabView_themeresources.xaml` and `TabView.xaml`
at the pinned commit (`TabViewWidthMode` Equal, `CloseButtonOverlayMode`
OnPointerOver as Komorebi uses it).
- Item: MinHeight 32 (`TabViewItemMinHeight`), MinWidth 100, MaxWidth 240;
  padding `8,3,4,3` with a close button (`8,3,8,3` without), selected
  `9,3,5,4`; top corners OverlayCornerRadius (8), bottom square; border 1,
  the selected item's `1,1,1,0` in CardStrokeColorDefault, so it joins the
  content below. Header text **12 px** (`TabViewItemHeaderFontSize`,
  Caption), not Body. Icon 16 px with a 10 px gap after it (margin
  `0,0,10,0`).
- Close button: 32 × 24, glyph E711 at 12 px, margin `4,0,0,0`, subtle
  (transparent, hover SubtleSecondary, pressed SubtleTertiary), not a Tab
  stop, `IsTextScaleFactorEnabled=False` (it does not grow with the text
  size). Shown on the selected tab and under the pointer
  (OnPointerOver).
- Backgrounds: rest LayerOnMicaBaseAltFillColorTransparent (transparent
  on the layer), hover LayerOnMicaBaseAltFillColorSecondary, pressed
  LayerOnMicaBaseAltFillColorDefault, **selected
  SolidBackgroundFillColorTertiary** (Komorebi: the `card` the content sits
  on, so the tab and its page read as one). Text TextSecondary at rest and
  under the pointer, TextTertiary pressed, **TextPrimary selected**,
  TextDisabled disabled; icons the same.
- Separator: 1 px DividerStrokeColorDefault (`stroke`), margin `0,8,0,8`,
  hidden beside the selected tab and the one under the pointer. The strip's
  bottom line is TabViewBorderBrush (CardStrokeColorDefault), broken under
  the selected tab.
- Strip: padding `0,8,0,0` above the items (`TabViewHeaderPadding`); scroll
  buttons 32 × 24 (glyphs EDD9 / EDDA at 8 px) when the items overflow
  after shrinking to MinWidth. Komorebi has no add (+) button (New mail
  and Ctrl+N make composer tabs) and puts an "All tabs" chevron (E70D)
  menu at the strip's end instead of the scroll buttons.
- High contrast (`HighContrast` dictionary): strip ButtonFace; items
  ButtonFace with WindowText; **selected Window with Highlight text**;
  pointer-over and pressed Highlight with HighlightText; disabled GrayText.
  The close glyph ButtonText with a 1 px ButtonText border on hover,
  pressed and selected; on the selected tab it is Highlight. Separator
  WindowText; the strip's bottom line and the selected item's edge
  Highlight.
- Keys (WinUI TabView): Ctrl+Tab / Ctrl+Shift+Tab next / previous,
  Ctrl+1..8 tab N, Ctrl+9 the last, Ctrl+W / Ctrl+F4 close, Left / Right
  between tabs while the strip has the focus. A middle-click closes a tab.
- Accessibility: the strip is a `tab-list`, each item a `tab` named by its
  header (Komorebi adds "draft" or "unread" and "N of M"), the close button
  "Close tab", the content a `tab-panel`.

Komorebi (`TabStrip` in `ui/widgets.slint`): a full-width row under the
title bar that shows only while more than the fixed Mail tab is open. The
Mail tab is first, an icon only (E715), pinned, with no close button.
Message tabs show E715, draft tabs E70F (pencil); titles elide with "…"
inside the 100-240 px tab and never clip (Portuguese titles run longer).
Below 720 px only the Mail tab, the active tab and the "All tabs" menu
show. Heights follow the text size through the header text; the close
button keeps its 32 × 24.

## Following the user's Windows theme

Everything is read on the UI thread with plain Win32 (windows-sys), once
before the first frame and again on a change notification. No WinRT, no
thread, no polling, no new crate.

| Setting | Read with | Cost [verified] |
|---|---|---|
| Accent ramp | `RegGetValueW(HKCU, "Software\Microsoft\Windows\CurrentVersion\Explorer\Accent", "AccentPalette", RRF_RT_REG_BINARY)` | 17 µs cold, 5.5 µs warm |
| App mode | `RegGetValueW(HKCU, "Software\Microsoft\Windows\CurrentVersion\Themes\Personalize", "AppsUseLightTheme", RRF_RT_REG_DWORD)`, else HKLM `SystemUsesLightTheme`, else light | about 5 µs |
| High contrast | `SystemParametersInfoW(SPI_GETHIGHCONTRAST)` + 8 × `GetSysColor` | under 1 µs each |
| Text size | `RegGetValueW(HKCU, "Software\Microsoft\Accessibility", "TextScaleFactor", RRF_RT_REG_DWORD)` | about 5 µs |
| Animation effects | `SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, &mut BOOL, 0)` | under 1 µs |
| Accent on title bars | `RegGetValueW(HKCU, "Software\Microsoft\Windows\DWM", "ColorPrevalence", RRF_RT_REG_DWORD)` | about 5 µs |

All reads together stay well under 0.1 ms in the app (winit has already
loaded user32 and advapi32). New windows-sys features:
`Win32_System_Registry`, `Win32_UI_Accessibility` (bindings only).

### Accent colour and ramp
- `AccentPalette` is `REG_BINARY`, 32 bytes, 8 entries of `[R, G, B, A]`:
  Light3, Light2, Light1, **Accent**, Dark1, Dark2, Dark3, unused.
  [verified] identical to `UISettings.GetColorValue(AccentDark3..AccentLight3)`,
  the call WinUI's `SystemThemingInterop` makes. The key is undocumented but
  stable; if it is missing or not 32 bytes, use the fallback blue ramp.
  Which Windows 10 build added it [unverified].
- Rejected: WinRT `UISettings` through hand-declared `combase` imports (1.9 ms
  cold, COM vtable code; windows-sys has no `Win32_System_WinRT`), and
  `DwmGetColorizationColor` / `AccentColorMenu`, which are not
  SystemAccentColor (here #1c422e and #1f4833 against the real #26593f).
- Slint 1.18 reads `DwmGetColorizationColor` only at window creation and on
  a light/dark flip, then keeps only its hue and chroma
  (`accentify` in fluent `styling.slint`); there is no public setter. So
  `Palette.accent-*` and `Palette.selection-*` are approximations and
  Komorebi's own tokens must not use them.

### Light / dark
- Read exactly as WinUI's `SystemThemingInterop::GetSystemTheme`:
  `AppsUseLightTheme` == 0 is dark.
- Komorebi keeps its System / Light / Dark setting; under System the
  registry value decides, and `Palette.color-scheme` is set from it.
  winit's `ThemeChanged` (uxtheme ordinal 132, false under HC) is no
  longer used.
- Under high contrast the contrast theme wins over Light / Dark, as in
  WinUI (the HighContrast dictionary replaces `RequestedTheme`):
  `Palette.color-scheme` follows the Window colour (below).

### High contrast
- On when `HIGHCONTRASTW.dwFlags & HCF_HIGHCONTRASTON` (1). Read the 8
  colours in the HC table; convert COLORREF `0x00BBGGRR` to RGB.
- WinUI classifies HC as black, white or custom from Window and WindowText.
  Komorebi sets `Palette.color-scheme` dark when Window is dark, so std
  widgets stay readable.
- Komorebi's widgets switch to the HC column. Slint's std widgets have no
  HC palette (known gap). The HTML reader renders the page as authored
  when Window is light (hcwhite) and in its dark-theme dimming when
  Window is dark (hc1, hc2, hcblack), as `Palette.color-scheme` follows
  Window; page colours are not replaced with the HC ones (mail is
  content, as in a browser). Links Komorebi draws (HyperlinkButton,
  Unsubscribe) are Hotlight.

### Text size
- `TextScaleFactor` is a percent DWORD, 100..=225, absent = 100. WinUI scales
  **font sizes only**, never layout units or font icons, with
  `TextFormatting::GetScaledFontSize` (the code, not its comment):
  `out = s + max(18 − e·ln(s), 0) · (f − 1)`, `s = max(size, 1)`,
  `f = factor / 100`. At f = 2.25: 12 → 26.1, 14 → 27.5, 16 → 29.1,
  20 → 32.3, 28 → 39.2.
- Layout reflows: heights that hold text grow with it (Komorebi's
  `row-height` grows by the line-height increase, about 1.33 × the size
  increase; confirm with a snapshot). Icon glyph sizes stay fixed.
- WinUI listens through `UISettings.TextScaleFactorChanged`. Whether the
  slider also broadcasts `WM_SETTINGCHANGE` is [unverified]; if not, the
  value applies at next launch, or one blocking `RegNotifyChangeKeyValue`
  thread on `HKCU\Software\Microsoft\Accessibility` posts the change.

### Animation effects
`SPI_GETCLIENTAREAANIMATION`; changes arrive as `WM_SETTINGCHANGE` with
`wParam = SPI_SETCLIENTAREAANIMATION` (0x1043). Off means every duration is
0 ms: every `animate` in Komorebi's `.slint` files uses `Theme.fast` /
`Theme.faster`, and the ProgressRing / ProgressBar stand still. Residual:
the std `ScrollView` (reader) and `ComboBox` keep Slint's own fluent
animations, which do not see the setting.

### Title bar and frame
- **When the bar is app-drawn ("custom"):** Windows build ≥ 22000
  (`RegGetValueW(HKLM, "SOFTWARE\Microsoft\Windows NT\CurrentVersion",
  "CurrentBuildNumber")`), not high contrast, and no
  `KOMOREBI_SYSTEM_TITLEBAR`. Otherwise the system caption stays
  ("system"). High contrast switches it live (the appearance listener sets
  the mode, the bar and the frame in one pass, so no double bar is drawn).
  `KOMOREBI_SNAPSHOT_TITLEBAR=custom|system` forces a mode; snapshots
  default to custom (system under `KOMOREBI_THEME=contrast`).
- **Frame (custom):** winit stays decorated and Slint `no-frame` stays
  false (Slint would flip winit to undecorated otherwise). A second window
  subclass, `chrome_proc` (id 2, `src/chrome.rs`), handles
  `WM_NCCALCSIZE`: Def first, then the client top goes back to the window
  top (plus `frame_y(dpi)` = SM_CYFRAME + SM_CXPADDEDBORDER when
  maximized, and 2 px off an edge holding an auto-hide taskbar). The left,
  right and bottom borders stay native and invisible, so the shadow,
  rounded corners and the DWM border stay. It is installed from a
  thread-local `WH_CBT` hook on `HCBT_CREATEWND` of the captioned
  top-level window, so the first `WM_NCCALCSIZE` is already custom and no
  system caption is drawn before the bar (the first window event installs
  it as a fallback if the hook missed).
- Never call `DwmExtendFrameIntoClientArea`: softbuffer writes alpha 0, so
  the extended area would turn transparent.
- **Geometry:** winit converts every client size through
  `AdjustWindowRectExForDpi` with `WS_CAPTION`, while the custom frame
  gives the caption back to the client. `chrome_proc` takes that height
  (`-AdjustWindowRectExForDpi(..).top`) back off the window height in
  `WM_WINDOWPOSCHANGING` for winit's size requests (no move, outside a
  size/move loop, not `SWP_FRAMECHANGED`) and during `WM_DPICHANGED`, and
  off `ptMinTrackSize` in `WM_GETMINMAXINFO`. So a requested client size
  (restored geometry, `KOMOREBI_SIZE`, Slint's minimum) is the client size
  shown, and save → restore → save is stable.
- **Hit testing** (`WM_NCHITTEST`): Def first for the native side and
  bottom borders; HTLEFT / HTRIGHT inside the top band (`y < frame_y`, not
  maximized) become HTTOPLEFT / HTTOPRIGHT. Inside the client the pure
  `chrome::hit_test` (physical px, Slint's scale) answers: the top band
  HTTOP / HTTOPLEFT / HTTOPRIGHT (not maximized); the caption buttons by
  46 px column HTMINBUTTON / HTMAXBUTTON (Snap Layouts) / HTCLOSE; HTCLIENT
  over the passthrough rects (search box, scope control) and everywhere
  but the buttons while an in-window popup is open; HTCAPTION for the rest
  of the bar (drag, double-click, system menu, Aero Snap, touch); HTCLIENT
  below it. The lParam is decoded signed (`as i16 as i32`).
- **Caption buttons:** mouse input arrives as NC messages. Hover tracks
  with `TrackMouseEvent(TME_LEAVE | TME_NONCLIENT)`; `WM_NCLBUTTONDOWN` /
  `DBLCLK` on a button set it pressed and return 0 (never DefWindowProc,
  which would paint classic buttons); `WM_NCLBUTTONUP` on the same button
  posts `WM_SYSCOMMAND` (SC_MINIMIZE, SC_MAXIMIZE / SC_RESTORE, SC_CLOSE).
  Hover and pressed reach Slint through one coalesced
  `invoke_from_event_loop`; the proc never touches Slint or the app state
  (it can run under a native dialog's nested loop).
- **Passthrough rects** come from the bar's children (`absolute-position`
  and size, in `changed` handlers and at init) through
  `title-bar-regions`, into a thread-local; the proc never reads Slint.
- `DWMWA_USE_IMMERSIVE_DARK_MODE` (20) follows the effective theme in both
  modes (it colours the border and the system menu); FALSE under HC.
- **System caption (fallback):** `DWMWA_CAPTION_COLOR` (35),
  `DWMWA_BORDER_COLOR` (34), `DWMWA_TEXT_COLOR` (36) take a COLORREF;
  `DWMWA_COLOR_DEFAULT` = 0xFFFFFFFF, `DWMWA_COLOR_NONE` = 0xFFFFFFFE. Build
  22000+; on Windows 10 the call fails harmlessly. Call
  `DwmSetWindowAttribute` directly: winit's
  `set_title_background_color(None)` sends COLOR_NONE, not the default.
  Rule: not HC and `ColorPrevalence` = 0 → caption = `layer` colour,
  border default; `ColorPrevalence` = 1 → caption and border default (the
  accent title bar); HC → all default. (`Themes\Personalize\ColorPrevalence`
  is Start/taskbar, not this.) In custom mode `ColorPrevalence` is ignored,
  as WinUI 3, new Outlook and Mail do.
- Windows 11 draws its 1 px top border over the custom frame in the usual
  configurations. If a hand check finds it missing, the bar draws a 1 px
  `stroke` line at y = 0 while not maximized (the Terminal approach;
  `KOMOREBI_TITLEBAR_BORDER=1` shows it) and the hit test keeps that pixel
  in the top band.
- The minimum width is 500 px with a one-pane layout below 720 px
  ("Layout breakpoints"), so Snap Layouts halves and thirds fit common
  screens (1366 / 2 = 683, 1920 / 3 = 640); winit passes Slint's minimum
  to `WM_GETMINMAXINFO`, whose height alone the chrome proc corrects.
- Window corners: leave `DWMWA_WINDOW_CORNER_PREFERENCE` at default.

### Secondary windows
Komorebi's read-only message windows ("Open in new window", Ctrl+O):
- **System title bar.** They keep the system caption (Komorebi's own mode
  for Windows 10 and high contrast, above), coloured for dark mode and the
  accent caption with `DwmSetWindowAttribute` like the main window, with
  the same icon. The app-drawn bar stays on the main window only (later:
  L2).
- **Placement.** A new window cascades 32 px right and down from the window
  it was opened from, at 900 × 760 by default (the last size used,
  `popout_size`), kept on that window's monitor.
- **Closing.** Esc, Ctrl+W or the caption's close closes the window.
  Closing the main window closes every message window first and then ends
  the app, so a message window never outlives it (no background mode). A
  second start brings the main window forward (only it is marked).
- **Following the theme.** Accent, light / dark, high contrast, text size,
  animation effects and the language switch live in every window: each has
  its own `Theme`, `Tr` and `Palette` globals, pushed when it opens and on
  every change.

### Mica
Not used: Komorebi ships the solid fallback below. With the software renderer every client pixel is opaque, so a
backdrop alone shows nothing; showing it needs a transparent root, the frame
extended into the client area and softbuffer's `BitBlt` to keep alpha
[unverified]. Komorebi uses the solid Mica fallback (layer #F3F3F3 /
#202020), which is exactly what WinUI renders when Mica is unavailable. A
later experiment may sit behind a `KOMOREBI_MICA=1` dev switch
(`DWMWA_SYSTEMBACKDROP_TYPE` 38 = `DWMSBT_MAINWINDOW` 2, build 22621+), shipped
only if text and partial repaints look right to the user.

### Live updates
- winit's `with_msg_hook` sees only posted messages, and these are **sent**,
  so Komorebi installs `SetWindowSubclass` (windows-sys `Win32_UI_Shell`,
  already enabled) on the winit HWND once, from the first window event.
- The subclass proc calls `DefSubclassProc` **first** (winit's own
  `WM_SETTINGCHANGE` handling resets the dark title bar), then on
  `WM_SETTINGCHANGE`, `WM_DWMCOLORIZATIONCOLORCHANGED`, `WM_SYSCOLORCHANGE`
  or `WM_THEMECHANGED`:
  1. re-applies the title-bar attributes from a `thread_local` of the
     desired values (plain DWM calls, no app borrow);
  2. coalesces the burst with a `thread_local` pending flag and one
     `slint::invoke_from_event_loop` that re-reads everything and pushes to
     Slint only what changed. Never call `with_app` or set Slint properties
     inside the window procedure: it can run under a native dialog's nested
     loop while the app state is borrowed.
  3. on `WM_NCDESTROY`, `RemoveWindowSubclass`.
- No lParam filtering: a full re-read costs about 60 µs and unrelated
  broadcasts change nothing visible.
- Startup reads once after `AppWindow::new()` and before `run()`, so the
  first frame has the right colours.
- If the app state is borrowed when the re-read runs (a native dialog's
  nested loop), it is retried 250 ms later rather than dropped.

### Windows 10
| Item | Behaviour |
|---|---|
| AccentPalette, AppsUseLightTheme | Present on supported builds; fall back to the blue ramp / HKLM / light |
| High contrast, animation effects | Same APIs |
| TextScaleFactor | Absent = 100 |
| Dark title bar (attribute 20) | Works on current builds |
| Caption colour, system backdrop | Not supported; the system caption stays |
| Segoe UI Variable, Segoe Fluent Icons | Absent unless installed: Segoe UI and Segoe MDL2 Assets (picked at startup) |

## Mapping to Komorebi

`ui/theme.slint` is the only place these values live. Komorebi's surfaces:
the folder pane sits on the **layer** (Mica fallback); the message list and
reader sit on one **card** = LayerFillColorAlt over the base (#ffffff /
#2b2b2b), chosen over LayerFillColorDefault (#f9f9f9 / #282828) because mail
bodies are authored for white and ContentDialog's top part uses the same
resource. Under HC both are Window.

### Inputs from Rust
New `in` properties, fed from the reads above (and the dev switches below):
- `ramp`: 7 colours, Dark3..Light3 (index 3 = SystemAccentColor).
- `contrast: bool` plus `hc-window`, `hc-text`, `hc-highlight`,
  `hc-highlight-text`, `hc-hotlight`, `hc-gray`, `hc-button`,
  `hc-button-text`.
- `animate: bool`; durations are `animate ? d : 0ms`.
- Scaled type sizes (`caption`, `body`, `subtitle`, `title`, `title-large`)
  computed in Rust with the text-scale formula; `row-height` derives from
  them in `theme.slint` (76 px plus 4/3 of the size gained by its two Body
  lines and one Caption line: 131 px at 225 %); `AppWindow.default-font-size:
  Theme.body` so std widgets scale too.
- `display-font`: "Segoe UI Variable Display" where SegUIVar.ttf exists,
  else "Segoe UI".
- `dark` stays `Palette.color-scheme == dark`, set by the Rust side.

### Tokens
`ui/theme.slint` writes the translucent values below as Slint `#rrggbbaa`
(XAML's alpha byte moved last: `#E4000000` is `#000000e4`); pasting the XAML
order unchanged gives wrong colours (`#e4000000` is invisible red). A test
checks two of them.

| Token | WinUI resource | Light | Dark | HC |
|---|---|---|---|---|
| `layer` | SolidBackgroundFillColorBase | #f3f3f3 | #202020 | Window |
| `card` | LayerFillColorAlt on base | #ffffff | #2b2b2b | Window |
| `card-fill` (settings cards) | CardBackgroundFillColorDefault | #b3ffffff | #0dffffff | Window |
| `flyout` (menus, tooltips, drop-downs, overlay pane) | Acrylic solid fallback | #f9f9f9 | #2c2c2c | Window |
| `scrim` | SmokeFillColorDefault | #4d000000 | #4d000000 | Window at 0.8 |
| `stroke` (dividers, card edge) | DividerStrokeColorDefault | #0f000000 | #15ffffff | WindowText |
| `card-stroke` (content-area and card outline) | CardStrokeColorDefault on layer (opaque: Slint draws a border over its own fill) | #e5e5e5 | #1d1d1d | WindowText |
| `card-edge` (InfoBar border, the line above a dialog's button bar) | CardStrokeColorDefault | #0f000000 | #19000000 | WindowText |
| `stroke-strong` | ControlStrokeColorSecondary | #29000000 | #18ffffff | ButtonText |
| `stroke-accessible` (input bottom edge, checkbox, toggle off) | ControlStrongStrokeColorDefault | #72000000 | #8bffffff | ButtonText |
| `control-stroke` (button sides) | ControlStrokeColorDefault | #0f000000 | #12ffffff | ButtonText |
| `control-stroke-edge` (button bottom in light, top in dark) | ControlElevationBorder edge | #29000000 | #18ffffff | ButtonText |
| `flyout-stroke` | SurfaceStrokeColorFlyout | #0f000000 | #33000000 | WindowText |
| `surface-stroke` (dialogs) | SurfaceStrokeColorDefault | #66757575 | #66757575 | WindowText |
| `hover` | SubtleFillColorSecondary | #09000000 | #0fffffff | Highlight |
| `tab-hover` (a tab under the pointer) | LayerOnMicaBaseAltFillColorSecondary | #0a000000 | #0fffffff | Highlight |
| `tab-pressed` | LayerOnMicaBaseAltFillColorDefault | #b3ffffff | #733a3a3a | Highlight |
| `tab-selected` (joins the content card) | SolidBackgroundFillColorTertiary (Komorebi: `card`) | #ffffff | #2b2b2b | Window |
| `tab-strip` | TabViewBackground | transparent | transparent | ButtonFace |
| `tab-text` / `tab-text-selected` | TextFillColorSecondary / Primary | `text-2` / `text` | `text-2` / `text` | WindowText / Highlight |
| `pressed` | SubtleFillColorTertiary | #06000000 | #0affffff | Highlight |
| `selected` | SubtleFillColorSecondary | #09000000 | #0fffffff | Highlight |
| `nav-selected` | SubtleFillColorSecondary on layer | #eaeaea | #2d2d2d | Highlight |
| `on-selected` (text and icons on any `hover` / `pressed` / `selected` fill: rows, folders, tabs, subtle buttons; `on-fill()`) | TextFillColorPrimary | = `text` | = `text` | HighlightText |
| `control` / `control-hover` / `control-pressed` | ControlFillColorDefault / Secondary / Tertiary | #b3ffffff / #80f9f9f9 / #4df9f9f9 | #0fffffff / #15ffffff / #08ffffff | ButtonFace |
| `control-alt` (toggle and checkbox off) | ControlAltFillColorSecondary | #06000000 | #19000000 | ButtonFace |
| `input-active` (focused input) | ControlFillColorInputActive | #ffffff | #b31e1e1e | ButtonFace |
| `text` | TextFillColorPrimary | #e4000000 | #ffffff | WindowText |
| `text-2` | TextFillColorSecondary | #9e000000 | #c5ffffff | WindowText |
| `text-3` | TextFillColorTertiary | #72000000 | #87ffffff | WindowText |
| `text-disabled` | TextFillColorDisabled | #5c000000 | #5dffffff | GrayText |
| `text-inverse` | TextFillColorInverse | #ffffff | #e4000000 | WindowText |
| `accent` | AccentFillColorDefault | Dark1 | Light2 | Highlight [inferred] |
| `accent-hover` / `accent-pressed` | AccentFillColorSecondary / Tertiary | `accent` at 0.9 / 0.8 | same | ButtonText / Highlight |
| `on-accent` / `on-accent-hover` | TextOnAccentFillColorPrimary | #ffffff | #000000 | HighlightText / ButtonFace |
| `on-accent-2` (pressed label) | TextOnAccentFillColorSecondary | #b3ffffff | #80000000 | HighlightText |
| `accent-edge` / `accent-edge-bottom` | AccentControlElevationBorder sides / bottom | #14ffffff / #66000000 | #14ffffff / #23000000 | WindowText |
| `accent-disabled` | AccentFillColorDisabled | #37000000 | #28ffffff | ButtonFace, GrayText border |
| `on-accent-disabled` | TextOnAccentFillColorDisabled | #ffffff | #87ffffff | GrayText |
| `accent-text` (links, accent labels) | AccentTextFillColorPrimary | Dark2 | Light3 | WindowText (Hotlight for links [inferred]) |
| `accent-text-hover` / `accent-text-pressed` | AccentTextFillColor Secondary / Tertiary | Dark3 / Dark1 | Light3 / Light2 | Hotlight |
| `selection-bg` / `selection-text` | AccentFillColorSelectedTextBackground | Accent / #ffffff | Accent / #ffffff | Highlight / HighlightText |
| `focus` | FocusStrokeColorOuter (2 px) | #e4000000 | #ffffff | WindowText |
| `focus-inner` | FocusStrokeColorInner (1 px) | #b3ffffff | #b3000000 | Window |
| `on-avatar` (the reader's sender initials) | none: the initials sit on the sender's Fluent 2 persona colour (FLUENT2 "Colour", shared palette for avatars; `format::avatar_color`), not on WinUI's PersonPicture fill (ControlAltFillColorQuarternary with TextPrimary initials, used only under HC: Window fill, WindowText ring) | #ffffff | #ffffff | WindowText |
| `danger` / `danger-bg` | SystemFillColorCritical (+Background) | #c42b1c / #fde7e9 | #ff99a4 / #442726 | WindowText / Window |
| `warning` / `caution-bg` | SystemFillColorCaution (+Background) | #9d5d00 / #fff4ce | #fce100 / #433519 | WindowText / Window |
| `success` / `success-bg` | SystemFillColorSuccess (+Background) | #0f7b0f / #dff6dd | #6ccb5f / #393d1b | WindowText / Window |
| `info-bg` | SystemFillColorAttentionBackground | #80f6f6f6 | #08ffffff | Window |
| `radius` / `radius-lg` | ControlCornerRadius / OverlayCornerRadius | 4 / 8 | | |
| `caption` / `body` | Caption / Body | 12 / 14 (scaled) | | |
| `subtitle` | Fluent web Subtitle 2 (FLUENT2 allows it where Windows has no step); headers in lists use Body Strong 14 | 16 (scaled) | | |
| `title` / `title-large` | Subtitle / Title (Windows ramp), in `display-font` | 20 / 28 (scaled) | | |
| `mono-font` | none (Show original's header fields and source) | Consolas | | |

No shadows anywhere: dialogs, menus and the overlay pane are outlined by
their 1 px stroke alone (FLUENT2 "Elevation").

Rules that follow:
- `accent-text`: keep `readable()` as a safety net for odd accents; with the
  real ramp it should rarely change anything.
- **`text-3` in light is 3.3:1**, below FLUENT2's 4.5:1. Use it only for
  non-essential text that WinUI also puts in Tertiary (inactive title,
  hints next to stronger labels, large or semibold text); row metadata,
  dates and placeholders use `text-2` (WinUI placeholders are Secondary).
- Selected message row: `selected` backplate inset `4,2` (radius 4) plus a
  3 px `accent` pill at x = 4, centred, `max(16, row − 40)` tall; text keeps
  its normal colours. Selected folder: `nav-selected` plus a 3×16 pill. Both
  are HC Highlight with HighlightText.
- Transparent tokens are composited by Slint on the surface below; the
  precomposited table above is for checks and for places that need an
  opaque fill (`nav-selected`).
- Under `contrast`, every token takes its HC column and no translucent
  colour remains; borders are drawn on every control.
  Wherever `hover`, `pressed` or `selected` is the fill, the text and icons
  on it use `on-selected` (HighlightText), never `text`; a hovered accent
  button's label is ButtonFace.

### Residual `Palette.*` uses
None in `ui/app.slint` or `ui/widgets.slint`. Since
After the native round the std `LineEdit`, `CheckBox` and `Spinner` are gone (Komorebi's
`TextField`, `CheckBox`, `ProgressRing`), and since the final polish pass
the Settings signature is a named-signature picker (a `ComboBox`, see
"Composer toolbar" and "Signatures page"); only the std `ComboBox` still
draws Slint's derived accent (in its open list) and has no HC palette
(accepted near match; `ComboEdge` gives every one its
edge).

### Components (`ui/widgets.slint`)
Each follows the section named, with its HC column:
- `FocusRing` ("Control metrics"): 2 px `focus` + 1 px `focus-inner`,
  only while `Focus.keyboard` (set by a Tab arrival, cleared by every
  pointer press); `inset` −3 outside (buttons, menu items), +1 inside
  (list and folder rows), −2 for SelectorBar items.
- `Button` ("Button", "ToggleButton"): standard; `accent: true` is the
  accent button and `checkable` + `checked` the ToggleButton. There is
  no `AccentButton` component: in Slint a component that inherits
  another is a second full copy of its code.
  `tooltip` shows after 500 ms of hover (also while disabled) and is the
  accessible name when there is no label (the compact New mail: "New
  mail (Ctrl+N)"); `description` is the accessible description.
- `IconButton`: the subtle button (icon `text`, pressed `text-2`,
  `checkable` accent fill), at least 32 × 32. Under HC it keeps a 1 px
  edge: ButtonText at rest (WinUI's `SubtleButtonBorderBrush` is
  transparent there; Komorebi follows "every control keeps its stroke"),
  Highlight hovered, HighlightText pressed, GrayText disabled
  (`Button_themeresources.xaml`).
- `ComboEdge`: drawn over every std `ComboBox` (the composer's From, the
  General choices and the account form's Provider and IMAP / SMTP
  security), the Button's elevation edge, or under HC the 1 px
  ButtonText border Slint's style lacks.
- `FieldButton` (a TextField's clear / reveal and the SearchBox's query
  glyph) keeps WinUI's 28 × 24 plate; its pointer area is 32 × 32, the
  field's full height (FLUENT2 "Layout and spacing").
- `ContentDialog`'s card is a `groupbox` named by its title and described
  by its message, so a screen reader names the question with the focused
  button (Slint 1.18's `AccessibleRole` has no dialog). Live regions:
  the status line (polite, assertive for Caution and Critical), the
  account form's sign-in error and the send guards' bar (assertive),
  "Signing in…" and Send's reason (polite).
- `RadioButtons` ("CheckBox / RadioButton / ToggleSwitch": a group with
  descriptions, `selected` -1 for none), `Expander` ("Expander": the
  header row; the page draws the content), and `TextField`'s `error` and
  `description` ("TextBox", error state).
- `HyperlinkButton` ("HyperlinkButton"), `TextField` ("TextBox", with the
  12 px clear or reveal glyph button), `CheckBox` and `ToggleSwitch`
  ("CheckBox / RadioButton / ToggleSwitch"), `SettingsSurface` and
  `SettingsCard` ("SettingsCard"), `InfoBar` ("InfoBar"; one
  action through `action-text` and up to two content buttons below the
  message through `button1` / `button2`, `stack-buttons` putting them one
  under the other), `ContentDialog` ("ContentDialog"; its
  message, a caption `detail` and `warnings` are properties),
  `ProgressRing` / `ProgressBar` ("Progress", shown after 1 s), `SelectorBar`
  ("SelectorBar"; `picked` reports a click or key, so a bar bound to a
  Rust-owned value such as the list filter follows it), `Flyout` /
  `FlyoutItem` ("MenuFlyout / ToolTip / Flyout").
- `Chip` (attachments): a Button's fills (`control` / `control-hover` /
  `control-pressed`) with a 1 px DividerStroke, radius 4, `chip-height`
  tall (40 px, taller with the text size), a 16 px icon by file type
  (`format::file_kind`), the name in Body TextPrimary and the size in
  Caption TextSecondary, and one subtle action (under HC, hover is a
Highlight border, as on a Button); the reader lays them in a
  wrapping grid, 180 to 280 px each, 8 apart, above the page and under the
  invitation card (no `if`: 0 px tall without attachments).
- Hidden instead of `if`: the header's three date texts (From line, To
  line, own line) and the measuring texts of the command bar are
  `visible: false` when not used; Slint marks hidden items hidden for
  screen readers (accesskit `set_hidden`), and the measuring texts also
  have `accessible-role: none`.
- Reader focus: the focused link on the page gets the double focus ring
  with radius 2 (2 px FocusStrokeColorOuter, 1 px FocusStrokeColorInner).
  A plain-text body stays a Tab stop (its text can be selected from the
  keyboard) and shows the outside ring while focused after a key; like
  the page's own ring, it relies on the last key or click elsewhere
  (`Focus.keyboard`), as a `TextInput` reports no focus reason.
- `MailItem` builds its quick actions (`QuickAction`: 28 px, subtle
  fills, a tooltip and an accessible name, no FocusScope, so never a Tab
  stop) and its check box only on the rows that show them (`if`); the
  Undo offer at the bottom of the list and the Move flyout's `MoveList`
  (filter `TextField`, "Recent", the indented tree in a 360 px
  `Flickable`) live in ui/app.slint.
- `MailItem` ("ListView item") and `SearchBox` ("AutoSuggestBox", in the
  title bar with its `ScopeBox`). The list header: title ("Search
  results" while a query runs), a 16 px ProgressRing ("Syncing…", after
  1 s), Refresh and More, centred by the row's padding; the SelectorBar
  All / Unread / Important in folder views; while searching, the results
  line (Caption TextSecondary, then the Search all folders and Clear
  search HyperlinkButtons, their text in line with the heading). An empty
  list shows, centred over it, a 32 px glyph in TextDisabled, its reason
  in Body TextSecondary (while searching, also what is searched, in
  Caption) and at most one HyperlinkButton (Show all; while searching the
  results line has the ways on). While the account shown is in its first
  download (just added) the empty list says "Downloading your mail…
  1,234 of 5,000. Newest messages arrive first." over a 200 px
  ProgressBar (determinate once the first count arrives), and rows fill
  in above it as they come. Once a sync pass has completed, an empty
  folder says "This folder is empty"; "Nothing here" stays for views that
  are not one folder.
- `TitleBar` (inline in app.slint), `CaptionButton` and `ScopeBox`
  ("Title bar", "AutoSuggestBox"): the bar pushes its height and the
  search and scope rectangles (`BarRect`) through `title-bar-regions`.
- The composer: labels in TextSecondary in a column `max(64, the widest
  label measured)` wide, so it grows with the text size; From is the std
  ComboBox with a Button's elevation edge drawn over it; To, Cc, Bcc,
  Subject and the link field are `TextField`s and the body is
  `ComposeBody` (plain mode) or `RichBody` (formatted mode) in the same frame (sides ControlStroke, bottom
  ControlStrongStroke, 2 px accent while focused), every field ending at
  the same edge: Cc and Bcc are HyperlinkButtons inside the To field's
  end. The header: the title (Subtitle) with "Draft saved 14:05" under it
  (Caption, TextSecondary; every save refreshes it, none before the first
  save), then Attach (subtle; icon-only first when the window is narrow),
  Discard (subtle, always labelled), Send (the accent `Button`, always
  labelled) and Close (a 32 px subtle dismiss button, tooltip "Close
  (Esc)", which parks the draft as Esc does). **Send is always enabled**
  (new Outlook): pressed with no recipient or an invalid address, the
  field at fault shows its FieldError (Caption, SystemFillColorCritical,
  under the field; To's is an assertive live region) and takes the focus;
  why it would fail stays its accessible description, never a line that
  moves the fields. Recipient suggestions are the ComboBox drop-down
  ("ComboBox": flyout surface, items margin 5,2, padding 11,5,11,7, radius
  3, the current one SubtleSecondary with the 3x16 pill), drawn over the
  composer 4 below the field typed in and as wide, not a popup (the field
  keeps the keyboard). The formatting row is the "Composer toolbar" below:
  subtle 32 px buttons 4 apart, the quote is E9B2 (Ctrl+Shift+Q), "1." is
  Body semibold text (E9D5 is CheckList in both
  Segoe Fluent Icons and Segoe MDL2 Assets, checked by rendering it;
  neither font has a numbered-list glyph, so the text stays), 1x20
  DividerStroke separators. **Preview** is in More (a checkable item):
  while on, the body's frame shows the HTML part as sent, rendered by
  `mail::render` off the UI thread on its own canvas colour (the reader's
  dim in the dark theme), 1 px ControlStrongStroke edge, radius 4,
  scrolled as the body, laid out at the column's width up to 960 and
  never stretched; drawn at once when turned on and again 150 ms after a
  resize or theme change settles; focusing the body (Tab, a click) turns
  Preview off; a Caption line under it says when only the beginning fits
  (one 4096-row band) or drawing failed. A reply's or forward's
  quote is folded behind a "Show quoted text" HyperlinkButton under the
  body (new Outlook's "..."), and its attribution or forwarded heading is
  in the language shown; the fold takes the blank lines above the heading
  with it, so a fresh reply's body is empty and shows the placeholder
  "Write your message", and text that replaces the whole body (Ctrl+A,
  then typing) is still a blank line away from the quote in what is
  saved and sent. The link bar's Insert is the accent button (Enter), Cancel
  subtle. The send guards' bar is a Caution InfoBar with "Send anyway";
  the Discard prompt is a destructive ContentDialog (both buttons
  standard, Cancel focused) whose text is the one discard sentence ("It is
  deleted here and from your Drafts folder on the server, with its
  attachments."), also used by the Outbox's Discard. The Outbox entry: a
  command bar where the main action is the accent Button by failure class
  (a permanent refusal: Edit accent, Retry subtle; a temporary one or an
  unsure send: Retry accent, Edit subtle; Discard always subtle), then the
  subject, To, an InfoBar (Critical when the send failed or may not have
  gone, else Informational) whose message is a plain sentence for the
  SMTP class (5.1.1: "{address} doesn't exist. Check the address and send
  again."), an Expander "Details" with the server's reply as it came
  (Caption, TextSecondary, in the Expander's content card), and the
  first lines of the text (Body); such rows carry a 12 px Critical error
  glyph and the same sentence as their preview. A failed send's status
  line does not time out. The status line's action is a standard Button
  with text only; the undo-send offer names the message ("Sending
  "Plans" in 9 s", the subject cut at 40 characters) and counts down in
  the bottom Undo slot, and a delivery says "Sent" (FLUENT2 "Wait UX").
  A message with a reply in progress shows an Informational InfoBar in
  the reader, "You started a reply to this message.", with Resume.
- **Drag and drop**: files dragged over the window cover the reader /
  composer column with LayerFillColor at 92 % and an inset (12) 2 px
  accent (HC Highlight) rounded (8) frame holding the attach glyph (32,
  accent) and "Drop to attach" (Subtitle semibold), or "Drop to attach
  to a new message" when no composer is open (the drop opens one). Not
  shown without an account or over Settings.
- The status line: an `InfoBar` ("InfoBar") at the top of the list
  (under the filters, 12 from the card's sides, above the divider) and of
  the Settings page column, never over a row (FLUENT2 "Wait UX": attached
  to its content). Severity Informational (5 s), Critical (12 s) or
  Caution for a server that can't be reached or is at its connection
  limit: that one has no timer, stays until the account syncs again or
  the bar is closed, and comes back when a later line (or an Undo offer,
  which it never replaces) goes. Its action ("Undo", "Resume draft",
  "Update password") is the bar's content button under the message, as
  WinUI places it when the bar is narrow (the list is 300 or 380 px);
  the message breaks a word longer than the bar (an address at a large
  text size) so the bar never outgrows its column. A polite live region,
  assertive for Caution and Critical. A download's progress is not on
  it while Settings shows (the account's card has its bar: one
  indicator).
- `InfoBar` draws its action and close buttons as plain elements, hidden
  and disabled when unused, not as `if`s: with five bars the `if`s cost
  12 KB.
- `TeachingTip` ("TeachingTip"): the card only (title, text, place,
  Got it, Skip tips; the place takes its own line when the three do not
  fit). `AppWindow` places the one tip shown (`tip-kind`) beside its
  target under an `if` (no code in the first frame), blocks tooltips
  through `Tip.blocked` and takes Esc in `keys`' capture handler.
- The ToolTip ("MenuFlyout / ToolTip / Flyout") is one overlay in
  `AppWindow` fed by the `Tip` global: under its control (above it at the
  window's bottom), centred, clamped to the window, no fade. The 500 ms
  delay is Win32's default initial tooltip delay (`TTDT_INITIAL`, the
  double-click time) [inferred]; WinUI's own delay was not read.
- Binary size: an element with children, or the element of an `if` or
  `for`, gets its own copy of the component's code. So dialogs and info
  bars take their content as properties, and a widget under an `if` sits
  in a layout (`if c: HorizontalLayout { Button { … } }`), where it stays
  one shared component. Inside a widget, an `if` is code of its own too, once
  for every place the widget is copied: `NavRow` draws its pill,
  separator, icon and dots as plain elements with transparent colours
  (removing five `if`s and merging two saved 27 KB), and the pane toggle and Settings
  share one `NavRow` repeater (an `IconButton` toggle alone was 10.7 KB).
  The same holds for repeaters inside a shared widget: a `for` of
  Buttons in a GridLayout inside `InfoBar` cost 25 KB, two fixed Buttons
  placed by `x` / `y` almost nothing; and a 12 px chevron as an `if` in
  `IconButton` cost 13.8 KB, so Move draws its chevron over the button.

### Dev switches (snapshots)
Read through `komorebi::dev_var` (`src/theme.rs`), over the Windows
values: `KOMOREBI_ACCENT=#rrggbb` (approximate ramp: the light steps mix
the accent 25/45/65 % towards white, the dark ones 22/45/69 % towards
black) or 7 comma-separated colours Dark3..Light3 (exact);
`KOMOREBI_THEME=contrast` with optional `KOMOREBI_CONTRAST=` a preset
(`hc1`, `hc2`, `hcblack` (default), `hcwhite`, read from
`C:\Windows\Resources\Ease of Access Themes\*.theme`) or 8 colours
(Window, WindowText, Highlight, HighlightText, Hotlight, GrayText,
ButtonFace, ButtonText); `KOMOREBI_TEXT_SCALE=1.5` (1.0..2.25);
`KOMOREBI_ANIMATE=0`; `KOMOREBI_SNAPSHOT_HOVER=x,y` (`:press` holds the
left button); `KOMOREBI_SNAPSHOT_ACTION=menu:list`, `menu:pane` and
`menu:rail` print a menu's entries and enabled state to stderr and exit
(no native popup is ever clicked in a snapshot run); `menu:row` (the row
menu on what is selected), `menu:more` (the reader's More menu) and
`menu:move` (the Move flyout's entries, recent first, with their depth)
likewise, each item with its accelerator text; `menu:scope` the search
scope menu; `menu:account` the account page's "…" menu (Move up / Move
down); `notice-why` the sender notice's "Why?" dialog;
`remote-always-bar` the "Images from … will always load" bar with Undo;
`settings-edit-folders` scrolls the account page to Folders; `offline`
also shows the caution badge and Try now; `select-mode` turns "Select messages" on, `move` opens the
Move flyout, `delete-confirm` asks the Trash / Junk question,
`not-downloaded` shows the offline body state; `fresh-inbox` adds a
made-up account to the throwaway cache (offline only) and shows its Inbox
in its first download (1,234 of 5,000); `ob-error-rejected` the onboarding
Sign in step after a rejected password (the InfoBar, Set username);
`compose-invalid` and `compose-no-to` press Send on a wrong or missing
recipient (the field's error), `compose-preview` shows Preview on,
`compose-hint` the one-time formatting caption (offline), `drop-hover` and
`drop-hover-compose` the drop overlay, `draft-reply` the reader's "You
started a reply" bar (offline), `outbox:permanent` and `outbox:transient`
a made-up failed send in the Outbox (offline; 550 5.1.1 or 421);
`tips`, `tips:folders`, `tips:search` and `tips:help` show a teaching tip
(its place as in the series; `tips:folders` at 1040 px or wider prints
"tip not applicable at this width" and exits 0), `whats-new` the What's
new bar and `shortcuts` the Keyboard shortcuts dialog;
`KOMOREBI_SNAPSHOT_ACTION=inactive` draws the window as
inactive; `KOMOREBI_SNAPSHOT_HOVER=caption-min|caption-max|caption-close`
(`:press` for pressed) the caption button states;
`KOMOREBI_SNAPSHOT_TITLEBAR=custom|system` the title bar mode (the pushed
bar height and passthrough rects are printed to stderr as `[titlebar]`,
and the breakpoint in effect as `[layout] width=… mode=wide|compact|one-pane
list=… reader=…`). Known exact ramps: green `#0c1c14,#173525,#1e4732,#26593f,#2e6a4b,#357c58,#40956a`,
WinUI blue `#002642,#004275,#005a9e,#0078d7,#429ce3,#76b9ed,#a6d8ff`.

## Open checks

Status: the startup read was confirmed on this machine (no
switches: dark apps and the green ramp's Light2 #357c58 on New mail, the
`AccentPalette` values above); the rest needs a person at the machine,
since snapshots render only the client area and send no system messages.

1. Does the text-size slider broadcast `WM_SETTINGCHANGE`? (Move it with the
   app open.) Not checked. The listener re-reads every setting on any
   `WM_SETTINGCHANGE`, so if it does, text size follows live; otherwise at
   the next start.
2. Does DWM keep `DWMWA_CAPTION_COLOR` on an inactive window and dim only
   the title text? Not checked (the caption is outside snapshots); the
   caption rule is implemented as written above.
3. Is `AccentPalette` written before the last message of an accent-change
   burst? Not observed live; the re-read runs from the event loop after the
   burst's first message is handled, and every later message of the burst
   that arrives after it posts another.
4. HC AccentButton (Highlight / HighlightText) and link (Hotlight) mappings
   are [inferred] from the legacy `SystemControl*` brushes; confirm in
   `generic.xaml`. The first round uses Highlight / HighlightText for `accent` /
   `on-accent` and WindowText for `accent-text`.
5. HC rendering of Slint's std `ComboBox` and `TextEdit` (the only std
   inputs left after the native round): with `Palette.color-scheme` from the Window
   colour they keep Slint's light or dark palette (seen in hcblack /
   hcwhite snapshots); no HC colours. `ComboEdge` gives every ComboBox a
   ButtonText border (the account form's since the final polish pass);
   the signature box has since been replaced by the account defaults'
   ComboBoxes (`ComboEdge`) and the Signatures page's `RichBody`.
6. Mica only if pursued: does softbuffer's `BitBlt` keep alpha on screen?
   Not pursued (Deferred `KOMOREBI_MICA=1`).

Status: 1, 2 and 3 are still open and can only be
closed by a person at the machine (TODO.md "Windows 11 look"),
 4 stays [inferred]: `generic.xaml` was not re-read,
and the hcblack / hcwhite snapshots show the mapping as implemented
(accent fills Highlight with HighlightText, links Hotlight). 5 is an
accepted gap: the General and From ComboBoxes have
the ButtonText edge, the account form's ComboBoxes and the signature
`TextEdit` keep Slint's faint one. 6 is not pursued.

Final polish pass (2026-10-12): 5 is closed as far as Komorebi can close
it. The account form's three ComboBoxes have `ComboEdge` and the
signature box was a `ComposeBody` then (since replaced, see "Signatures page"); what remains is
the std ComboBox's own open list and animations (accepted).
