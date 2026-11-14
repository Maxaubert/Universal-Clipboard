<script lang="ts">
  import BackBar from '../components/BackBar.svelte';
  import Art from '../components/Art.svelte';
  import { S } from '../lib/strings';
  import { store } from '../lib/store.svelte';
</script>

<div style="flex: 1; display: flex; flex-direction: column; padding: 4px 28px 28px 28px;">
  <BackBar onback={() => store.goBack('home')} />
  <div style="flex: 1; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 26px;">
    <Art name="vpn" />
    <div style="display: flex; flex-direction: column; gap: 8px; align-items: center;">
      <h1 style="font-size: 28px; line-height: 1.15; text-align: center; color: #1D1C1A;">{S.vpnTitle}</h1>
      <p style="margin: 0; font-size: 15px; line-height: 1.45; color: #5E5B56; text-align: center;">{S.vpnSub}</p>
    </div>
    <div style="width: 100%; min-height: 56px; padding: 14px 18px; box-sizing: border-box; display: flex; align-items: center; justify-content: center; background: #FFFFFF; border: 1px solid #CFCBC5; border-radius: 12px; font-size: 16px; font-weight: 600; line-height: 1.3; text-align: center; color: #1D1C1A;">{S.vpnTip}</div>
  </div>
  <div style="display: flex; flex-direction: column; align-items: center; gap: 10px;">
    <button type="button" class="btn" onclick={() => store.goBack('home')}>{S.vpnBack}</button>
  </div>
</div>
