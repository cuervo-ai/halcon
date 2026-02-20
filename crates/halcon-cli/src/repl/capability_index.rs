//! BM25 capability index for plugin tool discovery.
//!
//! Indexes all tool descriptors from registered plugins and supports ranked
//! full-text search by natural language query.  Reuses the same tokenisation
//! approach as the L3 SemanticStore (split on non-alphanumeric, skip length-1 tokens).

use std::collections::HashMap;
use super::plugin_manifest::{PluginManifest, RiskTier};

// ─── BM25 Constants ───────────────────────────────────────────────────────────

const K1: f64 = 1.5;
const B: f64 = 0.75;

// ─── Tokenization ─────────────────────────────────────────────────────────────

/// Tokenize a text string using the same rules as the SemanticStore.
///
/// Splits on non-alphanumeric characters except underscore, lowercases tokens,
/// and discards single-character tokens.
pub fn tokenize_text(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| w.len() >= 2)
        .map(str::to_string)
        .collect()
}

fn tokenize_and_count(text: &str) -> (HashMap<String, u32>, u32) {
    let mut freqs: HashMap<String, u32> = HashMap::new();
    let mut total = 0u32;
    for token in tokenize_text(text) {
        *freqs.entry(token).or_insert(0) += 1;
        total += 1;
    }
    (freqs, total)
}

// ─── Indexed Entry ────────────────────────────────────────────────────────────

/// One indexed capability.
pub struct IndexedCapability {
    pub plugin_id: String,
    pub tool_name: String,
    pub description: String,
    pub risk_tier: RiskTier,
    term_freqs: HashMap<String, u32>,
    term_count: u32,
}

// ─── Search Result ────────────────────────────────────────────────────────────

/// A ranked candidate returned by [`CapabilityIndex::search`].
#[derive(Debug, Clone)]
pub struct CapabilityCandidate {
    pub plugin_id: String,
    pub tool_name: String,
    pub score: f64,
    pub risk_tier: RiskTier,
}

// ─── Capability Index ─────────────────────────────────────────────────────────

/// BM25 search index over plugin capability descriptors.
pub struct CapabilityIndex {
    entries: Vec<IndexedCapability>,
    /// IDF denominator: per-term count of documents containing the term.
    doc_freq: HashMap<String, usize>,
    avg_doc_len: f64,
}

impl CapabilityIndex {
    /// Build an index from a slice of `(plugin_id, manifest)` pairs.
    pub fn build(plugins: &[(String, &PluginManifest)]) -> Self {
        let mut entries = Vec::new();
        let mut doc_freq: HashMap<String, usize> = HashMap::new();
        let mut total_len = 0u64;

        for (plugin_id, manifest) in plugins {
            for cap in &manifest.capabilities {
                let text = format!("{} {}", cap.name, cap.description);
                let (term_freqs, term_count) = tokenize_and_count(&text);

                // Update document-frequency table
                for term in term_freqs.keys() {
                    *doc_freq.entry(term.clone()).or_insert(0) += 1;
                }

                total_len += term_count as u64;
                entries.push(IndexedCapability {
                    plugin_id: plugin_id.clone(),
                    tool_name: cap.name.clone(),
                    description: cap.description.clone(),
                    risk_tier: cap.risk_tier,
                    term_freqs,
                    term_count,
                });
            }
        }

        let avg_doc_len = if entries.is_empty() {
            1.0
        } else {
            total_len as f64 / entries.len() as f64
        };

        Self { entries, doc_freq, avg_doc_len }
    }

    /// BM25-ranked search.  Returns up to `limit` candidates ordered by score descending.
    ///
    /// Returns an empty vec when the index is empty or the query has no terms.
    pub fn search(&self, query: &str, limit: usize) -> Vec<CapabilityCandidate> {
        if self.entries.is_empty() || query.is_empty() {
            return vec![];
        }

        let query_terms = tokenize_text(query);
        if query_terms.is_empty() {
            return vec![];
        }

        let n = self.entries.len() as f64;

        let mut scored: Vec<(f64, usize)> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(idx, entry)| {
                let score: f64 = query_terms.iter().map(|term| {
                    let df = *self.doc_freq.get(term).unwrap_or(&0) as f64;
                    if df == 0.0 {
                        return 0.0;
                    }
                    let tf = *entry.term_freqs.get(term).unwrap_or(&0) as f64;
                    let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
                    let tf_norm = (tf * (K1 + 1.0))
                        / (tf + K1 * (1.0 - B + B * entry.term_count as f64 / self.avg_doc_len));
                    idf * tf_norm
                }).sum();

                if score > 0.0 { Some((score, idx)) } else { None }
            })
            .collect();

        scored.sort_unstable_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        scored
            .into_iter()
            .take(limit)
            .map(|(score, idx)| {
                let e = &self.entries[idx];
                CapabilityCandidate {
                    plugin_id: e.plugin_id.clone(),
                    tool_name: e.tool_name.clone(),
                    score,
                    risk_tier: e.risk_tier,
                }
            })
            .collect()
    }

    /// Exact tool-name match (case-insensitive).  Returns the first matching entry.
    /// Preferred over BM25 for avoiding low-IDF false negatives in small indexes.
    pub fn exact_match(&self, tool_name: &str) -> Option<CapabilityCandidate> {
        let lower = tool_name.to_lowercase();
        self.entries.iter().find(|e| e.tool_name.to_lowercase() == lower).map(|e| {
            CapabilityCandidate {
                plugin_id: e.plugin_id.clone(),
                tool_name: e.tool_name.clone(),
                score: 1.0,
                risk_tier: e.risk_tier,
            }
        })
    }

    /// Returns `true` when at least one plugin is indexed.
    pub fn has_entries(&self) -> bool {
        !self.entries.is_empty()
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::plugin_manifest::{PluginManifest, ToolCapabilityDescriptor};

    fn make_manifest(id: &str, tools: &[(&str, &str)]) -> PluginManifest {
        let caps = tools.iter().map(|(name, desc)| ToolCapabilityDescriptor {
            name: name.to_string(),
            description: desc.to_string(),
            risk_tier: RiskTier::Low,
            idempotent: true,
            permission_level: halcon_core::types::PermissionLevel::ReadOnly,
            budget_tokens_per_call: 0,
        }).collect();
        PluginManifest::new_local(id, id, "1.0.0", caps)
    }

    #[test]
    fn build_index_and_search_returns_ranked() {
        let m1 = make_manifest("plugin-a", &[
            ("search_github", "Search GitHub repositories for code"),
            ("create_issue", "Create a GitHub issue"),
        ]);
        let m2 = make_manifest("plugin-b", &[
            ("query_db", "Query a database with SQL"),
        ]);
        let plugins: Vec<(String, &PluginManifest)> = vec![
            ("plugin-a".into(), &m1),
            ("plugin-b".into(), &m2),
        ];
        let index = CapabilityIndex::build(&plugins);
        let results = index.search("search github", 5);
        assert!(!results.is_empty());
        assert_eq!(results[0].plugin_id, "plugin-a");
        assert_eq!(results[0].tool_name, "search_github");
    }

    #[test]
    fn empty_query_returns_empty() {
        let m = make_manifest("p", &[("tool", "does stuff")]);
        let plugins = vec![("p".into(), &m)];
        let index = CapabilityIndex::build(&plugins);
        let r = index.search("", 5);
        assert!(r.is_empty());
    }

    #[test]
    fn empty_index_returns_empty() {
        let index = CapabilityIndex::build(&[]);
        let r = index.search("anything", 5);
        assert!(r.is_empty());
        assert!(!index.has_entries());
    }

    #[test]
    fn single_plugin_found() {
        let m = make_manifest("only-plugin", &[("do_task", "execute a specific task automatically")]);
        let plugins = vec![("only-plugin".into(), &m)];
        let index = CapabilityIndex::build(&plugins);
        let r = index.search("execute task", 3);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].plugin_id, "only-plugin");
    }
}
