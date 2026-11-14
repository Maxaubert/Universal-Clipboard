import { mount } from 'svelte';
import './app.css';
import App from './App.svelte';

if ('__TAURI_INTERNALS__' in window) document.documentElement.classList.add('tauri');

mount(App, { target: document.getElementById('app')! });
