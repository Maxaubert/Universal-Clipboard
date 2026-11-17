import { mount } from 'svelte';
import './app.css';
import App from './App.svelte';
import Panel from './Panel.svelte';
import { isMac } from './lib/platform';

if ('__TAURI_INTERNALS__' in window) {
  document.documentElement.classList.add('tauri');
  // No web context menu (Copy, Select all, Reload) outside text fields (Max 2026-11-14).
  document.addEventListener('contextmenu', (e) => {
    const t = e.target as HTMLElement | null;
    if (!t?.closest('input, textarea')) e.preventDefault();
  });
}
// macOS: native traffic lights and rounded corners (tauri.macos.conf.json), no drawn controls.
if (isMac) document.documentElement.classList.add('mac');

// The tray panel (tray spec 3.2): the same bundle, `#panel`, on a clear window.
const panel = location.hash === '#panel';
if (panel) document.documentElement.classList.add('panel');
mount(panel ? Panel : App, { target: document.getElementById('app')! });
