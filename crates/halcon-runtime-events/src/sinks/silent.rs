//! Silent sink — discards all events. Zero allocations, zero I/O.
//!
//! Use for:
//! - unit tests that only care about runtime correctness, not event output
//! - sub-agent loops (inner agents should not produce outer IDE output)
//! - benchmark harnesses

use crate::bus::EventSink;
use crate::event::RuntimeEvent;

/// A no-op `EventSink` that discards every event.
///
/// `#[derive(Default)]` makes it trivially constructable with `SilentSink::default()`.
#[derive(Debug, Clone, Default)]
pub struct SilentSink;

impl EventSink for SilentSink {
    #[inline(always)]
    fn emit(&self, _event: &RuntimeEvent) {}

    #[inline(always)]
    fn is_silent(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::RuntimeEventKind;
    use uuid::Uuid;

    #[test]
    fn silent_sink_does_not_panic() {
        let sink = SilentSink;
        let ev = RuntimeEvent::new(
            Uuid::new_v4(),
            RuntimeEventKind::RoundStarted {
                round: 1,
                model: "claude-sonnet-4-6".into(),
                tools_allowed: true,
                token_budget_remaining: 8192,
            },
        );
        sink.emit(&ev); // must not panic
        assert!(sink.is_silent());
    }
}
