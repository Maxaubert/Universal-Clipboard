// Tray and menu bar icons (tray spec 3.7): writes the source SVGs to packaging/assets/tray/svg
// and the PNGs the window shell includes (apps/desktop/src-tauri/icons/tray).
//
// Icon A (three layers, docs/design/tray/index.html) with a badge in the lower right corner:
//   idle      no badge
//   busy      a down arrow (files move)
//   paused    two bars
//   attention a dot (amber on Windows and Linux, one colour on macOS)
// Colours: "light" (white, for a dark taskbar or panel), "dark" (black, for a light taskbar),
// "template" (black on clear; macOS turns it into the menu bar colour).
//
// Run once after a change, then commit the files:
//   npm i --no-save @resvg/resvg-js   (in any folder; pass that folder's node_modules)
//   node tools/tray-icons/render.mjs [path/to/node_modules]
import { mkdirSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, '..', '..');
const req = createRequire(join(resolve(process.argv[2] ?? join(root, 'node_modules')), 'x.js'));
const { Resvg } = req('@resvg/resvg-js');

const LAYERS = (ink, w) => `
  <g fill="none" stroke="${ink}" stroke-width="${w}" stroke-linecap="round" stroke-linejoin="round">
    <path d="M2.2 6.2L8 3.6l5.8 2.6L8 8.8z"/><path d="M2.2 8.9L8 11.5l5.8-2.6"/><path d="M2.2 11.5L8 14.1l5.8-2.6"/>
  </g>
  <rect x="6.1" y="2.9" width="3.8" height="1.6" rx=".7" fill="${ink}" opacity=".6"/>`;

const BADGE = {
  idle: () => '',
  busy: (ink) =>
    `<path d="M12.8 9.7v5.3M10.6 12.9l2.2 2.2 2.2-2.2" fill="none" stroke="${ink}" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>`,
  paused: (ink) =>
    `<rect x="10.9" y="10.2" width="1.5" height="5" rx=".5" fill="${ink}"/><rect x="13.3" y="10.2" width="1.5" height="5" rx=".5" fill="${ink}"/>`,
  attention: (_ink, warn) => `<circle cx="12.8" cy="12.8" r="2.6" fill="${warn}"/>`,
};

const COLOURS = {
  light: { ink: '#FFFFFF', warn: '#F08A4B' },
  dark: { ink: '#000000', warn: '#C2410C' },
  template: { ink: '#000000', warn: '#000000' },
};

function svg(state, colour, strokeW) {
  const { ink, warn } = COLOURS[colour];
  const cut = state === 'idle' ? '' : ' mask="url(#cut)"';
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" width="16" height="16">
  <defs><mask id="cut"><rect width="16" height="16" fill="#fff"/><circle cx="12.8" cy="12.8" r="3.9" fill="#000"/></mask></defs>
  <g${cut}>${LAYERS(ink, strokeW)}</g>
  ${BADGE[state](ink, warn)}
</svg>
`;
}

const svgDir = join(root, 'packaging', 'assets', 'tray', 'svg');
const pngDir = join(root, 'apps', 'desktop', 'src-tauri', 'icons', 'tray');
mkdirSync(svgDir, { recursive: true });
mkdirSync(pngDir, { recursive: true });

// Windows picks by the scale of the screen (16 px at 100 %, 20, 24, 32 at 200 %); Linux uses
// 32; macOS 36 (18 pt at 2x). Thin screens get a slightly heavier stroke so lines stay clear.
const SETS = [
  ...['light', 'dark'].flatMap((c) => [16, 20, 24, 32].map((px) => [c, px])),
  ['template', 36],
];
let n = 0;
for (const state of Object.keys(BADGE)) {
  for (const colour of Object.keys(COLOURS)) {
    writeFileSync(join(svgDir, `${state}-${colour}.svg`), svg(state, colour, 1.15));
  }
  for (const [colour, px] of SETS) {
    const w = px <= 16 ? 1.2 : px <= 24 ? 1.2 : 1.15;
    const png = new Resvg(svg(state, colour, w), { fitTo: { mode: 'width', value: px } }).render().asPng();
    writeFileSync(join(pngDir, `${state}-${colour}-${px}.png`), png);
    n += 1;
  }
}
console.log(`wrote ${n} PNGs to ${pngDir}`);
