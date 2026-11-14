<script lang="ts">
  // Static illustrations from the locked design. Nothing here comes from the agent.
  // Welcome, Add and Connected use the classed copies from the S1 motion mockup (art-motion.ts),
  // so the screen flow can move their parts; they look the same at rest.
  import * as art from '../lib/art';
  import * as moving from '../lib/art-motion';
  import { osKind } from '../lib/types';
  let { name, os = 'macOS' }: { name: 'welcome' | 'add' | 'connected' | 'attention' | 'vpn'; os?: string } = $props();

  function connected(os: string): string {
    const logo = { windows: art.logoWindows, apple: art.logoApple, linux: art.logoLinux, android: art.logoAndroid }[osKind(os)];
    return moving.connected(logo);
  }
  const html = $derived(
    name === 'connected' ? connected(os) : name === 'welcome' ? moving.welcome : name === 'add' ? moving.add : art[name],
  );
</script>

{@html html}
