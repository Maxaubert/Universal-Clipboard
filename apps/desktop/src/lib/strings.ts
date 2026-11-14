// All English text of the window, in one place (ready for a Norwegian file later).
export const S = {
  appName: 'Universal Clipboard',
  minimize: 'Minimize',
  maximize: 'Maximize',
  close: 'Close',
  back: 'Back',

  welcomeTitle: 'Your latest copy, on every device.',
  welcomeSub: 'Copy once. Paste anywhere.',
  getStarted: 'Get started',

  permTitle: 'Two permissions',
  permSub: 'You only do this once.',
  allow: 'Allow',
  rowTitles: { autostart: 'Start when you sign in', firewall: 'Allow through the firewall' } as Record<string, string>,
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

  rename: 'Rename',
  save: 'Save',
  removeDevice: 'Remove this device',
  removeAsk: (name: string) => `Remove ${name}?`,
  removeYes: 'Remove',
  renameLabel: 'Device name',

  settingsTitle: 'Settings',
  startAtSignIn: 'Start at sign-in',
  startAtSignInLabel: 'Start when you sign in',
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

export function thisLabel(os: string): string {
  const o = os.toLowerCase();
  if (o.startsWith('windows')) return 'This PC';
  if (o === 'macos') return 'This Mac';
  return 'This computer';
}
