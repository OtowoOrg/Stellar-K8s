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

//! Quota tree: construction, validation and hierarchical entitlement computation.
//!
//! ## Allocation algorithm
//!
//! Per resource dimension independently (CPU and memory), top-down from the cluster
//! capacity, each node splits its budget among its children with **weighted max-min
//! fairness with guarantees and caps** (progressive filling):
//!
//! 1. each child first receives `min(guaranteed, demand)`;
//! 2. the remainder is shared in proportion to `weight` among children that still
//!    want more, never exceeding `min(max, demand)`; capacity freed by saturated
//!    children is redistributed until nothing is left or every child is satisfied.
//!
//! Integer arithmetic is used throughout, so guarantees are never approximated
//! below their configured value. If the declared guarantees of siblings exceed the
//! budget (misconfiguration) they are scaled down proportionally and
//! [`Entitlements::oversubscribed`] is set so the condition is visible.

use super::crd::TenantQuotaSpec;
use super::{parse_cpu, parse_mem, ResVec, UNBOUNDED};
use std::collections::{HashMap, HashSet};

/// A validation problem in the quota configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    pub node: Option<String>,
    pub message: String,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.node {
            Some(n) => write!(f, "TenantQuota {n}: {}", self.message),
            None => write!(f, "{}", self.message),
        }
    }
}

fn err(node: &str, message: impl Into<String>) -> ConfigError {
    ConfigError {
        node: Some(node.to_string()),
        message: message.into(),
    }
}

/// One validated node.
#[derive(Debug, Clone)]
pub struct Node {
    pub name: String,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub namespaces: Vec<String>,
    pub weight: f64,
    /// Effective guarantee (interior nodes: at least the sum of their children's).
    pub guaranteed: ResVec,
    /// Effective ceiling (inherited from the parent when not declared).
    pub max: ResVec,
    pub depth: usize,
}

/// Validated quota tree.
#[derive(Debug, Clone, Default)]
pub struct Tree {
    pub nodes: Vec<Node>,
    pub roots: Vec<usize>,
    by_name: HashMap<String, usize>,
    ns_to_leaf: HashMap<String, usize>,
}

/// Result of [`Tree::entitlements`].
#[derive(Debug, Clone)]
pub struct Entitlements {
    /// Entitlement per node (indexed like `Tree::nodes`), limited by demand.
    pub by_node: Vec<ResVec>,
    /// Some sibling set had guarantees exceeding its budget and was scaled down.
    pub oversubscribed: bool,
}

fn valid_ns(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 63
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !s.starts_with('-')
        && !s.ends_with('-')
}

impl Tree {
    /// Build and validate a tree from `(metadata.name, spec)` pairs.
    pub fn build(specs: &[(String, TenantQuotaSpec)]) -> Result<Tree, Vec<ConfigError>> {
        let mut errors = Vec::new();
        let mut by_name = HashMap::new();
        for (i, (name, _)) in specs.iter().enumerate() {
            if by_name.insert(name.clone(), i).is_some() {
                errors.push(err(name, "duplicate name"));
            }
        }

        // Quantities and scalar checks.
        struct Decl {
            g: [Option<u64>; 2],
            m: [Option<u64>; 2],
        }
        let mut decls = Vec::new();
        for (name, s) in specs {
            let q = |label: &str,
                     v: &Option<String>,
                     f: fn(&str) -> Result<u64, String>,
                     errors: &mut Vec<ConfigError>| {
                v.as_ref().and_then(|x| match f(x) {
                    Ok(n) => Some(n),
                    Err(e) => {
                        errors.push(err(name, format!("{label}: {e}")));
                        None
                    }
                })
            };
            decls.push(Decl {
                g: [
                    q("guaranteed.cpu", &s.guaranteed.cpu, parse_cpu, &mut errors),
                    q(
                        "guaranteed.memory",
                        &s.guaranteed.memory,
                        parse_mem,
                        &mut errors,
                    ),
                ],
                m: [
                    q("max.cpu", &s.max.cpu, parse_cpu, &mut errors),
                    q("max.memory", &s.max.memory, parse_mem, &mut errors),
                ],
            });
            if !(s.weight.is_finite() && s.weight > 0.0) {
                errors.push(err(
                    name,
                    format!("weight must be a finite number > 0 (got {})", s.weight),
                ));
            }
        }

        // Parents.
        let mut parent: Vec<Option<usize>> = vec![None; specs.len()];
        for (i, (name, s)) in specs.iter().enumerate() {
            if let Some(p) = s.parent.as_deref().filter(|p| !p.is_empty()) {
                if p == name {
                    errors.push(err(name, "parent refers to itself"));
                } else if let Some(&pi) = by_name.get(p) {
                    parent[i] = Some(pi);
                } else {
                    errors.push(err(name, format!("parent {p:?} does not exist")));
                }
            }
        }
        // Cycles.
        for i in 0..specs.len() {
            let (mut cur, mut steps) = (i, 0);
            while let Some(p) = parent[cur] {
                cur = p;
                steps += 1;
                if steps > specs.len() {
                    errors.push(err(&specs[i].0, "parent chain contains a cycle"));
                    break;
                }
            }
        }
        if !errors.is_empty() {
            return Err(errors);
        }

        let mut children: Vec<Vec<usize>> = vec![Vec::new(); specs.len()];
        for (i, p) in parent.iter().enumerate() {
            if let Some(p) = p {
                children[*p].push(i);
            }
        }
        let depth: Vec<usize> = (0..specs.len())
            .map(|mut i| {
                let mut d = 0;
                while let Some(p) = parent[i] {
                    i = p;
                    d += 1;
                }
                d
            })
            .collect();

        // Namespaces.
        let mut ns_owner: HashMap<&str, usize> = HashMap::new();
        for (i, (name, s)) in specs.iter().enumerate() {
            if !s.namespaces.is_empty() && !children[i].is_empty() {
                errors.push(err(name, "only leaf nodes may list namespaces"));
            }
            for ns in &s.namespaces {
                if !valid_ns(ns) {
                    errors.push(err(name, format!("invalid namespace name {ns:?}")));
                } else if let Some(&other) = ns_owner.get(ns.as_str()) {
                    if other != i {
                        errors.push(err(
                            name,
                            format!(
                                "namespace {ns:?} is already claimed by {:?}",
                                specs[other].0
                            ),
                        ));
                    } else {
                        errors.push(err(name, format!("namespace {ns:?} listed twice")));
                    }
                } else {
                    ns_owner.insert(ns, i);
                }
            }
        }

        // Effective guarantees, bottom-up.
        let mut order: Vec<usize> = (0..specs.len()).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(depth[i]));
        let mut guar = vec![ResVec::ZERO; specs.len()];
        for &i in &order {
            let sum = children[i]
                .iter()
                .fold(ResVec::ZERO, |a, &c| a.add(guar[c]));
            let d = &decls[i];
            let declared = [d.g[0], d.g[1]];
            let mut eff = [0u64; 2];
            let sums = [sum.cpu_m, sum.mem_b];
            let names = ["cpu", "memory"];
            for k in 0..2 {
                match declared[k] {
                    Some(v) if v < sums[k] && !children[i].is_empty() => {
                        errors.push(err(
                            &specs[i].0,
                            format!(
                                "guaranteed.{} ({v}) is lower than the sum of its children's guarantees ({})",
                                names[k], sums[k]
                            ),
                        ));
                        eff[k] = sums[k];
                    }
                    Some(v) => eff[k] = v.max(sums[k]),
                    None => eff[k] = sums[k],
                }
            }
            guar[i] = ResVec::new(eff[0], eff[1]);
        }

        // Effective maxima, top-down; a child may not declare more than its parent.
        let mut maxv = vec![ResVec::UNLIMITED; specs.len()];
        order.reverse();
        for &i in &order {
            let inherited = parent[i].map(|p| maxv[p]).unwrap_or(ResVec::UNLIMITED);
            let d = &decls[i];
            let mut eff = [inherited.cpu_m, inherited.mem_b];
            let names = ["cpu", "memory"];
            for k in 0..2 {
                if let Some(v) = d.m[k] {
                    if v > eff[k] && parent[i].is_some() {
                        errors.push(err(
                            &specs[i].0,
                            format!(
                                "max.{} ({v}) exceeds its parent's max ({})",
                                names[k], eff[k]
                            ),
                        ));
                    } else {
                        eff[k] = v;
                    }
                }
            }
            maxv[i] = ResVec::new(eff[0], eff[1]);
            if !guar[i].fits_in(maxv[i]) {
                errors.push(err(&specs[i].0, "guaranteed exceeds max"));
            }
        }

        if !errors.is_empty() {
            return Err(errors);
        }

        let nodes: Vec<Node> = specs
            .iter()
            .enumerate()
            .map(|(i, (name, s))| Node {
                name: name.clone(),
                parent: parent[i],
                children: children[i].clone(),
                namespaces: s.namespaces.clone(),
                weight: s.weight,
                guaranteed: guar[i],
                max: maxv[i],
                depth: depth[i],
            })
            .collect();
        let ns_to_leaf = ns_owner
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        let roots = (0..nodes.len())
            .filter(|&i| nodes[i].parent.is_none())
            .collect();
        Ok(Tree {
            nodes,
            roots,
            by_name,
            ns_to_leaf,
        })
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.by_name.get(name).copied()
    }
    /// Leaf governing `namespace`, if any.
    pub fn leaf_for_namespace(&self, namespace: &str) -> Option<usize> {
        self.ns_to_leaf.get(namespace).copied()
    }
    pub fn leaves(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.nodes.len()).filter(|&i| self.nodes[i].children.is_empty())
    }
    /// `node` and all its ancestors, node first.
    pub fn path_to_root(&self, node: usize) -> Vec<usize> {
        let mut v = vec![node];
        let mut cur = node;
        while let Some(p) = self.nodes[cur].parent {
            v.push(p);
            cur = p;
        }
        v
    }
    /// Lowest common ancestor of `a` and `b` (None when in different root trees).
    pub fn lca(&self, a: usize, b: usize) -> Option<usize> {
        let pa: HashSet<usize> = self.path_to_root(a).into_iter().collect();
        self.path_to_root(b).into_iter().find(|n| pa.contains(n))
    }

    /// Sum of root guarantees must fit the cluster; returns offending dimensions.
    pub fn check_capacity(&self, capacity: ResVec) -> Vec<ConfigError> {
        let total = self
            .roots
            .iter()
            .fold(ResVec::ZERO, |a, &r| a.add(self.nodes[r].guaranteed));
        let mut out = Vec::new();
        if total.cpu_m > capacity.cpu_m {
            out.push(ConfigError {
                node: None,
                message: format!(
                    "guaranteed cpu ({}m) exceeds cluster capacity ({}m)",
                    total.cpu_m, capacity.cpu_m
                ),
            });
        }
        if total.mem_b > capacity.mem_b {
            out.push(ConfigError {
                node: None,
                message: format!(
                    "guaranteed memory ({}) exceeds cluster capacity ({})",
                    total.mem_b, capacity.mem_b
                ),
            });
        }
        out
    }

    /// Hierarchical entitlements for the given per-leaf `demand` and cluster `capacity`.
    pub fn entitlements(&self, demand: &HashMap<usize, ResVec>, capacity: ResVec) -> Entitlements {
        let n = self.nodes.len();
        // Subtree demand, capped by max, bottom-up.
        let mut dem = vec![ResVec::ZERO; n];
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(self.nodes[i].depth));
        for &i in &order {
            let raw = if self.nodes[i].children.is_empty() {
                demand.get(&i).copied().unwrap_or(ResVec::ZERO)
            } else {
                self.nodes[i]
                    .children
                    .iter()
                    .fold(ResVec::ZERO, |a, &c| a.add(dem[c]))
            };
            dem[i] = raw.min(self.nodes[i].max);
        }
        let mut ent = vec![ResVec::ZERO; n];
        let mut over = false;
        let mut split = |budget: ResVec, kids: &[usize], ent: &mut Vec<ResVec>| {
            for dim in 0..2 {
                let get = |v: ResVec| if dim == 0 { v.cpu_m } else { v.mem_b };
                let items: Vec<(u64, u64, f64)> = kids
                    .iter()
                    .map(|&k| {
                        (
                            get(self.nodes[k].guaranteed),
                            get(dem[k]),
                            self.nodes[k].weight,
                        )
                    })
                    .collect();
                let (alloc, o) = waterfill(get(budget), &items);
                over |= o;
                for (j, &k) in kids.iter().enumerate() {
                    if dim == 0 {
                        ent[k].cpu_m = alloc[j];
                    } else {
                        ent[k].mem_b = alloc[j];
                    }
                }
            }
        };
        split(capacity, &self.roots, &mut ent);
        let mut by_depth: Vec<usize> = (0..n)
            .filter(|&i| !self.nodes[i].children.is_empty())
            .collect();
        by_depth.sort_by_key(|&i| self.nodes[i].depth);
        for i in by_depth {
            let budget = ent[i];
            let kids = self.nodes[i].children.clone();
            split(budget, &kids, &mut ent);
        }
        Entitlements {
            by_node: ent,
            oversubscribed: over,
        }
    }
}

/// Compare per-namespace `ResourceQuota` hard limits (the coarse safety net) with the
/// quota tree and report configurations where the two would fight (double enforcement).
///
/// `namespace_quota` maps namespace -> `spec.hard` requests (`requests.cpu`/`requests.memory`).
/// * hard < tenant guarantee => the safety net blocks a promised minimum (error);
/// * hard < tenant max       => elasticity is silently capped by the ResourceQuota (warning text).
pub fn check_resource_quota_coexistence(
    tree: &Tree,
    namespace_quota: &HashMap<String, ResVec>,
) -> Vec<ConfigError> {
    let mut out = Vec::new();
    for leaf in tree.leaves() {
        let n = &tree.nodes[leaf];
        if n.namespaces.is_empty()
            || !n
                .namespaces
                .iter()
                .all(|ns| namespace_quota.contains_key(ns))
        {
            continue; // a namespace without a ResourceQuota imposes no safety-net limit
        }
        let hard = n
            .namespaces
            .iter()
            .fold(ResVec::ZERO, |a, ns| a.add(namespace_quota[ns]));
        if !n.guaranteed.fits_in(hard) {
            out.push(err(&n.name, format!(
                "ResourceQuota hard limits ({}m cpu, {} B) are below the guarantee ({}m, {} B): the safety net would block guaranteed capacity",
                hard.cpu_m, hard.mem_b, n.guaranteed.cpu_m, n.guaranteed.mem_b)));
        } else if !n.max.fits_in(hard) && n.max.cpu_m < UNBOUNDED && n.max.mem_b < UNBOUNDED {
            out.push(err(&n.name, format!(
                "warning: ResourceQuota hard limits ({}m cpu, {} B) are below max ({}m, {} B): elastic borrowing is capped by the ResourceQuota",
                hard.cpu_m, hard.mem_b, n.max.cpu_m, n.max.mem_b)));
        }
    }
    out
}

/// Weighted max-min fair division of `budget` among `items = (guarantee, cap, weight)`.
///
/// Returns the allocation and whether guarantees had to be scaled down.
/// Invariants (property-tested): `alloc[i] <= cap[i]`; `alloc[i] >= min(guarantee[i], cap[i])`
/// unless oversubscribed; `sum(alloc) <= budget`; work-conserving when demand allows.
pub fn waterfill(budget: u64, items: &[(u64, u64, f64)]) -> (Vec<u64>, bool) {
    let n = items.len();
    let mut alloc: Vec<u64> = items.iter().map(|&(g, cap, _)| g.min(cap)).collect();
    let sum_g: u128 = alloc.iter().map(|&x| x as u128).sum();
    if sum_g > budget as u128 {
        for a in alloc.iter_mut() {
            *a = (*a as u128 * budget as u128 / sum_g) as u64;
        }
        return (alloc, true);
    }
    let mut rem = budget - sum_g as u64;
    let wint: Vec<u128> = items
        .iter()
        .map(|&(_, _, w)| ((w * 1_000_000.0) as u128).max(1))
        .collect();
    for _ in 0..(n + 64) {
        if rem == 0 {
            break;
        }
        let active: Vec<usize> = (0..n).filter(|&i| alloc[i] < items[i].1).collect();
        if active.is_empty() {
            break;
        }
        let wsum: u128 = active.iter().map(|&i| wint[i]).sum();
        let mut given = 0u64;
        for &i in &active {
            let share = (rem as u128 * wint[i] / wsum) as u64;
            let give = share.min(items[i].1 - alloc[i]);
            alloc[i] += give;
            given += give;
        }
        rem -= given;
        if given == 0 {
            // Fewer units than active items: hand out single units, heaviest first.
            let mut by_w = active.clone();
            by_w.sort_by(|&a, &b| wint[b].cmp(&wint[a]).then(a.cmp(&b)));
            for i in by_w {
                if rem > 0 && alloc[i] < items[i].1 {
                    alloc[i] += 1;
                    rem -= 1;
                }
            }
        }
    }
    (alloc, false)
}

#[allow(dead_code)]
const _: u64 = UNBOUNDED;
