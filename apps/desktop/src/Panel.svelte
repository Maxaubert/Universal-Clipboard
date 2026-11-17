<script lang="ts">
  // The tray panel (tray spec 2.3, design docs/design/tray/panel.html): dark, minimal text.
  // It reads status and devices from the agent and never starts it. Actions that outlive the
  // panel (pick and copy, open the window, quit) run in the shell (`api.tray`).
  import { onMount, tick } from 'svelte';
  import { failureText, getApi, type Api } from './lib/ipc';
  import { panelView } from './lib/panel';
  import type { AgentEvent, Device, Status, Transfer } from './lib/types';

  let api: Api | null = null;
  let status = $state<Status | null>(null);
  let devices = $state<Device[]>([]);
  let transfer = $state<Transfer | null>(null);
  let loaded = $state(false);
  let error = $state<string | null>(null);
  let now = $state(Date.now());
  let card: HTMLDivElement | undefined = $state();

  const v = $derived(panelView(status, devices, transfer, now));

  async function load() {
    if (!api) return;
    try {
      const [s, d] = await Promise.all([api.tray.call<Status>('status'), api.tray.call<Device[]>('devices.list')]);
      status = s;
      devices = d;
      transfer = s.transfer ?? null;
    } catch {
      status = null;
    }
    now = Date.now();
    loaded = true;
  }

  function onEvent(e: AgentEvent) {
    if (e.event === 'transfer') {
      const t = e.data as unknown as Transfer;
      transfer = t.phase === 'idle' ? null : t;
      if (t.phase === 'idle') void load();
    } else if (e.event !== 'open') {
      void load();
    }
  }

  async function act(f: () => Promise<unknown>) {
    error = null;
    try {
      await f();
    } catch (e) {
      error = failureText(e);
    }
  }

  const copy = (folder: boolean) => act(() => api!.tray.copy(folder));
  const open = (route: string | null) => act(() => api!.tray.open(route));
  const quit = () => act(() => api!.tray.quit());
  const stop = () => act(() => api!.tray.call('transfer.stop'));
  const toggleSync = () =>
    act(async () => {
      await api!.tray.call('pause.set', { mode: v.paused ? 'off' : 'forever' });
      await load();
    });

  onMount(() => {
    let off: (() => void) | null = null;
    let ro: ResizeObserver | null = null;
    const esc = (e: KeyboardEvent) => e.key === 'Escape' && void api?.window.close();
    window.addEventListener('keydown', esc);
    void (async () => {
      api = await getApi();
      off = api.onEvent(onEvent);
      await load();
      await tick();
      // Tell the shell the height, then it places and shows the panel. Again when it grows
      // or shrinks (a transfer row comes or goes).
      const report = () => card && void api!.tray.ready(card.getBoundingClientRect().height + 28);
      report();
      if (card && typeof ResizeObserver !== 'undefined') {
        ro = new ResizeObserver(report);
        ro.observe(card);
      }
    })();
    const clock = setInterval(() => (now = Date.now()), 30_000);
    return () => {
      off?.();
      ro?.disconnect();
      clearInterval(clock);
      window.removeEventListener('keydown', esc);
    };
  });
</script>

<div class="edge">
  <div class="p" bind:this={card} data-testid="panel" data-state={v.word} hidden={!loaded}>
    <div class="hd">
      <svg class="bgart" viewBox="0 0 16 16" aria-hidden="true">
        <g fill="none" stroke="currentColor" stroke-width=".55" stroke-linecap="round" stroke-linejoin="round">
          <path d="M2.2 6.2L8 3.6l5.8 2.6L8 8.8z" /><path d="M2.2 8.9L8 11.5l5.8-2.6" /><path d="M2.2 11.5L8 14.1l5.8-2.6" />
        </g>
      </svg>
      <div class="state"><span class="word">{v.word}</span><span class="lamp {v.lamp}"></span></div>
      {#if v.sub}<div class="sub">{v.sub}</div>{/if}
      {#if v.chips.length}
        <div class="devs">
          {#each v.chips as c (c.name)}<span class="chip" class:sleep={!c.on}><i></i>{c.name}</span>{/each}
        </div>
      {/if}
    </div>
    <div class="bd">
      {#if v.xfer}
        <div class="xfer" data-testid="xfer">
          <svg width="16" height="16" viewBox="0 0 24 24" aria-hidden="true">
            <g fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
              {#if v.xfer.up}<path d="M12 18V5M6 10l6-6 6 6" />{:else}<path d="M12 4v13M6 12l6 6 6-6" />{/if}<path d="M5 21h14" />
            </g>
          </svg>
          <div class="track"><div class="fill" style="width: {v.xfer.pct}%"></div></div>
          <span class="n">{v.xfer.pct} %</span>
          <button type="button" class="x" title="Stop" aria-label="Stop" onclick={stop}>
            <svg width="14" height="14" viewBox="0 0 24 24" aria-hidden="true"><path d="M6 6l12 12M18 6L6 18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" /></svg>
          </button>
        </div>
      {/if}
      {#if v.note}
        <div class="note">
          <svg width="16" height="16" viewBox="0 0 24 24" aria-hidden="true" style="flex: none; margin-top: 1px;">
            <g fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"><circle cx="12" cy="12" r="9" /><path d="M12 11v5M12 7.5v.5" /></g>
          </svg>
          <div>{v.note.text}<button type="button" class="fix" onclick={() => open(v.note!.route)}>{v.note.fix}</button></div>
        </div>
      {/if}
      <div class="seg">
        <button type="button" onclick={() => copy(false)}>
          <svg width="17" height="17" viewBox="0 0 24 24" aria-hidden="true"><g fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z" /><path d="M14 3v5h5" /></g></svg>Copy files...
        </button>
        <button type="button" onclick={() => copy(true)}>
          <svg width="17" height="17" viewBox="0 0 24 24" aria-hidden="true"><path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" /></svg>Copy folder...
        </button>
      </div>
      <div class="row">
        <svg width="18" height="18" viewBox="0 0 24 24" aria-hidden="true"><g fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M20 12a8 8 0 0 1-14 5.3" /><path d="M4 12A8 8 0 0 1 18 6.7" /><path d="M18 3v4h-4" /><path d="M6 21v-4h4" /></g></svg>
        <div class="t">Sync</div>
        <button type="button" role="switch" aria-checked={!v.paused} aria-label="Sync" class="sw" class:off={v.paused} disabled={!status} onclick={toggleSync}></button>
      </div>
      {#if error}<div class="err" role="alert">{error}</div>{/if}
    </div>
    <div class="ft">
      <button type="button" class="open" onclick={() => open(null)}>
        <svg width="18" height="18" viewBox="0 0 24 24" aria-hidden="true"><g fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M3.5 9.3L12 5.4l8.5 3.9L12 13.2z" /><path d="M3.5 13.3l8.5 3.9 8.5-3.9" /><path d="M3.5 17.2l8.5 3.9 8.5-3.9" /></g></svg>Open Universal Clipboard
      </button>
      <span class="sp"></span>
      <button type="button" class="ib" title="Settings" aria-label="Settings" onclick={() => open('settings')}>
        <svg width="18" height="18" viewBox="0 0 24 24" aria-hidden="true"><g fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"><path d="M4 7h10M18 7h2M4 17h2M10 17h10" /><circle cx="16" cy="7" r="2" /><circle cx="8" cy="17" r="2" /></g></svg>
      </button>
      <button type="button" class="ib" title="Quit (stops syncing)" aria-label="Quit (stops syncing)" onclick={quit}>
        <svg width="18" height="18" viewBox="0 0 24 24" aria-hidden="true"><g fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"><path d="M12 3v8" /><path d="M6.3 6.8a8 8 0 1 0 11.4 0" /></g></svg>
      </button>
    </div>
  </div>
</div>

<style>
  /* The clear edge around the panel holds its shadow (the window is transparent). */
  .edge { padding: 14px; box-sizing: border-box; width: 348px; }
  .p {
    --bg: #1A1918; --ink: #F2F0EC; --ink2: #A29E97; --line: #2C2A28; --well: #242321; --wellHover: #2C2A27;
    --head: #0E0D0C; --headInk: #F7F6F4; --headInk2: rgba(247, 246, 244, 0.58); --pattern: rgba(247, 246, 244, 0.06);
    --ok: #5BB585; --warn: #F08A4B; --bar: #F2F0EC; --track: #34322F;
    width: 320px; box-sizing: border-box; background: var(--bg); color: var(--ink); border-radius: 12px; overflow: hidden; line-height: 1.45;
    border: 1px solid rgba(255, 255, 255, 0.08); box-shadow: 0 6px 14px rgba(0, 0, 0, 0.45);
  }
  .p[hidden] { display: none; }
  .hd { position: relative; background: var(--head); color: var(--headInk); padding: 18px 20px 16px; overflow: hidden; }
  .hd .bgart { position: absolute; right: -34px; top: -26px; width: 190px; height: 190px; color: var(--pattern); pointer-events: none; }
  .state { position: relative; display: flex; align-items: center; gap: 9px; }
  .word { font-size: 30px; font-weight: 700; letter-spacing: -0.025em; line-height: 32px; }
  .lamp { width: 9px; height: 9px; border-radius: 50%; background: var(--ok); box-shadow: 0 0 0 3px rgba(91, 181, 133, 0.18); margin-top: 3px; }
  .lamp.off { background: #8d8983; box-shadow: none; }
  .lamp.warn { background: var(--warn); box-shadow: 0 0 0 3px rgba(240, 138, 75, 0.2); }
  .sub { position: relative; font-size: 13px; color: var(--headInk2); margin-top: 6px; }
  .devs { position: relative; display: flex; gap: 6px; margin-top: 12px; flex-wrap: wrap; }
  .chip { display: inline-flex; align-items: center; gap: 6px; height: 24px; padding: 0 9px; border-radius: 999px; font-size: 12px; font-weight: 500; background: rgba(247, 246, 244, 0.09); color: var(--headInk); }
  .chip i { width: 6px; height: 6px; border-radius: 50%; background: #5bb585; }
  .chip.sleep { color: var(--headInk2); }
  .chip.sleep i { background: #6f6b65; }
  .bd { padding: 14px 14px 12px; display: grid; gap: 10px; }
  .xfer { display: flex; align-items: center; gap: 10px; background: var(--well); border-radius: 9px; height: 44px; padding: 0 6px 0 12px; color: var(--ink2); }
  .track { flex: 1; height: 4px; border-radius: 4px; background: var(--track); overflow: hidden; }
  .fill { height: 100%; border-radius: 4px; background: var(--bar); transition: width 0.25s ease-out; }
  .n { font-size: 12.5px; font-variant-numeric: tabular-nums; color: var(--ink2); min-width: 34px; text-align: right; }
  .x { width: 30px; height: 30px; display: grid; place-items: center; border: 0; border-radius: 6px; background: transparent; color: var(--ink2); }
  .x:hover { background: var(--wellHover); color: var(--ink); }
  .note { font-size: 12.5px; line-height: 1.4; color: var(--ink); background: var(--well); border-radius: 9px; padding: 10px 12px; display: flex; gap: 10px; }
  .fix { display: block; margin-top: 6px; padding: 0; border: 0; background: none; color: inherit; font: inherit; font-weight: 600; text-decoration: underline; text-underline-offset: 3px; }
  .seg { display: grid; grid-template-columns: 1fr 1fr; background: var(--well); border-radius: 9px; padding: 3px; gap: 3px; }
  .seg button { display: flex; align-items: center; justify-content: center; gap: 8px; height: 40px; border: 0; border-radius: 7px; background: transparent; color: var(--ink); font-size: 13px; font-weight: 600; }
  .seg button:hover { background: #33312e; }
  .row { display: flex; align-items: center; gap: 11px; height: 44px; padding: 0 4px 0 6px; }
  .row .t { font-size: 13.5px; font-weight: 500; }
  .sw { margin-left: auto; width: 38px; height: 22px; border: 0; padding: 0; border-radius: 999px; background: var(--ink); position: relative; flex: none; }
  .sw::after { content: ''; position: absolute; top: 3px; left: 19px; width: 16px; height: 16px; border-radius: 50%; background: var(--bg); }
  .sw.off { background: var(--track); }
  .sw.off::after { left: 3px; background: #8d8983; }
  .err { font-size: 12.5px; color: var(--warn); padding: 0 6px; }
  .ft { display: flex; align-items: center; border-top: 1px solid var(--line); padding: 6px 8px 6px 10px; }
  .open { display: flex; align-items: center; gap: 9px; height: 38px; padding: 0 10px; border-radius: 7px; border: 0; background: transparent; color: var(--ink); font-size: 13px; font-weight: 600; }
  .ib { width: 38px; height: 38px; display: grid; place-items: center; border: 0; border-radius: 7px; background: transparent; color: var(--ink2); }
  .open:hover, .ib:hover { background: var(--well); color: var(--ink); }
  .sp { flex: 1; }
  svg { flex: none; }
  button:focus-visible { outline: 2px solid var(--ink); outline-offset: 1px; }
</style>
