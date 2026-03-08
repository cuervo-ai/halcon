//! Risk and Impact Evaluation — gates shallow execution for sensitive task categories.
//!
//! Certain task categories must never be handled with shallow analysis:
//!
//! | Task category        | Risk     | Blocks Fast | Blocks Balanced |
//! |----------------------|----------|-------------|-----------------|
//! | Security audit/CVE   | High     | ✓           | ✓               |
//! | Architecture review  | High     | ✓           | ✓               |
//! | Repository audit     | High     | ✓           | ✓               |
//! | Code review (broad)  | Elevated | ✓           | ✗               |
//! | Data migration       | Elevated | ✓           | ✗               |
//! | Infrastructure change| Elevated | ✓           | ✗               |
//! | Standard code task   | Standard | ✗           | ✗               |
//!
//! The risk assessment is **conservative by design**: when signals are ambiguous,
//! it over-classifies rather than under-classifies.  False positives (extra rounds)
//! cost time; false negatives (missed vulnerabilities) cost correctness.

use super::domain_detector::TechnicalDomain;
use super::complexity_estimator::ComplexityLevel;

// ── Risk tier ────────────────────────────────────────────────────────────────

/// Execution risk level for the task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExecutionRisk {
    /// Normal task — any SLA mode is acceptable.
    Standard,
    /// Sensitive task — Fast SLA is blocked; Balanced minimum required.
    Elevated,
    /// Critical task — Deep SLA is mandatory; synthesis always required.
    High,
}

impl ExecutionRisk {
    pub fn label(self) -> &'static str {
        match self {
            Self::Standard => "Standard",
            Self::Elevated => "Elevated",
            Self::High => "High",
        }
    }

    /// Whether this risk level prohibits Fast SLA mode.
    pub fn blocks_fast_mode(self) -> bool {
        matches!(self, Self::Elevated | Self::High)
    }

    /// Whether this risk level prohibits Balanced SLA mode (forces Deep).
    pub fn blocks_balanced_mode(self) -> bool {
        matches!(self, Self::High)
    }
}

// ── Risk patterns ─────────────────────────────────────────────────────────────

/// Patterns that independently raise risk to High.
const HIGH_RISK_PATTERNS: &[&str] = &[
    // Security intent
    "security audit", "auditoría de seguridad",
    "vulnerability assessment", "evaluación de vulnerabilidades",
    "penetration test", "pentest", "security review", "revisión de seguridad",
    "find vulnerabilities", "busca vulnerabilidades", "find security issues",
    "owasp", "cve", "exploit", "attack surface", "threat model",
    "security analysis", "análisis de seguridad",
    // Architecture intent
    "architecture review", "revisión de arquitectura",
    "architecture analysis", "análisis de arquitectura",
    "design review", "system design review", "review the architecture",
    "revisar la arquitectura", "analizar la arquitectura",
    // Repository scope
    "repository review", "repo review", "codebase review", "codebase audit",
    "review the codebase", "review the entire codebase",
    "revisar el repositorio", "revisar el código fuente",
    "revisar todo el proyecto", "review the whole project",
    "audit the codebase", "auditoría del código",
    // Compliance / governance
    "compliance", "cumplimiento", "regulatory", "regulatorio",
    "gdpr", "hipaa", "pci", "soc 2", "iso 27001",
];

/// Patterns that raise risk to Elevated.
const ELEVATED_RISK_PATTERNS: &[&str] = &[
    "code review", "revisión de código", "review the code", "revisar el código",
    "analyze the code", "analizar el código", "code quality", "calidad del código",
    "technical debt", "deuda técnica",
    "data migration", "migración de datos", "database migration",
    "breaking change", "cambio disruptivo", "api change",
    "production", "producción", "deploy to prod", "desplegar a producción",
    "critical path", "ruta crítica", "critical system", "sistema crítico",
    "large refactor", "gran refactorización", "full refactor",
];

// ── Risk assessment result ────────────────────────────────────────────────────

/// Full output of the risk assessment boundary.
#[derive(Debug, Clone)]
pub struct RiskAssessment {
    /// Coarse risk tier.
    pub risk: ExecutionRisk,
    /// Reason strings explaining why this risk level was assigned.
    pub reasons: Vec<&'static str>,
    /// Whether the agent must produce a final synthesis (cannot exit silently).
    pub requires_synthesis: bool,
    /// Minimum plan depth recommended (overrides SLA default if higher).
    pub min_plan_depth: u32,
}

// ── Assessor ─────────────────────────────────────────────────────────────────

/// Stateless risk assessor.
pub struct RiskAssessor;

impl RiskAssessor {
    /// Assess the execution risk for `query` given its detected `domain` and
    /// `complexity`.
    ///
    /// Domain and complexity are used as tie-breakers when query text alone is
    /// ambiguous.  The assessor is conservative: it prefers over-classifying risk.
    pub fn assess(
        query: &str,
        domain: TechnicalDomain,
        complexity: ComplexityLevel,
    ) -> RiskAssessment {
        let q = query.to_lowercase();
        let mut risk = ExecutionRisk::Standard;
        let mut reasons: Vec<&'static str> = Vec::new();

        // ── Pass 1: explicit High-risk pattern match ───────────────────────
        for &pattern in HIGH_RISK_PATTERNS {
            if q.contains(pattern) {
                risk = ExecutionRisk::High;
                reasons.push(pattern);
            }
        }

        // ── Pass 2: explicit Elevated-risk pattern match ───────────────────
        if risk < ExecutionRisk::Elevated {
            for &pattern in ELEVATED_RISK_PATTERNS {
                if q.contains(pattern) {
                    risk = ExecutionRisk::Elevated;
                    reasons.push(pattern);
                }
            }
        }

        // ── Pass 3: domain-driven risk escalation ─────────────────────────
        // If the domain detector already identified a sensitive domain, ensure
        // at least Elevated risk (even if the query text was sparse).
        let domain_floor = match domain {
            TechnicalDomain::SecurityAnalysis | TechnicalDomain::ArchitectureAnalysis => {
                ExecutionRisk::High
            }
            TechnicalDomain::CodeReview => ExecutionRisk::Elevated,
            TechnicalDomain::Infrastructure | TechnicalDomain::DataEngineering => {
                ExecutionRisk::Elevated
            }
            _ => ExecutionRisk::Standard,
        };

        if domain_floor > risk {
            risk = domain_floor;
            reasons.push(match domain {
                TechnicalDomain::SecurityAnalysis => "domain:SecurityAnalysis",
                TechnicalDomain::ArchitectureAnalysis => "domain:ArchitectureAnalysis",
                TechnicalDomain::CodeReview => "domain:CodeReview",
                TechnicalDomain::Infrastructure => "domain:Infrastructure",
                TechnicalDomain::DataEngineering => "domain:DataEngineering",
                _ => "domain:elevated",
            });
        }

        // ── Pass 4: complexity-driven escalation ──────────────────────────
        // High complexity alone implies Elevated risk (many moving parts).
        if complexity == ComplexityLevel::High && risk < ExecutionRisk::Elevated {
            risk = ExecutionRisk::Elevated;
            reasons.push("complexity:High");
        }

        let requires_synthesis = risk >= ExecutionRisk::Elevated;

        let min_plan_depth = match risk {
            ExecutionRisk::Standard => 2,
            ExecutionRisk::Elevated => 4,
            ExecutionRisk::High => 6,
        };

        RiskAssessment {
            risk,
            reasons,
            requires_synthesis,
            min_plan_depth,
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_question_is_standard() {
        let a = RiskAssessor::assess(
            "what is a function",
            TechnicalDomain::GeneralInquiry,
            ComplexityLevel::Low,
        );
        assert_eq!(a.risk, ExecutionRisk::Standard);
        assert!(!a.requires_synthesis);
    }

    #[test]
    fn security_audit_is_high() {
        let a = RiskAssessor::assess(
            "perform a security audit to find vulnerabilities owasp",
            TechnicalDomain::SecurityAnalysis,
            ComplexityLevel::High,
        );
        assert_eq!(a.risk, ExecutionRisk::High);
        assert!(a.requires_synthesis);
        assert!(a.min_plan_depth >= 6);
    }

    #[test]
    fn architecture_review_is_high() {
        let a = RiskAssessor::assess(
            "review the architecture of the microservice system",
            TechnicalDomain::ArchitectureAnalysis,
            ComplexityLevel::High,
        );
        assert_eq!(a.risk, ExecutionRisk::High);
        assert!(a.risk.blocks_balanced_mode());
    }

    #[test]
    fn code_review_is_elevated() {
        let a = RiskAssessor::assess(
            "review the code quality",
            TechnicalDomain::CodeReview,
            ComplexityLevel::Medium,
        );
        assert_eq!(a.risk, ExecutionRisk::Elevated);
        assert!(a.risk.blocks_fast_mode());
        assert!(!a.risk.blocks_balanced_mode());
    }

    #[test]
    fn high_complexity_standard_domain_is_elevated() {
        let a = RiskAssessor::assess(
            "write a comprehensive implementation with many components",
            TechnicalDomain::CodeOperations,
            ComplexityLevel::High,
        );
        assert_eq!(a.risk, ExecutionRisk::Elevated);
    }

    #[test]
    fn spanish_security_is_high() {
        let a = RiskAssessor::assess(
            "auditoría de seguridad busca vulnerabilidades",
            TechnicalDomain::SecurityAnalysis,
            ComplexityLevel::High,
        );
        assert_eq!(a.risk, ExecutionRisk::High);
    }

    #[test]
    fn domain_security_alone_elevates_to_high() {
        // Even a terse query; domain drives the risk.
        let a = RiskAssessor::assess(
            "check security",
            TechnicalDomain::SecurityAnalysis,
            ComplexityLevel::Low,
        );
        assert_eq!(a.risk, ExecutionRisk::High, "domain:SecurityAnalysis must floor at High");
    }

    #[test]
    fn risk_ordering_respected() {
        assert!(ExecutionRisk::Standard < ExecutionRisk::Elevated);
        assert!(ExecutionRisk::Elevated < ExecutionRisk::High);
    }

    #[test]
    fn reasons_populated_for_high_risk() {
        let a = RiskAssessor::assess(
            "security audit pentest owasp",
            TechnicalDomain::SecurityAnalysis,
            ComplexityLevel::High,
        );
        assert!(!a.reasons.is_empty(), "reasons must be populated");
    }

    #[test]
    fn repository_review_is_high() {
        let a = RiskAssessor::assess(
            "codebase review of the entire repository",
            TechnicalDomain::CodeReview,
            ComplexityLevel::High,
        );
        assert_eq!(a.risk, ExecutionRisk::High);
    }
}
