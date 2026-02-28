//! Evidence Boundary System (EBS) — "Zero Evidence → Zero Output" policy.
//!
//! Tracks textual evidence extracted from file-reading tools across all loop rounds.
//! When investigation tasks attempt file content reading but extract insufficient
//! readable text (e.g. binary PDFs, empty files, permission errors), the
//! `EvidenceGate` injects an explicit limitation notice instead of allowing
//! the coordinator to synthesize fabricated content from prior knowledge.
//!
//! ## Policy
//! - Zero Evidence → Zero Output: no synthesis of document content without real data.
//! - Binary files (PDF, images) are detected and reported explicitly.
//! - Gate applies only when content-reading tools were attempted and returned < threshold.
//! - Soft path: warning injected alongside synthesis message.
//! - Hard path: synthesis message replaced with limitation report directive.
//!
//! ## Integration Points
//! - `post_batch.rs`: calls `EvidenceBundle::record_tool_result()` for each success.
//! - `convergence_phase.rs`: checks `evidence_gate_fires()` before synthesis injection.
//! - `loop_state.rs`: owns `EvidenceBundle` as a field on `LoopState`.

// ── Constants ──────────────────────────────────────────────────────────────────

/// Minimum meaningful text bytes extracted to consider content "readable".
///
/// 30 bytes is roughly "a short sentence." Tool results below this threshold
/// when reading files indicate binary content, empty files, or permission errors.
pub const MIN_EVIDENCE_BYTES: usize = 30;

/// Tool names that are unambiguously for reading file content.
///
/// These tools are expected to return text content — short or empty results
/// indicate the file is binary, empty, or inaccessible.
const CONTENT_READ_TOOLS: &[&str] = &[
    "read_file",
    "read_multiple_files",
    "file_read",
    "read_multiple_files_content",
];

/// Substrings in tool output that indicate binary or unreadable file content.
const BINARY_INDICATORS: &[&str] = &[
    "%PDF-",             // PDF magic header bytes
    "Binary file",       // grep binary-file detection message
    "binary file",       // lowercase variant
    "is a binary file",  // extended grep message
    "cannot process binary file",
    "\x00\x00\x00",      // null-byte sequence typical in binary formats
];

// ── EvidenceBundle ─────────────────────────────────────────────────────────────

/// Aggregate evidence state collected from tool results across all loop rounds.
///
/// Tracks both quantitative (byte count) and qualitative (binary indicators)
/// signals to decide whether synthesis should proceed or be replaced by a
/// limitation report.
#[derive(Debug, Clone, Default)]
pub struct EvidenceBundle {
    /// Total printable text bytes extracted across all content-reading tool results.
    pub text_bytes_extracted: usize,

    /// Number of calls to content-reading tools (read_file, read_multiple_files, etc.).
    pub content_read_attempts: usize,

    /// Number of tool results that contained binary-content indicators.
    pub binary_file_count: usize,

    /// Short indicator strings that triggered binary detection (for diagnostics).
    pub unreadable_indicators: Vec<String>,

    /// Whether the evidence gate fired and synthesis was replaced with a limitation notice.
    pub synthesis_blocked: bool,
}

impl EvidenceBundle {
    // ── Gate Decision ─────────────────────────────────────────────────────────

    /// Returns `true` when the evidence gate should fire.
    ///
    /// Gate fires when:
    /// - At least one content-read was attempted (read_file, read_multiple_files), AND
    /// - Less than `MIN_EVIDENCE_BYTES` of readable text was extracted in total.
    ///
    /// This indicates the files are binary, empty, or inaccessible.
    /// When the gate fires, synthesis should be replaced with an explicit limitation report.
    pub fn evidence_gate_fires(&self) -> bool {
        self.content_read_attempts > 0 && self.text_bytes_extracted < MIN_EVIDENCE_BYTES
    }

    /// Returns `true` when there is sufficient evidence to proceed with synthesis.
    pub fn has_sufficient_evidence(&self) -> bool {
        !self.evidence_gate_fires()
    }

    // ── Evidence Recording ────────────────────────────────────────────────────

    /// Record evidence from a successful tool result.
    ///
    /// Called in `post_batch.rs` for each non-error tool result.
    /// Only content-reading tools contribute to evidence tracking;
    /// search/listing tools (grep, ls, glob) are intentionally excluded because
    /// they return file *names* rather than file *content*.
    pub fn record_tool_result(&mut self, tool_name: &str, content: &str) {
        let is_content_tool = CONTENT_READ_TOOLS
            .iter()
            .any(|t| tool_name == *t || tool_name.starts_with(*t));

        if !is_content_tool {
            return;
        }

        self.content_read_attempts += 1;

        // Detect binary-file indicators in the output.
        for indicator in BINARY_INDICATORS {
            if content.contains(indicator) {
                self.binary_file_count += 1;
                // Record first matching indicator for diagnostics (one per result).
                self.unreadable_indicators.push(indicator.to_string());
                // Don't count bytes from binary-indicator lines as real text.
                return;
            }
        }

        // Count printable text bytes (excludes control characters except newline/tab).
        let text_bytes = count_printable_bytes(content);
        self.text_bytes_extracted += text_bytes;
    }

    // ── Gate Message ──────────────────────────────────────────────────────────

    /// Build the synthesis-replacement directive injected when the gate fires.
    ///
    /// This message asks the model to honestly report the limitation instead of
    /// synthesizing content that was never extracted from the files.
    pub fn gate_message(&self) -> String {
        if self.binary_file_count > 0 {
            format!(
                "[System — Evidence Gate ACTIVE] {attempt}s file reading attempt(s) were \
                 made but only {bytes} bytes of readable text were extracted. \
                 {binary} file(s) appear to be in binary format (PDF or similar) and \
                 cannot be read by text tools. \
                 IMPORTANT: Do NOT fabricate or infer document content. \
                 Instead, respond to the user with a clear explanation: \
                 the requested files exist but are binary (likely PDF) and require \
                 a PDF-to-text conversion tool (e.g. pdftotext) to be read. \
                 Suggest how the user can extract the text and retry.",
                attempt = self.content_read_attempts,
                bytes = self.text_bytes_extracted,
                binary = self.binary_file_count,
            )
        } else {
            format!(
                "[System — Evidence Gate ACTIVE] {attempt}s file reading attempt(s) were \
                 made but only {bytes} bytes of readable text were extracted. \
                 The files may be empty, inaccessible, or in a non-text format. \
                 IMPORTANT: Do NOT fabricate or infer document content. \
                 Instead, respond to the user honestly: the files could not be read \
                 and no content is available to analyze. Describe what was found \
                 (file names, paths) and what was NOT found (readable content).",
                attempt = self.content_read_attempts,
                bytes = self.text_bytes_extracted,
            )
        }
    }

    /// Build a compact summary string for tracing/logs.
    pub fn summary(&self) -> String {
        format!(
            "evidence_bundle(attempts={}, text_bytes={}, binary={}, gate={})",
            self.content_read_attempts,
            self.text_bytes_extracted,
            self.binary_file_count,
            if self.evidence_gate_fires() { "FIRES" } else { "pass" },
        )
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────────

/// Count printable/meaningful bytes in a string.
///
/// Counts char by char; includes newlines and tabs (structural whitespace) but
/// excludes other control characters that appear in binary output. This prevents
/// binary-file bytes from inflating the evidence counter.
fn count_printable_bytes(content: &str) -> usize {
    content
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t' || *c == '\r')
        .count()
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Gate fires correctly ──────────────────────────────────────────────────

    #[test]
    fn gate_does_not_fire_when_no_content_read_attempted() {
        // No read_file calls → gate should NOT fire (nothing was attempted).
        let bundle = EvidenceBundle::default();
        assert!(
            !bundle.evidence_gate_fires(),
            "gate must not fire when no content read attempted"
        );
    }

    #[test]
    fn gate_fires_when_read_file_returned_empty() {
        let mut bundle = EvidenceBundle::default();
        bundle.record_tool_result("read_file", "");
        assert!(
            bundle.evidence_gate_fires(),
            "empty read_file result should trigger gate"
        );
        assert_eq!(bundle.content_read_attempts, 1);
        assert_eq!(bundle.text_bytes_extracted, 0);
    }

    #[test]
    fn gate_does_not_fire_when_sufficient_text_extracted() {
        let mut bundle = EvidenceBundle::default();
        // > MIN_EVIDENCE_BYTES of real text
        bundle.record_tool_result(
            "read_file",
            "This is a valid text document with sufficient content to satisfy the evidence gate.",
        );
        assert!(
            !bundle.evidence_gate_fires(),
            "real text content should NOT trigger gate"
        );
        assert!(bundle.text_bytes_extracted >= MIN_EVIDENCE_BYTES);
    }

    // ── Binary PDF detection ──────────────────────────────────────────────────

    #[test]
    fn binary_pdf_header_detected() {
        let mut bundle = EvidenceBundle::default();
        bundle.record_tool_result("read_file", "%PDF-1.4\x00\x00garbage binary content");
        assert_eq!(bundle.binary_file_count, 1, "PDF header must trigger binary count");
        assert!(bundle.evidence_gate_fires(), "PDF binary should trigger gate");
        assert_eq!(bundle.text_bytes_extracted, 0, "binary result must not add text bytes");
    }

    #[test]
    fn grep_binary_file_message_detected() {
        let mut bundle = EvidenceBundle::default();
        bundle.record_tool_result(
            "read_multiple_files",
            "Binary file /path/to/document.pdf matches",
        );
        assert_eq!(bundle.binary_file_count, 1);
        assert!(bundle.evidence_gate_fires());
    }

    // ── Non-content tools are ignored ─────────────────────────────────────────

    #[test]
    fn grep_search_tool_does_not_affect_evidence() {
        let mut bundle = EvidenceBundle::default();
        // grep returning filenames is NOT a content-read tool
        bundle.record_tool_result("bash", "/path/to/file1.pdf\n/path/to/file2.pdf\n");
        bundle.record_tool_result("grep", "/path/to/file1.pdf\n/path/to/file2.pdf\n");
        assert_eq!(bundle.content_read_attempts, 0, "grep/bash must not count as content reads");
        assert!(!bundle.evidence_gate_fires(), "no content read attempt → gate must not fire");
    }

    // ── Multiple reads accumulate ─────────────────────────────────────────────

    #[test]
    fn multiple_read_file_calls_accumulate_bytes() {
        let mut bundle = EvidenceBundle::default();
        bundle.record_tool_result("read_file", "Content fragment one.");
        bundle.record_tool_result("read_multiple_files", "Content fragment two and more.");
        assert_eq!(bundle.content_read_attempts, 2);
        assert!(bundle.text_bytes_extracted > MIN_EVIDENCE_BYTES);
        assert!(!bundle.evidence_gate_fires());
    }

    // ── Gate message contains useful info ─────────────────────────────────────

    #[test]
    fn gate_message_mentions_binary_when_detected() {
        let mut bundle = EvidenceBundle {
            content_read_attempts: 2,
            binary_file_count: 2,
            text_bytes_extracted: 0,
            ..Default::default()
        };
        let msg = bundle.gate_message();
        assert!(msg.contains("binary"), "gate message must mention binary format");
        assert!(msg.contains("PDF"), "gate message must mention PDF");
        assert!(msg.contains("pdftotext"), "gate message must suggest pdftotext");
    }

    #[test]
    fn gate_message_no_fabrication_directive_present() {
        let bundle = EvidenceBundle {
            content_read_attempts: 1,
            binary_file_count: 0,
            text_bytes_extracted: 5,
            ..Default::default()
        };
        let msg = bundle.gate_message();
        assert!(
            msg.contains("Do NOT fabricate"),
            "gate message must contain anti-fabrication directive"
        );
    }

    // ── summary() is informative ──────────────────────────────────────────────

    #[test]
    fn summary_includes_gate_status() {
        let mut bundle = EvidenceBundle::default();
        bundle.record_tool_result("read_file", "");
        let s = bundle.summary();
        assert!(s.contains("FIRES"), "summary must indicate gate fires");
    }
}
