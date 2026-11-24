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
//! Composite service health signal (epic #1524).
//!
//! Combines several existing SLI ratio series of a service into one weighted
//! objective, published through Prometheus recording rules so existing
//! alerting and burn-rate tooling can consume it unchanged:
//!
//! | Series | Meaning |
//! |--------|---------|
//! | `stellar:slo:composite:ratio` | `Σ weightᵢ · sliᵢ` per service |
//! | `stellar:slo:composite:ratio_rate<w>` | composite averaged over `1h`/`6h`/`1d`/`3d`/`30d` |
//! | `stellar:slo:composite:burn_rate<w>` | `(1 − ratio_rate<w>) / (1 − target)` |
//! | `stellar:slo:composite:error_budget_remaining` | `1 − burn_rate30d` |
//! | `stellar:slo:composite:weights_version` | reviewed weighting version in effect |
//!
//! Queries hit pre-recorded series, so they stay cheap. Definitions live in
//! `config/slo/composite-slos.yaml`; [`validate`] requires weights to sum to
//! 1 and every weight change to carry a matching versioned review record.
//! The committed rules file is checked against [`render_rules`] in tests.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Averaging windows published for burn-rate alerting.
pub const WINDOWS: [&str; 5] = ["1h", "6h", "1d", "3d", "30d"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompositeConfig {
    /// Services that must have a composite objective.
    pub tier1_services: Vec<String>,
    pub composites: Vec<CompositeSlo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompositeSlo {
    pub service: String,
    /// Objective for the composite ratio (e.g. `0.998`).
    pub target: f64,
    /// Weighting version; bumped on every weight change.
    pub version: u32,
    pub slis: Vec<WeightedSli>,
    pub reviews: Vec<WeightReview>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WeightedSli {
    pub name: String,
    /// Existing recording rule producing the SLI ratio (0..1).
    pub record: String,
    pub weight: f64,
}

/// Review record for one weighting version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WeightReview {
    pub version: u32,
    pub date: String,
    pub reviewed_by: String,
    pub reason: String,
    pub weights: BTreeMap<String, f64>,
}

const EPS: f64 = 1e-9;

/// Validate the configuration. `known_records` are the recording rules that
/// exist in the repository's rule files.
pub fn validate(cfg: &CompositeConfig, known_records: &[String]) -> Result<(), String> {
    for svc in &cfg.tier1_services {
        if !cfg.composites.iter().any(|c| &c.service == svc) {
            return Err(format!("tier-1 service '{svc}' has no composite SLO"));
        }
    }
    for c in &cfg.composites {
        let ctx = |m: String| format!("composite '{}': {m}", c.service);
        if !(0.0 < c.target && c.target < 1.0) {
            return Err(ctx(format!("target {} must be in (0, 1)", c.target)));
        }
        if c.slis.is_empty() {
            return Err(ctx("no SLIs".into()));
        }
        let sum: f64 = c.slis.iter().map(|s| s.weight).sum();
        if (sum - 1.0).abs() > EPS {
            return Err(ctx(format!("weights sum to {sum}, expected 1")));
        }
        for s in &c.slis {
            if s.weight <= 0.0 {
                return Err(ctx(format!("SLI '{}' weight must be positive", s.name)));
            }
            if !known_records.contains(&s.record) {
                return Err(ctx(format!(
                    "SLI '{}' references unknown recording rule '{}'",
                    s.name, s.record
                )));
            }
        }
        for (i, r) in c.reviews.iter().enumerate() {
            if r.version as usize != i + 1 {
                return Err(ctx(format!(
                    "review #{} has version {}, expected {}",
                    i + 1,
                    r.version,
                    i + 1
                )));
            }
            if r.reviewed_by.trim().is_empty() || r.reason.trim().is_empty() {
                return Err(ctx(format!(
                    "review v{} needs reviewedBy and reason",
                    r.version
                )));
            }
        }
        let latest = c
            .reviews
            .last()
            .ok_or_else(|| ctx("weights have no review record".into()))?;
        if latest.version != c.version {
            return Err(ctx(format!(
                "version {} has no review record (latest review is v{})",
                c.version, latest.version
            )));
        }
        let current: BTreeMap<&str, f64> =
            c.slis.iter().map(|s| (s.name.as_str(), s.weight)).collect();
        let reviewed: BTreeMap<&str, f64> = latest
            .weights
            .iter()
            .map(|(k, v)| (k.as_str(), *v))
            .collect();
        let same = current.len() == reviewed.len()
            && current
                .iter()
                .all(|(k, v)| reviewed.get(k).is_some_and(|r| (r - v).abs() < EPS));
        if !same {
            return Err(ctx(format!(
                "weights differ from review v{}; bump version and add a review",
                latest.version
            )));
        }
    }
    Ok(())
}

/// Composite ratio for one evaluation from per-SLI values keyed by name.
pub fn composite_ratio(slo: &CompositeSlo, sli_values: &BTreeMap<String, f64>) -> Option<f64> {
    slo.slis
        .iter()
        .map(|s| sli_values.get(&s.name).map(|v| s.weight * v))
        .sum()
}

/// Fraction of the error budget consumed over a window of composite ratio
/// samples (evenly spaced): `(1 − mean) / (1 − target)`.
pub fn budget_consumed(samples: &[f64], target: f64) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let mean = samples.iter().sum::<f64>() / samples.len() as f64;
    (1.0 - mean) / (1.0 - target)
}

fn fmt_num(v: f64) -> String {
    let s = format!("{v:.6}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Render the Prometheus rule file for all composites.
pub fn render_rules(cfg: &CompositeConfig) -> String {
    let mut out = String::from(
        "# Copyright 2024 Stellar-K8s Contributors\n\
         # Licensed under the Apache License, Version 2.0 (the \"License\");\n\
         # you may not use this file except in compliance with the License.\n\
         # You may obtain a copy of the License at\n\
         #\n\
         #     http://www.apache.org/licenses/LICENSE-2.0\n\
         #\n\
         # Unless required by applicable law or agreed to in writing, software\n\
         # distributed under the License is distributed on an \"AS IS\" BASIS,\n\
         # WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.\n\
         # See the License for the specific language governing permissions and\n\
         # limitations under the License.\n\
         #\n\
         # GENERATED from config/slo/composite-slos.yaml by src/composite_slo.rs.\n\
         # Do not edit by hand. Regenerate with:\n\
         #   STELLAR_REGENERATE_SLO_RULES=1 cargo test --lib composite_slo\n\
         groups:\n",
    );
    for c in &cfg.composites {
        let svc = &c.service;
        // The weighting version is published as its own series rather than a
        // label, so a weight change does not split the 30d averaging windows.
        let labels =
            format!("        labels:\n          service: {svc}\n          slo: {svc}-composite\n");
        out.push_str(&format!(
            "  - name: stellar_composite_slo_{}\n    interval: 1m\n    rules:\n",
            svc.replace('-', "_")
        ));
        let terms: Vec<String> = c
            .slis
            .iter()
            .map(|s| {
                format!(
                    "sum by (service) ({} * {}{{service=\"{svc}\"}})",
                    fmt_num(s.weight),
                    s.record
                )
            })
            .collect();
        out.push_str(&format!(
            "      - record: stellar:slo:composite:ratio\n        expr: |\n          {}\n{labels}",
            terms.join("\n          + ")
        ));
        let sel = format!("{{service=\"{svc}\"}}");
        for w in WINDOWS {
            out.push_str(&format!(
                "      - record: stellar:slo:composite:ratio_rate{w}\n        expr: avg_over_time(stellar:slo:composite:ratio{sel}[{w}])\n{labels}"
            ));
            out.push_str(&format!(
                "      - record: stellar:slo:composite:burn_rate{w}\n        expr: (1 - stellar:slo:composite:ratio_rate{w}{sel}) / {}\n{labels}",
                fmt_num(1.0 - c.target)
            ));
        }
        out.push_str(&format!(
            "      - record: stellar:slo:composite:error_budget_remaining\n        expr: 1 - stellar:slo:composite:burn_rate30d{sel}\n{labels}"
        ));
        out.push_str(&format!(
            "      - record: stellar:slo:composite:weights_version\n        expr: vector({})\n{labels}",
            c.version
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::SmallRng;
    use rand::{Rng, SeedableRng};

    const CONFIG: &str = include_str!("../config/slo/composite-slos.yaml");
    const RULES_PATH: &str = "monitoring/composite-slo-rules.yaml";

    fn config() -> CompositeConfig {
        serde_yaml::from_str(CONFIG).unwrap()
    }

    /// `record:` names across the repository's monitoring rule files.
    fn known_records() -> Vec<String> {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/monitoring");
        std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| {
                let p = e.ok()?.path();
                (p.extension()? == "yaml").then(|| std::fs::read_to_string(p).ok())?
            })
            .flat_map(|s| {
                s.lines()
                    .filter_map(|l| l.trim().strip_prefix("- record: ").map(str::to_string))
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    #[test]
    fn repository_config_is_valid() {
        validate(&config(), &known_records()).unwrap();
    }

    #[test]
    fn committed_rules_match_config() {
        let path = format!("{}/{RULES_PATH}", env!("CARGO_MANIFEST_DIR"));
        let rendered = render_rules(&config());
        if std::env::var_os("STELLAR_REGENERATE_SLO_RULES").is_some() {
            std::fs::write(&path, &rendered).unwrap();
        }
        let committed = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            committed == rendered,
            "{RULES_PATH} is out of date; run STELLAR_REGENERATE_SLO_RULES=1 cargo test --lib composite_slo"
        );
    }

    #[test]
    fn rules_publish_composite_and_budget() {
        let r = render_rules(&config());
        assert!(r.contains(
            "sum by (service) (0.5 * stellar:slo:api_availability:ratio{service=\"stellar-api\"})\n          + sum by (service) (0.3 * stellar:slo:api_latency:ratio{service=\"stellar-api\"})"
        ));
        assert!(r.contains(
            "expr: (1 - stellar:slo:composite:ratio_rate30d{service=\"stellar-api\"}) / 0.002"
        ));
        assert!(r.contains("record: stellar:slo:composite:error_budget_remaining"));
        let parsed: serde_yaml::Value = serde_yaml::from_str(&r).unwrap();
        assert_eq!(
            parsed["groups"][0]["rules"].as_sequence().unwrap().len(),
            13
        );
    }

    #[test]
    fn weight_changes_require_versioned_review() {
        let known = known_records();
        let mut cfg = config();
        cfg.composites[0].slis[0].weight = 0.4;
        cfg.composites[0].slis[1].weight = 0.4;
        let err = validate(&cfg, &known).unwrap_err();
        assert!(err.contains("weights differ from review v1"), "{err}");

        cfg.composites[0].version = 2;
        assert!(validate(&cfg, &known)
            .unwrap_err()
            .contains("version 2 has no review"));

        let mut review = cfg.composites[0].reviews[0].clone();
        review.version = 2;
        review.weights.insert("availability".into(), 0.4);
        review.weights.insert("latency".into(), 0.4);
        cfg.composites[0].reviews.push(review);
        validate(&cfg, &known).unwrap();

        cfg.composites[0].slis[2].weight = 0.3;
        assert!(validate(&cfg, &known)
            .unwrap_err()
            .contains("weights sum to"));
    }

    #[test]
    fn every_tier1_service_needs_a_composite() {
        let mut cfg = config();
        cfg.tier1_services.push("horizon".into());
        assert!(validate(&cfg, &known_records())
            .unwrap_err()
            .contains("tier-1 service 'horizon'"));
    }

    /// 30 days of per-minute SLI samples: composite error-budget burn must
    /// match the manual multi-SLI calculation within 1%.
    #[test]
    fn composite_budget_matches_manual_calculation_over_30_days() {
        let slo = config().composites.remove(0);
        let mut rng = SmallRng::seed_from_u64(1524);
        let minutes = 30 * 24 * 60;
        let mut per_sli: BTreeMap<String, Vec<f64>> = BTreeMap::new();
        let mut composite = Vec::with_capacity(minutes);
        for m in 0..minutes {
            let incident = (20_000..20_120).contains(&m);
            let values: BTreeMap<String, f64> = slo
                .slis
                .iter()
                .map(|s| {
                    let base: f64 = if incident { 0.9 } else { 0.9995 };
                    let v: f64 = (base + rng.gen_range(-0.0005..0.0005)).min(1.0);
                    per_sli.entry(s.name.clone()).or_default().push(v);
                    (s.name.clone(), v)
                })
                .collect();
            composite.push(composite_ratio(&slo, &values).unwrap());
        }
        let from_composite = budget_consumed(&composite, slo.target);

        // Manual: weighted average of each SLI's own 30d success ratio.
        let manual_mean: f64 = slo
            .slis
            .iter()
            .map(|s| {
                let v = &per_sli[&s.name];
                s.weight * v.iter().sum::<f64>() / v.len() as f64
            })
            .sum();
        let manual = (1.0 - manual_mean) / (1.0 - slo.target);
        assert!(from_composite > 0.0);
        assert!(((from_composite - manual) / manual).abs() < 0.01);
    }
}
