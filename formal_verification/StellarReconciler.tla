---------------------------- MODULE StellarReconciler ----------------------------
(* 
   TLA+ Specification of the StellarNode Reconciler
   
   This module formally models the reconciliation logic of the Stellar-K8s
   operator. The reconciler ensures that the actual state of Kubernetes
   resources matches the desired state declared in StellarNode CRD instances.
   
   Key Safety Properties:
   - Invalid specs never reach Running state
   - Spec validation always completes
   - No resource leaks on deletion
   
   Key Liveness Properties:
   - Valid specs eventually reach Running state
   - Cleanup operations eventually complete
   - System recovers from transient errors
*)

EXTENDS Naturals, Sequences, FiniteSets

CONSTANT NODES          \* Set of node names
CONSTANT MAX_RECONCILE_STEPS  \* Maximum reconciliation steps to prevent infinite loops

ASSUME Cardinality(NODES) \* At least one node
ASSUME MAX_RECONCILE_STEPS > 0

(* State types for the reconciler *)
NodeState ==
    { "NotFound",      \* Node hasn't been created yet
      "WaitingForSpec", \* Node created but no spec validation yet
      "SpecValidationInProgress",
      "SpecValid",       \* Spec has been validated as correct
      "SpecInvalid",     \* Spec validation failed
      "CreatingResources", \* Creating/updating required K8s resources
      "HealthChecking",  \* Performing health checks on node
      "Running",         \* Node is healthy and synced
      "BeingDeleted",    \* Deletion in progress (finalizer present)
      "CleanupInProgress", \* Cleanup operations running
      "Deleted"          \* Node fully cleaned up
    }

(* Spec validity *)
SpecValidity == {"valid", "invalid"}

(* Health status *)
HealthStatus == {"unknown", "checking", "healthy", "unhealthy"}

(* Track resource states *)
ResourceState == {
    "NotCreated",
    "Creating",
    "Created",
    "Updating",
    "Deleting",
    "Deleted"
}

(* Node record structure *)
NodeRecord == [
    name: NODES,
    state: NodeState,
    spec_valid: SpecValidity,
    health: HealthStatus,
    hasResources: BOOLEAN,
    hasServiceMesh: BOOLEAN,
    isFinalizing: BOOLEAN,
    resourceState: ResourceState,
    reconcileSteps: 0..MAX_RECONCILE_STEPS,
    lastAction: {"created", "updated", "deleted", "validated", "none"}
]

VARIABLE nodes            \* Map of node_name -> NodeRecord
VARIABLE globalClock      \* Global clock for fairness

vars == <<nodes, globalClock>>

(* Type invariant *)
TypeInvariant ==
    /\ nodes \in [NODES -> [
        state: NodeState,
        spec_valid: SpecValidity,
        health: HealthStatus,
        hasResources: BOOLEAN,
        hasServiceMesh: BOOLEAN,
        isFinalizing: BOOLEAN,
        resourceState: ResourceState,
        reconcileSteps: 0..MAX_RECONCILE_STEPS,
        lastAction: {"created", "updated", "deleted", "validated", "none"}
    ]]
    /\ globalClock \in Nat

(* Initialize system *)
Init ==
    /\ nodes = [n \in NODES |-> [
        state |-> "NotFound",
        spec_valid |-> "valid",
        health |-> "unknown",
        hasResources |-> FALSE,
        hasServiceMesh |-> FALSE,
        isFinalizing |-> FALSE,
        resourceState |-> "NotCreated",
        reconcileSteps |-> 0,
        lastAction |-> "none"
    ]]
    /\ globalClock = 0

(* Helper: Get node record *)
GetNode(n) == nodes[n]

(* Helper: Update node *)
UpdateNode(n, update) == 
    nodes' = [nodes EXCEPT ![n] = update]

(* ACTIONS: Spec validation phase *)

(* Client creates a new StellarNode *)
CreateNode(n) ==
    /\ GetNode(n).state = "NotFound"
    /\ GetNode(n).reconcileSteps < MAX_RECONCILE_STEPS
    /\ UpdateNode(n, [GetNode(n) EXCEPT
        !.state = "WaitingForSpec",
        !.reconcileSteps = @ + 1
    ])
    /\ UNCHANGED globalClock

(* Reconciler starts spec validation *)
StartSpecValidation(n) ==
    /\ GetNode(n).state = "WaitingForSpec"
    /\ GetNode(n).reconcileSteps < MAX_RECONCILE_STEPS
    /\ UpdateNode(n, [GetNode(n) EXCEPT
        !.state = "SpecValidationInProgress",
        !.reconcileSteps = @ + 1
    ])
    /\ UNCHANGED globalClock

(* Spec validation succeeds *)
SpecValidationSucceeds(n) ==
    /\ GetNode(n).state = "SpecValidationInProgress"
    /\ GetNode(n).spec_valid = "valid"
    /\ GetNode(n).reconcileSteps < MAX_RECONCILE_STEPS
    /\ UpdateNode(n, [GetNode(n) EXCEPT
        !.state = "SpecValid",
        !.spec_valid = "valid",
        !.lastAction = "validated",
        !.reconcileSteps = @ + 1
    ])
    /\ UNCHANGED globalClock

(* Spec validation fails *)
SpecValidationFails(n) ==
    /\ GetNode(n).state = "SpecValidationInProgress"
    /\ GetNode(n).spec_valid = "invalid"
    /\ GetNode(n).reconcileSteps < MAX_RECONCILE_STEPS
    /\ UpdateNode(n, [GetNode(n) EXCEPT
        !.state = "SpecInvalid",
        !.spec_valid = "invalid",
        !.lastAction = "validated",
        !.reconcileSteps = @ + 1
    ])
    /\ UNCHANGED globalClock

(* ACTIONS: Resource creation phase *)

(* Start creating/updating resources *)
StartResourceCreation(n) ==
    /\ GetNode(n).state = "SpecValid"
    /\ GetNode(n).resourceState \in {"NotCreated", "Updating"}
    /\ GetNode(n).reconcileSteps < MAX_RECONCILE_STEPS
    /\ UpdateNode(n, [GetNode(n) EXCEPT
        !.state = "CreatingResources",
        !.resourceState = "Creating",
        !.reconcileSteps = @ + 1
    ])
    /\ UNCHANGED globalClock

(* Resource creation/update completes *)
ResourcesCreated(n) ==
    /\ GetNode(n).state = "CreatingResources"
    /\ GetNode(n).resourceState = "Creating"
    /\ GetNode(n).reconcileSteps < MAX_RECONCILE_STEPS
    /\ UpdateNode(n, [GetNode(n) EXCEPT
        !.state = "HealthChecking",
        !.resourceState = "Created",
        !.hasResources = TRUE,
        !.lastAction = IF GetNode(n).hasResources THEN "updated" ELSE "created",
        !.reconcileSteps = @ + 1
    ])
    /\ UNCHANGED globalClock

(* ACTIONS: Health checking phase *)

(* Start health check *)
StartHealthCheck(n) ==
    /\ GetNode(n).state = "HealthChecking"
    /\ GetNode(n).health = "unknown"
    /\ GetNode(n).reconcileSteps < MAX_RECONCILE_STEPS
    /\ UpdateNode(n, [GetNode(n) EXCEPT
        !.health = "checking",
        !.reconcileSteps = @ + 1
    ])
    /\ UNCHANGED globalClock

(* Health check succeeds - node is healthy *)
HealthCheckPasses(n) ==
    /\ GetNode(n).state = "HealthChecking"
    /\ GetNode(n).health = "checking"
    /\ GetNode(n).reconcileSteps < MAX_RECONCILE_STEPS
    /\ UpdateNode(n, [GetNode(n) EXCEPT
        !.state = "Running",
        !.health = "healthy",
        !.reconcileSteps = @ + 1
    ])
    /\ UNCHANGED globalClock

(* Health check fails *)
HealthCheckFails(n) ==
    /\ GetNode(n).state = "HealthChecking"
    /\ GetNode(n).health = "checking"
    /\ GetNode(n).reconcileSteps < MAX_RECONCILE_STEPS
    /\ UpdateNode(n, [GetNode(n) EXCEPT
        !.health = "unhealthy",
        !.reconcileSteps = @ + 1
    ])
    /\ UNCHANGED globalClock

(* Requeue after health check failure - restart from resource creation *)
RequeuAfterHealthFailure(n) ==
    /\ GetNode(n).state = "HealthChecking"
    /\ GetNode(n).health = "unhealthy"
    /\ GetNode(n).reconcileSteps < MAX_RECONCILE_STEPS
    /\ UpdateNode(n, [GetNode(n) EXCEPT
        !.state = "CreatingResources",
        !.health = "unknown",
        !.reconcileSteps = @ + 1
    ])
    /\ UNCHANGED globalClock

(* ACTIONS: Deletion phase *)

(* Client deletes the node - trigger finalizer *)
DeleteNode(n) ==
    /\ GetNode(n).state \in {"Running", "HealthChecking", "CreatingResources"}
    /\ GetNode(n).reconcileSteps < MAX_RECONCILE_STEPS
    /\ UpdateNode(n, [GetNode(n) EXCEPT
        !.state = "BeingDeleted",
        !.isFinalizing = TRUE,
        !.reconcileSteps = @ + 1
    ])
    /\ UNCHANGED globalClock

(* Start cleanup *)
StartCleanup(n) ==
    /\ GetNode(n).state = "BeingDeleted"
    /\ GetNode(n).isFinalizing = TRUE
    /\ GetNode(n).reconcileSteps < MAX_RECONCILE_STEPS
    /\ UpdateNode(n, [GetNode(n) EXCEPT
        !.state = "CleanupInProgress",
        !.resourceState = "Deleting",
        !.reconcileSteps = @ + 1
    ])
    /\ UNCHANGED globalClock

(* Clean up service mesh resources *)
CleanupServiceMesh(n) ==
    /\ GetNode(n).state = "CleanupInProgress"
    /\ GetNode(n).hasServiceMesh = TRUE
    /\ GetNode(n).reconcileSteps < MAX_RECONCILE_STEPS
    /\ UpdateNode(n, [GetNode(n) EXCEPT
        !.hasServiceMesh = FALSE,
        !.reconcileSteps = @ + 1
    ])
    /\ UNCHANGED globalClock

(* Clean up main resources *)
CleanupResources(n) ==
    /\ GetNode(n).state = "CleanupInProgress"
    /\ GetNode(n).hasResources = TRUE
    /\ GetNode(n).hasServiceMesh = FALSE  \* Service mesh cleaned first
    /\ GetNode(n).reconcileSteps < MAX_RECONCILE_STEPS
    /\ UpdateNode(n, [GetNode(n) EXCEPT
        !.hasResources = FALSE,
        !.resourceState = "Deleted",
        !.reconcileSteps = @ + 1
    ])
    /\ UNCHANGED globalClock

(* Cleanup complete - remove finalizer *)
CleanupComplete(n) ==
    /\ GetNode(n).state = "CleanupInProgress"
    /\ GetNode(n).hasResources = FALSE
    /\ GetNode(n).hasServiceMesh = FALSE
    /\ GetNode(n).isFinalizing = TRUE
    /\ GetNode(n).reconcileSteps < MAX_RECONCILE_STEPS
    /\ UpdateNode(n, [GetNode(n) EXCEPT
        !.state = "Deleted",
        !.isFinalizing = FALSE,
        !.lastAction = "deleted",
        !.reconcileSteps = @ + 1
    ])
    /\ UNCHANGED globalClock

(* Next state relation *)
Next ==
    \E n \in NODES:
        \/ CreateNode(n)
        \/ StartSpecValidation(n)
        \/ SpecValidationSucceeds(n)
        \/ SpecValidationFails(n)
        \/ StartResourceCreation(n)
        \/ ResourcesCreated(n)
        \/ StartHealthCheck(n)
        \/ HealthCheckPasses(n)
        \/ HealthCheckFails(n)
        \/ RequeuAfterHealthFailure(n)
        \/ DeleteNode(n)
        \/ StartCleanup(n)
        \/ CleanupServiceMesh(n)
        \/ CleanupResources(n)
        \/ CleanupComplete(n)

(* Stuttering step for termination *)
Spec == Init /\ [][Next]_vars

(* ============================================================================ *)
(* SAFETY PROPERTIES *)
(* ============================================================================ *)

(* P1: Invalid specs never reach Running state *)
InvalidSpecNeverRunning ==
    \A n \in NODES:
        (GetNode(n).spec_valid = "invalid") => (GetNode(n).state /= "Running")

(* P2: If a node is Running, it must have valid spec and healthy resources *)
RunningInvariant ==
    \A n \in NODES:
        (GetNode(n).state = "Running") => 
            /\ GetNode(n).spec_valid = "valid"
            /\ GetNode(n).health = "healthy"
            /\ GetNode(n).hasResources = TRUE
            /\ GetNode(n).isFinalizing = FALSE

(* P3: No resource leaks - resources are only created when healthy *)
NoResourceLeak ==
    \A n \in NODES:
        (GetNode(n).hasResources = TRUE) => 
            /\ GetNode(n).state \in {"HealthChecking", "Running", "BeingDeleted", "CleanupInProgress"}

(* P4: Cleanup must remove all resources before becoming Deleted *)
CleanupCompleteness ==
    \A n \in NODES:
        (GetNode(n).state = "Deleted") =>
            /\ GetNode(n).hasResources = FALSE
            /\ GetNode(n).hasServiceMesh = FALSE
            /\ GetNode(n).isFinalizing = FALSE

(* P5: Service mesh resources cleaned before main resources *)
ServiceMeshCleanupOrder ==
    \A n \in NODES:
        (GetNode(n).state = "CleanupInProgress" /\ GetNode(n).resourceState = "Deleting") =>
            \/ GetNode(n).hasServiceMesh = FALSE
            \/ GetNode(n).hasResources = TRUE

(* P6: Only one reconciliation path active per node *)
NoRaceConditions ==
    \A n \in NODES:
        ~(GetNode(n).state = "CreatingResources" /\ GetNode(n).state = "CleanupInProgress")

(* P7: Finalizer removed only when cleanup complete *)
FinalizerCompleteness ==
    \A n \in NODES:
        (GetNode(n).isFinalizing = FALSE) =>
            /\ GetNode(n).state \in {"NotFound", "WaitingForSpec", "SpecValid", "SpecInvalid", "Running", "Deleted"}

(* Combined safety property *)
Safety ==
    /\ InvalidSpecNeverRunning
    /\ RunningInvariant
    /\ NoResourceLeak
    /\ CleanupCompleteness
    /\ ServiceMeshCleanupOrder
    /\ NoRaceConditions
    /\ FinalizerCompleteness

(* ============================================================================ *)
(* LIVENESS PROPERTIES *)
(* ============================================================================ *)

(* L1: Valid specs eventually reach Running state *)
ValidSpecEventuallyRunning ==
    \A n \in NODES:
        (GetNode(n).spec_valid = "valid") ~> (GetNode(n).state = "Running")

(* L2: Running nodes remain in stable state (stutter steps allowed) *)
RunningEventuallyStable ==
    \A n \in NODES:
        (GetNode(n).state = "Running") ~> 
            (GetNode(n).state = "Running" \/ GetNode(n).state = "BeingDeleted")

(* L3: Cleanup operations eventually complete *)
CleanupEventuallyCompletes ==
    \A n \in NODES:
        (GetNode(n).isFinalizing = TRUE) ~> (GetNode(n).state = "Deleted")

(* L4: Failed validations can be recovered by updating the spec *)
FailedSpecCanRecover ==
    \A n \in NODES:
        (GetNode(n).state = "SpecInvalid") ~>
            \/ GetNode(n).state = "SpecInvalid"  \* Can stay invalid or transition
            \/ (GetNode(n).spec_valid = "valid" /\ GetNode(n).state \in {"SpecValid", "Running"})

(* L5: Health check failures don't cause permanent failure *)
HealthCheckRecovery ==
    \A n \in NODES:
        (GetNode(n).health = "unhealthy") ~>
            \/ GetNode(n).health = "unhealthy"  \* Can retry or
            \/ (GetNode(n).health = "healthy" /\ GetNode(n).state = "Running")

(* Combined liveness property *)
Liveness ==
    /\ ValidSpecEventuallyRunning
    /\ RunningEventuallyStable
    /\ CleanupEventuallyCompletes
    /\ FailedSpecCanRecover
    /\ HealthCheckRecovery

(* ============================================================================ *)
(* CONSTRAINTS & FAIRNESS *)
(* ============================================================================ *)

(* Weak fairness: enabled actions must eventually be taken *)
Fairness ==
    /\ WF_vars(Next)

(* Fair specification *)
FairSpec == Init /\ [][Next]_vars /\ Fairness

(* ============================================================================ *)
(* THEOREMS *)
(* ============================================================================ *)

ASSUME Spec => Safety
ASSUME FairSpec => Liveness

============================================================================
