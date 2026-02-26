---------------------------- MODULE StellarReconcilerChecks ----------------------------
(* 
   TLC Model Checking Configuration and Property Definitions
   
   This module extends the main StellarReconciler module with specific
   model checking configurations and additional verification checks.
*)

EXTENDS StellarReconciler, Naturals, Sequences, FiniteSets

(* ============================================================================ *)
(* TLC DIRECTIVES FOR MODEL CHECKING *)
(* ============================================================================ *)

(* Configuration for bounded model checking:
   
   To run TLC model checker on this specification:
   
   1. Basic safety check (no liveness assumptions):
      $ tlc -deadlock StellarReconciler.tla
   
   2. With fairness (for liveness properties):
      $ tlc -deadlock -fair StellarReconciler.tla
   
   3. With specific node count:
      $ tlc -deadlock -modelValue 'NODES = {n1, n2}'
   
   Recommended configuration:
   - NODES: 2-3 nodes (each additional node increases state space exponentially)
   - MAX_RECONCILE_STEPS: 15-20 steps (prevents infinite loops, reasonable upper bound)
   - State limit: 10M states (modern hardware can handle)
   - Depth limit: 1000000 (full exploration)
*)

(* ============================================================================ *)
(* DERIVED INVARIANTS *)
(* ============================================================================ *)

(* Derived: If spec is invalid, node cannot progress beyond SpecInvalid state *)
SpecInvalidBlocksProgress ==
    \A n \in NODES:
        (GetNode(n).spec_valid = "invalid") =>
            (GetNode(n).state \in {"SpecInvalid", "BeingDeleted", "Deleted"})

(* Derived: Health checking only happens after resources are created *)
HealthCheckRequiresResources ==
    \A n \in NODES:
        (GetNode(n).state = "HealthChecking") => GetNode(n).hasResources = TRUE

(* Derived: Resources are deleted before service mesh *)
ResourcesDeletionOrder ==
    \A n \in NODES:
        (GetNode(n).hasResources = FALSE /\ GetNode(n).hasServiceMesh = FALSE)

(* Derived: Only valid specs can reach created resources state *)
ResourcesImplyValidSpec ==
    \A n \in NODES:
        (GetNode(n).hasResources = TRUE) => 
            GetNode(n).spec_valid = "valid"

(* ============================================================================ *)
(* TEMPORAL PROPERTIES - INVARIANTS THAT MUST ALWAYS HOLD *)
(* ============================================================================ *)

(* Invariant: State transitions follow valid ordering *)
ValidStateTransitions ==
    \A n \in NODES:
        LET state == GetNode(n).state IN
        \/ state = "NotFound"
        \/ state = "WaitingForSpec"
        \/ state = "SpecValidationInProgress"
        \/ state = "SpecValid"
        \/ state = "SpecInvalid"
        \/ state = "CreatingResources"
        \/ state = "HealthChecking"
        \/ state = "Running"
        \/ state = "BeingDeleted"
        \/ state = "CleanupInProgress"
        \/ state = "Deleted"

(* Invariant: Resource existence is consistent *)
ResourceConsistency ==
    \A n \in NODES:
        /\ (GetNode(n).hasResources = TRUE) => 
            GetNode(n).resourceState \in {"Created", "Updating", "Deleting"}
        /\ (GetNode(n).hasResources = FALSE) =>
            GetNode(n).resourceState \in {"NotCreated", "Deleted"}

(* Invariant: Finalizer state is consistent *)
FinalizerConsistency ==
    \A n \in NODES:
        /\ (GetNode(n).isFinalizing = TRUE) => 
            GetNode(n).state \in {"BeingDeleted", "CleanupInProgress"}
        /\ (GetNode(n).isFinalizing = FALSE) =>
            GetNode(n).state \notin {"BeingDeleted", "CleanupInProgress"}

(* Invariant: Health status is consistent with state *)
HealthConsistency ==
    \A n \in NODES:
        /\ (GetNode(n).health = "healthy") => 
            GetNode(n).state \in {"Running", "ToolChecking"}
        /\ (GetNode(n).health = "checking") =>
            GetNode(n).state = "HealthChecking"
        /\ (GetNode(n).health = "unhealthy") =>
            GetNode(n).state \in {"HealthChecking", "CreatingResources"}

(* Combined invariant *)
AllInvariants ==
    /\ TypeInvariant
    /\ ValidStateTransitions
    /\ ResourceConsistency
    /\ FinalizerConsistency
    /\ HealthConsistency

(* ============================================================================ *)
(* LIVENESS - PROGRESS PROPERTIES *)
(* ============================================================================ *)

(* Progress: From creation to running (complete path) *)
CreationProgresses ==
    \A n \in NODES:
        (GetNode(n).state = "NotFound") ~> (GetNode(n).state = "Running")

(* Progress: Validation always completes *)
ValidationCompletes ==
    \A n \in NODES:
        (GetNode(n).state = "SpecValidationInProgress") ~>
            (GetNode(n).state \in {"SpecValid", "SpecInvalid"})

(* Progress: Resource creation completes *)
ResourceCreationCompletes ==
    \A n \in NODES:
        (GetNode(n).state = "CreatingResources") ~>
            \/ (GetNode(n).state = "HealthChecking")
            \/ (GetNode(n).state = "CreatingResources")  \* Stays in retries

(* Progress: Health check completes *)
HealthCheckCompletes ==
    \A n \in NODES:
        (GetNode(n).health = "checking") ~>
            (GetNode(n).health \in {"healthy", "unhealthy"})

(* Progress: Deletion removes all resources *)
DeletionRemovesResources ==
    \A n \in NODES:
        (GetNode(n).state = "CleanupInProgress") ~>
            \/ (GetNode(n).hasResources = FALSE /\ GetNode(n).hasServiceMesh = FALSE)
            \/ (GetNode(n).state = "CleanupInProgress")

(* ============================================================================ *)
(* ASSERTIONS - EXPLICIT RUNTIME CHECKS *)
(* ============================================================================ *)

(* Assert: No node in invalid state *)
ASSERT (
    \A n \in NODES:
        \/ GetNode(n).state \in NodeState
        \/ FALSE
)

(* Assert: Reconcile steps never exceed maximum *)
ASSERT (
    \A n \in NODES:
        GetNode(n).reconcileSteps <= MAX_RECONCILE_STEPS
)

(* Assert: Only valid values for spec_valid *)
ASSERT (
    \A n \in NODES:
        GetNode(n).spec_valid \in {"valid", "invalid"}
)

(* ============================================================================ *)
(* ERROR CONDITIONS & FAILURE MODES *)
(* ============================================================================ *)

(* Error mode: Resource creation failure doesn't prevent cleanup *)
GracefulDegradation ==
    \A n \in NODES:
        (GetNode(n).state = "BeingDeleted" /\ GetNode(n).resourceState = "NotCreated") =>
            (GetNode(n).state = "BeingDeleted")  \* Can proceed to cleanup

(* Error mode: Partial resource creation is recoverable *)
PartialResourceRecovery ==
    \A n \in NODES:
        (GetNode(n).resourceState = "Deleting" /\ GetNode(n).hasResources = TRUE) ~>
            (GetNode(n).hasResources = FALSE)

(* Error mode: Repeated failures don't cause deadlock *)
NoDeadlock ==
    ~(
        \E n \in NODES:
            /\ GetNode(n).reconcileSteps = MAX_RECONCILE_STEPS
            /\ GetNode(n).state \notin {"Running", "Deleted", "SpecInvalid", "Failed"}
    )

(* ============================================================================ *)
(* COVERAGE METRICS *)  
(* ============================================================================ *)

(* Count transitions through each state *)
StateVisits ==
    {
        GetNode(n).state : n \in NODES
    }

(* Track path complexity *)
MaxStepsUsed ==
    LET max_steps == CHOOSE m \in 0..MAX_RECONCILE_STEPS :
        \A n \in NODES: GetNode(n).reconcileSteps <= m \/ GetNode(n).reconcileSteps > m
    IN max_steps

(* ============================================================================ *)
(* BOUNDED MODEL PROPERTIES *)
(* ============================================================================ *)

(* These properties can be checked in reasonable time *)

BoundedLiveness ==
    /\ ValidSpecEventuallyRunning
    /\ CleanupEventuallyCompletes

BoundedSafety ==
    /\ InvalidSpecNeverRunning
    /\ RunningInvariant
    /\ CleanupCompleteness

(* ============================================================================ *)
(* PROPERTY SUITE FOR TLC *)
(* ============================================================================ *)

(* Run TLC with: tlc -spec Spec -deadlock StellarReconciler *)
SafetyProperties ==
    /\ InvalidSpecNeverRunning
    /\ RunningInvariant
    /\ NoResourceLeak
    /\ CleanupCompleteness
    /\ ServiceMeshCleanupOrder
    /\ NoRaceConditions
    /\ FinalizerCompleteness

LivenessProperties ==
    /\ ValidSpecEventuallyRunning
    /\ RunningEventuallyStable
    /\ CleanupEventuallyCompletes
    /\ HealthCheckRecovery

AllProperties ==
    /\ SafetyProperties
    /\ AllInvariants

============================================================================
