<script lang="ts">
  import * as art from '../lib/art';
  import { osKind } from '../lib/types';
  let { os, size = 44, fill = '#1D1C1A' }: { os: string; size?: number; fill?: string } = $props();
  const inner = $derived(
    { windows: art.logoWindows, apple: art.logoApple, linux: art.logoLinux, android: art.logoAndroid }[osKind(os)],
  );
</script>

<svg width={size} height={size} viewBox="0 0 24 24" {fill} aria-hidden="true" style="flex: none;">{@html inner}</svg>
