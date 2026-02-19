/**
 * Performance benchmark: injectCssTokens()
 *
 * Verifies that the synchronous core of injectCssTokens() — which drives
 * 55+ CSS custom properties via WASM OKLCH computation — stays within the
 * 16ms frame budget on warm iterations.
 *
 * Budget rationale:
 *   - 16ms = 1 frame @ 60 fps (single-frame budget for first paint)
 *   - p95 < 16ms ensures 95% of renders complete in a single frame
 *   - p50 < 8ms provides comfortable headroom for compositing + layout
 *
 * Architecture:
 *   - globalSetup copies momoto_ui_core.{js,wasm} into tests/fixtures/
 *   - playwright.config.ts serves tests/fixtures/ via npx serve on :4399
 *   - perf-bench.html loads WASM, runs 5 warm-up + 30 measured iterations,
 *     then exposes statistics via window.__PERF_RESULT__
 */

import { test, expect } from '@playwright/test';

interface PerfResult {
  samples: number[];
  p50:     number;
  p95:     number;
  p99:     number;
  mean:    number;
  min:     number;
  max:     number;
  iters:   number;
}

test.describe('injectCssTokens() performance', () => {
  test('p95 < 16ms — single-frame budget', async ({ page }) => {
    // Navigate to the benchmark fixture
    await page.goto('/perf-bench.html');

    // Wait until the benchmark script exposes its results.
    // Timeout is generous because WASM init can be slow on first cold load.
    const resultHandle = await page.waitForFunction(
      () => (window as Window & { __PERF_RESULT__?: PerfResult }).__PERF_RESULT__,
      { timeout: 60_000 },
    );

    const result: PerfResult = await resultHandle.jsonValue() as PerfResult;

    // ── Print timing table ────────────────────────────────────────────────
    console.log('\n  injectCssTokens() — WASM colour-token derivation benchmark');
    console.log(`  iterations : ${result.iters} (+ 5 warm-up discarded)`);
    console.log(`  min        : ${result.min.toFixed(2)} ms`);
    console.log(`  mean       : ${result.mean.toFixed(2)} ms`);
    console.log(`  p50        : ${result.p50.toFixed(2)} ms`);
    console.log(`  p95        : ${result.p95.toFixed(2)} ms`);
    console.log(`  p99        : ${result.p99.toFixed(2)} ms`);
    console.log(`  max        : ${result.max.toFixed(2)} ms`);

    // ── Assertions ────────────────────────────────────────────────────────
    // P95 must fit within one 60-fps frame (16.67 ms).
    expect(
      result.p95,
      `p95 ${result.p95.toFixed(2)}ms exceeds 16ms frame budget. ` +
      `Check for WASM bridge regression or excessive object allocation in injectCssTokens().`,
    ).toBeLessThan(16);

    // P50 must fit within half a frame — comfortable headroom for layout.
    expect(
      result.p50,
      `p50 ${result.p50.toFixed(2)}ms exceeds 8ms. ` +
      `Median performance has regressed — investigate batch_derive_tokens() call pattern.`,
    ).toBeLessThan(8);

    // Sanity: we ran the expected number of iterations.
    expect(result.iters).toBe(30);

    // Sanity: results are not zero (WASM actually ran).
    expect(result.mean).toBeGreaterThan(0);
  });

  test('CSS tokens are actually injected', async ({ page }) => {
    // Secondary correctness check: verify the WASM actually computed
    // valid hex values and injected them into :root custom properties.
    await page.goto('/perf-bench.html');

    await page.waitForFunction(
      () => (window as Window & { __PERF_RESULT__?: PerfResult }).__PERF_RESULT__,
      { timeout: 60_000 },
    );

    // Read a sample of injected tokens from document.documentElement
    const tokens = await page.evaluate(() => {
      const style = getComputedStyle(document.documentElement);
      return {
        btnIdle:    style.getPropertyValue('--m-btn-idle').trim(),
        accentIdle: style.getPropertyValue('--m-accent-idle').trim(),
        surface0:   style.getPropertyValue('--m-surface-0').trim(),
        text1:      style.getPropertyValue('--m-text-1').trim(),
        brandFire:  style.getPropertyValue('--m-brand-fire').trim(),
        glowFire:   style.getPropertyValue('--m-glow-fire').trim(),
      };
    });

    // All token values should be non-empty strings
    for (const [name, value] of Object.entries(tokens)) {
      expect(value, `Token --m-${name} must not be empty`).not.toBe('');
    }

    // Hex colour tokens should match #rrggbb format
    const hexRe = /^#[0-9a-fA-F]{6}$/;
    expect(tokens.btnIdle,    '--m-btn-idle must be a valid hex colour').toMatch(hexRe);
    expect(tokens.accentIdle, '--m-accent-idle must be a valid hex colour').toMatch(hexRe);
    expect(tokens.surface0,   '--m-surface-0 must be a valid hex colour').toMatch(hexRe);
    expect(tokens.text1,      '--m-text-1 must be a valid hex colour').toMatch(hexRe);
    expect(tokens.brandFire,  '--m-brand-fire must be a valid hex colour').toMatch(hexRe);

    // Glow token should be rgba format
    expect(tokens.glowFire, '--m-glow-fire must be an rgba() value').toMatch(/^rgba\(/);
  });
});
