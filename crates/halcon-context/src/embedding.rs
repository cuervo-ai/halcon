//! Embedding engine for the L3 vector semantic store.
//!
//! ## Engine hierarchy (Sprint 1 Q3 — 2026 update)
//!
//! Resolution order (first available wins):
//!
//! 1. **`FastEmbedEngine`** *(feature = "fastembed", Q3)* — `all-MiniLM-L6-v2`
//!    via ONNX Runtime, 384-dim, ~5 ms/chunk on CPU.  Downloaded once (~23 MB)
//!    to `$FASTEMBED_CACHE_PATH` or `~/.cache/fastembed`.  Activated when:
//!    - The feature flag is compiled in AND the model is already cached, OR
//!    - `HALCON_EMBEDDING_ENGINE=fastembed` is set explicitly (triggers download).
//!
//!    No server required. Gives **real semantic similarity** unlike the
//!    `TfIdfHashEngine` fallback.
//!
//! 2. **`OllamaEmbeddingEngine`** — neural multilingual embeddings via a local
//!    Ollama server.  Activated when `http://localhost:11434` (or `OLLAMA_HOST`)
//!    is reachable within 300 ms.  768-dim with `nomic-embed-text`.
//!
//! 3. **`TfIdfHashEngine`** — FNV-1a hash projection (384-dim, pure Rust).
//!    **Last resort only.**  Has zero semantic properties: synonym queries map to
//!    orthogonal vectors.  Retained for air-gapped environments where neither
//!    Ollama nor fastembed are available.
//!
//! Use `EmbeddingEngineFactory::from_env()` to obtain the best available engine.
//!
//! ## References (2026)
//!
//! - Reimers & Gurevych (2019) "Sentence-BERT" — all-MiniLM-L6-v2 architecture
//! - Wang et al. (2024) "Improving Text Embeddings with LLMs" (E5-mistral)
//! - Muennighoff et al. (2023) "MTEB: Massive Text Embedding Benchmark"

use std::time::Duration;

/// Default embedding dimensionality for TfIdfHashEngine.
/// 384 matches AllMiniLML6V2Q for drop-in upgrade.
/// Neural engines (Ollama) may return different dimensions depending on model.
pub const DIMS: usize = 384;

/// Default Ollama endpoint for local inference.
pub const OLLAMA_DEFAULT_ENDPOINT: &str = "http://localhost:11434";

/// Default multilingual model for Ollama.
/// nomic-embed-text: 768-dim, trained on 300M multilingual pairs, strong MTEB performance.
/// Alternative: "mxbai-embed-large" (1024-dim), "all-minilm:l6-v2" (384-dim, drop-in).
pub const OLLAMA_DEFAULT_MODEL: &str = "nomic-embed-text";

/// Trait for embedding text into a fixed-dimension float vector.
pub trait EmbeddingEngine: Send + Sync {
    /// Embed `text` into a `DIMS`-dimensional L2-normalized vector.
    fn embed(&self, text: &str) -> Vec<f32>;
}

// ── TF-IDF hash projection ────────────────────────────────────────────────────

/// FNV-1a 64-bit offset basis.
const FNV_OFFSET: u64 = 14_695_981_039_346_656_037;
/// FNV-1a prime.
const FNV_PRIME: u64 = 1_099_511_628_211;

/// Hash a string token to a dimension index in [0, DIMS).
fn fnv1a_dim(token: &str) -> usize {
    let mut hash = FNV_OFFSET;
    for byte in token.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    (hash as usize) % DIMS
}

/// Tokenize text into lowercase alphanumeric + underscore tokens of length ≥ 2.
fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|t| t.len() >= 2)
        .map(|t| t.to_ascii_lowercase())
        .collect()
}

/// Default embedding engine: TF-IDF hash projection with FNV-1a, 384 dims, L2-normalized.
///
/// Multiple tokens that hash to the same dimension accumulate their weights (projection).
pub struct TfIdfHashEngine;

impl EmbeddingEngine for TfIdfHashEngine {
    fn embed(&self, text: &str) -> Vec<f32> {
        let tokens = tokenize(text);
        if tokens.is_empty() {
            return vec![0.0; DIMS];
        }

        // Compute term frequencies.
        let total = tokens.len() as f32;
        let mut tf: std::collections::HashMap<String, f32> = std::collections::HashMap::new();
        for tok in &tokens {
            *tf.entry(tok.clone()).or_insert(0.0) += 1.0 / total;
        }

        // Project into DIMS-dimensional space.
        let mut vec = vec![0.0f32; DIMS];
        for (tok, weight) in &tf {
            let dim = fnv1a_dim(tok);
            vec[dim] += weight;
        }

        // L2-normalize.
        let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 1e-9 {
            for x in vec.iter_mut() {
                *x /= norm;
            }
        }

        vec
    }
}

/// Cosine similarity between two L2-normalized vectors (dot product).
pub fn cosine_sim(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

// ── OllamaEmbeddingEngine ─────────────────────────────────────────────────────

/// Neural multilingual embedding engine backed by a local Ollama server.
///
/// Calls `POST {endpoint}/api/embeddings` and returns an L2-normalized embedding
/// vector of the model's native dimensionality. The same endpoint supports any
/// model installed in Ollama — change `model` to switch between:
///
/// | Model                           | Dims | Languages | Notes                    |
/// |----------------------------------|------|-----------|--------------------------|
/// | nomic-embed-text                 |  768 | 100+      | Best multilingual balance |
/// | mxbai-embed-large                | 1024 | EN-heavy  | Highest EN MTEB score     |
/// | paraphrase-multilingual-minilm   |  384 | 50+       | Drop-in for DIMS=384      |
/// | all-minilm:l6-v2                 |  384 | EN        | Fastest, EN only          |
///
/// When the server is unreachable, `embed()` returns an empty `Vec<f32>`.
/// Use `EmbeddingEngineFactory::best_available()` to fall back gracefully.
pub struct OllamaEmbeddingEngine {
    client: reqwest::blocking::Client,
    endpoint: String,
    model: String,
}

impl OllamaEmbeddingEngine {
    /// Construct with explicit endpoint, model, and HTTP timeout.
    pub fn new(endpoint: &str, model: &str, timeout_ms: u64) -> Self {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_millis(timeout_ms))
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new());
        Self {
            client,
            endpoint: endpoint.trim_end_matches('/').to_string(),
            model: model.to_string(),
        }
    }

    /// Probe availability: embed a short string and return dimensionality on success.
    ///
    /// Returns `None` when the server is unreachable or returns an empty vector.
    pub fn probe(&self) -> Option<usize> {
        let v = self.embed("probe");
        if !v.is_empty() {
            Some(v.len())
        } else {
            None
        }
    }
}

impl EmbeddingEngine for OllamaEmbeddingEngine {
    fn embed(&self, text: &str) -> Vec<f32> {
        #[derive(serde::Serialize)]
        struct EmbedRequest<'a> {
            model: &'a str,
            prompt: &'a str,
        }

        #[derive(serde::Deserialize)]
        struct EmbedResponse {
            embedding: Vec<f32>,
        }

        let url = format!("{}/api/embeddings", self.endpoint);
        let req = EmbedRequest {
            model: &self.model,
            prompt: text,
        };

        let resp = match self.client.post(&url).json(&req).send() {
            Ok(r) => r,
            Err(_) => return vec![],
        };

        let body: EmbedResponse = match resp.json() {
            Ok(b) => b,
            Err(_) => return vec![],
        };

        if body.embedding.is_empty() {
            return vec![];
        }

        // L2-normalize so cosine_sim = dot product (consistent with TfIdfHashEngine).
        let mut v = body.embedding;
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 1e-9 {
            v.iter_mut().for_each(|x| *x /= norm);
        }
        v
    }
}

// ── FastEmbedEngine (Q3 — Sprint 1) ──────────────────────────────────────────
//
// Requires feature flag: cargo build --features fastembed
//
// Uses fastembed-rs (ONNX Runtime) with `all-MiniLM-L6-v2`:
//   - 384-dim output (same as DIMS constant — drop-in compatible)
//   - ~23 MB model download, cached at $FASTEMBED_CACHE_PATH or ~/.cache/fastembed
//   - ~5 ms per text chunk on a modern CPU (no GPU required)
//   - Real semantic similarity: cosine_sim("car", "automobile") ≈ 0.85
//   - Self-contained: no server process required
//
// Activation logic (EmbeddingEngineFactory::from_env):
//   - If HALCON_EMBEDDING_ENGINE=fastembed → use it, downloading if needed
//   - Otherwise → try if already cached, skip download silently
//   - Fallback chain: FastEmbed → Ollama → TfIdfHash

#[cfg(feature = "fastembed")]
pub mod fastembed_engine {
    use super::{cosine_sim, EmbeddingEngine, DIMS};

    /// Cache directory for fastembed models.
    /// Resolution order: FASTEMBED_CACHE_PATH env var → ~/.cache/fastembed
    fn cache_dir() -> std::path::PathBuf {
        std::env::var("FASTEMBED_CACHE_PATH")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| {
                dirs_next::cache_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join("fastembed")
            })
    }

    /// Check whether the `all-MiniLM-L6-v2` model is already in the cache
    /// WITHOUT attempting a download.
    ///
    /// fastembed stores each model in a subdirectory named after the model slug.
    /// We look for the expected directory name as a reliable cache-presence signal.
    pub fn is_model_cached() -> bool {
        let dir = cache_dir();
        if !dir.exists() {
            return false;
        }
        // fastembed names the directory after the model's slug.
        let expected = "fast-all-MiniLM-L6-v2";
        std::fs::read_dir(&dir)
            .map(|mut entries| {
                entries.any(|e| {
                    e.ok()
                        .map(|e| e.file_name().to_string_lossy().contains(expected))
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false)
    }

    /// Local ONNX-based embedding engine using `all-MiniLM-L6-v2`.
    ///
    /// 384-dim, L2-normalized output, real semantic similarity.
    pub struct FastEmbedEngine {
        model: fastembed::TextEmbedding,
    }

    impl FastEmbedEngine {
        /// Try to construct the engine.
        ///
        /// - `allow_download = true`: download the model if not cached (use for
        ///   explicit `halcon embeddings download` or `HALCON_EMBEDDING_ENGINE=fastembed`)
        /// - `allow_download = false`: only succeed if model is already cached
        pub fn try_new(allow_download: bool) -> Option<Self> {
            if !allow_download && !is_model_cached() {
                tracing::debug!(
                    target: "halcon::embedding",
                    "FastEmbedEngine: model not cached — skipping (set \
                     HALCON_EMBEDDING_ENGINE=fastembed to download)"
                );
                return None;
            }

            let opts = fastembed::InitOptions::new(fastembed::EmbeddingModel::AllMiniLML6V2)
                .with_show_download_progress(allow_download)
                .with_cache_dir(cache_dir());

            match fastembed::TextEmbedding::try_new(opts) {
                Ok(model) => {
                    tracing::info!(
                        target: "halcon::embedding",
                        dims = DIMS,
                        "FastEmbedEngine: all-MiniLM-L6-v2 loaded (ONNX)"
                    );
                    Some(Self { model })
                }
                Err(e) => {
                    tracing::warn!(
                        target: "halcon::embedding",
                        error = %e,
                        "FastEmbedEngine: init failed"
                    );
                    None
                }
            }
        }
    }

    impl EmbeddingEngine for FastEmbedEngine {
        fn embed(&self, text: &str) -> Vec<f32> {
            match self.model.embed(vec![text.to_string()], None) {
                Ok(mut embeddings) => {
                    let mut v = embeddings.pop().unwrap_or_else(|| vec![0.0; DIMS]);
                    // fastembed returns L2-normalised vectors by default for
                    // all-MiniLM models, but we re-normalise to be safe.
                    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
                    if norm > 1e-9 {
                        v.iter_mut().for_each(|x| *x /= norm);
                    }
                    v
                }
                Err(e) => {
                    tracing::warn!(
                        target: "halcon::embedding",
                        error = %e,
                        "FastEmbedEngine: embed failed, returning zero vector"
                    );
                    vec![0.0; DIMS]
                }
            }
        }
    }

    // Re-export for EmbeddingEngineFactory use
    pub use super::cosine_sim;
}

// ── EmbeddingEngineFactory ────────────────────────────────────────────────────

/// Factory for obtaining the best available embedding engine at runtime.
///
/// ## Resolution order (Sprint 1 Q3 update)
///
/// 1. **FastEmbedEngine** (`feature = "fastembed"`) — ONNX local embeddings.
///    Selected when `HALCON_EMBEDDING_ENGINE=fastembed` (download allowed) or
///    when model is already cached (download skipped).
///
/// 2. **OllamaEmbeddingEngine** — neural embeddings via local Ollama server.
///    Selected when `{endpoint}` responds within 300 ms.
///
/// 3. **TfIdfHashEngine** — FNV-1a hash projection.
///    **Last resort only.** Zero semantic properties.  Used in air-gapped
///    environments where neither fastembed nor Ollama is available.
pub struct EmbeddingEngineFactory;

impl EmbeddingEngineFactory {
    /// Return the best available engine for the given Ollama endpoint and model.
    ///
    /// FastEmbed (when compiled in) is tried first — it is self-contained and
    /// needs no server.  Then Ollama.  Then the TfIdf hash fallback.
    ///
    /// The Ollama probe is isolated in `std::thread::spawn` to prevent panics
    /// when called from a tokio async context (reqwest::blocking cannot be
    /// dropped inside a tokio runtime).
    pub fn best_available(endpoint: &str, model: &str) -> Box<dyn EmbeddingEngine> {
        // ── Step 1: try FastEmbed (no-download path, honours explicit flag) ──
        #[cfg(feature = "fastembed")]
        {
            let explicit = std::env::var("HALCON_EMBEDDING_ENGINE")
                .map(|v| v.to_lowercase() == "fastembed")
                .unwrap_or(false);
            if let Some(engine) = fastembed_engine::FastEmbedEngine::try_new(explicit) {
                return Box::new(engine);
            }
        }

        // ── Step 2: try Ollama ──────────────────────────────────────────────
        const PROBE_TIMEOUT_MS: u64 = 300;
        const INFERENCE_TIMEOUT_MS: u64 = 5_000;
        let endpoint_s = endpoint.to_string();
        let model_s = model.to_string();

        let (tx, rx) = std::sync::mpsc::channel::<Option<Box<dyn EmbeddingEngine>>>();
        std::thread::spawn(move || {
            let probe = OllamaEmbeddingEngine::new(&endpoint_s, &model_s, PROBE_TIMEOUT_MS);
            if probe.probe().is_some() {
                let engine: Box<dyn EmbeddingEngine> = Box::new(OllamaEmbeddingEngine::new(
                    &endpoint_s,
                    &model_s,
                    INFERENCE_TIMEOUT_MS,
                ));
                let _ = tx.send(Some(engine));
            } else {
                let _ = tx.send(None);
            }
        });

        match rx
            .recv_timeout(std::time::Duration::from_millis(PROBE_TIMEOUT_MS + 100))
            .ok()
            .flatten()
        {
            Some(engine) => {
                tracing::info!(
                    target: "halcon::embedding",
                    endpoint = endpoint,
                    model = model,
                    "OllamaEmbeddingEngine active — multilingual mode"
                );
                engine
            }
            None => {
                // ── Step 3: TfIdf hash fallback ──────────────────────────────
                tracing::warn!(
                    target: "halcon::embedding",
                    endpoint = endpoint,
                    "No neural embedding engine available — \
                     TfIdfHashEngine active (zero semantic similarity). \
                     Install Ollama or set HALCON_EMBEDDING_ENGINE=fastembed."
                );
                Box::new(TfIdfHashEngine)
            }
        }
    }

    /// Probe the default local Ollama instance with the default model.
    pub fn default_local() -> Box<dyn EmbeddingEngine> {
        Self::best_available(OLLAMA_DEFAULT_ENDPOINT, OLLAMA_DEFAULT_MODEL)
    }

    /// Select the best engine, honouring environment variable overrides.
    ///
    /// Resolution order:
    /// 1. `HALCON_EMBEDDING_ENGINE=fastembed` — explicit FastEmbed selection
    /// 2. `HALCON_EMBEDDING_ENDPOINT` + `HALCON_EMBEDDING_MODEL` — Ollama overrides
    /// 3. `OLLAMA_HOST` — Ollama CLI convention
    /// 4. Compiled-in defaults
    pub fn from_env() -> Box<dyn EmbeddingEngine> {
        let endpoint = std::env::var("HALCON_EMBEDDING_ENDPOINT")
            .or_else(|_| std::env::var("OLLAMA_HOST"))
            .unwrap_or_else(|_| OLLAMA_DEFAULT_ENDPOINT.to_string());
        let model = std::env::var("HALCON_EMBEDDING_MODEL")
            .unwrap_or_else(|_| OLLAMA_DEFAULT_MODEL.to_string());
        Self::best_available(&endpoint, &model)
    }

    /// Select the best engine with explicit config, honouring env var overrides.
    pub fn with_config(endpoint: &str, model: &str) -> Box<dyn EmbeddingEngine> {
        let resolved_endpoint = std::env::var("HALCON_EMBEDDING_ENDPOINT")
            .or_else(|_| std::env::var("OLLAMA_HOST"))
            .unwrap_or_else(|_| endpoint.to_string());
        let resolved_model =
            std::env::var("HALCON_EMBEDDING_MODEL").unwrap_or_else(|_| model.to_string());
        Self::best_available(&resolved_endpoint, &resolved_model)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> TfIdfHashEngine {
        TfIdfHashEngine
    }

    #[test]
    fn embed_returns_correct_dims() {
        let v = engine().embed("hello world rust tokio");
        assert_eq!(v.len(), DIMS);
    }

    #[test]
    fn embed_is_l2_normalized() {
        let v = engine().embed("test sentence for normalization");
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "norm={norm}");
    }

    #[test]
    fn embed_empty_returns_zero_vec() {
        let v = engine().embed("");
        assert_eq!(v.len(), DIMS);
        assert!(v.iter().all(|&x| x == 0.0));
    }

    #[test]
    fn identical_texts_have_similarity_one() {
        let e = engine();
        let a = e.embed("rust async patterns tokio error handling");
        let b = e.embed("rust async patterns tokio error handling");
        let sim = cosine_sim(&a, &b);
        assert!((sim - 1.0).abs() < 1e-5, "sim={sim}");
    }

    #[test]
    fn similar_texts_score_higher_than_unrelated() {
        let e = engine();
        let query = e.embed("file path errors FASE-2 gate");
        let related = e.embed("FASE-2 path existence gate failed file read");
        let unrelated = e.embed("quantum physics superposition wavefunction collapse");
        let sim_rel = cosine_sim(&query, &related);
        let sim_unrel = cosine_sim(&query, &unrelated);
        assert!(sim_rel > sim_unrel, "expected {sim_rel} > {sim_unrel}");
    }

    #[test]
    fn fnv1a_dim_in_range() {
        for word in &["rust", "async", "tokio", "error", "halcon", "boundary"] {
            let d = fnv1a_dim(word);
            assert!(d < DIMS, "dim {d} out of range for word {word}");
        }
    }

    #[test]
    fn tokenize_lowercases_and_splits() {
        let tokens = tokenize("Hello, World! Rust_Code");
        assert!(tokens.contains(&"hello".to_string()));
        assert!(tokens.contains(&"world".to_string()));
        assert!(tokens.contains(&"rust_code".to_string()));
    }

    #[test]
    fn tokenize_skips_single_chars() {
        let tokens = tokenize("a b cc dd");
        assert!(!tokens.contains(&"a".to_string()));
        assert!(tokens.contains(&"cc".to_string()));
        assert!(tokens.contains(&"dd".to_string()));
    }
}
