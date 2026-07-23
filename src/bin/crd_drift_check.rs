//! crd-drift-check — Drift detection between Helm chart templates and the
//! repository's canonical rendered CRD manifests.
//!
//! Stellar-K8s's CustomResourceDefinitions exist in two places that must be
//! kept consistent by hand: the checked-in reference manifests under
//! `config/crd/*.yaml`, and the CRD templates rendered by the Helm chart
//! (`helm template charts/stellar-operator`). Nothing enforced that these
//! stay in sync — this tool does, using the same baseline/snapshot pattern
//! as `doc-check` (see `src/bin/doc_check.rs`): today's state (including any
//! already-known gaps) is captured once as the accepted baseline, and CI
//! fails whenever the *current* state differs from that baseline, forcing a
//! deliberate, reviewed `--update-baseline` instead of silent drift.
//!
//! For every CRD name found on either side, this tool hashes the
//! `openAPIV3Schema` of each version and compares:
//!   - whether the CRD is present in `config/crd/` and/or the Helm chart
//!   - a content hash per version on each side
//!
//! # Quick start
//!
//! ```text
//! # Check for drift against the last accepted baseline
//! cargo run --bin crd-drift-check
//!
//! # Show the current state of every known CRD without failing
//! cargo run --bin crd-drift-check -- status
//!
//! # Accept the current state as the new baseline (after a reviewed change)
//! cargo run --bin crd-drift-check -- update-baseline
//! ```
//!
//! See `docs/crd-drift-detection.md` for the full user guide.

use std::{
    collections::{BTreeMap, HashMap},
    fmt, fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── CLI ───────────────────────────────────────────────────────────────────────

/// Detect drift between `config/crd/*.yaml` and the Helm chart's rendered CRDs.
#[derive(Parser, Debug)]
#[command(
    name = "crd-drift-check",
    version,
    about = "Detect drift between config/crd/ and the Helm chart's rendered CRDs"
)]
struct Cli {
    /// Directory containing the canonical CRD YAML files.
    #[arg(long, default_value = "config/crd")]
    crd_dir: PathBuf,

    /// Path to the Helm chart to render.
    #[arg(long, default_value = "charts/stellar-operator")]
    chart_dir: PathBuf,

    /// Path to the baseline file recording the last-accepted state.
    #[arg(long, default_value = ".crd-drift-baseline.toml")]
    baseline: PathBuf,

    /// Exit with code 0 even when drift is found (useful in warn-only mode).
    #[arg(long)]
    warn_only: bool,

    #[command(subcommand)]
    command: Option<CliCommand>,
}

#[derive(Subcommand, Debug)]
enum CliCommand {
    /// Print the current state of every known CRD and exit (no drift check).
    Status,
    /// Accept the current state as the new baseline.
    UpdateBaseline,
}

// ── CRD state ────────────────────────────────────────────────────────────────

/// The union of a CRD's presence/schema-hash on both sides, for one version.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, Default)]
struct VersionState {
    #[serde(skip_serializing_if = "Option::is_none")]
    config_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    helm_hash: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, Default)]
struct CrdState {
    versions: BTreeMap<String, VersionState>,
}

#[derive(Serialize, Deserialize, Default)]
struct Baseline {
    #[serde(flatten)]
    crds: BTreeMap<String, CrdState>,
}

impl Baseline {
    fn load(path: &Path) -> Self {
        let content = fs::read_to_string(path).unwrap_or_default();
        toml::from_str(&content).unwrap_or_default()
    }

    fn save(&self, path: &Path) -> anyhow::Result<()> {
        let content = toml::to_string_pretty(self)?;
        fs::write(path, content)?;
        Ok(())
    }
}

fn stable_hash(value: &Value) -> String {
    // serde_json's default `Map` is BTreeMap-backed (no `preserve_order`
    // feature enabled in this workspace), so `to_string` yields keys in a
    // stable, canonical order — safe to hash directly.
    let canonical = serde_json::to_string(value).unwrap_or_default();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    canonical.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// One `(group, kind, name)` CRD's per-version `openAPIV3Schema` map.
struct RawCrd {
    name: String,
    versions: HashMap<String, Value>,
}

fn parse_crd_documents(content: &str) -> Vec<RawCrd> {
    let mut crds = Vec::new();
    for doc in serde_yaml::Deserializer::from_str(content) {
        let Ok(doc) = Value::deserialize(doc) else {
            continue;
        };
        if doc.get("kind").and_then(Value::as_str) != Some("CustomResourceDefinition") {
            continue;
        }
        let Some(name) = doc["metadata"]["name"].as_str() else {
            continue;
        };
        let mut versions = HashMap::new();
        if let Some(vs) = doc["spec"]["versions"].as_array() {
            for v in vs {
                let Some(vname) = v.get("name").and_then(Value::as_str) else {
                    continue;
                };
                if let Some(schema) = v.get("schema").and_then(|s| s.get("openAPIV3Schema")) {
                    versions.insert(vname.to_string(), schema.clone());
                }
            }
        }
        crds.push(RawCrd {
            name: name.to_string(),
            versions,
        });
    }
    crds
}

fn load_config_crds(dir: &Path) -> anyhow::Result<Vec<RawCrd>> {
    let mut out = Vec::new();
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .is_some_and(|ext| ext == "yaml" || ext == "yml")
        })
        .collect();
    entries.sort();
    for path in entries {
        let content = fs::read_to_string(&path)?;
        out.extend(parse_crd_documents(&content));
    }
    Ok(out)
}

/// Render the Helm chart with `helm template` and parse out its CRDs.
///
/// Returns an error string (not `anyhow::Error`) so the caller can print a
/// clear, actionable message without a Rust backtrace.
fn render_helm_crds(chart_dir: &Path) -> Result<Vec<RawCrd>, String> {
    let output = Command::new("helm")
        .args(["template", "stellar-operator", &chart_dir.to_string_lossy()])
        .output()
        .map_err(|e| format!("failed to run `helm`: {e} (is Helm installed and on PATH?)"))?;

    if !output.status.success() {
        return Err(format!(
            "`helm template` failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_crd_documents(&stdout))
}

fn compute_current_state(
    config_crds: &[RawCrd],
    helm_crds: &[RawCrd],
) -> BTreeMap<String, CrdState> {
    let mut state: BTreeMap<String, CrdState> = BTreeMap::new();

    for crd in config_crds {
        let entry = state.entry(crd.name.clone()).or_default();
        for (version, schema) in &crd.versions {
            entry
                .versions
                .entry(version.clone())
                .or_default()
                .config_hash = Some(stable_hash(schema));
        }
    }
    for crd in helm_crds {
        let entry = state.entry(crd.name.clone()).or_default();
        for (version, schema) in &crd.versions {
            entry.versions.entry(version.clone()).or_default().helm_hash =
                Some(stable_hash(schema));
        }
    }

    state
}

// ── Diffing ──────────────────────────────────────────────────────────────────

enum Drift {
    /// A CRD/version appears in the current state but not in the baseline.
    New { crd: String, version: String },
    /// A CRD/version was in the baseline but has disappeared entirely.
    Removed { crd: String, version: String },
    /// `config/crd/` has this version but the Helm chart no longer does (or vice versa).
    PresenceChanged {
        crd: String,
        version: String,
        in_config: bool,
        in_helm: bool,
    },
    /// The schema hash changed on at least one side relative to the baseline.
    SchemaChanged { crd: String, version: String },
}

impl fmt::Display for Drift {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Drift::New { crd, version } => {
                write!(f, "NEW       {crd} @ {version} (not yet baselined)")
            }
            Drift::Removed { crd, version } => {
                write!(f, "REMOVED   {crd} @ {version} (was baselined, now gone)")
            }
            Drift::PresenceChanged {
                crd,
                version,
                in_config,
                in_helm,
            } => write!(
                f,
                "PRESENCE  {crd} @ {version} — in config/crd: {in_config}, in Helm chart: {in_helm}"
            ),
            Drift::SchemaChanged { crd, version } => {
                write!(
                    f,
                    "SCHEMA    {crd} @ {version} — schema hash changed since baseline"
                )
            }
        }
    }
}

fn diff_against_baseline(current: &BTreeMap<String, CrdState>, baseline: &Baseline) -> Vec<Drift> {
    let mut drifts = Vec::new();

    let mut all_names: Vec<&String> = current.keys().chain(baseline.crds.keys()).collect();
    all_names.sort();
    all_names.dedup();

    for name in all_names {
        let cur = current.get(name);
        let base = baseline.crds.get(name);

        let mut all_versions: Vec<&String> = cur
            .map(|c| c.versions.keys().collect::<Vec<_>>())
            .unwrap_or_default();
        all_versions.extend(
            base.map(|b| b.versions.keys().collect::<Vec<_>>())
                .unwrap_or_default(),
        );
        all_versions.sort();
        all_versions.dedup();

        for version in all_versions {
            let cur_v = cur.and_then(|c| c.versions.get(version));
            let base_v = base.and_then(|b| b.versions.get(version));

            match (cur_v, base_v) {
                (Some(_), None) => drifts.push(Drift::New {
                    crd: name.clone(),
                    version: version.clone(),
                }),
                (None, Some(_)) => drifts.push(Drift::Removed {
                    crd: name.clone(),
                    version: version.clone(),
                }),
                (Some(c), Some(b)) => {
                    let cur_in_config = c.config_hash.is_some();
                    let cur_in_helm = c.helm_hash.is_some();
                    let base_in_config = b.config_hash.is_some();
                    let base_in_helm = b.helm_hash.is_some();
                    if cur_in_config != base_in_config || cur_in_helm != base_in_helm {
                        drifts.push(Drift::PresenceChanged {
                            crd: name.clone(),
                            version: version.clone(),
                            in_config: cur_in_config,
                            in_helm: cur_in_helm,
                        });
                    } else if c != b {
                        drifts.push(Drift::SchemaChanged {
                            crd: name.clone(),
                            version: version.clone(),
                        });
                    }
                }
                (None, None) => {}
            }
        }
    }

    drifts
}

// ── main ─────────────────────────────────────────────────────────────────────

fn find_repo_root() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        if dir.join("Cargo.toml").exists() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

fn print_status(state: &BTreeMap<String, CrdState>) {
    println!(
        "{} CRD name(s) known across config/crd/ and the Helm chart:\n",
        state.len()
    );
    for (name, crd) in state {
        for (version, v) in &crd.versions {
            let config = if v.config_hash.is_some() {
                "config/crd"
            } else {
                "—"
            };
            let helm = if v.helm_hash.is_some() { "helm" } else { "—" };
            let matches = if v.config_hash.is_some() && v.helm_hash.is_some() {
                if v.config_hash == v.helm_hash {
                    "identical"
                } else {
                    "differs"
                }
            } else {
                "n/a"
            };
            println!("  {name} @ {version} — sources: [{config}, {helm}], schema: {matches}");
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let repo_root = find_repo_root().unwrap_or_else(|| PathBuf::from("."));

    let config_crds = match load_config_crds(&repo_root.join(&cli.crd_dir)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: failed to load {}: {e}", cli.crd_dir.display());
            return ExitCode::FAILURE;
        }
    };
    let helm_crds = match render_helm_crds(&repo_root.join(&cli.chart_dir)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    let current_state = compute_current_state(&config_crds, &helm_crds);
    let baseline_path = repo_root.join(&cli.baseline);

    match cli.command {
        Some(CliCommand::Status) => {
            print_status(&current_state);
            return ExitCode::SUCCESS;
        }
        Some(CliCommand::UpdateBaseline) => {
            let baseline = Baseline {
                crds: current_state,
            };
            return match baseline.save(&baseline_path) {
                Ok(()) => {
                    println!(
                        "✓ Baseline updated: {} CRD(s) recorded at {}",
                        baseline.crds.len(),
                        baseline_path.display()
                    );
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("error: failed to write baseline: {e}");
                    ExitCode::FAILURE
                }
            };
        }
        None => {}
    }

    let baseline = Baseline::load(&baseline_path);
    let drifts = diff_against_baseline(&current_state, &baseline);

    if drifts.is_empty() {
        println!(
            "✅  No CRD drift detected — {} CRD(s) match the accepted baseline.",
            current_state.len()
        );
        return ExitCode::SUCCESS;
    }

    eprintln!(
        "\n🔴  CRD drift detected against {}:\n",
        baseline_path.display()
    );
    for drift in &drifts {
        eprintln!("    {drift}");
    }
    eprintln!(
        "\n  → config/crd/ and the Helm chart's rendered CRDs no longer match the last accepted baseline."
    );
    eprintln!("  → If this change is intentional, review it and then run:");
    eprintln!("      cargo run --bin crd-drift-check -- update-baseline");

    if cli.warn_only {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema(json: serde_json::Value) -> Value {
        json
    }

    #[test]
    fn stable_hash_is_deterministic_and_order_independent() {
        let a = serde_json::json!({"type": "object", "properties": {"x": {"type": "string"}}});
        let b = serde_json::json!({"properties": {"x": {"type": "string"}}, "type": "object"});
        assert_eq!(stable_hash(&a), stable_hash(&b));
    }

    #[test]
    fn stable_hash_differs_on_content_change() {
        let a = serde_json::json!({"type": "string"});
        let b = serde_json::json!({"type": "integer"});
        assert_ne!(stable_hash(&a), stable_hash(&b));
    }

    #[test]
    fn parses_crd_document_with_versions() {
        let yaml = r#"
apiVersion: apiextensions.k8s.io/v1
kind: CustomResourceDefinition
metadata:
  name: widgets.example.com
spec:
  versions:
    - name: v1
      schema:
        openAPIV3Schema:
          type: object
"#;
        let crds = parse_crd_documents(yaml);
        assert_eq!(crds.len(), 1);
        assert_eq!(crds[0].name, "widgets.example.com");
        assert!(crds[0].versions.contains_key("v1"));
    }

    #[test]
    fn ignores_non_crd_documents() {
        let yaml = "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: foo\n";
        assert!(parse_crd_documents(yaml).is_empty());
    }

    #[test]
    fn compute_current_state_merges_both_sides() {
        let config = vec![RawCrd {
            name: "widgets.example.com".into(),
            versions: HashMap::from([(
                "v1".to_string(),
                schema(serde_json::json!({"type": "object"})),
            )]),
        }];
        let helm = vec![RawCrd {
            name: "widgets.example.com".into(),
            versions: HashMap::from([(
                "v1".to_string(),
                schema(serde_json::json!({"type": "object"})),
            )]),
        }];
        let state = compute_current_state(&config, &helm);
        let v1 = &state["widgets.example.com"].versions["v1"];
        assert!(v1.config_hash.is_some());
        assert!(v1.helm_hash.is_some());
        assert_eq!(v1.config_hash, v1.helm_hash);
    }

    #[test]
    fn diff_detects_new_crd() {
        let mut current = BTreeMap::new();
        current.insert(
            "widgets.example.com".to_string(),
            CrdState {
                versions: BTreeMap::from([(
                    "v1".to_string(),
                    VersionState {
                        config_hash: Some("abc".into()),
                        helm_hash: None,
                    },
                )]),
            },
        );
        let baseline = Baseline::default();
        let drifts = diff_against_baseline(&current, &baseline);
        assert_eq!(drifts.len(), 1);
        assert!(matches!(drifts[0], Drift::New { .. }));
    }

    #[test]
    fn diff_detects_schema_change() {
        let mut current = BTreeMap::new();
        current.insert(
            "widgets.example.com".to_string(),
            CrdState {
                versions: BTreeMap::from([(
                    "v1".to_string(),
                    VersionState {
                        config_hash: Some("new-hash".into()),
                        helm_hash: Some("new-hash".into()),
                    },
                )]),
            },
        );
        let mut baseline = Baseline::default();
        baseline.crds.insert(
            "widgets.example.com".to_string(),
            CrdState {
                versions: BTreeMap::from([(
                    "v1".to_string(),
                    VersionState {
                        config_hash: Some("old-hash".into()),
                        helm_hash: Some("old-hash".into()),
                    },
                )]),
            },
        );
        let drifts = diff_against_baseline(&current, &baseline);
        assert_eq!(drifts.len(), 1);
        assert!(matches!(drifts[0], Drift::SchemaChanged { .. }));
    }

    #[test]
    fn diff_detects_presence_change() {
        let mut current = BTreeMap::new();
        current.insert(
            "widgets.example.com".to_string(),
            CrdState {
                versions: BTreeMap::from([(
                    "v1".to_string(),
                    VersionState {
                        config_hash: Some("h".into()),
                        helm_hash: None, // dropped from Helm chart
                    },
                )]),
            },
        );
        let mut baseline = Baseline::default();
        baseline.crds.insert(
            "widgets.example.com".to_string(),
            CrdState {
                versions: BTreeMap::from([(
                    "v1".to_string(),
                    VersionState {
                        config_hash: Some("h".into()),
                        helm_hash: Some("h".into()),
                    },
                )]),
            },
        );
        let drifts = diff_against_baseline(&current, &baseline);
        assert_eq!(drifts.len(), 1);
        assert!(matches!(drifts[0], Drift::PresenceChanged { .. }));
    }

    #[test]
    fn no_drift_when_state_matches_baseline() {
        let mut current = BTreeMap::new();
        current.insert(
            "widgets.example.com".to_string(),
            CrdState {
                versions: BTreeMap::from([(
                    "v1".to_string(),
                    VersionState {
                        config_hash: Some("h".into()),
                        helm_hash: Some("h".into()),
                    },
                )]),
            },
        );
        let mut baseline = Baseline::default();
        baseline.crds.insert(
            "widgets.example.com".to_string(),
            CrdState {
                versions: BTreeMap::from([(
                    "v1".to_string(),
                    VersionState {
                        config_hash: Some("h".into()),
                        helm_hash: Some("h".into()),
                    },
                )]),
            },
        );
        assert!(diff_against_baseline(&current, &baseline).is_empty());
    }

    #[test]
    fn baseline_toml_roundtrip() {
        let dir = std::env::temp_dir().join(format!("crd-drift-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("baseline.toml");

        let mut baseline = Baseline::default();
        baseline.crds.insert(
            "widgets.example.com".to_string(),
            CrdState {
                versions: BTreeMap::from([(
                    "v1".to_string(),
                    VersionState {
                        config_hash: Some("h1".into()),
                        helm_hash: Some("h2".into()),
                    },
                )]),
            },
        );
        baseline.save(&path).unwrap();
        let loaded = Baseline::load(&path);
        assert_eq!(
            loaded.crds["widgets.example.com"].versions["v1"].config_hash,
            Some("h1".to_string())
        );
        fs::remove_dir_all(&dir).ok();
    }
}
