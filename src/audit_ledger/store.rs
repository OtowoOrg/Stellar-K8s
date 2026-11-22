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

//! Append-only ledger store and the writer that chains events into it.
//!
//! The writer only ever opens the ledger with `O_APPEND`; there is no API to
//! overwrite or delete committed bytes. The one exception is rolling back
//! bytes of a batch that failed *before* being acknowledged (never committed).
//! Operational hardening (separate pod/service account, `chattr +a`, object
//! lock on exported archives) is described in `docs/audit-ledger.md`.

use super::verify::{parse_entry_line, ChainVerifier};
use super::{canonical, encode_entry, encode_header, genesis_hash, Hash};
use serde_json::Value;
use std::collections::{HashSet, VecDeque};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// How much of the ledger tail is re-read on startup to resume and to seed dedupe.
const RECOVERY_TAIL_BYTES: u64 = 16 << 20;

/// Errors from the ledger store/writer.
#[derive(Debug, thiserror::Error)]
pub enum LedgerError {
    #[error("ledger I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("event cannot be canonicalized: {0}")]
    Canonical(#[from] canonical::CanonicalError),
    #[error("ledger at {path} belongs to chain {found:?}, configured chain is {want:?}")]
    ChainIdMismatch {
        path: String,
        found: String,
        want: String,
    },
    #[error("ledger tail is corrupt: {0}")]
    Corrupt(String),
}

/// Result of appending one logical batch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppendOutcome {
    pub appended: u64,
    pub duplicates: u64,
    pub head_seq: Option<u64>,
}

/// Bounded FIFO set of recently seen `(auditID, stage)` keys.
struct DedupeWindow {
    cap: usize,
    set: HashSet<String>,
    order: VecDeque<String>,
}

impl DedupeWindow {
    fn new(cap: usize) -> Self {
        Self {
            cap,
            set: HashSet::new(),
            order: VecDeque::new(),
        }
    }
    fn contains(&self, k: &str) -> bool {
        self.set.contains(k)
    }
    fn insert(&mut self, k: String) {
        if self.set.insert(k.clone()) {
            self.order.push_back(k);
            while self.order.len() > self.cap {
                if let Some(old) = self.order.pop_front() {
                    self.set.remove(&old);
                }
            }
        }
    }
}

/// Dedupe key of an event: `auditID/stage`.
pub fn dedupe_key(event: &Value) -> Option<String> {
    let id = event.get("auditID")?.as_str()?;
    let stage = event.get("stage").and_then(Value::as_str).unwrap_or("");
    Some(format!("{id}/{stage}"))
}

/// Chains events and appends them to an append-only NDJSON file.
pub struct LedgerWriter {
    path: PathBuf,
    chain_id: String,
    file: File,
    /// Bytes known to be committed (durable) so far.
    durable_len: u64,
    next_seq: u64,
    head: Hash,
    dedupe: DedupeWindow,
    /// Staged (written, not yet fsynced) state.
    staged: Option<Staged>,
}

impl LedgerWriter {
    /// Open (creating if needed) the ledger at `path` and resume from its tail.
    pub fn open(
        path: impl AsRef<Path>,
        chain_id: &str,
        dedupe_window: usize,
    ) -> Result<Self, LedgerError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let mut opts = OpenOptions::new();
        opts.create(true).read(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o640);
        }
        let mut file = opts.open(&path)?;
        let mut len = file.metadata()?.len();
        let mut dedupe = DedupeWindow::new(dedupe_window.max(1));
        let genesis = genesis_hash(chain_id);

        if len == 0 {
            file.write_all(&encode_header(chain_id))?;
            file.sync_all()?;
            len = file.metadata()?.len();
            return Ok(Self {
                path,
                chain_id: chain_id.into(),
                file,
                durable_len: len,
                next_seq: 0,
                head: genesis,
                dedupe,
                staged: None,
            });
        }

        // Header check.
        let mut first = Vec::new();
        {
            let mut r = io::BufReader::new(File::open(&path)?);
            io::BufRead::read_until(&mut r, b'\n', &mut first)?;
        }
        let hv: Value = serde_json::from_slice(first.strip_suffix(b"\n").unwrap_or(&first))
            .map_err(|e| LedgerError::Corrupt(format!("header: {e}")))?;
        let found = hv
            .get("chainId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if found != chain_id {
            return Err(LedgerError::ChainIdMismatch {
                path: path.display().to_string(),
                found,
                want: chain_id.into(),
            });
        }

        // Read tail; drop a torn (non-newline-terminated) final line, which was never acked.
        let from = len
            .saturating_sub(RECOVERY_TAIL_BYTES)
            .max(first.len() as u64);
        let mut tail = Vec::new();
        let mut rf = File::open(&path)?;
        rf.seek(SeekFrom::Start(from))?;
        rf.read_to_end(&mut tail)?;
        let committed_len_in_tail = tail
            .iter()
            .rposition(|b| *b == b'\n')
            .map(|i| i + 1)
            .unwrap_or(0);
        if (committed_len_in_tail as u64) + from != len {
            let new_len = from + committed_len_in_tail as u64;
            tracing::warn!(path = %path.display(), torn_bytes = len - new_len, "discarding torn final audit line from interrupted write");
            file.set_len(new_len)?;
            file.sync_all()?;
            len = new_len;
        }
        tail.truncate(committed_len_in_tail);

        let mut lines: Vec<&[u8]> = tail
            .split(|b| *b == b'\n')
            .filter(|l| !l.is_empty())
            .collect();
        if from > first.len() as u64 && !lines.is_empty() {
            lines.remove(0); // first line may start mid-record
        }
        let mut next_seq = 0u64;
        let mut head = genesis;
        let mut last: Option<(u64, Hash)> = None;
        let mut linked = ChainVerifier::new(genesis, 0, genesis);
        let mut have_start = false;
        for (i, l) in lines.iter().enumerate() {
            let e = parse_entry_line(i as u64 + 2, l)
                .map_err(|v| LedgerError::Corrupt(v.to_string()))?;
            if !have_start {
                linked = ChainVerifier::new(genesis, e.seq, e.prev);
                have_start = true;
            }
            linked
                .check(&e)
                .map_err(|v| LedgerError::Corrupt(v.to_string()))?;
            last = Some((e.seq, e.hash));
            if let Ok(v) = serde_json::from_slice::<Value>(l) {
                if let Some(ev) = v.get("event") {
                    if let Some(k) = dedupe_key(ev) {
                        dedupe.insert(k);
                    }
                }
            }
        }
        if let Some((s, h)) = last {
            next_seq = s + 1;
            head = h;
        } else if len > first.len() as u64 {
            return Err(LedgerError::Corrupt(
                "could not locate last entry in ledger tail".into(),
            ));
        }
        Ok(Self {
            path,
            chain_id: chain_id.into(),
            file,
            durable_len: len,
            next_seq,
            head,
            dedupe,
            staged: None,
        })
    }

    /// Test hook: swap the underlying file (e.g. for `/dev/full`) to inject write failures.
    #[doc(hidden)]
    pub fn replace_file_for_test(&mut self, f: File) {
        self.file = f;
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn chain_id(&self) -> &str {
        &self.chain_id
    }
    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }
    pub fn head_hash(&self) -> Hash {
        self.head
    }

    /// Chain and write `events` (no fsync). Duplicates (same auditID+stage within
    /// the dedupe window or already staged) are skipped. Call
    /// [`commit`](Self::commit) to make durable or [`rollback`](Self::rollback)
    /// to discard; dedupe keys are only remembered on commit so a rolled-back
    /// batch can be retried by the sender.
    pub fn stage(&mut self, events: &[Value]) -> Result<AppendOutcome, LedgerError> {
        let mut buf = Vec::with_capacity(events.len() * 512);
        let mut st = self.staged.take().unwrap_or(Staged {
            seq: self.next_seq,
            bytes: 0,
            head: self.head,
            keys: HashSet::new(),
        });
        let mut out = AppendOutcome::default();
        let (mut seq, mut head) = (st.seq, st.head);
        let mut new_keys: HashSet<String> = HashSet::new();
        for ev in events {
            if let Some(k) = dedupe_key(ev) {
                if self.dedupe.contains(&k) || st.keys.contains(&k) || new_keys.contains(&k) {
                    out.duplicates += 1;
                    continue;
                }
                new_keys.insert(k);
            }
            let (line, h) = match encode_entry(seq, &head, ev) {
                Ok(x) => x,
                Err(e) => {
                    self.staged = Some(st);
                    return Err(e.into());
                }
            };
            buf.extend_from_slice(&line);
            head = h;
            seq += 1;
            out.appended += 1;
        }
        if out.appended > 0 {
            if let Err(e) = self.file.write_all(&buf) {
                self.staged = Some(st);
                return Err(e.into());
            }
            st.seq = seq;
            st.head = head;
            st.bytes += buf.len() as u64;
            st.keys.extend(new_keys);
            out.head_seq = Some(seq - 1);
        }
        self.staged = Some(st);
        Ok(out)
    }

    /// fsync staged entries and advance the committed state.
    pub fn commit(&mut self) -> Result<(), LedgerError> {
        if let Some(st) = self.staged.take() {
            if let Err(e) = self.file.sync_data() {
                self.staged = Some(st);
                return Err(e.into());
            }
            self.next_seq = st.seq;
            self.head = st.head;
            self.durable_len += st.bytes;
            for k in st.keys {
                self.dedupe.insert(k);
            }
        }
        Ok(())
    }

    /// Discard bytes written since the last commit (they were never acknowledged).
    pub fn rollback(&mut self) -> Result<(), LedgerError> {
        self.staged = None;
        if self.file.metadata()?.len() != self.durable_len {
            self.file.set_len(self.durable_len)?;
            self.file.sync_all()?;
        }
        Ok(())
    }

    /// Convenience: stage + commit, rolling back on failure.
    pub fn append(&mut self, events: &[Value]) -> Result<AppendOutcome, LedgerError> {
        let out = match self.stage(events) {
            Ok(o) => o,
            Err(e) => {
                let _ = self.rollback();
                return Err(e);
            }
        };
        if let Err(e) = self.commit() {
            let _ = self.rollback();
            return Err(e);
        }
        Ok(out)
    }
}

struct Staged {
    seq: u64,
    bytes: u64,
    head: Hash,
    keys: HashSet<String>,
}
