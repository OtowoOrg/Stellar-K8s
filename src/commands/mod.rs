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
//! Subcommand implementations for the Stellar-K8s operator CLI.
//!
//! Each module corresponds to a major functional area of the operator's
//! command-line interface, such as running the operator, the simulator,
//! or generating runbooks.

pub mod backup;
pub mod benchmark;
pub mod check_crd;
pub mod doctor;
pub mod export_compliance;
pub mod health_check;
pub mod info;
pub mod operator;
pub mod runbook;
pub mod simulator;
pub mod webhook;
