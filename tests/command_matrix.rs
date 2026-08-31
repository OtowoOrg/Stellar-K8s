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
//! Command matrix tests for all documented pipeline commands.
//!
//! These tests verify that each Makefile target compiles and runs successfully.
//! Run with: cargo test command_matrix -- --ignored

use std::process::Command;

fn run_make(target: &str) -> Result<String, String> {
    let output = Command::new("make")
        .arg(target)
        .env("K8S_OPENAPI_ENABLED_VERSION", "1.30")
        .output()
        .map_err(|e| format!("Failed to run make {}: {}", target, e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if !output.status.success() {
        return Err(format!(
            "make {} failed (exit {}):\nstdout: {}\nstderr: {}",
            target,
            output.status.code().unwrap_or(-1),
            stdout,
            stderr
        ));
    }
    Ok(stdout)
}

#[test]
#[ignore]
fn command_fmt_check() {
    run_make("fmt-check").expect("fmt-check must pass");
}

#[test]
#[ignore]
fn command_lint() {
    run_make("lint").expect("clippy lint must pass");
}

#[test]
#[ignore]
fn command_test() {
    run_make("test").expect("cargo test must pass");
}

#[test]
#[ignore]
fn command_build() {
    run_make("build").expect("release build must succeed");
    assert!(
        std::path::Path::new("target/release/stellar-operator").exists(),
        "Release binary target/release/stellar-operator must exist after build"
    );
}

#[test]
#[ignore]
fn command_quick() {
    run_make("quick").expect("quick check must pass");
}

#[test]
#[ignore]
fn command_shellcheck() {
    run_make("shellcheck").expect("shellcheck must pass");
}

#[test]
#[ignore]
fn command_helm_lint() {
    run_make("helm-lint").expect("helm lint must pass");
}

#[test]
#[ignore]
fn command_check_api_docs() {
    run_make("check-api-docs").expect("API docs check must pass");
}

#[test]
#[ignore]
fn command_completions() {
    run_make("completions").expect("completions generation must pass");
}

#[test]
#[ignore]
fn command_link_check() {
    run_make("link-check").expect("markdown link check must pass");
}

#[test]
#[ignore]
fn command_check_third_party_licenses() {
    run_make("check-third-party-licenses").expect("third-party license check must pass");
}
