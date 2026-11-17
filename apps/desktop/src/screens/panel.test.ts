import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import Panel from '../Panel.svelte';
import { setApi } from '../lib/ipc';
import { createMockApi, type Scenario } from '../lib/mock';
import { panelView, shortAgo } from '../lib/panel';
import type { Device, Status } from '../lib/types';

async function open(scenario: Scenario) {
  setApi(createMockApi(scenario));
  render(Panel);
  const p = await screen.findByTestId('panel');
  await waitFor(() => expect(p.hidden).toBe(false));
  return { m: window.__ucMock!, p };
}

const status = (s: Partial<Status>): Status => ({
  state: 'working',
  text: 'Working',
  reason: null,
  fix: null,
  fix_code: null,
  devices: 2,
  connected: 1,
  paused: false,
  this_device: "Max's PC",
  version: '0.3.0',
  ...s,
});
const devs: Device[] = [
  { id: 'a', name: "Max's PC", os: 'Windows', this: true, state: 'This device' },
  { id: 'b', name: "Max's Mac", os: 'macOS', this: false, state: 'Connected' },
  { id: 'c', name: "Max's iPhone", os: 'iOS', this: false, state: 'Asleep or away' },
];

describe('panelView', () => {
  const now = 1_000_000_000;
  it('synced shows one short fact and the other devices', () => {
    const v = panelView(
      status({ activity: [{ kind: 'Text', direction: 'received', device: "Max's Mac", time_ms: now - 120_000, note: null }] }),
      devs,
      null,
      now,
    );
    expect(v.word).toBe('Synced');
    expect(v.lamp).toBe('ok');
    expect(v.sub).toBe("Text from Max's Mac, 2 min ago");
    expect(v.chips).toEqual([
      { name: "Max's Mac", on: true },
      { name: "Max's iPhone", on: false },
    ]);
    expect(v.xfer).toBeNull();
  });
  it('a transfer is only a bar and a percent, no fact line', () => {
    const t = { id: '1', phase: 'receiving', device: "Max's Mac", done: 40, total: 100, files: 3 } as const;
    const v = panelView(status({}), devs, t, now);
    expect(v.word).toBe('Receiving');
    expect(v.xfer).toEqual({ up: false, pct: 40 });
    expect(v.sub).toBeNull();
    const s = panelView(status({}), devs, { ...t, phase: 'sending', done: 72 }, now);
    expect(s.word).toBe('Sending');
    expect(s.xfer).toEqual({ up: true, pct: 72 });
  });
  it('paused, not connected with the VPN note, needs attention, agent down', () => {
    expect(panelView(status({ state: 'paused', paused: true }), devs, null, now)).toMatchObject({ word: 'Paused', lamp: 'off', paused: true, sub: null });
    const vpn = panelView(
      status({ state: 'needs-attention', fix_code: 'vpn', reason: "Can't reach your other device? Your VPN blocks the local network." }),
      devs,
      null,
      now,
    );
    expect(vpn).toMatchObject({ word: 'Not connected', lamp: 'warn' });
    expect(vpn.note).toEqual({ text: "Can't reach your other device? Your VPN blocks the local network.", fix: 'Show me how', route: 'vpn' });
    const clock = panelView(status({ state: 'needs-attention', fix_code: 'clock', reason: 'The date and time are wrong.' }), devs, null, now);
    expect(clock.word).toBe('Needs attention');
    expect(clock.note?.route).toBeNull();
    expect(panelView(null, devs, null, now)).toMatchObject({ word: 'Not running', lamp: 'warn' });
    expect(panelView(status({ state: 'no-peers', connected: 0 }), devs, null, now).word).toBe('Not connected');
  });
  it('short times', () => {
    expect(shortAgo(now - 5_000, now)).toBe('just now');
    expect(shortAgo(now - 3 * 3_600_000, now)).toBe('3 h ago');
    expect(shortAgo(now - 86_400_000, now)).toBe('1 day ago');
  });
});

describe('Panel', () => {
  it('shows the state, reports its height and runs the actions in the shell', async () => {
    const { m, p } = await open('home');
    expect(p.dataset.state).toBe('Synced');
    await waitFor(() => expect(m.windowLog.some((l) => l.startsWith('panel-ready'))).toBe(true));
    await fireEvent.click(screen.getByRole('button', { name: /Copy files/ }));
    await fireEvent.click(screen.getByRole('button', { name: /Copy folder/ }));
    await fireEvent.click(screen.getByRole('button', { name: 'Open Universal Clipboard' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Settings' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Quit (stops syncing)' }));
    const tray = m.calls.filter((c) => c.call.startsWith('tray.'));
    expect(tray).toEqual([
      { call: 'tray.copy', params: { folder: false } },
      { call: 'tray.copy', params: { folder: true } },
      { call: 'tray.open', params: { route: null } },
      { call: 'tray.open', params: { route: 'settings' } },
      { call: 'tray.quit', params: {} },
    ]);
    // Never a send, a get or a pick from the page.
    expect(m.calls.some((c) => c.call === 'pick' || c.call === 'files.copy')).toBe(false);
  });
  it('the Sync switch pauses and resumes', async () => {
    const { m, p } = await open('home');
    const sw = screen.getByRole('switch', { name: 'Sync' });
    expect(sw.getAttribute('aria-checked')).toBe('true');
    await fireEvent.click(sw);
    await waitFor(() => expect(p.dataset.state).toBe('Paused'));
    expect(m.calls.some((c) => c.call === 'pause.set' && c.params.mode === 'forever')).toBe(true);
    await fireEvent.click(screen.getByRole('switch', { name: 'Sync' }));
    await waitFor(() => expect(p.dataset.state).toBe('Synced'));
  });
  it('a transfer shows the bar, the percent and the stop button only', async () => {
    const { m, p } = await open('home');
    m.setTransfer({ id: 'x', phase: 'receiving', device: 'Mac mini', done: 40, total: 100, files: 2 });
    const row = await screen.findByTestId('xfer');
    expect(p.dataset.state).toBe('Receiving');
    expect(row.textContent?.trim()).toBe('40 %');
    await fireEvent.click(screen.getByRole('button', { name: 'Stop' }));
    expect(m.calls.some((c) => c.call === 'transfer.stop')).toBe(true);
    m.setTransfer(null);
    await waitFor(() => expect(screen.queryByTestId('xfer')).toBeNull());
  });
  it('the VPN note opens the help screen', async () => {
    const { m, p } = await open('vpn');
    expect(p.dataset.state).toBe('Not connected');
    expect(p).toHaveTextContent('Your VPN blocks the local network.');
    await fireEvent.click(screen.getByRole('button', { name: 'Show me how' }));
    expect(m.calls.some((c) => c.call === 'tray.open' && c.params.route === 'vpn')).toBe(true);
  });
});
