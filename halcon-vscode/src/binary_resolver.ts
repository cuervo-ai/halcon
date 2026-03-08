/**
 * Resolves the correct Halcon binary path for the current platform + arch.
 *
 * Resolution order:
 * 1. `halcon.binaryPath` configuration override (user-specified)
 * 2. Bundled binary in `<extensionPath>/bin/`
 * 3. `halcon` on PATH (fallback for development)
 */

import * as fs from 'fs';
import * as path from 'path';
import * as vscode from 'vscode';

/** Maps platform+arch to the bundled binary filename. */
const BINARY_MAP: Record<string, string> = {
  'darwin-arm64': 'halcon-darwin-arm64',
  'darwin-x64':   'halcon-darwin-x64',
  'linux-x64':    'halcon-linux-x64',
  'win32-x64':    'halcon-win32-x64.exe',
};

export interface BinaryInfo {
  /** Absolute path to the halcon binary. */
  path: string;
  /** true if the binary was found via config override or bundled bin/. */
  bundled: boolean;
}

/**
 * Resolve the Halcon binary path.
 *
 * @param extensionPath - `context.extensionPath` from the extension activation context.
 * @throws if no binary can be found for the current platform.
 */
export function resolveBinary(extensionPath: string): BinaryInfo {
  const config = vscode.workspace.getConfiguration('halcon');
  const override = config.get<string>('binaryPath', '').trim();

  // 1. User override.
  if (override) {
    if (!fs.existsSync(override)) {
      throw new Error(`halcon.binaryPath is set but file not found: ${override}`);
    }
    return { path: override, bundled: false };
  }

  // 2. Bundled binary.
  const key = `${process.platform}-${process.arch}`;
  const filename = BINARY_MAP[key];
  if (filename) {
    const bundledPath = path.join(extensionPath, 'bin', filename);
    if (fs.existsSync(bundledPath)) {
      ensureExecutable(bundledPath);
      return { path: bundledPath, bundled: true };
    }
  }

  // 3. PATH fallback.
  const pathFallback = process.platform === 'win32' ? 'halcon.exe' : 'halcon';
  return { path: pathFallback, bundled: false };
}

/** Ensure the binary has execute permissions on POSIX platforms. */
function ensureExecutable(binaryPath: string): void {
  if (process.platform !== 'win32') {
    try {
      fs.chmodSync(binaryPath, 0o755);
    } catch {
      // Non-fatal: may already be executable.
    }
  }
}

/** Return a human-readable label for the current platform. */
export function platformLabel(): string {
  return `${process.platform}-${process.arch}`;
}
