import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// GitHub Pages serves this project from https://<user>.github.io/palworld-save-edit/,
// not from the domain root, so built asset URLs need that prefix. `BASE_PATH` lets CI
// override it (and keeps `npm run dev` at `/`).
const base = process.env.BASE_PATH ?? '/';

export default defineConfig({
  base,
  plugins: [svelte()],
  build: {
    // The wasm blob is ~330 KB; Vite's default 500 KB warning would fire on it every
    // build for no useful reason.
    chunkSizeWarningLimit: 1024,
  },
  worker: {
    // The save worker is a module worker (it uses `import` for the wasm glue).
    format: 'es',
  },
});
