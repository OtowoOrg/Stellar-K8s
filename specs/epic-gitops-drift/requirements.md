# Requirements: GitOps Drift Detection & Auto-Revert

## Overview
Build a continuous drift detection system that compares live Kubernetes cluster state against Git-declared intent using three-way merge semantics, classifies drift types (manual mutations vs. pending propagation), and provides optional auto-revert capabilities with safety checks.

## Goals
- Detect out-of-band cluster mutations within 60 seconds
- Distinguish between manual mutations and unpropagated Git changes
- Provide actionable diff reports for remediation
- Enable safe auto-revert for selected resource types
- Attribute drift to the actor who made the change

## Non-Goals
- Replace GitOps controllers (Flux, ArgoCD)
- Implement Git repository management
- Provide a full audit trail (delegate to Kubernetes audit logs)
- Support non-Kubernetes resources

## Functional Requirements

### FR1: Three-Way Drift Detection
- **FR1.1**: Maintain base state (last successful Git application)
- **FR1.2**: Fetch live state from Kubernetes API
- **FR1.3**: Fetch desired state from Git repository
- **FR1.4**: Compute three-way diff (base → live, base → git)
- **FR1.5**: Detect changes in live not present in Git (manual mutations)
- **FR1.6**: Detect changes in Git not applied to live (pending propagation)

### FR2: Server-Side-Default Handling
- **FR2.1**: Identify fields added by Kubernetes admission controllers
- **FR2.2**: Identify fields added by mutating webhooks
- **FR2.3**: Exclude server-defaulted fields from drift calculation
- **FR2.4**: Maintain allowlist of known defaulted fields per resource type
- **FR2.5**: Zero false positives for defaulted fields

### FR3: Drift Classification
- **FR3.1**: Classify drift as ManualMutation (live ≠ git, change in live)
- **FR3.2**: Classify drift as PendingPropagation (live ≠ git, change in git)
- **FR3.3**: Classify drift as Healthy (live = git)
- **FR3.4**: Store classification in DriftReport CR
- **FR3.5**: Include confidence score for classification

### FR4: Drift Reporting
- **FR4.1**: Create DriftReport CR per monitored resource
- **FR4.2**: Include structured diff (JSON Patch format)
- **FR4.3**: Include last-modifying actor attribution
- **FR4.4**: Include timestamp of divergence detection
- **FR4.5**: Export drift state as Prometheus metrics
- **FR4.6**: Provide kubectl plugin for drift inspection

### FR5: Auto-Revert Capability
- **FR5.1**: Enable auto-revert per resource class via policy
- **FR5.2**: Execute rollback safety checks before revert
- **FR5.3**: Verify resource is not in use by active workloads
- **FR5.4**: Create backup of current state before revert
- **FR5.5**: Apply Git state to revert mutation
- **FR5.6**: Record revert event with justification
- **FR5.7**: Alert on auto-revert actions

### FR6: Actor Attribution
- **FR6.1**: Query Kubernetes audit logs for resource modifications
- **FR6.2**: Extract user/serviceAccount from audit events
- **FR6.3**: Include source IP and user-agent if available
- **FR6.4**: Store attribution in DriftReport
- **FR6.5**: Handle kubectl, API clients, and controllers as actors

### FR7: Git Integration
- **FR7.1**: Support multiple Git sources (GitHub, GitLab, Gitea)
- **FR7.2**: Poll Git repository for changes (configurable interval)
- **FR7.3**: Support Git branch or tag as desired state
- **FR7.4**: Authenticate via SSH key or token
- **FR7.5**: Cache Git state to reduce API calls

## Non-Functional Requirements

### NFR1: Performance
- **NFR1.1**: Detect drift within 60 seconds of mutation
- **NFR1.2**: Support 10,000+ monitored resources per cluster
- **NFR1.3**: Diff computation under 100ms per resource (p95)
- **NFR1.4**: Git polling frequency configurable (default: 5 minutes)

### NFR2: Reliability
- **NFR2.1**: Drift detection continues if Git unreachable (use cache)
- **NFR2.2**: Auto-revert failures never leave resources in broken state
- **NFR2.3**: Leader election for controller HA
- **NFR2.4**: Drift state persisted across controller restarts

### NFR3: Security
- **NFR3.1**: RBAC for DriftReport and DriftPolicy resources
- **NFR3.2**: Git credentials stored in Kubernetes Secrets
- **NFR3.3**: Audit log for all auto-revert actions
- **NFR3.4**: Read-only access to Kubernetes audit logs

### NFR4: Observability
- **NFR4.1**: Metrics for drift detection rate per resource type
- **NFR4.2**: Metrics for auto-revert success/failure rate
- **NFR4.3**: Metrics for false positive rate
- **NFR4.4**: Structured logs for drift events
- **NFR4.5**: Alerts for sustained drift conditions

## Acceptance Criteria

### AC1: Detection Speed
- Drift detected within 60s of mutation
- Measured by applying manual change and timing drift report creation
- 95% of drifts detected within target window

### AC2: False Positive Rate
- Zero false drift for server-side-defaulted fields
- Test with 100+ resource types including CRDs
- No drift reported for resources matching Git after defaulting

### AC3: Auto-Revert Safety
- Auto-revert path covered by rollback safety checks
- 100% of auto-reverts create backup before change
- Zero cases of broken resources after revert

### AC4: Attribution
- Report attributes drift to last-modifying actor
- 95%+ attribution success rate when audit logs available
- Include username, source IP, and timestamp

## User Stories

### US1: Platform Operator
As a platform operator, I want to know immediately when someone manually modifies cluster state so that I can investigate and remediate.

**Acceptance**: Receive alert within 60s when deployment replicas manually scaled via kubectl.

### US2: Security Auditor
As a security auditor, I want to know who made out-of-band changes so that I can enforce policy compliance.

**Acceptance**: DriftReport shows username, timestamp, and source IP of actor who made change.

### US3: GitOps Engineer
As a GitOps engineer, I want to distinguish between manual mutations and Git changes not yet propagated so that I know which drifts require action.

**Acceptance**: DriftReport classified as ManualMutation vs. PendingPropagation with confidence score.

### US4: SRE Manager
As an SRE manager, I want auto-revert enabled for critical namespaces so that unauthorized changes are automatically rolled back.

**Acceptance**: Enable auto-revert for production namespace, manual change reverted within 2 minutes.

### US5: Application Developer
As an application developer, I want to see actionable diffs for drifted resources so that I can understand what changed.

**Acceptance**: kubectl plugin shows structured diff with field-level changes highlighted.

## Dependencies
- Kubernetes API server
- Git hosting service (GitHub, GitLab, etc.)
- Kubernetes audit logs (for attribution)
- GitOps controller (Flux, ArgoCD) for Git → cluster sync
- JSON Patch library for structured diffs

## Open Questions
1. Should we support drift detection for CRDs from external operators?
2. What is the strategy for handling namespace-level drift?
3. How do we handle drift in resources managed by Helm?
4. Should auto-revert be configurable per field path?
5. What is the retention policy for DriftReport CRs?
6. How do we handle drift in resources with generated names?
