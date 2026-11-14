import { mount } from 'svelte';
import './app.css';
import App from './App.svelte';
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

mount(App, { target: document.getElementById('app')! });
