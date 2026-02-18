/// SDLC phase abstraction for context server activation.
///
/// Each phase corresponds to a different stage in the software development lifecycle,
/// enabling context servers to provide phase-appropriate information.

use serde::{Deserialize, Serialize};

/// Software Development Lifecycle phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SdlcPhase {
    /// Requirements gathering, product discovery.
    Discovery,
    /// Architecture & design decisions.
    Planning,
    /// Coding & implementation.
    Implementation,
    /// Quality assurance & testing.
    Testing,
    /// Code review.
    Review,
    /// Release & deployment.
    Deployment,
    /// Runtime operations & monitoring.
    Monitoring,
    /// Incident response & support.
    Support,
}

impl SdlcPhase {
    /// Human-readable name for display.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Discovery => "Discovery",
            Self::Planning => "Planning",
            Self::Implementation => "Implementation",
            Self::Testing => "Testing",
            Self::Review => "Review",
            Self::Deployment => "Deployment",
            Self::Monitoring => "Monitoring",
            Self::Support => "Support",
        }
    }

    /// All phases in typical SDLC order.
    pub fn all() -> &'static [SdlcPhase] {
        &[
            Self::Discovery,
            Self::Planning,
            Self::Implementation,
            Self::Testing,
            Self::Review,
            Self::Deployment,
            Self::Monitoring,
            Self::Support,
        ]
    }
}

impl Default for SdlcPhase {
    /// Default phase is Implementation (most common in agent interactions).
    fn default() -> Self {
        Self::Implementation
    }
}

impl std::fmt::Display for SdlcPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sdlc_phase_serde() {
        let phase = SdlcPhase::Implementation;
        let json = serde_json::to_string(&phase).unwrap();
        assert_eq!(json, r#""implementation""#);

        let deserialized: SdlcPhase = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, phase);
    }

    #[test]
    fn test_all_phases_roundtrip() {
        for phase in SdlcPhase::all() {
            let json = serde_json::to_string(phase).unwrap();
            let deserialized: SdlcPhase = serde_json::from_str(&json).unwrap();
            assert_eq!(*phase, deserialized);
        }
    }

    #[test]
    fn test_default_phase() {
        assert_eq!(SdlcPhase::default(), SdlcPhase::Implementation);
    }

    #[test]
    fn test_display() {
        assert_eq!(SdlcPhase::Planning.to_string(), "Planning");
        assert_eq!(SdlcPhase::Deployment.to_string(), "Deployment");
    }
}
