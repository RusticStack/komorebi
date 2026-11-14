// Builds Komorebi 1.0's brand assets from the source SVGs in
// assets/brand-src (copied from the approved brand, komorebi-brand
// final/logo). See docs/BRANDING.md for what each output is and where it is
// used.
//
//   node scripts/brand-assets.js
//
// Needs @resvg/resvg-js 2.6 and sharp 0.33 from a folder outside the
// repository (no package.json here), for instance:
//
//   npm install --prefix F:\dev-cache\komorebi-brand-tools @resvg/resvg-js@2.6.2 sharp@0.33.5
//   $env:NODE_PATH = 'F:\dev-cache\komorebi-brand-tools\node_modules'
//
// The output is deterministic for a given resvg and sharp; the ICO and BMP
// containers are written here, not by a library.

'use strict';
const fs = require('fs');
const path = require('path');
const { Resvg } = require('@resvg/resvg-js');
const sharp = require('sharp');

const ROOT = path.resolve(__dirname, '..');
const SRC = path.join(ROOT, 'assets', 'brand-src');
const OUT10 = path.join(ROOT, 'assets');
const BRAND = path.join(ROOT, 'assets', 'brand');
const INSTALLER = path.join(ROOT, 'installer');
const GITHUB = path.join(ROOT, '.github', 'assets');

// Brand tokens (BRAND.md). The brand's persimmon (#B3503A) is in the source
// SVGs only: every output is neutral (docs/BRANDING.md "Neutral brand").
const PERSIMMON = '#B3503A';
const INK = '#171717';
const INK_DARK = '#F2F2F0';
const INK2 = '#555552';
const INK2_DARK = '#B0B0AC';
const INK3 = '#6A6A66';
const INK3_DARK = '#939390';
const PAPER = '#FFFFFF';
const SITE_DARK = '#141414';
const FALL = '#F1F1EE';
const FALL_DARK = '#202020';
// The tray glyphs (tray-16-*.svg): ink on a light taskbar, white on a dark one.
const TRAY = { light: '#1A1A1A', dark: '#FFFFFF' };

// The neutral seal (owner decision after 1.0, docs/BRANDING.md "Neutral
// brand"): the app icon, the taskbar, the toast image, the installer images,
// the assets/brand files and the repository images are ink-3 with the K in
// white, not persimmon (the app's own views draw the ink mark,
// crates/komorebi-gpui/assets/brand/seal.svg, instead). One icon has to read
// on a light and a dark taskbar: ink-3 is 4.7 to 4.9:1 on the light ones
// (#EEEEEE, #F3F3F3) and 3.0 to 3.1:1 on the dark ones (#202020, #1C1C1C),
// and the white K is 5.4:1 on it. Ink (#171717) would vanish on a dark
// taskbar (1.1:1), white on a light one.
const NEUTRAL = INK3;
const NEUTRAL_K = PAPER;

const read = (f) => fs.readFileSync(path.join(SRC, f), 'utf8');

// The source SVGs are single-colour; recolour every fill.
const recolour = (svg, colour) => svg.replace(/fill="[^"]*"/g, `fill="${colour}"`);

// Renders an SVG string to straight (not premultiplied) RGBA at w x h.
async function rgba(svg, w, h) {
  const png = new Resvg(svg, {
    fitTo: { mode: 'width', value: w },
    font: { loadSystemFonts: true },
  }).render().asPng();
  const { data, info } = await sharp(png)
    .resize(w, h, { fit: 'fill', kernel: 'nearest' })
    .ensureAlpha()
    .raw()
    .toBuffer({ resolveWithObject: true });
  if (info.width !== w || info.height !== h || info.channels !== 4) {
    throw new Error(`render ${w}x${h} gave ${info.width}x${info.height}x${info.channels}`);
  }
  return data;
}

async function pngOf(svg, w, h) {
  const png = new Resvg(svg, {
    fitTo: { mode: 'width', value: w },
    font: { loadSystemFonts: true },
  }).render().asPng();
  const out = await sharp(png).png({ compressionLevel: 9, palette: false }).toBuffer();
  const meta = await sharp(out).metadata();
  if (meta.width !== w || meta.height !== h) {
    throw new Error(`png ${w}x${h} gave ${meta.width}x${meta.height}`);
  }
  return out;
}

// A 32-bit DIB for an ICO entry: BITMAPINFOHEADER (height doubled for the
// AND mask), bottom-up BGRA rows, then the 1-bit AND mask (1 = transparent).
function dibEntry(px, w, h) {
  const xorSize = w * h * 4;
  const maskRow = Math.ceil(w / 32) * 4;
  const maskSize = maskRow * h;
  const b = Buffer.alloc(40 + xorSize + maskSize);
  b.writeUInt32LE(40, 0);
  b.writeInt32LE(w, 4);
  b.writeInt32LE(h * 2, 8);
  b.writeUInt16LE(1, 12);
  b.writeUInt16LE(32, 14);
  b.writeUInt32LE(0, 16); // BI_RGB
  b.writeUInt32LE(xorSize + maskSize, 20);
  for (let y = 0; y < h; y++) {
    const row = 40 + (h - 1 - y) * w * 4;
    for (let x = 0; x < w; x++) {
      const s = (y * w + x) * 4;
      const d = row + x * 4;
      b[d] = px[s + 2];
      b[d + 1] = px[s + 1];
      b[d + 2] = px[s];
      b[d + 3] = px[s + 3];
      if (px[s + 3] === 0) {
        const m = 40 + xorSize + (h - 1 - y) * maskRow + (x >> 3);
        b[m] |= 0x80 >> (x & 7);
      }
    }
  }
  return b;
}

// An ICO container from [{ size, data }] (data: a DIB or a PNG).
function ico(entries) {
  const head = Buffer.alloc(6 + 16 * entries.length);
  head.writeUInt16LE(0, 0);
  head.writeUInt16LE(1, 2);
  head.writeUInt16LE(entries.length, 4);
  let offset = head.length;
  entries.forEach((e, i) => {
    const o = 6 + i * 16;
    head[o] = e.size >= 256 ? 0 : e.size;
    head[o + 1] = e.size >= 256 ? 0 : e.size;
    head[o + 2] = 0;
    head[o + 3] = 0;
    head.writeUInt16LE(1, o + 4);
    head.writeUInt16LE(32, o + 6);
    head.writeUInt32LE(e.data.length, o + 8);
    head.writeUInt32LE(offset, o + 12);
    offset += e.data.length;
  });
  return Buffer.concat([head, ...entries.map((e) => e.data)]);
}

// A 24-bit bottom-up BMP (Inno Setup's wizard images) from RGBA over an
// opaque ground.
function bmp24(px, w, h) {
  const row = Math.ceil((w * 3) / 4) * 4;
  const size = 54 + row * h;
  const b = Buffer.alloc(size);
  b.write('BM', 0, 'ascii');
  b.writeUInt32LE(size, 2);
  b.writeUInt32LE(54, 10);
  b.writeUInt32LE(40, 14);
  b.writeInt32LE(w, 18);
  b.writeInt32LE(h, 22);
  b.writeUInt16LE(1, 26);
  b.writeUInt16LE(24, 28);
  b.writeUInt32LE(0, 30);
  b.writeUInt32LE(row * h, 34);
  b.writeInt32LE(3780, 38); // 96 dpi
  b.writeInt32LE(3780, 42);
  for (let y = 0; y < h; y++) {
    const r = 54 + (h - 1 - y) * row;
    for (let x = 0; x < w; x++) {
      const s = (y * w + x) * 4;
      if (px[s + 3] !== 255) throw new Error('bmp24 needs an opaque image');
      b[r + x * 3] = px[s + 2];
      b[r + x * 3 + 1] = px[s + 1];
      b[r + x * 3 + 2] = px[s];
    }
  }
  return b;
}

// The `d` of every path and the transform of a source SVG, for composing.
const pathsOf = (svg) => [...svg.matchAll(/<path[^>]*\sd="([^"]+)"/g)].map((m) => m[1]);
const viewBoxOf = (svg) => svg.match(/viewBox="([^"]+)"/)[1].split(/\s+/).map(Number);

// A seal path's subpaths: the first is the outer square, the rest the K.
const subpaths = (d) => d.split(/(?<=Z)/).map((s) => s.trim()).filter(Boolean);

// The seal path `d` under `transform`: with `k` (a colour) the square is
// solid and the K is drawn on it in that colour; without, the K is carved
// through (the brand's form).
function sealPaths(d, transform, fill, k) {
  if (!k) return `<path transform="${transform}" fill="${fill}" fill-rule="evenodd" d="${d}"/>`;
  const [outer, ...strokes] = subpaths(d);
  return (
    `<path transform="${transform}" fill="${fill}" d="${outer}"/>` +
    `<path transform="${transform}" fill="${k}" d="${strokes.join('')}"/>`
  );
}

// A source SVG with the brand's persimmon seal path (the square with the K
// carved through) redrawn neutral: the square in ink-3 and the K on it in
// white, under the same transform. Fails if any persimmon is left.
function neutral(svg) {
  const out = svg.replace(
    new RegExp(`<path fill="${PERSIMMON}" fill-rule="evenodd" d="([^"]+)"/>`, 'g'),
    (_, d) => {
      const [outer, ...strokes] = subpaths(d);
      return `<path fill="${NEUTRAL}" d="${outer}"/><path fill="${NEUTRAL_K}" d="${strokes.join('')}"/>`;
    },
  );
  if (out.toUpperCase().includes(PERSIMMON)) throw new Error('persimmon left after neutral()');
  return out;
}

// The seal at side `s` with its top-left at (x, y), from mark.svg (256 box).
function sealAt(x, y, s, fill = NEUTRAL, k = NEUTRAL_K) {
  const [d] = pathsOf(read('mark.svg'));
  const sc = s / 208; // the seal spans 24..232 of the 256 box
  return sealPaths(d, `translate(${x - 24 * sc} ${y - 24 * sc}) scale(${sc})`, fill, k);
}

// The hand-drawn pixel icons (16, 24) have the K carved through: fill every
// pixel inside the seal with the seal over `k`, so the K shows in `k`. A
// pixel is inside when it lies strictly between the first and last painted
// pixels of both its row and its column, so the soft outer edge stays.
function fillCarving(px, w, h, k) {
  const [kr, kg, kb] = [1, 3, 5].map((i) => parseInt(k.substr(i, 2), 16));
  const painted = (x, y) => px[(y * w + x) * 4 + 3] > 0;
  const rows = [];
  const cols = [];
  for (let y = 0; y < h; y++) {
    let a = -1;
    let b = -1;
    for (let x = 0; x < w; x++) if (painted(x, y)) (a < 0 && (a = x), (b = x));
    rows.push([a, b]);
  }
  for (let x = 0; x < w; x++) {
    let a = -1;
    let b = -1;
    for (let y = 0; y < h; y++) if (painted(x, y)) (a < 0 && (a = y), (b = y));
    cols.push([a, b]);
  }
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const [r0, r1] = rows[y];
      const [c0, c1] = cols[x];
      if (!(r0 < x && x < r1 && c0 < y && y < c1)) continue;
      const i = (y * w + x) * 4;
      const al = px[i + 3] / 255;
      px[i] = Math.round(px[i] * al + kr * (1 - al));
      px[i + 1] = Math.round(px[i + 1] * al + kg * (1 - al));
      px[i + 2] = Math.round(px[i + 2] * al + kb * (1 - al));
      px[i + 3] = 255;
    }
  }
  return px;
}

// The tray's unread state, neutral: the glyph `svg` (size x size) with a
// spot in its own colour at the top right, cut out of the seal with a gap
// of transparency (BRAND.md: unread is the ink spot; no colour needed).
function withSpot(svg, size, colour) {
  const r = (size * 3) / 16; // 3 px at 16, in the corner
  const gap = size / 16;
  const c = size - r;
  const inner = svg
    .replace(/<\?xml[^>]*>/, '')
    .replace(
      /<svg\b([^>]*)>/,
      (_, attrs) =>
        `<svg${attrs.replace(/\s(width|height)="[^"]*"/g, '')} x="0" y="0" width="${size}" height="${size}">`,
    );
  return doc(
    size,
    size,
    `<defs><mask id="m" maskUnits="userSpaceOnUse" x="0" y="0" width="${size}" height="${size}">` +
      `<rect width="${size}" height="${size}" fill="#FFFFFF"/>` +
      `<circle cx="${c}" cy="${size - c}" r="${r + gap}" fill="#000000"/></mask></defs>` +
      `<g mask="url(#m)">${inner}</g>` +
      `<circle cx="${c}" cy="${size - c}" r="${r}" fill="${colour}"/>`,
  );
}

// The wordmark `w` wide with its top-left at (x, y), from wordmark.svg.
function wordmarkAt(x, y, w, fill) {
  const svg = read('wordmark.svg');
  const [vx, vy, vw] = viewBoxOf(svg);
  const k = w / vw;
  return `<path transform="translate(${x - vx * k} ${y - vy * k}) scale(${k})" fill="${fill}" d="${pathsOf(svg)[0]}"/>`;
}
const wordmarkHeight = (w) => {
  const [, , vw, vh] = viewBoxOf(read('wordmark.svg'));
  return (w * vh) / vw;
};

const doc = (w, h, body, bg) =>
  `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}" viewBox="0 0 ${w} ${h}">` +
  (bg ? `<rect width="${w}" height="${h}" fill="${bg}"/>` : '') +
  body +
  '</svg>';

// Minifies a source SVG for the app: shorter numbers, no newlines.
function minify(svg) {
  return svg
    .replace(/\r?\n\s*/g, '')
    .replace(/(\d+\.\d{3})\d+/g, (_, n) => String(Number(n)))
    .replace(/>\s+</g, '><')
    .trim();
}

function write(file, data) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, data);
  const rel = path.relative(ROOT, file).replace(/\\/g, '/');
  console.log(`${String(data.length).padStart(8)}  ${rel}`);
  return data.length;
}

// app-icon.svg at `size` px with the seal's outer edge on whole pixels: the
// seal spans 16..240 of the 256 box, so the inset is size/16, rounded down
// (20 px gets 1 px, not a soft 1.25 px edge; 40 px gets 2).
function vectorIcon(size, fill, k = null) {
  const [d] = pathsOf(read('app-icon.svg'));
  const inset = Math.floor(size / 16);
  const sc = (size - 2 * inset) / 224;
  const t = inset - 16 * sc;
  return doc(size, size, sealPaths(d, `translate(${t} ${t}) scale(${sc})`, fill, k));
}

// The app icon's sizes; 16 and 24 are the hand-drawn pixel files.
const APP_SIZES = [16, 20, 24, 32, 40, 48, 64, 128, 256];

// The neutral app icon at `size` as straight RGBA.
async function appIconRgba(size) {
  if (size === 16 || size === 24) {
    const svg = recolour(read(`app-icon-${size}.svg`), NEUTRAL);
    return fillCarving(await rgba(svg, size, size), size, size, NEUTRAL_K);
  }
  return rgba(vectorIcon(size, NEUTRAL, NEUTRAL_K), size, size);
}

async function appIcon() {
  const entries = [];
  for (const size of APP_SIZES) {
    const px = await appIconRgba(size);
    if (size === 256) {
      const png = await sharp(px, { raw: { width: size, height: size, channels: 4 } })
        .png({ compressionLevel: 9, palette: false })
        .toBuffer();
      entries.push({ size, data: png });
    } else {
      entries.push({ size, data: dibEntry(px, size, size) });
    }
  }
  write(path.join(OUT10, 'komorebi.ico'), ico(entries));
  // Previews of the icon on a light and a dark taskbar (not shipped; for the
  // review of the change): every size side by side, at 1x and at 4x.
  if (process.env.KOMOREBI_ICON_PREVIEW) {
    const dir = process.env.KOMOREBI_ICON_PREVIEW;
    for (const [name, ground] of [['light', '#F3F3F3'], ['dark', '#202020']]) {
      const pad = 16;
      const sizes = APP_SIZES;
      const W = sizes.reduce((a, s) => a + s + pad, pad);
      const H = 256 + 2 * pad;
      const layers = [];
      let x = pad;
      for (const s of sizes) {
        const png = await sharp(await appIconRgba(s), { raw: { width: s, height: s, channels: 4 } }).png().toBuffer();
        layers.push({ input: png, left: x, top: pad + 256 - s });
        x += s + pad;
      }
      const base = sharp({ create: { width: W, height: H, channels: 4, background: ground } });
      const out = await base.composite(layers).png().toBuffer();
      write(path.join(dir, `app-icon-${name}.png`), out);
      const big = await sharp(out).resize(W * 2, H * 2, { kernel: 'nearest' }).png().toBuffer();
      write(path.join(dir, `app-icon-${name}@2x.png`), big);
    }
  }
}

async function trayIcons() {
  // 16 and 24 from the pixel files (the tray glyph and the 24 px app icon,
  // recoloured), 20 and 32 from the vector icon. The notification area asks
  // for the small icon size: 16 at 100 percent, 20 at 125, 24 at 150, 32 at 200.
  // Each taskbar also has an unread icon: the same glyph with the spot.
  for (const [state, colour] of Object.entries(TRAY)) {
    const plain = [];
    const unread = [];
    for (const size of [16, 20, 24, 32]) {
      const svg =
        size === 16
          ? read(`tray-16-${state}.svg`)
          : size === 24
            ? recolour(read('app-icon-24.svg'), colour)
            : vectorIcon(size, colour);
      plain.push({ size, data: dibEntry(await rgba(svg, size, size), size, size) });
      const spot = withSpot(svg, size, colour);
      unread.push({ size, data: dibEntry(await rgba(spot, size, size), size, size) });
    }
    write(path.join(OUT10, 'tray', `tray-${state}.ico`), ico(plain));
    write(path.join(OUT10, 'tray', `tray-unread-${state}.ico`), ico(unread));
  }
}

async function installerImages() {
  // WizardImageFile (164 x 314) and WizardSmallImageFile (55 x 58), plus 2x
  // copies Inno Setup picks on high DPI. Paper ground, the neutral seal, the
  // wordmark; no gradient.
  for (const scale of [1, 2]) {
    const w = 164 * scale;
    const h = 314 * scale;
    const seal = 56 * scale;
    const ww = 104 * scale;
    const top = 112 * scale;
    const gap = 24 * scale;
    const body = sealAt((w - seal) / 2, top, seal, NEUTRAL, NEUTRAL_K) + wordmarkAt((w - ww) / 2, top + seal + gap, ww, INK);
    const name = scale === 1 ? 'wizard-image.bmp' : 'wizard-image-2x.bmp';
    write(path.join(INSTALLER, name), bmp24(await rgba(doc(w, h, body, PAPER), w, h), w, h));
  }
  for (const scale of [1, 2]) {
    const w = 55 * scale;
    const h = 58 * scale;
    const seal = 40 * scale;
    const body = sealAt(Math.floor((w - seal) / 2), Math.floor((h - seal) / 2), seal, NEUTRAL, NEUTRAL_K);
    const name = scale === 1 ? 'wizard-small.bmp' : 'wizard-small-2x.bmp';
    write(path.join(INSTALLER, name), bmp24(await rgba(doc(w, h, body, PAPER), w, h), w, h));
  }
}

async function appBrand() {
  // SVGs the GPUI views load through their AssetSource, and PNGs at 1x and 2x
  // for anything that wants a bitmap (img()).
  let total = 0;
  const mark = neutral(read('mark.svg'));
  const logo = neutral(read('logo.svg'));
  const logoDark = neutral(read('logo-dark.svg'));
  const svgs = {
    'mark.svg': mark,
    'mark-mono.svg': read('mark-mono.svg'),
    'fall.svg': read('fall.svg'),
    'wordmark.svg': read('wordmark.svg'),
    'wordmark-dark.svg': read('wordmark-dark.svg'),
    'logo.svg': logo,
    'logo-dark.svg': logoDark,
  };
  for (const [name, svg] of Object.entries(svgs)) total += write(path.join(BRAND, name), minify(svg) + '\n');

  const [, , fw, fh] = viewBoxOf(read('fall.svg'));
  const [, , lw, lh] = viewBoxOf(read('logo.svg'));
  const [, , mw, mh] = viewBoxOf(read('wordmark.svg'));
  const pngs = [
    // [name, source svg, width at 1x, height at 1x]
    ['mark-18', mark, 18, 18],
    ['mark-40', mark, 40, 40],
    ['mark-72', mark, 72, 72],
    ['wordmark-24', read('wordmark.svg'), Math.round((24 * mw) / mh), 24],
    ['wordmark-dark-24', read('wordmark-dark.svg'), Math.round((24 * mw) / mh), 24],
    ['logo-40', logo, Math.round((40 * lw) / lh), 40],
    ['logo-dark-40', logoDark, Math.round((40 * lw) / lh), 40],
    ['fall-light', recolour(read('fall.svg'), FALL), 325, (325 * fh) / fw],
    ['fall-dark', recolour(read('fall.svg'), FALL_DARK), 325, (325 * fh) / fw],
  ];
  const index = {
    note: 'Komorebi 1.0 brand assets for the GPUI app. Sources: assets/brand-src; generator: scripts/brand-assets.js; guide: docs/BRANDING.md.',
    colours: { seal: NEUTRAL, seal_k: NEUTRAL_K, ink: INK, ink_dark: INK_DARK, fall: FALL, fall_dark: FALL_DARK },
    svg: {
      mark: { file: 'mark.svg', fill: 'seal + seal_k', use: 'The neutral seal as a picture (ink-3, white K), as the app icon. No accent colour; never in text.' },
      mark_mono: { file: 'mark-mono.svg', fill: 'currentColor', use: 'The ink mark (recoloured with the text colour): the title bar, welcome and the Sent notice.' },
      fall: { file: 'fall.svg', fill: 'currentColor', use: 'The large light shape on the welcome screen only, in the fall tone; never ink.' },
      wordmark: { file: 'wordmark.svg', fill: 'ink', use: 'The drawn wordmark on light grounds.' },
      wordmark_dark: { file: 'wordmark-dark.svg', fill: 'ink_dark', use: 'The drawn wordmark on dark grounds.' },
      logo: { file: 'logo.svg', fill: 'seal + seal_k + ink', use: 'Neutral seal and wordmark lockup on light grounds.' },
      logo_dark: { file: 'logo-dark.svg', fill: 'seal + seal_k + ink_dark', use: 'Neutral seal and wordmark lockup on dark grounds.' },
    },
    png: {},
  };
  for (const [name, svg, w, h] of pngs) {
    const entry = { w, h, x1: `png/${name}.png`, x2: `png/${name}@2x.png` };
    total += write(path.join(BRAND, entry.x1), await pngOf(svg, w, h));
    total += write(path.join(BRAND, entry.x2), await pngOf(svg, w * 2, h * 2));
    index.png[name] = entry;
  }
  total += write(path.join(BRAND, 'index.json'), JSON.stringify(index, null, 2) + '\n');
  console.log(`${String(total).padStart(8)}  assets/brand total (limit 150 KB = 153600)`);
  if (total > 153600) throw new Error('assets/brand is over 150 KB');
}

async function github() {
  // The README logo (light and dark) and the social preview. Screenshots are
  // not touched.
  const [, , lw, lh] = viewBoxOf(read('logo.svg'));
  const w = 640;
  const h = Math.round((w * lh) / lw);
  write(path.join(GITHUB, 'logo.png'), await pngOf(neutral(read('logo.svg')), w, h));
  write(path.join(GITHUB, 'logo-light.png'), await pngOf(neutral(read('logo-dark.svg')), w, h));

  for (const dark of [false, true]) {
    const W = 1280;
    const H = 640;
    const seal = 112;
    const ww = 360;
    const wh = wordmarkHeight(ww);
    const sealTop = 132;
    const wordTop = sealTop + seal + 40;
    const tagTop = wordTop + wh + 84;
    const ink = dark ? INK_DARK : INK;
    const ink2 = dark ? INK2_DARK : INK2;
    const ink3 = dark ? INK3_DARK : INK3;
    const font = `font-family="Segoe UI Variable Display, Segoe UI Variable Text, Segoe UI"`;
    const body =
      sealAt((W - seal) / 2, sealTop, seal) +
      wordmarkAt((W - ww) / 2, wordTop, ww, ink) +
      `<text x="${W / 2}" y="${tagTop}" text-anchor="middle" ${font} font-size="40" fill="${ink2}">Read. Reply. Rest.</text>` +
      `<text x="${W / 2}" y="${tagTop + 60}" text-anchor="middle" ${font} font-size="26" fill="${ink3}">Free for Windows 10 and 11</text>`;
    const png = await pngOf(doc(W, H, body, dark ? SITE_DARK : PAPER), W, H);
    const webp = await sharp(png).webp({ lossless: true }).toBuffer();
    write(path.join(GITHUB, dark ? 'social-preview-dark.webp' : 'social-preview.webp'), webp);
  }
}

(async () => {
  await appIcon();
  await trayIcons();
  await installerImages();
  await appBrand();
  await github();
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
