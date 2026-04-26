use dashmap::DashMap;
use halcon_core::traits::ChatExecutor;
use halcon_core::types::AppConfig;
use halcon_runtime::runtime::HalconRuntime;
use halcon_storage::AsyncDatabase;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{broadcast, RwLock};
use uuid::Uuid;

use crate::types::chat::{ChatSessionHandle, PersistableSession};
use crate::types::ws::WsServerEvent;

/// Shared application state for the API server.
#[derive(Clone)]
pub struct AppState {
    pub runtime: Arc<HalconRuntime>,
    pub auth_token: Arc<String>,
    pub started_at: Instant,
    pub event_tx: broadcast::Sender<WsServerEvent>,
    pub tool_states: Arc<RwLock<HashMap<String, ToolState>>>,
    pub task_executions: Arc<RwLock<HashMap<Uuid, crate::types::task::TaskExecution>>>,
    pub config: Arc<RwLock<AppConfig>>,
    /// Optional database for real metrics. `None` when server starts without a DB path.
    pub db: Option<Arc<AsyncDatabase>>,
    /// Active chat session handles keyed by session ID.
    pub active_chat_sessions: Arc<DashMap<Uuid, ChatSessionHandle>>,
    /// Optional chat executor — wired by halcon-cli at server startup.
    /// When None, submit_message returns 501 Not Implemented.
    pub chat_executor: Option<Arc<dyn ChatExecutor>>,
    /// Pending permission decisions: session_id → (request_id, approved) sender.
    pub perm_senders: Arc<DashMap<Uuid, tokio::sync::mpsc::UnboundedSender<(Uuid, bool)>>>,
    /// Path to persist chat sessions across server restarts.
    pub sessions_file: Option<PathBuf>,
    /// Cancellation tokens for running task executions.
    /// Keyed by execution_id; removed when execution completes or is cancelled.
    pub task_cancel_tokens: Arc<DashMap<Uuid, tokio_util::sync::CancellationToken>>,
}

/// Tracked state for a tool (enable/disable, execution count).
#[derive(Debug, Clone)]
pub struct ToolState {
    pub enabled: bool,
    pub execution_count: u64,
    pub last_executed: Option<chrono::DateTime<chrono::Utc>>,
}

impl AppState {
    /// Create new server state wrapping the given runtime.
    pub fn new(runtime: Arc<HalconRuntime>, auth_token: String) -> Self {
        let (event_tx, _) = broadcast::channel(4096);
        Self {
            runtime,
            auth_token: Arc::new(auth_token),
            started_at: Instant::now(),
            event_tx,
            tool_states: Arc::new(RwLock::new(HashMap::new())),
            task_executions: Arc::new(RwLock::new(HashMap::new())),
            config: Arc::new(RwLock::new(AppConfig::default())),
            db: None,
            active_chat_sessions: Arc::new(DashMap::new()),
            chat_executor: None,
            perm_senders: Arc::new(DashMap::new()),
            sessions_file: None,
            task_cancel_tokens: Arc::new(DashMap::new()),
        }
    }

    /// Attach a ChatExecutor (injected by halcon-cli at server startup).
    pub fn with_chat_executor(mut self, executor: Arc<dyn ChatExecutor>) -> Self {
        self.chat_executor = Some(executor);
        self
    }

    /// Attach a database for real metrics queries.
    ///
    /// Builder-pattern: `AppState::new(...).with_db(db)`.
    /// If not called, all metrics endpoints return zeros/empty (backward compatible).
    pub fn with_db(mut self, db: Arc<AsyncDatabase>) -> Self {
        self.db = Some(db);
        self
    }

    /// Attach a persistence file path for chat session cross-restart durability.
    pub fn with_sessions_file(mut self, path: PathBuf) -> Self {
        self.sessions_file = Some(path);
        self
    }

    /// Broadcast a WebSocket event to all connected clients.
    pub fn broadcast(&self, event: WsServerEvent) {
        // Ignore send error (no subscribers).
        let _ = self.event_tx.send(event);
    }

    /// Get uptime in seconds.
    pub fn uptime_seconds(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }

    /// Persist all active chat sessions + their histories to the sessions file.
    ///
    /// Non-blocking: serialization happens synchronously, but the file write is
    /// spawned as a background task so callers never wait on disk I/O.
    pub async fn persist_sessions(&self) {
        let path = match self.sessions_file {
            Some(ref p) => p.clone(),
            None => return,
        };

        // Step 1: Collect session metadata + Arc<Mutex<history>> without holding DashMap locks.
        type ChatEntry = (
            crate::types::chat::ChatSession,
            std::sync::Arc<tokio::sync::Mutex<Vec<(String, String)>>>,
        );
        let entries: Vec<ChatEntry> = self
            .active_chat_sessions
            .iter()
            .map(|e| {
                (
                    e.value().session.clone(),
                    std::sync::Arc::clone(&e.value().history),
                )
            })
            .collect();

        // Step 2: Async-lock each history Arc (DashMap refs are dropped).
        let mut snapshots: Vec<PersistableSession> = Vec::with_capacity(entries.len());
        for (session, history_arc) in entries {
            let history = history_arc.lock().await.clone();
            snapshots.push(PersistableSession { session, history });
        }

        // Step 3: Write to disk in a background task.
        match serde_json::to_string_pretty(&snapshots) {
            Ok(json) => {
                tokio::spawn(async move {
                    if let Some(parent) = path.parent() {
                        let _ = tokio::fs::create_dir_all(parent).await;
                    }
                    if let Err(e) = tokio::fs::write(&path, &json).await {
                        tracing::warn!(error = %e, "failed to persist chat sessions");
                    }
                });
            }
            Err(e) => tracing::warn!(error = %e, "failed to serialize chat sessions"),
        }
    }

    /// Load previously persisted chat sessions from the sessions file into AppState.
    ///
    /// Called once at server startup before the router is built.
    pub async fn load_sessions_from_file(&self) {
        let path = match self.sessions_file {
            Some(ref p) => p.clone(),
            None => return,
        };

        if !path.exists() {
            return;
        }

        match tokio::fs::read_to_string(&path).await {
            Ok(json) => {
                match serde_json::from_str::<Vec<PersistableSession>>(&json) {
                    Ok(snapshots) => {
                        let count = snapshots.len();
                        for ps in snapshots {
                            let (handle, _cancel_rx) = ChatSessionHandle::new(ps.session);
                            // Restore conversation history.
                            {
                                let mut h = handle.history.lock().await;
                                *h = ps.history;
                            }
                            self.active_chat_sessions.insert(handle.session.id, handle);
                        }
                        tracing::info!(count, "restored chat sessions from disk");
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, path = ?path, "failed to parse chat sessions file — starting fresh")
                    }
                }
            }
            Err(e) => tracing::warn!(error = %e, "failed to read chat sessions file"),
        }
    }
}
