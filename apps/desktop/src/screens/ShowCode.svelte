<script lang="ts">
  import { onMount } from 'svelte';
  import BackBar from '../components/BackBar.svelte';
  import Qr from '../components/Qr.svelte';
  import * as art from '../lib/art';
  import { failureText } from '../lib/ipc';
  import { anim, IO, msgIn, reduced, shake } from '../lib/motion';
  import { S } from '../lib/strings';
  import { store } from '../lib/store.svelte';
  import type { PairOffer } from '../lib/types';

  let offer = $state<PairOffer | null>(null);
  let tried = $state<string | null>(null);
  /** Counts "someone tried" notices, so the same text rises in again. */
  let triedN = $state(0);
  let error = $state<string | null>(null);
  let codeEl = $state<HTMLElement | null>(null);
  let alive = true;

  async function start() {
    try {
      offer = await store.api!.call<PairOffer>('pair.start');
      error = null;
    } catch (e) {
      error = failureText(e);
    }
  }

  async function leave() {
    alive = false;
    try {
      await store.api!.call('pair.cancel');
    } catch {
      /* nothing to cancel */
    }
    store.goBack('add');
  }

  // E1 "code no longer works": the digits fade out in a slow wave, then the notice rises in.
  function lockout(notice: string) {
    const spans = codeEl ? Array.from(codeEl.querySelectorAll('span')) : [];
    const r = reduced();
    spans.forEach((s, i) =>
      anim(s, [{ opacity: 1 }, { opacity: 0.3 }], { duration: r ? 400 : 700, delay: r ? 0 : i * 70, easing: IO, fill: 'forwards' }),
    );
    const wait = spans.length && codeEl?.animate ? (r ? 400 : 700 + 6 * 70) : 0;
    setTimeout(() => {
      if (offer) offer = { ...offer, code: null, notice };
    }, wait);
  }

  onMount(() => {
    void start();
    const off = store.onEvent((e) => {
      if (!alive) return;
      if (e.event === 'pair.added') {
        alive = false;
        store.added = String(e.data.name ?? '');
        void store.refresh().then(() => store.go('connected', { replace: true }));
      } else if (e.event === 'pair.tried') {
        tried = String(e.data.notice ?? '');
        triedN += 1;
        // The 5th wrong code starts a lockout: the code is gone, the QR code stays.
        if (e.data.lockout && offer) lockout(String(e.data.lockout));
        // E1: a wrong try shakes the code once.
        else shake(codeEl);
      } else if (e.event === 'pair.closed' && e.data.reason === 'expired') {
        void start();
      }
    });
    return () => {
      alive = false;
      off();
    };
  });

  const digits = $derived((offer?.code ?? '').replace(/\D/g, '').split('').join(' '));
</script>

<div style="flex: 1; display: flex; flex-direction: column; padding: 4px 28px 24px 28px; min-height: 0;">
  <div data-m="back"><BackBar onback={leave} margin="-12px" /></div>
  <div style="flex: 1; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 18px;">
    <h1 data-m="text" style="font-size: 28px; line-height: 1.15; color: #1D1C1A; text-align: center;">{S.pairTitle}</h1>
    <div data-shape style="width: 296px; box-sizing: border-box; background: #1D1C1A; border-radius: 18px; padding: 20px 20px 24px 20px; display: flex; flex-direction: column; align-items: center; gap: 20px;">
      <div class="m-qr">
        {#if offer?.qr}
          <Qr text={offer.qr} />
        {:else}
          <div style="width: 256px; height: 256px; border-radius: 12px; background: #FFFFFF;"></div>
        {/if}
      </div>
      <div class="m-code" style="display: flex; flex-direction: column; align-items: center; gap: 20px;">
        {#if offer?.code}
          <p bind:this={codeEl} data-testid="code" aria-label={`Code ${digits}`} style="margin: 0; font-size: 36px; font-weight: 700; letter-spacing: 0.08em; font-variant-numeric: tabular-nums; line-height: 1; color: #FFFFFF; text-align: center; white-space: pre;">{#each offer.code.split('') as ch}<span>{ch}</span>{/each}</p>
        {:else if offer?.notice}
          <p data-testid="lockout" role="status" in:msgIn style="margin: 0; font-size: 14px; font-weight: 600; line-height: 1.4; color: #FFFFFF; text-align: center;">{offer.notice}</p>
        {:else}
          <p style="margin: 0; font-size: 18px; font-weight: 600; line-height: 1; color: #BDB8B1; text-align: center;">{S.pairWaiting}</p>
        {/if}
        {#if tried}
          {#key triedN}
            <p data-testid="tried" role="status" in:msgIn|global style="margin: -8px 0 0; font-size: 13px; line-height: 1.3; color: #FFD2CC; text-align: center;">{tried}</p>
          {/key}
        {/if}
      </div>
    </div>
    {#if error}<div class="err" role="alert" in:msgIn>{error}</div>{/if}
    <div data-m="os" role="img" aria-label={S.worksOn} style="display: flex; gap: 18px; align-items: center;">{@html art.trio}</div>
  </div>
  <div data-m="cta" style="display: flex; justify-content: center;">
    <button type="button" class="link" onclick={() => store.go('vpn')}>{S.otherNetwork}</button>
  </div>
</div>
