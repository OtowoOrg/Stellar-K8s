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
//! Security integration tests
//!
//! This module contains tests to verify that security hardening measures
//! are properly configured and working as expected.

#[cfg(test)]
mod tests {
    use std::process::Command;

    #[test]
    fn test_cargo_deny_passes() {
        let output = Command::new("cargo")
            .args(["deny", "check", "--format", "json"])
            .output();

        match output {
            Ok(output) => {
                if !output.status.success() {
                    // Print the actual error for debugging
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    let stdout = String::from_utf8_lossy(&output.stdout);

                    // cargo-deny not installed is acceptable in some test environments
                    if stderr.contains("no such command") || stderr.contains("not found") {
                        eprintln!("cargo-deny not installed, skipping test");
                        return;
                    }

                    panic!(
                        "cargo deny check failed:\nstdout: {}\nstderr: {}",
                        stdout, stderr
                    );
                }
            }
            Err(e) => {
                eprintln!("Failed to run cargo deny: {}. This is acceptable if cargo-deny is not installed.", e);
                // Don't fail the test if cargo-deny is not available
            }
        }
    }

    #[test]
    fn test_no_banned_dependencies() {
        // Verify that banned dependencies are not in Cargo.toml
        let cargo_toml = std::fs::read_to_string("Cargo.toml").expect("Could not read Cargo.toml");

        // Check for banned dependencies from deny.toml
        assert!(
            !cargo_toml.contains("openssl = "),
            "openssl dependency is banned, use rustls instead"
        );
        assert!(
            !cargo_toml.contains("openssl-sys = "),
            "openssl-sys dependency is banned, use rustls instead"
        );
    }

    #[test]
    fn test_security_hardened_profile_exists() {
        let cargo_toml = std::fs::read_to_string("Cargo.toml").expect("Could not read Cargo.toml");

        // Verify security-hardened build profiles exist
        assert!(
            cargo_toml.contains("[profile.release]"),
            "Release profile must be configured"
        );
        assert!(
            cargo_toml.contains("strip = true"),
            "Symbol stripping must be enabled"
        );
        assert!(
            cargo_toml.contains("panic = \"abort\""),
            "Panic abort must be configured"
        );
        assert!(
            cargo_toml.contains("lto = true"),
            "Link-time optimization must be enabled"
        );

        // Check for production profile
        assert!(
            cargo_toml.contains("[profile.production]"),
            "Production profile must exist"
        );
    }

    #[test]
    fn test_security_documentation_exists() {
        // Verify security documentation files exist
        assert!(
            std::path::Path::new("SECURITY.md").exists(),
            "SECURITY.md must exist"
        );
        assert!(
            std::path::Path::new("DEPENDENCY_SECURITY_AUDIT.md").exists(),
            "Security audit document must exist"
        );
        assert!(
            std::path::Path::new("deny.toml").exists(),
            "deny.toml configuration must exist"
        );
        assert!(
            std::path::Path::new(".cargo/audit.toml").exists(),
            "audit.toml configuration must exist"
        );
    }

    #[test]
    fn test_security_makefile_targets_exist() {
        let makefile = std::fs::read_to_string("Makefile").expect("Could not read Makefile");

        // Verify security targets exist
        assert!(
            makefile.contains("security-all:"),
            "security-all target must exist"
        );
        assert!(makefile.contains("audit:"), "audit target must exist");
        assert!(
            makefile.contains("security-report:"),
            "security-report target must exist"
        );
        assert!(
            makefile.contains("security-scan:"),
            "security-scan target must exist"
        );
    }

    #[test]
    fn test_precommit_security_hooks_configured() {
        let precommit = std::fs::read_to_string(".pre-commit-config.yaml")
            .expect("Could not read .pre-commit-config.yaml");

        // Verify security hooks are configured
        assert!(
            precommit.contains("cargo-deny"),
            "cargo-deny hook must be configured"
        );
        assert!(
            precommit.contains("cargo-audit"),
            "cargo-audit hook must be configured"
        );
        assert!(
            precommit.contains("security-sensitive"),
            "Security sensitive content check must be configured"
        );
    }

    #[test]
    fn test_security_ci_workflow_exists() {
        assert!(
            std::path::Path::new(".github/workflows/security-audit.yml").exists(),
            "Security audit CI workflow must exist"
        );
    }

    #[test]
    fn test_current_dependency_versions() {
        let cargo_toml = std::fs::read_to_string("Cargo.toml").expect("Could not read Cargo.toml");

        // Verify security-critical dependencies are updated
        // These should be the versions we updated for security patches
        assert!(
            cargo_toml.contains("anyhow = \"1.0.103\""),
            "anyhow should be pinned to a published secure version"
        );
        assert!(
            cargo_toml.contains("bytes = \"1.11.1\""),
            "bytes should be pinned to a published secure version"
        );
        // Verify security-critical dependency pins match Cargo.toml / Cargo.lock.
        // 1.0.108 was never published on crates.io; keep the locked secure pin.
        assert!(
            cargo_toml.contains("anyhow = \"1.0.103\""),
            "anyhow should be pinned to the secure Cargo.lock version"
        );
        assert!(
            cargo_toml.contains("bytes = \"1.11.1\""),
            "bytes should be pinned to the secure Cargo.lock version"
        );
    }
}
