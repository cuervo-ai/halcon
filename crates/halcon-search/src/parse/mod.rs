//! HTML parsing (placeholder - to be implemented in Phase 3).

use crate::error::Result;
use crate::types::ParsedDocument;

pub struct HTMLParser;

impl HTMLParser {
    pub fn new() -> Self {
        Self
    }

    pub fn parse(&self, _html: &str, _url: &url::Url) -> Result<ParsedDocument> {
        // Stub implementation
        unimplemented!("HTML parsing not yet implemented")
    }
}

impl Default for HTMLParser {
    fn default() -> Self {
        Self::new()
    }
}
