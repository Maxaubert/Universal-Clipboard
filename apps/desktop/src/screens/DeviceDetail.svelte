<script lang="ts">
  import BackBar from '../components/BackBar.svelte';
  import Logo from '../components/Logo.svelte';
  import * as art from '../lib/art';
  import { failureText } from '../lib/ipc';
  import { S } from '../lib/strings';
  import { store } from '../lib/store.svelte';

  const d = $derived(store.devices.find((x) => x.id === store.selected));
  const paused = $derived(store.status?.state === 'paused');
  let renaming = $state(false);
  let name = $state('');
  let confirm = $state(false);
  let error = $state<string | null>(null);

  const stateText = $derived.by(() => {
    if (!d) return '';
    if (d.this) return 'This device';
    if (d.state === 'Connected') return paused ? S.tilePaused : S.tileConnected;
    return d.state === 'Blocked by a VPN or firewall' ? d.state : 'Asleep or away';
  });
  const dot = $derived(
    !d || d.this
      ? ''
      : paused
        ? 'background: #B3AEA7;'
        : d.state === 'Connected'
          ? 'background: #1E7A46;'
          : d.state === 'Blocked by a VPN or firewall'
            ? 'background: #B42318;'
            : 'border: 1.5px solid #6E6A64;',
  );

  function startRename() {
    name = d?.name ?? '';
    renaming = true;
    error = null;
  }

  async function saveRename() {
    if (!d) return;
    error = null;
    try {
      await store.api!.call('devices.rename', { device: d.id, name });
      await store.refresh();
      renaming = false;
    } catch (e) {
      error = failureText(e);
    }
  }

  async function remove() {
    if (!d) return;
    error = null;
    try {
      await store.api!.call('devices.remove', { device: d.id });
      await store.refresh();
      store.goBack('home');
    } catch (e) {
      error = failureText(e);
      confirm = false;
    }
  }

  const outline =
    'height: 44px; padding: 0 20px; border: 1px solid #BDB8B1; border-radius: 10px; background: #FFFFFF; color: #1D1C1A; font-size: 14px; font-weight: 600; display: flex; align-items: center; justify-content: center; gap: 8px; width: 100%;';
</script>

<div style="flex: 1; display: flex; flex-direction: column; padding: 4px 28px 28px 28px;">
  <BackBar onback={() => store.goBack('home')} />
  {#if d}
    <div style="flex: 1; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 14px;">
      <Logo os={d.os} size={108} />
      <div style="display: flex; flex-direction: column; align-items: center; gap: 8px; margin-top: 8px; width: 100%;">
        {#if renaming}
          <input
            aria-label={S.renameLabel}
            bind:value={name}
            maxlength="64"
            onkeydown={(e) => e.key === 'Enter' && saveRename()}
            style="width: 100%; box-sizing: border-box; height: 48px; border: 2px solid #1D1C1A; border-radius: 10px; background: #FFFFFF; font: inherit; font-size: 24px; font-weight: 700; text-align: center; color: #1D1C1A;"
          />
        {:else}
          <h1 data-testid="device-name" style="font-size: 30px; line-height: 1.15; text-align: center;">{d.name}</h1>
        {/if}
        <div style="display: flex; align-items: center; justify-content: center; gap: 8px; font-size: 14px; color: #5E5B56;">
          {#if dot}<span class="dot" style={dot}></span>{/if}
          <span data-testid="device-state">{stateText}</span>
        </div>
      </div>
      {#if error}<div class="err" role="alert">{error}</div>{/if}
    </div>
    <div style="display: flex; flex-direction: column; gap: 6px;">
      {#if renaming}
        <button type="button" class="btn" onclick={saveRename}>{S.save}</button>
        <button type="button" style={outline} onclick={() => (renaming = false)}>{S.cancel}</button>
      {:else if confirm}
        <div role="alertdialog" aria-label={S.removeAsk(d.name)} style="text-align: center; font-size: 14px; font-weight: 600; padding: 4px 0 6px;">{S.removeAsk(d.name)}</div>
        <button type="button" style="height: 44px; width: 100%; border: 0; border-radius: 10px; background: #B42318; color: #FFFFFF; font-size: 14px; font-weight: 600;" onclick={remove}>{S.removeYes}</button>
        <button type="button" style={outline} onclick={() => (confirm = false)}>{S.cancel}</button>
      {:else}
        <button type="button" style={outline} onclick={startRename}>
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{@html art.pencil}</svg>{S.rename}
        </button>
        {#if !d.this}
          <button
            type="button"
            style="height: 40px; padding: 0 12px; border: 0; border-radius: 8px; background: transparent; color: #B42318; font-size: 14px; font-weight: 600; display: flex; align-items: center; justify-content: center; gap: 8px; width: 100%;"
            onclick={() => (confirm = true)}>{S.removeDevice}</button>
        {/if}
      {/if}
    </div>
  {:else}
    <div style="flex: 1;"></div>
  {/if}
</div>
