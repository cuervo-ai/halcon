//! Search document retrieval functions.

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};

use crate::Database;

/// Document data retrieved from search_documents table.
#[derive(Debug, Clone)]
pub struct SearchDocument {
    pub id: Vec<u8>,
    pub url: String,
    pub domain: String,
    pub title: String,
    pub text: String,
    pub indexed_at: DateTime<Utc>,
    pub last_crawled: Option<DateTime<Utc>>,
    pub pagerank: f32,
    pub freshness_score: f32,
    pub outlink_count: u32,
    pub language: Option<String>,
    pub embedding: Option<Vec<u8>>,  // Serialized f32 vector (384 dims for bge-small)
}

impl Database {
    /// Retrieve document by FTS5 rowid.
    ///
    /// FTS5 table uses search_documents.rowid as content_rowid,
    /// so we can join via rowid directly.
    pub fn get_search_document_by_rowid(&self, rowid: i64) -> rusqlite::Result<SearchDocument> {
        self.with_connection(|conn| get_search_document_by_rowid_inner(conn, rowid))
    }

    /// Batch retrieve multiple documents by rowids.
    pub fn get_search_documents_by_rowids(
        &self,
        rowids: &[i64],
    ) -> rusqlite::Result<Vec<SearchDocument>> {
        self.with_connection(|conn| {
            rowids
                .iter()
                .map(|&rowid| get_search_document_by_rowid_inner(conn, rowid))
                .collect()
        })
    }
}

fn get_search_document_by_rowid_inner(
    conn: &Connection,
    rowid: i64,
) -> rusqlite::Result<SearchDocument> {
    conn.query_row(
        r#"
        SELECT
            id, url, domain, title, text, indexed_at, last_crawled,
            pagerank, freshness_score, outlink_count, language, embedding
        FROM search_documents
        WHERE rowid = ?1
        "#,
        params![rowid],
        |row| {
            let indexed_at_str: String = row.get(5)?;
            let last_crawled_str: Option<String> = row.get(6)?;

            let indexed_at = DateTime::parse_from_rfc3339(&indexed_at_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());

            let last_crawled = last_crawled_str.and_then(|s| {
                DateTime::parse_from_rfc3339(&s)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            });

            Ok(SearchDocument {
                id: row.get(0)?,
                url: row.get(1)?,
                domain: row.get(2)?,
                title: row.get(3)?,
                text: row.get(4)?,
                indexed_at,
                last_crawled,
                pagerank: row.get::<_, f64>(7)? as f32,
                freshness_score: row.get::<_, f64>(8)? as f32,
                outlink_count: row.get(9)?,
                language: row.get(10)?,
                embedding: row.get(11)?,
            })
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_document_by_rowid() {
        let db = Database::open_in_memory().unwrap();

        // Insert test document
        db.with_connection(|conn| {
            conn.execute(
                r#"
                INSERT INTO search_documents
                (id, url, domain, title, text, indexed_at, pagerank, freshness_score, outlink_count)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                "#,
                params![
                    &[1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
                    "https://example.com/test",
                    "example.com",
                    "Test Document",
                    "This is test content for BM25 search.",
                    "2026-02-17T10:00:00Z",
                    0.5,
                    1.0,
                    3
                ],
            )
        })
        .unwrap();

        // Get rowid
        let rowid: i64 = db
            .with_connection(|conn| {
                conn.query_row("SELECT rowid FROM search_documents LIMIT 1", [], |row| {
                    row.get(0)
                })
            })
            .unwrap();

        // Retrieve document
        let doc = db.get_search_document_by_rowid(rowid).unwrap();

        assert_eq!(doc.url, "https://example.com/test");
        assert_eq!(doc.domain, "example.com");
        assert_eq!(doc.title, "Test Document");
        assert_eq!(doc.text, "This is test content for BM25 search.");
        assert_eq!(doc.pagerank, 0.5);
        assert_eq!(doc.freshness_score, 1.0);
        assert_eq!(doc.outlink_count, 3);
    }

    #[test]
    fn test_batch_retrieve() {
        let db = Database::open_in_memory().unwrap();

        // Insert 3 test documents
        for i in 1u8..=3 {
            db.with_connection(|conn| {
                conn.execute(
                    r#"
                    INSERT INTO search_documents
                    (id, url, domain, title, text, indexed_at, pagerank, freshness_score, outlink_count)
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                    "#,
                    params![
                        &[i, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                        format!("https://example.com/doc{}", i),
                        "example.com",
                        format!("Document {}", i),
                        format!("Content for document {}", i),
                        "2026-02-17T10:00:00Z",
                        0.0,
                        1.0,
                        0
                    ],
                )
            })
            .unwrap();
        }

        // Get all rowids
        let rowids: Vec<i64> = db
            .with_connection(|conn| {
                let mut stmt = conn.prepare("SELECT rowid FROM search_documents").unwrap();
                let rows = stmt
                    .query_map([], |row| row.get(0))
                    .unwrap()
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap();
                Ok::<_, rusqlite::Error>(rows)
            })
            .unwrap();

        assert_eq!(rowids.len(), 3);

        // Batch retrieve
        let docs = db.get_search_documents_by_rowids(&rowids).unwrap();
        assert_eq!(docs.len(), 3);

        assert_eq!(docs[0].title, "Document 1");
        assert_eq!(docs[1].title, "Document 2");
        assert_eq!(docs[2].title, "Document 3");
    }

    #[test]
    fn test_document_not_found() {
        let db = Database::open_in_memory().unwrap();

        // Attempt to retrieve non-existent rowid
        let result = db.get_search_document_by_rowid(9999);
        assert!(result.is_err());
    }

    #[test]
    fn test_datetime_parsing() {
        let db = Database::open_in_memory().unwrap();

        // Insert document with last_crawled timestamp
        db.with_connection(|conn| {
            conn.execute(
                r#"
                INSERT INTO search_documents
                (id, url, domain, title, text, indexed_at, last_crawled, pagerank, freshness_score, outlink_count)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                "#,
                params![
                    &[99u8; 16],
                    "https://example.com/crawled",
                    "example.com",
                    "Crawled Document",
                    "Crawled content",
                    "2026-02-17T10:00:00Z",
                    "2026-02-17T12:00:00Z",
                    0.0,
                    1.0,
                    0
                ],
            )
        })
        .unwrap();

        let rowid: i64 = db
            .with_connection(|conn| {
                conn.query_row("SELECT rowid FROM search_documents LIMIT 1", [], |row| {
                    row.get(0)
                })
            })
            .unwrap();

        let doc = db.get_search_document_by_rowid(rowid).unwrap();

        assert!(doc.last_crawled.is_some());
        let crawled = doc.last_crawled.unwrap();
        assert_eq!(crawled.to_rfc3339(), "2026-02-17T12:00:00+00:00");
    }

    #[test]
    fn test_language_field() {
        let db = Database::open_in_memory().unwrap();

        db.with_connection(|conn| {
            conn.execute(
                r#"
                INSERT INTO search_documents
                (id, url, domain, title, text, indexed_at, pagerank, freshness_score, outlink_count, language)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                "#,
                params![
                    &[88u8; 16],
                    "https://example.com/es",
                    "example.com",
                    "Spanish Document",
                    "Contenido en español",
                    "2026-02-17T10:00:00Z",
                    0.0,
                    1.0,
                    0,
                    "es"
                ],
            )
        })
        .unwrap();

        let rowid: i64 = db
            .with_connection(|conn| {
                conn.query_row("SELECT rowid FROM search_documents LIMIT 1", [], |row| {
                    row.get(0)
                })
            })
            .unwrap();

        let doc = db.get_search_document_by_rowid(rowid).unwrap();
        assert_eq!(doc.language, Some("es".to_string()));
    }
}
