/**
 * Collects rich VS Code context for inclusion with each Halcon chat request.
 *
 * Collected fields:
 * - Active file: URI, language, content (≤50 KB), cursor position, selection
 * - Diagnostics: errors + warnings for the active file
 * - Git state: branch, staged/unstaged counts (via vscode.git extension API)
 * - Workspace root path
 * - Selected text (always included when non-empty)
 */

import * as vscode from 'vscode';

const MAX_FILE_BYTES = 50 * 1024; // 50 KB

export interface ActiveFileContext {
  uri: string;
  languageId: string;
  content: string | null;  // null if > MAX_FILE_BYTES
  contentTruncated: boolean;
  cursor: { line: number; character: number } | null;
  selection: { start: { line: number; character: number }; end: { line: number; character: number } } | null;
  selectedText: string | null;
}

export interface DiagnosticItem {
  severity: 'error' | 'warning' | 'info' | 'hint';
  message: string;
  range: { start: { line: number; character: number }; end: { line: number; character: number } };
  source: string | undefined;
}

export interface GitContext {
  branch: string | null;
  stagedCount: number;
  unstagedCount: number;
}

export interface VsCodeContext {
  workspaceRoot: string | null;
  activeFile: ActiveFileContext | null;
  diagnostics: DiagnosticItem[];
  git: GitContext | null;
}

/** Collect all available VS Code context for the current state. */
export function collectContext(): VsCodeContext {
  const workspaceRoot = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath ?? null;

  return {
    workspaceRoot,
    activeFile: collectActiveFile(),
    diagnostics: collectDiagnostics(),
    git: collectGitContext(),
  };
}

/** Collect context for a specific text selection (for "Ask About Selection"). */
export function collectSelectionContext(): VsCodeContext {
  const base = collectContext();
  // Selection is already included in activeFile; this function is a named alias
  // that makes intent explicit in the call site.
  return base;
}

// ── Active file ───────────────────────────────────────────────────────────────

function collectActiveFile(): ActiveFileContext | null {
  const editor = vscode.window.activeTextEditor;
  if (!editor) return null;

  const doc = editor.document;
  const text = doc.getText();
  const bytes = Buffer.byteLength(text, 'utf8');
  const truncated = bytes > MAX_FILE_BYTES;

  const sel = editor.selection;
  const hasSelection = !sel.isEmpty;

  return {
    uri: doc.uri.toString(),
    languageId: doc.languageId,
    content: truncated ? null : text,
    contentTruncated: truncated,
    cursor: {
      line: editor.selection.active.line,
      character: editor.selection.active.character,
    },
    selection: hasSelection ? {
      start: { line: sel.start.line, character: sel.start.character },
      end:   { line: sel.end.line,   character: sel.end.character   },
    } : null,
    selectedText: hasSelection ? doc.getText(sel) : null,
  };
}

// ── Diagnostics ───────────────────────────────────────────────────────────────

function collectDiagnostics(): DiagnosticItem[] {
  const editor = vscode.window.activeTextEditor;
  if (!editor) return [];

  const raw = vscode.languages.getDiagnostics(editor.document.uri);
  return raw
    .filter((d) => d.severity <= vscode.DiagnosticSeverity.Warning) // error + warning only
    .slice(0, 50) // cap at 50 to avoid bloating the context
    .map((d) => ({
      severity: severityLabel(d.severity),
      message: d.message,
      range: {
        start: { line: d.range.start.line, character: d.range.start.character },
        end:   { line: d.range.end.line,   character: d.range.end.character   },
      },
      source: d.source,
    }));
}

function severityLabel(s: vscode.DiagnosticSeverity): 'error' | 'warning' | 'info' | 'hint' {
  switch (s) {
    case vscode.DiagnosticSeverity.Error:       return 'error';
    case vscode.DiagnosticSeverity.Warning:     return 'warning';
    case vscode.DiagnosticSeverity.Information: return 'info';
    default:                                     return 'hint';
  }
}

// ── Git ───────────────────────────────────────────────────────────────────────

function collectGitContext(): GitContext | null {
  try {
    const gitExt = vscode.extensions.getExtension('vscode.git');
    if (!gitExt?.isActive) return null;

    const git = gitExt.exports.getAPI(1);
    const repo = git?.repositories?.[0];
    if (!repo) return null;

    const branch = repo.state?.HEAD?.name ?? null;
    const staged = repo.state?.indexChanges?.length ?? 0;
    const unstaged = repo.state?.workingTreeChanges?.length ?? 0;

    return { branch, stagedCount: staged, unstagedCount: unstaged };
  } catch {
    return null;
  }
}
