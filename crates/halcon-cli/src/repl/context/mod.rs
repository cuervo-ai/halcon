// context/ — context sources, memory retrieval, governance
// MIGRATION-2026: archivos movidos desde repl/ raíz

pub mod compaction;
pub mod consolidator;
pub mod episodic;
pub(crate) mod governance;
pub mod hybrid_retriever;
pub(crate) mod manager;
pub mod memory;
pub(crate) mod metrics;
pub mod reflection;
pub mod repo_map;
pub mod vector_memory;

// Re-exports — preserve API surface for callers in repl/
// consolidator: free functions (consolidate, maybe_consolidate), no MemoryConsolidator struct
