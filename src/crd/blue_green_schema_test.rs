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
//! Schema checks for Validator blue/green CRD surface (#1331).

#[cfg(test)]
mod blue_green_crd_schema {
    use std::fs;
    use std::path::PathBuf;

    use crate::crd::types::{BlueGreenStrategyConfig, RolloutStrategy, RolloutStrategyType};
    use crate::crd::{NodeType, StellarNetwork, StellarNodeSpec, ValidatorConfig};
    use schemars::schema_for;

    #[test]
    fn rust_rollout_strategy_schema_includes_blue_green() {
        let schema = schema_for!(RolloutStrategyType);
        let json = serde_json::to_string(&schema).expect("schema json");
        assert!(
            json.contains("BlueGreen") || json.contains("blueGreen"),
            "RolloutStrategyType schema must expose BlueGreen: {json}"
        );
    }

    #[test]
    fn committed_crd_yaml_includes_blue_green_enum() {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("config/crd/stellarnode-crd.yaml");
        let yaml = fs::read_to_string(&path).expect("read committed CRD");
        assert!(
            yaml.contains("- blueGreen"),
            "committed CRD must list blueGreen in strategy.type enum"
        );
        assert!(
            yaml.contains("blueGreen:"),
            "committed CRD must document blueGreen strategy config"
        );
        assert!(
            yaml.contains("blueGreenPhase:"),
            "committed CRD status must include blueGreenPhase"
        );
    }

    #[test]
    fn validator_accepts_blue_green_rejects_canary() {
        let ok = StellarNodeSpec {
            node_type: NodeType::Validator,
            network: StellarNetwork::Testnet,
            version: "v21.0.0".to_string(),
            validator_config: Some(ValidatorConfig {
                seed_secret_ref: "seed".into(),
                ..Default::default()
            }),
            strategy: RolloutStrategy {
                strategy_type: RolloutStrategyType::BlueGreen,
                canary: None,
                blue_green: Some(BlueGreenStrategyConfig::default()),
            },
            ..Default::default()
        };
        assert!(ok.validate().is_ok());

        let bad = StellarNodeSpec {
            strategy: RolloutStrategy {
                strategy_type: RolloutStrategyType::Canary,
                canary: None,
                blue_green: None,
            },
            ..ok
        };
        assert!(bad.validate().is_err());
    }
}
