import { defineConfig, devices } from '@playwright/test';

/**
 * Browser end-to-end tests.
 *
 * Everything below this layer is already covered: Rust unit and fixture tests for the
 * format, wasm-bindgen tests for the boundary. What none of them can catch is the app
 * failing to *run* — a worker that won't resolve its wasm URL, a file input wired to
 * nothing, a screen that throws on mount. Until this existed the UI had never been
 * executed at all, only typechecked and built.
 *
 * Runs against the production build via `vite preview`, not the dev server, so the
 * thing under test is what actually deploys — including the module worker and the
 * wasm asset, which is where the interesting failures live.
 */
export default defineConfig({
  testDir: './e2e',
  // A save has to decompress and parse before anything renders; the default 5s
  // assertion timeout is tight for that on a cold CI runner.
  expect: { timeout: 10_000 },
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [['html', { open: 'never' }], ['list']] : 'list',
  use: {
    baseURL: 'http://localhost:4173',
    trace: 'on-first-retry',
  },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
  webServer: {
    // NOT `npm run build` — that rebuilds the wasm every run. CI builds once, then
    // runs this. `BASE_PATH` is deliberately unset so `base` stays `/`; a CI build
    // sets it to `/<repo>/` for Pages and would 404 every asset here.
    command: 'npx vite preview --port 4173',
    port: 4173,
    reuseExistingServer: !process.env.CI,
    timeout: 60_000,
  },
});
