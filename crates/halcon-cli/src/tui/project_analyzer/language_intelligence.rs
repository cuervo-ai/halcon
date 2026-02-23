//! Language Intelligence Scanner — Phase 110-112
//!
//! Performs a bounded file-extension census (≤200 000 files) to classify the
//! primary language, secondary languages, frameworks and project scale.
//! Uses only file *presence* (no content parsing) for maximum reliability.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

use super::tools::ToolOutput;

// ─── Extension → Language map ─────────────────────────────────────────────────

/// Maps file extensions (lowercase, no dot) to canonical language names.
static LANG_EXTENSIONS: &[(&str, &str)] = &[
    // Rust
    ("rs", "Rust"),
    // Go
    ("go", "Go"),
    // JavaScript / TypeScript
    ("js", "JavaScript"),
    ("mjs", "JavaScript"),
    ("cjs", "JavaScript"),
    ("ts", "TypeScript"),
    ("tsx", "TypeScript"),
    ("jsx", "JavaScript"),
    // Python
    ("py", "Python"),
    ("pyi", "Python"),
    ("ipynb", "Python"),
    // Java / JVM
    ("java", "Java"),
    ("kt", "Kotlin"),
    ("kts", "Kotlin"),
    ("scala", "Scala"),
    ("groovy", "Groovy"),
    ("gradle", "Groovy"),
    // C# / .NET
    ("cs", "C#"),
    ("csx", "C#"),
    ("vb", "VB.NET"),
    ("fs", "F#"),
    ("fsx", "F#"),
    // C / C++
    ("c", "C"),
    ("h", "C"),
    ("cpp", "C++"),
    ("cxx", "C++"),
    ("cc", "C++"),
    ("hpp", "C++"),
    ("hxx", "C++"),
    // PHP
    ("php", "PHP"),
    // Ruby
    ("rb", "Ruby"),
    ("rake", "Ruby"),
    // Swift / Objective-C
    ("swift", "Swift"),
    ("m", "Objective-C"),
    ("mm", "Objective-C"),
    // Elixir / Erlang
    ("ex", "Elixir"),
    ("exs", "Elixir"),
    ("erl", "Erlang"),
    ("hrl", "Erlang"),
    // Haskell / OCaml
    ("hs", "Haskell"),
    ("lhs", "Haskell"),
    ("ml", "OCaml"),
    ("mli", "OCaml"),
    // Zig
    ("zig", "Zig"),
    // Dart / Flutter
    ("dart", "Dart"),
    // Lua
    ("lua", "Lua"),
    // R
    ("r", "R"),
    // Julia
    ("jl", "Julia"),
    // Infrastructure / Config
    ("tf", "HCL/Terraform"),
    ("tfvars", "HCL/Terraform"),
    ("yaml", "YAML"),
    ("yml", "YAML"),
    ("toml", "TOML"),
    ("json", "JSON"),
    // Shell
    ("sh", "Shell"),
    ("bash", "Shell"),
    ("zsh", "Shell"),
    ("fish", "Shell"),
    ("ps1", "PowerShell"),
    // HTML / CSS
    ("html", "HTML"),
    ("htm", "HTML"),
    ("css", "CSS"),
    ("scss", "CSS/SCSS"),
    ("sass", "CSS/SCSS"),
    ("less", "CSS/LESS"),
    // SQL
    ("sql", "SQL"),
    // Markdown / Docs
    ("md", "Markdown"),
    ("mdx", "Markdown"),
    ("rst", "reStructuredText"),
    // Proto
    ("proto", "Protobuf"),
];

// Skip directories that inflate file counts
static SKIP_DIRS: &[&str] = &[
    "target", "node_modules", ".git", ".svn", "dist", "build",
    "out", "__pycache__", ".venv", "venv", ".tox", ".mypy_cache",
    ".pytest_cache", ".cargo", "vendor", ".gradle", ".idea",
    ".vscode", "coverage", ".nyc_output", "tmp", "temp",
];

// ─── Framework / tooling detection ───────────────────────────────────────────

/// Frontend frameworks — keyed by a config/marker file at project root or nearby.
static FRONTEND_MARKERS: &[(&str, &str)] = &[
    ("next.config.js", "Next.js"),
    ("next.config.mjs", "Next.js"),
    ("next.config.ts", "Next.js"),
    ("nuxt.config.ts", "Nuxt.js"),
    ("nuxt.config.js", "Nuxt.js"),
    ("angular.json", "Angular"),
    ("svelte.config.js", "SvelteKit"),
    ("svelte.config.ts", "SvelteKit"),
    ("astro.config.mjs", "Astro"),
    ("astro.config.ts", "Astro"),
    ("vite.config.ts", "Vite"),
    ("vite.config.js", "Vite"),
    ("webpack.config.js", "Webpack"),
    ("webpack.config.ts", "Webpack"),
    ("remix.config.js", "Remix"),
    ("gatsby-config.js", "Gatsby"),
    ("gatsby-config.ts", "Gatsby"),
    ("_app.tsx", "Next.js"),
];

/// Mobile frameworks.
static MOBILE_MARKERS: &[(&str, &str)] = &[
    ("pubspec.yaml", "Flutter"),
    ("android/build.gradle", "React Native"),
    ("ios/Podfile", "React Native"),
    ("app.json", "React Native"),
    ("capacitor.config.ts", "Capacitor"),
    ("capacitor.config.json", "Capacitor"),
];

/// Data / AI / ML frameworks.
static DATA_MARKERS: &[(&str, &str)] = &[
    ("MLproject", "MLflow"),
    ("mlflow.yml", "MLflow"),
    ("dvc.yaml", "DVC"),
    ("dvc.yml", "DVC"),
    ("dagster.yaml", "Dagster"),
    ("airflow.cfg", "Airflow"),
    ("airflow_settings.yaml", "Airflow"),
    ("kedro_pipeline.py", "Kedro"),
    ("conf/base/catalog.yml", "Kedro"),
    ("requirements.txt", "Python/pip"),  // fallback for python-heavy repos
    ("environment.yml", "Conda"),
    ("pyproject.toml", "Python/Poetry"),
];

/// Infra / DevOps tooling.
static INFRA_MARKERS: &[(&str, &str)] = &[
    ("Pulumi.yaml", "Pulumi"),
    ("Pulumi.yml", "Pulumi"),
    ("main.tf", "Terraform"),
    ("terraform.tf", "Terraform"),
    ("playbook.yml", "Ansible"),
    ("playbook.yaml", "Ansible"),
    ("site.yml", "Ansible"),
    ("Chart.yaml", "Helm"),
    ("helmfile.yaml", "Helmfile"),
    ("Packer.pkr.hcl", "Packer"),
    ("packer.json", "Packer"),
    ("serverless.yml", "Serverless Framework"),
    ("serverless.yaml", "Serverless Framework"),
    ("cdk.json", "AWS CDK"),
    ("template.yaml", "SAM"),
];

// ─── Monorepo detection ───────────────────────────────────────────────────────

static MONOREPO_MARKERS: &[(&str, &str)] = &[
    ("turbo.json", "Turborepo"),
    ("nx.json", "Nx"),
    ("lerna.json", "Lerna"),
    ("pnpm-workspace.yaml", "pnpm workspaces"),
    ("rush.json", "Rush"),
    ("pnpm-workspace.yml", "pnpm workspaces"),
    ("WORKSPACE", "Bazel"),
    ("WORKSPACE.bazel", "Bazel"),
    ("BUILD.bazel", "Bazel"),
    ("pants.toml", "Pants"),
    ("BUILD", "Pants/Bazel"),
];

// Sub-project markers: a directory at depth ≤ 2 is a sub-project if it has one of these.
static SUBPROJECT_MARKERS: &[&str] = &[
    "package.json",
    "Cargo.toml",
    "pyproject.toml",
    "go.mod",
    "pom.xml",
    "build.gradle",
    "setup.py",
    "build.sbt",
];

// ─── Main entry point ─────────────────────────────────────────────────────────

/// Scan the project root and return language/scale/monorepo intelligence.
pub async fn language_intelligence_scanner(root: &Path) -> ToolOutput {
    let root = root.to_path_buf();
    let result = tokio::task::spawn_blocking(move || scan_language_intelligence(&root))
        .await
        .unwrap_or_default();
    result
}

fn scan_language_intelligence(root: &Path) -> ToolOutput {
    let _t0 = Instant::now();

    // 1. File extension census (bounded at 200 000 files)
    let (ext_counts, total_files) = count_extensions(root, 200_000);

    // 2. Primary + secondary languages
    let mut lang_counts: HashMap<String, u32> = HashMap::new();
    for (ext, lang) in LANG_EXTENSIONS {
        if let Some(&count) = ext_counts.get(*ext) {
            *lang_counts.entry(lang.to_string()).or_default() += count;
        }
    }

    // Sort by count descending
    let mut lang_vec: Vec<(String, u32)> = lang_counts.into_iter().collect();
    lang_vec.sort_by(|a, b| b.1.cmp(&a.1));

    // Filter: ignore pure-config languages (YAML/JSON/TOML) if other code languages exist
    let code_langs: Vec<(String, u32)> = lang_vec
        .iter()
        .filter(|(l, _)| !matches!(l.as_str(), "YAML" | "JSON" | "TOML" | "Markdown" | "reStructuredText"))
        .cloned()
        .collect();

    let display_langs = if code_langs.is_empty() { &lang_vec } else { &code_langs };

    let primary_language = display_langs
        .first()
        .map(|(l, _)| l.clone())
        .unwrap_or_else(|| "Unknown".to_string());

    let secondary_languages: Vec<String> = display_langs
        .iter()
        .skip(1)
        .take(4)
        .map(|(l, _)| l.clone())
        .collect();

    let is_polyglot = secondary_languages.len() >= 2;

    // 3. Framework detection (presence-only)
    let frontend_framework = detect_from_markers(root, FRONTEND_MARKERS);
    let mobile_framework   = detect_from_markers(root, MOBILE_MARKERS);
    let data_framework     = detect_framework_data(root);
    let infra_tool         = detect_from_markers(root, INFRA_MARKERS);

    // 4. Monorepo detection
    let (is_monorepo, monorepo_tool) = detect_monorepo(root);

    // 5. Sub-project detection (depth ≤ 2)
    let sub_projects = detect_sub_projects(root, 2);
    let sub_project_count = sub_projects.len() as u32;

    // 6. Project scale
    let project_scale = determine_project_scale(total_files);

    // 7. LOC estimate (rough: total files × average lines per file by language)
    let estimated_loc = estimate_loc(&primary_language, total_files);

    ToolOutput {
        primary_language: Some(primary_language),
        secondary_languages: Some(secondary_languages),
        is_polyglot: Some(is_polyglot),
        language_breakdown: Some(display_langs.iter().take(8).cloned().collect()),
        frontend_framework,
        mobile_framework,
        data_framework,
        infra_tool,
        is_monorepo: Some(is_monorepo),
        monorepo_tool,
        sub_project_count: Some(sub_project_count),
        sub_projects: Some(sub_projects),
        project_scale: Some(project_scale),
        total_file_count: Some(total_files),
        estimated_loc: Some(estimated_loc),
        ..Default::default()
    }
}

// ─── Helper: count file extensions ───────────────────────────────────────────

/// Bounded recursive scan that counts files by lowercase extension.
/// Returns (ext_map, total_files_visited).
fn count_extensions(root: &Path, max_files: u32) -> (HashMap<String, u32>, u32) {
    let mut counts: HashMap<String, u32> = HashMap::new();
    let mut total: u32 = 0;
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        if total >= max_files {
            break;
        }
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            if total >= max_files {
                break;
            }
            let path = entry.path();
            let file_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");

            if path.is_dir() {
                if !SKIP_DIRS.contains(&file_name) {
                    stack.push(path);
                }
            } else {
                total += 1;
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    *counts.entry(ext.to_lowercase()).or_default() += 1;
                }
            }
        }
    }
    (counts, total)
}

// ─── Helper: marker-based framework detection ─────────────────────────────────

fn detect_from_markers(root: &Path, markers: &[(&str, &str)]) -> Option<String> {
    for (marker, name) in markers {
        if root.join(marker).exists() {
            return Some(name.to_string());
        }
    }
    None
}

fn detect_framework_data(root: &Path) -> Option<String> {
    // Prefer specific ML tools over generic Python ones
    let priority: &[(&str, &str)] = &[
        ("MLproject", "MLflow"),
        ("mlflow.yml", "MLflow"),
        ("dvc.yaml", "DVC"),
        ("dvc.yml", "DVC"),
        ("dagster.yaml", "Dagster"),
        ("airflow.cfg", "Airflow"),
        ("environment.yml", "Conda"),
    ];
    for (marker, name) in priority {
        if root.join(marker).exists() {
            return Some(name.to_string());
        }
    }
    // Check for ML-flavored requirements or pyproject
    if root.join("requirements.txt").exists() {
        if let Ok(content) = fs::read_to_string(root.join("requirements.txt")) {
            if content.contains("torch") || content.contains("tensorflow") || content.contains("keras") {
                return Some("PyTorch/TensorFlow".to_string());
            }
            if content.contains("scikit-learn") || content.contains("sklearn") {
                return Some("scikit-learn".to_string());
            }
            if content.contains("fastapi") || content.contains("flask") || content.contains("django") {
                return None; // It's web, not data — let frontend_framework handle it
            }
        }
    }
    None
}

// ─── Helper: monorepo detection ───────────────────────────────────────────────

fn detect_monorepo(root: &Path) -> (bool, Option<String>) {
    // Explicit monorepo tool markers
    for (marker, name) in MONOREPO_MARKERS {
        if root.join(marker).exists() {
            return (true, Some(name.to_string()));
        }
    }

    // Cargo workspace: Cargo.toml with [workspace] section
    let cargo_toml = root.join("Cargo.toml");
    if cargo_toml.exists() {
        if let Ok(content) = fs::read_to_string(&cargo_toml) {
            if content.contains("[workspace]") {
                return (true, Some("Cargo workspaces".to_string()));
            }
        }
    }

    // pnpm / yarn workspaces in package.json
    let pkg_json = root.join("package.json");
    if pkg_json.exists() {
        if let Ok(content) = fs::read_to_string(&pkg_json) {
            if content.contains(r#""workspaces""#) {
                // Distinguish yarn vs npm
                let tool = if root.join("yarn.lock").exists() {
                    "Yarn workspaces"
                } else {
                    "npm workspaces"
                };
                return (true, Some(tool.to_string()));
            }
        }
    }

    // go.work file (Go workspace)
    if root.join("go.work").exists() {
        return (true, Some("Go workspaces".to_string()));
    }

    (false, None)
}

// ─── Helper: sub-project detection ───────────────────────────────────────────

fn detect_sub_projects(root: &Path, max_depth: u32) -> Vec<String> {
    let mut subs = Vec::new();
    detect_sub_projects_recursive(root, root, 0, max_depth, &mut subs);
    subs
}

fn detect_sub_projects_recursive(
    root: &Path,
    dir: &Path,
    depth: u32,
    max_depth: u32,
    result: &mut Vec<String>,
) {
    if depth > max_depth { return; }
    if depth == 0 {
        // Skip root itself; scan its children
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
                    if SKIP_DIRS.contains(&name.as_str()) { continue; }
                    detect_sub_projects_recursive(root, &path, 1, max_depth, result);
                }
            }
        }
        return;
    }
    // Check if this dir is a sub-project
    for marker in SUBPROJECT_MARKERS {
        if dir.join(marker).exists() {
            if let Ok(rel) = dir.strip_prefix(root) {
                result.push(rel.to_string_lossy().to_string());
            }
            return; // Don't recurse further into a sub-project
        }
    }
    // Not a sub-project; recurse if within depth
    if depth < max_depth {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
                    if SKIP_DIRS.contains(&name.as_str()) { continue; }
                    detect_sub_projects_recursive(root, &path, depth + 1, max_depth, result);
                }
            }
        }
    }
}

// ─── Helper: project scale ────────────────────────────────────────────────────

pub fn determine_project_scale(file_count: u32) -> String {
    match file_count {
        0..=500       => "Small".to_string(),
        501..=5_000   => "Medium".to_string(),
        5_001..=50_000 => "Large".to_string(),
        _              => "Enterprise".to_string(),
    }
}

// ─── Helper: LOC estimate ─────────────────────────────────────────────────────

fn estimate_loc(primary_language: &str, total_files: u32) -> u64 {
    // Rough average lines per file by language (heuristic)
    let avg_lines: u64 = match primary_language {
        "Rust" => 120,
        "Go" => 100,
        "TypeScript" | "JavaScript" => 80,
        "Python" => 90,
        "Java" | "Kotlin" | "Scala" => 130,
        "C#" => 125,
        "C" | "C++" => 150,
        "PHP" => 90,
        "Ruby" => 70,
        "Elixir" => 80,
        "Haskell" => 60,
        "Swift" => 100,
        "Dart" => 80,
        _ => 70,
    };
    (total_files as u64) * avg_lines
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_file(dir: &Path, name: &str) {
        fs::write(dir.join(name), b"x").unwrap();
    }
    fn make_dir(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        fs::create_dir_all(&p).unwrap();
        p
    }

    // Phase 112
    #[test]
    fn project_scale_small() {
        assert_eq!(determine_project_scale(0), "Small");
        assert_eq!(determine_project_scale(500), "Small");
    }
    #[test]
    fn project_scale_medium() {
        assert_eq!(determine_project_scale(501), "Medium");
        assert_eq!(determine_project_scale(5_000), "Medium");
    }
    #[test]
    fn project_scale_large() {
        assert_eq!(determine_project_scale(5_001), "Large");
        assert_eq!(determine_project_scale(50_000), "Large");
    }
    #[test]
    fn project_scale_enterprise() {
        assert_eq!(determine_project_scale(50_001), "Enterprise");
        assert_eq!(determine_project_scale(1_000_000), "Enterprise");
    }

    // Phase 110 — extension census
    #[test]
    fn counts_extensions_in_dir() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        make_file(root, "a.rs");
        make_file(root, "b.rs");
        make_file(root, "c.go");
        let (counts, total) = count_extensions(root, 200_000);
        assert_eq!(counts.get("rs"), Some(&2));
        assert_eq!(counts.get("go"), Some(&1));
        assert_eq!(total, 3);
    }

    #[test]
    fn skips_node_modules() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        make_file(root, "index.ts");
        let nm = make_dir(root, "node_modules");
        make_file(&nm, "hidden.ts");
        let (counts, total) = count_extensions(root, 200_000);
        assert_eq!(counts.get("ts"), Some(&1));
        assert_eq!(total, 1);
    }

    #[test]
    fn bounded_at_max_files() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        for i in 0..20 {
            make_file(root, &format!("f{i}.rs"));
        }
        let (_counts, total) = count_extensions(root, 10);
        assert!(total <= 10, "should be bounded at 10, got {total}");
    }

    // Phase 110 — language detection
    #[test]
    fn primary_language_rust() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        for i in 0..5 { make_file(root, &format!("f{i}.rs")); }
        make_file(root, "one.go");
        let out = scan_language_intelligence(root);
        assert_eq!(out.primary_language.as_deref(), Some("Rust"));
    }

    #[test]
    fn polyglot_detected_with_multiple_languages() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        for i in 0..3 { make_file(root, &format!("a{i}.rs")); }
        for i in 0..3 { make_file(root, &format!("b{i}.go")); }
        for i in 0..3 { make_file(root, &format!("c{i}.py")); }
        let out = scan_language_intelligence(root);
        assert_eq!(out.is_polyglot, Some(true));
    }

    // Phase 111 — monorepo
    #[test]
    fn detects_turborepo() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        make_file(root, "turbo.json");
        let (is_mono, tool) = detect_monorepo(root);
        assert!(is_mono);
        assert_eq!(tool.as_deref(), Some("Turborepo"));
    }

    #[test]
    fn detects_cargo_workspace() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(root.join("Cargo.toml"), b"[workspace]\nmembers = [\"crates/*\"]\n").unwrap();
        let (is_mono, tool) = detect_monorepo(root);
        assert!(is_mono);
        assert_eq!(tool.as_deref(), Some("Cargo workspaces"));
    }

    #[test]
    fn not_monorepo_plain_cargo() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(root.join("Cargo.toml"), b"[package]\nname = \"foo\"\n").unwrap();
        let (is_mono, _) = detect_monorepo(root);
        assert!(!is_mono);
    }

    #[test]
    fn detects_go_workspace() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        make_file(root, "go.work");
        let (is_mono, tool) = detect_monorepo(root);
        assert!(is_mono);
        assert_eq!(tool.as_deref(), Some("Go workspaces"));
    }

    // Phase 111 — sub-project detection
    #[test]
    fn detects_sub_projects() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let a = make_dir(root, "pkgA");
        make_file(&a, "package.json");
        let b = make_dir(root, "pkgB");
        make_file(&b, "Cargo.toml");
        let subs = detect_sub_projects(root, 2);
        assert!(subs.contains(&"pkgA".to_string()), "pkgA not in {subs:?}");
        assert!(subs.contains(&"pkgB".to_string()), "pkgB not in {subs:?}");
    }

    // Phase 111 — frontend framework
    #[test]
    fn detects_next_js() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        make_file(root, "next.config.js");
        let f = detect_from_markers(root, FRONTEND_MARKERS);
        assert_eq!(f.as_deref(), Some("Next.js"));
    }

    #[test]
    fn detects_angular() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        make_file(root, "angular.json");
        let f = detect_from_markers(root, FRONTEND_MARKERS);
        assert_eq!(f.as_deref(), Some("Angular"));
    }

    // LOC estimate
    #[test]
    fn loc_estimate_rust_1000_files() {
        let loc = estimate_loc("Rust", 1_000);
        assert_eq!(loc, 120_000);
    }

    // Full scanner returns reasonable output
    #[test]
    fn scanner_returns_default_for_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let out = scan_language_intelligence(tmp.path());
        assert!(out.project_scale.is_some());
        assert_eq!(out.project_scale.as_deref(), Some("Small"));
        assert_eq!(out.total_file_count, Some(0));
    }
}
