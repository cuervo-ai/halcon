/**
 * HalconPanel — VS Code WebviewPanel hosting the xterm.js terminal for Halcon sessions.
 *
 * The panel is a singleton: opening it while already visible just reveals it.
 * Session history is kept in-memory for the panel's lifetime.
 *
 * Message protocol (extension ↔ webview via postMessage):
 *   Extension → Webview: { type: 'token'|'tool_call'|'tool_result'|'done'|'error'|'clear'|'edit_proposal', ... }
 *   Webview → Extension: { type: 'send_message'|'cancel'|'apply_edit'|'reject_edit'|'new_session' }
 */

import * as vscode from 'vscode';
import * as path from 'path';
import * as crypto from 'crypto';
import { HalconProcess } from './halcon_process';
import { collectContext, collectSelectionContext } from './context_collector';
import { showEditProposal, EditProposal } from './diff_applier';

export class HalconPanel {
  private static instance: HalconPanel | undefined;

  private readonly panel: vscode.WebviewPanel;
  private readonly process: HalconProcess;
  private readonly extensionPath: string;
  private disposables: vscode.Disposable[] = [];

  private constructor(
    panel: vscode.WebviewPanel,
    proc: HalconProcess,
    extensionPath: string,
  ) {
    this.panel = panel;
    this.process = proc;
    this.extensionPath = extensionPath;

    // Wire up webview → extension messages.
    this.panel.webview.onDidReceiveMessage(
      (msg) => this.handleWebviewMessage(msg),
      undefined,
      this.disposables,
    );

    // Clean up when the panel is closed.
    this.panel.onDidDispose(() => this.dispose(), undefined, this.disposables);

    this.panel.webview.html = this.buildHtml();
  }

  // ── Singleton lifecycle ───────────────────────────────────────────────────

  static async create(
    extensionPath: string,
    proc: HalconProcess,
    context: vscode.ExtensionContext,
  ): Promise<HalconPanel> {
    if (HalconPanel.instance) {
      HalconPanel.instance.panel.reveal(vscode.ViewColumn.Two);
      return HalconPanel.instance;
    }

    const panel = vscode.window.createWebviewPanel(
      'halconPanel',
      'Halcon AI',
      vscode.ViewColumn.Two,
      {
        enableScripts: true,
        localResourceRoots: [vscode.Uri.file(path.join(extensionPath, 'media'))],
        retainContextWhenHidden: true,
      },
    );

    const instance = new HalconPanel(panel, proc, extensionPath);
    HalconPanel.instance = instance;
    context.subscriptions.push(instance);
    return instance;
  }

  static getInstance(): HalconPanel | undefined {
    return HalconPanel.instance;
  }

  // ── Public methods ────────────────────────────────────────────────────────

  /** Forward a streaming event from the Halcon process to the webview. */
  postEvent(event: string, data: unknown): void {
    if (!this.panel.visible) {
      this.panel.reveal(vscode.ViewColumn.Two, true);
    }
    this.panel.webview.postMessage({ type: event, data });
  }

  /** Show a file edit proposal in both the diff editor and the panel. */
  async showEditProposal(proposal: EditProposal): Promise<void> {
    // Show in the webview panel.
    this.panel.webview.postMessage({ type: 'edit_proposal', data: proposal });

    // Also open the VS Code diff editor.
    await showEditProposal(proposal, (decision) => {
      this.panel.webview.postMessage({ type: 'edit_decision', data: { decision } });
    });
  }

  reveal(): void {
    this.panel.reveal(vscode.ViewColumn.Two);
  }

  dispose(): void {
    HalconPanel.instance = undefined;
    this.panel.dispose();
    this.disposables.forEach((d) => d.dispose());
    this.disposables = [];
  }

  // ── Webview message handler ───────────────────────────────────────────────

  private handleWebviewMessage(msg: { type: string; [key: string]: unknown }): void {
    switch (msg['type']) {
      case 'send_message': {
        const text = msg['text'] as string;
        const ctx = collectContext();
        this.process.sendChat({ message: text, context: ctx as unknown as Record<string, unknown> });
        break;
      }
      case 'cancel':
        this.process.sendCancel();
        break;
      case 'new_session':
        this.panel.webview.postMessage({ type: 'clear' });
        break;
      case 'apply_edit': {
        // The webview confirmed apply — handled by diff_applier callback.
        break;
      }
      case 'reject_edit': {
        this.process.sendCancel();
        break;
      }
      default:
        break;
    }
  }

  // ── HTML generation ───────────────────────────────────────────────────────

  private buildHtml(): string {
    const nonce = crypto.randomBytes(16).toString('hex');
    const csp = [
      `default-src 'none'`,
      `script-src 'nonce-${nonce}' https://cdn.jsdelivr.net`,
      `style-src 'unsafe-inline' https://cdn.jsdelivr.net`,
      `font-src https://cdn.jsdelivr.net`,
      `connect-src 'none'`,
    ].join('; ');

    return `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <meta http-equiv="Content-Security-Policy" content="${csp}">
  <title>Halcon AI</title>
  <!-- xterm.js 5.x — terminal rendering with color + streaming support -->
  <link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/xterm@5.3.0/css/xterm.min.css">
  <script nonce="${nonce}" src="https://cdn.jsdelivr.net/npm/xterm@5.3.0/lib/xterm.min.js"></script>
  <script nonce="${nonce}" src="https://cdn.jsdelivr.net/npm/xterm-addon-fit@0.8.0/lib/xterm-addon-fit.min.js"></script>
  <style>
    * { box-sizing: border-box; margin: 0; padding: 0; }
    body {
      display: flex; flex-direction: column; height: 100vh;
      background: var(--vscode-editor-background);
      color: var(--vscode-editor-foreground);
      font-family: var(--vscode-font-family);
      font-size: var(--vscode-font-size);
    }
    #toolbar {
      display: flex; align-items: center; gap: 8px;
      padding: 6px 12px;
      background: var(--vscode-titleBar-activeBackground);
      border-bottom: 1px solid var(--vscode-panel-border);
      flex-shrink: 0;
    }
    #toolbar h1 { font-size: 13px; font-weight: 600; flex: 1; }
    #toolbar button {
      background: var(--vscode-button-background);
      color: var(--vscode-button-foreground);
      border: none; border-radius: 3px; padding: 3px 10px;
      cursor: pointer; font-size: 12px;
    }
    #toolbar button:hover { background: var(--vscode-button-hoverBackground); }
    #tool-indicator {
      display: none; align-items: center; gap: 6px;
      padding: 4px 12px;
      background: var(--vscode-editorInfo-background, rgba(0,120,212,0.1));
      border-bottom: 1px solid var(--vscode-panel-border);
      font-size: 11px; flex-shrink: 0;
    }
    #tool-indicator.visible { display: flex; }
    .spinner {
      width: 12px; height: 12px;
      border: 2px solid var(--vscode-progressBar-background, #0078d4);
      border-top-color: transparent;
      border-radius: 50%;
      animation: spin 0.8s linear infinite;
    }
    @keyframes spin { to { transform: rotate(360deg); } }
    #terminal-container { flex: 1; overflow: hidden; padding: 4px; }
    #edit-proposal {
      display: none; flex-direction: column; gap: 8px;
      padding: 12px; background: var(--vscode-editorWarning-background, rgba(255,200,0,0.1));
      border-top: 1px solid var(--vscode-panel-border); flex-shrink: 0;
    }
    #edit-proposal.visible { display: flex; }
    #edit-proposal .path { font-family: monospace; font-size: 11px; }
    #edit-proposal .actions { display: flex; gap: 8px; }
    #input-area {
      display: flex; gap: 8px; padding: 8px 12px;
      border-top: 1px solid var(--vscode-panel-border); flex-shrink: 0;
    }
    #input-box {
      flex: 1; padding: 6px 10px; border-radius: 4px;
      border: 1px solid var(--vscode-input-border);
      background: var(--vscode-input-background);
      color: var(--vscode-input-foreground);
      font-family: inherit; font-size: inherit;
      resize: none; min-height: 36px; max-height: 120px;
    }
    #send-btn {
      background: var(--vscode-button-background);
      color: var(--vscode-button-foreground);
      border: none; border-radius: 4px; padding: 6px 16px;
      cursor: pointer; align-self: flex-end;
    }
    #send-btn:disabled { opacity: 0.5; cursor: default; }
  </style>
</head>
<body>
  <div id="toolbar">
    <h1>⚡ Halcon AI</h1>
    <button id="new-session-btn" title="New Session">New Session</button>
    <button id="cancel-btn" title="Cancel Task" disabled>Cancel</button>
  </div>

  <div id="tool-indicator">
    <div class="spinner"></div>
    <span id="tool-name">Running tool…</span>
  </div>

  <div id="terminal-container"></div>

  <div id="edit-proposal">
    <strong>Proposed file edit:</strong>
    <span class="path" id="proposal-path"></span>
    <div class="actions">
      <button id="apply-btn">Apply Changes</button>
      <button id="reject-btn" style="background:var(--vscode-button-secondaryBackground,#555)">Reject</button>
    </div>
  </div>

  <div id="input-area">
    <textarea id="input-box" rows="1" placeholder="Ask Halcon anything… (Enter to send, Shift+Enter for newline)"></textarea>
    <button id="send-btn">Send</button>
  </div>

  <script nonce="${nonce}">
    const vscode = acquireVsCodeApi();

    // ── xterm.js setup ──────────────────────────────────────────────────────
    const term = new Terminal({
      theme: {
        background: getComputedStyle(document.body)
          .getPropertyValue('--vscode-editor-background').trim() || '#1e1e1e',
        foreground: getComputedStyle(document.body)
          .getPropertyValue('--vscode-editor-foreground').trim() || '#d4d4d4',
      },
      convertEol: true,
      cursorBlink: false,
      disableStdin: true,
      scrollback: 5000,
      fontSize: 13,
      fontFamily: getComputedStyle(document.body)
        .getPropertyValue('--vscode-editor-font-family').trim() || 'monospace',
    });
    const fitAddon = new FitAddon.FitAddon();
    term.loadAddon(fitAddon);
    term.open(document.getElementById('terminal-container'));
    fitAddon.fit();
    window.addEventListener('resize', () => fitAddon.fit());

    // ── State ───────────────────────────────────────────────────────────────
    let busy = false;

    function setBusy(b) {
      busy = b;
      document.getElementById('send-btn').disabled = b;
      document.getElementById('cancel-btn').disabled = !b;
    }

    // ── Extension → Webview messages ────────────────────────────────────────
    window.addEventListener('message', (e) => {
      const msg = e.data;
      switch (msg.type) {
        case 'token':
          term.write(msg.data?.text ?? '');
          break;
        case 'tool_call':
          showToolIndicator(msg.data?.name ?? 'tool');
          break;
        case 'tool_result':
          hideToolIndicator();
          if (!msg.data?.success) {
            term.writeln('\\r\\n\\x1b[31m[tool error] ' + (msg.data?.output ?? '') + '\\x1b[0m');
          }
          break;
        case 'done':
          hideToolIndicator();
          term.writeln('\\r\\n');
          setBusy(false);
          break;
        case 'error':
          term.writeln('\\r\\n\\x1b[31m[error] ' + (msg.data ?? 'Unknown error') + '\\x1b[0m\\r\\n');
          setBusy(false);
          break;
        case 'clear':
          term.clear();
          setBusy(false);
          break;
        case 'edit_proposal':
          showEditProposal(msg.data);
          break;
        case 'edit_decision':
          hideEditProposal();
          term.writeln('\\r\\n[edit ' + msg.data?.decision + ']\\r\\n');
          break;
        case 'process_exit':
          term.writeln('\\r\\n\\x1b[33m[Halcon process exited — reconnecting…]\\x1b[0m\\r\\n');
          setBusy(false);
          break;
        default:
          break;
      }
    });

    // ── Tool indicator ──────────────────────────────────────────────────────
    function showToolIndicator(name) {
      const el = document.getElementById('tool-indicator');
      document.getElementById('tool-name').textContent = 'Running: ' + name;
      el.classList.add('visible');
    }
    function hideToolIndicator() {
      document.getElementById('tool-indicator').classList.remove('visible');
    }

    // ── Edit proposal ───────────────────────────────────────────────────────
    function showEditProposal(data) {
      document.getElementById('proposal-path').textContent = data.path;
      document.getElementById('edit-proposal').classList.add('visible');
    }
    function hideEditProposal() {
      document.getElementById('edit-proposal').classList.remove('visible');
    }
    document.getElementById('apply-btn').addEventListener('click', () => {
      vscode.postMessage({ type: 'apply_edit' });
      hideEditProposal();
    });
    document.getElementById('reject-btn').addEventListener('click', () => {
      vscode.postMessage({ type: 'reject_edit' });
      hideEditProposal();
    });

    // ── Input ───────────────────────────────────────────────────────────────
    const inputBox = document.getElementById('input-box');
    const sendBtn = document.getElementById('send-btn');

    function sendMessage() {
      const text = inputBox.value.trim();
      if (!text || busy) return;
      term.writeln('\\r\\n\\x1b[1;34m> ' + text + '\\x1b[0m\\r\\n');
      vscode.postMessage({ type: 'send_message', text });
      inputBox.value = '';
      inputBox.style.height = 'auto';
      setBusy(true);
    }

    sendBtn.addEventListener('click', sendMessage);
    inputBox.addEventListener('keydown', (e) => {
      if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        sendMessage();
      }
    });
    inputBox.addEventListener('input', () => {
      inputBox.style.height = 'auto';
      inputBox.style.height = Math.min(inputBox.scrollHeight, 120) + 'px';
    });

    // ── Toolbar buttons ─────────────────────────────────────────────────────
    document.getElementById('new-session-btn').addEventListener('click', () => {
      vscode.postMessage({ type: 'new_session' });
    });
    document.getElementById('cancel-btn').addEventListener('click', () => {
      vscode.postMessage({ type: 'cancel' });
      setBusy(false);
    });

    // Greet on load.
    term.writeln('\\x1b[1;32mHalcon AI\\x1b[0m — type a message and press Enter.\\r\\n');
  </script>
</body>
</html>`;
  }
}
