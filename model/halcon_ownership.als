// ============================================================================
// halcon_ownership.als
//
// Alloy structural model para verificar ownership único por capability
// en el ecosistema CUERVO y ausencia de duplicación estratégica.
//
// Version:  0.1 (Ciclo 0 initial draft)
// Status:   Draft — pending refinement en Ciclo 4
// Related:  docs/architecture/halcon-v3-correction.md §3, §4 (Principios D-1, D-2)
// ============================================================================

// --- Universo de sistemas del ecosistema ---

abstract sig System {}

one sig Halcon, Paloma, Cenzontle, Tordo, PalomaLedger extends System {}

// --- Universo de capabilities ---

abstract sig Capability {
  owner: one System,        // Un ÚNICO owner por capability (P-UNIQ-OWNER)
  consumers: set System,    // Sistemas que consumen esta capability
}

// Control plane (Paloma)
one sig RoutingDecide, BudgetReserve, BudgetCommit, BudgetRelease,
        ScoringThompson, PlanSign extends Capability {}

// Data plane (Cenzontle)
one sig InferenceLlm, McpGateway, SseParse, StallDetect,
        CapabilityNegotiate, OutcomeRelay extends Capability {}

// Execution plane (Tordo)
one sig ExecutionDurable, StepDispatch, RetryPolicy,
        ReplayDeterministic, ArtifactPersist extends Capability {}

// Agent plane (Halcon)
one sig AgentPlan, AgentSandbox, SessionConversation, TuiRender,
        PlanBuildAndSign, ToolExecuteLocal extends Capability {}

// Audit plane (paloma-ledger)
one sig AuditImmutable, ComplianceQuery extends Capability {}

// --- Facts: ownership asignments ---

fact PalomaOwnership {
  RoutingDecide.owner = Paloma
  BudgetReserve.owner = Paloma
  BudgetCommit.owner = Paloma
  BudgetRelease.owner = Paloma
  ScoringThompson.owner = Paloma
  PlanSign.owner = Paloma
}

fact CenzontleOwnership {
  InferenceLlm.owner = Cenzontle
  McpGateway.owner = Cenzontle
  SseParse.owner = Cenzontle
  StallDetect.owner = Cenzontle
  CapabilityNegotiate.owner = Cenzontle
  OutcomeRelay.owner = Cenzontle
}

fact TordoOwnership {
  ExecutionDurable.owner = Tordo
  StepDispatch.owner = Tordo
  RetryPolicy.owner = Tordo
  ReplayDeterministic.owner = Tordo
  ArtifactPersist.owner = Tordo
}

fact HalconOwnership {
  AgentPlan.owner = Halcon
  AgentSandbox.owner = Halcon
  SessionConversation.owner = Halcon
  TuiRender.owner = Halcon
  PlanBuildAndSign.owner = Halcon
  ToolExecuteLocal.owner = Halcon
}

fact LedgerOwnership {
  AuditImmutable.owner = PalomaLedger
  ComplianceQuery.owner = PalomaLedger
}

// --- Facts: consumers permitidos ---

fact HalconConsumers {
  // Halcon consume (no posee):
  //   - inference via Cenzontle MCP
  //   - capability negotiation
  //   - execution durable via Tordo
  //   - replay via Tordo
  //   - audit ingest via ledger (append-only)
  Halcon in InferenceLlm.consumers
  Halcon in CapabilityNegotiate.consumers
  Halcon in ExecutionDurable.consumers
  Halcon in ReplayDeterministic.consumers
  Halcon in AuditImmutable.consumers

  // Halcon NO consume directamente routing ni budget de Paloma
  // (lo hace indirectamente via Cenzontle)
  Halcon not in RoutingDecide.consumers
  Halcon not in BudgetReserve.consumers
  Halcon not in BudgetCommit.consumers
  Halcon not in BudgetRelease.consumers
}

fact CenzontleConsumers {
  // Cenzontle consume routing, budget y outcome de Paloma
  Cenzontle in RoutingDecide.consumers
  Cenzontle in BudgetReserve.consumers
  Cenzontle in BudgetCommit.consumers
  Cenzontle in BudgetRelease.consumers
  Cenzontle in AuditImmutable.consumers
}

fact TordoConsumers {
  // Tordo consume inference y tool_call de Cenzontle
  Tordo in InferenceLlm.consumers
  Tordo in McpGateway.consumers
  Tordo in AuditImmutable.consumers
}

fact PalomaConsumers {
  // Paloma consume ledger (audit de sus propias decisiones)
  Paloma in AuditImmutable.consumers
}

// --- Properties to check ---

// P1: Unicidad de owner (P-UNIQ-OWNER)
// Cada capability tiene exactamente un owner.
// Garantizado estructuralmente por `owner: one System`.
assert UniqueOwnership {
  all c: Capability | one c.owner
}
check UniqueOwnership for 20

// P2: No forbidden duplication
// Ninguna capability tiene Halcon como owner de algo que Paloma/Cenzontle/Tordo
// ya poseen por diseño.
assert NoDuplication {
  // Halcon NO posee capabilities del control/data/execution plane
  no c: Capability | c.owner = Halcon and
    c in (RoutingDecide + BudgetReserve + BudgetCommit + BudgetRelease +
          ScoringThompson + PlanSign +
          InferenceLlm + McpGateway + SseParse + StallDetect +
          CapabilityNegotiate + OutcomeRelay +
          ExecutionDurable + StepDispatch + RetryPolicy +
          ReplayDeterministic + ArtifactPersist)
}
check NoDuplication for 20

// P3: No bypass — Halcon NO consume directamente de Paloma capabilities que
// deben venir vía Cenzontle
assert NoBypassOfGateway {
  Halcon not in RoutingDecide.consumers
  Halcon not in BudgetReserve.consumers
  Halcon not in BudgetCommit.consumers
  Halcon not in BudgetRelease.consumers
  Halcon not in ScoringThompson.consumers
}
check NoBypassOfGateway for 20

// P4: Halcon NO implementa inference
assert HalconNotInferenceGateway {
  no c: Capability | c.owner = Halcon and c in (InferenceLlm + McpGateway + SseParse)
}
check HalconNotInferenceGateway for 20

// P5: Halcon NO implementa execution durable
assert HalconNotExecutionFabric {
  no c: Capability | c.owner = Halcon and c in (ExecutionDurable + StepDispatch + ReplayDeterministic + ArtifactPersist)
}
check HalconNotExecutionFabric for 20

// P6: Audit append-only (conceptual)
// PalomaLedger es único owner de AuditImmutable; ningún otro sistema muta.
assert AuditSSOT {
  AuditImmutable.owner = PalomaLedger
  AuditImmutable not in (Halcon + Cenzontle + Tordo + Paloma).~owner
}
check AuditSSOT for 20

// --- Run: visualizar topología ---

pred show {}
run show for 5

// ============================================================================
// NOTES FOR CICLO 4
//
// Este model es DRAFT y faltan:
//  - Wire contracts como firma de consumers (no sólo presencia)
//  - Versioning de capabilities
//  - Modelado temporal (Alloy solo estructural)
//  - Integración con property tests de Rust que verifiquen impl real
//
// Para ejecutar:
//  1. Abrir en Alloy Analyzer 6+
//  2. Execute → Show All Assertions
//  3. Expected: 0 counterexamples en cada check
// ============================================================================
