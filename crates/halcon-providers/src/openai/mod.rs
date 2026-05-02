//! OpenAI provider — thin wrapper over `OpenAICompatibleProvider`.
//!
//! Models: gpt-4o, gpt-4o-mini, o1, o3-mini.
//! Default base URL: `https://api.openai.com/v1`
//! Env var: `OPENAI_API_KEY`

use async_trait::async_trait;
use futures::stream::BoxStream;
use tracing::instrument;

use halcon_core::error::Result;
use halcon_core::traits::ModelProvider;
use halcon_core::types::{HttpConfig, ModelChunk, ModelInfo, ModelRequest, TokenCost};

use crate::openai_compat::OpenAICompatibleProvider;

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

/// OpenAI provider for GPT-4o and o-series models.
pub struct OpenAIProvider {
    inner: OpenAICompatibleProvider,
}

impl std::fmt::Debug for OpenAIProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAIProvider")
            .field("inner", &self.inner)
            .finish()
    }
}

impl OpenAIProvider {
    /// Create a new OpenAI provider.
    pub fn new(api_key: String, base_url: Option<String>, http_config: HttpConfig) -> Self {
        let url = base_url.unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
        Self {
            inner: OpenAICompatibleProvider::new(
                "openai".into(),
                api_key,
                url,
                Self::default_models(),
                http_config,
            ),
        }
    }

    /// Static fallback — used at construction, replaced by live discovery.
    fn default_models() -> Vec<ModelInfo> {
        crate::model_registry::static_fallback_models("openai")
    }

    /// Discover models from OpenAI's live `/v1/models` endpoint.
    ///
    /// OpenAI implements the standard `/v1/models` API. We query it, then
    /// enrich model IDs with known capabilities from the static registry.
    /// New models released after the Halcon build are automatically discovered
    /// and get inferred capabilities.
    pub async fn discover_models(&mut self) {
        let models = crate::model_registry::discover_provider_models(
            self.inner.client(),
            "openai",
            self.inner.base_url(),
            self.inner.api_key(),
        )
        .await;
        if !models.is_empty() {
            self.inner.set_models(models);
        }
    }
}

#[async_trait]
impl ModelProvider for OpenAIProvider {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn supported_models(&self) -> &[ModelInfo] {
        self.inner.supported_models()
    }

    #[instrument(skip_all, fields(provider = "openai", model = %request.model, msgs = request.messages.len()))]
    async fn invoke(
        &self,
        request: &ModelRequest,
    ) -> Result<BoxStream<'static, Result<ModelChunk>>> {
        // Pre-flight payload guard (mirrors the wiring in cenzontle::invoke).
        // Skips silently when the model is unknown to the local registry —
        // fresh deployments still pass through. When the model is known and
        // the estimate exceeds the declared context window, fail at the
        // client with `LlmError::PayloadTooLarge` rather than wasting a
        // round-trip to OpenAI's API for a 400/413.
        if let Some(model_info) = self
            .inner
            .supported_models()
            .iter()
            .find(|m| m.id == request.model)
        {
            let hint = self.tokenizer_hint();
            let estimator = crate::estimator::HeuristicTokenEstimator;
            let est = <crate::estimator::HeuristicTokenEstimator as crate::estimator::TokenEstimator>::estimate(
                &estimator,
                request,
                hint,
            );
            if let Err(llm_err) = crate::estimator::validate_request_fits(
                "openai",
                &request.model,
                est,
                model_info.context_window,
                crate::estimator::DEFAULT_SAFETY_BUFFER,
            ) {
                tracing::warn!(
                    model = %request.model,
                    est_tokens = est.total,
                    max_context = model_info.context_window,
                    dominant = est.dominant_source(),
                    "OpenAI: pre-flight payload guard rejected oversize request"
                );
                return Err(halcon_core::error::HalconError::from(llm_err));
            }
            // TPM guard (P1-1 wiring) — only enforced when the registry
            // reports `tpm`. Skips silently otherwise.
            if let Some(tpm) = model_info.tpm {
                if let Err(llm_err) = crate::estimator::validate_fits_tpm(
                    "openai",
                    &request.model,
                    est,
                    tpm,
                    crate::estimator::DEFAULT_TPM_SAFETY_FACTOR,
                ) {
                    tracing::warn!(
                        model = %request.model,
                        est_tokens = est.total,
                        tpm,
                        "OpenAI: pre-flight TPM guard rejected request larger than per-minute budget"
                    );
                    return Err(halcon_core::error::HalconError::from(llm_err));
                }
            }
        }
        self.inner.invoke(request).await
    }

    async fn is_available(&self) -> bool {
        self.inner.is_available().await
    }

    fn estimate_cost(&self, request: &ModelRequest) -> TokenCost {
        self.inner.estimate_cost(request)
    }

    fn tool_format(&self) -> halcon_core::types::ToolFormat {
        halcon_core::types::ToolFormat::OpenAIFunctionObject
    }

    fn tokenizer_hint(&self) -> halcon_core::types::TokenizerHint {
        halcon_core::types::TokenizerHint::TiktokenCl100k
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use halcon_core::types::{ChatMessage, MessageContent, Role};

    fn make_request(msg: &str) -> ModelRequest {
        ModelRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: MessageContent::Text(msg.into()),
            }],
            tools: vec![],
            max_tokens: Some(1024),
            temperature: Some(0.7),
            system: None,
            stream: true,
        }
    }

    #[test]
    fn name_is_openai() {
        let provider = OpenAIProvider::new("sk-test".into(), None, HttpConfig::default());
        assert_eq!(provider.name(), "openai");
    }

    #[test]
    fn supported_models_count() {
        let provider = OpenAIProvider::new("sk-test".into(), None, HttpConfig::default());
        let models = provider.supported_models();
        assert!(
            models.len() >= 3,
            "should have at least gpt-4o-mini + gpt-4o + o3-mini"
        );
        for m in models {
            assert_eq!(m.provider, "openai");
        }
    }

    #[tokio::test]
    async fn is_available_with_key() {
        let provider = OpenAIProvider::new("sk-test".into(), None, HttpConfig::default());
        assert!(provider.is_available().await);
    }

    #[test]
    fn estimate_cost_positive() {
        let provider = OpenAIProvider::new("sk-test".into(), None, HttpConfig::default());
        let req = make_request("test message for cost");
        let cost = provider.estimate_cost(&req);
        assert!(cost.estimated_input_tokens > 0);
        assert!(cost.estimated_cost_usd > 0.0);
    }

    #[test]
    fn debug_redacts_key() {
        let provider = OpenAIProvider::new("sk-secret-key".into(), None, HttpConfig::default());
        let debug = format!("{provider:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("sk-secret-key"));
    }

    #[test]
    fn custom_base_url() {
        let provider = OpenAIProvider::new(
            "sk-test".into(),
            Some("https://custom.openai.com/v1".into()),
            HttpConfig::default(),
        );
        let debug = format!("{provider:?}");
        assert!(debug.contains("custom.openai.com"));
    }
}
