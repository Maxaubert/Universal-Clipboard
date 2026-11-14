// A pretend agent for tests and for opening the Vite build in a browser. It speaks the same
// calls as the real agent (docs/agent.md) with the same reply shapes. Tests steer it through
// `window.__ucMock` (see tests/e2e).
import type { Api } from './ipc';
import type {
  AgentEvent,
  Device,
  FirewallCheck,
  IpcFailure,
  PairOffer,
  Settings,
  SetupStatus,
  Status,
} from './types';

// 'fresh' is a new device: no setup rows on any OS (decision 2026-11-14), so the window skips
// the permissions screen. 'mac-blocked' is a new Mac where macOS blocks the local network and
// paste access: the only case with rows (the agent shows them only then).
// 'h3' is the device set of the H3 "You are here" mockup (docs/design/desktop/motion/home.html),
// for the visual check of Home.
export type Scenario = 'fresh' | 'mac-blocked' | 'home' | 'h3' | 'paused' | 'attention' | 'vpn' | 'lockout' | 'down';

interface World {
  status: Status;
  devices: Device[];
  setup: SetupStatus;
  settings: Settings;
  offer: PairOffer;
  down: boolean;
}

const dev = (name: string, os: string, state: Device['state'], me = false): Device => ({
  id: name.toLowerCase().replace(/\W+/g, '-'),
  name,
  os,
  this: me,
  state,
});

function homeDevices(): Device[] {
  return [
    dev("Max's PC", 'Windows', 'This device', true),
    dev('Mac mini', 'macOS', 'Connected'),
    dev('Ubuntu laptop', 'Linux', 'Connected'),
    dev('iPhone', 'iPhone', 'Connected'),
    dev('iPad', 'iPad', 'Asleep or away'),
    dev('Pixel 8', 'Android phone', 'Connected'),
  ];
}

function baseStatus(): Status {
  return {
    state: 'working',
    text: 'Working',
    reason: null,
    fix: null,
    fix_code: null,
    devices: 6,
    connected: 4,
    paused: false,
    this_device: "Max's PC",
    version: '1.0.0',
    activity: [{ kind: 'Text', direction: 'received', device: 'Mac mini', time_ms: Date.now(), note: null }],
  };
}

function baseSettings(): Settings {
  return {
    'start-at-login': { value: 'on', label: 'Start at login' },
    'sync-images': { value: 'on', label: 'Sync images' },
    'image-limit-mb': { value: '25', label: 'Largest image to sync (MB)' },
    'keep-formatting': { value: 'off', label: 'Keep formatting' },
    'device-name': { value: "Max's PC", label: 'Device name' },
  };
}

export function makeWorld(s: Scenario): World {
  const w: World = {
    status: baseStatus(),
    devices: homeDevices(),
    // No setup rows on any OS: the installer adds the Windows firewall rule, and start at
    // login is on by default (decision 2026-11-14).
    setup: { done: true, rows: [] },
    settings: baseSettings(),
    offer: { code: '418 207', qr: 'UCLIP1:mock-qr-payload-0123456789', expires_ms: 0, notice: null },
    down: false,
  };
  if (s === 'fresh' || s === 'mac-blocked') {
    w.devices = [dev("Max's PC", 'Windows', 'This device', true)];
    w.status = {
      ...w.status,
      state: 'not-set-up',
      text: 'Not set up',
      devices: 1,
      connected: 0,
      reason: 'This device has no other devices yet.',
    };
    w.setup.done = false;
  }
  if (s === 'mac-blocked') {
    w.devices = [dev("Max's Mac", 'macOS', 'This device', true)];
    w.status.this_device = "Max's Mac";
    w.setup.rows = [
      {
        row: 'local-network',
        title: 'Find devices on your network',
        why: 'Needed to reach devices on your network.',
        state: 'refused',
      },
      {
        row: 'paste',
        title: 'Read what you copy',
        why: 'The app must read the clipboard without asking each time.',
        state: 'refused',
      },
    ];
  }
  if (s === 'h3') {
    w.devices = [
      dev('MAX', 'Windows', 'This device', true),
      dev("Max's Mac Mini", 'macOS', 'Connected'),
      dev("Max's iPhone", 'iPhone', 'Asleep or away'),
      dev('lin-kde', 'Linux', 'Connected'),
    ];
    w.status.this_device = 'MAX';
    w.status.activity = [{ kind: 'Text', direction: 'received', device: "Max's Mac Mini", time_ms: Date.now(), note: null }];
  }
  if (s === 'fresh' || s === 'mac-blocked') w.status.activity = [];
  if (s === 'paused') {
    w.status = { ...w.status, state: 'paused', text: 'Paused', paused: true };
  }
  if (s === 'attention') {
    w.status = {
      ...w.status,
      state: 'needs-attention',
      text: 'Needs attention',
      reason: 'Firewall is blocking it.',
      fix: 'Allow Universal Clipboard through the firewall.',
      fix_code: 'firewall',
      connected: 0,
      note: {
        code: 'firewall',
        text: 'Other devices cannot connect to this PC.',
        fix: 'Run "uc firewall add".',
      },
    };
  }
  if (s === 'vpn') {
    w.status = {
      ...w.status,
      state: 'needs-attention',
      text: 'Needs attention',
      reason: "Can't reach your other device? Your VPN blocks the local network.",
      fix: 'Turn on Local network sharing in your VPN.',
      fix_code: 'vpn',
      connected: 0,
    };
    w.devices[1].state = 'Blocked by a VPN or firewall';
  }
  if (s === 'lockout') {
    w.offer = {
      code: null,
      qr: w.offer.qr,
      expires_ms: 0,
      notice: 'Too many wrong codes. Try again in 5 minutes.',
      lockout_ms: 5 * 60 * 1000,
    };
  }
  if (s === 'down') w.down = true;
  return w;
}

export interface MockHandle {
  world: World;
  calls: { call: string; params: Record<string, unknown> }[];
  load(s: Scenario): void;
  emit(event: string, data?: Record<string, unknown>): void;
  /** What the next pair.join does. */
  joinResult: { ok: true; name: string; os: string } | { ok: false; error: IpcFailure };
  windowLog: string[];
  /** What the next firewallAdd does: 0 done, 3 the user said no in the UAC prompt. */
  firewallAddResult: 0 | 3;
  /** A copy moved (facts only): the agent records it and sends an `activity` event. */
  copy(direction: 'sent' | 'received', kind?: string, device?: string | null): void;
}

declare global {
  interface Window {
    __ucMock?: MockHandle;
  }
}

const fail = (code: string, message: string): IpcFailure => ({ code, message });

export function createMockApi(initial?: Scenario): Api {
  const params = typeof location !== 'undefined' ? new URLSearchParams(location.search) : null;
  const first = (initial ?? (params?.get('scenario') as Scenario | null) ?? 'home') as Scenario;
  const subs = new Set<(e: AgentEvent) => void>();
  const h: MockHandle = {
    world: makeWorld(first),
    calls: [],
    load(s) {
      h.world = makeWorld(s);
      h.emit('status');
      h.emit('devices');
    },
    emit(event, data = {}) {
      for (const f of subs) f({ event, data });
    },
    joinResult: { ok: true, name: 'Mac mini', os: 'macOS' },
    windowLog: [],
    firewallAddResult: 0,
    copy(direction, kind = 'Text', device = null) {
      const a = { kind, direction, device: direction === 'received' ? device : null, time_ms: Date.now(), note: null };
      h.world.status.activity = [a, ...(h.world.status.activity ?? [])].slice(0, 5);
      h.emit('activity', { ...a });
    },
  };
  if (typeof window !== 'undefined') window.__ucMock = h;

  const refreshStatus = () => {
    const w = h.world;
    const others = w.devices.filter((d) => !d.this);
    w.status.devices = w.devices.length;
    w.status.connected = others.filter((d) => d.state === 'Connected').length;
    if (w.status.state === 'not-set-up' && others.length > 0) {
      w.status.state = 'working';
      w.status.text = 'Working';
      w.status.reason = null;
    }
  };

  async function handle(name: string, p: Record<string, unknown>): Promise<unknown> {
    const w = h.world;
    if (w.down) throw fail('agent-down', 'The background service is not running.');
    switch (name) {
      case 'subscribe':
        return { subscribed: true };
      case 'version':
        return { version: w.status.version };
      case 'status':
        refreshStatus();
        return structuredClone(w.status);
      case 'devices.list':
        return structuredClone(w.devices);
      case 'settings.get':
        return structuredClone(w.settings);
      case 'settings.set': {
        const k = String(p.key);
        if (!w.settings[k]) throw fail('bad-key', `No setting "${k}".`);
        w.settings[k].value = String(p.value);
        return { ok: true };
      }
      case 'setup.status':
        return structuredClone(w.setup);
      case 'setup.step': {
        const row = String(p.row);
        if (row === 'done') w.setup.done = true;
        const r = w.setup.rows.find((x) => x.row === row);
        if (r) r.state = p.answer ? 'done' : 'refused';
        return structuredClone(w.setup);
      }
      case 'firewall.check': {
        const absent = w.status.note?.code === 'firewall';
        return { rule: absent ? 'absent' : 'present', incoming: !absent } satisfies FirewallCheck;
      }
      case 'pause.set': {
        const off = p.mode === 'off';
        w.status.paused = !off;
        w.status.state = off ? 'working' : 'paused';
        w.status.text = off ? 'Working' : 'Paused';
        h.emit('status');
        return { paused: !off };
      }
      case 'devices.rename': {
        const d = w.devices.find((x) => x.name === p.device || x.id === p.device);
        const n = String(p.name ?? '').trim();
        if (!d) throw fail('no-device', 'No such device.');
        if (n.length < 1 || n.length > 64) throw fail('bad-name', 'A name has 1 to 64 characters.');
        d.name = n;
        if (d.this) w.status.this_device = n;
        h.emit('devices');
        return { renamed: true };
      }
      case 'devices.remove': {
        const i = w.devices.findIndex((x) => x.name === p.device || x.id === p.device);
        if (i < 0) throw fail('no-device', 'No such device.');
        if (w.devices[i].this) throw fail('self', 'To remove this device, use "uc leave".');
        w.devices.splice(i, 1);
        h.emit('devices');
        return { removed: true };
      }
      case 'leave': {
        w.devices = [dev(w.status.this_device, 'Windows', 'This device', true)];
        w.status = {
          ...w.status,
          state: 'not-set-up',
          text: 'Not set up',
          devices: 1,
          connected: 0,
        };
        w.setup.done = false;
        h.emit('status');
        return { left: true };
      }
      case 'pair.start':
        return structuredClone(w.offer);
      case 'pair.cancel':
        h.emit('pair.closed', { reason: 'cancelled' });
        return { cancelled: true };
      case 'pair.join': {
        const digits = String(p.code ?? '').replace(/\D/g, '');
        if (digits.length !== 6) throw fail('bad-code', 'Type the 6 digits of the code.');
        if (w.devices.length > 1 && !p.leave_other) {
          throw fail('in-group', 'Leave your other devices and join these?');
        }
        if (!h.joinResult.ok) throw h.joinResult.error;
        const r = h.joinResult;
        if (!w.devices.some((d) => d.name === r.name)) {
          w.devices.push(dev(r.name, r.os, 'Connected'));
        }
        refreshStatus();
        h.emit('pair.joined', { name: r.name });
        h.emit('devices');
        return { name: r.name };
      }
      case 'diagnostics':
        return { report: 'Universal Clipboard (mock)' };
      default:
        throw fail('unknown-call', `Unknown call "${name}".`);
    }
  }

  return {
    inTauri: false,
    async call<T>(name: string, p: Record<string, unknown> = {}) {
      h.calls.push({ call: name, params: p });
      return (await handle(name, p)) as T;
    },
    async firewallAdd() {
      h.calls.push({ call: 'firewall.add', params: {} });
      if (h.world.down) throw fail('agent-down', 'The background service is not running.');
      if (h.firewallAddResult === 3) {
        throw fail(
          'refused',
          'You said no. This PC only connects out; sync works when the other device accepts connections.',
        );
      }
      const w = h.world;
      if (w.status.note?.code === 'firewall') {
        w.status = { ...baseStatus(), devices: w.devices.length };
        h.emit('status');
      }
      return (await this.call('firewall.check')) as FirewallCheck;
    },
    onEvent(cb) {
      subs.add(cb);
      return () => subs.delete(cb);
    },
    window: {
      minimize: async () => void h.windowLog.push('minimize'),
      toggleMaximize: async () => void h.windowLog.push('maximize'),
      close: async () => void h.windowLog.push('close'),
    },
  };
}
