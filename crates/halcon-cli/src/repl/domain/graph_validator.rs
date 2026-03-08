//! Graph structural validator — validates `ExecutionGraph` before capability checking.
//!
//! Enforces 4 structural rules:
//! - Rule 1: Graph is acyclic (no back-edges in iterative DFS).
//! - Rule 2: No orphan nodes (every node reachable from node 0 via BFS).
//! - Rule 3: Modality consistency (`ToolUse` nodes must have `tool = Some(...)`).
//! - Rule 4: All node tools declared in `declared_tools` (skipped when empty).
//!
//! # Zero-drift guarantee
//! Linear plans (default derivation via `to_execution_graph()`) always pass:
//! - Acyclic by construction (no back-edges in i→i+1 chain).
//! - All nodes reachable (linear chain from node 0).
//! - Tool nodes set `modality = ToolUse` iff `tool = Some(...)`.
//! - `declared_tools` derived from the same `required_tools` set as node tools.
//!
//! # Integration
//! Called in `agent/mod.rs` BEFORE the Step 8.1 capability gate.

use std::collections::{HashMap, HashSet, VecDeque};

use halcon_core::types::capability_types::Modality;
use halcon_core::types::execution_graph::{ExecutionGraph, NodeId};

/// A structural violation in an `ExecutionGraph`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphViolation {
    /// A cycle was detected in the execution graph.
    CycleDetected,
    /// A node is unreachable from start node (node 0).
    OrphanNode(NodeId),
    /// A `ToolUse` node has no tool name — structurally inconsistent.
    ModalityMismatch { node_id: NodeId, tool: Option<String> },
    /// A step uses a tool not listed in `declared_tools`.
    /// Only raised when `declared_tools` is non-empty.
    ToolNotDeclared(String),
}

impl std::fmt::Display for GraphViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CycleDetected =>
                write!(f, "execution graph contains a cycle"),
            Self::OrphanNode(NodeId(id)) =>
                write!(f, "node {} is unreachable from start", id),
            Self::ModalityMismatch { node_id: NodeId(id), tool } =>
                write!(f, "node {} has ToolUse modality but tool={:?}", id, tool),
            Self::ToolNotDeclared(name) =>
                write!(f, "tool '{}' used in plan but not declared in capability descriptor", name),
        }
    }
}

/// Validates `ExecutionGraph` structural correctness before execution.
pub struct GraphValidator;

impl GraphValidator {
    /// Validate all 4 structural rules in order.
    ///
    /// Returns `Ok(())` if all rules pass.
    /// Returns `Err(violation)` on the first detected violation.
    /// Rule order: 1 (cycle) → 2 (orphan) → 3 (modality) → 4 (declared tools).
    pub fn validate(graph: &ExecutionGraph) -> Result<(), GraphViolation> {
        Self::rule1_acyclic(graph)?;
        Self::rule2_no_orphans(graph)?;
        Self::rule3_modality_consistency(graph)?;
        Self::rule4_declared_tools(graph)?;
        Ok(())
    }

    /// Rule 1: Graph must be acyclic.
    ///
    /// Iterative DFS with 3-color marking:
    /// - 0 = unvisited, 1 = in current DFS stack, 2 = fully processed.
    /// A back-edge (neighbor with color=1) indicates a cycle.
    fn rule1_acyclic(graph: &ExecutionGraph) -> Result<(), GraphViolation> {
        if graph.nodes.is_empty() {
            return Ok(());
        }

        // Build adjacency list: node_id → Vec<neighbor_node_id>.
        let mut adj: HashMap<usize, Vec<usize>> = HashMap::new();
        for node in &graph.nodes {
            adj.entry(node.id.0).or_default();
        }
        for edge in &graph.edges {
            adj.entry(edge.from.0).or_default().push(edge.to.0);
        }

        // 3-color marking.
        let mut color: HashMap<usize, u8> = HashMap::new();

        for start_node in graph.nodes.iter().map(|n| n.id.0) {
            if color.get(&start_node).copied().unwrap_or(0) != 0 {
                continue; // Already fully processed.
            }

            // Stack entries: (node_id, next_neighbor_index).
            let mut stack: Vec<(usize, usize)> = Vec::new();
            color.insert(start_node, 1);
            stack.push((start_node, 0));

            while !stack.is_empty() {
                // Peek top — copy values so we don't hold a reference across push/pop.
                let (top_node, top_idx) = {
                    let e = stack.last().unwrap();
                    (e.0, e.1)
                };
                let neighbors = adj.get(&top_node).cloned().unwrap_or_default();

                if top_idx < neighbors.len() {
                    let next = neighbors[top_idx];
                    // Advance the neighbor index on the top frame.
                    stack.last_mut().unwrap().1 += 1;

                    let c = color.get(&next).copied().unwrap_or(0);
                    if c == 1 {
                        return Err(GraphViolation::CycleDetected);
                    }
                    if c == 0 {
                        color.insert(next, 1);
                        stack.push((next, 0));
                    }
                } else {
                    // All neighbors processed — mark node done.
                    color.insert(top_node, 2);
                    stack.pop();
                }
            }
        }

        Ok(())
    }

    /// Rule 2: All nodes must be reachable from node 0.
    ///
    /// Empty graphs and single-node graphs trivially pass.
    /// Uses BFS from the first node in `graph.nodes`.
    fn rule2_no_orphans(graph: &ExecutionGraph) -> Result<(), GraphViolation> {
        if graph.nodes.len() <= 1 {
            return Ok(());
        }

        // Build adjacency list.
        let mut adj: HashMap<usize, Vec<usize>> = HashMap::new();
        for node in &graph.nodes {
            adj.entry(node.id.0).or_default();
        }
        for edge in &graph.edges {
            adj.entry(edge.from.0).or_default().push(edge.to.0);
        }

        // BFS from the first declared node.
        let start = graph.nodes[0].id.0;
        let mut visited: HashSet<usize> = HashSet::new();
        let mut queue: VecDeque<usize> = VecDeque::new();
        queue.push_back(start);
        visited.insert(start);

        while let Some(n) = queue.pop_front() {
            if let Some(nexts) = adj.get(&n) {
                for &next in nexts {
                    if visited.insert(next) {
                        queue.push_back(next);
                    }
                }
            }
        }

        for node in &graph.nodes {
            if !visited.contains(&node.id.0) {
                return Err(GraphViolation::OrphanNode(node.id));
            }
        }

        Ok(())
    }

    /// Rule 3: Every `ToolUse` node must have a non-`None` tool name.
    fn rule3_modality_consistency(graph: &ExecutionGraph) -> Result<(), GraphViolation> {
        for node in &graph.nodes {
            if node.modality == Modality::ToolUse && node.tool.is_none() {
                return Err(GraphViolation::ModalityMismatch {
                    node_id: node.id,
                    tool: None,
                });
            }
        }
        Ok(())
    }

    /// Rule 4: All tools used in nodes must appear in `declared_tools`.
    ///
    /// Skipped entirely when `declared_tools` is empty — zero-drift guarantee
    /// for plans where `derive_capability_descriptor()` has not yet been called.
    fn rule4_declared_tools(graph: &ExecutionGraph) -> Result<(), GraphViolation> {
        if graph.declared_tools.is_empty() {
            return Ok(());
        }
        let declared: HashSet<&str> =
            graph.declared_tools.iter().map(|s| s.as_str()).collect();
        for node in &graph.nodes {
            if let Some(ref tool) = node.tool {
                if !declared.contains(tool.as_str()) {
                    return Err(GraphViolation::ToolNotDeclared(tool.clone()));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use halcon_core::types::execution_graph::{ExecutionEdge, ExecutionGraph, ExecutionNode, NodeId};
    use halcon_core::types::capability_types::Modality;

    fn make_graph(
        nodes: Vec<ExecutionNode>,
        edges: Vec<ExecutionEdge>,
        declared: Vec<&str>,
    ) -> ExecutionGraph {
        ExecutionGraph {
            nodes,
            edges,
            declared_tools: declared.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn text_node(id: usize) -> ExecutionNode {
        ExecutionNode { id: NodeId(id), tool: None, modality: Modality::Text, base_cost: 0 }
    }

    fn tool_node(id: usize, tool: &str) -> ExecutionNode {
        ExecutionNode {
            id: NodeId(id),
            tool: Some(tool.to_string()),
            modality: Modality::ToolUse,
            base_cost: 0,
        }
    }

    fn edge(from: usize, to: usize) -> ExecutionEdge {
        ExecutionEdge { from: NodeId(from), to: NodeId(to) }
    }

    // ── Rule 1: Acyclicity ──────────────────────────────────────────────────────

    #[test]
    fn step9_rule1_empty_graph_passes() {
        let g = make_graph(vec![], vec![], vec![]);
        assert!(GraphValidator::validate(&g).is_ok());
    }

    #[test]
    fn step9_rule1_single_node_passes() {
        let g = make_graph(vec![text_node(0)], vec![], vec![]);
        assert!(GraphValidator::validate(&g).is_ok());
    }

    #[test]
    fn step9_rule1_linear_chain_passes() {
        let g = make_graph(
            vec![text_node(0), text_node(1), text_node(2)],
            vec![edge(0, 1), edge(1, 2)],
            vec![],
        );
        assert!(GraphValidator::validate(&g).is_ok());
    }

    #[test]
    fn step9_rule1_self_loop_detected() {
        let g = make_graph(
            vec![text_node(0)],
            vec![edge(0, 0)],
            vec![],
        );
        assert_eq!(GraphValidator::validate(&g), Err(GraphViolation::CycleDetected));
    }

    #[test]
    fn step9_rule1_back_edge_detected() {
        let g = make_graph(
            vec![text_node(0), text_node(1), text_node(2)],
            vec![edge(0, 1), edge(1, 2), edge(2, 0)],
            vec![],
        );
        assert_eq!(GraphValidator::validate(&g), Err(GraphViolation::CycleDetected));
    }

    // ── Rule 2: No orphans ──────────────────────────────────────────────────────

    #[test]
    fn step9_rule2_orphan_node_detected() {
        // Nodes 0→1 connected, node 2 isolated.
        let g = make_graph(
            vec![text_node(0), text_node(1), text_node(2)],
            vec![edge(0, 1)],
            vec![],
        );
        assert_eq!(
            GraphValidator::validate(&g),
            Err(GraphViolation::OrphanNode(NodeId(2)))
        );
    }

    #[test]
    fn step9_rule2_all_connected_passes() {
        // Node 0 fans out to 1 and 2 — both reachable.
        let g = make_graph(
            vec![text_node(0), text_node(1), text_node(2)],
            vec![edge(0, 1), edge(0, 2)],
            vec![],
        );
        assert!(GraphValidator::validate(&g).is_ok());
    }

    #[test]
    fn step9_rule2_single_node_trivially_passes() {
        let g = make_graph(vec![text_node(0)], vec![], vec![]);
        assert!(GraphValidator::validate(&g).is_ok());
    }

    // ── Rule 3: Modality consistency ─────────────────────────────────────────────

    #[test]
    fn step9_rule3_tooluse_without_tool_fails() {
        let node = ExecutionNode { id: NodeId(0), tool: None, modality: Modality::ToolUse, base_cost: 0 };
        let g = make_graph(vec![node], vec![], vec![]);
        assert!(matches!(
            GraphValidator::validate(&g),
            Err(GraphViolation::ModalityMismatch { .. })
        ));
    }

    #[test]
    fn step9_rule3_tooluse_with_tool_passes() {
        let g = make_graph(vec![tool_node(0, "file_read")], vec![], vec!["file_read"]);
        assert!(GraphValidator::validate(&g).is_ok());
    }

    #[test]
    fn step9_rule3_text_node_no_tool_passes() {
        let g = make_graph(vec![text_node(0)], vec![], vec![]);
        assert!(GraphValidator::validate(&g).is_ok());
    }

    // ── Rule 4: Declared tools ────────────────────────────────────────────────────

    #[test]
    fn step9_rule4_undeclared_tool_fails() {
        // file_write used but only file_read declared.
        let g = make_graph(
            vec![tool_node(0, "file_write")],
            vec![],
            vec!["file_read"],
        );
        assert_eq!(
            GraphValidator::validate(&g),
            Err(GraphViolation::ToolNotDeclared("file_write".to_string()))
        );
    }

    #[test]
    fn step9_rule4_empty_declared_skips_check() {
        // declared_tools is empty → Rule 4 skipped entirely (zero-drift guarantee).
        let g = make_graph(vec![tool_node(0, "any_tool")], vec![], vec![]);
        assert!(GraphValidator::validate(&g).is_ok());
    }

    #[test]
    fn step9_rule4_all_tools_declared_passes() {
        let g = make_graph(
            vec![tool_node(0, "file_read"), tool_node(1, "file_write")],
            vec![edge(0, 1)],
            vec!["file_read", "file_write"],
        );
        assert!(GraphValidator::validate(&g).is_ok());
    }

    // ── Zero-drift: linear plan always passes ──────────────────────────────────

    #[test]
    fn step9_linear_plan_always_passes() {
        // 5-step mix of text and tool nodes, linear chain, all tools declared.
        let nodes = vec![
            text_node(0),
            tool_node(1, "file_read"),
            tool_node(2, "file_write"),
            text_node(3),
            tool_node(4, "bash"),
        ];
        let edges = vec![edge(0, 1), edge(1, 2), edge(2, 3), edge(3, 4)];
        let declared = vec!["file_read", "file_write", "bash"];
        let g = make_graph(nodes, edges, declared);
        assert!(GraphValidator::validate(&g).is_ok());
    }

    // ── to_execution_graph() round-trip ──────────────────────────────────────────

    #[test]
    fn step9_to_execution_graph_zero_drift() {
        use halcon_core::traits::{ExecutionPlan, PlanStep};

        let mut plan = ExecutionPlan {
            goal: "test".into(),
            steps: vec![
                PlanStep {
                    tool_name: Some("file_read".into()),
                    description: "read".into(),
                    ..PlanStep::default()
                },
                PlanStep {
                    tool_name: None,
                    description: "think".into(),
                    ..PlanStep::default()
                },
                PlanStep {
                    tool_name: Some("file_write".into()),
                    description: "write".into(),
                    ..PlanStep::default()
                },
            ],
            ..ExecutionPlan::default()
        };
        // Derive capability descriptor so declared_tools is populated.
        plan.derive_capability_descriptor(0, 1);
        let graph = plan.to_execution_graph();

        // 3 nodes, 2 linear edges.
        assert_eq!(graph.nodes.len(), 3);
        assert_eq!(graph.edges.len(), 2);
        assert_eq!(graph.edges[0].from.0, 0);
        assert_eq!(graph.edges[0].to.0, 1);
        assert_eq!(graph.edges[1].from.0, 1);
        assert_eq!(graph.edges[1].to.0, 2);

        // Modalities derived correctly.
        assert_eq!(graph.nodes[0].modality, Modality::ToolUse); // file_read
        assert_eq!(graph.nodes[1].modality, Modality::Text);    // no tool
        assert_eq!(graph.nodes[2].modality, Modality::ToolUse); // file_write

        // Declared tools match capability descriptor.
        assert!(graph.declared_tools.contains(&"file_read".to_string()));
        assert!(graph.declared_tools.contains(&"file_write".to_string()));

        // Always passes structural validation.
        assert!(GraphValidator::validate(&graph).is_ok());
    }

    #[test]
    fn step9_empty_plan_graph_passes() {
        use halcon_core::traits::ExecutionPlan;
        let plan = ExecutionPlan::default();
        let graph = plan.to_execution_graph();
        assert_eq!(graph.nodes.len(), 0);
        assert_eq!(graph.edges.len(), 0);
        assert!(GraphValidator::validate(&graph).is_ok());
    }

    // ── Step 10: Graph Cost Propagation Engine ────────────────────────────────

    #[test]
    fn step10_text_only_graph_cost() {
        // 3 text nodes, avg=100, multiplier=2 → cost = 3 × 100 = 300 (no multiplier for Text).
        let mut g = make_graph(
            vec![text_node(0), text_node(1), text_node(2)],
            vec![edge(0, 1), edge(1, 2)],
            vec![],
        );
        g.assign_base_costs(100, 2);
        assert_eq!(g.nodes[0].base_cost, 100);
        assert_eq!(g.nodes[1].base_cost, 100);
        assert_eq!(g.nodes[2].base_cost, 100);
        assert_eq!(g.total_cost(), 300);
    }

    #[test]
    fn step10_tooluse_node_reflects_multiplier() {
        // 1 ToolUse node, avg=100, multiplier=2 → cost = 100 × 2 = 200.
        let mut g = make_graph(vec![tool_node(0, "bash")], vec![], vec!["bash"]);
        g.assign_base_costs(100, 2);
        assert_eq!(g.nodes[0].base_cost, 200);
        assert_eq!(g.total_cost(), 200);
    }

    #[test]
    fn step10_mixed_graph_sum_correct() {
        // 1 Text + 2 ToolUse, avg=100, multiplier=3.
        // Text cost = 100. ToolUse cost = 100 × 3 = 300 each.
        // Total = 100 + 300 + 300 = 700.
        let mut g = make_graph(
            vec![text_node(0), tool_node(1, "file_read"), tool_node(2, "file_write")],
            vec![edge(0, 1), edge(1, 2)],
            vec!["file_read", "file_write"],
        );
        g.assign_base_costs(100, 3);
        assert_eq!(g.nodes[0].base_cost, 100);  // Text
        assert_eq!(g.nodes[1].base_cost, 300);  // ToolUse
        assert_eq!(g.nodes[2].base_cost, 300);  // ToolUse
        assert_eq!(g.total_cost(), 700);
    }

    #[test]
    fn step10_budget_exceeded_triggers_violation() {
        use halcon_core::traits::{ExecutionPlan, PlanStep};
        use crate::repl::domain::capability_validator::{
            CapabilityValidator, CapabilityViolation, EnvironmentCapabilities,
        };
        use std::collections::HashSet;

        // 3 ToolUse steps, avg=300, multiplier=2 → cost = 3 × 300 × 2 = 1800.
        // Budget = 1000 → TokenBudgetExceeded.
        let mut plan = ExecutionPlan {
            steps: vec![
                PlanStep { tool_name: Some("file_read".into()), ..PlanStep::default() },
                PlanStep { tool_name: Some("grep".into()),      ..PlanStep::default() },
                PlanStep { tool_name: Some("bash".into()),      ..PlanStep::default() },
            ],
            ..ExecutionPlan::default()
        };
        plan.derive_capability_descriptor(300, 2); // topology-aware: 3 × 300 × 2 = 1800

        let env = EnvironmentCapabilities {
            available_tools: ["file_read", "grep", "bash"].iter()
                .map(|s| s.to_string()).collect::<HashSet<_>>(),
            supported_modalities: [Modality::Text, Modality::ToolUse].iter().copied().collect(),
            max_token_budget: 1_000, // 1800 > 1000 → violation
            known_tool_names: HashSet::new(),
        };
        assert!(matches!(
            CapabilityValidator::validate(&plan.capability_descriptor, &env),
            Err(CapabilityViolation::TokenBudgetExceeded)
        ));
    }

    #[test]
    fn step10_zero_drift_legacy_linear_plan() {
        use halcon_core::traits::{ExecutionPlan, PlanStep};

        // Zero-drift: linear plan with multiplier=1 produces same cost as flat formula.
        let mut plan = ExecutionPlan {
            steps: vec![
                PlanStep { tool_name: Some("file_read".into()), ..PlanStep::default() },
                PlanStep { tool_name: None, ..PlanStep::default() },
                PlanStep { tool_name: Some("bash".into()), ..PlanStep::default() },
            ],
            ..ExecutionPlan::default()
        };
        plan.derive_capability_descriptor(100, 1); // multiplier=1 → flat behavior

        // 2 ToolUse × 100 × 1 + 1 Text × 100 = 300.
        assert_eq!(plan.capability_descriptor.estimated_token_cost, 300);

        // Graph always passes validation.
        let graph = plan.to_execution_graph();
        assert!(GraphValidator::validate(&graph).is_ok());
    }

    #[test]
    fn step10_assign_costs_idempotent() {
        // Calling assign_base_costs twice with same params produces same result.
        let mut g = make_graph(
            vec![text_node(0), tool_node(1, "bash")],
            vec![edge(0, 1)],
            vec!["bash"],
        );
        g.assign_base_costs(200, 2);
        let cost1 = g.total_cost();
        g.assign_base_costs(200, 2);
        let cost2 = g.total_cost();
        assert_eq!(cost1, cost2);
        assert_eq!(cost1, 200 + 200 * 2); // Text=200, ToolUse=400 → 600
    }

    #[test]
    fn step10_empty_graph_total_cost_zero() {
        let g = make_graph(vec![], vec![], vec![]);
        assert_eq!(g.total_cost(), 0);
    }

    #[test]
    fn step10_topology_aware_tooluse_costs_more_than_text() {
        // For same avg, ToolUse plan costs more than text-only plan.
        let mut text_g = make_graph(
            vec![text_node(0), text_node(1)],
            vec![edge(0, 1)],
            vec![],
        );
        let mut tool_g = make_graph(
            vec![tool_node(0, "bash"), tool_node(1, "grep")],
            vec![edge(0, 1)],
            vec!["bash", "grep"],
        );
        text_g.assign_base_costs(100, 2);
        tool_g.assign_base_costs(100, 2);
        assert!(tool_g.total_cost() > text_g.total_cost(),
            "ToolUse plan must cost more than Text-only plan of equal length");
        assert_eq!(text_g.total_cost(), 200);  // 2 × 100
        assert_eq!(tool_g.total_cost(), 400);  // 2 × 100 × 2
    }
}
