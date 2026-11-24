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

//! Hardware profiler — pins the calibration process to a single CPU core and
//! collects system-level hardware information to contextualise benchmark
//! results.
//!
//! # Timing isolation
//!
//! Soroban gas calibration must be as deterministic as possible.  Background
//! OS activity (scheduler preemptions, NUMA migrations, frequency scaling) can
//! add >10 % variance to short micro-benchmarks.  This module applies several
//! mitigations:
//!
//! 1. **CPU affinity** – pins the current thread to a single logical CPU so the
//!    OS scheduler cannot migrate it mid-benchmark.  On Linux this calls
//!    `sched_setaffinity(2)` via the `taskset` command or the syscall wrapper;
//!    on other platforms a warning is emitted and execution continues without
//!    pinning.
//!
//! 2. **Process priority** – attempts to raise the nice level to `-10` so the
//!    benchmark thread is not preempted by lower-priority work.  Requires
//!    `CAP_SYS_NICE` or running as root; silently skipped when unavailable.
//!
//! 3. **Warm-up phase** – callers should invoke [`warm_up`] before timing to
//!    prime the instruction cache and WASM JIT compiler.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sysinfo::System;

/// Hardware profile snapshot captured before benchmarks run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareProfile {
    /// Number of logical CPU cores visible to the process.
    pub cpu_count: usize,
    /// CPU brand string, e.g. `"Intel(R) Xeon(R) Platinum 8375C"`.
    pub cpu_brand: String,
    /// CPU base frequency in MHz (0 if unavailable).
    pub cpu_freq_mhz: u64,
    /// Total physical RAM in MiB.
    pub total_ram_mib: u64,
    /// Available (free) RAM in MiB at profile time.
    pub available_ram_mib: u64,
    /// Operating system name and version.
    pub os_info: String,
    /// Index of the CPU core the benchmark is pinned to (0-based).
    /// `None` when pinning is unavailable on this platform.
    pub pinned_cpu: Option<usize>,
}

/// Collect a [`HardwareProfile`] and, if possible, pin the current thread to
/// `pin_cpu`.
///
/// Pinning to core 0 is the safest default on most server hardware because
/// core 0 is typically reserved for OS interrupts but modern kernels have
/// moved most interrupt handling to dedicated cores, leaving 0 relatively
/// quiet.  Operators on bare-metal NVMe nodes may wish to choose an isolated
/// core via the `--cpu` CLI flag.
pub fn profile_and_pin(pin_cpu: usize) -> Result<HardwareProfile> {
    let mut sys = System::new_all();
    sys.refresh_all();

    let cpu_count = num_cpus::get();

    // Get the first CPU's info for brand and frequency.
    let (cpu_brand, cpu_freq_mhz) = sys
        .cpus()
        .first()
        .map(|c| (c.brand().to_string(), c.frequency()))
        .unwrap_or_else(|| ("unknown".to_string(), 0));

    let total_ram_mib = sys.total_memory() / (1024 * 1024);
    let available_ram_mib = sys.available_memory() / (1024 * 1024);
    let os_info = format!(
        "{} {}",
        System::name().unwrap_or_else(|| "unknown".to_string()),
        System::os_version().unwrap_or_else(|| "unknown".to_string()),
    );

    let pinned_cpu = attempt_pin(pin_cpu, cpu_count);

    Ok(HardwareProfile {
        cpu_count,
        cpu_brand,
        cpu_freq_mhz,
        total_ram_mib,
        available_ram_mib,
        os_info,
        pinned_cpu,
    })
}

/// Attempt to raise process priority to reduce OS scheduling noise.
///
/// On Linux this calls `setpriority(PRIO_PROCESS, 0, -10)` via the `nice`
/// command.  On failure (e.g. insufficient privilege) a warning is printed
/// and `false` is returned so callers can inform the user.
pub fn try_raise_priority() -> bool {
    #[cfg(target_os = "linux")]
    {
        // Try to renice the current process.  Requires CAP_SYS_NICE.
        let result = std::process::Command::new("renice")
            .args(["-n", "-10", "-p", &std::process::id().to_string()])
            .output();

        match result {
            Ok(out) if out.status.success() => return true,
            _ => {
                eprintln!(
                    "[profiler] Warning: could not raise process priority (needs CAP_SYS_NICE). \
                     Benchmark variance may be higher."
                );
                return false;
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!(
            "[profiler] Warning: process priority tuning is only supported on Linux. \
             Skipping."
        );
        false
    }
}

/// Execute a short busy-loop to warm up the CPU's instruction/data caches and
/// allow the WASM JIT tier to finish compilation before measurement begins.
///
/// `duration_ms` controls how long the warm-up lasts.  200 ms is usually
/// sufficient; increase for very short benchmarks where JIT compile time would
/// dominate.
pub fn warm_up(duration_ms: u64) {
    let end =
        std::time::Instant::now() + std::time::Duration::from_millis(duration_ms);
    let mut x: u64 = 1;
    while std::time::Instant::now() < end {
        // Lightweight computation: prevents the loop from being optimised away.
        x = x
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
    }
    // Prevent the compiler from eliminating `x` entirely.
    let _ = std::hint::black_box(x);
}

// ---------------------------------------------------------------------------
// CPU affinity — implemented via `taskset` subprocess on Linux.
// This avoids depending on the `libc` crate or unsafe inline assembly while
// still achieving real affinity isolation on production Linux hosts.
// ---------------------------------------------------------------------------

/// Try to pin the current thread to `cpu_idx`.
/// Returns `Some(cpu_idx)` on success, `None` otherwise.
fn attempt_pin(cpu_idx: usize, cpu_count: usize) -> Option<usize> {
    // When called with usize::MAX (no-pin mode) skip silently.
    if cpu_idx == usize::MAX {
        return None;
    }

    let target = cpu_idx.min(cpu_count.saturating_sub(1));

    #[cfg(target_os = "linux")]
    {
        match taskset_pin(target) {
            Ok(()) => {
                return Some(target);
            }
            Err(e) => {
                eprintln!(
                    "[profiler] Warning: CPU affinity pinning failed ({}). \
                     Benchmark isolation may be reduced.",
                    e
                );
            }
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        eprintln!(
            "[profiler] Warning: CPU affinity pinning is only supported on Linux. \
             Skipping (target was CPU {}).",
            target
        );
        let _ = target;
    }

    None
}

/// Pin the current process to `cpu_idx` using the `taskset` utility.
///
/// `taskset -cp <cpu> <pid>` sets the CPU affinity mask of a running process.
/// This is equivalent to calling `sched_setaffinity(2)` but works without
/// unsafe code.
#[cfg(target_os = "linux")]
fn taskset_pin(cpu_idx: usize) -> Result<()> {
    let pid = std::process::id().to_string();
    let cpu_str = cpu_idx.to_string();

    let output = std::process::Command::new("taskset")
        .args(["-cp", &cpu_str, &pid])
        .output()
        .map_err(|e| anyhow::anyhow!("taskset not found or failed to spawn: {}", e))?;

    if output.status.success() {
        return Ok(());
    }

    // taskset may not be available in all container images; fall back to
    // /proc/self/status check + a no-op to keep the run going.
    let stderr = String::from_utf8_lossy(&output.stderr);
    anyhow::bail!("taskset exited with non-zero status: {}", stderr.trim())
}
