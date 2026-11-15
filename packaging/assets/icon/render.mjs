// Render every app icon file from the "Paper Layers" SVG (source/paper-layers.svg).
//
//   cd <any scratch folder> && npm i @resvg/resvg-js@2.6.2 sharp@0.33.5
//   node packaging/assets/icon/render.mjs <that scratch folder>
//
// Writes the derived master SVGs next to this file, then the PNG, ICO and iconset files under
// packaging/assets (see README.md). The Tauri icons are made after this with `npx tauri icon`
// (README.md has the commands). Nothing here runs at build time: the outputs are committed.
import { createRequire } from 'node:module';
import { mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const deps = process.argv[2];
if (!deps) throw new Error('usage: node render.mjs <folder with @resvg/resvg-js and sharp>');
const req = createRequire(join(resolve(deps), 'package.json'));
const { Resvg } = req('@resvg/resvg-js');
const sharp = req('sharp');

const HERE = dirname(fileURLToPath(import.meta.url));
const ASSETS = resolve(HERE, '..');

// --- The art. Same colours and shapes as source/paper-layers.svg (1024 canvas). ---
const GRAD = '<linearGradient id="tile" x2=".74" y2="1"><stop stop-color="#34373f"/><stop offset="1" stop-color="#101116"/></linearGradient>';
// Full symbol: three layers (a sheet and two layers under it) and the grey clip.
const SYMBOL =
  '<path d="M281 408L512 303L743 408L512 513Z M281 516L512 621L743 516M281 620L512 725L743 620" fill="none" stroke="#ffffff" stroke-width="42" stroke-linecap="round" stroke-linejoin="round"/>' +
  '<rect x="437" y="278" width="150" height="66" rx="26" fill="#9b9da6"/>';
// Small symbol (16 to 32 px): two layers, bigger and with a thicker stroke, so the layers do
// not run together. Same proportions as the full sheet (231:105).
const SYMBOL_SMALL =
  '<path d="M182 430L512 280L842 430L512 580Z M182 610L512 760L842 610" fill="none" stroke="#ffffff" stroke-width="104" stroke-linecap="round" stroke-linejoin="round"/>' +
  '<rect x="392" y="214" width="240" height="112" rx="44" fill="#9b9da6"/>';

const svg = (body, defs = '') =>
  `<svg width="1024" height="1024" viewBox="0 0 1024 1024" fill="none" xmlns="http://www.w3.org/2000/svg">\n<defs>${GRAD}${defs}</defs>\n${body}\n</svg>\n`;
const scaled = (s, k) => `<g transform="translate(512 512) scale(${k}) translate(-512 -512)">${s}</g>`;

// Windows and Linux: a rounded tile with a small margin.
const tile = svg(`<rect x="32" y="32" width="960" height="960" rx="208" fill="url(#tile)"/>\n${scaled(SYMBOL, 960 / 1024)}`);
const tileSmall = svg(`<rect x="0" y="0" width="1024" height="1024" rx="192" fill="url(#tile)"/>\n${SYMBOL_SMALL}`);

// macOS: the Big Sur icon grid. Body 824x824 at (100,100), continuous corners (radius 185.4,
// 60% smoothing, the squircle Apple's template uses), a soft drop shadow in the margin.
function squircle(x, y, w, h, R, xi) {
  const rad = (d) => (d * Math.PI) / 180;
  const p = (1 + xi) * R;
  const arc = 90 * (1 - xi);
  const s = Math.sin(rad(arc / 2)) * R * Math.SQRT2;
  const alpha = (90 - arc) / 2;
  const p34 = R * Math.tan(rad(alpha / 2));
  const beta = 45 * xi;
  const c = p34 * Math.cos(rad(beta));
  const d = c * Math.tan(rad(beta));
  const b = (p - s - c - d) / 3;
  const a = 2 * b;
  const f = (n) => +n.toFixed(3);
  const [A, B, C, Dd, Sx] = [a, b, c, d, s].map(f);
  return [
    `M${f(x + w - p)} ${f(y)}`,
    `c${A} 0 ${f(a + b)} 0 ${f(a + b + c)} ${Dd}`, `a${f(R)} ${f(R)} 0 0 1 ${Sx} ${Sx}`, `c${Dd} ${C} ${Dd} ${f(b + c)} ${Dd} ${f(a + b + c)}`,
    `L${f(x + w)} ${f(y + h - p)}`,
    `c0 ${A} 0 ${f(a + b)} ${-Dd} ${f(a + b + c)}`, `a${f(R)} ${f(R)} 0 0 1 ${-Sx} ${Sx}`, `c${-C} ${Dd} ${f(-(b + c))} ${Dd} ${f(-(a + b + c))} ${Dd}`,
    `L${f(x + p)} ${f(y + h)}`,
    `c${-A} 0 ${f(-(a + b))} 0 ${f(-(a + b + c))} ${-Dd}`, `a${f(R)} ${f(R)} 0 0 1 ${-Sx} ${-Sx}`, `c${-Dd} ${-C} ${-Dd} ${f(-(b + c))} ${-Dd} ${f(-(a + b + c))}`,
    `L${f(x)} ${f(y + p)}`,
    `c0 ${-A} 0 ${f(-(a + b))} ${Dd} ${f(-(a + b + c))}`, `a${f(R)} ${f(R)} 0 0 1 ${Sx} ${-Sx}`, `c${C} ${-Dd} ${f(b + c)} ${-Dd} ${f(a + b + c)} ${-Dd}`,
    'Z',
  ].join(' ');
}
const BODY = squircle(100, 100, 824, 824, 185.4, 0.6);
const SHADOW = '<filter id="shadow" x="0" y="0" width="1024" height="1024" filterUnits="userSpaceOnUse"><feDropShadow dx="0" dy="10" stdDeviation="12" flood-color="#000" flood-opacity="0.3"/></filter>';
const mac = (sym) =>
  svg(`<path d="${BODY}" fill="url(#tile)" filter="url(#shadow)"/>\n${scaled(sym, 824 / 1024)}`, SHADOW);
const macos = mac(SYMBOL);
const macosSmall = mac(SYMBOL_SMALL);

for (const [n, s] of Object.entries({ 'tile.svg': tile, 'tile-small.svg': tileSmall, 'macos.svg': macos, 'macos-small.svg': macosSmall })) {
  writeFileSync(join(HERE, n), s);
}

// --- Rendering ---
const render = (s, px) => Buffer.from(new Resvg(s, { fitTo: { mode: 'width', value: px }, shapeRendering: 2 }).render().asPng());
const SMALL_MAX = 32; // 16, 20, 24, 30, 32 use the two-layer art
const tileFor = (px) => (px <= SMALL_MAX ? tileSmall : tile);
const macFor = (px) => (px <= SMALL_MAX ? macosSmall : macos);
const put = (rel, buf) => {
  const p = join(ASSETS, rel);
  mkdirSync(dirname(p), { recursive: true });
  writeFileSync(p, buf);
};
// Art of `px` centred on a transparent `canvas`.
async function padded(s, px, canvas) {
  const off = Math.round((canvas - px) / 2);
  return sharp({ create: { width: canvas, height: canvas, channels: 4, background: { r: 0, g: 0, b: 0, alpha: 0 } } })
    .composite([{ input: render(s, px), left: off, top: off }]).png().toBuffer();
}

// Source PNGs for `npx tauri icon` (README.md).
put('icon/build/tauri-source.png', render(tile, 1024));
put('icon/build/tauri-source-macos.png', render(macos, 1024));

// Windows .ico: 16..48 as 32-bit BMP entries (any loader reads them), 256 as PNG.
async function ico(sizes) {
  const entries = [];
  for (const px of sizes) {
    const png = render(tileFor(px), px);
    if (px >= 256) { entries.push({ px, data: png }); continue; }
    const { data } = await sharp(png).ensureAlpha().raw().toBuffer({ resolveWithObject: true });
    const maskRow = Math.ceil(px / 32) * 4;
    const bmp = Buffer.alloc(40 + px * px * 4 + maskRow * px);
    bmp.writeUInt32LE(40, 0); bmp.writeInt32LE(px, 4); bmp.writeInt32LE(px * 2, 8);
    bmp.writeUInt16LE(1, 12); bmp.writeUInt16LE(32, 14);
    bmp.writeUInt32LE(px * px * 4 + maskRow * px, 20);
    for (let y = 0; y < px; y++) {
      for (let x = 0; x < px; x++) {
        const si = (y * px + x) * 4;
        const di = 40 + ((px - 1 - y) * px + x) * 4;
        bmp[di] = data[si + 2]; bmp[di + 1] = data[si + 1]; bmp[di + 2] = data[si]; bmp[di + 3] = data[si + 3];
      }
    }
    // AND mask stays 0: the alpha channel decides.
    entries.push({ px, data: bmp });
  }
  const head = Buffer.alloc(6 + 16 * entries.length);
  head.writeUInt16LE(1, 2); head.writeUInt16LE(entries.length, 4);
  let off = head.length;
  entries.forEach((e, i) => {
    const o = 6 + 16 * i;
    head[o] = e.px >= 256 ? 0 : e.px; head[o + 1] = e.px >= 256 ? 0 : e.px;
    head.writeUInt16LE(1, o + 4); head.writeUInt16LE(32, o + 6);
    head.writeUInt32LE(e.data.length, o + 8); head.writeUInt32LE(off, o + 12);
    off += e.data.length;
  });
  return Buffer.concat([head, ...entries.map((e) => e.data)]);
}
const icoBuf = await ico([16, 20, 24, 32, 40, 48, 64, 256]);
put('icon.ico', icoBuf);
put('icon/build/icon.ico', icoBuf);

// MSIX (packaging/windows/build.ps1 + makepri). BackgroundColor is transparent, so the plated
// and unplated app-list icons are the same tile: the tile is the plate.
const SCALES = [100, 125, 150, 200, 400];
for (const sc of SCALES) {
  const px44 = Math.round((44 * sc) / 100);
  put(`msix/Square44x44Logo.scale-${sc}.png`, render(tileFor(px44), px44));
  const px150 = Math.round((150 * sc) / 100);
  put(`msix/Square150x150Logo.scale-${sc}.png`, await padded(tile, Math.round(px150 * 0.66), px150));
  const px50 = Math.round((50 * sc) / 100);
  put(`msix/StoreLogo.scale-${sc}.png`, render(tile, px50));
}
for (const t of [16, 20, 24, 30, 32, 36, 40, 48, 60, 64, 72, 80, 96, 256]) {
  const png = render(tileFor(t), t);
  put(`msix/Square44x44Logo.targetsize-${t}.png`, png);
  put(`msix/Square44x44Logo.targetsize-${t}_altform-unplated.png`, png);
  put(`msix/Square44x44Logo.targetsize-${t}_altform-lightunplated.png`, png);
}

// Linux hicolor theme + scalable SVG.
rmSync(join(ASSETS, 'linux'), { recursive: true, force: true });
for (const s of [16, 22, 24, 32, 48, 64, 128, 256, 512]) {
  put(`linux/hicolor/${s}x${s}/apps/universal-clipboard.png`, render(tileFor(s), s));
}
put('linux/hicolor/scalable/apps/universal-clipboard.svg', Buffer.from(tile));

// macOS iconset (packaging/macos/build-app.sh runs iconutil on the Mac).
rmSync(join(ASSETS, 'macos'), { recursive: true, force: true });
for (const s of [16, 32, 128, 256, 512]) {
  put(`macos/AppIcon.iconset/icon_${s}x${s}.png`, render(macFor(s), s));
  put(`macos/AppIcon.iconset/icon_${s}x${s}@2x.png`, render(macFor(s * 2), s * 2));
}
console.log('done');
