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

//! # gas-calibrator
//!
//! Automated Soroban gas metering calibrator.
//!
//! Executes a suite of WASM micro-benchmarks on the local hardware and
//! generates a dynamically-tuned Soroban gas configuration profile that
//! reflects the actual compute cost on this machine.
//!
//! ## Usage
//!
//! ```text
//! gas-calibrator [OPTIONS]
//!
//! Options:
//!   -i, --iterations <N>   Benchmark iterations per test  [default: 100]
//!   -o, --output <FILE>    Write profile to file instead of stdout
//!   -f, --format <FMT>     Output format: json | yaml  [default: json]
//!       --cpu <N>          CPU core to pin the process to  [default: 0]
//!       --warmup-ms <MS>   Warm-up duration in milliseconds  [default: 200]
//!       --no-pin           Disable CPU affinity pinning
//!   -h, --help             Print help
//!   -V, --version          Print version
//! ```

use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use serde::{Deserialize, Serialize};

mod benchmarks;
mod profiler;

use benchmarks::BenchmarkResult;
use profiler::HardwareProfile;

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

/// Automated Soroban gas metering calibrator.
///
/// Benchmarks host hardware with WASM micro-benchmarks and emits a tuning
/// profile (JSON or YAML) for Soroban CPU instruction pricing tiers.
#[derive(Parser, Debug)]
#[command(
    name = "gas-calibrator",
    version,
    about,
    long_about = None
)]
struct Cli {
    /// Number of benchmark iterations per test.  Higher values reduce variance.
    #[arg(short = 'i', long = "iterations", default_value_t = 100)]
    iterations: u32,

    /// Write the tuning profile to this file instead of stdout.
    #[arg(short = 'o', long = "output")]
    output: Option<PathBuf>,

    /// Output format.
    #[arg(short = 'f', long = "format", value_enum, default_value_t = OutputFormat::Json)]
    format: OutputFormat,

    /// Logical CPU core index to pin the benchmark process to (0-based).
    #[arg(long = "cpu", default_value_t = 0)]
    cpu: usize,

    /// Warm-up duration in milliseconds before timing starts.
    #[arg(long = "warmup-ms", default_value_t = 200)]
    warmup_ms: u64,

    /// Disable CPU affinity pinning (reduces isolation but avoids permission
    /// errors on restricted environments).
    #[arg(long = "no-pin")]
    no_pin: bool,
}

#[derive(Debug, Clone, ValueEnum)]
enum OutputFormat {
    Json,
    Yaml,
}

// ---------------------------------------------------------------------------
// Output structures
// ---------------------------------------------------------------------------

/// A single Soroban CPU instruction pricing tier derived from benchmark data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GasTier {
    /// Soroban operation category.
    pub operation: String,
    /// Recommended cost in Soroban CPU instructions (relative to 10 000
    /// instructions == 1 ms on a baseline c6g.large instance).
    pub cpu_instructions: u64,
    /// Recommended cost in Soroban memory bytes.
    pub memory_bytes: u64,
    /// The benchmark result that drove this recommendation.
    pub derived_from: String,
    /// Confidence level: "high" (< 5 % stddev), "medium" (5–15 %), "low" (> 15 %).
    pub confidence: String,
}

/// Full calibration output: hardware context + benchmark data + gas tiers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationProfile {
    /// Schema version for forward compatibility.
    pub schema_version: String,
    /// ISO-8601 timestamp of this calibration run.
    pub generated_at: String,
    /// Hardware environment where this calibration was performed.
    pub hardware: HardwareProfile,
    /// Raw benchmark results.
    pub benchmarks: Vec<BenchmarkResult>,
    /// Derived Soroban gas tuning tiers.
    pub gas_tiers: Vec<GasTier>,
    /// Variance report: benchmarks with coefficient of variation > 5 %.
    pub variance_report: Vec<VarianceEntry>,
}

/// An entry in the variance report, flagging benchmarks with high noise.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VarianceEntry {
    pub benchmark: String,
    /// Coefficient of variation (stddev / mean) expressed as a percentage.
    pub cv_percent: f64,
    pub recommendation: String,
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

fn main() -> Result<()> {
    let cli = Cli::parse();

    eprintln!("[gas-calibrator] Soroban Gas Metering Calibrator");
    eprintln!(
        "[gas-calibrator] iterations={}, cpu={}, warmup={}ms, no-pin={}",
        cli.iterations, cli.cpu, cli.warmup_ms, cli.no_pin
    );

    // --- 1. Hardware profiling & CPU pinning ---------------------------------
    eprintln!("[gas-calibrator] Profiling hardware...");
    let hw = if cli.no_pin {
        // Build a profile without attempting to pin.
        profiler::profile_and_pin(usize::MAX)?
    } else {
        let hw = profiler::profile_and_pin(cli.cpu)?;
        eprintln!(
            "[gas-calibrator] Pinned to CPU {:?}",
            hw.pinned_cpu
        );
        hw
    };

    eprintln!(
        "[gas-calibrator] Hardware: {} × {} @ {} MHz, {} MiB RAM, {}",
        hw.cpu_count, hw.cpu_brand, hw.cpu_freq_mhz, hw.total_ram_mib, hw.os_info
    );

    // Attempt to raise scheduling priority for lower noise.
    if !cli.no_pin {
        let raised = profiler::try_raise_priority();
        if raised {
            eprintln!("[gas-calibrator] Process priority raised.");
        }
    }

    // --- 2. Warm-up ----------------------------------------------------------
    eprintln!(
        "[gas-calibrator] Warming up for {} ms...",
        cli.warmup_ms
    );
    profiler::warm_up(cli.warmup_ms);

    // --- 3. Run benchmarks ---------------------------------------------------
    eprintln!(
        "[gas-calibrator] Running {} iterations per benchmark...",
        cli.iterations
    );
    let bench_results = benchmarks::run_all(cli.iterations)
        .context("benchmark suite failed")?;

    for r in &bench_results {
        eprintln!(
            "[gas-calibrator]   {:30} mean={:.1}µs  p99={:.1}µs  cv={:.1}%",
            r.name,
            r.mean_micros,
            r.p99_micros,
            coefficient_of_variation(r)
        );
    }

    // --- 4. Derive gas tiers -------------------------------------------------
    let gas_tiers = derive_gas_tiers(&bench_results, &hw);

    // --- 5. Build variance report --------------------------------------------
    let variance_report = build_variance_report(&bench_results);

    // --- 6. Assemble profile -------------------------------------------------
    let profile = CalibrationProfile {
        schema_version: "1.0".to_string(),
        generated_at: timestamp_now(),
        hardware: hw,
        benchmarks: bench_results,
        gas_tiers,
        variance_report,
    };

    // --- 7. Serialise output -------------------------------------------------
    let serialised = match cli.format {
        OutputFormat::Json => {
            serde_json::to_string_pretty(&profile).context("JSON serialisation failed")?
        }
        OutputFormat::Yaml => {
            serde_yaml::to_string(&profile).context("YAML serialisation failed")?
        }
    };

    match cli.output {
        Some(ref path) => {
            fs::write(path, &serialised)
                .with_context(|| format!("could not write to {}", path.display()))?;
            eprintln!("[gas-calibrator] Profile written to {}", path.display());
        }
        None => {
            let stdout = io::stdout();
            stdout
                .lock()
                .write_all(serialised.as_bytes())
                .context("could not write to stdout")?;
            writeln!(io::stdout().lock())?;
        }
    }

    // Exit with error if variance is critically high (> 20 % CV on any benchmark).
    let critical = profile
        .variance_report
        .iter()
        .any(|v| v.cv_percent > 20.0);
    if critical {
        eprintln!(
            "[gas-calibrator] WARNING: One or more benchmarks have CV > 20 %. \
             Consider re-running with --no-pin disabled and a quieter system."
        );
        std::process::exit(1);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Gas tier derivation
// ---------------------------------------------------------------------------

/// Derive Soroban gas tiers from benchmark results.
///
/// The approach:
/// 1. Establish a baseline using the arithmetic loop (pure ALU throughput).
/// 2. Scale each benchmark's mean time relative to the baseline to get a
///    relative cost factor.
/// 3. Apply Soroban's instruction unit: 10 000 CPU instructions ≈ 1 ms on
///    a reference c6g.large (2 vCPU ARM Graviton3, ~3 GHz).
/// 4. Adjust for the measured CPU frequency.
fn derive_gas_tiers(results: &[BenchmarkResult], hw: &HardwareProfile) -> Vec<GasTier> {
    // Reference: 10 000 instructions per µs on a 3 GHz machine.
    // Scale by actual CPU frequency if known.
    let freq_ghz = if hw.cpu_freq_mhz > 0 {
        hw.cpu_freq_mhz as f64 / 1000.0
    } else {
        3.0 // safe fallback
    };
    let instructions_per_us = 10_000.0 * (freq_ghz / 3.0);

    let mut tiers = Vec::new();

    // Helper closure: find a benchmark by name
    let find = |name: &str| -> Option<&BenchmarkResult> {
        results.iter().find(|r| r.name == name)
    };

    // --- Compute hash (sha256) ---
    if let Some(sha) = find("sha256_hash_loop") {
        let cost = (sha.mean_micros * instructions_per_us).round() as u64;
        tiers.push(GasTier {
            operation: "vm_cached_instance_invoke_host_function_compute_hash_sha256".to_string(),
            cpu_instructions: cost,
            memory_bytes: 64,
            derived_from: "sha256_hash_loop".to_string(),
            confidence: confidence_label(sha),
        });
    }

    // --- Blake3 hash (history archive) ---
    if let Some(b3) = find("blake3_hash_loop") {
        let cost = (b3.mean_micros * instructions_per_us).round() as u64;
        tiers.push(GasTier {
            operation: "vm_cached_instance_invoke_host_function_compute_hash_blake3".to_string(),
            cpu_instructions: cost,
            memory_bytes: 64,
            derived_from: "blake3_hash_loop".to_string(),
            confidence: confidence_label(b3),
        });
    }

    // --- Memory allocation ---
    if let Some(mem) = find("memory_alloc") {
        // Memory cost is proportional to allocation time; 64 KiB per page.
        let cost_per_page = (mem.mean_micros * instructions_per_us).round() as u64;
        tiers.push(GasTier {
            operation: "vm_wasm_insn_cost_memory_grow".to_string(),
            cpu_instructions: cost_per_page,
            memory_bytes: 65_536,
            derived_from: "memory_alloc".to_string(),
            confidence: confidence_label(mem),
        });
    }

    // --- Integer arithmetic (ALU throughput baseline) ---
    if let Some(arith) = find("arithmetic_loop") {
        let cost = (arith.mean_micros * instructions_per_us).round() as u64;
        tiers.push(GasTier {
            operation: "vm_wasm_insn_cost_i64_mul_add".to_string(),
            cpu_instructions: cost,
            memory_bytes: 0,
            derived_from: "arithmetic_loop".to_string(),
            confidence: confidence_label(arith),
        });
    }

    // --- Branch-heavy code ---
    if let Some(br) = find("branch_heavy") {
        let cost = (br.mean_micros * instructions_per_us).round() as u64;
        tiers.push(GasTier {
            operation: "vm_wasm_insn_cost_branch".to_string(),
            cpu_instructions: cost,
            memory_bytes: 0,
            derived_from: "branch_heavy".to_string(),
            confidence: confidence_label(br),
        });
    }

    // --- Memory copy bandwidth ---
    if let Some(mc) = find("memory_copy") {
        // Cost expressed per byte: total cost / 32768 bytes per iteration.
        let total_cost = (mc.mean_micros * instructions_per_us).round() as u64;
        let per_byte = (total_cost / 32_768).max(1);
        tiers.push(GasTier {
            operation: "vm_wasm_insn_cost_mem_cpy_per_byte".to_string(),
            cpu_instructions: per_byte,
            memory_bytes: 1,
            derived_from: "memory_copy".to_string(),
            confidence: confidence_label(mc),
        });
    }

    tiers
}

// ---------------------------------------------------------------------------
// Variance report
// ---------------------------------------------------------------------------

fn build_variance_report(results: &[BenchmarkResult]) -> Vec<VarianceEntry> {
    results
        .iter()
        .filter_map(|r| {
            let cv = coefficient_of_variation(r);
            if cv > 5.0 {
                Some(VarianceEntry {
                    benchmark: r.name.clone(),
                    cv_percent: cv,
                    recommendation: if cv > 20.0 {
                        "Critical: re-run on isolated hardware or with --no-pin disabled. \
                         Do NOT use this profile in production."
                            .to_string()
                    } else if cv > 10.0 {
                        "Moderate noise detected. Consider closing background applications \
                         and re-running."
                            .to_string()
                    } else {
                        "Slight variance. Acceptable for development; re-run for production \
                         deployments."
                            .to_string()
                    },
                })
            } else {
                None
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Utility helpers
// ---------------------------------------------------------------------------

fn coefficient_of_variation(r: &BenchmarkResult) -> f64 {
    if r.mean_micros == 0.0 {
        return 0.0;
    }
    (r.stddev_micros / r.mean_micros) * 100.0
}

fn confidence_label(r: &BenchmarkResult) -> String {
    let cv = coefficient_of_variation(r);
    if cv < 5.0 {
        "high".to_string()
    } else if cv < 15.0 {
        "medium".to_string()
    } else {
        "low".to_string()
    }
}

/// Return an ISO-8601 UTC timestamp string.
fn timestamp_now() -> String {
    // Use a simple RFC 3339-compatible format without pulling in chrono.
    // std::time gives us seconds since UNIX epoch.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Convert epoch seconds to a human-readable UTC date-time string.
    // We implement a minimal converter to avoid adding a dependency.
    let (y, mo, d, h, mi, s) = epoch_to_utc(secs);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, mo, d, h, mi, s)
}

/// Minimal UNIX epoch → UTC (year, month, day, hour, minute, second).
fn epoch_to_utc(epoch: u64) -> (u32, u32, u32, u32, u32, u32) {
    let s = epoch % 60;
    let epoch = epoch / 60;
    let mi = epoch % 60;
    let epoch = epoch / 60;
    let h = epoch % 24;
    let days = epoch / 24;

    // Days since 1970-01-01
    let mut year = 1970u32;
    let mut rem = days;
    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        if rem < days_in_year {
            break;
        }
        rem -= days_in_year;
        year += 1;
    }
    let months = [31u32, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month = 1u32;
    for (i, &m) in months.iter().enumerate() {
        let days_in_month = if i == 1 && is_leap(year) { 29 } else { m };
        if rem < days_in_month {
            break;
        }
        rem -= days_in_month;
        month += 1;
    }
    let day = rem + 1;
    (year, month, day, h as u32, mi as u32, s as u32)
}

fn is_leap(y: u32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}
