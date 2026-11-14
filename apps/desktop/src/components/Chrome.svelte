<script lang="ts">
  import type { Snippet } from 'svelte';
  import { getApi } from '../lib/ipc';
  import { S } from '../lib/strings';
  import { stepOf, store } from '../lib/store.svelte';

  let { children }: { children: Snippet } = $props();
  const step = $derived(stepOf(store.route));

  async function act(a: 'minimize' | 'toggleMaximize' | 'close') {
    const api = await getApi();
    await api.window[a]();
  }
  const btn =
    'width: 32px; height: 28px; border: 0; border-radius: 6px; background: transparent; color: #5E5B56; display: flex; align-items: center; justify-content: center; padding: 0; cursor: pointer;';
</script>

<div class="card" data-testid="card">
  <div class="drag" data-tauri-drag-region></div>
  <div style="position: absolute; top: 8px; right: 8px; z-index: 1; display: flex; gap: 2px;">
    <button type="button" class="wc" aria-label={S.minimize} style={btn} onclick={() => act('minimize')}>
      <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1" aria-hidden="true"><path d="M0 5h10"></path></svg>
    </button>
    <button type="button" class="wc" aria-label={S.maximize} style={btn} onclick={() => act('toggleMaximize')}>
      <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1" aria-hidden="true"><rect x="0.5" y="0.5" width="9" height="9"></rect></svg>
    </button>
    <button type="button" class="wc wcx" aria-label={S.close} style={btn} onclick={() => act('close')}>
      <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1" aria-hidden="true"><path d="M0 0l10 10M10 0L0 10"></path></svg>
    </button>
  </div>
  {#if step > 0}
    <div
      role="img"
      aria-label={`Step ${step} of 5`}
      style="position: absolute; top: 19px; left: 0; right: 0; z-index: 1; display: flex; justify-content: center; gap: 5px; pointer-events: none;"
    >
      {#each [1, 2, 3, 4, 5] as i}
        <span
          style={`display: block; height: 6px; border-radius: 3px; width: ${i === step ? 22 : 8}px; background: ${i <= step ? '#1D1C1A' : '#BDB8B1'};`}
        ></span>
      {/each}
    </div>
  {/if}
  <div class="body">
    {@render children()}
  </div>
</div>
