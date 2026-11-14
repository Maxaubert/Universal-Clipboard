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
    expect(document.querySelectorAll('[data-device] svg[fill="#B3AEA7"]').length).toBe(6);
  });
});

describe('Needs attention', () => {
  it('shows the reason and Fix runs the firewall step', async () => {
    const m = await open('attention');
    expect(await screen.findByTestId('reason')).toHaveTextContent('Firewall is blocking it.');
    await fireEvent.click(screen.getByRole('button', { name: 'Fix' }));
    await waitFor(() => expect(screen.getByTestId('headline')).toHaveTextContent('Working'));
    expect(m.calls.some((c) => c.call === 'setup.step' && c.params.row === 'firewall')).toBe(true);
  });
  it('sends a VPN problem to the VPN help screen', async () => {
    await open('vpn');
    await fireEvent.click(await screen.findByRole('button', { name: 'Fix' }));
    expect(await screen.findByText("Can't reach your other device?")).toBeTruthy();
    expect(screen.getByText('Turn on Local network sharing in your VPN')).toBeTruthy();
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
