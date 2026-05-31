# SCP Analytics Pipeline

Real-time streaming of Stellar Consensus Protocol (SCP) messages to Kafka for network topology analysis and quorum health monitoring.

## Overview

The SCP Analytics Pipeline captures raw SCP messages from Stellar Core validators and streams them to Kafka topics for real-time analysis. This enables deep observability into consensus behavior, quorum health, and network topology at scale.

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                     Stellar Validator Pods                       │
│                                                                   │
│  ┌──────────────┐                    ┌──────────────┐           │
│  │ Stellar Core │                    │ SCP Sidecar  │           │
│  │              │◄───SCP Messages────│              │           │
│  │ :11626/scp   │                    │ (Streaming)  │           │
│  └──────────────┘                    └──────┬───────┘           │
│                                              │                   │
└──────────────────────────────────────────────┼───────────────────┘
                                               │
                                               │ Avro/Protobuf
                                               │ Serialization
                                               ▼
                                    ┌──────────────────┐
                                    │  Kafka Cluster   │
                                    │                  │
                                    │ Topic:           │
                                    │ scp-messages     │
                                    └────────┬─────────┘
                                             │
                    ┌────────────────────────┼────────────────────────┐
                    │                        │                        │
                    ▼                        ▼                        ▼
         ┌──────────────────┐    ┌──────────────────┐    ┌──────────────────┐
         │ Topology Health  │    │ Consensus        │    │ Custom           │
         │ Consumer         │    │ Latency Analyzer │    │ Analytics        │
         └──────────────────┘    └──────────────────┘    └──────────────────┘
```

## Features

- **High Throughput**: Handles thousands of SCP messages per second
- **Schema Support**: Avro and Protobuf serialization for efficient storage
- **Deduplication**: Prevents duplicate messages from being processed
- **Batching**: Configurable batch size and linger time for optimal throughput
- **Compression**: Supports Snappy, Gzip, LZ4, and Zstd compression
- **SASL Authentication**: Secure connection to Kafka clusters
- **Real-time Analysis**: Sample topology health consumer included
- **Metrics**: Prometheus metrics for pipeline monitoring

## Components

### 1. SCP Streaming Sidecar

A lightweight sidecar container that runs alongside Stellar Core validators and streams SCP messages to Kafka.

**Key Features:**
- Polls Stellar Core `/scp` endpoint at configurable intervals
- Serializes messages to Avro or Protobuf
- Deduplicates messages based on node ID and ballot counter
- Batches messages for efficient Kafka writes
- Handles connection failures with automatic retry

### 2. Kafka Topics

**Topic: `stellar-scp-messages`**
- Partitioned by validator node ID for ordering guarantees
- Configurable retention period
- Compressed for storage efficiency

### 3. Topology Health Consumer

A sample consumer that processes SCP messages and computes real-time topological health metrics.

**Metrics Computed:**
- **Health Score**: Overall network health (0.0 = unhealthy, 1.0 = healthy)
- **Active Validators**: Number of validators actively participating
- **Stalled Validators**: Validators with no phase change in 30+ seconds
- **Critical Nodes**: Validators whose removal would break consensus
- **Quorum Intersection**: Whether the network has quorum intersection
- **Consensus Latency**: Average time to reach consensus
- **Partition Detection**: Whether network partitions are detected

## Configuration

### Helm Values

```yaml
scpAnalytics:
  enabled: true

  kafka:
    bootstrapServers: "kafka:9092"
    topic: "stellar-scp-messages"
    format: "avro"  # or "protobuf", "json"
    compression: "snappy"
    batchSize: 1000000
    lingerMs: 100
    enableDeduplication: true
    pollIntervalSecs: 1

    # Schema registry for Avro
    schemaRegistryUrl: "http://schema-registry:8081"

    # SASL authentication (optional)
    sasl:
      enabled: true
      mechanism: "SCRAM-SHA-256"
      username: "stellar"
      password: "secret"

  sidecar:
    enabled: true
    resources:
      limits:
        cpu: 200m
        memory: 256Mi
      requests:
        cpu: 100m
        memory: 128Mi

  consumer:
    replicas: 1
    groupId: "topology-health-consumer"
    stallThresholdSecs: 30
    analysisWindowSecs: 60
    resources:
      limits:
        cpu: 500m
        memory: 512Mi
      requests:
        cpu: 200m
        memory: 256Mi
```

### StellarNode CRD

Enable SCP streaming for specific validators:

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: validator-with-scp-streaming
  namespace: stellar-system
spec:
  nodeType: Validator
  network: mainnet
  
  # Enable SCP analytics sidecar
  scpAnalytics:
    enabled: true
    kafkaConfig:
      bootstrapServers: "kafka:9092"
      topic: "stellar-scp-messages"
      format: "avro"
```

## Schemas

### Avro Schema

Located at `schemas/scp-message.avsc`:

```json
{
  "type": "record",
  "name": "ScpMessage",
  "namespace": "org.stellar.scp",
  "fields": [
    {"name": "message_id", "type": "string"},
    {"name": "timestamp", "type": {"type": "long", "logicalType": "timestamp-millis"}},
    {"name": "node_id", "type": "string"},
    {"name": "namespace", "type": "string"},
    {"name": "node_name", "type": "string"},
    {"name": "network", "type": "string"},
    {"name": "phase", "type": {"type": "enum", "name": "ScpPhase", "symbols": ["PREPARE", "CONFIRM", "EXTERNALIZE", "UNKNOWN"]}},
    {"name": "ballot_counter", "type": "int"},
    {"name": "value_hash", "type": "string"},
    {"name": "quorum_set", "type": "QuorumSet"},
    {"name": "nomination_votes", "type": {"type": "array", "items": "string"}},
    {"name": "nomination_accepted", "type": {"type": "array", "items": "string"}},
    {"name": "ledger_sequence", "type": ["null", "long"]},
    {"name": "metadata", "type": {"type": "map", "values": "string"}}
  ]
}
```

### Protobuf Schema

Located at `schemas/scp_message.proto`:

```protobuf
syntax = "proto3";

package stellar.scp;

message ScpMessage {
  string message_id = 1;
  int64 timestamp = 2;
  string node_id = 3;
  string namespace = 4;
  string node_name = 5;
  string network = 6;
  ScpPhase phase = 7;
  uint32 ballot_counter = 8;
  string value_hash = 9;
  QuorumSet quorum_set = 10;
  repeated string nomination_votes = 11;
  repeated string nomination_accepted = 12;
  optional uint64 ledger_sequence = 13;
  map<string, string> metadata = 14;
}

enum ScpPhase {
  UNKNOWN = 0;
  PREPARE = 1;
  CONFIRM = 2;
  EXTERNALIZE = 3;
}
```

## Deployment

### Prerequisites

1. **Kafka Cluster**: Running Kafka cluster (version 2.8+)
2. **Schema Registry** (optional): For Avro schema management
3. **Prometheus** (optional): For metrics collection

### Install with Helm

```bash
# Add Helm repository
helm repo add stellar-k8s https://stellar.github.io/stellar-k8s
helm repo update

# Install with SCP analytics enabled
helm install stellar-operator stellar-k8s/stellar-operator \
  --namespace stellar-system \
  --create-namespace \
  --set scpAnalytics.enabled=true \
  --set scpAnalytics.kafka.bootstrapServers="kafka:9092" \
  --set scpAnalytics.kafka.format="avro" \
  --set scpAnalytics.kafka.schemaRegistryUrl="http://schema-registry:8081"
```

### Deploy Kafka (Development)

For development/testing, deploy Kafka using Strimzi:

```bash
# Install Strimzi operator
kubectl create namespace kafka
kubectl create -f 'https://strimzi.io/install/latest?namespace=kafka' -n kafka

# Deploy Kafka cluster
kubectl apply -f - <<EOF
apiVersion: kafka.strimzi.io/v1beta2
kind: Kafka
metadata:
  name: stellar-kafka
  namespace: kafka
spec:
  kafka:
    version: 3.6.0
    replicas: 3
    listeners:
      - name: plain
        port: 9092
        type: internal
        tls: false
      - name: tls
        port: 9093
        type: internal
        tls: true
    config:
      offsets.topic.replication.factor: 3
      transaction.state.log.replication.factor: 3
      transaction.state.log.min.isr: 2
      default.replication.factor: 3
      min.insync.replicas: 2
    storage:
      type: jbod
      volumes:
      - id: 0
        type: persistent-claim
        size: 100Gi
        deleteClaim: false
  zookeeper:
    replicas: 3
    storage:
      type: persistent-claim
      size: 10Gi
      deleteClaim: false
  entityOperator:
    topicOperator: {}
    userOperator: {}
EOF

# Create SCP messages topic
kubectl apply -f - <<EOF
apiVersion: kafka.strimzi.io/v1beta2
kind: KafkaTopic
metadata:
  name: stellar-scp-messages
  namespace: kafka
  labels:
    strimzi.io/cluster: stellar-kafka
spec:
  partitions: 10
  replicas: 3
  config:
    retention.ms: 604800000  # 7 days
    compression.type: snappy
    segment.bytes: 1073741824
EOF
```

## Usage

### Consuming SCP Messages

#### Python Consumer Example

```python
from kafka import KafkaConsumer
import json

consumer = KafkaConsumer(
    'stellar-scp-messages',
    bootstrap_servers=['kafka:9092'],
    group_id='my-consumer-group',
    value_deserializer=lambda m: json.loads(m.decode('utf-8'))
)

for message in consumer:
    scp_msg = message.value
    print(f"Node: {scp_msg['node_id']}")
    print(f"Phase: {scp_msg['phase']}")
    print(f"Ballot: {scp_msg['ballot_counter']}")
    print(f"Quorum threshold: {scp_msg['quorum_set']['threshold']}")
    print("---")
```

#### Go Consumer Example

```go
package main

import (
    "context"
    "encoding/json"
    "fmt"
    "github.com/segmentio/kafka-go"
)

type ScpMessage struct {
    MessageID      string `json:"message_id"`
    NodeID         string `json:"node_id"`
    Phase          string `json:"phase"`
    BallotCounter  int    `json:"ballot_counter"`
}

func main() {
    reader := kafka.NewReader(kafka.ReaderConfig{
        Brokers: []string{"kafka:9092"},
        Topic:   "stellar-scp-messages",
        GroupID: "my-consumer-group",
    })
    defer reader.Close()

    for {
        msg, err := reader.ReadMessage(context.Background())
        if err != nil {
            panic(err)
        }

        var scpMsg ScpMessage
        json.Unmarshal(msg.Value, &scpMsg)
        
        fmt.Printf("Node: %s, Phase: %s, Ballot: %d\n",
            scpMsg.NodeID, scpMsg.Phase, scpMsg.BallotCounter)
    }
}
```

### Querying Topology Health

The topology health consumer exposes metrics via HTTP:

```bash
# Get latest health metrics
curl http://topology-health-consumer:9090/health

# Example response:
{
  "timestamp": "2024-01-15T10:30:00Z",
  "network": "mainnet",
  "health_score": 0.95,
  "active_validators": 25,
  "stalled_validators": 1,
  "critical_nodes": 3,
  "has_quorum_intersection": true,
  "avg_consensus_latency_ms": 2500,
  "partition_detected": false,
  "validator_health": [
    {
      "node_id": "GDTEST123...",
      "phase": "EXTERNALIZE",
      "time_since_last_change_secs": 5,
      "is_critical": true,
      "is_stalled": false,
      "peer_count": 10
    }
  ]
}
```

## Monitoring

### Prometheus Metrics

The topology health consumer exports the following metrics:

```
# Health score (0.0 = unhealthy, 1.0 = healthy)
stellar_topology_health_score{network="mainnet"} 0.95

# Active validators
stellar_topology_active_validators{network="mainnet"} 25

# Stalled validators
stellar_topology_stalled_validators{network="mainnet"} 1

# Critical nodes
stellar_topology_critical_nodes{network="mainnet"} 3

# Quorum intersection (1 = yes, 0 = no)
stellar_topology_quorum_intersection{network="mainnet"} 1

# Average consensus latency (milliseconds)
stellar_topology_consensus_latency_ms{network="mainnet"} 2500

# Partition detected (1 = yes, 0 = no)
stellar_topology_partition_detected{network="mainnet"} 0

# Kafka consumer lag
stellar_scp_consumer_lag{topic="stellar-scp-messages",partition="0"} 0
```

### Grafana Dashboard

Import the included Grafana dashboard for visualization:

```bash
kubectl apply -f dashboards/scp-analytics-dashboard.json
```

## Performance

### Throughput

- **Sidecar**: 1,000-5,000 messages/second per validator
- **Kafka**: 100,000+ messages/second (cluster-wide)
- **Consumer**: 10,000-50,000 messages/second

### Resource Usage

**Sidecar (per validator):**
- CPU: 50-100m
- Memory: 128-256Mi
- Network: 1-5 Mbps

**Consumer:**
- CPU: 200-500m
- Memory: 256-512Mi
- Network: 10-50 Mbps

### Latency

- **End-to-end**: 100-500ms (SCP event → Kafka → Consumer)
- **Sidecar polling**: 1 second (configurable)
- **Kafka write**: 10-50ms
- **Consumer processing**: 1-10ms per message

## Troubleshooting

### Sidecar Not Streaming

Check sidecar logs:
```bash
kubectl logs -n stellar-system validator-0 -c scp-sidecar
```

Common issues:
- Stellar Core not responding on port 11626
- Kafka connection refused
- SASL authentication failure

### Consumer Lag

Check consumer lag:
```bash
kubectl exec -n kafka kafka-0 -- \
  kafka-consumer-groups.sh \
  --bootstrap-server localhost:9092 \
  --describe \
  --group topology-health-consumer
```

Increase consumer replicas if lag is growing:
```bash
kubectl scale deployment topology-health-consumer --replicas=3
```

### Schema Registry Issues

Verify schema registration:
```bash
curl http://schema-registry:8081/subjects/stellar-scp-messages-value/versions/latest
```

## Best Practices

1. **Partitioning**: Use node ID as partition key for ordering guarantees
2. **Retention**: Set appropriate retention based on analysis needs (7-30 days)
3. **Compression**: Use Snappy for balance of speed and compression ratio
4. **Batching**: Tune batch size and linger time for your throughput requirements
5. **Monitoring**: Set up alerts for consumer lag and health score degradation
6. **Security**: Use SASL/SSL for production Kafka clusters
7. **Schema Evolution**: Use schema registry for backward compatibility

## Advanced Use Cases

### 1. Consensus Latency Analysis

Track time between SCP phases to identify slow validators:

```python
from collections import defaultdict
from datetime import datetime

phase_times = defaultdict(dict)

for message in consumer:
    node_id = message['node_id']
    phase = message['phase']
    timestamp = datetime.fromisoformat(message['timestamp'])
    
    phase_times[node_id][phase] = timestamp
    
    if 'PREPARE' in phase_times[node_id] and 'EXTERNALIZE' in phase_times[node_id]:
        latency = (phase_times[node_id]['EXTERNALIZE'] - 
                   phase_times[node_id]['PREPARE']).total_seconds()
        print(f"Node {node_id} consensus latency: {latency}s")
```

### 2. Quorum Topology Visualization

Build real-time network graph:

```python
import networkx as nx
import matplotlib.pyplot as plt

G = nx.DiGraph()

for message in consumer:
    node_id = message['node_id']
    quorum_set = message['quorum_set']
    
    G.add_node(node_id)
    for validator in quorum_set['validators']:
        G.add_edge(node_id, validator)

nx.draw(G, with_labels=True)
plt.show()
```

### 3. Anomaly Detection

Detect unusual SCP behavior:

```python
from sklearn.ensemble import IsolationForest

# Extract features
features = []
for message in consumer:
    features.append([
        message['ballot_counter'],
        len(message['nomination_votes']),
        len(message['nomination_accepted']),
        message['quorum_set']['threshold']
    ])

# Train anomaly detector
clf = IsolationForest(contamination=0.1)
clf.fit(features)

# Detect anomalies
predictions = clf.predict(features)
anomalies = [f for f, p in zip(features, predictions) if p == -1]
print(f"Detected {len(anomalies)} anomalies")
```

## Related Documentation

- [Quorum Analysis](./quorum-analysis.md)
- [Monitoring and Metrics](./monitoring.md)
- [Kafka Integration](./kafka-integration.md)
- [Schema Registry](./schema-registry.md)
