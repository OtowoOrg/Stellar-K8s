#!/usr/bin/env bash
set -euo pipefail

# shellcheck source=lib/common.sh
source "$(dirname "$0")/lib/common.sh"

show_help() {
  cat <<EOF
Usage: $(basename "$0") [-h|--help]

Creates GitHub issues for Stellar-K8s Hard difficulty batch 28 (200 Points each).
These are simpler 200-point issues focused on specific, well-defined features.

Prerequisites:
  - gh CLI installed and authenticated
  - Network access to api.github.com

Optional environment variables:
  REPO                Target repository (default: OtowoOrg/Stellar-K8s)
  DRY_RUN             Set to 1 to print commands without executing

Example:
  REPO=myorg/my-fork DRY_RUN=1 $(basename "$0")
EOF
}

for arg in "$@"; do
  case "$arg" in
    -h|--help) show_help; exit 0 ;;
  esac
done

EXPECTED_ISSUE_COUNT=12
ACTUAL_ISSUE_COUNT=$(grep -c '^gh issue create' "$0")
if [ "$ACTUAL_ISSUE_COUNT" -ne "$EXPECTED_ISSUE_COUNT" ]; then
  echo "ERROR: Expected $EXPECTED_ISSUE_COUNT issue create calls, found $ACTUAL_ISSUE_COUNT." >&2
  exit 1
fi

echo "Creating Batch 28 of 12 Hard (200-point) issues (simpler scope)..."

# Issue 1: Custom Resource Status Conditions
gh issue create --repo "$REPO" \
  --title "Implement comprehensive status conditions for StellarNode CRD" \
  --body "### 🔴 Difficulty: Hard (200 Points)

Add detailed status conditions to the StellarNode CRD following Kubernetes conventions to provide clear visibility into node lifecycle states.

### ✅ Acceptance Criteria
- Add standard condition types (Ready, Progressing, Degraded, Available)
- Implement condition transitions with proper timestamps
- Add human-readable messages and reasons for each condition
- Update controller to set conditions during reconciliation
- Add status subresource to CRD for efficient updates
- Create kubectl plugin command to display status conditions
- Add unit tests for condition logic
- Document condition types and transitions
" --label "stellar-wave,feature,kubernetes"

# Issue 2: Prometheus ServiceMonitor CRD
gh issue create --repo "$REPO" \
  --title "Add automatic ServiceMonitor creation for Prometheus Operator integration" \
  --body "### 🔴 Difficulty: Hard (200 Points)

Automatically create Prometheus ServiceMonitor resources for each StellarNode to enable seamless metrics collection without manual configuration.

### ✅ Acceptance Criteria
- Create ServiceMonitor CRD when StellarNode is created
- Configure proper metric endpoints and scrape intervals
- Add relabeling rules for consistent metric labels
- Support custom scrape configurations via annotations
- Handle ServiceMonitor cleanup on node deletion
- Add integration tests with Prometheus Operator
- Document ServiceMonitor configuration options
- Add examples for common monitoring scenarios
" --label "stellar-wave,feature,observability"

# Issue 3: Node Affinity and Tolerations
gh issue create --repo "$REPO" \
  --title "Add support for custom node affinity and tolerations in StellarNode spec" \
  --body "### 🔴 Difficulty: Hard (200 Points)

Allow users to specify node affinity rules and tolerations in the StellarNode spec to control pod placement on specific nodes.

### ✅ Acceptance Criteria
- Add nodeAffinity field to StellarNode spec
- Add tolerations field to StellarNode spec
- Implement proper validation for affinity rules
- Apply affinity and tolerations to all managed pods
- Support both required and preferred affinity rules
- Add examples for common placement scenarios
- Add unit tests for affinity logic
- Document best practices for node placement
" --label "stellar-wave,feature,kubernetes"

# Issue 4: Custom Environment Variables
gh issue create --repo "$REPO" \
  --title "Support custom environment variables for Stellar Core and Horizon containers" \
  --body "### 🔴 Difficulty: Hard (200 Points)

Allow users to inject custom environment variables into Stellar Core and Horizon containers for advanced configuration scenarios.

### ✅ Acceptance Criteria
- Add env and envFrom fields to StellarNode spec
- Support environment variables from ConfigMaps
- Support environment variables from Secrets
- Add validation to prevent overriding critical variables
- Apply environment variables to all container types
- Add examples showing common use cases
- Add unit tests for environment variable injection
- Document reserved variable names
" --label "stellar-wave,feature"

# Issue 5: Init Container Support
gh issue create --repo "$REPO" \
  --title "Add support for custom init containers in StellarNode pods" \
  --body "### 🔴 Difficulty: Hard (200 Points)

Allow users to specify custom init containers for pre-startup tasks like database migrations, configuration generation, or data seeding.

### ✅ Acceptance Criteria
- Add initContainers field to StellarNode spec
- Support full init container specification
- Ensure init containers run before main containers
- Add volume mount sharing between init and main containers
- Support init container ordering
- Add examples for common init container patterns
- Add integration tests with init containers
- Document init container best practices
" --label "stellar-wave,feature,kubernetes"

# Issue 6: Resource Quota Integration
gh issue create --repo "$REPO" \
  --title "Implement ResourceQuota awareness and validation" \
  --body "### 🔴 Difficulty: Hard (200 Points)

Make the operator aware of namespace ResourceQuotas and validate that StellarNode resource requests fit within available quota before creation.

### ✅ Acceptance Criteria
- Query namespace ResourceQuota before creating pods
- Validate resource requests against available quota
- Provide clear error messages when quota is exceeded
- Add admission webhook validation for quota checks
- Support LimitRange validation
- Add metrics for quota usage per namespace
- Add unit tests for quota validation logic
- Document quota planning guidelines
" --label "stellar-wave,feature,kubernetes"

# Issue 7: Custom Service Annotations
gh issue create --repo "$REPO" \
  --title "Support custom annotations and labels for generated Services" \
  --body "### 🔴 Difficulty: Hard (200 Points)

Allow users to specify custom annotations and labels for Kubernetes Services created by the operator to support cloud provider integrations and service mesh requirements.

### ✅ Acceptance Criteria
- Add serviceAnnotations field to StellarNode spec
- Add serviceLabels field to StellarNode spec
- Apply annotations to all generated Services
- Support annotation templates with variable substitution
- Preserve operator-managed annotations
- Add examples for AWS/GCP/Azure load balancer annotations
- Add unit tests for annotation merging logic
- Document common annotation patterns
" --label "stellar-wave,feature,kubernetes"

# Issue 8: PodSecurityPolicy Support
gh issue create --repo "$REPO" \
  --title "Add PodSecurityStandard and SecurityContext configuration" \
  --body "### 🔴 Difficulty: Hard (200 Points)

Implement comprehensive security context configuration to ensure StellarNode pods comply with Pod Security Standards (restricted, baseline, privileged).

### ✅ Acceptance Criteria
- Add securityContext field to StellarNode spec
- Set secure defaults (non-root, read-only filesystem where possible)
- Support custom security context overrides
- Add validation for security context compatibility
- Ensure compliance with restricted Pod Security Standard
- Add examples for different security levels
- Add unit tests for security context application
- Document security best practices
" --label "stellar-wave,feature,security"

# Issue 9: Horizontal Pod Autoscaler Integration
gh issue create --repo "$REPO" \
  --title "Add automatic HPA creation for Horizon and Soroban RPC nodes" \
  --body "### 🔴 Difficulty: Hard (200 Points)

Automatically create and manage HorizontalPodAutoscaler resources for scalable node types (Horizon, Soroban RPC) based on CPU, memory, or custom metrics.

### ✅ Acceptance Criteria
- Add autoscaling field to StellarNode spec
- Create HPA resources with proper target references
- Support CPU, memory, and custom metric scaling
- Add min/max replica configuration
- Handle HPA cleanup on node deletion
- Add integration tests with metrics server
- Add examples for different scaling strategies
- Document autoscaling best practices
" --label "stellar-wave,feature,performance"

# Issue 10: ConfigMap and Secret Volume Mounts
gh issue create --repo "$REPO" \
  --title "Support mounting arbitrary ConfigMaps and Secrets as volumes" \
  --body "### 🔴 Difficulty: Hard (200 Points)

Allow users to mount additional ConfigMaps and Secrets as volumes in StellarNode pods for custom configuration files and credentials.

### ✅ Acceptance Criteria
- Add volumes field to StellarNode spec
- Add volumeMounts field to StellarNode spec
- Support ConfigMap and Secret volume sources
- Support projected volumes for combining sources
- Add validation for volume name conflicts
- Add examples for common volume mount scenarios
- Add unit tests for volume mounting logic
- Document volume mount best practices
" --label "stellar-wave,feature,kubernetes"

# Issue 11: Liveness and Readiness Probe Customization
gh issue create --repo "$REPO" \
  --title "Allow customization of liveness and readiness probes" \
  --body "### 🔴 Difficulty: Hard (200 Points)

Provide users with the ability to customize liveness and readiness probe configurations to tune health checking for their specific deployment requirements.

### ✅ Acceptance Criteria
- Add livenessProbe field to StellarNode spec
- Add readinessProbe field to StellarNode spec
- Set sensible defaults for each node type
- Support HTTP, TCP, and exec probe types
- Add validation for probe configuration
- Add examples for different probe strategies
- Add unit tests for probe configuration
- Document probe tuning guidelines
" --label "stellar-wave,feature,reliability"

# Issue 12: Priority Class Support
gh issue create --repo "$REPO" \
  --title "Add PriorityClass support for pod scheduling priority" \
  --body "### 🔴 Difficulty: Hard (200 Points)

Allow users to specify PriorityClass for StellarNode pods to control scheduling priority and preemption behavior in resource-constrained clusters.

### ✅ Acceptance Criteria
- Add priorityClassName field to StellarNode spec
- Validate that specified PriorityClass exists
- Apply priority class to all managed pods
- Add default priority classes for different node types
- Support priority-based preemption
- Add examples for different priority scenarios
- Add unit tests for priority class application
- Document priority class best practices
" --label "stellar-wave,feature,kubernetes"

echo "✅ Created 12 hard (200-point) issues successfully!"
echo "Batch 28 issues should now be available in the repository."
