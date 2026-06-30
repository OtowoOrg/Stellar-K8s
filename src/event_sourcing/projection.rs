//! Projections - Read models for CQRS

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use chrono::{DateTime, Utc};
use tracing::debug;

use crate::error::Result;
use super::event::DomainEvent;

/// Projection configuration
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProjectionConfig {
    /// Enable projections
    pub enabled: bool,
    /// Projection update batch size
    pub batch_size: usize,
}

impl Default for ProjectionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            batch_size: 100,
        }
    }
}

/// Projection trait
pub trait Projection: Send + Sync {
    /// Get projection name
    fn name(&self) -> &str;

    /// Handle event
    fn handle_event(&mut self, event: &DomainEvent) -> Result<()>;

    /// Get projection state
    fn get_state(&self) -> serde_json::Value;
}

/// Projection Manager
pub struct ProjectionManager {
    config: ProjectionConfig,
    projections: tokio::sync::RwLock<HashMap<String, ProjectionState>>,
}

/// Projection state
#[derive(Clone, Debug, Serialize, Deserialize)]
struct ProjectionState {
    name: String,
    state: serde_json::Value,
    last_updated: DateTime<Utc>,
    event_count: u64,
}

impl ProjectionManager {
    /// Create a new projection manager
    pub async fn new(config: ProjectionConfig) -> Result<Self> {
        debug!("Initializing Projection Manager");
        Ok(Self {
            config,
            projections: tokio::sync::RwLock::new(HashMap::new()),
        })
    }

    /// Register projection
    pub async fn register_projection(&self, name: String, initial_state: serde_json::Value) -> Result<()> {
        debug!("Registering projection: {}", name);

        let mut projections = self.projections.write().await;
        projections.insert(
            name.clone(),
            ProjectionState {
                name,
                state: initial_state,
                last_updated: Utc::now(),
                event_count: 0,
            },
        );

        Ok(())
    }

    /// Update projection with event
    pub async fn update_projection(&self, projection_name: &str, event: &DomainEvent) -> Result<()> {
        let mut projections = self.projections.write().await;

        if let Some(projection) = projections.get_mut(projection_name) {
            // Apply event to projection state
            projection.state = apply_event_to_projection(&projection.state, event)?;
            projection.last_updated = Utc::now();
            projection.event_count += 1;
        }

        Ok(())
    }

    /// Get projection state
    pub async fn get_projection_state(&self, projection_name: &str) -> Result<Option<serde_json::Value>> {
        let projections = self.projections.read().await;
        Ok(projections.get(projection_name).map(|p| p.state.clone()))
    }

    /// Get all projections
    pub async fn get_all_projections(&self) -> Result<Vec<(String, serde_json::Value)>> {
        let projections = self.projections.read().await;
        Ok(projections
            .iter()
            .map(|(name, state)| (name.clone(), state.state.clone()))
            .collect())
    }

    /// Get projection statistics
    pub async fn get_statistics(&self) -> Result<ProjectionStatistics> {
        let projections = self.projections.read().await;

        let total_projections = projections.len();
        let total_events_processed: u64 = projections.values().map(|p| p.event_count).sum();

        Ok(ProjectionStatistics {
            total_projections,
            total_events_processed,
            timestamp: Utc::now(),
        })
    }
}

/// Apply event to projection state
fn apply_event_to_projection(
    state: &serde_json::Value,
    event: &DomainEvent,
) -> Result<serde_json::Value> {
    let mut new_state = state.clone();

    if let Some(obj) = new_state.as_object_mut() {
        obj.insert("last_event".to_string(), serde_json::json!(event.event_type));
        obj.insert("last_event_at".to_string(), serde_json::json!(event.metadata.timestamp));
        obj.insert("aggregate_id".to_string(), serde_json::json!(event.metadata.aggregate_id));
    }

    Ok(new_state)
}

/// Projection statistics
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectionStatistics {
    pub total_projections: usize,
    pub total_events_processed: u64,
    pub timestamp: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_projection_manager_creation() {
        let config = ProjectionConfig::default();
        let manager = ProjectionManager::new(config).await.unwrap();
        
        let stats = manager.get_statistics().await.unwrap();
        assert_eq!(stats.total_projections, 0);
    }

    #[tokio::test]
    async fn test_register_projection() {
        let config = ProjectionConfig::default();
        let manager = ProjectionManager::new(config).await.unwrap();

        manager
            .register_projection("test_projection".to_string(), serde_json::json!({}))
            .await
            .unwrap();

        let state = manager.get_projection_state("test_projection").await.unwrap();
        assert!(state.is_some());
    }
}
