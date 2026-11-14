<script lang="ts">
  import Art from '../components/Art.svelte';
  import { S } from '../lib/strings';
  import { store } from '../lib/store.svelte';
</script>

<div style="flex: 1; display: flex; flex-direction: column; padding: 84px 28px 28px 28px;">
  <div style="display: flex; flex-direction: column; gap: 12px;">
    <h1 style="font-size: 36px; line-height: 1.12; max-width: 360px;">{S.welcomeTitle}</h1>
    <p style="margin: 0; font-size: 15px; line-height: 1.45; color: #3F3C38;">{S.welcomeSub}</p>
  </div>
  <div style="margin-top: 28px;">
    <button
      type="button"
      style="height: 46px; padding: 0 26px; border: 0; border-radius: 10px; background: #1D1C1A; color: #FFFFFF; font-size: 15px; font-weight: 600;"
      onclick={() => store.go('perms')}>{S.getStarted}</button>
  </div>
  <div style="margin-top: auto; margin-left: -14px; margin-right: -14px;">
    <Art name="welcome" />
  </div>
</div>
