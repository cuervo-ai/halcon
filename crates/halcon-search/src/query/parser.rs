//! Query parsing.

use crate::error::Result;

/// Parsed query representation.
#[derive(Debug, Clone)]
pub struct ParsedQuery {
    /// FTS5-compatible query string.
    pub fts_query: String,
    /// Original query.
    pub original: String,
}

pub struct QueryParser;

impl QueryParser {
    pub fn new() -> Self {
        Self
    }

    /// Parse query into FTS5-compatible format.
    ///
    /// Currently: pass-through with basic sanitization.
    /// Future: support operators (AND, OR, NOT, "phrase", site:, etc.)
    pub fn parse(&self, query: &str) -> Result<ParsedQuery> {
        // Basic sanitization: remove special FTS5 chars that could break query
        let sanitized = query
            .replace('"', "")
            .replace('*', "")
            .replace(':', " ")
            .trim()
            .to_string();

        Ok(ParsedQuery {
            fts_query: sanitized.clone(),
            original: query.to_string(),
        })
    }
}

impl Default for QueryParser {
    fn default() -> Self {
        Self::new()
    }
}
