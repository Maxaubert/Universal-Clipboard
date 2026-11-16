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

describe('Home: files', () => {
  it('Copy files... sends the picked files to the agent as one copy', async () => {
    const m = await open('home');
    await fireEvent.click(await screen.findByRole('button', { name: 'Copy files...' }));
    await waitFor(() => expect(m.calls.some((c) => c.call === 'files.copy')).toBe(true));
    expect(m.calls.find((c) => c.call === 'pick')?.params).toEqual({ folder: false });
    expect(m.calls.find((c) => c.call === 'files.copy')?.params).toEqual({ paths: m.pickResult.files, picked: true });
    // The agent prepares them: the progress line shows, with Stop.
    expect(await screen.findByTestId('transfer')).toHaveTextContent('Preparing files...');
  });
  it('Copy folder... picks a folder; a cancelled picker sends nothing', async () => {
    const m = await open('home');
    m.pickResult = { files: [], folder: ['/home/max/Photos'] };
    await fireEvent.click(await screen.findByRole('button', { name: 'Copy files...' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Copy folder...' }));
    await waitFor(() => expect(m.calls.filter((c) => c.call === 'files.copy').length).toBe(1));
    expect(m.calls.filter((c) => c.call === 'pick').map((c) => c.params.folder)).toEqual([false, true]);
    expect(m.calls.find((c) => c.call === 'files.copy')?.params.paths).toEqual(['/home/max/Photos']);
  });
  it('shows why a copy cannot start', async () => {
    const m = await open('home');
    m.world.settings['sync-files'].value = 'off';
    await fireEvent.click(await screen.findByRole('button', { name: 'Copy files...' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('Files are not synced.');
  });
  it('shows the progress line and Stop stops the transfer', async () => {
    const m = await open('home');
    expect(screen.queryByTestId('transfer')).toBeNull();
    m.setTransfer({ id: 'ab12cd34', phase: 'sending', device: 'Mac mini', done: 45 << 20, total: 300 << 20, files: 3 });
    const line = await screen.findByTestId('transfer');
    expect(line).toHaveTextContent('Sending to Mac mini...');
    expect(line).toHaveTextContent('45 MB of 300 MB, 3 files');
    m.setTransfer({ id: 'ab12cd34', phase: 'receiving', device: 'Ubuntu laptop', done: 1, total: 2048, files: 1 });
    await waitFor(() => expect(screen.getByTestId('transfer')).toHaveTextContent('Receiving from Ubuntu laptop...'));
    expect(screen.getByTestId('transfer')).toHaveTextContent('1 byte of 2 KB, 1 file');
    await fireEvent.click(screen.getByRole('button', { name: 'Stop' }));
    await waitFor(() => expect(screen.queryByTestId('transfer')).toBeNull());
    expect(m.calls.some((c) => c.call === 'transfer.stop')).toBe(true);
  });
  it('takes the transfer from status when the window opens', async () => {
    await open('sending');
    expect(await screen.findByTestId('transfer')).toHaveTextContent("Sending to Max's Mac Mini...");
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
  it('turns file sync off and on, and sets the automatic limit', async () => {
    const m = await open('home', 'settings');
    const sw = await screen.findByRole('switch', { name: 'Sync files and folders you copy' });
    const box = screen.getByRole('textbox', { name: /Largest file copy/ }) as HTMLInputElement;
    expect(sw.getAttribute('aria-checked')).toBe('true');
    expect(box.value).toBe('100');
    await fireEvent.change(box, { target: { value: '500' } });
    await waitFor(() => expect(m.world.settings['file-limit-mb'].value).toBe('500'));
    expect(m.calls.some((c) => c.call === 'settings.set' && c.params.key === 'file-limit-mb' && c.params.value === '500')).toBe(true);
    await fireEvent.click(sw);
    await waitFor(() => expect(sw.getAttribute('aria-checked')).toBe('false'));
    expect(m.world.settings['sync-files'].value).toBe('off');
    expect(box).toBeDisabled();
  });
  it("shows the agent's answer for a limit out of range and keeps the old value", async () => {
    await open('home', 'settings');
    const box = (await screen.findByRole('textbox', { name: /Largest file copy/ })) as HTMLInputElement;
    // The mock takes any value; the real agent refuses this one.
    const api = store.api!;
    const call = api.call.bind(api);
    api.call = async <T,>(name: string, p: Record<string, unknown> = {}) => {
      if (name === 'settings.set' && p.key === 'file-limit-mb') throw { code: 'bad-setting', message: 'Use a whole number from 1 to 10240.' };
      return call<T>(name, p);
    };
    await fireEvent.change(box, { target: { value: '0' } });
    expect(await screen.findByRole('alert')).toHaveTextContent('Use a whole number from 1 to 10240.');
    expect(box.value).toBe('100');
  });
});

describe('Qr', () => {
  it('draws a path for the agent text', () => {
    render(Qr, { text: 'UCLIP1:abc' });
    const p = document.querySelector('svg path');
    expect(p?.getAttribute('d')?.length).toBeGreaterThan(100);
  });
});

