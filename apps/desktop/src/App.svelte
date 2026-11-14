<script lang="ts">
  import { onMount } from 'svelte';
  import Chrome from './components/Chrome.svelte';
  import AddDevice from './screens/AddDevice.svelte';
  import Attention from './screens/Attention.svelte';
  import Connected from './screens/Connected.svelte';
  import DeviceDetail from './screens/DeviceDetail.svelte';
  import EnterCode from './screens/EnterCode.svelte';
  import Home from './screens/Home.svelte';
  import Permissions from './screens/Permissions.svelte';
  import Settings from './screens/Settings.svelte';
  import ShowCode from './screens/ShowCode.svelte';
  import VpnHelp from './screens/VpnHelp.svelte';
  import Welcome from './screens/Welcome.svelte';
  import { screenIn, screenOut } from './lib/flow';
  import { S } from './lib/strings';
  import { store, type Route } from './lib/store.svelte';

  async function boot() {
    await store.init();
    // Test and browser builds only: open on a given screen (?route=show), without motion.
    const r = new URLSearchParams(location.search).get('route');
    if (r && !store.api?.inTauri) {
      store.back = [];
      store.nav = { kind: 'none', from: null };
      store.route = r as Route;
      if (r === 'device') store.selected = store.devices.find((d) => !d.this)?.id ?? null;
      if (r === 'connected') store.added = 'Mac mini';
    }
  }
  onMount(() => {
    void boot();
    return () => store.dispose();
  });

  const view = $derived(store.view);
  // A new key for every screen change, also when going back to a screen that is still
  // leaving: Svelte would otherwise bring the leaving layer back, half faded.
  let changes = 0;
  let lastView: string | null = null;
  const layerKey = $derived.by(() => {
    if (view !== lastView) {
      lastView = view;
      changes += 1;
    }
    return `${view}:${changes}`;
  });
</script>

<div class="stage">
  <Chrome>
    {#if store.down}
      <div style="flex: 1; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 16px; padding: 28px;">
        <h1 style="font-size: 24px; text-align: center;">{S.agentDown}</h1>
        <button type="button" class="btn" style="width: auto; padding: 0 26px;" onclick={() => store.init()}>{S.retry}</button>
      </div>
    {:else if !store.loaded}
      <div style="flex: 1;"></div>
    {:else}
      <!-- One layer per screen: the old one stays for its exit while the new one enters (src/lib/flow.ts). -->
      {#key layerKey}
        <div class="layer" data-view={view} use:screenIn={view} out:screenOut>
          {#if view === 'welcome'}<Welcome />
          {:else if view === 'perms'}<Permissions />
          {:else if view === 'add'}<AddDevice />
          {:else if view === 'show'}<ShowCode />
          {:else if view === 'enter'}<EnterCode />
          {:else if view === 'connected'}<Connected />
          {:else if view === 'attention'}<Attention />
          {:else if view === 'device'}<DeviceDetail />
          {:else if view === 'settings'}<Settings />
          {:else if view === 'vpn'}<VpnHelp />
          {:else}<Home />
          {/if}
        </div>
      {/key}
      <!-- S1: the black shape that carries through the setup flow. Decoration only. -->
      <div class="blob" aria-hidden="true"></div>
    {/if}
  </Chrome>
</div>
