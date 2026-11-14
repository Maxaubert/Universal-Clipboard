// Update (Max's pick 2026-11-14, docs/design/desktop/motion/update.html): U1 "One shape" when
// the agent opens the window for the update (route update), U3 "Home card" when Home is open.
// Each flow runs with full motion and with reduced motion (the OS setting, emulated).
import { expect, test, type Page } from '@playwright/test';

async function go(page: Page, q: string) {
  await page.goto(`/?${q}`);
  await expect(page.getByTestId('card')).toBeVisible();
  await page.waitForFunction(() => !!window.__ucMock);
}
const calls = (page: Page) => page.evaluate(() => window.__ucMock!.calls.map((c) => c.call));
const settled = (page: Page) =>
  page.waitForFunction(() => document.getAnimations().every((a) => a.playState !== 'running' && !a.pending), undefined, { timeout: 10_000 });
const heading = (page: Page, name: string) => page.getByRole('heading', { name });

/** Samples, every frame while `run` goes: the S1 shape's highest opacity and the largest
 * sideways move of the shaking part (E1). */
async function watch(page: Page, run: () => Promise<void>, ms = 1600): Promise<{ blob: number; shake: number }> {
  await page.evaluate(() => {
    const w = window as unknown as { __w: { blob: number; shake: number; stop: boolean } };
    w.__w = { blob: 0, shake: 0, stop: false };
    const f = () => {
      const b = document.querySelector('.blob');
      if (b) w.__w.blob = Math.max(w.__w.blob, Number(getComputedStyle(b).opacity));
      for (const el of Array.from(document.querySelectorAll('.shk, [data-testid="update-card"]'))) {
        const m = new DOMMatrix(getComputedStyle(el).transform);
        w.__w.shake = Math.max(w.__w.shake, Math.abs(m.m41));
      }
      if (!w.__w.stop) requestAnimationFrame(f);
    };
    requestAnimationFrame(f);
  });
  await run();
  await page.waitForTimeout(ms);
  return page.evaluate(() => {
    const w = window as unknown as { __w: { blob: number; shake: number; stop: boolean } };
    w.__w.stop = true;
    return { blob: w.__w.blob, shake: w.__w.shake };
  });
}

for (const mode of ['full', 'reduced'] as const) {
  test.describe(`update ${mode} motion`, () => {
    test.beforeEach(async ({ page }) => {
      await page.emulateMedia({ reducedMotion: mode === 'reduced' ? 'reduce' : 'no-preference' });
    });

    test('U1: the agent opens the window; Update now grows into the dial; after the restart the check', async ({ page }) => {
      await go(page, 'scenario=update&launch=update');
      await expect(heading(page, 'Version 0.1.1 is ready.')).toBeVisible();
      await expect(page.getByText('You have 0.1.0.', { exact: true })).toBeVisible();
      await settled(page);
      const w = await watch(page, async () => {
        await page.getByRole('button', { name: 'Update now' }).click();
        await expect(heading(page, 'Updating...')).toBeFocused();
      });
      // S1: the black button grows into the dial; reduced motion only fades.
      if (mode === 'full') expect(w.blob).toBeGreaterThan(0.9);
      else expect(w.blob).toBe(0);
      expect(await calls(page)).toContain('update.apply');
      await expect(page.locator('.core')).toBeVisible();
      await expect(page.locator('[role="status"]', { hasText: 'Updating...' })).toHaveCount(1);
      // The helper stops the agent: the screen stays.
      await page.evaluate(() => {
        window.__ucMock!.world.down = true;
        window.__ucMock!.emit('status');
      });
      await page.waitForTimeout(100);
      await expect(page.getByTestId('update')).toBeVisible();
      // The app opens again on the new version.
      await go(page, 'scenario=updated&launch=update');
      await expect(heading(page, 'Updated to 0.1.1')).toBeVisible();
      await settled(page);
      expect(await page.locator('.tick').evaluate((e) => getComputedStyle(e).strokeDashoffset)).toMatch(/^0(px)?$/);
      await page.getByRole('button', { name: 'Done' }).click();
      await expect(page.getByTestId('headline')).toHaveText('Working');
      await expect(page.getByTestId('update-card')).toHaveCount(0);
    });

    test('U1 failure: red circle, one shake (none when reduced), Try again and Later', async ({ page }) => {
      // The shake comes 1.3 s after the screen opens: watching starts right after the load.
      await go(page, 'scenario=update-failed&launch=update');
      const w = await watch(page, async () => {
        await expect(heading(page, 'The update did not work.')).toBeVisible();
      }, 2200);
      await expect(page.getByText('You still have 0.1.0.', { exact: true })).toBeVisible();
      if (mode === 'full') {
        expect(w.shake).toBeGreaterThan(4);
        expect(w.shake).toBeLessThanOrEqual(7.01);
      } else expect(w.shake).toBe(0);
      await page.getByRole('button', { name: 'Try again' }).click();
      await expect(heading(page, 'Updating...')).toBeFocused();
      // This time the helper cannot start: the failure comes back in place.
      await go(page, 'scenario=update&launch=update');
      await page.evaluate(() => (window.__ucMock!.updateApplyResult = 'fail'));
      await page.getByRole('button', { name: 'Update now' }).click();
      await expect(heading(page, 'The update did not work.')).toBeFocused();
      await page.getByRole('button', { name: 'Later' }).click();
      await expect(page.getByTestId('headline')).toHaveText('Working');
      expect(await calls(page)).toContain('update.later');
    });

    test('U3: Home gets the card; Update now continues with the dial', async ({ page }) => {
      await go(page, 'scenario=h3');
      await expect(page.getByTestId('headline')).toHaveText('Working');
      await settled(page);
      const tiles = page.locator('[data-m="tiles"]');
      const y0 = (await tiles.boundingBox())!.y;
      await page.evaluate(() => {
        window.__ucMock!.setUpdate({ state: 'available', available: '0.1.1' });
        window.__ucMock!.emit('open', { route: 'update' });
      });
      const card = page.getByTestId('update-card');
      await expect(card).toContainText('Version 0.1.1 is ready.');
      await settled(page);
      // The card pushes the tiles down; Home stays.
      expect((await tiles.boundingBox())!.y).toBeGreaterThan(y0 + 100);
      await expect(page.getByTestId('update')).toHaveCount(0);
      const w = await watch(page, async () => {
        await card.getByRole('button', { name: 'Update now' }).click();
        await expect(heading(page, 'Updating...')).toBeFocused();
      });
      if (mode === 'full') expect(w.blob).toBeGreaterThan(0.9);
      else expect(w.blob).toBe(0);
      await expect.poll(() => calls(page)).toContain('update.apply');
      await settled(page);
      await expect(page.locator('.layer')).toHaveCount(1);
    });

    test('U3 failure: back on Home the card shows it and shakes once; Later closes it', async ({ page }) => {
      await go(page, 'scenario=update');
      await page.evaluate(() => (window.__ucMock!.updateApplyResult = 'fail'));
      const card = page.getByTestId('update-card');
      await expect(card).toBeVisible();
      await settled(page);
      const w = await watch(page, async () => {
        await card.getByRole('button', { name: 'Update now' }).click();
        await expect(card).toContainText('The update did not work.');
      }, 2200);
      await expect(card).toContainText('You still have 0.1.0.');
      await expect(page.getByTestId('update')).toHaveCount(0);
      if (mode === 'full') expect(w.shake).toBeGreaterThan(4);
      else expect(w.shake).toBe(0);
      await card.getByRole('button', { name: 'Later' }).click();
      await expect(card).toHaveCount(0);
      expect(await calls(page)).toContain('update.later');
      await page.evaluate(() => window.__ucMock!.setUpdate({}));
      await page.waitForTimeout(100);
      await expect(page.getByTestId('update-card')).toHaveCount(0);
    });

    test('Later on the U1 screen goes home; the update does not come back in this run', async ({ page }) => {
      await go(page, 'scenario=update&launch=update');
      await page.getByRole('button', { name: 'Later' }).click();
      await expect(page.getByTestId('headline')).toHaveText('Working');
      await expect(page.getByTestId('update-card')).toHaveCount(0);
      expect(await calls(page)).toContain('update.later');
    });
  });
}

test('an update found during a pairing waits until the pairing screen is left', async ({ page }) => {
  await go(page, 'scenario=home&route=show');
  await expect(page.getByTestId('code')).toBeVisible();
  await page.evaluate(() => window.__ucMock!.setUpdate({ state: 'available', available: '0.1.1' }));
  await page.waitForTimeout(100);
  await expect(page.getByTestId('update')).toHaveCount(0);
  // Back leads to Add a device, not Home: there the news opens the U1 screen.
  await page.getByRole('button', { name: 'Back' }).click();
  await expect(heading(page, 'Version 0.1.1 is ready.')).toBeVisible();
});

test('from another screen the agent news opens the U1 screen', async ({ page }) => {
  await go(page, 'scenario=home&route=settings');
  await page.evaluate(() => window.__ucMock!.setUpdate({ state: 'available', available: '0.1.1' }));
  await expect(heading(page, 'Version 0.1.1 is ready.')).toBeVisible();
});
