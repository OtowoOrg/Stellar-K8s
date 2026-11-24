# Requirements: Alert Correlation & Incident Management

## Overview
Build an intelligent alert correlation and incident management system that deduplicates and correlates firing alerts from multiple sources (Prometheus, uptime checks, log-based detectors) into unified incidents with causality analysis and automated lifecycle management.

## Goals
- Reduce alert fatigue by grouping related alerts into single incidents
- Identify root causes and suppress derivative symptoms
- Provide clear incident timelines for faster resolution
- Automate incident lifecycle from creation to closure

## Non-Goals
- Replace existing alerting systems (Prometheus, uptime monitors)
- Implement a full-featured ticketing system
- Provide alert routing or on-call scheduling

## Functional Requirements

### FR1: Multi-Source Alert Ingestion
- **FR1.1**: Ingest alerts from Prometheus AlertManager webhooks
- **FR1.2**: Ingest alerts from uptime check systems
- **FR1.3**: Ingest alerts from log-based detector systems
- **FR1.4**: Normalize alert metadata to common schema

### FR2: Alert Correlation Engine
- **FR2.1**: Group alerts by normalized entity key (service, cluster, zone)
- **FR2.2**: Apply causality window for temporal correlation (configurable, default 5 minutes)
- **FR2.3**: Build entity graph from alert labels and relationships
- **FR2.4**: Correlate alerts sharing entity graph nodes
- **FR2.5**: Store correlation decision rationale with incident

### FR3: Root Cause Analysis
- **FR3.1**: Classify alerts as root cause vs. symptom based on dependency graph
- **FR3.2**: Suppress symptom alerts when root cause is firing
- **FR3.3**: Promote symptom to root cause if original root cause resolves first
- **FR3.4**: Track suppression count per incident

### FR4: Incident Timeline
- **FR4.1**: Create unified timeline showing all correlated alerts
- **FR4.2**: Record alert state changes (firing → resolved)
- **FR4.3**: Record correlation decisions and reasons
- **FR4.4**: Record root cause changes
- **FR4.5**: Provide chronological view of incident evolution

### FR5: Incident Lifecycle Management
- **FR5.1**: Auto-create incident when first correlated alert fires
- **FR5.2**: Add alerts to existing open incidents based on correlation
- **FR5.3**: Auto-close incident when all member alerts resolve
- **FR5.4**: Support manual incident closure with reason
- **FR5.5**: Prevent premature closure if new alerts arrive within grace period

### FR6: Incident API & Query
- **FR6.1**: REST API for incident CRUD operations
- **FR6.2**: Query incidents by status, service, timerange
- **FR6.3**: Retrieve incident details including timeline
- **FR6.4**: Export incident data for post-mortem analysis

## Non-Functional Requirements

### NFR1: Performance
- **NFR1.1**: Process incoming alerts within 2 seconds (p95)
- **NFR1.2**: Correlation decision within 5 seconds of alert arrival
- **NFR1.3**: Support 10,000+ alerts per minute ingestion rate
- **NFR1.4**: Incident query response time under 500ms (p95)

### NFR2: Reliability
- **NFR2.1**: 99.9% uptime for alert ingestion
- **NFR2.2**: Zero data loss for ingested alerts
- **NFR2.3**: Graceful degradation if correlation engine fails
- **NFR2.4**: Correlation state persisted for disaster recovery

### NFR3: Observability
- **NFR3.1**: Metrics for alert ingestion rate per source
- **NFR3.2**: Metrics for correlation decision types
- **NFR3.3**: Metrics for incident creation/closure rate
- **NFR3.4**: Metrics for alert suppression effectiveness
- **NFR3.5**: Structured logs for all correlation decisions

### NFR4: Maintainability
- **NFR4.1**: Correlation rules configuration via CRD or config file
- **NFR4.2**: Entity key normalization rules externalized
- **NFR4.3**: Causality window configurable per alert type
- **NFR4.4**: Comprehensive unit and integration tests

## Acceptance Criteria

### AC1: Alert Reduction
- Achieve >= 60% reduction in pages during seeded multi-symptom incidents
- Measured by comparing raw alert count vs. incident count in test scenarios

### AC2: Correlation Transparency
- Every incident payload includes correlation decision explanation
- Explanation shows which alerts were grouped and why
- Shows entity graph used for correlation

### AC3: Time-to-Incident
- Median time from first alert to incident creation reduced by 40%
- Measured against baseline of uncorrelated alert processing

### AC4: Root Cause Accuracy
- Zero missed root causes in 50-case replay set
- Replay set includes known incidents with verified root causes
- All root causes correctly identified in correlation output

## User Stories

### US1: On-Call Engineer
As an on-call engineer, I want to receive a single page for related alerts so that I can focus on the root cause instead of triaging dozens of symptoms.

**Acceptance**: During a service degradation, I receive one incident instead of 20+ individual alerts.

### US2: Incident Responder
As an incident responder, I want to see a unified timeline of all related alerts so that I can understand how the incident evolved.

**Acceptance**: The incident detail view shows chronological events including when each alert fired and resolved.

### US3: Platform Operator
As a platform operator, I want incidents to auto-close when issues resolve so that I don't manually clean up resolved incidents.

**Acceptance**: Incidents automatically transition to closed state when all member alerts clear.

### US4: SRE Manager
As an SRE manager, I want visibility into correlation effectiveness so that I can tune the system and measure alert fatigue reduction.

**Acceptance**: Dashboards show alert-to-incident ratio, suppression statistics, and correlation accuracy metrics.

## Dependencies
- Prometheus AlertManager webhook API
- Uptime check system webhook/API
- Log-based detector webhook/API
- Kubernetes CRD framework (for configuration)
- Time-series database for metrics
- Persistent storage for incident state

## Open Questions
1. What entity types should be supported in the entity graph? (service, pod, node, cluster, zone, region?)
2. Should correlation rules be ML-based or rule-based initially?
3. What is the retention policy for closed incidents?
4. Should we support cross-cluster incident correlation?
5. How do we handle alert flapping (resolve/fire cycles)?
