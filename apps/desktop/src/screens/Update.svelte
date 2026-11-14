<script lang="ts">
  // Update screen: U1 "One shape" (Max's pick 2026-11-14, docs/design/desktop/motion/update.html).
  // The agent opens the window here by itself once when it finds a newer version at start, and
  // again after the restart to show how the update went. Update now: the black button grows
  // into a round dial and a ring fills while the helper works ("Updating..."). The app closes
  // and opens again; the dial is then the green check ("Updated to X", Done), or a red circle
  // with a back arrow and one damped shake ("The update did not work.", Try again, Later).
  // Update now on Home's card (U3) lands here in the updating phase (src/lib/flow.ts); when that
  // try fails, the window goes back to Home and the card shows it.
  // Phase changes inside the screen: the old phase leaves, the new one arrives
  // (src/lib/update-motion.ts), focus moves to the new heading, the status line below speaks.
  import { onMount } from 'svelte';
  import { shapeBetween } from '../lib/flow';
  import { canAnimate } from '../lib/motion';
  import { S } from '../lib/strings';
  import { store } from '../lib/store.svelte';
  import { endIn, fadeOut, offerIn, offerOut, updatingIn } from '../lib/update-motion';

  const u = $derived(store.update);
  // An install asked for here (or on Home's card) and not answered yet.
  let pressed = $state(store.updateStart);
  store.updateStart = false;
  // The button that was pressed: the shape grows out of it.
  let from: Element | null = null;

  type Phase = 'offer' | 'updating' | 'ok' | 'fail' | 'none';
  const phase = $derived.by<Phase>(() => {
    if (pressed) return 'updating';
    switch (u?.state) {
      case 'available':
        return 'offer';
      case 'applying':
        return 'updating';
      case 'updated':
        return 'ok';
      case 'failed':
        return 'fail';
      default:
        return 'none';
    }
  });
  const canRetry = $derived(!!u?.available);
  const heading = $derived.by(() => {
    switch (phase) {
      case 'offer':
        return S.updateReady(u?.available ?? '');
      case 'updating':
        return S.updating;
      case 'ok':
        return S.updated(u?.current ?? '');
      case 'fail':
        return S.updateFailed;
      default:
        return S.updateNone(u?.current ?? '');
    }
  });
  const sub = $derived.by(() => {
    switch (phase) {
      case 'offer':
        return S.updateReadySub;
      case 'updating':
        return S.updatingSub;
      case 'ok':
        return S.updatedSub;
      case 'fail':
        return S.updateFailedSub(u?.current ?? '');
      default:
        return '';
    }
  });

  async function install(e?: Event) {
    from = (e?.currentTarget as Element | undefined) ?? null;
    pressed = true;
    try {
      await store.api!.call('update.apply');
    } catch {
      /* the agent says "failed" (event and status below) */
    }
    await store.refreshUpdate();
    pressed = false;
  }

  // From Home's card: the install starts as this screen arrives. A failure goes back to Home.
  onMount(() => {
    if (pressed) void install();
  });
  $effect(() => {
    if (phase === 'fail' && store.updateFrom === 'home') {
      store.updateFrom = null;
      store.goBack('home');
    }
  });

  async function later() {
    try {
      await store.api!.call('update.later');
    } catch {
      /* the agent may be gone; leaving the screen is all that matters */
    }
    store.leaveUpdate();
  }

  // The first phase arrives with the screen (src/lib/flow.ts); later ones arrive here.
  let first = true;
  function phaseIn(node: HTMLElement, ph: Phase) {
    if (first) {
      first = false;
      return;
    }
    setTimeout(() => {
      if (!node.isConnected || node.contains(document.activeElement)) return;
      const h = node.querySelector<HTMLElement>('h1');
      h?.focus({ preventScroll: true });
    }, 0);
    if (!canAnimate()) return;
    if (ph === 'updating') {
      const core = node.querySelector('.core');
      if (from && from.isConnected && core) shapeBetween(from, core, 150, 900);
      updatingIn(node, 900);
    } else if (ph === 'offer') {
      offerIn(node);
    } else {
      endIn(node, 0);
    }
  }
  function phaseOut(node: Element) {
    if (!canAnimate()) return { duration: 0 };
    node.setAttribute('aria-hidden', 'true');
    (node as HTMLElement).inert = true;
    const ph = (node as HTMLElement).dataset.phase;
    const d = ph === 'offer' || ph === 'fail' ? offerOut(node, from && node.contains(from) ? from : null) : fadeOut(node);
    return { duration: d };
  }
</script>

<div class="u" data-testid="update">
  {#key phase}
    <div class="u-scr" class:u-up={phase === 'updating'} data-phase={phase} use:phaseIn={phase} out:phaseOut>
      <div class="u-art">
        <div class="dial" data-m="art">
          {#if phase === 'updating'}
            <svg class="ring" width="140" height="140" viewBox="0 0 140 140" fill="none" aria-hidden="true">
              <circle class="trk" cx="70" cy="70" r="65" stroke="#D9D6D1" stroke-width="3"></circle>
              <circle class="prog" pathLength="1" cx="70" cy="70" r="65" stroke="#1D1C1A" stroke-width="3" stroke-linecap="round" transform="rotate(-90 70 70)"></circle>
            </svg>
            <span class="core" data-shape></span>
            <svg class="arw" width="140" height="140" viewBox="0 0 140 140" fill="none" aria-hidden="true">
              <path d="M70 49v40M55 75l15 15 15-15" stroke="#FFFFFF" stroke-width="3.5" stroke-linecap="round" stroke-linejoin="round"></path>
            </svg>
          {:else if phase === 'ok'}
            <svg width="140" height="140" viewBox="0 0 140 140" fill="none" aria-hidden="true">
              <circle cx="70" cy="70" r="54" fill="#FFFFFF" stroke="#1E7A46" stroke-width="3"></circle>
              <path class="tick" pathLength="1" d="M48 71l15 15 31-31" stroke="#1E7A46" stroke-width="5" stroke-linecap="round" stroke-linejoin="round"></path>
            </svg>
          {:else if phase === 'fail'}
            <svg class="shk" width="140" height="140" viewBox="0 0 140 140" fill="none" aria-hidden="true">
              <circle cx="70" cy="70" r="54" fill="#FFFFFF" stroke="#B42318" stroke-width="3"></circle>
              <path class="undo" pathLength="1" d="M50 62l12-12M50 62l12 12M50 62h28a14 14 0 0 1 0 28H64" stroke="#B42318" stroke-width="4.5" stroke-linecap="round" stroke-linejoin="round" fill="none"></path>
            </svg>
          {:else}
            <svg width="140" height="140" viewBox="0 0 140 140" fill="none" aria-hidden="true">
              <circle cx="70" cy="70" r="54" fill="#FFFFFF" stroke="#1D1C1A" stroke-width="2"></circle>
              <path d="M70 49v40M55 75l15 15 15-15" stroke="#1D1C1A" stroke-width="3.5" stroke-linecap="round" stroke-linejoin="round"></path>
            </svg>
          {/if}
        </div>
      </div>
      <div class="u-txt" data-m="text">
        <h1 tabindex="-1">{heading}</h1>
        {#if sub}<p class="sub">{sub}</p>{/if}
        {#if phase === 'offer'}<p class="small">{S.updateHave(u?.current ?? '')}</p>{/if}
      </div>
      {#if phase !== 'updating'}
        <div class="u-acts">
          {#if phase === 'offer'}
            <button type="button" class="btn u-go" onclick={install}><span class="lbl">{S.updateNow}</span></button>
            <button type="button" class="btn ghost u-later" onclick={later}>{S.updateLater}</button>
          {:else if phase === 'fail' && canRetry}
            <button type="button" class="btn u-go" onclick={install}><span class="lbl">{S.updateTryAgain}</span></button>
            <button type="button" class="btn ghost u-later" onclick={later}>{S.updateLater}</button>
          {:else if phase === 'ok'}
            <button type="button" class="btn u-go" onclick={later}><span class="lbl">{S.updateDone}</span></button>
          {:else}
            <button type="button" class="btn u-go" onclick={later}><span class="lbl">{S.updateOk}</span></button>
          {/if}
        </div>
      {/if}
    </div>
  {/key}
  <p class="sr" role="status">{heading} {sub}</p>
</div>

<style>
  .u { flex: 1; position: relative; }
  /* Same top on every phase, so nothing jumps (the mockup's .u-scr). */
  .u-scr {
    position: absolute; inset: 0; box-sizing: border-box; display: flex; flex-direction: column;
    align-items: center; padding: 72px 28px 28px 28px;
  }
  /* The updating phase sits above the black shape, like the mockup's top layer. */
  .u-up { z-index: 6; }
  .u-art { height: 160px; width: 100%; flex: none; display: flex; align-items: center; justify-content: center; }
  .dial { position: relative; width: 140px; height: 140px; }
  .dial > svg { position: absolute; inset: 0; display: block; overflow: visible; }
  .core { position: absolute; left: 16px; top: 16px; width: 108px; height: 108px; border-radius: 50%; background: #1D1C1A; }
  .prog, .tick, .undo { stroke-dasharray: 1 1; stroke-dashoffset: 0; }
  /* The ring rests most of the way full while the helper works (update-motion.ts). */
  .prog { stroke-dashoffset: 0.2; }
  .shk { transform-box: fill-box; }
  .u-txt { margin-top: 28px; display: flex; flex-direction: column; gap: 8px; align-items: center; text-align: center; max-width: 380px; }
  h1 { font-size: 28px; line-height: 1.15; text-wrap: balance; }
  .sub { margin: 0; font-size: 15px; line-height: 1.45; color: #3F3C38; text-wrap: balance; }
  .small { margin: 2px 0 0 0; font-size: 14px; line-height: 1.45; color: #5E5B56; }
  .u-acts { margin-top: auto; width: 100%; display: flex; flex-direction: column; gap: 10px; }
  .sr { position: absolute; width: 1px; height: 1px; margin: -1px; padding: 0; overflow: hidden; clip-path: inset(50%); white-space: nowrap; border: 0; }
</style>
