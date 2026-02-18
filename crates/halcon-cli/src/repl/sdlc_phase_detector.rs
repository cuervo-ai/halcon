/// SDLC Phase Detector — Automatic phase detection based on repository signals.
///
/// Analyzes Git repository state, file patterns, and user queries to determine
/// the current Software Development Life Cycle phase.
///
/// Phase Detection Heuristics:
/// - **Discovery**: README changes, requirements docs, design docs in recent commits
/// - **Planning**: Architecture docs, task breakdown files, project plans
/// - **Implementation**: Code files (.rs, .py, .ts, .js, .go) in active development
/// - **Testing**: Test files, CI config changes, test results
/// - **Review**: PR descriptions, review comments, CHANGELOG updates
/// - **Deployment**: Deployment scripts, Dockerfile, K8s manifests, release tags
/// - **Monitoring**: Metrics dashboards, alerting rules, observability configs
/// - **Support**: Bug reports, user feedback, hotfix branches

use halcon_core::types::SdlcPhase;
use std::collections::HashMap;
use std::path::Path;

/// SDLC Phase Detector.
///
/// Uses multiple signals (git history, file patterns, user queries) to infer
/// the current development phase.
pub struct SdlcPhaseDetector {
    /// Working directory (repository root).
    working_dir: std::path::PathBuf,
    /// File extension -> phase mapping.
    extension_weights: HashMap<&'static str, Vec<(SdlcPhase, f32)>>,
    /// Keyword -> phase mapping (for commit messages and queries).
    keyword_weights: HashMap<&'static str, Vec<(SdlcPhase, f32)>>,
}

impl SdlcPhaseDetector {
    pub fn new(working_dir: impl AsRef<Path>) -> Self {
        Self {
            working_dir: working_dir.as_ref().to_path_buf(),
            extension_weights: Self::build_extension_weights(),
            keyword_weights: Self::build_keyword_weights(),
        }
    }

    /// Detect SDLC phase based on all available signals.
    pub fn detect_phase(&self, user_query: Option<&str>) -> SdlcPhase {
        let mut scores: HashMap<SdlcPhase, f32> = HashMap::new();

        // Signal 1: Recent git commits (if .git exists)
        if self.working_dir.join(".git").exists() {
            if let Ok(commit_scores) = self.analyze_recent_commits() {
                for (phase, score) in commit_scores {
                    *scores.entry(phase).or_insert(0.0) += score * 1.0; // Weight: 1.0
                }
            }
        }

        // Signal 2: File patterns in working directory
        if let Ok(file_scores) = self.analyze_file_patterns() {
            for (phase, score) in file_scores {
                *scores.entry(phase).or_insert(0.0) += score * 0.8; // Weight: 0.8
            }
        }

        // Signal 3: User query keywords
        if let Some(query) = user_query {
            let query_scores = self.analyze_query_keywords(query);
            for (phase, score) in query_scores {
                *scores.entry(phase).or_insert(0.0) += score * 1.2; // Weight: 1.2 (highest)
            }
        }

        // Select phase with highest score, fallback to Implementation if no signals
        scores
            .into_iter()
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(phase, _)| phase)
            .unwrap_or(SdlcPhase::Implementation)
    }

    /// Analyze recent git commits for phase signals.
    fn analyze_recent_commits(&self) -> Result<Vec<(SdlcPhase, f32)>, std::io::Error> {
        let output = std::process::Command::new("git")
            .args(["log", "-20", "--oneline", "--no-decorate"])
            .current_dir(&self.working_dir)
            .output()?;

        if !output.status.success() {
            return Ok(vec![]);
        }

        let log_text = String::from_utf8_lossy(&output.stdout).to_lowercase();
        Ok(self.score_text_by_keywords(&log_text))
    }

    /// Analyze file patterns in working directory.
    fn analyze_file_patterns(&self) -> Result<Vec<(SdlcPhase, f32)>, std::io::Error> {
        let mut scores: HashMap<SdlcPhase, f32> = HashMap::new();

        // Walk directory (max depth 3 to avoid deep traversal)
        if let Ok(entries) = std::fs::read_dir(&self.working_dir) {
            for entry in entries.flatten() {
                if let Some(ext) = entry.path().extension() {
                    if let Some(ext_str) = ext.to_str() {
                        if let Some(phase_weights) = self.extension_weights.get(ext_str) {
                            for (phase, weight) in phase_weights {
                                *scores.entry(*phase).or_insert(0.0) += weight;
                            }
                        }
                    }
                }

                // Check specific filenames
                if let Some(filename) = entry.file_name().to_str() {
                    let fname_lower = filename.to_lowercase();
                    if fname_lower.contains("readme") || fname_lower.contains("requirements") {
                        *scores.entry(SdlcPhase::Discovery).or_insert(0.0) += 1.5;
                    }
                    if fname_lower.contains("architecture") || fname_lower.contains("design") {
                        *scores.entry(SdlcPhase::Planning).or_insert(0.0) += 1.5;
                    }
                    if fname_lower.contains("test") {
                        *scores.entry(SdlcPhase::Testing).or_insert(0.0) += 1.0;
                    }
                    if fname_lower.contains("deploy") || fname_lower == "dockerfile" {
                        *scores.entry(SdlcPhase::Deployment).or_insert(0.0) += 1.5;
                    }
                    if fname_lower.contains("changelog") {
                        *scores.entry(SdlcPhase::Review).or_insert(0.0) += 1.0;
                    }
                }
            }
        }

        Ok(scores.into_iter().collect())
    }

    /// Analyze user query for phase keywords.
    fn analyze_query_keywords(&self, query: &str) -> Vec<(SdlcPhase, f32)> {
        self.score_text_by_keywords(&query.to_lowercase())
    }

    /// Score text by matching keywords.
    fn score_text_by_keywords(&self, text: &str) -> Vec<(SdlcPhase, f32)> {
        let mut scores: HashMap<SdlcPhase, f32> = HashMap::new();

        for (keyword, phase_weights) in &self.keyword_weights {
            if text.contains(keyword) {
                for (phase, weight) in phase_weights {
                    *scores.entry(*phase).or_insert(0.0) += weight;
                }
            }
        }

        scores.into_iter().collect()
    }

    /// Build file extension -> phase weights.
    fn build_extension_weights() -> HashMap<&'static str, Vec<(SdlcPhase, f32)>> {
        let mut map = HashMap::new();

        // Code files → Implementation
        for ext in &["rs", "py", "js", "ts", "go", "java", "cpp", "c", "rb", "php"] {
            map.insert(*ext, vec![(SdlcPhase::Implementation, 1.0)]);
        }

        // Test files → Testing
        for ext in &["test.rs", "test.py", "test.js", "test.ts", "spec.ts", "spec.js"] {
            map.insert(*ext, vec![(SdlcPhase::Testing, 1.5)]);
        }

        // Documentation → Discovery/Planning
        for ext in &["md", "txt", "adoc", "rst"] {
            map.insert(
                *ext,
                vec![
                    (SdlcPhase::Discovery, 0.5),
                    (SdlcPhase::Planning, 0.5),
                ],
            );
        }

        // Config files → Deployment
        for ext in &["yaml", "yml", "toml", "json"] {
            map.insert(*ext, vec![(SdlcPhase::Deployment, 0.3)]);
        }

        // Dockerfiles, manifests → Deployment
        map.insert("dockerfile", vec![(SdlcPhase::Deployment, 2.0)]);

        map
    }

    /// Build keyword -> phase weights.
    fn build_keyword_weights() -> HashMap<&'static str, Vec<(SdlcPhase, f32)>> {
        let mut map = HashMap::new();

        // Discovery keywords
        for kw in &[
            "requirements",
            "requirement",
            "user story",
            "feature request",
            "roadmap",
            "discovery",
            "research",
        ] {
            map.insert(*kw, vec![(SdlcPhase::Discovery, 2.0)]);
        }

        // Planning keywords
        for kw in &[
            "architecture",
            "design",
            "plan",
            "planning",
            "task",
            "backlog",
            "sprint",
        ] {
            map.insert(*kw, vec![(SdlcPhase::Planning, 2.0)]);
        }

        // Implementation keywords
        for kw in &[
            "implement",
            "code",
            "coding",
            "develop",
            "feature",
            "function",
            "class",
            "method",
        ] {
            map.insert(*kw, vec![(SdlcPhase::Implementation, 2.0)]);
        }

        // Testing keywords
        for kw in &["test", "testing", "qa", "quality", "coverage", "ci"] {
            map.insert(*kw, vec![(SdlcPhase::Testing, 2.0)]);
        }

        // Review keywords
        for kw in &["review", "pr", "pull request", "merge", "changelog"] {
            map.insert(*kw, vec![(SdlcPhase::Review, 2.0)]);
        }

        // Deployment keywords
        for kw in &[
            "deploy",
            "deployment",
            "release",
            "production",
            "staging",
            "docker",
            "kubernetes",
            "k8s",
        ] {
            map.insert(*kw, vec![(SdlcPhase::Deployment, 2.0)]);
        }

        // Monitoring keywords
        for kw in &[
            "monitor",
            "monitoring",
            "metrics",
            "observability",
            "prometheus",
            "grafana",
            "logging",
            "tracing",
        ] {
            map.insert(*kw, vec![(SdlcPhase::Monitoring, 2.0)]);
        }

        // Support keywords
        for kw in &[
            "bug",
            "issue",
            "hotfix",
            "support",
            "incident",
            "troubleshoot",
        ] {
            map.insert(*kw, vec![(SdlcPhase::Support, 2.0)]);
        }

        map
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detector_creation() {
        let detector = SdlcPhaseDetector::new("/nonexistent/test/dir");
        assert_eq!(detector.working_dir, Path::new("/nonexistent/test/dir"));
    }

    #[test]
    fn test_detect_phase_discovery_keywords() {
        // Use nonexistent directory to avoid filesystem signal bias
        let detector = SdlcPhaseDetector::new("/nonexistent/test/discovery");
        let phase = detector.detect_phase(Some("What are the requirements for this feature?"));
        assert_eq!(phase, SdlcPhase::Discovery);
    }

    #[test]
    fn test_detect_phase_planning_keywords() {
        let detector = SdlcPhaseDetector::new("/nonexistent/test/planning");
        let phase = detector.detect_phase(Some("Let's design the architecture for the new module"));
        assert_eq!(phase, SdlcPhase::Planning);
    }

    #[test]
    fn test_detect_phase_implementation_keywords() {
        let detector = SdlcPhaseDetector::new("/nonexistent/test/implementation");
        let phase = detector.detect_phase(Some("Implement the authentication feature"));
        assert_eq!(phase, SdlcPhase::Implementation);
    }

    #[test]
    fn test_detect_phase_testing_keywords() {
        let detector = SdlcPhaseDetector::new("/nonexistent/test/testing");
        let phase = detector.detect_phase(Some("Run tests and check coverage"));
        assert_eq!(phase, SdlcPhase::Testing);
    }

    #[test]
    fn test_detect_phase_review_keywords() {
        let detector = SdlcPhaseDetector::new("/nonexistent/test/review");
        let phase = detector.detect_phase(Some("Review the pull request"));
        assert_eq!(phase, SdlcPhase::Review);
    }

    #[test]
    fn test_detect_phase_deployment_keywords() {
        let detector = SdlcPhaseDetector::new("/nonexistent/test/deployment");
        let phase = detector.detect_phase(Some("Deploy to production"));
        assert_eq!(phase, SdlcPhase::Deployment);
    }

    #[test]
    fn test_detect_phase_monitoring_keywords() {
        let detector = SdlcPhaseDetector::new("/nonexistent/test/monitoring");
        let phase = detector.detect_phase(Some("Check prometheus metrics"));
        assert_eq!(phase, SdlcPhase::Monitoring);
    }

    #[test]
    fn test_detect_phase_support_keywords() {
        let detector = SdlcPhaseDetector::new("/nonexistent/test/support");
        let phase = detector.detect_phase(Some("Fix this critical bug"));
        assert_eq!(phase, SdlcPhase::Support);
    }

    #[test]
    fn test_detect_phase_no_query_defaults_to_implementation() {
        let detector = SdlcPhaseDetector::new("/nonexistent/test/default");
        let phase = detector.detect_phase(None);
        // Without any signals, should default to Implementation
        assert_eq!(phase, SdlcPhase::Implementation);
    }

    #[test]
    fn test_score_text_by_keywords() {
        let detector = SdlcPhaseDetector::new("/tmp");
        let scores = detector.score_text_by_keywords("let's write tests for this feature");

        // Should have Testing phase with high score
        let testing_score = scores
            .iter()
            .find(|(phase, _)| *phase == SdlcPhase::Testing)
            .map(|(_, score)| *score)
            .unwrap_or(0.0);

        assert!(testing_score > 0.0, "Expected Testing phase score > 0");
    }

    #[test]
    fn test_keyword_weights_include_all_phases() {
        let weights = SdlcPhaseDetector::build_keyword_weights();

        // Verify we have keywords for all phases
        let mut phases_covered = std::collections::HashSet::new();
        for phase_weights in weights.values() {
            for (phase, _) in phase_weights {
                phases_covered.insert(*phase);
            }
        }

        assert_eq!(phases_covered.len(), 8, "Should have keywords for all 8 SDLC phases");
    }

    #[test]
    fn test_extension_weights_implementation() {
        let weights = SdlcPhaseDetector::build_extension_weights();

        // Rust files should map to Implementation
        let rs_weights = weights.get("rs").unwrap();
        assert_eq!(rs_weights.len(), 1);
        assert_eq!(rs_weights[0].0, SdlcPhase::Implementation);
    }
}
