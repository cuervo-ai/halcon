use futures_util::{SinkExt, StreamExt};
use halcon_api::types::ws::{WsChannel, WsClientMessage, WsServerEvent};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::config::ClientConfig;
use crate::error::ClientError;

/// A WebSocket event stream connection to the control plane.
pub struct EventStream {
    /// Channel receiving server events.
    pub rx: mpsc::UnboundedReceiver<WsServerEvent>,
    /// Handle to the background connection task.
    _task: tokio::task::JoinHandle<()>,
    /// Channel for sending commands to the WebSocket.
    cmd_tx: mpsc::UnboundedSender<WsClientMessage>,
}

impl EventStream {
    /// Connect to the WebSocket event stream.
    ///
    /// Authenticates via `Authorization: Bearer <token>` header (not query param).
    pub async fn connect(config: &ClientConfig) -> Result<Self, ClientError> {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;

        let ws_url = config.ws_url();

        // Build the WS handshake request using tungstenite's own IntoClientRequest
        // so it can generate the required Sec-WebSocket-Key header automatically.
        // Then inject the Authorization header for the server's auth middleware.
        let mut request = ws_url
            .as_str()
            .into_client_request()
            .map_err(|e| ClientError::WebSocket(e.to_string()))?;
        request.headers_mut().insert(
            "Authorization",
            format!("Bearer {}", config.auth_token).parse().map_err(
                |e: http::header::InvalidHeaderValue| ClientError::WebSocket(e.to_string()),
            )?,
        );

        let (ws_stream, _response) = tokio_tungstenite::connect_async(request)
            .await
            .map_err(|e| ClientError::WebSocket(e.to_string()))?;

        let (mut ws_sink, mut ws_source) = ws_stream.split();
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<WsClientMessage>();

        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    // Forward incoming WebSocket messages to the event channel.
                    msg = ws_source.next() => {
                        match msg {
                            Some(Ok(Message::Text(text))) => {
                                if let Ok(event) = serde_json::from_str::<WsServerEvent>(&text) {
                                    if event_tx.send(event).is_err() {
                                        break;
                                    }
                                }
                            }
                            Some(Ok(Message::Close(_))) | None => break,
                            _ => {}
                        }
                    }
                    // Forward outgoing commands to the WebSocket.
                    cmd = cmd_rx.recv() => {
                        match cmd {
                            Some(msg) => {
                                if let Ok(json) = serde_json::to_string(&msg) {
                                    if ws_sink.send(Message::Text(json)).await.is_err() {
                                        break;
                                    }
                                }
                            }
                            None => break,
                        }
                    }
                }
            }
        });

        Ok(Self {
            rx: event_rx,
            _task: task,
            cmd_tx,
        })
    }

    /// Subscribe to specific event channels.
    pub fn subscribe(&self, channels: Vec<WsChannel>) -> Result<(), ClientError> {
        self.cmd_tx
            .send(WsClientMessage::Subscribe { channels })
            .map_err(|_| ClientError::NotConnected)
    }

    /// Unsubscribe from event channels.
    pub fn unsubscribe(&self, channels: Vec<WsChannel>) -> Result<(), ClientError> {
        self.cmd_tx
            .send(WsClientMessage::Unsubscribe { channels })
            .map_err(|_| ClientError::NotConnected)
    }

    /// Send a ping to keep the connection alive.
    pub fn ping(&self) -> Result<(), ClientError> {
        self.cmd_tx
            .send(WsClientMessage::Ping)
            .map_err(|_| ClientError::NotConnected)
    }

    /// Receive the next event (async).
    pub async fn next_event(&mut self) -> Option<WsServerEvent> {
        self.rx.recv().await
    }
}
