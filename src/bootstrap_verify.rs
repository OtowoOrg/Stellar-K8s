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
//! Cross-platform developer bootstrap verifier
//!
//! `scripts/preflight.sh` is a bash script, so a Windows developer without
//! WSL or Git Bash has no equivalent way to check that their machine is
//! ready to build and run the operator. This module fills that gap: every
//! check goes through [`std::process::Command`] directly (never a shell), so
//! the exact same logic — and the `stellar-bootstrap-verify` binary built
//! from it — runs unmodified on Linux, macOS, and Windows. It is wired into
//! `make dev-setup` (via the `dev-setup-verify` target) as the final step,
//! so a fresh clone ends with a clear pass/fail report of the local
//! environment.
//!
//! Checks performed:
//! - Presence and reported version of every tool in
//!   [`crate::preflight::REQUIRED_LOCAL_TOOLS`] (docker, kind, kubectl, helm,
//!   cargo, gh).
//! - `rustc` version meets [`MIN_RUST_VERSION`].
//! - The current directory is inside a git work tree.
//! - The Docker daemon is reachable (best-effort; a `Warning`, not
//!   `Critical`, since some environments intentionally build without it).

use std::process::Command;

use crate::preflight::{CheckResult, CheckSeverity, REQUIRED_LOCAL_TOOLS};

/// Minimum supported Rust compiler version `(major, minor)`.
///
/// Kept in sync with the CI-enforced minimum in
/// `scripts/lib/versions.sh` (`RUST_TOOLCHAIN`) and the `lint` job's
/// pinned toolchain in `.github/workflows/ci.yml` — bump all three
/// together.
pub const MIN_RUST_VERSION: (u32, u32) = (1, 92);

/// Run the full cross-platform bootstrap verification suite.
///
/// Returns every check result, both passed and failed, so callers can print
/// a complete report rather than stopping at the first failure.
pub fn run_bootstrap_verification() -> Vec<CheckResult> {
    let mut results = Vec::with_capacity(REQUIRED_LOCAL_TOOLS.len() + 3);

    for (binary, hint) in REQUIRED_LOCAL_TOOLS {
        results.push(check_tool_version(binary, hint));
    }

    results.push(check_rustc_version());
    results.push(check_git_repository());
    results.push(check_docker_daemon());

    results
}

/// Check that `binary` is on `PATH` and capture the first line of its
/// `--version` output for the report.
fn check_tool_version(binary: &'static str, hint: &str) -> CheckResult {
    match Command::new(binary).arg("--version").output() {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Some tools (older kubectl) print version info to stderr.
            let line = if stdout.trim().is_empty() {
                stderr.lines().next()
            } else {
                stdout.lines().next()
            };
            CheckResult {
                name: binary,
                passed: true,
                severity: CheckSeverity::Critical,
                message: line.unwrap_or("(no version output)").trim().to_string(),
            }
        }
        Ok(output) => CheckResult {
            name: binary,
            passed: false,
            severity: CheckSeverity::Critical,
            message: format!(
                "`{binary} --version` exited with {} — {hint}",
                output.status
            ),
        },
        Err(_) => CheckResult {
            name: binary,
            passed: false,
            severity: CheckSeverity::Critical,
            message: format!("not found in PATH — {hint}"),
        },
    }
}

fn check_rustc_version() -> CheckResult {
    let output = match Command::new("rustc").arg("--version").output() {
        Ok(output) => output,
        Err(_) => {
            return CheckResult {
                name: "rustc-version",
                passed: false,
                severity: CheckSeverity::Critical,
                message: "rustc not found in PATH — install via https://rustup.rs/".to_string(),
            }
        }
    };

    if !output.status.success() {
        return CheckResult {
            name: "rustc-version",
            passed: false,
            severity: CheckSeverity::Critical,
            message: format!("`rustc --version` exited with {}", output.status),
        };
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    match parse_rustc_version(&stdout) {
        Some((major, minor)) if (major, minor) >= MIN_RUST_VERSION => CheckResult {
            name: "rustc-version",
            passed: true,
            severity: CheckSeverity::Critical,
            message: format!(
                "rustc {major}.{minor} (minimum {}.{})",
                MIN_RUST_VERSION.0, MIN_RUST_VERSION.1
            ),
        },
        Some((major, minor)) => CheckResult {
            name: "rustc-version",
            passed: false,
            severity: CheckSeverity::Critical,
            message: format!(
                "rustc {major}.{minor} is older than the minimum supported {}.{} — run `rustup update`",
                MIN_RUST_VERSION.0, MIN_RUST_VERSION.1
            ),
        },
        None => CheckResult {
            name: "rustc-version",
            passed: false,
            severity: CheckSeverity::Warning,
            message: format!("could not parse rustc version from: {}", stdout.trim()),
        },
    }
}

/// Parse `"rustc 1.78.0 (abcd1234 2024-05-02)"` into `(1, 78)`.
fn parse_rustc_version(text: &str) -> Option<(u32, u32)> {
    let version = text.split_whitespace().nth(1)?;
    let mut parts = version.split('.');
    let major: u32 = parts.next()?.parse().ok()?;
    let minor: u32 = parts.next()?.parse().ok()?;
    Some((major, minor))
}

fn check_git_repository() -> CheckResult {
    match Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
    {
        Ok(output) if output.status.success() => {
            let inside = String::from_utf8_lossy(&output.stdout).trim() == "true";
            if inside {
                CheckResult {
                    name: "git-repository",
                    passed: true,
                    severity: CheckSeverity::Warning,
                    message: "running inside a git work tree".to_string(),
                }
            } else {
                CheckResult {
                    name: "git-repository",
                    passed: false,
                    severity: CheckSeverity::Warning,
                    message: "not inside a git work tree".to_string(),
                }
            }
        }
        Ok(_) => CheckResult {
            name: "git-repository",
            passed: false,
            severity: CheckSeverity::Warning,
            message:
                "not inside a git work tree — clone with `git clone` rather than a zip download"
                    .to_string(),
        },
        Err(_) => CheckResult {
            name: "git-repository",
            passed: false,
            severity: CheckSeverity::Critical,
            message: "git not found in PATH — install from https://git-scm.com/downloads"
                .to_string(),
        },
    }
}

/// Best-effort check that the Docker daemon is reachable. Not `Critical`
/// because some contributors intentionally build/test without Docker
/// running (e.g. `cargo check` on a laptop with Docker Desktop asleep).
fn check_docker_daemon() -> CheckResult {
    match Command::new("docker").arg("info").output() {
        Ok(output) if output.status.success() => CheckResult {
            name: "docker-daemon",
            passed: true,
            severity: CheckSeverity::Warning,
            message: "Docker daemon is reachable".to_string(),
        },
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let reason = stderr.lines().next().unwrap_or("unknown error").trim();
            CheckResult {
                name: "docker-daemon",
                passed: false,
                severity: CheckSeverity::Warning,
                message: format!("Docker CLI present but daemon unreachable: {reason}"),
            }
        }
        Err(_) => CheckResult {
            name: "docker-daemon",
            passed: false,
            severity: CheckSeverity::Warning,
            message: "docker not found in PATH".to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_standard_rustc_version_string() {
        assert_eq!(
            parse_rustc_version("rustc 1.78.0 (9b00956e5 2024-04-29)"),
            Some((1, 78))
        );
    }

    #[test]
    fn parses_version_with_extra_channel_suffix() {
        assert_eq!(
            parse_rustc_version("rustc 1.80.1-nightly (abcdef123 2024-08-01)"),
            Some((1, 80))
        );
    }

    #[test]
    fn returns_none_for_unparseable_input() {
        assert_eq!(parse_rustc_version("not a version string"), None);
        assert_eq!(parse_rustc_version(""), None);
    }

    #[test]
    fn run_bootstrap_verification_covers_every_required_tool_plus_extra_checks() {
        let results = run_bootstrap_verification();
        assert_eq!(results.len(), REQUIRED_LOCAL_TOOLS.len() + 3);

        let names: Vec<&str> = results.iter().map(|r| r.name).collect();
        assert!(names.contains(&"rustc-version"));
        assert!(names.contains(&"git-repository"));
        assert!(names.contains(&"docker-daemon"));
        for (tool, _) in REQUIRED_LOCAL_TOOLS {
            assert!(names.contains(tool));
        }
    }

    #[test]
    fn missing_binary_reports_critical_failure_with_hint() {
        let result = check_tool_version("definitely-not-a-real-binary", "install it from nowhere");
        assert!(!result.passed);
        assert_eq!(result.severity, CheckSeverity::Critical);
        assert!(result.message.contains("install it from nowhere"));
    }
}
