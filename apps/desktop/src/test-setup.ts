import '@testing-library/jest-dom/vitest';
import { afterEach } from 'vitest';
import { cleanup } from '@testing-library/svelte';
import { setApi } from './lib/ipc';
import { store } from './lib/store.svelte';

afterEach(() => {
  cleanup();
  store.dispose();
  setApi(null);
});
