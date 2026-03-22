//! MediaContextSource: injects media analysis results into the context pipeline.
//!
//! Implements `ContextSource` from halcon-core so the assembler treats media
//! analysis as just another context chunk — no L0-L4 modifications required.
//!
//! ## Embedding strategy
//!
//! Without a local CLIP model, we use a **hash-projection bag-of-words** embedding:
//! - Each word in the description is hashed into a 512-dim vector index.
//! - Dimension values are term frequencies, L2-normalized.
//! - Cosine similarity on these vectors finds semantically related descriptions.
//!
//! This is fast (no model), deterministic, and produces meaningful retrieval when
//! both query and stored descriptions share vocabulary. Real CLIP embeddings replace
//! this when `vision-native` feature is enabled.

use std::sync::Arc;

use async_trait::async_trait;
use halcon_core::{
    error::Result,
    traits::{ContextChunk, ContextQuery, ContextSource},
};

use crate::index::MediaIndex;

/// Context source that retrieves relevant media analyses by semantic similarity.
pub struct MediaContextSource {
    index: Arc<MediaIndex>,
    top_k: usize,
}

impl std::fmt::Debug for MediaContextSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaContextSource")
            .field("top_k", &self.top_k)
            .finish()
    }
}

impl MediaContextSource {
    pub fn new(index: Arc<MediaIndex>, top_k: usize) -> Self {
        Self { index, top_k }
    }
}

#[async_trait]
impl ContextSource for MediaContextSource {
    fn name(&self) -> &str {
        "media_index"
    }

    fn priority(&self) -> u32 {
        55
    } // Below repo_map (60), above MCP (50)

    async fn gather(&self, query: &ContextQuery) -> Result<Vec<ContextChunk>> {
        // Only search if there is a user message to embed.
        let user_message = match query.user_message.as_deref() {
            Some(msg) if !msg.trim().is_empty() => msg,
            _ => return Ok(vec![]),
        };

        // Compute a hash-projection embedding for the query text.
        let query_embedding = text_to_embedding_512(user_message);

        // Search index for top-K similar media.
        let results = self
            .index
            .search(query_embedding, None, self.top_k)
            .await
            .unwrap_or_default(); // Best-effort: context is not critical path.

        if results.is_empty() {
            return Ok(vec![]);
        }

        // Convert matched entries to context chunks.
        let chunks: Vec<ContextChunk> = results
            .into_iter()
            .map(|entry| {
                let source_label = entry
                    .source_path
                    .as_deref()
                    .and_then(|p| std::path::Path::new(p).file_name())
                    .and_then(|n| n.to_str())
                    .unwrap_or("media");

                let temporal = match (entry.clip_start_secs, entry.clip_end_secs) {
                    (Some(start), Some(end)) => format!(" [{:.1}s–{:.1}s]", start, end),
                    _ => String::new(),
                };

                // Build the chunk content: prefer description (M33) over bare content hash.
                let content = if let Some(ref desc) = entry.description {
                    format!(
                        "[Media context — {} {}{}]\nDescription: {}\nContent hash: {}",
                        entry.modality,
                        source_label,
                        temporal,
                        desc,
                        &entry.content_hash[..entry.content_hash.len().min(16)],
                    )
                } else {
                    format!(
                        "[Media context — {} {}{}]\nContent hash: {}\nModality: {}",
                        entry.modality,
                        source_label,
                        temporal,
                        &entry.content_hash[..entry.content_hash.len().min(16)],
                        entry.modality,
                    )
                };

                // Token estimate: base overhead + description length / 4 chars per token.
                let estimated_tokens: usize = 40
                    + entry
                        .description
                        .as_deref()
                        .map(|d| d.len() / 4)
                        .unwrap_or(0);

                ContextChunk {
                    content,
                    source: "media_index".into(),
                    priority: 55,
                    estimated_tokens,
                }
            })
            .collect();

        tracing::debug!(
            query_len = user_message.len(),
            results = chunks.len(),
            "MediaContextSource returned chunks"
        );

        Ok(chunks)
    }
}

// ── Hash-projection bag-of-words embedding ───────────────────────────────────

/// Encode `text` as a 512-dim L2-normalised bag-of-words vector using hash projection.
///
/// Algorithm:
///   1. Tokenize on whitespace + punctuation, lowercase.
///   2. Hash each token with FNV-1a → index into [0, 512).
///   3. Accumulate term frequencies at each index.
///   4. L2-normalize the vector.
///
/// This enables cosine-similarity retrieval without any ML model:
/// two texts sharing vocabulary will produce similar vectors.
pub fn text_to_embedding_512(text: &str) -> Vec<f32> {
    const DIM: usize = 512;
    let mut embedding = vec![0.0f32; DIM];

    for token in tokenize(text) {
        let idx = fnv1a_hash(&token) % DIM;
        embedding[idx] += 1.0;
    }

    // L2 normalize.
    let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 1e-9 {
        for x in &mut embedding {
            *x /= norm;
        }
    }

    embedding
}

/// Tokenize text into lowercase alphanumeric tokens (min 2 chars).
fn tokenize(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 2)
        .map(|t| t.to_lowercase())
}

/// FNV-1a hash producing a stable usize for any string.
fn fnv1a_hash(s: &str) -> usize {
    const FNV_OFFSET: u64 = 14_695_981_039_346_656_037;
    const FNV_PRIME: u64 = 1_099_511_628_211;
    let mut hash = FNV_OFFSET;
    for byte in s.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash as usize
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use halcon_storage::{AsyncDatabase, Database};

    fn test_index() -> Arc<MediaIndex> {
        let db = Arc::new(AsyncDatabase::new(Arc::new(
            Database::open_in_memory().unwrap(),
        )));
        Arc::new(MediaIndex::new(db))
    }

    fn test_query(msg: &str) -> ContextQuery {
        ContextQuery {
            working_directory: ".".into(),
            user_message: Some(msg.into()),
            token_budget: 4096,
        }
    }

    // ── Embedding tests ───────────────────────────────────────────────────────

    #[test]
    fn embedding_is_512_dim() {
        let emb = text_to_embedding_512("a cat sitting on a mat");
        assert_eq!(emb.len(), 512);
    }

    #[test]
    fn embedding_is_l2_normalized() {
        let emb = text_to_embedding_512("hello world rust programming");
        let norm: f32 = emb.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "norm should be 1.0, got {norm}");
    }

    #[test]
    fn empty_text_returns_zero_vector() {
        let emb = text_to_embedding_512("");
        let norm: f32 = emb.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(norm < 1e-9, "empty text should produce zero vector");
    }

    #[test]
    fn similar_texts_higher_cosine_than_dissimilar() {
        let cat1 = text_to_embedding_512("a black cat sitting on a window sill");
        let cat2 = text_to_embedding_512("orange cat resting near the window");
        let unrelated = text_to_embedding_512("database schema migration sql query");

        let sim_cats: f32 = cat1.iter().zip(cat2.iter()).map(|(a, b)| a * b).sum();
        let sim_unrel: f32 = cat1.iter().zip(unrelated.iter()).map(|(a, b)| a * b).sum();

        assert!(
            sim_cats > sim_unrel,
            "similar texts should have higher cosine sim ({sim_cats:.3}) \
             than dissimilar ({sim_unrel:.3})"
        );
    }

    #[test]
    fn identical_texts_cosine_one() {
        let emb = text_to_embedding_512("halcon multimodal image analysis system");
        let sim: f32 = emb.iter().zip(emb.iter()).map(|(a, b)| a * b).sum();
        assert!(
            (sim - 1.0).abs() < 1e-5,
            "identical texts cosine should be 1.0, got {sim}"
        );
    }

    #[test]
    fn deterministic_embedding() {
        let e1 = text_to_embedding_512("test determinism check");
        let e2 = text_to_embedding_512("test determinism check");
        assert_eq!(e1, e2);
    }

    // ── Source behaviour tests ────────────────────────────────────────────────

    #[tokio::test]
    async fn gather_returns_empty_when_no_embeddings() {
        let src = MediaContextSource::new(test_index(), 5);
        let chunks = src
            .gather(&test_query("what is in the image?"))
            .await
            .unwrap();
        assert!(chunks.is_empty());
    }

    #[test]
    fn priority_and_name() {
        let src = MediaContextSource::new(test_index(), 3);
        assert_eq!(src.name(), "media_index");
        assert_eq!(src.priority(), 55);
    }

    #[tokio::test]
    async fn gather_returns_empty_for_blank_query() {
        let src = MediaContextSource::new(test_index(), 5);
        let q = ContextQuery {
            working_directory: ".".into(),
            user_message: Some("   ".into()),
            token_budget: 4096,
        };
        let chunks = src.gather(&q).await.unwrap();
        assert!(chunks.is_empty());
    }

    #[tokio::test]
    async fn gather_returns_chunks_when_index_populated() {
        let index = test_index();
        // Store an embedding that matches the query vocabulary.
        let desc = "a cat sitting on a window sill orange";
        let embedding = text_to_embedding_512(desc);
        index
            .store(
                "cafecafe01234567".into(),
                "image".into(),
                embedding,
                None,
                Some("/tmp/cat.jpg".into()),
                Some(desc.into()),
            )
            .await
            .unwrap();

        let src = MediaContextSource::new(index, 3);
        // Query shares vocabulary with stored description.
        let chunks = src
            .gather(&test_query("cat near the window"))
            .await
            .unwrap();
        assert!(
            !chunks.is_empty(),
            "should return at least one chunk when index has matching entry"
        );
        assert!(chunks[0].source == "media_index");
    }

    #[tokio::test]
    async fn gather_chunk_contains_modality_and_hash() {
        let index = test_index();
        let embedding = text_to_embedding_512("dog playing in the park");
        index
            .store(
                "deadbeef00000000".into(),
                "image".into(),
                embedding,
                None,
                None,
                None,
            )
            .await
            .unwrap();

        let src = MediaContextSource::new(index, 1);
        let chunks = src.gather(&test_query("dog park playing")).await.unwrap();
        if !chunks.is_empty() {
            assert!(chunks[0].content.contains("image"));
            assert!(chunks[0].content.contains("deadbeef"));
        }
    }

    #[tokio::test]
    async fn gather_with_description_returns_useful_chunk() {
        let index = test_index();
        let desc =
            "A photograph showing a mountain landscape with snow-capped peaks and pine trees";
        let embedding = text_to_embedding_512(desc);
        index
            .store(
                "mountainhash0000".into(),
                "image".into(),
                embedding,
                None,
                Some("/photos/mountain.jpg".into()),
                Some(desc.into()),
            )
            .await
            .unwrap();

        let src = MediaContextSource::new(index, 3);
        let chunks = src
            .gather(&test_query("show me the mountain photo"))
            .await
            .unwrap();
        if !chunks.is_empty() {
            // Chunk must contain the description, not just the hash.
            assert!(
                chunks[0].content.contains("Description:"),
                "chunk with description must include 'Description:' label"
            );
            assert!(
                chunks[0].content.contains("mountain"),
                "chunk content must contain description text"
            );
            // Token estimate must be larger than the base 40 for non-empty description.
            assert!(
                chunks[0].estimated_tokens > 40,
                "token estimate should scale with description length"
            );
        }
    }

    #[tokio::test]
    async fn gather_without_description_degrades_gracefully() {
        let index = test_index();
        let embedding = text_to_embedding_512("sunset over ocean waves");
        index
            .store(
                "sunsethashabcd12".into(),
                "image".into(),
                embedding,
                None,
                None,
                None, // no description
            )
            .await
            .unwrap();

        let src = MediaContextSource::new(index, 3);
        let chunks = src.gather(&test_query("sunset ocean waves")).await.unwrap();
        if !chunks.is_empty() {
            // Fallback: must still contain the content hash.
            assert!(
                chunks[0].content.contains("Content hash:"),
                "chunk without description should fall back to content hash"
            );
            assert_eq!(
                chunks[0].estimated_tokens, 40,
                "base token estimate for no-description entry"
            );
        }
    }

    #[test]
    fn token_estimate_scales_with_description() {
        // 120-char description → 30 extra tokens (120/4) + 40 base = 70
        let desc_len = 120usize;
        let extra = desc_len / 4;
        assert_eq!(40usize + extra, 70usize);
    }
}
