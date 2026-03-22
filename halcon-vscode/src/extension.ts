/**
 * Halcon VS Code Extension — main entry point.
 *
 * Activation: onStartupFinished + onCommand:halcon.openPanel
 *
 * Registers commands:
 *   halcon.openPanel          — reveal (or create) the Halcon panel
 *   halcon.askAboutSelection  — open panel with current selection pre-loaded
 *   halcon.editFile           — ask Halcon to edit the current file
 *   halcon.newSession         — clear history, start fresh
 *   halcon.cancelTask         — cancel the running agent task
 */

import * as vscode from 'vscode';
import { resolveBinary } from './binary_resolver';
import { HalconProcess } from './halcon_process';
import { HalconPanel } from './webview_panel';
import { collectSelectionContext } from './context_collector';
import { EditProposal } from './diff_applier';

let halconProcess: HalconProcess | undefined;

export async function activate(context: vscode.ExtensionContext): Promise<void> {
  // Resolve binary once at activation; errors surface only when panel is opened.
  let binaryPath: string;
  try {
    const info = resolveBinary(context.extensionPath);
    binaryPath = info.path;
  } catch (err) {
    // Defer error to command execution so it doesn't block activation.
    binaryPath = 'halcon';
  }

  // ── Commands ────────────────────────────────────────────────────────────────

  context.subscriptions.push(
    vscode.commands.registerCommand('halcon.openPanel', async () => {
      await openPanel(context, binaryPath);
    }),

    vscode.commands.registerCommand('halcon.askAboutSelection', async () => {
      const panel = await openPanel(context, binaryPath);
      const ctx = collectSelectionContext();
      const sel = ctx.activeFile?.selectedText;
      if (sel) {
        const prompt = `Tell me about this code:\n\n\`\`\`\n${sel}\n\`\`\``;
        panel.postEvent('auto_send', { message: prompt });
        // Actually send via process.
        halconProcess?.sendChat({ message: prompt, context: ctx as unknown as Record<string, unknown> });
      }
    }),

    vscode.commands.registerCommand('halcon.editFile', async () => {
      const panel = await openPanel(context, binaryPath);
      const editor = vscode.window.activeTextEditor;
      if (!editor) {
        vscode.window.showWarningMessage('No active file to edit.');
        return;
      }
      const uri = editor.document.uri.fsPath;
      const message = `Please review and improve ${uri}`;
      const ctx = collectSelectionContext();
      halconProcess?.sendChat({ message, context: ctx as unknown as Record<string, unknown> });
      panel.postEvent('token', { text: `> ${message}\n\n` });
    }),

    vscode.commands.registerCommand('halcon.newSession', () => {
      HalconPanel.getInstance()?.postEvent('clear', {});
    }),

    vscode.commands.registerCommand('halcon.cancelTask', () => {
      halconProcess?.sendCancel();
      HalconPanel.getInstance()?.postEvent('done', {});
    }),
  );
}

export function deactivate(): void {
  halconProcess?.dispose();
  halconProcess = undefined;
}

// ── Panel creation ────────────────────────────────────────────────────────────

async function openPanel(
  context: vscode.ExtensionContext,
  binaryPath: string,
): Promise<HalconPanel> {
  // Re-use existing process if healthy.
  if (!halconProcess) {
    halconProcess = createProcess(binaryPath, context);
  }

  // Re-use or create the panel.
  const panel = await HalconPanel.create(context.extensionPath, halconProcess, context);

  // Start the process (idempotent).
  try {
    await halconProcess.start();
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    vscode.window.showErrorMessage(
      `Failed to start Halcon: ${msg}`,
      'Open Settings',
    ).then((choice: string | undefined) => {
      if (choice === 'Open Settings') {
        vscode.commands.executeCommand('workbench.action.openSettings', 'halcon.binaryPath');
      }
    });
  }

  return panel;
}

function createProcess(binaryPath: string, context: vscode.ExtensionContext): HalconProcess {
  const proc = new HalconProcess(
    binaryPath,
    ['--no-banner'],
    // onEvent — forward all events to the active panel.
    (event, data) => {
      const panel = HalconPanel.getInstance();
      if (!panel) return;

      if (event === 'file_edit_proposal') {
        panel.showEditProposal(data as EditProposal);
      } else {
        panel.postEvent(event, data);
      }
    },
    // onError — surface errors to the user.
    (error) => {
      const panel = HalconPanel.getInstance();
      panel?.postEvent('error', error.message);
      vscode.window.showErrorMessage(
        `Halcon: ${error.message}`,
        'Restart',
      ).then((choice: string | undefined) => {
        if (choice === 'Restart') {
          halconProcess = createProcess(binaryPath, context);
          HalconPanel.getInstance()?.postEvent('token', { text: '\n[Restarting Halcon…]\n' });
          halconProcess.start().catch((e: unknown) => console.error('[halcon]', e));
        }
      });
    },
  );

  context.subscriptions.push({ dispose: () => proc.dispose() });
  return proc;
}
