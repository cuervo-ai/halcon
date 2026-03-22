/// Execution context propagated via tokio task-local storage.
///
/// Allows `DomainEvent::new()` to automatically inject `session_id`,
/// `trace_id`, and `span_id` without modifying every callsite.
///
/// Usage:
/// ```rust,ignore
/// let ctx = ExecutionContext::new(session_id);
/// EXECUTION_CTX.scope(ctx, async {
///     // All DomainEvent::new() calls here auto-inject context
/// }).await;
/// ```
use uuid::Uuid;

tokio::task_local! {
    /// Task-local execution context — auto-injected into every DomainEvent
    /// created within the same tokio task (or any task spawned inside a
    /// `EXECUTION_CTX.scope(...)` block).
    pub static EXECUTION_CTX: ExecutionContext;
}

/// Ambient context for a single agent session.
#[derive(Debug, Clone)]
pub struct ExecutionContext {
    pub session_id: Uuid,
    pub trace_id: TraceId,
    pub span_id: SpanId,
    pub agent_id: Option<String>,
}

impl ExecutionContext {
    pub fn new(session_id: Uuid) -> Self {
        Self {
            session_id,
            trace_id: TraceId::generate(),
            span_id: SpanId::generate(),
            agent_id: None,
        }
    }

    pub fn with_agent(mut self, agent_id: impl Into<String>) -> Self {
        self.agent_id = Some(agent_id.into());
        self
    }
}

/// Opaque trace identifier (W3C Trace-Context compatible, 16 random bytes → 32 hex chars).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TraceId(pub String);

impl TraceId {
    pub fn generate() -> Self {
        use std::fmt::Write;
        let mut buf = String::with_capacity(32);
        let bytes = Uuid::new_v4().as_bytes().to_owned();
        let bytes2 = Uuid::new_v4().as_bytes().to_owned();
        for b in bytes.iter().chain(bytes2.iter()) {
            let _ = write!(buf, "{b:02x}");
        }
        // Keep only first 32 hex chars (128-bit)
        buf.truncate(32);
        Self(buf)
    }
}

/// Opaque span identifier (8 random bytes → 16 hex chars).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SpanId(pub String);

impl SpanId {
    pub fn generate() -> Self {
        use std::fmt::Write;
        let mut buf = String::with_capacity(16);
        for b in &Uuid::new_v4().as_bytes()[..8] {
            let _ = write!(buf, "{b:02x}");
        }
        Self(buf)
    }
}

/// Read the current session_id from task-local storage, if available.
///
/// Returns `None` when called outside of a `EXECUTION_CTX.scope(...)` block
/// (e.g., background tasks, sub-agents that haven't set their own context yet).
pub fn current_session_id() -> Option<Uuid> {
    EXECUTION_CTX.try_with(|ctx| ctx.session_id).ok()
}

/// Read the current trace_id from task-local storage, if available.
pub fn current_trace_id() -> Option<TraceId> {
    EXECUTION_CTX.try_with(|ctx| ctx.trace_id.clone()).ok()
}

/// Read the current span_id from task-local storage, if available.
pub fn current_span_id() -> Option<SpanId> {
    EXECUTION_CTX.try_with(|ctx| ctx.span_id.clone()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn context_not_set_returns_none() {
        // Outside any scope, try_with returns Err → current_session_id → None
        assert!(current_session_id().is_none());
    }

    #[tokio::test]
    async fn context_injected_inside_scope() {
        let sid = Uuid::new_v4();
        let ctx = ExecutionContext::new(sid);
        EXECUTION_CTX
            .scope(ctx, async {
                let found = current_session_id().expect("session_id must be set inside scope");
                assert_eq!(found, sid);
            })
            .await;
    }

    #[tokio::test]
    async fn nested_scope_shadows_outer() {
        let outer = Uuid::new_v4();
        let inner = Uuid::new_v4();

        let ctx_outer = ExecutionContext::new(outer);
        EXECUTION_CTX
            .scope(ctx_outer, async move {
                assert_eq!(current_session_id(), Some(outer));

                let ctx_inner = ExecutionContext::new(inner);
                EXECUTION_CTX
                    .scope(ctx_inner, async {
                        assert_eq!(current_session_id(), Some(inner));
                    })
                    .await;

                // Outer context restored after inner scope exits
                assert_eq!(current_session_id(), Some(outer));
            })
            .await;
    }

    #[test]
    fn trace_id_is_32_chars() {
        let t = TraceId::generate();
        assert_eq!(t.0.len(), 32);
        assert!(t.0.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn span_id_is_16_chars() {
        let s = SpanId::generate();
        assert_eq!(s.0.len(), 16);
        assert!(s.0.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn trace_ids_are_unique() {
        let a = TraceId::generate();
        let b = TraceId::generate();
        assert_ne!(a, b);
    }
}
