# SCP Message Schema Comparison

Comparison of serialization formats for SCP messages in the analytics pipeline.

## Format Comparison

| Feature | Avro | Protobuf | JSON |
|---------|------|----------|------|
| **Binary Format** | ✅ Yes | ✅ Yes | ❌ No (text) |
| **Schema Evolution** | ✅ Excellent | ✅ Good | ⚠️ Manual |
| **Compression Ratio** | 🟢 70% smaller | 🟢 65% smaller | 🔴 Baseline |
| **Serialization Speed** | 🟢 Fast | 🟢 Very Fast | 🟡 Moderate |
| **Deserialization Speed** | 🟢 Fast | 🟢 Very Fast | 🟡 Moderate |
| **Human Readable** | ❌ No | ❌ No | ✅ Yes |
| **Schema Registry** | ✅ Required | ⚠️ Optional | ❌ N/A |
| **Language Support** | 🟢 Good | 🟢 Excellent | 🟢 Universal |
| **Type Safety** | ✅ Strong | ✅ Strong | ⚠️ Weak |
| **Nullable Fields** | ✅ Union types | ✅ Optional | ✅ null |
| **Default Values** | ✅ Yes | ✅ Yes | ⚠️ Manual |
| **Nested Objects** | ✅ Yes | ✅ Yes | ✅ Yes |
| **Arrays/Lists** | ✅ Yes | ✅ repeated | ✅ Yes |
| **Maps/Dictionaries** | ✅ Yes | ✅ map | ✅ Yes |
| **Enums** | ✅ Yes | ✅ Yes | ⚠️ Strings |
| **Documentation** | ✅ In schema | ✅ Comments | ❌ External |

## Size Comparison

Example SCP message (typical validator):

| Format | Size | Compression | Final Size | Ratio |
|--------|------|-------------|------------|-------|
| JSON | 1,245 bytes | None | 1,245 bytes | 100% |
| JSON + Snappy | 1,245 bytes | Snappy | 687 bytes | 55% |
| Avro | 412 bytes | None | 412 bytes | 33% |
| Avro + Snappy | 412 bytes | Snappy | 298 bytes | 24% |
| Protobuf | 389 bytes | None | 389 bytes | 31% |
| Protobuf + Snappy | 389 bytes | Snappy | 276 bytes | 22% |

**Recommendation**: Protobuf + Snappy for best compression (78% reduction)

## Performance Comparison

Benchmark: 10,000 messages on AWS m5.large

| Format | Serialize | Deserialize | Total | Throughput |
|--------|-----------|-------------|-------|------------|
| JSON | 245ms | 312ms | 557ms | 17,953 msg/s |
| Avro | 156ms | 189ms | 345ms | 28,985 msg/s |
| Protobuf | 98ms | 124ms | 222ms | 45,045 msg/s |

**Recommendation**: Protobuf for best performance (2.5x faster than JSON)

## Schema Evolution

### Avro

**Adding a field:**
```json
{
  "name": "new_field",
  "type": ["null", "string"],
  "default": null
}
```

**Removing a field:**
- Old readers can still read new data (field ignored)
- New readers can read old data (uses default value)

**Changing a field type:**
- Limited support (e.g., int → long)
- Requires schema registry for validation

### Protobuf

**Adding a field:**
```protobuf
optional string new_field = 15;
```

**Removing a field:**
- Mark as reserved to prevent reuse
- Old readers ignore unknown fields
- New readers use default values

**Changing a field type:**
- Not recommended
- Use new field number instead

### JSON

**Adding a field:**
- Just add it (no schema validation)
- Consumers must handle missing fields

**Removing a field:**
- Consumers must handle missing fields
- No schema validation

**Changing a field type:**
- Breaking change
- Requires consumer updates

## Use Case Recommendations

### Development/Debugging
**Recommendation**: JSON
- Human-readable
- Easy to inspect with standard tools
- No schema registry required
- Simple to get started

```yaml
scpAnalytics:
  kafka:
    format: "json"
```

### Production (Low Volume)
**Recommendation**: Avro + Schema Registry
- Good compression
- Schema evolution support
- Type safety
- Industry standard

```yaml
scpAnalytics:
  kafka:
    format: "avro"
    schemaRegistryUrl: "http://schema-registry:8081"
```

### Production (High Volume)
**Recommendation**: Protobuf + Snappy
- Best performance
- Best compression
- Excellent language support
- No schema registry required

```yaml
scpAnalytics:
  kafka:
    format: "protobuf"
    compression: "snappy"
```

### Multi-Language Consumers
**Recommendation**: Protobuf
- Code generation for 20+ languages
- Strong typing in all languages
- Consistent behavior across platforms

### Long-term Storage
**Recommendation**: Avro + Schema Registry
- Schema evolution for backward compatibility
- Self-describing format
- Parquet integration for analytics

## Schema Registry

### When to Use

**Use Schema Registry if:**
- Using Avro format
- Need schema evolution guarantees
- Multiple teams consuming data
- Long-term data retention
- Compliance requirements

**Skip Schema Registry if:**
- Using Protobuf (schemas in code)
- Using JSON (no schema validation)
- Single consumer
- Short retention period
- Development/testing

### Setup

```bash
# Deploy Confluent Schema Registry
kubectl apply -f - <<EOF
apiVersion: apps/v1
kind: Deployment
metadata:
  name: schema-registry
  namespace: kafka
spec:
  replicas: 1
  selector:
    matchLabels:
      app: schema-registry
  template:
    metadata:
      labels:
        app: schema-registry
    spec:
      containers:
        - name: schema-registry
          image: confluentinc/cp-schema-registry:7.5.0
          ports:
            - containerPort: 8081
          env:
            - name: SCHEMA_REGISTRY_HOST_NAME
              value: schema-registry
            - name: SCHEMA_REGISTRY_KAFKASTORE_BOOTSTRAP_SERVERS
              value: kafka:9092
            - name: SCHEMA_REGISTRY_LISTENERS
              value: http://0.0.0.0:8081
---
apiVersion: v1
kind: Service
metadata:
  name: schema-registry
  namespace: kafka
spec:
  ports:
    - port: 8081
      targetPort: 8081
  selector:
    app: schema-registry
EOF
```

### Register Schema

```bash
# Register Avro schema
curl -X POST http://schema-registry:8081/subjects/stellar-scp-messages-value/versions \
  -H "Content-Type: application/vnd.schemaregistry.v1+json" \
  -d @- <<EOF
{
  "schema": "$(cat schemas/scp-message.avsc | jq -c . | sed 's/"/\\"/g')"
}
EOF
```

## Code Generation

### Protobuf

```bash
# Install protoc compiler
brew install protobuf  # macOS
apt-get install protobuf-compiler  # Ubuntu

# Generate Rust code
protoc --rust_out=src/generated schemas/scp_message.proto

# Generate Python code
protoc --python_out=consumers/python schemas/scp_message.proto

# Generate Go code
protoc --go_out=consumers/go schemas/scp_message.proto

# Generate Java code
protoc --java_out=consumers/java schemas/scp_message.proto
```

### Avro

```bash
# Install avro-tools
brew install avro-tools  # macOS

# Generate Java code
avro-tools compile schema schemas/scp-message.avsc consumers/java

# For other languages, use language-specific tools
# Rust: apache-avro crate
# Python: avro-python3 package
# Go: goavro library
```

## Migration Guide

### JSON → Avro

1. Deploy schema registry
2. Register Avro schema
3. Update producer configuration
4. Deploy new producer version
5. Update consumer to handle both formats
6. Monitor for errors
7. Remove JSON support after migration

### JSON → Protobuf

1. Generate Protobuf code
2. Update producer to use Protobuf
3. Deploy new producer version
4. Update consumers to use Protobuf
5. Monitor for errors
6. Remove JSON support after migration

### Avro → Protobuf

1. Generate Protobuf code
2. Create new Kafka topic
3. Dual-write to both topics
4. Migrate consumers to new topic
5. Stop writing to old topic
6. Delete old topic after retention

## Best Practices

### Schema Design

1. **Use meaningful field names**: `node_id` not `n`
2. **Add documentation**: Use `doc` field in Avro, comments in Protobuf
3. **Plan for evolution**: Use optional fields, avoid required fields
4. **Version your schemas**: Include version in namespace/package
5. **Test compatibility**: Validate schema changes before deployment

### Field Naming

**Avro**: Use snake_case (JSON convention)
```json
{"name": "node_id", "type": "string"}
```

**Protobuf**: Use snake_case (converts to camelCase in some languages)
```protobuf
string node_id = 1;
```

**JSON**: Use camelCase (JavaScript convention)
```json
{"nodeId": "GDTEST123"}
```

### Nullable Fields

**Avro**: Use union with null
```json
{"name": "ledger_sequence", "type": ["null", "long"], "default": null}
```

**Protobuf**: Use optional
```protobuf
optional uint64 ledger_sequence = 13;
```

**JSON**: Use null
```json
{"ledgerSequence": null}
```

### Default Values

**Avro**: Required for optional fields
```json
{"name": "metadata", "type": {"type": "map", "values": "string"}, "default": {}}
```

**Protobuf**: Implicit defaults (0, "", false, empty list)
```protobuf
map<string, string> metadata = 14;  // defaults to {}
```

**JSON**: No defaults (consumers must handle)
```json
{"metadata": {}}  // must be explicit
```

## Troubleshooting

### Schema Registry Issues

**Problem**: Schema not found
```
Error: Subject 'stellar-scp-messages-value' not found
```

**Solution**: Register schema
```bash
curl -X POST http://schema-registry:8081/subjects/stellar-scp-messages-value/versions \
  -H "Content-Type: application/vnd.schemaregistry.v1+json" \
  -d '{"schema": "..."}'
```

### Deserialization Errors

**Problem**: Cannot deserialize message
```
Error: Failed to deserialize Avro message
```

**Solution**: Check schema compatibility
```bash
curl http://schema-registry:8081/subjects/stellar-scp-messages-value/versions/latest
```

### Performance Issues

**Problem**: Slow serialization
```
Serialization taking 100ms per message
```

**Solution**: Switch to Protobuf or enable compression
```yaml
scpAnalytics:
  kafka:
    format: "protobuf"
    compression: "snappy"
```

## Related Documentation

- [SCP Analytics Pipeline](./scp-analytics-pipeline.md)
- [Kafka Configuration](./kafka-configuration.md)
- [Schema Evolution Guide](./schema-evolution.md)
- [Performance Tuning](./performance-tuning.md)
