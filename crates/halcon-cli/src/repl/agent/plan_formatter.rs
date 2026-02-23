use halcon_core::traits::{ExecutionPlan, StepOutcome};

// Plan injection markers for surgical replacement in the system prompt.
pub(super) const PLAN_SECTION_START: &str = "<!-- HALCON_PLAN_START -->";
pub(super) const PLAN_SECTION_END: &str = "<!-- HALCON_PLAN_END -->";

/// Format an execution plan as a system prompt section.
///
/// Renders the plan with step statuses (done/failed/current/pending) and
/// a directive telling the model which step to execute next.
pub(crate) fn format_plan_for_prompt(plan: &ExecutionPlan, current_step: usize) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(out, "{PLAN_SECTION_START}");
    let _ = writeln!(out);
    let _ = writeln!(out, "## Active Execution Plan");
    let _ = writeln!(out);
    let _ = writeln!(out, "**Goal**: {}", plan.goal);
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "Follow these steps in order. Execute the current step, then proceed to the next."
    );
    let _ = writeln!(out);

    for (i, step) in plan.steps.iter().enumerate() {
        let tool_hint = step
            .tool_name
            .as_deref()
            .map(|t| format!(" (tool: {t})"))
            .unwrap_or_default();
        let (icon, marker) = match &step.outcome {
            Some(StepOutcome::Success { .. }) => ("\u{2713}", ""),       // ✓
            Some(StepOutcome::Failed { .. }) => ("\u{2717}", ""),        // ✗
            Some(StepOutcome::Skipped { .. }) => ("-", ""),
            None if i == current_step => ("\u{25b8}", " \u{2190} CURRENT"), // ▸ ← CURRENT
            None => ("\u{25cb}", ""),                                     // ○
        };
        let _ = writeln!(
            out,
            "  {icon}  Step {}: {}{tool_hint}{marker}",
            i + 1,
            step.description
        );
    }

    let _ = writeln!(out);
    if current_step < plan.steps.len() {
        let step = &plan.steps[current_step];
        let _ = writeln!(
            out,
            "You are on Step {}. Execute: {}",
            current_step + 1,
            step.description
        );
        if let Some(ref args) = step.expected_args {
            let _ = writeln!(out, "Expected args: {args}");
        }
    } else {
        let _ = writeln!(out, "All steps completed.");
    }

    let _ = writeln!(out);
    let _ = write!(out, "{PLAN_SECTION_END}");
    out
}

/// Surgically replace the plan section in a system prompt string.
/// If no plan section exists, appends it.
pub(crate) fn update_plan_in_system(system: &mut String, plan_section: &str) {
    if let Some(start) = system.find(PLAN_SECTION_START) {
        if let Some(end) = system.find(PLAN_SECTION_END) {
            let end = end + PLAN_SECTION_END.len();
            system.replace_range(start..end, plan_section);
            return;
        }
    }
    // No existing section — append.
    system.push_str("\n\n");
    system.push_str(plan_section);
}

/// Validate a plan before execution to catch errors early.
///
/// Checks:
/// - All tools referenced in plan steps exist in the tool registry
/// - No invalid tool names
///
/// Returns list of validation warnings (empty = valid plan).
pub(super) fn validate_plan(
    plan: &ExecutionPlan,
    tool_registry: &halcon_tools::ToolRegistry,
) -> Vec<String> {
    let mut warnings = Vec::new();

    // Check each step's tool reference.
    for (idx, step) in plan.steps.iter().enumerate() {
        if let Some(ref tool_name) = step.tool_name {
            // Verify tool exists in registry.
            if tool_registry.get(tool_name).is_none() {
                warnings.push(format!(
                    "Step {}: tool '{}' not found in registry ({})",
                    idx + 1,
                    tool_name,
                    step.description
                ));
            }
        }
    }

    // Check for empty plan (suspicious, but not an error).
    if plan.steps.is_empty() {
        warnings.push("Plan has 0 steps — may be a planning failure".to_string());
    }

    warnings
}
