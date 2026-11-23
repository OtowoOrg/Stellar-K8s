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

//! Independent, offline verification of an audit ledger archive.
//!
//! Verification needs nothing but the archive bytes: no cluster, no producer
//! state, no keys. It recomputes every hash and reports the exact sequence
//! number at which integrity fails, classified as one of [`ViolationKind`].
//!
//! Event bodies are parsed, canonicalized and content-hashed on a worker pool
//! (they are independent of each other); only the 104-byte link hash and the
//! continuity checks run sequentially.

use super::{canonical, content_hash, entry_hash, genesis_hash, Hash, FORMAT};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::{self, BufRead};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::sync_channel;
use std::sync::{Arc, Mutex};

/// How many entries after a sequence gap are scanned to tell "deleted" from "reordered".
const LOOKAHEAD_ENTRIES: u64 = 100_000;
const CHUNK_BYTES: usize = 4 << 20;

/// Classification of an integrity failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ViolationKind {
    /// A line is not valid ledger syntax (also: torn final line, unknown fields).
    Malformed,
    /// One or more entries are absent (`seq..=missingTo`).
    MissingEntries,
    /// The same entry appears twice in a row.
    DuplicateEntry,
    /// An entry appears out of sequence order.
    ReorderedEntry,
    /// An entry's content, sequence or hash was altered.
    ModifiedEntry,
    /// An entry is self-consistent but commits to a different predecessor, so
    /// the entry at `seq` (its real predecessor) was replaced or altered.
    PredecessorMismatch,
    /// The first entry does not chain from the genesis value.
    GenesisMismatch,
    /// Header missing, wrong format, or inconsistent chain id/genesis.
    HeaderInvalid,
    /// Footer disagrees with the verified content (e.g. truncation).
    FooterMismatch,
    /// Verified head differs from the externally pinned head.
    HeadMismatch,
    /// The ledger shrank below an offset that was previously verified.
    Truncated,
}

/// The first integrity failure found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Violation {
    pub kind: ViolationKind,
    /// Exact sequence number at which verification fails (when known).
    pub seq: Option<u64>,
    /// For [`ViolationKind::MissingEntries`]: last missing sequence number.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missing_to: Option<u64>,
    /// 1-based line number in the archive.
    pub line: u64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<String>,
}

impl Violation {
    fn new(kind: ViolationKind, seq: Option<u64>, line: u64, message: impl Into<String>) -> Self {
        Self {
            kind,
            seq,
            missing_to: None,
            line,
            message: message.into(),
            expected: None,
            actual: None,
        }
    }
    fn with(mut self, expected: impl Into<String>, actual: impl Into<String>) -> Self {
        self.expected = Some(expected.into());
        self.actual = Some(actual.into());
        self
    }
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.seq {
            Some(s) => write!(
                f,
                "{:?} at seq {} (line {}): {}",
                self.kind, s, self.line, self.message
            ),
            None => write!(f, "{:?} (line {}): {}", self.kind, self.line, self.message),
        }
    }
}

/// Verification outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyReport {
    pub valid: bool,
    pub chain_id: Option<String>,
    /// Entries verified before stopping (all of them when `valid`).
    pub entries: u64,
    pub first_seq: Option<u64>,
    pub last_seq: Option<u64>,
    /// Hex head hash of the verified prefix.
    pub head_hash: Option<String>,
    pub footer_present: bool,
    pub violation: Option<Violation>,
}

/// Verification options.
#[derive(Debug, Clone)]
pub struct VerifyOptions {
    /// Pin the head hash obtained out-of-band; detects tail truncation.
    pub expect_head: Option<String>,
    /// Require this chain id in the header.
    pub expect_chain_id: Option<String>,
    /// Require the archive to end with a footer.
    pub require_footer: bool,
    /// Worker threads (0 = available parallelism).
    pub threads: usize,
}

impl Default for VerifyOptions {
    fn default() -> Self {
        Self {
            expect_head: None,
            expect_chain_id: None,
            require_footer: false,
            threads: 0,
        }
    }
}

// ───────────────────────────── line parsing ─────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EntryLine<'a> {
    seq: u64,
    prev_hash: &'a str,
    hash: &'a str,
    event: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HeaderLine {
    #[serde(rename = "type")]
    _type: String,
    format: String,
    chain_id: String,
    genesis_hash: String,
    #[serde(default)]
    start_seq: Option<u64>,
    #[serde(default)]
    start_prev_hash: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FooterLine {
    #[serde(rename = "type")]
    _type: String,
    count: u64,
    head_seq: Option<u64>,
    head_hash: String,
}

/// A parsed, content-hashed entry (hash fields decoded).
#[derive(Debug, Clone)]
pub struct ParsedEntry {
    pub line: u64,
    pub seq: u64,
    pub prev: Hash,
    pub hash: Hash,
    pub content: Hash,
}

enum Line {
    Header(HeaderLine),
    Entry(ParsedEntry),
    Footer(FooterLine),
}

fn decode_hash(s: &str) -> Option<Hash> {
    if s.len() != 64 || !s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return None;
    }
    let mut out = [0u8; 32];
    hex::decode_to_slice(s, &mut out).ok()?;
    Some(out)
}

fn parse_line(line_no: u64, bytes: &[u8]) -> Result<Line, Violation> {
    let bad = |m: String| Violation::new(ViolationKind::Malformed, None, line_no, m);
    if bytes.starts_with(br#"{"type":"header""#) {
        return serde_json::from_slice::<HeaderLine>(bytes)
            .map(Line::Header)
            .map_err(|e| bad(format!("invalid header: {e}")));
    }
    if bytes.starts_with(br#"{"type":"footer""#) {
        return serde_json::from_slice::<FooterLine>(bytes)
            .map(Line::Footer)
            .map_err(|e| bad(format!("invalid footer: {e}")));
    }
    let e: EntryLine =
        serde_json::from_slice(bytes).map_err(|e| bad(format!("invalid entry: {e}")))?;
    let seq = e.seq;
    let bad_seq = |m: &str| Violation::new(ViolationKind::Malformed, Some(seq), line_no, m);
    let prev = decode_hash(e.prev_hash)
        .ok_or_else(|| bad_seq("prevHash is not 64 lowercase hex chars"))?;
    let hash = decode_hash(e.hash).ok_or_else(|| bad_seq("hash is not 64 lowercase hex chars"))?;
    let canon = canonical::to_vec(&e.event).map_err(|err| {
        Violation::new(
            ViolationKind::Malformed,
            Some(seq),
            line_no,
            err.to_string(),
        )
    })?;
    Ok(Line::Entry(ParsedEntry {
        line: line_no,
        seq,
        prev,
        hash,
        content: content_hash(&canon),
    }))
}

/// Parse a single entry line (used by the incremental monitor).
pub fn parse_entry_line(line_no: u64, bytes: &[u8]) -> Result<ParsedEntry, Violation> {
    match parse_line(line_no, bytes)? {
        Line::Entry(e) => Ok(e),
        _ => Err(Violation::new(
            ViolationKind::Malformed,
            None,
            line_no,
            "expected an entry line",
        )),
    }
}

// ───────────────────────────── chain state ─────────────────────────────

/// Sequential chain checker. Usable incrementally (the monitor resumes it).
#[derive(Debug, Clone)]
pub struct ChainVerifier {
    genesis: Hash,
    expected_seq: u64,
    prev: Hash,
    pub entries: u64,
    pub first_seq: Option<u64>,
}

impl ChainVerifier {
    /// Start at `start_seq` chained from `start_prev` (genesis for a full chain).
    pub fn new(genesis: Hash, start_seq: u64, start_prev: Hash) -> Self {
        Self {
            genesis,
            expected_seq: start_seq,
            prev: start_prev,
            entries: 0,
            first_seq: None,
        }
    }

    pub fn from_genesis(chain_id: &str) -> Self {
        let g = genesis_hash(chain_id);
        Self::new(g, 0, g)
    }

    pub fn expected_seq(&self) -> u64 {
        self.expected_seq
    }
    pub fn head(&self) -> Hash {
        self.prev
    }
    pub fn last_seq(&self) -> Option<u64> {
        self.entries.checked_sub(1).map(|_| self.expected_seq - 1)
    }

    /// Check one entry; on success advance the chain.
    pub fn check(&mut self, e: &ParsedEntry) -> Result<(), Violation> {
        let computed = entry_hash(e.seq, &e.prev, &e.content);
        if computed != e.hash {
            return Err(Violation::new(
                ViolationKind::ModifiedEntry,
                Some(e.seq),
                e.line,
                "entry hash does not match its sequence, prevHash and event content",
            )
            .with(hex::encode(computed), hex::encode(e.hash)));
        }
        if e.seq > self.expected_seq {
            let mut v = Violation::new(
                ViolationKind::MissingEntries,
                Some(self.expected_seq),
                e.line,
                format!(
                    "entries {}..={} are missing (found seq {} instead)",
                    self.expected_seq,
                    e.seq - 1,
                    e.seq
                ),
            );
            v.missing_to = Some(e.seq - 1);
            return Err(v);
        }
        if e.seq < self.expected_seq {
            let immediate_dup =
                self.entries > 0 && e.seq + 1 == self.expected_seq && e.hash == self.prev;
            return Err(if immediate_dup {
                Violation::new(
                    ViolationKind::DuplicateEntry,
                    Some(e.seq),
                    e.line,
                    "entry is a duplicate of the previous entry",
                )
            } else {
                Violation::new(
                    ViolationKind::ReorderedEntry,
                    Some(e.seq),
                    e.line,
                    format!(
                        "sequence went backwards: expected {}, found {} (duplicated or out-of-order entry)",
                        self.expected_seq, e.seq
                    ),
                )
            });
        }
        if e.prev != self.prev {
            return Err(if self.entries == 0 && e.seq == 0 {
                Violation::new(
                    ViolationKind::GenesisMismatch,
                    Some(0),
                    e.line,
                    "first entry does not chain from the genesis hash",
                )
                .with(hex::encode(self.genesis), hex::encode(e.prev))
            } else {
                Violation::new(
                    ViolationKind::PredecessorMismatch,
                    e.seq.checked_sub(1),
                    e.line,
                    format!(
                        "entry {} commits to a different predecessor: entry {} was replaced or modified",
                        e.seq,
                        e.seq.saturating_sub(1)
                    ),
                )
                .with(hex::encode(self.prev), hex::encode(e.prev))
            });
        }
        self.prev = e.hash;
        self.expected_seq = e.seq + 1;
        self.entries += 1;
        self.first_seq.get_or_insert(e.seq);
        Ok(())
    }
}

// ───────────────────────────── driver ─────────────────────────────

struct Chunk {
    idx: u64,
    first_line: u64,
    data: Vec<u8>,
}

type ParsedChunk = (u64, Vec<(u64, Result<Line, Violation>)>);

fn split_lines(data: &[u8]) -> impl Iterator<Item = &[u8]> {
    data.split(|b| *b == b'\n')
}

fn parse_chunk(c: Chunk) -> ParsedChunk {
    let mut out = Vec::new();
    let mut no = c.first_line;
    let mut lines: Vec<&[u8]> = split_lines(&c.data).collect();
    if c.data.ends_with(b"\n") {
        lines.pop();
    }
    let last_idx = lines.len().saturating_sub(1);
    let torn_tail = !c.data.ends_with(b"\n");
    for (i, l) in lines.into_iter().enumerate() {
        let r = if torn_tail && i == last_idx {
            // Reader only produces a non-terminated chunk at EOF: torn final line.
            match parse_line(no, l) {
                Ok(Line::Footer(f)) => Ok(Line::Footer(f)),
                Ok(_) | Err(_) => Err(Violation::new(
                    ViolationKind::Malformed,
                    None,
                    no,
                    "final line is not newline-terminated (torn write or truncated archive)",
                )),
            }
        } else {
            parse_line(no, l)
        };
        out.push((no, r));
        no += 1;
    }
    (c.idx, out)
}

/// Verify the archive at `path`.
pub fn verify_file(path: &std::path::Path, opts: &VerifyOptions) -> io::Result<VerifyReport> {
    let f = std::fs::File::open(path)?;
    verify_reader(io::BufReader::with_capacity(1 << 20, f), opts)
}

/// Verify an archive from any buffered reader.
pub fn verify_reader<R: BufRead + Send>(
    mut reader: R,
    opts: &VerifyOptions,
) -> io::Result<VerifyReport> {
    let mut report = VerifyReport {
        valid: false,
        chain_id: None,
        entries: 0,
        first_seq: None,
        last_seq: None,
        head_hash: None,
        footer_present: false,
        violation: None,
    };

    // ── header ──
    let mut first = Vec::new();
    reader.read_until(b'\n', &mut first)?;
    let fail = |mut r: VerifyReport, v: Violation| {
        r.violation = Some(v);
        r
    };
    if first.is_empty() {
        return Ok(fail(
            report,
            Violation::new(
                ViolationKind::HeaderInvalid,
                None,
                1,
                "archive is empty (no header)",
            ),
        ));
    }
    if first.last() == Some(&b'\n') {
        first.pop();
    }
    let header = match parse_line(1, &first) {
        Ok(Line::Header(h)) => h,
        Ok(_) => {
            return Ok(fail(
                report,
                Violation::new(
                    ViolationKind::HeaderInvalid,
                    None,
                    1,
                    "first line must be the header",
                ),
            ))
        }
        Err(v) => {
            return Ok(fail(
                report,
                Violation {
                    kind: ViolationKind::HeaderInvalid,
                    ..v
                },
            ))
        }
    };
    report.chain_id = Some(header.chain_id.clone());
    let hv = |m: String| Violation::new(ViolationKind::HeaderInvalid, None, 1, m);
    if header.format != FORMAT {
        return Ok(fail(
            report,
            hv(format!(
                "unsupported format {:?}, want {FORMAT:?}",
                header.format
            )),
        ));
    }
    if let Some(want) = &opts.expect_chain_id {
        if *want != header.chain_id {
            return Ok(fail(
                report,
                hv(format!(
                    "chainId {:?} does not match expected {want:?}",
                    header.chain_id
                )),
            ));
        }
    }
    let genesis = genesis_hash(&header.chain_id);
    if decode_hash(&header.genesis_hash) != Some(genesis) {
        return Ok(fail(
            report,
            hv("genesisHash does not equal SHA256(domain || chainId)".to_string()),
        ));
    }
    let (start_seq, start_prev) = match (header.start_seq, header.start_prev_hash.as_deref()) {
        (None, None) => (0, genesis),
        (Some(s), Some(p)) => match decode_hash(p) {
            Some(h) => (s, h),
            None => {
                return Ok(fail(
                    report,
                    hv("startPrevHash is not 64 lowercase hex chars".into()),
                ))
            }
        },
        _ => {
            return Ok(fail(
                report,
                hv("startSeq and startPrevHash must be given together".into()),
            ))
        }
    };

    let mut verifier = ChainVerifier::new(genesis, start_seq, start_prev);
    let mut footer: Option<(u64, FooterLine)> = None;
    let mut violation: Option<Violation> = None;
    let mut pending_gap: Option<(Violation, u64)> = None; // (violation, entries scanned)

    let threads = if opts.threads == 0 {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
    } else {
        opts.threads
    };

    let stop = AtomicBool::new(false);
    let io_err: Mutex<Option<io::Error>> = Mutex::new(None);
    let (work_tx, work_rx) = sync_channel::<Chunk>(threads * 2);
    let work_rx = Arc::new(Mutex::new(work_rx));
    let (res_tx, res_rx) = sync_channel::<ParsedChunk>(threads * 4);

    std::thread::scope(|s| {
        // reader
        s.spawn(|| {
            let mut idx = 0u64;
            let mut line_no = 2u64;
            loop {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let mut data = Vec::with_capacity(CHUNK_BYTES + 4096);
                let mut lines = 0u64;
                let mut eof = false;
                while data.len() < CHUNK_BYTES {
                    match reader.read_until(b'\n', &mut data) {
                        Ok(0) => {
                            eof = true;
                            break;
                        }
                        Ok(_) => lines += 1,
                        Err(e) => {
                            *io_err.lock().unwrap() = Some(e);
                            return;
                        }
                    }
                }
                if lines > 0 {
                    let chunk = Chunk {
                        idx,
                        first_line: line_no,
                        data,
                    };
                    idx += 1;
                    line_no += lines;
                    if work_tx.send(chunk).is_err() {
                        break;
                    }
                }
                if eof {
                    break;
                }
            }
            drop(work_tx);
        });
        // workers
        for _ in 0..threads {
            let rx = Arc::clone(&work_rx);
            let tx = res_tx.clone();
            s.spawn(move || loop {
                let chunk = { rx.lock().unwrap().recv() };
                match chunk {
                    Ok(c) => {
                        if tx.send(parse_chunk(c)).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            });
        }
        drop(res_tx);

        // consumer (this thread): strictly in-order
        let mut next_idx = 0u64;
        let mut buf: BTreeMap<u64, Vec<(u64, Result<Line, Violation>)>> = BTreeMap::new();
        'outer: for (idx, lines) in res_rx.iter() {
            buf.insert(idx, lines);
            while let Some(lines) = buf.remove(&next_idx) {
                next_idx += 1;
                for (no, parsed) in lines {
                    if footer.is_some() {
                        violation = Some(Violation::new(
                            ViolationKind::Malformed,
                            None,
                            no,
                            "data found after footer",
                        ));
                        break 'outer;
                    }
                    match parsed {
                        Err(v) => {
                            // Report a pending gap in preference to a later parse error.
                            violation = Some(pending_gap.take().map(|(g, _)| g).unwrap_or(v));
                            break 'outer;
                        }
                        Ok(Line::Header(_)) => {
                            violation = Some(Violation::new(
                                ViolationKind::Malformed,
                                None,
                                no,
                                "unexpected second header",
                            ));
                            break 'outer;
                        }
                        Ok(Line::Footer(f)) => {
                            if let Some((g, _)) = pending_gap.take() {
                                violation = Some(g);
                                break 'outer;
                            }
                            footer = Some((no, f));
                        }
                        Ok(Line::Entry(e)) => {
                            if let Some((gap, scanned)) = pending_gap.as_mut() {
                                // Look ahead: is the "missing" entry merely out of order?
                                if e.seq == gap.seq.unwrap_or(u64::MAX)
                                    && entry_hash(e.seq, &e.prev, &e.content) == e.hash
                                {
                                    let mut v = Violation::new(
                                        ViolationKind::ReorderedEntry,
                                        gap.seq,
                                        gap.line,
                                        format!(
                                            "entry {} is out of order: it appears later at line {}",
                                            e.seq, e.line
                                        ),
                                    );
                                    v.expected = gap.expected.clone();
                                    violation = Some(v);
                                    break 'outer;
                                }
                                *scanned += 1;
                                if *scanned >= LOOKAHEAD_ENTRIES {
                                    violation = Some(pending_gap.take().unwrap().0);
                                    break 'outer;
                                }
                                continue;
                            }
                            if let Err(v) = verifier.check(&e) {
                                if v.kind == ViolationKind::MissingEntries {
                                    pending_gap = Some((v, 0));
                                    continue;
                                }
                                violation = Some(v);
                                break 'outer;
                            }
                        }
                    }
                }
            }
        }
        if violation.is_none() {
            if let Some((g, _)) = pending_gap.take() {
                violation = Some(g);
            }
        }
        stop.store(true, Ordering::Relaxed);
        // Dropping res_rx (end of scope closure) unblocks workers and the reader.
        drop(res_rx);
    });

    if let Some(e) = io_err.into_inner().unwrap() {
        return Err(e);
    }

    report.entries = verifier.entries;
    report.first_seq = verifier.first_seq;
    report.last_seq = verifier.last_seq();
    report.head_hash = Some(hex::encode(verifier.head()));
    report.footer_present = footer.is_some();

    if violation.is_none() {
        if let Some((no, f)) = &footer {
            let head = hex::encode(verifier.head());
            if f.count != verifier.entries
                || f.head_seq != verifier.last_seq()
                || f.head_hash != head
            {
                violation = Some(
                    Violation::new(
                        ViolationKind::FooterMismatch,
                        verifier.last_seq().map(|s| s + 1).or(Some(start_seq)),
                        *no,
                        format!(
                            "footer claims {} entries ending at seq {:?}; verified {} entries ending at seq {:?} (truncated or appended?)",
                            f.count, f.head_seq, verifier.entries, verifier.last_seq()
                        ),
                    )
                    .with(format!("{}:{}", f.count, f.head_hash), format!("{}:{}", verifier.entries, head)),
                );
            }
        } else if opts.require_footer {
            violation = Some(Violation::new(
                ViolationKind::FooterMismatch,
                verifier.last_seq().map(|s| s + 1),
                0,
                "footer required but absent (archive may be truncated)",
            ));
        }
    }
    if violation.is_none() {
        if let Some(want) = &opts.expect_head {
            let head = hex::encode(verifier.head());
            if want.to_lowercase() != head {
                violation = Some(
                    Violation::new(
                        ViolationKind::HeadMismatch,
                        verifier.last_seq().map(|s| s + 1).or(Some(start_seq)),
                        0,
                        "verified head differs from the pinned head (tail truncated or rewritten)",
                    )
                    .with(want.to_lowercase(), head),
                );
            }
        }
    }
    report.valid = violation.is_none();
    report.violation = violation;
    Ok(report)
}
