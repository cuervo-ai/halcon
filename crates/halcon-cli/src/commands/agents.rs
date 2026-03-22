//! CLI commands for managing declarative sub-agent definitions (Feature 4).
//!
//! Sub-commands:
//! - `halcon agents list`    — list all registered agents across scopes
//! - `halcon agents validate` — validate agent definition files, report all errors

use std::path::PathBuf;

use anyhow::Result;

use crate::repl::agent_registry::AgentRegistry;

/// List all registered sub-agents.
///
/// Discovers agents from project scope (`.halcon/agents/`) and user scope
/// (`~/.halcon/agents/`).  Prints a structured listing with scope, model,
/// max_turns, description, and any validation warnings.
pub fn list(working_dir: &str, verbose: bool) -> Result<()> {
    let registry = AgentRegistry::load(&[], std::path::Path::new(working_dir));

    // Print any load-time warnings first.
    let warnings = registry.warnings();
    if !warnings.is_empty() {
        eprintln!("Warnings during agent discovery:");
        for w in warnings {
            eprintln!("  {w}");
        }
        eprintln!();
    }

    if registry.is_empty() {
        println!("No agents registered.");
        println!();
        println!("To add a sub-agent, create a Markdown file with YAML frontmatter:");
        println!("  Project scope: .halcon/agents/<name>.md");
        println!("  User scope:    ~/.halcon/agents/<name>.md");
        println!();
        println!("Example (code-reviewer.md):");
        println!("  ---");
        println!("  name: code-reviewer");
        println!("  description: Expert code reviewer. Use after any code changes.");
        println!("  tools: [file_read, grep, glob]");
        println!("  model: haiku");
        println!("  max_turns: 15");
        println!("  ---");
        println!();
        println!("  You are an expert code reviewer. Focus on security and maintainability.");
        return Ok(());
    }

    let output = registry.format_list();
    print!("{output}");

    if verbose {
        println!();
        if let Some(manifest) = registry.routing_manifest() {
            println!("## Routing Manifest (injected into system prompt)");
            println!();
            println!("{manifest}");
        }
    }

    println!("{} agent(s) registered.", registry.len());
    Ok(())
}

/// Validate agent definition files and report all errors/warnings.
///
/// If `paths` is empty, discovers agents from the standard scopes.
/// If `paths` is non-empty, validates only those specific files.
///
/// Exits with status 1 if any hard errors are found.
pub fn validate(working_dir: &str, paths: &[PathBuf]) -> Result<()> {
    use crate::repl::agent_registry::loader::{load_agent_file, load_scope};
    use crate::repl::agent_registry::{validator::Diagnostic, AgentScope};

    let defs = if paths.is_empty() {
        let mut all = Vec::new();
        all.extend(load_scope(
            AgentScope::Project,
            std::path::Path::new(working_dir),
        ));
        all.extend(load_scope(
            AgentScope::User,
            std::path::Path::new(working_dir),
        ));
        all
    } else {
        paths
            .iter()
            .filter_map(|p| load_agent_file(p, AgentScope::Session))
            .collect()
    };

    if defs.is_empty() {
        println!("No agent definition files found.");
        return Ok(());
    }

    let skills =
        crate::repl::agent_registry::skills::load_all_skills(std::path::Path::new(working_dir));
    let known_skills: std::collections::HashSet<String> = skills.keys().cloned().collect();

    let mut total_errors = 0;
    let mut total_warnings = 0;

    for def in &defs {
        let diags = crate::repl::agent_registry::validator::validate_agent(def, &known_skills);
        if diags.is_empty() {
            println!("  OK  {}", def.source_path.display());
            continue;
        }
        for d in &diags {
            match d {
                Diagnostic::Error { message, .. } => {
                    println!("ERROR  {}: {message}", def.source_path.display());
                    total_errors += 1;
                }
                Diagnostic::Warning { message, .. } => {
                    println!(" WARN  {}: {message}", def.source_path.display());
                    total_warnings += 1;
                }
            }
        }
    }

    println!();
    println!(
        "Validated {} file(s): {} error(s), {} warning(s).",
        defs.len(),
        total_errors,
        total_warnings
    );

    if total_errors > 0 {
        anyhow::bail!("{total_errors} validation error(s) found");
    }

    Ok(())
}
