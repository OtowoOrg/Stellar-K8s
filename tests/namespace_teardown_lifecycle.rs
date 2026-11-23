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
//! Namespace teardown lifecycle tests (#1978): archive checksum gate,
//! credential-revocation deadline, stuck-step conditions, and a 100-namespace
//! parallel create/destroy audit for orphans.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use sha2::{Digest, Sha256};
use stellar_k8s::controller::namespace_teardown::{
    advance, audit_orphans, TeardownAttestation, TeardownBackend, REVOCATION_CONFIRM_DEADLINE_SECS,
};
use stellar_k8s::crd::namespace_teardown::{
    ArchiveSpec, NamespaceTeardownSpec, NamespaceTeardownStatus, TeardownPhase, TeardownStepStatus,
};

#[derive(Default)]
struct World {
    namespaces: HashSet<String>,
    dns: HashSet<String>,
    creds: HashSet<String>,
    attestations: Vec<String>,
    archive_checksum: String,
    /// When true, revoke is a no-op (credential store never confirms).
    revocation_stuck: bool,
}

#[derive(Clone, Default)]
struct FakeBackend(Arc<Mutex<World>>);

#[async_trait]
impl TeardownBackend for FakeBackend {
    async fn archive(&self, _ns: &str, _a: &ArchiveSpec) -> Result<String, String> {
        Ok(self.0.lock().unwrap().archive_checksum.clone())
    }
    async fn revoke_credential(&self, ns: &str, c: &str) -> Result<(), String> {
        let mut w = self.0.lock().unwrap();
        if !w.revocation_stuck {
            w.creds.remove(&format!("{ns}/{c}"));
        }
        Ok(())
    }
    async fn credential_live(&self, ns: &str, c: &str) -> Result<bool, String> {
        Ok(self.0.lock().unwrap().creds.contains(&format!("{ns}/{c}")))
    }
    async fn delete_dns_record(&self, ns: &str, r: &str) -> Result<(), String> {
        self.0.lock().unwrap().dns.remove(&format!("{ns}/{r}"));
        Ok(())
    }
    async fn dns_record_exists(&self, ns: &str, r: &str) -> Result<bool, String> {
        Ok(self.0.lock().unwrap().dns.contains(&format!("{ns}/{r}")))
    }
    async fn write_attestation(&self, _s: &str, a: &TeardownAttestation) -> Result<(), String> {
        self.0
            .lock()
            .unwrap()
            .attestations
            .push(a.target_namespace.clone());
        Ok(())
    }
    async fn delete_namespace(&self, ns: &str) -> Result<(), String> {
        self.0.lock().unwrap().namespaces.remove(ns);
        Ok(())
    }
    async fn namespace_exists(&self, ns: &str) -> Result<bool, String> {
        Ok(self.0.lock().unwrap().namespaces.contains(ns))
    }
}

fn checksum() -> String {
    hex::encode(Sha256::digest(b"archive"))
}

fn spec(ns: &str) -> NamespaceTeardownSpec {
    NamespaceTeardownSpec {
        target_namespace: ns.to_string(),
        archive: Some(ArchiveSpec {
            destination: "s3://backups/x".to_string(),
            expected_checksum: Some(checksum()),
            verify: true,
        }),
        credential_refs: vec!["db".to_string(), "api".to_string()],
        dns_records: vec![format!("{ns}.example.org")],
        attestation_sink: Some("attest".to_string()),
        step_timeout_seconds: 0,
    }
}

fn seed(world: &FakeBackend, s: &NamespaceTeardownSpec) {
    let mut w = world.0.lock().unwrap();
    w.archive_checksum = checksum();
    let ns = &s.target_namespace;
    w.namespaces.insert(ns.clone());
    for c in &s.credential_refs {
        w.creds.insert(format!("{ns}/{c}"));
    }
    for r in &s.dns_records {
        w.dns.insert(format!("{ns}/{r}"));
    }
}

async fn run_to_end(
    s: &NamespaceTeardownSpec,
    b: &FakeBackend,
    mut status: NamespaceTeardownStatus,
) -> NamespaceTeardownStatus {
    let now = Utc::now();
    for _ in 0..20 {
        if NamespaceTeardownSpec::is_terminal(&status.phase) {
            break;
        }
        status = advance(s, &status, b, now).await;
    }
    status
}

#[tokio::test]
async fn happy_path_completes_with_no_orphans() {
    let b = FakeBackend::default();
    let s = spec("team-a");
    seed(&b, &s);
    let status = run_to_end(&s, &b, NamespaceTeardownStatus::default()).await;
    assert_eq!(status.phase, TeardownPhase::Complete);
    assert_eq!(
        status.archive_checksum.as_deref(),
        Some(checksum().as_str())
    );
    assert!(status.completed_at.is_some());
    assert!(audit_orphans(&s, &b).await.is_empty());
    assert_eq!(b.0.lock().unwrap().attestations, vec!["team-a"]);
}

#[tokio::test]
async fn checksum_mismatch_blocks_namespace_delete() {
    let b = FakeBackend::default();
    let s = spec("team-b");
    seed(&b, &s);
    b.0.lock().unwrap().archive_checksum = hex::encode(Sha256::digest(b"tampered"));
    let status = run_to_end(&s, &b, NamespaceTeardownStatus::default()).await;
    assert_eq!(status.phase, TeardownPhase::Failed);
    let cond = status
        .conditions
        .iter()
        .find(|c| c.step == "Archiving")
        .unwrap();
    assert_eq!(cond.reason.as_deref(), Some("ChecksumMismatch"));
    assert!(
        b.0.lock().unwrap().namespaces.contains("team-b"),
        "namespace must survive"
    );
}

#[tokio::test]
async fn unconfirmed_revocation_fails_after_deadline_with_actionable_condition() {
    let b = FakeBackend::default();
    let s = spec("team-c");
    seed(&b, &s);
    b.0.lock().unwrap().revocation_stuck = true;
    let t0 = Utc::now();
    let mut status = NamespaceTeardownStatus::default();
    // Reach RevokingCredentials.
    while status.phase != TeardownPhase::RevokingCredentials {
        status = advance(&s, &status, &b, t0).await;
    }
    status = advance(&s, &status, &b, t0).await;
    let cond = status
        .conditions
        .iter()
        .find(|c| c.step == "RevokingCredentials")
        .unwrap();
    assert_eq!(cond.status, TeardownStepStatus::Running);
    assert_eq!(
        cond.reason.as_deref(),
        Some("AwaitingRevocationConfirmation")
    );

    let later = t0 + Duration::seconds(REVOCATION_CONFIRM_DEADLINE_SECS + 1);
    status = advance(&s, &status, &b, later).await;
    assert_eq!(status.phase, TeardownPhase::Failed);
    let cond = status
        .conditions
        .iter()
        .find(|c| c.step == "RevokingCredentials")
        .unwrap();
    assert_eq!(cond.reason.as_deref(), Some("RevocationNotConfirmed"));
    assert!(cond.message.as_deref().unwrap().contains("db"));
}

#[tokio::test]
async fn stuck_namespace_deletion_times_out_with_step_timed_out() {
    // Namespace deletion never completes (blocking finalizer).
    #[derive(Clone)]
    struct Stuck(FakeBackend);
    #[async_trait]
    impl TeardownBackend for Stuck {
        async fn archive(&self, n: &str, a: &ArchiveSpec) -> Result<String, String> {
            self.0.archive(n, a).await
        }
        async fn revoke_credential(&self, n: &str, c: &str) -> Result<(), String> {
            self.0.revoke_credential(n, c).await
        }
        async fn credential_live(&self, n: &str, c: &str) -> Result<bool, String> {
            self.0.credential_live(n, c).await
        }
        async fn delete_dns_record(&self, n: &str, r: &str) -> Result<(), String> {
            self.0.delete_dns_record(n, r).await
        }
        async fn dns_record_exists(&self, n: &str, r: &str) -> Result<bool, String> {
            self.0.dns_record_exists(n, r).await
        }
        async fn write_attestation(&self, s: &str, a: &TeardownAttestation) -> Result<(), String> {
            self.0.write_attestation(s, a).await
        }
        async fn delete_namespace(&self, _n: &str) -> Result<(), String> {
            Ok(())
        }
        async fn namespace_exists(&self, _n: &str) -> Result<bool, String> {
            Ok(true)
        }
    }

    let inner = FakeBackend::default();
    let s = spec("team-d");
    seed(&inner, &s);
    let b = Stuck(inner);
    let t0 = Utc::now();
    let mut status = NamespaceTeardownStatus::default();
    while status.phase != TeardownPhase::Deleting {
        status = advance(&s, &status, &b, t0).await;
    }
    status = advance(&s, &status, &b, t0).await;
    let cond = status
        .conditions
        .iter()
        .find(|c| c.step == "Deleting")
        .unwrap();
    assert_eq!(cond.reason.as_deref(), Some("NamespaceTerminating"));

    status = advance(&s, &status, &b, t0 + Duration::seconds(301)).await;
    assert_eq!(status.phase, TeardownPhase::Failed);
    let cond = status
        .conditions
        .iter()
        .find(|c| c.step == "Deleting")
        .unwrap();
    assert_eq!(cond.reason.as_deref(), Some("StepTimedOut"));
    assert!(cond.message.as_deref().unwrap().contains("re-create"));
}

#[tokio::test]
async fn hundred_parallel_teardowns_leave_no_orphans() {
    let b = FakeBackend::default();
    let specs: Vec<_> = (0..100).map(|i| spec(&format!("tenant-{i}"))).collect();
    for s in &specs {
        seed(&b, s);
    }
    let mut tasks = Vec::new();
    for s in specs.clone() {
        let b = b.clone();
        tasks.push(tokio::spawn(async move {
            run_to_end(&s, &b, NamespaceTeardownStatus::default()).await
        }));
    }
    for t in tasks {
        assert_eq!(t.await.unwrap().phase, TeardownPhase::Complete);
    }
    for s in &specs {
        assert!(
            audit_orphans(s, &b).await.is_empty(),
            "orphans for {}",
            s.target_namespace
        );
    }
    let w = b.0.lock().unwrap();
    assert!(w.namespaces.is_empty() && w.dns.is_empty() && w.creds.is_empty());
    assert_eq!(w.attestations.len(), 100);
}
