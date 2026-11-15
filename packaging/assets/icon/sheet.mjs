// Contact sheet of the rendered icons at real size (docs/design/icon/contact-sheet.png).
//   node packaging/assets/icon/sheet.mjs <deps folder> [extra output path]
// Run render.mjs first. Rows: real sizes on light and dark, then 16/24/32 zoomed 4x
// (nearest neighbour) with the full three-layer art beside the small two-layer art.
import { createRequire } from 'node:module';
import { readFileSync, writeFileSync, mkdirSync, copyFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const req = createRequire(join(resolve(process.argv[2]), 'package.json'));
const { Resvg } = req('@resvg/resvg-js');
const sharp = req('sharp');
const HERE = dirname(fileURLToPath(import.meta.url));
const A = resolve(HERE, '..');
const ROOT = resolve(A, '../..');
const read = (p) => readFileSync(join(A, p));
const svgOf = (n) => readFileSync(join(HERE, n), 'utf8');
const render = (s, px) => Buffer.from(new Resvg(s, { fitTo: { mode: 'width', value: px } }).render().asPng());

const W = 1400;
const comps = [];
const texts = [];
let y = 24;
const label = (t, x, yy, fill) => texts.push(`<text x="${x}" y="${yy}" font-family="Segoe UI, Arial, sans-serif" font-size="15" fill="${fill}">${t}</text>`);

function row(title, items, bg, fg) {
  const h = Math.max(...items.map((i) => i.px)) + 56;
  comps.push({ input: { create: { width: W, height: h, channels: 4, background: bg } }, left: 0, top: y });
  label(title, 16, y + 22, fg);
  let x = 16;
  for (const it of items) {
    comps.push({ input: it.buf, left: x, top: y + 36 });
    label(it.cap, x, y + 36 + it.px + 18, fg);
    x += Math.max(it.px, it.cap.length * 8) + 28;
  }
  y += h + 8;
}
const LIGHT = { r: 243, g: 243, b: 243, alpha: 1 };
const DARK = { r: 32, g: 32, b: 32, alpha: 1 };
const sizes = [16, 24, 32, 48, 64, 128, 256];
const lin = (s) => ({ px: s, cap: `${s}`, buf: read(`linux/hicolor/${s}x${s}/apps/universal-clipboard.png`) });
const linSizes = [16, 24, 32, 48, 64, 128, 256];
row('Windows / Linux tile (real size, light)', linSizes.map(lin), LIGHT, '#222');
row('Windows / Linux tile (real size, dark)', linSizes.map(lin), DARK, '#eee');
const mac = (f, px, cap) => ({ px, cap, buf: read(`macos/AppIcon.iconset/${f}`) });
const macItems = [mac('icon_16x16.png', 16, '16'), mac('icon_32x32.png', 32, '32'), mac('icon_32x32@2x.png', 64, '64'), mac('icon_128x128.png', 128, '128'), mac('icon_256x256.png', 256, '256'), mac('icon_512x512.png', 512, '512')];
row('macOS (Big Sur grid: 824 body, continuous corners, shadow), light', macItems, LIGHT, '#222');
row('macOS, dark', macItems.slice(0, 5), DARK, '#eee');
const msix = (f, cap) => ({ cap, buf: read(`msix/${f}`) });
const ms = await Promise.all([
  msix('StoreLogo.scale-100.png', 'StoreLogo 50'), msix('Square44x44Logo.scale-100.png', '44x44 100%'), msix('Square44x44Logo.scale-200.png', '44x44 200%'),
  msix('Square150x150Logo.scale-100.png', '150x150 100%'),
].map(async (i) => ({ ...i, px: (await sharp(i.buf).metadata()).width })));
row('MSIX assets (Start, taskbar, Store)', ms, LIGHT, '#222');
const zoom = async (s, px, cap) => ({ px: px * 4, cap, buf: await sharp(render(s, px)).resize(px * 4, px * 4, { kernel: 'nearest' }).png().toBuffer() });
const z = [];
for (const px of [16, 24, 32]) {
  z.push(await zoom(svgOf('tile.svg'), px, `${px} three layers`));
  z.push(await zoom(svgOf('tile-small.svg'), px, `${px} small art (used)`));
}
row('16 / 24 / 32 zoomed 4x: full art vs the small two-layer art', z, LIGHT, '#222');

const H = y + 8;
const out = await sharp({ create: { width: W, height: H, channels: 4, background: { r: 255, g: 255, b: 255, alpha: 1 } } })
  .composite([...comps, { input: Buffer.from(`<svg width="${W}" height="${H}" xmlns="http://www.w3.org/2000/svg">${texts.join('')}</svg>`), left: 0, top: 0 }])
  .png().toBuffer();
const dest = join(ROOT, 'docs/design/icon/contact-sheet.png');
mkdirSync(dirname(dest), { recursive: true });
writeFileSync(dest, out);
if (process.argv[3]) copyFileSync(dest, process.argv[3]);
console.log(dest);
