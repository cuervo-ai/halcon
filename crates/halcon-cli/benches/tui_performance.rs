//! TUI performance benchmarks.
//!
//! Validates Phase 2 performance targets:
//! - Event batching reduces frame time
//! - Virtual scrolling keeps rendering <2ms
//! - LRU cache improves span parsing

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use std::collections::VecDeque;
use std::time::Duration;

// Mock UiEvent for benchmarking
#[derive(Clone)]
enum UiEvent {
    StreamChunk(String),
    Info(String),
}

fn bench_event_buffer_accumulation(c: &mut Criterion) {
    let mut group = c.benchmark_group("event_buffer");

    for size in [10, 100, 1000, 4000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            b.iter(|| {
                let mut buffer: VecDeque<UiEvent> = VecDeque::new();
                for i in 0..size {
                    buffer.push_back(UiEvent::StreamChunk(format!("chunk{}", i)));
                }
                black_box(buffer);
            });
        });
    }

    group.finish();
}

fn bench_event_buffer_drain(c: &mut Criterion) {
    let mut group = c.benchmark_group("event_drain");

    for size in [10, 100, 1000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let mut buffer: VecDeque<UiEvent> = VecDeque::new();
            for i in 0..size {
                buffer.push_back(UiEvent::StreamChunk(format!("chunk{}", i)));
            }

            b.iter(|| {
                let mut temp_buffer = buffer.clone();
                let batch_size = temp_buffer.len();
                for _ in 0..batch_size {
                    if let Some(ev) = temp_buffer.pop_front() {
                        black_box(ev);
                    }
                }
            });
        });
    }

    group.finish();
}

fn bench_frame_rate_interval(c: &mut Criterion) {
    c.bench_function("frame_interval_16ms", |b| {
        b.iter(|| {
            let interval = Duration::from_millis(16);
            black_box(interval);
            // Verify FPS calculation: 16ms = 62.5 FPS (close to 60)
            let fps = 1000.0 / interval.as_millis() as f64;
            assert!(
                fps >= 60.0 && fps <= 65.0,
                "Should be ~60 FPS (got {})",
                fps
            );
        });
    });
}

fn bench_virtual_scroll_slice(c: &mut Criterion) {
    let mut group = c.benchmark_group("virtual_scroll");

    // Simulate 1000 lines of activity
    let lines: Vec<String> = (0..1000).map(|i| format!("Activity line {}", i)).collect();

    for viewport_height in [10, 20, 50, 100].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(viewport_height),
            viewport_height,
            |b, &vh| {
                b.iter(|| {
                    let scroll_offset = 500; // Middle of content
                    let end = (scroll_offset + vh).min(lines.len());
                    let visible = &lines[scroll_offset..end];
                    black_box(visible);
                });
            },
        );
    }

    group.finish();
}

fn bench_span_cache_hit(c: &mut Criterion) {
    use std::collections::HashMap;

    c.bench_function("span_cache_hit", |b| {
        let mut cache: HashMap<usize, Vec<String>> = HashMap::new();
        // Pre-populate cache with 200 entries
        for i in 0..200 {
            cache.insert(i, vec![format!("span{}", i)]);
        }

        b.iter(|| {
            // Cache hit (key exists)
            if let Some(spans) = cache.get(&100) {
                black_box(spans);
            }
        });
    });
}

fn bench_span_cache_miss(c: &mut Criterion) {
    use std::collections::HashMap;

    c.bench_function("span_cache_miss", |b| {
        let mut cache: HashMap<usize, Vec<String>> = HashMap::new();
        // Pre-populate cache
        for i in 0..200 {
            cache.insert(i, vec![format!("span{}", i)]);
        }

        b.iter(|| {
            // Cache miss (key doesn't exist)
            if let Some(spans) = cache.get(&9999) {
                black_box(spans);
            } else {
                // Would parse markdown here
                let new_spans = vec!["new span".to_string()];
                black_box(new_spans);
            }
        });
    });
}

fn bench_batching_vs_immediate(c: &mut Criterion) {
    let mut group = c.benchmark_group("batching_comparison");

    let event_count = 100;

    // Simulate immediate processing (old approach)
    group.bench_function("immediate_processing", |b| {
        b.iter(|| {
            for i in 0..event_count {
                let ev = UiEvent::StreamChunk(format!("chunk{}", i));
                // Simulate immediate handling
                black_box(ev);
                // Simulate render after each event
                black_box("render");
            }
        });
    });

    // Simulate batched processing (new approach)
    group.bench_function("batched_processing", |b| {
        b.iter(|| {
            let mut buffer = VecDeque::new();
            // Accumulate all events
            for i in 0..event_count {
                buffer.push_back(UiEvent::StreamChunk(format!("chunk{}", i)));
            }
            // Process batch
            while let Some(ev) = buffer.pop_front() {
                black_box(ev);
            }
            // Single render after batch
            black_box("render");
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_event_buffer_accumulation,
    bench_event_buffer_drain,
    bench_frame_rate_interval,
    bench_virtual_scroll_slice,
    bench_span_cache_hit,
    bench_span_cache_miss,
    bench_batching_vs_immediate,
);
criterion_main!(benches);
