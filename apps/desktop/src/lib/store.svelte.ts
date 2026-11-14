import { failureText, getApi, type Api } from './ipc';
import type { AgentEvent, Device, Settings, SetupStatus, Status } from './types';

export type Route =
  | 'welcome'
  | 'perms'
  | 'add'
  | 'show'
  | 'enter'
  | 'connected'
  | 'home'
  | 'device'
  | 'settings'
  | 'vpn';

/** Step of the 5-step indicator for the setup screens, else 0 (no indicator). */
export function stepOf(r: Route): number {
  return { welcome: 1, perms: 2, add: 3, show: 4, enter: 4, connected: 5 }[r as string] ?? 0;
}

class Store {
  api: Api | null = null;
  route = $state<Route>('welcome');
  back = $state<Route[]>([]);
  status = $state<Status | null>(null);
  devices = $state<Device[]>([]);
  settings = $state<Settings>({});
  setup = $state<SetupStatus | null>(null);
  loaded = $state(false);
  down = $state<string | null>(null);
  /** Device shown on the detail screen. */
  selected = $state<string | null>(null);
  /** Name shown on the "is added" screen. */
  added = $state<string | null>(null);
  private listeners = new Set<(e: AgentEvent) => void>();
  private off: (() => void) | null = null;

  async init(): Promise<void> {
    this.api = await getApi();
    this.off?.();
    this.off = this.api.onEvent((e) => {
      if (['status', 'devices', 'removed', 'pair.added', 'pair.joined'].includes(e.event)) {
        void this.refresh();
      }
      for (const l of this.listeners) l(e);
    });
    await this.refresh();
    if (this.down) return;
    try {
      await this.api.call('subscribe');
    } catch {
      /* events are optional; polling below covers it */
    }
    // The first screen depends on what the agent says.
    const s = this.status;
    if (s && s.state === 'not-set-up' && !(this.setup?.done && this.devices.length > 1)) {
      this.route = 'welcome';
    } else {
      this.route = 'home';
    }
    this.back = [];
    this.loaded = true;
  }

  /** Dispose of the event subscription (tests). */
  dispose(): void {
    this.off?.();
    this.off = null;
    this.listeners.clear();
    this.loaded = false;
    this.down = null;
    this.status = null;
    this.devices = [];
    this.settings = {};
    this.setup = null;
    this.route = 'welcome';
    this.back = [];
    this.selected = null;
    this.added = null;
  }

  onEvent(cb: (e: AgentEvent) => void): () => void {
    this.listeners.add(cb);
    return () => this.listeners.delete(cb);
  }

  async refresh(): Promise<void> {
    const api = this.api;
    if (!api) return;
    try {
      const [status, devices, settings, setup] = await Promise.all([
        api.call<Status>('status'),
        api.call<Device[]>('devices.list'),
        api.call<Settings>('settings.get'),
        api.call<SetupStatus>('setup.status'),
      ]);
      this.status = status;
      this.devices = devices;
      this.settings = settings;
      this.setup = setup;
      this.down = null;
    } catch (e) {
      this.down = failureText(e);
    }
  }

  go(r: Route, opts: { replace?: boolean } = {}): void {
    if (!opts.replace) this.back = [...this.back, this.route];
    this.route = r;
  }

  goBack(fallback: Route = 'home'): void {
    const prev = this.back[this.back.length - 1];
    this.back = this.back.slice(0, -1);
    this.route = prev ?? fallback;
  }

  get thisDevice(): Device | undefined {
    return this.devices.find((d) => d.this);
  }
}

export const store = new Store();
