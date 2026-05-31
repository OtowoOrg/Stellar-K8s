/// Advanced deployment strategies with progressive delivery (Issue #797)
///
/// Implements blue-green, canary, rolling, and feature-flag-based deployments
/// with automated rollback, deployment analytics, approval workflow, and
/// full audit trail.
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

// ── Strategy types ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentStrategyKind {
    BlueGreen,
    Canary,
    Rolling,
    FeatureFlag,
    Recreate,
}

// ── Health / rollback ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentHealth {
    Healthy,
    Degraded,
    Failed,
}

/// Criteria that trigger an automatic rollback.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackPolicy {
    /// Error rate threshold (0.0–1.0) that triggers rollback.
    pub error_rate_threshold: f64,
    /// Latency p99 threshold in milliseconds.
    pub latency_p99_threshold_ms: u64,
    /// Minimum number of healthy replicas required.
    pub min_healthy_replicas: u32,
    /// How many consecutive health-check failures before rollback.
    pub failure_count_threshold: u32,
}

impl Default for RollbackPolicy {
    fn default() -> Self {
        Self {
            error_rate_threshold: 0.05,
            latency_p99_threshold_ms: 2000,
            min_healthy_replicas: 1,
            failure_count_threshold: 3,
        }
    }
}

// ── Feature flags ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureFlagRollout {
    pub flag_name: String,
    /// Percentage of traffic (0–100) that sees the new version.
    pub rollout_percent: u8,
    /// Specific user/tenant IDs that always get the new version.
    pub allowlist: Vec<String>,
}

// ── Deployment config ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentConfig {
    pub name: String,
    pub namespace: String,
    pub strategy: DeploymentStrategyKind,
    pub image: String,
    pub target_replicas: u32,
    /// For canary: initial traffic percentage to new version.
    pub canary_initial_percent: u8,
    /// For canary: how much to increment per step.
    pub canary_step_percent: u8,
    /// For rolling: max pods unavailable at once.
    pub rolling_max_unavailable: u32,
    /// For rolling: max pods above desired during update.
    pub rolling_max_surge: u32,
    pub rollback_policy: RollbackPolicy,
    pub feature_flags: Vec<FeatureFlagRollout>,
    /// Require manual approval before proceeding past initial rollout.
    pub require_approval: bool,
}

impl Default for DeploymentConfig {
    fn default() -> Self {
        Self {
            name: "deployment".to_string(),
            namespace: "default".to_string(),
            strategy: DeploymentStrategyKind::Rolling,
            image: String::new(),
            target_replicas: 3,
            canary_initial_percent: 10,
            canary_step_percent: 10,
            rolling_max_unavailable: 1,
            rolling_max_surge: 1,
            rollback_policy: RollbackPolicy::default(),
            feature_flags: vec![],
            require_approval: false,
        }
    }
}

// ── Deployment state ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentPhase {
    Pending,
    WaitingApproval,
    InProgress,
    Paused,
    Succeeded,
    RollingBack,
    RolledBack,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentStatus {
    pub id: String,
    pub config: DeploymentConfig,
    pub phase: DeploymentPhase,
    pub current_traffic_percent: u8,
    pub healthy_replicas: u32,
    pub total_replicas: u32,
    pub error_rate: f64,
    pub latency_p99_ms: u64,
    pub consecutive_failures: u32,
    pub created_at: i64,
    pub updated_at: i64,
    pub completed_at: Option<i64>,
    pub rollback_reason: Option<String>,
}

impl DeploymentStatus {
    fn new(id: impl Into<String>, config: DeploymentConfig) -> Self {
        let now = Utc::now().timestamp();
        Self {
            id: id.into(),
            config,
            phase: DeploymentPhase::Pending,
            current_traffic_percent: 0,
            healthy_replicas: 0,
            total_replicas: 0,
            error_rate: 0.0,
            latency_p99_ms: 0,
            consecutive_failures: 0,
            created_at: now,
            updated_at: now,
            completed_at: None,
            rollback_reason: None,
        }
    }

    fn touch(&mut self) {
        self.updated_at = Utc::now().timestamp();
    }

    pub fn health(&self) -> DeploymentHealth {
        let policy = &self.config.rollback_policy;
        if self.error_rate >= policy.error_rate_threshold
            || self.latency_p99_ms >= policy.latency_p99_threshold_ms
            || self.healthy_replicas < policy.min_healthy_replicas
        {
            DeploymentHealth::Failed
        } else if self.error_rate >= policy.error_rate_threshold / 2.0 {
            DeploymentHealth::Degraded
        } else {
            DeploymentHealth::Healthy
        }
    }

    pub fn should_rollback(&self) -> bool {
        self.consecutive_failures >= self.config.rollback_policy.failure_count_threshold
            || self.health() == DeploymentHealth::Failed
    }
}

// ── Audit trail ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentEvent {
    pub deployment_id: String,
    pub timestamp: i64,
    pub event_type: String,
    pub actor: String,
    pub details: String,
}

impl DeploymentEvent {
    fn new(
        deployment_id: impl Into<String>,
        event_type: impl Into<String>,
        actor: impl Into<String>,
        details: impl Into<String>,
    ) -> Self {
        Self {
            deployment_id: deployment_id.into(),
            timestamp: Utc::now().timestamp(),
            event_type: event_type.into(),
            actor: actor.into(),
            details: details.into(),
        }
    }
}

// ── Analytics ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeploymentAnalytics {
    pub total_deployments: u64,
    pub successful_deployments: u64,
    pub rolled_back_deployments: u64,
    pub failed_deployments: u64,
    pub avg_duration_secs: f64,
    pub by_strategy: HashMap<String, u64>,
}

// ── Controller ────────────────────────────────────────────────────────────────

pub struct DeploymentController {
    deployments: HashMap<String, DeploymentStatus>,
    history: Vec<DeploymentStatus>,
    audit_log: Vec<DeploymentEvent>,
    analytics: DeploymentAnalytics,
}

impl DeploymentController {
    pub fn new() -> Self {
        Self {
            deployments: HashMap::new(),
            history: vec![],
            audit_log: vec![],
            analytics: DeploymentAnalytics::default(),
        }
    }

    // ── Lifecycle ─────────────────────────────────────────────────────────────

    pub fn create(&mut self, config: DeploymentConfig) -> String {
        let id = format!(
            "{}-{}",
            config.name,
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        let mut status = DeploymentStatus::new(&id, config.clone());

        if config.require_approval {
            status.phase = DeploymentPhase::WaitingApproval;
        }

        self.audit(DeploymentEvent::new(
            &id,
            "created",
            "system",
            format!("Strategy: {:?}, image: {}", config.strategy, config.image),
        ));
        self.analytics.total_deployments += 1;
        *self
            .analytics
            .by_strategy
            .entry(format!("{:?}", config.strategy))
            .or_insert(0) += 1;

        info!("Deployment {} created ({:?})", id, config.strategy);
        self.deployments.insert(id.clone(), status);
        id
    }

    pub fn approve(&mut self, id: &str, approver: impl Into<String>) -> bool {
        let approver = approver.into();
        if let Some(d) = self.deployments.get_mut(id) {
            if d.phase == DeploymentPhase::WaitingApproval {
                d.phase = DeploymentPhase::InProgress;
                d.touch();
                self.audit_log.push(DeploymentEvent::new(
                    id,
                    "approved",
                    &approver,
                    "Deployment approved",
                ));
                info!("Deployment {} approved by {}", id, approver);
                return true;
            }
        }
        false
    }

    /// Advance a canary deployment by one traffic step.
    pub fn advance_canary(&mut self, id: &str) -> Result<u8, String> {
        let d = self
            .deployments
            .get_mut(id)
            .ok_or_else(|| format!("Deployment '{}' not found", id))?;

        if d.config.strategy != DeploymentStrategyKind::Canary {
            return Err("Not a canary deployment".to_string());
        }
        if d.phase != DeploymentPhase::InProgress {
            return Err(format!("Deployment is in phase {:?}", d.phase));
        }
        if d.should_rollback() {
            return Err("Health check failed; rollback required".to_string());
        }

        let new_pct = (d.current_traffic_percent + d.config.canary_step_percent).min(100);
        d.current_traffic_percent = new_pct;
        d.touch();

        if new_pct >= 100 {
            d.phase = DeploymentPhase::Succeeded;
            d.completed_at = Some(Utc::now().timestamp());
            self.analytics.successful_deployments += 1;
            info!("Canary {} reached 100% — succeeded", id);
        }

        self.audit_log.push(DeploymentEvent::new(
            id,
            "canary_advanced",
            "system",
            format!("Traffic: {}%", new_pct),
        ));
        Ok(new_pct)
    }

    /// Perform a rolling update step (advances one batch of pods).
    pub fn advance_rolling(&mut self, id: &str) -> Result<u32, String> {
        let d = self
            .deployments
            .get_mut(id)
            .ok_or_else(|| format!("Deployment '{}' not found", id))?;

        if d.config.strategy != DeploymentStrategyKind::Rolling {
            return Err("Not a rolling deployment".to_string());
        }
        if d.should_rollback() {
            return Err("Health check failed; rollback required".to_string());
        }

        let batch = d.config.rolling_max_surge.max(1);
        let updated = (d.healthy_replicas + batch).min(d.config.target_replicas);
        d.healthy_replicas = updated;
        d.total_replicas = d.config.target_replicas;
        d.touch();

        if updated >= d.config.target_replicas {
            d.phase = DeploymentPhase::Succeeded;
            d.completed_at = Some(Utc::now().timestamp());
            self.analytics.successful_deployments += 1;
            info!("Rolling deployment {} succeeded", id);
        }

        self.audit_log.push(DeploymentEvent::new(
            id,
            "rolling_step",
            "system",
            format!("{}/{} replicas updated", updated, d.config.target_replicas),
        ));
        Ok(updated)
    }

    /// Complete a blue-green switch (traffic 0 → 100 atomically).
    pub fn switch_blue_green(&mut self, id: &str) -> Result<(), String> {
        let d = self
            .deployments
            .get_mut(id)
            .ok_or_else(|| format!("Deployment '{}' not found", id))?;

        if d.config.strategy != DeploymentStrategyKind::BlueGreen {
            return Err("Not a blue-green deployment".to_string());
        }
        if d.should_rollback() {
            return Err("Health check failed; rollback required".to_string());
        }

        d.current_traffic_percent = 100;
        d.phase = DeploymentPhase::Succeeded;
        d.completed_at = Some(Utc::now().timestamp());
        d.touch();
        self.analytics.successful_deployments += 1;

        self.audit_log.push(DeploymentEvent::new(
            id,
            "blue_green_switched",
            "system",
            "Traffic switched to green",
        ));
        info!("Blue-green deployment {} switched", id);
        Ok(())
    }

    /// Update a feature-flag rollout percentage.
    pub fn update_feature_flag(&mut self, id: &str, flag: &str, percent: u8) -> bool {
        if let Some(d) = self.deployments.get_mut(id) {
            if let Some(ff) = d.config.feature_flags.iter_mut().find(|f| f.flag_name == flag) {
                ff.rollout_percent = percent.min(100);
                d.touch();
                self.audit_log.push(DeploymentEvent::new(
                    id,
                    "feature_flag_updated",
                    "system",
                    format!("Flag '{}' set to {}%", flag, percent),
                ));
                if percent >= 100 {
                    d.phase = DeploymentPhase::Succeeded;
                    d.completed_at = Some(Utc::now().timestamp());
                    self.analytics.successful_deployments += 1;
                }
                return true;
            }
        }
        false
    }

    // ── Rollback ──────────────────────────────────────────────────────────────

    pub fn rollback(&mut self, id: &str, reason: impl Into<String>) -> bool {
        let reason = reason.into();
        if let Some(d) = self.deployments.get_mut(id) {
            warn!("Rolling back deployment {}: {}", id, reason);
            d.phase = DeploymentPhase::RolledBack;
            d.current_traffic_percent = 0;
            d.rollback_reason = Some(reason.clone());
            d.completed_at = Some(Utc::now().timestamp());
            d.touch();
            self.analytics.rolled_back_deployments += 1;
            self.audit_log.push(DeploymentEvent::new(
                id,
                "rolled_back",
                "system",
                &reason,
            ));
            return true;
        }
        false
    }

    /// Check all in-progress deployments and auto-rollback if health fails.
    pub fn reconcile_health(&mut self) {
        let ids: Vec<String> = self
            .deployments
            .keys()
            .cloned()
            .collect();

        for id in ids {
            let should = self
                .deployments
                .get(&id)
                .map(|d| {
                    d.phase == DeploymentPhase::InProgress && d.should_rollback()
                })
                .unwrap_or(false);

            if should {
                self.rollback(&id, "Automated rollback: health check failed");
            }
        }
    }

    // ── Metrics update ────────────────────────────────────────────────────────

    pub fn update_metrics(
        &mut self,
        id: &str,
        error_rate: f64,
        latency_p99_ms: u64,
        healthy_replicas: u32,
    ) {
        if let Some(d) = self.deployments.get_mut(id) {
            d.error_rate = error_rate;
            d.latency_p99_ms = latency_p99_ms;
            d.healthy_replicas = healthy_replicas;
            if d.health() == DeploymentHealth::Failed {
                d.consecutive_failures += 1;
            } else {
                d.consecutive_failures = 0;
            }
            d.touch();
        }
    }

    // ── Accessors ─────────────────────────────────────────────────────────────

    pub fn get(&self, id: &str) -> Option<&DeploymentStatus> {
        self.deployments.get(id)
    }

    pub fn list_active(&self) -> Vec<&DeploymentStatus> {
        self.deployments
            .values()
            .filter(|d| {
                !matches!(
                    d.phase,
                    DeploymentPhase::Succeeded
                        | DeploymentPhase::RolledBack
                        | DeploymentPhase::Failed
                )
            })
            .collect()
    }

    pub fn audit_log(&self) -> &[DeploymentEvent] {
        &self.audit_log
    }

    pub fn analytics(&self) -> &DeploymentAnalytics {
        &self.analytics
    }

    pub fn history(&self) -> &[DeploymentStatus] {
        &self.history
    }

    fn audit(&mut self, event: DeploymentEvent) {
        self.audit_log.push(event);
    }
}

impl Default for DeploymentController {
    fn default() -> Self {
        Self::new()
    }
}

pub type SharedDeploymentController = Arc<RwLock<DeploymentController>>;

pub fn new_shared() -> SharedDeploymentController {
    Arc::new(RwLock::new(DeploymentController::new()))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn canary_config() -> DeploymentConfig {
        DeploymentConfig {
            name: "my-app".to_string(),
            strategy: DeploymentStrategyKind::Canary,
            image: "stellar/node:v2".to_string(),
            canary_initial_percent: 10,
            canary_step_percent: 25,
            ..Default::default()
        }
    }

    fn rolling_config() -> DeploymentConfig {
        DeploymentConfig {
            name: "my-app".to_string(),
            strategy: DeploymentStrategyKind::Rolling,
            image: "stellar/node:v2".to_string(),
            target_replicas: 4,
            rolling_max_surge: 2,
            ..Default::default()
        }
    }

    #[test]
    fn test_canary_progressive_rollout() {
        let mut ctrl = DeploymentController::new();
        let id = ctrl.create(canary_config());
        // Manually set to InProgress
        ctrl.deployments.get_mut(&id).unwrap().phase = DeploymentPhase::InProgress;

        let pct = ctrl.advance_canary(&id).unwrap();
        assert_eq!(pct, 25);
        let pct = ctrl.advance_canary(&id).unwrap();
        assert_eq!(pct, 50);
        let pct = ctrl.advance_canary(&id).unwrap();
        assert_eq!(pct, 75);
        let pct = ctrl.advance_canary(&id).unwrap();
        assert_eq!(pct, 100);
        assert_eq!(ctrl.get(&id).unwrap().phase, DeploymentPhase::Succeeded);
    }

    #[test]
    fn test_canary_auto_rollback_on_health_failure() {
        let mut ctrl = DeploymentController::new();
        let id = ctrl.create(canary_config());
        ctrl.deployments.get_mut(&id).unwrap().phase = DeploymentPhase::InProgress;

        // Inject bad metrics
        ctrl.update_metrics(&id, 0.9, 5000, 0);
        ctrl.reconcile_health();

        assert_eq!(ctrl.get(&id).unwrap().phase, DeploymentPhase::RolledBack);
        assert!(ctrl.get(&id).unwrap().rollback_reason.is_some());
    }

    #[test]
    fn test_rolling_deployment() {
        let mut ctrl = DeploymentController::new();
        let id = ctrl.create(rolling_config());
        ctrl.deployments.get_mut(&id).unwrap().phase = DeploymentPhase::InProgress;

        ctrl.advance_rolling(&id).unwrap();
        ctrl.advance_rolling(&id).unwrap();
        assert_eq!(ctrl.get(&id).unwrap().phase, DeploymentPhase::Succeeded);
    }

    #[test]
    fn test_blue_green_switch() {
        let mut ctrl = DeploymentController::new();
        let id = ctrl.create(DeploymentConfig {
            strategy: DeploymentStrategyKind::BlueGreen,
            ..Default::default()
        });
        ctrl.deployments.get_mut(&id).unwrap().phase = DeploymentPhase::InProgress;
        ctrl.switch_blue_green(&id).unwrap();
        assert_eq!(ctrl.get(&id).unwrap().current_traffic_percent, 100);
        assert_eq!(ctrl.get(&id).unwrap().phase, DeploymentPhase::Succeeded);
    }

    #[test]
    fn test_approval_workflow() {
        let mut ctrl = DeploymentController::new();
        let id = ctrl.create(DeploymentConfig {
            require_approval: true,
            ..Default::default()
        });
        assert_eq!(ctrl.get(&id).unwrap().phase, DeploymentPhase::WaitingApproval);
        assert!(ctrl.approve(&id, "ops-team"));
        assert_eq!(ctrl.get(&id).unwrap().phase, DeploymentPhase::InProgress);
    }

    #[test]
    fn test_feature_flag_rollout() {
        let mut ctrl = DeploymentController::new();
        let id = ctrl.create(DeploymentConfig {
            strategy: DeploymentStrategyKind::FeatureFlag,
            feature_flags: vec![FeatureFlagRollout {
                flag_name: "new-ui".to_string(),
                rollout_percent: 0,
                allowlist: vec![],
            }],
            ..Default::default()
        });
        ctrl.deployments.get_mut(&id).unwrap().phase = DeploymentPhase::InProgress;
        assert!(ctrl.update_feature_flag(&id, "new-ui", 50));
        assert_eq!(
            ctrl.get(&id).unwrap().config.feature_flags[0].rollout_percent,
            50
        );
        ctrl.update_feature_flag(&id, "new-ui", 100);
        assert_eq!(ctrl.get(&id).unwrap().phase, DeploymentPhase::Succeeded);
    }

    #[test]
    fn test_audit_trail() {
        let mut ctrl = DeploymentController::new();
        let id = ctrl.create(canary_config());
        ctrl.rollback(&id, "manual rollback");
        let events: Vec<&str> = ctrl.audit_log().iter().map(|e| e.event_type.as_str()).collect();
        assert!(events.contains(&"created"));
        assert!(events.contains(&"rolled_back"));
    }

    #[test]
    fn test_analytics() {
        let mut ctrl = DeploymentController::new();
        let id1 = ctrl.create(canary_config());
        ctrl.deployments.get_mut(&id1).unwrap().phase = DeploymentPhase::InProgress;
        // Drive to success
        for _ in 0..4 {
            let _ = ctrl.advance_canary(&id1);
        }
        assert_eq!(ctrl.analytics().successful_deployments, 1);
        assert_eq!(ctrl.analytics().total_deployments, 1);
    }

    #[test]
    fn test_manual_rollback() {
        let mut ctrl = DeploymentController::new();
        let id = ctrl.create(canary_config());
        ctrl.rollback(&id, "operator decision");
        assert_eq!(ctrl.get(&id).unwrap().phase, DeploymentPhase::RolledBack);
        assert_eq!(ctrl.analytics().rolled_back_deployments, 1);
    }
}
