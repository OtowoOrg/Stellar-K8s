# [EPIC] Deploy Event Correlation with Traffic Anomaly Detection

**Labels:** `epic`, `200-points`, `observability`, `automation`, `phase-3`

## Epic Overview

Correlate deployment events with traffic-shape anomalies to automatically flag deploys that statistically perturb user-facing behavior. This epic enables rapid detection and rollback of performance-degrading or bug-introducing deployments using statistical analysis rather than fixed thresholds, adapting to each service's unique baseline patterns.

## Business Value

- **Rapid incident detection**: Flag problematic deploys within 10 minutes of rollout
- **Service-specific baselining**: Adapts to natural variance in each service's behavior
- **False-positive reduction**: Statistical approach eliminates noise-driven alerts
- **Automated rollback**: Trigger automatic rollback of bad deploys
- **Confidence scoring**: Rank severity of detected anomalies by statistical confidence

## Scope & Requirements

### Core Requirements

1. **Change-Point Detection Algorithm**
   - Implement statistical change-point detection (not fixed thresholds)
   - Adapt to each service's normal variance
   - Support multiple metrics: latency, error rate, throughput, saturation
   - Configure sensitivity per metric/service

2. **Deployment Event Correlation**
   - Watch Deployment and StatefulSet resource changes
   - Record exact deployment timestamp and canary phase
   - Correlate with metrics within rolling time windows
   - Cross-reference with rollout events

3. **Traffic Shape Analysis**
   - Analyze request latency distributions (p50, p95, p99)
   - Monitor error rate changes (4xx, 5xx, network timeouts)
   - Track throughput patterns (QPS, connections)
   - Detect saturation (CPU, memory, queue depth)

4. **Anomaly Scoring**
   - Calculate statistical confidence of detected anomaly
   - Score based on deviation magnitude, consistency, and duration
   - Distinguish between coincidence and causation
   - Output confidence score (0-100%) to alert

5. **Automatic Flagging & Alerting**
   - Create `DeploymentAnomaly` CRD on detection
   - Emit alerts with statistical evidence
   - Flag seeded bad deploys with >= 90% recall rate
   - Keep false-positive rate below 5%

6. **Rollback Automation**
   - Optionally trigger automatic rollback on high-confidence anomalies
   - Preserve rollback decision trail for audit
   - Notify team of automatic rollback
   - Allow manual override

### Non-Functional Requirements

- **Detection latency**: Flag within 10 minutes of deploy (including canary phase)
- **Bad deploy recall**: Detect >= 90% of known problematic deployments
- **False-positive rate**: < 5% across all services
- **Statistical model**: Support timeframes from 5 minutes to 24 hours baseline
- **Metric support**: Minimum latency, error rate, throughput
- **Concurrency**: Handle 100+ deployments/hour across fleet

## Technical Design

### Architecture Components

```
┌─────────────────────────────────────────────────────────────┐
│           Deploy Anomaly Detection System                    │
│  ┌───────────────────────────────────────────────────────┐  │
│  │ Deployment Event Watcher                              │  │
│  │ (tracks rollouts in real-time)                        │  │
│  └───────────────────────────────────────────────────────┘  │
│         │                    │                    │          │
│         v                    v                    v          │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────┐  │
│  │ Pre-Deploy   │    │ Canary Window│    │ Stable Window│  │
│  │ Baseline(1h) │    │ (0-30 min)   │    │ (30-60 min)  │  │
│  └──────────────┘    └──────────────┘    └──────────────┘  │
│         │                    │                    │          │
│         └────────────────────┴────────────────────┘          │
│                       │                                      │
│                       v                                      │
│  ┌────────────────────────────────────────────────────────┐ │
│  │ Change-Point Detection Engine                          │ │
│  │ (CUSUM, Bayesian, Prophet, or custom)                 │ │
│  │ Analyzes: latency, error_rate, throughput, saturation │ │
│  └────────────────────────────────────────────────────────┘ │
│                       │                                      │
│                       v                                      │
│  ┌────────────────────────────────────────────────────────┐ │
│  │ Anomaly Scoring & Confidence Calculation              │ │
│  │ Outputs: [anomaly_type, confidence, metrics_affected] │ │
│  └────────────────────────────────────────────────────────┘ │
│                       │                                      │
│                       v                                      │
│  ┌────────────────────────────────────────────────────────┐ │
│  │ DeploymentAnomaly CRD + AlertManager Integration      │ │
│  └────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────┘
```

### Implementation Approach

1. **DeploymentAnomaly CRD**
   ```yaml
   apiVersion: stellar.org/v1alpha1
   kind: DeploymentAnomaly
   metadata:
     name: horizon-api-2024-11-20-deploy-001
     namespace: stellar-horizon
   spec:
     deployment:
       name: horizon-api
       namespace: stellar-horizon
       image: "stellar-horizon:v4.2.5"
     
     deploymentEvent:
       timestamp: 2024-11-20T10:15:00Z
       canaryStartTime: 2024-11-20T10:15:00Z
       canaryDuration: 5m
       canaryPercentage: 5
     
     detectionWindow:
       preDeployBaseline: 60m
       anomalyWindow: 30m
       analysisStartTime: 2024-11-20T10:15:00Z
     
     anomalies:
       - metricType: "request_latency"
         percentile: p99
         baselineValue: 45  # ms
         observedValue: 320  # ms
         percentageChange: 611
         anomalyType: "increase"
         confidence: 0.94
         pValue: 0.001  # Statistical significance
       
       - metricType: "error_rate_5xx"
         baselineValue: 0.02  # 0.02%
         observedValue: 1.45  # 1.45%
         percentageChange: 7150
         anomalyType: "increase"
         confidence: 0.99
         pValue: 0.0001
     
     overallAnomalyScore: 0.96  # Aggregate confidence
     recommendation: "rollback"  # rollback, investigate, ignore
     severity: "high"  # critical, high, medium, low
   
   status:
     detectionTime: 2024-11-20T10:42:00Z
     timeToDetection: 27m
     verified: true
     verificationMethod: "manual-inspection"  # or auto-verified-by-rollback
     resolved: false
     action: "pending"  # pending, auto-rollback-triggered, manual-rollback, resolved
   ```

2. **Deployment Watcher Controller**
   ```python
   # Pseudocode for detection logic
   
   def watch_deployments():
       while True:
           deployment = get_new_deployment()
           
           # Record deployment event
           record_deployment_event(deployment)
           
           # Wait for canary phase + stabilization
           wait(35 * MINUTES)
           
           # Collect baseline metrics (pre-deploy)
           baseline = fetch_metrics(
               deployment,
               start_time=deployment.timestamp - 60*MIN,
               end_time=deployment.timestamp
           )
           
           # Collect post-deploy metrics
           current = fetch_metrics(
               deployment,
               start_time=deployment.timestamp,
               end_time=deployment.timestamp + 30*MIN
           )
           
           # Run change-point detection
           anomalies = detect_anomalies(baseline, current, deployment)
           
           if anomalies and max_confidence(anomalies) > THRESHOLD:
               create_deployment_anomaly(deployment, anomalies)
               emit_alert(deployment, anomalies)
               
               if should_auto_rollback(anomalies):
                   trigger_rollback(deployment)
   ```

3. **Change-Point Detection Algorithms**
   ```yaml
   # Configuration for different algorithms per metric
   detectionStrategies:
     latency_p99:
       algorithm: "cusum"  # Cumulative Sum Control Chart
       sensitivity: 3.5  # Standard deviations
       windowSize: 5m
       minimumSamples: 100
       
     error_rate_5xx:
       algorithm: "bayesian_blocks"  # Bayesian change-point
       minChangeSize: 0.01  # 1% absolute change
       windowSize: 2m
       minimumSamples: 50
       
     throughput:
       algorithm: "prophet"  # Facebook's Prophet
       yearlySeasonality: false
       weeklySeasonality: true
       dailySeasonality: true
       changePointPrior: 0.01
       
     saturation_cpu:
       algorithm: "static_threshold"  # Fallback for simple metrics
       threshold: 80  # percent
       minimumDuration: 2m
   ```

4. **Baseline Model Training**
   ```yaml
   # Historical metrics aggregated per service
   apiVersion: stellar.org/v1alpha1
   kind: ServiceMetricBaseline
   metadata:
     name: horizon-api-baseline
   spec:
     service: horizon-api
     metrics:
       - name: request_latency_p99
         distribution:
           mean: 45.2  # ms
           stddev: 8.5
           min: 32.1
           max: 78.9
           percentiles:
             p50: 43.2
             p95: 58.3
             p99: 68.1
           samples: 86400
           dataCollectionStart: 2024-10-20T00:00:00Z
         seasonality:
           daily: true
           weekly: true
           yearly: false
       
       - name: error_rate_5xx
         distribution:
           mean: 0.02  # percent
           stddev: 0.005
           samples: 86400
   ```

5. **Confidence Scoring**
   ```python
   # Pseudocode for confidence calculation
   
   def calculate_confidence(baseline, observed, metric_type):
       # Statistical significance
       z_score = abs(observed.mean - baseline.mean) / baseline.stddev
       p_value = calculate_p_value(z_score)
       
       # Magnitude of change
       pct_change = abs(observed.mean - baseline.mean) / baseline.mean
       
       # Duration of anomaly (must persist)
       duration_confidence = min(observed.duration / REQUIRED_DURATION, 1.0)
       
       # Consistency (not random spike)
       consistency = 1.0 - (observed.variance / baseline.variance)
       
       # Aggregate confidence
       confidence = (
           0.3 * min(z_score / 3, 1.0) +  # Statistical significance (3-sigma)
           0.3 * min(pct_change, 1.0) +   # Magnitude (>100% = max confidence)
           0.2 * duration_confidence +    # Must persist for 5+ minutes
           0.2 * max(consistency, 0)      # Consistency check
       )
       
       return confidence * 100  # 0-100%
   ```

## Acceptance Criteria

- [ ] `DeploymentAnomaly` CRD with comprehensive anomaly tracking
- [ ] Deployment event watcher running continuously
- [ ] Change-point detection algorithm with >= 90% recall on seeded bad deploys
- [ ] False-positive rate < 5% across all services in production
- [ ] Detection completes within 10 minutes of deploy (including canary phase)
- [ ] Confidence scoring calibrated against manual verification outcomes
- [ ] Statistical significance (p-value) calculated for all anomalies
- [ ] ServiceMetricBaseline CRD for per-service normalization
- [ ] Auto-rollback capability with configurable confidence thresholds
- [ ] AlertManager integration for anomaly alerting
- [ ] Grafana dashboard showing detected anomalies and trends
- [ ] Comparison between algorithm approaches (CUSUM, Bayesian, Prophet)
- [ ] E2E tests: seeded bad deploys flagged, good deploys pass, false positives < 5%
- [ ] Production baseline collection for all monitored services
- [ ] Documentation on tuning per-service sensitivity

## Dependencies & Blockers

- Requires Prometheus or similar metrics backend with 1-hour+ retention
- Depends on Deployment/StatefulSet event webhooks for precise timestamps
- Needs service-specific metric collection (may require Prometheus scrape tuning)
- May require manual baseline training for 1-2 weeks before detection is reliable

## Testing Strategy

### Unit Tests
- Change-point detection algorithms (CUSUM, Bayesian)
- Confidence score calculations
- Baseline distribution modeling

### Integration Tests
- Deployment watcher behavior
- Metrics collection and preprocessing
- CRD creation and status updates

### E2E Tests
- Real deployment with latency increase (verify detection)
- Real deployment with error rate increase (verify detection)
- Normal deployment without issues (verify no false positive)
- Multiple simultaneous deployments (verify isolation)
- Degraded metrics from external factors (not deploy-related)

### Chaos Tests
- Deployment with 50% latency increase (verify detected)
- Deployment with 10x error rate increase (verify detected)
- Deployment with 3% error rate increase (verify confidence < 5%)
- Simultaneous external load spike + deploy (verify attributed correctly)
- Network partition affecting one region (verify regional anomalies)

### Validation Tests (Production)
- Collect baseline metrics for 1-2 weeks
- Manual review of baseline quality
- Tuning sensitivity per service
- A/B testing: detection on vs off
- Measuring false-positive rate in production

## Estimated Effort

**200 Story Points** (~4-6 weeks for 2 engineers)

## Related Issues

- #TBD: Automatic rollback workflow
- #TBD: Metric collection optimization for large deployments
- #TBD: Statistical model comparison and benchmarking

## References

- [Change-Point Detection Survey](https://arxiv.org/abs/2110.03624)
- [CUSUM Algorithm](https://en.wikipedia.org/wiki/CUSUM)
- [Bayesian Blocks](https://arxiv.org/abs/1309.2392)
- [Prophet Forecasting](https://facebook.github.io/prophet/)
- [Statistical Hypothesis Testing](https://en.wikipedia.org/wiki/Statistical_hypothesis_testing)
- [Kubernetes Deployment Lifecycle](https://kubernetes.io/docs/concepts/workloads/controllers/deployment/)

