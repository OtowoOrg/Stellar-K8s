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
//! `stellar-bootstrap-verify` — cross-platform developer bootstrap verifier.
//!
//! `scripts/preflight.sh` is a bash script, so it doesn't run on a plain
//! Windows machine (no WSL, no Git Bash). This binary is pure Rust — every
//! check runs through `std::process::Command` directly rather than a shell —
//! so it builds and runs unmodified on Linux, macOS, and Windows via `cargo
//! run --bin stellar-bootstrap-verify`, or via `make dev-setup` /
//! `make dev-setup-verify`.
//!
//! It checks: presence and version of every required local tool (docker,
//! kind, kubectl, helm, cargo, gh), the `rustc` version, whether the current
//! directory is a git work tree, and whether the Docker daemon is reachable.
//!
//! # Usage
//!
//! ```text
//! cargo run --bin stellar-bootstrap-verify
//! cargo run --bin stellar-bootstrap-verify -- --quiet
//! ```
//!
//! Exit code is `0` if every `Critical` check passed (`Warning` failures are
//! reported but do not fail the run), non-zero otherwise.

use std::process::ExitCode;

use clap::Parser;
use stellar_k8s::bootstrap_verify::run_bootstrap_verification;
use stellar_k8s::preflight::CheckSeverity;

#[derive(Parser, Debug)]
#[command(
    name = "stellar-bootstrap-verify",
    version,
    about = "Cross-platform developer bootstrap verifier — checks that this machine is ready to build and run Stellar-K8s"
)]
struct Args {
    /// Only print failing checks.
    #[arg(long)]
    quiet: bool,
}

fn main() -> ExitCode {
    let args = Args::parse();
    let results = run_bootstrap_verification();

    println!("=== Stellar-K8s Bootstrap Verification ===");
    for result in &results {
        if args.quiet && result.passed {
            continue;
        }
        let status = if result.passed { "PASS" } else { "FAIL" };
        println!("  [{status}] {} — {}", result.name, result.message);
    }

    let total = results.len();
    let passed = results.iter().filter(|r| r.passed).count();
    let critical_failures: Vec<_> = results
        .iter()
        .filter(|r| !r.passed && r.severity == CheckSeverity::Critical)
        .collect();

    println!(
        "=== {passed}/{total} checks passed, {} critical failure(s) ===",
        critical_failures.len()
    );

    if critical_failures.is_empty() {
        ExitCode::SUCCESS
    } else {
        eprintln!("\nBootstrap verification failed. Fix the critical issues above and re-run.");
        ExitCode::FAILURE
    }
}
