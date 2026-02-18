//! Crawl engine (placeholder - to be implemented in Phase 4).

use crate::error::Result;
use crate::types::CrawlSession;
use url::Url;

pub struct CrawlEngine;

impl CrawlEngine {
    pub fn new() -> Self {
        Self
    }

    pub async fn crawl(&self, seed: Url, _depth: u32) -> Result<CrawlSession> {
        // Stub implementation
        Ok(CrawlSession::new(seed))
    }
}

impl Default for CrawlEngine {
    fn default() -> Self {
        Self::new()
    }
}
