//! Cenzontle API response types for model listing and auth profile.

use serde::Deserialize;

/// A single model returned by `GET /v1/llm/models`.
#[derive(Debug, Deserialize)]
pub struct CenzontleModel {
    pub id: String,
    pub name: Option<String>,
    pub tier: Option<String>,
    #[serde(default)]
    pub context_window: Option<u32>,
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
    #[serde(default = "default_true")]
    pub supports_streaming: bool,
    #[serde(default = "default_true")]
    pub supports_tools: bool,
    #[serde(default)]
    pub supports_vision: bool,
}

fn default_true() -> bool {
    true
}

/// Response from `GET /v1/llm/models`.
#[derive(Debug, Deserialize)]
pub struct CenzontleModelsResponse {
    pub data: Vec<CenzontleModel>,
}

/// Auth profile from `GET /v1/auth/me`.
#[derive(Debug, Deserialize)]
pub struct CenzontleAuthMe {
    pub sub: String,
    pub email: Option<String>,
    pub tenant_slug: Option<String>,
    pub role: Option<String>,
}
