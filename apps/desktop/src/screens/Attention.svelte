<script lang="ts">
  import Art from '../components/Art.svelte';
  import * as art from '../lib/art';
  import { failureText } from '../lib/ipc';
  import { S } from '../lib/strings';
  import { store } from '../lib/store.svelte';

  const st = $derived(store.status);
  const code = $derived(st?.fix_code ?? '');
  // Codes with a button that does the fix. The others show the steps the agent gives.
  const oneClick = $derived(['vpn', 'firewall', 'removed', 'setup'].includes(code));
  let error = $state<string | null>(null);

  async function fix() {
    error = null;
    try {
      if (code === 'vpn') store.go('vpn');
      else if (code === 'firewall') {
        await store.api!.call('setup.step', { row: 'firewall', answer: true });
        await store.refresh();
      } else if (code === 'removed') store.go('add');
      else if (code === 'setup') store.go('perms');
      else await store.refresh();
    } catch (e) {
      error = failureText(e);
    }
  }
</script>

<div style="flex: 1; display: flex; flex-direction: column; padding: 4px 28px 28px 28px;">
  <div style="height: 36px; flex: none; display: flex; align-items: center; justify-content: flex-end; margin-right: 92px;">
    <button type="button" class="iconbtn" aria-label={S.settings} onclick={() => store.go('settings')}>
      <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{@html art.gear}</svg>
    </button>
  </div>
  <div style="flex: 1; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 30px;">
    <Art name="attention" />
    <div style="display: flex; flex-direction: column; gap: 8px; align-items: center;">
      <h1 style="font-size: 28px; line-height: 1.15; text-align: center; color: #1D1C1A;">{S.attentionTitle}</h1>
      <p data-testid="reason" style="margin: 0; font-size: 15px; line-height: 1.45; color: #5E5B56; text-align: center;">{st?.reason ?? ''}</p>
      {#if !oneClick && st?.fix}
        <p data-testid="fix-text" style="margin: 0; font-size: 14px; line-height: 1.45; color: #3F3C38; text-align: center;">{st.fix}</p>
      {/if}
      {#if error}<div class="err" role="alert">{error}</div>{/if}
    </div>
  </div>
  <div style="display: flex; flex-direction: column; align-items: center;">
    <button type="button" class="btn" onclick={fix}>{S.fix}</button>
  </div>
</div>
