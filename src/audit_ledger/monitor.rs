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

//! Continuous integrity monitoring of the ledger.
//!
//! Every scan interval (default 10 s) the monitor verifies only the bytes
//! appended since the previous scan (O(new data)) and re-reads the last
//! verified line to prove the already-verified region was not shifted or
//! rewritten. A violation latches `stellar_audit_ledger_chain_valid = 0` and
//! increments the gap/failure counters, which the shipped alert rule pages on.
//! A periodic full verification (see [`ChainMonitor::full_verify`]) catches
//! same-length in-place edits deep in history.

use super::sink::{now_unix, SinkMetrics};
use super::verify::{
    parse_entry_line, verify_file, ChainVerifier, VerifyOptions, Violation, ViolationKind,
};
use super::{genesis_hash, Hash};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

const MAX_SCAN_BYTES: u64 = 64 << 20;

/// Incremental verifier over a live ledger file.
pub struct ChainMonitor {
    path: PathBuf,
    metrics: Arc<SinkMetrics>,
    verifier: Option<ChainVerifier>,
    offset: u64,
    line_no: u64,
    /// (start offset, sha256) of the last verified line, including its `\n`.
    anchor: Option<(u64, Hash)>,
    violation: Option<Violation>,
}

impl ChainMonitor {
    pub fn new(path: impl Into<PathBuf>, metrics: Arc<SinkMetrics>) -> Self {
        Self {
            path: path.into(),
            metrics,
            verifier: None,
            offset: 0,
            line_no: 1,
            anchor: None,
            violation: None,
        }
    }

    /// The latched violation, if any.
    pub fn violation(&self) -> Option<&Violation> {
        self.violation.as_ref()
    }

    fn latch(&mut self, v: Violation) {
        let m = &self.metrics;
        m.chain_valid.store(0, Ordering::Relaxed);
        m.verification_failures.fetch_add(1, Ordering::Relaxed);
        if matches!(
            v.kind,
            ViolationKind::MissingEntries | ViolationKind::Truncated
        ) {
            m.chain_gaps.fetch_add(1, Ordering::Relaxed);
        }
        tracing::error!(violation = %v, "AUDIT LEDGER INTEGRITY VIOLATION");
        self.violation = Some(v);
    }

    /// Scan newly appended data. Returns the latched violation, if any.
    pub fn scan_once(&mut self) -> io::Result<Option<Violation>> {
        if self.violation.is_some() {
            self.metrics
                .last_scan_unix
                .store(now_unix(), Ordering::Relaxed);
            return Ok(self.violation.clone());
        }
        let mut f = File::open(&self.path)?;
        let len = f.metadata()?.len();

        if self.verifier.is_none() {
            // Header.
            let mut head = Vec::new();
            let mut r = io::BufReader::new(f.try_clone()?);
            io::BufRead::read_until(&mut r, b'\n', &mut head)?;
            if head.last() != Some(&b'\n') {
                return Ok(None); // header not fully written yet
            }
            let v: serde_json::Value = serde_json::from_slice(&head[..head.len() - 1])
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            let chain = v
                .get("chainId")
                .and_then(|c| c.as_str())
                .unwrap_or_default()
                .to_string();
            let g = genesis_hash(&chain);
            self.verifier = Some(ChainVerifier::new(g, 0, g));
            self.offset = head.len() as u64;
            self.line_no = 2;
        }

        // The verified region must still be intact.
        if let Some((start, want)) = self.anchor {
            if len < self.offset {
                let seq = self.verifier.as_ref().unwrap().expected_seq();
                let v = Violation {
                    kind: ViolationKind::Truncated,
                    seq: Some(seq.saturating_sub(1)),
                    missing_to: None,
                    line: self.line_no,
                    message: format!(
                        "ledger shrank from {} to {} bytes below the verified offset",
                        self.offset, len
                    ),
                    expected: Some(self.offset.to_string()),
                    actual: Some(len.to_string()),
                };
                self.latch(v.clone());
                return Ok(Some(v));
            }
            f.seek(SeekFrom::Start(start))?;
            let mut buf = vec![0u8; (self.offset - start) as usize];
            f.read_exact(&mut buf)?;
            let got: Hash = Sha256::digest(&buf).into();
            if got != want {
                let seq = self.verifier.as_ref().unwrap().last_seq();
                let v = Violation {
                    kind: ViolationKind::ModifiedEntry,
                    seq,
                    missing_to: None,
                    line: self.line_no.saturating_sub(1),
                    message: "previously verified region of the ledger changed (entries deleted, inserted or rewritten)".into(),
                    expected: Some(hex::encode(want)),
                    actual: Some(hex::encode(got)),
                };
                self.latch(v.clone());
                return Ok(Some(v));
            }
        }

        // New complete lines.
        if len > self.offset {
            f.seek(SeekFrom::Start(self.offset))?;
            let want = (len - self.offset).min(MAX_SCAN_BYTES) as usize;
            let mut data = vec![0u8; want];
            f.read_exact(&mut data)?;
            let complete = data
                .iter()
                .rposition(|b| *b == b'\n')
                .map(|i| i + 1)
                .unwrap_or(0);
            let mut pos = 0usize;
            while pos < complete {
                let end = pos
                    + data[pos..complete]
                        .iter()
                        .position(|b| *b == b'\n')
                        .unwrap()
                    + 1;
                let line = &data[pos..end - 1];
                let res = parse_entry_line(self.line_no, line)
                    .and_then(|e| self.verifier.as_mut().unwrap().check(&e));
                if let Err(v) = res {
                    self.latch(v.clone());
                    return Ok(Some(v));
                }
                let start = self.offset + pos as u64;
                self.anchor = Some((start, Sha256::digest(&data[pos..end]).into()));
                self.line_no += 1;
                pos = end;
            }
            self.offset += complete as u64;
        }

        if let Some(last) = self.verifier.as_ref().and_then(|v| v.last_seq()) {
            self.metrics
                .verified_seq
                .store(last as i64, Ordering::Relaxed);
        }
        self.metrics
            .last_scan_unix
            .store(now_unix(), Ordering::Relaxed);
        Ok(None)
    }

    /// Full (parallel) verification of the whole file; latches on violation.
    pub fn full_verify(&mut self) -> io::Result<Option<Violation>> {
        let report = verify_file(&self.path, &VerifyOptions::default())?;
        if let Some(v) = report.violation {
            if self.violation.is_none() {
                self.latch(v.clone());
            }
            return Ok(Some(v));
        }
        Ok(None)
    }

    /// Run scans forever on the current thread: incremental every `interval`,
    /// full verification every `full_every` scans (0 disables).
    pub fn run(mut self, interval: std::time::Duration, full_every: u64) {
        let mut n = 0u64;
        loop {
            if let Err(e) = self.scan_once() {
                self.metrics.scan_errors.fetch_add(1, Ordering::Relaxed);
                tracing::warn!(error = %e, "audit ledger scan failed");
            }
            n += 1;
            if full_every > 0 && n % full_every == 0 {
                if let Err(e) = self.full_verify() {
                    self.metrics.scan_errors.fetch_add(1, Ordering::Relaxed);
                    tracing::warn!(error = %e, "audit ledger full verification failed");
                }
            }
            std::thread::sleep(interval);
        }
    }
}
