<script lang="ts">
  // Home (2026-11-14, Max): H3 "You are here" from docs/design/desktop/motion/home.html. The
  // other devices are tiles; THIS device is a dark bar docked at the bottom with the last copy
  // as plain facts (never content). Motion: H3 for the build-in and device changes, A3
  // "Ticker" for a copy that moves (the line under the headline rolls to it and back), A4
  // "Hush" for Pause and Resume (a slow wave from the button across the tiles).
  import { onMount } from 'svelte';
  import Logo from '../components/Logo.svelte';
  import * as art from '../lib/art';
  import { failureText } from '../lib/ipc';
  import { anim, canAnimate, IN, IO, OUT, reduced, rollIn, rollOut, track } from '../lib/motion';
  import { copyFact, S, thisLabel } from '../lib/strings';
  import { store } from '../lib/store.svelte';
  import type { Activity, Device } from '../lib/types';
  import { cardCloses, cardOpens } from '../lib/update-motion';

  const paused = $derived(store.status?.state === 'paused');
  const me = $derived(store.thisDevice);
  const others = $derived(store.devices.filter((d) => !d.this));
  let error = $state<string | null>(null);
  let now = $state(Date.now());

  const count = $derived.by(() => {
    const s = store.status;
    if (!s) return '';
    if (paused) return `Copies stay on ${(me ? thisLabel(me.os) : 'This computer').replace('This', 'this')}`;
    if (s.connected === 0) return s.text;
    const n = s.connected + 1;
    return `${n} ${n === 1 ? 'device' : 'devices'} connected`;
  });

  // The last copy that moved, as facts.
  const last = $derived((store.status?.activity ?? []).find((a) => a.direction === 'sent' || a.direction === 'received') ?? null);
  const fact = $derived(last ? copyFact(last, now) : null);

  // A3 Ticker: after a copy moves, the line under the headline shows it for a moment.
  let tick = $state<{ text: string; dir: string; at: number } | null>(null);
  const line = $derived(tick ? tick.text : count);
  const icon = $derived(tick ? tick.dir : 'dot');
  let tickTimer: ReturnType<typeof setTimeout> | undefined;

  // Delays for the rolling texts: a pause change rolls 200 ms after the press (A4), a new
  // copy rolls the bar's fact after the light line has crossed (H3).
  let rollDelay = 0;
  let factDelay = 0;
  const fDelay = () => factDelay;
  /** When Pause or Resume was pressed: tile changes soon after it ride the A4 wave. */
  let waveAt = -1e9;
  const inWave = () => performance.now() - waveAt < 1500;

  // A4: each tile changes after a lag that grows with its distance from the Pause button.
  let pauseBtn = $state<HTMLElement | null>(null);
  let grid = $state<HTMLElement | null>(null);
  let lags = $state<Record<string, number>>({});
  const D = () => (reduced() ? 400 : 620);
  function measure() {
    if (!pauseBtn || !grid) return;
    const b = pauseBtn.getBoundingClientRect();
    const bx = b.left + b.width / 2;
    const by = b.top + b.height / 2;
    const els = Array.from(grid.querySelectorAll<HTMLElement>('[data-wave]'));
    const dist = els.map((t) => {
      const r = t.getBoundingClientRect();
      return Math.hypot(r.left + r.width / 2 - bx, r.top + r.height / 2 - by);
    });
    const min = Math.min(...dist);
    const out: Record<string, number> = {};
    els.forEach((t, i) => (out[t.dataset.wave!] = reduced() ? 0 : Math.round((dist[i] - min) * 1.25)));
    lags = out;
  }
  $effect(() => {
    void others.length;
    if (typeof requestAnimationFrame === 'function') requestAnimationFrame(measure);
  });
  const waveVars = (key: string) => {
    const d = 350 + (lags[key] ?? 0);
    return `--d: ${d}ms; --dd: ${paused ? d : d + 160}ms; --dur: ${D()}ms; --dot-ease: ${paused ? 'var(--ease-in-out)' : 'var(--ease-out)'};`;
  };

  function tileText(d: Device): { text: string; dot: string } {
    if (paused && d.state === 'Connected') return { text: S.tilePaused, dot: 'background: #B3AEA7;' };
    if (d.state === 'Connected') return { text: S.tileConnected, dot: 'background: #1E7A46;' };
    if (d.state === 'Blocked by a VPN or firewall') return { text: S.tileBlocked, dot: 'background: #B42318;' };
    return { text: S.tileAsleep, dot: 'background: rgba(30, 122, 70, 0); box-shadow: inset 0 0 0 1.5px #6E6A64;' };
  }

  // H3: a device that comes on gets its dot popped in and a soft halo, once.
  let prev = new Map<string, string>();
  $effect(() => {
    const next = new Map(others.map((d) => [d.id, d.state] as [string, string]));
    if (canAnimate() && !reduced() && grid) {
      for (const d of others) {
        const was = prev.get(d.id);
        if (was && was !== 'Connected' && d.state === 'Connected') {
          const tile = grid.querySelector(`[data-wave="${CSS.escape(d.id)}"]`);
          anim(tile?.querySelector('.dot'), [{ transform: 'scale(0.2)' }, { transform: 'scale(1)' }], { duration: 560, delay: 220, easing: OUT });
          anim(tile?.querySelector('.halo'), [{ opacity: 0.6, transform: 'scale(1)' }, { opacity: 0, transform: 'scale(3.4)' }], {
            duration: 1400,
            delay: 320,
            easing: 'cubic-bezier(0.2, 0.6, 0.3, 1)',
          });
        }
      }
    }
    prev = next;
  });

  // A4 Resume: the grey dots shrink a little, then fill green again, along the wave.
  let wasPaused: boolean | null = null;
  $effect(() => {
    const p = paused;
    if (wasPaused === true && !p && grid && !reduced()) {
      for (const t of Array.from(grid.querySelectorAll<HTMLElement>('[data-wave]'))) {
        const dot = t.querySelector('.dot');
        if (!dot || !t.dataset.connected) continue;
        const r = 350 + (lags[t.dataset.wave!] ?? 0);
        track(dot, [[0, { transform: 'scale(1)' }], [r, { transform: 'scale(1)' }, IN], [r + 160, { transform: 'scale(0.55)' }, OUT], [r + 160 + D(), { transform: 'scale(1)' }]], 'none');
      }
    }
    wasPaused = p;
  });

  let hereLine = $state<HTMLElement | null>(null);
  let hereRing = $state<HTMLElement | null>(null);

  function onCopy(a: Activity) {
    if (a.direction !== 'sent' && a.direction !== 'received') return;
    if (paused) return;
    rollDelay = 0;
    tick = { text: copyFact(a, Date.now()), dir: a.direction, at: Date.now() };
    clearTimeout(tickTimer);
    tickTimer = setTimeout(() => {
      rollDelay = 0;
      tick = null;
    }, 2800);
    if (!canAnimate()) return;
    const r = reduced();
    factDelay = 1000;
    setTimeout(() => (factDelay = 0), 1200);
    // H3: a light line runs across this device's bar, then its logo rings once.
    if (r) {
      anim(hereLine, [{ opacity: 0 }, { opacity: 1, offset: 0.3 }, { opacity: 0 }], { duration: 1600, delay: 500, easing: 'ease-in-out', fill: 'none' });
      anim(hereRing, [{ opacity: 0 }, { opacity: 0.9, offset: 0.3 }, { opacity: 0 }], { duration: 1500, delay: 1100, easing: 'ease-in-out', fill: 'none' });
    } else {
      anim(hereLine, [{ opacity: 1, transform: 'scaleX(0)' }, { opacity: 1, transform: 'scaleX(1)', offset: 0.55 }, { opacity: 0, transform: 'scaleX(1)' }], {
        duration: 1700,
        delay: 500,
        easing: IO,
      });
      anim(hereRing, [{ opacity: 0.9, transform: 'scale(1)' }, { opacity: 0, transform: 'scale(1.3)' }], { duration: 1300, delay: 1100, easing: 'cubic-bezier(0.2, 0.6, 0.3, 1)', fill: 'none' });
    }
    // A3: the sending device's edge turns green for a moment.
    if (a.direction === 'received' && a.device && grid) {
      const tile = Array.from(grid.querySelectorAll<HTMLElement>('[data-device]')).find((t) => t.dataset.device === a.device);
      const L = { borderColor: '#D9D6D1' };
      const G = { borderColor: 'rgba(30, 122, 70, 0.6)' };
      track(tile, [[0, L], [100, L, OUT], [600, G], [2000, G, IO], [2800, L]], 'none');
    }
  }

  onMount(() => {
    const t = setInterval(() => (now = Date.now()), 30_000);
    const off = store.onEvent((e) => {
      if (e.event === 'activity') onCopy(e.data as unknown as Activity);
    });
    return () => {
      clearInterval(t);
      clearTimeout(tickTimer);
      off();
    };
  });

  async function toggle() {
    error = null;
    // A4: the button presses in; the headline rolls 200 ms later; the wave follows.
    if (!reduced()) {
      track(pauseBtn, [[0, { transform: 'scale(1)' }, OUT], [120, { transform: 'scale(0.92)' }, OUT], [420, { transform: 'scale(1)' }]], 'none');
    }
    rollDelay = 200;
    waveAt = performance.now();
    clearTimeout(tickTimer);
    tick = null;
    try {
      await store.api!.call('pause.set', { mode: paused ? 'off' : 'forever' });
      await store.refresh();
    } catch (e) {
      error = failureText(e);
    }
    setTimeout(() => (rollDelay = 0), 800);
  }

  // U3 "Home card" (Max's pick 2026-11-14, docs/design/desktop/motion/update.html): a ready or
  // failed update shows as a card at the top that pushes the tiles down. Update now (or Try
  // again) goes on to the update screen (U1, the shape grows from the button); Later closes it.
  const card = $derived(store.updateCard);
  const cardTitle = $derived(card === 'failed' ? S.updateFailed : S.updateReady(store.update?.available ?? ''));
  const cardSub = $derived(card === 'failed' ? S.updateFailedSub(store.update?.current ?? '') : S.updateReadySub);
  // The card opens after Home builds in, or at once when the news comes while Home shows.
  const navKind = store.nav.kind;
  const mountedAt = typeof performance !== 'undefined' ? performance.now() : 0;
  function cardIn(node: HTMLElement) {
    if (!canAnimate()) return;
    const fresh = performance.now() - mountedAt < 100;
    if (fresh && navKind === 'none') return;
    const t = !fresh ? 0 : navKind === 'boot' ? 700 : navKind === 'back' ? 450 : 500;
    cardOpens(node, grid, t);
  }
  function cardOut(node: Element) {
    if (!canAnimate()) return { duration: 0 };
    return { duration: cardCloses(node as HTMLElement) };
  }
  async function cardLater() {
    try {
      await store.api!.call('update.later');
    } catch {
      /* the agent may be gone */
    }
    await store.refreshUpdate();
  }

  function open(d: Device) {
    store.selected = d.id;
    store.go('device');
  }
  const hdrBtn =
    'width: 40px; height: 40px; flex: none; border-radius: 10px; display: flex; align-items: center; justify-content: center; padding: 0;';
  const tileBtn =
    'aspect-ratio: 1 / 1; background: #FFFFFF; border: 1px solid #D9D6D1; border-radius: 12px; padding: 12px; display: flex; flex-direction: column; justify-content: center; align-items: center; gap: 10px; font-family: inherit; color: #1D1C1A;';
  const roll = (o: { dist?: string; d?: number }) => ({ ...o, delay: rollDelay });
</script>

<div style="flex: 1; display: flex; flex-direction: column; padding: 44px 20px 20px 20px; gap: 16px; min-height: 0;">
  <div data-m="hdr" style="background: #FFFFFF; border: 1px solid #D9D6D1; border-radius: 12px; padding: 16px 14px 16px 18px; display: flex; align-items: center; gap: 8px;">
    <div role="status" style="flex: 1; display: flex; flex-direction: column; gap: 6px; min-width: 0;">
      <h1 data-testid="headline" style="font-size: 26px; line-height: 1.15;">
        <span class="stack clip">
          {#key paused}<span in:rollIn={roll({})} out:rollOut={roll({})}>{paused ? S.paused : S.working}</span>{/key}
        </span>
      </h1>
      <div style="display: flex; align-items: center; gap: 8px; font-size: 14px; color: #5E5B56;">
        <span class="tk-ico">
          {#key icon}
            <span class="pop" in:rollIn={{ pop: true, d: 420 }} out:rollOut={{ pop: true, d: 420 }}>
              {#if icon === 'sent'}
                <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="#1D1C1A" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M7 17L17 7M9 7h8v8"></path></svg>
              {:else if icon === 'received'}
                <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="#1E7A46" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M17 7L7 17M15 17H7V9"></path></svg>
              {:else}
                <span class="dot hdot" class:off={paused}></span>
              {/if}
            </span>
          {/key}
        </span>
        <span data-testid="subtitle" class="stack clip tk">
          {#key line}<span in:rollIn={roll({})} out:rollOut={roll({})}>{line}</span>{/key}
        </span>
      </div>
    </div>
    <button
      bind:this={pauseBtn}
      type="button"
      class="pz"
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
  {#if card}
    <div class="ucard" data-m="card" data-testid="update-card" data-state={card} use:cardIn out:cardOut>
      <div class="uc-top">
        {#if card === 'failed'}
          <svg class="ub" width="40" height="40" viewBox="0 0 40 40" fill="none" aria-hidden="true">
            <circle cx="20" cy="20" r="18" fill="#FFFFFF" stroke="#B42318" stroke-width="2.5"></circle>
            <path class="undo" pathLength="1" d="M12 17l5-5M12 17l5 5M12 17h12a6 6 0 0 1 0 12h-6" stroke="#B42318" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"></path>
          </svg>
        {:else}
          <svg class="ub" width="40" height="40" viewBox="0 0 40 40" fill="none" aria-hidden="true">
            <circle cx="20" cy="20" r="18" fill="#FFFFFF" stroke="#1D1C1A" stroke-width="2"></circle>
            <path d="M20 11v17M12.5 21l7.5 7.5 7.5-7.5" stroke="#1D1C1A" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"></path>
          </svg>
        {/if}
        <div class="uc-txt">
          <p class="uc-t">{cardTitle}</p>
          <p class="uc-s">{cardSub}</p>
        </div>
      </div>
      <div class="uc-acts">
        {#if card === 'available' || store.update?.available}
          <button type="button" class="btn sm" onclick={(e) => store.startUpdate(e.currentTarget)}>
            <span class="lbl">{card === 'failed' ? S.updateTryAgain : S.updateNow}</span>
          </button>
        {/if}
        <button type="button" class="btn sm ghost" onclick={cardLater}>{S.updateLater}</button>
      </div>
    </div>
  {/if}
  <p class="sr" role="status">{card ? `${cardTitle} ${cardSub}` : ''}</p>
  <div bind:this={grid} data-m="tiles" style="display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 10px; overflow: auto; min-height: 0; flex: 0 1 auto;">
    {#each others as d (d.id)}
      {@const t = tileText(d)}
      {@const asleep = d.state !== 'Connected' && d.state !== 'Blocked by a VPN or firewall'}
      <button
        type="button"
        class="tile wave"
        data-device={d.name}
        data-wave={d.id}
        data-connected={d.state === 'Connected' ? '1' : undefined}
        style={`${tileBtn} ${waveVars(d.id)}`}
        onclick={() => open(d)}
      >
        <span class="g" style={`display: flex; opacity: ${asleep ? 0.45 : 1};`}><Logo os={d.os} fill={paused ? '#B3AEA7' : '#1D1C1A'} /></span>
        <span style="display: flex; flex-direction: column; align-items: center; gap: 3px;">
          <span class="nm" style={`font-size: 13.5px; font-weight: 600; line-height: 1.25; color: ${paused ? '#6E6A64' : '#1D1C1A'};`}>{d.name}</span>
          <span class="st" style={`display: flex; align-items: center; gap: 6px; font-size: 12px; color: ${paused ? '#6E6A64' : '#5E5B56'}; justify-content: center;`}>
            <span class="dw"><span class="dot" style={t.dot}></span><span class="halo"></span></span>
            <span class="stack clip">
              {#key t.text}
                <span
                  in:rollIn={inWave() ? { d: 460, delay: 350 + (lags[d.id] ?? 0) } : { dist: '6px', d: 520 }}
                  out:rollOut={inWave() ? { d: 460, delay: 350 + (lags[d.id] ?? 0) } : { dist: '6px', d: 440 }}>{t.text}</span>
              {/key}
            </span>
          </span>
        </span>
      </button>
    {/each}
    {#if store.devices.length < 8}
      <button
        type="button"
        class="wave add"
        data-testid="add-tile"
        data-wave="add"
        style={`aspect-ratio: 1 / 1; background: transparent; border: 1.5px dashed ${paused ? '#B3AEA7' : '#8A857E'}; border-radius: 12px; padding: 14px; display: flex; flex-direction: column; justify-content: center; align-items: center; gap: 10px; font-family: inherit; color: ${paused ? '#6E6A64' : '#1D1C1A'}; ${waveVars('add')}`}
        onclick={() => store.go('add')}
      >
        <svg width="30" height="30" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true">{@html art.plus}</svg>
        <span style="font-size: 13.5px; font-weight: 600;">{S.addDevice}</span>
      </button>
    {/if}
  </div>
  {#if me}
    <button type="button" class="here" data-m="here" data-device={me.name} onclick={() => open(me)}>
      <span class="here-line" bind:this={hereLine}></span>
      <span class="here-g"><Logo os={me.os} size={26} fill="#FFFFFF" /><span class="here-ring" bind:this={hereRing}></span></span>
      <span class="here-t"><b>{me.name}</b><span>{thisLabel(me.os)}</span></span>
      {#if fact && last}
        <span class="here-f">
          <span class="here-l">{S.lastCopy}</span>
          <span class="stack end" data-testid="last-copy">
            {#key last.time_ms}
              <span in:rollIn={{ dist: '10px', d: 520, delay: fDelay() }} out:rollOut={{ dist: '10px', d: 440, delay: fDelay() }}>{fact}</span>
            {/key}
          </span>
        </span>
      {/if}
    </button>
  {/if}
</div>

<style>
  /* Rolling text: old and new copies sit in one grid cell; .clip hides them outside the line. */
  /* Long names never push the text out of its box: the line ends in an ellipsis instead. */
  .stack { display: inline-grid; grid-template-columns: minmax(0, max-content); min-width: 0; max-width: 100%; }
  .stack > :global(*) { grid-area: 1 / 1; white-space: nowrap; min-width: 0; max-width: 100%; overflow: hidden; text-overflow: ellipsis; }
  .stack.clip { overflow: hidden; padding: 2px 0; margin: -2px 0; }
  .stack.end { justify-items: end; }
  .tile .stack { justify-items: center; }

  /* A3: the icon before the line under the headline (dot, or an arrow while a copy moves). */
  .tk-ico { position: relative; width: 15px; height: 15px; flex: none; display: inline-grid; place-items: center; margin: 0 -3px; }
  .tk-ico > :global(*) { grid-area: 1 / 1; display: flex; }
  .hdot {
    background: #1E7A46; box-shadow: inset 0 0 0 1.5px rgba(110, 106, 100, 0);
    transition: background-color 500ms var(--ease-in-out) 200ms, box-shadow 500ms var(--ease-in-out) 200ms;
  }
  .hdot.off { background: rgba(30, 122, 70, 0); box-shadow: inset 0 0 0 1.5px rgba(110, 106, 100, 1); }

  /* A4: colours follow the wave; each tile sets --d (its delay) and --dur. */
  .pz { transition: background-color 280ms var(--ease-out) 60ms, color 280ms var(--ease-out) 60ms, border-color 280ms var(--ease-out) 60ms; }
  .wave :global(svg) { transition: fill var(--dur) var(--ease-in-out) var(--d); }
  .wave .nm, .wave .st { transition: color var(--dur) var(--ease-in-out) var(--d); }
  .wave .dot { transition: background-color var(--dur) var(--dot-ease) var(--dd), box-shadow var(--dur) var(--dot-ease) var(--dd); }
  .wave.add { transition: border-color var(--dur) var(--ease-in-out) var(--d), color var(--dur) var(--ease-in-out) var(--d); }
  .g { transition: opacity 900ms var(--ease-in-out); }
  .dw { position: relative; display: inline-flex; }
  .halo { position: absolute; inset: 0; border-radius: 50%; border: 1.5px solid #1E7A46; box-sizing: border-box; opacity: 0; pointer-events: none; }

  /* H3: this device, docked at the bottom as a dark bar. */
  .here {
    margin-top: auto; position: relative; height: 76px; flex: none; box-sizing: border-box; border: 0; border-radius: 12px;
    background: #1D1C1A; color: #FFFFFF; display: flex; align-items: center; gap: 14px; padding: 0 18px 0 16px;
    box-shadow: 0 1px 2px rgba(40,35,30,0.10), 0 8px 20px rgba(40,35,30,0.16); overflow: hidden; text-align: left; width: 100%;
  }
  .here-line {
    position: absolute; left: 0; right: 0; top: 0; height: 2px; background: rgba(255,255,255,0.75);
    opacity: 0; transform-origin: left center;
  }
  .here-g {
    position: relative; width: 44px; height: 44px; border-radius: 10px; background: rgba(255,255,255,0.08);
    display: flex; align-items: center; justify-content: center; flex: none;
  }
  .here-ring { position: absolute; inset: -4px; border-radius: 13px; border: 1.5px solid rgba(255,255,255,0.6); opacity: 0; }
  .here-t { display: flex; flex-direction: column; gap: 3px; flex: none; min-width: 0; max-width: 150px; }
  .here-t b { font-size: 16px; font-weight: 700; letter-spacing: 0.01em; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .here-t span { font-size: 13px; color: #CFCBC5; }
  .here-f { flex: 1; min-width: 0; display: flex; flex-direction: column; align-items: flex-end; gap: 3px; font-size: 13px; color: #E9E6E1; text-align: right; }
  .here-l { color: #B3AEA7; font-size: 12.5px; }
  .here:focus-visible { outline: 2px solid #1D1C1A; outline-offset: 2px; }

  /* U3: the update card at the top (sizes from the mockup's .ucard). */
  .ucard {
    background: #FFFFFF; border: 1px solid #D9D6D1; border-radius: 12px; padding: 16px; display: flex;
    flex-direction: column; gap: 14px; flex: none; position: relative; z-index: 1; box-sizing: border-box; overflow: hidden;
  }
  .uc-top { display: flex; gap: 14px; align-items: center; }
  .ub { flex: none; display: block; overflow: visible; }
  .ub .undo { stroke-dasharray: 1 1; stroke-dashoffset: 0; }
  .uc-txt { display: flex; flex-direction: column; gap: 3px; min-width: 0; }
  .uc-t { margin: 0; font-size: 17px; font-weight: 700; line-height: 1.3; letter-spacing: -0.01em; }
  .uc-s { margin: 0; font-size: 14px; line-height: 1.4; color: #5E5B56; }
  .uc-acts { display: flex; gap: 10px; padding-left: 54px; }
  .btn.sm { height: 38px; padding: 0 16px; font-size: 14px; width: auto; box-sizing: border-box; }
  /* With the card, the tiles can scroll under it; no scrollbar strip on any OS (it would squeeze
     the tiles on Windows and Linux, and the designs have none). The cut tile shows there is more. */
  [data-m='tiles'] { position: relative; z-index: 0; scrollbar-width: none; }
  .sr { position: absolute; width: 1px; height: 1px; margin: -1px; padding: 0; overflow: hidden; clip-path: inset(50%); white-space: nowrap; border: 0; }
</style>
