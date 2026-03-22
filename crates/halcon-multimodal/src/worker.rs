//! CPU-bound inference worker pool.
//!
//! Routes CPU-intensive tasks (ONNX vision, Whisper audio) through a dedicated
//! rayon thread pool, bridged back to async callers via tokio oneshot channels.
//! This prevents blocking the tokio blocking pool (which has a 512-thread limit).

use std::sync::Arc;

use tokio::sync::oneshot;

use crate::error::{MultimodalError, Result};

/// CPU-bound worker pool backed by rayon.
///
/// Provides an async-safe interface for submitting CPU-intensive tasks
/// (ONNX inference, Whisper transcription) without blocking the tokio runtime.
pub struct MediaWorkerPool {
    pool: rayon::ThreadPool,
}

impl MediaWorkerPool {
    /// Create a new worker pool with `num_threads` threads.
    ///
    /// If `num_threads` is 0, uses rayon's default (CPU count).
    pub fn new(num_threads: usize) -> Result<Arc<Self>> {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(num_threads)
            .thread_name(|i| format!("halcon-media-{i}"))
            .build()
            .map_err(|e| MultimodalError::WorkerError(e.to_string()))?;
        Ok(Arc::new(Self { pool }))
    }

    /// Submit a CPU-bound task and await its result asynchronously.
    ///
    /// The closure runs on the rayon pool; results return via a tokio oneshot channel.
    pub async fn submit<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce() -> Result<R> + Send + 'static,
        R: Send + 'static,
    {
        let (tx, rx) = oneshot::channel::<Result<R>>();
        self.pool.spawn(move || {
            let _ = tx.send(f());
        });
        rx.await
            .map_err(|_| MultimodalError::WorkerError("worker channel dropped".to_string()))?
    }

    /// Number of threads in the pool.
    pub fn num_threads(&self) -> usize {
        self.pool.current_num_threads()
    }
}

impl std::fmt::Debug for MediaWorkerPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaWorkerPool")
            .field("num_threads", &self.pool.current_num_threads())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn submit_cpu_work_returns_result() {
        let pool = MediaWorkerPool::new(2).unwrap();
        let result = pool
            .submit(|| Ok::<u32, MultimodalError>(42))
            .await
            .unwrap();
        assert_eq!(result, 42);
    }

    #[tokio::test]
    async fn submit_propagates_error() {
        let pool = MediaWorkerPool::new(1).unwrap();
        let err = pool
            .submit(|| Err::<u32, MultimodalError>(MultimodalError::Internal("test".into())))
            .await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn parallel_submissions_complete() {
        let pool = MediaWorkerPool::new(4).unwrap();
        let pool = Arc::clone(&pool);
        let mut handles = Vec::new();
        for i in 0u32..8 {
            let p = Arc::clone(&pool);
            handles.push(tokio::spawn(async move {
                p.submit(move || Ok::<u32, MultimodalError>(i * 2))
                    .await
                    .unwrap()
            }));
        }
        let mut results: Vec<u32> = futures::future::join_all(handles)
            .await
            .into_iter()
            .map(|r| r.unwrap())
            .collect();
        results.sort_unstable();
        assert_eq!(results, (0u32..8).map(|i| i * 2).collect::<Vec<_>>());
    }

    #[test]
    fn num_threads_matches_construction() {
        let pool = MediaWorkerPool::new(3).unwrap();
        assert_eq!(pool.num_threads(), 3);
    }
}
