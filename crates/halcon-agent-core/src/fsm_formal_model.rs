//! Phase J1 — Formal FSM Model (Model-Checkable).
//!
//! Extracts the AgentFSM transition system into a pure mathematical model:
//!
//! ```text
//! S  = { Idle, Planning, Executing, Verifying, Replanning, Terminating, Converged, Error }
//! A  = { BeginPlanning, BeginExecuting, BeginVerifying, GoalReached, RequestReplan,
//!         RetryExecution, RequestTermination, EncounterError }
//! T  ⊆ S × A → S   (deterministic, total on defined pairs)
//! ```
//!
//! ## Properties verified
//!
//! | Property | Status |
//! |----------|--------|
//! | No unreachable states | PROVED by BFS |
//! | No dead non-terminal states | PROVED by inspection |
//! | Liveness (every state reaches a terminal) | PROVED by BFS |
//! | Safety (no invalid transitions) | PROVED by exhaustive table |
//! | Determinism (≤1 target per (state, action)) | PROVED by scan |
//! | All cycles pass through Executing | PROVED by DFS |

use std::collections::{HashMap, HashSet, VecDeque};

// ─── FsmModelState ────────────────────────────────────────────────────────────

/// Pure enumeration of agent states (no payload; algebraically clean).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FsmModelState {
    Idle,
    Planning,
    Executing,
    Verifying,
    Replanning,
    Terminating,
    Converged,
    Error,
}

impl FsmModelState {
    /// All states in a canonical order.
    pub const ALL: &'static [FsmModelState] = &[
        FsmModelState::Idle,
        FsmModelState::Planning,
        FsmModelState::Executing,
        FsmModelState::Verifying,
        FsmModelState::Replanning,
        FsmModelState::Terminating,
        FsmModelState::Converged,
        FsmModelState::Error,
    ];

    /// Whether this state has no valid outgoing transitions.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            FsmModelState::Terminating | FsmModelState::Converged | FsmModelState::Error
        )
    }

    /// Short label.
    pub fn label(self) -> &'static str {
        match self {
            FsmModelState::Idle => "idle",
            FsmModelState::Planning => "planning",
            FsmModelState::Executing => "executing",
            FsmModelState::Verifying => "verifying",
            FsmModelState::Replanning => "replanning",
            FsmModelState::Terminating => "terminating",
            FsmModelState::Converged => "converged",
            FsmModelState::Error => "error",
        }
    }
}

// ─── FsmAction ────────────────────────────────────────────────────────────────

/// Every distinct trigger that causes a state transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FsmAction {
    /// Idle → Planning, Replanning → Planning
    BeginPlanning,
    /// Planning → Executing
    BeginExecuting,
    /// Executing → Verifying
    BeginVerifying,
    /// Verifying → Converged
    GoalReached,
    /// Verifying → Replanning
    RequestReplan,
    /// Verifying → Executing (continue without replan)
    RetryExecution,
    /// Any non-terminal → Terminating
    RequestTermination,
    /// Any non-terminal → Error
    EncounterError,
}

impl FsmAction {
    pub const ALL: &'static [FsmAction] = &[
        FsmAction::BeginPlanning,
        FsmAction::BeginExecuting,
        FsmAction::BeginVerifying,
        FsmAction::GoalReached,
        FsmAction::RequestReplan,
        FsmAction::RetryExecution,
        FsmAction::RequestTermination,
        FsmAction::EncounterError,
    ];
}

// ─── Transition table ─────────────────────────────────────────────────────────

/// Canonical deterministic transition table  `T ⊆ S × A → S`.
///
/// Derived directly from `crates/halcon-agent-core/src/fsm.rs::is_valid_transition`.
/// Any update to the runtime FSM **must** be reflected here.
pub const TRANSITION_TABLE: &[(FsmModelState, FsmAction, FsmModelState)] = &[
    // ── Idle
    (
        FsmModelState::Idle,
        FsmAction::BeginPlanning,
        FsmModelState::Planning,
    ),
    (
        FsmModelState::Idle,
        FsmAction::RequestTermination,
        FsmModelState::Terminating,
    ),
    // ── Planning
    (
        FsmModelState::Planning,
        FsmAction::BeginExecuting,
        FsmModelState::Executing,
    ),
    (
        FsmModelState::Planning,
        FsmAction::RequestTermination,
        FsmModelState::Terminating,
    ),
    (
        FsmModelState::Planning,
        FsmAction::EncounterError,
        FsmModelState::Error,
    ),
    // ── Executing
    (
        FsmModelState::Executing,
        FsmAction::BeginVerifying,
        FsmModelState::Verifying,
    ),
    (
        FsmModelState::Executing,
        FsmAction::RequestTermination,
        FsmModelState::Terminating,
    ),
    (
        FsmModelState::Executing,
        FsmAction::EncounterError,
        FsmModelState::Error,
    ),
    // ── Verifying
    (
        FsmModelState::Verifying,
        FsmAction::GoalReached,
        FsmModelState::Converged,
    ),
    (
        FsmModelState::Verifying,
        FsmAction::RequestReplan,
        FsmModelState::Replanning,
    ),
    (
        FsmModelState::Verifying,
        FsmAction::RetryExecution,
        FsmModelState::Executing,
    ),
    (
        FsmModelState::Verifying,
        FsmAction::RequestTermination,
        FsmModelState::Terminating,
    ),
    (
        FsmModelState::Verifying,
        FsmAction::EncounterError,
        FsmModelState::Error,
    ),
    // ── Replanning
    (
        FsmModelState::Replanning,
        FsmAction::BeginPlanning,
        FsmModelState::Planning,
    ),
    (
        FsmModelState::Replanning,
        FsmAction::RequestTermination,
        FsmModelState::Terminating,
    ),
    (
        FsmModelState::Replanning,
        FsmAction::EncounterError,
        FsmModelState::Error,
    ),
];

// ─── Adjacency helpers ────────────────────────────────────────────────────────

/// Compute the adjacency map `state → {reachable states}` from TRANSITION_TABLE.
pub fn adjacency() -> HashMap<FsmModelState, Vec<FsmModelState>> {
    let mut map: HashMap<FsmModelState, Vec<FsmModelState>> = HashMap::new();
    for &(from, _, to) in TRANSITION_TABLE {
        map.entry(from).or_default().push(to);
    }
    map
}

// ─── Property verifiers ───────────────────────────────────────────────────────

/// Result of a model-checker property verification.
#[derive(Debug, Clone)]
pub struct PropertyResult {
    pub property: &'static str,
    pub satisfied: bool,
    pub evidence: String,
}

impl PropertyResult {
    fn pass(property: &'static str) -> Self {
        Self {
            property,
            satisfied: true,
            evidence: "OK".into(),
        }
    }
    fn fail(property: &'static str, reason: impl Into<String>) -> Self {
        Self {
            property,
            satisfied: false,
            evidence: reason.into(),
        }
    }
}

/// **P1** — No unreachable states.
///
/// BFS from Idle must visit every state in `FsmModelState::ALL`.
pub fn verify_reachability() -> PropertyResult {
    let adj = adjacency();
    let mut visited: HashSet<FsmModelState> = HashSet::new();
    let mut queue = VecDeque::new();
    queue.push_back(FsmModelState::Idle);
    visited.insert(FsmModelState::Idle);

    while let Some(s) = queue.pop_front() {
        if let Some(neighbors) = adj.get(&s) {
            for &next in neighbors {
                if visited.insert(next) {
                    queue.push_back(next);
                }
            }
        }
    }

    let unreachable: Vec<&str> = FsmModelState::ALL
        .iter()
        .filter(|&&s| !visited.contains(&s))
        .map(|s| s.label())
        .collect();

    if unreachable.is_empty() {
        PropertyResult::pass("P1: No unreachable states")
    } else {
        PropertyResult::fail(
            "P1: No unreachable states",
            format!("Unreachable: {:?}", unreachable),
        )
    }
}

/// **P2** — No dead non-terminal states.
///
/// Every state that is not terminal must have ≥1 outgoing transition.
pub fn verify_no_dead_nonterminal() -> PropertyResult {
    let adj = adjacency();
    let dead: Vec<&str> = FsmModelState::ALL
        .iter()
        .filter(|&&s| !s.is_terminal())
        .filter(|&&s| adj.get(&s).map_or(true, |v| v.is_empty()))
        .map(|s| s.label())
        .collect();

    if dead.is_empty() {
        PropertyResult::pass("P2: No dead non-terminal states")
    } else {
        PropertyResult::fail(
            "P2: No dead non-terminal states",
            format!("Dead non-terminals: {:?}", dead),
        )
    }
}

/// **P3** — Liveness: every state has a path to a terminal.
///
/// Reverse BFS from {Terminating, Converged, Error} — every state must be reached.
pub fn verify_liveness() -> PropertyResult {
    // Build reverse adjacency
    let mut rev: HashMap<FsmModelState, Vec<FsmModelState>> = HashMap::new();
    for &(from, _, to) in TRANSITION_TABLE {
        rev.entry(to).or_default().push(from);
    }

    let mut can_reach_terminal: HashSet<FsmModelState> = HashSet::new();
    let mut queue = VecDeque::new();
    for &s in FsmModelState::ALL {
        if s.is_terminal() {
            can_reach_terminal.insert(s);
            queue.push_back(s);
        }
    }
    while let Some(s) = queue.pop_front() {
        if let Some(preds) = rev.get(&s) {
            for &p in preds {
                if can_reach_terminal.insert(p) {
                    queue.push_back(p);
                }
            }
        }
    }

    let no_terminal_path: Vec<&str> = FsmModelState::ALL
        .iter()
        .filter(|&&s| !can_reach_terminal.contains(&s))
        .map(|s| s.label())
        .collect();

    if no_terminal_path.is_empty() {
        PropertyResult::pass("P3: Liveness — all states reach a terminal")
    } else {
        PropertyResult::fail(
            "P3: Liveness",
            format!("No terminal path: {:?}", no_terminal_path),
        )
    }
}

/// **P4** — Determinism: no (state, action) pair maps to two different targets.
pub fn verify_determinism() -> PropertyResult {
    let mut seen: HashMap<(FsmModelState, FsmAction), FsmModelState> = HashMap::new();
    for &(from, action, to) in TRANSITION_TABLE {
        if let Some(&existing) = seen.get(&(from, action)) {
            if existing != to {
                return PropertyResult::fail(
                    "P4: Determinism",
                    format!(
                        "({}, {:?}) → {:?} AND {:?}",
                        from.label(),
                        action,
                        existing.label(),
                        to.label()
                    ),
                );
            }
        } else {
            seen.insert((from, action), to);
        }
    }
    PropertyResult::pass("P4: Determinism — no (state,action) has two distinct targets")
}

/// **P5** — All cycles in the transition graph pass through `Executing`.
///
/// This guarantees that budget (decremented per execution round) is consumed
/// by every cycle, bounding the total number of cycles to `max_rounds`.
///
/// Method: DFS from each non-terminal, detect back edges. For each discovered
/// cycle, verify that `Executing` is on the cycle path.
pub fn verify_cycles_through_executing() -> PropertyResult {
    let adj = adjacency();

    // We enumerate all simple cycles using DFS + path tracking.
    // For bounded state space (8 states), this is exhaustive.
    fn find_cycles(
        adj: &HashMap<FsmModelState, Vec<FsmModelState>>,
        start: FsmModelState,
    ) -> Vec<Vec<FsmModelState>> {
        let mut cycles = Vec::new();
        let mut path: Vec<FsmModelState> = vec![start];
        let mut visited_in_path: HashSet<FsmModelState> = HashSet::from([start]);

        fn dfs(
            adj: &HashMap<FsmModelState, Vec<FsmModelState>>,
            path: &mut Vec<FsmModelState>,
            visited_in_path: &mut HashSet<FsmModelState>,
            cycles: &mut Vec<Vec<FsmModelState>>,
            start: FsmModelState,
        ) {
            let current = *path.last().unwrap();
            if let Some(neighbors) = adj.get(&current) {
                for &next in neighbors {
                    if next == start {
                        // Found a cycle back to start
                        cycles.push(path.clone());
                    } else if !visited_in_path.contains(&next) && !next.is_terminal() {
                        path.push(next);
                        visited_in_path.insert(next);
                        dfs(adj, path, visited_in_path, cycles, start);
                        path.pop();
                        visited_in_path.remove(&next);
                    }
                }
            }
        }

        dfs(adj, &mut path, &mut visited_in_path, &mut cycles, start);
        cycles
    }

    for &s in FsmModelState::ALL {
        if s.is_terminal() {
            continue;
        }
        let cycles = find_cycles(&adj, s);
        for cycle in &cycles {
            if !cycle.contains(&FsmModelState::Executing) {
                return PropertyResult::fail(
                    "P5: All cycles through Executing",
                    format!(
                        "Cycle found not through Executing: {:?}",
                        cycle.iter().map(|s| s.label()).collect::<Vec<_>>()
                    ),
                );
            }
        }
    }
    PropertyResult::pass("P5: All cycles pass through Executing (budget consumed per cycle)")
}

/// **P6** — Terminal states have no outgoing transitions in the table.
pub fn verify_terminal_closure() -> PropertyResult {
    for &(from, _, _) in TRANSITION_TABLE {
        if from.is_terminal() {
            return PropertyResult::fail(
                "P6: Terminal closure",
                format!(
                    "Terminal state '{}' has an outgoing transition",
                    from.label()
                ),
            );
        }
    }
    PropertyResult::pass("P6: Terminal states have no outgoing transitions")
}

/// Run all 6 property verifiers and return results.
pub fn verify_all() -> Vec<PropertyResult> {
    vec![
        verify_reachability(),
        verify_no_dead_nonterminal(),
        verify_liveness(),
        verify_determinism(),
        verify_cycles_through_executing(),
        verify_terminal_closure(),
    ]
}

// ─── BFS exhaustive exploration ───────────────────────────────────────────────

/// Result of BFS exploration from a given initial state.
#[derive(Debug, Clone)]
pub struct ExplorationResult {
    pub initial_state: FsmModelState,
    pub states_visited: Vec<FsmModelState>,
    pub transitions_taken: Vec<(FsmModelState, FsmAction, FsmModelState)>,
}

/// Exhaustive BFS over all reachable (state, action) pairs from `initial`.
pub fn exhaustive_bfs(initial: FsmModelState) -> ExplorationResult {
    let mut visited_states: HashSet<FsmModelState> = HashSet::new();
    let mut transitions_taken = Vec::new();
    let mut queue: VecDeque<FsmModelState> = VecDeque::new();

    visited_states.insert(initial);
    queue.push_back(initial);

    while let Some(state) = queue.pop_front() {
        for &(from, action, to) in TRANSITION_TABLE {
            if from == state {
                transitions_taken.push((from, action, to));
                if visited_states.insert(to) {
                    queue.push_back(to);
                }
            }
        }
    }

    let mut states_visited: Vec<FsmModelState> = visited_states.into_iter().collect();
    states_visited.sort();

    ExplorationResult {
        initial_state: initial,
        states_visited,
        transitions_taken,
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn all_props_pass() -> bool {
        verify_all().iter().all(|r| r.satisfied)
    }

    #[test]
    fn all_six_properties_satisfied() {
        for result in verify_all() {
            assert!(
                result.satisfied,
                "Property FAILED: {} — {}",
                result.property, result.evidence
            );
        }
    }

    #[test]
    fn model_has_eight_states() {
        assert_eq!(FsmModelState::ALL.len(), 8);
    }

    #[test]
    fn model_has_eight_actions() {
        assert_eq!(FsmAction::ALL.len(), 8);
    }

    #[test]
    fn transition_table_has_sixteen_entries() {
        assert_eq!(TRANSITION_TABLE.len(), 16);
    }

    #[test]
    fn all_states_reachable_from_idle() {
        let result = verify_reachability();
        assert!(result.satisfied, "{}", result.evidence);
    }

    #[test]
    fn no_dead_non_terminal_states() {
        let result = verify_no_dead_nonterminal();
        assert!(result.satisfied, "{}", result.evidence);
    }

    #[test]
    fn liveness_every_state_reaches_terminal() {
        let result = verify_liveness();
        assert!(result.satisfied, "{}", result.evidence);
    }

    #[test]
    fn transition_table_is_deterministic() {
        let result = verify_determinism();
        assert!(result.satisfied, "{}", result.evidence);
    }

    #[test]
    fn all_cycles_pass_through_executing() {
        let result = verify_cycles_through_executing();
        assert!(result.satisfied, "{}", result.evidence);
    }

    #[test]
    fn terminal_states_have_no_outgoing_transitions() {
        let result = verify_terminal_closure();
        assert!(result.satisfied, "{}", result.evidence);
    }

    #[test]
    fn bfs_from_idle_visits_all_eight_states() {
        let result = exhaustive_bfs(FsmModelState::Idle);
        assert_eq!(
            result.states_visited.len(),
            FsmModelState::ALL.len(),
            "BFS should visit all 8 states; got {:?}",
            result
                .states_visited
                .iter()
                .map(|s| s.label())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn happy_path_modeled_correctly() {
        // Idle→Planning→Executing→Verifying→Converged must all be in TRANSITION_TABLE
        let table: HashSet<(FsmModelState, FsmModelState)> =
            TRANSITION_TABLE.iter().map(|&(f, _, t)| (f, t)).collect();

        assert!(table.contains(&(FsmModelState::Idle, FsmModelState::Planning)));
        assert!(table.contains(&(FsmModelState::Planning, FsmModelState::Executing)));
        assert!(table.contains(&(FsmModelState::Executing, FsmModelState::Verifying)));
        assert!(table.contains(&(FsmModelState::Verifying, FsmModelState::Converged)));
    }

    #[test]
    fn error_reachable_from_all_active_non_terminals() {
        // Planning, Executing, Verifying, Replanning should all be able to reach Error
        let adj = adjacency();
        let active = [
            FsmModelState::Planning,
            FsmModelState::Executing,
            FsmModelState::Verifying,
            FsmModelState::Replanning,
        ];
        for &s in &active {
            let reaches_error = adj
                .get(&s)
                .map_or(false, |v| v.contains(&FsmModelState::Error));
            assert!(reaches_error, "{} should be able to reach Error", s.label());
        }
    }

    #[test]
    fn three_terminal_states_exist() {
        let terminals: Vec<FsmModelState> = FsmModelState::ALL
            .iter()
            .copied()
            .filter(|s| s.is_terminal())
            .collect();
        assert_eq!(
            terminals.len(),
            3,
            "Expected 3 terminals (Terminating, Converged, Error); got {:?}",
            terminals.iter().map(|s| s.label()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn five_non_terminal_states_exist() {
        let non_terminals: Vec<FsmModelState> = FsmModelState::ALL
            .iter()
            .copied()
            .filter(|s| !s.is_terminal())
            .collect();
        assert_eq!(
            non_terminals.len(),
            5,
            "Expected 5 non-terminals; got {}",
            non_terminals.len()
        );
    }

    #[test]
    fn verify_all_returns_all_passing() {
        assert!(all_props_pass(), "One or more model properties failed");
    }
}
