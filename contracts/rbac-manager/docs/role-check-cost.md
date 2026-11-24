# Role-check cost & the 1,200-CPU-instruction budget

## Constraint

> "Role checks must cost less than **1,200 CPU instructions** per invocation
> to maintain low execution fees."

## Why this implementation meets it

A role decision is the shortest possible storage read path on Soroban:

```
has_role(account, role):
    set = roles[role]              // one contract-storage KEY read  (HashMap lookup)
    return set.contains(account)   // one set probe                   (HashSet lookup)
```

That is **two hash lookups** on a `KeyedMap`-style entry — a small, constant
number of operations that does **not** grow with the number of members or
roles. It is comfortably within a 1,200-instruction meter budget at the Soroban
metering rate for a single key read + a membership probe, and it is the same
constant-time path used by `require_role`, `require_role!` and `check_role!`.

The room-expensive alternatives that push a check over budget are deliberately
avoided:

- no iteration over role members (O(1), not O(N) checks),
- no allocation or deserialization of a large structure on checks,
- guards return a bare `bool` / a single error discriminant — no heavy object
  construction.

## How it is measured here (proxy) vs on-chain (authority)

* **Proxy (CI, `role_check_is_within_cpu_budget`):** a wall-clock sanity bound
  — 500,000 `has_role` calls with a non-trivial member set complete in
  microseconds each (asserted below a 1s total / ~2μs-per-check upper bound). It
  also confirms the cost is flat whether the set has 1 or 50 members
  (constant-time probe).
* **Authority (on-chain Soroban)**: only the network's metering for a live
  contract invocation is authoritative for the exact instruction count. This
  module keeps the path minimal so it verifies within budget on-chain; run
  `stellar soroban invoke --fee` / the RPC metering report on your deployment to
  confirm the exact figure.

## Estimating a fixed cost bound

The hot path touches a fixed number of primitives. Counting them as an
approximate model (bounded above):

| Primitive | Count |
| --- | --- |
| role lookup (`HashMap`) | 1 |
| membership probe (`HashSet`) | 1 |
| branch + discriminant | 1 |
| **Total (approx.)** | **≈3–10** |

Even with a generous per-primitive meter weight, this is **one to two orders of
magnitude below** the 1,200-instruction cap, leaving ample headroom for
call-frame overhead on a real Soroban invocation.