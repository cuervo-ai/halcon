//! Task complexity and type analysis for adaptive reasoning.
//!
//! Classifies user queries by:
//! - Complexity (Simple, Moderate, Complex)
//! - Task type (CodeGeneration, Debugging, Research, etc.)
//! - Content hash (SHA-256 for experience lookup)

use sha2::{Digest, Sha256};

/// Task complexity derived from query length and keyword presence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaskComplexity {
    /// Short query (< 15 words), no complex keywords.
    Simple,
    /// Medium query (15-50 words) or simple code keywords.
    Moderate,
    /// Long query (> 50 words) or complex patterns.
    Complex,
}

/// Task type classification for strategy selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaskType {
    /// Write new code, create functions/classes.
    CodeGeneration,
    /// Modify existing code, refactor, update.
    CodeModification,
    /// Fix bugs, resolve errors, diagnose issues.
    Debugging,
    /// Explain concepts, find information, analyze.
    Research,
    /// File operations, directory management.
    FileManagement,
    /// Git operations (commit, status, diff).
    GitOperation,
    /// Explain how something works.
    Explanation,
    /// Configure settings, setup tools.
    Configuration,
    /// General tasks that don't fit categories.
    General,
}

impl TaskType {
    /// Convert to string for database storage.
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskType::CodeGeneration => "code_generation",
            TaskType::CodeModification => "code_modification",
            TaskType::Debugging => "debugging",
            TaskType::Research => "research",
            TaskType::FileManagement => "file_management",
            TaskType::GitOperation => "git_operation",
            TaskType::Explanation => "explanation",
            TaskType::Configuration => "configuration",
            TaskType::General => "general",
        }
    }

    /// Parse from string (database roundtrip).
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "code_generation" => Some(TaskType::CodeGeneration),
            "code_modification" => Some(TaskType::CodeModification),
            "debugging" => Some(TaskType::Debugging),
            "research" => Some(TaskType::Research),
            "file_management" => Some(TaskType::FileManagement),
            "git_operation" => Some(TaskType::GitOperation),
            "explanation" => Some(TaskType::Explanation),
            "configuration" => Some(TaskType::Configuration),
            "general" => Some(TaskType::General),
            _ => None,
        }
    }
}

/// Result of task analysis.
#[derive(Debug, Clone)]
pub struct TaskAnalysis {
    pub complexity: TaskComplexity,
    pub task_type: TaskType,
    pub task_hash: String, // SHA-256 hex for experience lookup
    pub word_count: usize,
}

/// Analyzer for classifying user queries.
pub struct TaskAnalyzer;

impl TaskAnalyzer {
    /// Analyze a user query and return classification.
    pub fn analyze(query: &str) -> TaskAnalysis {
        let complexity = Self::classify_complexity(query);
        let task_type = Self::classify_type(query);
        let task_hash = Self::compute_hash(query);
        let word_count = query.split_whitespace().count();

        TaskAnalysis {
            complexity,
            task_type,
            task_hash,
            word_count,
        }
    }

    /// Classify complexity based on length and keywords.
    fn classify_complexity(query: &str) -> TaskComplexity {
        let word_count = query.split_whitespace().count();
        let query_lower = query.to_lowercase();

        // Complex patterns (regardless of length)
        let complex_keywords = [
            "refactor",
            "optimize",
            "migrate",
            "integrate",
            "architecture",
            "design pattern",
            "performance",
            "scale",
            "distributed",
            "microservice",
        ];

        if complex_keywords
            .iter()
            .any(|kw| query_lower.contains(kw))
        {
            return TaskComplexity::Complex;
        }

        // Length-based classification
        if word_count < 10 {
            TaskComplexity::Simple
        } else if word_count <= 35 {
            TaskComplexity::Moderate
        } else {
            TaskComplexity::Complex
        }
    }

    /// Classify task type based on keywords.
    fn classify_type(query: &str) -> TaskType {
        let query_lower = query.to_lowercase();

        // Git operation keywords (check first - most specific)
        if Self::contains_any(
            &query_lower,
            &[
                "git commit",
                "git status",
                "git diff",
                "git log",
                "git add",
                "commit changes",
                "stage files",
            ],
        ) {
            return TaskType::GitOperation;
        }

        // Code generation keywords
        if Self::contains_any(
            &query_lower,
            &[
                "write",
                "create",
                "implement",
                "add function",
                "add method",
                "add class",
                "generate",
                "scaffold",
            ],
        ) {
            return TaskType::CodeGeneration;
        }

        // Debugging keywords
        if Self::contains_any(
            &query_lower,
            &[
                "fix",
                "error",
                "bug",
                "why doesn't",
                "not working",
                "broken",
                "crash",
                "fails",
                "issue",
                "problem",
            ],
        ) {
            return TaskType::Debugging;
        }

        // Code modification keywords
        if Self::contains_any(
            &query_lower,
            &[
                "modify",
                "change",
                "update",
                "edit",
                "refactor",
                "rename",
                "move",
                "replace",
            ],
        ) {
            return TaskType::CodeModification;
        }

        // File management keywords
        if Self::contains_any(
            &query_lower,
            &[
                "delete file",
                "create directory",
                "move file",
                "copy file",
                "list files",
                "find files",
                "search files",
            ],
        ) {
            return TaskType::FileManagement;
        }

        // Research keywords
        if Self::contains_any(
            &query_lower,
            &[
                "find",
                "search",
                "lookup",
                "research",
                "investigate",
                "analyze",
                "compare",
                "review",
            ],
        ) {
            return TaskType::Research;
        }

        // Explanation keywords
        if Self::contains_any(
            &query_lower,
            &[
                "explain",
                "how does",
                "what is",
                "why does",
                "describe",
                "tell me about",
                "what are",
            ],
        ) {
            return TaskType::Explanation;
        }

        // Configuration keywords
        if Self::contains_any(
            &query_lower,
            &[
                "configure",
                "setup",
                "install",
                "initialize",
                "settings",
                "config",
            ],
        ) {
            return TaskType::Configuration;
        }

        // Default to General
        TaskType::General
    }

    /// Compute SHA-256 hash of query for experience lookup.
    fn compute_hash(query: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(query.trim().to_lowercase().as_bytes());
        format!("{:x}", hasher.finalize())
    }

    /// Check if query contains any of the keywords with word-boundary awareness.
    ///
    /// Multi-word phrases (containing spaces) use substring matching.
    /// Single words require proper word boundaries to avoid false positives
    /// like "fix" matching "prefix" or "write" matching "rewrite".
    fn contains_any(query: &str, keywords: &[&str]) -> bool {
        keywords.iter().any(|kw| {
            if kw.contains(' ') {
                // Multi-word phrase: substring match is correct
                query.contains(kw)
            } else {
                // Single word: require word boundaries
                Self::contains_word(query, kw)
            }
        })
    }

    /// Check if `text` contains `word` at a word boundary.
    ///
    /// A word boundary is defined as the position where an alphanumeric
    /// character is preceded/followed by a non-alphanumeric character (or
    /// the start/end of the string). Underscores count as word characters.
    fn contains_word(text: &str, word: &str) -> bool {
        let bytes = text.as_bytes();
        let wbytes = word.as_bytes();
        let wlen = wbytes.len();
        let tlen = bytes.len();
        if wlen > tlen {
            return false;
        }
        let mut start = 0;
        while start + wlen <= tlen {
            if let Some(rel) = text[start..].find(word) {
                let pos = start + rel;
                let before_ok = pos == 0 || !bytes[pos - 1].is_ascii_alphanumeric() && bytes[pos - 1] != b'_';
                let after_pos = pos + wlen;
                let after_ok = after_pos >= tlen || !bytes[after_pos].is_ascii_alphanumeric() && bytes[after_pos] != b'_';
                if before_ok && after_ok {
                    return true;
                }
                start = pos + 1;
            } else {
                break;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complexity_simple_short_query() {
        let analysis = TaskAnalyzer::analyze("list files");
        assert_eq!(analysis.complexity, TaskComplexity::Simple);
        assert_eq!(analysis.word_count, 2);
    }

    #[test]
    fn complexity_moderate_medium_query() {
        let analysis = TaskAnalyzer::analyze("create a new function that takes a string and returns uppercase");
        assert_eq!(analysis.complexity, TaskComplexity::Moderate);
        assert!(analysis.word_count >= 10 && analysis.word_count <= 35);
    }

    #[test]
    fn complexity_complex_long_query() {
        let long_query = "write a function that reads data from a database, processes it through multiple transformations, validates the output against a schema, handles errors gracefully, logs all operations, and returns a structured response with metadata including timestamps and processing stats";
        let analysis = TaskAnalyzer::analyze(long_query);
        assert_eq!(analysis.complexity, TaskComplexity::Complex);
        assert!(analysis.word_count > 35);
    }

    #[test]
    fn complexity_complex_keyword_override() {
        let analysis = TaskAnalyzer::analyze("refactor this code"); // Only 3 words but has "refactor"
        assert_eq!(analysis.complexity, TaskComplexity::Complex);
    }

    #[test]
    fn type_code_generation() {
        let analysis = TaskAnalyzer::analyze("write a new function to parse JSON");
        assert_eq!(analysis.task_type, TaskType::CodeGeneration);
    }

    #[test]
    fn type_debugging() {
        let analysis = TaskAnalyzer::analyze("fix the error in authentication");
        assert_eq!(analysis.task_type, TaskType::Debugging);
    }

    #[test]
    fn type_code_modification() {
        let analysis = TaskAnalyzer::analyze("modify the login function to use async");
        assert_eq!(analysis.task_type, TaskType::CodeModification);
    }

    #[test]
    fn type_file_management() {
        let analysis = TaskAnalyzer::analyze("delete file temp.txt");
        assert_eq!(analysis.task_type, TaskType::FileManagement);
    }

    #[test]
    fn type_git_operation() {
        let analysis = TaskAnalyzer::analyze("git commit with message 'fix bug'");
        assert_eq!(analysis.task_type, TaskType::GitOperation);
    }

    #[test]
    fn type_research() {
        let analysis = TaskAnalyzer::analyze("find all uses of this function");
        assert_eq!(analysis.task_type, TaskType::Research);
    }

    #[test]
    fn type_explanation() {
        let analysis = TaskAnalyzer::analyze("explain how async/await works");
        assert_eq!(analysis.task_type, TaskType::Explanation);
    }

    #[test]
    fn type_configuration() {
        let analysis = TaskAnalyzer::analyze("configure the database connection");
        assert_eq!(analysis.task_type, TaskType::Configuration);
    }

    #[test]
    fn type_general_fallback() {
        let analysis = TaskAnalyzer::analyze("hello there");
        assert_eq!(analysis.task_type, TaskType::General);
    }

    #[test]
    fn hash_is_consistent() {
        let hash1 = TaskAnalyzer::compute_hash("test query");
        let hash2 = TaskAnalyzer::compute_hash("test query");
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn hash_is_case_insensitive() {
        let hash1 = TaskAnalyzer::compute_hash("Test Query");
        let hash2 = TaskAnalyzer::compute_hash("test query");
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn hash_trims_whitespace() {
        let hash1 = TaskAnalyzer::compute_hash("  test query  ");
        let hash2 = TaskAnalyzer::compute_hash("test query");
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn hash_is_sha256_hex() {
        let hash = TaskAnalyzer::compute_hash("test");
        assert_eq!(hash.len(), 64); // SHA-256 = 32 bytes = 64 hex chars
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn task_type_roundtrip() {
        let types = [
            TaskType::CodeGeneration,
            TaskType::Debugging,
            TaskType::Research,
            TaskType::FileManagement,
            TaskType::GitOperation,
        ];

        for ty in &types {
            let s = ty.as_str();
            let parsed = TaskType::from_str(s).unwrap();
            assert_eq!(*ty, parsed);
        }
    }

    #[test]
    fn task_type_from_str_invalid() {
        assert_eq!(TaskType::from_str("invalid"), None);
    }

    // --- Word boundary matching tests ---

    #[test]
    fn contains_word_matches_whole_word() {
        assert!(TaskAnalyzer::contains_word("fix the bug", "fix"));
    }

    #[test]
    fn contains_word_rejects_substring_prefix() {
        // "fix" should NOT match inside "prefix"
        assert!(!TaskAnalyzer::contains_word("prefix this code", "fix"));
    }

    #[test]
    fn contains_word_rejects_substring_middle() {
        // "write" should NOT match inside "rewrite"
        assert!(!TaskAnalyzer::contains_word("rewrite the function", "write"));
    }

    #[test]
    fn contains_word_matches_at_end_of_string() {
        assert!(TaskAnalyzer::contains_word("please fix", "fix"));
    }

    #[test]
    fn contains_word_matches_at_start_of_string() {
        assert!(TaskAnalyzer::contains_word("fix the issue", "fix"));
    }

    #[test]
    fn contains_word_surrounded_by_punctuation() {
        assert!(TaskAnalyzer::contains_word("can you fix? yes", "fix"));
    }

    #[test]
    fn type_debugging_no_false_positive_from_prefix() {
        // "prefix" contains "fix" but should NOT classify as Debugging
        let analysis = TaskAnalyzer::analyze("prefix the function name");
        assert_ne!(analysis.task_type, TaskType::Debugging);
    }

    #[test]
    fn type_code_generation_no_false_positive_from_rewrite() {
        // "rewrite" contains "write" but should NOT classify as CodeGeneration
        // (it should fall through to CodeModification via "modify/update" keywords,
        // or fall to General since "rewrite" alone has no match in code_generation keywords)
        let analysis = TaskAnalyzer::analyze("rewrite this module");
        // "rewrite" does not have a word-boundary match for "write" → not CodeGeneration
        assert_ne!(analysis.task_type, TaskType::CodeGeneration);
    }

    #[test]
    fn type_debugging_direct_fix_keyword() {
        let analysis = TaskAnalyzer::analyze("fix this bug in the function");
        assert_eq!(analysis.task_type, TaskType::Debugging);
    }
}
