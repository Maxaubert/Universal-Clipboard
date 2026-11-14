// Visual check against the locked design (docs/design/desktop/L01..L12). Each screen of the
// app (Vite build, mock agent) is screenshotted headless and compared with the design file
// rendered in the same browser. Side-by-side images (design | app | difference) go to
// docs/design/desktop/impl-check/. The QR code is hidden in both pictures: the design has a
// fixed picture and the app draws the real one.
import { expect, test, type Page } from '@playwright/test';
import fs from 'node:fs';
import path from 'node:path';
import pixelmatch from 'pixelmatch';
import { PNG } from 'pngjs';

const designDir = path.resolve(process.cwd(), '../../docs/design/desktop');
const outDir = path.join(designDir, 'impl-check');
const fontDir = path.resolve(process.cwd(), 'node_modules/@fontsource/figtree/files');
const MAX_DIFF_PERCENT = 1;

interface Screen {
  id: string;
  file: string;
  scenario: string;
  route?: string;
  act?: (page: Page) => Promise<void>;
}

const screens: Screen[] = [
  { id: 'L01', file: 'L01-welcome', scenario: 'fresh', route: 'welcome' },
  { id: 'L02', file: 'L02-permissions', scenario: 'fresh', route: 'perms' },
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
  { id: 'L07', file: 'L07-home', scenario: 'home' },
  { id: 'L08', file: 'L08-device', scenario: 'home', route: 'device' },
  { id: 'L09', file: 'L09-settings', scenario: 'home', route: 'settings' },
  { id: 'L10', file: 'L10-paused', scenario: 'paused' },
  { id: 'L11', file: 'L11-attention', scenario: 'attention' },
  { id: 'L12', file: 'L12-other-network', scenario: 'home', route: 'vpn' },
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

async function shotDesign(page: Page, file: string): Promise<Buffer> {
  let html = fs.readFileSync(path.join(designDir, `${file}.dc.html`), 'utf8');
  html = html.replace(/<link[^>]*fonts\.googleapis[^>]*>/g, '').replace(/<script[^>]*support\.js[^>]*><\/script>/g, '');
  await page.setContent(html, { waitUntil: 'load' });
  await page.addStyleTag({ content: fontCss() + hideQr });
  await page.evaluate(() => document.fonts.ready);
  return page.screenshot({ clip: { x: 0, y: 0, width: 540, height: 720 } });
}

async function shotApp(page: Page, s: Screen): Promise<Buffer> {
  const q = new URLSearchParams({ scenario: s.scenario });
  if (s.route) q.set('route', s.route);
  await page.goto(`/?${q}`);
  await expect(page.getByTestId('card')).toBeVisible();
  await page.waitForFunction(() => !!window.__ucMock);
  if (s.route === 'show') await expect(page.getByTestId('code')).toBeVisible();
  if (s.act) await s.act(page);
  await page.addStyleTag({ content: hideQr });
  await page.evaluate(() => document.fonts.ready);
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
  return page.screenshot({ clip: { x: 0, y: 0, width: 540, height: 720 } });
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
const results: { id: string; file: string; percent: number }[] = [];

test.beforeAll(() => fs.mkdirSync(outDir, { recursive: true }));

for (const s of screens) {
  test(`${s.id} ${s.file} matches the locked design`, async ({ page }) => {
    const design = await shotDesign(page, s.file);
    const app = await shotApp(page, s);
    const r = compare(design, app);
    fs.writeFileSync(path.join(outDir, `${s.file}.png`), r.png);
    results.push({ id: s.id, file: s.file, percent: r.percent });
    expect(r.percent, `${s.id} differs from the design by ${r.percent.toFixed(2)}%`).toBeLessThanOrEqual(MAX_DIFF_PERCENT);
  });
}

test.afterAll(() => {
  const rows = results
    .map((r) => `| ${r.id} | ${r.file} | ${r.percent.toFixed(2)}% | ![${r.id}](${r.file}.png) |`)
    .join('\n');
  fs.writeFileSync(
    path.join(outDir, 'README.md'),
    `# Implementation check (generated)

Made by \`apps/desktop/tests/e2e/visual.spec.ts\` (\`npm run e2e\`). Each picture shows three
parts, from left to right: the locked design, the app, and the difference in red. The QR code
is hidden in both (the design has a fixed picture, the app draws the real code). Limit: ${MAX_DIFF_PERCENT}% of pixels.

| Screen | File | Different pixels | Picture |
|---|---|---|---|
${rows}
`,
  );
});
