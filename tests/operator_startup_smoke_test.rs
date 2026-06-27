/// tests/operator_startup_smoke_test.rs
///
/// Smoke tests for operator binary startup and library readiness.
///
/// The binary tests spawn `stellar-operator` as a subprocess and assert on its
/// exit code and output.  They do **not** require a running Kubernetes cluster.
///
/// The library unit tests exercise `OperatorConfig` directly without spawning
/// any process or connecting to a cluster.
///
/// # Usage
///
/// ```bash
/// # Run all startup smoke tests
/// cargo test --test operator_startup_smoke_test
///
/// # Run with log output visible
/// cargo test --test operator_startup_smoke_test -- --nocapture
/// ```
use std::path::PathBuf;
use std::process::Command;

/// Returns the path to the compiled `stellar-operator` binary.
///
/// We resolve via `CARGO_MANIFEST_DIR` so the test works regardless of whether
/// Cargo converts hyphens to underscores in `CARGO_BIN_EXE_*` variable names
/// (the exact behaviour has varied between Cargo versions).
fn operator_bin() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // `cargo test` defaults to the debug profile; `cargo test --release` uses
    // release.  Use debug_assertions as a proxy since that's how Cargo sets it.
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    path.push("target");
    path.push(profile);
    path.push("stellar-operator");
    if cfg!(windows) {
        path.set_extension("exe");
    }
    path
}

// ---------------------------------------------------------------------------
// Subprocess helper
// ---------------------------------------------------------------------------

struct ProcessOutput {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

/// Spawn `stellar-operator <args>` and capture exit status + output.
///
/// `STELLAR_OFFLINE=true` is always injected to suppress the background
/// GitHub version check, which would require network access.
fn run_binary(args: &[&str]) -> ProcessOutput {
    let bin = operator_bin();
    let output = Command::new(&bin)
        .args(args)
        .env("STELLAR_OFFLINE", "true")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn {}: {e}", bin.display()));

    ProcessOutput {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

// ---------------------------------------------------------------------------
// Binary process tests — no Kubernetes required
// ---------------------------------------------------------------------------

/// `--version` exits 0 and prints the crate version.
#[test]
fn binary_version_flag_exits_zero_and_prints_version() {
    let out = run_binary(&["--version"]);
    assert!(
        out.status.success(),
        "`--version` should exit 0; got {}\nstderr: {}",
        out.status,
        out.stderr
    );
    let version = env!("CARGO_PKG_VERSION");
    assert!(
        out.stdout.contains(version),
        "expected version {version} in output; got:\n{}",
        out.stdout
    );
}

/// `version` subcommand exits 0 and prints build metadata fields.
#[test]
fn binary_version_subcommand_prints_build_metadata() {
    let out = run_binary(&["version"]);
    assert!(
        out.status.success(),
        "`version` subcommand should exit 0; got {}\nstderr: {}",
        out.status,
        out.stderr
    );
    for field in &["Build Date", "Git SHA", "Rust Version"] {
        assert!(
            out.stdout.contains(field),
            "expected '{field}' in `version` output:\n{}",
            out.stdout
        );
    }
}

/// `--help` exits 0 and lists all core subcommands.
#[test]
fn binary_help_lists_core_subcommands() {
    let out = run_binary(&["--help"]);
    assert!(
        out.status.success(),
        "`--help` should exit 0; got {}\nstderr: {}",
        out.status,
        out.stderr
    );
    for cmd in &["run", "version", "doctor", "webhook", "check-crd"] {
        assert!(
            out.stdout.contains(cmd),
            "expected subcommand '{cmd}' in --help output:\n{}",
            out.stdout
        );
    }
}

/// `run --help` exits 0 and describes the `--namespace` and `--dump-config` flags.
#[test]
fn binary_run_help_describes_key_flags() {
    let out = run_binary(&["run", "--help"]);
    assert!(
        out.status.success(),
        "`run --help` should exit 0; got {}",
        out.status
    );
    for flag in &["--namespace", "--dump-config", "--dry-run"] {
        assert!(
            out.stdout.contains(flag),
            "expected flag '{flag}' in `run --help` output:\n{}",
            out.stdout
        );
    }
}

/// `run --dump-config` exits 0 **without** a Kubernetes cluster.
///
/// The `--dump-config` path returns before any cluster connectivity is attempted,
/// making it a safe pre-flight check that the operator binary and its default
/// configuration are intact.
#[test]
fn binary_dump_config_exits_zero_without_cluster() {
    let out = run_binary(&["run", "--dump-config"]);
    assert!(
        out.status.success(),
        "`run --dump-config` must exit 0 without a cluster; got {}\nstderr: {}",
        out.status,
        out.stderr
    );
}

/// `run --dump-config` emits a YAML document with the expected top-level keys.
#[test]
fn binary_dump_config_output_contains_operator_config_key() {
    let out = run_binary(&["run", "--dump-config"]);
    assert!(out.status.success());
    assert!(
        out.stdout.contains("operator_config"),
        "expected 'operator_config' in dump-config output:\n{}",
        out.stdout
    );
}

/// `run --dump-config` output is valid YAML that can be parsed without error.
#[test]
fn binary_dump_config_output_is_valid_yaml() {
    let out = run_binary(&["run", "--dump-config"]);
    assert!(out.status.success());

    let parsed: serde_yaml::Value = serde_yaml::from_str(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "`run --dump-config` produced invalid YAML: {e}\n---\n{}",
            out.stdout
        )
    });
    assert!(
        parsed.is_mapping(),
        "expected a YAML mapping at the top level; got: {parsed:?}"
    );
}

/// An unrecognised subcommand exits non-zero (clap parse error).
#[test]
fn binary_unknown_subcommand_exits_nonzero() {
    let out = run_binary(&["this-subcommand-does-not-exist"]);
    assert!(
        !out.status.success(),
        "expected non-zero exit for unknown subcommand; got {}",
        out.status
    );
}

// ---------------------------------------------------------------------------
// Library unit tests — no binary spawn, no Kubernetes
// ---------------------------------------------------------------------------

use stellar_k8s::controller::OperatorConfig;

/// `OperatorConfig::load_from_file` falls back to defaults when the path does
/// not exist, without panicking.
#[test]
fn operator_config_load_falls_back_to_defaults_on_missing_file() {
    let cfg = OperatorConfig::load_from_file("/tmp/this-config-file-does-not-exist-smoke.yaml");
    // Verify a handful of well-known defaults to catch regressions.
    assert!(
        cfg.disk_scaling.enabled,
        "disk scaling should be enabled by default"
    );
    assert_eq!(
        cfg.disk_scaling.expansion_threshold, 80,
        "default expansion threshold should be 80%"
    );
}

/// Disk-scaling defaults are within valid parameter bounds.
#[test]
fn operator_config_disk_scaling_defaults_in_valid_range() {
    let cfg = OperatorConfig::default();
    assert!(
        cfg.disk_scaling.expansion_threshold <= 100,
        "expansion_threshold must be a percentage (≤ 100), got {}",
        cfg.disk_scaling.expansion_threshold
    );
    assert!(
        cfg.disk_scaling.expansion_increment > 0,
        "expansion_increment must be > 0"
    );
    assert!(
        cfg.disk_scaling.min_expansion_interval_secs > 0,
        "min_expansion_interval_secs must be > 0"
    );
    assert!(
        cfg.disk_scaling.max_expansions > 0,
        "max_expansions must be > 0"
    );
}

/// `OperatorConfig` round-trips through YAML serialization without data loss.
#[test]
fn operator_config_round_trips_through_yaml() {
    let original = OperatorConfig::default();

    let yaml = serde_yaml::to_string(&original).expect("OperatorConfig serialization failed");
    assert!(!yaml.is_empty(), "serialized YAML should not be empty");

    let restored: OperatorConfig =
        serde_yaml::from_str(&yaml).expect("OperatorConfig deserialization failed");

    assert_eq!(
        original.disk_scaling.enabled, restored.disk_scaling.enabled,
        "disk_scaling.enabled changed after round-trip"
    );
    assert_eq!(
        original.disk_scaling.expansion_threshold, restored.disk_scaling.expansion_threshold,
        "disk_scaling.expansion_threshold changed after round-trip"
    );
    assert_eq!(
        original.disk_scaling.max_expansions, restored.disk_scaling.max_expansions,
        "disk_scaling.max_expansions changed after round-trip"
    );
}

/// `OperatorConfig::load_from_file` accepts and parses a valid minimal YAML file.
#[test]
fn operator_config_loads_minimal_valid_yaml() {
    use std::io::Write;

    let yaml = "diskScaling:\n  enabled: false\n  expansionThreshold: 90\n";
    let mut tmp =
        tempfile::NamedTempFile::new().expect("failed to create temp file for config test");
    tmp.write_all(yaml.as_bytes())
        .expect("failed to write temp config");

    let cfg = OperatorConfig::load_from_file(
        tmp.path()
            .to_str()
            .expect("temp file path is not valid UTF-8"),
    );

    assert!(
        !cfg.disk_scaling.enabled,
        "expected disk scaling disabled per YAML"
    );
    assert_eq!(
        cfg.disk_scaling.expansion_threshold, 90,
        "expected expansion_threshold=90 per YAML"
    );
}
