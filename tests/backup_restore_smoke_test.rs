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
// tests/backup_restore_smoke_test.rs
// Command-level smoke tests for backup and restore CLI commands.
// These tests validate end-to-end behavior using assert-cmd.
// Related: #1149 - Add command-level smoke tests for backup and restore workflows

use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn test_backup_help_exits_successfully() {
    Command::cargo_bin("stellar-operator")
        .unwrap()
        .args(["backup", "--help"])
        .assert()
        .success();
}

#[test]
fn test_restore_help_exits_successfully() {
    Command::cargo_bin("stellar-operator")
        .unwrap()
        .args(["backup", "restore", "--help"])
        .assert()
        .success();
}

#[test]
fn test_backup_list_help_exits_successfully() {
    Command::cargo_bin("stellar-operator")
        .unwrap()
        .args(["backup", "list", "--help"])
        .assert()
        .success();
}

#[test]
fn test_restore_help_documents_destination() {
    Command::cargo_bin("stellar-operator")
        .unwrap()
        .args(["backup", "restore", "--help"])
        .assert()
        .success()
        .stdout(
            predicates::str::contains("--destination").or(predicates::str::contains("DESTINATION")),
        );
}

#[test]
fn test_backup_create_help_exits_successfully() {
    Command::cargo_bin("stellar-operator")
        .unwrap()
        .args(["backup", "create", "--help"])
        .assert()
        .success();
}
