//! Actual-cost computation from token counts + per-model pricing.
//!
//! Replaces `ModelProvider::estimate_cost()` for budget accounting purposes.
//! Prices are sourced from this table at startup; callers can override via
//! `HalconConfig.pricing` (not yet wired — fallback to this table is always safe).
//!
//! ## Token classes priced
//!
//! - `input_tokens`           — fresh input, billed at input rate
//! - `output_tokens`          — assistant-generated, billed at output rate
//! - `cache_read_tokens`      — prompt-cache hits, typically 10 % of input rate
//! - `cache_creation_tokens`  — prompt-cache writes, typically 125 % of input rate
//!
//! Reasoning tokens (o1/o3/deepseek-reasoner) are billed as output tokens —
//! providers include them in `output_tokens`, so no separate line item is needed.
//!
//! ## Precision
//! Costs are computed in f64 dollars.  At the precision levels involved
//! (< $1/request typical) f64 is exact to better than 0.01 micro-cent.

use std::collections::HashMap;
use std::sync::OnceLock;

use halcon_core::types::TokenUsage;

/// Per-model pricing in USD per 1 000 tokens for each token class.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelPricing {
    pub input_per_1k_usd: f64,
    pub output_per_1k_usd: f64,
    pub cache_read_per_1k_usd: f64,
    pub cache_creation_per_1k_usd: f64,
}

impl ModelPricing {
    /// Convenience constructor for providers that do not bill cache separately.
    /// Cache read is priced at 10% of input; creation at 125% of input
    /// (Anthropic default ratio — adjust per-provider when necessary).
    pub const fn simple(input_per_1k_usd: f64, output_per_1k_usd: f64) -> Self {
        Self {
            input_per_1k_usd,
            output_per_1k_usd,
            cache_read_per_1k_usd: input_per_1k_usd * 0.10,
            cache_creation_per_1k_usd: input_per_1k_usd * 1.25,
        }
    }
}

static PRICE_TABLE: OnceLock<HashMap<&'static str, ModelPricing>> = OnceLock::new();

fn price_table() -> &'static HashMap<&'static str, ModelPricing> {
    PRICE_TABLE.get_or_init(|| {
        let mut m = HashMap::new();
        // Anthropic — cache pricing per https://docs.anthropic.com/en/docs/build-with-claude/prompt-caching
        m.insert(
            "anthropic:claude-opus-4-7",
            ModelPricing {
                input_per_1k_usd: 0.015,
                output_per_1k_usd: 0.075,
                cache_read_per_1k_usd: 0.0015,
                cache_creation_per_1k_usd: 0.01875,
            },
        );
        m.insert(
            "anthropic:claude-sonnet-4-6",
            ModelPricing {
                input_per_1k_usd: 0.003,
                output_per_1k_usd: 0.015,
                cache_read_per_1k_usd: 0.0003,
                cache_creation_per_1k_usd: 0.00375,
            },
        );
        m.insert(
            "anthropic:claude-haiku-4-5",
            ModelPricing {
                input_per_1k_usd: 0.00025,
                output_per_1k_usd: 0.00125,
                cache_read_per_1k_usd: 0.000025,
                cache_creation_per_1k_usd: 0.0003125,
            },
        );
        m.insert(
            "anthropic:claude-haiku-4-5-20251001",
            ModelPricing {
                input_per_1k_usd: 0.00025,
                output_per_1k_usd: 0.00125,
                cache_read_per_1k_usd: 0.000025,
                cache_creation_per_1k_usd: 0.0003125,
            },
        );
        // OpenAI — cache read at 50% of input, no separate creation charge
        m.insert(
            "openai:gpt-4o",
            ModelPricing {
                input_per_1k_usd: 0.0025,
                output_per_1k_usd: 0.010,
                cache_read_per_1k_usd: 0.00125,
                cache_creation_per_1k_usd: 0.0025,
            },
        );
        m.insert(
            "openai:gpt-4o-mini",
            ModelPricing {
                input_per_1k_usd: 0.00015,
                output_per_1k_usd: 0.00060,
                cache_read_per_1k_usd: 0.000075,
                cache_creation_per_1k_usd: 0.00015,
            },
        );
        m.insert(
            "openai:o3-mini",
            ModelPricing {
                input_per_1k_usd: 0.0011,
                output_per_1k_usd: 0.0044,
                cache_read_per_1k_usd: 0.00055,
                cache_creation_per_1k_usd: 0.0011,
            },
        );
        m.insert("openai:o1", ModelPricing::simple(0.015, 0.060));
        // DeepSeek — official pricing with cache hit discount
        m.insert(
            "deepseek:deepseek-chat",
            ModelPricing {
                input_per_1k_usd: 0.00014,
                output_per_1k_usd: 0.00028,
                cache_read_per_1k_usd: 0.000014,
                cache_creation_per_1k_usd: 0.00014,
            },
        );
        m.insert(
            "deepseek:deepseek-reasoner",
            ModelPricing {
                input_per_1k_usd: 0.00055,
                output_per_1k_usd: 0.00219,
                cache_read_per_1k_usd: 0.000137,
                cache_creation_per_1k_usd: 0.00055,
            },
        );
        // Gemini — no separate cache pricing at this tier
        m.insert(
            "gemini:gemini-2.5-pro",
            ModelPricing::simple(0.00125, 0.010),
        );
        m.insert(
            "gemini:gemini-2.0-flash",
            ModelPricing::simple(0.000075, 0.00030),
        );
        m.insert(
            "gemini:gemini-1.5-pro",
            ModelPricing::simple(0.00125, 0.005),
        );
        // Azure (OpenAI-compatible — same prices as OpenAI upstream)
        m.insert(
            "azure:gpt-4o",
            ModelPricing {
                input_per_1k_usd: 0.0025,
                output_per_1k_usd: 0.010,
                cache_read_per_1k_usd: 0.00125,
                cache_creation_per_1k_usd: 0.0025,
            },
        );
        // Cenzontle (internal — zero external billing)
        m.insert(
            "cenzontle:cenzontle",
            ModelPricing {
                input_per_1k_usd: 0.0,
                output_per_1k_usd: 0.0,
                cache_read_per_1k_usd: 0.0,
                cache_creation_per_1k_usd: 0.0,
            },
        );
        // Sentinel — used when provider/model is unknown
        m.insert("default", ModelPricing::simple(0.001, 0.003));
        m
    })
}

/// Resolve pricing for a `(provider, model)` pair.
///
/// Exact match first, then prefix match ("anthropic:claude-sonnet-4-6" matches
/// "anthropic:claude-sonnet-4-6-20251022"), finally the `default` tier.
fn resolve_pricing(provider: &str, model: &str) -> &'static ModelPricing {
    let key = format!("{provider}:{model}");
    let table = price_table();

    if let Some(p) = table.get(key.as_str()) {
        return p;
    }
    if let Some((_, p)) = table
        .iter()
        .filter(|(k, _)| *k != &"default")
        .find(|(k, _)| key.starts_with(*k))
    {
        return p;
    }

    tracing::debug!(
        provider = provider,
        model = model,
        "cost_table: unknown model, falling back to default pricing"
    );
    table.get("default").expect("default tier always present")
}

/// Compute actual cost in USD from a `TokenUsage` struct, accounting for
/// every token class the provider reported (input / output / cache-read /
/// cache-creation).  Zero-usage → zero cost.
pub fn compute_actual_cost(usage: &TokenUsage, provider: &str, model: &str) -> f64 {
    let p = resolve_pricing(provider, model);
    let input = usage.input_tokens as f64;
    let output = usage.output_tokens as f64;
    let cache_read = usage.cache_read_tokens.unwrap_or(0) as f64;
    let cache_write = usage.cache_creation_tokens.unwrap_or(0) as f64;

    (input * p.input_per_1k_usd / 1_000.0)
        + (output * p.output_per_1k_usd / 1_000.0)
        + (cache_read * p.cache_read_per_1k_usd / 1_000.0)
        + (cache_write * p.cache_creation_per_1k_usd / 1_000.0)
}

/// Like `compute_actual_cost` but falls back to `estimated_usd` when every
/// token count is zero (provider did not report usage at all).
pub fn compute_actual_cost_with_fallback(
    usage: &TokenUsage,
    provider: &str,
    model: &str,
    estimated_usd: f64,
) -> f64 {
    let any_tokens = usage.input_tokens > 0
        || usage.output_tokens > 0
        || usage.cache_read_tokens.unwrap_or(0) > 0
        || usage.cache_creation_tokens.unwrap_or(0) > 0;
    if !any_tokens {
        return estimated_usd;
    }
    compute_actual_cost(usage, provider, model)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(input: u32, output: u32, cache_read: u32, cache_write: u32) -> TokenUsage {
        TokenUsage {
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: if cache_read == 0 {
                None
            } else {
                Some(cache_read)
            },
            cache_creation_tokens: if cache_write == 0 {
                None
            } else {
                Some(cache_write)
            },
            reasoning_tokens: None,
        }
    }

    #[test]
    fn known_model_simple_usage() {
        // claude-sonnet-4-6: $3 input / $15 output per Mtok
        let cost = compute_actual_cost(&usage(1_000, 500, 0, 0), "anthropic", "claude-sonnet-4-6");
        // 1.0 * 0.003 + 0.5 * 0.015 = 0.003 + 0.0075 = 0.0105
        assert!((cost - 0.0105).abs() < 1e-9, "got {cost}");
    }

    #[test]
    fn zero_tokens_zero_cost() {
        let cost = compute_actual_cost(&usage(0, 0, 0, 0), "anthropic", "claude-opus-4-7");
        assert_eq!(cost, 0.0);
    }

    #[test]
    fn unknown_model_falls_back_to_default() {
        let cost = compute_actual_cost(
            &usage(1_000, 1_000, 0, 0),
            "unknown_provider",
            "unknown_model",
        );
        // default: $0.001 input + $0.003 output = $0.004
        assert!((cost - 0.004).abs() < 1e-9, "got {cost}");
    }

    #[test]
    fn fallback_used_when_all_tokens_zero() {
        let cost = compute_actual_cost_with_fallback(
            &usage(0, 0, 0, 0),
            "anthropic",
            "claude-opus-4-7",
            0.42,
        );
        assert_eq!(cost, 0.42);
    }

    #[test]
    fn fallback_ignored_when_tokens_present() {
        let cost = compute_actual_cost_with_fallback(
            &usage(1_000, 0, 0, 0),
            "anthropic",
            "claude-sonnet-4-6",
            9999.0,
        );
        // 1.0 * 0.003 = 0.003 — estimated is ignored
        assert!((cost - 0.003).abs() < 1e-9, "got {cost}");
    }

    #[test]
    fn fallback_ignored_when_only_cache_reads() {
        // A cached response with no input/output still costs cache-read money.
        let cost = compute_actual_cost_with_fallback(
            &usage(0, 0, 10_000, 0),
            "anthropic",
            "claude-sonnet-4-6",
            9999.0,
        );
        // 10.0 * 0.0003 = 0.003 — estimated fallback must be ignored
        assert!((cost - 0.003).abs() < 1e-9, "got {cost}");
    }

    #[test]
    fn prefix_match_versioned_model() {
        let cost = compute_actual_cost(
            &usage(1_000, 0, 0, 0),
            "anthropic",
            "claude-sonnet-4-6-20251022",
        );
        assert!(cost > 0.0, "prefix match failed, got {cost}");
    }

    #[test]
    fn deepseek_with_cache_hit_is_cheaper() {
        // 1k input + 10k cache reads vs 11k fresh input
        let cached = compute_actual_cost(&usage(1_000, 0, 10_000, 0), "deepseek", "deepseek-chat");
        let fresh = compute_actual_cost(&usage(11_000, 0, 0, 0), "deepseek", "deepseek-chat");
        assert!(
            cached < fresh,
            "cache-read must be cheaper than fresh input (cached={cached:.6}, fresh={fresh:.6})"
        );
    }

    #[test]
    fn cenzontle_zero_cost() {
        let cost = compute_actual_cost(
            &usage(100_000, 100_000, 100_000, 100_000),
            "cenzontle",
            "cenzontle",
        );
        assert_eq!(cost, 0.0);
    }

    #[test]
    fn cache_tokens_billed_correctly_for_sonnet() {
        // The XIYO-audit bug: sonnet-4-6 with 100k cache reads → not ~$0.015
        let cost = compute_actual_cost(&usage(0, 0, 100_000, 0), "anthropic", "claude-sonnet-4-6");
        // 100.0 * 0.0003 = 0.030
        assert!((cost - 0.030).abs() < 1e-9, "got {cost} (expected 0.03)");
    }

    #[test]
    fn cache_creation_more_expensive_than_input() {
        // Anthropic cache-creation is 125% of input
        let create = compute_actual_cost(&usage(0, 0, 0, 1_000), "anthropic", "claude-sonnet-4-6");
        let fresh = compute_actual_cost(&usage(1_000, 0, 0, 0), "anthropic", "claude-sonnet-4-6");
        assert!(
            create > fresh,
            "cache-creation must exceed input: create={create}, fresh={fresh}"
        );
    }

    #[test]
    fn simple_constructor_applies_default_ratios() {
        let p = ModelPricing::simple(0.010, 0.020);
        assert_eq!(p.input_per_1k_usd, 0.010);
        assert_eq!(p.output_per_1k_usd, 0.020);
        assert!((p.cache_read_per_1k_usd - 0.001).abs() < 1e-12);
        assert!((p.cache_creation_per_1k_usd - 0.0125).abs() < 1e-12);
    }

    #[test]
    fn monotonic_in_tokens() {
        // More tokens → more cost
        let small = compute_actual_cost(&usage(100, 50, 0, 0), "anthropic", "claude-sonnet-4-6");
        let big = compute_actual_cost(
            &usage(100_000, 50_000, 0, 0),
            "anthropic",
            "claude-sonnet-4-6",
        );
        assert!(big > small * 100.0, "cost must scale linearly");
    }
}
