//! Cenzontle AI platform provider.
//!
//! Connects to a Cenzontle instance using a JWT access token obtained via
//! the Zuclubit SSO OAuth 2.1 / PKCE flow (`halcon login cenzontle`).
//!
//! The chat endpoint (`POST /v1/llm/chat`) is OpenAI-compatible.
//! Models are discovered at construction time from `GET /v1/llm/models`.
//!
//! # Configuration
//!
//! - `CENZONTLE_BASE_URL` — base URL of the Cenzontle instance
//!   (default: `https://api.cenzontle.app`)
//! - `CENZONTLE_ACCESS_TOKEN` — JWT access token (takes precedence over keychain)
//!
//! Run `halcon login cenzontle` to perform the SSO browser flow and store the
//! token in the OS keychain automatically.

pub mod types;

use std::time::Duration;

use async_trait::async_trait;
use eventsource_stream::Eventsource as _;
use futures::stream::{self, BoxStream};
use futures::StreamExt;
use tracing::{debug, info, instrument, warn};

use halcon_core::error::{HalconError, Result};
use halcon_core::traits::ModelProvider;
use halcon_core::types::{HttpConfig, ModelChunk, ModelInfo, ModelRequest, TokenCost, ToolFormat};

use crate::http;
use crate::openai_compat::types::OpenAISseChunk;
use crate::openai_compat::OpenAICompatibleProvider;
use types::{CenzontleModelsResponse, CenzontleModel};

pub const DEFAULT_BASE_URL: &str = "https://api.cenzontle.app";
const PROVIDER_NAME: &str = "cenzontle";

/// Tier → context window / max output heuristics (Cenzontle doesn't always return these).
fn tier_context_window(tier: Option<&str>) -> u32 {
    match tier {
        Some("FLAGSHIP") => 200_000,
        Some("BALANCED") => 128_000,
        Some("FAST") => 64_000,
        Some("ECONOMY") => 32_000,
        _ => 128_000,
    }
}

fn tier_max_output(tier: Option<&str>) -> u32 {
    match tier {
        Some("FLAGSHIP") => 16_000,
        Some("BALANCED") => 8_192,
        Some("FAST") => 4_096,
        Some("ECONOMY") => 2_048,
        _ => 4_096,
    }
}

/// Cenzontle AI platform provider.
pub struct CenzonzleProvider {
    /// reqwest client for API calls.
    client: reqwest::Client,
    /// Bearer JWT access token (from SSO flow or env var).
    access_token: String,
    /// Cenzontle base URL, e.g. `https://api.cenzontle.app`.
    base_url: String,
    /// Chat endpoint: `{base_url}/v1/llm/chat`.
    chat_url: String,
    /// Models available to this account.
    models: Vec<ModelInfo>,
    http_config: HttpConfig,
    /// Inner OpenAI-compat provider — used only for request building.
    inner: OpenAICompatibleProvider,
}

impl std::fmt::Debug for CenzonzleProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CenzonzleProvider")
            .field("base_url", &self.base_url)
            .field("access_token", &"[REDACTED]")
            .finish()
    }
}

impl CenzonzleProvider {
    /// Create from an access token and a list of pre-fetched models.
    ///
    /// Prefer `from_token()` which calls the API to discover real models.
    pub fn new(access_token: String, base_url: Option<String>, models: Vec<ModelInfo>) -> Self {
        let http_config = HttpConfig::default();
        let base_url = base_url.unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
        let chat_url = format!("{}/v1/llm/chat", base_url);
        let client = http::build_client(&http_config);

        // Inner provider used only for build_request() — its base_url is unused.
        let inner = OpenAICompatibleProvider::new(
            PROVIDER_NAME.to_string(),
            access_token.clone(),
            format!("{}/v1/llm", base_url),
            models.clone(),
            http_config.clone(),
        );

        Self {
            client,
            access_token,
            base_url,
            chat_url,
            models,
            http_config,
            inner,
        }
    }

    /// Construct a provider by fetching models from the Cenzontle API.
    ///
    /// Returns `None` if the token is empty or the models endpoint is unreachable.
    pub async fn from_token(access_token: String, base_url: Option<String>) -> Option<Self> {
        if access_token.is_empty() {
            return None;
        }
        let base = base_url.unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
        let http_config = HttpConfig::default();
        let client = http::build_client(&http_config);

        let models = fetch_models(&client, &base, &access_token)
            .await
            .unwrap_or_else(|e| {
                warn!(error = %e, "Cenzontle: failed to fetch models, using empty list");
                Vec::new()
            });

        if models.is_empty() {
            warn!("Cenzontle: no models available for this account");
        } else {
            info!(count = models.len(), "Cenzontle: discovered models");
        }

        Some(Self::new(access_token, Some(base), models))
    }

}

/// Fetch the model list from Cenzontle `GET /v1/llm/models`.
async fn fetch_models(
    client: &reqwest::Client,
    base_url: &str,
    access_token: &str,
) -> Result<Vec<ModelInfo>> {
    let url = format!("{}/v1/llm/models", base_url);
    let resp = client
        .get(&url)
        .bearer_auth(access_token)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| HalconError::ConnectionError {
            provider: PROVIDER_NAME.to_string(),
            message: format!("Cannot reach Cenzontle at {base_url}: {e}"),
        })?;

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        return Err(HalconError::ApiError {
            message: format!("Cenzontle /v1/llm/models returned HTTP {status}"),
            status: Some(status),
        });
    }

    let body: CenzontleModelsResponse = resp.json().await.map_err(|e| HalconError::ApiError {
        message: format!("Failed to parse Cenzontle models response: {e}"),
        status: None,
    })?;

    let models = body
        .data
        .into_iter()
        .map(|m| model_info_from_cenzontle(m))
        .collect();

    Ok(models)
}

fn model_info_from_cenzontle(m: CenzontleModel) -> ModelInfo {
    let tier = m.tier.as_deref();
    ModelInfo {
        id: m.id.clone(),
        name: m.name.unwrap_or_else(|| m.id.clone()),
        provider: PROVIDER_NAME.to_string(),
        context_window: m.context_window.unwrap_or_else(|| tier_context_window(tier)),
        max_output_tokens: m.max_output_tokens.unwrap_or_else(|| tier_max_output(tier)),
        supports_streaming: m.supports_streaming,
        supports_tools: m.supports_tools,
        supports_vision: m.supports_vision,
        supports_reasoning: false,
        cost_per_input_token: 0.0,  // billed through Cenzontle account
        cost_per_output_token: 0.0,
    }
}

// Token loading from OS keychain is intentionally NOT done here.
// halcon-providers does not depend on halcon-auth.
// The provider_factory (halcon-cli) is responsible for resolving the token
// from env var or keychain before calling CenzonzleProvider::new().

/// Build an SSE stream from a Cenzontle chat response (OpenAI-compatible format).
fn build_sse_stream(response: reqwest::Response) -> BoxStream<'static, Result<ModelChunk>> {
    let byte_stream = response.bytes_stream();
    let sse_stream = byte_stream.eventsource();

    let chunk_stream = sse_stream.flat_map(|sse_result| match sse_result {
        Ok(event) => {
            let data = event.data;
            if data.trim() == "[DONE]" {
                return stream::iter(vec![]);
            }
            match serde_json::from_str::<OpenAISseChunk>(&data) {
                Ok(chunk) => {
                    let mapped: Vec<Result<ModelChunk>> =
                        OpenAICompatibleProvider::map_sse_chunk(&chunk)
                            .into_iter()
                            .map(Ok)
                            .collect();
                    stream::iter(mapped)
                }
                Err(e) => {
                    warn!(error = %e, data = %data, "Cenzontle: failed to parse SSE chunk");
                    stream::iter(vec![])
                }
            }
        }
        Err(e) => stream::iter(vec![Err(HalconError::StreamError(format!(
            "Cenzontle SSE error: {e}"
        )))]),
    });

    Box::pin(chunk_stream)
}

#[async_trait]
impl ModelProvider for CenzonzleProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn supported_models(&self) -> &[ModelInfo] {
        &self.models
    }

    fn tool_format(&self) -> ToolFormat {
        ToolFormat::OpenAIFunctionObject
    }

    #[instrument(skip_all, fields(provider = "cenzontle", model = %request.model, msgs = request.messages.len()))]
    async fn invoke(
        &self,
        request: &ModelRequest,
    ) -> Result<BoxStream<'static, Result<ModelChunk>>> {
        // Use the inner provider to build the OpenAI-compatible request body.
        let chat_request = self.inner.build_request(request);
        let max_retries = self.http_config.max_retries;
        let timeout_secs = self.http_config.request_timeout_secs;

        debug!(
            model = %chat_request.model,
            messages = chat_request.messages.len(),
            url = %self.chat_url,
            "Cenzontle: invoking chat API"
        );

        for attempt in 0..=max_retries {
            if attempt > 0 {
                let delay = http::backoff_delay(1000, attempt);
                tokio::time::sleep(delay).await;
            }

            let result = tokio::time::timeout(
                Duration::from_secs(timeout_secs),
                self.client
                    .post(&self.chat_url)
                    .bearer_auth(&self.access_token)
                    .json(&chat_request)
                    .send(),
            )
            .await;

            let response = match result {
                Ok(Ok(resp)) => resp,
                Ok(Err(e)) if e.is_connect() => {
                    if attempt < max_retries {
                        warn!(attempt = attempt + 1, "Cenzontle: connection error, retrying");
                        continue;
                    }
                    return Err(HalconError::ConnectionError {
                        provider: PROVIDER_NAME.to_string(),
                        message: format!("Cannot connect to {}: {e}", self.base_url),
                    });
                }
                Ok(Err(e)) => {
                    return Err(HalconError::ApiError {
                        message: format!("Cenzontle request failed: {e}"),
                        status: e.status().map(|s| s.as_u16()),
                    });
                }
                Err(_) => {
                    if attempt < max_retries {
                        warn!(attempt = attempt + 1, "Cenzontle: request timeout, retrying");
                        continue;
                    }
                    return Err(HalconError::ApiError {
                        message: format!("Cenzontle request timed out after {timeout_secs}s"),
                        status: None,
                    });
                }
            };

            let status = response.status();
            if status == reqwest::StatusCode::UNAUTHORIZED {
                return Err(HalconError::ApiError {
                    message: "Cenzontle: access token expired or invalid. Run `halcon login cenzontle` to refresh.".to_string(),
                    status: Some(401),
                });
            }
            if status == reqwest::StatusCode::FORBIDDEN {
                return Err(HalconError::ApiError {
                    message: "Cenzontle: insufficient permissions for this model.".to_string(),
                    status: Some(403),
                });
            }
            if !status.is_success() {
                let code = status.as_u16();
                let body = response.text().await.unwrap_or_default();
                return Err(HalconError::ApiError {
                    message: format!("Cenzontle HTTP {code}: {body}"),
                    status: Some(code),
                });
            }

            return Ok(build_sse_stream(response));
        }

        Err(HalconError::ApiError {
            message: "Cenzontle: all retry attempts exhausted".to_string(),
            status: None,
        })
    }

    async fn is_available(&self) -> bool {
        let url = format!("{}/v1/auth/me", self.base_url);
        self.client
            .get(&url)
            .bearer_auth(&self.access_token)
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    fn estimate_cost(&self, _request: &ModelRequest) -> TokenCost {
        // Billed through Cenzontle account — not tracked locally.
        TokenCost::default()
    }
}
