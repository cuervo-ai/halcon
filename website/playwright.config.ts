import { defineConfig } from '@playwright/test';

/**
 * Playwright configuration for Halcón website performance benchmarks.
 *
 * Run:  npm run test:perf
 * CI:   PLAYWRIGHT_BROWSERS_PATH=0 npx playwright install chromium && npm run test:perf
 */
export default defineConfig({
  testDir:     './tests/perf',
  timeout:     90_000,   // generous — WASM init can take a few seconds in CI
  retries:     0,        // perf benchmarks must not retry (timing distortion)
  workers:     1,        // sequential: no CPU contention during measurement
  globalSetup: './tests/global-setup.ts',

  use: {
    baseURL:        'http://localhost:4399',
    // Headless Chromium for consistent timing (no compositor overhead)
    headless:       true,
    // Disable GPU acceleration for deterministic CPU-only timing
    launchOptions: {
      args: ['--disable-gpu', '--no-sandbox', '--disable-dev-shm-usage'],
    },
  },

  // Serve tests/fixtures/ over HTTP — WASM fetch() requires a real origin
  webServer: {
    command:             'npx --yes serve ./tests/fixtures --listen 4399 --no-clipboard',
    port:                4399,
    reuseExistingServer: !process.env['CI'],
    timeout:             20_000,
    // globalSetup runs before webServer, so fixtures are ready
  },

  reporter: [
    ['list'],
    // Machine-readable output for CI performance tracking
    ['json', { outputFile: 'tests/perf/results.json' }],
  ],
});
