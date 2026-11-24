# Intelligent Resource Scheduling with ML-Based Bin Packing

**Epic**: #877  
**Difficulty**: Hard (200 Points)  
**Duration**: 12 weeks  
**Status**: Planning

## Overview

Enhance the existing Stellar-K8s custom scheduler with ML-based intelligent decision-making that learns from historical placement outcomes to optimize pod placement. The system will integrate machine learning models with the current topology-aware, quorum-proximity, and cost-aware scheduling algorithms to achieve >20% improvement in SCP consensus times, >80% resource utilization, and >30% cost reduction.

## Business Value

- **Performance**: 20-30% reduction in SCP consensus latency through learned placement patterns
- **Cost Efficiency**: 30-40% cost reduction through optimized node selection and bin packing
- **Resource Utilization**: Achieve >80% cluster-wide utilization while maintaining performance
- **Sustainability**: 15-25% reduction in power consumption through power-aware placement
- **Adaptability**: Self-improving scheduler that learns from production workload patterns

## Current State Analysis

### Existing Scheduler Capabilities

The Stellar-K8s scheduler (`src/scheduler/`) already provides a sophisticated foundation:

#### Core Scheduling (`core.rs`)
- Custom Kubernetes scheduler implementation
- Scheduling loop with pod binding
- Node filtering and scoring pipeline
- Prometheus integration for metrics-based decisions
- Integrated latency monitor for dynamic rescheduling

#### Advanced Scoring Algorithms (`scoring.rs`)
- **Quorum Proximity Scoring**: SCP validator-aware placement
  - Parses quorum set TOML from StellarNode CRDs
  - Uses real-time latency metrics from Prometheus
  - Applies anti-affinity penalties (same-node: -1000 points)
  - Topology heuristics (same zone: +50, same region: +100)
  - Latency-based bonus: `(1000 / latency_ms)`
- **Carbon-Aware Scheduling**: Environmental sustainability
  - Routes non-critical workloads to low-carbon regions
  - Mock carbon intensity data (ready for real API integration)
  - Region-based scoring
- **Topology-Based Scoring**: Peer locality optimization
  - Zone/region affinity calculation
  - Center-of-gravity computation

#### Optimization Framework (`optimizer.rs`)
- Multi-objective optimization with weighted scoring:
  - Resource fit (40% default weight)
  - Cost (25% default weight)
  - Locality (20% default weight)
  - Balance (15% default weight)
- Bin packing algorithms
- Feasibility filtering (hard constraints)
- Configurable priority weights

#### Cost Awareness (`cost.rs`)
- Per-node hourly cost tracking
- Cheapest viable node selection
- Spot instance preference
- Cluster cost analysis
- Under-utilization detection

#### Dynamic Latency Monitoring (`latency_monitor.rs`)
- Real-time quorum latency measurement
- Automatic pod eviction when latency > 150ms threshold
- Eviction cooldown (5 minutes)
- Benchmarking with metrics:
  - Pods evaluated
  - Above-threshold count
  - Evictions triggered
  - Avg/max latency

#### Policy Framework (`constraints.rs`, `affinity.rs`)
- Hard and soft constraints
- Affinity/anti-affinity rules
- Topology spread constraints
- Cost budget limits
- Node taint handling

#### Metrics & Observability (`metrics.rs`, `prometheus.rs`)
- Scheduling latency percentiles (p50, p95, p99)
- Success/failure tracking
- Preemption counts
- Cost savings calculation

#### Additional Features
- Preemption (`preemption.rs`)
- Simulation (`simulation.rs`)
- Visualization (`visualization.rs`)

### Gaps Requiring ML Enhancement

1. **No Learning from Historical Data**: Current scheduler uses static rules and weights; doesn't improve from outcomes
2. **No Predictive Placement**: Cannot predict which placements will perform best based on past patterns
3. **Limited Power Awareness**: No tracking or optimization for power consumption
4. **Static Weights**: Priority weights are hardcoded; not adapted to workload patterns
5. **No Feature Engineering**: Rich scheduling context not extracted for model training
6. **No A/B Testing**: Cannot compare ML-based vs rule-based decisions
7. **No Online Learning**: Model doesn't adapt in real-time to cluster changes

## Architecture

### High-Level Design

```
┌─────────────────────────────────────────────────────────────────┐
│                     Kubernetes API Server                        │
└────────────────────────────┬────────────────────────────────────┘
                             │
                             ▼
┌─────────────────────────────────────────────────────────────────┐
│              Stellar-K8s Enhanced Scheduler                      │
│                                                                  │
│  ┌────────────────┐      ┌──────────────────┐                  │
│  │  Scheduling    │──────▶  ML Inference     │                  │
│  │  Loop (core)   │      │  Engine           │                  │
│  └────────────────┘      └──────────────────┘                  │
│         │                         │                              │
│         │                         ▼                              │
│         │                ┌──────────────────┐                  │
│         │                │  Feature Store   │                  │
│         │                │  (Redis/Memory)  │                  │
│         │                └──────────────────┘                  │
│         │                         │                              │
│         ▼                         ▼                              │
│  ┌────────────────┐      ┌──────────────────┐                  │
│  │  Hybrid Scorer │◀─────│  Model Registry  │                  │
│  │  (scoring.rs)  │      │  (ONNX Runtime)  │                  │
│  └────────────────┘      └──────────────────┘                  │
│         │                         │                              │
│         ▼                         ▼                              │
│  ┌────────────────┐      ┌──────────────────┐                  │
│  │  Node Binding  │      │  Training Data   │                  │
│  │                │      │  Collector       │                  │
│  └────────────────┘      └──────────────────┘                  │
└─────────────────────────────────────────────────────────────────┘
                             │
                             ▼
┌─────────────────────────────────────────────────────────────────┐
│                     Offline Training Pipeline                     │
│                                                                  │
│  ┌────────────────┐  ┌──────────────┐  ┌──────────────────┐   │
│  │  Time Series   │─▶│  Feature     │─▶│  Model Training  │   │
│  │  DB (VictoriaDB)│  │  Engineering │  │  (XGBoost/LightGBM)│  │
│  └────────────────┘  └──────────────┘  └──────────────────┘   │
│                                                  │               │
│                                                  ▼               │
│                                         ┌──────────────────┐   │
│                                         │  Model Export    │   │
│                                         │  (ONNX)          │   │
│                                         └──────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
                             │
                             ▼
┌─────────────────────────────────────────────────────────────────┐
│                      Power Monitoring Service                    │
│                                                                  │
│  ┌────────────────┐  ┌──────────────┐  ┌──────────────────┐   │
│  │  Node Power    │─▶│  Aggregation │─▶│  Prometheus      │   │
│  │  Metrics       │  │  & Analysis  │  │  Exporter        │   │
│  └────────────────┘  └──────────────┘  └──────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
```

### Components

#### 1. ML Inference Engine
- **Purpose**: Real-time model inference during scheduling
- **Technology**: ONNX Runtime for Rust
- **Models**: XGBoost/LightGBM exported to ONNX
- **Input**: Feature vectors for (pod, node) pairs
- **Output**: Predicted placement score (0-1)

#### 2. Feature Store
- **Purpose**: Fast access to pre-computed features
- **Technology**: Redis (with fallback to in-memory cache)
- **Features**:
  - Node characteristics (CPU, memory, cost, region, zone)
  - Historical placement outcomes
  - Current cluster state
  - Workload patterns

#### 3. Hybrid Scorer
- **Purpose**: Combine ML predictions with rule-based scores
- **Strategy**: `final_score = α * ml_score + (1-α) * rule_score`
- **Fallback**: Use rule-based scoring if ML unavailable
- **A/B Testing**: Random traffic split for evaluation

#### 4. Training Data Collector
- **Purpose**: Record scheduling decisions and outcomes
- **Metrics Collected**:
  - Placement decision
  - SCP consensus latency (5min, 1hr, 24hr post-placement)
  - Resource utilization
  - Cost incurred
  - Pod disruptions/evictions
  - Power consumption

#### 5. Offline Training Pipeline
- **Purpose**: Train models on historical data
- **Frequency**: Daily retraining
- **Process**:
  1. Extract training data from VictoriaMetrics/Prometheus
  2. Engineer features
  3. Train gradient boosting model
  4. Evaluate on holdout set
  5. Export to ONNX
  6. Deploy to scheduler

#### 6. Power Monitoring Service
- **Purpose**: Track node-level power consumption
- **Methods**:
  - RAPL (Running Average Power Limit) on bare metal
  - Cloud provider APIs (AWS CloudWatch, GCP metrics)
  - Estimated from CPU utilization (fallback)
- **Export**: Prometheus metrics

## Key Features

### F1: ML-Based Scoring
- Train XGBoost model on historical placement outcomes
- Features: pod requirements, node state, topology, historical performance
- Target: Composite score based on latency, utilization, cost
- Inference latency: <5ms per node
- Model update: Daily automated retraining

### F2: Power-Aware Placement
- Track real-time power consumption per node
- Prefer nodes with lower power/utilization ratio
- Consider carbon intensity of region
- Optimization mode: "performance", "balanced", "power-saver"

### F3: Intelligent Weight Tuning
- Learn optimal `PriorityWeights` from cluster behavior
- Adapt to workload patterns (validator-heavy vs read-heavy)
- Per-tenant weight profiles

### F4: Predictive Rescheduling
- Predict when current placement will degrade
- Proactively reschedule before latency threshold hit
- Use time-series models for latency prediction

### F5: Multi-Objective Optimization with RL
- Reinforcement learning for long-term placement strategies
- Reward function: `-cost - latency - power + utilization`
- Policy network trained with PPO (Proximal Policy Optimization)

### F6: A/B Testing Framework
- Split traffic between ML and rule-based schedulers
- Statistical comparison of outcomes
- Gradual rollout of ML scheduler (10% → 50% → 100%)

### F7: Real-Time Feature Engineering
- Extract rich features from cluster state
- Caching for low-latency inference
- Feature versioning for model compatibility

### F8: Online Learning
- Incremental model updates from recent data
- Concept drift detection
- Automatic fallback to previous model if performance degrades

## Success Metrics

### Performance Targets
- **SCP Consensus Latency**: >20% reduction (baseline: ~80ms → target: <64ms)
- **Scheduling Latency**: <10ms p99 (including ML inference)
- **Resource Utilization**: >80% cluster-wide (CPU and memory)
- **Cost Reduction**: >30% vs default scheduler
- **Power Consumption**: >15% reduction in kWh/pod
- **Model Accuracy**: >0.85 AUC-ROC on placement quality prediction

### Reliability Targets
- **Scheduler Uptime**: 99.9%
- **ML Inference Availability**: 99.5% (with rule-based fallback)
- **Model Staleness**: <24 hours
- **A/B Test Statistical Significance**: p < 0.05

## Dependencies

### Runtime Dependencies
- Rust crates:
  - `onnxruntime = "0.0.14"` (ML inference)
  - `redis = "0.24"` (feature store)
  - `tokio-postgres = "0.7"` (training data storage)
  - `ndarray = "0.15"` (feature tensors)
- Kubernetes: 1.28+
- Prometheus: Metrics collection
- Redis: Feature caching
- VictoriaMetrics: Training data storage

### Training Dependencies
- Python 3.11+
- Libraries:
  - `xgboost = "2.0.3"`
  - `lightgbm = "4.1.0"`
  - `scikit-learn = "1.4.0"`
  - `onnx = "1.15.0"`
  - `onnxmltools = "1.12.0"`
  - `pandas = "2.2.0"`
  - `numpy = "1.26.3"`

### Infrastructure
- Storage: 100GB for training data (1 year retention)
- Compute: 4 vCPU, 16GB RAM for training (runs nightly)
- Redis: 2GB memory for feature cache

## Risks and Mitigations

| Risk | Impact | Likelihood | Mitigation |
|------|---------|-----------|------------|
| ML model makes worse decisions than rules | High | Medium | A/B testing, gradual rollout, automatic fallback |
| Inference latency degrades scheduling speed | High | Medium | <5ms SLA, ONNX optimization, feature caching |
| Training pipeline failures | Medium | Low | Rule-based fallback, alerting, model versioning |
| Overfitting to specific workload patterns | Medium | Medium | Cross-validation, regularization, diverse training data |
| Model staleness after cluster changes | Medium | High | Daily retraining, online learning, drift detection |
| Power metrics unavailable | Low | Medium | Fallback to CPU-based estimation |
| Feature store downtime | Medium | Low | In-memory cache fallback, async updates |

## Implementation Phases

### Phase 1: Data Collection Infrastructure (Week 1-2)
- Set up training data collector
- VictoriaMetrics integration
- Outcome tracking (latency, utilization, cost)

### Phase 2: Feature Engineering (Week 2-3)
- Design feature schema
- Implement feature extraction
- Redis feature store
- Historical feature backfill

### Phase 3: Offline Training Pipeline (Week 3-5)
- Training data ETL
- Model training scripts (XGBoost/LightGBM)
- Model evaluation framework
- ONNX export

### Phase 4: ML Inference Integration (Week 5-7)
- ONNX Runtime integration in Rust
- Hybrid scorer implementation
- Feature store client
- Fallback mechanisms

### Phase 5: Power Monitoring (Week 7-8)
- Power metrics collector
- RAPL integration
- Cloud provider API integration
- Prometheus exporter

### Phase 6: A/B Testing Framework (Week 8-9)
- Traffic splitting logic
- Statistical comparison tools
- Grafana dashboards
- Automated rollout

### Phase 7: Advanced Features (Week 9-11)
- Intelligent weight tuning
- Predictive rescheduling
- Online learning
- Concept drift detection

### Phase 8: Testing & Optimization (Week 11-12)
- Performance benchmarking
- Chaos testing
- Production validation
- Documentation

## Documentation

- [Requirements](./requirements.md) - Detailed requirements (R1-R10)
- [Design](./design.md) - Architecture and component design
- [Tasks](./tasks.md) - Implementation task breakdown
- [Examples](./examples.yaml) - Configuration examples
- [Migration Guide](./migration-guide.md) - Upgrade from current scheduler
- [Training Guide](./training-guide.md) - ML model training procedures

## Related Epics

- #870: Multi-Tenancy Platform (per-tenant scheduling policies)
- #875: Service Mesh Integration (topology-aware routing)
- #876: Advanced Backup & Restore (stateful workload placement)
- Performance Dashboard: Monitor scheduler decisions

## References

- [Kubernetes Scheduler](https://kubernetes.io/docs/concepts/scheduling-eviction/kube-scheduler/)
- [Google Borg: Resource Management](https://research.google/pubs/pub43438/)
- [DeepRM: Deep Reinforcement Learning for Resource Management](https://arxiv.org/abs/1810.07439)
- [Azure Resource Scheduler with ML](https://www.microsoft.com/en-us/research/publication/resource-management-with-deep-reinforcement-learning/)
- [XGBoost: A Scalable Tree Boosting System](https://arxiv.org/abs/1603.02754)
