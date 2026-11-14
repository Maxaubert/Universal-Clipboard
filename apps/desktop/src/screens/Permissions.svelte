<script lang="ts">
  import BackBar from '../components/BackBar.svelte';
  import * as art from '../lib/art';
  import { failureText } from '../lib/ipc';
  import { S } from '../lib/strings';
  import { store } from '../lib/store.svelte';
  import type { SetupRow } from '../lib/types';

  let error = $state<string | null>(null);
  // The design shows "Start when you sign in" first.
  const rows = $derived(
    [...(store.setup?.rows ?? [])].sort((a, b) => Number(b.row === 'autostart') - Number(a.row === 'autostart')),
  );

  const label = (r: SetupRow) => S.rowTitles[r.row] ?? r.title;

  function icon(r: SetupRow): string {
    return r.row === 'autostart' ? art.permStart : r.row === 'firewall' ? art.permFirewall : art.permStart;
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

  async function next() {
    error = null;
    try {
      await store.api!.call('setup.step', { row: 'done', answer: true });
      await store.refresh();
      store.go('add');
    } catch (e) {
      error = failureText(e);
    }
  }
</script>

<div style="flex: 1; display: flex; flex-direction: column; padding: 4px 24px 24px 24px; min-height: 0;">
  <BackBar onback={() => store.goBack('welcome')} />
  <div style="flex: 1; display: flex; flex-direction: column; justify-content: center; align-items: center; gap: 32px;">
    <div style="display: flex; flex-direction: column; align-items: center; gap: 8px; text-align: center;">
      <h1 style="font-size: 28px; line-height: 1.15;">{S.permTitle}</h1>
      <p style="margin: 0; font-size: 15px; color: #3F3C38;">{S.permSub}</p>
    </div>
    <div style="display: flex; gap: 14px; flex-wrap: wrap; justify-content: center;">
      {#each rows as r (r.row)}
        {@const done = r.state === 'done'}
        <div
          data-row={r.row}
          data-state={r.state}
          style={`width: 190px; height: 200px; box-sizing: border-box; background: #FFFFFF; border: 1px solid ${done ? '#D9D6D1' : '#1D1C1A'}; border-radius: 14px; padding: 18px 14px 16px 14px; display: flex; flex-direction: column; align-items: center; text-align: center;`}
        >
          <svg width="52" height="52" viewBox="0 0 24 24" fill="none" stroke="#1D1C1A" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" style="margin-top: 10px;">{@html icon(r)}</svg>
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
    {#if error}<div class="err" role="alert">{error}</div>{/if}
  </div>
  <div style="display: flex; flex-direction: column; align-items: center; gap: 16px;">
    <button type="button" class="btn" onclick={next}>{S.continue}</button>
  </div>
</div>
