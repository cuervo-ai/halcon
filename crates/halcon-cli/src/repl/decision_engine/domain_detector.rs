//! Domain Detection Layer — identifies the primary technical domain of a request.
//!
//! Each domain has an independent weighted signal table. The detector scores every
//! domain independently, selects the highest-scoring one as primary, and the
//! second-highest as secondary when it clears the shadow threshold (≥ 40% of
//! primary).
//!
//! Domain hierarchy (routing implications, ascending):
//!
//! | Domain               | SLA floor  | Convergence guard |
//! |----------------------|------------|-------------------|
//! | GeneralInquiry       | Quick      | none              |
//! | CodeOperations       | Extended   | none              |
//! | Infrastructure       | Extended   | none              |
//! | DataEngineering      | Extended   | none              |
//! | CodeReview           | Deep       | EvidenceThreshold |
//! | ArchitectureAnalysis | Deep       | all signals       |
//! | SecurityAnalysis     | Deep       | all signals       |
//!
//! All methods are pure functions — zero I/O, zero allocations on the hot path
//! beyond the returned `DomainDetection` struct.

// ── Domain enum ──────────────────────────────────────────────────────────────

/// Primary technical domain of a user request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TechnicalDomain {
    /// Simple conversational / informational queries.
    GeneralInquiry = 0,
    /// Writing, creating, or modifying code (non-review).
    CodeOperations = 1,
    /// Infrastructure, deployment, CI/CD, containers.
    Infrastructure = 2,
    /// Database design, migrations, data pipelines.
    DataEngineering = 3,
    /// Read-only analysis of existing code — always requires synthesis.
    CodeReview = 4,
    /// System design, patterns, cross-component analysis.
    ArchitectureAnalysis = 5,
    /// Vulnerability analysis, CVE investigation, penetration testing.
    SecurityAnalysis = 6,
}

impl TechnicalDomain {
    /// Human-readable name for logging and decision traces.
    pub fn label(self) -> &'static str {
        match self {
            Self::GeneralInquiry => "GeneralInquiry",
            Self::CodeOperations => "CodeOperations",
            Self::Infrastructure => "Infrastructure",
            Self::DataEngineering => "DataEngineering",
            Self::CodeReview => "CodeReview",
            Self::ArchitectureAnalysis => "ArchitectureAnalysis",
            Self::SecurityAnalysis => "SecurityAnalysis",
        }
    }

    /// Whether this domain mandates Deep SLA (cannot be downgraded).
    pub fn requires_deep_sla(self) -> bool {
        matches!(
            self,
            Self::CodeReview | Self::ArchitectureAnalysis | Self::SecurityAnalysis
        )
    }

    /// Whether this domain disables *all* early-convergence signals.
    pub fn blocks_all_convergence(self) -> bool {
        matches!(self, Self::ArchitectureAnalysis | Self::SecurityAnalysis)
    }

    /// Whether this domain disables the EvidenceThreshold signal specifically.
    pub fn blocks_evidence_threshold(self) -> bool {
        matches!(
            self,
            Self::CodeReview | Self::ArchitectureAnalysis | Self::SecurityAnalysis
        )
    }
}

// ── Signal tables ─────────────────────────────────────────────────────────────

/// `(signal_text, weight)` — single-word signals matched as whole words; multi-word
/// signals matched as substrings (both via `contains_word` helper).
type SignalTable = &'static [(&'static str, f32)];

const GENERAL_SIGNALS: SignalTable = &[
    ("hello", 0.5),
    ("hi", 0.5),
    ("hey", 0.5),
    ("what is", 0.7),
    ("what are", 0.7),
    ("how do", 0.6),
    ("explain", 0.8),
    ("tell me", 0.6),
    ("describe", 0.7),
    ("help", 0.4),
    ("question", 0.5),
    ("qué es", 0.7),
    ("cómo", 0.6),
    ("explica", 0.8),
    ("qué significa", 0.7),
];

const CODE_OPS_SIGNALS: SignalTable = &[
    ("implement", 0.9),
    ("create", 0.7),
    ("write", 0.7),
    ("build", 0.8),
    ("add function", 1.0),
    ("add method", 1.0),
    ("add class", 1.0),
    ("scaffold", 1.0),
    ("generate", 0.8),
    ("fix bug", 0.9),
    ("fix the", 0.5),
    ("refactor", 0.9),
    ("rewrite", 0.9),
    ("update", 0.6),
    ("modify", 0.7),
    ("debug", 0.9),
    ("patch", 0.8),
    ("compile", 0.8),
    ("run tests", 0.8),
    ("implementar", 0.9),
    ("crear", 0.7),
    ("escribir", 0.7),
    ("corregir", 0.9),
    ("refactorizar", 0.9),
    ("depurar", 0.9),
    ("compilar", 0.8),
];

const ARCH_SIGNALS: SignalTable = &[
    ("architecture", 1.0),
    ("design", 0.8),
    ("pattern", 0.7),
    ("structure", 0.8),
    ("component", 0.7),
    ("system design", 1.0),
    ("microservice", 1.0),
    ("module boundary", 1.0),
    ("coupling", 0.9),
    ("cohesion", 0.9),
    ("scalability", 0.9),
    ("distributed", 0.9),
    ("service mesh", 1.0),
    ("event-driven", 1.0),
    ("cqrs", 1.0),
    ("ddd", 1.0),
    ("hexagonal", 1.0),
    ("dependency", 0.7),
    ("abstraction", 0.8),
    ("layer", 0.6),
    ("arquitectura", 1.0),
    ("diseño del sistema", 1.0),
    ("microservicios", 1.0),
    ("acoplamiento", 0.9),
    ("escalabilidad", 0.9),
    // review/analysis framing
    ("review the architecture", 1.0),
    ("analyze the architecture", 1.0),
    ("architecture review", 1.0),
    ("design review", 0.9),
    ("revisar la arquitectura", 1.0),
    ("analizar la arquitectura", 1.0),
];

const SECURITY_SIGNALS: SignalTable = &[
    ("security", 1.0),
    ("vulnerability", 1.0),
    ("vulnerabilidad", 1.0),
    ("cve", 1.0),
    ("exploit", 1.0),
    ("attack", 0.9),
    ("threat", 0.9),
    ("injection", 1.0),
    ("sql injection", 1.0),
    ("xss", 1.0),
    ("csrf", 1.0),
    ("authentication", 0.9),
    ("authorization", 0.9),
    ("privilege", 0.9),
    ("penetration", 1.0),
    ("pentest", 1.0),
    ("owasp", 1.0),
    ("audit", 0.9),
    ("secure", 0.7),
    ("hardening", 1.0),
    ("encryption", 0.9),
    ("secret", 0.7),
    ("credentials", 0.8),
    ("token", 0.6),
    ("jwt", 0.9),
    ("seguridad", 1.0),
    ("vulnerabilidades", 1.0),
    ("brecha", 0.9),
    ("brechas de seguridad", 1.0),
    ("auditoría", 0.9),
    ("amenaza", 0.9),
    // explicit task framing
    ("security review", 1.0),
    ("security audit", 1.0),
    ("vulnerability scan", 1.0),
    ("revisión de seguridad", 1.0),
    ("auditoría de seguridad", 1.0),
    ("find vulnerabilities", 1.0),
    ("busca vulnerabilidades", 1.0),
    ("check security", 0.9),
    ("security analysis", 1.0),
];

const CODE_REVIEW_SIGNALS: SignalTable = &[
    ("review", 0.8),
    ("revisar", 0.8),
    ("code review", 1.0),
    ("revisión de código", 1.0),
    ("analyze", 0.7),
    ("analizar", 0.7),
    ("analyze the code", 1.0),
    ("inspect", 0.8),
    ("inspect the code", 1.0),
    ("code quality", 1.0),
    ("read the code", 0.9),
    ("leer el código", 0.9),
    ("look at the code", 0.8),
    ("check the code", 0.9),
    ("find issues", 0.9),
    ("find problems", 0.9),
    ("identify issues", 0.9),
    ("code smell", 1.0),
    ("technical debt", 1.0),
    ("best practices", 0.9),
    ("what does", 0.5),
    ("how does this work", 0.7),
    ("understand the code", 0.9),
    ("calidad del código", 1.0),
    ("deuda técnica", 1.0),
    ("buenas prácticas", 0.9),
    ("revisar el código", 1.0),
    ("analizar el código", 1.0),
    // repository-scope framing
    ("repository review", 1.0),
    ("repo review", 1.0),
    ("codebase review", 1.0),
    ("review the project", 1.0),
    ("review the codebase", 1.0),
    ("revisar el proyecto", 1.0),
    ("revisar el repositorio", 1.0),
    ("source code", 0.9),
    ("código fuente", 0.9),
];

const INFRA_SIGNALS: SignalTable = &[
    ("docker", 1.0),
    ("kubernetes", 1.0),
    ("k8s", 1.0),
    ("container", 0.9),
    ("deploy", 0.9),
    ("deployment", 0.9),
    ("ci/cd", 1.0),
    ("pipeline", 0.8),
    ("terraform", 1.0),
    ("ansible", 1.0),
    ("helm", 1.0),
    ("nginx", 0.9),
    ("load balancer", 1.0),
    ("autoscaling", 1.0),
    ("monitoring", 0.8),
    ("prometheus", 1.0),
    ("grafana", 1.0),
    ("logging", 0.7),
    ("metrics", 0.7),
    ("aws", 0.9),
    ("gcp", 0.9),
    ("azure", 0.9),
    ("cloud", 0.8),
    ("desplegar", 0.9),
    ("despliegue", 0.9),
    ("contenedor", 0.9),
];

const DATA_SIGNALS: SignalTable = &[
    ("database", 0.9),
    ("sql", 1.0),
    ("query", 0.7),
    ("migration", 1.0),
    ("schema", 0.9),
    ("table", 0.7),
    ("index", 0.7),
    ("join", 0.8),
    ("orm", 1.0),
    ("postgres", 1.0),
    ("mysql", 1.0),
    ("mongodb", 1.0),
    ("redis", 0.9),
    ("elasticsearch", 1.0),
    ("data pipeline", 1.0),
    ("etl", 1.0),
    ("spark", 1.0),
    ("kafka", 1.0),
    ("stream", 0.7),
    ("base de datos", 0.9),
    ("consulta", 0.7),
    ("migración", 1.0),
];

// ── Detection result ──────────────────────────────────────────────────────────

/// Output of the domain detection boundary.
#[derive(Debug, Clone)]
pub struct DomainDetection {
    /// Highest-scoring domain.
    pub primary: TechnicalDomain,
    /// Second domain when its score ≥ 40% of primary (cross-domain tasks).
    pub secondary: Option<TechnicalDomain>,
    /// Confidence in the primary classification, [0, 1].
    pub confidence: f32,
    /// All signals that contributed to the primary domain score.
    pub matched_signals: Vec<&'static str>,
    /// Raw per-domain scores for observability.
    pub scores: [f32; 7],
}

// ── Detector ─────────────────────────────────────────────────────────────────

/// Stateless domain detector. All state is in the returned `DomainDetection`.
pub struct DomainDetector;

impl DomainDetector {
    /// Classify the technical domain of `query`.
    ///
    /// Never panics. Always returns a valid `DomainDetection`.
    pub fn detect(query: &str) -> DomainDetection {
        let q = query.to_lowercase();

        // Score each domain independently.
        let tables: [(&[(&str, f32)], TechnicalDomain); 7] = [
            (GENERAL_SIGNALS, TechnicalDomain::GeneralInquiry),
            (CODE_OPS_SIGNALS, TechnicalDomain::CodeOperations),
            (ARCH_SIGNALS, TechnicalDomain::ArchitectureAnalysis),
            (SECURITY_SIGNALS, TechnicalDomain::SecurityAnalysis),
            (CODE_REVIEW_SIGNALS, TechnicalDomain::CodeReview),
            (INFRA_SIGNALS, TechnicalDomain::Infrastructure),
            (DATA_SIGNALS, TechnicalDomain::DataEngineering),
        ];

        let mut scores = [0f32; 7];
        let mut all_signals: Vec<(&'static str, TechnicalDomain, f32)> = Vec::new();

        for (i, (table, domain)) in tables.iter().enumerate() {
            let (score, fired) = score_table(&q, table);
            scores[i] = score;
            for sig in fired {
                all_signals.push((sig, *domain, score));
            }
        }

        // Reorder scores to match TechnicalDomain discriminant values.
        // The `tables` array above is ordered by domain index (0..6) already.
        // Map: GeneralInquiry=0, CodeOperations=1, Infrastructure=5, DataEngineering=6,
        //      CodeReview=4, ArchitectureAnalysis=2 (ord=5), SecurityAnalysis=3 (ord=6).
        // We need scores aligned to discriminant order for the output array.
        // Simpler: use a local indexed search.

        // Find the two best domains.
        let mut indexed: Vec<(usize, f32)> = scores.iter().copied().enumerate().collect();
        indexed.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let domain_for_idx = |i: usize| tables[i].1;

        let (best_idx, best_score) = indexed[0];
        let primary = if best_score <= 0.0 {
            TechnicalDomain::GeneralInquiry
        } else {
            domain_for_idx(best_idx)
        };

        let (second_idx, second_score) = indexed[1];
        let secondary = if best_score > 0.0 && second_score >= best_score * 0.40 {
            Some(domain_for_idx(second_idx))
        } else {
            None
        };

        // Confidence: ratio between winner and runner-up, clamped.
        let confidence = if best_score <= 0.0 {
            0.2
        } else if second_score <= 0.0 {
            0.95
        } else {
            let ratio = best_score / (best_score + second_score);
            ratio.clamp(0.2, 1.0)
        };

        let matched_signals: Vec<&'static str> = all_signals
            .iter()
            .filter(|(_, d, _)| *d == primary)
            .map(|(s, _, _)| *s)
            .collect();

        // Build output score array in discriminant order (0..=6).
        let mut out_scores = [0f32; 7];
        for (i, (_, domain)) in tables.iter().enumerate() {
            out_scores[*domain as usize] = scores[i];
        }

        DomainDetection {
            primary,
            secondary,
            confidence,
            matched_signals,
            scores: out_scores,
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Sum the weights of all signals that appear in `text`.
/// Multi-word signals → substring match. Single words → word-boundary match.
fn score_table(text: &str, table: &[(&'static str, f32)]) -> (f32, Vec<&'static str>) {
    let mut total = 0f32;
    let mut fired: Vec<&'static str> = Vec::new();
    for &(signal, weight) in table {
        let matched = if signal.contains(' ') {
            text.contains(signal)
        } else {
            contains_word(text, signal)
        };
        if matched {
            total += weight;
            fired.push(signal);
        }
    }
    (total, fired)
}

/// Word-boundary match for single-token keywords.
fn contains_word(text: &str, word: &str) -> bool {
    let bytes = text.as_bytes();
    let wbytes = word.as_bytes();
    let wlen = wbytes.len();
    if wlen > bytes.len() {
        return false;
    }
    for i in 0..=(bytes.len().saturating_sub(wlen)) {
        if bytes[i..].starts_with(wbytes) {
            let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'_';
            let end = i + wlen;
            let after_ok =
                end >= bytes.len() || !bytes[end].is_ascii_alphanumeric() && bytes[end] != b'_';
            if before_ok && after_ok {
                return true;
            }
        }
    }
    false
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greeting_is_general() {
        let d = DomainDetector::detect("hello how are you");
        assert_eq!(d.primary, TechnicalDomain::GeneralInquiry);
    }

    #[test]
    fn implement_is_code_ops() {
        let d = DomainDetector::detect("implement the new authentication module");
        assert_eq!(d.primary, TechnicalDomain::CodeOperations);
    }

    #[test]
    fn architecture_review_detected() {
        let d = DomainDetector::detect("review the architecture of the payment service");
        assert_eq!(d.primary, TechnicalDomain::ArchitectureAnalysis);
    }

    #[test]
    fn security_audit_detected() {
        let d = DomainDetector::detect("find security vulnerabilities in the authentication code");
        assert_eq!(d.primary, TechnicalDomain::SecurityAnalysis);
    }

    #[test]
    fn code_review_detected() {
        let d = DomainDetector::detect("review the source code of the project and find issues");
        assert_eq!(d.primary, TechnicalDomain::CodeReview);
    }

    #[test]
    fn spanish_security_detected() {
        let d = DomainDetector::detect("busca brechas de seguridad en el código fuente");
        assert_eq!(d.primary, TechnicalDomain::SecurityAnalysis);
    }

    #[test]
    fn spanish_code_review_detected() {
        let d = DomainDetector::detect("revisar el código fuente del proyecto y buscar problemas");
        assert_eq!(d.primary, TechnicalDomain::CodeReview);
    }

    #[test]
    fn docker_is_infrastructure() {
        let d = DomainDetector::detect("deploy the application using docker and kubernetes");
        assert_eq!(d.primary, TechnicalDomain::Infrastructure);
    }

    #[test]
    fn sql_is_data_engineering() {
        let d = DomainDetector::detect("write a sql migration for the new user table schema");
        assert_eq!(d.primary, TechnicalDomain::DataEngineering);
    }

    #[test]
    fn security_requires_deep_sla() {
        assert!(TechnicalDomain::SecurityAnalysis.requires_deep_sla());
        assert!(TechnicalDomain::ArchitectureAnalysis.requires_deep_sla());
        assert!(TechnicalDomain::CodeReview.requires_deep_sla());
        assert!(!TechnicalDomain::CodeOperations.requires_deep_sla());
    }

    #[test]
    fn convergence_guards_correct() {
        assert!(TechnicalDomain::SecurityAnalysis.blocks_all_convergence());
        assert!(TechnicalDomain::ArchitectureAnalysis.blocks_all_convergence());
        assert!(!TechnicalDomain::CodeReview.blocks_all_convergence());
        assert!(TechnicalDomain::CodeReview.blocks_evidence_threshold());
    }

    #[test]
    fn confidence_high_for_clear_security() {
        let d = DomainDetector::detect("security audit vulnerability scan owasp pentest");
        assert!(d.confidence > 0.6, "confidence={}", d.confidence);
    }

    #[test]
    fn secondary_detected_for_cross_domain() {
        // Architecture + security overlap
        let d = DomainDetector::detect(
            "analyze the system architecture for security vulnerabilities and threat models",
        );
        assert!(
            d.secondary.is_some(),
            "expected secondary domain for mixed query"
        );
    }

    #[test]
    fn large_repo_review_is_code_review() {
        let d = DomainDetector::detect(
            "perform a comprehensive review of the entire codebase and repository source code",
        );
        assert_eq!(d.primary, TechnicalDomain::CodeReview);
    }
}
