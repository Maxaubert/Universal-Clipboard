<script lang="ts">
  import type { Snippet } from 'svelte';
  import { S } from '../lib/strings';
  let { onback, margin = '-8px', right }: { onback: () => void; margin?: string; right?: Snippet } = $props();
</script>

<div style={`height: 36px; flex: none; display: flex; align-items: center; justify-content: space-between; margin-left: ${margin};`}>
  <button type="button" class="iconbtn" aria-label={S.back} onclick={onback}>
    <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M15 5l-7 7 7 7"></path></svg>
  </button>
  {#if right}{@render right()}{/if}
</div>
