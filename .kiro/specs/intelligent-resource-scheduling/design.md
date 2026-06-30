# Design: Intelligent Resource Scheduling with ML

## Architecture Overview

This document details the architecture for integrating ML-based intelligent scheduling into the existing Stellar-K8s custom scheduler.

### System Context

```
┌──────────────────────────────────────────────────────────────────────┐
│                      Kubernetes Control Plane                         │
│                                                                       │
│  ┌────────────┐  ┌────────────┐  ┌────────────┐  ┌────────────┐   │
│  │   API      │  │ Controller │  │   etcd     │  │  Kubelet   │   │
│  │  Server    │  │  Manager   │  │            │  │  (nodes)   │   │
│  └────────────┘  └────────────┘  └────────────┘  └────────────┘   │
│         │                                                 │          │
└─────────┼─────────────────────────────────────────────────┼──────────┘
          │                                                 │
          ▼                                                 ▼
┌──────────────────────────────────────────────────────────────────────┐
│              Stellar-K8s Enhanced Scheduler (Rust)                   │
│                                                                       │
│  ┌────────────────────────────────────────────────────────────┐    │
│  │  scheduler::core::Scheduler                                 │    │
│  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐        │    │
│  │  │  Watch Pods │─▶│  Filter     │─▶│  Score &    │        │    │
│  │  │  (unbound)  │  │  Nodes      │  │  Select     │        │    │
│  │  └─────────────┘  └─────────────┘  └──────┬──────┘        │    │
│  │                                            │               │    │
│  │                                            ▼               │    │
│  │                                   ┌─────────────┐          │    │
│  │                                   │  Bind Pod   │          │    │
│  │                                   │  to Node    │          │    │
│  │                                   └─────────────┘          │    │
│  └────────────────────────────────────────────────────────────┘    │
│                           │                                          │
│                           ▼                                          │
│  ┌────────────────────────────────────────────────────────────┐    │
│  │  scheduler::ml::HybridScorer (NEW)                          │    │
│  │  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐    │    │
│  │  │  Rule-Based  │  │  ML-Based    │  │  Combiner    │    │    │
│  │  │  Scoring     │  │  Inference   │  │  (α blend)   │    │    │
│  │  │  (existing)  │  │  (NEW)       │  │              │    │    │
│  │  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘    │    │
│  │         │                  │                  │            │    │
│  │         └──────────────────┴──────────────────┘            │    │
│  │                           │                                │    │
│  │                           ▼                                │    │
│  │                  ┌─────────────────┐                      │    │
│  │                  │  Best Node      │                      │    │
│  │                  └─────────────────┘                      │    │
│  └────────────────────────────────────────────────────────────┘    │
│                                                                       │
│  ┌────────────────────────────────────────────────────────────┐    │
│  │  scheduler::ml::ModelRegistry (NEW)                         │    │
│  │  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐    │    │
│  │  │  ONNX        │  │  Model       │  │  Version     │    │    │
│  │  │  Runtime     │  │  Loader      │  │  Manager     │    │    │
│  │  └──────────────┘  └──────────────┘  └──────────────┘    │    │
│  └────────────────────────────────────────────────────────────┘    │
│                                                                       │
│  ┌────────────────────────────────────────────────────────────┐    │
│  │  scheduler::ml::FeatureExtractor (NEW)                      │    │
│  │  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐    │    │
│  │  │  Pod         │  │  Node        │  │  Cluster     │    │    │
│  │  │  Features    │  │  Features    │  │  Features    │    │    │
│  │  └──────────────┘  └──────────────┘  └──────────────┘    │    │
│  └────────────────────────────────────────────────────────────┘    │
│                                                                       │
│  ┌────────────────────────────────────────────────────────────┐    │
│  │  scheduler::ml::TrainingDataCollector (NEW)                 │    │
│  │  ┌──────────────┐  ┌──────────────┐                       │    │
│  │  │  Outcome     │  │  PostgreSQL  │                       │    │
│  │  │  Tracker     │  │  Writer      │                       │    │
│  │  └──────────────┘  └──────────────┘                       │    │
│  └────────────────────────────────────────────────────────────┘    │
│                                                                       │
│  ┌────────────────────────────────────────────────────────────┐    │
│  │  scheduler::power::PowerMonitor (NEW)                       │    │
│  │  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐    │    │
│  │  │  RAPL        │  │  Cloud API   │  │  Prometheus  │    │    │
│  │  │  Collector   │  │  Collector   │  │  Exporter    │    │    │
│  │  └──────────────┘  └──────────────┘  └──────────────┘    │    │
│  └────────────────────────────────────────────────────────────┘    │
└───────────────────────────────────────────────────────────────────┘
          │                                                 │
          ▼                                                 ▼
┌──────────────────────────────────────────────────────────────────────┐
│                    External Services                                 │
│                                                                       │
│  ┌────────────┐  ┌────────────┐  ┌────────────┐  ┌────────────┐   │
│  │ Redis      │  │ PostgreSQL │  │ Prometheus │  │ VictoriaDB │   │
│  │ (features) │  │ (training) │  │ (metrics)  │  │ (TSDB)     │   │
│  └────────────┘  └────────────┘  └────────────┘  └────────────┘   │
└───────────────────────────────────────────────────────────────────┘
          │
          ▼
┌──────────────────────────────────────────────────────────────────────┐
│              Offline Training Pipeline (Python)                       │
│                                                                       │
│  ┌────────────┐  ┌────────────┐  ┌────────────┐  ┌────────────┐   │
│  │ Data ETL   │─▶│ Feature    │─▶│ Model      │─▶│ ONNX       │   │
│  │            │  │ Engineering│  │ Training   │  │ Export     │   │
│  └────────────┘  └────────────┘  └────────────┘  └────────────┘   │
│                                          │                           │
│                                          ▼                           │
│                                   ┌────────────┐                    │
│                                   │ Model      │                    │
│                                   │ Registry   │                    │
│                                   └────────────┘                    │
└───────────────────────────────────────────────────────────────────┘
```

## Component Design

### 1. ML Inference Engine

#### Purpose
Real-time scoring of (pod, node) pairs using trained ML models during scheduling decisions.

#### Technology Stack
- **Language**: Rust
- **ML Runtime**: `onnxruntime` crate v0.0.14
- **Model Format**: ONNX (Open Neural Network Exchange)
- **Tensor Library**: `ndarray` v0.15

#### Module Structure
```rust
// src/scheduler/ml/mod.rs
pub mod inference;
pub mod features;
pub mod model_registry;
pub mod hybrid_scorer;
pub mod training_collector;
pub mod ab_testing;

// src/scheduler/ml/inference.rs
pub struct MLInferenceEngine {
    runtime: ort::Environment,
    session: ort::Session,
    feature_extractor: Arc<FeatureExtractor>,
    model_version: String,
}

impl MLInferenceEngine {
    pub fn new(model_path: &str) -> Result<Self>;
    pub async fn predict_score(&self, pod: &Pod, node: &Node) -> Result<f64>;
    pub fn reload_model(&mut self, model_path: &str) -> Result<()>;
}
```

#### Key Features
- **Hot Model Reloading**: Replace model without scheduler restart
- **Batch Inference**: Score multiple nodes in single inference call
- **Latency Optimization**: Pre-allocate tensors, cache features
- **Graceful Degradation**: Return None on error, trigger fallback

#### Performance Targets
- Inference latency: p99 < 5ms
- Throughput: >1000 predictions/second
- Memory overhead: <100MB

---

### 2. Feature Extraction

#### Purpose
Transform scheduling context into numerical feature vectors for ML model input.

#### Feature Schema
```rust
// src/scheduler/ml/features.rs
#[derive(Debug, Clone)]
pub struct SchedulingFeatures {
    // Pod features (15 dims)
    pub pod_cpu_request_milli: f64,
    pub pod_memory_request_mb: f64,
    pub pod_qos_class: f64,           // 0=BestEffort, 1=Burstable, 2=Guaranteed
    pub pod_priority: f64,
    pub pod_workload_type: f64,       // 0=validator, 1=horizon, 2=read-replica
    pub pod_has_affinity: f64,        // 0=no, 1=yes
    pub pod_tenant_id_hash: f64,
    
    // Node features (20 dims)
    pub node_allocatable_cpu_milli: f64,
    pub node_allocatable_memory_mb: f64,
    pub node_used_cpu_milli: f64,
    pub node_used_memory_mb: f64,
    pub node_cpu_utilization_pct: f64,
    pub node_memory_utilization_pct: f64,
    pub node_hourly_cost_usd: f64,
    pub node_pod_count: f64,
    pub node_age_hours: f64,
    pub node_is_spot: f64,            // 0=on-demand, 1=spot
    pub node_power_watts: f64,
    pub node_power_efficiency: f64,   // perf / watt
    
    // Topology features (10 dims)
    pub same_zone: f64,               // 0=no, 1=yes
    pub same_region: f64,
    pub quorum_peer_distance: f64,    // avg latency to peers (ms)
    pub zone_pod_count: f64,          // pods in this zone
    pub region_pod_count: f64,
    
    // Cluster features (8 dims)
    pub cluster_total_cpu_utilization: f64,
    pub cluster_total_memory_utilization: f64,
    pub cluster_pending_pod_count: f64,
    pub hour_of_day: f64,             // 0-23
    pub day_of_week: f64,             // 0-6
    
    // Historical features (7 dims)
    pub node_avg_pod_lifetime_hours: f64,
    pub node_recent_eviction_rate: f64,
    pub node_placement_success_rate: f64,
}

impl SchedulingFeatures {
    pub fn to_tensor(&self) -> ndarray::Array1<f32>;
    pub fn feature_names() -> Vec<String>;
}
```

#### Feature Store Integration
```rust
// src/scheduler/ml/feature_store.rs
pub struct FeatureStore {
    redis: redis::Client,
    cache: Arc<RwLock<LruCache<String, CachedFeatures>>>,
}

impl FeatureStore {
    pub async fn get_node_features(&self, node_name: &str) -> Result<NodeFeatures>;
    pub async fn update_node_features(&self, node_name: &str, features: NodeFeatures);
    pub async fn get_historical_features(&self, key: &str) -> Result<HistoricalFeatures>;
}
```

#### Caching Strategy
- **L1 Cache**: In-memory LRU (1000 entries, 10s TTL)
- **L2 Cache**: Redis (10000 entries, 5min TTL)
- **Source of Truth**: Kubernetes API + Prometheus

---

### 3. Hybrid Scorer

#### Purpose
Combine ML-based scores with existing rule-based scores for robust decision-making.

#### Implementation
```rust
// src/scheduler/ml/hybrid_scorer.rs
pub struct HybridScorer {
    ml_engine: Option<MLInferenceEngine>,
    ml_weight: f64,  // α in formula
    ab_tester: AbTester,
}

impl HybridScorer {
    pub async fn score_node(
        &self,
        pod: &Pod,
        node: &Node,
        rule_score: f64,
    ) -> Result<ScoredNode> {
        // Determine variant (ML vs rule-based only)
        let variant = self.ab_tester.assign_variant(pod);
        
        match variant {
            Variant::ML => {
                let ml_score = self.ml_engine
                    .as_ref()
                    .and_then(|e| e.predict_score(pod, node).ok());
                
                let final_score = match ml_score {
                    Some(ml) => self.ml_weight * ml + (1.0 - self.ml_weight) * rule_score,
                    None => rule_score, // Fallback
                };
                
                Ok(ScoredNode {
                    node_name: node.name_any(),
                    total_score: final_score,
                    ml_score,
                    rule_score,
                    variant,
                })
            }
            Variant::RuleBased => {
                Ok(ScoredNode {
                    node_name: node.name_any(),
                    total_score: rule_score,
                    ml_score: None,
                    rule_score,
                    variant,
                })
            }
        }
    }
}
```

#### Scoring Strategy

1. **Phase 1 (10% ML)**: α = 0.5 (equal weight)
2. **Phase 2 (50% ML)**: α = 0.7 (ML-preferred)
3. **Phase 3 (100% ML)**: α = 0.8 (ML-dominant)

#### Fallback Logic
```
if ml_inference_fails or ml_score_invalid:
    use rule_score only
if ml_inference_timeout (>5ms):
    use rule_score only
    log warning
    increment fallback counter
```

---

### 4. Training Data Collector

#### Purpose
Capture scheduling decisions and their outcomes for model training.

#### Data Model
```sql
-- PostgreSQL schema
CREATE TABLE scheduling_events (
    id BIGSERIAL PRIMARY KEY,
    timestamp TIMESTAMPTZ NOT NULL,
    pod_name VARCHAR(255) NOT NULL,
    pod_namespace VARCHAR(255) NOT NULL,
    node_name VARCHAR(255) NOT NULL,
    
    -- Features at scheduling time (JSONB)
    features JSONB NOT NULL,
    
    -- Scores
    ml_score DOUBLE PRECISION,
    rule_score DOUBLE PRECISION,
    final_score DOUBLE PRECISION,
    
    -- Variant
    scheduler_variant VARCHAR(50) NOT NULL,
    
    -- Outcomes (updated post-scheduling)
    placement_success BOOLEAN,
    pod_lifetime_hours DOUBLE PRECISION,
    avg_consensus_latency_ms DOUBLE PRECISION,
    cost_per_hour_usd DOUBLE PRECISION,
    disruption_count INTEGER,
    
    INDEX idx_timestamp (timestamp),
    INDEX idx_node (node_name),
    INDEX idx_variant (scheduler_variant)
);
```

#### Outcome Tracking
```rust
// src/scheduler/ml/training_collector.rs
pub struct TrainingDataCollector {
    db_pool: tokio_postgres::Pool,
    prometheus: PrometheusClient,
}

impl TrainingDataCollector {
    pub async fn record_scheduling_decision(&self, event: SchedulingEvent);
    
    pub async fn update_outcomes(&self) {
        // Run every 5 minutes
        // Query pods scheduled in last 24 hours
        // Fetch current metrics from Prometheus
        // Update scheduling_events table
    }
}
```

---

### 5. Power Monitoring Service

#### Architecture
```rust
// src/scheduler/power/mod.rs
pub mod rapl;
pub mod cloud_provider;
pub mod estimator;
pub mod exporter;

pub struct PowerMonitor {
    collectors: Vec<Box<dyn PowerCollector>>,
    exporter: PrometheusExporter,
}

pub trait PowerCollector: Send + Sync {
    async fn collect(&self) -> Result<Vec<NodePower>>;
}

#[derive(Debug, Clone)]
pub struct NodePower {
    pub node_name: String,
    pub power_watts: f64,
    pub timestamp: DateTime<Utc>,
    pub source: PowerSource,  // RAPL, CloudAPI, Estimated
}
```

#### RAPL Integration (Linux)
```rust
// src/scheduler/power/rapl.rs
pub struct RaplCollector {
    package_energy_path: PathBuf,  // /sys/class/powercap/intel-rapl:0/
}

impl PowerCollector for RaplCollector {
    async fn collect(&self) -> Result<Vec<NodePower>> {
        // Read energy_uj (microjoules)
        // Calculate power = ΔE / Δt
        // Convert to watts
    }
}
```

#### Cloud Provider Integration
```rust
// src/scheduler/power/cloud_provider.rs
pub struct AwsCloudWatchCollector {
    client: aws_sdk_cloudwatch::Client,
}

impl PowerCollector for AwsCloudWatchCollector {
    async fn collect(&self) -> Result<Vec<NodePower>> {
        // Query EC2 instance metrics
        // Estimate power from CPU utilization and instance type
    }
}
```

#### CPU-Based Estimation (Fallback)
```rust
// src/scheduler/power/estimator.rs
pub fn estimate_power_from_cpu(
    instance_type: &str,
    cpu_utilization_pct: f64,
) -> f64 {
    let tdp = get_thermal_design_power(instance_type);  // Watts at 100%
    let idle_power = tdp * 0.3;  // 30% at idle
    idle_power + (tdp - idle_power) * (cpu_utilization_pct / 100.0)
}
```

---

### 6. Offline Training Pipeline

#### Pipeline Architecture
```python
# training/pipeline.py
class TrainingPipeline:
    def __init__(self, config):
        self.db = PostgresConnector(config.db_url)
        self.model_registry = ModelRegistry(config.registry_path)
        
    def run(self):
        # 1. Extract data
        df = self.extract_training_data(days=30)
        
        # 2. Feature engineering
        X, y = self.engineer_features(df)
        
        # 3. Train/val/test split
        X_train, X_val, X_test, y_train, y_val, y_test = split_data(X, y)
        
        # 4. Hyperparameter tuning
        best_params = self.tune_hyperparameters(X_train, y_train, X_val, y_val)
        
        # 5. Train final model
        model = self.train_model(X_train, y_train, best_params)
        
        # 6. Evaluate
        metrics = self.evaluate_model(model, X_test, y_test)
        
        # 7. Export to ONNX
        onnx_path = self.export_to_onnx(model)
        
        # 8. Register model
        self.model_registry.register(onnx_path, metrics)
```

#### Feature Engineering
```python
# training/features.py
def engineer_features(df):
    features = []
    
    # Extract from JSONB column
    for col in ['pod_cpu_request_milli', 'node_cpu_utilization_pct', ...]:
        features.append(df['features'].apply(lambda x: x.get(col, 0)))
    
    # Derived features
    features.append(df['node_used_cpu'] / df['node_allocatable_cpu'])  # utilization
    features.append(df['pod_cpu_request'] / df['node_free_cpu'])        # fit score
    
    # Target engineering
    # Composite score: lower is better
    y = (
        -1.0 * normalize(df['avg_consensus_latency_ms']) +
        -1.0 * normalize(df['cost_per_hour_usd']) +
        1.0 * normalize(df['pod_lifetime_hours']) +
        -10.0 * df['disruption_count']
    )
    
    return pd.DataFrame(features).T, y
```

#### Model Training
```python
# training/model.py
def train_xgboost(X_train, y_train, params):
    model = xgb.XGBRegressor(
        n_estimators=params['n_estimators'],
        max_depth=params['max_depth'],
        learning_rate=params['learning_rate'],
        subsample=params['subsample'],
        colsample_bytree=params['colsample_bytree'],
        objective='reg:squarederror',
        tree_method='hist',
        enable_categorical=False,
    )
    
    model.fit(
        X_train, y_train,
        eval_set=[(X_val, y_val)],
        early_stopping_rounds=50,
        verbose=True,
    )
    
    return model
```

#### ONNX Export
```python
# training/export.py
import onnxmltools
from onnxmltools.convert import convert_xgboost

def export_to_onnx(model, feature_names, output_path):
    initial_type = [('float_input', FloatTensorType([None, len(feature_names)]))]
    onnx_model = convert_xgboost(model, initial_types=initial_type)
    
    # Optimize for inference
    from onnxruntime.transformers import optimizer
    optimized_model = optimizer.optimize_model(onnx_model)
    
    onnxmltools.utils.save_model(optimized_model, output_path)
```

---

### 7. A/B Testing Framework

#### Implementation
```rust
// src/scheduler/ml/ab_testing.rs
pub struct AbTester {
    ml_traffic_percentage: Arc<AtomicU8>,  // 0-100
}

impl AbTester {
