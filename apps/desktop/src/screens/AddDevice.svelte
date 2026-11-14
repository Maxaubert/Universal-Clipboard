<script lang="ts">
  import Art from '../components/Art.svelte';
  import * as art from '../lib/art';
  import { S } from '../lib/strings';
  import { store } from '../lib/store.svelte';
</script>

<div style="flex: 1; display: flex; flex-direction: column; padding: 40px 28px 28px 28px;">
  <div style="flex: 1; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 30px;">
    <div data-m="art"><Art name="add" /></div>
    <div data-m="text" style="display: flex; flex-direction: column; gap: 8px; align-items: center;">
      <h1 style="font-size: 28px; line-height: 1.15; text-align: center; color: #1D1C1A;">{S.addTitle}</h1>
      <p style="margin: 0; font-size: 15px; line-height: 1.45; color: #3F3C38; text-align: center;">{S.addSub}</p>
    </div>
  </div>
  <div data-m="cta" style="display: flex; flex-direction: column; gap: 10px;">
    <button type="button" class="btn" data-shape onclick={(e) => store.go('show', { from: e.currentTarget })}>
      <span class="lbl"><svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="#FFFFFF" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{@html art.iconShow}</svg>{S.showCode}</span>
    </button>
    <button type="button" class="btn ghost" style="background: #FFFFFF; color: #1D1C1A; border: 1px solid #BDB8B1;" onclick={(e) => store.go('enter', { from: e.currentTarget })}>
      <span class="lbl"><svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="#1D1C1A" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{@html art.iconEnter}</svg>{S.enterCode}</span>
    </button>
  </div>
</div>
