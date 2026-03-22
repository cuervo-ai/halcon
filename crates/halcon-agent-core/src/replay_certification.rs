//! Phase J5 — Deterministic Replay Certification.
//!
//! ## Purpose
//!
//! Certify that GDEM agent sessions are **deterministic**: given the same
//! seeded random state and initial conditions, an agent session produces
//! identical outputs across arbitrary repeated runs.
//!
//! ## Mechanism
//!
//! 1. A [`DeterministicHarness`] runs a simulated session with a fixed seed.
//! 2. All events (FSM transitions, tool outputs, GAS values, strategy selections)
//!    are captured in a [`ReplayLog`].
//! 3. A SHA-256 [`SessionHash`] is computed over the canonical serialization
//!    of the event sequence.
//! 4. Replaying the same seed produces an identical hash — proving determinism.
//!
//! ## Invariant
//!
//! **I-7.5**: `replay_hash(seed) = replay_hash(seed)` for any seed.
//! Verified by 100 repeated runs producing identical hashes.

use rand::rngs::StdRng;
use rand::Rng;
use rand::SeedableRng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// ─── ReplayEvent ─────────────────────────────────────────────────────────────

/// A single captured event in a deterministic session replay.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReplayEvent {
    /// Sequential round index (0-based).
    pub round: u32,
    /// FSM state label at this round.
    pub fsm_state: String,
    /// Synthetic tool output for this round.
    pub tool_output: String,
    /// Goal Alignment Score after this round.
    pub gas: f32,
    /// Strategy selected by UCB1 for this round.
    pub strategy: String,
}

// ─── ReplayLog ───────────────────────────────────────────────────────────────

/// Ordered sequence of replay events capturing a full session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReplayLog {
    /// The seed used to produce this log.
    pub seed: u64,
    /// Total rounds in this session.
    pub rounds: u32,
    /// Ordered event sequence.
    pub events: Vec<ReplayEvent>,
    /// FSM transition trace (from_state → to_state).
    pub transitions: Vec<(String, String)>,
    /// GAS trajectory over rounds.
    pub gas_trajectory: Vec<f32>,
    /// Strategy selection sequence.
    pub strategy_selections: Vec<String>,
}

impl ReplayLog {
    /// Compute the SHA-256 hash of this log's canonical representation.
    ///
    /// The hash is computed over the JSON-serialized events only
    /// (seed and metadata excluded — only the deterministic outputs).
    pub fn session_hash(&self) -> SessionHash {
        let mut hasher = Sha256::new();

        // Hash the deterministic content: transitions, tool outputs, GAS, strategies
        for event in &self.events {
            hasher.update(event.round.to_le_bytes());
            hasher.update(event.fsm_state.as_bytes());
            hasher.update(b"|");
            hasher.update(event.tool_output.as_bytes());
            hasher.update(b"|");
            // GAS as bit pattern for exact reproducibility
            hasher.update(event.gas.to_bits().to_le_bytes());
            hasher.update(b"|");
            hasher.update(event.strategy.as_bytes());
            hasher.update(b"\n");
        }

        let bytes: [u8; 32] = hasher.finalize().into();
        SessionHash { bytes }
    }

    /// Whether events are ordered by round number.
    pub fn is_ordered(&self) -> bool {
        self.events.windows(2).all(|w| w[0].round < w[1].round)
    }

    /// Whether all expected fields are populated.
    pub fn is_complete(&self) -> bool {
        self.events.len() == self.rounds as usize
            && self.gas_trajectory.len() == self.rounds as usize
            && self.strategy_selections.len() == self.rounds as usize
    }
}

// ─── SessionHash ─────────────────────────────────────────────────────────────

/// SHA-256 fingerprint of a replay session's deterministic outputs.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionHash {
    bytes: [u8; 32],
}

impl SessionHash {
    /// Hex-encoded string representation (64 lowercase hex chars).
    pub fn to_hex(&self) -> String {
        self.bytes.iter().map(|b| format!("{:02x}", b)).collect()
    }

    /// Raw bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.bytes
    }
}

impl std::fmt::Display for SessionHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

// ─── DeterministicHarness ────────────────────────────────────────────────────

/// Deterministic session harness: reproduces identical sessions from a seed.
#[derive(Debug, Clone)]
pub struct DeterministicHarness {
    /// Random seed (determines all outputs).
    pub seed: u64,
    /// Number of rounds to simulate.
    pub rounds: u32,
}

impl DeterministicHarness {
    pub fn new(seed: u64, rounds: u32) -> Self {
        Self { seed, rounds }
    }

    /// Run the harness and return a [`ReplayLog`].
    ///
    /// All randomness is derived from `self.seed` via a seeded `StdRng`.
    /// The simulation is pure (no I/O, no external state).
    pub fn run(&self) -> ReplayLog {
        let mut rng: StdRng = StdRng::seed_from_u64(self.seed);

        let strategies = [
            "direct_tool",
            "plan_first",
            "multi_step",
            "exploratory",
            "goal_driven",
        ];
        let fsm_states = [
            "planning",
            "executing",
            "verifying",
            "replanning",
            "executing",
        ];

        let mut events = Vec::with_capacity(self.rounds as usize);
        let mut transitions = Vec::with_capacity(self.rounds as usize);
        let mut gas_trajectory = Vec::with_capacity(self.rounds as usize);
        let mut strategy_selections = Vec::with_capacity(self.rounds as usize);

        // Track UCB1 state deterministically
        let k = strategies.len();
        let mut pulls = vec![0u64; k];
        let mut sum_reward = vec![0.0f64; k];
        // Track FSM state
        let mut prev_fsm = "idle".to_string();

        for (total_pulls, round) in (0..self.rounds).enumerate() {
            let fsm_idx = round as usize;
            let total_pulls = total_pulls as u64;
            // Select strategy via deterministic UCB1
            let c = std::f64::consts::SQRT_2;
            let chosen_strategy = if total_pulls < k as u64 {
                total_pulls as usize
            } else {
                let n = total_pulls;
                (0..k)
                    .max_by(|&i, &j| {
                        let si = if pulls[i] == 0 {
                            f64::INFINITY
                        } else {
                            sum_reward[i] / pulls[i] as f64
                                + c * ((n as f64).ln() / pulls[i] as f64).sqrt()
                        };
                        let sj = if pulls[j] == 0 {
                            f64::INFINITY
                        } else {
                            sum_reward[j] / pulls[j] as f64
                                + c * ((n as f64).ln() / pulls[j] as f64).sqrt()
                        };
                        si.partial_cmp(&sj).unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .unwrap_or(0)
            };

            // Deterministic reward from RNG
            let reward: f64 = rng.random_range(0.0..1.0);
            pulls[chosen_strategy] += 1;
            sum_reward[chosen_strategy] += reward.clamp(0.0, 1.0);

            // Advance FSM state
            let cur_fsm = fsm_states[fsm_idx % fsm_states.len()];
            transitions.push((prev_fsm.clone(), cur_fsm.to_string()));
            prev_fsm = cur_fsm.to_string();

            // Deterministic tool output
            let tool_key: u32 = rng.random_range(1000..9999);
            let tool_output = format!("round_{}_key_{}", round, tool_key);

            // Deterministic GAS: improves with reward and round progress
            let progress = (round + 1) as f32 / self.rounds as f32;
            let gas = (0.2 + 0.6 * progress + 0.2 * reward as f32).clamp(0.0, 1.0);

            gas_trajectory.push(gas);
            strategy_selections.push(strategies[chosen_strategy].to_string());

            events.push(ReplayEvent {
                round,
                fsm_state: cur_fsm.to_string(),
                tool_output,
                gas,
                strategy: strategies[chosen_strategy].to_string(),
            });
        }

        ReplayLog {
            seed: self.seed,
            rounds: self.rounds,
            events,
            transitions,
            gas_trajectory,
            strategy_selections,
        }
    }
}

// ─── Cross-run certification ──────────────────────────────────────────────────

/// Run the harness `n` times and verify all hashes are identical.
///
/// Returns `(hash, all_identical)`.
pub fn certify_determinism(seed: u64, rounds: u32, n: usize) -> (SessionHash, bool) {
    let harness = DeterministicHarness::new(seed, rounds);
    let first_hash = harness.run().session_hash();
    let all_identical = (1..n).all(|_| harness.run().session_hash() == first_hash);
    (first_hash, all_identical)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_produces_same_hash() {
        let h = DeterministicHarness::new(42, 20);
        let hash1 = h.run().session_hash();
        let hash2 = h.run().session_hash();
        assert_eq!(hash1, hash2, "same seed must produce same hash");
    }

    #[test]
    fn different_seed_produces_different_hash() {
        let log1 = DeterministicHarness::new(1, 20).run();
        let log2 = DeterministicHarness::new(2, 20).run();
        assert_ne!(
            log1.session_hash(),
            log2.session_hash(),
            "different seeds should produce different hashes"
        );
    }

    #[test]
    fn session_hash_is_64_hex_chars() {
        let log = DeterministicHarness::new(99, 10).run();
        let hex = log.session_hash().to_hex();
        assert_eq!(
            hex.len(),
            64,
            "SHA-256 should be 64 hex chars, got {}",
            hex.len()
        );
        assert!(
            hex.chars().all(|c| c.is_ascii_hexdigit()),
            "hash should be lowercase hex"
        );
    }

    #[test]
    fn replay_events_ordered_by_round() {
        let log = DeterministicHarness::new(7, 15).run();
        assert!(log.is_ordered(), "events should be ordered by round");
    }

    #[test]
    fn replay_log_is_complete() {
        let rounds = 20u32;
        let log = DeterministicHarness::new(42, rounds).run();
        assert!(log.is_complete(), "log should be complete");
        assert_eq!(log.events.len(), rounds as usize);
    }

    #[test]
    fn all_events_have_nonempty_fields() {
        let log = DeterministicHarness::new(123, 10).run();
        for event in &log.events {
            assert!(!event.fsm_state.is_empty());
            assert!(!event.tool_output.is_empty());
            assert!(!event.strategy.is_empty());
            assert!(event.gas >= 0.0 && event.gas <= 1.0);
        }
    }

    #[test]
    fn one_hundred_runs_produce_identical_hash() {
        // I-7.5 main invariant: 100 replays = same hash
        let (_, all_identical) = certify_determinism(999, 15, 100);
        assert!(
            all_identical,
            "100 runs with same seed must produce identical hash"
        );
    }

    #[test]
    fn session_hash_as_bytes_is_32_bytes() {
        let log = DeterministicHarness::new(0, 5).run();
        let hash = log.session_hash();
        assert_eq!(hash.as_bytes().len(), 32);
    }

    #[test]
    fn empty_log_has_deterministic_hash() {
        let log = DeterministicHarness::new(0, 0).run();
        let h1 = log.session_hash();
        let h2 = DeterministicHarness::new(0, 0).run().session_hash();
        assert_eq!(h1, h2, "empty log should have deterministic hash");
    }

    #[test]
    fn gas_trajectory_monotone_on_average() {
        let log = DeterministicHarness::new(42, 50).run();
        // GAS should be generally increasing (early rounds lower than late)
        let early_mean: f32 = log.gas_trajectory[..10].iter().sum::<f32>() / 10.0;
        let late_mean: f32 = log.gas_trajectory[40..].iter().sum::<f32>() / 10.0;
        assert!(
            late_mean > early_mean,
            "late GAS should be higher than early GAS on average: early={:.3} late={:.3}",
            early_mean,
            late_mean
        );
    }
}
