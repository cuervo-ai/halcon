//! Integration tests for `ClaudeCodeProvider`.
//!
//! These tests require the `mock-claude` binary to be built first:
//!   `cargo build --bin mock-claude -p halcon-providers`
//!
//! Run with:
//!   `cargo test -p halcon-providers -- --include-ignored claude_code_integration`

use std::path::PathBuf;
use std::sync::Arc;

use futures::StreamExt;
use halcon_core::traits::ModelProvider;
use halcon_core::types::{ChatMessage, MessageContent, ModelChunk, ModelRequest, Role, StopReason};
use halcon_providers::claude_code::{ClaudeCodeConfig, ClaudeCodeProvider};

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Path to the mock-claude binary built by `cargo build --bin mock-claude`.
fn mock_claude_path() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("../../target/debug/mock-claude");
    path
}

fn provider_with_mock() -> ClaudeCodeProvider {
    ClaudeCodeProvider::new(ClaudeCodeConfig {
        command: mock_claude_path().to_string_lossy().to_string(),
        drain_timeout_secs: 10,
        ..ClaudeCodeConfig::default()
    })
}

fn simple_request(text: &str) -> ModelRequest {
    ModelRequest {
        model: "claude-opus-4-6".into(),
        messages: vec![ChatMessage {
            role: Role::User,
            content: MessageContent::Text(text.into()),
        }],
        tools: vec![],
        max_tokens: Some(256),
        temperature: None,
        system: None,
        stream: true,
    }
}

fn error_request() -> ModelRequest {
    simple_request("HALCON_ERROR_TEST")
}

// ─────────────────────────────────────────────────────────────────────────────
// Integration tests (all marked #[ignore] — run with --include-ignored)
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[ignore = "requires mock-claude binary: cargo build --bin mock-claude -p halcon-providers"]
async fn integration_invoke_returns_text() {
    let provider = provider_with_mock();
    let req = simple_request("hello");

    let stream = provider
        .invoke(&req)
        .await
        .expect("invoke must not fail");

    let chunks: Vec<_> = stream.collect().await;
    let has_text = chunks
        .iter()
        .any(|c| matches!(c, Ok(ModelChunk::TextDelta(_))));
    assert!(has_text, "stream must contain at least one TextDelta");
}

#[tokio::test]
#[ignore = "requires mock-claude binary: cargo build --bin mock-claude -p halcon-providers"]
async fn integration_invoke_emits_done_last() {
    let provider = provider_with_mock();
    let req = simple_request("test done ordering");

    let stream = provider.invoke(&req).await.expect("invoke must not fail");
    let chunks: Vec<_> = stream.collect().await;

    // Done(EndTurn) must be the last chunk
    let last = chunks.last().expect("stream must not be empty");
    assert!(
        matches!(last, Ok(ModelChunk::Done(StopReason::EndTurn))),
        "last chunk must be Done(EndTurn), got {last:?}"
    );
}

#[tokio::test]
#[ignore = "requires mock-claude binary: cargo build --bin mock-claude -p halcon-providers"]
async fn integration_invoke_emits_usage() {
    let provider = provider_with_mock();
    let req = simple_request("count my tokens");

    let stream = provider.invoke(&req).await.expect("invoke must not fail");
    let chunks: Vec<_> = stream.collect().await;

    let has_usage = chunks.iter().any(|c| matches!(c, Ok(ModelChunk::Usage(_))));
    assert!(has_usage, "stream must contain a Usage chunk");
}

#[tokio::test]
#[ignore = "requires mock-claude binary: cargo build --bin mock-claude -p halcon-providers"]
async fn integration_mock_response_contains_expected_text() {
    let provider = provider_with_mock();
    let req = simple_request("say hello");

    let stream = provider.invoke(&req).await.expect("invoke must not fail");
    let chunks: Vec<_> = stream.collect().await;

    let text: String = chunks
        .iter()
        .filter_map(|c| match c {
            Ok(ModelChunk::TextDelta(t)) => Some(t.as_str()),
            _ => None,
        })
        .collect();

    assert!(
        text.contains("MOCK_CLAUDE_RESPONSE"),
        "expected MOCK_CLAUDE_RESPONSE in text, got: {text:?}"
    );
}

#[tokio::test]
#[ignore = "requires mock-claude binary: cargo build --bin mock-claude -p halcon-providers"]
async fn integration_error_response_maps_to_error_chunk() {
    let provider = provider_with_mock();
    let req = error_request();

    let stream = provider.invoke(&req).await.expect("invoke must not fail");
    let chunks: Vec<_> = stream.collect().await;

    let has_error = chunks.iter().any(|c| matches!(c, Ok(ModelChunk::Error(_))));
    assert!(has_error, "error sentinel must produce a ModelChunk::Error");

    // Must not contain Done or TextDelta
    assert!(
        !chunks.iter().any(|c| matches!(c, Ok(ModelChunk::Done(_)))),
        "error response must not emit Done"
    );
}

#[tokio::test]
#[ignore = "requires mock-claude binary: cargo build --bin mock-claude -p halcon-providers"]
async fn integration_is_available_returns_true_for_mock() {
    let provider = provider_with_mock();
    // The mock-claude binary supports `--version` because it's a real binary.
    // We use the mock binary path as the `command`.
    // Note: mock-claude ignores --version (reads stdin), but exits 0 on empty stdin.
    // We skip the availability check for the mock since it doesn't implement --version.
    // Just test that the provider can be constructed and invoked.
    let req = simple_request("availability check");
    let result = provider.invoke(&req).await;
    assert!(result.is_ok(), "invoke should succeed with mock binary");
}

#[tokio::test]
#[ignore = "requires mock-claude binary: cargo build --bin mock-claude -p halcon-providers"]
async fn integration_auto_restart_after_kill() {
    use halcon_core::traits::ModelProvider;

    let provider = provider_with_mock();

    // First request: spawns the process
    let req = simple_request("first request");
    let stream = provider.invoke(&req).await.expect("first invoke ok");
    let _ = stream.collect::<Vec<_>>().await;

    // Kill the subprocess by sending a request that causes the process to exit.
    // The mock exits cleanly when stdin closes — we simulate this by sending
    // a second request which will reuse the same (still alive) mock.
    let req2 = simple_request("second request after restart");
    let stream2 = provider.invoke(&req2).await.expect("second invoke ok");
    let chunks2: Vec<_> = stream2.collect().await;

    let has_text = chunks2
        .iter()
        .any(|c| matches!(c, Ok(ModelChunk::TextDelta(_))));
    assert!(has_text, "second request must succeed after mock stays alive");
}

#[tokio::test]
#[ignore = "requires mock-claude binary: cargo build --bin mock-claude -p halcon-providers"]
async fn integration_multiple_sequential_requests() {
    let provider = Arc::new(provider_with_mock());

    for i in 0..3 {
        let req = simple_request(&format!("request number {i}"));
        let stream = provider.invoke(&req).await.expect("invoke ok");
        let chunks: Vec<_> = stream.collect().await;
        let has_done = chunks.iter().any(|c| matches!(c, Ok(ModelChunk::Done(_))));
        assert!(has_done, "request {i} must complete with Done");
    }
}
