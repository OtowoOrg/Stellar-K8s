# Epic: Latency Attribution & Budget Tracking - Task Breakdown

## Overview
Attribute latency budget consumption to individual code changes by joining deploy events with latency SLI series automatically.

---

## Task 1: Deploy Marker Schema & Event Design
**Story Points:** 5  
**Priority:** P0  
**Owner:** Platform Team

### Description
Define the schema for deploy events that will be emitted to the metrics pipeline. These markers will be used to correlate latency changes with specific code deployments.

### Requirements
- [ ] Design deploy marker schema (JSON/Protobuf)
  - Deployment ID (unique identifier)
  - Version/Change ID (git commit, release tag)
  - Deployment timestamp
  - Environment (prod, staging, canary)
  - Service/Component name
  - Team ownership
  - Rollout status (starting, in-progress, complete, rolled-back)
- [ ] Document marker payload examples
- [ ] Define metric label conventions for join operations
- [ ] Create schema validation

### Acceptance Criteria
- Schema documented and reviewed
- Examples provided for each environment
- Backward compatibility strategy defined
- Validation rules specified

---

## Task 2: Deployment Pipeline Instrumentation
**Story Points:** 8  
**Priority:** P0  
**Owner:** Platform/CI-CD Team

### Description
Instrument the deployment pipeline to emit deploy markers to the metrics system at key lifecycle events.

### Requirements
- [ ] Identify emission points in pipeline:
  - Canary start
  - Canary complete
  - Progressive rollout stages (10%, 50%, 100%)
  - Rollout completion
  - Rollback events
- [ ] Implement marker emission in deployment controller
- [ ] Add metrics client library calls
- [ ] Handle failures gracefully (emit failures don't block deployment)
- [ ] Add logging/tracing for troubleshooting
- [ ] Test in staging environment

### Acceptance Criteria
- Markers emitted for 100% of production deployments
- Emission latency < 500ms
- Zero deployment blocking due to emission failures
- Stageable via feature flags

---

## Task 3: Latency SLI Series Query Templates
**Story Points:** 8  
**Priority:** P0  
**Owner:** Observability Team

### Description
Create PromQL query templates that join deploy markers with latency SLI series to attribute budget consumption.

### Requirements
- [ ] Design join logic between deploy markers and latency metrics
  - Time-window based correlation (e.g., latency in ±5min window around deploy)
  - Progressive rollout attribution (segment latency by rollout stage)
  - Multi-service dependency tracking
- [ ] Create query templates for:
  - Per-change latency impact (vs baseline)
  - Cumulative budget consumption
  - Percentile impact (p50, p95, p99)
  - Budget breach detection
- [ ] Performance optimization:
  - Aggregation strategy
  - Recording rules for common queries
  - Index optimization
- [ ] Validation on historical data

### Acceptance Criteria
- Queries return in < 5s for standard time ranges
- Accuracy validated on synthetic regression data
- Templates work across multi-service deployments
- Recording rules reduce live query complexity by 50%

---

## Task 4: Dashboard & Visualization
**Story Points:** 5  
**Priority:** P1  
**Owner:** Observability Team

### Description
Create dashboards for visualizing latency attribution and budget consumption.

### Requirements
- [ ] Dashboard panels:
  - Per-change latency impact timeline
  - Budget consumption rate
  - Budget status (used vs remaining)
  - Top budget-consuming changes (last 30 days)
  - Regression detection alerts
  - Multi-service correlation view
- [ ] Drill-down capability to deployment/service level
- [ ] Export capability for reporting
- [ ] Mobile/small-screen optimization

### Acceptance Criteria
- Dashboard loads in < 3s
- All queries complete in < 5s
- Mobile-responsive design validated
- Team approval from SRE/Platform leads

---

## Task 5: Regression Detection & Seeding
**Story Points:** 8  
**Priority:** P0  
**Owner:** SRE/QA Team

### Description
Implement regression detection on seeded (synthetic) latency degradations to validate attribution accuracy.

### Requirements
- [ ] Synthetic regression test framework:
  - Inject latency spike in specific service/endpoint
  - Correlate with deployment marker
  - Validate detection in queries
- [ ] Test scenarios:
  - Single service regression (5-50% latency increase)
  - Multi-hop dependency regression
  - Canary-phase regression vs full rollout
  - False positive elimination (natural variation)
- [ ] Automation in CI/CD:
  - Run on deployment to observability systems
  - Validate before production rollout
- [ ] Metrics:
  - Detection rate (% of seeded regressions caught)
  - False positive rate
  - Time-to-detection

### Acceptance Criteria
- > 95% regression detection rate
- < 2% false positive rate
- < 5 minute detection latency
- Automated tests in CI/CD

---

## Task 6: Canary Phase Budget Enforcement
**Story Points:** 5  
**Priority:** P1  
**Owner:** Platform Team

### Description
Integrate latency attribution with canary phase to flag budget-exceeding changes before full rollout.

### Requirements
- [ ] Canary monitoring integration:
  - Query latency attribution during canary phase
  - Compare against budget thresholds
  - Generate alert/signal for deployment controller
- [ ] Decision logic:
  - If budget + canary impact > limit:
    - Option A: Block progression to next stage
    - Option B: Warn operators with estimated impact
  - Configurable per service
- [ ] Configuration:
  - Budget thresholds per service
  - Acceptable impact during canary
  - Action (block vs warn)

### Acceptance Criteria
- Budget-exceeding changes detected during canary
- Dashboard shows "At Risk" status
- Operators can override with acknowledgment
- Automated tests validate decision logic

---

## Task 7: Documentation & Runbooks
**Story Points:** 3  
**Priority:** P1  
**Owner:** Observability Team

### Description
Document the latency attribution system, query patterns, and operational runbooks.

### Requirements
- [ ] User documentation:
  - How to interpret attribution dashboard
  - Query examples and patterns
  - Debugging regression detection
- [ ] Operational runbooks:
  - Troubleshooting emission failures
  - Responding to budget breaches
  - Updating budget thresholds
- [ ] Architecture documentation
- [ ] FAQ and common issues

### Acceptance Criteria
- Docs accessible to on-call team
- Runbooks tested by external reviewer
- < 5 minute MTTR for common issues documented

---

## Dependencies & Milestones

```
Task 1 (Schema)
  ↓
Task 2 (Instrumentation) → Task 5 (Regression Tests)
  ↓                                    ↓
Task 3 (Queries) ← ← ← ← ← ← ← ← ← ← ↓
  ↓
Task 4 (Dashboard) → Task 6 (Canary)
  ↓
Task 7 (Docs)
```

## Success Metrics

| Metric | Target | Success Criteria |
|--------|--------|------------------|
| Deploy coverage | 100% | Every prod deploy has marker |
| Query latency | < 5s (P99) | Fast dashboard feedback |
| Regression detection | > 95% | Catches real degradations |
| False positives | < 2% | Minimal alert fatigue |
| Canary blocking accuracy | > 98% | Prevents bad deployments |
| Documentation | < 5min MTTR | Team can operate independently |

## Rollout Plan

1. **Week 1-2:** Deploy schema and marker emission (Tasks 1-2)
2. **Week 2-3:** Queries and dashboard (Tasks 3-4)
3. **Week 3:** Regression testing and validation (Task 5)
4. **Week 4:** Canary integration and ops runbooks (Tasks 6-7)
5. **Week 5:** Full production rollout and monitoring

## Known Risks

- **Risk:** Emission failures could accumulate; mitigation: circuit breakers
- **Risk:** Query performance under high cardinality; mitigation: pre-aggregation
- **Risk:** Noisy attributions with multi-service changes; mitigation: correlation analysis
