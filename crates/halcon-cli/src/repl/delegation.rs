//! Delegation router: maps plan steps to sub-agent tasks for orchestrator execution.
//!
//! Analyzes an `ExecutionPlan` and decides which steps can be delegated to
//! specialized sub-agents via the existing `run_orchestrator()` infrastructure.

use std::collections::HashSet;

use uuid::Uuid;

use halcon_core::traits::{ExecutionPlan, PlanStep};
use halcon_core::types::{AgentType, SubAgentTask};

/// Capability profile for routing plan steps to appropriate sub-agents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StepCapability {
    /// file_read, file_write, file_edit, file_delete, file_inspect, directory_tree
    FileOperations,
    /// bash
    CodeExecution,
    /// grep, glob, fuzzy_find, symbol_search
    Search,
    /// git_status, git_diff, git_log, git_add, git_commit
    GitOperations,
    /// web_search, web_fetch, http_request
    WebAccess,
    /// No specific capability needed (synthesis, general reasoning).
    General,
}

/// Delegation decision for a single plan step.
pub(crate) struct DelegationDecision {
    /// Whether this step should be delegated.
    #[allow(dead_code)]
    pub delegate: bool,
    /// Detected capability category.
    pub capability: StepCapability,
    /// Tool names suggested for the sub-agent's `allowed_tools`.
    pub suggested_tools: HashSet<String>,
    /// Human-readable reason for the decision.
    #[allow(dead_code)]
    pub reason: String,
}

/// Routes plan steps to sub-agent tasks based on capability matching heuristics.
pub(crate) struct DelegationRouter {
    /// Minimum confidence threshold to consider delegation.
    min_confidence: f64,
    /// Whether delegation is enabled.
    enabled: bool,
}

impl DelegationRouter {
    pub fn new(enabled: bool) -> Self {
        Self {
            min_confidence: 0.7,
            enabled,
        }
    }

    pub fn with_min_confidence(mut self, confidence: f64) -> Self {
        self.min_confidence = confidence;
        self
    }

    /// Analyze a plan and decide which steps should be delegated.
    ///
    /// Returns `(step_index, DelegationDecision)` pairs for delegatable steps.
    pub fn analyze_plan(&self, plan: &ExecutionPlan) -> Vec<(usize, DelegationDecision)> {
        if !self.enabled {
            return Vec::new();
        }

        // Don't delegate plans with fewer than 2 steps — a single-step plan has no
        // parallelism benefit and is cheaper to run inline.
        if plan.steps.len() < 2 {
            return Vec::new();
        }

        let last_index = plan.steps.len().saturating_sub(1);

        plan.steps
            .iter()
            .enumerate()
            .filter_map(|(i, step)| {
                // Skip synthesis steps (last step with no tool_name).
                if i == last_index && step.tool_name.is_none() {
                    return None;
                }

                // Must have a specific tool_name.
                let tool_name = step.tool_name.as_deref()?;

                // Must meet confidence threshold.
                if step.confidence < self.min_confidence {
                    return None;
                }

                // Already has an outcome — skip.
                if step.outcome.is_some() {
                    return None;
                }

                let capability = Self::classify_step(step);
                let suggested_tools = Self::tools_for_capability(&capability, tool_name);

                Some((
                    i,
                    DelegationDecision {
                        delegate: true,
                        capability,
                        suggested_tools,
                        reason: format!("tool '{tool_name}' eligible for delegation"),
                    },
                ))
            })
            .collect()
    }

    /// Convert delegation decisions to `SubAgentTask`s for the orchestrator.
    ///
    /// Returns `(step_index, SubAgentTask)` pairs preserving the step→task mapping.
    pub fn build_tasks(
        &self,
        plan: &ExecutionPlan,
        decisions: &[(usize, DelegationDecision)],
        parent_model: &str,
    ) -> Vec<(usize, SubAgentTask)> {
        // Pre-compute task IDs for dependency resolution.
        let task_ids: Vec<(usize, Uuid)> = decisions
            .iter()
            .map(|(idx, _)| (*idx, Uuid::new_v4()))
            .collect();

        decisions
            .iter()
            .enumerate()
            .filter_map(|(di, (step_idx, decision))| {
                // Audit fix: bounds-check before indexing to avoid panic on stale decisions.
                let Some(step) = plan.steps.get(*step_idx) else {
                    tracing::warn!(
                        step_idx = *step_idx,
                        total_steps = plan.steps.len(),
                        "build_tasks: step index out of bounds — skipping delegation decision"
                    );
                    return None;
                };

                // Determine dependencies: sequential steps depend on the previous delegated step.
                let depends_on = if !step.parallel && di > 0 {
                    vec![task_ids[di - 1].1]
                } else {
                    vec![]
                };

                // Prefix instruction with explicit tool-use directive so the LLM
                // calls the tool immediately instead of describing what it will do.
                // Without this, models like deepseek-chat return planning text first
                // (end_turn), which gets cached and permanently blocks tool execution.
                //
                // For file_write: include the target file path so the sub-agent knows
                // WHERE to write. Without a path, models generate the content as text
                // (end_turn) instead of calling file_write, resulting in 0 tools used.
                let instruction = if let Some(ref tool) = step.tool_name {
                    let path_hint = if tool == "file_write" {
                        // Try to get path from expected_args first.
                        let path = step.expected_args
                            .as_ref()
                            .and_then(|a| a.get("path").and_then(|p| p.as_str()).map(|s| s.to_string()))
                            .unwrap_or_else(|| Self::infer_file_path(&step.description));
                        format!(
                            "\nTarget file path: {path}\n\
                             Call file_write with path=\"{path}\" and the complete file content."
                        )
                    } else {
                        String::new()
                    };
                    format!(
                        "IMPORTANT: Call the `{tool}` tool NOW to complete this task. \
                         Do NOT describe, plan, or explain — execute the tool immediately.{path_hint}\n\n\
                         Task: {}",
                        step.description
                    )
                } else {
                    step.description.clone()
                };

                let task = SubAgentTask {
                    task_id: task_ids[di].1,
                    instruction,
                    agent_type: Self::agent_type_for_capability(&decision.capability),
                    model: Some(parent_model.to_string()),
                    provider: None,
                    allowed_tools: decision.suggested_tools.clone(),
                    limits_override: None,
                    depends_on,
                    priority: 0,
                };

                Some((*step_idx, task))
            })
            .collect()
    }

    /// Classify a plan step's capability from its `tool_name`.
    fn classify_step(step: &PlanStep) -> StepCapability {
        let tool = match step.tool_name.as_deref() {
            Some(t) => t,
            None => return StepCapability::General,
        };

        match tool {
            "file_read" | "file_write" | "file_edit" | "file_delete" | "file_inspect"
            | "directory_tree" => StepCapability::FileOperations,
            "bash" => StepCapability::CodeExecution,
            "grep" | "glob" | "fuzzy_find" | "symbol_search" => StepCapability::Search,
            "git_status" | "git_diff" | "git_log" | "git_add" | "git_commit" => {
                StepCapability::GitOperations
            }
            "web_search" | "web_fetch" | "http_request" => StepCapability::WebAccess,
            _ => StepCapability::General,
        }
    }

    /// Suggest the set of tools a sub-agent needs for a given capability.
    fn tools_for_capability(capability: &StepCapability, primary_tool: &str) -> HashSet<String> {
        let mut tools = HashSet::new();
        tools.insert(primary_tool.to_string());

        match capability {
            StepCapability::FileOperations => {
                // File tools often need bash for verification.
                tools.insert("file_read".into());
            }
            StepCapability::CodeExecution => {
                // bash is self-contained.
            }
            StepCapability::Search => {
                // Search tools are self-contained.
            }
            StepCapability::GitOperations => {
                // Git ops may need related tools.
                tools.insert("git_status".into());
            }
            StepCapability::WebAccess => {
                // Web tools are self-contained.
            }
            StepCapability::General => {}
        }

        tools
    }

    /// Infer a reasonable output file path from a step description.
    ///
    /// Used when no explicit `expected_args.path` is set for a `file_write` step.
    /// Checks for common file type keywords and returns a sensible default name.
    fn infer_file_path(description: &str) -> String {
        let lower = description.to_lowercase();
        if lower.contains(".html") || lower.contains("html") || lower.contains("web page") || lower.contains("webpage") {
            "output.html".to_string()
        } else if lower.contains(".py") || lower.contains("python script") || lower.contains("python program") {
            "script.py".to_string()
        } else if lower.contains(".js") || lower.contains("javascript") {
            "script.js".to_string()
        } else if lower.contains(".ts") || lower.contains("typescript") {
            "script.ts".to_string()
        } else if lower.contains(".sh") || lower.contains("shell script") || lower.contains("bash script") {
            "script.sh".to_string()
        } else if lower.contains(".md") || lower.contains("markdown") || lower.contains("readme") {
            "README.md".to_string()
        } else if lower.contains(".json") || lower.contains("json file") {
            "output.json".to_string()
        } else if lower.contains(".rs") || lower.contains("rust") {
            "main.rs".to_string()
        } else if lower.contains(".toml") || lower.contains("config") {
            "config.toml".to_string()
        } else if lower.contains(".txt") || lower.contains("text file") {
            "output.txt".to_string()
        } else {
            "output.txt".to_string()
        }
    }

    /// Map capability to the most appropriate sub-agent type.
    fn agent_type_for_capability(capability: &StepCapability) -> AgentType {
        match capability {
            StepCapability::FileOperations | StepCapability::CodeExecution => AgentType::Coder,
            StepCapability::Search | StepCapability::GitOperations => AgentType::Coder,
            StepCapability::WebAccess => AgentType::Chat,
            StepCapability::General => AgentType::Chat,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use halcon_core::traits::ExecutionPlan;

    fn make_step(desc: &str, tool: Option<&str>, confidence: f64) -> PlanStep {
        PlanStep {
            step_id: uuid::Uuid::new_v4(),
            description: desc.into(),
            tool_name: tool.map(|t| t.into()),
            parallel: false,
            confidence,
            expected_args: None,
            outcome: None,
        }
    }

    fn make_parallel_step(desc: &str, tool: &str, confidence: f64) -> PlanStep {
        PlanStep {
            step_id: uuid::Uuid::new_v4(),
            description: desc.into(),
            tool_name: Some(tool.into()),
            parallel: true,
            confidence,
            expected_args: None,
            outcome: None,
        }
    }

    fn make_plan(steps: Vec<PlanStep>) -> ExecutionPlan {
        ExecutionPlan {
            goal: "Test goal".into(),
            steps,
            requires_confirmation: false,
            plan_id: Uuid::nil(),
            replan_count: 0,
            parent_plan_id: None,
        }
    }

    #[test]
    fn classify_file_read() {
        let step = make_step("Read file", Some("file_read"), 0.9);
        assert_eq!(DelegationRouter::classify_step(&step), StepCapability::FileOperations);
    }

    #[test]
    fn classify_file_write() {
        let step = make_step("Write file", Some("file_write"), 0.9);
        assert_eq!(DelegationRouter::classify_step(&step), StepCapability::FileOperations);
    }

    #[test]
    fn classify_file_edit() {
        let step = make_step("Edit file", Some("file_edit"), 0.9);
        assert_eq!(DelegationRouter::classify_step(&step), StepCapability::FileOperations);
    }

    #[test]
    fn classify_bash() {
        let step = make_step("Run command", Some("bash"), 0.9);
        assert_eq!(DelegationRouter::classify_step(&step), StepCapability::CodeExecution);
    }

    #[test]
    fn classify_grep() {
        let step = make_step("Search files", Some("grep"), 0.9);
        assert_eq!(DelegationRouter::classify_step(&step), StepCapability::Search);
    }

    #[test]
    fn classify_glob() {
        let step = make_step("Find files", Some("glob"), 0.9);
        assert_eq!(DelegationRouter::classify_step(&step), StepCapability::Search);
    }

    #[test]
    fn classify_git_status() {
        let step = make_step("Check status", Some("git_status"), 0.9);
        assert_eq!(DelegationRouter::classify_step(&step), StepCapability::GitOperations);
    }

    #[test]
    fn classify_git_diff() {
        let step = make_step("Show diff", Some("git_diff"), 0.9);
        assert_eq!(DelegationRouter::classify_step(&step), StepCapability::GitOperations);
    }

    #[test]
    fn classify_web_search() {
        let step = make_step("Search web", Some("web_search"), 0.9);
        assert_eq!(DelegationRouter::classify_step(&step), StepCapability::WebAccess);
    }

    #[test]
    fn classify_none_tool() {
        let step = make_step("Synthesize", None, 1.0);
        assert_eq!(DelegationRouter::classify_step(&step), StepCapability::General);
    }

    #[test]
    fn classify_unknown_tool() {
        let step = make_step("Custom", Some("custom_tool"), 0.9);
        assert_eq!(DelegationRouter::classify_step(&step), StepCapability::General);
    }

    #[test]
    fn analyze_plan_empty() {
        let router = DelegationRouter::new(true);
        let plan = make_plan(vec![]);
        let decisions = router.analyze_plan(&plan);
        assert!(decisions.is_empty());
    }

    #[test]
    fn analyze_plan_single_step_skipped() {
        let router = DelegationRouter::new(true);
        let plan = make_plan(vec![make_step("Read file", Some("file_read"), 0.9)]);
        let decisions = router.analyze_plan(&plan);
        assert!(decisions.is_empty(), "Single-step plans should not be delegated");
    }

    #[test]
    fn analyze_plan_two_steps_delegated() {
        // Threshold lowered to ≥2 steps — two-step plans with tool_names are eligible.
        let router = DelegationRouter::new(true);
        let plan = make_plan(vec![
            make_step("Read file", Some("file_read"), 0.9),
            make_step("Edit file", Some("file_edit"), 0.9),
        ]);
        let decisions = router.analyze_plan(&plan);
        assert_eq!(decisions.len(), 2, "Two-step plans with tool_names should now be delegated");
    }

    #[test]
    fn analyze_plan_one_step_skipped() {
        // Single-step plans are still skipped (< 2 threshold).
        let router = DelegationRouter::new(true);
        let plan = make_plan(vec![make_step("Read file", Some("file_read"), 0.9)]);
        let decisions = router.analyze_plan(&plan);
        assert!(decisions.is_empty(), "Single-step plans should not be delegated");
    }

    #[test]
    fn analyze_plan_three_steps() {
        let router = DelegationRouter::new(true);
        let plan = make_plan(vec![
            make_step("Read file", Some("file_read"), 0.9),
            make_step("Edit file", Some("file_edit"), 0.8),
            make_step("Run tests", Some("bash"), 0.9),
        ]);
        let decisions = router.analyze_plan(&plan);
        assert_eq!(decisions.len(), 3);
        assert_eq!(decisions[0].0, 0);
        assert_eq!(decisions[1].0, 1);
        assert_eq!(decisions[2].0, 2);
    }

    #[test]
    fn analyze_plan_low_confidence_filtered() {
        let router = DelegationRouter::new(true).with_min_confidence(0.7);
        let plan = make_plan(vec![
            make_step("Read file", Some("file_read"), 0.9),
            make_step("Maybe edit", Some("file_edit"), 0.5), // Below threshold
            make_step("Run tests", Some("bash"), 0.9),
        ]);
        let decisions = router.analyze_plan(&plan);
        assert_eq!(decisions.len(), 2);
        assert_eq!(decisions[0].0, 0); // file_read
        assert_eq!(decisions[1].0, 2); // bash
    }

    #[test]
    fn analyze_plan_no_tool_name_skipped() {
        let router = DelegationRouter::new(true);
        let plan = make_plan(vec![
            make_step("Read file", Some("file_read"), 0.9),
            make_step("Think about it", None, 1.0), // No tool
            make_step("Run tests", Some("bash"), 0.9),
        ]);
        let decisions = router.analyze_plan(&plan);
        assert_eq!(decisions.len(), 2);
        assert!(decisions.iter().all(|(_, d)| d.delegate));
    }

    #[test]
    fn analyze_plan_synthesis_step_skipped() {
        let router = DelegationRouter::new(true);
        let plan = make_plan(vec![
            make_step("Read file", Some("file_read"), 0.9),
            make_step("Edit file", Some("file_edit"), 0.9),
            make_step("Summarize changes", None, 1.0), // Last step, no tool = synthesis
        ]);
        let decisions = router.analyze_plan(&plan);
        assert_eq!(decisions.len(), 2);
        // Last synthesis step is excluded.
        assert!(decisions.iter().all(|(idx, _)| *idx < 2));
    }

    #[test]
    fn router_disabled_returns_empty() {
        let router = DelegationRouter::new(false);
        let plan = make_plan(vec![
            make_step("Read file", Some("file_read"), 0.9),
            make_step("Edit file", Some("file_edit"), 0.9),
            make_step("Run tests", Some("bash"), 0.9),
        ]);
        let decisions = router.analyze_plan(&plan);
        assert!(decisions.is_empty());
    }

    #[test]
    fn build_tasks_maps_correctly() {
        let router = DelegationRouter::new(true);
        let plan = make_plan(vec![
            make_step("Read file", Some("file_read"), 0.9),
            make_step("Edit file", Some("file_edit"), 0.8),
            make_step("Run tests", Some("bash"), 0.9),
        ]);
        let decisions = router.analyze_plan(&plan);
        let tasks = router.build_tasks(&plan, &decisions, "deepseek-chat");

        assert_eq!(tasks.len(), 3);
        // Step indices preserved.
        assert_eq!(tasks[0].0, 0);
        assert_eq!(tasks[1].0, 1);
        assert_eq!(tasks[2].0, 2);
        // Task IDs are unique.
        let ids: HashSet<_> = tasks.iter().map(|(_, t)| t.task_id).collect();
        assert_eq!(ids.len(), 3);
        // Model inherited.
        assert_eq!(tasks[0].1.model.as_deref(), Some("deepseek-chat"));
        // Instructions are prefixed with tool-use directive to force immediate tool execution.
        assert!(tasks[0].1.instruction.starts_with("IMPORTANT: Call the `file_read` tool NOW"));
        assert!(tasks[0].1.instruction.contains("Task: Read file"));
        assert!(tasks[2].1.instruction.starts_with("IMPORTANT: Call the `bash` tool NOW"));
        assert!(tasks[2].1.instruction.contains("Task: Run tests"));
        // Agent types mapped correctly.
        assert_eq!(tasks[0].1.agent_type, AgentType::Coder); // FileOperations
        assert_eq!(tasks[2].1.agent_type, AgentType::Coder); // CodeExecution
    }

    #[test]
    fn build_tasks_sequential_dependencies() {
        let router = DelegationRouter::new(true);
        let plan = make_plan(vec![
            make_step("Read file", Some("file_read"), 0.9),
            make_step("Edit file", Some("file_edit"), 0.9),
            make_step("Run tests", Some("bash"), 0.9),
        ]);
        let decisions = router.analyze_plan(&plan);
        let tasks = router.build_tasks(&plan, &decisions, "test-model");

        // First task has no deps.
        assert!(tasks[0].1.depends_on.is_empty());
        // Second depends on first.
        assert_eq!(tasks[1].1.depends_on, vec![tasks[0].1.task_id]);
        // Third depends on second.
        assert_eq!(tasks[2].1.depends_on, vec![tasks[1].1.task_id]);
    }

    #[test]
    fn build_tasks_parallel_no_dependency() {
        let router = DelegationRouter::new(true);
        let plan = make_plan(vec![
            make_step("Read file A", Some("file_read"), 0.9),
            make_parallel_step("Read file B", "file_read", 0.9),
            make_parallel_step("Read file C", "file_read", 0.9),
        ]);
        let decisions = router.analyze_plan(&plan);
        let tasks = router.build_tasks(&plan, &decisions, "test-model");

        assert_eq!(tasks.len(), 3);
        // First has no deps.
        assert!(tasks[0].1.depends_on.is_empty());
        // Parallel steps have no deps (parallel: true).
        assert!(tasks[1].1.depends_on.is_empty());
        assert!(tasks[2].1.depends_on.is_empty());
    }

    #[test]
    fn delegation_decision_includes_suggested_tools() {
        let router = DelegationRouter::new(true);
        let plan = make_plan(vec![
            make_step("Read file", Some("file_read"), 0.9),
            make_step("Run tests", Some("bash"), 0.9),
            make_step("Search code", Some("grep"), 0.9),
        ]);
        let decisions = router.analyze_plan(&plan);

        // file_read → FileOperations → includes file_read.
        assert!(decisions[0].1.suggested_tools.contains("file_read"));
        // bash → CodeExecution → includes bash.
        assert!(decisions[1].1.suggested_tools.contains("bash"));
        // grep → Search → includes grep.
        assert!(decisions[2].1.suggested_tools.contains("grep"));
    }

    // ── Audit fix: bounds-checked build_tasks ────────────────────────────────

    /// Out-of-bounds step_idx in decisions must not panic — bad entry is skipped.
    /// Simulated by using a plan with fewer steps than the decision's step_idx references.
    #[test]
    fn build_tasks_oob_step_idx_skipped_not_panic() {
        let router = DelegationRouter::new(true);
        // Plan has 3 steps (indices 0-2), used to generate valid decisions.
        let plan_full = make_plan(vec![
            make_step("Read file", Some("file_read"), 0.9),
            make_step("Run tests", Some("bash"), 0.9),
            make_step("Search code", Some("grep"), 0.9),
        ]);
        let decisions = router.analyze_plan(&plan_full);

        // Replay against a shorter plan (2 steps → indices 0-1).
        // Decision at step_idx=2 is now out of bounds.
        let plan_short = make_plan(vec![
            make_step("Read file", Some("file_read"), 0.9),
            make_step("Run tests", Some("bash"), 0.9),
        ]);
        // Must not panic — OOB decision is skipped; only 2 tasks produced.
        let tasks = router.build_tasks(&plan_short, &decisions, "test-model");
        assert_eq!(tasks.len(), 2, "OOB decision must be silently skipped");
        assert!(
            tasks.iter().all(|(idx, _)| *idx < 2),
            "all produced tasks must have valid step indices"
        );
    }

    /// All decisions OOB — build_tasks returns empty vec without panicking.
    #[test]
    fn build_tasks_all_oob_returns_empty() {
        let router = DelegationRouter::new(true);
        let plan_full = make_plan(vec![
            make_step("Read file", Some("file_read"), 0.9),
            make_step("Run tests", Some("bash"), 0.9),
            make_step("Search code", Some("grep"), 0.9),
        ]);
        let decisions = router.analyze_plan(&plan_full);

        // Empty plan — every decision (indices 0, 1, 2) is now OOB.
        let plan_empty = make_plan(vec![]);
        let tasks = router.build_tasks(&plan_empty, &decisions, "test-model");
        assert!(tasks.is_empty(), "all-OOB decisions produce empty result");
    }

    /// Regression guard: in-bounds build_tasks still works normally after the bounds fix.
    #[test]
    fn build_tasks_inbounds_regression_guard() {
        let router = DelegationRouter::new(true);
        let plan = make_plan(vec![
            make_step("Read", Some("file_read"), 0.9),
            make_step("Edit", Some("file_edit"), 0.9),
            make_step("Test", Some("bash"), 0.9),
        ]);
        let decisions = router.analyze_plan(&plan);
        let tasks = router.build_tasks(&plan, &decisions, "model");
        assert_eq!(tasks.len(), 3);
        assert!(tasks[0].1.instruction.starts_with("IMPORTANT: Call the `file_read` tool NOW"));
        assert!(tasks[0].1.instruction.contains("Task: Read"));
        assert!(tasks[2].1.instruction.starts_with("IMPORTANT: Call the `bash` tool NOW"));
        assert!(tasks[2].1.instruction.contains("Task: Test"));
    }

    // ── Fix 2: file_write delegation path injection ───────────────────────────

    /// file_write step with explicit path in expected_args → path appears in instruction.
    #[test]
    fn file_write_with_explicit_path_uses_expected_args() {
        let router = DelegationRouter::new(true);
        let mut step = make_step("Write the HTML game to disk", Some("file_write"), 0.9);
        step.expected_args = Some(serde_json::json!({"path": "/tmp/game.html", "content": "..."}));
        let plan = make_plan(vec![
            step,
            make_step("Run tests", Some("bash"), 0.9),
        ]);
        let decisions = router.analyze_plan(&plan);
        let tasks = router.build_tasks(&plan, &decisions, "deepseek-chat");

        let file_write_task = &tasks[0].1;
        assert!(file_write_task.instruction.contains("/tmp/game.html"),
            "Instruction must contain explicit path from expected_args");
        assert!(file_write_task.instruction.contains("path=\"/tmp/game.html\""),
            "Instruction must include file_write path= directive");
    }

    /// file_write step with no expected_args but HTML description → inferred .html path.
    #[test]
    fn file_write_infers_html_path_from_description() {
        let router = DelegationRouter::new(true);
        let plan = make_plan(vec![
            make_step("Create a web page HTML game", Some("file_write"), 0.9),
            make_step("Run tests", Some("bash"), 0.9),
        ]);
        let decisions = router.analyze_plan(&plan);
        let tasks = router.build_tasks(&plan, &decisions, "deepseek-chat");

        let file_write_task = &tasks[0].1;
        assert!(file_write_task.instruction.contains("output.html"),
            "HTML description must infer .html path, got: {}", file_write_task.instruction);
    }

    /// file_write step with Python description → inferred .py path.
    #[test]
    fn file_write_infers_python_path_from_description() {
        let router = DelegationRouter::new(true);
        let plan = make_plan(vec![
            make_step("Write a Python script to sort files", Some("file_write"), 0.9),
            make_step("Run tests", Some("bash"), 0.9),
        ]);
        let decisions = router.analyze_plan(&plan);
        let tasks = router.build_tasks(&plan, &decisions, "deepseek-chat");

        let file_write_task = &tasks[0].1;
        assert!(file_write_task.instruction.contains("script.py"),
            "Python description must infer .py path, got: {}", file_write_task.instruction);
    }

    /// Non-file_write tools do NOT get a path_hint appended.
    #[test]
    fn non_file_write_tools_have_no_path_hint() {
        let router = DelegationRouter::new(true);
        let plan = make_plan(vec![
            make_step("Read data", Some("file_read"), 0.9),
            make_step("Run bash script", Some("bash"), 0.9),
        ]);
        let decisions = router.analyze_plan(&plan);
        let tasks = router.build_tasks(&plan, &decisions, "deepseek-chat");

        for (_, task) in &tasks {
            assert!(!task.instruction.contains("Target file path:"),
                "Only file_write tasks should have path hints, got: {}", task.instruction);
        }
    }

    // ── infer_file_path unit tests ────────────────────────────────────────────

    #[test]
    fn infer_html_variants() {
        assert_eq!(DelegationRouter::infer_file_path("create an HTML game"), "output.html");
        assert_eq!(DelegationRouter::infer_file_path("write a web page"), "output.html");
        assert_eq!(DelegationRouter::infer_file_path("build a .html file"), "output.html");
    }

    #[test]
    fn infer_python_variants() {
        assert_eq!(DelegationRouter::infer_file_path("python script to parse logs"), "script.py");
        assert_eq!(DelegationRouter::infer_file_path("write a .py program"), "script.py");
    }

    #[test]
    fn infer_shell_variants() {
        assert_eq!(DelegationRouter::infer_file_path("write a bash script"), "script.sh");
        assert_eq!(DelegationRouter::infer_file_path("create shell script"), "script.sh");
    }

    #[test]
    fn infer_default_for_unknown() {
        assert_eq!(DelegationRouter::infer_file_path("write some output"), "output.txt");
        assert_eq!(DelegationRouter::infer_file_path(""), "output.txt");
    }
}
