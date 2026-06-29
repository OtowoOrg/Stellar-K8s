//! Repository validation health-check command.
//!
//! Runs a set of lightweight, offline repository checks that can be performed
//! without a live Kubernetes cluster.  Designed to surface common issues before
//! a contributor opens a PR.
//!
//! Checks performed:
//!   1. Required files present (README, LICENSE, CONTRIBUTING, CHANGELOG, Makefile)
//!   2. Cargo.toml version matches git tag (when HEAD is a tag)
//!   3. Dependency update template present (.github/ISSUE_TEMPLATE/dependency_update.yml)
//!   4. Release process doc present (docs/release-process.md)
//!   5. No uncommitted changes in tracked files (optional; skip with --allow-dirty)

use std::fs;
use std::process::Command;
use stellar_k8s::Error;

/// Arguments for the `health-check` subcommand.
#[derive(clap::Parser, Debug)]
#[command(about = "Run repository validation checks (offline)")]
pub struct HealthCheckArgs {
    /// Allow dirty working tree (skip uncommitted-changes check).
    #[arg(long)]
    pub allow_dirty: bool,

    /// Emit results as JSON (one object per line).
    #[arg(long)]
    pub json: bool,
}

struct Check {
    name: &'static str,
    passed: bool,
    message: String,
}

impl Check {
    fn pass(name: &'static str, msg: impl Into<String>) -> Self {
        Self { name, passed: true, message: msg.into() }
    }
    fn fail(name: &'static str, msg: impl Into<String>) -> Self {
        Self { name, passed: false, message: msg.into() }
    }
}

fn check_file(name: &'static str, path: &str) -> Check {
    if fs::metadata(path).is_ok() {
        Check::pass(name, format!("{path} found"))
    } else {
        Check::fail(name, format!("{path} is missing"))
    }
}

fn check_cargo_version_matches_tag() -> Check {
    // Read version from Cargo.toml
    let cargo_toml = match fs::read_to_string("Cargo.toml") {
        Ok(c) => c,
        Err(e) => return Check::fail("Cargo version / tag", format!("cannot read Cargo.toml: {e}")),
    };

    let version = cargo_toml
        .lines()
        .find(|l| l.trim_start().starts_with("version ="))
        .and_then(|l| l.split('"').nth(1))
        .map(|v| v.to_string());

    let Some(version) = version else {
        return Check::fail("Cargo version / tag", "version field not found in Cargo.toml");
    };

    // Get the tag pointing at HEAD (if any)
    let output = Command::new("git")
        .args(["tag", "--points-at", "HEAD"])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let tags = String::from_utf8_lossy(&out.stdout);
            let matching_tag = tags
                .lines()
                .filter(|t| !t.is_empty())
                .find(|t| t.trim_start_matches('v') == version);

            if tags.lines().all(|l| l.is_empty()) {
                // HEAD is not tagged — nothing to validate against
                Check::pass("Cargo version / tag", format!("Cargo.toml version = {version} (HEAD is not tagged)"))
            } else if matching_tag.is_some() {
                Check::pass("Cargo version / tag", format!("v{version} matches HEAD tag"))
            } else {
                let existing: Vec<_> = tags.lines().filter(|l| !l.is_empty()).collect();
                Check::fail(
                    "Cargo version / tag",
                    format!("Cargo.toml={version} does not match HEAD tags: {existing:?}"),
                )
            }
        }
        Ok(_) | Err(_) => {
            // git not available or error — skip
            Check::pass("Cargo version / tag", format!("Cargo.toml version = {version} (git unavailable, skipped tag check)"))
        }
    }
}

fn check_git_clean() -> Check {
    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let dirty: Vec<_> = String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter(|l| !l.is_empty())
                .map(|l| l.to_string())
                .collect();
            if dirty.is_empty() {
                Check::pass("Working tree", "no uncommitted changes")
            } else {
                Check::fail(
                    "Working tree",
                    format!("{} uncommitted change(s): {}", dirty.len(), dirty.join(", ")),
                )
            }
        }
        _ => Check::pass("Working tree", "git unavailable — skipped"),
    }
}

fn print_check(check: &Check, json: bool) {
    if json {
        let status = if check.passed { "pass" } else { "fail" };
        println!(
            r#"{{"check":"{name}","status":"{status}","message":"{msg}"}}"#,
            name = check.name,
            msg = check.message.replace('"', r#"\""#),
        );
    } else {
        let icon = if check.passed { "✓" } else { "✗" };
        println!("  [{icon}] {name}: {msg}", name = check.name, msg = check.message);
    }
}

pub fn run_health_check(args: HealthCheckArgs) -> Result<(), Error> {
    if !args.json {
        println!("=== Stellar-K8s Repository Health Check ===");
        println!();
    }

    let mut checks = vec![
        // Required repo files
        check_file("README", "README.md"),
        check_file("LICENSE", "LICENSE"),
        check_file("CONTRIBUTING", "CONTRIBUTING.md"),
        check_file("CHANGELOG", "CHANGELOG.md"),
        check_file("Makefile", "Makefile"),
        // Hygiene files added by the repo-hygiene wave
        check_file(
            "Dependency update template",
            ".github/ISSUE_TEMPLATE/dependency_update.yml",
        ),
        check_file("Release process doc", "docs/release-process.md"),
        // Version consistency
        check_cargo_version_matches_tag(),
    ];

    if !args.allow_dirty {
        checks.push(check_git_clean());
    }

    let passed = checks.iter().filter(|c| c.passed).count();
    let total = checks.len();

    for check in &checks {
        print_check(check, args.json);
    }

    if !args.json {
        println!();
        println!("SUMMARY: {passed}/{total} checks passed");
    }

    if checks.iter().all(|c| c.passed) {
        Ok(())
    } else {
        Err(Error::ConfigError(format!(
            "{} of {total} health-check(s) failed",
            total - passed
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_file_pass_for_existing_file() {
        // Cargo.toml always exists at repo root
        let c = check_file("Cargo.toml", "Cargo.toml");
        assert!(c.passed);
    }

    #[test]
    fn check_file_fail_for_missing_file() {
        let c = check_file("Missing", "this/file/does/not/exist.txt");
        assert!(!c.passed);
    }
}
