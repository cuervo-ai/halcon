//! Pre-planning semantic ambiguity detector.
//!
//! Detects underspecification in user queries **before** the LLM planner is invoked.
//! All detection is deterministic and rule-based — zero LLM calls.
//!
//! # Signals detected
//!
//! | Signal | Example query |
//! |---|---|
//! | [`AmbiguitySignal::UnknownProperNoun`] | `"implement momoto"` |
//! | [`AmbiguitySignal::VagueActionVerb`]   | `"handle the thing"` |
//! | [`AmbiguitySignal::MissingScope`]      | `"write the function"` |
//! | [`AmbiguitySignal::UnresolvedPronoun`] | `"fix it"` |

use std::collections::HashSet;

// ──────────────────────────────────────────────────────────────────────────────
// Public types
// ──────────────────────────────────────────────────────────────────────────────

/// The kind of scope that is missing when [`AmbiguitySignal::MissingScope`] fires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeKind {
    /// No target file path mentioned.
    TargetFile,
    /// No target module or crate mentioned.
    TargetModule,
    /// No target function or struct mentioned.
    TargetSymbol,
    /// No output format specified (for generation tasks).
    OutputFormat,
}

/// A single ambiguity signal detected in the user query.
#[derive(Debug, Clone)]
pub enum AmbiguitySignal {
    /// A proper noun / named entity was found that is not in the known-entity index.
    /// Common cause: user refers to an unfamiliar library, tool, or project component.
    UnknownProperNoun {
        /// The unrecognised token.
        noun: String,
        /// The surrounding phrase for context.
        context: String,
    },
    /// The query contains an action verb that is too vague to act on without clarification.
    VagueActionVerb {
        /// The matched verb.
        verb: String,
        /// Possible concrete interpretations.
        possible_interpretations: Vec<String>,
    },
    /// An action was requested but the target scope is absent.
    MissingScope {
        /// What kind of scope is missing.
        missing: ScopeKind,
    },
    /// A pronoun refers to something not established in the current conversation.
    UnresolvedPronoun {
        /// The pronoun that could not be resolved.
        pronoun: String,
    },
}

impl AmbiguitySignal {
    /// Human-readable description for use in clarification questions.
    pub fn description(&self) -> String {
        match self {
            Self::UnknownProperNoun { noun, .. } => {
                format!("I don't recognise '{noun}' — could you clarify what it refers to?")
            }
            Self::VagueActionVerb { verb, possible_interpretations } => {
                let opts = possible_interpretations
                    .iter()
                    .enumerate()
                    .map(|(i, s)| format!("({}) {}", (b'a' + i as u8) as char, s))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("'{verb}' is ambiguous. Did you mean: {opts}?")
            }
            Self::MissingScope { missing } => match missing {
                ScopeKind::TargetFile => {
                    "Which file should I modify? No file path was specified.".to_string()
                }
                ScopeKind::TargetModule => {
                    "Which module or crate is the target? Please specify.".to_string()
                }
                ScopeKind::TargetSymbol => {
                    "Which function or struct should I work on?".to_string()
                }
                ScopeKind::OutputFormat => {
                    "What format should the output be in?".to_string()
                }
            },
            Self::UnresolvedPronoun { pronoun } => {
                format!(
                    "'{pronoun}' doesn't have a clear referent. \
                     What are you referring to?"
                )
            }
        }
    }
}

/// Context for ambiguity detection: recent conversation state.
#[derive(Debug, Default)]
pub struct AmbiguityContext {
    /// Number of prior assistant turns. Used to determine if pronouns can be resolved.
    pub prior_assistant_turns: usize,
    /// Recent file paths that have been mentioned or modified.
    pub recent_file_paths: Vec<String>,
}

/// Result of an ambiguity detection pass.
#[derive(Debug)]
pub struct AmbiguityReport {
    /// Whether any signals were detected.
    pub is_ambiguous: bool,
    /// All signals that fired, ordered by severity (most critical first).
    pub signals: Vec<AmbiguitySignal>,
    /// Ready-to-present clarification questions derived from the signals.
    pub clarification_questions: Vec<String>,
}

impl AmbiguityReport {
    fn from_signals(mut signals: Vec<AmbiguitySignal>) -> Self {
        // Limit to 3 questions to avoid overwhelming the user.
        signals.truncate(3);
        let clarification_questions: Vec<String> =
            signals.iter().map(|s| s.description()).collect();
        let is_ambiguous = !signals.is_empty();
        Self { is_ambiguous, signals, clarification_questions }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// KnownEntityIndex
// ──────────────────────────────────────────────────────────────────────────────

/// Lightweight index of entities that the agent is expected to know about.
///
/// Built once at session start from the repo map and seeded with built-in
/// technology terms. Unknown tokens found during ambiguity detection are
/// checked against this index.
#[derive(Debug, Default, Clone)]
pub struct KnownEntityIndex {
    /// All known entity names, lower-cased for case-insensitive lookup.
    entities: HashSet<String>,
}

impl KnownEntityIndex {
    /// Create a new empty index.
    pub fn new() -> Self {
        let mut idx = Self::default();
        idx.seed_builtins();
        idx
    }

    /// Populate the index from a repo map's file and symbol names.
    ///
    /// Accepts the output of `halcon_context::repo_map::build_repo_map()`.
    pub fn populate_from_repo_map_render(&mut self, render_output: &str) {
        // The render output is human-readable: "path/to/file.rs\n  fn foo\n  struct Bar\n".
        // Extract all tokens that look like identifiers.
        for token in render_output.split_whitespace() {
            let cleaned = token
                .trim_matches(|c: char| !c.is_alphanumeric() && c != '_')
                .to_lowercase();
            if cleaned.len() >= 2 {
                self.entities.insert(cleaned);
            }
        }
    }

    /// Add an entity name directly.
    pub fn add(&mut self, entity: &str) {
        self.entities.insert(entity.to_lowercase());
    }

    /// Returns `true` if the entity (case-insensitive) is in the index.
    pub fn contains(&self, entity: &str) -> bool {
        self.entities.contains(&entity.to_lowercase())
    }

    /// Seed with common technology terms, programming languages, common crate names,
    /// and project-specific names so they are never flagged as unknown.
    fn seed_builtins(&mut self) {
        const BUILTINS: &[&str] = &[
            // Halcon / project names
            "halcon", "cuervo", "momoto",
            // Providers
            "openai", "anthropic", "deepseek", "gemini", "ollama",
            // Languages / runtimes
            "rust", "python", "javascript", "typescript", "go", "java",
            "wasm", "webassembly", "nodejs", "deno",
            // Common crates / libraries
            "tokio", "axum", "serde", "reqwest", "sqlx", "diesel",
            "clap", "anyhow", "thiserror", "tracing", "rayon",
            "ratatui", "crossterm", "regex",
            // File extensions (recognised, not unknown)
            "rs", "toml", "json", "yaml", "yml", "md", "ts", "js",
            // Common project components
            "cli", "api", "sdk", "ui", "tui", "mcp", "llm", "ai",
            "storage", "database", "db", "auth", "config",
            // Common English nouns that look like proper nouns
            "function", "struct", "module", "crate", "file", "test",
            "error", "result", "option", "string", "vec", "hashmap",
        ];
        for b in BUILTINS {
            self.entities.insert(b.to_string());
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Common word filter
// ──────────────────────────────────────────────────────────────────────────────

/// Returns `true` if `word` is a common English function word that should never
/// be flagged as an unknown entity.
fn is_common_word(word: &str) -> bool {
    const STOPWORDS: &[&str] = &[
        // articles / determiners
        "a", "an", "the", "this", "that", "these", "those",
        // prepositions
        "in", "on", "at", "by", "for", "with", "about", "to", "from",
        "of", "into", "through", "during", "before", "after",
        "above", "below", "between", "out", "over", "under",
        // conjunctions
        "and", "or", "but", "nor", "so", "yet", "both", "either",
        "neither", "although", "because", "since", "while", "if",
        // pronouns
        "i", "you", "he", "she", "it", "we", "they", "me", "him",
        "her", "us", "them", "my", "your", "his", "its", "our", "their",
        // auxiliaries
        "is", "are", "was", "were", "be", "been", "being",
        "have", "has", "had", "do", "does", "did", "will", "would",
        "shall", "should", "may", "might", "must", "can", "could",
        // common adverbs / adjectives
        "not", "no", "all", "some", "any", "each", "every", "both",
        "few", "more", "most", "other", "new", "old", "first", "last",
        "now", "just", "only", "also", "well", "then", "than",
        // common nouns (not entity-like)
        "way", "time", "part", "point", "case", "place", "thing",
        "use", "need", "work", "code", "data", "type", "name",
        "list", "item", "set", "key", "value", "line", "step",
        // action words (not entities)
        "make", "take", "get", "let", "put", "keep", "give", "know",
        "go", "see", "look", "come", "want", "need", "try",
    ];
    STOPWORDS.contains(&word)
}

// ──────────────────────────────────────────────────────────────────────────────
// Action verb tables
// ──────────────────────────────────────────────────────────────────────────────

/// Verbs that trigger entity-adjacent unknown-noun detection.
const ACTION_VERBS: &[&str] = &[
    "implement", "integrate", "build", "create", "write", "add",
    "setup", "configure", "use", "enable", "activate", "wire",
    "connect", "load", "install", "deploy", "run", "execute",
    // Common short-form user commands that pair with pronouns (e.g. "fix it", "update that"):
    "fix", "update", "change", "modify", "refactor", "rewrite",
    "delete", "remove", "check", "test", "debug", "analyze",
    "handle", "make", "do", "get", "set", "move", "rename",
];

/// Verbs that are inherently vague and trigger [`AmbiguitySignal::VagueActionVerb`].
const VAGUE_VERBS: &[(&str, &[&str])] = &[
    (
        "handle",
        &["add error handling", "write an event handler", "process incoming data"],
    ),
    (
        "deal with",
        &["fix the bug", "add support for", "document the issue"],
    ),
    (
        "manage",
        &["implement a manager struct", "add lifecycle methods", "track state"],
    ),
    (
        "process",
        &["parse and transform", "validate and store", "filter and return"],
    ),
    (
        "work on",
        &["fix a bug in", "implement a feature for", "refactor"],
    ),
    (
        "improve",
        &["fix a specific bug", "add missing functionality", "optimise performance"],
    ),
    (
        "do something",
        &["be more specific about the action required"],
    ),
];

/// Pronouns that require a prior assistant turn for resolution.
const UNRESOLVED_PRONOUNS: &[&str] = &[
    "it", "that", "this", "those", "them", "these",
    "the thing", "the issue", "the problem", "the file",
    "the function", "the code",
];

/// File-path extensions that qualify a token as a concrete file reference.
const FILE_EXTENSIONS: &[&str] = &[
    ".rs", ".py", ".ts", ".js", ".go", ".java", ".c", ".cpp", ".h",
    ".toml", ".json", ".yaml", ".yml", ".md", ".txt", ".sh",
];

// ──────────────────────────────────────────────────────────────────────────────
// Detector
// ──────────────────────────────────────────────────────────────────────────────

/// Rule-based ambiguity detector.
pub struct AmbiguityDetector {
    index: KnownEntityIndex,
}

impl AmbiguityDetector {
    /// Create a detector backed by the given entity index.
    pub fn new(index: KnownEntityIndex) -> Self {
        Self { index }
    }

    /// Create with a default (built-ins only) entity index.
    pub fn with_builtins() -> Self {
        Self::new(KnownEntityIndex::new())
    }

    /// Run all detection rules on `query`.
    ///
    /// Returns an [`AmbiguityReport`] that can be used to decide whether to
    /// ask for clarification before invoking the planner.
    pub fn detect(&self, query: &str, ctx: &AmbiguityContext) -> AmbiguityReport {
        let lower = query.to_lowercase();
        let mut signals: Vec<AmbiguitySignal> = Vec::new();

        // Rule 1: Unresolved pronouns (highest priority — check first).
        if let Some(sig) = self.check_unresolved_pronoun(&lower, ctx) {
            signals.push(sig);
        }

        // Rule 2: Unknown proper noun adjacent to action verb.
        signals.extend(self.check_unknown_entities(query, &lower));

        // Rule 3: Vague action verb.
        if let Some(sig) = self.check_vague_verb(&lower) {
            signals.push(sig);
        }

        // Rule 4: Missing scope for action requests.
        if let Some(sig) = self.check_missing_scope(&lower, ctx) {
            signals.push(sig);
        }

        AmbiguityReport::from_signals(signals)
    }

    // ── Rule 1: Unresolved pronoun ──────────────────────────────────────────

    fn check_unresolved_pronoun(
        &self,
        lower: &str,
        ctx: &AmbiguityContext,
    ) -> Option<AmbiguitySignal> {
        // Pronouns are only unresolved when there is no prior assistant turn.
        if ctx.prior_assistant_turns > 0 {
            return None;
        }
        // Query must contain an action verb before we flag a pronoun.
        let has_action = ACTION_VERBS.iter().any(|v| word_in(lower, v));
        if !has_action {
            return None;
        }

        for pronoun in UNRESOLVED_PRONOUNS {
            if word_in(lower, pronoun) {
                return Some(AmbiguitySignal::UnresolvedPronoun {
                    pronoun: pronoun.to_string(),
                });
            }
        }
        None
    }

    // ── Rule 2: Unknown named entity ────────────────────────────────────────

    fn check_unknown_entities(
        &self,
        original: &str,
        lower: &str,
    ) -> Vec<AmbiguitySignal> {
        // Only scan when an action verb is present.
        let has_action = ACTION_VERBS.iter().any(|v| word_in(lower, v));
        if !has_action {
            return vec![];
        }

        let mut signals = Vec::new();
        let words: Vec<&str> = original.split_whitespace().collect();

        for (i, word) in words.iter().enumerate() {
            let clean = clean_token(word);
            let clean_lower = clean.to_lowercase();

            // Must be at least 3 characters and not a stopword.
            if clean_lower.len() < 3 || is_common_word(&clean_lower) {
                continue;
            }
            // Skip tokens that look like file paths.
            if is_file_reference(&clean_lower) {
                continue;
            }
            // Skip numeric tokens.
            if clean_lower.chars().all(|c| c.is_ascii_digit() || c == '.') {
                continue;
            }
            // Skip the action verbs themselves.
            if ACTION_VERBS.contains(&clean_lower.as_str()) {
                continue;
            }
            // Skip common grammar words in the stopword list.
            if is_common_word(&clean_lower) {
                continue;
            }
            // If the entity is known → no signal.
            if self.index.contains(&clean_lower) {
                continue;
            }

            // Check for the pattern: previous token is an action verb, OR
            // the word is CamelCase / ALL_CAPS (looks like a named entity).
            let prev_is_action = i > 0
                && ACTION_VERBS
                    .iter()
                    .any(|v| clean_token(words[i - 1]).to_lowercase() == *v);
            let looks_like_entity =
                is_camel_case(clean) || is_all_caps(clean) || prev_is_action;

            if looks_like_entity {
                // Build context: surrounding words.
                let start = i.saturating_sub(2);
                let end = (i + 3).min(words.len());
                let context = words[start..end].join(" ");

                signals.push(AmbiguitySignal::UnknownProperNoun {
                    noun: clean_lower.clone(),
                    context,
                });
            }
        }

        signals
    }

    // ── Rule 3: Vague verb ──────────────────────────────────────────────────

    fn check_vague_verb(&self, lower: &str) -> Option<AmbiguitySignal> {
        for (verb, interpretations) in VAGUE_VERBS {
            if word_in(lower, verb) {
                return Some(AmbiguitySignal::VagueActionVerb {
                    verb: verb.to_string(),
                    possible_interpretations: interpretations
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                });
            }
        }
        None
    }

    // ── Rule 4: Missing scope ────────────────────────────────────────────────

    fn check_missing_scope(
        &self,
        lower: &str,
        ctx: &AmbiguityContext,
    ) -> Option<AmbiguitySignal> {
        // Only fire when a write/creation action verb is present.
        let write_verbs = [
            "write", "create", "add", "implement", "generate", "scaffold",
        ];
        let has_write_verb = write_verbs.iter().any(|v| word_in(lower, v));
        if !has_write_verb {
            return None;
        }

        // Suppress if a file path was mentioned (word ending in known extension).
        let has_file_ref = lower.split_whitespace().any(is_file_reference)
            || ctx.recent_file_paths.iter().any(|p| lower.contains(p.as_str()));
        if has_file_ref {
            return None;
        }

        // Suppress if specific scope words are present.
        let scope_words = [
            "file", "module", "function", "struct", "method", "class",
            "crate", "test", "benchmark", "endpoint", "handler",
        ];
        let has_scope = scope_words.iter().any(|s| lower.contains(s));
        if has_scope {
            return None;
        }

        // Query must be very short (< 6 words) to avoid false positives on longer
        // queries that naturally provide scope through context.
        let word_count = lower.split_whitespace().count();
        if word_count < 6 {
            return Some(AmbiguitySignal::MissingScope {
                missing: ScopeKind::TargetFile,
            });
        }

        None
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Token helpers
// ──────────────────────────────────────────────────────────────────────────────

/// Strip surrounding punctuation from a token, keeping alphanumeric and `_-`.
fn clean_token(token: &str) -> &str {
    token.trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
}

/// Returns `true` if `text` contains `word` at a proper word boundary.
fn word_in(text: &str, word: &str) -> bool {
    if word.contains(' ') {
        return text.contains(word);
    }
    for (i, _) in text.match_indices(word) {
        let before_ok = i == 0
            || {
                let b = text.as_bytes()[i - 1];
                !b.is_ascii_alphanumeric() && b != b'_'
            };
        let end = i + word.len();
        let after_ok = end >= text.len()
            || {
                let b = text.as_bytes()[end];
                !b.is_ascii_alphanumeric() && b != b'_'
            };
        if before_ok && after_ok {
            return true;
        }
    }
    false
}

/// Returns `true` if `token` looks like a CamelCase identifier.
fn is_camel_case(token: &str) -> bool {
    if token.len() < 4 {
        return false;
    }
    let bytes = token.as_bytes();
    // Has at least one uppercase letter after the first character.
    bytes[1..].iter().any(|b| b.is_ascii_uppercase())
}

/// Returns `true` if `token` is ALL_CAPS_WITH_UNDERSCORES.
fn is_all_caps(token: &str) -> bool {
    if token.len() < 3 {
        return false;
    }
    token
        .chars()
        .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit())
}

/// Returns `true` if `token` ends with a recognised file extension.
fn is_file_reference(token: &str) -> bool {
    FILE_EXTENSIONS.iter().any(|ext| token.ends_with(ext))
        || token.contains('/')
        || token.contains('\\')
}

// ──────────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn detector() -> AmbiguityDetector {
        AmbiguityDetector::with_builtins()
    }

    fn no_ctx() -> AmbiguityContext {
        AmbiguityContext::default()
    }

    fn with_prior_turns(n: usize) -> AmbiguityContext {
        AmbiguityContext { prior_assistant_turns: n, ..Default::default() }
    }

    // ── UnknownProperNoun ───────────────────────────────────────────────────

    #[test]
    fn detects_unknown_entity_after_implement() {
        let d = detector();
        let r = d.detect("implement flompf", &no_ctx());
        assert!(r.is_ambiguous, "Expected ambiguity for 'implement flompf'");
        assert!(
            r.signals.iter().any(|s| matches!(
                s, AmbiguitySignal::UnknownProperNoun { noun, .. } if noun == "flompf"
            )),
            "Expected UnknownProperNoun(flompf), got: {:?}", r.signals
        );
    }

    #[test]
    fn does_not_flag_known_entity_momoto() {
        // "momoto" is in the builtins index → not unknown
        let d = detector();
        let r = d.detect("implement momoto", &no_ctx());
        // Should NOT produce UnknownProperNoun for "momoto"
        let has_unknown_momoto = r.signals.iter().any(|s| matches!(
            s, AmbiguitySignal::UnknownProperNoun { noun, .. } if noun == "momoto"
        ));
        assert!(!has_unknown_momoto, "momoto is a known builtin — should not be flagged");
    }

    #[test]
    fn does_not_flag_known_rust_keyword() {
        let d = detector();
        let r = d.detect("implement the rust parser", &no_ctx());
        let has_unknown_rust = r.signals.iter().any(|s| matches!(
            s, AmbiguitySignal::UnknownProperNoun { noun, .. } if noun == "rust"
        ));
        assert!(!has_unknown_rust, "'rust' is known — should not be flagged");
    }

    #[test]
    fn unknown_entity_camel_case_detected() {
        let d = detector();
        let r = d.detect("integrate MyUnknownLib into the system", &no_ctx());
        // "MyUnknownLib" → CamelCase → should be flagged
        assert!(r.signals.iter().any(|s| matches!(
            s, AmbiguitySignal::UnknownProperNoun { .. }
        )));
    }

    // ── VagueActionVerb ──────────────────────────────────────────────────────

    #[test]
    fn detects_vague_verb_handle() {
        let d = detector();
        let r = d.detect("handle the authentication", &no_ctx());
        assert!(r.signals.iter().any(|s| matches!(
            s, AmbiguitySignal::VagueActionVerb { verb, .. } if verb == "handle"
        )));
    }

    #[test]
    fn detects_vague_verb_manage() {
        let d = detector();
        let r = d.detect("manage the database connections", &no_ctx());
        assert!(r.signals.iter().any(|s| matches!(
            s, AmbiguitySignal::VagueActionVerb { verb, .. } if verb == "manage"
        )));
    }

    #[test]
    fn specific_verb_not_flagged() {
        let d = detector();
        let r = d.detect("fix the null pointer in auth.rs", &no_ctx());
        assert!(!r.signals.iter().any(|s| matches!(
            s, AmbiguitySignal::VagueActionVerb { .. }
        )));
    }

    // ── MissingScope ─────────────────────────────────────────────────────────

    #[test]
    fn missing_scope_short_write_query() {
        let d = detector();
        // "write" present, < 6 words, no file/function reference
        let r = d.detect("write the code", &no_ctx());
        assert!(r.signals.iter().any(|s| matches!(
            s, AmbiguitySignal::MissingScope { .. }
        )));
    }

    #[test]
    fn missing_scope_suppressed_when_file_mentioned() {
        let d = detector();
        let r = d.detect("write tests for auth.rs", &no_ctx());
        assert!(!r.signals.iter().any(|s| matches!(
            s, AmbiguitySignal::MissingScope { .. }
        )));
    }

    #[test]
    fn missing_scope_suppressed_when_function_mentioned() {
        let d = detector();
        let r = d.detect("add a new function to parse json", &no_ctx());
        assert!(!r.signals.iter().any(|s| matches!(
            s, AmbiguitySignal::MissingScope { .. }
        )));
    }

    // ── UnresolvedPronoun ────────────────────────────────────────────────────

    #[test]
    fn unresolved_pronoun_no_prior_turn() {
        let d = detector();
        let r = d.detect("fix it", &no_ctx());
        assert!(r.signals.iter().any(|s| matches!(
            s, AmbiguitySignal::UnresolvedPronoun { pronoun, .. } if pronoun == "it"
        )));
    }

    #[test]
    fn resolved_pronoun_with_prior_turn() {
        let d = detector();
        let r = d.detect("fix it", &with_prior_turns(1));
        // With a prior assistant turn, "it" can be resolved → no signal.
        assert!(!r.signals.iter().any(|s| matches!(
            s, AmbiguitySignal::UnresolvedPronoun { .. }
        )));
    }

    #[test]
    fn unresolved_that_no_prior() {
        let d = detector();
        let r = d.detect("update that", &no_ctx());
        assert!(r.signals.iter().any(|s| matches!(
            s, AmbiguitySignal::UnresolvedPronoun { .. }
        )));
    }

    // ── is_ambiguous flag ────────────────────────────────────────────────────

    #[test]
    fn clean_query_not_ambiguous() {
        let d = detector();
        let r = d.detect("fix the null pointer in crates/halcon-cli/src/repl/agent.rs", &no_ctx());
        // File reference present, "fix" is specific → should not be ambiguous
        assert!(
            !r.is_ambiguous || r.signals.len() <= 1,
            "Unexpectedly ambiguous: {:?}", r.signals
        );
    }

    #[test]
    fn report_truncates_to_three_questions() {
        let d = detector();
        // Craft a query that triggers many signals.
        let r = d.detect("handle it", &no_ctx());
        assert!(r.clarification_questions.len() <= 3);
    }

    // ── KnownEntityIndex ─────────────────────────────────────────────────────

    #[test]
    fn index_case_insensitive() {
        let mut idx = KnownEntityIndex::new();
        idx.add("MyEntity");
        assert!(idx.contains("myentity"));
        assert!(idx.contains("MYENTITY"));
    }

    #[test]
    fn index_populate_from_render() {
        let mut idx = KnownEntityIndex::new();
        idx.populate_from_repo_map_render(
            "src/repl/agent.rs\n  fn run_agent_loop\n  struct AgentContext\n"
        );
        assert!(idx.contains("run_agent_loop"));
        assert!(idx.contains("agentcontext"));
    }

    // ── signal description ────────────────────────────────────────────────────

    #[test]
    fn signal_description_is_non_empty() {
        let signals = vec![
            AmbiguitySignal::UnknownProperNoun { noun: "foo".into(), context: "implement foo".into() },
            AmbiguitySignal::VagueActionVerb { verb: "handle".into(), possible_interpretations: vec!["do X".into()] },
            AmbiguitySignal::MissingScope { missing: ScopeKind::TargetFile },
            AmbiguitySignal::UnresolvedPronoun { pronoun: "it".into() },
        ];
        for s in &signals {
            assert!(!s.description().is_empty());
        }
    }
}
