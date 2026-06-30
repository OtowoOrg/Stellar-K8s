---
title: Multi-Tenancy Platform - Overview
epic_id: 870
difficulty: hard
points: 200
status: ready_for_implementation
created: 2026-06-02
---

# Multi-Tenancy Platform with Resource Isolation

## 🎯 Epic Overview

Transform Stellar-K8s into a comprehensive multi-tenancy platform that enables multiple teams/organizations to share Stellar infrastructure with strong resource isolation, quota management, cost allocation, and tenant-specific policies.

## 💼 Business Value

- **Cost Efficiency**: Share infrastructure across teams while maintaining strict isolation (estimated 30% cost reduction)
- **Resource Isolation**: Prevent noisy neighbor problems with hard resource limits and network policies
- **Simplified Management**: Centralized platform for managing multiple tenants
- **Chargeback**: Accurate cost allocation and billing per tenant
- **Self-Service**: Web portal enables teams to manage their own Stellar resources
- **Compliance**: Tenant-specific security and compliance policies

## 📊 Project Summary

| Aspect | Details |
|--------|---------|
| **Duration** | 10 weeks (2 months) |
| **Team Size** | 2-3 engineers |
| **Difficulty** | Hard (200 points) |
| **Phases** | 7 phases, 50+ tasks |
| **Dependencies** | Kubernetes 1.28+, Grafana 9.0+, OIDC provider |

## 🏗️ Architecture Highlights

### Core Components

1. **Tenant Controller** - Reconciles Tenant CRD to provision isolated namespaces, quotas, network policies, and RBAC
2. **Cost Collector** - Daily job that calculates resource usage and generates cost reports per tenant
3. **Webhook Validator** - Enforces tenant quotas and policies at admission time
4. **Portal REST API** - Axum-based HTTP API with OIDC authentication
5. **Portal Web UI** - React-based self-service dashboard for tenant administrators

### Key Features

✅ **Tenant Isolation**
- Dedicated namespace per tenant with network policies
- Cross-tenant traffic blocked by default
- RBAC prevents privilege escalation

✅ **Resource Management**
- ResourceQuota enforcement (CPU, memory, storage, object counts)
- Custom quotas (e.g., max validators, max Horizon nodes)
- Real-time quota validation in webhook

✅ **Cost Tracking**
- Automated daily usage collection from Prometheus
- Configurable cost rates per resource type
- Per-node cost breakdown
- Monthly cost reports

✅ **Hierarchical Tenancy**
- Parent organizations with child tenants
- Child quotas roll up to parent
- Prevent parent deletion with active children

✅ **Self-Service Portal**
- Web UI for tenant administrators
- Create/manage StellarNodes within quota
- View usage metrics and cost reports
- Per-tenant Grafana dashboards

✅ **Security**
- Multi-layer validation (PSS, org standards, tenant policies)
- Image registry allowlists per tenant
- Audit logging with tenant context
- OIDC authentication for portal

## 📂 Repository Structure

```
.kiro/specs/multi-tenancy-platform/
├── README.md          # This file (overview)
├── requirements.md    # Detailed functional requirements
├── design.md          # Architecture and technical design
└── tasks.md           # Implementation task breakdown
```

## 🔄 Implementation Phases

### Phase 1: Foundation (Week 1-2)
- Extend Tenant CRD schema
- Implement Tenant controller
- Namespace provisioning with ResourceQuota
- Network isolation policies
- Basic RBAC per tenant

### Phase 2: Quotas & Enforcement (Week 3)
- Quota enforcement in controller
- Webhook tenant validation
- Quota usage metrics
- Alerts for quota violations

### Phase 3: Cost Tracking (Week 4)
- TenantUsage CRD
- Cost collector job
- Usage API endpoints
- Cost Grafana dashboard

### Phase 4: Portal REST API (Week 5)
- Axum HTTP server scaffolding
- OIDC authentication middleware
- RBAC authorization
- Tenant and Node API endpoints

### Phase 5: Portal UI (Week 6)
- React + Vite + TailwindCSS project
- OIDC login flow
- Dashboard, Nodes, Usage, Costs pages
- Responsive design

### Phase 6: Advanced Features (Week 7-8)
- Hierarchical tenancy (parent/child)
- Per-tenant Grafana dashboards
- Tenant-specific security policies
- Lifecycle management (suspend/reactivate)

### Phase 7: Documentation & Polish (Week 9-10)
- Architecture documentation
- User guides (onboarding, cost config, API)
- Load testing (100 tenants, 1000 nodes)
- Security review
- Performance tuning

## 🚀 Quick Start (After Implementation)

### For Platform Administrators

**1. Install Helm Chart with Multi-Tenancy Enabled**
```bash
helm upgrade --install stellar-operator stellar-k8s/stellar-operator \
  --set tenantController.enabled=true \
  --set portal.enabled=true \
  --set portal.oidc.issuerUrl=https://accounts.google.com \
  --set portal.oidc.clientId=<your-client-id>
```

**2. Create Your First Tenant**
```bash
kubectl apply -f - <<EOF
apiVersion: stellar.org/v1alpha1
kind: Tenant
metadata:
  name: acme-corp
spec:
  tenantId: acme-corp
  displayName: "ACME Corporation"
  namespace: acme-corp-stellar
  quota:
    hard:
      cpu: "16"
      memory: "64Gi"
      storage: "1Ti"
      stellar.org/validators: "2"
  contacts:
    adminEmail: stellar-admin@acme.com
EOF
```

**3. Access Portal**
```bash
kubectl get ingress -n stellar-system stellar-portal-ui
# Navigate to the ingress URL and login with OIDC
```

### For Tenant Administrators

**1. Login to Portal**
- Navigate to portal URL
- Authenticate via OIDC (Google, Okta, etc.)

**2. Create Stellar Nodes**
- Click "Create Node" in the portal
- Select node type (Validator, Horizon, Soroban RPC)
- Configure resources (within your quota)
- Deploy

**3. Monitor Usage**
- View real-time quota utilization
- Check cost trends
- Access Grafana dashboard

## 📋 Acceptance Criteria (Epic-Level)

### Functional
- [ ] Create 10+ tenants with different quotas
- [ ] Each tenant can deploy StellarNodes within quota
- [ ] Quota enforcement prevents exceeding limits
- [ ] Network isolation blocks cross-tenant traffic
- [ ] Cost reports are accurate (<1% error)
- [ ] Portal UI is functional and user-friendly
- [ ] Hierarchical tenancy works (parent + children)

### Performance
- [ ] Tenant controller reconciles in <5s (P95)
- [ ] Webhook responds in <100ms (P99)
- [ ] Portal API responds in <500ms (P95)
- [ ] Cost collector completes in <30s for 100 tenants
- [ ] Support 100+ tenants without degradation

### Security
- [ ] Tenants cannot access other tenants' resources
- [ ] Tenants cannot bypass quotas
- [ ] OIDC authentication works
- [ ] RBAC prevents privilege escalation
- [ ] Audit logs capture all operations

### Operational
- [ ] Controller HA with leader election
- [ ] Portal HA with 2+ replicas
- [ ] Graceful degradation if Grafana unavailable
- [ ] Comprehensive Prometheus metrics
- [ ] Runbooks for troubleshooting

## 🧪 Testing Strategy

### Unit Tests
- CRD validation logic
- Quota calculation functions
- Cost collector calculations
- Policy enforcement logic
- API handlers

### Integration Tests (kind cluster)
- Tenant creation workflow
- Quota enforcement end-to-end
- Network isolation validation
- Webhook admission control
- Cost report generation

### E2E Tests (Cypress)
- Portal login flow
- Create/edit/delete nodes
- View usage and costs
- Admin tenant management

### Load Tests (k6)
- 100 tenants × 10 nodes each
- Webhook throughput (100 req/s)
- Portal API load (1000 concurrent users)
- Cost calculation job performance

## 📚 Documentation

### For Platform Administrators
- [ ] Multi-tenancy architecture guide
- [ ] Tenant onboarding playbook
- [ ] Cost rate configuration guide
- [ ] RBAC setup guide
- [ ] Troubleshooting runbook

### For Tenant Administrators
- [ ] Portal user guide
- [ ] Quota management guide
- [ ] Node creation best practices
- [ ] Cost optimization tips

### For Developers
- [ ] API reference documentation
- [ ] Tenant CRD schema reference
- [ ] Webhook integration guide
- [ ] Portal development setup

## 🎯 Success Metrics

- **Adoption**: 10+ production tenants within 3 months
- **Cost Transparency**: 100% of tenants have accurate cost reports
- **Self-Service**: 80% of node creation via portal (vs kubectl)
- **Efficiency**: 30% reduction in infrastructure costs via multi-tenancy
- **Satisfaction**: >4.5/5 rating from tenant administrators
- **Reliability**: 99.9% uptime for portal and controller

## ⚠️ Risks and Mitigations

| Risk | Impact | Probability | Mitigation |
|------|--------|-------------|------------|
| Quota bypass bugs | High | Low | Extensive webhook testing, fuzzing |
| Cost calculation errors | High | Medium | Unit tests, manual audits |
| Portal security vulnerabilities | Critical | Low | OWASP compliance, security scanning |
| Performance degradation at scale | Medium | Medium | Load testing, caching, horizontal scaling |
| OIDC integration complexity | Medium | High | Early prototype, fallback to static tokens |

## 🔗 Dependencies

### External
- Kubernetes 1.28+ (in-place pod resource updates)
- Grafana 9.0+ (dashboard provisioning)
- Prometheus + kube-state-metrics
- OIDC provider (Google, Okta, Azure AD, etc.)

### Internal (Existing Stellar-K8s Features)
- StellarNode CRD and controller
- Network isolation (stellar.org/network labels)
- Webhook validation pipeline (PSS, org standards)
- Metrics infrastructure (Prometheus, Grafana)
- RBAC patterns from Helm chart

## 📖 References

### External
- [Kubernetes Multi-Tenancy Best Practices](https://kubernetes.io/docs/concepts/security/multi-tenancy/)
- [kube-rs Controller Tutorial](https://kube.rs/controllers/intro/)
- [Axum Web Framework](https://github.com/tokio-rs/axum)
- [Grafana Operator](https://github.com/grafana-operator/grafana-operator)

### Internal Codebase
- `src/crd/tenant.rs` - Base Tenant CRD
- `src/controller/reconciler.rs` - Controller patterns
- `src/controller/network_isolation.rs` - Network safety checks
- `src/webhook/server.rs` - Webhook server structure

## 🤝 Contributing

See detailed implementation tasks in [`tasks.md`](./tasks.md). Each task includes:
- Priority level
- Time estimate
- Dependencies
- Acceptance criteria
- Files to modify/create

## 📝 License

Same as Stellar-K8s project (see root LICENSE file).

---

**Status**: Ready for Implementation  
**Last Updated**: 2026-06-02  
**Issue**: [#870](https://github.com/OtowoOrg/Stellar-K8s/issues/870)
