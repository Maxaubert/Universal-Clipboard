<script lang="ts">
  // Static illustrations from the locked design. Nothing here comes from the agent.
  import * as art from '../lib/art';
  import { osKind } from '../lib/types';
  let { name, os = 'macOS' }: { name: 'welcome' | 'add' | 'connected' | 'attention' | 'vpn'; os?: string } = $props();

  function connected(os: string): string {
    const logo = { windows: art.logoWindows, apple: art.logoApple, linux: art.logoLinux, android: art.logoAndroid }[osKind(os)];
    // The right-hand device carries the OS logo of the device that was just added.
    return art.connected.replace(/(<g transform="translate\(263\.0 48\.5\) scale\(0\.92\)"[^>]*>)(.*?)(<\/g>)/, `$1${logo}$3`);
  }
  const html = $derived(name === 'connected' ? connected(os) : art[name]);
</script>

{@html html}
