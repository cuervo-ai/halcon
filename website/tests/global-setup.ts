/**
 * Playwright global setup for perf benchmarks.
 *
 * Copies the WASM bindings JS + binary into tests/fixtures/ so that
 * `npx serve tests/fixtures` can host them alongside perf-bench.html.
 * The copies are generated artifacts and are listed in .gitignore.
 */
import { copyFileSync, mkdirSync } from 'fs';
import { join, dirname } from 'path';
import { fileURLToPath } from 'url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const root      = join(__dirname, '..');

export default function globalSetup(): void {
  const fixturesDir = join(__dirname, 'fixtures');
  mkdirSync(fixturesDir, { recursive: true });

  const copies: [string, string][] = [
    // WASM JS bindings (TypeScript-stripped plain JS)
    [
      join(root, 'src', 'lib', 'momoto', 'momoto_ui_core.js'),
      join(fixturesDir, 'momoto_ui_core.js'),
    ],
    // WASM binary — the engine itself
    [
      join(root, 'public', 'momoto_ui_core_bg.wasm'),
      join(fixturesDir, 'momoto_ui_core_bg.wasm'),
    ],
  ];

  for (const [src, dst] of copies) {
    copyFileSync(src, dst);
    console.log(`[global-setup] copied ${src.replace(root, '.')} → ${dst.replace(root, '.')}`);
  }
}
