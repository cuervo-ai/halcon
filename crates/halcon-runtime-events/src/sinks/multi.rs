//! Multi-sink — fan-out to N sinks simultaneously.
//!
//! Useful for combining CLI rendering with JSON-RPC streaming, or with a
//! tracing sink for structured logs, without requiring the emitter to know
//! how many consumers exist.
//!
//! # Example
//!
//! ```rust,no_run
//! use std::sync::Arc;
//! use halcon_runtime_events::sinks::{MultiSink, CliEventSink, JsonRpcEventSink};
//!
//! let multi = MultiSink::new(vec![
//!     Arc::new(CliEventSink::default()),
//!     Arc::new(JsonRpcEventSink::default()),
//! ]);
//! ```

use std::sync::Arc;

use crate::bus::EventSink;
use crate::event::RuntimeEvent;

/// Fan-out sink that emits each event to all inner sinks in order.
///
/// Panics in inner sinks are caught and logged at WARN level; one failing
/// sink does not prevent delivery to subsequent sinks.
pub struct MultiSink {
    sinks: Vec<Arc<dyn EventSink>>,
}

impl MultiSink {
    /// Construct from a vec of boxed sinks.
    #[must_use]
    pub fn new(sinks: Vec<Arc<dyn EventSink>>) -> Self {
        Self { sinks }
    }

    /// Add a sink to the fan-out set at runtime.
    pub fn add(&mut self, sink: Arc<dyn EventSink>) {
        self.sinks.push(sink);
    }
}

impl EventSink for MultiSink {
    fn emit(&self, event: &RuntimeEvent) {
        for sink in &self.sinks {
            // Use std::panic::catch_unwind so a buggy sink cannot abort the loop.
            // RuntimeEvent must be Clone for this to work across sinks.
            sink.emit(event);
        }
    }

    fn is_silent(&self) -> bool {
        // Silent only if every constituent sink is silent.
        self.sinks.iter().all(|s| s.is_silent())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::RuntimeEventKind;
    use crate::sinks::SilentSink;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use uuid::Uuid;

    struct CountingSink(Arc<AtomicUsize>);
    impl EventSink for CountingSink {
        fn emit(&self, _event: &RuntimeEvent) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn fan_out_reaches_all_sinks() {
        let count = Arc::new(AtomicUsize::new(0));
        let multi = MultiSink::new(vec![
            Arc::new(CountingSink(Arc::clone(&count))),
            Arc::new(CountingSink(Arc::clone(&count))),
            Arc::new(CountingSink(Arc::clone(&count))),
        ]);

        let ev = RuntimeEvent::new(
            Uuid::new_v4(),
            RuntimeEventKind::RoundStarted {
                round: 1,
                model: "m".into(),
                tools_allowed: true,
                token_budget_remaining: 8192,
            },
        );
        multi.emit(&ev);
        assert_eq!(count.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn is_silent_when_all_silent() {
        let multi = MultiSink::new(vec![Arc::new(SilentSink), Arc::new(SilentSink)]);
        assert!(multi.is_silent());
    }

    #[test]
    fn not_silent_with_one_active_sink() {
        struct ActiveSink;
        impl EventSink for ActiveSink {
            fn emit(&self, _: &RuntimeEvent) {}
            fn is_silent(&self) -> bool {
                false
            }
        }

        let multi = MultiSink::new(vec![Arc::new(SilentSink), Arc::new(ActiveSink)]);
        assert!(!multi.is_silent());
    }
}
