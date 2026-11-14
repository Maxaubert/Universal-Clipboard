// Visual check against the design. Each screen of the app (Vite build, mock agent) is
// screenshotted headless, after its motion has settled, and compared with its reference
// rendered in the same browser:
// - 'design': the locked design file (docs/design/desktop/L01..L12),
// - 'mockup': the end frame of a picked motion mockup (docs/design/desktop/motion), for Home,
//   which is H3 "You are here" since 2026-11-14 (Max),
// - 'baseline': a picture of the app itself (tests/e2e/baselines), where no design or mockup
//   shows the new layout (Paused in the H3 layout). UPDATE_BASELINES=1 writes it again.
// Side-by-side images (reference | app | difference) go to docs/design/desktop/impl-check/.
// The QR code is hidden in both pictures: the design has a fixed picture and the app draws the
// real one.
import { expect, test, type Page } from '@playwright/test';
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import pixelmatch from 'pixelmatch';
import { PNG } from 'pngjs';

const designDir = path.resolve(process.cwd(), '../../docs/design/desktop');
const motionDir = path.join(designDir, 'motion');
const outDir = path.join(designDir, 'impl-check');
const baseDir = path.resolve(process.cwd(), 'tests/e2e/baselines');
const fontDir = path.resolve(process.cwd(), 'node_modules/@fontsource/figtree/files');
const MAX_DIFF_PERCENT = 1;

interface Screen {
  id: string;
  file: string;
  scenario: string;
  route?: string;
  /** More query switches for the mock (for example update=update-failed). */
  extra?: Record<string, string>;
  act?: (page: Page) => Promise<void>;
  /** A higher limit where the app differs from the reference on purpose. */
  maxDiff?: number;
  /** What the app is compared with (default 'design'). */
  ref?: { kind: 'design' } | { kind: 'mockup'; page: string; option: string } | { kind: 'baseline' };
  why?: string;
}

// Decision 2026-11-14: no OS asks anything in setup, so the setup screens show 4 steps, not the
// design's 5 (a few pixels of the step dots differ). The permissions screen shows only when
// macOS blocks something ('mac-blocked'): L02 shows the two macOS rows instead of the design's
// Windows rows.
const screens: Screen[] = [
  { id: 'L01', file: 'L01-welcome', scenario: 'fresh', route: 'welcome' },
  // L02: the macOS rows (other titles and icons) differ from the design's Windows rows on
  // purpose; the layout is the same. 0.93% on 2026-11-14.
  { id: 'L02', file: 'L02-permissions', scenario: 'mac-blocked', route: 'perms', maxDiff: 2 },
  { id: 'L03', file: 'L03-add', scenario: 'fresh', route: 'add' },
  { id: 'L04', file: 'L04-show-code', scenario: 'fresh', route: 'show' },
  {
    id: 'L05',
    file: 'L05-enter-code',
    scenario: 'fresh',
    route: 'enter',
    act: async (p) => {
      await p.getByRole('textbox', { name: 'Code' }).pressSequentially('4182');
    },
  },
  { id: 'L06', file: 'L06-connected', scenario: 'fresh', route: 'connected' },
  // Home is H3 "You are here" (2026-11-14): compared with the H3 mockup's last frame, with the
  // mockup's devices ('h3'). The mockup counts the other devices ("2 devices connected"), the
  // app counts all connected devices with this one ("3 devices connected"), as on L07: one digit.
  {
    id: 'L07',
    file: 'L07-home',
    scenario: 'h3',
    ref: { kind: 'mockup', page: 'home.html', option: 'h3' },
    why: 'H3 mockup end frame',
  },
  { id: 'L08', file: 'L08-device', scenario: 'home', route: 'device' },
  { id: 'L09', file: 'L09-settings', scenario: 'home', route: 'settings' },
  { id: 'L10', file: 'L10-paused', scenario: 'paused', ref: { kind: 'baseline' }, why: 'app baseline (H3 layout, paused)' },
  { id: 'L11', file: 'L11-attention', scenario: 'attention' },
  { id: 'L12', file: 'L12-other-network', scenario: 'home', route: 'vpn' },
  // Update (Max's pick 2026-11-14, motion/update.html): U1 "One shape" and U3 "Home card". App
  // baselines like L10: the U1 frames match the mockup (0.06 to 0.15% on 2026-11-14), but the
  // mockup has no frame of the app's H3 Home with the card, so all six are pictures of the app.
  { id: 'U1', file: 'U1-ready', scenario: 'update', route: 'update', ref: { kind: 'baseline' }, why: 'app baseline (U1 ready)' },
  { id: 'U1', file: 'U1-updating', scenario: 'update-applying', route: 'update', ref: { kind: 'baseline' }, why: 'app baseline (U1 updating)' },
  { id: 'U1', file: 'U1-updated', scenario: 'updated', route: 'update', ref: { kind: 'baseline' }, why: 'app baseline (U1 updated)' },
  { id: 'U1', file: 'U1-failed', scenario: 'update-failed', route: 'update', ref: { kind: 'baseline' }, why: 'app baseline (U1 failed)' },
  // U3: the baselines were made on Windows; Linux CI draws the many lines of text a little
  // differently (1.06% and 0.88% on 2026-11-14, text edges only), so the limit is 2%.
  { id: 'U3', file: 'U3-card', scenario: 'h3', extra: { update: 'update' }, maxDiff: 2, ref: { kind: 'baseline' }, why: 'app baseline (U3 card on H3 Home)' },
  { id: 'U3', file: 'U3-card-failed', scenario: 'h3', extra: { update: 'update-failed' }, maxDiff: 2, ref: { kind: 'baseline' }, why: 'app baseline (U3 card, failed)' },
];

function fontCss(): string {
  return [400, 500, 600, 700]
    .map((w) => {
      const b64 = fs.readFileSync(path.join(fontDir, `figtree-latin-${w}-normal.woff2`)).toString('base64');
      return `@font-face{font-family:'Figtree';font-weight:${w};font-style:normal;src:url(data:font/woff2;base64,${b64}) format('woff2');}`;
    })
    .join('');
}

const hideQr = 'svg[aria-label^="QR code"]{visibility:hidden !important}';
const clip = { x: 0, y: 0, width: 540, height: 720 };

/** Wait until no animation runs (screen changes, entrances, rolling text). */
async function settle(page: Page) {
  await page.waitForFunction(() => document.getAnimations().every((a) => a.playState !== 'running' && !a.pending), undefined, {
    timeout: 10_000,
  });
}

async function shotDesign(page: Page, file: string): Promise<Buffer> {
  let html = fs.readFileSync(path.join(designDir, `${file}.dc.html`), 'utf8');
  html = html.replace(/<link[^>]*fonts\.googleapis[^>]*>/g, '').replace(/<script[^>]*support\.js[^>]*><\/script>/g, '');
  await page.setContent(html, { waitUntil: 'load' });
  await page.addStyleTag({ content: fontCss() + hideQr });
  await page.evaluate(() => document.fonts.ready);
  return page.screenshot({ clip });
}

/** The end frame of one option of a motion mockup, alone at 540 x 720 (scale 1). */
async function shotMockup(page: Page, file: string, option: string): Promise<Buffer> {
  const url = pathToFileURL(path.join(motionDir, file)).href + '?reduce=1&t=60000';
  await page.goto(url);
  await page.waitForFunction(() => document.documentElement.classList.contains('frozen'));
  await page.addStyleTag({
    content: `.gal-head,figcaption,.opt:not(#${option}){display:none !important}
      body{margin:0}.gal-grid{--s:1;padding:0;margin:0;display:block;max-width:none}
      .opt-frame{border-radius:0;box-shadow:none}${hideQr}`,
  });
  await page.evaluate(() => document.fonts.ready);
  return page.screenshot({ clip });
}

async function shotApp(page: Page, s: Screen): Promise<Buffer> {
  const q = new URLSearchParams({ scenario: s.scenario });
  if (s.route) q.set('route', s.route);
  for (const [k, v] of Object.entries(s.extra ?? {})) q.set(k, v);
  await page.goto(`/?${q}`);
  await expect(page.getByTestId('card')).toBeVisible();
  await page.waitForFunction(() => !!window.__ucMock);
  if (s.route === 'show') await expect(page.getByTestId('code')).toBeVisible();
  if (s.act) await s.act(page);
  await page.addStyleTag({ content: hideQr });
  await page.evaluate(() => document.fonts.ready);
  await settle(page);
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
  return page.screenshot({ clip });
}

function compare(a: Buffer, b: Buffer) {
  const A = PNG.sync.read(a);
  const B = PNG.sync.read(b);
  const D = new PNG({ width: A.width, height: A.height });
  const bad = pixelmatch(A.data, B.data, D.data, A.width, A.height, { threshold: 0.1 });
  const out = new PNG({ width: A.width * 3 + 20, height: A.height });
  out.data.fill(255);
  for (const [i, img] of [A, B, D].entries()) PNG.bitblt(img, out, 0, 0, A.width, A.height, i * (A.width + 10), 0);
  return { percent: (bad / (A.width * A.height)) * 100, png: PNG.sync.write(out) };
}

test.describe.configure({ mode: 'serial' });
const results: { id: string; file: string; percent: number; ref: string }[] = [];

test.beforeAll(() => fs.mkdirSync(outDir, { recursive: true }));

for (const s of screens) {
  test(`${s.file} matches its reference`, async ({ page }) => {
    const ref = s.ref ?? { kind: 'design' };
    let reference: Buffer;
    if (ref.kind === 'mockup') reference = await shotMockup(page, ref.page, ref.option);
    else if (ref.kind === 'design') reference = await shotDesign(page, s.file);
    const app = await shotApp(page, s);
    if (ref.kind === 'baseline') {
      const f = path.join(baseDir, `${s.file}.png`);
      if (!fs.existsSync(f) || process.env.UPDATE_BASELINES === '1') {
        fs.mkdirSync(baseDir, { recursive: true });
        fs.writeFileSync(f, app);
      }
      reference = fs.readFileSync(f);
    }
    const r = compare(reference!, app);
    fs.writeFileSync(path.join(outDir, `${s.file}.png`), r.png);
    results.push({ id: s.id, file: s.file, percent: r.percent, ref: s.why ?? 'locked design' });
    expect(r.percent, `${s.id} differs from its reference by ${r.percent.toFixed(2)}%`).toBeLessThanOrEqual(s.maxDiff ?? MAX_DIFF_PERCENT);
  });
}

test.afterAll(() => {
  const rows = results
    .map((r) => `| ${r.id} | ${r.file} | ${r.ref} | ${r.percent.toFixed(2)}% | ![${r.id}](${r.file}.png) |`)
    .join('\n');
  fs.writeFileSync(
    path.join(outDir, 'README.md'),
    `# Implementation check (generated)

Made by \`apps/desktop/tests/e2e/visual.spec.ts\` (\`npm run e2e\`). Each picture shows three
parts, from left to right: the reference, the app after its motion has settled, and the
difference in red. The reference is the locked design, except Home (L07), which is the end
frame of the H3 "You are here" motion mockup (picked 2026-11-14), and Paused (L10) and the update
screens (U1, U3), which are pictures of the app (\`tests/e2e/baselines\`) because no design shows
them in the H3 layout.
The QR code is hidden in both (the design has a fixed picture, the app draws the real code).
Limit: ${MAX_DIFF_PERCENT}% of pixels.

| Screen | File | Reference | Different pixels | Picture |
|---|---|---|---|---|
${rows}
`,
  );
});
