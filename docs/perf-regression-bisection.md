# Automated Performance Regression Bisection

Epic: #1526 · Module: `src/benchmark_bisect.rs`

## Detection

`detect_regression(baseline, candidate, cfg)` compares a PR's benchmark
samples with the baseline distribution. It uses a one-sided Mann-Whitney U
test, which is based on ranks and robust to outliers, plus a minimum effect
size. A regression is flagged when `p < alpha` (default 0.01) and the median
slows by at least `min_effect_pct` (default 2%). Samples are "lower is
better", so negate throughput metrics before passing them in.

## Bisection

`bisect(commits, good_samples, bad_samples, runner, cfg)` binary-searches the
range from the last good commit to the first bad one. Each midpoint is tested
against both the known-good and the known-bad distributions:

- slower than good and not faster than bad: the midpoint is bad
- faster than bad and not slower than good: the midpoint is good
- the two tests disagree: the midpoint is re-sampled, up to `max_attempts`,
  and then classified by the nearest median

When every midpoint classifies cleanly, bisection takes exactly
`ceil(log2(N))` benchmark runs. `BenchmarkRunner` is the only integration
point: it checks out a commit, runs the suite and returns its samples.

## Evidence

`Evidence` names the offending commit, its author, the last good commit, the
effect size, the p-value and confidence, and how many runs were used.
`Evidence::to_markdown()` renders the notification for the PR comment or
issue that is assigned to the author.

## Validation

The unit tests seed a 10% regression in the middle of a 64-commit history,
with 3% Gaussian noise and 3% outliers. Bisection must find the exact commit
within `ceil(log2(N))` runs at more than 99% confidence. Across 200 seeded
trials, the false attribution rate must stay below 5%.
