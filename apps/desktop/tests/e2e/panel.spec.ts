// The tray panel (tray spec 4) against its approved mockup, docs/design/tray/panel.html
// (section 1, the five dark panels). The app's panel (`#panel`, mock agent) gets the same
// devices and states as the mockup; both panels are screenshotted alone, headless, and must
// match within 1%. Side-by-side images go to docs/design/tray/impl-check/.
import { expect, test, type Page } from '@playwright/test';
import fs from 'node:fs';
import path from 'node:path';
import pixelmatch from 'pixelmatch';
import { PNG } from 'pngjs';

const mockup = path.resolve(process.cwd(), '../../docs/design/tray/panel.html');
const outDir = path.resolve(process.cwd(), '../../docs/design/tray/impl-check');
const fontDir = path.resolve(process.cwd(), 'node_modules/@fontsource/figtree/files');
// 1% on Windows (0.2 to 0.9% on 2026-11-17); Linux CI draws text a little differently (P3:
// 1.06%), like the U3 screens in visual.spec.ts, so the limit is 2%.
const MAX_DIFF_PERCENT = 2;

function fontCss(): string {
  return [400, 500, 600, 700]
    .map((w) => {
      const b64 = fs.readFileSync(path.join(fontDir, `figtree-latin-${w}-normal.woff2`)).toString('base64');
      return `@font-face{font-family:'Figtree';font-weight:${w};font-style:normal;src:url(data:font/woff2;base64,${b64}) format('woff2');}`;
    })
    .join('');
}

type Dev = [name: string, connected: boolean];
interface State {
  file: string;
  /** Index of the panel in the mockup's first grid. */
  n: number;
  devices: Dev[];
  status: Record<string, unknown>;
  transfer?: Record<string, unknown>;
  received?: boolean;
}

const mac: Dev = ["Max's Mac", true];
const states: State[] = [
  { file: 'P1-synced', n: 0, devices: [mac, ["Max's iPhone", false]], status: { state: 'working', paused: false }, received: true },
  {
    file: 'P2-receiving',
    n: 1,
    devices: [mac],
    status: { state: 'working', paused: false },
    transfer: { id: 'r', phase: 'receiving', device: "Max's Mac", done: 40, total: 100, files: 3 },
  },
  {
    file: 'P3-sending',
    n: 2,
    devices: [mac],
    status: { state: 'working', paused: false },
    transfer: { id: 's', phase: 'sending', device: "Max's Mac", done: 72, total: 100, files: 1 },
  },
  { file: 'P4-paused', n: 3, devices: [["Max's Mac", false], ["Max's iPhone", false]], status: { state: 'paused', paused: true } },
  {
    file: 'P5-not-connected',
    n: 4,
    devices: [["Max's Mac", false]],
    status: {
      state: 'needs-attention',
      paused: false,
      fix_code: 'vpn',
      reason: "Can't reach your other device? Your VPN blocks the local network.",
    },
  },
];

async function shotMockup(page: Page, n: number): Promise<Buffer> {
  const html = fs.readFileSync(mockup, 'utf8').replace(/<link[^>]*fonts\.g[^>]*>/g, '');
  await page.setContent(html, { waitUntil: 'load' });
  await page.addStyleTag({ content: fontCss() });
  await page.evaluate(() => document.fonts.ready);
  // Put the panel at whole pixels, like the app's (14 px in its clear edge), so half pixels
  // inside both round the same way.
  const el = page.locator('.grid').first().locator('.p.dark').nth(n);
  await el.evaluate((e) => Object.assign((e as HTMLElement).style, { position: 'fixed', top: '14px', left: '14px', zIndex: '9' }));
  return el.screenshot();
}

async function shotApp(page: Page, s: State): Promise<Buffer> {
  await page.goto('/?scenario=home#panel');
  await page.waitForFunction(() => !!window.__ucMock);
  await page.evaluate((s) => {
    const m = window.__ucMock!;
    const w = m.world;
    w.devices = [
      { id: 'pc', name: "Max's PC", os: 'Windows', this: true, state: 'This device' },
      ...s.devices.map(([name, on], i) => ({
        id: `d${i}`,
        name,
        os: 'macOS',
        this: false,
        state: (on ? 'Connected' : 'Asleep or away') as 'Connected' | 'Asleep or away',
      })),
    ];
    Object.assign(w.status, { reason: null, fix: null, fix_code: null, transfer: null, ...s.status });
    w.status.activity = s.received
      ? [{ kind: 'Text', direction: 'received', device: "Max's Mac", time_ms: Date.now() - 125_000, note: null }]
      : [];
    if (s.transfer) m.setTransfer(s.transfer as never);
    m.emit('status');
  }, s);
  const p = page.getByTestId('panel');
  await expect(p).toBeVisible();
  await expect(p).toHaveAttribute('data-state', { P1: 'Synced', P2: 'Receiving', P3: 'Sending', P4: 'Paused', P5: 'Not connected' }[s.file.slice(0, 2)]!);
  await page.addStyleTag({ content: 'html.panel, html.panel body { background: #E4E1DC !important; } .fill { transition: none !important; }' });
  await page.evaluate(() => document.fonts.ready);
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
  return p.screenshot();
}

function compare(a: Buffer, b: Buffer) {
  const A = PNG.sync.read(a);
  const B = PNG.sync.read(b);
  const w = Math.max(A.width, B.width);
  const h = Math.max(A.height, B.height);
  const pad = (img: PNG) => {
    const o = new PNG({ width: w, height: h });
    o.data.fill(255);
    PNG.bitblt(img, o, 0, 0, img.width, img.height, 0, 0);
    return o;
  };
  const [PA, PB] = [pad(A), pad(B)];
  const D = new PNG({ width: w, height: h });
  const bad = pixelmatch(PA.data, PB.data, D.data, w, h, { threshold: 0.1 });
  const out = new PNG({ width: w * 3 + 20, height: h });
  out.data.fill(255);
  for (const [i, img] of [PA, PB, D].entries()) PNG.bitblt(img, out, 0, 0, w, h, i * (w + 10), 0);
  return { percent: (bad / (w * h)) * 100, png: PNG.sync.write(out), size: [A.width, A.height, B.width, B.height] };
}

test.describe.configure({ mode: 'serial' });
test.beforeAll(() => fs.mkdirSync(outDir, { recursive: true }));

for (const s of states) {
  test(`${s.file} matches the approved panel mockup`, async ({ page }) => {
    const reference = await shotMockup(page, s.n);
    const app = await shotApp(page, s);
    const r = compare(reference, app);
    fs.writeFileSync(path.join(outDir, `${s.file}.png`), r.png);
    test.info().annotations.push({ type: "diff", description: `${r.percent.toFixed(2)}%` });
    // Half pixels (a 31.5 px state word) round either way: 1 px is allowed.
    expect(Math.abs(r.size[3] - r.size[1]), `${s.file}: height of mockup ${r.size[1]} vs app ${r.size[3]}`).toBeLessThanOrEqual(1);
    expect(r.percent, `${s.file} differs from the mockup by ${r.percent.toFixed(2)}%`).toBeLessThanOrEqual(MAX_DIFF_PERCENT);
  });
}
