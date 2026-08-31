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
//! Feature flag evaluation for gradual rollouts (issue #1337).
//!
//! [`crate::controller::feature_flags`] answers "is this operator-wide
//! capability on?" with a plain boolean. That is the right shape for a
//! cluster-level switch like `enable_dr`, but it cannot express "turn this on
//! for 5% of tenants" or "only in staging", which is what a gradual rollout
//! needs. This module adds that evaluation layer.
//!
//! Both read the same `stellar-operator-config` ConfigMap, so a flag is
//! toggled by editing the ConfigMap — no rebuild, no redeploy, no restart.
//!
//! # Targeting
//!
//! A [`FlagRule`] is evaluated in a fixed precedence order, and
//! [`Decision::reason`] always reports which step decided:
//!
//! 1. **Flag missing** → off. An unknown flag is never on by accident.
//! 2. **`enabled: false`** → off. The master switch is the kill switch.
//! 3. **Deny list** → off. Wins over everything below, including allow.
//! 4. **Allow list** → on, skipping segment and percentage checks.
//! 5. **Segments** → all must match, or off.
//! 6. **Percentage** → deterministic bucketing (below).
//!
//! # Deterministic bucketing
//!
//! A subject's bucket is `fnv1a64("{flag}:{subject}") % 10_000`, and the flag
//! is on when `bucket < percentage * 100`. Three properties matter:
//!
//! * **Stable** — the same subject always lands in the same bucket, so a user
//!   does not see the feature flicker between requests or replicas.
//! * **Monotonic** — raising the percentage only ever *adds* subjects. Nobody
//!   loses a feature because the rollout widened, which is what makes a
//!   staged 1% → 10% → 50% ramp safe.
//! * **Independent per flag** — the flag name is hashed with the subject, so
//!   a subject is not permanently in the first 1% of every flag.
//!
//! FNV-1a is used rather than [`std::collections::hash_map::DefaultHasher`],
//! whose output is explicitly not stable across releases or processes —
//! bucketing on it would reshuffle every rollout on upgrade.
//!
//! # ConfigMap format
//!
//! Each rollout flag is a `flag.<name>` key holding a JSON object:
//!
//! ```yaml
//! apiVersion: v1
//! kind: ConfigMap
//! metadata:
//!   name: stellar-operator-config
//!   namespace: stellar-system
//! data:
//!   flag.new_archive_pruner: |
//!     {
//!       "enabled": true,
//!       "rollout_percentage": 25,
//!       "segments": [{"key": "env", "op": "in", "values": ["staging", "canary"]}],
//!       "deny_subjects": ["tenant-critical"]
//!     }
//! ```
//!
//! # Example
//!
//! ```
//! use stellar_k8s::feature_flags::{EvaluationContext, FlagRule, FlagSet};
//!
//! let mut flags = FlagSet::new();
//! flags.insert("new_pruner", FlagRule::percentage(25.0));
//!
//! let ctx = EvaluationContext::new("tenant-42");
//! let decision = flags.evaluate("new_pruner", &ctx);
//! println!("{} because {}", decision.enabled, decision.reason);
//!
//! // Unknown flags are off, never on by accident.
//! assert!(!flags.is_enabled("typo_in_flag_name", &ctx));
//! ```

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

/// Prefix marking a ConfigMap key as a rollout flag definition.
pub const FLAG_KEY_PREFIX: &str = "flag.";

/// Buckets a subject can fall into. 10,000 gives 0.01% resolution, which is
/// finer than any rollout step an operator realistically types.
const BUCKET_COUNT: u64 = 10_000;

// ─────────────────────────────────────────────────────────────────────────────
// Bucketing
// ─────────────────────────────────────────────────────────────────────────────

/// FNV-1a, 64-bit.
///
/// Chosen for being fixed by specification: the same input yields the same
/// output on every platform and every release, which is the whole requirement
/// for stable rollout bucketing.
fn fnv1a64(bytes: &[u8]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET_BASIS;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// The stable bucket (`0..10_000`) for `subject` under `flag`.
pub fn bucket_for(flag: &str, subject: &str) -> u64 {
    fnv1a64(format!("{flag}:{subject}").as_bytes()) % BUCKET_COUNT
}

// ─────────────────────────────────────────────────────────────────────────────
// Evaluation context
// ─────────────────────────────────────────────────────────────────────────────

/// Who or what a flag is being evaluated for.
///
/// The `subject` must be *stable* for the entity being rolled out to — a
/// tenant id, node name, or account id. A per-request value such as a request
/// id would re-bucket on every call and make the rollout look random.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EvaluationContext {
    subject: String,
    attributes: BTreeMap<String, String>,
}

impl EvaluationContext {
    /// Build a context for `subject`.
    pub fn new(subject: impl Into<String>) -> Self {
        Self {
            subject: subject.into(),
            attributes: BTreeMap::new(),
        }
    }

    /// Attach a targeting attribute (builder style).
    pub fn with_attribute(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(key.into(), value.into());
        self
    }

    /// The subject this context represents.
    pub fn subject(&self) -> &str {
        &self.subject
    }

    /// Look up a targeting attribute.
    pub fn attribute(&self, key: &str) -> Option<&str> {
        self.attributes.get(key).map(String::as_str)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Segment matching
// ─────────────────────────────────────────────────────────────────────────────

/// Comparison used by a [`SegmentRule`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MatchOp {
    /// Attribute equals one of `values`.
    In,
    /// Attribute equals none of `values`.
    NotIn,
    /// Attribute contains one of `values` as a substring.
    Contains,
    /// Attribute starts with one of `values`.
    Prefix,
}

/// One targeting predicate over a context attribute.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentRule {
    /// Attribute name to read from the [`EvaluationContext`].
    pub key: String,
    /// How to compare it.
    pub op: MatchOp,
    /// Values to compare against.
    pub values: Vec<String>,
}

impl SegmentRule {
    /// Attribute `key` must be one of `values`.
    pub fn is_in(key: impl Into<String>, values: &[&str]) -> Self {
        Self {
            key: key.into(),
            op: MatchOp::In,
            values: values.iter().map(|v| (*v).to_string()).collect(),
        }
    }

    /// Does `ctx` satisfy this rule?
    ///
    /// A missing attribute never matches a positive operator. It *does*
    /// satisfy `NotIn`, since a subject with no `env` is genuinely not in
    /// `env: [prod]`.
    pub fn matches(&self, ctx: &EvaluationContext) -> bool {
        let Some(actual) = ctx.attribute(&self.key) else {
            return self.op == MatchOp::NotIn;
        };
        match self.op {
            MatchOp::In => self.values.iter().any(|v| v == actual),
            MatchOp::NotIn => !self.values.iter().any(|v| v == actual),
            MatchOp::Contains => self.values.iter().any(|v| actual.contains(v.as_str())),
            MatchOp::Prefix => self.values.iter().any(|v| actual.starts_with(v.as_str())),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Flag rule
// ─────────────────────────────────────────────────────────────────────────────

/// The rollout configuration for one flag.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", default)]
pub struct FlagRule {
    /// Master switch. `false` turns the flag off for everyone regardless of
    /// allow lists, segments, or percentage.
    pub enabled: bool,
    /// Share of eligible subjects that get the flag, `0.0..=100.0`.
    pub rollout_percentage: f64,
    /// Subjects that always get the flag, bypassing segments and percentage.
    pub allow_subjects: Vec<String>,
    /// Subjects that never get the flag. Takes precedence over everything.
    pub deny_subjects: Vec<String>,
    /// Predicates a subject must *all* satisfy to be eligible.
    pub segments: Vec<SegmentRule>,
}

impl Default for FlagRule {
    fn default() -> Self {
        Self {
            // A flag that exists but says nothing is off: rollouts start
            // closed and are opened deliberately.
            enabled: false,
            rollout_percentage: 0.0,
            allow_subjects: Vec::new(),
            deny_subjects: Vec::new(),
            segments: Vec::new(),
        }
    }
}

impl FlagRule {
    /// On for everyone.
    pub fn on() -> Self {
        Self {
            enabled: true,
            rollout_percentage: 100.0,
            ..Default::default()
        }
    }

    /// Off for everyone.
    pub fn off() -> Self {
        Self::default()
    }

    /// On for `percentage` of subjects.
    pub fn percentage(percentage: f64) -> Self {
        Self {
            enabled: true,
            rollout_percentage: percentage,
            ..Default::default()
        }
    }

    /// Restrict this rule to subjects matching `segment` (builder style).
    pub fn with_segment(mut self, segment: SegmentRule) -> Self {
        self.segments.push(segment);
        self
    }

    /// Always enable `subject` (builder style).
    pub fn allowing(mut self, subject: impl Into<String>) -> Self {
        self.allow_subjects.push(subject.into());
        self
    }

    /// Never enable `subject` (builder style).
    pub fn denying(mut self, subject: impl Into<String>) -> Self {
        self.deny_subjects.push(subject.into());
        self
    }

    /// Percentage clamped into range, so a typo like `500` or `-1` cannot
    /// produce a nonsensical threshold.
    fn effective_percentage(&self) -> f64 {
        self.rollout_percentage.clamp(0.0, 100.0)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Decision
// ─────────────────────────────────────────────────────────────────────────────

/// Why a flag evaluated the way it did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// No rule is defined for this flag.
    FlagMissing,
    /// The rule's master switch is off.
    FlagDisabled,
    /// The subject is on the deny list.
    DenyListed,
    /// The subject is on the allow list.
    AllowListed,
    /// The subject failed at least one segment predicate.
    SegmentMismatch,
    /// The subject's bucket falls inside the rollout percentage.
    RolloutIncluded,
    /// The subject's bucket falls outside the rollout percentage.
    RolloutExcluded,
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Reason::FlagMissing => "flag is not defined",
            Reason::FlagDisabled => "flag is disabled",
            Reason::DenyListed => "subject is deny-listed",
            Reason::AllowListed => "subject is allow-listed",
            Reason::SegmentMismatch => "subject does not match the targeted segment",
            Reason::RolloutIncluded => "subject is inside the rollout percentage",
            Reason::RolloutExcluded => "subject is outside the rollout percentage",
        })
    }
}

/// The outcome of evaluating one flag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    /// Whether the feature is on for this subject.
    pub enabled: bool,
    /// Which evaluation step decided.
    pub reason: Reason,
    /// The subject's bucket, when bucketing was reached. Useful for
    /// explaining a rollout to an operator ("you're at 7134, the cut is
    /// 2500").
    pub bucket: Option<u64>,
}

impl Decision {
    fn new(enabled: bool, reason: Reason) -> Self {
        Self {
            enabled,
            reason,
            bucket: None,
        }
    }

    fn bucketed(enabled: bool, reason: Reason, bucket: u64) -> Self {
        Self {
            enabled,
            reason,
            bucket: Some(bucket),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Flag set
// ─────────────────────────────────────────────────────────────────────────────

/// A problem found while parsing flag definitions from a ConfigMap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlagParseWarning {
    /// The value for `flag.<name>` is not valid JSON for a [`FlagRule`].
    InvalidDefinition { key: String, error: String },
    /// `rollout_percentage` was outside `0..=100` and has been clamped.
    PercentageClamped { key: String, value: String },
}

impl fmt::Display for FlagParseWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDefinition { key, error } => {
                write!(f, "invalid flag definition '{key}': {error} — flag ignored")
            }
            Self::PercentageClamped { key, value } => {
                write!(
                    f,
                    "rollout_percentage '{value}' for '{key}' is outside 0..=100 — clamped"
                )
            }
        }
    }
}

/// A named set of flag rules.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FlagSet {
    flags: BTreeMap<String, FlagRule>,
}

impl FlagSet {
    /// An empty set. Every flag evaluates to off.
    pub fn new() -> Self {
        Self::default()
    }

    /// Define or replace a flag.
    pub fn insert(&mut self, name: impl Into<String>, rule: FlagRule) {
        self.flags.insert(name.into(), rule);
    }

    /// Look up a rule.
    pub fn get(&self, name: &str) -> Option<&FlagRule> {
        self.flags.get(name)
    }

    /// Names of every defined flag, sorted.
    pub fn flag_names(&self) -> Vec<&str> {
        self.flags.keys().map(String::as_str).collect()
    }

    /// Number of defined flags.
    pub fn len(&self) -> usize {
        self.flags.len()
    }

    /// Whether no flags are defined.
    pub fn is_empty(&self) -> bool {
        self.flags.is_empty()
    }

    /// Evaluate `flag` for `ctx`, reporting which step decided.
    pub fn evaluate(&self, flag: &str, ctx: &EvaluationContext) -> Decision {
        let Some(rule) = self.flags.get(flag) else {
            return Decision::new(false, Reason::FlagMissing);
        };

        if !rule.enabled {
            return Decision::new(false, Reason::FlagDisabled);
        }
        // Deny before allow: an explicit exclusion is a safety valve and must
        // not be defeated by the subject also appearing on the allow list.
        if rule.deny_subjects.iter().any(|s| s == ctx.subject()) {
            return Decision::new(false, Reason::DenyListed);
        }
        if rule.allow_subjects.iter().any(|s| s == ctx.subject()) {
            return Decision::new(true, Reason::AllowListed);
        }
        if !rule.segments.iter().all(|s| s.matches(ctx)) {
            return Decision::new(false, Reason::SegmentMismatch);
        }

        let bucket = bucket_for(flag, ctx.subject());
        let threshold = (rule.effective_percentage() * (BUCKET_COUNT as f64 / 100.0)) as u64;
        if bucket < threshold {
            Decision::bucketed(true, Reason::RolloutIncluded, bucket)
        } else {
            Decision::bucketed(false, Reason::RolloutExcluded, bucket)
        }
    }

    /// Whether `flag` is on for `ctx`.
    pub fn is_enabled(&self, flag: &str, ctx: &EvaluationContext) -> bool {
        self.evaluate(flag, ctx).enabled
    }

    /// Parse rollout flags from a ConfigMap `data` map.
    ///
    /// Only `flag.<name>` keys are read, so this coexists with the boolean
    /// keys [`crate::controller::feature_flags`] owns in the same ConfigMap.
    /// A malformed definition is skipped with a warning rather than failing
    /// the whole reload — one bad edit must not drop every other flag.
    pub fn from_config_map_data(data: &BTreeMap<String, String>) -> (Self, Vec<FlagParseWarning>) {
        let mut set = Self::new();
        let mut warnings = Vec::new();

        for (key, value) in data {
            let Some(name) = key.strip_prefix(FLAG_KEY_PREFIX) else {
                continue;
            };
            match serde_json::from_str::<FlagRule>(value) {
                Ok(rule) => {
                    if rule.rollout_percentage != rule.effective_percentage() {
                        warnings.push(FlagParseWarning::PercentageClamped {
                            key: key.clone(),
                            value: rule.rollout_percentage.to_string(),
                        });
                    }
                    set.insert(name, rule);
                }
                Err(err) => warnings.push(FlagParseWarning::InvalidDefinition {
                    key: key.clone(),
                    error: err.to_string(),
                }),
            }
        }

        (set, warnings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(subject: &str) -> EvaluationContext {
        EvaluationContext::new(subject)
    }

    // ── Bucketing ────────────────────────────────────────────────────────

    #[test]
    fn bucketing_is_stable_across_calls() {
        let first = bucket_for("f", "subject-1");
        for _ in 0..100 {
            assert_eq!(bucket_for("f", "subject-1"), first);
        }
    }

    #[test]
    fn buckets_stay_in_range() {
        for i in 0..2_000 {
            assert!(bucket_for("f", &format!("s{i}")) < BUCKET_COUNT);
        }
    }

    #[test]
    fn the_same_subject_buckets_differently_per_flag() {
        // Otherwise a subject would be in the first 1% of every rollout.
        let a = bucket_for("flag_a", "subject-1");
        let b = bucket_for("flag_b", "subject-1");
        assert_ne!(a, b);
    }

    #[test]
    fn buckets_are_reasonably_uniform() {
        let mut deciles = [0usize; 10];
        let n = 10_000;
        for i in 0..n {
            let bucket = bucket_for("uniformity", &format!("subject-{i}"));
            deciles[(bucket * 10 / BUCKET_COUNT) as usize] += 1;
        }
        // Perfect uniformity is 1000 per decile; allow a generous ±25% band.
        for (index, count) in deciles.iter().enumerate() {
            assert!(
                (750..=1250).contains(count),
                "decile {index} had {count} of {n} subjects — distribution is skewed"
            );
        }
    }

    #[test]
    fn fnv1a_matches_the_published_vectors() {
        // Pins the hash so a refactor cannot silently reshuffle every rollout.
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a64(b"foobar"), 0x8594_4171_f739_67e8);
    }

    // ── Percentage rollout ───────────────────────────────────────────────

    #[test]
    fn zero_percent_is_off_for_everyone() {
        let mut flags = FlagSet::new();
        flags.insert("f", FlagRule::percentage(0.0));
        for i in 0..500 {
            assert!(!flags.is_enabled("f", &ctx(&format!("s{i}"))));
        }
    }

    #[test]
    fn one_hundred_percent_is_on_for_everyone() {
        let mut flags = FlagSet::new();
        flags.insert("f", FlagRule::percentage(100.0));
        for i in 0..500 {
            assert!(flags.is_enabled("f", &ctx(&format!("s{i}"))));
        }
    }

    #[test]
    fn a_partial_rollout_hits_approximately_the_configured_share() {
        let mut flags = FlagSet::new();
        flags.insert("f", FlagRule::percentage(25.0));
        let n = 10_000;
        let enabled = (0..n)
            .filter(|i| flags.is_enabled("f", &ctx(&format!("s{i}"))))
            .count();
        let share = enabled as f64 / n as f64 * 100.0;
        assert!(
            (23.0..=27.0).contains(&share),
            "25% rollout reached {share:.1}% of subjects"
        );
    }

    #[test]
    fn raising_the_percentage_only_ever_adds_subjects() {
        // The property that makes a staged ramp safe: nobody loses the feature
        // when the rollout widens.
        let subjects: Vec<String> = (0..2_000).map(|i| format!("s{i}")).collect();
        let mut previously_on: Vec<&String> = Vec::new();

        for percentage in [1.0, 5.0, 10.0, 25.0, 50.0, 100.0] {
            let mut flags = FlagSet::new();
            flags.insert("ramp", FlagRule::percentage(percentage));
            let now_on: Vec<&String> = subjects
                .iter()
                .filter(|s| flags.is_enabled("ramp", &ctx(s)))
                .collect();

            for subject in &previously_on {
                assert!(
                    now_on.contains(subject),
                    "{subject} lost the flag when the rollout widened to {percentage}%"
                );
            }
            previously_on = now_on;
        }
    }

    #[test]
    fn a_subject_gets_the_same_answer_on_every_replica() {
        // Two independently constructed sets stand in for two processes.
        let mut a = FlagSet::new();
        a.insert("f", FlagRule::percentage(37.0));
        let mut b = FlagSet::new();
        b.insert("f", FlagRule::percentage(37.0));
        for i in 0..500 {
            let subject = ctx(&format!("s{i}"));
            assert_eq!(a.is_enabled("f", &subject), b.is_enabled("f", &subject));
        }
    }

    #[test]
    fn out_of_range_percentages_are_clamped() {
        let mut flags = FlagSet::new();
        flags.insert("high", FlagRule::percentage(500.0));
        flags.insert("low", FlagRule::percentage(-10.0));
        assert!(flags.is_enabled("high", &ctx("s")));
        assert!(!flags.is_enabled("low", &ctx("s")));
    }

    #[test]
    fn the_decision_reports_the_bucket_when_bucketing_ran() {
        let mut flags = FlagSet::new();
        flags.insert("f", FlagRule::percentage(50.0));
        assert!(flags.evaluate("f", &ctx("s")).bucket.is_some());
    }

    // ── Precedence ───────────────────────────────────────────────────────

    #[test]
    fn an_undefined_flag_is_off() {
        let decision = FlagSet::new().evaluate("nope", &ctx("s"));
        assert!(!decision.enabled);
        assert_eq!(decision.reason, Reason::FlagMissing);
    }

    #[test]
    fn the_master_switch_overrides_a_full_rollout() {
        let mut flags = FlagSet::new();
        flags.insert(
            "f",
            FlagRule {
                enabled: false,
                rollout_percentage: 100.0,
                ..Default::default()
            },
        );
        let decision = flags.evaluate("f", &ctx("s"));
        assert!(!decision.enabled);
        assert_eq!(decision.reason, Reason::FlagDisabled);
    }

    #[test]
    fn the_master_switch_overrides_the_allow_list() {
        let mut flags = FlagSet::new();
        flags.insert(
            "f",
            FlagRule {
                enabled: false,
                ..FlagRule::on().allowing("vip")
            },
        );
        assert!(!flags.is_enabled("f", &ctx("vip")));
    }

    #[test]
    fn the_allow_list_bypasses_the_percentage() {
        let mut flags = FlagSet::new();
        flags.insert("f", FlagRule::percentage(0.0).allowing("vip"));
        let decision = flags.evaluate("f", &ctx("vip"));
        assert!(decision.enabled);
        assert_eq!(decision.reason, Reason::AllowListed);
    }

    #[test]
    fn the_allow_list_bypasses_segments() {
        let mut flags = FlagSet::new();
        flags.insert(
            "f",
            FlagRule::on()
                .with_segment(SegmentRule::is_in("env", &["staging"]))
                .allowing("vip"),
        );
        assert!(flags.is_enabled("f", &ctx("vip").with_attribute("env", "prod")));
    }

    #[test]
    fn the_deny_list_wins_over_the_allow_list() {
        let mut flags = FlagSet::new();
        flags.insert("f", FlagRule::on().allowing("x").denying("x"));
        let decision = flags.evaluate("f", &ctx("x"));
        assert!(!decision.enabled);
        assert_eq!(decision.reason, Reason::DenyListed);
    }

    #[test]
    fn the_deny_list_wins_over_a_full_rollout() {
        let mut flags = FlagSet::new();
        flags.insert("f", FlagRule::on().denying("blocked"));
        assert!(!flags.is_enabled("f", &ctx("blocked")));
        assert!(flags.is_enabled("f", &ctx("other")));
    }

    // ── Segment targeting ────────────────────────────────────────────────

    #[test]
    fn a_matching_segment_admits_the_subject() {
        let mut flags = FlagSet::new();
        flags.insert(
            "f",
            FlagRule::on().with_segment(SegmentRule::is_in("env", &["staging"])),
        );
        assert!(flags.is_enabled("f", &ctx("s").with_attribute("env", "staging")));
    }

    #[test]
    fn a_non_matching_segment_excludes_the_subject() {
        let mut flags = FlagSet::new();
        flags.insert(
            "f",
            FlagRule::on().with_segment(SegmentRule::is_in("env", &["staging"])),
        );
        let decision = flags.evaluate("f", &ctx("s").with_attribute("env", "prod"));
        assert!(!decision.enabled);
        assert_eq!(decision.reason, Reason::SegmentMismatch);
    }

    #[test]
    fn a_missing_attribute_fails_a_positive_segment() {
        let mut flags = FlagSet::new();
        flags.insert(
            "f",
            FlagRule::on().with_segment(SegmentRule::is_in("env", &["staging"])),
        );
        assert!(!flags.is_enabled("f", &ctx("s")));
    }

    #[test]
    fn segments_are_combined_with_and() {
        let mut flags = FlagSet::new();
        flags.insert(
            "f",
            FlagRule::on()
                .with_segment(SegmentRule::is_in("env", &["staging"]))
                .with_segment(SegmentRule::is_in("region", &["eu"])),
        );
        let base = ctx("s").with_attribute("env", "staging");
        assert!(!flags.is_enabled("f", &base.clone().with_attribute("region", "us")));
        assert!(flags.is_enabled("f", &base.with_attribute("region", "eu")));
    }

    #[test]
    fn not_in_excludes_listed_values() {
        let rule = SegmentRule {
            key: "env".into(),
            op: MatchOp::NotIn,
            values: vec!["prod".into()],
        };
        assert!(!rule.matches(&ctx("s").with_attribute("env", "prod")));
        assert!(rule.matches(&ctx("s").with_attribute("env", "staging")));
    }

    #[test]
    fn not_in_matches_when_the_attribute_is_absent() {
        // A subject with no `env` really is not in `env: [prod]`.
        let rule = SegmentRule {
            key: "env".into(),
            op: MatchOp::NotIn,
            values: vec!["prod".into()],
        };
        assert!(rule.matches(&ctx("s")));
    }

    #[test]
    fn contains_and_prefix_match_substrings() {
        let contains = SegmentRule {
            key: "node".into(),
            op: MatchOp::Contains,
            values: vec!["canary".into()],
        };
        let prefix = SegmentRule {
            key: "node".into(),
            op: MatchOp::Prefix,
            values: vec!["val-".into()],
        };
        let subject = ctx("s").with_attribute("node", "val-canary-3");
        assert!(contains.matches(&subject));
        assert!(prefix.matches(&subject));
        assert!(!prefix.matches(&ctx("s").with_attribute("node", "hz-1")));
    }

    #[test]
    fn segments_and_percentage_compose() {
        let mut flags = FlagSet::new();
        flags.insert(
            "f",
            FlagRule::percentage(50.0).with_segment(SegmentRule::is_in("env", &["staging"])),
        );
        // Out-of-segment subjects are excluded regardless of their bucket.
        for i in 0..200 {
            assert!(!flags.is_enabled("f", &ctx(&format!("s{i}")).with_attribute("env", "prod")));
        }
        // In-segment subjects are split by the percentage.
        let enabled = (0..1_000)
            .filter(|i| {
                flags.is_enabled("f", &ctx(&format!("s{i}")).with_attribute("env", "staging"))
            })
            .count();
        assert!((400..=600).contains(&enabled), "got {enabled} of 1000");
    }

    // ── ConfigMap parsing ────────────────────────────────────────────────

    fn data(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn flags_parse_from_configmap_data() {
        let (flags, warnings) = FlagSet::from_config_map_data(&data(&[(
            "flag.new_pruner",
            r#"{"enabled": true, "rollout_percentage": 25}"#,
        )]));
        assert!(warnings.is_empty());
        assert_eq!(flags.len(), 1);
        assert_eq!(flags.get("new_pruner").unwrap().rollout_percentage, 25.0);
    }

    #[test]
    fn non_flag_keys_are_left_to_the_boolean_flag_module() {
        let (flags, warnings) =
            FlagSet::from_config_map_data(&data(&[("enable_dr", "true"), ("unrelated", "x")]));
        assert!(flags.is_empty());
        assert!(warnings.is_empty());
    }

    #[test]
    fn a_malformed_definition_warns_without_dropping_the_others() {
        let (flags, warnings) = FlagSet::from_config_map_data(&data(&[
            ("flag.good", r#"{"enabled": true}"#),
            ("flag.bad", "{not json"),
        ]));
        assert_eq!(flags.len(), 1, "the valid flag must survive");
        assert!(flags.get("good").is_some());
        assert!(matches!(
            warnings.as_slice(),
            [FlagParseWarning::InvalidDefinition { key, .. }] if key == "flag.bad"
        ));
    }

    #[test]
    fn an_out_of_range_percentage_warns_and_is_clamped() {
        let (flags, warnings) = FlagSet::from_config_map_data(&data(&[(
            "flag.f",
            r#"{"enabled": true, "rollout_percentage": 150}"#,
        )]));
        assert!(matches!(
            warnings.as_slice(),
            [FlagParseWarning::PercentageClamped { .. }]
        ));
        assert!(flags.is_enabled("f", &ctx("s")));
    }

    #[test]
    fn omitted_fields_fall_back_to_off() {
        let (flags, _) = FlagSet::from_config_map_data(&data(&[("flag.f", "{}")]));
        let rule = flags.get("f").unwrap();
        assert!(!rule.enabled);
        assert_eq!(rule.rollout_percentage, 0.0);
        assert!(!flags.is_enabled("f", &ctx("s")));
    }

    #[test]
    fn segments_parse_from_json() {
        let (flags, warnings) = FlagSet::from_config_map_data(&data(&[(
            "flag.f",
            r#"{"enabled": true, "rollout_percentage": 100,
                "segments": [{"key": "env", "op": "in", "values": ["staging"]}]}"#,
        )]));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(flags.is_enabled("f", &ctx("s").with_attribute("env", "staging")));
        assert!(!flags.is_enabled("f", &ctx("s").with_attribute("env", "prod")));
    }

    #[test]
    fn allow_and_deny_lists_parse_from_json() {
        let (flags, _) = FlagSet::from_config_map_data(&data(&[(
            "flag.f",
            r#"{"enabled": true, "allow_subjects": ["vip"], "deny_subjects": ["blocked"]}"#,
        )]));
        assert!(flags.is_enabled("f", &ctx("vip")));
        assert!(!flags.is_enabled("f", &ctx("blocked")));
    }

    #[test]
    fn a_rule_round_trips_through_json() {
        let rule = FlagRule::percentage(42.0)
            .with_segment(SegmentRule::is_in("env", &["staging"]))
            .allowing("vip");
        let json = serde_json::to_string(&rule).unwrap();
        assert_eq!(serde_json::from_str::<FlagRule>(&json).unwrap(), rule);
    }

    #[test]
    fn flag_names_are_reported_sorted() {
        let (flags, _) =
            FlagSet::from_config_map_data(&data(&[("flag.zebra", "{}"), ("flag.alpha", "{}")]));
        assert_eq!(flags.flag_names(), vec!["alpha", "zebra"]);
    }

    // ── Toggling without redeploying ─────────────────────────────────────

    #[test]
    fn editing_the_configmap_changes_the_answer() {
        // Stands in for the operator editing the ConfigMap and the watcher
        // swapping the FlagSet in: no rebuild, no restart.
        let subject = ctx("tenant-7");
        let (off, _) = FlagSet::from_config_map_data(&data(&[("flag.f", r#"{"enabled": false}"#)]));
        assert!(!off.is_enabled("f", &subject));

        let (on, _) = FlagSet::from_config_map_data(&data(&[(
            "flag.f",
            r#"{"enabled": true, "rollout_percentage": 100}"#,
        )]));
        assert!(on.is_enabled("f", &subject));
    }

    #[test]
    fn reasons_render_as_readable_text() {
        for reason in [
            Reason::FlagMissing,
            Reason::FlagDisabled,
            Reason::DenyListed,
            Reason::AllowListed,
            Reason::SegmentMismatch,
            Reason::RolloutIncluded,
            Reason::RolloutExcluded,
        ] {
            assert!(!reason.to_string().is_empty());
        }
    }
}
