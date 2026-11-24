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

//! # Hierarchical multi-tenant fair-share scheduling (#1959)
//!
//! Replaces rigid per-namespace quotas with a **quota tree** declared through
//! [`crd::TenantQuota`] resources:
//!
//! * every leaf owns namespaces, a **guaranteed minimum** and an **elastic maximum**;
//! * interior nodes group tenants (parent/child hierarchy) and bound their subtree;
//! * idle capacity can be borrowed up to the maximum and is **reclaimed through
//!   preemption** when a tenant below its guarantee needs it back;
//! * existing Kubernetes `ResourceQuota` objects stay in place as a coarse safety net.
//!
//! The scheduler integration is a kube-scheduler **extender** (`/filter`,
//! `/prioritize`, `/preempt`; see [`extender`]). Upstream kube-scheduler is not forked.
//!
//! See `docs/fair-share-scheduling.md` for the algorithm and migration guide.

pub mod crd;
pub mod engine;
pub mod extender;
pub mod tree;

use serde::{Deserialize, Serialize};

/// "No limit" sentinel (kept far below `u64::MAX` so sums never overflow).
pub const UNBOUNDED: u64 = u64::MAX / 4;

/// CPU (millicores) and memory (bytes). Extended resources are out of scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ResVec {
    pub cpu_m: u64,
    pub mem_b: u64,
}

impl ResVec {
    pub const ZERO: ResVec = ResVec { cpu_m: 0, mem_b: 0 };
    pub const UNLIMITED: ResVec = ResVec {
        cpu_m: UNBOUNDED,
        mem_b: UNBOUNDED,
    };

    pub fn new(cpu_m: u64, mem_b: u64) -> Self {
        Self { cpu_m, mem_b }
    }
    pub fn add(self, o: Self) -> Self {
        Self::new(
            self.cpu_m.saturating_add(o.cpu_m).min(UNBOUNDED),
            self.mem_b.saturating_add(o.mem_b).min(UNBOUNDED),
        )
    }
    pub fn sub(self, o: Self) -> Self {
        Self::new(
            self.cpu_m.saturating_sub(o.cpu_m),
            self.mem_b.saturating_sub(o.mem_b),
        )
    }
    pub fn min(self, o: Self) -> Self {
        Self::new(self.cpu_m.min(o.cpu_m), self.mem_b.min(o.mem_b))
    }
    pub fn max(self, o: Self) -> Self {
        Self::new(self.cpu_m.max(o.cpu_m), self.mem_b.max(o.mem_b))
    }
    /// True if both dimensions are `<=` the other's.
    pub fn fits_in(self, o: Self) -> bool {
        self.cpu_m <= o.cpu_m && self.mem_b <= o.mem_b
    }
    pub fn is_zero(self) -> bool {
        self == Self::ZERO
    }
    /// Dominant share of `self` relative to `denom` (max over dimensions, 0 if denom is 0).
    pub fn dominant_share(self, denom: Self) -> f64 {
        let r = |a: u64, b: u64| if b == 0 { 0.0 } else { a as f64 / b as f64 };
        r(self.cpu_m, denom.cpu_m).max(r(self.mem_b, denom.mem_b))
    }
}

/// Parse a Kubernetes CPU quantity (`"2"`, `"1.5"`, `"500m"`) into millicores.
pub fn parse_cpu(s: &str) -> Result<u64, String> {
    let s = s.trim();
    if let Some(m) = s.strip_suffix('m') {
        return m
            .parse::<u64>()
            .map_err(|_| format!("invalid cpu quantity {s:?}"));
    }
    let v: f64 = s
        .parse()
        .map_err(|_| format!("invalid cpu quantity {s:?}"))?;
    if !v.is_finite() || v < 0.0 {
        return Err(format!("invalid cpu quantity {s:?}"));
    }
    Ok((v * 1000.0).round() as u64)
}

/// Parse a Kubernetes memory quantity (`"128974848"`, `"64Mi"`, `"2Gi"`, `"1G"`) into bytes.
pub fn parse_mem(s: &str) -> Result<u64, String> {
    let s = s.trim();
    const UNITS: &[(&str, u64)] = &[
        ("Ki", 1 << 10),
        ("Mi", 1 << 20),
        ("Gi", 1 << 30),
        ("Ti", 1 << 40),
        ("k", 1_000),
        ("K", 1_000),
        ("M", 1_000_000),
        ("G", 1_000_000_000),
        ("T", 1_000_000_000_000),
    ];
    for (suffix, mult) in UNITS {
        if let Some(n) = s.strip_suffix(suffix) {
            let v: f64 = n
                .parse()
                .map_err(|_| format!("invalid memory quantity {s:?}"))?;
            if !v.is_finite() || v < 0.0 {
                return Err(format!("invalid memory quantity {s:?}"));
            }
            return Ok((v * *mult as f64).round() as u64);
        }
    }
    s.parse::<u64>()
        .map_err(|_| format!("invalid memory quantity {s:?}"))
}

/// Jain's fairness index over `xs` (1.0 = perfectly fair; empty/all-zero => 1.0).
pub fn jain_index(xs: &[f64]) -> f64 {
    let n = xs.len() as f64;
    let sum: f64 = xs.iter().sum();
    let sq: f64 = xs.iter().map(|x| x * x).sum();
    if xs.is_empty() || sq == 0.0 {
        1.0
    } else {
        sum * sum / (n * sq)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantities() {
        assert_eq!(parse_cpu("500m").unwrap(), 500);
        assert_eq!(parse_cpu("2").unwrap(), 2000);
        assert_eq!(parse_cpu("1.5").unwrap(), 1500);
        assert!(parse_cpu("-1").is_err() && parse_cpu("x").is_err());
        assert_eq!(parse_mem("64Mi").unwrap(), 64 << 20);
        assert_eq!(parse_mem("2Gi").unwrap(), 2 << 30);
        assert_eq!(parse_mem("1G").unwrap(), 1_000_000_000);
        assert_eq!(parse_mem("1024").unwrap(), 1024);
        assert!(parse_mem("12Xi").is_err());
    }

    #[test]
    fn jain() {
        assert!((jain_index(&[1.0, 1.0, 1.0]) - 1.0).abs() < 1e-12);
        assert!((jain_index(&[1.0, 0.0]) - 0.5).abs() < 1e-12);
        assert_eq!(jain_index(&[]), 1.0);
    }

    #[test]
    fn resvec_never_overflows() {
        assert_eq!(ResVec::UNLIMITED.add(ResVec::UNLIMITED), ResVec::UNLIMITED);
        assert_eq!(ResVec::new(1, 1).sub(ResVec::new(5, 5)), ResVec::ZERO);
    }
}
