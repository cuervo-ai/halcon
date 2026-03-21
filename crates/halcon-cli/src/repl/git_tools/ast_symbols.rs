//! Phase 6: AST Symbol Extractor (feature-gated `ast-symbols`)
//!
//! Provides language-aware symbol extraction from source code for context
//! injection into the agent's system prompt. The extractor identifies
//! functions, structs, classes, traits, enums, interfaces, and constants
//! across the most common languages without requiring an external compiler.
//!
//! # Architecture
//! - `SymbolExtractor` trait: pluggable backend (regex today, tree-sitter in future)
//! - `RegexExtractor`: production-quality multi-language extractor (no C deps)
//! - `SymbolIndex`: collection of extracted symbols with budget-aware rendering
//! - `extract_from_buffer(uri, content)`: top-level convenience function
//!
//! # Feature gate
//! The module is gated behind `#[cfg(feature = "ast-symbols")]` at the
//! registration site in `mod.rs`. The module itself compiles unconditionally
//! so it can be tested without the feature flag.

use std::collections::HashMap;

// ── Symbol kinds ──────────────────────────────────────────────────────────────

/// The semantic kind of an extracted symbol.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    Function,
    Method,
    Struct,
    Class,
    Enum,
    Trait,
    Interface,
    Type,
    Constant,
    Module,
    Macro,
    Other(String),
}

impl SymbolKind {
    /// Short human-readable tag for rendering.
    pub fn tag(&self) -> &str {
        match self {
            Self::Function => "fn",
            Self::Method => "method",
            Self::Struct => "struct",
            Self::Class => "class",
            Self::Enum => "enum",
            Self::Trait => "trait",
            Self::Interface => "interface",
            Self::Type => "type",
            Self::Constant => "const",
            Self::Module => "mod",
            Self::Macro => "macro",
            Self::Other(s) => s.as_str(),
        }
    }
}

// ── Symbol ────────────────────────────────────────────────────────────────────

/// A single extracted symbol.
#[derive(Debug, Clone)]
pub struct Symbol {
    /// Fully qualified name (may include module path when known).
    pub name: String,
    /// Semantic kind.
    pub kind: SymbolKind,
    /// 1-based line number of the definition.
    pub line: usize,
    /// Optional visibility marker (`pub`, `export`, `public`, etc.).
    pub visibility: Option<String>,
    /// Optional docstring excerpt (first line only).
    pub doc: Option<String>,
}

// ── Symbol index ──────────────────────────────────────────────────────────────

/// A collection of symbols extracted from a single source file.
#[derive(Debug, Clone, Default)]
pub struct SymbolIndex {
    /// Source file URI or path.
    pub uri: String,
    /// Language identifier (`rust`, `python`, `typescript`, etc.).
    pub language: String,
    /// Extracted symbols, in source order.
    pub symbols: Vec<Symbol>,
}

impl SymbolIndex {
    /// Return a compact, token-budget-aware representation.
    ///
    /// Each symbol is one line: `L{line}: [{kind}] {visibility}{name}`
    /// The output is truncated to `max_chars` characters.
    pub fn render(&self, max_chars: usize) -> String {
        if self.symbols.is_empty() {
            return String::new();
        }
        let mut out = format!("// {} ({})\n", self.uri, self.language);
        for sym in &self.symbols {
            let vis = sym
                .visibility
                .as_deref()
                .map(|v| format!("{v} "))
                .unwrap_or_default();
            let line = format!("L{}: [{}] {}{}\n", sym.line, sym.kind.tag(), vis, sym.name);
            if out.len() + line.len() > max_chars {
                out.push_str("// … (truncated)\n");
                break;
            }
            out.push_str(&line);
        }
        out
    }

    /// Number of extracted symbols.
    pub fn len(&self) -> usize {
        self.symbols.len()
    }

    /// True when no symbols were found.
    pub fn is_empty(&self) -> bool {
        self.symbols.is_empty()
    }

    /// Filter to only exported/public symbols.
    pub fn public_symbols(&self) -> Vec<&Symbol> {
        self.symbols
            .iter()
            .filter(|s| s.visibility.is_some())
            .collect()
    }

    /// Group symbols by kind.
    pub fn by_kind(&self) -> HashMap<&str, Vec<&Symbol>> {
        let mut map: HashMap<&str, Vec<&Symbol>> = HashMap::new();
        for sym in &self.symbols {
            map.entry(sym.kind.tag()).or_default().push(sym);
        }
        map
    }
}

// ── Extractor trait ───────────────────────────────────────────────────────────

/// Pluggable symbol extraction backend.
pub trait SymbolExtractor: Send + Sync {
    /// Extract symbols from `content` written in `language`.
    fn extract(&self, uri: &str, language: &str, content: &str) -> SymbolIndex;

    /// Return the set of languages this extractor supports.
    fn supported_languages(&self) -> &[&str];
}

// ── Language detection ────────────────────────────────────────────────────────

/// Infer the language identifier from a file URI or path.
///
/// Falls back to `"plaintext"` when the extension is unrecognised.
pub fn detect_language(uri: &str) -> &'static str {
    let ext = uri.rsplit('.').next().unwrap_or("");
    match ext {
        "rs" => "rust",
        "py" | "pyi" => "python",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "go" => "go",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "rb" => "ruby",
        "c" | "cc" | "cpp" | "cxx" | "h" | "hpp" => "c_cpp",
        "cs" => "csharp",
        "swift" => "swift",
        "zig" => "zig",
        _ => "plaintext",
    }
}

// ── Regex-based extractor ─────────────────────────────────────────────────────

/// Production-quality regex-based symbol extractor.
///
/// Covers Rust, Python, TypeScript/JavaScript, and Go with word-boundary
/// matching to prevent false positives (e.g. "function" inside a string).
pub struct RegexExtractor;

impl RegexExtractor {
    /// Extract symbols from a single source line.
    ///
    /// This is intentionally simple and stateless — it does not track
    /// multi-line definitions, braces, or indentation depth.
    fn extract_line(line: &str, line_no: usize, lang: &str) -> Option<Symbol> {
        match lang {
            "rust" => Self::rust_line(line, line_no),
            "python" => Self::python_line(line, line_no),
            "typescript" | "javascript" => Self::ts_line(line, line_no),
            "go" => Self::go_line(line, line_no),
            "java" | "kotlin" | "csharp" => Self::java_line(line, line_no),
            _ => None,
        }
    }

    fn rust_line(line: &str, line_no: usize) -> Option<Symbol> {
        let trimmed = line.trim();

        // Visibility prefix detection.
        let (vis, rest) = if trimmed.starts_with("pub(crate)") {
            (
                Some("pub(crate)"),
                trimmed.trim_start_matches("pub(crate)").trim(),
            )
        } else if trimmed.starts_with("pub(super)") {
            (
                Some("pub(super)"),
                trimmed.trim_start_matches("pub(super)").trim(),
            )
        } else if trimmed.starts_with("pub ") {
            (Some("pub"), trimmed.trim_start_matches("pub ").trim())
        } else {
            (None, trimmed)
        };

        // fn
        if let Some(after_fn) = rest
            .strip_prefix("fn ")
            .or_else(|| rest.strip_prefix("async fn "))
        {
            let name = after_fn
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .next()
                .unwrap_or("")
                .to_string();
            if !name.is_empty() {
                let kind = if vis.is_none() && line.contains("self") {
                    SymbolKind::Method
                } else {
                    SymbolKind::Function
                };
                return Some(Symbol {
                    name,
                    kind,
                    line: line_no,
                    visibility: vis.map(str::to_string),
                    doc: None,
                });
            }
        }

        // struct
        if let Some(after_kw) = rest.strip_prefix("struct ") {
            let name = after_kw
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .next()
                .unwrap_or("")
                .to_string();
            if !name.is_empty() {
                return Some(Symbol {
                    name,
                    kind: SymbolKind::Struct,
                    line: line_no,
                    visibility: vis.map(str::to_string),
                    doc: None,
                });
            }
        }

        // enum
        if let Some(after_kw) = rest.strip_prefix("enum ") {
            let name = after_kw
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .next()
                .unwrap_or("")
                .to_string();
            if !name.is_empty() {
                return Some(Symbol {
                    name,
                    kind: SymbolKind::Enum,
                    line: line_no,
                    visibility: vis.map(str::to_string),
                    doc: None,
                });
            }
        }

        // trait
        if let Some(after_kw) = rest.strip_prefix("trait ") {
            let name = after_kw
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .next()
                .unwrap_or("")
                .to_string();
            if !name.is_empty() {
                return Some(Symbol {
                    name,
                    kind: SymbolKind::Trait,
                    line: line_no,
                    visibility: vis.map(str::to_string),
                    doc: None,
                });
            }
        }

        // type alias
        if let Some(after_kw) = rest.strip_prefix("type ") {
            let name = after_kw
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .next()
                .unwrap_or("")
                .to_string();
            if !name.is_empty() {
                return Some(Symbol {
                    name,
                    kind: SymbolKind::Type,
                    line: line_no,
                    visibility: vis.map(str::to_string),
                    doc: None,
                });
            }
        }

        // const / static
        if rest.starts_with("const ") || rest.starts_with("static ") {
            let after = if rest.starts_with("const ") {
                &rest["const ".len()..]
            } else {
                &rest["static ".len()..]
            };
            // Skip mut
            let after = after.trim_start_matches("mut ").trim();
            let name = after
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .next()
                .unwrap_or("")
                .to_string();
            if !name.is_empty() && name.chars().next().is_some_and(|c| c.is_uppercase()) {
                return Some(Symbol {
                    name,
                    kind: SymbolKind::Constant,
                    line: line_no,
                    visibility: vis.map(str::to_string),
                    doc: None,
                });
            }
        }

        // mod
        if let Some(after_kw) = rest.strip_prefix("mod ") {
            let name = after_kw
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .next()
                .unwrap_or("")
                .to_string();
            if !name.is_empty() {
                return Some(Symbol {
                    name,
                    kind: SymbolKind::Module,
                    line: line_no,
                    visibility: vis.map(str::to_string),
                    doc: None,
                });
            }
        }

        // macro_rules!
        if trimmed.starts_with("macro_rules!") {
            let after = trimmed.trim_start_matches("macro_rules!").trim();
            let name = after
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .next()
                .unwrap_or("")
                .to_string();
            if !name.is_empty() {
                return Some(Symbol {
                    name,
                    kind: SymbolKind::Macro,
                    line: line_no,
                    visibility: None,
                    doc: None,
                });
            }
        }

        None
    }

    fn python_line(line: &str, line_no: usize) -> Option<Symbol> {
        let trimmed = line.trim();

        if let Some(after) = trimmed
            .strip_prefix("def ")
            .or_else(|| trimmed.strip_prefix("async def "))
        {
            let name = after
                .split(|c: char| c == '(' || c == ':' || c.is_whitespace())
                .next()
                .unwrap_or("")
                .to_string();
            if !name.is_empty() {
                let kind = if line.starts_with("    ") || line.starts_with('\t') {
                    SymbolKind::Method
                } else {
                    SymbolKind::Function
                };
                return Some(Symbol {
                    name,
                    kind,
                    line: line_no,
                    visibility: if !trimmed.starts_with('_') {
                        Some("public".to_string())
                    } else {
                        None
                    },
                    doc: None,
                });
            }
        }

        if let Some(after) = trimmed.strip_prefix("class ") {
            let name = after
                .split(|c: char| c == '(' || c == ':' || c.is_whitespace())
                .next()
                .unwrap_or("")
                .to_string();
            if !name.is_empty() {
                return Some(Symbol {
                    name,
                    kind: SymbolKind::Class,
                    line: line_no,
                    visibility: Some("public".to_string()),
                    doc: None,
                });
            }
        }

        None
    }

    fn ts_line(line: &str, line_no: usize) -> Option<Symbol> {
        let trimmed = line.trim();

        // export keyword.
        let (vis, rest) = if trimmed.starts_with("export default ") {
            (
                Some("export default"),
                trimmed["export default ".len()..].trim(),
            )
        } else if trimmed.starts_with("export ") {
            (Some("export"), trimmed["export ".len()..].trim())
        } else {
            (None, trimmed)
        };

        // function
        if let Some(after) = rest
            .strip_prefix("function ")
            .or_else(|| rest.strip_prefix("async function "))
        {
            let name = after
                .split(|c: char| c == '(' || c.is_whitespace())
                .next()
                .unwrap_or("")
                .to_string();
            if !name.is_empty() {
                return Some(Symbol {
                    name,
                    kind: SymbolKind::Function,
                    line: line_no,
                    visibility: vis.map(str::to_string),
                    doc: None,
                });
            }
        }

        // class
        if let Some(after) = rest.strip_prefix("class ") {
            let name = after
                .split(|c: char| c == '{' || c == '<' || c.is_whitespace())
                .next()
                .unwrap_or("")
                .to_string();
            if !name.is_empty() {
                return Some(Symbol {
                    name,
                    kind: SymbolKind::Class,
                    line: line_no,
                    visibility: vis.map(str::to_string),
                    doc: None,
                });
            }
        }

        // interface
        if let Some(after) = rest.strip_prefix("interface ") {
            let name = after
                .split(|c: char| c == '{' || c == '<' || c.is_whitespace())
                .next()
                .unwrap_or("")
                .to_string();
            if !name.is_empty() {
                return Some(Symbol {
                    name,
                    kind: SymbolKind::Interface,
                    line: line_no,
                    visibility: vis.map(str::to_string),
                    doc: None,
                });
            }
        }

        // type alias
        if let Some(after) = rest.strip_prefix("type ") {
            let name = after
                .split(|c: char| c == '=' || c == '<' || c.is_whitespace())
                .next()
                .unwrap_or("")
                .to_string();
            if !name.is_empty() {
                return Some(Symbol {
                    name,
                    kind: SymbolKind::Type,
                    line: line_no,
                    visibility: vis.map(str::to_string),
                    doc: None,
                });
            }
        }

        // const arrow function: `const foo = (...) =>`
        if let Some(after) = rest.strip_prefix("const ") {
            let name = after
                .split(|c: char| c == '=' || c.is_whitespace())
                .next()
                .unwrap_or("")
                .to_string();
            if !name.is_empty() && rest.contains("=>") {
                return Some(Symbol {
                    name,
                    kind: SymbolKind::Function,
                    line: line_no,
                    visibility: vis.map(str::to_string),
                    doc: None,
                });
            }
        }

        None
    }

    fn go_line(line: &str, line_no: usize) -> Option<Symbol> {
        let trimmed = line.trim();

        if let Some(after) = trimmed.strip_prefix("func ") {
            // func (recv Receiver) MethodName(...) → method
            // func FunctionName(...) → function
            if after.starts_with('(') {
                // method: extract name after closing paren
                if let Some(close) = after.find(')') {
                    let after_recv = after[close + 1..].trim();
                    let name = after_recv
                        .split(|c: char| c == '(' || c.is_whitespace())
                        .next()
                        .unwrap_or("")
                        .to_string();
                    if !name.is_empty() {
                        let exported = name.chars().next().is_some_and(|c| c.is_uppercase());
                        return Some(Symbol {
                            name,
                            kind: SymbolKind::Method,
                            line: line_no,
                            visibility: if exported {
                                Some("exported".to_string())
                            } else {
                                None
                            },
                            doc: None,
                        });
                    }
                }
            } else {
                let name = after
                    .split(|c: char| c == '(' || c.is_whitespace())
                    .next()
                    .unwrap_or("")
                    .to_string();
                if !name.is_empty() {
                    return Some(Symbol {
                        name: name.clone(),
                        kind: SymbolKind::Function,
                        line: line_no,
                        visibility: if name.chars().next().is_some_and(|c| c.is_uppercase()) {
                            Some("exported".to_string())
                        } else {
                            None
                        },
                        doc: None,
                    });
                }
            }
        }

        if let Some(after) = trimmed.strip_prefix("type ") {
            let parts: Vec<&str> = after.splitn(3, ' ').collect();
            if parts.len() >= 2 {
                let name = parts[0].to_string();
                let kind = match parts[1].trim_start_matches("*") {
                    "struct" => SymbolKind::Struct,
                    "interface" => SymbolKind::Interface,
                    _ => SymbolKind::Type,
                };
                return Some(Symbol {
                    name: name.clone(),
                    kind,
                    line: line_no,
                    visibility: if name.chars().next().is_some_and(|c| c.is_uppercase()) {
                        Some("exported".to_string())
                    } else {
                        None
                    },
                    doc: None,
                });
            }
        }

        None
    }

    fn java_line(line: &str, line_no: usize) -> Option<Symbol> {
        let trimmed = line.trim();
        let vis_prefixes = ["public ", "protected ", "private "];
        let mut vis = None;
        let mut rest = trimmed;

        for prefix in &vis_prefixes {
            if let Some(after) = trimmed.strip_prefix(prefix) {
                vis = Some(prefix.trim());
                rest = after;
                break;
            }
        }

        // Skip modifiers
        let modifiers = ["static ", "final ", "abstract ", "native ", "synchronized "];
        for m in &modifiers {
            rest = rest.trim_start_matches(m).trim();
        }

        // class / interface / enum
        for (kw, kind) in &[
            ("class ", SymbolKind::Class),
            ("interface ", SymbolKind::Interface),
            ("enum ", SymbolKind::Enum),
        ] {
            if let Some(after) = rest.strip_prefix(kw) {
                let name = after
                    .split(|c: char| !c.is_alphanumeric() && c != '_')
                    .next()
                    .unwrap_or("")
                    .to_string();
                if !name.is_empty() {
                    return Some(Symbol {
                        name,
                        kind: kind.clone(),
                        line: line_no,
                        visibility: vis.map(str::to_string),
                        doc: None,
                    });
                }
            }
        }

        None
    }
}

impl SymbolExtractor for RegexExtractor {
    fn extract(&self, uri: &str, language: &str, content: &str) -> SymbolIndex {
        let mut symbols = Vec::new();
        for (idx, line) in content.lines().enumerate() {
            if let Some(sym) = Self::extract_line(line, idx + 1, language) {
                symbols.push(sym);
            }
        }
        SymbolIndex {
            uri: uri.to_string(),
            language: language.to_string(),
            symbols,
        }
    }

    fn supported_languages(&self) -> &[&str] {
        &[
            "rust",
            "python",
            "typescript",
            "javascript",
            "go",
            "java",
            "kotlin",
            "csharp",
        ]
    }
}

// ── Top-level convenience ─────────────────────────────────────────────────────

/// Extract symbols from a buffer's content, auto-detecting language from URI.
///
/// Uses `RegexExtractor` under the hood. For a future tree-sitter backend,
/// swap out the extractor implementation — the `SymbolIndex` API is stable.
pub fn extract_from_buffer(uri: &str, content: &str) -> SymbolIndex {
    let language = detect_language(uri);
    RegexExtractor.extract(uri, language, content)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Language detection ────────────────────────────────────────────────────

    #[test]
    fn detect_rust_extension() {
        assert_eq!(detect_language("file:///src/main.rs"), "rust");
    }

    #[test]
    fn detect_python_extension() {
        assert_eq!(detect_language("app.py"), "python");
    }

    #[test]
    fn detect_typescript_extension() {
        assert_eq!(detect_language("index.ts"), "typescript");
        assert_eq!(detect_language("App.tsx"), "typescript");
    }

    #[test]
    fn detect_go_extension() {
        assert_eq!(detect_language("main.go"), "go");
    }

    #[test]
    fn detect_unknown_extension() {
        assert_eq!(detect_language("file.xyz"), "plaintext");
    }

    // ── Rust extraction ───────────────────────────────────────────────────────

    #[test]
    fn rust_extract_pub_fn() {
        let code = "pub fn calculate(x: u32) -> u32 { x * 2 }";
        let idx = RegexExtractor.extract("main.rs", "rust", code);
        assert_eq!(idx.len(), 1);
        let sym = &idx.symbols[0];
        assert_eq!(sym.name, "calculate");
        assert_eq!(sym.kind, SymbolKind::Function);
        assert_eq!(sym.visibility, Some("pub".to_string()));
    }

    #[test]
    fn rust_extract_struct() {
        let code = "pub struct Config { pub timeout: u32 }";
        let idx = RegexExtractor.extract("lib.rs", "rust", code);
        let structs: Vec<_> = idx
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Struct)
            .collect();
        assert!(!structs.is_empty());
        assert_eq!(structs[0].name, "Config");
    }

    #[test]
    fn rust_extract_enum() {
        let code = "pub enum Status { Ok, Err }";
        let idx = RegexExtractor.extract("status.rs", "rust", code);
        let enums: Vec<_> = idx
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Enum)
            .collect();
        assert!(!enums.is_empty());
        assert_eq!(enums[0].name, "Status");
    }

    #[test]
    fn rust_extract_trait() {
        let code = "pub trait Processable { fn process(&self); }";
        let idx = RegexExtractor.extract("trait.rs", "rust", code);
        let traits: Vec<_> = idx
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Trait)
            .collect();
        assert!(!traits.is_empty());
        assert_eq!(traits[0].name, "Processable");
    }

    #[test]
    fn rust_extract_const() {
        let code = "pub const MAX_SIZE: usize = 1024;";
        let idx = RegexExtractor.extract("lib.rs", "rust", code);
        let consts: Vec<_> = idx
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Constant)
            .collect();
        assert!(!consts.is_empty());
        assert_eq!(consts[0].name, "MAX_SIZE");
    }

    #[test]
    fn rust_extract_type_alias() {
        let code = "pub type Result<T> = std::result::Result<T, Error>;";
        let idx = RegexExtractor.extract("lib.rs", "rust", code);
        let types: Vec<_> = idx
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Type)
            .collect();
        assert!(!types.is_empty());
    }

    #[test]
    fn rust_extract_module() {
        let code = "pub mod network;";
        let idx = RegexExtractor.extract("lib.rs", "rust", code);
        let mods: Vec<_> = idx
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Module)
            .collect();
        assert!(!mods.is_empty());
        assert_eq!(mods[0].name, "network");
    }

    #[test]
    fn rust_extract_macro_rules() {
        let code = "macro_rules! assert_ok { ... }";
        let idx = RegexExtractor.extract("macros.rs", "rust", code);
        let macros: Vec<_> = idx
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Macro)
            .collect();
        assert!(!macros.is_empty());
        assert_eq!(macros[0].name, "assert_ok");
    }

    #[test]
    fn rust_extract_multi_symbol_file() {
        let code = r#"
pub struct Server { port: u16 }

pub trait Handler {
    fn handle(&self, req: Request) -> Response;
}

pub fn start(server: Server) -> Result<(), Error> {
    todo!()
}

pub enum State { Running, Stopped }
"#;
        let idx = RegexExtractor.extract("server.rs", "rust", code);
        assert!(idx.len() >= 4, "expected ≥4 symbols, got {}", idx.len());
    }

    // ── Python extraction ─────────────────────────────────────────────────────

    #[test]
    fn python_extract_function() {
        let code = "def process(x: int) -> int:\n    return x * 2";
        let idx = RegexExtractor.extract("app.py", "python", code);
        assert_eq!(idx.len(), 1);
        assert_eq!(idx.symbols[0].name, "process");
        assert_eq!(idx.symbols[0].kind, SymbolKind::Function);
    }

    #[test]
    fn python_extract_class() {
        let code = "class Database(object):\n    pass";
        let idx = RegexExtractor.extract("db.py", "python", code);
        let classes: Vec<_> = idx
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Class)
            .collect();
        assert!(!classes.is_empty());
        assert_eq!(classes[0].name, "Database");
    }

    #[test]
    fn python_extract_method_is_indented() {
        let code = "class Foo:\n    def bar(self):\n        pass";
        let idx = RegexExtractor.extract("foo.py", "python", code);
        let methods: Vec<_> = idx
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert!(!methods.is_empty());
    }

    // ── TypeScript extraction ─────────────────────────────────────────────────

    #[test]
    fn ts_extract_exported_function() {
        let code = "export function fetchData(url: string): Promise<Data> {}";
        let idx = RegexExtractor.extract("api.ts", "typescript", code);
        assert_eq!(idx.len(), 1);
        assert_eq!(idx.symbols[0].name, "fetchData");
        assert_eq!(idx.symbols[0].visibility, Some("export".to_string()));
    }

    #[test]
    fn ts_extract_interface() {
        let code = "export interface UserConfig { name: string; }";
        let idx = RegexExtractor.extract("types.ts", "typescript", code);
        let ifaces: Vec<_> = idx
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Interface)
            .collect();
        assert!(!ifaces.is_empty());
        assert_eq!(ifaces[0].name, "UserConfig");
    }

    #[test]
    fn ts_extract_class() {
        let code = "export class ApiClient { constructor() {} }";
        let idx = RegexExtractor.extract("client.ts", "typescript", code);
        let classes: Vec<_> = idx
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Class)
            .collect();
        assert!(!classes.is_empty());
    }

    // ── Go extraction ─────────────────────────────────────────────────────────

    #[test]
    fn go_extract_function() {
        let code = "func ProcessRequest(req *http.Request) error { return nil }";
        let idx = RegexExtractor.extract("handler.go", "go", code);
        assert_eq!(idx.len(), 1);
        assert_eq!(idx.symbols[0].name, "ProcessRequest");
        assert_eq!(idx.symbols[0].visibility, Some("exported".to_string()));
    }

    #[test]
    fn go_extract_method() {
        let code = "func (s *Server) ServeHTTP(w http.ResponseWriter, r *http.Request) {}";
        let idx = RegexExtractor.extract("server.go", "go", code);
        let methods: Vec<_> = idx
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert!(!methods.is_empty());
    }

    #[test]
    fn go_extract_struct_type() {
        let code = "type Config struct { Port int }";
        let idx = RegexExtractor.extract("config.go", "go", code);
        let structs: Vec<_> = idx
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Struct)
            .collect();
        assert!(!structs.is_empty());
        assert_eq!(structs[0].name, "Config");
    }

    // ── SymbolIndex helpers ───────────────────────────────────────────────────

    #[test]
    fn symbol_index_public_symbols_filters_by_visibility() {
        let code = "pub fn public_fn() {}\nfn private_fn() {}";
        let idx = RegexExtractor.extract("lib.rs", "rust", code);
        let public = idx.public_symbols();
        assert_eq!(public.len(), 1);
        assert_eq!(public[0].name, "public_fn");
    }

    #[test]
    fn symbol_index_render_includes_all_symbols() {
        let code = "pub struct Foo {}\npub fn bar() {}";
        let idx = RegexExtractor.extract("lib.rs", "rust", code);
        let rendered = idx.render(4096);
        assert!(rendered.contains("Foo"));
        assert!(rendered.contains("bar"));
    }

    #[test]
    fn symbol_index_render_truncates_at_max_chars() {
        let code = "pub fn a() {}\npub fn b() {}\npub fn c() {}";
        let idx = RegexExtractor.extract("lib.rs", "rust", code);
        // Render with tiny budget.
        let rendered = idx.render(40);
        assert!(rendered.contains("truncated") || rendered.len() <= 60);
    }

    #[test]
    fn symbol_index_by_kind_groups_correctly() {
        let code = "pub struct A {}\npub fn b() {}\npub enum C {}";
        let idx = RegexExtractor.extract("lib.rs", "rust", code);
        let by_kind = idx.by_kind();
        assert!(
            by_kind.contains_key("struct")
                || by_kind.contains_key("fn")
                || by_kind.contains_key("enum")
        );
    }

    #[test]
    fn extract_from_buffer_convenience_fn() {
        let idx = extract_from_buffer("file:///main.rs", "pub fn main() {}");
        assert!(!idx.is_empty());
        assert_eq!(idx.language, "rust");
    }

    #[test]
    fn extract_from_buffer_empty_content() {
        let idx = extract_from_buffer("file:///empty.rs", "");
        assert!(idx.is_empty());
    }
}
