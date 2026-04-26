----------------------------- MODULE halcon_plan_lifecycle -----------------------------
(***************************************************************************)
(* Halcon ExecutionPlan lifecycle FSM                                       *)
(*                                                                          *)
(* Cubre la máquina de estados de un plan desde Halcon emite hasta Tordo lo *)
(* ejecuta y cierra, incluyendo retry bounded, replay determinista y        *)
(* closure de reservation (INV-5).                                          *)
(*                                                                          *)
(* Version:  0.1 (Ciclo 0 initial draft)                                    *)
(* Status:   Draft — pending refinement in Ciclo 4                          *)
(* Authors:  Halcon architecture team                                       *)
(* Related:  docs/architecture/halcon-v3-correction.md §6 (I-H4, I-H7, I-H8)*)
(***************************************************************************)

EXTENDS Integers, Sequences, FiniteSets, TLC

CONSTANTS
  Plans,             \* Conjunto finito de plan_ids
  Reservations,      \* Conjunto finito de reservation_ids
  MaxRetries,        \* Cota superior de reintentos por step
  MaxRounds          \* Cota superior de rounds agent loop

ASSUME MaxRetries \in Nat /\ MaxRetries > 0
ASSUME MaxRounds  \in Nat /\ MaxRounds > 0

VARIABLES
  plan_state,        \* plan_id -> estado actual
  reservation_state, \* reservation_id -> estado
  retry_count,       \* plan_id -> nro de retries acumulados
  audit_log          \* secuencia de eventos de audit

vars == <<plan_state, reservation_state, retry_count, audit_log>>

(* Estados válidos del plan *)
PlanStates == {"Unsigned", "Signed", "Submitted", "Running", "Completed", "Failed", "Cancelled", "Replayed"}

(* Estados válidos de reservation *)
ReservationStates == {"Open", "Committed", "Released"}

(* Eventos de audit (simplified) *)
AuditEvents == {"plan_emitted", "plan_submitted", "plan_completed", "plan_failed",
                "reservation_committed", "reservation_released"}

-----------------------------------------------------------------------------
(* ─── TypeInvariant ─────────────────────────────────────────────────────── *)
TypeInv ==
  /\ plan_state \in [Plans -> PlanStates]
  /\ reservation_state \in [Reservations -> ReservationStates \cup {"None"}]
  /\ retry_count \in [Plans -> 0..MaxRetries]
  /\ audit_log \in Seq(AuditEvents)

-----------------------------------------------------------------------------
(* ─── Init ──────────────────────────────────────────────────────────────── *)
Init ==
  /\ plan_state = [p \in Plans |-> "Unsigned"]
  /\ reservation_state = [r \in Reservations |-> "None"]
  /\ retry_count = [p \in Plans |-> 0]
  /\ audit_log = <<>>

-----------------------------------------------------------------------------
(* ─── Actions ───────────────────────────────────────────────────────────── *)

(* Halcon firma un plan Unsigned → Signed *)
SignPlan(p) ==
  /\ plan_state[p] = "Unsigned"
  /\ plan_state' = [plan_state EXCEPT ![p] = "Signed"]
  /\ audit_log' = Append(audit_log, "plan_emitted")
  /\ UNCHANGED <<reservation_state, retry_count>>

(* Halcon envía plan firmado a Tordo → Submitted *)
SubmitPlan(p) ==
  /\ plan_state[p] = "Signed"
  /\ plan_state' = [plan_state EXCEPT ![p] = "Submitted"]
  /\ audit_log' = Append(audit_log, "plan_submitted")
  /\ UNCHANGED <<reservation_state, retry_count>>

(* Tordo inicia ejecución → Running *)
StartExecution(p) ==
  /\ plan_state[p] = "Submitted"
  /\ plan_state' = [plan_state EXCEPT ![p] = "Running"]
  /\ UNCHANGED <<reservation_state, retry_count, audit_log>>

(* Paloma abre reservation al routear un step *)
OpenReservation(r) ==
  /\ reservation_state[r] = "None"
  /\ reservation_state' = [reservation_state EXCEPT ![r] = "Open"]
  /\ UNCHANGED <<plan_state, retry_count, audit_log>>

(* Commit de reservation tras ejecución exitosa *)
CommitReservation(r) ==
  /\ reservation_state[r] = "Open"
  /\ reservation_state' = [reservation_state EXCEPT ![r] = "Committed"]
  /\ audit_log' = Append(audit_log, "reservation_committed")
  /\ UNCHANGED <<plan_state, retry_count>>

(* Release de reservation tras fallo/cancel *)
ReleaseReservation(r) ==
  /\ reservation_state[r] = "Open"
  /\ reservation_state' = [reservation_state EXCEPT ![r] = "Released"]
  /\ audit_log' = Append(audit_log, "reservation_released")
  /\ UNCHANGED <<plan_state, retry_count>>

(* Tordo completa plan exitosamente *)
CompletePlan(p) ==
  /\ plan_state[p] = "Running"
  /\ plan_state' = [plan_state EXCEPT ![p] = "Completed"]
  /\ audit_log' = Append(audit_log, "plan_completed")
  /\ UNCHANGED <<reservation_state, retry_count>>

(* Tordo falla plan (después de retries agotados) *)
FailPlan(p) ==
  /\ plan_state[p] = "Running"
  /\ retry_count[p] >= MaxRetries
  /\ plan_state' = [plan_state EXCEPT ![p] = "Failed"]
  /\ audit_log' = Append(audit_log, "plan_failed")
  /\ UNCHANGED <<reservation_state, retry_count>>

(* Tordo reintenta step tras fallo transient — bounded *)
RetryStep(p) ==
  /\ plan_state[p] = "Running"
  /\ retry_count[p] < MaxRetries
  /\ retry_count' = [retry_count EXCEPT ![p] = retry_count[p] + 1]
  /\ UNCHANGED <<plan_state, reservation_state, audit_log>>

(* Operador cancela plan *)
CancelPlan(p) ==
  /\ plan_state[p] \in {"Submitted", "Running"}
  /\ plan_state' = [plan_state EXCEPT ![p] = "Cancelled"]
  /\ UNCHANGED <<reservation_state, retry_count, audit_log>>

(* Replay de plan completado/fallido *)
ReplayPlan(p) ==
  /\ plan_state[p] \in {"Completed", "Failed"}
  /\ plan_state' = [plan_state EXCEPT ![p] = "Replayed"]
  /\ UNCHANGED <<reservation_state, retry_count, audit_log>>

-----------------------------------------------------------------------------
(* ─── Next-state relation ───────────────────────────────────────────────── *)
Next ==
  \/ \E p \in Plans:
       \/ SignPlan(p) \/ SubmitPlan(p) \/ StartExecution(p)
       \/ CompletePlan(p) \/ FailPlan(p) \/ RetryStep(p)
       \/ CancelPlan(p) \/ ReplayPlan(p)
  \/ \E r \in Reservations:
       \/ OpenReservation(r) \/ CommitReservation(r) \/ ReleaseReservation(r)

-----------------------------------------------------------------------------
(* ─── Invariants ────────────────────────────────────────────────────────── *)

(* I-H4: Plan integrity — no plan llega a Submitted sin pasar por Signed *)
PlanIntegrity ==
  \A p \in Plans:
    plan_state[p] \in {"Submitted", "Running", "Completed", "Failed"}
      => audit_log # <<>> /\ "plan_emitted" \in {audit_log[i]: i \in 1..Len(audit_log)}

(* I-H7: Bounded retry — retry_count nunca excede MaxRetries *)
BoundedRetry ==
  \A p \in Plans: retry_count[p] <= MaxRetries

(* INV-5: Reservation no doble commit / no doble release *)
ReservationClosure ==
  \A r \in Reservations:
    reservation_state[r] # "Committed" \/ reservation_state[r] # "Released"

(* INV-5: Reservation committed y released son estados terminales *)
ReservationTerminal ==
  \A r \in Reservations:
    reservation_state[r] \in {"Committed", "Released"}
      => \A r2 \in Reservations: r2 = r => reservation_state'[r2] = reservation_state[r2]

-----------------------------------------------------------------------------
(* ─── Temporal properties (liveness) ────────────────────────────────────── *)

(* Eventualmente: todo plan Submitted alcanza estado terminal *)
PlanEventualTermination ==
  \A p \in Plans:
    (plan_state[p] = "Submitted") ~>
      (plan_state[p] \in {"Completed", "Failed", "Cancelled", "Replayed"})

(* Eventualmente: toda reservation Open alcanza Committed o Released *)
ReservationEventualClosure ==
  \A r \in Reservations:
    (reservation_state[r] = "Open") ~>
      (reservation_state[r] \in {"Committed", "Released"})

-----------------------------------------------------------------------------
(* ─── Spec ──────────────────────────────────────────────────────────────── *)
Fairness ==
  /\ WF_vars(Next)

Spec == Init /\ [][Next]_vars /\ Fairness

-----------------------------------------------------------------------------
(* ─── Theorems (to be model-checked by TLC) ─────────────────────────────── *)

THEOREM Spec => []TypeInv
THEOREM Spec => []PlanIntegrity
THEOREM Spec => []BoundedRetry
THEOREM Spec => []ReservationClosure
THEOREM Spec => PlanEventualTermination
THEOREM Spec => ReservationEventualClosure

(***************************************************************************)
(* NOTES FOR CICLO 4                                                        *)
(*                                                                          *)
(* Este spec es DRAFT y faltan:                                             *)
(*  - Modelo explícito de idempotency (plan_id repetido → mismo job_id)     *)
(*  - Concurrencia multi-plan (partial order de eventos)                    *)
(*  - Modelado de fair scheduling vs starvation                             *)
(*  - TLC config con constantes específicas + check de counterexamples      *)
(*                                                                          *)
(* Para ejecutar:                                                           *)
(*  1. Abrir en TLA+ Toolbox                                                *)
(*  2. Configurar MODEL: Plans = {p1,p2}, Reservations = {r1,r2},           *)
(*     MaxRetries = 3, MaxRounds = 10                                       *)
(*  3. Añadir invariantes a check: TypeInv, PlanIntegrity, BoundedRetry,    *)
(*     ReservationClosure                                                   *)
(*  4. Run TLC — expected: 0 counterexamples                                *)
(***************************************************************************)

=============================================================================
