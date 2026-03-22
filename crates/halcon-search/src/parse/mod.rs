//! HTML parsing using the `scraper` crate.
//!
//! Provides real content extraction, metadata parsing, and link discovery.
//! Correctly excludes script/style content from indexed text.

use crate::error::{Result, SearchError};
use crate::types::{DocumentMetadata, ParsedDocument};

/// HTML document parser.
///
/// Extracts structured content from HTML pages:
/// - Plain text (excluding script/style/noscript)
/// - Title from `<title>` tag
/// - Meta description and keywords
/// - Outbound links
/// - Language (via whatlang detection)
pub struct HTMLParser;

impl HTMLParser {
    pub fn new() -> Self {
        Self
    }

    /// Parse an HTML document into a `ParsedDocument`.
    ///
    /// # Errors
    /// Returns `SearchError::ConfigError` if CSS selectors fail to compile
    /// (should not happen with hardcoded selectors).
    pub fn parse(&self, html: &str, url: &url::Url) -> Result<ParsedDocument> {
        use scraper::{Html, Selector};

        let document = Html::parse_document(html);

        // ── Title ────────────────────────────────────────────────────────────
        let title_sel = Selector::parse("title")
            .map_err(|e| SearchError::ConfigError(format!("title selector error: {e:?}")))?;
        let title = document
            .select(&title_sel)
            .next()
            .map(|el| el.text().collect::<String>().trim().to_string())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| url.to_string());

        // ── Body text (excluding script/style) ───────────────────────────────
        let text = extract_text_excluding_scripts(&document);

        // ── Meta description ─────────────────────────────────────────────────
        let desc_sel = Selector::parse("meta[name='description']")
            .map_err(|e| SearchError::ConfigError(format!("desc selector: {e:?}")))?;
        let description = document
            .select(&desc_sel)
            .next()
            .and_then(|el| el.value().attr("content"))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        // ── Meta keywords ─────────────────────────────────────────────────────
        let kw_sel = Selector::parse("meta[name='keywords']")
            .map_err(|e| SearchError::ConfigError(format!("keywords selector: {e:?}")))?;
        let keywords: Vec<String> = document
            .select(&kw_sel)
            .next()
            .and_then(|el| el.value().attr("content"))
            .map(|s| {
                s.split(',')
                    .map(|k| k.trim().to_string())
                    .filter(|k| !k.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        // ── OG meta ───────────────────────────────────────────────────────────
        let og_img_sel = Selector::parse("meta[property='og:image']")
            .map_err(|e| SearchError::ConfigError(format!("og:image selector: {e:?}")))?;
        let og_image = document
            .select(&og_img_sel)
            .next()
            .and_then(|el| el.value().attr("content"))
            .map(|s| s.to_string());

        // ── Canonical URL ─────────────────────────────────────────────────────
        let canonical_sel = Selector::parse("link[rel='canonical']")
            .map_err(|e| SearchError::ConfigError(format!("canonical selector: {e:?}")))?;
        let canonical_url = document
            .select(&canonical_sel)
            .next()
            .and_then(|el| el.value().attr("href"))
            .and_then(|href| url.join(href).ok());

        // ── Outbound links ───────────────────────────────────────────────────
        let link_sel = Selector::parse("a[href]")
            .map_err(|e| SearchError::ConfigError(format!("link selector: {e:?}")))?;
        let outlinks: Vec<url::Url> = document
            .select(&link_sel)
            .filter_map(|el| el.value().attr("href"))
            .filter_map(|href| url.join(href).ok())
            // Only keep http/https links
            .filter(|u| u.scheme() == "http" || u.scheme() == "https")
            .collect();

        // ── Language detection ────────────────────────────────────────────────
        let language = if text.len() >= 20 {
            whatlang::detect_lang(&text).map(|l| l.code().to_string())
        } else {
            None
        };

        Ok(ParsedDocument {
            url: url.clone(),
            title: title.clone(),
            text,
            html: Some(html.to_string()),
            metadata: DocumentMetadata {
                title,
                description,
                keywords,
                og_image,
                canonical_url,
                language: language.clone(),
                ..DocumentMetadata::default()
            },
            outlinks,
            language,
        })
    }
}

impl Default for HTMLParser {
    fn default() -> Self {
        Self::new()
    }
}

/// Walk the parsed HTML tree iteratively and collect text,
/// skipping script/style/noscript/head subtrees.
///
/// Uses an iterative stack to avoid recursion and avoids importing ego_tree
/// directly (prevents version-mismatch with scraper's bundled ego_tree).
fn extract_text_excluding_scripts(document: &scraper::Html) -> String {
    use scraper::node::Node;

    let mut parts: Vec<String> = Vec::new();
    // Stack of tree nodes; type is inferred from document.tree.root()
    let mut stack = vec![document.tree.root()];

    while let Some(node) = stack.pop() {
        match node.value() {
            Node::Element(el) => {
                // Skip script/style/noscript/head subtrees entirely
                if matches!(
                    el.name(),
                    "script" | "style" | "noscript" | "head" | "meta" | "link"
                ) {
                    continue;
                }
                // Push children in reverse order to maintain document order
                let children: Vec<_> = node.children().collect();
                for child in children.into_iter().rev() {
                    stack.push(child);
                }
            }
            Node::Text(text) => {
                let s = text.trim();
                if !s.is_empty() {
                    parts.push(s.to_string());
                }
            }
            // Document / Comment / Fragment → recurse into children
            _ => {
                let children: Vec<_> = node.children().collect();
                for child in children.into_iter().rev() {
                    stack.push(child);
                }
            }
        }
    }

    // Normalize whitespace
    parts
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use url::Url;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn parse_basic_html() {
        let html = r#"<html><head><title>Test Page</title></head>
            <body><p>Hello world</p></body></html>"#;
        let parser = HTMLParser::new();
        let doc = parser.parse(html, &url("https://example.com/")).unwrap();
        assert_eq!(doc.title, "Test Page");
        assert!(doc.text.contains("Hello world"), "text: {}", doc.text);
    }

    #[test]
    fn excludes_script_content() {
        let html = r#"<html><body>
            <p>Visible text</p>
            <script>var x = "hidden script content";</script>
            <p>More visible</p>
        </body></html>"#;
        let parser = HTMLParser::new();
        let doc = parser.parse(html, &url("https://example.com/")).unwrap();
        assert!(doc.text.contains("Visible text"));
        assert!(doc.text.contains("More visible"));
        assert!(
            !doc.text.contains("hidden script content"),
            "script leaked: {}",
            doc.text
        );
    }

    #[test]
    fn excludes_style_content() {
        let html = r#"<html><body>
            <style>.class { color: red; display: hidden-css; }</style>
            <p>Real content here</p>
        </body></html>"#;
        let parser = HTMLParser::new();
        let doc = parser.parse(html, &url("https://example.com/")).unwrap();
        assert!(doc.text.contains("Real content"));
        assert!(
            !doc.text.contains("hidden-css"),
            "style leaked: {}",
            doc.text
        );
    }

    #[test]
    fn title_case_insensitive_via_scraper() {
        // scraper normalises tags so <TITLE> becomes <title>
        let html = "<html><head><TITLE>UPPERCASE TITLE</TITLE></head><body>text</body></html>";
        let parser = HTMLParser::new();
        let doc = parser.parse(html, &url("https://example.com/")).unwrap();
        assert_eq!(doc.title, "UPPERCASE TITLE");
    }

    #[test]
    fn fallback_title_is_url() {
        let html = "<html><body>no title here</body></html>";
        let parser = HTMLParser::new();
        let u = url("https://fallback.example.com/page");
        let doc = parser.parse(html, &u).unwrap();
        assert_eq!(doc.title, u.to_string());
    }

    #[test]
    fn extracts_meta_description() {
        let html = r#"<html><head>
            <meta name="description" content="A great page about Rust">
        </head><body>content</body></html>"#;
        let parser = HTMLParser::new();
        let doc = parser.parse(html, &url("https://example.com/")).unwrap();
        assert_eq!(
            doc.metadata.description.as_deref(),
            Some("A great page about Rust")
        );
    }

    #[test]
    fn extracts_outlinks() {
        let html = r#"<html><body>
            <a href="/page1">Link 1</a>
            <a href="https://other.com/page2">Link 2</a>
            <a href="mailto:foo@bar.com">Email</a>
        </body></html>"#;
        let parser = HTMLParser::new();
        let doc = parser.parse(html, &url("https://example.com/")).unwrap();
        // Should have 2 http(s) links; mailto filtered
        assert_eq!(doc.outlinks.len(), 2, "outlinks: {:?}", doc.outlinks);
    }

    #[test]
    fn empty_html_returns_ok() {
        let parser = HTMLParser::new();
        let result = parser.parse("", &url("https://example.com/"));
        assert!(result.is_ok());
    }

    #[test]
    fn extracts_keywords() {
        let html = r#"<html><head>
            <meta name="keywords" content="rust, systems, fast">
        </head><body>text</body></html>"#;
        let parser = HTMLParser::new();
        let doc = parser.parse(html, &url("https://example.com/")).unwrap();
        assert_eq!(doc.metadata.keywords, vec!["rust", "systems", "fast"]);
    }
}
