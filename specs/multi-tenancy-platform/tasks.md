---
title: Multi-Tenancy Platform - Implementation Tasks
epic_id: 870
status: draft
created: 2026-06-02
---

# Multi-Tenancy Platform - Implementation Tasks

## Phase 1: Foundation (Week 1-2)

### Task 1.1: Extend Tenant CRD Schema
**Priority**: High  
**Estimate**: 2 days  
**Dependencies**: None

**Acceptance Criteria**:
- [ ] Add hierarchy fields (`parentTenantId`, `isOrganization`)
- [ ] Add lifecycle phase enum (`Provisioning`, `Active`, `Suspended`, `Terminating`)
- [ ] Add security policy spec (`podSecurityStandard`, `imageRegistryAllowlist`, `requiredLabels`)
- [ ] Add dashboard configuration fields
- [ ] Add contact information fields
- [ ] Update TenantStatus with quota status, hierarchy info, dashboard URL
- [ ] Add validation via OpenAPI schema
- [ ] Update CRD YAML in `charts/stellar-operator/crds/`

**Files to Modify**:
- `src/crd/tenant.rs`
- `charts/stellar-operator/crds/tenant.yaml`

---

### Task 1.2: Implement Tenant Controller Scaffolding
**Priority**: High  
**Estimate**: 3 days  
**Dependencies**: Task 1.1

**Acceptance Criteria**:
- [ ] Create `src/controller/tenant_controller.rs`
- [ ] Implement reconcile function with kube-rs Controller
- [ ] Add finalizer handling
- [ ] Register controller in main operator binary
- [ ] Add feature flag `ENABLE_TENANT_CONTROLLER`
- [ ] Add metrics: `stellar_tenant_reconcile_duration_seconds`, `stellar_tenant_reconcile_errors_total`
- [ ] Add unit tests for reconcile logic structure

**Files to Create**:
- `src/controller/tenant_controller.rs`

**Files to Modify**:
- `src/main.rs` (register controller)
- `src/controller/mod.rs` (export tenant_controller)

---

### Task 1.3: Implement Namespace Provisioning
**Priority**: High  
**Estimate**: 2 days  
**Dependencies**: Task 1.2

**Acceptance Criteria**:
- [ ] Function `ensure_tenant_namespace()` creates namespace with labels
- [ ] Labels include: `tenant.stellar.org/id`, `tenant.stellar.org/parent`, `stellar.org/network`
- [ ] Handle namespace already exists (idempotent)
- [ ] Add annotations for tenant metadata
- [ ] Update TenantStatus.namespaceCreated = true
- [ ] Unit test: namespace creation
- [ ] Integration test: end-to-end tenant provisioning

**Files to Modify**:
- `src/controller/tenant_controller.rs`

---

### Task 1.4: Implement ResourceQuota Provisioning
**Priority**: High  
**Estimate**: 2 days  
**Dependencies**: Task 1.2

**Acceptance Criteria**:
- [ ] Function `ensure_resource_quota()` creates/updates ResourceQuota
- [ ] Map `TenantQuotaHard` to Kubernetes ResourceQuota spec
- [ ] Support CPU, memory, storage, PVC count, custom resources
- [ ] Update TenantStatus.quotaCreated = true
- [ ] Update TenantStatus.quotaStatus with used/remaining
- [ ] Unit test: quota generation
- [ ] Integration test: quota enforcement

**Files to Modify**:
- `src/controller/tenant_controller.rs`

---

### Task 1.5: Implement NetworkPolicy Generation
**Priority**: High  
**Estimate**: 3 days  
**Dependencies**: Task 1.2

**Acceptance Criteria**:
- [ ] Function `ensure_network_policies()` generates tenant isolation policies
- [ ] Ingress rules: allow same namespace, monitoring, ingress-controller
- [ ] Egress rules: allow same namespace, DNS, Stellar peer ports, block cross-tenant
- [ ] Reuse patterns from `network_isolation.rs`
- [ ] Update TenantStatus.networkPoliciesCreated = true
- [ ] Unit test: policy generation
- [ ] Integration test: cross-tenant ping blocked

**Files to Modify**:
- `src/controller/tenant_controller.rs`

**Files to Reference**:
- `src/controller/network_isolation.rs`

---

### Task 1.6: Implement RBAC Provisioning
**Priority**: Medium  
**Estimate**: 2 days  
**Dependencies**: Task 1.2

**Acceptance Criteria**:
- [ ] Function `ensure_tenant_rbac()` creates Roles and RoleBindings
- [ ] Create `tenant-admin` Role (full StellarNode CRUD)
- [ ] Create `tenant-viewer` Role (read-only)
- [ ] Bindings for OIDC groups (`tenant-{id}-admin`, `tenant-{id}-viewer`)
- [ ] Update TenantStatus.rbacCreated = true
- [ ] Unit test: RBAC generation

**Files to Modify**:
- `src/controller/tenant_controller.rs`

---

### Task 1.7: Implement Status Updates
**Priority**: Medium  
**Estimate**: 1 day  
**Dependencies**: Tasks 1.3-1.6

**Acceptance Criteria**:
- [ ] Function `update_tenant_status()` patches TenantStatus
- [ ] Update phase based on resource creation progress
- [ ] Calculate and update quota usage
- [ ] Add conditions (Ready, QuotaHealthy, etc.)
- [ ] Set lastReconciled timestamp
- [ ] Unit test: status calculation

**Files to Modify**:
- `src/controller/tenant_controller.rs`

---

## Phase 2: Quotas & Enforcement (Week 3)

### Task 2.1: Implement Webhook Tenant Validation
**Priority**: High  
**Estimate**: 3 days  
**Dependencies**: Phase 1

**Acceptance Criteria**:
- [ ] Create `src/webhook/tenant_validator.rs`
- [ ] Function `validate_stellar_node_for_tenant()` checks:
  - Tenant exists for namespace
  - Tenant is in Active phase
  - Quota would not be exceeded
  - Tenant security policies satisfied
- [ ] Return AdmissionResponse with clear error messages
- [ ] Unit tests for each validation rule
- [ ] Integration test: create node exceeding quota → denied

**Files to Create**:
- `src/webhook/tenant_validator.rs`

**Files to Modify**:
- `src/webhook/server.rs` (call tenant validator)

---

### Task 2.2: Implement Quota Usage Calculation
**Priority**: High  
**Estimate**: 2 days  
**Dependencies**: Task 2.1

**Acceptance Criteria**:
- [ ] Function `calculate_current_resource_usage()` queries ResourceQuota status
- [ ] Function `calculate_node_resources()` computes resources from StellarNodeSpec
- [ ] Check CPU, memory, storage, PVC count
- [ ] Support custom quotas (e.g., `stellar.org/validators`)
- [ ] Cache ResourceQuota with 10s TTL for performance
- [ ] Unit test: usage calculation
- [ ] Unit test: caching behavior

**Files to Modify**:
- `src/webhook/tenant_validator.rs`

---

### Task 2.3: Add Tenant Metrics
**Priority**: Medium  
**Estimate**: 1 day  
**Dependencies**: Phase 1

**Acceptance Criteria**:
- [ ] Add metrics to `src/controller/metrics.rs`:
  - `stellar_tenant_quota_used{tenant_id, resource}`
  - `stellar_tenant_quota_limit{tenant_id, resource}`
  - `stellar_tenant_node_count{tenant_id, node_type}`
  - `stellar_tenant_admission_denials_total{tenant_id, reason}`
- [ ] Export in `/metrics` endpoint
- [ ] Unit test: metrics incremented correctly

**Files to Modify**:
- `src/controller/metrics.rs`
- `src/controller/tenant_controller.rs` (emit metrics)
- `src/webhook/tenant_validator.rs` (emit metrics)

---

### Task 2.4: Create Prometheus Alerts
**Priority**: Medium  
**Estimate**: 1 day  
**Dependencies**: Task 2.3

**Acceptance Criteria**:
- [ ] Create `monitoring/prometheus/tenant-alerts.yaml`
- [ ] Alert: `TenantQuotaNearLimit` (>80% used)
- [ ] Alert: `TenantQuotaExceeded` (100% used)
- [ ] Alert: `TenantNodesDegraded` (any node degraded)
- [ ] Alert: `TenantControllerDown`
- [ ] Test alerts fire correctly

**Files to Create**:
- `monitoring/prometheus/tenant-alerts.yaml`

---

## Phase 3: Cost Tracking (Week 4)

### Task 3.1: Define TenantUsage CRD
**Priority**: High  
**Estimate**: 1 day  
**Dependencies**: None

**Acceptance Criteria**:
- [ ] Create `TenantUsage` CRD in `src/crd/tenant_usage.rs`
- [ ] Fields: tenantId, period (start/end), resources, costs, breakdown
- [ ] Status section for calculated costs
- [ ] Update CRD YAML in `charts/stellar-operator/crds/`
- [ ] Unit test: CRD deserialization

**Files to Create**:
- `src/crd/tenant_usage.rs`
- `charts/stellar-operator/crds/tenant-usage.yaml`

**Files to Modify**:
- `src/crd/mod.rs` (export tenant_usage)

---

### Task 3.2: Implement Cost Collector Job
**Priority**: High  
**Estimate**: 4 days  
**Dependencies**: Task 3.1

**Acceptance Criteria**:
- [ ] Create `src/controller/cost_collector.rs`
- [ ] Function `collect_usage()` queries Prometheus for CPU/memory/storage
- [ ] Calculate costs using configurable rates
- [ ] Generate per-node cost breakdown
- [ ] Create/update TenantUsage CRD
- [ ] Support multiple cost rate configurations (global + per-tenant)
- [ ] Handle Prometheus query failures gracefully
- [ ] Unit test: cost calculation
- [ ] Unit test: Prometheus query construction
- [ ] Integration test: generate usage for 10 tenants

**Files to Create**:
- `src/controller/cost_collector.rs`

**Files to Modify**:
- `src/main.rs` (add cost-collector subcommand)

---

### Task 3.3: Create Cost Collector CronJob
**Priority**: Medium  
**Estimate**: 1 day  
**Dependencies**: Task 3.2

**Acceptance Criteria**:
- [ ] Add CronJob YAML in `charts/stellar-operator/templates/cost-collector-cronjob.yaml`
- [ ] Schedule: daily at 2 AM UTC
- [ ] ServiceAccount with permissions to read Tenants, create TenantUsage
- [ ] ConfigMap for cost rates
- [ ] Resource limits (100m CPU, 128Mi memory)
- [ ] Helm values for schedule and cost rates

**Files to Create**:
- `charts/stellar-operator/templates/cost-collector-cronjob.yaml`
- `charts/stellar-operator/templates/cost-rates-configmap.yaml`

**Files to Modify**:
- `charts/stellar-operator/values.yaml`

---

### Task 3.4: Create Cost Report Grafana Dashboard
**Priority**: Medium  
**Estimate**: 2 days  
**Dependencies**: Task 3.2

**Acceptance Criteria**:
- [ ] Create `monitoring/grafana/tenant-cost-dashboard.json`
- [ ] Panels:
  - Total monthly cost (gauge)
  - Cost trends (line chart)
  - Cost breakdown by node (bar chart)
  - Top 10 most expensive tenants (table)
- [ ] Variables: Tenant ID, time range
- [ ] Export functionality (CSV, PDF)

**Files to Create**:
- `monitoring/grafana/tenant-cost-dashboard.json`

---

## Phase 4: Portal REST API (Week 5)

### Task 4.1: Scaffold Axum API Server
**Priority**: High  
**Estimate**: 2 days  
**Dependencies**: None

**Acceptance Criteria**:
- [ ] Create `src/api/server.rs`
- [ ] Define API routes with Axum
- [ ] Health check endpoint `/health`
- [ ] Prometheus metrics endpoint `/metrics`
- [ ] Error handling with proper HTTP status codes
- [ ] CORS configuration
- [ ] Graceful shutdown
- [ ] Unit test: health check

**Files to Create**:
- `src/api/server.rs`
- `src/api/mod.rs`

**Files to Modify**:
- `src/main.rs` (add api-server subcommand)
- `Cargo.toml` (add axum, tower, tower-http dependencies)

---

### Task 4.2: Implement OIDC Authentication
**Priority**: High  
**Estimate**: 3 days  
**Dependencies**: Task 4.1

**Acceptance Criteria**:
- [ ] Create `src/api/auth.rs`
- [ ] Middleware `auth_middleware()` extracts and validates Bearer token
- [ ] Support OIDC discovery (Google, Okta, Azure AD)
- [ ] Extract user claims (sub, email, groups)
- [ ] Attach UserInfo to request extensions
- [ ] Return 401 for invalid tokens
- [ ] Unit test: token validation
- [ ] Integration test: OIDC flow with mock provider

**Files to Create**:
- `src/api/auth.rs`

**Files to Modify**:
- `Cargo.toml` (add openidconnect, jsonwebtoken dependencies)

---

### Task 4.3: Implement RBAC Authorization
**Priority**: High  
**Estimate**: 2 days  
**Dependencies**: Task 4.2

**Acceptance Criteria**:
- [ ] Function `check_tenant_access()` validates user can access tenant
- [ ] Roles: PlatformAdmin, TenantAdmin, TenantViewer
- [ ] Platform admins can access all tenants
- [ ] Tenant users can only access their tenant
- [ ] Return 403 for unauthorized access
- [ ] Unit test: authorization logic

**Files to Modify**:
- `src/api/auth.rs`

---

### Task 4.4: Implement Tenant API Endpoints
**Priority**: High  
**Estimate**: 3 days  
**Dependencies**: Tasks 4.2, 4.3

**Acceptance Criteria**:
- [ ] Create `src/api/tenants.rs`
- [ ] `GET /api/v1/tenants` - list tenants (platform admin only)
- [ ] `GET /api/v1/tenants/:id` - get tenant details
- [ ] `POST /api/v1/tenants` - create tenant (platform admin only)
- [ ] `PATCH /api/v1/tenants/:id` - update tenant
- [ ] `GET /api/v1/tenants/:id/usage` - get usage stats
- [ ] `GET /api/v1/tenants/:id/costs` - get cost breakdown
- [ ] Unit tests for each endpoint
- [ ] Integration test: CRUD operations

**Files to Create**:
- `src/api/tenants.rs`

---

### Task 4.5: Implement Node API Endpoints
**Priority**: High  
**Estimate**: 3 days  
**Dependencies**: Tasks 4.2, 4.3

**Acceptance Criteria**:
- [ ] Create `src/api/nodes.rs`
- [ ] `GET /api/v1/tenants/:id/nodes` - list tenant's nodes
- [ ] `POST /api/v1/tenants/:id/nodes` - create node (check quota)
- [ ] `GET /api/v1/tenants/:id/nodes/:name` - get node details
- [ ] `PATCH /api/v1/tenants/:id/nodes/:name` - update node
- [ ] `DELETE /api/v1/tenants/:id/nodes/:name` - delete node
- [ ] Validate tenant ownership
- [ ] Unit tests for each endpoint
- [ ] Integration test: node CRUD

**Files to Create**:
- `src/api/nodes.rs`

---

### Task 4.6: Deploy Portal API
**Priority**: Medium  
**Estimate**: 2 days  
**Dependencies**: Tasks 4.1-4.5

**Acceptance Criteria**:
- [ ] Create `charts/stellar-operator/templates/portal-api-deployment.yaml`
- [ ] Deployment with 2 replicas (HA)
- [ ] Service and Ingress
- [ ] ConfigMap for OIDC configuration
- [ ] Secret for OIDC client credentials
- [ ] Resource limits (200m CPU, 256Mi memory)
- [ ] Liveness and readiness probes

**Files to Create**:
- `charts/stellar-operator/templates/portal-api-deployment.yaml`
- `charts/stellar-operator/templates/portal-api-service.yaml`
- `charts/stellar-operator/templates/portal-api-ingress.yaml`

**Files to Modify**:
- `charts/stellar-operator/values.yaml`

---

## Phase 5: Portal UI (Week 6)

### Task 5.1: Scaffold React Project
**Priority**: High  
**Estimate**: 1 day  
**Dependencies**: None

**Acceptance Criteria**:
- [ ] Create `portal/ui/` directory
- [ ] Initialize Vite + React + TypeScript project
- [ ] Add TailwindCSS and shadcn/ui
- [ ] Configure Axios API client
- [ ] Setup routing (react-router-dom)
- [ ] Create basic layout with navigation
- [ ] Build Docker image for production

**Files to Create**:
- `portal/ui/package.json`
- `portal/ui/vite.config.ts`
- `portal/ui/tailwind.config.js`
- `portal/ui/src/main.tsx`
- `portal/ui/Dockerfile`

---

### Task 5.2: Implement OIDC Login Flow
**Priority**: High  
**Estimate**: 2 days  
**Dependencies**: Task 5.1

**Acceptance Criteria**:
- [ ] Create `src/lib/auth.ts` with OIDC client
- [ ] Login page redirects to OIDC provider
- [ ] Handle callback and store tokens in localStorage
- [ ] Axios interceptor adds Bearer token to requests
- [ ] Logout functionality
- [ ] Protected routes require authentication
- [ ] E2E test: login flow (Cypress)

**Files to Create**:
- `portal/ui/src/lib/auth.ts`
- `portal/ui/src/pages/Login.tsx`
- `portal/ui/src/hooks/useAuth.ts`

---

### Task 5.3: Implement Dashboard Page
**Priority**: High  
**Estimate**: 3 days  
**Dependencies**: Task 5.2

**Acceptance Criteria**:
- [ ] Create `src/pages/Dashboard.tsx`
- [ ] Display tenant name and ID
- [ ] Quota usage cards (CPU, memory, storage)
- [ ] Node count summary
- [ ] Cost summary (current month)
- [ ] Recent nodes table
- [ ] Responsive design (mobile-friendly)
- [ ] E2E test: dashboard loads with data

**Files to Create**:
- `portal/ui/src/pages/Dashboard.tsx`
- `portal/ui/src/components/QuotaCard.tsx`
- `portal/ui/src/hooks/useTenant.ts`

---

### Task 5.4: Implement Nodes Page
**Priority**: High  
**Estimate**: 3 days  
**Dependencies**: Task 5.2

**Acceptance Criteria**:
- [ ] Create `src/pages/Nodes.tsx`
- [ ] Table of all tenant's nodes (name, type, status, age)
- [ ] Create node dialog with form validation
- [ ] Edit node functionality
- [ ] Delete node with confirmation
- [ ] Filters: node type, status
- [ ] Search by name
- [ ] E2E test: create/edit/delete node

**Files to Create**:
- `portal/ui/src/pages/Nodes.tsx`
- `portal/ui/src/components/NodeTable.tsx`
- `portal/ui/src/components/CreateNodeDialog.tsx`
- `portal/ui/src/hooks/useNodes.ts`

---

### Task 5.5: Implement Usage Page
**Priority**: Medium  
**Estimate**: 2 days  
**Dependencies**: Task 5.2

**Acceptance Criteria**:
- [ ] Create `src/pages/Usage.tsx`
- [ ] Line charts for CPU/memory/storage over time
- [ ] Time range selector (1d, 7d, 30d, custom)
- [ ] Quota limit lines on charts
- [ ] Export chart as PNG
- [ ] E2E test: usage charts load

**Files to Create**:
- `portal/ui/src/pages/Usage.tsx`
- `portal/ui/src/components/UsageChart.tsx`

---

### Task 5.6: Implement Costs Page
**Priority**: Medium  
**Estimate**: 2 days  
**Dependencies**: Task 5.2

**Acceptance Criteria**:
- [ ] Create `src/pages/Costs.tsx`
- [ ] Monthly cost summary
- [ ] Cost breakdown by node (bar chart)
- [ ] Cost trends (line chart)
- [ ] Export to CSV
- [ ] Historical cost data table
- [ ] E2E test: cost page loads

**Files to Create**:
- `portal/ui/src/pages/Costs.tsx`
- `portal/ui/src/components/CostChart.tsx`

---

### Task 5.7: Deploy Portal UI
**Priority**: Medium  
**Estimate**: 1 day  
**Dependencies**: Tasks 5.1-5.6

**Acceptance Criteria**:
- [ ] Create `charts/stellar-operator/templates/portal-ui-deployment.yaml`
- [ ] Deployment with 2 replicas
- [ ] Nginx container serving static files
- [ ] Service and Ingress
- [ ] Environment variables for API URL
- [ ] Resource limits (50m CPU, 64Mi memory)

**Files to Create**:
- `charts/stellar-operator/templates/portal-ui-deployment.yaml`
- `charts/stellar-operator/templates/portal-ui-service.yaml`
- `charts/stellar-operator/templates/portal-ui-ingress.yaml`
- `portal/ui/.env.production`

---

## Phase 6: Advanced Features (Week 7-8)

### Task 6.1: Implement Hierarchical Tenancy
**Priority**: Medium  
**Estimate**: 3 days  
**Dependencies**: Phase 1

**Acceptance Criteria**:
- [ ] Extend controller to handle `hierarchy` fields
- [ ] Function `validate_parent_tenant()` checks parent exists
- [ ] Function `calculate_hierarchical_quota()` sums child quotas
- [ ] Prevent parent deletion if active children exist
- [ ] Status shows child tenants and total child quota
- [ ] Unit test: hierarchical quota calculation
- [ ] Integration test: create parent + 2 children

**Files to Modify**:
- `src/controller/tenant_controller.rs`

---

### Task 6.2: Implement Grafana Dashboard Provisioning
**Priority**: Medium  
**Estimate**: 3 days  
**Dependencies**: Phase 1

**Acceptance Criteria**:
- [ ] Create `src/controller/grafana.rs`
- [ ] Function `ensure_grafana_dashboard()` provisions dashboard via Grafana Operator
- [ ] Generate dashboard JSON with tenant-specific filters
- [ ] Write dashboard URL to TenantStatus
- [ ] Support Grafana RBAC (tenant users can only see their dashboard)
- [ ] Unit test: dashboard JSON generation
- [ ] Integration test: dashboard created in Grafana

**Files to Create**:
- `src/controller/grafana.rs`

**Files to Modify**:
- `src/controller/tenant_controller.rs`

---

### Task 6.3: Implement Tenant-Specific Security Policies
**Priority**: Medium  
**Estimate**: 2 days  
**Dependencies**: Phase 2

**Acceptance Criteria**:
- [ ] Extend webhook to validate `securityPolicy` fields
- [ ] Function `validate_image_registry()` checks allowlist
- [ ] Function `validate_required_labels()` checks labels present
- [ ] Clear error messages for policy violations
- [ ] Metrics: `stellar_tenant_policy_violations_total{tenant_id, policy}`
- [ ] Unit test: policy validation

**Files to Modify**:
- `src/webhook/tenant_validator.rs`

---

### Task 6.4: Implement Tenant Lifecycle Management
**Priority**: Medium  
**Estimate**: 3 days  
**Dependencies**: Phase 1, Phase 2

**Acceptance Criteria**:
- [ ] Function `suspend_tenant()` scales all nodes to 0
- [ ] Function `reactivate_tenant()` restores original replica counts
- [ ] Block node creation when suspended
- [ ] Generate final cost report on deletion
- [ ] Archive audit logs before cleanup
- [ ] Unit test: suspend/reactivate logic
- [ ] Integration test: suspend → verify nodes scaled to 0

**Files to Modify**:
- `src/controller/tenant_controller.rs`

---

## Phase 7: Documentation & Polish (Week 9-10)

### Task 7.1: Write Architecture Documentation
**Priority**: High  
**Estimate**: 2 days  
**Dependencies**: All phases

**Acceptance Criteria**:
- [ ] Document architecture overview with diagrams
- [ ] Component responsibilities
- [ ] Data model (CRD schemas)
- [ ] Network isolation design
- [ ] Security model
- [ ] Publish to `docs/architecture/multi-tenancy.md`

**Files to Create**:
- `docs/architecture/multi-tenancy.md`

---

### Task 7.2: Write Tenant Onboarding Guide
**Priority**: High  
**Estimate**: 1 day  
**Dependencies**: All phases

**Acceptance Criteria**:
- [ ] Step-by-step onboarding playbook
- [ ] Example Tenant YAML
- [ ] RBAC setup instructions
- [ ] OIDC configuration guide
- [ ] Troubleshooting common issues
- [ ] Publish to `docs/guides/tenant-onboarding.md`

**Files to Create**:
- `docs/guides/tenant-onboarding.md`

---

### Task 7.3: Write Cost Configuration Guide
**Priority**: Medium  
**Estimate**: 1 day  
**Dependencies**: Phase 3

**Acceptance Criteria**:
- [ ] Explain cost rate configuration
- [ ] Example ConfigMap for cost rates
- [ ] Per-tenant cost rate overrides
- [ ] Billing webhook integration
- [ ] Publish to `docs/guides/cost-configuration.md`

**Files to Create**:
- `docs/guides/cost-configuration.md`

---

### Task 7.4: Write API Reference Documentation
**Priority**: Medium  
**Estimate**: 2 days  
**Dependencies**: Phase 4

**Acceptance Criteria**:
- [ ] OpenAPI 3.0 spec for Portal API
- [ ] Example requests/responses for each endpoint
- [ ] Authentication instructions
- [ ] Error codes and messages
- [ ] Publish to `docs/api/portal-api.md`
- [ ] Generate interactive docs with Swagger UI

**Files to Create**:
- `docs/api/portal-api.md`
- `portal/api/openapi.yaml`

---

### Task 7.5: Load Testing
**Priority**: High  
**Estimate**: 2 days  
**Dependencies**: All phases

**Acceptance Criteria**:
- [ ] k6 script: create 100 tenants with 10 nodes each
- [ ] k6 script: webhook throughput test (100 req/s)
- [ ] k6 script: portal API load test (1000 concurrent users)
- [ ] Verify controller P95 < 5s
- [ ] Verify webhook P99 < 100ms
- [ ] Verify portal API P95 < 500ms
- [ ] No crashes or OOMs under load
- [ ] Document results in `docs/performance/multi-tenancy-load-test.md`

**Files to Create**:
- `tests/load/tenant-creation.js`
- `tests/load/webhook-throughput.js`
- `tests/load/portal-api.js`
- `docs/performance/multi-tenancy-load-test.md`

---

### Task 7.6: Security Review
**Priority**: High  
**Estimate**: 2 days  
**Dependencies**: All phases

**Acceptance Criteria**:
- [ ] Audit RBAC configurations
- [ ] Verify secret isolation
- [ ] Test cross-tenant access attempts (should fail)
- [ ] Verify audit logging captures all operations
- [ ] Run OWASP ZAP against Portal API
- [ ] Fix any identified vulnerabilities
- [ ] Document security model in `docs/security/multi-tenancy-security.md`

**Files to Create**:
- `docs/security/multi-tenancy-security.md`

---

## Summary

**Total Phases**: 7  
**Total Tasks**: 50  
**Estimated Duration**: 10 weeks (2 months)  
**Team Size**: 2-3 engineers

**Critical Path**:
1. Phase 1 (Foundation) → Phase 2 (Quotas) → Phase 3 (Costs)
2. Phase 4 (API) can start in parallel with Phase 3
3. Phase 5 (UI) depends on Phase 4
4. Phase 6 (Advanced) can be done in parallel with UI
5. Phase 7 (Polish) happens last

**Risks**:
- OIDC integration complexity (Phase 4.2, 5.2)
- Grafana Operator learning curve (Phase 6.2)
- Load testing may reveal performance issues requiring rework

**Mitigation Strategies**:
- Start OIDC prototype early
- Consider fallback to ConfigMap for Grafana if Operator is too complex
- Reserve Phase 7 time for addressing load test findings
