//! Dynamic code instrumentation for runtime debugging.
//!
//! Injects print statements into source code at specific lines to capture
//! variable values at failure points. Automatically reverts changes after execution.
//!
//! Pattern inspired by InspectCoder: insert logging → execute → extract values → revert.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Instrumentation request: inject print at specific location for variables.
#[derive(Debug, Clone)]
pub struct InstrumentationRequest {
    /// File to instrument.
    pub file: PathBuf,
    /// Line number to insert print statement before (1-indexed).
    pub line: u32,
    /// Variable names to inspect.
    pub variables: Vec<String>,
    /// Language for syntax-appropriate print generation.
    pub language: Language,
}

/// Supported languages for instrumentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Python,
    Rust,
    JavaScript,
    TypeScript,
}

impl Language {
    /// Auto-detect from file extension.
    pub fn from_path(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()? {
            "py" => Some(Language::Python),
            "rs" => Some(Language::Rust),
            "js" => Some(Language::JavaScript),
            "ts" | "tsx" => Some(Language::TypeScript),
            _ => None,
        }
    }

    /// Generate print statement for this language.
    fn generate_print(&self, variables: &[String]) -> String {
        match self {
            Language::Python => {
                // f-string: print(f"[INSPECT] x={x}, y={y}")
                let var_list = variables
                    .iter()
                    .map(|v| format!("{v}={{{v}}}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("    print(f\"[INSPECT] {var_list}\")")
            }
            Language::Rust => {
                // println!: println!("[INSPECT] x={:?}, y={:?}", x, y);
                let var_list = variables
                    .iter()
                    .map(|_| "{:?}")
                    .collect::<Vec<_>>()
                    .join(", ");
                let var_args = variables.join(", ");
                format!("    println!(\"[INSPECT] {var_list}\", {var_args});")
            }
            Language::JavaScript | Language::TypeScript => {
                // console.log: console.log("[INSPECT]", {x, y});
                let obj = variables
                    .iter()
                    .map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("    console.log(\"[INSPECT]\", {{{obj}}});")
            }
        }
    }
}

/// Result of code instrumentation.
pub struct InstrumentedCode {
    /// Temporary directory holding instrumented file (auto-cleanup on drop).
    pub _temp_dir: TempDir,
    /// Path to instrumented file.
    pub instrumented_path: PathBuf,
    /// Original file content for revert.
    pub original_content: String,
}

impl InstrumentedCode {
    /// Revert instrumentation by writing original content back.
    pub fn revert(&self, original_file: &Path) -> Result<()> {
        std::fs::write(original_file, &self.original_content)
            .context("Failed to revert instrumentation")
    }
}

/// Instrument source file with print statements.
///
/// Strategy:
/// 1. Read original file
/// 2. Insert print statement at target line
/// 3. Write to temp file
/// 4. Return instrumented path for execution
pub fn instrument_code(request: &InstrumentationRequest) -> Result<InstrumentedCode> {
    let original_content = std::fs::read_to_string(&request.file)
        .with_context(|| format!("Failed to read {}", request.file.display()))?;

    let lines: Vec<&str> = original_content.lines().collect();

    if request.line == 0 || request.line as usize > lines.len() {
        anyhow::bail!(
            "Invalid line number {} for file with {} lines",
            request.line,
            lines.len()
        );
    }

    // Generate print statement for language
    let print_stmt = request.language.generate_print(&request.variables);

    // Insert print before target line (1-indexed → 0-indexed)
    let target_idx = (request.line - 1) as usize;
    let mut instrumented_lines = Vec::new();

    for (i, line) in lines.iter().enumerate() {
        if i == target_idx {
            // Insert print with matching indentation
            let indent = detect_indentation(line);
            instrumented_lines.push(format!("{}{}", indent, print_stmt.trim()));
        }
        instrumented_lines.push((*line).to_string());
    }

    let instrumented_content = instrumented_lines.join("\n");

    // Write to temp file (same directory for relative imports to work)
    let temp_dir = tempfile::tempdir_in(request.file.parent().unwrap_or(Path::new(".")))?;
    let temp_file = temp_dir.path().join(
        request
            .file
            .file_name()
            .context("Invalid file name")?,
    );
    std::fs::write(&temp_file, &instrumented_content)?;

    Ok(InstrumentedCode {
        _temp_dir: temp_dir,
        instrumented_path: temp_file,
        original_content,
    })
}

/// Extract variable values from execution output.
///
/// Looks for lines matching "[INSPECT] x=3, y=5" pattern.
pub fn extract_inspect_values(output: &str) -> Vec<(String, String)> {
    let mut values = Vec::new();

    for line in output.lines() {
        if let Some(rest) = line.strip_prefix("[INSPECT]") {
            // Parse "x=3, y=5" or "{x: 3, y: 5}"
            let trimmed = rest.trim().trim_matches(|c| c == '{' || c == '}');

            for pair in trimmed.split(',') {
                let parts: Vec<&str> = pair.split('=').collect();
                if parts.len() == 2 {
                    values.push((
                        parts[0].trim().to_string(),
                        parts[1].trim().to_string(),
                    ));
                } else if pair.contains(':') {
                    // JS object syntax: "x: 3"
                    let parts: Vec<&str> = pair.split(':').collect();
                    if parts.len() == 2 {
                        values.push((
                            parts[0].trim().to_string(),
                            parts[1].trim().to_string(),
                        ));
                    }
                }
            }
        }
    }

    values
}

/// Detect indentation level of a line (spaces or tabs).
fn detect_indentation(line: &str) -> String {
    let leading_ws: String = line.chars().take_while(|c| c.is_whitespace()).collect();
    if leading_ws.is_empty() {
        // Default to 4 spaces
        "    ".to_string()
    } else {
        leading_ws
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_language_detection() {
        assert_eq!(
            Language::from_path(Path::new("foo.py")),
            Some(Language::Python)
        );
        assert_eq!(
            Language::from_path(Path::new("bar.rs")),
            Some(Language::Rust)
        );
        assert_eq!(
            Language::from_path(Path::new("baz.js")),
            Some(Language::JavaScript)
        );
    }

    #[test]
    fn test_python_print_generation() {
        let print = Language::Python.generate_print(&["x".to_string(), "y".to_string()]);
        assert!(print.contains("print(f\"[INSPECT]"));
        assert!(print.contains("x={x}"));
        assert!(print.contains("y={y}"));
    }

    #[test]
    fn test_rust_print_generation() {
        let print = Language::Rust.generate_print(&["foo".to_string()]);
        assert!(print.contains("println!"));
        assert!(print.contains("[INSPECT]"));
        assert!(print.contains("{:?}"));
    }

    #[test]
    fn test_extract_inspect_values_python() {
        let output = "[INSPECT] x=3, result=hello";
        let values = extract_inspect_values(output);
        assert_eq!(values.len(), 2);
        assert_eq!(values[0], ("x".to_string(), "3".to_string()));
        assert_eq!(values[1], ("result".to_string(), "hello".to_string()));
    }

    #[test]
    fn test_extract_inspect_values_js() {
        let output = "[INSPECT] {x: 42, name: 'test'}";
        let values = extract_inspect_values(output);
        assert_eq!(values.len(), 2);
        assert_eq!(values[0], ("x".to_string(), "42".to_string()));
    }

    #[test]
    fn test_detect_indentation() {
        assert_eq!(detect_indentation("    def foo():"), "    ");
        assert_eq!(detect_indentation("\t\tlet x = 5;"), "\t\t");
        assert_eq!(detect_indentation("no_indent"), "    ");
    }

    #[test]
    fn test_instrument_python_code() {
        use std::fs;
        use tempfile::NamedTempFile;

        let temp_file = NamedTempFile::new().unwrap();
        let content = "def add(a, b):\n    result = a + b\n    return result\n";
        fs::write(temp_file.path(), content).unwrap();

        let request = InstrumentationRequest {
            file: temp_file.path().to_path_buf(),
            line: 3, // Before "return result"
            variables: vec!["result".to_string()],
            language: Language::Python,
        };

        let instrumented = instrument_code(&request).unwrap();
        let instrumented_content = fs::read_to_string(&instrumented.instrumented_path).unwrap();

        assert!(instrumented_content.contains("print(f\"[INSPECT] result={result}\")"));
        assert!(instrumented_content.contains("return result"));
    }
}
