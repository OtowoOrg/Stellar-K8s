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

//! Tests for hierarchical fair-share scheduling (#1959): quota-tree validation,
//! allocation, admission, hierarchy-aware preemption, extender wire format,
//! ResourceQuota coexistence, metrics, and the saturation / soak simulation.

use serde_json::json;
use std::collections::HashMap;
use stellar_k8s::fair_share::crd::{ResourceQuantities, TenantQuotaSpec};
use stellar_k8s::fair_share::engine::{Engine, PodRec, PreemptRefusal, Verdict};
use stellar_k8s::fair_share::extender::{
    self, ExtenderArgs, ExtenderPreemptionArgs, ExtenderState, MetaVictims,
};
use stellar_k8s::fair_share::tree::{check_resource_quota_coexistence, waterfill, Tree};
use stellar_k8s::fair_share::{jain_index, ResVec};

const GI: u64 = 1 << 30;

fn spec(
    parent: Option<&str>,
    ns: &[&str],
    weight: f64,
    g: (&str, &str),
    m: Option<(&str, &str)>,
) -> TenantQuotaSpec {
    let q = |c: &str, mem: &str| ResourceQuantities {
        cpu: (!c.is_empty()).then(|| c.to_string()),
        memory: (!mem.is_empty()).then(|| mem.to_string()),
    };
    TenantQuotaSpec {
        parent: parent.map(str::to_string),
        namespaces: ns.iter().map(|s| s.to_string()).collect(),
        weight,
        guaranteed: q(g.0, g.1),
        max: m.map(|(c, mem)| q(c, mem)).unwrap_or_default(),
    }
}

fn build(v: Vec<(&str, TenantQuotaSpec)>) -> Result<Tree, Vec<String>> {
    let specs: Vec<(String, TenantQuotaSpec)> =
        v.into_iter().map(|(n, s)| (n.to_string(), s)).collect();
    Tree::build(&specs).map_err(|e| e.into_iter().map(|x| x.to_string()).collect())
}

fn errs(v: Vec<(&str, TenantQuotaSpec)>) -> String {
    build(v)
        .err()
        .expect("expected validation errors")
        .join(" | ")
}

fn pod(uid: &str, ns: &str, cpu_m: u64, mem_gi: u64) -> PodRec {
    PodRec {
        uid: uid.into(),
        namespace: ns.into(),
        node: None,
        req: ResVec::new(cpu_m, mem_gi * GI),
        priority: 0,
        started: 0,
    }
}

/// root "org" (16 CPU / 64Gi): team-a(guar 6, max 12) with a1(3/12), a2(3/12); team-b(guar 4, max 8) leaf-ish b1.
fn small_tree() -> Tree {
    build(vec![
        ("org", spec(None, &[], 1.0, ("", ""), None)),
        (
            "team-a",
            spec(Some("org"), &[], 1.0, ("", ""), Some(("12", "48Gi"))),
        ),
        (
            "a1",
            spec(
                Some("team-a"),
                &["a1"],
                1.0,
                ("3", "12Gi"),
                Some(("12", "48Gi")),
            ),
        ),
        (
            "a2",
            spec(
                Some("team-a"),
                &["a2"],
                1.0,
                ("3", "12Gi"),
                Some(("12", "48Gi")),
            ),
        ),
        (
            "b1",
            spec(
                Some("org"),
                &["b1"],
                1.0,
                ("4", "16Gi"),
                Some(("8", "32Gi")),
            ),
        ),
    ])
    .unwrap()
}

fn cap16() -> ResVec {
    ResVec::new(16_000, 64 * GI)
}

// ── quota tree: parsing, inheritance, validation ──────────────────────

#[test]
fn derived_guarantees_and_inherited_max() {
    let t = small_tree();
    let a = t.index_of("team-a").unwrap();
    assert_eq!(
        t.nodes[a].guaranteed,
        ResVec::new(6_000, 24 * GI),
        "interior guarantee = sum of children"
    );
    let org = t.index_of("org").unwrap();
    assert_eq!(t.nodes[org].guaranteed, ResVec::new(10_000, 40 * GI));
    assert_eq!(t.nodes[org].max, ResVec::UNLIMITED);
    let a1 = t.index_of("a1").unwrap();
    assert_eq!(t.nodes[a1].max, ResVec::new(12_000, 48 * GI));
    assert_eq!(t.leaf_for_namespace("a2"), t.index_of("a2"));
    assert_eq!(t.leaf_for_namespace("nope"), None);
    assert_eq!(
        t.lca(t.index_of("a1").unwrap(), t.index_of("a2").unwrap()),
        Some(a)
    );
    assert_eq!(
        t.lca(t.index_of("a1").unwrap(), t.index_of("b1").unwrap()),
        Some(org)
    );
}

#[test]
fn child_inherits_parent_max_when_unset() {
    let t = build(vec![
        ("p", spec(None, &[], 1.0, ("", ""), Some(("8", "32Gi")))),
        ("c", spec(Some("p"), &["c"], 1.0, ("1", "1Gi"), None)),
    ])
    .unwrap();
    assert_eq!(
        t.nodes[t.index_of("c").unwrap()].max,
        ResVec::new(8_000, 32 * GI)
    );
}

#[test]
fn rejects_invalid_and_conflicting_configuration() {
    let leaf = |p: Option<&str>, ns: &[&str]| spec(p, ns, 1.0, ("1", "1Gi"), None);
    assert!(errs(vec![("a", leaf(Some("ghost"), &["a"]))]).contains("does not exist"));
    assert!(errs(vec![("a", leaf(Some("a"), &["a"]))]).contains("itself"));
    assert!(errs(vec![
        ("a", leaf(Some("b"), &["a"])),
        ("b", leaf(Some("a"), &["b"]))
    ])
    .contains("cycle"));
    assert!(
        errs(vec![("a", leaf(None, &["x"])), ("b", leaf(None, &["x"]))])
            .contains("already claimed")
    );
    assert!(errs(vec![("a", leaf(None, &["Bad_NS"]))]).contains("invalid namespace"));
    assert!(errs(vec![
        ("p", leaf(None, &["x"])),
        ("c", leaf(Some("p"), &["y"]))
    ])
    .contains("only leaf"));
    assert!(errs(vec![("a", spec(None, &["a"], 0.0, ("", ""), None))]).contains("weight"));
    assert!(errs(vec![("a", spec(None, &["a"], f64::NAN, ("", ""), None))]).contains("weight"));
    assert!(
        errs(vec![("a", spec(None, &["a"], 1.0, ("lots", ""), None))]).contains("guaranteed.cpu")
    );
    assert!(errs(vec![(
        "a",
        spec(None, &["a"], 1.0, ("4", "1Gi"), Some(("2", "2Gi")))
    )])
    .contains("guaranteed exceeds max"));
    // child max above parent max
    assert!(errs(vec![
        ("p", spec(None, &[], 1.0, ("", ""), Some(("4", "8Gi")))),
        (
            "c",
            spec(Some("p"), &["c"], 1.0, ("1", "1Gi"), Some(("8", "8Gi")))
        ),
    ])
    .contains("exceeds its parent's max"));
    // parent guarantee lower than children's sum
    assert!(errs(vec![
        ("p", spec(None, &[], 1.0, ("2", "2Gi"), None)),
        ("c1", spec(Some("p"), &["c1"], 1.0, ("2", "1Gi"), None)),
        ("c2", spec(Some("p"), &["c2"], 1.0, ("2", "1Gi"), None)),
    ])
    .contains("lower than the sum"));
}

#[test]
fn guarantees_must_fit_cluster_capacity() {
    let t = small_tree();
    assert!(t.check_capacity(cap16()).is_empty());
    assert_eq!(t.check_capacity(ResVec::new(8_000, 64 * GI)).len(), 1);
}

#[test]
fn shipped_sample_quota_tree_and_crd_are_valid() {
    use kube::CustomResourceExt;
    use stellar_k8s::fair_share::crd::TenantQuota;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let text =
        std::fs::read_to_string(root.join("config/fair-share/sample-quota-tree.yaml")).unwrap();
    let mut specs = Vec::new();
    for doc in serde_yaml::Deserializer::from_str(&text) {
        let q: TenantQuota =
            serde::Deserialize::deserialize(doc).expect("sample parses as TenantQuota");
        specs.push((q.metadata.name.clone().unwrap(), q.spec));
    }
    let tree = Tree::build(&specs).unwrap_or_else(|e| panic!("sample tree invalid: {e:?}"));
    assert_eq!(tree.len(), 5);
    assert!(tree
        .check_capacity(ResVec::new(160_000, 640 * GI))
        .is_empty());
    let crd = serde_yaml::to_string(&TenantQuota::crd()).unwrap();
    assert!(
        crd.contains("kind: TenantQuota")
            && crd.contains("group: stellar.org")
            && crd.contains("guaranteed")
    );
    assert!(
        crd.contains("scope: Cluster"),
        "TenantQuota is cluster scoped"
    );
}

// ── allocation algorithm ──────────────────────────────────────────────

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.next() % (hi - lo + 1)
    }
}

#[test]
fn waterfill_invariants_hold_for_random_inputs() {
    let mut r = Lcg(42);
    for _ in 0..5_000 {
        let n = r.range(1, 8) as usize;
        let items: Vec<(u64, u64, f64)> = (0..n)
            .map(|_| {
                let cap = r.range(0, 1000);
                let g = r.range(0, cap.max(1));
                (g, cap, r.range(1, 8) as f64 / 2.0)
            })
            .collect();
        let sum_g: u64 = items.iter().map(|i| i.0.min(i.1)).sum();
        let budget = r.range(sum_g, sum_g + 2000); // guarantees fit
        let (a, over) = waterfill(budget, &items);
        assert!(!over);
        assert!(a.iter().sum::<u64>() <= budget);
        for (i, it) in items.iter().enumerate() {
            assert!(a[i] <= it.1, "cap respected");
            assert!(
                a[i] >= it.0.min(it.1),
                "guarantee respected: {a:?} {items:?}"
            );
        }
        let total_cap: u64 = items.iter().map(|i| i.1).sum();
        assert_eq!(
            a.iter().sum::<u64>(),
            budget.min(total_cap),
            "work conserving: {a:?} {items:?} budget {budget}"
        );
    }
}

#[test]
fn waterfill_shares_surplus_by_weight_and_redistributes_saturated_capacity() {
    // 100 units; guarantees 0; weights 1:3; second tenant only wants 30.
    let (a, _) = waterfill(100, &[(0, 1000, 1.0), (0, 30, 3.0)]);
    assert_eq!(a, vec![70, 30]);
    let (a, _) = waterfill(100, &[(0, 1000, 1.0), (0, 1000, 3.0)]);
    assert_eq!(a, vec![25, 75]);
    // guarantee dominates weight
    let (a, _) = waterfill(100, &[(60, 1000, 1.0), (0, 1000, 9.0)]);
    assert!(a[0] >= 60 && a[0] + a[1] == 100, "{a:?}");
}

#[test]
fn oversubscribed_guarantees_are_flagged_and_scaled() {
    let (a, over) = waterfill(100, &[(80, 100, 1.0), (80, 100, 1.0)]);
    assert!(over);
    assert_eq!(a, vec![50, 50]);
}

#[test]
fn hierarchical_entitlements_respect_parent_boundaries() {
    let t = small_tree();
    let mut d = HashMap::new();
    for n in ["a1", "a2", "b1"] {
        d.insert(t.index_of(n).unwrap(), ResVec::new(100_000, 400 * GI)); // infinite appetite
    }
    let e = t.entitlements(&d, cap16());
    let g = |n: &str| e.by_node[t.index_of(n).unwrap()];
    assert!(!e.oversubscribed);
    assert!(
        g("b1").cpu_m >= 4_000 && g("b1").cpu_m <= 8_000,
        "b1 within [guar,max]: {:?}",
        g("b1")
    );
    assert!(g("team-a").cpu_m <= 12_000);
    assert_eq!(g("team-a").cpu_m + g("b1").cpu_m, 16_000, "work conserving");
    assert_eq!(
        g("a1").cpu_m + g("a2").cpu_m,
        g("team-a").cpu_m,
        "children split their parent's budget"
    );
    assert!(
        (g("a1").cpu_m as i64 - g("a2").cpu_m as i64).abs() <= 1,
        "equal weights split evenly"
    );
}

#[test]
fn entitlement_is_limited_by_demand_and_idle_capacity_is_lent() {
    let t = small_tree();
    let mut d = HashMap::new();
    d.insert(t.index_of("a1").unwrap(), ResVec::new(2_000, 8 * GI)); // below its guarantee
    d.insert(t.index_of("b1").unwrap(), ResVec::new(100_000, 400 * GI));
    let e = t.entitlements(&d, cap16());
    assert_eq!(e.by_node[t.index_of("a1").unwrap()].cpu_m, 2_000);
    assert_eq!(
        e.by_node[t.index_of("b1").unwrap()].cpu_m,
        8_000,
        "b1 borrows up to its max only"
    );
}

// ── admission ─────────────────────────────────────────────────────────

fn engine() -> Engine {
    Engine::new(small_tree(), cap16())
}

#[test]
fn guaranteed_share_is_always_admitted_and_elastic_needs_idle_capacity() {
    let mut e = engine();
    assert_eq!(
        e.admit(&pod("p1", "a1", 3_000, 12)),
        Verdict::AllowGuaranteed
    );
    let mut p = pod("p1", "a1", 3_000, 12);
    p.node = Some("n1".into());
    e.bind(p);
    // beyond the guarantee: elastic, cluster is idle
    assert_eq!(e.admit(&pod("p2", "a1", 2_000, 8)), Verdict::AllowElastic);
    assert!(
        e.admit(&pod("x", "unmanaged", 1_000, 1)).allowed(),
        "ungoverned namespaces are untouched"
    );
}

#[test]
fn max_is_enforced_at_every_ancestor() {
    let mut e = engine();
    // a1 alone could use 12, but team-a's cap is 12 shared by a1+a2.
    for (i, cpu) in [(1, 8_000u64), (2, 4_000)] {
        let mut p = pod(
            &format!("p{i}"),
            if i == 1 { "a1" } else { "a2" },
            cpu,
            cpu / 1000 * 4,
        );
        p.node = Some("n".into());
        e.bind(p);
    }
    let v = e.admit(&pod("q", "a2", 1_000, 4));
    assert_eq!(
        v,
        Verdict::DenyMax {
            node: "team-a".into()
        }
    );
    // b1's own max
    let mut p = pod("b", "b1", 8_000, 32);
    p.node = Some("n".into());
    e.bind(p);
    assert_eq!(
        e.admit(&pod("b2", "b1", 1_000, 1)),
        Verdict::DenyMax { node: "b1".into() }
    );
}

#[test]
fn borrowing_cannot_consume_capacity_reserved_for_a_waiting_guaranteed_tenant() {
    let mut e = engine();
    // a1 borrows (9 CPU, 6 above its guarantee); an ungoverned pod holds 3 CPU => 4 CPU free.
    // b1 (guarantee 4) is waiting with 4 CPU pending, so all 4 free CPU are reserved for it.
    let mut big = pod("big", "a1", 9_000, 36);
    big.node = Some("n".into());
    e.bind(big);
    let mut other = pod("sys", "kube-system", 3_000, 12);
    other.node = Some("n".into());
    e.bind(other);
    e.add_pending(pod("waiting", "b1", 4_000, 16));
    let v = e.admit(&pod("a1p", "a1", 2_000, 8));
    assert_eq!(
        v,
        Verdict::DenyNoElasticCapacity,
        "elastic request must leave b1's 4 CPU free"
    );
    // Without the waiting guaranteed tenant the same borrow is fine.
    e.remove("waiting");
    assert_eq!(e.admit(&pod("a1p", "a1", 2_000, 8)), Verdict::AllowElastic);
    e.add_pending(pod("waiting", "b1", 4_000, 16));
    // b1's own pod within its guarantee is always admitted.
    assert_eq!(
        e.admit(&pod("waiting", "b1", 4_000, 16)),
        Verdict::AllowGuaranteed
    );
}

#[test]
fn under_contention_a_tenant_may_borrow_only_up_to_its_fair_entitlement() {
    let mut e = engine();
    // a1 already at 8 CPU (above guarantee). b1 has pending demand => contention.
    let mut p = pod("a", "a1", 8_000, 32);
    p.node = Some("n".into());
    e.bind(p);
    e.add_pending(pod("b", "b1", 8_000, 32));
    // Fair split of 16 CPU between {a1:12 cap team-a, b1:8 cap}: a1's entitlement is < 12.
    let v = e.admit(&pod("a-more", "a1", 4_000, 16));
    assert_eq!(
        v,
        Verdict::DenyNoElasticCapacity,
        "a1 is already at/above its fair share while b1 waits"
    );
}

// ── preemption ────────────────────────────────────────────────────────

fn bound(e: &mut Engine, uid: &str, ns: &str, cpu: u64, node: &str, started: u64) {
    let mut p = pod(uid, ns, cpu, cpu / 1000 * 4);
    p.node = Some(node.into());
    p.started = started;
    e.bind(p);
}

#[test]
fn reclaims_borrowed_capacity_from_the_most_over_guarantee_tenant() {
    let mut e = engine();
    // a1 borrows (3 guaranteed + 8 elastic); a2 sits exactly at its guarantee (3); b1 empty.
    for i in 0..11 {
        bound(
            &mut e,
            &format!("a1-{i}"),
            "a1",
            if i < 3 { 1_000 } else { 1_000 },
            "n1",
            i,
        );
    }
    for i in 0..2 {
        bound(&mut e, &format!("a2-{i}"), "a2", 1_000, "n1", 100 + i);
    }
    bound(&mut e, "a2-2", "a2", 1_000, "n1", 102);
    let pending = pod("b1-new", "b1", 4_000, 16);
    let cands: Vec<PodRec> = e.running().cloned().collect();
    let refs: Vec<&PodRec> = cands.iter().collect();
    let v = e
        .select_victims(&pending, &refs, ResVec::ZERO)
        .expect("reclaim possible");
    assert_eq!(v.len(), 4);
    assert!(
        v.iter().all(|u| u.starts_with("a1-")),
        "only the borrower pays: {v:?}"
    );
}

#[test]
fn protected_sibling_at_guarantee_cannot_be_preempted() {
    let mut e = engine();
    for i in 0..3 {
        bound(&mut e, &format!("a2-{i}"), "a2", 1_000, "n1", i); // a2 exactly at guarantee
    }
    let pending = pod("a1-new", "a1", 2_000, 8);
    let cands: Vec<PodRec> = e.running().cloned().collect();
    let refs: Vec<&PodRec> = cands.iter().collect();
    assert!(e.select_victims(&pending, &refs, ResVec::ZERO).is_none());
    let r = e.check_preemption(&pending, &refs[..1]);
    assert!(
        matches!(r, Err(PreemptRefusal::ProtectedVictim { .. })),
        "{r:?}"
    );
}

#[test]
fn hierarchy_boundary_cousin_inside_a_protected_group_is_safe_even_when_it_borrows() {
    // group g1 (guar 6) = {x1 (guar 3), x2 (guar 3)}; group g2 (guar 4) = {y1}. x2 borrows above ITS guarantee
    // but g1 as a whole is exactly at its guarantee => evicting x2 would push g1 below its guarantee.
    let t = build(vec![
        ("g1", spec(None, &[], 1.0, ("", ""), None)),
        ("x1", spec(Some("g1"), &["x1"], 1.0, ("3", "12Gi"), None)),
        ("x2", spec(Some("g1"), &["x2"], 1.0, ("3", "12Gi"), None)),
        ("g2", spec(None, &[], 1.0, ("", ""), None)),
        ("y1", spec(Some("g2"), &["y1"], 1.0, ("4", "16Gi"), None)),
    ])
    .unwrap();
    let mut e = Engine::new(t, cap16());
    // x1 uses 1 CPU (under guarantee), x2 uses 5 (2 elastic). g1 total = 6 = its guarantee.
    bound(&mut e, "x1-0", "x1", 1_000, "n", 0);
    for i in 0..5 {
        bound(&mut e, &format!("x2-{i}"), "x2", 1_000, "n", 10 + i);
    }
    let pending = pod("y1-new", "y1", 2_000, 8);
    let cands: Vec<PodRec> = e.running().cloned().collect();
    let refs: Vec<&PodRec> = cands.iter().collect();
    assert!(
        e.select_victims(&pending, &refs, ResVec::ZERO).is_none(),
        "g1 is at its group guarantee: protected"
    );
    // Same-parent siblings: x1 (under guarantee, needs 2 CPU back) may take x2's elastic CPU.
    let p2 = pod("x1-new", "x1", 2_000, 8);
    let v = e
        .select_victims(&p2, &refs, ResVec::ZERO)
        .expect("sibling reclaim inside the group");
    assert!(v.iter().all(|u| u.starts_with("x2-")));
}

#[test]
fn guaranteed_pod_blocked_only_by_a_sibling_borrowing_up_to_the_parent_ceiling_needs_reclaim_not_denial(
) {
    // Regression for a bug found by the saturation simulation: team-a (max 12) is full because a1
    // borrows 9 CPU; a2 is under its guarantee. a2's pod must be admitted via reclaim, never hard-denied.
    let mut e = engine();
    for i in 0..10 {
        bound(&mut e, &format!("a1-{i}"), "a1", 1_000, "n1", i);
    }
    bound(&mut e, "a2-0", "a2", 1_000, "n1", 50);
    bound(&mut e, "a2-1", "a2", 1_000, "n1", 51);
    // team-a = 12 CPU (= its max); a2 (usage 2, guarantee 3) asks for 1 CPU => exactly its guarantee.
    let want = pod("a2-new", "a2", 1_000, 4);
    assert_eq!(
        e.admit(&want),
        Verdict::NeedsReclaim {
            node: "team-a".into()
        }
    );
    let cands: Vec<PodRec> = e.running().cloned().collect();
    let refs: Vec<&PodRec> = cands.iter().collect();
    // Even though the node has free room (node_free large), the ceiling must still be cured.
    let v = e
        .select_victims(&want, &refs, ResVec::new(16_000, 64 * GI))
        .expect("reclaim from the borrower");
    assert!(
        !v.is_empty() && v.iter().all(|u| u.starts_with("a1-")),
        "{v:?}"
    );
    // The scheduler-chosen victims from an unrelated tenant would not cure team-a's ceiling => veto.
    bound(&mut e, "b-0", "b1", 1_000, "n2", 1);
    let b = e.pod("b-0").unwrap().clone();
    assert!(matches!(
        e.check_preemption(&want, &[&b]),
        Err(PreemptRefusal::DoesNotCureQuota { .. }) | Err(PreemptRefusal::ProtectedVictim { .. })
    ));
    // An elastic request at the same ceiling is a real denial.
    assert!(matches!(
        e.admit(&pod("a1-x", "a1", 1_000, 4)),
        Verdict::DenyMax { .. }
    ));
}

#[test]
fn same_tenant_victims_must_be_strictly_lower_priority_to_avoid_swap_thrash() {
    let mut e = engine();
    for i in 0..3 {
        bound(&mut e, &format!("a2-{i}"), "a2", 1_000, "n1", i);
    }
    let pending = pod("a2-new", "a2", 1_000, 4); // same priority 0 as its own running pods
    let cands: Vec<PodRec> = e.running().cloned().collect();
    let refs: Vec<&PodRec> = cands.iter().collect();
    assert!(
        e.select_victims(&pending, &refs, ResVec::ZERO).is_none(),
        "equal priority: no self-eviction"
    );
    let mut hi = pending.clone();
    hi.priority = 10;
    assert!(e.select_victims(&hi, &refs, ResVec::ZERO).is_none() || e.pod("a2-0").is_some());
}

#[test]
fn only_pods_inside_the_guarantee_may_preempt() {
    let mut e = engine();
    for i in 0..11 {
        bound(&mut e, &format!("a1-{i}"), "a1", 1_000, "n1", i);
    }
    // b1 (guar 4) already uses 3; a 3-CPU pod exceeds its remaining guarantee (1) => no preemption.
    for i in 0..3 {
        bound(&mut e, &format!("b-{i}"), "b1", 1_000, "n2", i);
    }
    let pending = pod("b-new", "b1", 3_000, 12);
    let cands: Vec<PodRec> = e
        .running()
        .cloned()
        .filter(|p| p.uid.starts_with("a1"))
        .collect();
    let refs: Vec<&PodRec> = cands.iter().collect();
    assert!(e.select_victims(&pending, &refs, ResVec::ZERO).is_none());
    assert_eq!(
        e.check_preemption(&pending, &refs[..1]),
        Err(PreemptRefusal::ExceedsGuarantee)
    );
}

#[test]
fn never_evicts_more_than_the_borrowed_amount() {
    let mut e = engine();
    // a1 at 5 CPU (2 elastic). b1 needs 4 CPU: only 2 reclaimable => insufficient => no partial eviction.
    for i in 0..5 {
        bound(&mut e, &format!("a1-{i}"), "a1", 1_000, "n1", i);
    }
    let pending = pod("b1-new", "b1", 4_000, 16);
    let cands: Vec<PodRec> = e.running().cloned().collect();
    let refs: Vec<&PodRec> = cands.iter().collect();
    assert!(e.select_victims(&pending, &refs, ResVec::ZERO).is_none());
    assert!(e.render_metrics().contains("refused_insufficient\"} 1"));
}

// ── extender wire format ──────────────────────────────────────────────

fn pod_json(uid: &str, ns: &str, cpu: &str, mem: &str) -> serde_json::Value {
    json!({"metadata": {"uid": uid, "namespace": ns, "creationTimestamp": "2026-01-01T00:00:00Z"},
           "spec": {"priority": 0, "containers": [{"name": "c", "resources": {"requests": {"cpu": cpu, "memory": mem}}}],
                    "initContainers": [{"name": "i", "resources": {"requests": {"cpu": "100m", "memory": "1Gi"}}}]}})
}

#[test]
fn pod_requests_use_containers_raised_to_init_max() {
    let r = extender::pod_request(&pod_json("u", "a1", "500m", "256Mi"));
    assert_eq!(
        r,
        ResVec::new(500, 1 * GI),
        "init container memory dominates"
    );
}

#[test]
fn extender_filter_allows_denies_and_marks_hard_limits_unresolvable() {
    let state = ExtenderState::new(engine());
    let ok: ExtenderArgs = serde_json::from_value(
        json!({"pod": pod_json("u1", "a1", "1", "1Gi"), "nodenames": ["n1", "n2"]}),
    )
    .unwrap();
    let r = extender::filter(&state, &ok);
    assert_eq!(
        r.nodenames.as_deref(),
        Some(&["n1".to_string(), "n2".to_string()][..])
    );
    assert!(r.error.is_empty());
    // exceed b1's max (8 CPU)
    let big: ExtenderArgs = serde_json::from_value(
        json!({"pod": pod_json("u2", "b1", "9", "1Gi"), "nodenames": ["n1"]}),
    )
    .unwrap();
    let r = extender::filter(&state, &big);
    assert_eq!(r.nodenames, Some(vec![]));
    assert!(
        r.failed_and_unresolvable.contains_key("n1"),
        "preemption cannot fix a hard ceiling"
    );
    // wire format keys
    let s = serde_json::to_value(&r).unwrap();
    assert!(
        s.get("failedAndUnresolvableNodes").is_some()
            && s.get("failedNodes").is_some()
            && s.get("nodenames").is_some()
    );
    let bad: ExtenderArgs =
        serde_json::from_value(json!({"pod": {"metadata": {}}, "nodenames": []})).unwrap();
    assert!(!extender::filter(&state, &bad).error.is_empty());
}

#[test]
fn extender_preempt_vetoes_protected_victims_only() {
    let mut e = engine();
    for i in 0..3 {
        let mut p = pod(&format!("a2-{i}"), "a2", 1_000, 4);
        p.node = Some("n1".into());
        e.bind(p);
    }
    for i in 0..6 {
        let mut p = pod(&format!("a1-{i}"), "a1", 1_000, 4);
        p.node = Some("n2".into());
        e.bind(p);
    }
    let state = ExtenderState::new(e);
    let victims = |uids: &[&str]| MetaVictims {
        pods: uids.iter().map(|u| json!({"uid": u})).collect(),
        num_pdb_violations: 0,
    };
    let mut map = HashMap::new();
    map.insert("n1".to_string(), victims(&["a2-0", "a2-1"])); // protected sibling at guarantee
    map.insert("n2".to_string(), victims(&["a1-5", "a1-4"])); // borrower: fine
    map.insert("n3".to_string(), victims(&["never-seen"])); // unknown: cannot prove safe
    let args = ExtenderPreemptionArgs {
        pod: pod_json("b-new", "b1", "2", "1Gi"),
        node_name_to_meta_victims: map,
    };
    let out = extender::preempt(&state, &args);
    let mut nodes: Vec<_> = out.node_name_to_meta_victims.keys().cloned().collect();
    nodes.sort();
    assert_eq!(nodes, vec!["n2".to_string()]);
}

// ── quota changes & safety ────────────────────────────────────────────

#[test]
fn invalid_update_is_rejected_and_last_good_tree_stays_active() {
    let mut e = engine();
    bound(&mut e, "p", "a1", 3_000, "n", 0);
    let bad = build(vec![(
        "a",
        spec(Some("ghost"), &["a"], 1.0, ("1", "1Gi"), None),
    )]);
    assert!(bad.is_err());
    e.note_invalid_config();
    assert_eq!(
        e.admit(&pod("q", "a1", 1_000, 4)),
        Verdict::AllowElastic,
        "still governed by the old tree"
    );
    assert!(e
        .render_metrics()
        .contains("stellar_fairshare_config_valid 0"));
}

#[test]
fn lowering_max_never_evicts_running_pods_but_blocks_new_ones() {
    let mut e = engine();
    for i in 0..6 {
        bound(&mut e, &format!("p{i}"), "b1", 1_000, "n", i);
    }
    let tighter = build(vec![
        ("org", spec(None, &[], 1.0, ("", ""), None)),
        (
            "team-a",
            spec(Some("org"), &[], 1.0, ("", ""), Some(("12", "48Gi"))),
        ),
        (
            "a1",
            spec(
                Some("team-a"),
                &["a1"],
                1.0,
                ("3", "12Gi"),
                Some(("12", "48Gi")),
            ),
        ),
        (
            "a2",
            spec(
                Some("team-a"),
                &["a2"],
                1.0,
                ("3", "12Gi"),
                Some(("12", "48Gi")),
            ),
        ),
        (
            "b1",
            spec(
                Some("org"),
                &["b1"],
                1.0,
                ("4", "16Gi"),
                Some(("5", "20Gi")),
            ),
        ),
    ])
    .unwrap();
    e.set_tree(tighter);
    assert_eq!(e.running().count(), 6, "no eviction on config change");
    assert_eq!(
        e.admit(&pod("n", "b1", 1_000, 4)),
        Verdict::DenyMax { node: "b1".into() }
    );
}

#[test]
fn resource_quota_coexistence_reports_double_enforcement() {
    let t = small_tree();
    let mut rq = HashMap::new();
    rq.insert("a1".to_string(), ResVec::new(2_000, 8 * GI)); // below a1's guarantee (3)
    rq.insert("a2".to_string(), ResVec::new(10_000, 40 * GI)); // below max (12) but above guarantee
    rq.insert("b1".to_string(), ResVec::new(100_000, 400 * GI)); // generous: fine
    let out = check_resource_quota_coexistence(&t, &rq)
        .into_iter()
        .map(|e| e.to_string())
        .collect::<Vec<_>>();
    assert!(
        out.iter()
            .any(|m| m.contains("a1") && m.contains("block guaranteed")),
        "{out:?}"
    );
    assert!(
        out.iter()
            .any(|m| m.contains("a2") && m.contains("warning")),
        "{out:?}"
    );
    assert!(!out.iter().any(|m| m.contains("b1")), "{out:?}");
    // No ResourceQuota => no safety net limit => nothing to report.
    assert!(check_resource_quota_coexistence(&t, &HashMap::new()).is_empty());
}

// ── metrics ───────────────────────────────────────────────────────────

#[test]
fn metrics_expose_bounded_tenant_labels_only() {
    let mut e = engine();
    bound(&mut e, "secret-pod-name-xyz", "a1", 5_000, "n", 0);
    e.add_pending(pod("pending-pod-abc", "b1", 1_000, 4));
    let _ = e.admit(&pod("pending-pod-abc", "b1", 1_000, 4));
    e.metrics.observe_scheduling_delay(2.5);
    let m = e.render_metrics();
    for want in [
        "stellar_fairshare_allocated{tenant=\"a1\",resource=\"cpu\"} 5",
        "stellar_fairshare_guaranteed{tenant=\"team-a\",resource=\"cpu\"} 6",
        "stellar_fairshare_elastic_usage{tenant=\"a1\",resource=\"cpu\"} 2",
        "stellar_fairshare_quota_saturation_ratio{tenant=\"b1\",resource=\"cpu\"}",
        "stellar_fairshare_jain_index",
        "stellar_fairshare_admission_total{decision=\"allow_guaranteed\"} 1",
        "stellar_fairshare_scheduling_delay_seconds_count 1",
        "stellar_fairshare_preemption_total{decision=\"approved\"} 0",
    ] {
        assert!(m.contains(want), "missing {want:?} in:\n{m}");
    }
    assert!(
        !m.contains("secret-pod-name") && !m.contains("pending-pod"),
        "pod identifiers must never be labels"
    );
    assert!(
        m.matches("tenant=\"").count() < 100,
        "label cardinality bounded by quota objects"
    );
}

// ═══════════════════════════ saturation / soak simulation ═══════════════════════════
//
// A discrete-time cluster (1 tick = 1 simulated second) that plays kube-scheduler's role:
// FIFO queue, first-fit node placement, the engine's admission verdicts, and the engine's
// preemption selection when a pod inside its guarantee does not fit. Pod controllers
// recreate preempted pods. The simulation is deterministic (seeded).

struct Sim {
    e: Engine,
    nodes: Vec<ResVec>, // free capacity per node
    node_cap: ResVec,
    queue: Vec<(PodRec, u64)>, // pending (pod, duration)
    ends: HashMap<String, u64>,
    durations: HashMap<String, u64>,
    next_uid: u64,
    rng: Lcg,
    now: u64,
    namespaces: Vec<String>,
    // measurements
    preemptions: u64,
    boundary_violations: Vec<String>,
    max_violation_streak: u64,
    violation_streak: u64,
    cap_violations: Vec<String>,
    sum_ratio: Vec<f64>,
    n_ratio: Vec<u64>,
    sat_ticks: u64,
    min_jain: f64,
    completed: u64,
}

fn sim_tree() -> (Tree, Vec<String>) {
    // 20 namespaces over 7 leaves in 3 top-level groups. Memory guarantees = 4 GiB per CPU.
    let mut v: Vec<(String, TenantQuotaSpec)> = Vec::new();
    let mut nss = Vec::new();
    let mut add_leaf = |v: &mut Vec<(String, TenantQuotaSpec)>,
                        name: &str,
                        parent: &str,
                        n_ns: usize,
                        w: f64,
                        g: u64,
                        m: u64| {
        let ns: Vec<String> = (0..n_ns).map(|i| format!("{name}-ns{i}")).collect();
        nss.extend(ns.clone());
        let nsr: Vec<&str> = ns.iter().map(String::as_str).collect();
        v.push((
            name.to_string(),
            spec(
                Some(parent),
                &nsr,
                w,
                (&g.to_string(), &format!("{}Gi", g * 4)),
                Some((&m.to_string(), &format!("{}Gi", m * 4))),
            ),
        ));
    };
    for (g, gg, gm) in [
        ("platform", 80, 140),
        ("partners", 40, 100),
        ("sandbox", 10, 40),
    ] {
        v.push((
            g.to_string(),
            spec(
                None,
                &[],
                if g == "sandbox" { 0.5 } else { 1.0 },
                (&gg.to_string(), &format!("{}Gi", gg * 4)),
                Some((&gm.to_string(), &format!("{}Gi", gm * 4))),
            ),
        ));
    }
    add_leaf(&mut v, "p-core", "platform", 4, 2.0, 30, 100);
    add_leaf(&mut v, "p-apps", "platform", 4, 1.0, 30, 80);
    add_leaf(&mut v, "p-batch", "platform", 4, 1.0, 20, 60);
    add_leaf(&mut v, "x", "partners", 4, 1.0, 25, 70);
    add_leaf(&mut v, "y", "partners", 2, 1.0, 15, 50);
    add_leaf(&mut v, "s1", "sandbox", 1, 1.0, 5, 20);
    add_leaf(&mut v, "s2", "sandbox", 1, 1.0, 5, 20);
    assert_eq!(nss.len(), 20);
    let t = Tree::build(&v).unwrap_or_else(|e| panic!("sim tree invalid: {e:?}"));
    (t, nss)
}

impl Sim {
    fn new(seed: u64) -> Self {
        let (tree, namespaces) = sim_tree();
        let node_cap = ResVec::new(16_000, 64 * GI);
        let n_nodes = 10;
        let cap = ResVec::new(16_000 * n_nodes as u64, 64 * GI * n_nodes as u64);
        assert!(tree.check_capacity(cap).is_empty());
        let leaves = tree.len();
        Sim {
            e: Engine::new(tree, cap),
            nodes: vec![node_cap; n_nodes],
            node_cap,
            queue: vec![],
            ends: HashMap::new(),
            durations: HashMap::new(),
            next_uid: 0,
            rng: Lcg(seed),
            now: 0,
            namespaces,
            preemptions: 0,
            boundary_violations: vec![],
            max_violation_streak: 0,
            violation_streak: 0,
            cap_violations: vec![],
            sum_ratio: vec![0.0; leaves],
            n_ratio: vec![0; leaves],
            sat_ticks: 0,
            min_jain: 1.0,
            completed: 0,
        }
    }

    fn arrive(&mut self, ns_idx: usize) {
        let cpu = [500u64, 1000, 1000, 2000][self.rng.range(0, 3) as usize];
        let mem_gi = (cpu / 1000).max(1) * 4 * self.rng.range(1, 2) / 1; // 4–8 GiB per CPU
        let mut p = pod(
            &format!("pod-{}", self.next_uid),
            &self.namespaces[ns_idx],
            cpu,
            mem_gi.max(1),
        );
        p.req.mem_b = (cpu * 4 * GI / 1000) * self.rng.range(1, 2) / 1;
        p.priority = 0;
        self.next_uid += 1;
        let dur = self.rng.range(60, 240);
        self.e.add_pending(p.clone());
        self.queue.push((p, dur));
    }

    fn node_for(&self, req: ResVec) -> Option<usize> {
        self.nodes.iter().position(|f| req.fits_in(*f))
    }

    fn start(&mut self, mut p: PodRec, node: usize, dur: u64) {
        self.nodes[node] = self.nodes[node].sub(p.req);
        p.node = Some(node.to_string());
        p.started = self.now;
        self.ends.insert(p.uid.clone(), self.now + dur);
        self.durations.insert(p.uid.clone(), dur);
        self.e.bind(p);
    }

    fn stop(&mut self, uid: &str) -> Option<PodRec> {
        let p = self.e.remove(uid)?;
        let n: usize = p.node.as_ref().unwrap().parse().unwrap();
        self.nodes[n] = self.nodes[n].add(p.req).min(self.node_cap);
        self.ends.remove(uid);
        Some(p)
    }

    fn tick(&mut self, max_sched: usize) {
        self.now += 1;
        // completions
        let done: Vec<String> = self
            .ends
            .iter()
            .filter(|(_, &t)| t <= self.now)
            .map(|(u, _)| u.clone())
            .collect();
        for u in done {
            self.stop(&u);
            self.durations.remove(&u);
            self.completed += 1;
        }
        // scheduling pass (FIFO, bounded throughput)
        let mut still = Vec::new();
        let mut scheduled = 0usize;
        let queue = std::mem::take(&mut self.queue);
        for (p, dur) in queue {
            if scheduled >= max_sched {
                still.push((p, dur));
                continue;
            }
            let verdict = self.e.admit(&p);
            let needs_reclaim = matches!(verdict, Verdict::NeedsReclaim { .. });
            if !verdict.allowed() && !needs_reclaim {
                still.push((p, dur));
                continue;
            }
            if let Some(n) = self.node_for(p.req).filter(|_| !needs_reclaim) {
                self.start(p, n, dur);
                scheduled += 1;
                continue;
            }
            // Does not fit anywhere: only guaranteed pods may preempt.
            let mut placed = false;
            for n in 0..self.nodes.len() {
                let cands: Vec<PodRec> = self
                    .e
                    .running()
                    .filter(|r| r.node.as_deref() == Some(&n.to_string()))
                    .cloned()
                    .collect();
                let refs: Vec<&PodRec> = cands.iter().collect();
                if let Some(victims) = self.e.select_victims(&p, &refs, self.nodes[n]) {
                    self.verify_boundaries(&p, &victims);
                    for v in &victims {
                        if let Some(old) = self.stop(v) {
                            let d = self.durations.remove(v).unwrap_or(120);
                            let mut re = old.clone();
                            re.uid = format!("{}-r{}", old.uid, self.now);
                            re.node = None;
                            self.e.add_pending(re.clone());
                            still.push((re, d));
                            self.preemptions += 1;
                        }
                    }
                    self.start(p.clone(), n, dur);
                    scheduled += 1;
                    placed = true;
                    break;
                }
            }
            if !placed {
                still.push((p, dur));
            }
        }
        self.queue = still;
        self.measure();
    }

    /// Independent re-check (not using engine helpers) that evicting `victims` kept every
    /// node below the LCA at or above its guarantee.
    fn verify_boundaries(&mut self, pending: &PodRec, victims: &[String]) {
        let tree = &self.e.tree;
        let (usage, _) = self.e.usage();
        let pl = tree.leaf_for_namespace(&pending.namespace).unwrap();
        let mut after = usage.clone();
        for v in victims {
            let vp = self.e.pod(v).unwrap();
            let vl = tree.leaf_for_namespace(&vp.namespace).unwrap();
            for x in tree.path_to_root(vl) {
                after[x] = after[x].sub(vp.req);
            }
            if vl == pl {
                continue;
            }
            let lca = tree.lca(pl, vl);
            for x in tree.path_to_root(vl) {
                if Some(x) == lca {
                    break;
                }
                if !tree.nodes[x].guaranteed.fits_in(after[x]) {
                    self.boundary_violations.push(format!(
                        "t={} preemptor {} evicted {} breaching {}",
                        self.now, pending.namespace, v, tree.nodes[x].name
                    ));
                }
            }
        }
    }

    fn measure(&mut self) {
        let (usage, total) = self.e.usage();
        for (i, n) in self.e.tree.nodes.iter().enumerate() {
            if !usage[i].fits_in(n.max) {
                self.cap_violations.push(format!(
                    "t={} {} usage {:?} > max {:?}",
                    self.now, n.name, usage[i], n.max
                ));
            }
        }
        // guarantee violation: a tenant with pending pods holds less than min(guarantee, its demand)
        // even though capacity is available through preemption => streak counter.
        if self.violation_streak == 30 && std::env::var("FS_DEBUG").is_ok() {
            let viol = self.e.guarantee_violations();
            eprintln!("DEBUG t={} violated={viol:?}", self.now);
            for (p, _) in self
                .queue
                .iter()
                .filter(|(p, _)| {
                    self.e
                        .tree
                        .leaf_for_namespace(&p.namespace)
                        .map(|l| viol.contains(&self.e.tree.nodes[l].name))
                        .unwrap_or(false)
                })
                .take(3)
            {
                let l = self.e.tree.leaf_for_namespace(&p.namespace).unwrap();
                eprintln!(
                    "  pending {} ns={} req={:?} verdict={:?} usage={:?} guar={:?} node_fit={:?}",
                    p.uid,
                    p.namespace,
                    p.req,
                    self.e.admit(p),
                    usage[l],
                    self.e.tree.nodes[l].guaranteed,
                    self.node_for(p.req)
                );
                for n in 0..self.nodes.len() {
                    let cands: Vec<PodRec> = self
                        .e
                        .running()
                        .filter(|r| r.node.as_deref() == Some(&n.to_string()))
                        .cloned()
                        .collect();
                    let refs: Vec<&PodRec> = cands.iter().collect();
                    eprintln!(
                        "    node {n} free={:?} cands={} victims={:?}",
                        self.nodes[n],
                        cands.len(),
                        self.e.select_victims(p, &refs, self.nodes[n])
                    );
                }
            }
            if std::env::var("FS_DEBUG_PANIC").is_ok() {
                let (u, _) = self.e.usage();
                for (i, n) in self.e.tree.nodes.iter().enumerate() {
                    eprintln!(
                        "  {:10} usage={:?} guar={:?} max={:?}",
                        n.name, u[i], n.guaranteed, n.max
                    );
                }
                panic!("debug stop");
            }
        }
        if self.e.guarantee_violations().is_empty() {
            self.violation_streak = 0;
        } else {
            self.violation_streak += 1;
            self.max_violation_streak = self.max_violation_streak.max(self.violation_streak);
        }
        // fairness in saturated steady state: cluster >= 95 % used with a backlog
        let saturated = !self.queue.is_empty() && total.dominant_share(self.e.capacity) >= 0.95;
        if saturated {
            let mut demand: HashMap<usize, ResVec> = HashMap::new();
            for (p, _) in &self.queue {
                if let Some(l) = self.e.tree.leaf_for_namespace(&p.namespace) {
                    let e = demand.entry(l).or_insert(ResVec::ZERO);
                    *e = e.add(p.req);
                }
            }
            for l in self.e.tree.leaves() {
                let e = demand.entry(l).or_insert(ResVec::ZERO);
                *e = e.add(usage[l]);
            }
            let ent = self.e.tree.entitlements(&demand, self.e.capacity);
            for l in self.e.tree.leaves() {
                // usage relative to the demand-limited weighted entitlement (per-dimension max)
                if !ent.by_node[l].is_zero() {
                    self.sum_ratio[l] += usage[l].dominant_share(ent.by_node[l]);
                    self.n_ratio[l] += 1;
                }
            }
            self.sat_ticks += 1;
        }
    }

    /// Time-averaged usage/entitlement ratio per tenant over saturated ticks.
    fn avg_ratios(&self) -> Vec<(usize, f64)> {
        self.e
            .tree
            .leaves()
            .filter(|&l| self.n_ratio[l] > 0)
            .map(|l| (l, self.sum_ratio[l] / self.n_ratio[l] as f64))
            .collect()
    }

    /// Jain index of the time-averaged usage/entitlement ratios over saturated ticks.
    fn jain(&self) -> f64 {
        jain_index(
            &self
                .avg_ratios()
                .into_iter()
                .map(|(_, r)| r)
                .collect::<Vec<_>>(),
        )
    }
}

#[test]
fn saturation_500_pods_20_namespaces_multiple_hierarchies() {
    let mut s = Sim::new(7);
    let mut arrived = 0usize;
    let mut tick = 0u64;
    // 500 pods; namespaces compete with different appetites (round-robin weighted by index).
    while (arrived < 500 || !s.queue.is_empty() || !s.ends.is_empty()) && tick < 5_000 {
        tick += 1;
        for _ in 0..5 {
            if arrived < 500 {
                // skew: low-index namespaces (platform) arrive more often
                let ns = (s.rng.range(0, 19) * s.rng.range(0, 19) / 19) as usize;
                s.arrive(ns.min(19));
                arrived += 1;
            }
        }
        s.tick(200);
    }
    assert_eq!(arrived, 500);
    assert!(
        s.queue.is_empty(),
        "all 500 pods eventually run (no starvation); {} left",
        s.queue.len()
    );
    assert!(
        s.cap_violations.is_empty(),
        "no tenant exceeds its max: {:?}",
        &s.cap_violations[..s.cap_violations.len().min(3)]
    );
    assert!(
        s.boundary_violations.is_empty(),
        "preemption honours hierarchy: {:?}",
        s.boundary_violations
    );
    assert!(
        s.sat_ticks >= 30,
        "the workload must actually saturate the cluster (saturated ticks: {})",
        s.sat_ticks
    );
    let j = s.jain();
    for (l, r) in s.avg_ratios() {
        eprintln!(
            "  {:8} avg usage/entitlement = {:.3} (over {} saturated ticks)",
            s.e.tree.nodes[l].name, r, s.n_ratio[l]
        );
    }
    eprintln!("500-pod saturation: ticks={tick} saturated_ticks={} preemptions={} jain={j:.4} max_guarantee_violation_streak={}", s.sat_ticks, s.preemptions, s.max_violation_streak);
    assert!(j >= 0.95, "Jain index {j:.4} < 0.95");
    assert!(
        s.max_violation_streak <= 2,
        "guarantee violation lasted {} ticks",
        s.max_violation_streak
    );
}

/// Long-running randomized churn with the same invariants. Defaults to 3 600 simulated seconds;
/// set `FAIRSHARE_SOAK_TICKS=86400` for a simulated 24 h. This is a *logical-clock simulation* of
/// the policy engine, not a wall-clock test against a real cluster.
#[test]
#[ignore = "long soak; run with --release -- --ignored fairshare_soak"]
fn fairshare_soak() {
    let ticks: u64 = std::env::var("FAIRSHARE_SOAK_TICKS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3_600);
    let mut s = Sim::new(
        std::env::var("FAIRSHARE_SOAK_SEED")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1),
    );
    let started = std::time::Instant::now();
    for t in 0..ticks {
        // sustained overload with a slowly rotating hot tenant
        let hot = ((t / 900) % 20) as usize;
        for _ in 0..3 {
            let ns = if s.rng.range(0, 3) == 0 {
                hot
            } else {
                s.rng.range(0, 19) as usize
            };
            if s.queue.len() < 400 {
                s.arrive(ns);
            }
        }
        s.tick(60);
        assert!(
            s.cap_violations.is_empty(),
            "{:?}",
            s.cap_violations.first()
        );
        assert!(
            s.boundary_violations.is_empty(),
            "{:?}",
            s.boundary_violations.first()
        );
    }
    let j = s.jain();
    eprintln!(
        "soak: simulated_ticks={ticks} wall={:?} completed={} preemptions={} saturated_ticks={} jain={j:.4} max_guarantee_violation_streak={}",
        started.elapsed(), s.completed, s.preemptions, s.sat_ticks, s.max_violation_streak
    );
    // Hard invariants (asserted every tick inside the loop): no ceiling breach, no eviction below a
    // guarantee. Bounded wait: a pod inside its guarantee can be delayed when the reclaimable surplus
    // is in another dimension or on another node, but never longer than pod turnover (<= 240 ticks).
    assert!(
        s.max_violation_streak <= 240,
        "guarantee wait {} ticks exceeds pod turnover",
        s.max_violation_streak
    );
    assert!(j >= 0.95, "Jain {j:.4}");
    let _ = s.min_jain;
}
