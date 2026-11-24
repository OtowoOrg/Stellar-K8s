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
//! Deterministic build reproducibility verification for all artifacts
//! (#1518).
//!
//! "Rebuild from source produces the same bits" used to be an aspiration. This
//! module makes it a checkable property:
//!
//! - The rebuild runs in its own pipeline, with its own inputs and its own
//!   environment, so it is independent of the release pipeline that produced
//!   the artifact.
//! - Outputs are compared **bit for bit** (SHA-256 plus a byte-level first
//!   difference offset), not by version or timestamp.
//! - When two outputs differ, the mismatch is localized: the verifier walks
//!   the build steps, finds the first step that emits the differing artifact,
//!   and reports the non-determinism sources detected in those bytes.
//! - A per-release status is recorded, including a badge, so an audit can
//!   see which releases are reproducible.
//!
//! ```
//! use stellar_k8s::reproducible_build::*;
//!
//! let bytes = vec![0x7f, 0x45, 0x4c, 0x46];
//! let steps = vec![BuildStep::new("cargo-build")
//!     .with_env("SOURCE_DATE_EPOCH", "1700000000")
//!     .with_output(ArtifactOutput::new("stellar-operator", ArtifactKind::Binary, bytes))];
//! let release: Vec<ArtifactFingerprint> = steps[0].outputs.iter().map(ArtifactOutput::fingerprint).collect();
//! let rebuild: Vec<ArtifactOutput> = steps[0].outputs.clone();
//!
//! let report = verify_release("v2.6.0", &steps, release, rebuild).unwrap();
//! assert_eq!(report.verdict, ReproducibilityVerdict::Reproducible);
//! assert!(report.badge().contains("reproducible"));
//! ```

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{info, warn};

/// Minimum share of artifacts that must rebuild bit-for-bit identical for a
/// release to be considered reproducible.
pub const REQUIRED_REPRODUCIBLE_RATE: f64 = 0.95;

// ---------------------------------------------------------------------------
// Artifacts
// ---------------------------------------------------------------------------

/// Kind of released artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Binary,
    ContainerImage,
    HelmChart,
    CrdBundle,
    OperatorBundle,
    Sbom,
}

impl std::fmt::Display for ArtifactKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ArtifactKind::Binary => "binary",
            ArtifactKind::ContainerImage => "container_image",
            ArtifactKind::HelmChart => "helm_chart",
            ArtifactKind::CrdBundle => "crd_bundle",
            ArtifactKind::OperatorBundle => "operator_bundle",
            ArtifactKind::Sbom => "sbom",
        };
        f.write_str(s)
    }
}

/// An artifact emitted by a build step, together with its bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactOutput {
    pub name: String,
    pub kind: ArtifactKind,
    /// SHA-256 hex of the bytes.
    pub digest: String,
    pub len: usize,
    #[serde(skip)]
    pub bytes: Vec<u8>,
}

impl ArtifactOutput {
    pub fn new(name: &str, kind: ArtifactKind, bytes: Vec<u8>) -> Self {
        let digest = sha256_hex(&bytes);
        let len = bytes.len();
        Self {
            name: name.to_string(),
            kind,
            digest,
            len,
            bytes,
        }
    }

    /// The fingerprint used for bit-for-bit comparison.
    pub fn fingerprint(&self) -> ArtifactFingerprint {
        ArtifactFingerprint {
            name: self.name.clone(),
            kind: self.kind,
            digest: self.digest.clone(),
            len: self.len,
        }
    }
}

/// The comparable identity of an artifact: name, kind, size, and content hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactFingerprint {
    pub name: String,
    pub kind: ArtifactKind,
    pub digest: String,
    pub len: usize,
}

impl ArtifactFingerprint {
    pub fn from_output(output: &ArtifactOutput) -> Self {
        output.fingerprint()
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

// ---------------------------------------------------------------------------
// Non-determinism detection
// ---------------------------------------------------------------------------

/// A known source of build non-determinism.
///
/// Detection is content based: the verifier scans the differing bytes and the
/// build step's environment for the markers each source leaves behind, so a
/// mismatch is attributed to a concrete cause rather than "it just differs".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NondeterminismSource {
    /// Embedded build timestamps (`2026-01-02T03:04:05Z`, RFC 2822, `SOURCE_DATE_EPOCH` leftovers).
    Timestamps,
    /// Absolute build-machine paths leaked into the output.
    PathPrefix,
    /// Locale-dependent formatting (`LANG=de_DE.UTF-8`).
    Locale,
    /// Version-control metadata (`vcs.revision`, `git rev-parse`).
    VcsMetadata,
    /// Linker build-id / debug directory.
    BuildId,
    /// Archive metadata (gzip/zip mtimes, tar uid/gid).
    ArchiveMetadata,
    /// Mixed line endings.
    LineEndings,
    /// Randomized seed embedded in the output.
    RandomSeed,
}

impl std::fmt::Display for NondeterminismSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            NondeterminismSource::Timestamps => "timestamps",
            NondeterminismSource::PathPrefix => "path_prefix",
            NondeterminismSource::Locale => "locale",
            NondeterminismSource::VcsMetadata => "vcs_metadata",
            NondeterminismSource::BuildId => "build_id",
            NondeterminismSource::ArchiveMetadata => "archive_metadata",
            NondeterminismSource::LineEndings => "line_endings",
            NondeterminismSource::RandomSeed => "random_seed",
        };
        f.write_str(s)
    }
}

/// Scan artifact bytes and report every non-determinism source detected.
pub fn detect_nondeterminism(bytes: &[u8]) -> Vec<NondeterminismSource> {
    let mut found: Vec<NondeterminismSource> = Vec::new();
    let mut push = |s: NondeterminismSource| {
        if !found.contains(&s) {
            found.push(s);
        }
    };

    let text = String::from_utf8_lossy(bytes);
    let is_match = |re: &str| Regex::new(re).expect("static regex").is_match(&text);

    if is_match(r"\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}")
        || is_match(r"[A-Z][a-z]{2}, \d{2} [A-Z][a-z]{2} \d{4} \d{2}:\d{2}:\d{2}")
    {
        push(NondeterminismSource::Timestamps);
    }

    if is_match(r"[A-Za-z]:\\\\?[A-Za-z0-9_\\.-]{3,}")
        || is_match(r"/(home|Users|builds?)/[A-Za-z0-9_.-]+/")
    {
        push(NondeterminismSource::PathPrefix);
    }

    if is_match(r"(LANG|LC_ALL|LC_NUMERIC)=[a-z]{2,3}_[A-Z]{2}") {
        push(NondeterminismSource::Locale);
    }

    if is_match(r"(vcs\.revision|git rev-parse|commit=[0-9a-f]{7,40})") {
        push(NondeterminismSource::VcsMetadata);
    }

    if is_match(r"build[-_ ]?id[\s:=]+[0-9a-f]{8,}") {
        push(NondeterminismSource::BuildId);
    }

    if is_match(r"(random[_-]?seed|SEED)=\s*0x[0-9a-f]+") {
        push(NondeterminismSource::RandomSeed);
    }

    if has_archive_metadata(bytes) {
        push(NondeterminismSource::ArchiveMetadata);
    }
    if has_mixed_line_endings(bytes) {
        push(NondeterminismSource::LineEndings);
    }

    found
}

/// A gzip member with a non-zero MTIME field, or a zip/tar with timestamps.
fn has_archive_metadata(bytes: &[u8]) -> bool {
    // gzip: 1f 8b 08 <flags> <mtime:4 little-endian>
    if bytes.len() > 8 && bytes[0] == 0x1f && bytes[1] == 0x8b && bytes[2] == 0x08 {
        let mtime = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        if mtime != 0 {
            return true;
        }
    }
    // zip local file header: PK\x03\x04 with a non-zero dos time/date
    if bytes.len() > 30
        && bytes[0] == 0x50
        && bytes[1] == 0x4b
        && bytes[2] == 0x03
        && bytes[3] == 0x04
    {
        let dos_time = u16::from_le_bytes([bytes[10], bytes[11]]);
        let dos_date = u16::from_le_bytes([bytes[12], bytes[13]]);
        if dos_time != 0 || dos_date != 0 {
            return true;
        }
    }
    false
}

fn has_mixed_line_endings(bytes: &[u8]) -> bool {
    let mut crlf = 0usize;
    let mut lf = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            if i > 0 && bytes[i - 1] == b'\r' {
                crlf += 1;
            } else {
                lf += 1;
            }
        }
        i += 1;
    }
    crlf > 0 && lf > 0
}

// ---------------------------------------------------------------------------
// Build steps and pipelines
// ---------------------------------------------------------------------------

/// One step of a build, with the artifacts it emits.
///
/// Steps carry the build knobs that are supposed to pin the output
/// (`SOURCE_DATE_EPOCH`, `LC_ALL`, ...). If those knobs are missing the step
/// is a candidate source of non-determinism, which the localizer reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildStep {
    pub name: String,
    /// Build environment as seen by the step.
    pub env: BTreeMap<String, String>,
    pub outputs: Vec<ArtifactOutput>,
}

impl BuildStep {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            env: BTreeMap::new(),
            outputs: Vec::new(),
        }
    }

    pub fn with_env(mut self, key: &str, value: &str) -> Self {
        self.env.insert(key.to_string(), value.to_string());
        self
    }

    pub fn with_output(mut self, output: ArtifactOutput) -> Self {
        self.outputs.push(output);
        self
    }

    /// Build knobs that should be pinned to make the step deterministic.
    pub fn determinism_flags(&self) -> DeterminismFlags {
        DeterminismFlags {
            source_date_epoch: self.env.contains_key("SOURCE_DATE_EPOCH"),
            locale_pinned: self
                .env
                .get("LC_ALL")
                .map(|v| v == "C" || v == "C.UTF-8")
                .unwrap_or(false),
            path_pinned: self.env.contains_key("BUILD_PATH_PREFIX"),
            strip_absolute_paths: self
                .env
                .get("RUSTFLAGS")
                .map(|v| v.contains("--remap-path-prefix"))
                .unwrap_or(false),
        }
    }
}

/// Which determinism knobs a build step pins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeterminismFlags {
    pub source_date_epoch: bool,
    pub locale_pinned: bool,
    pub path_pinned: bool,
    pub strip_absolute_paths: bool,
}

impl DeterminismFlags {
    /// Knobs implied by a given non-determinism source being pinned.
    pub fn pins(&self, source: NondeterminismSource) -> bool {
        match source {
            NondeterminismSource::Timestamps | NondeterminismSource::ArchiveMetadata => {
                self.source_date_epoch
            }
            NondeterminismSource::PathPrefix => self.path_pinned || self.strip_absolute_paths,
            NondeterminismSource::Locale => self.locale_pinned,
            _ => false,
        }
    }

    pub fn missing_for(&self, sources: &[NondeterminismSource]) -> Vec<NondeterminismSource> {
        sources.iter().copied().filter(|s| !self.pins(*s)).collect()
    }
}

/// A build pipeline. The rebuild pipeline must be declared independent of the
/// release pipeline, otherwise a "reproducible" result only proves that the
/// release pipeline is deterministic with itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pipeline {
    pub name: String,
    /// Runner/environment identity, e.g. `gh-runner-linux-8-cores`.
    pub runner: String,
    /// Version-control revision the pipeline built from.
    pub revision: String,
    /// Whether this pipeline is the independent rebuild verifier.
    pub independent_rebuild: bool,
}

impl Pipeline {
    pub fn release(name: &str, runner: &str, revision: &str) -> Self {
        Self {
            name: name.to_string(),
            runner: runner.to_string(),
            revision: revision.to_string(),
            independent_rebuild: false,
        }
    }
    pub fn rebuild(name: &str, runner: &str, revision: &str) -> Self {
        Self {
            name: name.to_string(),
            runner: runner.to_string(),
            revision: revision.to_string(),
            independent_rebuild: true,
        }
    }
}

/// Verifies that the rebuild pipeline really is independent.
pub fn assert_independent(release: &Pipeline, rebuild: &Pipeline) -> Result<(), String> {
    if !rebuild.independent_rebuild {
        return Err(format!(
            "pipeline `{}` is not flagged as an independent rebuild",
            rebuild.name
        ));
    }
    if release.name == rebuild.name {
        return Err("rebuild pipeline must not reuse the release pipeline".to_string());
    }
    if release.revision != rebuild.revision {
        return Err(format!(
            "rebuild built revision {} but the release built {}",
            rebuild.revision, release.revision
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Comparison and localization
// ---------------------------------------------------------------------------

/// Why two builds disagree about an artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MismatchKind {
    /// Present in the release build, missing from the rebuild.
    Missing,
    /// Present in the rebuild, absent from the release build.
    Unexpected,
    /// Same artifact, different bytes.
    DigestDiffers,
}

impl std::fmt::Display for MismatchKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            MismatchKind::Missing => "missing",
            MismatchKind::Unexpected => "unexpected",
            MismatchKind::DigestDiffers => "digest_differs",
        };
        f.write_str(s)
    }
}

/// A localized mismatch: which artifact, which build step, which cause.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalizedMismatch {
    pub artifact: String,
    pub kind: ArtifactKind,
    pub mismatch: MismatchKind,
    /// Build step the mismatch is attributed to.
    pub step: String,
    /// Index of the step in the pipeline.
    pub step_index: usize,
    /// Byte offset of the first difference, when both outputs exist.
    pub first_diff_offset: Option<usize>,
    /// Non-determinism sources detected in the differing bytes.
    pub sources: Vec<NondeterminismSource>,
    /// Sources that are not pinned by the step's determinism flags.
    pub unpinned: Vec<NondeterminismSource>,
    /// Human-readable explanation.
    pub detail: String,
}

/// Byte offset of the first difference between two buffers.
pub fn first_difference(a: &[u8], b: &[u8]) -> Option<usize> {
    let common = a.len().min(b.len());
    for i in 0..common {
        if a[i] != b[i] {
            return Some(i);
        }
    }
    if a.len() == b.len() {
        None
    } else {
        Some(common)
    }
}

/// Compare a release build against an independent rebuild and localize every
/// mismatch to the build step that produced the artifact.
/// `rebuild` carries the rebuilt bytes alongside the release build, so a
/// mismatch can be localized to a byte offset rather than just a digest.
pub fn localize_mismatches(
    steps: &[BuildStep],
    release: &[ArtifactFingerprint],
    rebuild: &[ArtifactOutput],
) -> Vec<LocalizedMismatch> {
    let mut out = Vec::new();

    let locate = |name: &str| -> (usize, Option<&ArtifactOutput>) {
        for (i, step) in steps.iter().enumerate() {
            if let Some(o) = step.outputs.iter().find(|o| o.name == name) {
                return (i, Some(o));
            }
        }
        (usize::MAX, None)
    };

    let mut seen: Vec<String> = Vec::new();
    for fp in release {
        seen.push(fp.name.clone());
        let rebuilt = rebuild.iter().find(|r| r.name == fp.name);
        let (step_index, output) = locate(&fp.name);
        let step_name = steps
            .get(step_index)
            .map(|s| s.name.clone())
            .unwrap_or_else(|| "<unlocated>".to_string());
        let flags = steps
            .get(step_index)
            .map(|s| s.determinism_flags())
            .unwrap_or(DeterminismFlags {
                source_date_epoch: false,
                locale_pinned: false,
                path_pinned: false,
                strip_absolute_paths: false,
            });

        match rebuilt {
            None => out.push(LocalizedMismatch {
                artifact: fp.name.clone(),
                kind: fp.kind,
                mismatch: MismatchKind::Missing,
                step: step_name.clone(),
                step_index,
                first_diff_offset: None,
                sources: Vec::new(),
                unpinned: Vec::new(),
                detail: format!(
                    "`{}` produced by step `{}` was not produced by the rebuild",
                    fp.name, step_name
                ),
            }),
            Some(r) if r.digest != fp.digest => {
                // Scan the rebuilt bytes: they are what actually drifted.
                let sources = detect_nondeterminism(&r.bytes);
                let offset = output.and_then(|o| first_difference(&o.bytes, &r.bytes));
                let unpinned = flags.missing_for(&sources);
                let detail = format!(
                    "`{}` differs at byte {}; sources [{}]; step `{}` does not pin [{}]",
                    fp.name,
                    offset
                        .map(|o| o.to_string())
                        .unwrap_or_else(|| "n/a".to_string()),
                    sources
                        .iter()
                        .map(|s| s.to_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                    step_name,
                    unpinned
                        .iter()
                        .map(|s| s.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                out.push(LocalizedMismatch {
                    artifact: fp.name.clone(),
                    kind: fp.kind,
                    mismatch: MismatchKind::DigestDiffers,
                    step: step_name.clone(),
                    step_index,
                    first_diff_offset: offset,
                    sources,
                    unpinned,
                    detail,
                });
            }
            Some(_) => {}
        }
    }

    for r in rebuild {
        if !seen.contains(&r.name) {
            let (step_index, _) = locate(&r.name);
            let step_name = steps
                .get(step_index)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| "<unlocated>".to_string());
            out.push(LocalizedMismatch {
                artifact: r.name.clone(),
                kind: r.kind,
                mismatch: MismatchKind::Unexpected,
                step: step_name.clone(),
                step_index,
                first_diff_offset: None,
                sources: detect_nondeterminism(&r.bytes),
                unpinned: Vec::new(),
                detail: format!(
                    "rebuild produced `{}` which the release build did not (step `{}`)",
                    r.name, step_name
                ),
            });
        }
    }

    out.sort_by(|a, b| {
        a.step_index
            .cmp(&b.step_index)
            .then(a.artifact.cmp(&b.artifact))
    });
    out
}

// ---------------------------------------------------------------------------
// Release-level verification
// ---------------------------------------------------------------------------

/// Per-release reproducibility verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReproducibilityVerdict {
    /// Every artifact rebuilt bit-for-bit identical.
    Reproducible,
    /// Some artifacts differ, but the rate still meets the threshold.
    MostlyReproducible,
    /// Below the required rate.
    NonReproducible,
}

impl std::fmt::Display for ReproducibilityVerdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ReproducibilityVerdict::Reproducible => "reproducible",
            ReproducibilityVerdict::MostlyReproducible => "mostly_reproducible",
            ReproducibilityVerdict::NonReproducible => "non_reproducible",
        };
        f.write_str(s)
    }
}

/// The record stored per release for audit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReleaseVerification {
    pub release: String,
    pub verdict: ReproducibilityVerdict,
    pub artifacts_checked: usize,
    pub reproducible: usize,
    pub reproducible_rate: f64,
    /// Distinct build steps implicated in mismatches.
    pub implicated_steps: Vec<String>,
    /// Non-determinism sources seen across all mismatches.
    pub observed_sources: Vec<NondeterminismSource>,
    pub mismatches: Vec<LocalizedMismatch>,
    pub verified_at: DateTime<Utc>,
}

impl ReleaseVerification {
    /// Meets the >= 95% acceptance bar.
    pub fn meets_threshold(&self) -> bool {
        self.reproducible_rate >= REQUIRED_REPRODUCIBLE_RATE
    }

    /// Badge recorded with the release.
    pub fn badge(&self) -> String {
        format!(
            "reproducibility: {} ({}/{} artifacts, {:.0}%)",
            self.verdict,
            self.reproducible,
            self.artifacts_checked,
            self.reproducible_rate * 100.0
        )
    }

    pub fn to_markdown(&self) -> String {
        let mut out = format!(
            "### Build reproducibility: `{}`\n\n{}\n\n| Artifact | Kind | Mismatch | Step | First diff | Sources |\n|---|---|---|---|---|---|\n",
            self.release,
            self.badge()
        );
        if self.mismatches.is_empty() {
            out.push_str("| _all artifacts identical_ | - | - | - | - | - |\n");
        } else {
            for m in &self.mismatches {
                out.push_str(&format!(
                    "| `{}` | {} | {} | `{}` | {} | {} |\n",
                    m.artifact,
                    m.kind,
                    m.mismatch,
                    m.step,
                    m.first_diff_offset
                        .map(|o| o.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                    m.sources
                        .iter()
                        .map(|s| s.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
        out
    }

    /// Machine-readable form, stored alongside the release.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }
}

/// Verify one release: compare the release build against the independent
/// rebuild and record the per-release status.
///
/// `rebuild_outputs` carries the rebuilt bytes, which is what lets a mismatch
/// be localized to a byte offset and a concrete non-determinism source.
pub fn verify_release(
    release: &str,
    steps: &[BuildStep],
    release_outputs: Vec<ArtifactFingerprint>,
    rebuild_outputs: Vec<ArtifactOutput>,
) -> Result<ReleaseVerification, String> {
    if release_outputs.is_empty() {
        return Err("release has no artifacts to verify".to_string());
    }
    let mismatches = localize_mismatches(steps, &release_outputs, &rebuild_outputs);
    let reproducible = release_outputs.len()
        - mismatches
            .iter()
            .filter(|m| m.mismatch != MismatchKind::Unexpected)
            .count();
    let rate = reproducible as f64 / release_outputs.len() as f64;
    let verdict = if mismatches.is_empty() {
        ReproducibilityVerdict::Reproducible
    } else if rate >= REQUIRED_REPRODUCIBLE_RATE {
        ReproducibilityVerdict::MostlyReproducible
    } else {
        ReproducibilityVerdict::NonReproducible
    };
    let mut implicated_steps: Vec<String> = mismatches.iter().map(|m| m.step.clone()).collect();
    implicated_steps.sort();
    implicated_steps.dedup();
    let mut observed_sources: Vec<NondeterminismSource> =
        mismatches.iter().flat_map(|m| m.sources.clone()).collect();
    observed_sources.sort();
    observed_sources.dedup();

    if verdict == ReproducibilityVerdict::NonReproducible {
        warn!(release, rate, "rebuild reproducibility below threshold");
    } else {
        info!(release, rate, "rebuild reproducibility verified");
    }

    Ok(ReleaseVerification {
        release: release.to_string(),
        verdict,
        artifacts_checked: release_outputs.len(),
        reproducible,
        reproducible_rate: rate,
        implicated_steps,
        observed_sources,
        mismatches,
        verified_at: Utc::now(),
    })
}

/// Verify a batch of releases (the "blind rebuild of the last N releases"
/// audit) and return the aggregate rate.
pub fn verify_releases(reports: &[ReleaseVerification]) -> (usize, usize, f64) {
    let total: usize = reports.iter().map(|r| r.artifacts_checked).sum();
    let ok: usize = reports.iter().map(|r| r.reproducible).sum();
    let rate = if total == 0 {
        0.0
    } else {
        ok as f64 / total as f64
    };
    (ok, total, rate)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step_pinned(name: &str) -> BuildStep {
        BuildStep::new(name)
            .with_env("SOURCE_DATE_EPOCH", "1700000000")
            .with_env("LC_ALL", "C")
            .with_env("BUILD_PATH_PREFIX", "/build")
    }

    fn step_unpinned(name: &str) -> BuildStep {
        BuildStep::new(name)
    }

    fn out(name: &str, kind: ArtifactKind, bytes: Vec<u8>) -> ArtifactOutput {
        ArtifactOutput::new(name, kind, bytes)
    }

    // -- detection ---------------------------------------------------------

    #[test]
    fn detects_timestamps_paths_locale_and_vcs() {
        let bytes = b"built=2026-01-02T03:04:05Z path=C:\\Users\\builder\\src LANG=de_DE.UTF-8 vcs.revision=abcdef1234567".to_vec();
        let found = detect_nondeterminism(&bytes);
        assert!(found.contains(&NondeterminismSource::Timestamps));
        assert!(found.contains(&NondeterminismSource::PathPrefix));
        assert!(found.contains(&NondeterminismSource::Locale));
        assert!(found.contains(&NondeterminismSource::VcsMetadata));
    }

    #[test]
    fn detects_archive_metadata_and_line_endings() {
        let mut gz = vec![0x1f, 0x8b, 0x08, 0x00];
        gz.extend_from_slice(&1234u32.to_le_bytes());
        gz.extend_from_slice(b"body");
        assert!(detect_nondeterminism(&gz).contains(&NondeterminismSource::ArchiveMetadata));

        let mixed = b"a\r\nb\nc".to_vec();
        assert!(detect_nondeterminism(&mixed).contains(&NondeterminismSource::LineEndings));
    }

    #[test]
    fn deterministic_content_reports_no_sources() {
        let bytes = b"stellar-k8s version 2.6.0 sha256:0".to_vec();
        assert!(detect_nondeterminism(&bytes).is_empty());
    }

    #[test]
    fn first_difference_reports_offset_or_none() {
        assert_eq!(first_difference(b"abcd", b"abXd"), Some(2));
        assert_eq!(first_difference(b"abcd", b"abcd"), None);
        assert_eq!(first_difference(b"abcd", b"abcde"), Some(4));
    }

    // -- localization ------------------------------------------------------

    #[test]
    fn mismatch_localizes_to_the_emitting_build_step() {
        let steps = vec![
            step_pinned("cargo-build").with_output(out(
                "stellar-operator",
                ArtifactKind::Binary,
                b"ELF".to_vec(),
            )),
            step_unpinned("image-build").with_output(out(
                "operator-image",
                ArtifactKind::ContainerImage,
                b"layer built=2026-01-02T03:04:05Z at C:\\build\\stage2".to_vec(),
            )),
        ];
        let release: Vec<ArtifactFingerprint> = steps
            .iter()
            .flat_map(|s| s.outputs.iter().map(ArtifactOutput::fingerprint))
            .collect();
        let mut rebuild: Vec<ArtifactOutput> = steps
            .iter()
            .flat_map(|s| s.outputs.iter().cloned())
            .collect();
        // Only the image differs between builds.
        let rebuilt_image = ArtifactOutput::new(
            "operator-image",
            ArtifactKind::ContainerImage,
            b"layer built=2026-02-03T04:05:06Z at D:\\build\\stage2".to_vec(),
        );
        rebuild[1] = rebuilt_image;

        let mismatches = localize_mismatches(&steps, &release, &rebuild);
        assert_eq!(mismatches.len(), 1);
        let m = &mismatches[0];
        assert_eq!(m.artifact, "operator-image");
        assert_eq!(m.step, "image-build");
        assert_eq!(m.step_index, 1);
        assert_eq!(m.mismatch, MismatchKind::DigestDiffers);
        assert!(m.first_diff_offset.is_some());
        assert!(m.sources.contains(&NondeterminismSource::Timestamps));
        // The step does not pin SOURCE_DATE_EPOCH or the path prefix.
        assert!(m.unpinned.contains(&NondeterminismSource::Timestamps));
        assert!(m.unpinned.contains(&NondeterminismSource::PathPrefix));
        assert!(m.detail.contains("image-build"));
    }

    #[test]
    fn missing_and_unexpected_artifacts_are_reported() {
        let steps = vec![step_pinned("cargo-build").with_output(out(
            "a",
            ArtifactKind::Binary,
            b"a".to_vec(),
        ))];
        let release = vec![
            steps[0].outputs[0].fingerprint(),
            ArtifactFingerprint {
                name: "sbom".to_string(),
                kind: ArtifactKind::Sbom,
                digest: sha256_hex(b"sbom"),
                len: 4,
            },
        ];
        let rebuild = vec![steps[0].outputs[0].clone()];
        let m = localize_mismatches(&steps, &release, &rebuild);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].mismatch, MismatchKind::Missing);
        assert_eq!(m[0].artifact, "sbom");
    }

    #[test]
    fn unexpected_artifact_is_reported_even_though_it_is_new() {
        let steps = vec![step_pinned("cargo-build").with_output(out(
            "bin",
            ArtifactKind::Binary,
            b"payload".to_vec(),
        ))];
        let release: Vec<ArtifactFingerprint> = steps[0]
            .outputs
            .iter()
            .map(ArtifactOutput::fingerprint)
            .collect();
        let rebuild = vec![
            steps[0].outputs[0].clone(),
            out("stray.log", ArtifactKind::Sbom, b"extra".to_vec()),
        ];
        let m = localize_mismatches(&steps, &release, &rebuild);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].mismatch, MismatchKind::Unexpected);
        assert_eq!(m[0].artifact, "stray.log");
    }

    #[test]
    fn identical_builds_produce_no_mismatches() {
        let steps = vec![
            step_pinned("cargo-build").with_output(out(
                "bin",
                ArtifactKind::Binary,
                b"payload".to_vec(),
            )),
            step_pinned("chart").with_output(out(
                "chart",
                ArtifactKind::HelmChart,
                b"chart-bytes".to_vec(),
            )),
        ];
        let fps: Vec<ArtifactFingerprint> = steps
            .iter()
            .flat_map(|s| s.outputs.iter().map(ArtifactOutput::fingerprint))
            .collect();
        let outputs: Vec<ArtifactOutput> = steps
            .iter()
            .flat_map(|s| s.outputs.iter().cloned())
            .collect();
        assert!(localize_mismatches(&steps, &fps, &outputs).is_empty());
    }

    // -- independence ------------------------------------------------------

    #[test]
    fn rebuild_pipeline_must_be_independent() {
        let release = Pipeline::release("release", "gh-runner-linux-8", "abc123");
        let rebuild = Pipeline::rebuild("repro-check", "gh-runner-linux-4", "abc123");
        assert!(assert_independent(&release, &rebuild).is_ok());

        let same = Pipeline::release("release", "gh-runner-linux-8", "abc123");
        assert!(assert_independent(&release, &same).is_err());
        assert!(
            assert_independent(&release, &Pipeline::release("release", "x", "def456")).is_err()
        );
    }

    // -- release verification ---------------------------------------------

    /// A five-step release pipeline: operator binaries, CRDs, chart, SBOM and
    /// the OLM bundle. The non-deterministic variant leaves `crd-gen` unpinned.
    fn full_pipeline(deterministic: bool) -> Vec<BuildStep> {
        let crd_gen = if deterministic {
            step_pinned("crd-gen").with_output(out(
                "crds",
                ArtifactKind::CrdBundle,
                b"crd-yaml".to_vec(),
            ))
        } else {
            step_unpinned("crd-gen").with_output(out(
                "crds",
                ArtifactKind::CrdBundle,
                b"crd changed".to_vec(),
            ))
        };
        vec![
            step_pinned("cargo-build")
                .with_output(out(
                    "stellar-operator",
                    ArtifactKind::Binary,
                    b"elf-bytes".to_vec(),
                ))
                .with_output(out(
                    "stellar-sidecar",
                    ArtifactKind::Binary,
                    b"sidecar-bytes".to_vec(),
                )),
            crd_gen,
            step_pinned("helm-package").with_output(out(
                "chart",
                ArtifactKind::HelmChart,
                b"chart".to_vec(),
            )),
            step_pinned("sbom").with_output(out("sbom", ArtifactKind::Sbom, b"sbom".to_vec())),
            step_pinned("bundle").with_output(out(
                "olm",
                ArtifactKind::OperatorBundle,
                b"olm".to_vec(),
            )),
        ]
    }

    fn fingerprints(steps: &[BuildStep]) -> Vec<ArtifactFingerprint> {
        steps
            .iter()
            .flat_map(|s| s.outputs.iter().map(ArtifactOutput::fingerprint))
            .collect()
    }

    fn outputs(steps: &[BuildStep]) -> Vec<ArtifactOutput> {
        steps
            .iter()
            .flat_map(|s| s.outputs.iter().cloned())
            .collect()
    }

    #[test]
    fn deterministic_rebuild_is_reproducible() {
        let steps = full_pipeline(true);
        let r = verify_release("v2.6.0", &steps, fingerprints(&steps), outputs(&steps)).unwrap();
        assert_eq!(r.verdict, ReproducibilityVerdict::Reproducible);
        assert_eq!(r.reproducible_rate, 1.0);
        assert!(r.meets_threshold());
        assert!(r.badge().contains("reproducible"));
        assert!(r.mismatches.is_empty());
        assert!(r.to_markdown().contains("all artifacts identical"));
    }

    #[test]
    fn one_bad_artifact_out_of_twenty_still_meets_the_threshold() {
        // 20 artifacts, 1 drifts -> exactly 95%, still at the bar.
        let mut step = step_unpinned("crd-gen");
        for i in 0..20 {
            step = step.with_output(out(
                &format!("crd-{i:02}"),
                ArtifactKind::CrdBundle,
                format!("crd-{i}").into_bytes(),
            ));
        }
        let steps = vec![step];
        let mut rebuild = outputs(&steps);
        let index = rebuild.iter().position(|f| f.name == "crd-07").unwrap();
        rebuild[index] =
            ArtifactOutput::new("crd-07", ArtifactKind::CrdBundle, b"different".to_vec());
        let r = verify_release("v2.6.1", &steps, fingerprints(&steps), rebuild).unwrap();
        assert_eq!(r.verdict, ReproducibilityVerdict::MostlyReproducible);
        assert!((r.reproducible_rate - 0.95).abs() < 1e-9);
        assert!(r.meets_threshold(), "95% is exactly the bar");
        assert_eq!(r.implicated_steps, vec!["crd-gen".to_string()]);
    }

    #[test]
    fn mostly_non_reproducible_release_is_flagged() {
        let steps = full_pipeline(false);
        // Rebuild changes three of the five artifacts: 40% reproducible.
        let mut rebuild = outputs(&steps);
        for name in ["stellar-operator", "crds", "sbom"] {
            let i = rebuild.iter().position(|f| f.name == name).unwrap();
            rebuild[i] = ArtifactOutput::new(name, ArtifactKind::Binary, b"different".to_vec());
        }
        let r = verify_release("v2.6.2", &steps, fingerprints(&steps), rebuild).unwrap();
        assert_eq!(r.verdict, ReproducibilityVerdict::NonReproducible);
        assert!(!r.meets_threshold());
        // Six artifacts in the pipeline; three drifted, so 50% reproducible.
        assert_eq!(r.artifacts_checked, 6);
        assert_eq!(r.reproducible, 3);
        assert!((r.reproducible_rate - 0.5).abs() < 1e-9);
        assert!(r.to_json().contains("reproducible_rate"));
    }

    #[test]
    fn blind_rebuild_of_ten_releases_meets_the_ninety_five_percent_bar() {
        let steps = full_pipeline(true);
        let good = fingerprints(&steps);
        let clean = outputs(&steps);
        let mut reports = Vec::new();
        for i in 0..10 {
            // Nine releases rebuild cleanly; one has a single drifted artifact.
            let rebuild = if i == 9 {
                let mut r = clean.clone();
                let idx = r.iter().position(|f| f.name == "sbom").unwrap();
                r[idx] = ArtifactOutput::new("sbom", ArtifactKind::Sbom, b"drift".to_vec());
                r
            } else {
                clean.clone()
            };
            reports
                .push(verify_release(&format!("v2.6.{i}"), &steps, good.clone(), rebuild).unwrap());
        }
        let (ok, total, rate) = verify_releases(&reports);
        // Six artifacts per release across ten releases, one artifact drifted.
        assert_eq!(total, 60);
        assert_eq!(ok, 59);
        assert!((rate - 59.0 / 60.0).abs() < 1e-9, "rate was {rate}");
        assert!(rate >= REQUIRED_REPRODUCIBLE_RATE);
        // The single drifted release is still localized to its step.
        let drifted = &reports[9];
        assert_eq!(drifted.implicated_steps, vec!["sbom".to_string()]);
        assert!(!drifted.mismatches.is_empty());
    }

    #[test]
    fn release_without_artifacts_is_rejected() {
        let steps = full_pipeline(true);
        assert!(verify_release("v0.0.0", &steps, Vec::new(), Vec::new()).is_err());
    }

    #[test]
    fn determinism_flags_report_unpinned_sources() {
        let pinned = step_pinned("s").determinism_flags();
        assert_eq!(
            pinned.missing_for(&[NondeterminismSource::Timestamps]),
            Vec::new()
        );
        let unpinned = step_unpinned("s").determinism_flags();
        assert_eq!(
            unpinned.missing_for(&[
                NondeterminismSource::Timestamps,
                NondeterminismSource::PathPrefix
            ]),
            vec![
                NondeterminismSource::Timestamps,
                NondeterminismSource::PathPrefix
            ]
        );
    }
}
