<script lang="ts">
  import Art from '../components/Art.svelte';
  import { S } from '../lib/strings';
  import { store } from '../lib/store.svelte';

  const name = $derived(store.added ?? '');
  const os = $derived(store.devices.find((d) => d.name === name)?.os ?? 'macOS');
</script>

<div style="flex: 1; display: flex; flex-direction: column; padding: 40px 28px 28px 28px;">
  <div style="flex: 1; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 30px;">
    <Art name="connected" {os} />
    <div style="display: flex; flex-direction: column; gap: 8px; align-items: center;">
      <h1 style="font-size: 28px; line-height: 1.15; text-align: center; color: #1D1C1A;">{name}{S.addedSuffix}</h1>
      <p style="margin: 0; font-size: 15px; line-height: 1.45; color: #5E5B56; text-align: center;">{S.addedSub}</p>
    </div>
  </div>
  <div style="display: flex; flex-direction: column; align-items: center; gap: 10px;">
    <button type="button" class="btn" onclick={() => { store.back = []; store.go('home', { replace: true }); }}>{S.goToDevices}</button>
    <button type="button" class="link" onclick={() => store.go('add')}>{S.addAnother}</button>
  </div>
</div>
