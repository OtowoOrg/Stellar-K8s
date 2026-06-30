//! Fine-Grained Access Control Engine
//!
//! Implements Attribute-Based Access Control (ABAC) and Role-Based Access Control (RBAC)

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use chrono::{DateTime, Utc};
use tracing::{debug, warn};

use crate::error::Result;
use super::types::{Identity, IdentityAttributes};

/// Access control configuration
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AccessControlConfig {
    /// Enable RBAC
    pub rbac_enabled: bool,
    /// Enable ABAC
    pub abac_enabled: bool,
    /// Default deny policy
    pub default_deny: bool,
}

/// Access request
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccessRequest {
    /// Identity requesting access
    pub identity: Identity,
    /// Resource being accessed
    pub resource: String,
    /// Action being performed
    pub action: String,
    /// Additional context
    pub context: HashMap<String, serde_json::Value>,
}

/// Access decision
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum AccessDecision {
    /// Access granted
    Allow,
    /// Access denied
    Deny,
    /// Requires additional verification
    RequiresMfa,
}

/// Access policy
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccessPolicy {
    /// Policy ID
    pub id: String,
    /// Policy name
    pub name: String,
    /// Policy description
    pub description: Option<String>,
    /// Policy rules
    pub rules: Vec<AccessRule>,
    /// When policy was created
    pub created_at: DateTime<Utc>,
    /// When policy was last updated
    pub updated_at: DateTime<Utc>,
    /// Whether policy is active
    pub active: bool,
}

/// Access rule
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccessRule {
    /// Rule ID
    pub id: String,
    /// Rule name
    pub name: String,
    /// Effect (Allow or Deny)
    pub effect: RuleEffect,
    /// Principal (who)
    pub principal: Principal,
    /// Action (what)
    pub actions: Vec<String>,
    /// Resource (on what)
    pub resources: Vec<String>,
    /// Conditions (when)
    pub conditions: Vec<Condition>,
}

/// Rule effect
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum RuleEffect {
    Allow,
    Deny,
}

/// Principal specification
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Principal {
    /// All principals
    All,
    /// Specific user
    User { id: String },
    /// Users with specific role
    Role { name: String },
    /// Users in specific group
    Group { name: String },
    /// Users matching attribute condition
    Attribute { key: String, value: String },
}

/// Condition for rule evaluation
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Condition {
    /// Condition key
    pub key: String,
    /// Condition operator
    pub operator: ConditionOperator,
    /// Condition value
    pub value: serde_json::Value,
}

/// Condition operator
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConditionOperator {
    /// Equals
    Equals,
    /// Not equals
    NotEquals,
    /// Contains
    Contains,
    /// In list
    In,
    /// Not in list
    NotIn,
    /// Greater than
    GreaterThan,
    /// Less than
    LessThan,
    /// IP address match
    IpMatch,
    /// Time range
    TimeRange,
}

/// Access Control Engine
pub struct AccessControlEngine {
    config: AccessControlConfig,
    policies: tokio::sync::RwLock<HashMap<String, AccessPolicy>>,
    role_mappings: tokio::sync::RwLock<HashMap<String, Vec<String>>>,
}

impl AccessControlEngine {
    /// Create a new access control engine
    pub async fn new(config: AccessControlConfig) -> Result<Self> {
        debug!("Initializing Access Control Engine");
        Ok(Self {
            config,
            policies: tokio::sync::RwLock::new(HashMap::new()),
            role_mappings: tokio::sync::RwLock::new(HashMap::new()),
        })
    }

    /// Evaluate access request
    pub async fn evaluate(&self, request: &AccessRequest) -> Result<AccessDecision> {
        debug!(
            "Evaluating access request: {} {} {}",
            request.identity.id.0, request.action, request.resource
        );

        let policies = self.policies.read().await;

        // Collect all applicable policies
        let mut allow_rules = Vec::new();
        let mut deny_rules = Vec::new();

        for policy in policies.values() {
            if !policy.active {
                continue;
            }

            for rule in &policy.rules {
                if self.matches_principal(&request.identity, &rule.principal).await?
                    && self.matches_actions(&request.action, &rule.actions)
                    && self.matches_resources(&request.resource, &rule.resources)
                    && self.matches_conditions(&request.context, &rule.conditions).await?
                {
                    match rule.effect {
                        RuleEffect::Allow => allow_rules.push(rule),
                        RuleEffect::Deny => deny_rules.push(rule),
                    }
                }
            }
        }

        // Deny takes precedence
        if !deny_rules.is_empty() {
            warn!(
                "Access denied for {}: matched {} deny rules",
                request.identity.id.0,
                deny_rules.len()
            );
            return Ok(AccessDecision::Deny);
        }

        // Check if any allow rules matched
        if !allow_rules.is_empty() {
            debug!(
                "Access allowed for {}: matched {} allow rules",
                request.identity.id.0,
                allow_rules.len()
            );
            return Ok(AccessDecision::Allow);
        }

        // Default policy
        if self.config.default_deny {
            warn!("Access denied for {}: default deny policy", request.identity.id.0);
            Ok(AccessDecision::Deny)
        } else {
            debug!("Access allowed for {}: default allow policy", request.identity.id.0);
            Ok(AccessDecision::Allow)
        }
    }

    /// Add access policy
    pub async fn add_policy(&self, policy: AccessPolicy) -> Result<()> {
        debug!("Adding access policy: {}", policy.name);
        let mut policies = self.policies.write().await;
        policies.insert(policy.id.clone(), policy);
        Ok(())
    }

    /// Remove access policy
    pub async fn remove_policy(&self, policy_id: &str) -> Result<()> {
        debug!("Removing access policy: {}", policy_id);
        let mut policies = self.policies.write().await;
        policies.remove(policy_id);
        Ok(())
    }

    /// Get access policy
    pub async fn get_policy(&self, policy_id: &str) -> Result<Option<AccessPolicy>> {
        let policies = self.policies.read().await;
        Ok(policies.get(policy_id).cloned())
    }

    /// List all policies
    pub async fn list_policies(&self) -> Result<Vec<AccessPolicy>> {
        let policies = self.policies.read().await;
        Ok(policies.values().cloned().collect())
    }

    /// Map role to permissions
    pub async fn map_role(&self, role: String, permissions: Vec<String>) -> Result<()> {
        debug!("Mapping role {} to {} permissions", role, permissions.len());
        let mut mappings = self.role_mappings.write().await;
        mappings.insert(role, permissions);
        Ok(())
    }

    /// Get role permissions
    pub async fn get_role_permissions(&self, role: &str) -> Result<Vec<String>> {
        let mappings = self.role_mappings.read().await;
        Ok(mappings.get(role).cloned().unwrap_or_default())
    }

    // Private helper methods

    async fn matches_principal(&self, identity: &Identity, principal: &Principal) -> Result<bool> {
        match principal {
            Principal::All => Ok(true),
            Principal::User { id } => Ok(&identity.id.0 == id),
            Principal::Role { name } => {
                Ok(identity.attributes.roles.contains(name))
            }
            Principal::Group { name } => {
                Ok(identity.attributes.groups.contains(name))
            }
            Principal::Attribute { key, value } => {
                if let Some(attr_value) = identity.attributes.custom_attributes.get(key) {
                    Ok(attr_value.as_str() == Some(value))
                } else {
                    Ok(false)
                }
            }
        }
    }

    fn matches_actions(&self, action: &str, actions: &[String]) -> bool {
        actions.iter().any(|a| {
            if a.ends_with('*') {
                action.starts_with(&a[..a.len() - 1])
            } else {
                a == action
            }
        })
    }

    fn matches_resources(&self, resource: &str, resources: &[String]) -> bool {
        resources.iter().any(|r| {
            if r.ends_with('*') {
                resource.starts_with(&r[..r.len() - 1])
            } else {
                r == resource
            }
        })
    }

    async fn matches_conditions(
        &self,
        context: &HashMap<String, serde_json::Value>,
        conditions: &[Condition],
    ) -> Result<bool> {
        for condition in conditions {
            if !self.evaluate_condition(context, condition).await? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    async fn evaluate_condition(
        &self,
        context: &HashMap<String, serde_json::Value>,
        condition: &Condition,
    ) -> Result<bool> {
        let context_value = match context.get(&condition.key) {
            Some(v) => v,
            None => return Ok(false),
        };

        match condition.operator {
            ConditionOperator::Equals => Ok(context_value == &condition.value),
            ConditionOperator::NotEquals => Ok(context_value != &condition.value),
            ConditionOperator::Contains => {
                if let (Some(s), Some(substr)) = (context_value.as_str(), condition.value.as_str()) {
                    Ok(s.contains(substr))
                } else {
                    Ok(false)
                }
            }
            ConditionOperator::In => {
                if let Some(arr) = condition.value.as_array() {
                    Ok(arr.contains(context_value))
                } else {
                    Ok(false)
                }
            }
            ConditionOperator::NotIn => {
                if let Some(arr) = condition.value.as_array() {
                    Ok(!arr.contains(context_value))
                } else {
                    Ok(true)
                }
            }
            ConditionOperator::GreaterThan => {
                if let (Some(cv), Some(cv_num)) = (context_value.as_i64(), condition.value.as_i64()) {
                    Ok(cv > cv_num)
                } else {
                    Ok(false)
                }
            }
            ConditionOperator::LessThan => {
                if let (Some(cv), Some(cv_num)) = (context_value.as_i64(), condition.value.as_i64()) {
                    Ok(cv < cv_num)
                } else {
                    Ok(false)
                }
            }
            ConditionOperator::IpMatch => {
                // Simplified IP matching
                if let (Some(ip), Some(pattern)) = (context_value.as_str(), condition.value.as_str()) {
                    Ok(ip.starts_with(pattern))
                } else {
                    Ok(false)
                }
            }
            ConditionOperator::TimeRange => {
                // Simplified time range check
                Ok(true)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_access_control_engine_creation() {
        let config = AccessControlConfig::default();
        let engine = AccessControlEngine::new(config).await.unwrap();
        assert!(!engine.config.default_deny);
    }

    #[tokio::test]
    async fn test_principal_matching() {
        let config = AccessControlConfig::default();
        let engine = AccessControlEngine::new(config).await.unwrap();

        let mut identity = Identity::new(
            super::super::types::IdentityId::new("user123"),
            "google".to_string(),
            "user@example.com".to_string(),
        );
        identity.attributes.roles.push("admin".to_string());

        let principal = Principal::Role {
            name: "admin".to_string(),
        };

        let matches = engine.matches_principal(&identity, &principal).await.unwrap();
        assert!(matches);
    }
}
