import { describe, expect, it } from 'vitest';
import { ALLOWED_CALLS, failureText } from './ipc';
import { createMockApi } from './mock';
import { S, sizeText, thisLabel, transferText } from './strings';
import { STEPS, stepOf } from './store.svelte';
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

describe('file copies in words', () => {
  it('writes sizes as the agent does', () => {
    expect(sizeText(0)).toBe('0 bytes');
    expect(sizeText(1)).toBe('1 byte');
    expect(sizeText(1500)).toBe('1 KB');
    expect(sizeText(12 << 20)).toBe('12 MB');
    expect(sizeText((3 << 20) + 1005)).toBe('3 MB');
    expect(sizeText(3 * 2 ** 29)).toBe('1.5 GB');
  });
  it('says what moves and how much', () => {
    expect(transferText({ phase: 'preparing', device: null, done: 0, total: 0, files: 2 })).toEqual({
      title: 'Preparing files...',
      amount: '2 files',
    });
    expect(transferText({ phase: 'sending', device: '2 devices', done: 1 << 20, total: 4 << 20, files: 1 })).toEqual({
      title: 'Sending to 2 devices...',
      amount: '1 MB of 4 MB, 1 file',
    });
    expect(transferText({ phase: 'receiving', device: 'Mac', done: 0, total: 10, files: 1 }).title).toBe('Receiving from Mac...');
  });
  it('the window may copy files and stop a transfer', () => {
    expect(ALLOWED_CALLS).toContain('files.copy');
    expect(ALLOWED_CALLS).toContain('transfer.stop');
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
  it('has the 4-step indicator only on setup screens, the same on every OS', () => {
    expect(STEPS).toBe(4);
    expect(['welcome', 'add', 'show', 'connected'].map((r) => stepOf(r as never))).toEqual([1, 2, 3, 4]);
    // No permissions step (decision 2026-11-14): the screen, when the OS blocks something, is part of step 1.
    expect(stepOf('perms')).toBe(1);
    expect(stepOf('enter')).toBe(3);
    expect(stepOf('home')).toBe(0);
    expect(stepOf('settings')).toBe(0);
  });
  it('has no em-dash in any string', () => {
    const all = Object.values(S).map((v) =>
      typeof v === 'function' ? (v as (a: unknown) => unknown)(v === S.permTitle ? 3 : 'x') : v,
    );
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
  it('has no setup rows, except the macOS rows when macOS blocks something', async () => {
    expect((await createMockApi('fresh').call<{ rows: unknown[] }>('setup.status')).rows).toEqual([]);
    const mac = await createMockApi('mac-blocked').call<{ rows: { row: string }[] }>('setup.status');
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
