// Makes the bundled provider icons (assets/providers/<id>.png at 32 px and
// <id>@2x.png at 64 px) from the providers' own icons, downloaded once by
// hand from the URLs in assets/providers/README.md (never at run time).
//
//   $env:NODE_PATH = 'C:\dev-overflow\komorebi-brand-tools\node_modules'
//   $env:ZOHO_CROP = '0 0 290 296'   # the envelope of Zoho's wide logo
//   node scripts/provider-icons.js <raw dir> <raw2 dir> assets/providers [preview.png]
//
// The same tools as scripts/brand-assets.js (resvg, sharp). SVGs with a
// script, foreignObject or an external reference are refused. A transparent
// margin is trimmed; each icon is fitted into a square and saved as a
// palette PNG at the highest compression.
'use strict';
const fs = require('fs');
const path = require('path');
const { Resvg } = require('@resvg/resvg-js');
const sharp = require('sharp');

const [RAW, RAW2, OUT, PREVIEW] = process.argv.slice(2);

// id -> [file, crop] (crop: a viewBox for a wide SVG logo, to keep its mark)
const SOURCES = {
  aol: [path.join(RAW, 'aol.png')],
  fastmail: [path.join(RAW, 'fastmail.svg')],
  free: [path.join(RAW, 'free.ico')],
  gmail: [path.join(RAW, 'gmail.svg')],
  'google-workspace': [path.join(RAW2, 'google-g.png')],
  gmx: [path.join(RAW, 'gmx.svg')],
  icloud: [path.join(RAW, 'icloud.png')],
  infomaniak: [path.join(RAW, 'infomaniak.ico')],
  libero: [path.join(RAW, 'libero.svg')],
  mailcom: [path.join(RAW2, 'mailcom-touch.png')],
  mailboxorg: [path.join(RAW, 'mailboxorg.svg')],
  orange: [path.join(RAW, 'orange.png')],
  outlook: [path.join(RAW, 'outlook.png')],
  postale: [path.join(RAW, 'postale.svg')],
  posteo: [path.join(RAW, 'posteo.png')],
  proton: [path.join(RAW, 'proton.svg')],
  purelymail: [path.join(RAW, 'purelymail.ico')],
  seznam: [path.join(RAW, 'seznam.png')],
  webde: [path.join(RAW, 'webde.ico')],
  yahoo: [path.join(RAW2, 'yahoo-apple-touch-icon.png')],
  yandex: [path.join(RAW, 'yandex.png')],
  zoho: [path.join(RAW, 'zoho.svg'), process.env.ZOHO_CROP],
};

// The largest image of an ICO, as PNG bytes.
async function fromIco(b) {
  if (b[0] === 0x89) return b; // a PNG under an .ico name
  const n = b.readUInt16LE(4);
  let best = null;
  for (let i = 0; i < n; i++) {
    const o = 6 + i * 16;
    const w = b[o] || 256;
    const size = b.readUInt32LE(o + 8);
    const off = b.readUInt32LE(o + 12);
    if (!best || w > best.w) best = { w, data: b.subarray(off, off + size) };
  }
  const d = best.data;
  if (d[0] === 0x89) return Buffer.from(d);
  const w = d.readInt32LE(4);
  const h = d.readInt32LE(8) / 2;
  const bpp = d.readUInt16LE(14);
  if (bpp !== 32) throw new Error(`ico entry ${bpp} bpp`);
  const raw = Buffer.alloc(w * h * 4);
  for (let y = 0; y < h; y++)
    for (let x = 0; x < w; x++) {
      const s = 40 + ((h - 1 - y) * w + x) * 4;
      const t = (y * w + x) * 4;
      raw[t] = d[s + 2];
      raw[t + 1] = d[s + 1];
      raw[t + 2] = d[s];
      raw[t + 3] = d[s + 3];
    }
  return sharp(raw, { raw: { width: w, height: h, channels: 4 } }).png().toBuffer();
}

function checkSvg(svg, file) {
  if (/<script|foreignObject|xlink:href="http|href="http|@import|url\(\s*['"]?http/i.test(svg)) {
    throw new Error(`${file}: unsafe SVG`);
  }
}

async function render(file, crop, size) {
  const b = fs.readFileSync(file);
  let png;
  if (file.endsWith('.svg')) {
    let svg = b.toString('utf8').replace(/<!DOCTYPE[^>]*>/i, '');
    checkSvg(svg, file);
    if (crop) svg = svg.replace(/viewBox="[^"]*"/, `viewBox="${crop}"`).replace(/\s(width|height)="[^"]*"/g, '');
    png = new Resvg(svg, { fitTo: { mode: 'width', value: size * 4 } }).render().asPng();
  } else if (file.endsWith('.ico')) {
    png = await fromIco(b);
  } else {
    png = b;
  }
  // Trim only a transparent margin: an opaque icon's own ground is the icon.
  const { data } = await sharp(png).ensureAlpha().raw().toBuffer({ resolveWithObject: true });
  let img = sharp(png);
  if (data[3] === 0) img = sharp(await img.trim({ threshold: 0 }).toBuffer());
  return img
    .resize(size, size, { fit: 'contain', background: { r: 0, g: 0, b: 0, alpha: 0 }, kernel: 'lanczos3' })
    .png({ compressionLevel: 9, palette: true, quality: 100, effort: 10 })
    .toBuffer();
}

(async () => {
  fs.mkdirSync(OUT, { recursive: true });
  const previews = [];
  let total = 0;
  let x = 8;
  for (const [id, [file, crop]] of Object.entries(SOURCES)) {
    const one = await render(file, crop, 32);
    const two = await render(file, crop, 64);
    fs.writeFileSync(path.join(OUT, `${id}.png`), one);
    fs.writeFileSync(path.join(OUT, `${id}@2x.png`), two);
    total += one.length + two.length;
    console.log(`${id.padEnd(18)} ${String(one.length).padStart(6)} ${String(two.length).padStart(6)}`);
    previews.push({ input: two, left: x, top: 8 }, { input: one, left: x + 16, top: 80 });
    x += 72;
  }
  console.log(`total ${total}`);
  if (PREVIEW) {
    for (const [name, bg] of [['light', '#FFFFFF'], ['dark', '#1A1A1A']]) {
      await sharp({ create: { width: x, height: 120, channels: 4, background: bg } })
        .composite(previews)
        .png()
        .toFile(PREVIEW.replace('.png', `-${name}.png`));
    }
  }
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
