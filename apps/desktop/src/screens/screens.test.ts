import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import App from '../App.svelte';
import Qr from '../components/Qr.svelte';
import { setApi } from '../lib/ipc';
import { createMockApi, type Scenario } from '../lib/mock';
import { store, type Route } from '../lib/store.svelte';

async function open(scenario: Scenario, route?: Route) {
  const api = createMockApi(scenario);
  setApi(api);
  render(App);
  await waitFor(() => expect(store.loaded || store.down).toBeTruthy());
  if (route) store.route = route;
  return window.__ucMock!;
}

describe('Home', () => {
  it('shows every device, the count and a dashed add tile', async () => {
    await open('home');
    expect(await screen.findByTestId('headline')).toHaveTextContent('Working');
    expect(screen.getByTestId('subtitle')).toHaveTextContent('5 devices connected');
    for (const n of ["Max's PC", 'Mac mini', 'Ubuntu laptop', 'iPhone', 'iPad', 'Pixel 8']) {
      expect(document.querySelector(`[data-device="${n}"]`)).not.toBeNull();
    }
    expect(screen.getByTestId('add-tile')).toBeTruthy();
  });
  it('pauses and resumes through pause.set', async () => {
    const m = await open('home');
    await fireEvent.click(await screen.findByRole('button', { name: 'Pause' }));
    await waitFor(() => expect(screen.getByTestId('headline')).toHaveTextContent('Paused'));
    expect(m.calls.some((c) => c.call === 'pause.set' && c.params.mode === 'forever')).toBe(true);
    const resume = screen.getByRole('button', { name: 'Resume' });
    expect(resume.getAttribute('aria-pressed')).toBe('true');
    await fireEvent.click(resume);
    await waitFor(() => expect(screen.getByTestId('headline')).toHaveTextContent('Working'));
    expect(m.calls.some((c) => c.call === 'pause.set' && c.params.mode === 'off')).toBe(true);
  });
  it('greys the tiles when paused', async () => {
    await open('paused');
    expect(await screen.findByTestId('subtitle')).toHaveTextContent('Copies stay on this PC');
    // H3: the five other devices are tiles; this PC is the dark bar and keeps its white logo.
    expect(document.querySelectorAll('[data-device] svg[fill="#B3AEA7"]').length).toBe(5);
  });
  it('docks this device as a bar with the last copy as facts (H3)', async () => {
    const m = await open('home');
    const bar = await screen.findByRole('button', { name: /Max's PC/ });
    expect(bar).toHaveTextContent('This PC');
    expect(screen.getByTestId('last-copy')).toHaveTextContent('Text, from Mac mini, just now');
    expect(document.querySelectorAll('.tile[data-device]').length).toBe(5);
    m.copy('sent', 'Image');
    await waitFor(() => expect(screen.getByTestId('last-copy')).toHaveTextContent('Image, sent, just now'));
    // A3: the line under the headline shows the copy for a moment.
    await waitFor(() => expect(screen.getByTestId('subtitle')).toHaveTextContent('Image, sent, just now'));
  });
});

describe('Needs attention', () => {
  it('shows the reason and Fix adds the firewall rule like "uc firewall add"', async () => {
    const m = await open('attention');
    expect(await screen.findByTestId('reason')).toHaveTextContent('Firewall is blocking it.');
    await fireEvent.click(screen.getByRole('button', { name: 'Fix' }));
    await waitFor(() => expect(screen.getByTestId('headline')).toHaveTextContent('Working'));
    expect(m.calls.map((c) => c.call)).toEqual(expect.arrayContaining(['firewall.add', 'firewall.check']));
    expect(m.calls.some((c) => c.call === 'setup.step')).toBe(false);
  });
  it('shows the answer when the user says no in the UAC prompt', async () => {
    const m = await open('attention');
    m.firewallAddResult = 3;
    await fireEvent.click(await screen.findByRole('button', { name: 'Fix' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('You said no.');
  });
  it('sends a VPN problem to the VPN help screen', async () => {
    await open('vpn');
    await fireEvent.click(await screen.findByRole('button', { name: 'Fix' }));
    expect(await screen.findByText("Can't reach your other device?")).toBeTruthy();
    expect(screen.getByText('Turn on Local network sharing in your VPN')).toBeTruthy();
  });
});

describe('Setup', () => {
  it('skips the permissions screen when there are no rows (every OS)', async () => {
    const m = await open('fresh');
    await fireEvent.click(await screen.findByRole('button', { name: 'Get started' }));
    expect(await screen.findByRole('heading', { name: 'Add a device' })).toBeTruthy();
    expect(screen.getByRole('img', { name: 'Step 2 of 4' })).toBeTruthy();
    expect(m.calls.some((c) => c.call === 'setup.step' && c.params.row === 'done')).toBe(true);
    expect(m.world.setup.done).toBe(true);
  });
  it('shows the macOS rows only when macOS blocks something, as part of step 1', async () => {
    await open('mac-blocked');
    await fireEvent.click(await screen.findByRole('button', { name: 'Get started' }));
    expect(await screen.findByRole('heading', { name: 'Two permissions' })).toBeTruthy();
    expect(screen.getByRole('img', { name: 'Step 1 of 4' })).toBeTruthy();
    expect(document.querySelector('[data-row="local-network"]')).not.toBeNull();
    expect(document.querySelector('[data-row="paste"]')).not.toBeNull();
    expect(document.querySelector('[data-row="firewall"]')).toBeNull();
    expect(document.querySelector('[data-row="autostart"]')).toBeNull();
  });
});

describe('Enter a code', () => {
  it('turns on Connect at 6 digits and shows the agent error for a wrong code', async () => {
    const m = await open('fresh', 'enter');
    const connect = await screen.findByRole('button', { name: 'Connect' });
    expect(connect).toBeDisabled();
    const input = screen.getByLabelText('Code') as HTMLInputElement;
    await fireEvent.input(input, { target: { value: '418 207' } });
    expect(connect).toBeEnabled();
    expect(screen.getByRole('group').getAttribute('aria-label')).toBe('Code, 6 of 6 digits entered');
    m.joinResult = { ok: false, error: { code: 'wrong-code', message: 'That code is not right. Check it and try again.' } };
    await fireEvent.click(connect);
    expect(await screen.findByRole('alert')).toHaveTextContent('That code is not right. Check it and try again.');
  });
});

describe('Settings', () => {
  it('toggles start at sign-in and keeps Skip passwords locked on', async () => {
    const m = await open('home', 'settings');
    const sw = await screen.findByRole('switch', { name: 'Start when you sign in' });
    expect(sw.getAttribute('aria-checked')).toBe('true');
    await fireEvent.click(sw);
    await waitFor(() => expect(sw.getAttribute('aria-checked')).toBe('false'));
    expect(m.calls.some((c) => c.call === 'settings.set' && c.params.key === 'start-at-login' && c.params.value === 'off')).toBe(true);
    const locked = screen.getByRole('switch', { name: /always on/ });
    expect(locked).toBeDisabled();
  });
});

describe('Qr', () => {
  it('draws a path for the agent text', () => {
    render(Qr, { text: 'UCLIP1:abc' });
    const p = document.querySelector('svg path');
    expect(p?.getAttribute('d')?.length).toBeGreaterThan(100);
  });
});

describe('Update (the agent checks once at start, asks before installing)', () => {
  async function launched(scenario: Scenario) {
    const api = createMockApi(scenario);
    window.__ucMock!.launchRoute = 'update';
    setApi(api);
    render(App);
    await waitFor(() => expect(store.loaded).toBeTruthy());
    return window.__ucMock!;
  }
  it('opens on "Version X is ready." when the agent starts the window for it', async () => {
    await launched('update');
    expect(await screen.findByRole('heading', { name: 'Version 1.1.0 is ready.' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Update now' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Later' })).toBeTruthy();
  });
  it('Update now runs update.apply and shows the applying state', async () => {
    const m = await launched('update');
    await fireEvent.click(await screen.findByRole('button', { name: 'Update now' }));
    expect(await screen.findByRole('heading', { name: 'Installing version 1.1.0...' })).toBeTruthy();
    expect(m.calls.some((c) => c.call === 'update.apply')).toBe(true);
    // The helper stops the agent: the window keeps the update screen, not "agent is down".
    m.world.down = true;
    m.emit('status');
    await new Promise((r) => setTimeout(r, 50));
    expect(screen.getByTestId('update')).toBeTruthy();
  });
  it('a helper that cannot start shows the failed state', async () => {
    const m = await launched('update');
    m.updateApplyResult = 'fail';
    await fireEvent.click(await screen.findByRole('button', { name: 'Update now' }));
    expect(await screen.findByRole('heading', { name: 'Version 1.1.0 could not be installed.' })).toBeTruthy();
    expect(screen.getByRole('alert')).toHaveTextContent('Could not start the update');
  });
  it('Later sends update.later and goes home; a dismissed update does not come back', async () => {
    const m = await launched('update');
    await fireEvent.click(await screen.findByRole('button', { name: 'Later' }));
    expect(await screen.findByTestId('headline')).toHaveTextContent('Working');
    expect(m.calls.some((c) => c.call === 'update.later')).toBe(true);
    m.setUpdate({});
    await new Promise((r) => setTimeout(r, 50));
    expect(screen.queryByTestId('update')).toBeNull();
  });
  it('shows the failed state after an update that rolled back', async () => {
    await launched('update-failed');
    expect(await screen.findByRole('heading', { name: 'Version 1.1.0 could not be installed.' })).toBeTruthy();
    await fireEvent.click(screen.getByRole('button', { name: 'OK' }));
    expect(await screen.findByTestId('headline')).toHaveTextContent('Working');
  });
  it('an open window follows the update event, but not in the middle of pairing', async () => {
    const m = await open('home', 'show');
    m.setUpdate({ state: 'available', available: '1.1.0' });
    await new Promise((r) => setTimeout(r, 50));
    expect(screen.queryByTestId('update')).toBeNull();
    store.goBack('home');
    expect(await screen.findByRole('heading', { name: 'Version 1.1.0 is ready.' })).toBeTruthy();
  });
  it('a window opened by the user stays on Home when nothing is ready', async () => {
    await open('home');
    expect(await screen.findByTestId('headline')).toHaveTextContent('Working');
    expect(screen.queryByTestId('update')).toBeNull();
  });
});
