// A pretend agent for tests and for opening the Vite build in a browser. It speaks the same
// calls as the real agent (docs/agent.md) with the same reply shapes. Tests steer it through
// `window.__ucMock` (see tests/e2e).
import type { Api } from './ipc';
import type {
  AgentEvent,
  Device,
  IpcFailure,
  PairOffer,
  Settings,
  SetupStatus,
  Status,
} from './types';

export type Scenario = 'fresh' | 'home' | 'paused' | 'attention' | 'vpn' | 'lockout' | 'down';

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
    setup: {
      done: true,
      rows: [
        {
          row: 'autostart',
          title: 'Start with Windows',
          why: 'The app works after a restart.',
          state: 'done',
        },
        {
          row: 'firewall',
          title: 'Let your devices find this PC',
          why: 'Other devices can connect to this PC.',
          state: 'done',
        },
      ],
    },
    settings: baseSettings(),
    offer: { code: '418 207', qr: 'UCLIP1:mock-qr-payload-0123456789', expires_ms: 0, notice: null },
    down: false,
  };
  if (s === 'fresh') {
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
    w.setup.rows[1].state = 'todo';
    w.settings['start-at-login'].value = 'off';
  }
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
    };
    w.setup.rows[1].state = 'refused';
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
        if (row === 'autostart') w.settings['start-at-login'].value = p.answer ? 'on' : 'off';
        if (row === 'firewall' && p.answer && w.status.fix_code === 'firewall') {
          w.status = { ...baseStatus(), devices: w.devices.length };
          h.emit('status');
        }
        return structuredClone(w.setup);
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
        if (w.status.state === 'not-set-up' && w.setup.rows.some((r) => r.state === 'todo' && r.row === 'autostart')) {
          throw fail('no-network', 'The network is not on yet. Run "uc setup" first.');
        }
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
