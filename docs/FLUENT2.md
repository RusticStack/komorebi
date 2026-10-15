# Fluent 2 reference for Komorebi

The design source of truth for all UI work. Distilled on 2026-10-09 from
the official site and the official token source; read the pages
themselves when in doubt:

- https://fluent2.microsoft.design/ (and /get-started/design)
- https://fluent2.microsoft.design/design-principles
- https://fluent2.microsoft.design/color
- https://fluent2.microsoft.design/elevation
- https://fluent2.microsoft.design/iconography
- https://fluent2.microsoft.design/layout
- https://fluent2.microsoft.design/material
- https://fluent2.microsoft.design/motion
- https://fluent2.microsoft.design/shapes
- https://fluent2.microsoft.design/typography
- https://fluent2.microsoft.design/accessibility
- https://fluent2.microsoft.design/design-tokens
- https://fluent2.microsoft.design/onboarding
- https://fluent2.microsoft.design/wait-ux
- Token values (the site shows them only as images):
  https://github.com/microsoft/fluentui/tree/master/packages/tokens/src
  (`global/colors.ts`, `global/brandColors.ts`, `alias/lightColor.ts`,
  `alias/darkColor.ts`, `global/borderRadius.ts`, `global/fonts.ts`,
  `global/durations.ts`, `global/curves.ts`, `utils/shadows.ts`)

Komorebi is a Windows app: where Fluent gives a Windows-specific rule, the
Windows rule wins. All values go through `ui/theme.slint` tokens; no
hard-coded colours or sizes in other `.slint` files.

## Principles

1. **Natural on every platform**: look and behave like a Windows 11 app;
   reuse native patterns (about 80% of the time) and spend effort on the
   few signature moments.
2. **Built for focus**: stay out of the way; less clutter and noise.
3. **One for all**: inclusive, accessible by default.
4. **Unmistakably Microsoft**: consistent colour, icons and type across
   the app; "a little personality goes a long way".

## Colour

- Three palettes: **neutral** (surfaces, text, layout; neutrals also show
  state changes), **shared** (avatars, badges, calendars; use sparingly;
  semantic subset only for feedback/status/urgency, never decoration,
  always paired with an icon or text), **brand** (primary buttons/CTAs and
  selected states; never on large surfaces).
- Brand on Windows: the user's system accent (as WinUI does), with the web
  "communication blue" ramp as the fallback: 10 #061724, 20 #082338,
  30 #0a2e4a, 40 #0c3b5e, 50 #0e4775, 60 #0f548c, 70 #115ea3,
  80 #0f6cbd, 90 #2886de, 100 #479ef5, 110 #62abf5, 120 #77b7f7,
  130 #96c6fa, 140 #b4d6fa, 150 #cfe4fa, 160 #ebf3fc.
- **Interaction states**: generally rest → hover → pressed get darker,
  selected darkest; **on Windows controls get lighter on interaction**
  (follow WinUI for Windows-native controls). Focus never changes the
  control colour: it adds a thicker stroke (focus ring).
- Don't rely on colour alone; contrast must pass WCAG AA.

### Grey ramp (global)

2 #050505, 4 #0a0a0a, 6 #0f0f0f, 8 #141414, 10 #1a1a1a, 12 #1f1f1f,
14 #242424, 16 #292929, 18 #2e2e2e, 20 #333333, 22 #383838, 24 #3d3d3d,
26 #424242, 28 #474747, 30 #4d4d4d, 32 #525252, 34 #575757, 36 #5c5c5c,
38 #616161, 40 #666666, 42 #6b6b6b, 44 #707070, 46 #757575, 48 #7a7a7a,
50 #808080, 52 #858585, 54 #8a8a8a, 56 #8f8f8f, 58 #949494, 60 #999999,
62 #9e9e9e, 64 #a3a3a3, 66 #a8a8a8, 68 #adadad, 70 #b3b3b3, 72 #b8b8b8,
74 #bdbdbd, 76 #c2c2c2, 78 #c7c7c7, 80 #cccccc, 82 #d1d1d1, 84 #d6d6d6,
86 #dbdbdb, 88 #e0e0e0, 90 #e6e6e6, 92 #ebebeb, 94 #f0f0f0, 96 #f5f5f5,
98 #fafafa, 99 #fcfcfc.

### Key alias tokens (light / dark)

| Token | Light | Dark |
|---|---|---|
| NeutralForeground1 (text) | grey14 #242424 | white |
| NeutralForeground2 | grey26 #424242 | grey84 #d6d6d6 |
| NeutralForeground3 (secondary/meta) | grey38 #616161 | grey68 #adadad |
| NeutralForeground4 | grey44 #707070 | grey60 #999999 |
| NeutralForegroundDisabled | grey74 #bdbdbd | grey36 #5c5c5c |
| NeutralForegroundOnBrand | white | white |
| BrandForeground1 / Link | brand80 / brand70 | brand100 / brand100 |
| NeutralBackground1 (+Hover/Pressed/Selected) | white (96/88/92) | grey16 #292929 (24/12/22) |
| NeutralBackground2 | grey98 (94/86/90) | grey12 #1f1f1f (20/8/18) |
| NeutralBackground3 | grey96 (92/84/88) | grey8 #141414 (16/4/14) |
| NeutralBackground4 | grey94 | grey4 |
| SubtleBackground (transparent) Hover/Pressed/Selected | grey96 / grey88 / grey92 | grey22 / grey18 / grey20 |
| NeutralBackgroundDisabled | grey94 | grey8 |
| NeutralCardBackground (+Hover/Pressed/Selected) | grey98 (white/96/92) | grey20 (24/18/22) |
| NeutralStroke1 (+Hover/Pressed/Selected) | grey82 (78/70/74) | grey40 (46/42/44) |
| NeutralStroke2 (dividers) | grey88 | grey32 |
| NeutralStroke3 | grey94 | grey24 |
| NeutralStrokeAccessible (input bottom edge) | grey38 | grey68 |
| NeutralStrokeDisabled | grey88 | grey26 |
| BrandBackground / Hover / Pressed / Selected | brand80 / 70 / 40 / 60 | brand70 / 80 / 40 / 60 |
| BrandBackground2 (tint) | brand160 | brand20 |
| BrandStroke1 | brand80 | brand100 |
| StrokeFocus1 (inner) / StrokeFocus2 (outer) | white / black | black / white |
| BackgroundOverlay (smoke/scrim) | black 40% | black 50% |
| NeutralStencil1 / 2 (skeleton) | grey90 / grey98 | grey34 / grey20 |
| Shadow ambient / key | rgba(0,0,0,.12) / .14 | .24 / .28 |
| Shadow ambient / key, darker (high ramp) | .20 / .24 | .40 / .48 |

Windows 11 shell surfaces (WinUI) that Komorebi may use for the window
layer and cards: layer #f3f3f3 / #202020, card #ffffff / #2b2b2b.

### Shared / status colours

- red: primary #d13438, shade10 #bc2f32, shade20 #9f282b, shade30 #751d1f,
  tint30 #e37d80, tint40 #f1bbbc, tint50 #f8dadb, tint60 #fdf6f6.
- green: primary #107c10, shade10 #0e700e, shade30 #094509,
  shade40 #052505, tint30 #54b054, tint40 #9fd89f, tint60 #f1faf1.
- darkOrange (warning): primary #da3b01, shade10 #c43501,
  shade40 #411200, tint30 #e9835e, tint40 #f4bfab, tint60 #fdf6f3.
- marigold: primary #eaa300, shade30 #835b00, tint40 #f9e2ae.
- yellow: primary #fde300, shade30 #817400, tint40 #fef7b2.
- Dark mode shifts shared colours in saturation and brightness (use the
  shades as backgrounds and tints as foregrounds).

## Typography

- Windows typeface: **Segoe UI Variable** (Text optical size for small
  sizes, Display for 20 px and up). Sentence case everywhere; no all caps
  for emphasis. Left-align text; centre only short supporting copy.
- **Windows ramp**: Caption 12/16 regular; Body 14/20 regular; Body
  Strong 14/20 semibold; Body Large 18/24 regular; Subtitle 20/28
  semibold; Title 28/36 semibold; Large Title 40/52 semibold; Display
  68/92 semibold.
- Web ramp (allowed where Windows has no step): Caption 2 10/14,
  Caption 1 12/16 (Strong 600, Stronger 700), Body 1 14/20, Subtitle 2
  16/22 semibold, Subtitle 1 20/26, Title 3 24/32, Title 2 28/36,
  Title 1 32/40, Large Title 40/52, Display 68/92.
- Weights: 400 regular, 600 semibold (700 only for "Stronger" styles).
- Emphasis through colour: Foreground1 for primary, lighter neutrals
  (Foreground2/3) to subdue; brand colour for prominence. Baseline
  alignment gives the vertical rhythm.
- Contrast: 4.5:1 for text, 3:1 for large text (≥ 18.5 px bold or 24 px
  regular) and for icons/control boundaries.

## Layout and spacing

- 4 px base unit. Ramp: 0, 2, 4, 6, 8, 10, 12, 16, 20, 24, 28, 32, 36,
  40, 48, 52, 56 (2, 6 and 10 exist to align icons' built-in padding).
- Space measured from bounding boxes. Proximity groups; use space instead
  of divider lines where possible; more space around what matters.
- In lists: left-align text, centre icons.
- Targets: 44×44 recommended for touch (web/iOS); desktop controls
  32 px high minimum hit area.
- Size classes: small 320-479, medium 480-639, large 640-1023,
  x-large 1024-1365, xx-large 1366-1919, xxx-large 1920+. Responsive
  techniques: reposition, resize, reflow, show/hide, re-architect (list
  beside details on large windows; focus on one on small).
- Must work down to 320 px wide, 200% text zoom without clipping.

## Shapes and strokes

- Forms: rectangle (buttons, inputs, menus, cards), circle (avatars,
  people), pill (toggles, tags, list selection indicators), beak (callout).
- Radius tokens: None 0 (nav bars, tab bars, anything touching the screen
  edge), Small 2 (small badges, shapes < 32 px), Medium 4 (buttons,
  dropdowns, inputs: the default), Large 6, XLarge 8 (cards, large
  buttons, dialogs), 2XLarge 12 (popovers, sheets), 3XLarge 16,
  Circular (avatars, pills).
  Komorebi's tab strip follows WinUI's TabView instead (top corners 8,
  `docs/WINUI3.md` "TabView"): on Windows that reference wins.
- Don't round where it makes awkward gaps, don't split one control into
  separately rounded pieces (split button), don't round edge-to-edge
  components.
- Stroke widths: Thin 1, Thick 2, Thicker 3, Thickest 4; scale with
  element size. Rounded caps only.

## Elevation

- Shadows: `shadowN = 0 0 2px ambient, 0 (N/2)px Npx key` for 2, 4, 8, 16;
  shadow28 = `0 0 8px ambient, 0 14px 28px key(darker)`; shadow64 =
  `0 0 8px ambient, 0 32px 64px key(darker)`.
- Use: 2 small raised items; 4 cards/list items; 8 command bars,
  dropdowns, tooltips; 16 callouts, hover cards; 28 side navigation,
  sheets; 64 dialogs.
- **On Windows, strokes replace key shadows to outline objects.** Keep a
  single consistent light direction. Coloured surfaces use the brand
  shadow tokens.
- Komorebi's software renderer has no blur: draw elevation as a 1 px
  stroke plus at most 1-2 offset translucent rings; never stack many.

## Material

- Solid: opaque, the default; separation by colour and elevation.
- Mica: opaque base tinted by the desktop when active, neutral when
  inactive (window background). Acrylic: frosted, only for transient
  light-dismiss surfaces (menus, flyouts). Smoke: translucent black under
  modals to block the content behind (not mode aware).
- Komorebi (software renderer, no blur): solid layers everywhere, the
  layer colour standing in for Mica, an opaque card for menus in place of
  Acrylic, and the overlay token as Smoke behind dialogs.

## Iconography

- Fluent System Icons style (Komorebi uses the matching Segoe Fluent
  Icons glyphs on Windows): **regular** for wayfinding/at rest, **filled**
  for selected/on states and small moments needing weight.
- Sizes: 16 px in dense command bars and buttons, 20 px standard, 24 px
  larger; 12 px only for information, never as a click target.
- One solid colour per icon, no gradients; modifiers bottom-right, filled.
- Icons are literal metaphors (named for the object, not the function).
- Every icon-only control needs an accessible name and a tooltip.

## Motion

- Purposeful, natural, consistent; quick, never sluggish; duration scales
  with element size and travel distance.
- Durations: ultraFast 50, faster 100, fast 150, normal 200, gentle 250,
  slow 300, slower 400, ultraSlow 500 ms.
- Curves: decelerateMax (0.1,0.9,0.2,1), decelerateMid (0,0,0,1),
  decelerateMin (0.33,0,0.1,1), accelerateMax (0.9,0.1,1,0.2),
  accelerateMid (1,0,1,1), accelerateMin (0.8,0,0.78,1), easyEaseMax
  (0.8,0,0.2,1), easyEase (0.33,0,0.67,1), linear. Enter = decelerate,
  exit = accelerate.
- Page/top-level changes: a quick fade, not big slides. Stagger lists
  with short offsets. Keep motion on the focused element; honour the
  system "animation effects off" setting; no flashing.
- Komorebi: animations must not cost frames on the software renderer;
  prefer opacity/colour transitions of 100-200 ms on small elements.

## Accessibility

- WCAG 2.1 AA: text 4.5:1, large text and non-text UI (icons, control
  boundaries, focus) 3:1.
- Focus moves in a "z" pattern (left→right, top→bottom), is never lost
  after a dialog closes (return it to the invoker), and is shown as the
  Fluent double ring (2 px outer StrokeFocus2 + 1 px inner StrokeFocus1)
  only for keyboard focus.
- Logical heading order; plain, concise, consistent language; alt text;
  reflow at 400% zoom, 200% text zoom, 320 px width; status changes
  announced to screen readers.

## Wait UX

- Under 1 s: show nothing (a flash confuses). 1-3 s: spinner with an
  "-ing…" label ("Loading…", "Connecting…", "Sending…"). Over 3 s:
  determinate progress bar (label on top, optional status "30% complete -
  about 20 seconds remaining") or a reassuring string ("Working on it…").
- Skeletons (stencil tokens) for content that is still rendering; never
  blank screens or static loaders; one indicator at a time, attached to
  the content it concerns; keep the user in their current view.
- Copy: -ing verbs while in progress, past tense when done ("Sent"),
  active voice, one short phrase; non-breaking space before the ellipsis;
  announce state changes.

## Onboarding

- Not a one-time event: teach in context, when the user is ready.
  Relevant, non-distracting, optional (dismissible, findable later),
  benefit-focused, built from standard Fluent components.
- Goals: Welcome (once, short, the benefit), Orient (empty states,
  teaching popovers), Notify (message bar/toast for changes), Explain
  (empty states with a clear call to action, so nobody is stuck), Take
  action (setup flow showing steps remaining).
- Copy: write for action ("Add an account"), point of need, no long
  passive text, inclusive ("Need help?"), set expectations ("This takes
  about a minute").
- Komorebi's first run: no tour, nothing before the first frame.
  **Take action** is a short setup flow that shows the steps remaining
  (Fluent "Take action", "We'll guide you through 3 quick steps"):
  **Your email → Sign in → Make it yours**, three steps with a step
  indicator. Each step asks one question. Every step after the first has
  Back. The flow can be left at any point (Esc, Back from the email step)
  and resumed from the welcome. Teaching stays in context (TeachingTips,
  empty states), never in the flow. The layout, copy and metrics are in
  docs/WINUI3.md "Onboarding".
  - **Welcome**: the whole window (no folder pane, rail or toggle; the
    title bar without its search): the brand mark (vector, no image), a
    headline and a one-sentence subhead with the benefit, the accent "Add
    your email account" and the time it takes ("It takes about a
    minute."), three short value rows (private, fast, works with your
    provider), then the subtle links "Need help?" (the README's setup
    section, through the link prompt), "Privacy" (PRIVACY.md, the same
    way) and a compact Language box that switches live, with Settings one
    press away beside it. Its entrance (opacity, `gentle` and `stagger`)
    starts after the first frame (at once with Animation effects off).
  - **Your email**: one field. Continue checks the address inline (never
    a disabled button), refuses an account already added, and decides how
    the servers are found. Looking the domain up happens only on a press:
    **Continue** on the email step (or Find settings on the manual page),
    never on typing or focus change. A preset domain (`providers::guess`)
    skips the lookup. The lookup never sends the password. Under
    `KOMOREBI_OFFLINE` it refuses, and the flow goes straight to manual
    setup with a one-line note.
  - **Sign in**: the provider chip says what was found (looking, found,
    not found with the provider choice inline, or Change for manual
    setup), then the Password box. Sign in pressed while the lookup still
    runs waits for it.
  - **Wait**: the check shows three grouped stages (Connecting securely,
    Checking your password, Getting ready to send) at once; Back and Esc
    stay usable and drop the answer; after 3 s the wait reassures, after
    10 s it says what to check.
  - **Explain**: a failure is one Critical InfoBar inside the card: a
    short title, one sentence and one next action (Set username, Review
    certificate, Try again, Set up manually); the 7 steps and the host
    names fold under "Show details". A rejected password focuses the
    Password box with its text selected.
  - **Make it yours**: the account is already saved; the optional name
    and signature with a live From preview, Skip or Done. The first
    download runs meanwhile.
  - **Land**: "You're all set" with the first download's progress
    (indeterminate until a folder reports numbers, then "{0} of {1}";
    never an overall percentage) and "Open your inbox", which closes the
    flow on the new account's Inbox (the list focused). No auto-advance,
    no confetti; there is no "Added …" status line.

## Signatures

A signature is a named, formatted block (at most one small logo) that
starts new mail and replies. Settings > Signatures lists them as Settings
cards (name, one-line preview, Edit, Delete); deleting asks first and says
which accounts use it. Each account picks two defaults, one for new messages
and one for replies and forwards, from a ComboBox that lists None and the
names. The composer's Signature dropdown replaces the block and can be
undone. The editor is the composer's, with fewer commands (no table, no
quote). Use theme tokens only, wrap rather than clip in Portuguese, and keep
text colours at the system text colour in high contrast.
