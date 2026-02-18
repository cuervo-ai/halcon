//! Snippet generation (KWIC - Keyword In Context).

pub struct Snippeter {
    max_length: usize,
}

impl Snippeter {
    pub fn new(max_length: usize) -> Self {
        Self { max_length }
    }

    /// Generate snippet around query terms.
    ///
    /// TODO: Implement KWIC algorithm.
    pub fn generate(&self, _text: &str, _query_terms: &[String]) -> String {
        // Stub implementation
        String::from("...")
    }
}
