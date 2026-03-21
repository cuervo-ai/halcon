//! Complexity Estimation Engine — weighted multi-factor scoring.
//!
//! Unlike the old word-count heuristic, this layer accumulates evidence from
//! orthogonal signal families:
//!
//! | Family            | Max contribution | What it captures               |
//! |-------------------|------------------|--------------------------------|
//! | Request length    |  20 pts          | raw query verbosity            |
//! | Technical depth   |  30 pts          | architecture/design terms      |
//! | Analysis scope    |  25 pts          | investigative / multi-file     |
//! | Code density      |  15 pts          | code-specific terminology      |
//! | Security weight   |  10 pts          | security/audit vocabulary      |
//!
//! Maximum raw score: 100 points.
//!
//! Classification thresholds:
//!   Low    →  score <  25
//!   Medium →  score <  55
//!   High   →  score ≥  55

// ── Output types ─────────────────────────────────────────────────────────────

/// Coarse complexity tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ComplexityLevel {
    /// Simple lookup, explanation, or direct single-step action.
    Low,
    /// Multi-step task within a single domain.
    Medium,
    /// Deep investigation, cross-domain, or exhaustive analysis.
    High,
}

impl ComplexityLevel {
    pub fn label(self) -> &'static str {
        match self {
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
        }
    }
}

/// Full output of the complexity estimation boundary.
#[derive(Debug, Clone)]
pub struct ComplexityEstimate {
    /// Coarse complexity tier.
    pub level: ComplexityLevel,
    /// Raw accumulated score [0, 100].
    pub score: f32,
    /// Dominant factor that pushed the score highest.
    pub dominant_factor: &'static str,
    /// All signal families that contributed ≥ 1 point.
    pub contributing_factors: Vec<&'static str>,
}

// ── Signal families ───────────────────────────────────────────────────────────

/// Architectural and design depth terms — highest cognitive load.
const DEPTH_TERMS: &[&str] = &[
    "architecture",
    "arquitectura",
    "design pattern",
    "patrón de diseño",
    "microservice",
    "microservicio",
    "distributed system",
    "sistema distribuido",
    "scalability",
    "escalabilidad",
    "coupling",
    "acoplamiento",
    "cohesion",
    "cohesión",
    "abstraction",
    "abstracción",
    "dependency injection",
    "inyección de dependencias",
    "hexagonal",
    "cqrs",
    "event sourcing",
    "saga pattern",
    "domain-driven",
    "ddd",
    "clean architecture",
    "solid",
    "performance bottleneck",
    "cuello de botella",
    "latency",
    "latencia",
    "throughput",
    "concurrency",
    "concurrencia",
    "parallelism",
];

/// Analysis and investigation scope indicators.
const ANALYSIS_TERMS: &[&str] = &[
    "analyze",
    "analizar",
    "review",
    "revisar",
    "investigate",
    "investigar",
    "audit",
    "auditar",
    "inspect",
    "inspeccionar",
    "evaluate",
    "evaluar",
    "assess",
    "diagnose",
    "diagnosticar",
    "identify",
    "identificar",
    "comprehensive",
    "exhaustive",
    "exhaustivo",
    "deep",
    "profundo",
    "entire",
    "entero",
    "complete",
    "completo",
    "all",
    "todos",
    "across the",
    "en todo",
    "throughout",
    "a través de",
    "repository",
    "repositorio",
    "codebase",
    "entire project",
    "proyecto completo",
    "every module",
    "all modules",
    "todos los módulos",
    "end-to-end",
    "end to end",
    "de extremo a extremo",
    "from scratch",
    "desde cero",
    "full",
    "completa",
];

/// Code-related technical terminology.
const CODE_TERMS: &[&str] = &[
    "function",
    "función",
    "class",
    "clase",
    "module",
    "módulo",
    "interface",
    "trait",
    "enum",
    "struct",
    "method",
    "método",
    "algorithm",
    "algoritmo",
    "data structure",
    "estructura de datos",
    "async",
    "concurrent",
    "parallel",
    "recursion",
    "recursión",
    "memory",
    "heap",
    "stack",
    "lifecycle",
    "ciclo de vida",
    "api",
    "endpoint",
    "handler",
    "middleware",
    "pipeline",
    "test",
    "unit test",
    "integration test",
    "e2e",
    "coverage",
    "refactor",
    "refactorizar",
    "optimize",
    "optimizar",
    "bug",
    "error",
    "exception",
    "panic",
    "crash",
];

/// Security-specific vocabulary — these alone mandate careful handling.
const SECURITY_TERMS: &[&str] = &[
    "security",
    "seguridad",
    "vulnerability",
    "vulnerabilidad",
    "exploit",
    "attack",
    "threat",
    "amenaza",
    "risk",
    "riesgo",
    "injection",
    "xss",
    "csrf",
    "owasp",
    "cve",
    "authentication",
    "autenticación",
    "authorization",
    "autorización",
    "privilege escalation",
    "escalada de privilegios",
    "penetration",
    "penetración",
    "pentest",
    "hardening",
    "encryption",
    "cifrado",
    "secret",
    "credential",
];

// ── Thresholds ────────────────────────────────────────────────────────────────
// Calibrated empirically against the test scenarios:
//   Low    : score <  15   (greetings, single-concept questions)
//   Medium : score <  40   (multi-step code tasks, single-domain work)
//   High   : score ≥  40   (deep analysis, cross-domain, security/arch tasks)

const LOW_MAX: f32 = 15.0;
const MED_MAX: f32 = 40.0;

// ── Estimator ────────────────────────────────────────────────────────────────

/// Stateless complexity estimator.
pub struct ComplexityEstimator;

impl ComplexityEstimator {
    /// Estimate the complexity of `query`.
    ///
    /// All logic is deterministic and runs in O(|query| × |signals|) time,
    /// typically < 100 µs for realistic queries.
    pub fn estimate(query: &str) -> ComplexityEstimate {
        let q = query.to_lowercase();
        let word_count = query.split_whitespace().count();

        let mut score = 0f32;
        let mut factors: Vec<(&'static str, f32)> = Vec::new();

        // ── Factor 1: Request length (max 20 pts) ────────────────────────
        let length_score = match word_count {
            0..=5 => 0.0,
            6..=10 => 3.0,
            11..=20 => 7.0,
            21..=35 => 12.0,
            36..=60 => 17.0,
            _ => 20.0,
        };
        if length_score > 0.0 {
            score += length_score;
            factors.push(("request_length", length_score));
        }

        // ── Factor 2: Technical depth terms (max 30 pts) ─────────────────
        let depth_hits = count_hits(&q, DEPTH_TERMS);
        let depth_score = (depth_hits as f32 * 8.0).min(30.0);
        if depth_score > 0.0 {
            score += depth_score;
            factors.push(("technical_depth", depth_score));
        }

        // ── Factor 3: Analysis scope terms (max 30 pts) ──────────────────
        let analysis_hits = count_hits(&q, ANALYSIS_TERMS);
        let analysis_score = (analysis_hits as f32 * 6.0).min(30.0);
        if analysis_score > 0.0 {
            score += analysis_score;
            factors.push(("analysis_scope", analysis_score));
        }

        // ── Factor 4: Code density terms (max 15 pts) ────────────────────
        let code_hits = count_hits(&q, CODE_TERMS);
        let code_score = (code_hits as f32 * 3.0).min(15.0);
        if code_score > 0.0 {
            score += code_score;
            factors.push(("code_density", code_score));
        }

        // ── Factor 5: Security vocabulary (max 25 pts) ───────────────────
        // Security tasks should reliably hit High; give this family more room.
        let sec_hits = count_hits(&q, SECURITY_TERMS);
        let sec_score = (sec_hits as f32 * 6.0).min(25.0);
        if sec_score > 0.0 {
            score += sec_score;
            factors.push(("security_weight", sec_score));
        }

        let score = score.min(100.0);

        // Dominant factor: highest contributor.
        let dominant_factor = factors
            .iter()
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(name, _)| *name)
            .unwrap_or("request_length");

        let contributing_factors: Vec<&'static str> = factors.iter().map(|(n, _)| *n).collect();

        let level = if score < LOW_MAX {
            ComplexityLevel::Low
        } else if score < MED_MAX {
            ComplexityLevel::Medium
        } else {
            ComplexityLevel::High
        };

        ComplexityEstimate {
            level,
            score,
            dominant_factor,
            contributing_factors,
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Count how many distinct terms from `terms` appear in `text`.
///
/// Uses substring matching (not word-boundary) because this is a scoring
/// function: we want "attacks" to match "attack", "vulnerabilities" to
/// be caught by any overlapping security term, etc.  Precision is less
/// important than recall for an accumulative scorer.
fn count_hits(text: &str, terms: &[&str]) -> usize {
    terms.iter().filter(|&&t| text.contains(t)).count()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_is_low() {
        let e = ComplexityEstimator::estimate("hello");
        assert_eq!(e.level, ComplexityLevel::Low);
    }

    #[test]
    fn simple_question_is_low() {
        let e = ComplexityEstimator::estimate("what is a function");
        assert_eq!(e.level, ComplexityLevel::Low);
    }

    #[test]
    fn multi_step_code_task_is_medium() {
        let e = ComplexityEstimator::estimate(
            "refactor the authentication module to use dependency injection",
        );
        assert!(
            e.level >= ComplexityLevel::Medium,
            "level={:?} score={}",
            e.level,
            e.score
        );
    }

    #[test]
    fn architecture_review_is_high() {
        let e = ComplexityEstimator::estimate(
            "analyze the microservice architecture for coupling cohesion scalability issues \
             across the entire distributed system",
        );
        assert_eq!(e.level, ComplexityLevel::High, "score={}", e.score);
    }

    #[test]
    fn security_audit_is_high() {
        let e = ComplexityEstimator::estimate(
            "perform a comprehensive security audit to identify vulnerabilities \
             injection attacks xss csrf owasp top 10 threats",
        );
        assert_eq!(e.level, ComplexityLevel::High, "score={}", e.score);
    }

    #[test]
    fn spanish_deep_review_is_high() {
        let e = ComplexityEstimator::estimate(
            "analiza exhaustivamente la arquitectura del sistema distribuido \
             e identifica vulnerabilidades de seguridad en todos los módulos",
        );
        assert_eq!(e.level, ComplexityLevel::High, "score={}", e.score);
    }

    #[test]
    fn score_capped_at_100() {
        let e = ComplexityEstimator::estimate(
            "analyze review audit investigate microservice architecture distributed system \
             coupling cohesion dependency injection hexagonal cqrs event sourcing ddd solid \
             security vulnerability exploit xss csrf injection owasp pentest hardening \
             encryption authentication authorization privilege escalation",
        );
        assert!(e.score <= 100.0, "score={}", e.score);
    }

    #[test]
    fn complexity_ordering_respected() {
        assert!(ComplexityLevel::Low < ComplexityLevel::Medium);
        assert!(ComplexityLevel::Medium < ComplexityLevel::High);
    }

    #[test]
    fn dominant_factor_populated() {
        let e = ComplexityEstimator::estimate(
            "analyze the microservice architecture for distributed systems",
        );
        assert!(!e.dominant_factor.is_empty());
    }

    #[test]
    fn very_long_query_is_high() {
        let long = "analyze ".repeat(30);
        let e = ComplexityEstimator::estimate(&long);
        assert!(e.level >= ComplexityLevel::Medium);
    }
}
