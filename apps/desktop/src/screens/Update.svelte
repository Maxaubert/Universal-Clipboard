<script lang="ts">
  // Update: PLACEHOLDER screen (2026-11-14). The agent opens the window here once when it
  // finds a newer version at start. Final visuals and motion come from Max's mockup pick;
  // keep everything about this screen in this one file so it can be restyled alone.
  import { failureText } from '../lib/ipc';
  import { S } from '../lib/strings';
  import { store } from '../lib/store.svelte';

  const u = $derived(store.update);
  const phase = $derived(u?.state ?? 'none');
  let error = $state<string | null>(null);
  let busy = $state(false);

  async function now() {
    error = null;
    busy = true;
    try {
      await store.api!.call('update.apply');
      await store.refreshUpdate();
    } catch (e) {
      error = failureText(e);
      await store.refreshUpdate();
    } finally {
      busy = false;
    }
  }

  async function later() {
    try {
      await store.api!.call('update.later');
    } catch {
      /* the agent may be gone; leaving the screen is all that matters */
    }
    store.leaveUpdate();
  }
</script>

<div data-testid="update" style="flex: 1; display: flex; flex-direction: column; padding: 4px 28px 28px 28px;">
  <div style="height: 36px; flex: none;"></div>
  <div style="flex: 1; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 10px;">
    {#if phase === 'applying'}
      <h1 style="font-size: 28px; line-height: 1.15; text-align: center; color: #1D1C1A;">{S.updateApplying(u?.available ?? '')}</h1>
      <p style="margin: 0; font-size: 15px; line-height: 1.45; color: #5E5B56; text-align: center;">{S.updateApplyingSub}</p>
    {:else if phase === 'failed'}
      <h1 style="font-size: 28px; line-height: 1.15; text-align: center; color: #1D1C1A;">{S.updateFailed(u?.failed_version ?? '')}</h1>
      <p style="margin: 0; font-size: 15px; line-height: 1.45; color: #5E5B56; text-align: center;">{S.updateFailedSub(u?.current ?? '')}</p>
    {:else if phase === 'available'}
      <h1 style="font-size: 28px; line-height: 1.15; text-align: center; color: #1D1C1A;">{S.updateReady(u?.available ?? '')}</h1>
      <p style="margin: 0; font-size: 15px; line-height: 1.45; color: #5E5B56; text-align: center;">{S.updateReadySub(u?.current ?? '')}</p>
    {:else}
      <h1 style="font-size: 28px; line-height: 1.15; text-align: center; color: #1D1C1A;">{S.updateNone(u?.current ?? '')}</h1>
    {/if}
    {#if error}<div class="err" role="alert">{error}</div>{/if}
  </div>
  <div style="display: flex; flex-direction: column; align-items: center; gap: 10px;">
    {#if phase === 'available'}
      <button type="button" class="btn" disabled={busy} onclick={now}>{S.updateNow}</button>
      <button type="button" class="btn ghost" disabled={busy} onclick={later}>{S.updateLater}</button>
    {:else if phase !== 'applying'}
      <button type="button" class="btn" onclick={() => store.leaveUpdate()}>{S.updateOk}</button>
    {/if}
  </div>
</div>
