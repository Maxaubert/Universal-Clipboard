// Update screens: U1 "One shape" (the agent opened the window) and U3 "Home card" (Max's pick
// 2026-11-14, docs/design/desktop/motion/update.html), against the mock agent.
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import App from '../App.svelte';
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

async function launched(scenario: Scenario) {
  const api = createMockApi(scenario);
  window.__ucMock!.launchRoute = 'update';
  setApi(api);
  render(App);
  await waitFor(() => expect(store.loaded).toBeTruthy());
  return window.__ucMock!;
}
const h = (name: string) => screen.findByRole('heading', { name });
const live = () => document.querySelector('[data-testid="update"] [role="status"]')?.textContent ?? '';

describe('Update: U1 screen (the agent opened the window)', () => {
  it('opens on "Version X is ready." with the version you have', async () => {
    await launched('update');
    expect(await h('Version 0.1.1 is ready.')).toBeTruthy();
    expect(screen.getByText('A newer Universal Clipboard is ready to install.')).toBeTruthy();
    expect(screen.getByText('You have 0.1.0.')).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Update now' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Later' })).toBeTruthy();
  });
  it('Update now runs update.apply and shows Updating...; the agent going away keeps it', async () => {
    const m = await launched('update');
    await fireEvent.click(await screen.findByRole('button', { name: 'Update now' }));
    expect(await h('Updating...')).toBeTruthy();
    expect(screen.getByText('The app closes and opens again by itself.')).toBeTruthy();
    expect(m.calls.some((c) => c.call === 'update.apply')).toBe(true);
    expect(live()).toContain('Updating...');
    m.world.down = true;
    m.emit('status');
    await new Promise((r) => setTimeout(r, 50));
    expect(screen.getByTestId('update')).toBeTruthy();
  });
  it('after the restart: "Updated to X" and Done goes home', async () => {
    const m = await launched('updated');
    expect(await h('Updated to 0.1.1')).toBeTruthy();
    expect(screen.getByText('You have the newest version.')).toBeTruthy();
    await fireEvent.click(screen.getByRole('button', { name: 'Done' }));
    expect(await screen.findByTestId('headline')).toHaveTextContent('Working');
    expect(m.calls.some((c) => c.call === 'update.later')).toBe(true);
  });
  it('a window that lives through the update follows it to the check', async () => {
    const m = await launched('update');
    await fireEvent.click(await screen.findByRole('button', { name: 'Update now' }));
    expect(await h('Updating...')).toBeTruthy();
    m.finishUpdate(true);
    expect(await h('Updated to 0.1.1')).toBeTruthy();
  });
  it('a failed update: Try again installs once more', async () => {
    const m = await launched('update-failed');
    expect(await h('The update did not work.')).toBeTruthy();
    expect(screen.getByText('You still have 0.1.0.')).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Later' })).toBeTruthy();
    await fireEvent.click(screen.getByRole('button', { name: 'Try again' }));
    expect(await h('Updating...')).toBeTruthy();
    expect(m.calls.filter((c) => c.call === 'update.apply').length).toBe(1);
  });
  it('a helper that cannot start shows the failure in place', async () => {
    const m = await launched('update');
    m.updateApplyResult = 'fail';
    await fireEvent.click(await screen.findByRole('button', { name: 'Update now' }));
    expect(await h('The update did not work.')).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Try again' })).toBeTruthy();
  });
  it('a version marked bad cannot be tried again: only OK', async () => {
    const m = await launched('update-failed');
    await h('The update did not work.');
    m.setUpdate({ available: null });
    await waitFor(() => expect(screen.getByRole('button', { name: 'OK' })).toBeTruthy());
    expect(screen.queryByRole('button', { name: 'Try again' })).toBeNull();
  });
  it('Later sends update.later and goes home; a dismissed update does not come back', async () => {
    const m = await launched('update');
    await fireEvent.click(await screen.findByRole('button', { name: 'Later' }));
    expect(await screen.findByTestId('headline')).toHaveTextContent('Working');
    expect(m.calls.some((c) => c.call === 'update.later')).toBe(true);
    m.setUpdate({});
    await new Promise((r) => setTimeout(r, 50));
    expect(screen.queryByTestId('update')).toBeNull();
    expect(screen.queryByTestId('update-card')).toBeNull();
  });
});

describe('Update: U3 card on Home (the window was open, or opened by the user)', () => {
  it('a ready update shows as a card on Home, not a new screen', async () => {
    await open('update');
    const c = await screen.findByTestId('update-card');
    expect(c).toHaveTextContent('Version 0.1.1 is ready.');
    expect(screen.queryByTestId('update')).toBeNull();
  });
  it('an open Home gets the card when the agent finds an update', async () => {
    const m = await open('home');
    expect(await screen.findByTestId('headline')).toHaveTextContent('Working');
    m.setUpdate({ state: 'available', available: '0.1.1' });
    m.emit('open', { route: 'update' });
    expect(await screen.findByTestId('update-card')).toBeTruthy();
    expect(screen.queryByTestId('update')).toBeNull();
  });
  it('Update now on the card goes on to the update screen and installs', async () => {
    const m = await open('update');
    await fireEvent.click(await screen.findByRole('button', { name: 'Update now' }));
    expect(await h('Updating...')).toBeTruthy();
    await waitFor(() => expect(m.calls.some((c) => c.call === 'update.apply')).toBe(true));
  });
  it('a failure after Update now on the card goes back to Home, with Try again on the card', async () => {
    const m = await open('update');
    m.updateApplyResult = 'fail';
    await fireEvent.click(await screen.findByRole('button', { name: 'Update now' }));
    await waitFor(() => expect(screen.getByTestId('update-card')).toHaveTextContent('The update did not work.'));
    expect(screen.getByTestId('update-card')).toHaveTextContent('You still have 0.1.0.');
    expect(screen.getByRole('button', { name: 'Try again' })).toBeTruthy();
    expect(screen.queryByTestId('update')).toBeNull();
  });
  it('Later closes the card', async () => {
    const m = await open('update');
    await screen.findByTestId('update-card');
    await fireEvent.click(screen.getByRole('button', { name: 'Later' }));
    await waitFor(() => expect(screen.queryByTestId('update-card')).toBeNull());
    expect(m.calls.some((c) => c.call === 'update.later')).toBe(true);
  });
  it('another screen still moves to the update screen; a pairing waits', async () => {
    const m = await open('home', 'show');
    m.setUpdate({ state: 'available', available: '0.1.1' });
    await new Promise((r) => setTimeout(r, 50));
    expect(screen.queryByTestId('update')).toBeNull();
    store.goBack('settings');
    expect(await h('Version 0.1.1 is ready.')).toBeTruthy();
  });
  it('nothing ready: no card', async () => {
    await open('home');
    expect(await screen.findByTestId('headline')).toHaveTextContent('Working');
    expect(screen.queryByTestId('update-card')).toBeNull();
  });
});
