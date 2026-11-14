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
  /** A non-blocking note (for example a missing Windows firewall rule) and its fix. */
  note?: { code: string; text: string; fix: string } | null;
  devices: number;
  connected: number;
  paused: boolean;
  this_device: string;
  version: string;
  /** The last copies, newest first: facts only, never content (agent `status`). */
  activity?: Activity[];
}

/** One copy that moved, as facts: "Text", "received", "Mac mini", when. Also the data of the
 * agent's `activity` event. */
export interface Activity {
  kind: string;
  direction: 'sent' | 'received' | 'not sent' | string;
  device: string | null;
  time_ms: number;
  note: string | null;
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

/** Reply of `firewall.check`. */
export interface FirewallCheck {
  rule: 'present' | 'absent' | 'not-needed';
  incoming: boolean;
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
  /** Set while the short code is off: "Too many wrong codes. Try again in 5 minutes." */
  notice: string | null;
  /** Time left in the lockout, 0 when the code works. */
  lockout_ms?: number;
}

/** Reply of `update.status`, also the data of the agent's `update` event. The agent checks
 * once when it starts and installs nothing without the user. */
export interface UpdateStatus {
  enabled: boolean;
  current: string;
  available: string | null;
  failed_version: string | null;
  notes_url: string | null;
  state: 'none' | 'available' | 'applying' | 'failed' | 'updated';
  dismissed: boolean;
  checked: boolean;
  checks: number;
  last_check_ms: number;
  last_error: string | null;
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
