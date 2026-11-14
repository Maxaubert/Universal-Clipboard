<script lang="ts">
  import BackBar from '../components/BackBar.svelte';
  import * as art from '../lib/art';
  import { failureText } from '../lib/ipc';
  import { msgIn } from '../lib/motion';
  import { S } from '../lib/strings';
  import { askRows, store } from '../lib/store.svelte';
  import type { SetupRow } from '../lib/types';

  let error = $state<string | null>(null);
  // No OS asks anything in setup any more (decision 2026-11-14). Rows appear only when the
  // OS blocks something (macOS: Local Network, paste access); otherwise Welcome skips this.
  const rows = $derived(askRows(store.setup));

  const label = (r: SetupRow) => S.rowTitles[r.row] ?? r.title;

  function icon(r: SetupRow): string {
    if (r.row === 'firewall') return art.permFirewall;
    if (r.row === 'local-network') return art.permNetwork;
    if (r.row === 'paste') return art.permPaste;
    return art.permStart;
  }

  async function allow(r: SetupRow) {
    error = null;
    try {
      await store.api!.call('setup.step', { row: r.row, answer: true });
      await store.refresh();
    } catch (e) {
      error = failureText(e);
    }
  }

  async function next(from: Element) {
    error = null;
    try {
      await store.api!.call('setup.step', { row: 'done', answer: true });
      await store.refresh();
      store.go('add', { from });
    } catch (e) {
      error = failureText(e);
    }
  }
</script>

<div style="flex: 1; display: flex; flex-direction: column; padding: 4px 24px 24px 24px; min-height: 0;">
  <div data-m="back"><BackBar onback={() => store.goBack('welcome')} /></div>
  <div style="flex: 1; display: flex; flex-direction: column; justify-content: center; align-items: center; gap: 32px;">
    <div data-m="text" style="display: flex; flex-direction: column; align-items: center; gap: 8px; text-align: center;">
      <h1 style="font-size: 28px; line-height: 1.15;">{S.permTitle(rows.length)}</h1>
      <p style="margin: 0; font-size: 15px; color: #3F3C38;">{S.permSub}</p>
    </div>
    <div data-m="rows" style="display: flex; gap: 14px; flex-wrap: wrap; justify-content: center;">
      {#each rows as r (r.row)}
        {@const done = r.state === 'done'}
        <div
          data-row={r.row}
          data-state={r.state}
          style={`width: 190px; height: 200px; box-sizing: border-box; background: #FFFFFF; border: 1px solid ${done ? '#D9D6D1' : '#1D1C1A'}; border-radius: 14px; padding: 18px 14px 16px 14px; display: flex; flex-direction: column; align-items: center; text-align: center;`}
        >
          <svg width="52" height="52" viewBox="0 0 24 24" fill="none" stroke="#1D1C1A" color="#1D1C1A" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" style="margin-top: 10px;">{@html icon(r)}</svg>
          <div style="margin-top: 16px; font-size: 14px; font-weight: 600; line-height: 1.3;">{label(r)}</div>
          {#if done}
            <div style="margin-top: auto; height: 34px; display: flex; align-items: center;">
              <svg width="26" height="26" viewBox="0 0 24 24" fill="none" stroke="#1E7A46" stroke-width="2.8" stroke-linecap="round" stroke-linejoin="round" role="img" aria-label={S.allowed}>{@html art.checkBig}</svg>
            </div>
          {:else}
            <button
              type="button"
              aria-label={label(r)}
              style="margin-top: auto; height: 34px; width: 100%; border: 1px solid #1D1C1A; border-radius: 8px; background: #FFFFFF; color: #1D1C1A; font-size: 13px; font-weight: 700;"
              onclick={() => allow(r)}>{S.allow}</button>
          {/if}
        </div>
      {/each}
    </div>
    {#if error}<div class="err" role="alert" in:msgIn>{error}</div>{/if}
  </div>
  <div data-m="cta" style="display: flex; flex-direction: column; align-items: center; gap: 16px;">
    <button type="button" class="btn" data-shape onclick={(e) => next(e.currentTarget)}><span class="lbl">{S.continue}</span></button>
  </div>
</div>
