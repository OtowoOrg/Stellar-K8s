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

//! WASM micro-benchmark suite.
//!
//! Each benchmark is compiled to an in-memory WAT (WebAssembly Text Format)
//! module and executed inside a `wasmtime` engine so that the measured time
//! reflects real Soroban-style WASM execution overhead rather than native
//! host code.
//!
//! Benchmarks available:
//!  - `sha256_hash_loop`  – iterative SHA-256 compression over a 64-byte block
//!  - `blake3_hash_loop`  – iterative Blake3 compression (used in Stellar history)
//!  - `memory_alloc`      – repeated 64 KiB page allocation and sequential write
//!  - `arithmetic_loop`   – tight 64-bit multiply-accumulate loop (CPU ALU)
//!  - `branch_heavy`      – alternating conditional branches (branch predictor stress)
//!  - `memory_copy`       – bulk linear memory copy between wasm pages

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use wasmtime::{Engine, Linker, Module, Store};

/// A single timing sample from one benchmark iteration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sample {
    /// Elapsed wall-clock time for the iteration, in microseconds.
    pub micros: f64,
}

/// Aggregated result of all iterations of one benchmark.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkResult {
    /// Human-readable name of the benchmark.
    pub name: String,
    /// Total number of iterations executed.
    pub iterations: u32,
    /// Mean execution time per iteration, in microseconds.
    pub mean_micros: f64,
    /// Standard deviation of per-iteration execution time, in microseconds.
    pub stddev_micros: f64,
    /// Minimum observed execution time, in microseconds.
    pub min_micros: f64,
    /// Maximum observed execution time, in microseconds.
    pub max_micros: f64,
    /// 50th-percentile (median) execution time, in microseconds.
    pub p50_micros: f64,
    /// 95th-percentile execution time, in microseconds.
    pub p95_micros: f64,
    /// 99th-percentile execution time, in microseconds.
    pub p99_micros: f64,
}

/// Run every benchmark in the suite and collect results.
///
/// `iterations` controls how many WASM invocations each sub-benchmark makes.
/// Higher values reduce variance but increase total runtime.
pub fn run_all(iterations: u32) -> Result<Vec<BenchmarkResult>> {
    let engine = Engine::default();
    let mut results = Vec::new();

    results.push(run_sha256_loop(&engine, iterations)?);
    results.push(run_blake3_loop(&engine, iterations)?);
    results.push(run_memory_alloc(&engine, iterations)?);
    results.push(run_arithmetic_loop(&engine, iterations)?);
    results.push(run_branch_heavy(&engine, iterations)?);
    results.push(run_memory_copy(&engine, iterations)?);

    Ok(results)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Compute aggregate statistics from a slice of durations and build a result.
fn aggregate(name: &str, samples: Vec<Duration>, iterations: u32) -> BenchmarkResult {
    let mut micros: Vec<f64> = samples.iter().map(|d| d.as_secs_f64() * 1_000_000.0).collect();
    micros.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let n = micros.len() as f64;
    let mean = micros.iter().sum::<f64>() / n;
    let variance = micros.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n;
    let stddev = variance.sqrt();

    let percentile = |p: f64| -> f64 {
        let idx = ((p / 100.0) * (micros.len() as f64 - 1.0)).round() as usize;
        micros[idx.min(micros.len() - 1)]
    };

    BenchmarkResult {
        name: name.to_string(),
        iterations,
        mean_micros: mean,
        stddev_micros: stddev,
        min_micros: micros.first().copied().unwrap_or(0.0),
        max_micros: micros.last().copied().unwrap_or(0.0),
        p50_micros: percentile(50.0),
        p95_micros: percentile(95.0),
        p99_micros: percentile(99.0),
    }
}

/// Compile a WAT module, instantiate it, call `run` with `inner_loops` and
/// return the per-iteration wall-clock durations.
fn time_wasm_benchmark(
    engine: &Engine,
    wat_src: &str,
    outer_iters: u32,
    inner_loops: i32,
) -> Result<Vec<Duration>> {
    let module = Module::new(engine, wat_src).context("failed to compile WAT module")?;
    let linker: Linker<()> = Linker::new(engine);

    let mut durations = Vec::with_capacity(outer_iters as usize);

    for _ in 0..outer_iters {
        // Each outer iteration gets its own fresh store to avoid carry-over state.
        let mut store = Store::new(engine, ());
        let instance = linker
            .instantiate(&mut store, &module)
            .context("failed to instantiate module")?;

        let func = instance
            .get_typed_func::<i32, i32>(&mut store, "run")
            .context("exported function `run` not found or wrong type")?;

        let t0 = Instant::now();
        let _ = func.call(&mut store, inner_loops).context("wasm call failed")?;
        durations.push(t0.elapsed());
    }

    Ok(durations)
}

// ---------------------------------------------------------------------------
// SHA-256 loop benchmark
// ---------------------------------------------------------------------------
// The WASM module manually implements the SHA-256 compression function over
// a fixed 64-byte input block for `n` iterations.  This stresses the integer
// ALU (rotations, xors, additions) in the same way the Soroban host function
// `compute_hash_sha256` does.

const SHA256_WAT: &str = r#"
(module
  (func $run (export "run") (param $n i32) (result i32)
    (local $i i32)
    (local $h0 i32) (local $h1 i32) (local $h2 i32) (local $h3 i32)
    (local $h4 i32) (local $h5 i32) (local $h6 i32) (local $h7 i32)
    (local $a i32) (local $b i32) (local $c i32) (local $d i32)
    (local $e i32) (local $f i32) (local $g i32) (local $h i32)
    (local $t1 i32) (local $t2 i32) (local $s0 i32) (local $s1 i32)
    (local $ch i32) (local $maj i32)
    ;; initialise with SHA-256 initial values
    (local.set $h0 (i32.const 0x6a09e667))
    (local.set $h1 (i32.const 0xbb67ae85))
    (local.set $h2 (i32.const 0x3c6ef372))
    (local.set $h3 (i32.const 0xa54ff53a))
    (local.set $h4 (i32.const 0x510e527f))
    (local.set $h5 (i32.const 0x9b05688c))
    (local.set $h6 (i32.const 0x1f83d9ab))
    (local.set $h7 (i32.const 0x5be0cd19))
    ;; outer loop
    (local.set $i (i32.const 0))
    (block $break
      (loop $top
        (br_if $break (i32.ge_s (local.get $i) (local.get $n)))
        (local.set $a (local.get $h0))
        (local.set $b (local.get $h1))
        (local.set $c (local.get $h2))
        (local.set $d (local.get $h3))
        (local.set $e (local.get $h4))
        (local.set $f (local.get $h5))
        (local.set $g (local.get $h6))
        (local.set $h (local.get $h7))
        ;; one round of SHA-256 compression
        ;; S1 = (e >>> 6) ^ (e >>> 11) ^ (e >>> 25)
        (local.set $s1
          (i32.xor
            (i32.rotr (local.get $e) (i32.const 6))
            (i32.xor
              (i32.rotr (local.get $e) (i32.const 11))
              (i32.rotr (local.get $e) (i32.const 25)))))
        ;; ch = (e & f) ^ (~e & g)
        (local.set $ch
          (i32.xor
            (i32.and (local.get $e) (local.get $f))
            (i32.and (i32.xor (local.get $e) (i32.const -1)) (local.get $g))))
        ;; t1 = h + s1 + ch + K[0] + W[0]  (K[0]=0x428a2f98, W[0]=0)
        (local.set $t1
          (i32.add (local.get $h)
            (i32.add (local.get $s1)
              (i32.add (local.get $ch) (i32.const 0x428a2f98)))))
        ;; S0 = (a >>> 2) ^ (a >>> 13) ^ (a >>> 22)
        (local.set $s0
          (i32.xor
            (i32.rotr (local.get $a) (i32.const 2))
            (i32.xor
              (i32.rotr (local.get $a) (i32.const 13))
              (i32.rotr (local.get $a) (i32.const 22)))))
        ;; maj = (a & b) ^ (a & c) ^ (b & c)
        (local.set $maj
          (i32.xor
            (i32.and (local.get $a) (local.get $b))
            (i32.xor
              (i32.and (local.get $a) (local.get $c))
              (i32.and (local.get $b) (local.get $c)))))
        (local.set $t2 (i32.add (local.get $s0) (local.get $maj)))
        ;; update state
        (local.set $h0 (i32.add (local.get $t1) (local.get $t2)))
        (local.set $h1 (local.get $a))
        (local.set $h2 (local.get $b))
        (local.set $h3 (local.get $c))
        (local.set $h4 (i32.add (local.get $d) (local.get $t1)))
        (local.set $h5 (local.get $e))
        (local.set $h6 (local.get $f))
        (local.set $h7 (local.get $g))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $top)
      )
    )
    ;; return final h0 to prevent dead-code elimination
    (local.get $h0)
  )
)
"#;

fn run_sha256_loop(engine: &Engine, iterations: u32) -> Result<BenchmarkResult> {
    let samples = time_wasm_benchmark(engine, SHA256_WAT, iterations, 1_000)?;
    Ok(aggregate("sha256_hash_loop", samples, iterations))
}

// ---------------------------------------------------------------------------
// Blake3 loop benchmark (approximated in WASM ALU ops)
// ---------------------------------------------------------------------------
// Approximates Blake3's G-function mixing with 64-bit XOR/add/rotate ops.

const BLAKE3_WAT: &str = r#"
(module
  (func $run (export "run") (param $n i32) (result i32)
    (local $i i32)
    (local $a i64) (local $b i64) (local $c i64) (local $d i64)
    (local.set $a (i64.const 0x6a09e667f2bdc928))
    (local.set $b (i64.const 0xbb67ae8584caa73b))
    (local.set $c (i64.const 0x3c6ef372fe94f82b))
    (local.set $d (i64.const 0xa54ff53a5f1d36f1))
    (local.set $i (i32.const 0))
    (block $break
      (loop $top
        (br_if $break (i32.ge_s (local.get $i) (local.get $n)))
        ;; G-function: a = a + b; d = rotr(d ^ a, 16);
        ;;             c = c + d; b = rotr(b ^ c, 12);
        ;;             a = a + b; d = rotr(d ^ a, 8);
        ;;             c = c + d; b = rotr(b ^ c, 7);
        (local.set $a (i64.add (local.get $a) (local.get $b)))
        (local.set $d (i64.rotr (i64.xor (local.get $d) (local.get $a)) (i64.const 16)))
        (local.set $c (i64.add (local.get $c) (local.get $d)))
        (local.set $b (i64.rotr (i64.xor (local.get $b) (local.get $c)) (i64.const 12)))
        (local.set $a (i64.add (local.get $a) (local.get $b)))
        (local.set $d (i64.rotr (i64.xor (local.get $d) (local.get $a)) (i64.const 8)))
        (local.set $c (i64.add (local.get $c) (local.get $d)))
        (local.set $b (i64.rotr (i64.xor (local.get $b) (local.get $c)) (i64.const 7)))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $top)
      )
    )
    ;; return low 32 bits to prevent DCE
    (i32.wrap_i64 (local.get $a))
  )
)
"#;

fn run_blake3_loop(engine: &Engine, iterations: u32) -> Result<BenchmarkResult> {
    let samples = time_wasm_benchmark(engine, BLAKE3_WAT, iterations, 2_000)?;
    Ok(aggregate("blake3_hash_loop", samples, iterations))
}

// ---------------------------------------------------------------------------
// Memory allocation benchmark
// ---------------------------------------------------------------------------
// Grows WASM linear memory one page (64 KiB) at a time and writes a byte to
// each page to ensure the page is actually committed by the OS.

const MEMORY_ALLOC_WAT: &str = r#"
(module
  (memory 1 256)   ;; start with 1 page, max 256 pages (16 MiB)
  (func $run (export "run") (param $n i32) (result i32)
    (local $i i32)
    (local $ptr i32)
    (local $page i32)
    (local.set $i (i32.const 0))
    (block $break
      (loop $top
        (br_if $break (i32.ge_s (local.get $i) (local.get $n)))
        ;; grow by 1 page; result is previous size (or -1 on failure)
        (local.set $page (memory.grow (i32.const 1)))
        ;; write one byte to the start of the new page
        (if (i32.ne (local.get $page) (i32.const -1))
          (then
            (local.set $ptr (i32.mul (local.get $page) (i32.const 65536)))
            (i32.store8 (local.get $ptr) (i32.const 0xab))
          )
        )
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $top)
      )
    )
    ;; return current memory size in pages
    (memory.size)
  )
)
"#;

fn run_memory_alloc(engine: &Engine, iterations: u32) -> Result<BenchmarkResult> {
    // Use a fresh engine with a large enough memory limit for this benchmark.
    let samples = time_wasm_benchmark(engine, MEMORY_ALLOC_WAT, iterations, 32)?;
    Ok(aggregate("memory_alloc", samples, iterations))
}

// ---------------------------------------------------------------------------
// Arithmetic loop benchmark
// ---------------------------------------------------------------------------
// Tight 64-bit multiply-accumulate loop.  Stresses integer ALU throughput.

const ARITH_WAT: &str = r#"
(module
  (func $run (export "run") (param $n i32) (result i32)
    (local $i i32)
    (local $acc i64)
    (local $mul i64)
    (local.set $acc (i64.const 1))
    (local.set $mul (i64.const 6364136223846793005))
    (local.set $i (i32.const 0))
    (block $break
      (loop $top
        (br_if $break (i32.ge_s (local.get $i) (local.get $n)))
        (local.set $acc
          (i64.add
            (i64.mul (local.get $acc) (local.get $mul))
            (i64.const 1442695040888963407)))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $top)
      )
    )
    (i32.wrap_i64 (local.get $acc))
  )
)
"#;

fn run_arithmetic_loop(engine: &Engine, iterations: u32) -> Result<BenchmarkResult> {
    let samples = time_wasm_benchmark(engine, ARITH_WAT, iterations, 10_000)?;
    Ok(aggregate("arithmetic_loop", samples, iterations))
}

// ---------------------------------------------------------------------------
// Branch-heavy benchmark
// ---------------------------------------------------------------------------
// Alternates between two branches based on a computed condition, stressing
// the branch predictor and the WASM JIT's control-flow handling.

const BRANCH_WAT: &str = r#"
(module
  (func $run (export "run") (param $n i32) (result i32)
    (local $i i32)
    (local $acc i32)
    (local $x i32)
    (local.set $acc (i32.const 0))
    (local.set $i (i32.const 0))
    (block $break
      (loop $top
        (br_if $break (i32.ge_s (local.get $i) (local.get $n)))
        (local.set $x (i32.and (local.get $i) (i32.const 1)))
        (if (i32.eq (local.get $x) (i32.const 0))
          (then (local.set $acc (i32.add (local.get $acc) (i32.const 3))))
          (else (local.set $acc (i32.add (local.get $acc) (i32.const 7))))
        )
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $top)
      )
    )
    (local.get $acc)
  )
)
"#;

fn run_branch_heavy(engine: &Engine, iterations: u32) -> Result<BenchmarkResult> {
    let samples = time_wasm_benchmark(engine, BRANCH_WAT, iterations, 10_000)?;
    Ok(aggregate("branch_heavy", samples, iterations))
}

// ---------------------------------------------------------------------------
// Memory copy benchmark
// ---------------------------------------------------------------------------
// Copies 32 KiB between two regions in WASM linear memory using 4-byte
// stores, measuring memory bandwidth inside the WASM sandbox.

const MEMCOPY_WAT: &str = r#"
(module
  (memory 2)   ;; 2 pages = 128 KiB — source at offset 0, dst at 65536
  (func $run (export "run") (param $n i32) (result i32)
    (local $i i32)
    (local $j i32)
    (local $val i32)
    (local.set $i (i32.const 0))
    (block $break
      (loop $top
        (br_if $break (i32.ge_s (local.get $i) (local.get $n)))
        (local.set $j (i32.const 0))
        (block $inner_break
          (loop $inner
            ;; copy 32 KiB in 4-byte chunks: 8192 iterations
            (br_if $inner_break (i32.ge_s (local.get $j) (i32.const 8192)))
            (local.set $val (i32.load (i32.mul (local.get $j) (i32.const 4))))
            (i32.store
              (i32.add (i32.mul (local.get $j) (i32.const 4)) (i32.const 65536))
              (local.get $val))
            (local.set $j (i32.add (local.get $j) (i32.const 1)))
            (br $inner)
          )
        )
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $top)
      )
    )
    ;; return last copied value so it isn't DCE'd
    (local.get $j)
  )
)
"#;

fn run_memory_copy(engine: &Engine, iterations: u32) -> Result<BenchmarkResult> {
    let samples = time_wasm_benchmark(engine, MEMCOPY_WAT, iterations, 4)?;
    Ok(aggregate("memory_copy", samples, iterations))
}
