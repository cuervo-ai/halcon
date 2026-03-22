/**
 * Handles file edit proposals from the Halcon agent.
 *
 * Flow:
 * 1. Agent sends {event: "file_edit_proposal", data: {path, before, after}}
 * 2. Extension creates a virtual "before" document for the diff view
 * 3. VS Code's diff editor opens: left = before, right = after (proposed)
 * 4. Webview shows "Apply" / "Reject" buttons
 * 5. Apply → workspace.applyEdit() writes the change
 * 6. Reject → sends {method: "cancel_edit"} to the process
 */

import * as vscode from 'vscode';

export interface EditProposal {
  /** Workspace-relative or absolute path of the file to modify. */
  path: string;
  /** Original file content (before). */
  before: string;
  /** Proposed file content (after). */
  after: string;
}

/** Result returned to the caller after user decides. */
export type EditDecision = 'applied' | 'rejected';

/** Pending proposal registry so we don't stack multiple diff editors. */
let pendingProposal: EditProposal | null = null;

/**
 * Show the diff editor and prompt the user to apply or reject.
 *
 * Returns a promise that resolves when the user has decided.
 * Returns 'rejected' on timeout or dismissal.
 */
export async function showEditProposal(
  proposal: EditProposal,
  onDecision: (decision: EditDecision) => void,
): Promise<void> {
  pendingProposal = proposal;

  // Resolve absolute path.
  const workspaceRoot = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath ?? '';
  const absolutePath = proposal.path.startsWith('/')
    ? proposal.path
    : `${workspaceRoot}/${proposal.path}`;

  const fileUri = vscode.Uri.file(absolutePath);

  // Create a virtual "before" URI using the diff scheme.
  const beforeUri = vscode.Uri.parse(`halcon-diff:${absolutePath}?before`);

  // Register a content provider for the "before" side.
  const provider = vscode.workspace.registerTextDocumentContentProvider(
    'halcon-diff',
    {
      provideTextDocumentContent: () => proposal.before,
    }
  );

  try {
    // Open the diff editor: left = before (virtual), right = after (proposed).
    // We first write the "after" content to a temp in-memory document.
    const afterUri = vscode.Uri.parse(`halcon-diff-after:${absolutePath}`);
    const afterProvider = vscode.workspace.registerTextDocumentContentProvider(
      'halcon-diff-after',
      {
        provideTextDocumentContent: () => proposal.after,
      }
    );

    await vscode.commands.executeCommand(
      'vscode.diff',
      beforeUri,
      afterUri,
      `Halcon Edit: ${proposal.path}`,
      { preview: true }
    );

    // Show action buttons via an information message.
    const choice = await vscode.window.showInformationMessage(
      `Halcon proposes changes to ${proposal.path}. Apply?`,
      { modal: false },
      'Apply',
      'Reject'
    );

    if (choice === 'Apply') {
      await applyEdit(fileUri, proposal.after);
      onDecision('applied');
    } else {
      onDecision('rejected');
    }

    afterProvider.dispose();
  } finally {
    provider.dispose();
    pendingProposal = null;
  }
}

/**
 * Directly apply a file edit without showing the diff editor.
 * Used when the user explicitly accepts via the panel buttons.
 */
export async function applyEdit(fileUri: vscode.Uri, newContent: string): Promise<void> {
  const edit = new vscode.WorkspaceEdit();

  // Check if the file exists to determine whether to create or replace.
  let fileExists = false;
  try {
    await vscode.workspace.fs.stat(fileUri);
    fileExists = true;
  } catch {
    // File does not exist yet — will be created.
  }

  if (fileExists) {
    const doc = await vscode.workspace.openTextDocument(fileUri);
    const fullRange = new vscode.Range(
      new vscode.Position(0, 0),
      doc.lineAt(doc.lineCount - 1).range.end,
    );
    edit.replace(fileUri, fullRange, newContent);
  } else {
    edit.createFile(fileUri, { overwrite: true, ignoreIfExists: false });
    edit.insert(fileUri, new vscode.Position(0, 0), newContent);
  }

  const success = await vscode.workspace.applyEdit(edit);
  if (!success) {
    throw new Error(`Failed to apply edit to ${fileUri.fsPath}`);
  }

  // Save the file after applying.
  const doc = await vscode.workspace.openTextDocument(fileUri);
  await doc.save();
}

/** Returns true if there is currently a pending edit proposal. */
export function hasPendingProposal(): boolean {
  return pendingProposal !== null;
}
