//! Lexical VAD (Valence-Arousal-Dominance) sentiment analyzer for TUI color adaptation.
//!
//! Analyzes user and agent messages to extract emotional signals (frustration,
//! fatigue, confusion, excitement, satisfaction) and produce a VAD score used
//! by the emotional palette system to adapt TUI colors perceptually.
//!
//! # Model
//!
//! Uses a lightweight lexical approach — no ML inference, no network calls.
//! Word lists are compiled into `LazyLock<Vec<...>>` statics for zero
//! per-call overhead after first access.
//!
//! # VAD space
//! - **Valence**: -1.0 (very negative) → +1.0 (very positive)
//! - **Arousal**: 0.0 (calm/fatigued) → 1.0 (excited/anxious)
//! - **Dominance**: 0.0 (helpless) → 1.0 (in full control)

use std::collections::{HashSet, VecDeque};
use std::sync::LazyLock;

// ── VAD score ────────────────────────────────────────────────────────────────

/// VAD sentiment score: Valence-Arousal-Dominance model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SentimentScore {
    /// -1.0 (very negative) → +1.0 (very positive)
    pub valence: f64,
    /// 0.0 (calm) → 1.0 (excited/anxious)
    pub arousal: f64,
    /// 0.0 (helpless) → 1.0 (in control)
    pub dominance: f64,
}

impl SentimentScore {
    /// Neutral baseline score.
    pub fn neutral() -> Self {
        Self { valence: 0.0, arousal: 0.3, dominance: 0.5 }
    }

    /// Clamp all fields to valid ranges.
    pub fn clamped(self) -> Self {
        Self {
            valence: self.valence.clamp(-1.0, 1.0),
            arousal: self.arousal.clamp(0.0, 1.0),
            dominance: self.dominance.clamp(0.0, 1.0),
        }
    }

    /// Weighted blend of two scores (weight=0.0 → self, weight=1.0 → other).
    pub fn blend(self, other: Self, weight: f64) -> Self {
        let w = weight.clamp(0.0, 1.0);
        let iw = 1.0 - w;
        Self {
            valence: self.valence * iw + other.valence * w,
            arousal: self.arousal * iw + other.arousal * w,
            dominance: self.dominance * iw + other.dominance * w,
        }
        .clamped()
    }
}

impl Default for SentimentScore {
    fn default() -> Self {
        Self::neutral()
    }
}

// ── Message source ────────────────────────────────────────────────────────────

/// Source of a message for sentiment analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageSource {
    User,
    Agent,
}

// ── Word lists (compiled once) ────────────────────────────────────────────────

static FRUSTRATION_WORDS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    vec![
        "again", "still", "broken", "wrong", "error", "fail", "failed",
        "terrible", "awful", "useless", "why", "stupid", "bad",
        "doesn't work", "not working", "cant", "cannot", "wont", "won't",
        "keeps", "always fails", "impossible", "never works", "ridiculous",
        "frustrating", "unacceptable", "pathetic",
    ]
});

static FATIGUE_WORDS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    vec![
        "please", "just", "already", "keep", "same", "tired", "exhausted",
        "endless", "forever", "still not", "still broken",
        "give up", "forget it", "whatever", "seriously", "honestly",
    ]
});

static CONFUSION_WORDS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    vec![
        "what", "how", "why", "understand", "confused", "unclear", "lost",
        "help", "explain", "dont get", "don't get", "not sure", "unsure",
        "no idea", "huh", "what do you mean", "not following", "unclear",
    ]
});

static SATISFACTION_WORDS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    vec![
        "great", "perfect", "thanks", "thank", "excellent", "wonderful",
        "amazing", "love", "awesome", "nice", "good job", "thank you",
        "well done", "brilliant", "incredible", "fantastic", "beautiful",
        "worked", "fixed", "solved",
    ]
});

static POSITIVE_WORDS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    vec![
        "yes", "works", "done", "ok", "good", "correct", "right", "success",
        "solved", "fixed", "cool", "nice", "yep", "exactly", "yep",
        "got it", "makes sense", "understood",
    ]
});

static APOLOGY_WORDS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    vec![
        "sorry", "unfortunately", "error", "mistake", "couldn't",
        "unable to", "cannot", "apologize", "regret", "failed to",
    ]
});

// ── Analyzer ──────────────────────────────────────────────────────────────────

/// Sentiment analyzer with rolling decay-weighted history.
///
/// Maintains a queue of the last `max_history` message scores and can
/// compute a decay-weighted conversation-level score at any time.
pub struct SentimentAnalyzer {
    /// Last N message scores (oldest first).
    pub(crate) history: VecDeque<(SentimentScore, MessageSource)>,
    /// Decay factor for older messages (0.7 = recent messages weighted more).
    decay_factor: f64,
    /// Maximum history size.
    pub(crate) max_history: usize,
    /// Last few user message texts for repetition detection.
    last_user_texts: VecDeque<String>,
}

impl SentimentAnalyzer {
    /// Create a new analyzer with default settings.
    pub fn new() -> Self {
        Self {
            history: VecDeque::new(),
            decay_factor: 0.7,
            max_history: 8,
            last_user_texts: VecDeque::new(),
        }
    }

    /// Analyze a single message and update history.
    ///
    /// Returns the score for this message (not the conversation average).
    pub fn analyze_text(&mut self, text: &str, source: MessageSource) -> SentimentScore {
        let text_lower = text.to_lowercase();
        let mut score = SentimentScore::neutral();

        // ── Lexical scoring ───────────────────────────────────────────────────

        let frustration_count = count_matches(&text_lower, &FRUSTRATION_WORDS);
        let fatigue_count = count_matches(&text_lower, &FATIGUE_WORDS);
        let confusion_count = count_matches(&text_lower, &CONFUSION_WORDS);
        let satisfaction_count = count_matches(&text_lower, &SATISFACTION_WORDS);
        let positive_count = count_matches(&text_lower, &POSITIVE_WORDS);

        if frustration_count > 0 {
            score.valence -= 0.20 * frustration_count.min(3) as f64;
            score.arousal += 0.15 * frustration_count.min(2) as f64;
            score.dominance -= 0.10;
        }

        if fatigue_count > 0 {
            score.valence -= 0.10 * fatigue_count.min(2) as f64;
            score.arousal -= 0.10 * fatigue_count.min(2) as f64;
            score.dominance -= 0.05;
        }

        if confusion_count > 0 {
            score.arousal += 0.05 * confusion_count.min(3) as f64;
            score.dominance -= 0.10 * confusion_count.min(2) as f64;
        }

        if satisfaction_count > 0 {
            score.valence += 0.25 * satisfaction_count.min(3) as f64;
            score.arousal += 0.10;
            score.dominance += 0.10;
        }

        if positive_count > 0 {
            score.valence += 0.10 * positive_count.min(3) as f64;
        }

        // ── Punctuation signals ───────────────────────────────────────────────

        let excl_count = text.chars().filter(|&c| c == '!').count();
        if excl_count > 0 {
            score.arousal += (0.15 * excl_count.min(4) as f64).min(0.40);
        }
        if text.contains("!!!") {
            score.valence -= 0.25;
            score.arousal += 0.15;
        }
        if text.trim_end().ends_with('?') {
            score.dominance -= 0.10;
            score.arousal += 0.05;
        }
        if text.contains("...") {
            score.arousal -= 0.10;
        }

        // ── Sentence length signal ────────────────────────────────────────────

        let word_count = text.split_whitespace().count();
        let sentence_count = text.split(['.', '!', '?'])
            .filter(|s| !s.trim().is_empty())
            .count();
        let avg_words = if sentence_count == 0 {
            word_count as f64
        } else {
            word_count as f64 / sentence_count as f64
        };
        if avg_words < 5.0 && word_count > 0 {
            score.arousal += 0.10; // Short/terse = high arousal
        } else if avg_words > 30.0 {
            score.arousal -= 0.05; // Long/detailed = lower arousal
        }

        // ── Repetition detection (user only) ─────────────────────────────────

        if source == MessageSource::User {
            if let Some(prev) = self.last_user_texts.back() {
                let overlap = jaccard_word_overlap(text, prev);
                if overlap >= 0.60 {
                    score.valence -= 0.20;
                    score.arousal += 0.10;
                }
            }
            self.last_user_texts.push_back(text.to_string());
            if self.last_user_texts.len() > 3 {
                self.last_user_texts.pop_front();
            }
        }

        // ── Agent apology signal ──────────────────────────────────────────────

        if source == MessageSource::Agent {
            let apology_count = count_matches(&text_lower, &APOLOGY_WORDS);
            if apology_count > 0 {
                score.valence -= 0.10;
            }
        }

        let final_score = score.clamped();

        self.history.push_back((final_score, source));
        if self.history.len() > self.max_history {
            self.history.pop_front();
        }

        final_score
    }

    /// Compute decay-weighted average over message history.
    ///
    /// Most recent messages have higher weight (decay^0 = 1.0 for newest,
    /// decay^(n-1) ≈ 0.7^7 ≈ 0.08 for the 8th oldest).
    pub fn conversation_score(&self) -> SentimentScore {
        if self.history.is_empty() {
            return SentimentScore::neutral();
        }

        let n = self.history.len();
        let mut total_weight = 0.0_f64;
        let mut v = 0.0_f64;
        let mut a = 0.0_f64;
        let mut d = 0.0_f64;

        for (i, (score, _)) in self.history.iter().enumerate() {
            // i=0 is oldest → recency=0.0; i=n-1 is newest → recency=1.0
            let recency = i as f64 / (n as f64 - 1.0).max(1.0);
            // weight: newest = decay^0 = 1.0; oldest = decay^1 = 0.7
            let weight = self.decay_factor.powf(1.0 - recency);
            v += score.valence * weight;
            a += score.arousal * weight;
            d += score.dominance * weight;
            total_weight += weight;
        }

        if total_weight > 0.0 {
            SentimentScore {
                valence: (v / total_weight).clamp(-1.0, 1.0),
                arousal: (a / total_weight).clamp(0.0, 1.0),
                dominance: (d / total_weight).clamp(0.0, 1.0),
            }
        } else {
            SentimentScore::neutral()
        }
    }

    /// Reset history (on session restart).
    pub fn reset(&mut self) {
        self.history.clear();
        self.last_user_texts.clear();
    }
}

impl Default for SentimentAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Count how many phrases/words from the list appear in the already-lowercased text.
fn count_matches(text_lower: &str, words: &[&'static str]) -> usize {
    words.iter().filter(|&&w| text_lower.contains(w)).count()
}

/// Jaccard similarity between word sets of two texts (0.0 to 1.0).
pub(crate) fn jaccard_word_overlap(a: &str, b: &str) -> f64 {
    let set_a: HashSet<&str> = a.split_whitespace().collect();
    let set_b: HashSet<&str> = b.split_whitespace().collect();
    if set_a.is_empty() && set_b.is_empty() {
        return 0.0;
    }
    let intersection = set_a.intersection(&set_b).count();
    let union = set_a.len() + set_b.len() - intersection;
    if union == 0 { 0.0 } else { intersection as f64 / union as f64 }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn analyzer() -> SentimentAnalyzer {
        SentimentAnalyzer::new()
    }

    // ── Neutral baseline ──────────────────────────────────────────────────────

    #[test]
    fn neutral_baseline_values() {
        let s = SentimentScore::neutral();
        assert_eq!(s.valence, 0.0);
        assert_eq!(s.arousal, 0.3);
        assert_eq!(s.dominance, 0.5);
    }

    #[test]
    fn empty_history_returns_neutral() {
        let a = SentimentAnalyzer::new();
        let s = a.conversation_score();
        assert_eq!(s.valence, 0.0);
        assert_eq!(s.arousal, 0.3);
    }

    // ── Frustration signals ───────────────────────────────────────────────────

    #[test]
    fn frustration_words_lower_valence() {
        let mut a = analyzer();
        let s = a.analyze_text("this is broken again and still fails", MessageSource::User);
        assert!(s.valence < -0.10, "frustration should lower valence: {}", s.valence);
        assert!(s.arousal > 0.3, "frustration should raise arousal: {}", s.arousal);
    }

    #[test]
    fn multiple_exclamations_raise_arousal() {
        let mut a = analyzer();
        let s = a.analyze_text("what!! this is still broken!!", MessageSource::User);
        assert!(s.arousal > 0.40, "exclamations should raise arousal: {}", s.arousal);
    }

    #[test]
    fn triple_exclamation_lowers_valence() {
        let mut a = analyzer();
        let s = a.analyze_text("This is terrible!!!", MessageSource::User);
        assert!(s.valence < -0.20, "!!! should lower valence: {}", s.valence);
    }

    // ── Fatigue signals ───────────────────────────────────────────────────────

    #[test]
    fn fatigue_words_lower_arousal() {
        let mut a = analyzer();
        let s = a.analyze_text("please just already fix this same thing", MessageSource::User);
        assert!(s.arousal < 0.3, "fatigue should lower arousal: {}", s.arousal);
    }

    #[test]
    fn ellipsis_lowers_arousal() {
        let mut a = analyzer();
        let s = a.analyze_text("ok... I'll try again...", MessageSource::User);
        assert!(s.arousal < 0.3, "ellipsis should lower arousal: {}", s.arousal);
    }

    // ── Confusion signals ─────────────────────────────────────────────────────

    #[test]
    fn confusion_words_lower_dominance() {
        let mut a = analyzer();
        let s = a.analyze_text("I dont get it, I'm confused and unclear what to do", MessageSource::User);
        assert!(s.dominance < 0.5, "confusion should lower dominance: {}", s.dominance);
    }

    #[test]
    fn question_mark_at_end_lowers_dominance() {
        let mut a = analyzer();
        let s = a.analyze_text("What does this mean?", MessageSource::User);
        assert!(s.dominance < 0.5, "question mark should lower dominance: {}", s.dominance);
    }

    // ── Satisfaction signals ──────────────────────────────────────────────────

    #[test]
    fn satisfaction_words_raise_valence() {
        let mut a = analyzer();
        let s = a.analyze_text("Perfect, thanks! That's amazing and wonderful!", MessageSource::User);
        assert!(s.valence > 0.15, "satisfaction should raise valence: {}", s.valence);
    }

    #[test]
    fn thank_you_raises_valence() {
        let mut a = analyzer();
        let s = a.analyze_text("Thank you, this works great!", MessageSource::User);
        assert!(s.valence > 0.10, "thanks should raise valence: {}", s.valence);
    }

    // ── Positive signals ──────────────────────────────────────────────────────

    #[test]
    fn positive_words_raise_valence() {
        let mut a = analyzer();
        let s = a.analyze_text("Yes it works and it's correct!", MessageSource::User);
        assert!(s.valence > 0.0, "positive words should raise valence: {}", s.valence);
    }

    // ── Sentence length ───────────────────────────────────────────────────────

    #[test]
    fn very_short_text_raises_arousal() {
        let mut a = analyzer();
        let s = a.analyze_text("No.", MessageSource::User);
        // word_count=1, avg_words<5 → arousal += 0.10 → 0.3 + 0.10 = 0.40
        assert!(s.arousal > 0.35, "very short text should raise arousal: {}", s.arousal);
    }

    // ── Repetition detection ──────────────────────────────────────────────────

    #[test]
    fn identical_repeat_lowers_valence() {
        let mut a = analyzer();
        a.analyze_text("the file is not working correctly please fix it now", MessageSource::User);
        let s = a.analyze_text("the file is not working correctly please fix it now", MessageSource::User);
        assert!(s.valence < -0.10, "repetition should lower valence: {}", s.valence);
    }

    #[test]
    fn high_overlap_triggers_repetition_penalty() {
        let mut a = analyzer();
        a.analyze_text("edit the config file in the src directory now", MessageSource::User);
        let s = a.analyze_text("please edit the config file in the src directory", MessageSource::User);
        assert!(s.valence < 0.0, "high word overlap should lower valence: {}", s.valence);
    }

    // ── Agent source ──────────────────────────────────────────────────────────

    #[test]
    fn agent_apology_lowers_valence() {
        let mut a = analyzer();
        let s = a.analyze_text(
            "Sorry, I couldn't complete that. Unfortunately, an error occurred.",
            MessageSource::Agent,
        );
        assert!(s.valence < -0.05, "agent apology should lower valence: {}", s.valence);
    }

    // ── History and decay ─────────────────────────────────────────────────────

    #[test]
    fn history_bounded_to_max_size() {
        let mut a = analyzer();
        for _ in 0..20 {
            a.analyze_text("test message here", MessageSource::User);
        }
        assert!(a.history.len() <= a.max_history, "history should be bounded");
    }

    #[test]
    fn reset_clears_history_and_repetition_buffer() {
        let mut a = analyzer();
        a.analyze_text("test message", MessageSource::User);
        assert!(!a.history.is_empty());
        a.reset();
        assert!(a.history.is_empty());
        assert!(a.last_user_texts.is_empty());
    }

    #[test]
    fn conversation_score_in_valid_range() {
        let mut a = analyzer();
        a.analyze_text("broken wrong failed terrible", MessageSource::User);
        a.analyze_text("broken wrong failed again", MessageSource::User);
        a.analyze_text("perfect thanks amazing wonderful", MessageSource::User);
        let s = a.conversation_score();
        assert!(s.valence >= -1.0 && s.valence <= 1.0);
        assert!(s.arousal >= 0.0 && s.arousal <= 1.0);
        assert!(s.dominance >= 0.0 && s.dominance <= 1.0);
    }

    // ── SentimentScore helpers ────────────────────────────────────────────────

    #[test]
    fn blend_midpoint_is_average() {
        let a = SentimentScore { valence: 1.0, arousal: 0.8, dominance: 0.9 };
        let b = SentimentScore { valence: -1.0, arousal: 0.2, dominance: 0.1 };
        let mid = a.blend(b, 0.5);
        assert!((mid.valence - 0.0).abs() < 0.01);
        assert!((mid.arousal - 0.5).abs() < 0.01);
        assert!((mid.dominance - 0.5).abs() < 0.01);
    }

    #[test]
    fn blend_weight_zero_returns_self() {
        let a = SentimentScore { valence: 0.5, arousal: 0.6, dominance: 0.7 };
        let b = SentimentScore::neutral();
        let result = a.blend(b, 0.0);
        assert!((result.valence - 0.5).abs() < 0.001);
    }

    #[test]
    fn blend_weight_one_returns_other() {
        let a = SentimentScore { valence: 0.5, arousal: 0.6, dominance: 0.7 };
        let b = SentimentScore::neutral();
        let result = a.blend(b, 1.0);
        assert!((result.valence - 0.0).abs() < 0.001);
    }

    // ── Jaccard helper ────────────────────────────────────────────────────────

    #[test]
    fn jaccard_identical_texts_score_one() {
        assert!((jaccard_word_overlap("hello world", "hello world") - 1.0).abs() < 0.01);
    }

    #[test]
    fn jaccard_disjoint_texts_score_zero() {
        assert_eq!(jaccard_word_overlap("hello", "world"), 0.0);
    }

    #[test]
    fn jaccard_partial_overlap() {
        // "a b c" and "b c d" → intersection={b,c}=2, union={a,b,c,d}=4 → 0.5
        let overlap = jaccard_word_overlap("a b c", "b c d");
        assert!((overlap - 0.5).abs() < 0.01);
    }

    #[test]
    fn clamped_keeps_values_in_range() {
        let s = SentimentScore { valence: 2.0, arousal: -0.5, dominance: 1.5 };
        let c = s.clamped();
        assert_eq!(c.valence, 1.0);
        assert_eq!(c.arousal, 0.0);
        assert_eq!(c.dominance, 1.0);
    }
}
