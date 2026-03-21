//! Batch validation for agent definition frontmatter.
//!
//! All errors are collected before returning — no fail-fast.  This gives
//! developers a full picture of every problem in one pass.
//!
//! # Validation rules
//!
//! | Field        | Rule |
//! |--------------|------|
//! | `name`       | kebab-case: `^[a-z][a-z0-9-]*$`, max 64 chars, non-empty |
//! | `description`| non-empty, ≤ 256 chars |
//! | `max_turns`  | 1–100 inclusive |
//! | `tools`      | known tool names (warning only, not error) |
//! | `skills`     | declared skills exist in skill map |
//! | name collision | higher-priority scope wins; warning only |

use std::collections::HashMap;

use super::schema::{AgentDefinition, AgentScope};

static KEBAB_CASE_RE: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"^[a-z][a-z0-9-]*$").unwrap());

/// A validation diagnostic.  Errors prevent the agent from being registered;
/// warnings are surfaced but do not block loading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Diagnostic {
    /// Hard error — agent will not be loaded.
    Error {
        agent_source: String,
        message: String,
    },
    /// Soft warning — agent is loaded but user is notified.
    Warning {
        agent_source: String,
        message: String,
    },
}

impl Diagnostic {
    pub fn is_error(&self) -> bool {
        matches!(self, Diagnostic::Error { .. })
    }

    pub fn message(&self) -> &str {
        match self {
            Diagnostic::Error { message, .. } | Diagnostic::Warning { message, .. } => message,
        }
    }

    pub fn source(&self) -> &str {
        match self {
            Diagnostic::Error { agent_source, .. } | Diagnostic::Warning { agent_source, .. } => {
                agent_source
            }
        }
    }
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (level, source, msg) = match self {
            Diagnostic::Error {
                agent_source,
                message,
            } => ("ERROR", agent_source, message),
            Diagnostic::Warning {
                agent_source,
                message,
            } => ("WARN ", agent_source, message),
        };
        write!(f, "[{level}] {source}: {msg}")
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Validate a single agent definition.  Returns all diagnostics found.
///
/// `known_skills` is the set of available skill names.
pub fn validate_agent(
    def: &AgentDefinition,
    known_skills: &std::collections::HashSet<String>,
) -> Vec<Diagnostic> {
    let source = def.source_path.display().to_string();
    let mut diags = Vec::new();

    // name: non-empty
    if def.name.is_empty() {
        diags.push(Diagnostic::Error {
            agent_source: source.clone(),
            message: "field 'name' is required and must not be empty".to_string(),
        });
    } else {
        // name: kebab-case
        if !KEBAB_CASE_RE.is_match(&def.name) {
            diags.push(Diagnostic::Error {
                agent_source: source.clone(),
                message: format!(
                    "field 'name' must match ^[a-z][a-z0-9-]*$ (got '{}')",
                    def.name
                ),
            });
        }
        // name: max length
        if def.name.len() > 64 {
            diags.push(Diagnostic::Error {
                agent_source: source.clone(),
                message: format!(
                    "field 'name' must be ≤64 characters (got {})",
                    def.name.len()
                ),
            });
        }
    }

    // description: non-empty
    if def.description.is_empty() {
        diags.push(Diagnostic::Error {
            agent_source: source.clone(),
            message: "field 'description' is required and must not be empty".to_string(),
        });
    } else if def.description.len() > 256 {
        diags.push(Diagnostic::Warning {
            agent_source: source.clone(),
            message: format!(
                "field 'description' exceeds 256 characters ({}); truncated in routing manifest",
                def.description.len()
            ),
        });
    }

    // max_turns: 1–100
    if def.max_turns == 0 || def.max_turns > 100 {
        diags.push(Diagnostic::Error {
            agent_source: source.clone(),
            message: format!(
                "field 'max_turns' must be in range 1–100 (got {})",
                def.max_turns
            ),
        });
    }

    // skills: all declared skills must exist in the skill map
    for skill_name in &def.skills {
        if !known_skills.contains(skill_name.as_str()) {
            // Suggest the closest known skill name.
            let suggestion = closest_name(skill_name, known_skills.iter().map(|s| s.as_str()));
            let hint = if let Some(s) = suggestion {
                format!(" (did you mean '{s}'?)")
            } else {
                String::new()
            };
            diags.push(Diagnostic::Warning {
                agent_source: source.clone(),
                message: format!("skill '{}' not found{}", skill_name, hint),
            });
        }
    }

    diags
}

/// Resolve name collisions across scopes and return the winning set of agents.
///
/// When two agents have the same name, the higher-priority scope wins:
/// `Session > Project > User`.  A `Warning` diagnostic is emitted for each
/// collision.
pub fn resolve_collisions(
    all_defs: Vec<AgentDefinition>,
) -> (Vec<AgentDefinition>, Vec<Diagnostic>) {
    // Sort by descending scope priority so the first occurrence of a name wins.
    let mut sorted = all_defs;
    sorted.sort_by(|a, b| b.scope.cmp(&a.scope)); // Session=2 > Project=1 > User=0

    let mut seen: HashMap<String, AgentScope> = HashMap::new();
    let mut winners = Vec::new();
    let mut diags = Vec::new();

    for def in sorted {
        if let Some(&existing_scope) = seen.get(&def.name) {
            diags.push(Diagnostic::Warning {
                agent_source: def.source_path.display().to_string(),
                message: format!(
                    "agent name '{}' already registered from {} scope; {} scope version ignored",
                    def.name, existing_scope, def.scope
                ),
            });
        } else {
            seen.insert(def.name.clone(), def.scope);
            winners.push(def);
        }
    }

    // Re-sort winners by name for deterministic output.
    winners.sort_by(|a, b| a.name.cmp(&b.name));
    (winners, diags)
}

// ── Levenshtein helper ────────────────────────────────────────────────────────

/// Return the name with the smallest Levenshtein distance to `query`,
/// or `None` if the set is empty or the best distance exceeds 3.
pub fn closest_name<'a>(query: &str, candidates: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    let mut best: Option<(&str, usize)> = None;
    for candidate in candidates {
        let dist = levenshtein(query, candidate);
        match best {
            None => best = Some((candidate, dist)),
            Some((_, d)) if dist < d => best = Some((candidate, dist)),
            _ => {}
        }
    }
    best.filter(|(_, d)| *d <= 3).map(|(name, _)| name)
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let m = a.len();
    let n = b.len();

    let mut dp = vec![vec![0usize; n + 1]; m + 1];
    for i in 0..=m {
        dp[i][0] = i;
    }
    for j in 0..=n {
        dp[0][j] = j;
    }

    for i in 1..=m {
        for j in 1..=n {
            dp[i][j] = if a[i - 1] == b[j - 1] {
                dp[i - 1][j - 1]
            } else {
                1 + dp[i - 1][j - 1].min(dp[i - 1][j]).min(dp[i][j - 1])
            };
        }
    }
    dp[m][n]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::path::PathBuf;

    fn make_def(
        name: &str,
        description: &str,
        max_turns: u32,
        scope: AgentScope,
    ) -> AgentDefinition {
        AgentDefinition {
            name: name.to_string(),
            description: description.to_string(),
            tools: vec![],
            disallowed_tools: vec![],
            resolved_model: None,
            max_turns,
            memory: None,
            skills: vec![],
            background: false,
            system_prompt: String::new(),
            source_path: PathBuf::from(format!("{name}.md")),
            scope,
        }
    }

    // ── validate_agent ────────────────────────────────────────────────────────

    #[test]
    fn valid_agent_has_no_diagnostics() {
        let def = make_def(
            "code-reviewer",
            "Reviews code carefully",
            15,
            AgentScope::Project,
        );
        let skills: HashSet<String> = HashSet::new();
        let diags = validate_agent(&def, &skills);
        assert!(
            diags.is_empty(),
            "expected no diagnostics, got: {:?}",
            diags
        );
    }

    #[test]
    fn empty_name_is_error() {
        let def = make_def("", "Has description", 10, AgentScope::Project);
        let skills = HashSet::new();
        let diags = validate_agent(&def, &skills);
        assert!(diags
            .iter()
            .any(|d| d.is_error() && d.message().contains("name")));
    }

    #[test]
    fn uppercase_name_is_error() {
        let def = make_def("CodeReviewer", "Has description", 10, AgentScope::Project);
        let skills = HashSet::new();
        let diags = validate_agent(&def, &skills);
        assert!(diags
            .iter()
            .any(|d| d.is_error() && d.message().contains("name")));
    }

    #[test]
    fn name_with_underscore_is_error() {
        let def = make_def("code_reviewer", "Has description", 10, AgentScope::Project);
        let skills = HashSet::new();
        let diags = validate_agent(&def, &skills);
        assert!(diags
            .iter()
            .any(|d| d.is_error() && d.message().contains("name")));
    }

    #[test]
    fn name_starting_with_digit_is_error() {
        let def = make_def("1agent", "Has description", 10, AgentScope::Project);
        let skills = HashSet::new();
        let diags = validate_agent(&def, &skills);
        assert!(diags
            .iter()
            .any(|d| d.is_error() && d.message().contains("name")));
    }

    #[test]
    fn name_too_long_is_error() {
        let long_name = "a".repeat(65);
        let def = make_def(&long_name, "Has description", 10, AgentScope::Project);
        let skills = HashSet::new();
        let diags = validate_agent(&def, &skills);
        assert!(diags
            .iter()
            .any(|d| d.is_error() && d.message().contains("64")));
    }

    #[test]
    fn empty_description_is_error() {
        let def = make_def("my-agent", "", 10, AgentScope::Project);
        let skills = HashSet::new();
        let diags = validate_agent(&def, &skills);
        assert!(diags
            .iter()
            .any(|d| d.is_error() && d.message().contains("description")));
    }

    #[test]
    fn long_description_is_warning_not_error() {
        let long_desc = "x".repeat(300);
        let def = make_def("my-agent", &long_desc, 10, AgentScope::Project);
        let skills = HashSet::new();
        let diags = validate_agent(&def, &skills);
        let errors: Vec<_> = diags.iter().filter(|d| d.is_error()).collect();
        let warnings: Vec<_> = diags.iter().filter(|d| !d.is_error()).collect();
        assert!(errors.is_empty(), "long description must not be an error");
        assert!(
            !warnings.is_empty(),
            "long description must produce a warning"
        );
    }

    #[test]
    fn max_turns_zero_is_error() {
        let def = make_def("my-agent", "Description", 0, AgentScope::Project);
        let skills = HashSet::new();
        let diags = validate_agent(&def, &skills);
        assert!(diags
            .iter()
            .any(|d| d.is_error() && d.message().contains("max_turns")));
    }

    #[test]
    fn max_turns_101_is_error() {
        let def = make_def("my-agent", "Description", 101, AgentScope::Project);
        let skills = HashSet::new();
        let diags = validate_agent(&def, &skills);
        assert!(diags
            .iter()
            .any(|d| d.is_error() && d.message().contains("max_turns")));
    }

    #[test]
    fn max_turns_100_is_valid() {
        let def = make_def("my-agent", "Description", 100, AgentScope::Project);
        let skills = HashSet::new();
        let diags = validate_agent(&def, &skills);
        assert!(diags.iter().all(|d| !d.is_error()));
    }

    #[test]
    fn unknown_skill_is_warning() {
        let mut def = make_def("my-agent", "Description", 10, AgentScope::Project);
        def.skills = vec!["nonexistent-skill".to_string()];
        let skills: HashSet<String> = HashSet::new();
        let diags = validate_agent(&def, &skills);
        assert!(diags
            .iter()
            .any(|d| !d.is_error() && d.message().contains("nonexistent-skill")));
    }

    #[test]
    fn unknown_skill_suggests_close_match() {
        let mut def = make_def("my-agent", "Description", 10, AgentScope::Project);
        def.skills = vec!["security-guidlines".to_string()]; // typo
        let mut skills: HashSet<String> = HashSet::new();
        skills.insert("security-guidelines".to_string());
        let diags = validate_agent(&def, &skills);
        let warn = diags
            .iter()
            .find(|d| !d.is_error())
            .expect("should have warning");
        assert!(
            warn.message().contains("security-guidelines"),
            "should suggest correct name"
        );
    }

    // ── resolve_collisions ────────────────────────────────────────────────────

    #[test]
    fn no_collision_returns_all_agents() {
        let defs = vec![
            make_def("agent-a", "A", 10, AgentScope::Project),
            make_def("agent-b", "B", 10, AgentScope::User),
        ];
        let (winners, diags) = resolve_collisions(defs);
        assert_eq!(winners.len(), 2);
        assert!(diags.is_empty());
    }

    #[test]
    fn project_wins_over_user_on_collision() {
        let defs = vec![
            make_def("shared-agent", "User version", 10, AgentScope::User),
            make_def("shared-agent", "Project version", 10, AgentScope::Project),
        ];
        let (winners, diags) = resolve_collisions(defs);
        assert_eq!(winners.len(), 1);
        assert_eq!(winners[0].description, "Project version");
        assert_eq!(diags.len(), 1);
        assert!(!diags[0].is_error()); // collision is a warning, not error
    }

    #[test]
    fn session_wins_over_project_on_collision() {
        let defs = vec![
            make_def("shared-agent", "Project version", 10, AgentScope::Project),
            make_def("shared-agent", "Session version", 10, AgentScope::Session),
        ];
        let (winners, diags) = resolve_collisions(defs);
        assert_eq!(winners.len(), 1);
        assert_eq!(winners[0].description, "Session version");
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn winners_are_sorted_by_name() {
        let defs = vec![
            make_def("z-agent", "Z", 10, AgentScope::Project),
            make_def("a-agent", "A", 10, AgentScope::Project),
            make_def("m-agent", "M", 10, AgentScope::Project),
        ];
        let (winners, _) = resolve_collisions(defs);
        assert_eq!(winners[0].name, "a-agent");
        assert_eq!(winners[1].name, "m-agent");
        assert_eq!(winners[2].name, "z-agent");
    }

    // ── levenshtein ───────────────────────────────────────────────────────────

    #[test]
    fn levenshtein_equal_strings() {
        assert_eq!(levenshtein("hello", "hello"), 0);
    }

    #[test]
    fn levenshtein_single_substitution() {
        assert_eq!(levenshtein("hello", "hxllo"), 1);
    }

    #[test]
    fn levenshtein_insertion() {
        assert_eq!(levenshtein("abc", "abcd"), 1);
    }

    #[test]
    fn levenshtein_deletion() {
        assert_eq!(levenshtein("abcd", "abc"), 1);
    }

    #[test]
    fn levenshtein_empty_strings() {
        assert_eq!(levenshtein("", ""), 0);
        assert_eq!(levenshtein("abc", ""), 3);
        assert_eq!(levenshtein("", "abc"), 3);
    }

    // ── closest_name ─────────────────────────────────────────────────────────

    #[test]
    fn closest_finds_near_match() {
        let candidates = vec!["security-guidelines", "code-review", "test-runner"];
        let result = closest_name("security-guidlines", candidates.iter().copied());
        assert_eq!(result, Some("security-guidelines"));
    }

    #[test]
    fn closest_returns_none_when_too_far() {
        let candidates = vec!["xyz", "abc"];
        let result = closest_name("security-guidelines", candidates.iter().copied());
        assert!(result.is_none());
    }

    #[test]
    fn closest_returns_none_for_empty_candidates() {
        let result = closest_name("anything", std::iter::empty());
        assert!(result.is_none());
    }
}
