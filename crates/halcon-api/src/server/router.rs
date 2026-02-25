use axum::{
    middleware,
    routing::{delete, get, patch, post},
    Router,
};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::trace::TraceLayer;

use super::auth::auth_middleware;
use super::handlers;
use super::state::AppState;
use super::ws::ws_handler;

/// Build the full API router with all routes, middleware, and state.
pub fn build_router(state: AppState) -> Router {
    let api_routes = Router::new()
        // Agent endpoints
        .route("/agents", get(handlers::agents::list_agents))
        .route("/agents/:id", get(handlers::agents::get_agent))
        .route("/agents/:id", delete(handlers::agents::stop_agent))
        .route("/agents/:id/invoke", post(handlers::agents::invoke_agent))
        .route("/agents/:id/health", get(handlers::agents::agent_health))
        // Task endpoints
        .route("/tasks", get(handlers::tasks::list_tasks))
        .route("/tasks", post(handlers::tasks::submit_task))
        .route("/tasks/:id", get(handlers::tasks::get_task))
        .route("/tasks/:id", delete(handlers::tasks::cancel_task))
        // Tool endpoints
        .route("/tools", get(handlers::tools::list_tools))
        .route("/tools/:name/toggle", post(handlers::tools::toggle_tool))
        .route("/tools/:name/history", get(handlers::tools::tool_history))
        // Observability endpoints
        .route("/metrics", get(handlers::observability::get_metrics))
        // System endpoints
        .route("/system/status", get(handlers::system::get_status))
        .route("/system/shutdown", post(handlers::system::shutdown))
        // Config endpoints
        .route(
            "/system/config",
            get(handlers::config::get_config).put(handlers::config::update_config),
        )
        // Chat endpoints
        .route("/chat/sessions", get(handlers::chat::list_sessions).post(handlers::chat::create_session))
        .route("/chat/sessions/:id", get(handlers::chat::get_session).delete(handlers::chat::delete_session).patch(handlers::chat::update_session))
        .route("/chat/sessions/:id/messages", get(handlers::chat::list_messages).post(handlers::chat::submit_message))
        .route("/chat/sessions/:id/active", delete(handlers::chat::cancel_active))
        .route("/chat/sessions/:id/permissions/:req_id", post(handlers::chat::resolve_permission));

    // Routes that require Bearer token authentication.
    // The auth middleware is scoped to this sub-router so it is impossible for
    // a future refactor to accidentally expose protected routes without auth.
    let protected = Router::new()
        .nest("/api/v1", api_routes)
        .route("/ws/events", get(ws_handler))
        .layer(middleware::from_fn_with_state(state.clone(), auth_middleware));

    Router::new()
        // Health check is explicitly PUBLIC — no auth, no state required.
        .route("/health", get(health_check))
        .merge(protected)
        .layer(
            // Restrict CORS to localhost origins only.
            // This prevents cross-origin browser requests from arbitrary websites
            // while allowing egui desktop clients (no Origin header) to connect freely.
            CorsLayer::new()
                .allow_origin(AllowOrigin::predicate(|origin, _req| {
                    let b = origin.as_bytes();
                    b.starts_with(b"http://127.0.0.1")
                        || b.starts_with(b"http://localhost")
                        || b.starts_with(b"https://127.0.0.1")
                        || b.starts_with(b"https://localhost")
                }))
                .allow_methods([
                    axum::http::Method::GET,
                    axum::http::Method::POST,
                    axum::http::Method::PUT,
                    axum::http::Method::DELETE,
                    axum::http::Method::OPTIONS,
                ])
                .allow_headers([
                    axum::http::header::AUTHORIZATION,
                    axum::http::header::CONTENT_TYPE,
                    axum::http::header::ACCEPT,
                ]),
        )
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Simple health check (no auth required).
async fn health_check() -> &'static str {
    "ok"
}
