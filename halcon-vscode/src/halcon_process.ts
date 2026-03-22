/**
 * Manages the Halcon subprocess communicating via newline-delimited JSON-RPC.
 *
 * Protocol:
 *   Extension → Halcon (stdin):  {id?, method, params?}\n
 *   Halcon → Extension (stdout): {event, data?}\n  or  {id, result?, error?}\n
 *
 * Health check: ping/pong with 5 s timeout; auto-restart on failure.
 * Windows: spawns via `cmd /c` wrapper to avoid stdio pipe buffering issues.
 */

import * as cp from 'child_process';
import * as readline from 'readline';
import * as vscode from 'vscode';

export interface ChatParams {
  message: string;
  context?: Record<string, unknown>;
}

export type EventHandler = (event: string, data: unknown) => void;
export type ErrorHandler = (error: Error) => void;

export class HalconProcess {
  private proc: cp.ChildProcess | null = null;
  private rl: readline.Interface | null = null;
  private nextId = 1;
  private pendingPings = new Map<number, { resolve: () => void; reject: (e: Error) => void }>();
  private onEvent: EventHandler;
  private onError: ErrorHandler;
  private readonly binaryPath: string;
  private readonly extraArgs: string[];
  private restartCount = 0;
  private readonly maxRestarts = 5;

  constructor(
    binaryPath: string,
    extraArgs: string[],
    onEvent: EventHandler,
    onError: ErrorHandler,
  ) {
    this.binaryPath = binaryPath;
    this.extraArgs = extraArgs;
    this.onEvent = onEvent;
    this.onError = onError;
  }

  /** Start the subprocess. Safe to call multiple times (idempotent if already running). */
  async start(): Promise<void> {
    if (this.proc && !this.proc.killed) {
      return;
    }

    const { cmd, args } = this.buildCommand();

    this.proc = cp.spawn(cmd, args, {
      stdio: ['pipe', 'pipe', 'pipe'],
      env: {
        ...process.env,
        // Force UTF-8 on all platforms.
        PYTHONIOENCODING: 'utf-8',
        HALCON_NO_COLOR: '1',
        HALCON_NO_BANNER: '1',
      },
    });

    this.proc.on('error', (err) => {
      this.onError(new Error(`Halcon process error: ${err.message}`));
      this.scheduleRestart();
    });

    this.proc.on('exit', (code, signal) => {
      if (code !== 0 && !signal) {
        this.onError(new Error(`Halcon process exited with code ${code}`));
        this.scheduleRestart();
      }
      this.onEvent('process_exit', { code, signal });
    });

    if (this.proc.stderr) {
      this.proc.stderr.setEncoding('utf8');
      this.proc.stderr.on('data', (chunk: string) => {
        // Forward stderr as a debug event so the webview can optionally show it.
        this.onEvent('stderr', { text: chunk });
      });
    }

    if (this.proc.stdout) {
      this.proc.stdout.setEncoding('utf8');
      this.rl = readline.createInterface({ input: this.proc.stdout, crlfDelay: Infinity });
      this.rl.on('line', (line) => this.handleLine(line.trim()));
    }

    // Wait for the process to be ready (initial pong confirms it).
    await this.ping(3000).catch(() => {
      throw new Error('Halcon binary did not respond to ping — check binary path and permissions.');
    });
  }

  /** Send a chat message with optional VS Code context. */
  sendChat(params: ChatParams): void {
    this.send({ method: 'chat', params });
  }

  /** Cancel the current running task. */
  sendCancel(): void {
    this.send({ method: 'cancel' });
  }

  /** Health-check ping; resolves on pong within `timeoutMs`. */
  ping(timeoutMs = 5000): Promise<void> {
    return new Promise((resolve, reject) => {
      const id = this.nextId++;
      const timer = setTimeout(() => {
        this.pendingPings.delete(id);
        reject(new Error(`ping timeout after ${timeoutMs}ms`));
      }, timeoutMs);

      this.pendingPings.set(id, {
        resolve: () => { clearTimeout(timer); resolve(); },
        reject: (e) => { clearTimeout(timer); reject(e); },
      });

      this.send({ id, method: 'ping' });
    });
  }

  /** Gracefully terminate the subprocess. */
  dispose(): void {
    this.rl?.close();
    if (this.proc) {
      this.proc.stdin?.end();
      setTimeout(() => this.proc?.kill('SIGTERM'), 500);
    }
  }

  // ── Private ────────────────────────────────────────────────────────────────

  private buildCommand(): { cmd: string; args: string[] } {
    const config = vscode.workspace.getConfiguration('halcon');
    const model = config.get<string>('model', '');
    const maxTurns = config.get<number>('maxTurns', 20);
    const provider = config.get<string>('provider', '');

    const halconArgs = [
      '--mode', 'json-rpc',
      '--max-turns', String(maxTurns),
      ...(model ? ['--model', model] : []),
      ...(provider ? ['--provider', provider] : []),
      ...this.extraArgs,
    ];

    if (process.platform === 'win32') {
      // Windows: spawn via cmd /c to handle stdio pipe buffering correctly.
      return {
        cmd: 'cmd',
        args: ['/c', this.binaryPath, ...halconArgs],
      };
    }

    return { cmd: this.binaryPath, args: halconArgs };
  }

  private send(msg: Record<string, unknown>): void {
    if (!this.proc?.stdin?.writable) {
      this.onError(new Error('Halcon process stdin is not writable'));
      return;
    }
    const line = JSON.stringify(msg) + '\n';
    this.proc.stdin.write(line, 'utf8');
  }

  private handleLine(line: string): void {
    if (!line) return;

    let msg: Record<string, unknown>;
    try {
      msg = JSON.parse(line);
    } catch {
      // Non-JSON output (e.g., startup logging) — ignore.
      return;
    }

    // Pong response.
    if (msg['event'] === 'pong' || msg['method'] === 'pong') {
      const id = msg['id'] as number | undefined;
      if (id !== undefined) {
        this.pendingPings.get(id)?.resolve();
        this.pendingPings.delete(id);
      } else {
        // Un-ID'd pong (during start ping).
        this.pendingPings.forEach((p) => p.resolve());
        this.pendingPings.clear();
      }
      return;
    }

    // Streamed event.
    const event = msg['event'] as string | undefined;
    if (event) {
      this.onEvent(event, msg['data']);
      return;
    }
  }

  private scheduleRestart(): void {
    if (this.restartCount >= this.maxRestarts) {
      this.onError(new Error(
        `Halcon process failed ${this.maxRestarts} times — giving up. Use "Halcon: Open Panel" to retry.`
      ));
      return;
    }
    this.restartCount++;
    const delay = Math.min(1000 * this.restartCount, 10_000);
    setTimeout(() => {
      this.start().catch((e) => this.onError(e));
    }, delay);
  }
}
