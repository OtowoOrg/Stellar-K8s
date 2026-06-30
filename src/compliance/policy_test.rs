//! Policy testing and validation framework.

use serde::{Deserialize, Serialize};

use super::opa::Policy;
use super::policy_engine::PolicyEngine;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyTestCase {
    pub name: String,
    pub policy_id: String,
    pub input_json: serde_json::Value,
    pub expected_pass: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestCaseResult {
    pub test_name: String,
    pub policy_id: String,
    pub passed: bool,
    pub expected: bool,
    pub actual: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResults {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub results: Vec<TestCaseResult>,
}

impl TestResults {
    pub fn all_passed(&self) -> bool {
        self.failed == 0
    }
}

pub struct PolicyTestRunner {
    engine: PolicyEngine,
}

impl PolicyTestRunner {
    pub fn new(policies: Vec<Policy>) -> Self {
        Self {
            engine: PolicyEngine::new(policies),
        }
    }

    pub fn run_tests(&self, cases: &[PolicyTestCase]) -> TestResults {
        let mut results = Vec::new();

        for case in cases {
            let violations =
                self.engine
                    .evaluate_all("test-resource", "StellarNode", &case.input_json);
            let policy_violated = violations.iter().any(|v| v.policy_id == case.policy_id);
            let actual_pass = !policy_violated;
            let test_passed = actual_pass == case.expected_pass;

            results.push(TestCaseResult {
                test_name: case.name.clone(),
                policy_id: case.policy_id.clone(),
                passed: test_passed,
                expected: case.expected_pass,
                actual: actual_pass,
                message: if test_passed {
                    "OK".to_string()
                } else {
                    format!(
                        "Expected policy to {}, but it {}",
                        if case.expected_pass { "pass" } else { "fail" },
                        if actual_pass { "passed" } else { "failed" }
                    )
                },
            });
        }

        let passed = results.iter().filter(|r| r.passed).count();
        let failed = results.len() - passed;

        TestResults {
            total: results.len(),
            passed,
            failed,
            results,
        }
    }
}
