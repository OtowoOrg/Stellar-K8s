// Copyright 2026 Stellar-K8s Contributors
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
//! Release reproducibility verifier (#1997).
//!
//! Compares a release build against an independent rebuild from the same
//! source revision, bit for bit. Each build directory holds the artifacts and a
//! `build-manifest.json` written by `scripts/reproducible-build.sh`:
//!
//! ```json
//! {
//!   "revision": "<git sha>",
//!   "pipeline": { "name": "release", "runner": "ubuntu-24.04" },
//!   "steps": [{ "name": "cargo-build", "env": { "SOURCE_DATE_EPOCH": "..." },
//!               "artifacts": ["stellar-operator"] }],
//!   "artifacts": [{ "name": "stellar-operator", "kind": "binary",
//!                   "path": "stellar-operator", "sha256": "...", "len": 123 }]
//! }
//! ```
//!
//! A mismatch is reported per artifact with the build step that produced it,
//! the byte offset of the first difference and the non-determinism sources
//! detected in the differing bytes. Every verification is appended to an audit
//! log (one JSON line per release) so an audit can see which releases are
//! reproducible.
//!
//! Exit codes: `0` verified, `1` below the reproducibility threshold (or not
//! fully reproducible with `--strict`), `2` the verification could not run.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use chrono::Utc;
use clap::Parser;
use serde::{Deserialize, Serialize};

use stellar_k8s::reproducible_build::{
    assert_independent, sha256_hex, verify_release, ArtifactFingerprint, ArtifactKind,
    ArtifactOutput, BuildStep, Pipeline, ReleaseVerification, ReproducibilityVerdict,
};

/// File name of the manifest inside a build directory.
const MANIFEST_NAME: &str = "build-manifest.json";

#[derive(Debug, Deserialize)]
struct Manifest {
    revision: String,
    pipeline: PipelineInfo,
    #[serde(default)]
    steps: Vec<StepInfo>,
    artifacts: Vec<ArtifactInfo>,
}

#[derive(Debug, Deserialize)]
struct PipelineInfo {
    name: String,
    runner: String,
}

#[derive(Debug, Deserialize)]
struct StepInfo {
    name: String,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    artifacts: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ArtifactInfo {
    name: String,
    kind: ArtifactKind,
    /// Path relative to the build directory.
    path: String,
    sha256: String,
    len: usize,
}

/// One line of the release audit log.
#[derive(Debug, Serialize)]
struct AuditRecord {
    recorded_at: String,
    release: String,
    revision: String,
    verdict: ReproducibilityVerdict,
    artifacts_checked: usize,
    reproducible: usize,
    reproducible_rate: f64,
    implicated_steps: Vec<String>,
    release_pipeline: String,
    rebuild_pipeline: String,
    /// SHA-256 of the full report JSON written alongside, tying the audit line
    /// to the detailed evidence.
    report_sha256: String,
}

#[derive(Debug, Parser)]
#[command(
    name = "verify-reproducible-build",
    about = "Verify a release rebuilds bit-for-bit and record the result in the release audit log"
)]
struct Cli {
    /// Release being verified, e.g. v2.6.0.
    #[arg(long)]
    release: String,
    /// Directory with the release build's artifacts and build-manifest.json.
    #[arg(long)]
    release_dir: PathBuf,
    /// Directory with the independent rebuild's artifacts and build-manifest.json.
    #[arg(long)]
    rebuild_dir: PathBuf,
    /// Audit log to append to (one JSON line per verification).
    #[arg(long, default_value = "release-audit/reproducibility.jsonl")]
    audit_log: PathBuf,
    /// Write the full verification report as JSON.
    #[arg(long)]
    report_json: Option<PathBuf>,
    /// Write the verification report as Markdown.
    #[arg(long)]
    report_md: Option<PathBuf>,
    /// Fail unless every artifact is identical, not just above the 95% bar.
    #[arg(long)]
    strict: bool,
}

fn read_manifest(dir: &Path) -> Result<Manifest, String> {
    let path = dir.join(MANIFEST_NAME);
    let json = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    serde_json::from_str(&json).map_err(|e| format!("invalid {}: {e}", path.display()))
}

/// Load every artifact in `manifest` from `dir`.
///
/// A file whose content does not match the digest in its manifest is an error:
/// the manifest and the bytes must describe the same build. With
/// `require_bytes` false a missing file is tolerated and the manifest digest is
/// trusted, which lets the release side be checked from its manifest alone.
fn load_outputs(
    dir: &Path,
    manifest: &Manifest,
    require_bytes: bool,
) -> Result<Vec<ArtifactOutput>, String> {
    let mut outputs = Vec::new();
    for artifact in &manifest.artifacts {
        let path = dir.join(&artifact.path);
        match std::fs::read(&path) {
            Ok(bytes) => {
                let digest = sha256_hex(&bytes);
                if digest != artifact.sha256 {
                    return Err(format!(
                        "{}: manifest says sha256 {} but the file hashes to {digest}",
                        path.display(),
                        artifact.sha256
                    ));
                }
                outputs.push(ArtifactOutput::new(&artifact.name, artifact.kind, bytes));
            }
            Err(e) if !require_bytes && e.kind() == std::io::ErrorKind::NotFound => {
                outputs.push(ArtifactOutput {
                    name: artifact.name.clone(),
                    kind: artifact.kind,
                    digest: artifact.sha256.clone(),
                    len: artifact.len,
                    bytes: Vec::new(),
                });
            }
            Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
        }
    }
    Ok(outputs)
}

/// Build steps with the artifacts each one emitted, from the release build.
fn build_steps(manifest: &Manifest, outputs: &[ArtifactOutput]) -> Vec<BuildStep> {
    manifest
        .steps
        .iter()
        .map(|info| {
            let mut step = BuildStep::new(&info.name);
            for (key, value) in &info.env {
                step = step.with_env(key, value);
            }
            for name in &info.artifacts {
                if let Some(output) = outputs.iter().find(|o| &o.name == name) {
                    step = step.with_output(output.clone());
                }
            }
            step
        })
        .collect()
}

fn verify(cli: &Cli) -> Result<(ReleaseVerification, AuditRecord, String), String> {
    let release_manifest = read_manifest(&cli.release_dir)?;
    let rebuild_manifest = read_manifest(&cli.rebuild_dir)?;

    assert_independent(
        &Pipeline::release(
            &release_manifest.pipeline.name,
            &release_manifest.pipeline.runner,
            &release_manifest.revision,
        ),
        &Pipeline::rebuild(
            &rebuild_manifest.pipeline.name,
            &rebuild_manifest.pipeline.runner,
            &rebuild_manifest.revision,
        ),
    )?;

    let release_outputs = load_outputs(&cli.release_dir, &release_manifest, false)?;
    let rebuild_outputs = load_outputs(&cli.rebuild_dir, &rebuild_manifest, true)?;
    let steps = build_steps(&release_manifest, &release_outputs);
    let release_fingerprints: Vec<ArtifactFingerprint> = release_outputs
        .iter()
        .map(ArtifactOutput::fingerprint)
        .collect();

    let report = verify_release(&cli.release, &steps, release_fingerprints, rebuild_outputs)?;
    let report_json = report.to_json();
    let record = AuditRecord {
        recorded_at: Utc::now().to_rfc3339(),
        release: cli.release.clone(),
        revision: release_manifest.revision.clone(),
        verdict: report.verdict,
        artifacts_checked: report.artifacts_checked,
        reproducible: report.reproducible,
        reproducible_rate: report.reproducible_rate,
        implicated_steps: report.implicated_steps.clone(),
        release_pipeline: release_manifest.pipeline.name.clone(),
        rebuild_pipeline: rebuild_manifest.pipeline.name.clone(),
        report_sha256: sha256_hex(report_json.as_bytes()),
    };
    Ok((report, record, report_json))
}

fn append_audit(path: &Path, record: &AuditRecord) -> Result<(), String> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    let line = serde_json::to_string(record).map_err(|e| e.to_string())?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    writeln!(file, "{line}").map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// Verify, record and report. `Ok(true)` means the release passed.
fn run(cli: &Cli) -> Result<bool, String> {
    let (report, record, report_json) = verify(cli)?;

    println!("{}", report.badge());
    for mismatch in &report.mismatches {
        println!("  MISMATCH {}", mismatch.detail);
    }

    // The audit line is written before the exit code is decided, so a failed
    // verification is recorded as well as a passing one.
    append_audit(&cli.audit_log, &record)?;
    if let Some(path) = &cli.report_json {
        std::fs::write(path, &report_json)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }
    if let Some(path) = &cli.report_md {
        std::fs::write(path, report.to_markdown())
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }

    Ok(match report.verdict {
        ReproducibilityVerdict::Reproducible => true,
        ReproducibilityVerdict::MostlyReproducible => report.meets_threshold() && !cli.strict,
        ReproducibilityVerdict::NonReproducible => false,
    })
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(message) => {
            eprintln!("verify-reproducible-build: {message}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_build(
        dir: &Path,
        pipeline: &str,
        revision: &str,
        files: &[(&str, &str)],
        env: &[(&str, &str)],
    ) {
        fs::create_dir_all(dir).unwrap();
        let artifacts: Vec<serde_json::Value> = files
            .iter()
            .map(|(name, bytes)| {
                fs::write(dir.join(name), bytes.as_bytes()).unwrap();
                serde_json::json!({
                    "name": name, "kind": "binary", "path": name,
                    "sha256": sha256_hex(bytes.as_bytes()), "len": bytes.len(),
                })
            })
            .collect();
        let env: serde_json::Map<String, serde_json::Value> = env
            .iter()
            .map(|(k, v)| (k.to_string(), serde_json::Value::String(v.to_string())))
            .collect();
        let names: Vec<&str> = files.iter().map(|(n, _)| *n).collect();
        let manifest = serde_json::json!({
            "revision": revision,
            "pipeline": { "name": pipeline, "runner": "test-runner" },
            "steps": [{ "name": "cargo-build", "env": env, "artifacts": names }],
            "artifacts": artifacts,
        });
        fs::write(dir.join(MANIFEST_NAME), manifest.to_string()).unwrap();
    }

    fn cli(root: &Path) -> Cli {
        Cli {
            release: "v1.0.0".to_string(),
            release_dir: root.join("release"),
            rebuild_dir: root.join("rebuild"),
            audit_log: root.join("audit/reproducibility.jsonl"),
            report_json: None,
            report_md: None,
            strict: false,
        }
    }

    const PINNED: &[(&str, &str)] = &[
        ("SOURCE_DATE_EPOCH", "1700000000"),
        ("LC_ALL", "C"),
        ("RUSTFLAGS", "--remap-path-prefix=/build=/src"),
    ];

    #[test]
    fn identical_builds_are_reproducible_and_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let bytes = "ELF identical bytes";
        write_build(
            &root.join("release"),
            "release",
            "abc123",
            &[("op", bytes)],
            PINNED,
        );
        write_build(
            &root.join("rebuild"),
            "rebuild",
            "abc123",
            &[("op", bytes)],
            PINNED,
        );

        let cli = cli(root);
        assert!(run(&cli).unwrap());

        let log = fs::read_to_string(&cli.audit_log).unwrap();
        let record: serde_json::Value = serde_json::from_str(log.lines().next().unwrap()).unwrap();
        assert_eq!(record["release"], "v1.0.0");
        assert_eq!(record["verdict"], "reproducible");
        assert_eq!(record["revision"], "abc123");
        assert_eq!(record["reproducible"], 1);
    }

    #[test]
    fn a_differing_artifact_is_localized_and_fails() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_build(
            &root.join("release"),
            "release",
            "abc123",
            &[("op", "built 2026-01-02T03:04:05Z")],
            &[],
        );
        write_build(
            &root.join("rebuild"),
            "rebuild",
            "abc123",
            &[("op", "built 2026-02-03T04:05:06Z")],
            &[],
        );

        let cli = cli(root);
        let (report, record, _) = verify(&cli).unwrap();

        assert_eq!(report.verdict, ReproducibilityVerdict::NonReproducible);
        assert_eq!(report.mismatches.len(), 1);
        assert_eq!(report.mismatches[0].artifact, "op");
        assert_eq!(report.mismatches[0].step, "cargo-build");
        assert_eq!(report.mismatches[0].first_diff_offset, Some(12));
        assert_eq!(record.implicated_steps, vec!["cargo-build".to_string()]);
        assert!(!run(&cli).unwrap());

        // The failure is audited too.
        let log = fs::read_to_string(&cli.audit_log).unwrap();
        assert!(log.contains("non_reproducible"));
    }

    #[test]
    fn audit_lines_accumulate_across_releases() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let bytes = "same";
        write_build(
            &root.join("release"),
            "release",
            "abc",
            &[("op", bytes)],
            PINNED,
        );
        write_build(
            &root.join("rebuild"),
            "rebuild",
            "abc",
            &[("op", bytes)],
            PINNED,
        );

        let mut cli = cli(root);
        run(&cli).unwrap();
        cli.release = "v1.0.1".to_string();
        run(&cli).unwrap();

        assert_eq!(
            fs::read_to_string(&cli.audit_log).unwrap().lines().count(),
            2
        );
    }

    #[test]
    fn a_rebuild_of_a_different_revision_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let bytes = "same";
        write_build(
            &root.join("release"),
            "release",
            "abc",
            &[("op", bytes)],
            PINNED,
        );
        write_build(
            &root.join("rebuild"),
            "rebuild",
            "def",
            &[("op", bytes)],
            PINNED,
        );

        let err = verify(&cli(root)).unwrap_err();
        assert!(err.contains("revision"), "{err}");
    }

    #[test]
    fn a_rebuild_that_reuses_the_release_pipeline_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let bytes = "same";
        write_build(
            &root.join("release"),
            "release",
            "abc",
            &[("op", bytes)],
            PINNED,
        );
        write_build(
            &root.join("rebuild"),
            "release",
            "abc",
            &[("op", bytes)],
            PINNED,
        );

        assert!(verify(&cli(root)).is_err());
    }

    #[test]
    fn a_file_that_does_not_match_its_manifest_digest_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_build(
            &root.join("release"),
            "release",
            "abc",
            &[("op", "good")],
            PINNED,
        );
        write_build(
            &root.join("rebuild"),
            "rebuild",
            "abc",
            &[("op", "good")],
            PINNED,
        );
        fs::write(root.join("rebuild/op"), "tampered").unwrap();

        let err = verify(&cli(root)).unwrap_err();
        assert!(err.contains("hashes to"), "{err}");
    }

    #[test]
    fn a_missing_rebuild_artifact_is_reported_as_missing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_build(
            &root.join("release"),
            "release",
            "abc",
            &[("op", "a"), ("cli", "b")],
            PINNED,
        );
        write_build(
            &root.join("rebuild"),
            "rebuild",
            "abc",
            &[("op", "a")],
            PINNED,
        );

        let (report, _, _) = verify(&cli(root)).unwrap();
        assert_eq!(report.mismatches.len(), 1);
        assert_eq!(report.mismatches[0].artifact, "cli");
    }

    #[test]
    fn the_release_side_can_be_checked_from_its_manifest_alone() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let bytes = "same";
        write_build(
            &root.join("release"),
            "release",
            "abc",
            &[("op", bytes)],
            PINNED,
        );
        write_build(
            &root.join("rebuild"),
            "rebuild",
            "abc",
            &[("op", bytes)],
            PINNED,
        );
        fs::remove_file(root.join("release/op")).unwrap();

        let (report, _, _) = verify(&cli(root)).unwrap();
        assert_eq!(report.verdict, ReproducibilityVerdict::Reproducible);
    }
}
