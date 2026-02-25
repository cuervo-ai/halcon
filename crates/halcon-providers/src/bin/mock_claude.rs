//! Minimal mock binary for `ClaudeCodeProvider` integration tests.
//!
//! Reads one NDJSON line at a time from stdin and writes canned response chunks
//! to stdout, then loops to handle the next request — mimicking a persistent
//! `claude --input-format stream-json --output-format stream-json` subprocess.
//!
//! Sentinel: if the user message contains `"HALCON_ERROR_TEST"`, the mock
//! responds with an `is_error: true` result chunk instead of a success response.
//!
//! Build: `cargo build --bin mock-claude -p halcon-providers`

use std::io::{self, BufRead, Write};

fn main() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    for raw_line in stdin.lock().lines() {
        let line = match raw_line {
            Ok(l) => l,
            Err(_) => break,
        };

        if line.trim().is_empty() {
            continue;
        }

        if line.contains("HALCON_ERROR_TEST") {
            // Respond with an error result.
            let result = serde_json::json!({
                "type": "result",
                "subtype": "error",
                "is_error": true,
                "error": "mock error: HALCON_ERROR_TEST sentinel triggered",
                "cost_usd": null,
                "usage": null,
                "duration_ms": 1
            });
            writeln!(out, "{result}").ok();
            out.flush().ok();
            continue;
        }

        // ── Normal success path ──────────────────────────────────────────────

        // 1. System init
        let system = serde_json::json!({
            "type": "system",
            "subtype": "init",
            "cwd": "/tmp",
            "session_id": "mock-session"
        });
        writeln!(out, "{system}").ok();

        // 2. Assistant text response
        let assistant = serde_json::json!({
            "type": "assistant",
            "message": {
                "id": "msg_mock_001",
                "type": "message",
                "role": "assistant",
                "content": [
                    {"type": "text", "text": "MOCK_CLAUDE_RESPONSE"}
                ],
                "model": "claude-opus-4-6",
                "stop_reason": "end_turn",
                "stop_sequence": null,
                "usage": {"input_tokens": 10, "output_tokens": 5}
            }
        });
        writeln!(out, "{assistant}").ok();

        // 3. Result chunk
        let result = serde_json::json!({
            "type": "result",
            "subtype": "success",
            "is_error": false,
            "cost_usd": 0.001,
            "usage": {
                "input_tokens": 10,
                "output_tokens": 5,
                "cache_creation_input_tokens": 0,
                "cache_read_input_tokens": 0
            },
            "duration_ms": 100,
            "session_id": "mock-session"
        });
        writeln!(out, "{result}").ok();
        out.flush().ok();
    }
}
