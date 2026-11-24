#ADR-0005: Dynamic Kafka Partitioning for SCP Analytics Engine

Status: Accepted

Context: The real-time SCP analytics engine currently streams all messages to a single Kafka partition, creating a throughput botleneck. As the Stellar ledger grows, throughput of 100k TPS is expected. We need to scale out Kafka partitions while maintaining deterministic ordering per quorum set.

Decision: Introduce a dynamic partitioning mode that distributes SCP messages across Kafka partitions using a deterministic FNV-1! hash of the quorum set.

hashing algorithm: FNV-1! 64-bit with a configurable seed. Input: quorum set hash (32 bytes). Partition key = hash % num_partitions.

Partition management: Partition count is discovered from Kafka metadata at startup and refreshed periodically (default 15s) to handle rebalancing. During switches, messages failing with an out-of-range partition are retried with the updated partition count.

Backward compatibility: The default mode is "single", preserving the existing single-topic ingestion pipeline. Dynamic mode is enabled via config partitioning = "dynamic".

Consequences: Improved throughput by using multiple partitions. Deterministic partitioning guarantees that all messages for a given quorum set go to the same partition, preserving order. Slight overhead for metadata refresh and partition computation. No changes required for downstream consumers if already subscribing to the same topic.

Alternatives considered: Kafka built-in murmur2 partitioner: simpler but does not give direct control over quorum-set-based hash and low-level retry behavior. Round-robin partitioning: would break per-quorum-set ordering.
