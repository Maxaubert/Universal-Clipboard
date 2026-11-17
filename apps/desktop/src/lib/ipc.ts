// The only way the window talks to the agent: JSON-line calls proxied by Tauri commands
// (src-tauri) to the pipe or socket. Outside Tauri (the Vite build in tests and in a browser)
// a mock agent answers instead. There is no call that sends or fetches clipboard content.
import type { AgentEvent, FirewallCheck, IpcFailure } from './types';

export interface Api {
  call<T = unknown>(name: string, params?: Record<string, unknown>): Promise<T>;
  onEvent(cb: (e: AgentEvent) => void): () => void;
  /** Windows: add the firewall rule (the helper, one UAC prompt), then `firewall.check`. The
   * same path as `uc firewall add`. Not an agent call: the window shell runs the helper. */
  firewallAdd(): Promise<FirewallCheck>;
  /** The screen the window was started for (`--route update` from the agent), or null. */
  launchRoute(): Promise<string | null>;
  /** "Copy files..." and "Copy folder..." on Home: the system file picker (files, or a folder:
   * Windows cannot pick both in one dialog). The picked paths, or none when cancelled. Not an
   * agent call: the window shell opens the picker; `files.copy` then sends the paths. */
  pickPaths(folder: boolean): Promise<string[]>;
  /** The tray panel's actions (tray spec 3.3). They run in the shell, so they finish even
   * when the panel closes. */
  tray: {
    /** An agent call that never starts the agent (status, devices.list, pause.set,
     * transfer.stop). A stopped agent fails with `agent-down`. */
    call<T = unknown>(name: string, params?: Record<string, unknown>): Promise<T>;
    /** The panel measured itself (CSS px, clear edge included): place and show it. */
    ready(height: number): Promise<void>;
    /** Close the panel, open the picker, copy what was picked. */
    copy(folder: boolean): Promise<void>;
    /** Open the window, on `settings`, `vpn` or Home (null). */
    open(route: string | null): Promise<void>;
    /** Stop the agent (never starts one) and end the app. */
    quit(): Promise<void>;
  };
  window: {
    minimize(): Promise<void>;
    toggleMaximize(): Promise<void>;
    close(): Promise<void>;
  };
  inTauri: boolean;
}

/** Calls the window may make. Anything else is refused before it leaves the window. */
export const ALLOWED_CALLS = [
  'status',
  'subscribe',
  'pair.start',
  'pair.join',
  'pair.cancel',
  'devices.list',
  'devices.rename',
  'devices.remove',
  'pause.set',
  'settings.get',
  'settings.set',
  'files.copy',
  'transfer.stop',
  'diagnostics',
  'leave',
  'setup.status',
  'setup.step',
  'firewall.check',
  'version',
  'update.status',
  'update.apply',
  'update.later',
  'quit',
] as const;

export function isFailure(e: unknown): e is IpcFailure {
  return typeof e === 'object' && e !== null && 'message' in e && 'code' in e;
}

export function failureText(e: unknown): string {
  if (isFailure(e)) return e.message;
  if (e instanceof Error) return e.message;
  return String(e);
}

function tauriApi(): Api {
  let invokeP: Promise<typeof import('@tauri-apps/api/core')> | null = null;
  const core = () => (invokeP ??= import('@tauri-apps/api/core'));
  const win = async () => (await import('@tauri-apps/api/window')).getCurrentWindow();
  return {
    inTauri: true,
    async call<T>(name: string, params: Record<string, unknown> = {}) {
      if (!(ALLOWED_CALLS as readonly string[]).includes(name)) {
        throw { code: 'unknown-call', message: `Unknown call "${name}".` } satisfies IpcFailure;
      }
      const { invoke } = await core();
      return (await invoke('ipc_call', { call: name, params })) as T;
    },
    async firewallAdd() {
      const { invoke } = await core();
      return (await invoke('firewall_add')) as FirewallCheck;
    },
    async launchRoute() {
      const { invoke } = await core();
      return ((await invoke('launch_route')) as string | null) ?? null;
    },
    async pickPaths(folder: boolean) {
      const { invoke } = await core();
      return ((await invoke('pick_paths', { folder })) as string[] | null) ?? [];
    },
    tray: {
      call: async <T,>(name: string, params: Record<string, unknown> = {}) =>
        (await (await core()).invoke('panel_call', { call: name, params })) as T,
      ready: async (height: number) => void (await (await core()).invoke('panel_ready', { height })),
      copy: async (folder: boolean) => void (await (await core()).invoke('tray_copy', { folder })),
      open: async (route: string | null) => void (await (await core()).invoke('tray_open', { route })),
      quit: async () => void (await (await core()).invoke('tray_quit')),
    },
    onEvent(cb) {
      let off: (() => void) | null = null;
      let dead = false;
      void import('@tauri-apps/api/event').then(async ({ listen }) => {
        const u = await listen<AgentEvent>('uc-event', (e) => cb(e.payload));
        if (dead) u();
        else off = u;
      });
      return () => {
        dead = true;
        off?.();
      };
    },
    window: {
      minimize: async () => (await win()).minimize(),
      toggleMaximize: async () => (await win()).toggleMaximize(),
      close: async () => (await win()).close(),
    },
  };
}

let api: Api | null = null;

export async function getApi(): Promise<Api> {
  if (api) return api;
  const inTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
  if (inTauri) {
    api = tauriApi();
  } else {
    const { createMockApi } = await import('./mock');
    api = createMockApi();
  }
  return api;
}

/** For tests: use this API instead. */
export function setApi(a: Api | null): void {
  api = a;
}
