//! Unit tests for Pod Security Standard validation and security context helpers.

use super::pss::*;
use crate::crd::types::{ContainerCapabilities, SeccompProfileOverride, StellarSecurityContext};

// ── validate_stellar_security_context ────────────────────────────────────────

#[test]
fn compliant_default_ctx_has_no_violations() {
    let ctx = StellarSecurityContext {
        run_as_non_root: Some(true),
        allow_privilege_escalation: Some(false),
        read_only_root_filesystem: Some(true),
        capabilities: Some(ContainerCapabilities {
            drop: vec!["ALL".to_string()],
            add: vec![],
        }),
        seccomp_profile: Some(SeccompProfileOverride {
            type_: "RuntimeDefault".to_string(),
            localhost_profile: None,
        }),
        ..Default::default()
    };
    let violations = validate_stellar_security_context(&ctx);
    assert!(violations.is_empty(), "expected no violations, got: {violations:?}");
}

#[test]
fn privileged_true_is_a_violation() {
    let ctx = StellarSecurityContext {
        privileged: Some(true),
        ..Default::default()
    };
    let v = validate_stellar_security_context(&ctx);
    assert!(v.iter().any(|x| x.field.contains("privileged")));
}

#[test]
fn allow_privilege_escalation_true_is_a_violation() {
    let ctx = StellarSecurityContext {
        allow_privilege_escalation: Some(true),
        ..Default::default()
    };
    let v = validate_stellar_security_context(&ctx);
    assert!(v.iter().any(|x| x.field.contains("allowPrivilegeEscalation")));
}

#[test]
fn run_as_non_root_false_is_a_violation() {
    let ctx = StellarSecurityContext {
        run_as_non_root: Some(false),
        ..Default::default()
    };
    let v = validate_stellar_security_context(&ctx);
    assert!(v.iter().any(|x| x.field.contains("runAsNonRoot")));
}

#[test]
fn run_as_user_zero_is_a_violation() {
    let ctx = StellarSecurityContext {
        run_as_user: Some(0),
        ..Default::default()
    };
    let v = validate_stellar_security_context(&ctx);
    assert!(v.iter().any(|x| x.field.contains("runAsUser")));
}

#[test]
fn forbidden_capability_is_a_violation() {
    let ctx = StellarSecurityContext {
        capabilities: Some(ContainerCapabilities {
            add: vec!["SYS_ADMIN".to_string()],
            drop: vec![],
        }),
        ..Default::default()
    };
    let v = validate_stellar_security_context(&ctx);
    assert!(v.iter().any(|x| x.message.contains("SYS_ADMIN")));
}

#[test]
fn unconfined_seccomp_is_a_violation() {
    let ctx = StellarSecurityContext {
        seccomp_profile: Some(SeccompProfileOverride {
            type_: "Unconfined".to_string(),
            localhost_profile: None,
        }),
        ..Default::default()
    };
    let v = validate_stellar_security_context(&ctx);
    assert!(v.iter().any(|x| x.field.contains("seccompProfile")));
}

#[test]
fn non_forbidden_capability_add_is_allowed() {
    let ctx = StellarSecurityContext {
        capabilities: Some(ContainerCapabilities {
            add: vec!["NET_BIND_SERVICE".to_string()],
            drop: vec!["ALL".to_string()],
        }),
        ..Default::default()
    };
    let v = validate_stellar_security_context(&ctx);
    assert!(v.is_empty(), "NET_BIND_SERVICE should be allowed");
}

// ── build_container_security_context ─────────────────────────────────────────

#[test]
fn default_container_sc_is_restricted() {
    let sc = build_container_security_context(None);
    assert_eq!(sc.run_as_non_root, Some(true));
    assert_eq!(sc.allow_privilege_escalation, Some(false));
    assert_eq!(sc.read_only_root_filesystem, Some(true));
    assert_eq!(sc.privileged, Some(false));
}

#[test]
fn override_read_only_root_filesystem_is_respected() {
    let ctx = StellarSecurityContext {
        read_only_root_filesystem: Some(false),
        ..Default::default()
    };
    let sc = build_container_security_context(Some(&ctx));
    assert_eq!(sc.read_only_root_filesystem, Some(false));
    // Other secure defaults should still be in place
    assert_eq!(sc.allow_privilege_escalation, Some(false));
}

#[test]
fn override_run_as_user_is_applied() {
    let ctx = StellarSecurityContext {
        run_as_user: Some(65534),
        ..Default::default()
    };
    let sc = build_container_security_context(Some(&ctx));
    assert_eq!(sc.run_as_user, Some(65534));
}

// ── build_pod_security_context ────────────────────────────────────────────────

#[test]
fn default_pod_sc_sets_non_root_and_fs_group() {
    let psc = build_pod_security_context(None);
    assert_eq!(psc.run_as_non_root, Some(true));
    assert_eq!(psc.fs_group, Some(10000));
}

#[test]
fn override_fs_group_is_respected() {
    let ctx = StellarSecurityContext {
        fs_group: Some(1000),
        ..Default::default()
    };
    let psc = build_pod_security_context(Some(&ctx));
    assert_eq!(psc.fs_group, Some(1000));
    assert_eq!(psc.run_as_non_root, Some(true));
}

// ── restricted helpers ────────────────────────────────────────────────────────

#[test]
fn restricted_container_sc_drops_all_capabilities() {
    let sc = restricted_container_security_context();
    let caps = sc.capabilities.expect("capabilities must be set");
    let drop = caps.drop.expect("drop must be set");
    assert!(drop.contains(&"ALL".to_string()));
    assert!(caps.add.is_none());
}

#[test]
fn restricted_pod_sc_has_runtime_default_seccomp() {
    let psc = restricted_pod_security_context();
    let seccomp = psc.seccomp_profile.expect("seccompProfile must be set");
    assert_eq!(seccomp.type_, "RuntimeDefault");
}
