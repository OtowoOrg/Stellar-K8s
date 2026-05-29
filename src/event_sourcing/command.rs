//! Command handling for CQRS

use serde::{Deserialize, Serialize};
use async_trait::async_trait;

use crate::error::Result;

/// Command trait
#[async_trait]
pub trait Command: Send + Sync {
    /// Get command name
    fn name(&self) -> &str;

    /// Get aggregate ID
    fn aggregate_id(&self) -> &str;

    /// Validate command
    async fn validate(&self) -> Result<()>;
}

/// Command result
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommandResult {
    pub success: bool,
    pub message: String,
    pub data: Option<serde_json::Value>,
}

impl CommandResult {
    pub fn success(message: impl Into<String>) -> Self {
        Self {
            success: true,
            message: message.into(),
            data: None,
        }
    }

    pub fn success_with_data(message: impl Into<String>, data: serde_json::Value) -> Self {
        Self {
            success: true,
            message: message.into(),
            data: Some(data),
        }
    }

    pub fn failure(message: impl Into<String>) -> Self {
        Self {
            success: false,
            message: message.into(),
            data: None,
        }
    }
}

/// Command handler trait
#[async_trait]
pub trait CommandHandler: Send + Sync {
    /// Handle command
    async fn handle(&self, command: &dyn Command) -> Result<CommandResult>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_result_success() {
        let result = CommandResult::success("Operation completed");
        assert!(result.success);
        assert_eq!(result.message, "Operation completed");
    }

    #[test]
    fn test_command_result_failure() {
        let result = CommandResult::failure("Operation failed");
        assert!(!result.success);
        assert_eq!(result.message, "Operation failed");
    }
}
