// The only way the window talks to the agent: JSON-line calls proxied by Tauri commands
// (src-tauri) to the pipe or socket. Outside Tauri (the Vite build in tests and in a browser)
// a mock agent answers instead. There is no call that sends or fetches clipboard content.
import type { AgentEvent, IpcFailure } from './types';

export interface Api {
  call<T = unknown>(name: string, params?: Record<string, unknown>): Promise<T>;
  onEvent(cb: (e: AgentEvent) => void): () => void;
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
  'diagnostics',
  'leave',
  'setup.status',
  'setup.step',
  'version',
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
