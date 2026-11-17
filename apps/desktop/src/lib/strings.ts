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

  // File sync (spec 2026-11-16): the picker buttons on Home and the progress line.
  copyFiles: 'Copy files...',
  copyFolder: 'Copy folder...',
  stop: 'Stop',
  preparing: 'Preparing files...',
  sendingTo: (d: string) => `Sending to ${d}...`,
  receivingFrom: (d: string) => `Receiving from ${d}...`,

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
  trayIcon: isMac ? 'Show icon in the menu bar' : 'Show icon in the taskbar',
  trayIconSub: 'Copy files, pause and quit from one click.',
  autoFileCopy: 'Copy smaller files automatically',
  autoFileCopySub: 'Files and folders you copy, up to the size below, go to your other devices by themselves.',
  fileLimit: 'Largest automatic file copy',
  fileLimitLabel: 'Largest automatic file copy, in MB, on this device',
  fileLimitSub: 'On this device. Bigger copies: Copy files... on Home.',
  mb: 'MB',
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

  // Update: U1 "One shape" screen and U3 "Home card" (Max's pick 2026-11-14,
  // docs/design/desktop/motion/update.html).
  updateReady: (v: string) => `Version ${v} is ready.`,
  updateReadySub: 'A newer Universal Clipboard is ready to install.',
  updateHave: (cur: string) => `You have ${cur}.`,
  updateNow: 'Update now',
  updateLater: 'Later',
  updating: 'Updating...',
  updatingSub: 'The app closes and opens again by itself.',
  updated: (v: string) => `Updated to ${v}`,
  updatedSub: 'You have the newest version.',
  updateDone: 'Done',
  updateFailed: 'The update did not work.',
  updateFailedSub: (cur: string) => `You still have ${cur}.`,
  updateTryAgain: 'Try again',
  updateNone: (cur: string) => `Version ${cur} is the newest.`,
  updateOk: 'OK',

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

/** A size as the agent writes it: "512 bytes", "12 KB", "12 MB", "1.5 GB" (1 KB = 1024 bytes,
 * rounded to the nearest unit). */
export function sizeText(bytes: number): string {
  const K = 1024;
  const round = (unit: number) => Math.max(1, Math.round(bytes / unit));
  if (bytes === 1) return '1 byte';
  if (bytes < K) return `${bytes} bytes`;
  if (bytes < K * K) return `${round(K)} KB`;
  if (bytes < K * K * K) return `${round(K * K)} MB`;
  const tenths = round((K * K * K) / 10);
  return `${Math.floor(tenths / 10)}.${tenths % 10} GB`;
}

/** The progress line of a file copy: what moves, and how much ("45 MB of 300 MB, 3 files"). */
export function transferText(t: { phase: string; device: string | null; done: number; total: number; files: number }): {
  title: string;
  amount: string;
} {
  const d = t.device ?? '';
  const title = t.phase === 'sending' ? S.sendingTo(d) : t.phase === 'receiving' ? S.receivingFrom(d) : S.preparing;
  const files = t.files === 1 ? '1 file' : `${t.files} files`;
  const amount = t.total > 0 ? `${sizeText(t.done)} of ${sizeText(t.total)}, ${files}` : files;
  return { title, amount };
}

export function thisLabel(os: string): string {
  const o = os.toLowerCase();
  if (o.startsWith('windows')) return 'This PC';
  if (o === 'macos') return 'This Mac';
  return 'This computer';
}
