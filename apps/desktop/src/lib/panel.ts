// The tray panel's view (tray spec 2.3, design docs/design/tray/panel.html): state word,
// lamp, one short fact, device chips, the transfer bar and the one note. Minimal text: a
// transfer shows only its bar and percent.
import type { Activity, Device, Status, Transfer } from './types';

export type Lamp = 'ok' | 'off' | 'warn';

export interface PanelView {
  word: string;
  lamp: Lamp;
  /** One short fact, only when synced: "Text from Max's Mac, 2 min ago". */
  sub: string | null;
  chips: { name: string; on: boolean }[];
  /** The file copy that moves now: direction and percent. */
  xfer: { up: boolean; pct: number } | null;
  /** Needs attention: the agent's sentence, and what the link opens. */
  note: { text: string; fix: string; route: 'vpn' | null } | null;
  paused: boolean;
}

/** "just now", "2 min ago", "3 h ago", "2 days ago" (short, for the panel). */
export function shortAgo(ms: number, now: number): string {
  const s = Math.max(0, Math.floor((now - ms) / 1000));
  if (s < 60) return 'just now';
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86_400) return `${Math.floor(s / 3600)} h ago`;
  const d = Math.floor(s / 86_400);
  return `${d} day${d === 1 ? '' : 's'} ago`;
}

function fact(a: Activity | undefined, now: number): string | null {
  if (!a) return null;
  if (a.direction === 'received' && a.device) return `${a.kind} from ${a.device}, ${shortAgo(a.time_ms, now)}`;
  if (a.direction === 'sent') return `${a.kind} copied here, ${shortAgo(a.time_ms, now)}`;
  return null;
}

export function panelView(
  status: Status | null,
  devices: Device[],
  transfer: Transfer | null,
  now: number = Date.now(),
): PanelView {
  const chips = devices.filter((d) => !d.this).map((d) => ({ name: d.name, on: d.state === 'Connected' }));
  if (!status) {
    return { word: 'Not running', lamp: 'warn', sub: null, chips, xfer: null, note: null, paused: false };
  }
  const t = transfer ?? status.transfer ?? null;
  const moving = !!t && t.phase !== 'idle';
  const xfer = moving
    ? { up: t!.phase !== 'receiving', pct: t!.total > 0 ? Math.min(100, Math.floor((t!.done * 100) / t!.total)) : 0 }
    : null;
  const paused = status.paused;
  const base = { chips, xfer, paused, sub: null, note: null } as const;
  switch (status.state) {
    case 'needs-attention': {
      const code = status.fix_code ?? '';
      const net = code === 'vpn' || code === 'no-lan' || code === 'firewall';
      return {
        ...base,
        xfer: null,
        word: net ? 'Not connected' : 'Needs attention',
        lamp: 'warn',
        note: status.reason
          ? { text: status.reason, fix: code === 'vpn' ? 'Show me how' : 'Fix', route: code === 'vpn' ? 'vpn' : null }
          : null,
      };
    }
    case 'paused':
      return { ...base, word: 'Paused', lamp: 'off' };
    case 'not-set-up':
      return { ...base, word: 'Not set up', lamp: 'off' };
    case 'no-peers':
      return { ...base, word: moving ? (xfer!.up ? 'Sending' : 'Receiving') : 'Not connected', lamp: 'off' };
    default:
      if (moving) return { ...base, word: xfer!.up ? 'Sending' : 'Receiving', lamp: 'ok' };
      return {
        ...base,
        word: 'Synced',
        lamp: 'ok',
        sub: fact((status.activity ?? []).find((a) => a.direction === 'sent' || a.direction === 'received'), now),
      };
  }
}
