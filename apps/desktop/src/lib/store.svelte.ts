import { failureText, getApi, type Api } from './ipc';
import type { AgentEvent, Device, Settings, SetupStatus, Status, Transfer, UpdateStatus } from './types';

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
  /** The file copy that moves now (the progress line on Home), or null. */
  transfer = $state<Transfer | null>(null);
  /** The agent's update status (`update.status`), or null when it has none. */
  update = $state<UpdateStatus | null>(null);
  /** The update screen waits for pairing to end before it shows. */
  updatePending = $state(false);
  /** Update now was pressed on Home's card (U3): the update screen starts the install at once,
   * and a failure goes back to Home, where the card shows it. Plain fields, read once. */
  updateStart = false;
  updateFrom: 'home' | null = null;
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
      // The agent is back while an update ran (a window that lived through it): ask how it went.
      if (e.event === 'status' && this.update?.state === 'applying') void this.refreshUpdate();
      if (e.event === 'transfer') {
        const t = e.data as unknown as Transfer;
        this.transfer = t.phase === 'idle' ? null : t;
      }
      if (e.event === 'update') {
        this.update = e.data as unknown as UpdateStatus;
        if (this.loaded && this.shouldAsk()) this.openUpdate();
      }
      // The shell asks for a screen: the agent started the window again with `--route update`.
      if (e.event === 'open' && e.data.route === 'update' && this.loaded) this.openUpdate();
      if (e.event === 'open' && this.loaded) this.openFromTray(e.data.route as string | undefined);
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
    const launch = await this.api.launchRoute();
    if (launch === 'update' && this.update?.state !== 'none' && !this.update?.dismissed) {
      this.back = [this.route];
      this.route = 'update';
    } else if ((launch === 'settings' || launch === 'vpn') && this.route === 'home') {
      this.back = ['home'];
      this.route = launch;
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

  /** True when the agent's news should take the screen: a ready, failed or done update, no Later. */
  shouldAsk(): boolean {
    const u = this.update;
    return !!u && (u.state === 'available' || u.state === 'failed' || u.state === 'updated') && !u.dismissed && this.route !== 'update';
  }

  /** The U3 card on Home: a ready or failed update the user has not put off with Later. */
  get updateCard(): 'available' | 'failed' | null {
    const u = this.update;
    if (!u || u.dismissed) return null;
    return u.state === 'available' || u.state === 'failed' ? u.state : null;
  }

  /** Show the update news: on Home as the card (U3), elsewhere the update screen (U1), and
   * during a pairing right after it. */
  /** The tray asked for a screen (Settings, VPN help) while the window is open. */
  openFromTray(route: string | undefined): void {
    if ((route === 'settings' || route === 'vpn') && this.route !== route && this.view !== 'welcome') {
      this.go(route);
    }
  }

  openUpdate(): void {
    if (this.route === 'update') return;
    if (this.route === 'show' || this.route === 'enter') {
      this.updatePending = true;
      return;
    }
    this.updatePending = false;
    if (this.view === 'home' && this.updateCard) return;
    this.updateFrom = null;
    this.go('update');
  }

  /** Update now (or Try again) on Home's card: the update screen takes over from the button. */
  startUpdate(from: Element | null): void {
    this.updateStart = true;
    this.updateFrom = 'home';
    this.go('update', { from });
  }

  /** Leave the update screen (Later, Done, OK). */
  leaveUpdate(): void {
    this.updateFrom = null;
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
    this.transfer = null;
    this.updatePending = false;
    this.updateStart = false;
    this.updateFrom = null;
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
      this.transfer = status.transfer ?? null;
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
