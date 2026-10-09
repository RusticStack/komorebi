# Komorebi HTML email rendering corpus

Synthetic, realistic HTML email bodies for measuring a no-web-engine renderer (Slint 1.18 software renderer). **No real mail.** Only `example.com` / `example.org` / `example.net` / `example.invalid` domains; every remote URL is `https://*.example.invalid/...` and must never be fetched without explicit user opt-in.

Each message exists twice:

- `NN-name.html` - the decoded `text/html` body exactly as the renderer would receive it after MIME decoding (bytes in the declared charset; `cid:` references left as-is).
- `NN-name.eml` - the full RFC 5322 / MIME message with `text/plain` alternative and the inline images as `multipart/related` parts (`Content-ID` + `Content-Disposition: inline`). Decoded HTML in the .eml equals the .html file (modulo a trailing newline / CRLF).

`images/` holds the source PNG/JPEG files used for the cid parts (also embedded in the .eml files). `_manifest.json` is the machine-readable version of this table. Generator: `../html-work/corpus-gen/gen.py` (+ `make_images.ps1`, `verify.py`).

Note on the brief: the 15 listed exemplars are covered by 14 files - the Apple Mail exemplar is merged into the plain-ish personal email (12), since Apple Mail markup is what a personal message looks like.

| File | .html bytes | .eml bytes | HTML charset / CTE | cid parts |
|---|---:|---:|---|---:|
| `01-mailchimp-campaign` | 56,841 | 89,519 | utf-8 / quoted-printable | 6 |
| `02-substack-post` | 37,161 | 102,871 | utf-8 / base64 | 1 |
| `03-github-review` | 4,969 | 7,354 | utf-8 / quoted-printable | 0 |
| `04-order-receipt` | 10,391 | 26,356 | utf-8 / quoted-printable | 5 |
| `05-bank-statement` | 197,995 | 247,030 | iso-8859-1 / quoted-printable | 2 |
| `06-outlook-word` | 45,028 | 59,150 | windows-1252 / quoted-printable | 1 |
| `07-gmail-reply-chain` | 21,385 | 45,859 | utf-8 / quoted-printable | 1 |
| `08-dark-marketing` | 11,189 | 33,278 | utf-8 / base64 | 2 |
| `09-image-heavy-promo` | 9,854 | 29,089 | utf-8 / quoted-printable | 2 |
| `10-transactional-buttons` | 5,367 | 10,018 | utf-8 / quoted-printable | 1 |
| `11-newsletter-2col` | 19,423 | 54,154 | utf-8 / base64 | 6 |
| `12-personal-applemail` | 1,796 | 4,827 | utf-8 / quoted-printable | 1 |
| `13-calendar-invite` | 6,990 | 12,819 | utf-8 / quoted-printable | 0 |
| `14-adversarial-malformed` | 27,845 | 34,805 | utf-8 / 8bit | 3 |

## 01-mailchimp-campaign

Mailchimp 'Sell Products' template: MSO OfficeDocumentSettings XML, <link> + @import remote web fonts, ~7 KB <style> with ID/class selectors and 20 max-width/min-width media queries, hidden preheader, <center>, ghost tables, nested mcn* block tables, cid logo/hero/product images, remote social icons + lifestyle images, VML-free bulletproof button (bg-colored td), open-tracking pixel, &euro;/&rarr; entities, vcard footer, HTML comment containing markup.

- MIME tree (walk order): `related/alternative/plain/html/png/jpeg/jpeg/jpeg/jpeg/jpeg`
- cid map: `cid:logo-lumen@lumen.example.com` -> `images/logo-lumen.png`, `cid:hero-autumn@lumen.example.com` -> `images/hero-autumn.jpg`, `cid:prod-mug@lumen.example.com` -> `images/prod-mug.jpg`, `cid:prod-lamp@lumen.example.com` -> `images/prod-lamp.jpg`, `cid:prod-plant@lumen.example.com` -> `images/prod-plant.jpg`, `cid:prod-book@lumen.example.com` -> `images/prod-book.jpg`

## 02-substack-post

Substack long-form post: div-based (not table) layout with max-width:550px;margin:0 auto, class CSS, system font stacks, h1-h3, blockquote, pullquote, <figure>/<picture>/<source srcset> with remote image, <pre><code>, nested ol/ul with <p> in <li>, footnote anchors (#fragment links), tracked redirect links, &#847;/&zwnj; preheader padding, large cid hero (1200x600 downscaled to 550), inline-block buttons with border-radius, open pixel.

- MIME tree (walk order): `alternative/plain/related/html/jpeg`
- cid map: `cid:hero-substack@fieldnotes.example.com` -> `images/hero-substack.jpg`

## 03-github-review

GitHub PR review notification: fragment HTML with no <html>/<head>/<body>, leading empty <p>, <pre> with escaped quoted diff, diff table with white-space:pre per-row background colors, rgba() colors, <code> inline styling, disabled task-list checkboxes, blockquote, data: URI GIF icon, remote beacon pixel, hidden Message-ID span, JSON-LD <script type=application/ld+json> (must be ignored, never executed). No cid parts (multipart/alternative only).

- MIME tree (walk order): `alternative/plain/html`

## 04-order-receipt

Amazon-style order confirmation: XHTML 1.0 Transitional doctype, bgcolor on body/table/td, fixed 600 px wrapper + max-width, MSO ghost table, CSS font shorthand (font:14px/20px Arial), right-aligned price column with nowrap, align=right floated totals table, colspan spacer row (1px line-height trick), border-radius:8px card with border-collapse:separate, pill button (bgcolor td + border-radius:20px), cid logo + 4 cid thumbnails, 3 remote recommendation images with styled alt text, &euro;/&minus;/&ordm;/&Oslash; entities, media query hiding nav, open pixel.

- MIME tree (walk order): `alternative/plain/related/html/png/jpeg/jpeg/jpeg/jpeg`
- cid map: `cid:logo-parcelo@parcelo.example.com` -> `images/logo-parcelo.png`, `cid:prod-mug@parcelo.example.com` -> `images/prod-mug.jpg`, `cid:prod-lamp@parcelo.example.com` -> `images/prod-lamp.jpg`, `cid:prod-plant@parcelo.example.com` -> `images/prod-plant.jpg`, `cid:prod-book@parcelo.example.com` -> `images/prod-book.jpg`

## 05-bank-statement

Bank statement, ~200 KB: legacy uppercase HTML 3.2-era markup, charset=iso-8859-1 (QP body with =E3/=E7 bytes) declared in both MIME and <META>, BODY BGCOLOR/LEFTMARGIN/TEXT/LINK attrs, <CENTER>, CSS inside <!-- --> in <STYLE>, 240-row table with THEAD/TBODY/TFOOT, alternating TR bgcolor zebra, <FONT face size color> everywhere (size=1/2), red/green signed amounts, BORDER=1 + BORDERCOLOR + border-collapse summary table, cid logo + cid PNG chart, tel: link, Portuguese accents. Stresses large-table layout performance.

- MIME tree (walk order): `related/alternative/plain/html/png/png`
- cid map: `cid:logo-northbank@northbank.example.com` -> `images/logo-northbank.png`, `cid:chart-spend@northbank.example.com` -> `images/chart-spend.png`

## 06-outlook-word

Outlook/Word 'filtered medium' HTML in windows-1252 (QP, 0x93/0x94/0x96 smart punctuation bytes): v:/o:/w:/m: namespaces, unquoted attributes, VML behavior styles inside <!--[if !mso]>, @font-face panose blocks, @page/@list mso-* CSS, p.MsoNormal with <o:p></o:p>, downlevel-revealed <![if !supportLists]> fake bullets with Symbol font, MsoTableGrid with pt widths and per-cell borders + Excel-like conditional fills, Outlook-style cid:image001.png@01DB... signature image, 'From:/Sent:/To:' reply header with border-top:solid #E1E1E1, mailto link.

- MIME tree (walk order): `related/alternative/plain/html/png`
- cid map: `cid:image001.png@01DB1A2B.3C4D5E60` -> `images/avatar-mr.png`

## 07-gmail-reply-chain

26-message Gmail thread with 26 levels of nested quoting: <blockquote class=gmail_quote> with border-left rgb(), gmail_attr headers with U+202F narrow no-break space, gmail_signature with '-- ' prefix, embedded Outlook segment (divRplyFwdMsg + <hr style=display:inline-block>, <font face color>), embedded Apple Mail segment (<blockquote type=cite>, dir=auto), pasted inline image (cid:ii_...), nested lists, <font color> + rgb() background highlight. Fragment HTML (no <html>/<body>). Tests deep nesting indentation and quote collapsing.

- MIME tree (walk order): `alternative/plain/related/html/jpeg`
- cid map: `cid:ii_m1x2y3z40@gmail.example.com` -> `images/hero-autumn.jpg`

## 08-dark-marketing

Dark-background SaaS promo: <meta name=color-scheme content=dark>, :root color-scheme, @media (prefers-color-scheme: dark/light), Outlook.com [data-ogsc]/[data-ogsb] overrides, Gmail 'u + .body' hack, white-on-transparent alpha PNG logo (must composite on dark), CSS font shorthand with numeric weights (900), letter-spacing/text-transform, linear-gradient backgrounds with bgcolor fallback, background-clip:text gradient headline with color fallback, box-shadow/border-radius on image, dashed-border coupon, two VML v:roundrect bulletproof buttons (filled + ghost/transparent), role=article wrapper div, <link rel=preconnect> + remote Inter font, &#8199;&#65279; preheader padding, remote white social icons, display:none tracking pixel.

- MIME tree (walk order): `alternative/plain/related/html/png/jpeg`
- cid map: `cid:logo-atlas@atlas.example.com` -> `images/logo-atlas.png`, `cid:hero-neon@atlas.example.com` -> `images/hero-neon.jpg`

## 09-image-heavy-promo

Image-heavy retail promo: VML <v:background> body fill and <v:rect>/<v:fill type=frame>/<v:textbox> hero, <td background=cid:...> attribute + CSS background-image:url('cid:...') + background-size:cover with text overlaid on rgba(0,0,0,.35) panel, text-shadow, tiled remote background on outer table (background attr + CSS), CSS background shorthand with position/size and remote url, 12 remote image slices in a 3x4 grid with styled alt text (font/color/background on <img>), image map <map>/<area>, data: URI PNG logo, cid banner, media query swapping background image, <font> footer. Most pixels are remote -> tests blocked-image placeholder layout.

- MIME tree (walk order): `related/alternative/plain/html/jpeg/jpeg`
- cid map: `cid:banner-wide@shop.example.com` -> `images/banner-wide.jpg`, `cid:hero-bg-promo@shop.example.com` -> `images/hero-bg-promo.jpg`

## 10-transactional-buttons

Transactional verification email (pt-PT): three bulletproof button techniques (VML v:roundrect + <a> fallback, padding-based bgcolor td/a with mso-padding-alt, thick same-colour border-based <a> with outline), :hover rule, hidden preheader with &zwnj;&nbsp; filler, <center>, IE/MSO ghost table, min-width/max-width media queries, monospace OTP with letter-spacing and zero-width space (&#8203;), long unbreakable URL with word-break:break-all, border-radius card, 'yahoo' boolean attribute on body, HTML entities for Portuguese accents, single cid logo.

- MIME tree (walk order): `alternative/plain/related/html/png`
- cid map: `cid:logo-fieldnotes@fieldnotes.example.com` -> `images/logo-fieldnotes.png`

## 11-newsletter-2col

Hybrid/fluid newsletter (Parcel/Good Email Code style): role=presentation tables, inline-block 'column' divs with max-width in a font-size:0 parent (columns must sit side by side at >=600 px and stack when narrow), dir=rtl column-reversal trick, MSO ghost tables per row, min-width/max-width media queries (col-sml 27% / col-lge 73%), 3-column footer row with .stack and .hide-sm, @import Merriweather, MSO letter-spacing button padding hack (mso-text-raise), border-spacing:0, rgba border-color, mixed cid + remote story images, &reg;/&rsaquo; entities.

- MIME tree (walk order): `related/alternative/plain/html/png/jpeg/jpeg/jpeg/jpeg/jpeg`
- cid map: `cid:logo-fieldnotes@fieldnotes.example.com` -> `images/logo-fieldnotes.png`, `cid:hero-autumn@fieldnotes.example.com` -> `images/hero-autumn.jpg`, `cid:prod-mug@fieldnotes.example.com` -> `images/prod-mug.jpg`, `cid:prod-plant@fieldnotes.example.com` -> `images/prod-plant.jpg`, `cid:prod-book@fieldnotes.example.com` -> `images/prod-book.jpg`, `cid:prod-lamp@fieldnotes.example.com` -> `images/prod-lamp.jpg`

## 12-personal-applemail

Small (~2 KB) personal message as written by Apple Mail: <div dir=auto> paragraphs with <div><br></div> spacing, -webkit-nbsp-mode/line-break body styles, raw UTF-8 Portuguese + emoji (no entities), <b>/<i>, link, tel:/mailto: links, signature table with rgb() colours and border-radius avatar, UUID-style cid (no @domain), Apple MIME-Version comment header. Baseline for the common case: must render instantly.

- MIME tree (walk order): `alternative/plain/related/html/png`
- cid map: `cid:6F1D2A8B-3C4E-4F5A-9B7C-0D1E2F3A4B5C` -> `images/avatar-mr.png`

## 13-calendar-invite

Google-Calendar-style invitation: fragment HTML wrapped in schema.org microdata (itemscope/itemprop/<meta>/<time> inside body), summary attrs, display:inline-table, border-width per side, CSS keyword case ('1px Solid #ccc'), pill RSVP links, Meet button with remote icon, tel: link with %3B/%23 escapes, &raquo;. MIME: multipart/mixed[alternative[text/plain, text/html, text/calendar; method=REQUEST], application/ics attachment] - tests that the calendar part is not chosen as the body.

- MIME tree (walk order): `mixed/alternative/plain/html/calendar/ics`

## 14-adversarial-malformed

Adversarial/malformed: <script> (inline, src, JSON in body), on* handlers (onload/onerror/onfocus/onmouseover), javascript:/vbscript:/data:text/html/file: hrefs incl. entity- and tab-obfuscated, meta refresh, <base href>, <iframe>/srcdoc, <object>/<embed>/<applet>, <video>/<audio autoplay>, <form> with password field, SVG with onload/script/foreignObject, mXSS vectors (math/style, noscript, template), CSS expression()/behavior/-moz-binding/javascript: url()/CSS-escaped url(), remote @import/@font-face/background tracking, position:fixed full-viewport overlay at max z-index, negative margins, absolute-positioned fake banner, font-size:99999px, white-on-white text, RTL override U+202E, zero-width chars, 5000-char unbroken word, misnested/unclosed tags, table inside <p>, colspan/rowspan=99999, width=99999, 400 nested divs, 150 nested tables (30 left unclosed), bad entities, CDATA/PI/xmp/plaintext, NUL + ANSI control chars, invalid UTF-8 sequences (overlong, surrogate, truncated), conflicting charsets (shift_jis meta vs windows-1251 meta vs duplicate MIME charset params), no </body></html>, truncated final tag. EML: 8bit HTML with raw invalid bytes, preamble with script, broken RFC 2047 subject, 2000-char header, two text/html alternatives, cid to missing part, truncated data: URIs, image/png part that is actually HTML, path-traversal filename, truncated JPEG part, missing closing MIME boundary.

- MIME tree (walk order): `related/alternative/plain/html/html/jpeg/png/jpeg`
- cid map: `cid:inline-ok@adv.example.com` -> `images/prod-plant.jpg`

## Expected-failure notes (14-adversarial-malformed)

- `cid:missing-part@nowhere.invalid` and `cid:not-a-link` intentionally have no MIME part; `not-really-png@...` (HTML bytes labelled image/png) and `truncated@...` (first third of a JPEG) are intentionally unreferenced/broken.
- The .eml has no closing `--outer-b--` boundary (Python's parser reports 1 defect), 8bit body with invalid UTF-8, duplicate `charset` params, a 2000-char header, a broken RFC 2047 encoded-word, and a `../../` filename.
- The .html file contains a literal NUL byte, ANSI escape bytes and invalid UTF-8 sequences; it has no `</body></html>` and ends mid-tag.

## Suggested measurements

Per file: parse+sanitize time, style/cascade time, layout time at 600 px and 360 px widths, paint time, peak RSS delta, number of boxes/text runs, and whether remote requests were attempted (must be zero by default). 05 (240-row table, ~200 KB), 01 (~7 KB stylesheet, 20 media queries) and 14 (150-deep tables, 400-deep divs) are the stress cases; 12 is the common-case latency baseline.
