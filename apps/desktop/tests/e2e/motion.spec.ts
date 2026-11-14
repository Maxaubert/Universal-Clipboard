// The flows still work end to end with the motion of 2026-11-14 (S1, E1, H3, A3, A4), once
// with full motion and once with reduced motion (the OS setting, emulated).
import { expect, test, type Page } from '@playwright/test';

async function open(page: Page, scenario: string, route?: string) {
  const q = new URLSearchParams({ scenario });
  if (route) q.set('route', route);
  await page.goto(`/?${q}`);
  await expect(page.getByTestId('card')).toBeVisible();
  await page.waitForFunction(() => !!window.__ucMock);
}
const steps = (page: Page, n: number) => expect(page.getByRole('img', { name: `Step ${n} of 4` })).toBeVisible();
/** Highest opacity the S1 shape reaches while `run` goes (sampled every frame). */
async function watchBlob(page: Page, run: () => Promise<void>): Promise<number> {
  await page.evaluate(() => {
    const w = window as unknown as { __blobMax: number; __blobStop: boolean };
    w.__blobMax = 0;
    w.__blobStop = false;
    const f = () => {
      const b = document.querySelector('.blob');
      if (b) w.__blobMax = Math.max(w.__blobMax, Number(getComputedStyle(b).opacity));
      if (!w.__blobStop) requestAnimationFrame(f);
    };
    requestAnimationFrame(f);
  });
  await run();
  await page.waitForTimeout(1200);
  return page.evaluate(() => {
    const w = window as unknown as { __blobMax: number; __blobStop: boolean };
    w.__blobStop = true;
    return w.__blobMax;
  });
}
const settled = (page: Page) =>
  page.waitForFunction(() => document.getAnimations().every((a) => a.playState !== 'running' && !a.pending), undefined, { timeout: 10_000 });

for (const mode of ['full', 'reduced'] as const) {
  test.describe(`motion ${mode}`, () => {
    test.beforeEach(async ({ page }) => {
      await page.emulateMedia({ reducedMotion: mode === 'reduced' ? 'reduce' : 'no-preference' });
    });

    test('setup flow and pairing success (S1), focus on each new heading', async ({ page }) => {
      await open(page, 'fresh');
      await steps(page, 1);
      const shape = await watchBlob(page, async () => {
        await page.getByRole('button', { name: 'Get started' }).click();
        await expect(page.getByRole('heading', { name: 'Add a device' })).toBeFocused();
      });
      // S1: the black shape carries the button to the next one; reduced motion only fades.
      if (mode === 'full') expect(shape).toBeGreaterThan(0.9);
      else expect(shape).toBe(0);
      await steps(page, 2);
      await page.getByRole('button', { name: 'Show a code' }).click();
      await expect(page.getByRole('heading', { name: 'Pair a device' })).toBeFocused();
      await expect(page.getByTestId('code')).toHaveText('418 207');
      await steps(page, 3);
      const node = await watchBlob(page, async () => {
        await page.evaluate(() => {
          const w = window.__ucMock!;
          w.world.devices.push({ id: 'mac-mini', name: 'Mac mini', os: 'macOS', this: false, state: 'Connected' });
          w.emit('pair.added', { name: 'Mac mini' });
        });
        await expect(page.getByRole('heading', { name: 'Mac mini is added' })).toBeFocused();
      });
      if (mode === 'full') expect(node).toBeGreaterThan(0.9);
      else expect(node).toBe(0);
      await steps(page, 4);
      await settled(page);
      // Only one screen is left, and the check is drawn.
      await expect(page.locator('.layer')).toHaveCount(1);
      expect(await page.locator('.l-check').evaluate((e) => getComputedStyle(e).opacity)).toBe('1');
      await page.getByRole('button', { name: 'Go to my devices' }).click();
      await expect(page.getByTestId('headline')).toHaveText('Working');
      await expect(page.locator('.tile[data-device="Mac mini"]')).toBeVisible();
    });

    test('enter a code: pairing success from the Connect button', async ({ page }) => {
      await open(page, 'fresh', 'enter');
      await page.getByRole('textbox', { name: 'Code' }).pressSequentially('418207');
      await page.getByRole('button', { name: 'Connect' }).click();
      await expect(page.getByRole('heading', { name: 'Mac mini is added' })).toBeFocused();
      await steps(page, 4);
    });

    test('wrong code: shake, red outlines, boxes clear, caret back in the first box (E1)', async ({ page }) => {
      await open(page, 'fresh', 'enter');
      const input = page.getByRole('textbox', { name: 'Code' });
      await input.pressSequentially('418207');
      await page.evaluate(() => {
        window.__ucMock!.joinResult = { ok: false, error: { code: 'wrong-code', message: 'That code is not right. Check it and try again.' } };
      });
      await page.getByRole('button', { name: 'Connect' }).click();
      await expect(page.getByRole('alert')).toHaveText('That code is not right. Check it and try again.');
      const row = page.getByRole('group');
      // The shake runs only with full motion; the red outline says it in both.
      const shaking = await row.evaluate((e) => e.getAnimations().length);
      if (mode === 'full') expect(shaking).toBeGreaterThan(0);
      else expect(shaking).toBe(0);
      await expect(page.getByTestId('digit').first()).toHaveCSS('border-top-color', 'rgb(180, 35, 24)');
      await expect(row).toHaveAttribute('aria-label', 'Code, 0 of 6 digits entered');
      await expect(input).toBeFocused();
      await expect(page.getByRole('button', { name: 'Connect' })).toBeDisabled();
      await page.evaluate(() => {
        window.__ucMock!.joinResult = { ok: true, name: 'Mac mini', os: 'macOS' };
      });
      await input.pressSequentially('418207');
      await expect(page.getByRole('alert')).toHaveCount(0);
      await page.getByRole('button', { name: 'Connect' }).click();
      await expect(page.getByRole('heading', { name: 'Mac mini is added' })).toBeVisible();
    });

    test('dead code: boxes go grey, typing starts a new code (E1)', async ({ page }) => {
      await open(page, 'fresh', 'enter');
      const input = page.getByRole('textbox', { name: 'Code' });
      await input.pressSequentially('418207');
      await page.evaluate(() => {
        window.__ucMock!.joinResult = { ok: false, error: { code: 'code-dead', message: 'That code no longer works. Make a new code on the other device.' } };
      });
      await page.getByRole('button', { name: 'Connect' }).click();
      await expect(page.getByRole('alert')).toContainText('That code no longer works.');
      await expect(page.getByRole('button', { name: 'Connect' })).toBeDisabled();
      await expect(page.getByTestId('digit').last()).toHaveCSS('color', 'rgb(110, 106, 100)');
      await input.press('5');
      await expect(page.getByRole('group')).toHaveAttribute('aria-label', 'Code, 1 of 6 digits entered');
    });

    test('pause and resume: wave across the tiles (A4), headline rolls', async ({ page }) => {
      await open(page, 'home');
      await settled(page);
      await page.getByRole('button', { name: 'Pause' }).click();
      await expect(page.getByTestId('headline')).toHaveText('Paused');
      await expect(page.getByTestId('subtitle')).toHaveText('Copies stay on this PC');
      await expect(page.locator('.tile[data-device="Mac mini"]')).toContainText('Paused');
      // The farthest tile changes later than the nearest one (full motion only).
      const delays = await page.locator('.tile').evaluateAll((ts) => ts.map((t) => parseFloat(getComputedStyle(t).getPropertyValue('--d'))));
      if (mode === 'full') expect(Math.max(...delays)).toBeGreaterThan(Math.min(...delays));
      else expect(new Set(delays).size).toBe(1);
      await settled(page);
      await expect(page.locator('.tile[data-device="Mac mini"] svg')).toHaveCSS('fill', 'rgb(179, 174, 167)');
      await page.getByRole('button', { name: 'Resume' }).click();
      await expect(page.getByTestId('headline')).toHaveText('Working');
      await expect(page.getByTestId('subtitle')).toHaveText('5 devices connected');
      await settled(page);
      await expect(page.locator('.tile[data-device="Mac mini"] svg')).toHaveCSS('fill', 'rgb(29, 28, 26)');
    });

    test('a copy rolls through the line and the bar (A3, H3), facts only', async ({ page }) => {
      await open(page, 'home');
      await settled(page);
      await page.evaluate(() => window.__ucMock!.copy('received', 'Image', 'iPhone'));
      await expect(page.getByTestId('subtitle')).toHaveText('Image, from iPhone, just now');
      await expect(page.getByTestId('last-copy')).toHaveText('Image, from iPhone, just now');
      await expect(page.getByTestId('subtitle')).toHaveText('5 devices connected', { timeout: 6000 });
      await page.evaluate(() => window.__ucMock!.copy('sent', 'Text'));
      await expect(page.getByTestId('subtitle')).toHaveText('Text, sent, just now');
      await expect(page.getByRole('button', { name: /Max's PC/ })).toContainText('Text, sent, just now');
    });
  });
}
