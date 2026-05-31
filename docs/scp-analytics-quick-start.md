# SCP Analytics Quick Start

Get started with real-time SCP message streaming in 5 minutes.

## Prerequisites

- Kubernetes cluster (1.28+)
- Helm 3.x
- Kafka cluster (or use provided development setup)

## Quick Setup

### 1. Deploy Kafka (Development)

```bash
# Install Strimzi operator
kubectl create namespace kafka
kubectl apply -f 'https://strimzi.io/install/latest?namespace=kafka' -n kafka

# Wait for operator
kubectl wait --for=condition=ready pod -l name=strimzi-cluster-operator -n kafka --timeout=300s

# Deploy Kafka cluster
cat <<EOF | kubectl apply -f -
apiVersion: kafka.strimzi.io/v1beta2
kind: Kafka
metadata:
  name: stellar-kafka
  namespace: kafka
spec:
  kafka:
    version: 3.6.0
    replicas: 1
    listeners:
      - name: plain
        port: 9092
        type: internal
        tls: false
    storage:
      type: ephemeral
  zookeeper:
    replicas: 1
    storage:
      type: ephemeral
EOF

# Wait for Kafka
kubectl wait kafka/stellar-kafka --for=condition=Ready --timeout=300s -n kafka

# Create topic
cat <<EOF | kubectl apply -f -
apiVersion: kafka.strimzi.io/v1beta2
kind: KafkaTopic
metadata:
  name: stellar-scp-messages
  namespace: kafka
  labels:
    strimzi.io/cluster: stellar-kafka
spec:
  partitions: 3
  replicas: 1
  config:
    retention.ms: 86400000  # 1 day
    compression.type: snappy
EOF
```

### 2. Install Stellar Operator with SCP Analytics

```bash
helm install stellar-operator stellar-k8s/stellar-operator \
  --namespace stellar-system \
  --create-namespace \
  --set scpAnalytics.enabled=true \
  --set scpAnalytics.kafka.bootstrapServers="stellar-kafka-kafka-bootstrap.kafka:9092"
```

### 3. Deploy a Validator with SCP Streaming

```bash
cat <<EOF | kubectl apply -f -
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: validator-scp-demo
  namespace: stellar-system
spec:
  nodeType: Validator
  network: testnet
  version: "21.0.0"
  
  # Enable SCP streaming
  scpAnalytics:
    enabled: true
    kafkaConfig:
      bootstrapServers: "stellar-kafka-kafka-bootstrap.kafka:9092"
      topic: "stellar-scp-messages"
      format: "json"  # Use JSON for easy debugging
      pollIntervalSecs: 1
  
  quorumSet:
    threshold: 2
    validators:
      - name: sdf1
        publicKey: "GCGB2S2KGYARPVIA37HYZXVRM2YZUEXA6S33ZU5BUDC6THSB62LZSTYH"
      - name: sdf2
        publicKey: "GCM6QMP3DLRPTAZW2UZPCPX2LF3SXWXKPMP3GKFZBDSF3QZGV2G5QSTK"
EOF
```

### 4. Verify Messages are Flowing

```bash
# Check sidecar logs
kubectl logs -n stellar-system validator-scp-demo-0 -c scp-sidecar

# Consume messages from Kafka
kubectl run kafka-consumer -ti --image=quay.io/strimzi/kafka:latest-kafka-3.6.0 --rm=true --restart=Never -- \
  bin/kafka-console-consumer.sh \
  --bootstrap-server stellar-kafka-kafka-bootstrap.kafka:9092 \
  --topic stellar-scp-messages \
  --from-beginning
```

### 5. View Topology Health

```bash
# Port-forward to topology health consumer
kubectl port-forward -n stellar-system svc/stellar-operator-topology-health-consumer 9090:9090

# Get health metrics
curl http://localhost:9090/health | jq
```

## Example Output

### SCP Message

```json
{
  "message_id": "GDTEST123-42-1705320000000",
  "timestamp": "2024-01-15T10:00:00Z",
  "node_id": "GDTEST123...",
  "namespace": "stellar-system",
  "node_name": "validator-scp-demo",
  "network": "testnet",
  "phase": "EXTERNALIZE",
  "ballot_counter": 42,
  "value_hash": "abc123...",
  "quorum_set": {
    "threshold": 2,
    "validators": ["VAL1", "VAL2"],
    "inner_sets": []
  },
  "nomination_votes": ["vote1"],
  "nomination_accepted": ["accepted1"],
  "ledger_sequence": 12345,
  "metadata": {}
}
```

### Topology Health

```json
{
  "timestamp": "2024-01-15T10:00:00Z",
  "network": "testnet",
  "health_score": 1.0,
  "active_validators": 1,
  "stalled_validators": 0,
  "critical_nodes": 0,
  "has_quorum_intersection": true,
  "avg_consensus_latency_ms": 1500,
  "partition_detected": false,
  "validator_health": [
    {
      "node_id": "GDTEST123...",
      "phase": "EXTERNALIZE",
      "time_since_last_change_secs": 2,
      "is_critical": false,
      "is_stalled": false,
      "peer_count": 2
    }
  ]
}
```

## Common Commands

### Check Kafka Topic

```bash
# List topics
kubectl exec -n kafka stellar-kafka-kafka-0 -- \
  bin/kafka-topics.sh --bootstrap-server localhost:9092 --list

# Describe topic
kubectl exec -n kafka stellar-kafka-kafka-0 -- \
  bin/kafka-topics.sh --bootstrap-server localhost:9092 \
  --describe --topic stellar-scp-messages
```

### Monitor Consumer Lag

```bash
kubectl exec -n kafka stellar-kafka-kafka-0 -- \
  bin/kafka-consumer-groups.sh --bootstrap-server localhost:9092 \
  --describe --group topology-health-consumer
```

### View Sidecar Logs

```bash
# Follow logs
kubectl logs -n stellar-system validator-scp-demo-0 -c scp-sidecar -f

# Last 100 lines
kubectl logs -n stellar-system validator-scp-demo-0 -c scp-sidecar --tail=100
```

### Scale Consumer

```bash
# Scale up for higher throughput
kubectl scale deployment -n stellar-system \
  stellar-operator-topology-health-consumer --replicas=3
```

## Troubleshooting

### Sidecar Not Starting

```bash
# Check pod events
kubectl describe pod -n stellar-system validator-scp-demo-0

# Check sidecar logs
kubectl logs -n stellar-system validator-scp-demo-0 -c scp-sidecar
```

### No Messages in Kafka

```bash
# Verify Kafka is running
kubectl get kafka -n kafka

# Check topic exists
kubectl get kafkatopic -n kafka

# Test Kafka connectivity from sidecar
kubectl exec -n stellar-system validator-scp-demo-0 -c scp-sidecar -- \
  nc -zv stellar-kafka-kafka-bootstrap.kafka 9092
```

### Consumer Not Processing

```bash
# Check consumer logs
kubectl logs -n stellar-system -l app.kubernetes.io/component=topology-health-consumer

# Verify consumer group
kubectl exec -n kafka stellar-kafka-kafka-0 -- \
  bin/kafka-consumer-groups.sh --bootstrap-server localhost:9092 --list
```

## Next Steps

1. **Production Setup**: Configure SASL/SSL for secure Kafka access
2. **Schema Registry**: Set up Confluent Schema Registry for Avro
3. **Custom Consumers**: Build your own analytics consumers
4. **Monitoring**: Set up Prometheus alerts for health metrics
5. **Visualization**: Create Grafana dashboards for topology

## Clean Up

```bash
# Delete validator
kubectl delete stellarnode validator-scp-demo -n stellar-system

# Delete operator
helm uninstall stellar-operator -n stellar-system

# Delete Kafka
kubectl delete kafka stellar-kafka -n kafka
kubectl delete namespace kafka
```

## Related Documentation

- [Full SCP Analytics Documentation](./scp-analytics-pipeline.md)
- [Kafka Configuration](./kafka-configuration.md)
- [Schema Reference](./scp-schemas.md)
- [Consumer Development](./consumer-development.md)
