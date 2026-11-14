import { expect, test, type Page } from '@playwright/test';

async function open(page: Page, scenario: string, route?: string) {
  const q = new URLSearchParams({ scenario });
  if (route) q.set('route', route);
  await page.goto(`/?${q}`);
  await expect(page.getByTestId('card')).toBeVisible();
  await page.waitForFunction(() => !!window.__ucMock);
}

const steps = (page: Page, n: number) => expect(page.getByRole('img', { name: `Step ${n} of 5` })).toBeVisible();
const mockCalls = (page: Page) => page.evaluate(() => window.__ucMock!.calls.map((c) => c.call));

test('whole setup flow: welcome, permissions, add, show a code, connected, home', async ({ page }) => {
  await open(page, 'fresh');
  await steps(page, 1);
  await expect(page.getByRole('heading', { name: 'Your latest copy, on every device.' })).toBeVisible();
  await page.getByRole('button', { name: 'Get started' }).click();

  await steps(page, 2);
  await expect(page.getByRole('heading', { name: 'Two permissions' })).toBeVisible();
  await expect(page.locator('[data-row="autostart"]').getByRole('img', { name: 'Allowed' })).toBeVisible();
  await page.getByRole('button', { name: 'Allow through the firewall' }).click();
  await expect(page.locator('[data-row="firewall"]').getByRole('img', { name: 'Allowed' })).toBeVisible();
  await page.getByRole('button', { name: 'Continue' }).click();

  await steps(page, 3);
  await expect(page.getByRole('heading', { name: 'Add a device' })).toBeVisible();
  await page.getByRole('button', { name: 'Show a code' }).click();

  await steps(page, 4);
  await expect(page.getByTestId('code')).toHaveText('418 207');
  await expect(page.getByRole('img', { name: 'QR code to scan with your other device' })).toBeVisible();
  await page.evaluate(() => {
    const m = window.__ucMock!;
    m.world.devices.push({ id: 'mac-mini', name: 'Mac mini', os: 'macOS', this: false, state: 'Connected' });
    m.emit('pair.added', { name: 'Mac mini' });
  });

  await steps(page, 5);
  await expect(page.getByRole('heading', { name: 'Mac mini is added' })).toBeVisible();
  await page.getByRole('button', { name: 'Go to my devices' }).click();
  await expect(page.getByTestId('headline')).toHaveText('Working');
  await expect(page.locator('[data-device="Mac mini"]')).toBeVisible();
  expect(await mockCalls(page)).toContain('pair.start');
});

test('enter a code: wrong code, then the right one', async ({ page }) => {
  await open(page, 'fresh', 'add');
  await page.getByRole('button', { name: 'Enter a code' }).click();
  await steps(page, 4);
  await expect(page.getByRole('button', { name: 'Connect' })).toBeDisabled();
  await page.getByRole('textbox', { name: 'Code' }).pressSequentially('418207');
  await expect(page.getByRole('group')).toHaveAttribute('aria-label', 'Code, 6 of 6 digits entered');
  await page.evaluate(() => {
    window.__ucMock!.joinResult = {
      ok: false,
      error: { code: 'wrong-code', message: 'That code is not right. Check it and try again.' },
    };
  });
  await page.getByRole('button', { name: 'Connect' }).click();
  await expect(page.getByRole('alert')).toHaveText('That code is not right. Check it and try again.');
  await page.evaluate(() => {
    window.__ucMock!.joinResult = { ok: true, name: 'Mac mini', os: 'macOS' };
  });
  await page.getByRole('button', { name: 'Connect' }).click();
  await expect(page.getByRole('heading', { name: 'Mac mini is added' })).toBeVisible();
  await steps(page, 5);
});

test('enter a code in a group asks before leaving it', async ({ page }) => {
  await open(page, 'home', 'enter');
  await page.getByRole('textbox', { name: 'Code' }).pressSequentially('418207');
  await page.getByRole('button', { name: 'Connect' }).click();
  await expect(page.getByRole('alertdialog')).toContainText('Leave your other devices and join these?');
  await page.getByRole('button', { name: 'Leave and join' }).click();
  await expect(page.getByRole('heading', { name: 'Mac mini is added' })).toBeVisible();
  expect(await page.evaluate(() => window.__ucMock!.calls.filter((c) => c.call === 'pair.join').map((c) => c.params.leave_other))).toEqual([false, true]);
});

test('show a code: lockout notice keeps the QR, someone tried the code', async ({ page }) => {
  await open(page, 'lockout', 'show');
  await expect(page.getByTestId('lockout')).toHaveText('Too many wrong codes. Try again in 5 minutes.');
  await expect(page.getByTestId('code')).toHaveCount(0);
  await expect(page.getByRole('img', { name: 'QR code to scan with your other device' })).toBeVisible();

  await open(page, 'fresh', 'show');
  await expect(page.getByTestId('code')).toBeVisible();
  await page.evaluate(() => window.__ucMock!.emit('pair.tried', { notice: 'Someone else tried this code', code_dead: false }));
  await expect(page.getByTestId('tried')).toHaveText('Someone else tried this code');

  // The 5th wrong code starts the lockout: the code goes, the time left shows, the QR stays.
  await page.evaluate(() =>
    window.__ucMock!.emit('pair.tried', {
      notice: 'Someone else tried this code',
      code_dead: true,
      lockout: 'Too many wrong codes. Try again in 1 minute.',
      lockout_ms: 60000,
    }),
  );
  await expect(page.getByTestId('code')).toHaveCount(0);
  await expect(page.getByTestId('lockout')).toHaveText('Too many wrong codes. Try again in 1 minute.');
  await expect(page.getByRole('img', { name: 'QR code to scan with your other device' })).toBeVisible();
});

test('show a code: expiry makes a new code, back cancels', async ({ page }) => {
  await open(page, 'fresh', 'show');
  await expect(page.getByTestId('code')).toBeVisible();
  await page.evaluate(() => window.__ucMock!.emit('pair.closed', { reason: 'expired' }));
  await expect.poll(async () => (await mockCalls(page)).filter((c) => c === 'pair.start').length).toBe(2);
  await page.getByRole('button', { name: 'Back' }).click();
  expect(await mockCalls(page)).toContain('pair.cancel');
});

test('home: device detail, rename, remove', async ({ page }) => {
  await open(page, 'home');
  await expect(page.getByTestId('subtitle')).toHaveText('5 devices connected');
  await page.locator('[data-device="Mac mini"]').click();
  await expect(page.getByTestId('device-name')).toHaveText('Mac mini');
  await expect(page.getByTestId('device-state')).toHaveText('Connected');

  await page.getByRole('button', { name: 'Rename' }).click();
  await page.getByLabel('Device name').fill('Studio Mac');
  await page.getByRole('button', { name: 'Save' }).click();
  await expect(page.getByTestId('device-name')).toHaveText('Studio Mac');

  await page.getByRole('button', { name: 'Remove this device' }).click();
  await expect(page.getByRole('alertdialog')).toHaveAttribute('aria-label', 'Remove Studio Mac?');
  await page.getByRole('button', { name: 'Remove', exact: true }).click();
  await expect(page.getByTestId('headline')).toHaveText('Working');
  await expect(page.locator('[data-device="Studio Mac"]')).toHaveCount(0);
  expect(await mockCalls(page)).toEqual(expect.arrayContaining(['devices.rename', 'devices.remove']));
});

test('device detail: this device can be renamed but not removed', async ({ page }) => {
  await open(page, 'home');
  await page.locator('[data-device="Max\'s PC"]').click();
  await expect(page.getByRole('button', { name: 'Rename' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Remove this device' })).toHaveCount(0);
});

test('rename with an empty name shows the agent text', async ({ page }) => {
  await open(page, 'home', 'device');
  await page.getByRole('button', { name: 'Rename' }).click();
  await page.getByLabel('Device name').fill('   ');
  await page.getByRole('button', { name: 'Save' }).click();
  await expect(page.getByRole('alert')).toHaveText('A name has 1 to 64 characters.');
});

test('settings: start at sign-in, and remove this device forgets everything', async ({ page }) => {
  await open(page, 'home');
  await page.getByRole('button', { name: 'Settings' }).click();
  await expect(page.getByRole('heading', { name: 'Settings' })).toBeVisible();
  const sw = page.getByRole('switch', { name: 'Start when you sign in' });
  await expect(sw).toHaveAttribute('aria-checked', 'true');
  await sw.click();
  await expect(sw).toHaveAttribute('aria-checked', 'false');
  await expect(page.getByRole('switch', { name: /always on/ })).toBeDisabled();
  await expect(page.getByText('Version 1.0.0')).toBeVisible();

  await page.getByRole('button', { name: 'Remove', exact: true }).click();
  await expect(page.getByRole('alertdialog')).toContainText('Remove this device and forget everything?');
  await page.getByRole('button', { name: 'Remove and forget' }).click();
  await steps(page, 1);
  expect(await mockCalls(page)).toContain('leave');
});

test('paused: dark Resume button, greyed tiles, Resume works', async ({ page }) => {
  await open(page, 'paused');
  await expect(page.getByTestId('headline')).toHaveText('Paused');
  await expect(page.getByTestId('subtitle')).toHaveText('Copies stay on this PC');
  const resume = page.getByRole('button', { name: 'Resume' });
  await expect(resume).toHaveAttribute('aria-pressed', 'true');
  await expect(resume).toHaveCSS('background-color', 'rgb(29, 28, 26)');
  await expect(page.locator('[data-device="Mac mini"]')).toContainText('Paused');
  await resume.click();
  await expect(page.getByTestId('headline')).toHaveText('Working');
});

test('needs attention: firewall Fix, VPN Fix opens the VPN help', async ({ page }) => {
  await open(page, 'attention');
  await expect(page.getByRole('heading', { name: 'Needs attention' })).toBeVisible();
  await expect(page.getByTestId('reason')).toHaveText('Firewall is blocking it.');
  await page.getByRole('button', { name: 'Fix' }).click();
  await expect(page.getByTestId('headline')).toHaveText('Working');

  await open(page, 'vpn');
  await page.getByRole('button', { name: 'Fix' }).click();
  await expect(page.getByRole('heading', { name: "Can't reach your other device?" })).toBeVisible();
  await expect(page.getByText('Turn on Local network sharing in your VPN')).toBeVisible();
  await page.getByRole('button', { name: 'Back', exact: true }).last().click();
  await expect(page.getByRole('heading', { name: 'Needs attention' })).toBeVisible();
});

test('VPN help from "On another network?" and back', async ({ page }) => {
  await open(page, 'fresh', 'add');
  await page.getByRole('button', { name: 'Show a code' }).click();
  await page.getByRole('button', { name: 'On another network?' }).click();
  await expect(page.getByRole('heading', { name: "Can't reach your other device?" })).toBeVisible();
  await page.getByRole('button', { name: 'Back' }).first().click();
  await expect(page.getByRole('heading', { name: 'Pair a device' })).toBeVisible();
});

test('the agent is down: plain message and retry', async ({ page }) => {
  await open(page, 'down');
  await expect(page.getByRole('heading', { name: 'The background service is not running.' })).toBeVisible();
  await page.evaluate(() => {
    window.__ucMock!.world.down = false;
  });
  await page.getByRole('button', { name: 'Try again' }).click();
  await expect(page.getByTestId('headline')).toHaveText('Working');
});

test('title bar: custom controls and a drag region, no native decorations', async ({ page }) => {
  await open(page, 'home');
  await expect(page.locator('[data-tauri-drag-region]')).toHaveCount(1);
  await page.getByRole('button', { name: 'Minimize' }).click();
  await page.getByRole('button', { name: 'Maximize' }).click();
  await page.getByRole('button', { name: 'Close' }).click();
  expect(await page.evaluate(() => window.__ucMock!.windowLog)).toEqual(['minimize', 'maximize', 'close']);
});

test('no send, get or fetch anywhere in the window', async ({ page }) => {
  for (const [scenario, route] of [['home', ''], ['fresh', 'add'], ['home', 'settings'], ['home', 'device'], ['paused', '']]) {
    await open(page, scenario, route || undefined);
    const text = (await page.getByTestId('card').innerText()).toLowerCase();
    expect(text).not.toMatch(/\b(send|get|fetch)\b/);
  }
});
