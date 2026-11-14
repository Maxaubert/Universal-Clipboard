import { mount } from 'svelte';
import './app.css';
import App from './App.svelte';
import { isMac } from './lib/platform';

if ('__TAURI_INTERNALS__' in window) document.documentElement.classList.add('tauri');
// macOS: native traffic lights and rounded corners (tauri.macos.conf.json), no drawn controls.
if (isMac) document.documentElement.classList.add('mac');

mount(App, { target: document.getElementById('app')! });
