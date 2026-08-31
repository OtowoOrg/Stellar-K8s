// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//! Tests for health check functionality

#[cfg(test)]
mod tests {
    use super::super::health::*;

    #[test]
    fn test_health_check_result_synced() {
        let result = HealthCheckResult::synced(Some(12345));
        assert!(result.healthy);
        assert!(result.synced);
        assert_eq!(result.ledger_sequence, Some(12345));
    }

    #[test]
    fn test_health_check_result_syncing() {
        let result = HealthCheckResult::syncing("Syncing...".to_string(), Some(100));
        assert!(result.healthy);
        assert!(!result.synced);
        assert_eq!(result.ledger_sequence, Some(100));
    }

    #[test]
    fn test_health_check_result_unhealthy() {
        let result = HealthCheckResult::unhealthy("Connection failed".to_string());
        assert!(!result.healthy);
        assert!(!result.synced);
        assert_eq!(result.ledger_sequence, None);
    }

    #[test]
    fn test_health_check_result_pending() {
        let result = HealthCheckResult::pending("Pod not ready".to_string());
        assert!(!result.healthy);
        assert!(!result.synced);
        assert_eq!(result.ledger_sequence, None);
    }
}
