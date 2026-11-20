// The tray panel's view (tray spec 2.3, design docs/design/tray/panel.html): state word,
// lamp, device chips, the transfer bar and the one note. Minimal text: a transfer shows only
// its bar and percent, and there is no status line (Max 2026-11-17).
import type { Device, Status, Transfer } from './types';

export type Lamp = 'ok' | 'off' | 'warn';

export interface PanelView {
  word: string;
  lamp: Lamp;
  chips: { name: string; on: boolean }[];
  /** The file copy that moves now: direction and percent. */
  xfer: { up: boolean; pct: number } | null;
  /** Needs attention: the agent's sentence, and what the link opens. */
  note: { text: string; fix: string; route: 'vpn' | null } | null;
  paused: boolean;
}

export function panelView(
  status: Status | null,
  devices: Device[],
  transfer: Transfer | null,
): PanelView {
  const chips = devices.filter((d) => !d.this).map((d) => ({ name: d.name, on: d.state === 'Connected' }));
  if (!status) {
    return { word: 'Not running', lamp: 'warn', chips, xfer: null, note: null, paused: false };
  }
  const t = transfer ?? status.transfer ?? null;
  const moving = !!t && t.phase !== 'idle';
  const xfer = moving
    ? { up: t!.phase !== 'receiving', pct: t!.total > 0 ? Math.min(100, Math.floor((t!.done * 100) / t!.total)) : 0 }
    : null;
  const paused = status.paused;
  const base = { chips, xfer, paused, note: null } as const;
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
      return { ...base, word: 'Synced', lamp: 'ok' };
  }
}
