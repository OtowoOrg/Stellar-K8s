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

//! `stellar-audit-verify` — offline verifier for tamper-evident audit ledgers (#1958).
//!
//! Needs no cluster and no network: it reads an archive (file or stdin),
//! recomputes every SHA-256 link and reports the exact sequence number where the
//! chain breaks.
//!
//! Exit codes: `0` chain valid, `1` integrity violation, `2` usage / I/O error.

use clap::{Parser, Subcommand};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use stellar_k8s::audit_ledger::verify::{verify_file, verify_reader, VerifyOptions, VerifyReport};
use stellar_k8s::audit_ledger::{encode_entry, encode_header, genesis_hash};

#[derive(Parser)]
#[command(
    name = "stellar-audit-verify",
    version,
    about = "Verify and export hash-chained audit ledgers offline"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Verify an archive (`-` reads stdin).
    Verify {
        path: String,
        /// Head hash obtained out-of-band; detects tail truncation.
        #[arg(long)]
        expect_head: Option<String>,
        #[arg(long)]
        expect_chain_id: Option<String>,
        /// Fail if the archive has no footer.
        #[arg(long)]
        require_footer: bool,
        /// Worker threads (0 = all cores).
        #[arg(long, default_value_t = 0)]
        threads: usize,
        /// Machine-readable JSON report.
        #[arg(long)]
        json: bool,
    },
    /// Verify a live ledger and write a sealed archive (header + entries + footer).
    Export { ledger: PathBuf, out: PathBuf },
    /// Generate a synthetic ledger (benchmarks, red-team drills).
    Generate {
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value = "bench")]
        chain_id: String,
        #[arg(long, default_value_t = 1_000_000)]
        count: u64,
    },
}

fn print_report(r: &VerifyReport, json: bool) {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(r).expect("report serializes")
        );
        return;
    }
    match &r.violation {
        None => println!(
            "OK: chain {:?} valid — {} entries (seq {}..={}), head {}{}",
            r.chain_id.as_deref().unwrap_or("?"),
            r.entries,
            r.first_seq
                .map(|s| s.to_string())
                .unwrap_or_else(|| "-".into()),
            r.last_seq
                .map(|s| s.to_string())
                .unwrap_or_else(|| "-".into()),
            r.head_hash.as_deref().unwrap_or("-"),
            if r.footer_present {
                ", footer verified"
            } else {
                ", no footer"
            },
        ),
        Some(v) => {
            println!("FAIL: {v}");
            if let Some(e) = &v.expected {
                println!("  expected: {e}");
            }
            if let Some(a) = &v.actual {
                println!("  actual:   {a}");
            }
            println!("  verified {} entries before the failure", r.entries);
        }
    }
}

#[cfg(unix)]
fn stdin_file() -> std::io::Result<std::fs::File> {
    use std::os::fd::AsFd;
    Ok(std::fs::File::from(
        std::io::stdin().as_fd().try_clone_to_owned()?,
    ))
}

#[cfg(not(unix))]
fn stdin_file() -> std::io::Result<std::fs::File> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "reading stdin is only supported on unix; pass a file path",
    ))
}

fn real_main() -> Result<ExitCode, Box<dyn std::error::Error>> {
    match Cli::parse().cmd {
        Cmd::Verify {
            path,
            expect_head,
            expect_chain_id,
            require_footer,
            threads,
            json,
        } => {
            let opts = VerifyOptions {
                expect_head,
                expect_chain_id,
                require_footer,
                threads,
            };
            let report = if path == "-" {
                verify_reader(BufReader::with_capacity(1 << 20, stdin_file()?), &opts)?
            } else {
                verify_file(std::path::Path::new(&path), &opts)?
            };
            print_report(&report, json);
            Ok(if report.valid {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            })
        }
        Cmd::Export { ledger, out } => {
            let report = verify_file(&ledger, &VerifyOptions::default())?;
            if !report.valid {
                print_report(&report, false);
                eprintln!("refusing to export an archive that fails verification");
                return Ok(ExitCode::from(1));
            }
            let mut dst = BufWriter::new(
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&out)?,
            );
            let mut src = BufReader::new(std::fs::File::open(&ledger)?);
            let mut line = Vec::new();
            let mut n = 0u64;
            let limit = report.entries + 1; // header + entries; never copy a footer twice
            while n < limit {
                line.clear();
                if src.read_until(b'\n', &mut line)? == 0 {
                    break;
                }
                dst.write_all(&line)?;
                n += 1;
            }
            dst.write_all(&stellar_k8s::audit_ledger::encode_footer(
                report.entries,
                report.last_seq,
                report.head_hash.as_deref().unwrap_or_default(),
            ))?;
            dst.flush()?;
            dst.get_ref().sync_all()?;
            println!(
                "exported {} entries to {} (head {})",
                report.entries,
                out.display(),
                report.head_hash.unwrap_or_default()
            );
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Generate {
            out,
            chain_id,
            count,
        } => {
            let mut w = BufWriter::with_capacity(
                1 << 20,
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&out)?,
            );
            w.write_all(&encode_header(&chain_id))?;
            let mut prev = genesis_hash(&chain_id);
            let verbs = ["create", "update", "patch", "delete"];
            for seq in 0..count {
                let ev = serde_json::json!({
                    "auditID": format!("{:08x}-0000-4000-8000-{:012x}", seq as u32, seq),
                    "stage": "ResponseComplete",
                    "verb": verbs[(seq % 4) as usize],
                    "requestURI": format!("/api/v1/namespaces/ns-{}/configmaps/cm-{}", seq % 97, seq % 1013),
                    "user": {"username": format!("system:serviceaccount:ns-{}:sa", seq % 97), "groups": ["system:serviceaccounts", "system:authenticated"]},
                    "objectRef": {"resource": "configmaps", "namespace": format!("ns-{}", seq % 97), "name": format!("cm-{}", seq % 1013), "apiVersion": "v1"},
                    "responseStatus": {"code": 200},
                    "requestReceivedTimestamp": "2026-01-01T00:00:00.000000Z",
                    "stageTimestamp": "2026-01-01T00:00:00.001000Z",
                });
                let (line, h) = encode_entry(seq, &prev, &ev)?;
                w.write_all(&line)?;
                prev = h;
            }
            w.flush()?;
            println!("generated {count} entries in {}", out.display());
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn main() -> ExitCode {
    match real_main() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(2)
        }
    }
}
