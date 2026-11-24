# Design: Distributed Tracing for Async Message Queues

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                      Application Services                        │
├─────────────┬───────────────┬───────────────┬──────────────────┤
│   Service A │   Service B   │   Service C   │    Service D     │
│  (Producer) │  (Consumer)   │ (Pub/Sub)     │  (Webhook)       │
└──────┬──────┴───────┬───────┴───────┬───────┴────────┬─────────┘
       │              │               │                │
┌──────▼──────┐  ┌───▼────┐  ┌───────▼────┐  ┌────────▼────────┐
│Traced Kafka │  │Traced  │  │Traced NATS │  │Traced Webhook   │
│  Producer   │  │Kafka   │  │ Pub/Sub    │  │    Client       │
│    Shim     │  │Consumer│  │    Shim    │  │     Shim        │
└──────┬──────┘  └───┬────┘  └───────┬────┘  └────────┬────────┘
       │             │               │                 │
       │ Inject      │ Extract       │ Inject/Extract  │ Inject
       │ Context     │ Context       │ Context         │ Context
       │             │               │                 │
┌──────▼─────────────▼───────────────▼─────────────────▼─────────┐
│              W3C Trace Context Propagator                       │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │  traceparent: 00-{trace-id}-{span-id}-{flags}            │  │
│  │  tracestate: vendor1=value1,vendor2=value2               │  │
│  └──────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
                             │
                    ┌────────▼──────────┐
                    │  Message Headers  │
                    └────────┬──────────┘
                             │
        ┌────────────────────┼────────────────────┐
        │                    │                    │
┌───────▼────────┐  ┌────────▼────────┐  ┌───────▼────────┐
│  Kafka Broker  │  │  NATS Server    │  │  HTTP Server   │
└───────┬────────┘  └────────┬────────┘  └───────┬────────┘
        │                    │                    │
        └────────────────────┴────────────────────┘
                             │
                    ┌────────▼──────────┐
                    │ OpenTelemetry SDK │
                    │   (Global Tracer) │
                    └────────┬──────────┘
                             │
                    ┌────────▼──────────┐
                    │  Jaeger / Tempo   │
                    │  (Trace Backend)  │
                    └───────────────────┘
```

## Component Design

### 1. W3C Trace Context Propagator

**Responsibility**: Serialize and deserialize W3C trace context to/from message headers.

**Interface**:
```rust
pub trait TraceContextPropagator {
    /// Inject current trace context into message headers
    fn inject(&self, context: &SpanContext, headers: &mut Headers) -> Result<()>;
    
    /// Extract trace context from message headers
    fn extract(&self, headers: &Headers) -> Result<Option<SpanContext>>;
}

pub struct W3CTracePropagator {
    // Stateless, thread-safe
}

impl TraceContextPropagator for W3CTracePropagator {
    fn inject(&self, context: &SpanContext, headers: &mut Headers) -> Result<()> {
        // Format: 00-{trace-id}-{span-id}-{flags}
        let traceparent = format!(
            "00-{:032x}-{:016x}-{:02x}",
            context.trace_id(),
            context.span_id(),
            context.trace_flags()
        );
        headers.insert("traceparent", traceparent);
        
        // Optional tracestate
        if let Some(state) = context.trace_state() {
            headers.insert("tracestate", state.to_string());
        }
        
        Ok(())
    }
    
    fn extract(&self, headers: &Headers) -> Result<Option<SpanContext>> {
        let traceparent = match headers.get("traceparent") {
            Some(v) => v,
            None => return Ok(None),
        };
        
        // Parse: 00-{trace-id}-{span-id}-{flags}
        let parts: Vec<&str> = traceparent.split('-').collect();
        if parts.len() != 4 || parts[0] != "00" {
            return Err(Error::MalformedTraceContext);
        }
        
        let trace_id = u128::from_str_radix(parts[1], 16)?;
        let span_id = u64::from_str_radix(parts[2], 16)?;
        let flags = u8::from_str_radix(parts[3], 16)?;
        
        let trace_state = headers.get("tracestate")
            .map(|s| TraceState::from_str(s))
            .transpose()?;
        
        Ok(Some(SpanContext::new(trace_id, span_id, flags, trace_state)))
    }
}
```

### 2. Kafka Traced Producer

**Responsibility**: Wrap Kafka producer to inject trace context into message headers.

**Interface**:
```rust
pub struct TracedKafkaProducer<P: Producer> {
    inner: P,
    propagator: Arc<W3CTracePropagator>,
    tracer: Arc<dyn Tracer>,
    metrics: ProducerMetrics,
}

impl<P: Producer> TracedKafkaProducer<P> {
    pub fn new(producer: P, tracer: Arc<dyn Tracer>) -> Self {
        Self {
            inner: producer,
            propagator: Arc::new(W3CTracePropagator::new()),
            tracer,
            metrics: ProducerMetrics::new(),
        }
    }
    
    pub async fn send(
        &self,
        topic: &str,
        key: Option<&[u8]>,
        payload: &[u8],
    ) -> Result<RecordMetadata> {
        // Create producer span
        let mut span = self.tracer
            .span_builder("kafka.send")
            .with_kind(SpanKind::Producer)
            .with_attribute("messaging.system", "kafka")
            .with_attribute("messaging.destination", topic)
            .with_attribute("messaging.message_payload_size_bytes", payload.len() as i64)
            .start(&*self.tracer);
        
        // Inject trace context into headers
        let mut headers = Headers::new();
        if let Err(e) = self.propagator.inject(&span.context(), &mut headers) {
            self.metrics.injection_errors.inc();
            tracing::warn!("Failed to inject trace context: {}", e);
        } else {
            self.metrics.injections.inc();
        }
        
        // Send message with traced headers
        let result = self.inner.send(
            topic,
            key,
            payload,
            Some(headers),
        ).await;
        
        // Record span result
        match &result {
            Ok(metadata) => {
                span.set_attribute("messaging.kafka.partition", metadata.partition);
                span.set_attribute("messaging.kafka.offset", metadata.offset);
                span.set_status(StatusCode::Ok);
            }
            Err(e) => {
                span.set_status(StatusCode::Error, e.to_string());
            }
        }
        
        span.end();
        result
    }
}
```

**Drop-in Replacement Pattern**:
```rust
// Before:
use kafka::producer::Producer;
let producer = Producer::new(config)?;

// After:
use traced_messaging::kafka::TracedKafkaProducer;
let producer = TracedKafkaProducer::new(
    kafka::producer::Producer::new(config)?,
    global::tracer("my-service"),
);
// Same API, now with tracing!
```

### 3. Kafka Traced Consumer

**Responsibility**: Wrap Kafka consumer to extract trace context and create consumer spans.

**Interface**:
```rust
pub struct TracedKafkaConsumer<C: Consumer> {
    inner: C,
    propagator: Arc<W3CTracePropagator>,
    tracer: Arc<dyn Tracer>,
    metrics: ConsumerMetrics,
}

impl<C: Consumer> TracedKafkaConsumer<C> {
    pub async fn poll(&self, timeout: Duration) -> Result<Option<Message>> {
        let message = self.inner.poll(timeout).await?;
        
        let message = match message {
            Some(msg) => msg,
            None => return Ok(None),
        };
        
        // Extract trace context from headers
        let parent_context = match self.propagator.extract(&message.headers) {
            Ok(Some(ctx)) => {
                self.metrics.extractions.inc();
                Some(ctx)
            }
            Ok(None) => {
                self.metrics.broken_chains
                    .with_label_values(&[message.topic, "missing_context"])
                    .inc();
                tracing::warn!(
                    topic = message.topic,
                    partition = message.partition,
                    offset = message.offset,
                    "Received message without trace context"
                );
                None
            }
            Err(e) => {
                self.metrics.broken_chains
                    .with_label_values(&[message.topic, "malformed_context"])
                    .inc();
                tracing::warn!(
                    topic = message.topic,
                    error = %e,
                    "Failed to extract trace context"
                );
                None
            }
        };
        
        // Create consumer span linked to producer span
        let mut span_builder = self.tracer
            .span_builder("kafka.receive")
            .with_kind(SpanKind::Consumer)
            .with_attribute("messaging.system", "kafka")
            .with_attribute("messaging.destination", message.topic)
            .with_attribute("messaging.kafka.partition", message.partition)
            .with_attribute("messaging.kafka.offset", message.offset)
            .with_attribute("messaging.message_payload_size_bytes", message.payload.len() as i64);
        
        if let Some(parent) = parent_context {
            span_builder = span_builder.with_parent_context(parent);
        }
        
        let span = span_builder.start(&*self.tracer);
        
        // Attach span to message for downstream processing
        Ok(Some(TracedMessage {
            inner: message,
            span,
        }))
    }
}

pub struct TracedMessage {
    inner: Message,
    span: Span,
}

impl TracedMessage {
    /// Access the underlying message
    pub fn message(&self) -> &Message {
        &self.inner
    }
    
    /// Get the consumer span to use as parent for downstream operations
    pub fn span(&self) -> &Span {
        &self.span
    }
    
    /// Mark processing as complete
    pub fn complete(mut self) {
        self.span.set_status(StatusCode::Ok);
        self.span.end();
    }
    
    /// Mark processing as failed
    pub fn fail(mut self, error: &dyn std::error::Error) {
        self.span.set_status(StatusCode::Error, error.to_string());
        self.span.end();
    }
}
```

### 4. NATS Traced Publisher/Subscriber

**Similar pattern to Kafka**:
```rust
pub struct TracedNatsClient {
    client: nats::Client,
    propagator: Arc<W3CTracePropagator>,
    tracer: Arc<dyn Tracer>,
    metrics: NatsMetrics,
}

impl TracedNatsClient {
    pub async fn publish(
        &self,
        subject: &str,
        payload: &[u8],
    ) -> Result<()> {
        let mut span = self.tracer
            .span_builder("nats.publish")
            .with_kind(SpanKind::Producer)
            .with_attribute("messaging.system", "nats")
            .with_attribute("messaging.destination", subject)
            .start(&*self.tracer);
        
        let mut headers = nats::Headers::new();
        self.propagator.inject(&span.context(), &mut headers)?;
        
        self.client.publish_with_headers(subject, headers, payload).await?;
        span.end();
        Ok(())
    }
    
    pub async fn subscribe(&self, subject: &str) -> Result<TracedSubscription> {
        let subscription = self.client.subscribe(subject).await?;
        Ok(TracedSubscription {
            inner: subscription,
            propagator: self.propagator.clone(),
            tracer: self.tracer.clone(),
            metrics: self.metrics.clone(),
        })
    }
}
```

### 5. Webhook HTTP Middleware

**For Outbound Webhooks (Client)**:
```rust
pub struct TracedHttpClient {
    client: reqwest::Client,
    propagator: Arc<W3CTracePropagator>,
}

impl TracedHttpClient {
    pub async fn post(
        &self,
        url: &str,
        body: Vec<u8>,
    ) -> Result<Response> {
        let span = tracer()
            .span_builder("http.client.post")
            .with_kind(SpanKind::Client)
            .with_attribute("http.url", url)
            .start(tracer());
        
        let mut headers = reqwest::header::HeaderMap::new();
        self.propagator.inject(&span.context(), &mut headers)?;
        
        let response = self.client
            .post(url)
            .headers(headers)
            .body(body)
            .send()
            .await?;
        
        span.set_attribute("http.status_code", response.status().as_u16());
        span.end();
        Ok(response)
    }
}
```

**For Inbound Webhooks (Server)**:
```rust
// Axum middleware example
pub async fn tracing_middleware(
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Response {
    let propagator = W3CTracePropagator::new();
    
    let parent_context = propagator.extract(&headers).ok().flatten();
    
    let mut span_builder = tracer()
        .span_builder("http.server.request")
        .with_kind(SpanKind::Server)
        .with_attribute("http.method", request.method().as_str())
        .with_attribute("http.url", request.uri().to_string());
    
    if let Some(parent) = parent_context {
        span_builder = span_builder.with_parent_context(parent);
    }
    
    let span = span_builder.start(tracer());
    let _guard = span.enter();
    
    let response = next.run(request).await;
    
    span.set_attribute("http.status_code", response.status().as_u16());
    span.end();
    
    response
}
```

## Header Format & Overhead

### W3C Trace Context Headers

**traceparent** (55 bytes):
```
00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01
│  │                                │                │
│  └─ trace-id (32 hex chars)      │                └─ flags (2 hex)
│                                   └─ span-id (16 hex chars)
└─ version (2 hex chars)
```

**tracestate** (variable, ~50-100 bytes):
```
vendor1=value1,vendor2=value2
```

**Total overhead**: ~105-155 bytes (well under 200 byte target)

## Broken Chain Detection

### Detection Points
1. Consumer receives message without traceparent header
2. Consumer receives message with malformed traceparent
3. Trace ID in consumer span doesn't link to known producer trace

### Metrics
```prometheus
# Successful propagation
trace_context_injections_total{transport="kafka|nats|http"}
trace_context_extractions_total{transport="kafka|nats|http"}

# Failures
trace_context_broken_chains_total{transport, queue, topic, reason="missing|malformed"}
trace_context_injection_errors_total{transport}
trace_context_extraction_errors_total{transport}

# Latency
trace_context_injection_duration_seconds{transport}
trace_context_extraction_duration_seconds{transport}
```

## Performance Optimizations

### 1. Zero-Copy Header Injection
- Reuse header buffers
- Pre-allocate traceparent string
- Avoid unnecessary allocations

### 2. Lazy Span Creation
- Only create spans if sampling enabled
- Defer expensive attributes until needed

### 3. Batching Support
- For batch producers, create parent span for batch
- Individual messages link to batch span

### 4. Thread-Local Caching
```rust
thread_local! {
    static PROPAGATOR: W3CTracePropagator = W3CTracePropagator::new();
    static HEADER_BUFFER: RefCell<String> = RefCell::new(String::with_capacity(64));
}
```

## Configuration

**Environment Variables** (OpenTelemetry standard):
```bash
OTEL_SDK_DISABLED=false
OTEL_TRACES_SAMPLER=parentbased_traceidratio
OTEL_TRACES_SAMPLER_ARG=0.1
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317
```

**Programmatic Configuration**:
```rust
use traced_messaging::config::TracingConfig;

let config = TracingConfig::builder()
    .with_sampling_rate(0.1)
    .with_broken_chain_alerts(true)
    .with_max_header_size(200)
    .build();

let producer = TracedKafkaProducer::with_config(inner, tracer, config);
```

## Testing Strategy

### Unit Tests
- Traceparent formatting and parsing
- Header injection/extraction correctness
- Broken chain detection logic
- Error handling paths

### Integration Tests
- End-to-end trace propagation through Kafka
- End-to-end trace propagation through NATS
- End-to-end trace propagation through webhooks
- Multi-hop trace continuity
- Broken chain metric emission

### Performance Tests
- Throughput regression benchmark
- Latency overhead measurement
- Header size validation
- Memory allocation profiling

### Acceptance Tests
- 10,000+ message trace stitch rate measurement
- Broken chain detection rate validation
- Header overhead verification
- Performance regression validation

## Migration Guide

### For Kafka Users
```rust
// Step 1: Add dependency
[dependencies]
traced-messaging = "0.1"

// Step 2: Replace imports
// Before:
use kafka::producer::Producer;
// After:
use traced_messaging::kafka::TracedKafkaProducer;

// Step 3: Wrap existing producer
let inner_producer = kafka::producer::Producer::new(config)?;
let producer = TracedKafkaProducer::new(inner_producer, global::tracer("my-service"));

// Step 4: Use identical API
producer.send("my-topic", None, b"payload").await?;
```

### For NATS Users
Similar pattern with `TracedNatsClient`

### For Webhook Clients
Replace `reqwest::Client` with `TracedHttpClient`

## Rollout Plan

### Phase 1: Shadow Mode
- Deploy traced clients alongside existing clients
- Compare trace data without affecting production
- Validate overhead is acceptable

### Phase 2: Producer Rollout
- Replace producers with traced versions
- Monitor injection rate and broken chain metrics
- Ensure consumers still work with new headers

### Phase 3: Consumer Rollout
- Replace consumers with traced versions
- Verify trace continuity end-to-end
- Monitor trace stitch rate

### Phase 4: Full Production
- All services using traced clients
- Alerts configured for broken chains
- Dashboards showing trace coverage
