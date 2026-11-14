// Shapes of the agent's IPC replies (docs/agent.md, plan 3.2). The window holds no keys and
// no clipboard content: only names, states and short texts.

export type StatusState =
  | 'working'
  | 'paused'
  | 'needs-attention'
  | 'not-set-up'
  | 'no-peers';

export interface Status {
  state: StatusState;
  text: string;
  reason: string | null;
  fix: string | null;
  fix_code: string | null;
  devices: number;
  connected: number;
  paused: boolean;
  this_device: string;
  version: string;
}

export type DeviceState =
  | 'This device'
  | 'Connected'
  | 'Asleep or away'
  | 'Blocked by a VPN or firewall';

export interface Device {
  id: string;
  name: string;
  os: string;
  this: boolean;
  state: DeviceState;
}

export interface SetupRow {
  row: string;
  title: string;
  why: string;
  if_refused?: string;
  state: 'todo' | 'done' | 'refused';
}

export interface SetupStatus {
  rows: SetupRow[];
  done: boolean;
  sandboxed?: boolean;
}

export interface SettingValue {
  value: string;
  label: string;
}
export type Settings = Record<string, SettingValue>;

export interface PairOffer {
  code: string | null;
  qr: string | null;
  expires_ms: number;
  notice: string | null;
}

export interface IpcFailure {
  code: string;
  message: string;
}

export interface AgentEvent {
  event: string;
  data: Record<string, unknown>;
}

export type OsKind = 'windows' | 'apple' | 'linux' | 'android';

export function osKind(os: string): OsKind {
  const o = os.toLowerCase();
  if (o.startsWith('windows')) return 'windows';
  if (o === 'macos' || o === 'iphone' || o === 'ipad') return 'apple';
  if (o.startsWith('android')) return 'android';
  return 'linux';
}
