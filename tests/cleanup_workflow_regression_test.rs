//! Regression coverage for cleanup-sensitive test workflows.
//!
//! These tests verify that teardown guards keep issuing the expected kubectl
//! cleanup commands (and in the expected order) when dropped.

mod common;

use common::{E2eTestGuard, ManifestGuard, NamespaceGuard, StellarNodeGuard};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn env_lock() -> &'static Mutex<()> {
    ENV_LOCK.get_or_init(|| Mutex::new(()))
}

fn unique_test_dir() -> PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock must be after unix epoch")
        .as_nanos();
    env::temp_dir().join(format!("stellar-k8s-cleanup-regression-{now}"))
}

#[cfg(unix)]
fn write_fake_kubectl(bin_path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let script = r#"#!/usr/bin/env sh
echo "$*" >> "$SK8S_KUBECTL_LOG"
cat >/dev/null
exit 0
"#;
    fs::write(bin_path, script).expect("failed to write fake kubectl script");
    let perms = fs::Permissions::from_mode(0o755);
    fs::set_permissions(bin_path, perms).expect("failed to make fake kubectl executable");
}

#[cfg(windows)]
fn write_fake_kubectl(bin_path: &Path) {
    let script = "@echo off\r\necho %*>> \"%SK8S_KUBECTL_LOG%\"\r\nexit /b 0\r\n";
    fs::write(bin_path, script).expect("failed to write fake kubectl script");
}

fn fake_kubectl_name() -> &'static str {
    #[cfg(windows)]
    {
        "kubectl.cmd"
    }
    #[cfg(not(windows))]
    {
        "kubectl"
    }
}

fn with_fake_kubectl<T>(f: impl FnOnce(&Path) -> T) -> T {
    let _guard = env_lock()
        .lock()
        .expect("failed to acquire environment lock");
    let test_dir = unique_test_dir();
    fs::create_dir_all(&test_dir).expect("failed to create temp test directory");

    let log_path = test_dir.join("kubectl.log");
    let kubectl_path = test_dir.join(fake_kubectl_name());
    write_fake_kubectl(&kubectl_path);

    let old_path = env::var_os("PATH");
    let mut new_path = std::ffi::OsString::from(test_dir.as_os_str());
    new_path.push(if cfg!(windows) { ";" } else { ":" });
    if let Some(p) = &old_path {
        new_path.push(p);
    }

    env::set_var("SK8S_KUBECTL_LOG", &log_path);
    env::set_var("PATH", new_path);

    let result = f(&log_path);

    if let Some(p) = old_path {
        env::set_var("PATH", p);
    } else {
        env::remove_var("PATH");
    }
    env::remove_var("SK8S_KUBECTL_LOG");
    let _ = fs::remove_dir_all(&test_dir);

    result
}

fn read_log_lines(log_path: &Path) -> Vec<String> {
    let content = fs::read_to_string(log_path).expect("failed to read fake kubectl log");
    content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

#[test]
fn e2e_test_guard_drop_preserves_cleanup_order() {
    with_fake_kubectl(|log_path| {
        let guard = E2eTestGuard::new()
            .track_node("node-a", "stellar-a")
            .track_operator_manifest("apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: test\n")
            .track_namespace("stellar-a")
            .track_namespace("stellar-b");
        drop(guard);

        let lines = read_log_lines(log_path);
        assert_eq!(lines.len(), 4, "expected 4 cleanup kubectl invocations");
        assert_eq!(
            lines[0],
            "delete stellarnode node-a -n stellar-a --ignore-not-found=true --timeout=60s --wait=true"
        );
        assert_eq!(lines[1], "delete -f - --ignore-not-found=true");
        assert_eq!(
            lines[2],
            "delete namespace stellar-a --ignore-not-found=true"
        );
        assert_eq!(
            lines[3],
            "delete namespace stellar-b --ignore-not-found=true"
        );
    });
}

#[test]
fn drop_guards_keep_cleanup_flags_for_safe_teardown() {
    with_fake_kubectl(|log_path| {
        drop(NamespaceGuard {
            name: "ns-guard".to_string(),
        });
        drop(StellarNodeGuard::new("node-guard", "ns-guard"));
        drop(ManifestGuard::new(
            "apiVersion: v1\nkind: Service\nmetadata:\n  name: svc\n",
        ));

        let lines = read_log_lines(log_path);
        assert_eq!(lines.len(), 3, "expected three guard cleanup invocations");
        assert_eq!(
            lines[0],
            "delete namespace ns-guard --ignore-not-found=true --wait=false"
        );
        assert_eq!(
            lines[1],
            "delete stellarnode node-guard -n ns-guard --ignore-not-found=true --timeout=60s --wait=true"
        );
        assert_eq!(lines[2], "delete -f - --ignore-not-found=true");
    });
}
