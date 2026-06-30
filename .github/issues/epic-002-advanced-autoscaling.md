# [EPIC] Advanced Autoscaling with Predictive Scaling and Custom Metrics

**Labels:** `epic`, `200-points`, `performance`, `autoscaling`

## Epic Overview

Implement intelligent autoscaling for Horizon and Soroban RPC nodes that goes beyond basic CPU/memory metrics. This system uses machine learning for predictive scaling, custom Stellar-specific metrics (TPS, ledger lag, contract invocations), and cost-aware scaling policies to optimize both performance and infrastructure costs.

## Business Value

- **Cost optimization**: Scale down during low-traffic periods (30-50% cost reduction)
- **Performance guarantee**: Proactive scaling before traffic spikes
- **Better user experience**: Maintain consistent API response times
- **Resource efficiency**: Right-size deployments based on actual workload

## Scope & Requirements

### Core Requirements

1. **Custom Metrics Autoscaling**
   - Scale based on Stellar-specific metrics:
     - Transactions per second (TPS)
     - Ledger ingestion lag
     - RPC request queue depth
     - Contract invocation rate (Soroban)
     - Database connection pool utilization
   - Support for composite metrics (e.g., TPS × avg_latency)

2. **Predictive Scaling**
   - Time-series analysis of historical traffic patterns
   - Predict traffic spikes 5-15 minutes in advance
   - Scale proactively before load increases
   - Learn from past scaling events

3. **Cost-Aware Scaling**
   - Prefer horizontal scaling over vertical when cost-effective
   - Support for spot instances with automatic fallback
   - Schedule-based scaling for known patterns (e.g., weekday peaks)
   - Budget constraints and cost alerts

4. **Multi-Dimensional Scaling**
   - Independent scaling for different node types
   - Separate read-replica pools for Horizon
   - Soroban RPC scaling based on contract complexity
   - Validator scaling for network participation

5. **Scaling Policies**
   - Aggressive: Fast scale-up, slow scale-down
   - Conservative: Slow scale-up, fast scale-down
   - Balanced: Moderate both directions
   - Custom: User-defined thresholds and cooldowns

6. **Integration with HPA/KEDA**
   - Extend Kubernetes HPA with custom metrics
   - KEDA ScaledObject support for event-driven scaling
   - Prometheus adapter for metrics exposure

## Technical Design

### Architecture

```
┌─────────────────────────────────────────────────────────┐
│              Autoscaling Decision Engine                 │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  │
│  │  Metrics     │  │  Predictor   │  │  Cost        │  │
│  │  Collector   │  │  (ML Model)  │  │  Optimizer   │  │
│  └──────────────┘  └──────────────┘  └──────────────┘  │
└─────────────────────────────────────────────────────────┘
         │                    │                    │
         ▼                    ▼                    ▼
┌─────────────────────────────────────────────────────────┐
│                  Scaling Executor                        │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  │
│  │     HPA      │  │    KEDA      │  │   Custom     │  │
│  │  Controller  │  │  ScaledObj   │  │   Scaler     │  │
│  └──────────────┘  └──────────────┘  └──────────────┘  │
└─────────────────────────────────────────────────────────┘
```

### New CRD: `StellarAutoscaler`

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarAutoscaler
metadata:
  name: horizon-autoscaler
spec:
  targetRef:
    apiVersion: stellar.org/v1alpha1
    kind: StellarNode
    name: my-horizon
  
  minReplicas: 2
  maxReplicas: 20
  
  scalingPolicy: balanced  # aggressive | conservative | balanced | custom
  
  metrics:
    - type: Custom
      custom:
        metricName: stellar_horizon_tps
        targetValue: "1000"
        aggregation: average
    
    - type: Custom
      custom:
        metricName: stellar_horizon_ledger_lag
        targetValue: "5"
        aggregation: max
    
    - type: Resource
      resource:
        name: cpu
        targetAverageUtilization: 70
  
  predictiveScaling:
    enabled: true
    lookbackWindow: 7d
    predictionHorizon: 15m
    confidenceThreshold: 0.8
  
  costOptimization:
    enabled: true
    maxHourlyCost: "50.00"
    preferSpotInstances: true
    spotFallbackEnabled: true
  
  schedules:
    - name: business-hours
      cron: "0 8 * * 1-5"  # 8 AM weekdays
      minReplicas: 5
    - name: off-hours
      cron: "0 20 * * *"   # 8 PM daily
      minReplicas: 2
  
  behavior:
    scaleUp:
      stabilizationWindowSeconds: 60
      policies:
        - type: Percent
          value: 50
          periodSeconds: 60
        - type: Pods
          value: 2
          periodSeconds: 60
    scaleDown:
      stabilizationWindowSeconds: 300
      policies:
        - type: Percent
          value: 10
          periodSeconds: 60
```

### Implementation Components

1. **Metrics Collector**
   - Query Prometheus for Stellar-specific metrics
   - Calculate composite metrics
   - Expose via Prometheus adapter for HPA consumption

2. **Predictive Model**
   - Time-series forecasting using Prophet or ARIMA
   - Store model state in ConfigMap/PVC
   - Retrain weekly on historical data
   - Fallback to reactive scaling if prediction confidence low

3. **Cost Optimizer**
   - Track current infrastructure costs via cloud provider APIs
   - Calculate cost per replica
   - Enforce budget constraints
   - Generate cost reports

4. **Scaling Executor**
   - Update HPA/KEDA resources based on decisions
   - Handle scaling conflicts (multiple autoscalers)
   - Implement safety limits (max scale rate)
   - Emit scaling events for audit trail

5. **Metrics Exposure**
   ```
   stellar_autoscaler_current_replicas{name, namespace}
   stellar_autoscaler_desired_replicas{name, namespace}
   stellar_autoscaler_predicted_load{name, namespace, horizon}
   stellar_autoscaler_scaling_events_total{name, namespace, direction}
   stellar_autoscaler_cost_per_hour{name, namespace}
   stellar_autoscaler_prediction_accuracy{name, namespace}
   ```

## Acceptance Criteria

- [ ] `StellarAutoscaler` CRD implemented with full validation
- [ ] Support for 5+ custom Stellar metrics
- [ ] Predictive scaling with >70% accuracy on test workloads
- [ ] Cost tracking and budget enforcement
- [ ] Schedule-based scaling working correctly
- [ ] Integration with Kubernetes HPA
- [ ] Integration with KEDA (optional)
- [ ] Grafana dashboard showing autoscaling decisions and predictions
- [ ] Documentation with example configurations
- [ ] E2E tests simulating traffic patterns
- [ ] Performance benchmarks showing cost savings
- [ ] Helm chart support for autoscaler deployment
- [ ] kubectl-stellar plugin commands for autoscaler management

## Dependencies & Blockers

- Requires Prometheus with Stellar metrics
- May need KEDA for advanced event-driven scaling
- Predictive model requires historical data (7+ days)
- Cloud provider API access for cost tracking

## Testing Strategy

### Unit Tests
- Metrics calculation logic
- Prediction algorithm accuracy
- Cost calculation correctness
- Policy evaluation logic

### Integration Tests
- HPA integration with custom metrics
- KEDA ScaledObject creation
- Prometheus adapter configuration
- Schedule-based scaling triggers

### E2E Tests
- Simulate traffic spike and verify proactive scaling
- Test scale-down during low traffic
- Verify cost limits are enforced
- Test spot instance fallback

### Load Tests
- Generate realistic traffic patterns
- Measure scaling latency (time to add replica)
- Verify no service degradation during scaling
- Test with 100+ replicas

### Chaos Tests
- Metrics endpoint failures
- Prediction model errors
- Cloud API unavailability
- Conflicting autoscaler policies

## Estimated Effort

**200 Story Points** (~5-7 weeks for 2 engineers)

## Related Issues

- #TBD: Prometheus metrics optimization
- #TBD: KEDA integration
- #TBD: Cost reporting dashboard
- #TBD: ML model training pipeline

## References

- [Kubernetes HPA](https://kubernetes.io/docs/tasks/run-application/horizontal-pod-autoscale/)
- [KEDA](https://keda.sh/)
- [Prometheus Adapter](https://github.com/kubernetes-sigs/prometheus-adapter)
- [Prophet Forecasting](https://facebook.github.io/prophet/)
- [AWS Predictive Scaling](https://docs.aws.amazon.com/autoscaling/ec2/userguide/ec2-auto-scaling-predictive-scaling.html)
