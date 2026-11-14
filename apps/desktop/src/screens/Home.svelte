<script lang="ts">
  import Logo from '../components/Logo.svelte';
  import * as art from '../lib/art';
  import { failureText } from '../lib/ipc';
  import { S, thisLabel } from '../lib/strings';
  import { store } from '../lib/store.svelte';
  import type { Device } from '../lib/types';

  const paused = $derived(store.status?.state === 'paused');
  const me = $derived(store.thisDevice);
  let error = $state<string | null>(null);

  const subtitle = $derived.by(() => {
    const s = store.status;
    if (!s) return '';
    if (paused) return `Copies stay on ${(me ? thisLabel(me.os) : 'This computer').replace('This', 'this')}`;
    if (s.connected === 0) return s.text;
    const n = s.connected + 1;
    return `${n} ${n === 1 ? 'device' : 'devices'} connected`;
  });

  function tileText(d: Device): { text: string; dot: string } {
    if (d.this) return { text: thisLabel(d.os), dot: '' };
    if (paused && d.state === 'Connected') return { text: S.tilePaused, dot: 'background: #B3AEA7;' };
    if (d.state === 'Connected') return { text: S.tileConnected, dot: 'background: #1E7A46;' };
    if (d.state === 'Blocked by a VPN or firewall') return { text: S.tileBlocked, dot: 'background: #B42318;' };
    return { text: S.tileAsleep, dot: 'border: 1.5px solid #6E6A64;' };
  }

  async function toggle() {
    error = null;
    try {
      await store.api!.call('pause.set', { mode: paused ? 'off' : 'forever' });
      await store.refresh();
    } catch (e) {
      error = failureText(e);
    }
  }

  function open(d: Device) {
    store.selected = d.id;
    store.go('device');
  }
  const hdrBtn =
    'width: 40px; height: 40px; flex: none; border-radius: 10px; display: flex; align-items: center; justify-content: center; padding: 0;';
  const tileBtn =
    'aspect-ratio: 1 / 1; background: #FFFFFF; border: 1px solid #D9D6D1; border-radius: 12px; padding: 12px; display: flex; flex-direction: column; justify-content: center; align-items: center; gap: 10px; font-family: inherit; color: #1D1C1A;';
</script>

<div style="flex: 1; display: flex; flex-direction: column; padding: 44px 20px 22px 20px; gap: 16px; min-height: 0;">
  <div style="background: #FFFFFF; border: 1px solid #D9D6D1; border-radius: 12px; padding: 16px 14px 16px 18px; display: flex; align-items: center; gap: 8px;">
    <div style="flex: 1; display: flex; flex-direction: column; gap: 6px;">
      <h1 data-testid="headline" style="font-size: 26px; line-height: 1.15;">{paused ? S.paused : S.working}</h1>
      <div style="display: flex; align-items: center; gap: 8px; font-size: 14px; color: #5E5B56;">
        <span class="dot" style={paused ? 'border: 1.5px solid #6E6A64;' : 'background: #1E7A46;'}></span>
        <span data-testid="subtitle">{subtitle}</span>
      </div>
    </div>
    <button
      type="button"
      aria-label={paused ? S.resume : S.pause}
      aria-pressed={paused}
      style={`${hdrBtn} ${paused ? 'border: 1px solid #1D1C1A; background: #1D1C1A; color: #FFFFFF;' : 'border: 1px solid #BDB8B1; background: #FFFFFF; color: #1D1C1A;'}`}
      onclick={toggle}
    >
      <svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">{@html art.pause}</svg>
    </button>
    <button type="button" aria-label={S.settings} style={`${hdrBtn} border: 1px solid #BDB8B1; background: #FFFFFF; color: #1D1C1A;`} onclick={() => store.go('settings')}>
      <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{@html art.gear}</svg>
    </button>
  </div>
  {#if error}<div class="err" role="alert">{error}</div>{/if}
  <div style="display: grid; grid-template-columns: repeat(3, 1fr); gap: 10px; overflow: auto;">
    {#each store.devices as d (d.id)}
      {@const t = tileText(d)}
      <button type="button" class="tile" data-device={d.name} style={tileBtn} onclick={() => open(d)}>
        <Logo os={d.os} fill={paused ? '#B3AEA7' : '#1D1C1A'} />
        <span style="display: flex; flex-direction: column; align-items: center; gap: 3px;">
          <span style={`font-size: 13.5px; font-weight: 600; line-height: 1.25; ${paused ? 'color: #6E6A64;' : ''}`}>{d.name}</span>
          <span style={`display: flex; align-items: center; gap: 6px; font-size: 12px; color: ${paused ? '#6E6A64' : '#5E5B56'}; justify-content: center;`}>
            {#if t.dot}<span class="dot" style={t.dot}></span>{/if}{t.text}
          </span>
        </span>
      </button>
    {/each}
    {#if store.devices.length < 8}
      <button
        type="button"
        data-testid="add-tile"
        style={`aspect-ratio: 1 / 1; background: transparent; border: 1.5px dashed ${paused ? '#B3AEA7' : '#8A857E'}; border-radius: 12px; padding: 14px; display: flex; flex-direction: column; justify-content: center; align-items: center; gap: 10px; font-family: inherit; color: ${paused ? '#6E6A64' : '#1D1C1A'};`}
        onclick={() => store.go('add')}
      >
        <svg width="30" height="30" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true">{@html art.plus}</svg>
        <span style="font-size: 13.5px; font-weight: 600;">{S.addDevice}</span>
      </button>
    {/if}
  </div>
</div>
