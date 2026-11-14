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

// Brand tokens (BRAND.md).
const SEAL = '#B3503A';
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
const TRAY = { light: '#1A1A1A', dark: '#FFFFFF', unread: SEAL };

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

// The seal at side `s` with its top-left at (x, y), from mark.svg (256 box).
function sealAt(x, y, s, fill = SEAL) {
  const [d] = pathsOf(read('mark.svg'));
  const k = s / 208; // the seal spans 24..232 of the 256 box
  return `<path transform="translate(${x - 24 * k} ${y - 24 * k}) scale(${k})" fill="${fill}" fill-rule="evenodd" d="${d}"/>`;
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
function vectorIcon(size, fill) {
  const [d] = pathsOf(read('app-icon.svg'));
  const inset = Math.floor(size / 16);
  const k = (size - 2 * inset) / 224;
  const t = inset - 16 * k;
  return doc(size, size, `<path transform="translate(${t} ${t}) scale(${k})" fill="${fill}" fill-rule="evenodd" d="${d}"/>`);
}

async function appIcon() {
  // 16 and 24 are the hand-drawn pixel files; the rest are the vector icon.
  const entries = [];
  for (const size of [16, 20, 24, 32, 40, 48, 64, 128, 256]) {
    const svg =
      size === 16 ? read('app-icon-16.svg') : size === 24 ? read('app-icon-24.svg') : vectorIcon(size, SEAL);
    if (size === 256) {
      entries.push({ size, data: await pngOf(svg, 256, 256) });
    } else {
      entries.push({ size, data: dibEntry(await rgba(svg, size, size), size, size) });
    }
  }
  write(path.join(OUT10, 'komorebi.ico'), ico(entries));
}

async function trayIcons() {
  // 16 and 24 from the pixel files (the tray glyph and the 24 px app icon,
  // recoloured), 20 and 32 from the vector icon. The notification area asks
  // for the small icon size: 16 at 100 percent, 20 at 125, 24 at 150, 32 at 200.
  for (const [state, colour] of Object.entries(TRAY)) {
    const entries = [];
    for (const size of [16, 20, 24, 32]) {
      const svg =
        size === 16
          ? read(`tray-16-${state}.svg`)
          : size === 24
            ? recolour(read('app-icon-24.svg'), colour)
            : vectorIcon(size, colour);
      entries.push({ size, data: dibEntry(await rgba(svg, size, size), size, size) });
    }
    write(path.join(OUT10, 'tray', `tray-${state}.ico`), ico(entries));
  }
}

async function installerImages() {
  // WizardImageFile (164 x 314) and WizardSmallImageFile (55 x 58), plus 2x
  // copies Inno Setup picks on high DPI. Paper ground, the seal, the
  // wordmark; no gradient.
  for (const scale of [1, 2]) {
    const w = 164 * scale;
    const h = 314 * scale;
    const seal = 56 * scale;
    const ww = 104 * scale;
    const top = 112 * scale;
    const gap = 24 * scale;
    const body = sealAt((w - seal) / 2, top, seal) + wordmarkAt((w - ww) / 2, top + seal + gap, ww, INK);
    const name = scale === 1 ? 'wizard-image.bmp' : 'wizard-image-2x.bmp';
    write(path.join(INSTALLER, name), bmp24(await rgba(doc(w, h, body, PAPER), w, h), w, h));
  }
  for (const scale of [1, 2]) {
    const w = 55 * scale;
    const h = 58 * scale;
    const seal = 40 * scale;
    const body = sealAt(Math.floor((w - seal) / 2), Math.floor((h - seal) / 2), seal);
    const name = scale === 1 ? 'wizard-small.bmp' : 'wizard-small-2x.bmp';
    write(path.join(INSTALLER, name), bmp24(await rgba(doc(w, h, body, PAPER), w, h), w, h));
  }
}

async function appBrand() {
  // SVGs the GPUI views load through their AssetSource, and PNGs at 1x and 2x
  // for anything that wants a bitmap (img()).
  let total = 0;
  const svgs = {
    'mark.svg': read('mark.svg'),
    'mark-mono.svg': read('mark-mono.svg'),
    'fall.svg': read('fall.svg'),
    'wordmark.svg': read('wordmark.svg'),
    'wordmark-dark.svg': read('wordmark-dark.svg'),
    'logo.svg': read('logo.svg'),
    'logo-dark.svg': read('logo-dark.svg'),
  };
  for (const [name, svg] of Object.entries(svgs)) total += write(path.join(BRAND, name), minify(svg) + '\n');

  const [, , fw, fh] = viewBoxOf(read('fall.svg'));
  const [, , lw, lh] = viewBoxOf(read('logo.svg'));
  const [, , mw, mh] = viewBoxOf(read('wordmark.svg'));
  const pngs = [
    // [name, source svg, width at 1x, height at 1x]
    ['mark-18', read('mark.svg'), 18, 18],
    ['mark-40', read('mark.svg'), 40, 40],
    ['mark-72', read('mark.svg'), 72, 72],
    ['wordmark-24', read('wordmark.svg'), Math.round((24 * mw) / mh), 24],
    ['wordmark-dark-24', read('wordmark-dark.svg'), Math.round((24 * mw) / mh), 24],
    ['logo-40', read('logo.svg'), Math.round((40 * lw) / lh), 40],
    ['logo-dark-40', read('logo-dark.svg'), Math.round((40 * lw) / lh), 40],
    ['fall-light', recolour(read('fall.svg'), FALL), 325, (325 * fh) / fw],
    ['fall-dark', recolour(read('fall.svg'), FALL_DARK), 325, (325 * fh) / fw],
  ];
  const index = {
    note: 'Komorebi 1.0 brand assets for the GPUI app. Sources: assets/brand-src; generator: scripts/brand-assets.js; guide: docs/BRANDING.md.',
    colours: { seal: SEAL, ink: INK, ink_dark: INK_DARK, fall: FALL, fall_dark: FALL_DARK },
    svg: {
      mark: { file: 'mark.svg', fill: 'seal', use: 'The seal: welcome, About, the Sent notice. Persimmon, never in text.' },
      mark_mono: { file: 'mark-mono.svg', fill: 'currentColor', use: 'The ink mark in the title bar (recoloured with the text colour).' },
      fall: { file: 'fall.svg', fill: 'currentColor', use: 'The large light shape on the welcome screen only, in the fall tone; never persimmon or ink.' },
      wordmark: { file: 'wordmark.svg', fill: 'ink', use: 'The drawn wordmark on light grounds.' },
      wordmark_dark: { file: 'wordmark-dark.svg', fill: 'ink_dark', use: 'The drawn wordmark on dark grounds.' },
      logo: { file: 'logo.svg', fill: 'seal + ink', use: 'Seal and wordmark lockup on light grounds (About).' },
      logo_dark: { file: 'logo-dark.svg', fill: 'seal + ink_dark', use: 'Seal and wordmark lockup on dark grounds (About).' },
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
  write(path.join(GITHUB, 'logo.png'), await pngOf(read('logo.svg'), w, h));
  write(path.join(GITHUB, 'logo-light.png'), await pngOf(read('logo-dark.svg'), w, h));

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
