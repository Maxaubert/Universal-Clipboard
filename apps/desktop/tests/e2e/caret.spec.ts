// The real text caret of the code field must sit in the box that takes the next digit.
// Screen magnifiers (Max uses Wind) follow the caret the browser reports to the OS, and the
// browser reports the caret it paints. So this test paints the invisible input's caret red
// (text and background stay transparent), takes a screenshot with the caret shown, and finds
// the red pixels. It checks 0, 3 and 6 digits, and after backspace and paste.
import { expect, test, type Page } from '@playwright/test';
import { PNG } from 'pngjs';

interface Box {
  x: number;
  y: number;
  width: number;
  height: number;
}

async function openEnter(page: Page) {
  await page.goto('/?scenario=fresh&route=enter');
  await expect(page.getByTestId('card')).toBeVisible();
  await page.waitForFunction(() => !!window.__ucMock);
  await page.evaluate(() => document.fonts.ready);
  await page.addStyleTag({
    content:
      'input[aria-label="Code"]{opacity:1 !important;color:transparent !important;caret-color:rgb(255,0,0) !important;background:transparent !important}',
  });
}

/** The painted caret, as a box in page pixels. Retries across the blink. */
async function caret(page: Page): Promise<Box> {
  const field = page.getByRole('textbox', { name: 'Code' });
  const r = (await field.boundingBox())!;
  const clip = { x: Math.floor(r.x) - 20, y: Math.floor(r.y) - 20, width: Math.ceil(r.width) + 40, height: Math.ceil(r.height) + 40 };
  for (let attempt = 0; attempt < 20; attempt++) {
    const png = PNG.sync.read(await page.screenshot({ clip, caret: 'initial' }));
    let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
    for (let y = 0; y < png.height; y++) {
      for (let x = 0; x < png.width; x++) {
        const i = (y * png.width + x) * 4;
        if (png.data[i] > 200 && png.data[i + 1] < 80 && png.data[i + 2] < 80) {
          x0 = Math.min(x0, x); y0 = Math.min(y0, y); x1 = Math.max(x1, x); y1 = Math.max(y1, y);
        }
      }
    }
    if (x0 !== Infinity) return { x: clip.x + x0, y: clip.y + y0, width: x1 - x0 + 1, height: y1 - y0 + 1 };
    await page.waitForTimeout(120);
  }
  throw new Error('no caret painted');
}

async function boxes(page: Page): Promise<Box[]> {
  const out: Box[] = [];
  for (const d of await page.getByTestId('digit').all()) out.push((await d.boundingBox())!);
  return out;
}

function inside(c: Box, b: Box, label: string) {
  const cx = c.x + c.width / 2;
  expect(cx, `${label}: caret x ${cx} in box ${b.x}..${b.x + b.width}`).toBeGreaterThan(b.x);
  expect(cx, `${label}: caret x ${cx} in box ${b.x}..${b.x + b.width}`).toBeLessThan(b.x + b.width);
  expect(c.y, `${label}: caret top ${c.y} below box top ${b.y}`).toBeGreaterThanOrEqual(b.y);
  expect(c.y + c.height, `${label}: caret bottom inside the box`).toBeLessThanOrEqual(b.y + b.height);
  expect(c.height, `${label}: caret is about digit height`).toBeGreaterThan(20);
}

const scrolled = (page: Page) => page.getByRole('textbox', { name: 'Code' }).evaluate((e) => (e as HTMLInputElement).scrollLeft);

test('the real caret sits in the box that takes the next digit (0, 3, 6 digits)', async ({ page }) => {
  await openEnter(page);
  const field = page.getByRole('textbox', { name: 'Code' });
  await expect(field).toBeFocused();
  const b = await boxes(page);
  expect(b).toHaveLength(6);

  const seen: string[] = [];
  const at0 = await caret(page);
  inside(at0, b[0], '0 digits');
  seen.push(`0 digits: caret x=${at0.x} (box 0 ${b[0].x}..${b[0].x + b[0].width})`);

  await field.pressSequentially('418');
  const at3 = await caret(page);
  inside(at3, b[3], '3 digits');
  seen.push(`3 digits: caret x=${at3.x} (box 3 ${b[3].x}..${b[3].x + b[3].width})`);

  await field.pressSequentially('207');
  await expect(page.getByRole('group')).toHaveAttribute('aria-label', 'Code, 6 of 6 digits entered');
  const at6 = await caret(page);
  inside(at6, b[5], '6 digits');
  seen.push(`6 digits: caret x=${at6.x} (box 5 ${b[5].x}..${b[5].x + b[5].width})`);
  expect(await scrolled(page)).toBe(0);

  await field.press('Backspace');
  await field.press('Backspace');
  await expect(field).toHaveValue('4182');
  inside(await caret(page), b[4], 'after backspace (4 digits)');
  expect(await scrolled(page)).toBe(0);
  console.log(seen.join('\n'));
});

test('paste of a whole code: numeric only, caret in the last box', async ({ page }) => {
  await openEnter(page);
  const field = page.getByRole('textbox', { name: 'Code' });
  await field.evaluate((e) => {
    const el = e as HTMLInputElement;
    el.value = '418 207';
    el.dispatchEvent(new Event('input', { bubbles: true }));
  });
  await expect(field).toHaveValue('418207');
  await expect(page.getByRole('group')).toHaveAttribute('aria-label', 'Code, 6 of 6 digits entered');
  await expect(page.getByRole('button', { name: 'Connect' })).toBeEnabled();
  const b = await boxes(page);
  inside(await caret(page), b[5], 'pasted');
  expect(await scrolled(page)).toBe(0);
});
