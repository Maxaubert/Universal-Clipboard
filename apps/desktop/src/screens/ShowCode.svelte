<script lang="ts">
  import { onMount } from 'svelte';
  import BackBar from '../components/BackBar.svelte';
  import Qr from '../components/Qr.svelte';
  import * as art from '../lib/art';
  import { failureText } from '../lib/ipc';
  import { S } from '../lib/strings';
  import { store } from '../lib/store.svelte';
  import type { PairOffer } from '../lib/types';

  let offer = $state<PairOffer | null>(null);
  let tried = $state<string | null>(null);
  let error = $state<string | null>(null);
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
  <BackBar onback={leave} margin="-12px" />
  <div style="flex: 1; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 18px;">
    <h1 style="font-size: 28px; line-height: 1.15; color: #1D1C1A; text-align: center;">{S.pairTitle}</h1>
    <div style="width: 296px; box-sizing: border-box; background: #1D1C1A; border-radius: 18px; padding: 20px 20px 24px 20px; display: flex; flex-direction: column; align-items: center; gap: 20px;">
      {#if offer?.qr}
        <Qr text={offer.qr} />
      {:else}
        <div style="width: 256px; height: 256px; border-radius: 12px; background: #FFFFFF;"></div>
      {/if}
      {#if offer?.code}
        <p data-testid="code" aria-label={`Code ${digits}`} style="margin: 0; font-size: 36px; font-weight: 700; letter-spacing: 0.08em; font-variant-numeric: tabular-nums; line-height: 1; color: #FFFFFF; text-align: center;">{offer.code}</p>
      {:else if offer?.notice}
        <p data-testid="lockout" role="status" style="margin: 0; font-size: 14px; font-weight: 600; line-height: 1.4; color: #FFFFFF; text-align: center;">{offer.notice}</p>
      {:else}
        <p style="margin: 0; font-size: 18px; font-weight: 600; line-height: 1; color: #BDB8B1; text-align: center;">{S.pairWaiting}</p>
      {/if}
      {#if tried}
        <p data-testid="tried" role="status" style="margin: -8px 0 0; font-size: 13px; line-height: 1.3; color: #FFD2CC; text-align: center;">{tried}</p>
      {/if}
    </div>
    {#if error}<div class="err" role="alert">{error}</div>{/if}
    <div role="img" aria-label={S.worksOn} style="display: flex; gap: 18px; align-items: center;">{@html art.trio}</div>
  </div>
  <div style="display: flex; justify-content: center;">
    <button type="button" class="link" onclick={() => store.go('vpn')}>{S.otherNetwork}</button>
  </div>
</div>
