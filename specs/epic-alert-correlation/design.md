# Design: Alert Correlation & Incident Management

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                     Alert Sources                                │
├─────────────┬─────────────┬─────────────┬─────────────────────┤
│ Prometheus  │   Uptime    │ Log-based   │   Other Webhooks    │
│ AlertManager│   Checks    │ Detectors   │                     │
└──────┬──────┴──────┬──────┴──────┬──────┴──────────┬──────────┘
       │             │              │                 │
       └─────────────┴──────────────┴─────────────────┘
                            │
                    ┌───────▼────────┐
                    │ Alert Ingestion│
                    │   Webhook API  │
                    └───────┬────────┘
                            │
                    ┌───────▼────────┐
                    │ Normalization  │
                    │     Engine     │
                    └───────┬────────┘
                            │
                    ┌───────▼────────┐
                    │  Alert Queue   │
                    │    (Kafka)     │
                    └───────┬────────┘
                            │
                    ┌───────▼────────┐
                    │  Correlation   │
                    │     Engine     │
                    │                │
                    │ ┌────────────┐ │
                    │ │Entity Graph│ │
                    │ └────────────┘ │
                    │ ┌────────────┐ │
                    │ │Causality   │ │
                    │ │Window      │ │
                    │ └────────────┘ │
                    └───────┬────────┘
                            │
                    ┌───────▼────────┐
                    │   Incident     │
                    │   Manager      │
                    └───────┬────────┘
                            │
                ┌───────────┴───────────┐
                │                       │
        ┌───────▼────────┐     ┌───────▼────────┐
        │  Incident DB   │     │  Timeline DB   │
        │  (PostgreSQL)  │     │  (TimescaleDB) │
        └───────┬────────┘     └───────┬────────┘
                │                       │
                └───────────┬───────────┘
                            │
                    ┌───────▼────────┐
                    │ Incident API   │
                    │   (REST/gRPC)  │
                    └────────────────┘
```

## Component Design

### 1. Alert Ingestion Service

**Responsibility**: Accept webhooks from multiple alert sources and normalize to internal format.

**Interface**:
```rust
pub struct AlertIngestionService {
    normalizers: HashMap<AlertSource, Box<dyn AlertNormalizer>>,
    queue_producer: KafkaProducer,
    metrics: MetricsRegistry,
}

impl AlertIngestionService {
    pub async fn handle_webhook(
        &self,
        source: AlertSource,
        payload: Vec<u8>,
    ) -> Result<(), IngestionError>;
}
```

**Normalization Schema**:
```rust
pub struct NormalizedAlert {
    pub id: AlertId,
    pub source: AlertSource,
    pub name: String,
    pub severity: Severity,
    pub state: AlertState, // Firing, Resolved
    pub fired_at: Timestamp,
    pub resolved_at: Option<Timestamp>,
    pub labels: HashMap<String, String>,
    pub annotations: HashMap<String, String>,
    pub entity_keys: Vec<EntityKey>,
    pub fingerprint: String,
}

pub struct EntityKey {
    pub entity_type: EntityType, // Service, Cluster, Zone, Pod, Node
    pub entity_id: String,
}
```

**Entity Key Extraction Rules**:
- Service: `labels.service` or `labels.job`
- Cluster: `labels.cluster`
- Zone: `labels.zone` or `labels.availability_zone`
- Pod: `labels.pod` or `labels.pod_name`
- Node: `labels.node` or `labels.instance`

### 2. Correlation Engine

**Responsibility**: Group related alerts into incidents based on entity graph and causality window.

**Core Algorithm**:
```rust
pub struct CorrelationEngine {
    entity_graph: EntityGraph,
    open_incidents: IncidentCache,
    config: CorrelationConfig,
}

pub struct CorrelationConfig {
    pub causality_window_seconds: u64, // Default: 300 (5 min)
    pub entity_graph_depth: u32,       // Default: 2
    pub min_correlation_score: f32,     // Default: 0.6
}

impl CorrelationEngine {
    pub async fn correlate(
        &mut self,
        alert: NormalizedAlert,
    ) -> CorrelationDecision {
        // Step 1: Find candidate incidents within causality window
        let candidates = self.find_candidate_incidents(
            &alert,
            self.config.causality_window_seconds,
        );
        
        // Step 2: Build entity graph for the alert
        let alert_graph = self.entity_graph.build_subgraph(
            &alert.entity_keys,
            self.config.entity_graph_depth,
        );
        
        // Step 3: Score each candidate by graph overlap
        let mut scored_candidates = Vec::new();
        for incident in candidates {
            let score = self.compute_overlap_score(
                &alert_graph,
                &incident.entity_graph,
            );
            scored_candidates.push((incident, score));
        }
        
        // Step 4: Select best match or create new incident
        if let Some((incident, score)) = scored_candidates
            .iter()
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
        {
            if *score >= self.config.min_correlation_score {
                return CorrelationDecision::AddToIncident {
                    incident_id: incident.id,
                    score: *score,
                    reason: self.explain_correlation(&alert, incident),
                };
            }
        }
        
        CorrelationDecision::CreateNewIncident {
            reason: "No matching incident found".to_string(),
        }
    }
    
    fn compute_overlap_score(
        &self,
        graph_a: &EntityGraph,
        graph_b: &EntityGraph,
    ) -> f32 {
        let common_nodes = graph_a.nodes()
            .intersection(graph_b.nodes())
            .count() as f32;
        let total_nodes = graph_a.nodes()
            .union(graph_b.nodes())
            .count() as f32;
        
        // Jaccard similarity
        common_nodes / total_nodes
    }
}
```

**Correlation Decision Types**:
```rust
pub enum CorrelationDecision {
    AddToIncident {
        incident_id: IncidentId,
        score: f32,
        reason: String,
    },
    CreateNewIncident {
        reason: String,
    },
}
```

### 3. Entity Graph

**Responsibility**: Maintain relationships between entities for correlation scoring.

**Data Model**:
```rust
pub struct EntityGraph {
    nodes: HashMap<EntityKey, EntityNode>,
    edges: Vec<EntityEdge>,
}

pub struct EntityNode {
    pub key: EntityKey,
    pub metadata: HashMap<String, String>,
}

pub struct EntityEdge {
    pub from: EntityKey,
    pub to: EntityKey,
    pub relationship: RelationshipType,
}

pub enum RelationshipType {
    RunsOn,      // Pod runs on Node
    PartOf,      // Service part of Cluster
    DependsOn,   // Service depends on Database
    LocatedIn,   // Resource located in Zone
}
```

**Graph Construction**:
- Static relationships from service mesh topology
- Dynamic relationships from observed alert patterns
- Kubernetes object relationships (Pod → Node, Service → Pod)

### 4. Root Cause Analysis

**Responsibility**: Classify alerts as root cause or symptom.

**Algorithm**:
```rust
pub struct RootCauseAnalyzer {
    dependency_graph: DependencyGraph,
}

impl RootCauseAnalyzer {
    pub fn analyze(&self, incident: &Incident) -> RootCauseAnalysis {
        let mut alert_classifications = Vec::new();
        
        for alert in &incident.alerts {
            let classification = self.classify_alert(alert, incident);
            alert_classifications.push((alert.id, classification));
        }
        
        RootCauseAnalysis {
            root_causes: alert_classifications
                .iter()
                .filter(|(_, c)| c.is_root_cause)
                .map(|(id, _)| *id)
                .collect(),
            symptoms: alert_classifications
                .iter()
                .filter(|(_, c)| !c.is_root_cause)
                .map(|(id, c)| (*id, c.suppressed_by.clone()))
                .collect(),
        }
    }
    
    fn classify_alert(
        &self,
        alert: &NormalizedAlert,
        incident: &Incident,
    ) -> AlertClassification {
        // Check if alert's entities are upstream of other alerts
        let downstream_alerts = self.find_downstream_alerts(alert, incident);
        
        if downstream_alerts.is_empty() {
            // Leaf node - likely a symptom
            return AlertClassification {
                is_root_cause: false,
                suppressed_by: self.find_upstream_alert(alert, incident),
                confidence: 0.8,
            };
        }
        
        // Has downstream effects - likely root cause
        AlertClassification {
            is_root_cause: true,
            suppressed_by: None,
            confidence: 0.9,
        }
    }
}
```

### 5. Incident Manager

**Responsibility**: Manage incident lifecycle and state transitions.

**State Machine**:
```
┌──────┐  create   ┌──────┐  all alerts   ┌────────┐
│ None │ ────────> │ Open │  resolved     │ Closed │
└──────┘           └──┬───┘  ───────────> └────────┘
                      │                         │
                      │  new alert              │
                      │  within grace           │
                      └─────────────────────────┘
```

**Interface**:
```rust
pub struct IncidentManager {
    db: IncidentDatabase,
    timeline_db: TimelineDatabase,
    config: IncidentConfig,
}

pub struct IncidentConfig {
    pub auto_close_grace_period_seconds: u64, // Default: 300
    pub max_open_incidents_per_service: usize, // Default: 10
}

impl IncidentManager {
    pub async fn create_incident(
        &self,
        alert: NormalizedAlert,
        correlation_reason: String,
    ) -> Result<Incident, IncidentError>;
    
    pub async fn add_alert_to_incident(
        &self,
        incident_id: IncidentId,
        alert: NormalizedAlert,
        correlation_score: f32,
    ) -> Result<(), IncidentError>;
    
    pub async fn check_auto_close(
        &self,
        incident_id: IncidentId,
    ) -> Result<bool, IncidentError>;
}
```

**Incident Model**:
```rust
pub struct Incident {
    pub id: IncidentId,
    pub status: IncidentStatus,
    pub created_at: Timestamp,
    pub closed_at: Option<Timestamp>,
    pub alerts: Vec<NormalizedAlert>,
    pub entity_graph: EntityGraph,
    pub root_cause_analysis: RootCauseAnalysis,
    pub correlation_decisions: Vec<CorrelationDecision>,
    pub timeline: Vec<TimelineEvent>,
    pub metadata: IncidentMetadata,
}

pub enum IncidentStatus {
    Open,
    Closed,
}

pub struct IncidentMetadata {
    pub affected_services: Vec<String>,
    pub affected_clusters: Vec<String>,
    pub severity: Severity,
    pub alert_count: usize,
    pub suppressed_alert_count: usize,
}
```

### 6. Timeline Store

**Responsibility**: Record chronological events for each incident.

**Schema**:
```rust
pub struct TimelineEvent {
    pub incident_id: IncidentId,
    pub timestamp: Timestamp,
    pub event_type: EventType,
    pub data: serde_json::Value,
}

pub enum EventType {
    IncidentCreated,
    AlertAdded { alert_id: AlertId },
    AlertResolved { alert_id: AlertId },
    RootCauseIdentified { alert_id: AlertId },
    RootCauseChanged { from: AlertId, to: AlertId },
    AlertSuppressed { alert_id: AlertId, suppressed_by: AlertId },
    IncidentClosed { reason: CloseReason },
}
```

**Storage**: TimescaleDB for efficient time-series queries and retention policies.

### 7. Incident API

**REST Endpoints**:
```
GET    /api/v1/incidents              # List incidents
GET    /api/v1/incidents/:id          # Get incident details
GET    /api/v1/incidents/:id/timeline # Get incident timeline
POST   /api/v1/incidents/:id/close    # Manually close incident
GET    /api/v1/incidents/stats        # Correlation statistics
```

**gRPC Service** (for high-throughput integrations):
```protobuf
service IncidentService {
  rpc GetIncident(GetIncidentRequest) returns (Incident);
  rpc ListIncidents(ListIncidentsRequest) returns (stream Incident);
  rpc GetTimeline(GetTimelineRequest) returns (stream TimelineEvent);
  rpc GetCorrelationStats(GetStatsRequest) returns (CorrelationStats);
}
```

## Data Flow

### Alert Processing Flow
1. Alert webhook arrives at Ingestion Service
2. Normalize to internal format, extract entity keys
3. Publish to Kafka alert queue
4. Correlation Engine consumes from queue
5. Build entity graph for alert
6. Find candidate incidents in causality window
7. Score candidates by graph overlap
8. Decision: add to existing incident or create new
9. Incident Manager applies decision
10. Update timeline with event
11. Emit metrics

### Auto-Close Flow
1. Alert resolved event arrives
2. Correlation Engine updates incident
3. Incident Manager checks all alerts in incident
4. If all resolved, start grace period timer
5. If new alert arrives during grace, cancel timer
6. If grace expires, auto-close incident
7. Record close event in timeline

## Configuration

**CorrelationRules CRD**:
```yaml
apiVersion: observability.stellar.io/v1
kind: CorrelationRules
metadata:
  name: default-correlation
spec:
  causalityWindowSeconds: 300
  entityGraphDepth: 2
  minCorrelationScore: 0.6
  autoCloseGracePeriodSeconds: 300
  entityKeyExtractors:
    - type: Service
      labelKeys: ["service", "job"]
    - type: Cluster
      labelKeys: ["cluster"]
    - type: Zone
      labelKeys: ["zone", "availability_zone"]
  dependencyRules:
    - from: Pod
      to: Node
      type: RunsOn
    - from: Service
      to: Database
      type: DependsOn
```

## Metrics

```prometheus
# Alert ingestion
alert_ingestion_total{source="prometheus|uptime|logs"}
alert_ingestion_errors_total{source, error_type}
alert_normalization_duration_seconds

# Correlation
correlation_decisions_total{decision="add_to_incident|create_new"}
correlation_score_histogram
correlation_duration_seconds
incidents_created_total
incidents_closed_total{reason="auto|manual"}

# Effectiveness
alerts_per_incident_histogram
alert_suppression_total
incident_lifetime_seconds
root_cause_changes_total
```

## Testing Strategy

### Unit Tests
- Entity key extraction for all alert formats
- Entity graph overlap scoring
- Root cause classification logic
- State machine transitions

### Integration Tests
- End-to-end alert ingestion to incident creation
- Multi-alert correlation scenarios
- Auto-close with grace period
- Timeline event ordering

### Acceptance Tests
- 50-case replay set with known root causes
- Multi-symptom incident simulation
- Time-to-incident measurement
- Alert reduction calculation

## Deployment Architecture

```
Kubernetes Deployment:
- alert-ingestion: 3 replicas (stateless)
- correlation-engine: 2 replicas (with leader election)
- incident-manager: 2 replicas (with leader election)
- incident-api: 3 replicas (stateless)

Dependencies:
- Kafka: 3 brokers
- PostgreSQL: primary + read replica
- TimescaleDB: for timeline storage
- Redis: for caching open incidents
```

## Migration & Rollout Plan

### Phase 1: Shadow Mode
- Deploy system, ingest alerts but don't suppress
- Compare decisions against manual triage
- Tune correlation parameters

### Phase 2: Partial Rollout
- Enable for 10% of services
- Monitor accuracy and effectiveness
- Adjust entity graph rules

### Phase 3: Full Rollout
- Enable for all services
- Switch paging from raw alerts to incidents
- Monitor reduction metrics
