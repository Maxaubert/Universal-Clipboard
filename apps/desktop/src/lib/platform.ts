// Which window frame to draw. In Tauri the webview's user agent says the OS; in the browser
// and test build `?platform=mac` previews the macOS frame (default: Windows, the locked design).
export type Frame = 'mac' | 'windows';

export function detectFrame(): Frame {
  if (typeof window === 'undefined') return 'windows';
  if ('__TAURI_INTERNALS__' in window) {
    return /Macintosh|Mac OS X/.test(navigator.userAgent) ? 'mac' : 'windows';
  }
  return new URLSearchParams(location.search).get('platform') === 'mac' ? 'mac' : 'windows';
}

export const frame: Frame = detectFrame();
/** The real app (not the browser or test build). */
export const inTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
export const isMac = frame === 'mac';
