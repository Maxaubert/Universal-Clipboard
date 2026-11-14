import { failureText, getApi, type Api } from './ipc';
import type { AgentEvent, Device, Settings, SetupStatus, Status, UpdateStatus } from './types';

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
  | 'vpn'
  | 'update';

export type View = Route | 'attention';

/** Setup steps: 4 on every OS (decision 2026-11-14): no OS asks anything in setup, so there is
 * no permissions step. When the OS blocks something (macOS rows) the permissions screen still
 * shows, as part of step 1. */
export const STEPS = 4;

/** Step of the setup indicator for the setup screens, else 0 (no indicator). */
export function stepOf(r: Route): number {
  const steps: Record<string, number> = { welcome: 1, perms: 1, add: 2, show: 3, enter: 3, connected: 4 };
  return steps[r as string] ?? 0;
}

/** Setup rows the permissions screen asks ("desktop" only explains, Needs attention shows it). */
export function askRows(s: SetupStatus | null): SetupStatus['rows'] {
  return (s?.rows ?? []).filter((r) => r.row !== 'desktop');
}

class Store {
  api: Api | null = null;
  route = $state<Route>('welcome');
  back = $state<Route[]>([]);
  status = $state<Status | null>(null);
  devices = $state<Device[]>([]);
  settings = $state<Settings>({});
  setup = $state<SetupStatus | null>(null);
  /** The agent's update status (`update.status`), or null when it has none. */
  update = $state<UpdateStatus | null>(null);
  /** The update screen waits for pairing to end before it shows. */
  updatePending = $state(false);
  loaded = $state(false);
  down = $state<string | null>(null);
  /** Device shown on the detail screen. */
  selected = $state<string | null>(null);
  /** Name shown on the "is added" screen. */
  added = $state<string | null>(null);
  /** How the last route change happened, for the screen motion (src/lib/flow.ts). A plain
   * field: the flow reads it once per change. 'none' (a direct `route =`) swaps without motion. */
  nav: { kind: 'forward' | 'back' | 'none' | 'boot'; from: Element | null } = { kind: 'none', from: null };
  private listeners = new Set<(e: AgentEvent) => void>();
  private off: (() => void) | null = null;

  async init(): Promise<void> {
    this.api = await getApi();
    this.off?.();
    this.off = this.api.onEvent((e) => {
      if (['status', 'devices', 'removed', 'pair.added', 'pair.joined', 'activity'].includes(e.event)) {
        void this.refresh();
      }
      if (e.event === 'update') {
        this.update = e.data as unknown as UpdateStatus;
        if (this.loaded && this.shouldAsk()) this.openUpdate();
      }
      // The shell asks for a screen: the agent started the window again with `--route update`.
      if (e.event === 'open' && e.data.route === 'update' && this.loaded) this.openUpdate();
      for (const l of this.listeners) l(e);
    });
    await this.refresh();
    if (this.down) return;
    try {
      await this.api.call('subscribe');
    } catch {
      /* events are optional; polling below covers it */
    }
    await this.refreshUpdate();
    // The first screen depends on what the agent says.
    const s = this.status;
    if (s && s.state === 'not-set-up' && !(this.setup?.done && this.devices.length > 1)) {
      this.route = 'welcome';
    } else {
      this.route = 'home';
    }
    this.back = [];
    // The agent opened the window for an update (`--route update`).
    if ((await this.api.launchRoute()) === 'update' && this.update?.state !== 'none') {
      this.back = [this.route];
      this.route = 'update';
    }
    this.nav = { kind: 'boot', from: null };
    this.loaded = true;
  }

  /** Read `update.status` again (not every agent has an updater; then nothing shows). */
  async refreshUpdate(): Promise<void> {
    try {
      this.update = (await this.api?.call<UpdateStatus>('update.status')) ?? null;
    } catch {
      /* an older agent or no agent: no update screen */
    }
  }

  /** True when the agent's news should take the screen: a ready or failed update, no Later. */
  shouldAsk(): boolean {
    const u = this.update;
    return !!u && (u.state === 'available' || u.state === 'failed') && !u.dismissed && this.route !== 'update';
  }

  /** Show the update screen, unless a pairing is on: then right after it. */
  openUpdate(): void {
    if (this.route === 'update') return;
    if (this.route === 'show' || this.route === 'enter') {
      this.updatePending = true;
      return;
    }
    this.updatePending = false;
    this.go('update');
  }

  /** Leave the update screen (Later, OK). */
  leaveUpdate(): void {
    this.goBack('home');
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
    this.update = null;
    this.updatePending = false;
    this.route = 'welcome';
    this.back = [];
    this.selected = null;
    this.added = null;
    this.nav = { kind: 'none', from: null };
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

  /** Go to a screen. `from` is the control that was pressed: in setup the S1 shape grows out
   * of it (src/lib/flow.ts). */
  go(r: Route, opts: { replace?: boolean; from?: Element | null } = {}): void {
    if (!opts.replace) this.back = [...this.back, this.route];
    this.nav = { kind: 'forward', from: opts.from ?? null };
    this.route = r;
    this.afterPairing();
  }

  /** A pairing screen was left while an update waited: show it now. */
  private afterPairing(): void {
    if (this.updatePending && this.route !== 'show' && this.route !== 'enter') {
      this.updatePending = false;
      queueMicrotask(() => this.openUpdate());
    }
  }

  goBack(fallback: Route = 'home'): void {
    const prev = this.back[this.back.length - 1];
    this.back = this.back.slice(0, -1);
    this.nav = { kind: 'back', from: null };
    this.route = prev ?? fallback;
    this.afterPairing();
  }

  /** The agent is gone because the update helper stopped it: keep the update screen. */
  get updating(): boolean {
    return this.route === 'update' && this.update?.state === 'applying';
  }

  /** The screen on show: Home turns into Needs attention when the agent says so. */
  get view(): View {
    if (this.route === 'home' && this.status?.state === 'needs-attention') return 'attention';
    return this.route;
  }

  /** True when the permissions screen (L02) has rows to ask: only when macOS blocks something. */
  get needsPerms(): boolean {
    return askRows(this.setup).length > 0;
  }

  get thisDevice(): Device | undefined {
    return this.devices.find((d) => d.this);
  }
}

export const store = new Store();
