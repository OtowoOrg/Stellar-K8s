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

//! Integration tests for the audit ledger: hashing, verification (red-team tamper cases),
//! the append-only writer, the webhook sink and the incremental monitor.

use serde_json::{json, Value};
use std::io::Cursor;
use std::sync::atomic::Ordering;
use stellar_k8s::audit_ledger::monitor::ChainMonitor;
use stellar_k8s::audit_ledger::sink::{
    normalize, spawn_sink, spawn_with_writer, NormalizeConfig, SinkConfig, SinkHandle, SinkMetrics,
    SubmitError,
};
use stellar_k8s::audit_ledger::store::LedgerWriter;
use stellar_k8s::audit_ledger::verify::{
    verify_reader, ChainVerifier, VerifyOptions, ViolationKind,
};
use stellar_k8s::audit_ledger::*;

fn ev(i: u64) -> Value {
    json!({
        "auditID": format!("id-{i}"),
        "stage": "ResponseComplete",
        "verb": "create",
        "user": {"username": "alice", "groups": ["dev"]},
        "objectRef": {"resource": "pods", "namespace": "ns", "name": format!("p{i}")},
        "responseStatus": {"code": 201},
    })
}

/// Build an in-memory archive of `n` entries as a list of lines (header first).
fn lines(n: u64) -> Vec<Vec<u8>> {
    let mut v = vec![encode_header("test")];
    let mut prev = genesis_hash("test");
    for i in 0..n {
        let (l, h) = encode_entry(i, &prev, &ev(i)).unwrap();
        v.push(l);
        prev = h;
    }
    v
}

fn verify(lines: &[Vec<u8>]) -> stellar_k8s::audit_ledger::verify::VerifyReport {
    verify_reader(
        Cursor::new(lines.concat()),
        &VerifyOptions {
            threads: 2,
            ..Default::default()
        },
    )
    .unwrap()
}

fn rehash_entry(seq: u64, prev: &Hash, e: &Value) -> Vec<u8> {
    encode_entry(seq, prev, e).unwrap().0
}

// ── genesis & continuity ──────────────────────────────────────────────

#[test]
fn valid_chain_verifies_with_genesis_and_continuity() {
    let r = verify(&lines(50));
    assert!(r.valid, "{:?}", r.violation);
    assert_eq!(
        (r.entries, r.first_seq, r.last_seq),
        (50, Some(0), Some(49))
    );
    assert_eq!(r.chain_id.as_deref(), Some("test"));
}

#[test]
fn empty_chain_is_valid() {
    let r = verify(&lines(0));
    assert!(r.valid);
    assert_eq!(r.entries, 0);
}

#[test]
fn first_entry_must_chain_from_genesis() {
    let mut l = lines(3);
    // Re-chain entry 0 from a wrong predecessor but keep it self-consistent.
    l[1] = rehash_entry(0, &genesis_hash("other"), &ev(0));
    let r = verify(&l);
    let v = r.violation.unwrap();
    assert_eq!(v.kind, ViolationKind::GenesisMismatch);
    assert_eq!(v.seq, Some(0));
}

#[test]
fn chain_verifier_tracks_expected_seq_and_head() {
    let mut cv = ChainVerifier::from_genesis("test");
    let mut prev = genesis_hash("test");
    for i in 0..3 {
        let (l, h) = encode_entry(i, &prev, &ev(i)).unwrap();
        let e =
            stellar_k8s::audit_ledger::verify::parse_entry_line(i + 2, &l[..l.len() - 1]).unwrap();
        cv.check(&e).unwrap();
        prev = h;
    }
    assert_eq!(cv.expected_seq(), 3);
    assert_eq!(cv.head(), prev);
    assert_eq!(cv.last_seq(), Some(2));
}

// ── red-team: tamper classes, exact sequence number ───────────────────

#[test]
fn deleted_event_is_pinpointed_as_missing() {
    for victim in [0usize, 1, 5, 8] {
        let mut l = lines(10);
        l.remove(victim + 1); // +1 for header
        let r = verify(&l);
        let v = r.violation.expect("deletion must be detected");
        assert_eq!(v.kind, ViolationKind::MissingEntries, "victim {victim}");
        assert_eq!(v.seq, Some(victim as u64), "exact first-missing seq");
        assert_eq!(v.missing_to, Some(victim as u64));
        assert_eq!(r.entries, victim as u64, "verified prefix");
    }
}

#[test]
fn deleting_several_adjacent_events_reports_the_range() {
    let mut l = lines(10);
    l.drain(4..7); // seqs 3,4,5
    let v = verify(&l).violation.unwrap();
    assert_eq!(
        (v.kind, v.seq, v.missing_to),
        (ViolationKind::MissingEntries, Some(3), Some(5))
    );
}

#[test]
fn deleting_the_last_event_is_caught_by_footer_or_pinned_head() {
    let full = lines(5);
    let head = verify(&full).head_hash.unwrap();
    let mut footered = full.clone();
    footered.push(encode_footer(5, Some(4), &head));
    assert!(verify(&footered).valid);
    // Drop seq 4 but keep the footer.
    footered.remove(5);
    let v = verify(&footered).violation.unwrap();
    assert_eq!(v.kind, ViolationKind::FooterMismatch);
    assert_eq!(v.seq, Some(4));
    // Truncate the tail with an externally pinned head.
    let mut t = full.clone();
    t.pop();
    let r = verify_reader(
        Cursor::new(t.concat()),
        &VerifyOptions {
            expect_head: Some(head),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(r.violation.unwrap().kind, ViolationKind::HeadMismatch);
}

#[test]
fn modified_event_is_pinpointed() {
    let mut l = lines(10);
    let s = String::from_utf8(l[5].clone())
        .unwrap()
        .replace("\"alice\"", "\"mallory\"");
    l[5] = s.into_bytes();
    let v = verify(&l).violation.unwrap();
    assert_eq!(v.kind, ViolationKind::ModifiedEntry);
    assert_eq!(v.seq, Some(4)); // lines[5] is seq 4
    assert!(v.expected.is_some() && v.actual.is_some());
}

#[test]
fn modified_event_with_recomputed_hash_breaks_the_next_link() {
    // A smarter attacker rewrites seq 4 and recomputes its own hash.
    let mut l = lines(10);
    let mut prev = genesis_hash("test");
    for i in 0..4 {
        prev = encode_entry(i, &prev, &ev(i)).unwrap().1;
    }
    let mut forged = ev(4);
    forged["user"]["username"] = json!("mallory");
    l[5] = rehash_entry(4, &prev, &forged);
    let v = verify(&l).violation.unwrap();
    assert_eq!(v.kind, ViolationKind::PredecessorMismatch);
    assert_eq!(v.seq, Some(4), "suspect entry is the replaced predecessor");
    assert_eq!(v.line, 7);
}

#[test]
fn reordered_events_are_detected() {
    let mut l = lines(10);
    l.swap(4, 5); // swap seq 3 and 4
    let v = verify(&l).violation.unwrap();
    assert_eq!(v.kind, ViolationKind::ReorderedEntry);
    assert_eq!(v.seq, Some(3));
}

#[test]
fn duplicate_event_is_detected() {
    let mut l = lines(10);
    let dup = l[4].clone();
    l.insert(5, dup);
    let v = verify(&l).violation.unwrap();
    assert_eq!(v.kind, ViolationKind::DuplicateEntry);
    assert_eq!(v.seq, Some(3));
}

#[test]
fn replayed_old_entry_is_detected_as_out_of_order() {
    let mut l = lines(10);
    let old = l[2].clone();
    l.insert(8, old);
    let v = verify(&l).violation.unwrap();
    assert_eq!(v.kind, ViolationKind::ReorderedEntry);
}

#[test]
fn inserted_forged_event_is_detected() {
    let mut l = lines(10);
    let forged = rehash_entry(5, &genesis_hash("test"), &ev(99));
    l.insert(6, forged);
    let v = verify(&l).violation.unwrap();
    assert!(
        matches!(
            v.kind,
            ViolationKind::ReorderedEntry
                | ViolationKind::PredecessorMismatch
                | ViolationKind::DuplicateEntry
        ),
        "{v:?}"
    );
}

#[test]
fn torn_final_line_unknown_fields_and_bad_header_are_rejected() {
    let mut l = lines(3);
    let last = l.last_mut().unwrap();
    last.pop(); // drop trailing newline => torn write
    assert_eq!(verify(&l).violation.unwrap().kind, ViolationKind::Malformed);

    let mut l = lines(3);
    let s = String::from_utf8(l[2].clone())
        .unwrap()
        .replacen("{\"seq\"", "{\"x\":1,\"seq\"", 1);
    l[2] = s.into_bytes();
    assert_eq!(verify(&l).violation.unwrap().kind, ViolationKind::Malformed);

    let mut l = lines(3);
    l[0] = String::from_utf8(l[0].clone())
        .unwrap()
        .replace(&hex::encode(genesis_hash("test")), &"0".repeat(64))
        .into_bytes();
    assert_eq!(
        verify(&l).violation.unwrap().kind,
        ViolationKind::HeaderInvalid
    );

    let r = verify_reader(Cursor::new(Vec::new()), &VerifyOptions::default()).unwrap();
    assert_eq!(r.violation.unwrap().kind, ViolationKind::HeaderInvalid);
}

#[test]
fn chain_id_pinning() {
    let r = verify_reader(
        Cursor::new(lines(2).concat()),
        &VerifyOptions {
            expect_chain_id: Some("prod".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(r.violation.unwrap().kind, ViolationKind::HeaderInvalid);
}

#[test]
fn verification_is_deterministic_across_thread_counts_and_chunking() {
    let mut l = lines(5000);
    l.remove(3001);
    let bytes = l.concat();
    let a = verify_reader(
        Cursor::new(bytes.clone()),
        &VerifyOptions {
            threads: 1,
            ..Default::default()
        },
    )
    .unwrap();
    let b = verify_reader(
        Cursor::new(bytes),
        &VerifyOptions {
            threads: 7,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(a, b);
    assert_eq!(a.violation.unwrap().seq, Some(3000));
}

// ── writer: durability, resume, dedupe, failure ───────────────────────

#[test]
fn writer_resumes_chain_after_restart_and_output_verifies() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("l.ndjson");
    {
        let mut w = LedgerWriter::open(&p, "test", 1000).unwrap();
        assert_eq!(w.append(&[ev(0), ev(1), ev(2)]).unwrap().appended, 3);
    }
    {
        let mut w = LedgerWriter::open(&p, "test", 1000).unwrap();
        assert_eq!(w.next_seq(), 3);
        assert_eq!(w.append(&[ev(3)]).unwrap().appended, 1);
    }
    let r = stellar_k8s::audit_ledger::verify::verify_file(&p, &VerifyOptions::default()).unwrap();
    assert!(r.valid, "{:?}", r.violation);
    assert_eq!(r.entries, 4);
}

#[test]
fn writer_dedupes_redelivery_within_and_across_restarts() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("l.ndjson");
    let mut w = LedgerWriter::open(&p, "test", 1000).unwrap();
    let o = w.append(&[ev(0), ev(1), ev(0)]).unwrap();
    assert_eq!((o.appended, o.duplicates), (2, 1));
    let o = w.append(&[ev(1)]).unwrap();
    assert_eq!((o.appended, o.duplicates), (0, 1));
    drop(w);
    let mut w = LedgerWriter::open(&p, "test", 1000).unwrap();
    let o = w.append(&[ev(1), ev(2)]).unwrap();
    assert_eq!(
        (o.appended, o.duplicates),
        (1, 1),
        "dedupe window is rebuilt from the tail"
    );
    // Same auditID, different stage is a distinct event.
    let mut other = ev(2);
    other["stage"] = json!("ResponseStarted");
    assert_eq!(w.append(&[other]).unwrap().appended, 1);
    assert!(
        stellar_k8s::audit_ledger::verify::verify_file(&p, &VerifyOptions::default())
            .unwrap()
            .valid
    );
}

#[test]
fn rolled_back_batch_can_be_retried_and_is_not_deduped() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("l.ndjson");
    let mut w = LedgerWriter::open(&p, "test", 1000).unwrap();
    w.append(&[ev(0)]).unwrap();
    let len = std::fs::metadata(&p).unwrap().len();
    assert_eq!(w.stage(&[ev(1), ev(2)]).unwrap().appended, 2);
    w.rollback().unwrap();
    assert_eq!(
        std::fs::metadata(&p).unwrap().len(),
        len,
        "uncommitted bytes removed"
    );
    assert_eq!(w.next_seq(), 1);
    assert_eq!(
        w.append(&[ev(1), ev(2)]).unwrap().appended,
        2,
        "retry is not treated as duplicate"
    );
    assert!(
        stellar_k8s::audit_ledger::verify::verify_file(&p, &VerifyOptions::default())
            .unwrap()
            .valid
    );
}

#[test]
fn writer_discards_torn_tail_on_recovery_and_refuses_foreign_chain() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("l.ndjson");
    {
        let mut w = LedgerWriter::open(&p, "test", 100).unwrap();
        w.append(&[ev(0), ev(1)]).unwrap();
    }
    std::fs::OpenOptions::new()
        .append(true)
        .open(&p)
        .unwrap()
        .write_all(b"{\"seq\":2,\"prevH")
        .unwrap();
    let mut w = LedgerWriter::open(&p, "test", 100).unwrap();
    assert_eq!(w.next_seq(), 2);
    w.append(&[ev(2)]).unwrap();
    assert!(
        stellar_k8s::audit_ledger::verify::verify_file(&p, &VerifyOptions::default())
            .unwrap()
            .valid
    );
    drop(w);
    assert!(matches!(
        LedgerWriter::open(&p, "other", 100),
        Err(stellar_k8s::audit_ledger::store::LedgerError::ChainIdMismatch { .. })
    ));
}

#[cfg(unix)]
#[test]
fn write_failure_is_reported_and_state_is_not_advanced() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("l.ndjson");
    let mut w = LedgerWriter::open(&p, "test", 100).unwrap();
    w.append(&[ev(0)]).unwrap();
    w.replace_file_for_test(
        std::fs::OpenOptions::new()
            .write(true)
            .open("/dev/full")
            .unwrap(),
    );
    assert!(
        w.append(&[ev(1)]).is_err(),
        "ENOSPC must surface, not be swallowed"
    );
    assert_eq!(w.next_seq(), 1);
}

#[test]
fn non_integer_numbers_are_rejected_not_silently_altered() {
    let dir = tempfile::tempdir().unwrap();
    let mut w = LedgerWriter::open(dir.path().join("l"), "test", 10).unwrap();
    let mut e = ev(0);
    e["x"] = json!(1.5);
    assert!(w.append(&[e]).is_err());
    assert_eq!(w.next_seq(), 0);
}

// ── normalization / privacy ───────────────────────────────────────────

#[test]
fn normalize_keeps_mutations_and_strips_bodies_and_query() {
    let raw = json!({
        "auditID": "a1", "stage": "ResponseComplete", "verb": "patch", "level": "RequestResponse",
        "requestURI": "/api/v1/namespaces/x/secrets/s?token=abc",
        "user": {"username": "bob", "groups": ["g"], "extra": {"k": ["v"]}},
        "sourceIPs": ["10.0.0.1"],
        "objectRef": {"resource": "secrets", "namespace": "x", "name": "s", "uid": "u"},
        "responseStatus": {"code": 200, "message": "m"},
        "requestObject": {"data": {"password": "hunter2"}},
        "responseObject": {"data": {"password": "hunter2"}},
        "annotations": {"authorization.k8s.io/reason": "r"},
        "requestReceivedTimestamp": "2026-01-01T00:00:00.000000Z",
        "stageTimestamp": "2026-01-01T00:00:00.001000Z",
    });
    let n = normalize(&raw, &NormalizeConfig::default())
        .unwrap()
        .unwrap();
    let s = n.to_string();
    assert!(
        !s.contains("hunter2")
            && !s.contains("token=abc")
            && !s.contains("10.0.0.1")
            && !s.contains("annotations")
    );
    assert_eq!(n["verb"], "patch");
    assert_eq!(n["objectRef"]["resource"], "secrets");
    assert_eq!(n["responseStatus"]["code"], 200);
    assert_eq!(n["requestURI"], "/api/v1/namespaces/x/secrets/s");
    let with_ip = normalize(
        &raw,
        &NormalizeConfig {
            include_source_ips: true,
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(with_ip["sourceIPs"][0], "10.0.0.1");
}

#[test]
fn normalize_filters_reads_and_rejects_malformed() {
    let mut raw = ev(1);
    raw["verb"] = json!("get");
    assert!(normalize(&raw, &NormalizeConfig::default())
        .unwrap()
        .is_none());
    let mut bad = ev(1);
    bad.as_object_mut().unwrap().remove("auditID");
    assert!(normalize(&bad, &NormalizeConfig::default()).is_err());
    let mut bad = ev(1);
    bad["stage"] = json!("Nope");
    assert!(normalize(&bad, &NormalizeConfig::default()).is_err());
    let mut bad = ev(1);
    bad["auditID"] = json!("x".repeat(500));
    assert!(normalize(&bad, &NormalizeConfig::default()).is_err());
}

// ── sink service ──────────────────────────────────────────────────────

#[tokio::test]
async fn sink_counts_filtered_invalid_duplicates_and_persists() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("l.ndjson");
    let (sink, _j) = spawn_sink(SinkConfig::new(&p, "test")).unwrap();
    let mut read = ev(100);
    read["verb"] = json!("list");
    let mut bad = ev(101);
    bad["stage"] = json!("???");
    let (a, b, c) = (ev(1), ev(1), ev(2));
    let o = sink.ingest_raw(&[&a, &b, &c, &read, &bad]).await.unwrap();
    assert_eq!((o.appended, o.duplicates), (2, 1));
    let m = &sink.metrics;
    assert_eq!(m.events_received.load(Ordering::Relaxed), 5);
    assert_eq!(m.events_filtered.load(Ordering::Relaxed), 1);
    assert_eq!(m.events_invalid.load(Ordering::Relaxed), 1);
    assert_eq!(m.events_appended.load(Ordering::Relaxed), 2);
    assert_eq!(m.head_seq.load(Ordering::Relaxed), 1);
    let text = m.render();
    assert!(text.contains("stellar_audit_ledger_events_appended_total 2"));
    assert!(text.contains("stellar_audit_ledger_processing_latency_seconds_count 1"));
    assert!(
        stellar_k8s::audit_ledger::verify::verify_file(&p, &VerifyOptions::default())
            .unwrap()
            .valid
    );
}

#[tokio::test]
async fn concurrent_submitters_get_unique_gap_free_sequences() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("l.ndjson");
    let (sink, _j) = spawn_sink(SinkConfig::new(&p, "test")).unwrap();
    let mut tasks = Vec::new();
    for t in 0..8u64 {
        let s = sink.clone();
        tasks.push(tokio::spawn(async move {
            for i in 0..50u64 {
                loop {
                    match s.submit(vec![ev(t * 1000 + i)]).await {
                        Ok(_) => break,
                        Err(SubmitError::Backpressure) => tokio::task::yield_now().await,
                        Err(e) => panic!("{e}"),
                    }
                }
            }
        }));
    }
    for t in tasks {
        t.await.unwrap();
    }
    let r = stellar_k8s::audit_ledger::verify::verify_file(&p, &VerifyOptions::default()).unwrap();
    assert!(r.valid, "{:?}", r.violation);
    assert_eq!(r.entries, 400);
}

#[tokio::test]
async fn full_queue_applies_backpressure_instead_of_dropping() {
    let (tx, _rx_kept_but_never_read) = tokio::sync::mpsc::channel(1);
    let sink = SinkHandle {
        tx,
        metrics: SinkMetrics::new(),
        config: std::sync::Arc::new(SinkConfig::new("x", "test")),
    };
    let first = sink.clone();
    let _pending = tokio::spawn(async move { first.submit(vec![ev(1)]).await });
    tokio::task::yield_now().await;
    let r = sink.submit(vec![ev(2)]).await;
    assert!(matches!(r, Err(SubmitError::Backpressure)));
    assert_eq!(
        sink.metrics.requests_backpressured.load(Ordering::Relaxed),
        1
    );
}

#[cfg(unix)]
#[tokio::test]
async fn store_failure_is_surfaced_to_the_sender_and_metered() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("l.ndjson");
    let cfg = SinkConfig::new(&p, "test");
    let mut w = LedgerWriter::open(&p, "test", 10).unwrap();
    w.replace_file_for_test(
        std::fs::OpenOptions::new()
            .write(true)
            .open("/dev/full")
            .unwrap(),
    );
    let (sink, _j) = spawn_with_writer(cfg, w).unwrap();
    let r = sink.submit(vec![ev(1)]).await;
    assert!(matches!(r, Err(SubmitError::WriteFailed(_))));
    assert_eq!(sink.metrics.write_failures.load(Ordering::Relaxed), 1);
    assert_eq!(sink.metrics.sink_up.load(Ordering::Relaxed), 0);
    assert!(sink
        .metrics
        .render()
        .contains("stellar_audit_ledger_sink_up 0"));
}

// ── monitor: gap detection ────────────────────────────────────────────

#[test]
fn monitor_follows_appends_and_latches_on_deletion() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("l.ndjson");
    let m = SinkMetrics::new();
    let mut w = LedgerWriter::open(&p, "test", 100).unwrap();
    let mut mon = ChainMonitor::new(&p, m.clone());
    w.append(&(0..20).map(ev).collect::<Vec<_>>()).unwrap();
    assert!(mon.scan_once().unwrap().is_none());
    assert_eq!(m.verified_seq.load(Ordering::Relaxed), 19);
    w.append(&(20..30).map(ev).collect::<Vec<_>>()).unwrap();
    assert!(mon.scan_once().unwrap().is_none());
    assert_eq!(m.verified_seq.load(Ordering::Relaxed), 29);
    assert_eq!(m.chain_valid.load(Ordering::Relaxed), 1);

    // Attacker deletes one event in the middle of the store.
    let text = std::fs::read_to_string(&p).unwrap();
    let kept: Vec<&str> = text
        .lines()
        .enumerate()
        .filter(|(i, _)| *i != 11)
        .map(|(_, l)| l)
        .collect();
    std::fs::write(&p, kept.join("\n") + "\n").unwrap();
    let v = mon
        .scan_once()
        .unwrap()
        .expect("deletion must be detected on the next scan");
    assert!(
        matches!(
            v.kind,
            ViolationKind::ModifiedEntry | ViolationKind::Truncated | ViolationKind::MissingEntries
        ),
        "{v:?}"
    );
    assert_eq!(m.chain_valid.load(Ordering::Relaxed), 0);
    assert!(m.verification_failures.load(Ordering::Relaxed) >= 1);
}

#[test]
fn monitor_detects_a_gap_in_newly_appended_data_with_exact_seq() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("l.ndjson");
    let m = SinkMetrics::new();
    let mut w = LedgerWriter::open(&p, "test", 100).unwrap();
    w.append(&(0..5).map(ev).collect::<Vec<_>>()).unwrap();
    let mut mon = ChainMonitor::new(&p, m.clone());
    assert!(mon.scan_once().unwrap().is_none());
    // Append entries 7.. directly (5 and 6 never arrive).
    let mut prev = genesis_hash("test");
    let mut all = Vec::new();
    for i in 0..8 {
        let (l, h) = encode_entry(i, &prev, &ev(i)).unwrap();
        all.push(l);
        prev = h;
    }
    let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
    f.write_all(&all[7]).unwrap();
    let v = mon.scan_once().unwrap().unwrap();
    assert_eq!(
        (v.kind, v.seq, v.missing_to),
        (ViolationKind::MissingEntries, Some(5), Some(6))
    );
    assert_eq!(m.chain_gaps.load(Ordering::Relaxed), 1);
}

#[test]
fn monitor_full_verify_catches_same_length_rewrite_deep_in_history() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("l.ndjson");
    let m = SinkMetrics::new();
    let mut w = LedgerWriter::open(&p, "test", 100).unwrap();
    w.append(&(0..30).map(ev).collect::<Vec<_>>()).unwrap();
    let mut mon = ChainMonitor::new(&p, m);
    assert!(mon.scan_once().unwrap().is_none());
    let text = std::fs::read_to_string(&p)
        .unwrap()
        .replacen("\"p3\"", "\"q3\"", 1);
    std::fs::write(&p, text).unwrap();
    assert!(
        mon.scan_once().unwrap().is_none(),
        "incremental scan cannot see this by design"
    );
    let v = mon.full_verify().unwrap().expect("full verification must");
    assert_eq!((v.kind, v.seq), (ViolationKind::ModifiedEntry, Some(3)));
}

// ── end to end: webhook → ledger → offline verify ─────────────────────

#[cfg(feature = "axum")]
#[tokio::test]
async fn webhook_end_to_end_with_auth_dedupe_and_offline_verification() {
    use stellar_k8s::audit_ledger::sink::http;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("l.ndjson");
    let (sink, _j) = spawn_sink(SinkConfig::new(&p, "e2e")).unwrap();
    let app = http::router(sink.clone(), Some("s3cret".into()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let client = reqwest::Client::new();
    let url = format!("http://{addr}/audit");
    let list = json!({"kind":"EventList","apiVersion":"audit.k8s.io/v1","items":[ev(1), ev(2), {"auditID":"r","stage":"ResponseComplete","verb":"get","user":{"username":"u"}}]});

    assert_eq!(
        client.post(&url).json(&list).send().await.unwrap().status(),
        401
    );
    assert_eq!(
        client
            .post(&url)
            .bearer_auth("wrong")
            .json(&list)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let ok = client
        .post(&url)
        .bearer_auth("s3cret")
        .json(&list)
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), 200);
    let body: Value = ok.json().await.unwrap();
    assert_eq!(
        (body["appended"].as_u64(), body["duplicates"].as_u64()),
        (Some(2), Some(0))
    );
    // apiserver retry of the same batch: acknowledged, not re-appended.
    let again: Value = client
        .post(&url)
        .bearer_auth("s3cret")
        .json(&list)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        (again["appended"].as_u64(), again["duplicates"].as_u64()),
        (Some(0), Some(2))
    );
    assert_eq!(
        client
            .post(&url)
            .bearer_auth("s3cret")
            .body("not json")
            .send()
            .await
            .unwrap()
            .status(),
        400
    );

    let metrics = client
        .get(format!("http://{addr}/metrics"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        metrics.contains("stellar_audit_ledger_events_appended_total 2"),
        "{metrics}"
    );
    assert_eq!(
        client
            .get(format!("http://{addr}/healthz"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );

    let r = stellar_k8s::audit_ledger::verify::verify_file(
        &p,
        &VerifyOptions {
            expect_chain_id: Some("e2e".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(r.valid, "{:?}", r.violation);
    assert_eq!(r.entries, 2);
}

// ── scale (opt-in): cargo test --release -- --ignored audit_ledger_scale ──

/// Generates and verifies `AUDIT_SCALE_N` (default 10M) entries and prints timings.
/// Ignored by default because it writes several GB; run explicitly for the #1958 benchmark.
#[test]
#[ignore = "10M-entry benchmark; run with --release -- --ignored audit_ledger_scale"]
fn audit_ledger_scale() {
    use std::io::Write;
    let n: u64 = std::env::var("AUDIT_SCALE_N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10_000_000);
    let dir = std::env::var("AUDIT_SCALE_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    let path = dir.join("audit-scale.ndjson");
    let t = std::time::Instant::now();
    {
        let mut w =
            std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(&path).unwrap());
        w.write_all(&encode_header("scale")).unwrap();
        let mut prev = genesis_hash("scale");
        for i in 0..n {
            let (l, h) = encode_entry(i, &prev, &ev(i)).unwrap();
            w.write_all(&l).unwrap();
            prev = h;
        }
    }
    eprintln!("generated {n} entries in {:?}", t.elapsed());
    let t = std::time::Instant::now();
    let r =
        stellar_k8s::audit_ledger::verify::verify_file(&path, &VerifyOptions::default()).unwrap();
    let el = t.elapsed();
    eprintln!(
        "verified {} entries in {:?} ({:.0} entries/s)",
        r.entries,
        el,
        r.entries as f64 / el.as_secs_f64()
    );
    std::fs::remove_file(&path).ok();
    assert!(r.valid);
    assert!(el.as_secs() < 60, "verification of {n} entries took {el:?}");
}
