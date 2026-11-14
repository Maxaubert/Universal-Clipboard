<script lang="ts">
  import BackBar from '../components/BackBar.svelte';
  import Logo from '../components/Logo.svelte';
  import * as art from '../lib/art';
  import { failureText } from '../lib/ipc';
  import { S } from '../lib/strings';
  import { store } from '../lib/store.svelte';

  const autostart = $derived(store.settings['start-at-login']?.value === 'on');
  const me = $derived(store.thisDevice);
  let confirm = $state(false);
  let error = $state<string | null>(null);

  async function toggleAutostart() {
    error = null;
    try {
      await store.api!.call('settings.set', { key: 'start-at-login', value: autostart ? 'off' : 'on' });
      await store.refresh();
    } catch (e) {
      error = failureText(e);
    }
  }

  async function leave() {
    error = null;
    try {
      await store.api!.call('leave', { forget: true });
      await store.refresh();
      confirm = false;
      store.back = [];
      store.go('welcome', { replace: true });
    } catch (e) {
      error = failureText(e);
      confirm = false;
    }
  }

  const sw = (on: boolean) =>
    `width: 44px; height: 26px; border: 0; border-radius: 13px; background: ${on ? '#1D1C1A' : '#BDB8B1'}; padding: 0; position: relative;`;
  const knob = (on: boolean) =>
    `position: absolute; top: 3px; left: ${on ? 21 : 3}px; width: 20px; height: 20px; border-radius: 50%; background: #FFFFFF;`;
  const tile =
    'aspect-ratio: 1 / 1; background: #FFFFFF; border: 1px solid #D9D6D1; border-radius: 12px; padding: 14px; box-sizing: border-box; display: flex; flex-direction: column; justify-content: space-between;';
</script>

<div style="flex: 1; display: flex; flex-direction: column; padding: 4px 20px 22px 20px; gap: 18px;">
  <div style="display: flex; flex-direction: column; gap: 10px;">
    <BackBar onback={() => store.goBack('home')} />
    <h1 style="font-size: 26px; line-height: 1.15;">{S.settingsTitle}</h1>
  </div>
  <div style="display: grid; grid-template-columns: repeat(2, 1fr); gap: 10px;">
    <div style={tile}>
      <svg width="30" height="30" viewBox="0 0 24 24" fill="none" stroke="#1D1C1A" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{@html art.iconPower}</svg>
      <span style="font-size: 13.5px; font-weight: 600; line-height: 1.25;">{S.startAtSignIn}</span>
      <button type="button" role="switch" aria-checked={autostart} aria-label={S.startAtSignInLabel} style={sw(autostart)} onclick={toggleAutostart}>
        <span style={knob(autostart)}></span>
      </button>
    </div>
    <div style={tile}>
      <svg width="30" height="30" viewBox="0 0 24 24" fill="none" stroke="#1D1C1A" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{@html art.iconKey}</svg>
      <span style="font-size: 13.5px; font-weight: 600; line-height: 1.25;">{S.skipPasswords}</span>
      <span style="display: flex; align-items: center; gap: 8px;">
        <button type="button" role="switch" aria-checked="true" aria-disabled="true" disabled aria-label={S.skipPasswordsLabel} style={`${sw(true)} cursor: not-allowed;`}>
          <span style={knob(true)}></span>
        </button>
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="#5E5B56" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{@html art.lock}</svg>
      </span>
    </div>
  </div>
  {#if error}<div class="err" role="alert">{error}</div>{/if}
  {#if confirm}
    <div role="alertdialog" aria-label={S.leaveAsk} style="margin-top: auto; background: #FFFFFF; border: 1px solid #B42318; border-radius: 12px; padding: 14px 16px; display: flex; flex-direction: column; gap: 12px;">
      <span style="font-size: 14px; font-weight: 600;">{S.leaveAsk}</span>
      <div style="display: flex; gap: 8px; justify-content: flex-end;">
        <button type="button" style="height: 36px; padding: 0 16px; border: 1px solid #BDB8B1; border-radius: 8px; background: #FFFFFF; color: #1D1C1A; font-size: 13.5px; font-weight: 600;" onclick={() => (confirm = false)}>{S.cancel}</button>
        <button type="button" style="height: 36px; padding: 0 16px; border: 0; border-radius: 8px; background: #B42318; color: #FFFFFF; font-size: 13.5px; font-weight: 600;" onclick={leave}>{S.leaveYes}</button>
      </div>
    </div>
  {:else}
    <div style="margin-top: auto; background: #FFFFFF; border: 1px solid #D9D6D1; border-radius: 12px; padding: 14px 16px; display: flex; align-items: center; gap: 14px;">
      <Logo os={me?.os ?? 'Windows'} size={30} />
      <div style="flex: 1; display: flex; flex-direction: column; gap: 2px;">
        <span data-testid="this-name" style="font-size: 15px; font-weight: 600;">{store.status?.this_device}</span>
        <span style="font-size: 12.5px; color: #5E5B56;">{S.version(store.status?.version ?? '')}</span>
      </div>
      <button type="button" style="height: 36px; padding: 0 16px; border: 1px solid #B42318; border-radius: 8px; background: #FFFFFF; color: #B42318; font-size: 13.5px; font-weight: 600;" onclick={() => (confirm = true)}>{S.removeThis}</button>
    </div>
  {/if}
</div>
