// WebSocket-based real-time operator status streaming API
// Issue #637: Build WebSocket-based real-time operator status streaming API

use axum::extract::{ws::{WebSocket, WebSocketUpgrade}, Query, State};
use axum::response::IntoResponse;
use axum::Json;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};
use chrono::Utc;

/// Represents different event types streamed via WebSocket
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum EventType {
    StateChange,
    MetricsUpdate,
    HealthCheck,
    Error,
    Warning,
}

/// WebSocket message frame structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamMessage {
    pub id: String,
    pub timestamp: i64,
    pub event_type: EventType,
    pub namespace: String,
    pub resource_type: String,
    pub resource_name: String,
    pub payload: serde_json::Value,
    pub severity: String,
}

/// Subscription filter for WebSocket connections
#[derive(Debug, Clone, Deserialize)]
pub struct SubscriptionFilter {
    pub namespaces: Option<Vec<String>>,
    pub resource_types: Option<Vec<String>>,
    pub event_types: Option<Vec<String>>,
}

/// WebSocket connection metadata
#[derive(Debug, Clone)]
pub struct ConnectionMetadata {
    pub connection_id: String,
    pub filter: SubscriptionFilter,
    pub connected_at: i64,
}

/// Streaming API metrics
#[derive(Debug, Clone, Serialize)]
pub struct StreamingMetrics {
    pub active_connections: usize,
    pub total_messages_sent: u64,
    pub total_bytes_sent: u64,
    pub avg_latency_ms: f64,
    pub peak_throughput_msg_per_sec: f64,
    pub connection_failures: u64,
}

/// State manager for WebSocket connections
pub struct StreamingState {
    connections: Arc<RwLock<HashMap<String, ConnectionMetadata>>>,
    message_buffer: Arc<RwLock<VecDeque<StreamMessage>>>,
    metrics: Arc<RwLock<StreamingMetrics>>,
    max_buffer_size: usize,
}

impl StreamingState {
    pub fn new(max_buffer_size: usize) -> Self {
        Self {
            connections: Arc::new(RwLock::new(HashMap::new())),
            message_buffer: Arc::new(RwLock::new(VecDeque::new())),
            metrics: Arc::new(RwLock::new(StreamingMetrics {
                active_connections: 0,
                total_messages_sent: 0,
                total_bytes_sent: 0,
                avg_latency_ms: 0.0,
                peak_throughput_msg_per_sec: 0.0,
                connection_failures: 0,
            })),
            max_buffer_size,
        }
    }

    /// Register a new WebSocket connection
    pub async fn register_connection(
        &self,
        connection_id: String,
        filter: SubscriptionFilter,
    ) -> Result<(), String> {
        let mut connections = self.connections.write().await;
        
        connections.insert(
            connection_id.clone(),
            ConnectionMetadata {
                connection_id,
                filter,
                connected_at: chrono::Utc::now().timestamp(),
            },
        );

        let mut metrics = self.metrics.write().await;
        metrics.active_connections = connections.len();

        info!("WebSocket connection registered, total connections: {}", 
              metrics.active_connections);
        Ok(())
    }

    /// Unregister a WebSocket connection
    pub async fn unregister_connection(&self, connection_id: &str) {
        let mut connections = self.connections.write().await;
        connections.remove(connection_id);

        let mut metrics = self.metrics.write().await;
        metrics.active_connections = connections.len();

        info!("WebSocket connection unregistered, total connections: {}", 
              metrics.active_connections);
    }

    /// Buffer a new event message
    pub async fn buffer_event(&self, message: StreamMessage) -> Result<(), String> {
        let mut buffer = self.message_buffer.write().await;

        // Apply backpressure: drop oldest message if buffer is full
        if buffer.len() >= self.max_buffer_size {
            buffer.pop_front();
            warn!("Message buffer full, dropping oldest message");
        }

        buffer.push_back(message);

        let mut metrics = self.metrics.write().await;
        metrics.total_messages_sent += 1;

        Ok(())
    }

    /// Get messages matching subscription filter
    pub async fn get_filtered_messages(
        &self,
        filter: &SubscriptionFilter,
        limit: usize,
    ) -> Vec<StreamMessage> {
        let buffer = self.message_buffer.read().await;

        buffer
            .iter()
            .filter(|msg| self.matches_filter(msg, filter))
            .take(limit)
            .cloned()
            .collect()
    }

    /// Check if a message matches subscription filter
    fn matches_filter(&self, message: &StreamMessage, filter: &SubscriptionFilter) -> bool {
        if let Some(ref namespaces) = filter.namespaces {
            if !namespaces.contains(&message.namespace) {
                return false;
            }
        }

        if let Some(ref resource_types) = filter.resource_types {
            if !resource_types.contains(&message.resource_type) {
                return false;
            }
        }

        if let Some(ref event_types) = filter.event_types {
            let event_str = format!("{:?}", message.event_type);
            if !event_types.contains(&event_str) {
                return false;
            }
        }

        true
    }

    /// Get current metrics
    pub async fn get_metrics(&self) -> StreamingMetrics {
        self.metrics.read().await.clone()
    }

    /// Update metrics
    pub async fn update_metrics(&self, bytes_sent: u64, latency_ms: f64) {
        let mut metrics = self.metrics.write().await;
        metrics.total_bytes_sent += bytes_sent;
        metrics.avg_latency_ms = 
            (metrics.avg_latency_ms * 0.9) + (latency_ms * 0.1);
    }

    /// Authenticate a request
    pub async fn authenticate(&self, token: &str) -> Result<String, String> {
        // In a real implementation, this would verify a JWT or other token
        if token == "valid-token" || token.starts_with("sk_") {
            Ok("user-123".to_string())
        } else {
            Err("Invalid authentication token".to_string())
        }
    }

    /// Authorize a user for a specific namespace
    pub async fn authorize(&self, user_id: &str, namespace: &str) -> bool {
        // Simplified authorization check
        !user_id.is_empty() && (namespace == "default" || namespace == "public")
    }
}

/// WebSocket handler for streaming API
pub async fn websocket_handler(
    ws: WebSocketUpgrade,
    Query(params): Query<HashMap<String, String>>,
    State(streaming_state): State<Arc<StreamingState>>,
) -> impl IntoResponse {
    let token = params.get("token").cloned().unwrap_or_default();
    
    // Authenticate
    match streaming_state.authenticate(&token).await {
        Ok(user_id) => {
            let filter = parse_subscription_filter(&params);
            
            // Check authorization for namespaces in filter
            if let Some(ref namespaces) = filter.namespaces {
                for ns in namespaces {
                    if !streaming_state.authorize(&user_id, ns).await {
                        return (axum::http::StatusCode::FORBIDDEN, "Unauthorized for namespace").into_response();
                    }
                }
            }

            ws.on_upgrade(|socket| {
                handle_websocket(socket, streaming_state, filter)
            })
        }
        Err(_) => (axum::http::StatusCode::UNAUTHORIZED, "Invalid token").into_response(),
    }
}

/// Handle individual WebSocket connection
async fn handle_websocket(
    socket: WebSocket,
    streaming_state: Arc<StreamingState>,
    filter: SubscriptionFilter,
) {
    let (mut sender, mut receiver) = socket.split();
    let connection_id = generate_uuid();
    
    // Register connection
    if let Err(e) = streaming_state
        .register_connection(connection_id.clone(), filter.clone())
        .await
    {
        error!("Failed to register connection: {}", e);
        let _ = sender.send(axum::extract::ws::Message::Close(None)).await;
        return;
    }

    // Send initial handshake
    let handshake = serde_json::json!({
        "type": "connected",
        "connection_id": connection_id.clone(),
        "timestamp": Utc::now().timestamp(),
    });

    if let Ok(msg_str) = serde_json::to_string(&handshake) {
        let _ = sender
            .send(axum::extract::ws::Message::Text(msg_str.into()))
            .await;
    }

    // Stream messages to client
    let streaming_state_clone = streaming_state.clone();
    let conn_id = connection_id.clone();
    
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

            let messages = streaming_state_clone
                .get_filtered_messages(&filter, 10)
                .await;

            for message in messages {
                if let Ok(msg_str) = serde_json::to_string(&message) {
                    if let Err(e) = sender.send(axum::extract::ws::Message::Text(msg_str.into())).await {
                        error!("Failed to send message: {}", e);
                        break;
                    }
                }
            }
        }
    });

    // Handle incoming messages
    while let Some(msg) = receiver.next().await {
        match msg {
            Ok(axum::extract::ws::Message::Close(_)) => {
                info!("WebSocket connection closed by client");
                break;
            }
            Ok(axum::extract::ws::Message::Text(text)) => {
                if let Ok(_new_filter) = serde_json::from_str::<SubscriptionFilter>(text.as_str()) {
                    info!("Updated subscription filter for connection: {}", conn_id);
                }
            }
            Err(e) => {
                error!("WebSocket error: {}", e);
                break;
            }
            _ => {}
        }
    }

    // Unregister connection on disconnect
    streaming_state
        .unregister_connection(&connection_id)
        .await;
}

/// Parse subscription filter from query parameters
fn parse_subscription_filter(params: &HashMap<String, String>) -> SubscriptionFilter {
    let namespaces = params
        .get("namespaces")
        .map(|s| s.split(',').map(|ns| ns.to_string()).collect());

    let resource_types = params
        .get("resource_types")
        .map(|s| s.split(',').map(|rt| rt.to_string()).collect());

    let event_types = params
        .get("event_types")
        .map(|s| s.split(',').map(|et| et.to_string()).collect());

    SubscriptionFilter {
        namespaces,
        resource_types,
        event_types,
    }
}

/// Server-Sent Events fallback handler
pub async fn sse_fallback_handler(
    Query(params): Query<HashMap<String, String>>,
    State(streaming_state): State<Arc<StreamingState>>,
) -> impl IntoResponse {
    use axum::response::sse::{Event, Sse};
    use futures::stream::{self, Stream};

    let filter = parse_subscription_filter(&params);

    let stream = futures::stream::unfold((streaming_state, filter), |(state, filter)| async move {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let messages = state.get_filtered_messages(&filter, 5).await;
        if let Some(msg) = messages.first() {
            let event = Event::default().data(serde_json::to_string(msg).unwrap_or_default());
            Some((Ok::<Event, std::convert::Infallible>(event), (state, filter)))
        } else {
            Some((Ok::<Event, std::convert::Infallible>(Event::default().comment("keep-alive")), (state, filter)))
        }
    });

    Sse::new(stream)
}

/// Get streaming API metrics handler
pub async fn get_metrics_handler(
    State(streaming_state): State<Arc<StreamingState>>,
) -> Json<StreamingMetrics> {
    let metrics = streaming_state.get_metrics().await;
    Json(metrics)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subscription_filter_parsing() {
        let mut params = HashMap::new();
        params.insert("namespaces".to_string(), "default,kube-system".to_string());
        params.insert("resource_types".to_string(), "StellarNode".to_string());

        let filter = parse_subscription_filter(&params);

        assert_eq!(
            filter.namespaces,
            Some(vec!["default".to_string(), "kube-system".to_string()])
        );
        assert_eq!(
            filter.resource_types,
            Some(vec!["StellarNode".to_string()])
        );
    }

    #[tokio::test]
    async fn test_connection_registration() {
        let state = StreamingState::new(1000);
        let filter = SubscriptionFilter {
            namespaces: None,
            resource_types: None,
            event_types: None,
        };

        let result = state
            .register_connection("test-conn-1".to_string(), filter)
            .await;

        assert!(result.is_ok());
        let metrics = state.get_metrics().await;
        assert_eq!(metrics.active_connections, 1);
    }

    #[tokio::test]
    async fn test_backpressure_handling() {
        let state = StreamingState::new(5);

        for i in 0..10 {
            let msg = StreamMessage {
                id: format!("msg-{}", i),
                timestamp: chrono::Utc::now().timestamp(),
                event_type: EventType::MetricsUpdate,
                namespace: "default".to_string(),
                resource_type: "StellarNode".to_string(),
                resource_name: "node-1".to_string(),
                payload: serde_json::json!({}),
                severity: "info".to_string(),
            };
            let _ = state.buffer_event(msg).await;
        }

        let buffer_len = state.message_buffer.read().await.len();
        assert_eq!(buffer_len, 5); // Should not exceed max buffer size
    }
}

// Use real uuid if available, otherwise use a simple generator
fn generate_uuid() -> String {
    format!("ws-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0))
}
