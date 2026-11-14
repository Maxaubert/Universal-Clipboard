<script lang="ts">
  // Draws the QR text the agent made. The window never makes pairing secrets: it only draws.
  import qrcode from 'qrcode-generator';
  import { S } from '../lib/strings';

  let { text, size = 256 }: { text: string; size?: number } = $props();
  const quiet = 2;

  const model = $derived.by(() => {
    const q = qrcode(0, 'M');
    q.addData(text);
    q.make();
    const n = q.getModuleCount();
    let d = '';
    for (let r = 0; r < n; r++) {
      for (let c = 0; c < n; c++) {
        if (q.isDark(r, c)) d += `M${c + quiet} ${r + quiet}h1v1h-1z`;
      }
    }
    return { n: n + quiet * 2, d };
  });
</script>

<svg
  role="img"
  aria-label={S.qrLabel}
  width={size}
  height={size}
  viewBox={`0 0 ${model.n} ${model.n}`}
  shape-rendering="crispEdges"
  style="display: block; flex: none; box-sizing: border-box; border-radius: 12px; background: #FFFFFF;"
>
  <path d={model.d} fill="#1D1C1A"></path>
</svg>
