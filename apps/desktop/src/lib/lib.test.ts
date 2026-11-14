import { describe, expect, it } from 'vitest';
import { ALLOWED_CALLS, failureText } from './ipc';
import { createMockApi } from './mock';
import { S, thisLabel } from './strings';
import { stepOf } from './store.svelte';
import { osKind } from './types';

describe('ipc', () => {
  it('has no send, get or fetch call and no debug call', () => {
    for (const c of ALLOWED_CALLS) {
      expect(c).not.toMatch(/^(send|get|fetch)$|send|fetch|debug|clipboard/);
    }
  });
  it('turns failures into text', () => {
    expect(failureText({ code: 'x', message: 'Plain text.' })).toBe('Plain text.');
    expect(failureText(new Error('boom'))).toBe('boom');
  });
});

describe('types and strings', () => {
  it('maps the agent OS names to logos', () => {
    expect(osKind('Windows')).toBe('windows');
    expect(osKind('macOS')).toBe('apple');
    expect(osKind('iPhone')).toBe('apple');
    expect(osKind('iPad')).toBe('apple');
    expect(osKind('Android tablet')).toBe('android');
    expect(osKind('Linux')).toBe('linux');
  });
  it('labels this device by OS', () => {
    expect(thisLabel('Windows')).toBe('This PC');
    expect(thisLabel('macOS')).toBe('This Mac');
    expect(thisLabel('Linux')).toBe('This computer');
  });
  it('has the 5-step indicator only on setup screens', () => {
    expect(['welcome', 'perms', 'add', 'show', 'connected'].map((r) => stepOf(r as never))).toEqual([1, 2, 3, 4, 5]);
    expect(stepOf('enter')).toBe(4);
    expect(stepOf('home')).toBe(0);
    expect(stepOf('settings')).toBe(0);
  });
  it('has 4 steps without the permissions screen (Windows, Linux)', () => {
    expect(['welcome', 'add', 'show', 'connected'].map((r) => stepOf(r as never, false))).toEqual([1, 2, 3, 4]);
    expect(stepOf('enter', false)).toBe(3);
    expect(stepOf('perms', false)).toBe(0);
  });
  it('has no em-dash in any string', () => {
    const all = Object.values(S).map((v) => (typeof v === 'function' ? v('x') : v));
    for (const t of all) expect(t).not.toContain('—');
  });
});

describe('mock agent', () => {
  it('answers like the real agent', async () => {
    const api = createMockApi('home');
    const st = await api.call<{ state: string; devices: number }>('status');
    expect(st.state).toBe('working');
    expect(st.devices).toBe(6);
    await expect(api.call('debug.copy')).rejects.toMatchObject({ code: 'unknown-call' });
  });
  it('has no setup rows on Windows and two on macOS', async () => {
    expect((await createMockApi('fresh').call<{ rows: unknown[] }>('setup.status')).rows).toEqual([]);
    const mac = await createMockApi('fresh-mac').call<{ rows: { row: string }[] }>('setup.status');
    expect(mac.rows.map((r) => r.row)).toEqual(['local-network', 'paste']);
  });
  it('refuses a short code', async () => {
    const api = createMockApi('fresh');
    await expect(api.call('pair.join', { code: '12' })).rejects.toMatchObject({ code: 'bad-code' });
  });
  it('asks before leaving a group', async () => {
    const api = createMockApi('home');
    await expect(api.call('pair.join', { code: '418207' })).rejects.toMatchObject({ code: 'in-group' });
  });
});
