//! Tokenization for indexing.
//!
//! Uses unicode segmentation + porter stemming (delegated to FTS5).

use crate::error::Result;
use unicode_segmentation::UnicodeSegmentation;

pub struct Tokenizer;

impl Tokenizer {
    pub fn new() -> Self {
        Self
    }

    /// Tokenize text into normalized terms.
    ///
    /// - Unicode word segmentation
    /// - Lowercase normalization
    /// - Min length 2 characters
    /// - Filters stopwords (basic English)
    ///
    /// Note: Porter stemming is handled by FTS5's `porter` tokenizer.
    pub fn tokenize(&self, text: &str, _language: Option<&str>) -> Result<Vec<String>> {
        let tokens: Vec<String> = text
            .unicode_words()
            .map(|w| w.to_lowercase())
            .filter(|w| w.len() >= 2 && !self.is_stopword(w))
            .collect();

        Ok(tokens)
    }

    /// Check if word is a stopword (basic English list).
    fn is_stopword(&self, word: &str) -> bool {
        matches!(
            word,
            "the" | "a" | "an" | "and" | "or" | "but" | "in" | "on" | "at" | "to" | "for" | "of"
                | "with" | "by" | "from" | "as" | "is" | "was" | "are" | "were" | "be" | "been"
                | "this" | "that" | "these" | "those" | "it" | "its" | "he" | "she" | "they"
                | "we" | "you" | "i" | "me" | "my" | "your" | "their" | "his" | "her"
        )
    }
}

impl Default for Tokenizer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_basic() {
        let tokenizer = Tokenizer::new();
        let tokens = tokenizer.tokenize("Hello, world! This is a test.", None).unwrap();
        assert!(tokens.contains(&"hello".to_string()));
        assert!(tokens.contains(&"world".to_string()));
        assert!(tokens.contains(&"test".to_string()));
        // Stopwords removed
        assert!(!tokens.contains(&"this".to_string()));
        assert!(!tokens.contains(&"is".to_string()));
        assert!(!tokens.contains(&"a".to_string()));
    }

    #[test]
    fn tokenize_unicode() {
        let tokenizer = Tokenizer::new();
        let tokens = tokenizer
            .tokenize("Rust 🦀 programming language", None)
            .unwrap();
        assert!(tokens.contains(&"rust".to_string()));
        assert!(tokens.contains(&"programming".to_string()));
        assert!(tokens.contains(&"language".to_string()));
    }

    #[test]
    fn tokenize_min_length() {
        let tokenizer = Tokenizer::new();
        let tokens = tokenizer.tokenize("I am ok", None).unwrap();
        // "I" is 1 char, filtered
        // "am" is 2 chars but stopword
        assert!(tokens.contains(&"ok".to_string()));
        assert_eq!(tokens.len(), 1);
    }
}
