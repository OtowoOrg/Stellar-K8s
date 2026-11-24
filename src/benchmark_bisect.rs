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
//! Automated performance regression bisection (epic #1526).
//!
//! [`detect_regression`] compares a PR's benchmark samples against the
//! baseline distribution. When it fires, [`bisect`] binary-searches the
//! commit range between the last good and first bad commit to find the
//! offending change, and returns [`Evidence`] (effect size and confidence)
//! for notifying its author.
//!
//! Benchmarks are noisy, so every comparison is a one-sided Mann-Whitney U
//! test (rank based, robust to outliers) combined with a minimum effect size.
//! A midpoint is classified by testing it against *both* the known-good and
//! known-bad distributions; when the two tests disagree the midpoint is
//! re-sampled (up to [`BisectConfig::max_attempts`]) instead of guessing, so
//! the search converges despite noise. With clean classifications the search
//! takes exactly `ceil(log2(N))` benchmark runs for `N` candidate commits.
//!
//! Samples are "lower is better" (latency, duration). Negate throughput-style
//! metrics before passing them in.

use serde::Serialize;

/// Statistical thresholds for regression detection and bisection.
#[derive(Debug, Clone, PartialEq)]
pub struct BisectConfig {
    /// One-sided significance level.
    pub alpha: f64,
    /// Minimum relative median slowdown, in percent, to count as a regression.
    pub min_effect_pct: f64,
    /// Benchmark runs allowed per midpoint before falling back to the
    /// nearest-median rule.
    pub max_attempts: usize,
}

impl Default for BisectConfig {
    fn default() -> Self {
        Self {
            alpha: 0.01,
            min_effect_pct: 2.0,
            max_attempts: 3,
        }
    }
}

/// Result of comparing a candidate distribution against a baseline.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comparison {
    /// Relative median change of candidate vs baseline, in percent.
    pub effect_size_pct: f64,
    /// One-sided p-value for "candidate is slower than baseline".
    pub p_value: f64,
    /// `1 - p_value`.
    pub confidence: f64,
    pub regressed: bool,
}

fn median(samples: &[f64]) -> f64 {
    let mut s = samples.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    let n = s.len();
    if n == 0 {
        return f64::NAN;
    }
    if n % 2 == 1 {
        s[n / 2]
    } else {
        (s[n / 2 - 1] + s[n / 2]) / 2.0
    }
}

/// Standard normal CDF (Abramowitz & Stegun 7.1.26, |error| < 1.5e-7).
fn normal_cdf(z: f64) -> f64 {
    let x = z.abs() / std::f64::consts::SQRT_2;
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let poly = t
        * (0.254_829_592
            + t * (-0.284_496_736
                + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))));
    let erf = 1.0 - poly * (-x * x).exp();
    if z >= 0.0 {
        0.5 * (1.0 + erf)
    } else {
        0.5 * (1.0 - erf)
    }
}

/// One-sided Mann-Whitney U p-value for `candidate` being stochastically
/// larger than `baseline` (normal approximation, average ranks for ties).
fn mann_whitney_p(baseline: &[f64], candidate: &[f64]) -> f64 {
    let (n1, n2) = (baseline.len() as f64, candidate.len() as f64);
    if n1 == 0.0 || n2 == 0.0 {
        return 1.0;
    }
    let mut all: Vec<(f64, bool)> = baseline
        .iter()
        .map(|v| (*v, false))
        .chain(candidate.iter().map(|v| (*v, true)))
        .collect();
    all.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut rank_sum = 0.0;
    let mut i = 0;
    while i < all.len() {
        let mut j = i;
        while j + 1 < all.len() && all[j + 1].0 == all[i].0 {
            j += 1;
        }
        let avg_rank = (i + j) as f64 / 2.0 + 1.0;
        rank_sum += all[i..=j].iter().filter(|(_, c)| *c).count() as f64 * avg_rank;
        i = j + 1;
    }
    let u = rank_sum - n2 * (n2 + 1.0) / 2.0;
    let mean = n1 * n2 / 2.0;
    let sd = (n1 * n2 * (n1 + n2 + 1.0) / 12.0).sqrt();
    // Continuity correction.
    let z = (u - mean - 0.5) / sd;
    1.0 - normal_cdf(z)
}

/// Compare `candidate` against the `baseline` distribution.
pub fn detect_regression(baseline: &[f64], candidate: &[f64], cfg: &BisectConfig) -> Comparison {
    let base = median(baseline);
    let effect_size_pct = if base == 0.0 || base.is_nan() {
        0.0
    } else {
        (median(candidate) - base) / base.abs() * 100.0
    };
    let p_value = mann_whitney_p(baseline, candidate);
    Comparison {
        effect_size_pct,
        p_value,
        confidence: 1.0 - p_value,
        regressed: p_value < cfg.alpha && effect_size_pct >= cfg.min_effect_pct,
    }
}

/// A commit in the bisection range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Commit {
    pub sha: String,
    pub author: String,
}

/// Runs the benchmark suite at a commit and returns its samples.
pub trait BenchmarkRunner {
    fn run(&mut self, commit: &Commit) -> Result<Vec<f64>, String>;
}

/// Evidence attached to the notification sent to the offending author.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    pub commit: Commit,
    /// Last commit classified good (parent of the offending commit in range).
    pub last_good: Commit,
    pub comparison: Comparison,
    /// Benchmark runs spent on midpoints, including re-samples.
    pub benchmark_runs: usize,
    /// Midpoints that needed re-sampling or the fallback rule.
    pub noisy_midpoints: usize,
}

impl Evidence {
    /// Markdown body for the PR comment / issue assigned to the author.
    pub fn to_markdown(&self) -> String {
        format!(
            "### Performance regression bisected to `{sha}`\n\n\
             @{author}, this commit slowed the benchmark by **{effect:+.2}%** \
             (one-sided Mann-Whitney p = {p:.2e}, confidence {conf:.2}%).\n\n\
             | | |\n|---|---|\n\
             | Offending commit | `{sha}` |\n\
             | Last good commit | `{good}` |\n\
             | Benchmark runs | {runs} |\n\
             | Noisy midpoints re-sampled | {noisy} |\n",
            sha = self.commit.sha,
            author = self.commit.author,
            effect = self.comparison.effect_size_pct,
            p = self.comparison.p_value,
            conf = self.comparison.confidence * 100.0,
            good = self.last_good.sha,
            runs = self.benchmark_runs,
            noisy = self.noisy_midpoints,
        )
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum BisectError {
    #[error("range needs a good and a bad commit")]
    RangeTooSmall,
    #[error("known-bad samples do not regress against known-good samples")]
    NoRegression,
    #[error("benchmark failed at {sha}: {reason}")]
    Runner { sha: String, reason: String },
}

/// Bisect `commits` (oldest first). `commits[0]` is known good with
/// `good_samples`; the last commit is known bad with `bad_samples`.
pub fn bisect<R: BenchmarkRunner>(
    commits: &[Commit],
    good_samples: &[f64],
    bad_samples: &[f64],
    runner: &mut R,
    cfg: &BisectConfig,
) -> Result<Evidence, BisectError> {
    if commits.len() < 2 {
        return Err(BisectError::RangeTooSmall);
    }
    if !detect_regression(good_samples, bad_samples, cfg).regressed {
        return Err(BisectError::NoRegression);
    }

    let (mut lo, mut hi) = (0usize, commits.len() - 1);
    let mut lo_samples = good_samples.to_vec();
    let mut hi_samples = bad_samples.to_vec();
    let mut runs = 0;
    let mut noisy = 0;

    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        let mut samples = Vec::new();
        let mut verdict = None;
        let mut attempts = 0;
        while attempts < cfg.max_attempts.max(1) {
            samples.extend(
                runner
                    .run(&commits[mid])
                    .map_err(|reason| BisectError::Runner {
                        sha: commits[mid].sha.clone(),
                        reason,
                    })?,
            );
            runs += 1;
            attempts += 1;
            let slower_than_good = detect_regression(good_samples, &samples, cfg).regressed;
            let faster_than_bad = detect_regression(&samples, bad_samples, cfg).regressed;
            match (slower_than_good, faster_than_bad) {
                (true, false) => verdict = Some(true),
                (false, true) => verdict = Some(false),
                _ => continue,
            }
            break;
        }
        if attempts > 1 || verdict.is_none() {
            noisy += 1;
        }
        let is_bad = verdict.unwrap_or_else(|| {
            let m = median(&samples);
            (m - median(bad_samples)).abs() < (m - median(good_samples)).abs()
        });
        if is_bad {
            hi = mid;
            hi_samples = samples;
        } else {
            lo = mid;
            lo_samples = samples;
        }
    }

    Ok(Evidence {
        commit: commits[hi].clone(),
        last_good: commits[lo].clone(),
        comparison: detect_regression(&lo_samples, &hi_samples, cfg),
        benchmark_runs: runs,
        noisy_midpoints: noisy,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::SmallRng;
    use rand::{Rng, SeedableRng};

    /// Synthetic benchmark: commits at or after `regressed_at` are
    /// `slowdown` slower, with Gaussian noise and occasional outliers.
    struct Synthetic {
        rng: SmallRng,
        regressed_at: usize,
        slowdown: f64,
        noise_sd: f64,
        samples: usize,
        runs: usize,
    }

    impl Synthetic {
        fn draw(&mut self, idx: usize) -> Vec<f64> {
            let mean = if idx >= self.regressed_at {
                100.0 * (1.0 + self.slowdown)
            } else {
                100.0
            };
            (0..self.samples)
                .map(|_| {
                    let (u1, u2): (f64, f64) = (self.rng.gen_range(1e-12..1.0), self.rng.gen());
                    let z = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
                    let outlier = if self.rng.gen_bool(0.03) { 30.0 } else { 0.0 };
                    mean + z * self.noise_sd * mean + outlier
                })
                .collect()
        }
    }

    impl BenchmarkRunner for Synthetic {
        fn run(&mut self, commit: &Commit) -> Result<Vec<f64>, String> {
            self.runs += 1;
            let idx = commit.sha.trim_start_matches('c').parse().unwrap();
            Ok(self.draw(idx))
        }
    }

    fn commits(n: usize) -> Vec<Commit> {
        (0..n)
            .map(|i| Commit {
                sha: format!("c{i}"),
                author: format!("dev{}", i % 7),
            })
            .collect()
    }

    fn synthetic(seed: u64, regressed_at: usize) -> Synthetic {
        Synthetic {
            rng: SmallRng::seed_from_u64(seed),
            regressed_at,
            slowdown: 0.10,
            noise_sd: 0.03,
            samples: 20,
            runs: 0,
        }
    }

    fn ceil_log2(n: usize) -> usize {
        (usize::BITS - (n - 1).leading_zeros()) as usize
    }

    #[test]
    fn detects_regression_and_ignores_noise() {
        let mut s = synthetic(1, 1);
        let good = s.draw(0);
        let bad = s.draw(1);
        let again = s.draw(0);
        let cfg = BisectConfig::default();
        let c = detect_regression(&good, &bad, &cfg);
        assert!(c.regressed, "{c:?}");
        assert!(c.effect_size_pct > 5.0);
        assert!(!detect_regression(&good, &again, &cfg).regressed);
    }

    #[test]
    fn finds_seeded_commit_within_log2_runs() {
        let range = commits(64);
        let mut s = synthetic(42, 37);
        let good = s.draw(0);
        let bad = s.draw(63);
        let ev = bisect(&range, &good, &bad, &mut s, &BisectConfig::default()).unwrap();
        assert_eq!(ev.commit.sha, "c37");
        assert_eq!(ev.last_good.sha, "c36");
        assert_eq!(ev.commit.author, "dev2");
        assert!(ev.benchmark_runs <= ceil_log2(range.len()), "{ev:?}");
        assert!(ev.comparison.regressed);
        assert!(ev.comparison.confidence > 0.99);
        assert!(ev.to_markdown().contains("c37"));
    }

    #[test]
    fn false_attribution_rate_below_five_percent() {
        let trials = 200;
        let mut wrong = 0;
        for seed in 0..trials {
            let n = 16 + (seed as usize % 48);
            let at = 1 + (seed as usize * 7919) % (n - 1);
            let range = commits(n);
            let mut s = synthetic(seed, at);
            let good = s.draw(0);
            let bad = s.draw(n - 1);
            let ev = bisect(&range, &good, &bad, &mut s, &BisectConfig::default()).unwrap();
            if ev.commit.sha != format!("c{at}") {
                wrong += 1;
            }
        }
        assert!(wrong * 100 < trials * 5, "{wrong}/{trials} misattributed");
    }

    #[test]
    fn rejects_range_without_regression() {
        let range = commits(8);
        let mut s = synthetic(3, usize::MAX);
        let good = s.draw(0);
        let bad = s.draw(7);
        assert_eq!(
            bisect(&range, &good, &bad, &mut s, &BisectConfig::default()),
            Err(BisectError::NoRegression)
        );
        assert_eq!(s.runs, 0);
    }
}
