# Requirements: Distributed Tracing for Async Message Queues

## Overview
Enable end-to-end distributed tracing across asynchronous message queue boundaries by propagating W3C trace context through message envelope headers, supporting multiple transport protocols with zero-config integration.

## Goals
- Stitch async request paths into unified distributed traces
- Support major message queue transports (Kafka, NATS, webhooks)
- Detect and report broken trace chains
- Minimize overhead and maintain performance

## Non-Goals
- Replace existing tracing systems (OpenTelemetry, Jaeger)
- Implement custom trace storage or visualization
- Support legacy non-W3C tracing formats
- Trace message payload content (security concern)

## Functional Requirements

### FR1: Trace Context Propagation
- **FR1.1**: Inject W3C traceparent header into message envelopes
- **FR1.2**: Inject W3C tracestate header for vendor-specific data
- **FR1.3**: Extract trace context from incoming message headers
- **FR1.4**: Continue existing traces when context present
- **FR1.5**: Start new traces when context absent

### FR2: Transport Support
- **FR2.1**: Kafka producer/consumer trace context injection/extraction
- **FR2.2**: NATS publisher/subscriber trace context injection/extraction
- **FR2.3**: Webhook HTTP request header trace context injection/extraction
- **FR2.4**: Pluggable transport adapter interface for future protocols

### FR3: SDK Integration Shims
- **FR3.1**: Kafka wrapper preserving existing producer API
- **FR3.2**: Kafka wrapper preserving existing consumer API
- **FR3.3**: NATS wrapper preserving existing publisher API
- **FR3.4**: NATS wrapper preserving existing subscriber API
- **FR3.5**: Webhook client wrapper for outbound requests
- **FR3.6**: Webhook server middleware for inbound requests

### FR4: Broken Chain Detection
- **FR4.1**: Detect when consumer receives message without trace context
- **FR4.2**: Detect when trace context is malformed
- **FR4.3**: Emit metric for broken chain events with labels (queue, topic, reason)
- **FR4.4**: Log broken chain events with message metadata

### FR5: Zero-Config Operation
- **FR5.1**: Auto-detect trace context in environment (OpenTelemetry SDK)
- **FR5.2**: Use process-global tracer by default
- **FR5.3**: No configuration required for conforming producers/consumers
- **FR5.4**: Optional configuration for custom sampling rates

### FR6: Span Creation
- **FR6.1**: Create producer span on message send
- **FR6.2**: Create consumer span on message receive
- **FR6.3**: Link consumer span to producer span via trace context
- **FR6.4**: Record queue/topic name in span attributes
- **FR6.5**: Record message size in span attributes

## Non-Functional Requirements

### NFR1: Performance
- **NFR1.1**: Header overhead under 200 bytes per message
- **NFR1.2**: Zero measurable throughput regression (< 1%)
- **NFR1.3**: Latency overhead under 1ms (p99) for inject/extract
- **NFR1.4**: No additional network calls for trace propagation

### NFR2: Reliability
- **NFR2.1**: Trace propagation failures never block message delivery
- **NFR2.2**: Graceful degradation if tracing backend unavailable
- **NFR2.3**: No message loss due to tracing instrumentation
- **NFR2.4**: Thread-safe for concurrent producer/consumer usage

### NFR3: Compatibility
- **NFR3.1**: Compatible with OpenTelemetry SDK v1.x
- **NFR3.2**: Compatible with Kafka clients 3.x+
- **NFR3.3**: Compatible with NATS clients 2.x+
- **NFR3.4**: Compatible with W3C Trace Context specification

### NFR4: Observability
- **NFR4.1**: Metrics for trace context injection rate
- **NFR4.2**: Metrics for trace context extraction rate
- **NFR4.3**: Metrics for broken chain events by transport
- **NFR4.4**: Metrics for span creation latency

## Acceptance Criteria

### AC1: Trace Stitch Rate
- End-to-end trace stitch rate above 95% for async paths
- Measured by checking trace continuity from HTTP ingress → queue → consumer → downstream
- Test with 10,000+ messages across all supported transports

### AC2: Broken Chain Reporting
- Broken chain events exported with queue + topic labels
- 100% detection of missing or malformed trace context
- Alerts configured for broken chain rate threshold

### AC3: Overhead
- Added header overhead measured at < 200 bytes
- Measured for typical trace IDs and tracestate values
- Includes both traceparent and tracestate headers

### AC4: Performance
- No measurable throughput regression (< 1% degradation)
- Benchmark comparing raw client vs. wrapped client
- Tested at 10,000 messages/second sustained load
- P99 latency increase under 1ms

## User Stories

### US1: Backend Developer
As a backend developer, I want my async message processing to automatically appear in distributed traces so that I can debug request flows without manual instrumentation.

**Acceptance**: Replace Kafka client import with traced wrapper, traces appear in Jaeger automatically.

### US2: SRE Investigating Latency
As an SRE investigating latency, I want to see the full request path including queue wait time so that I can identify bottlenecks in async processing.

**Acceptance**: Trace spans show producer → queue → consumer with timing for each hop.

### US3: Platform Engineer
As a platform engineer, I want to know when traces break across queue boundaries so that I can identify misconfigured services.

**Acceptance**: Dashboard shows broken chain rate per queue/topic with alerts on anomalies.

### US4: Service Owner
As a service owner, I want zero-config tracing integration so that I don't have to modify my message processing code.

**Acceptance**: Drop-in replacement for standard Kafka/NATS clients with no code changes.

## Dependencies
- OpenTelemetry SDK (Rust/Go/Java)
- W3C Trace Context specification
- Kafka client libraries (kafka-rs, confluent-kafka)
- NATS client libraries (nats.rs, nats.go)
- HTTP client/server frameworks
- Metrics backend (Prometheus)

## Open Questions
1. Should we support Baggage propagation in addition to trace context?
2. What is the sampling strategy for high-volume queues?
3. How do we handle trace context in compressed message batches?
4. Should we provide middleware for other frameworks (Actix, Axum, Gin)?
5. What is the migration path for services using legacy tracing formats?
