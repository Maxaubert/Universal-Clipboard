// All English text of the window, in one place (ready for a Norwegian file later).
import { isMac } from './platform';

export const S = {
  appName: 'Universal Clipboard',
  minimize: 'Minimize',
  maximize: 'Maximize',
  close: 'Close',
  back: 'Back',

  welcomeTitle: 'Your latest copy, on every device.',
  welcomeSub: 'Copy once. Paste anywhere.',
  getStarted: 'Get started',

  // Rows appear only when the OS blocks something (macOS: up to two), so the count varies.
  permTitle: (n: number) => `${['No', 'One', 'Two', 'Three', 'Four'][n] ?? n} permission${n === 1 ? '' : 's'}`,
  permSub: 'You only do this once.',
  allow: 'Allow',
  rowTitles: {} as Record<string, string>,
  allowed: 'Allowed',
  continue: 'Continue',

  addTitle: 'Add a device',
  addSub: 'Show a code here, or type one in.',
  showCode: 'Show a code',
  enterCode: 'Enter a code',

  pairTitle: 'Pair a device',
  qrLabel: 'QR code to scan with your other device',
  worksOn: 'Works on Mac, Windows and Android',
  otherNetwork: 'On another network?',
  pairWaiting: 'Making a code...',

  enterTitle: 'Enter the code',
  enterSub: 'It is on your other device.',
  connect: 'Connect',
  connecting: 'Connecting...',
  leaveOtherTitle: 'Leave your other devices and join these?',
  leaveOtherYes: 'Leave and join',
  cancel: 'Cancel',

  addedSuffix: ' is added',
  addedSub: 'Your latest copy is on both.',
  goToDevices: 'Go to my devices',
  addAnother: 'Add another',

  pause: 'Pause',
  resume: 'Resume',
  settings: 'Settings',
  addDevice: 'Add a device',
  working: 'Working',
  paused: 'Paused',
  tileConnected: 'Connected',
  tileAsleep: 'Asleep',
  tileBlocked: 'Blocked',
  tilePaused: 'Paused',

  lastCopy: 'Last copy',

  rename: 'Rename',
  save: 'Save',
  removeDevice: 'Remove this device',
  removeAsk: (name: string) => `Remove ${name}?`,
  removeYes: 'Remove',
  renameLabel: 'Device name',

  settingsTitle: 'Settings',
  // macOS says "login" (System Settings > General > Login Items); Windows says "sign in".
  startAtSignIn: isMac ? 'Open at login' : 'Start at sign-in',
  startAtSignInLabel: isMac ? 'Open when you log in' : 'Start when you sign in',
  skipPasswords: 'Skip passwords',
  skipPasswordsLabel: 'Skip password manager copies, always on',
  version: (v: string) => `Version ${v}`,
  removeThis: 'Remove',
  leaveAsk: 'Remove this device and forget everything?',
  leaveYes: 'Remove and forget',

  attentionTitle: 'Needs attention',
  fix: 'Fix',

  vpnTitle: "Can't reach your other device?",
  vpnSub: 'Your VPN may be blocking your local network.',
  vpnTip: 'Turn on Local network sharing in your VPN',
  vpnBack: 'Back',

  agentDown: 'The background service is not running.',
  retry: 'Try again',
} as const;

/** "just now", "2 minutes ago", "1 hour ago", "3 days ago". */
export function ago(ms: number, now: number): string {
  const s = Math.max(0, Math.floor((now - ms) / 1000));
  const n = (v: number, u: string) => `${v} ${u}${v === 1 ? '' : 's'} ago`;
  if (s < 60) return 'just now';
  if (s < 3600) return n(Math.floor(s / 60), 'minute');
  if (s < 86400) return n(Math.floor(s / 3600), 'hour');
  return n(Math.floor(s / 86400), 'day');
}

/** A copy as facts, never content: "Text, from Mac mini, just now" or "Image, sent, 2 minutes ago". */
export function copyFact(a: { kind: string; direction: string; device: string | null; time_ms: number }, now: number): string {
  const kind = a.kind.charAt(0).toUpperCase() + a.kind.slice(1);
  const way = a.direction === 'received' && a.device ? `from ${a.device}` : a.direction;
  return `${kind}, ${way}, ${ago(a.time_ms, now)}`;
}

export function thisLabel(os: string): string {
  const o = os.toLowerCase();
  if (o.startsWith('windows')) return 'This PC';
  if (o === 'macos') return 'This Mac';
  return 'This computer';
}
