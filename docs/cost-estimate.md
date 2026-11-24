# Cost Estimate

This page provides reference cost estimates for running Stellar-K8s on AWS.
All figures use **on-demand pricing in `us-east-1`** as a baseline; actual costs
vary by region, reserved/spot discounts, and workload profile.

> For live pricing see the [AWS Pricing Calculator](https://calculator.aws).
> For cost-optimization strategies (right-sizing, spot instances, budget alerts)
> see the [Cost Optimization Guide](cost-optimization-guide.md).

---

## Programmatic estimate

The tables on this page are backed by a machine-readable pricing model in
`config/cost-estimate.yaml`. The
`make cost-estimate` target reads that model and prints the estimate, so the
documentation and the tooling always agree:

```bash
make cost-estimate                              # every profile
make cost-estimate-check                        # validate the model only
python3 scripts/cost-estimate.py --profile mainnet-ha
python3 scripts/cost-estimate.py --discount reserved1y
python3 scripts/cost-estimate.py --format json  # machine-readable output
```

`make cost-estimate` prints the same compute and storage split as the tables
below, plus the profile total. The reference tables are rounded; the tool
prints two decimals. The `--discount` flag applies one of the savings
strategies listed in the model (`reserved1y`, `savingsPlan`, `spot`) to the
compute portion of the estimate.

To model a different deployment, copy the model, edit the component list, and
point the script at it:

```bash
python3 scripts/cost-estimate.py --config config/cost-estimate.yaml
```

The script exits non-zero when the model is invalid (missing component, count
below one, negative price, or an unknown profile or discount), so it can be
used as a configuration gate in CI.

---

## Node Type Reference

| Node Type    | Typical Instance | vCPU | RAM   | Storage (EBS gp3) |
|--------------|-----------------|------|-------|-------------------|
| Validator    | `m5.xlarge`     | 4    | 16 GB | 500 GB            |
| Horizon API  | `m5.large`      | 2    | 8 GB  | 200 GB            |
| Soroban RPC  | `c5.2xlarge`    | 8    | 16 GB | 300 GB            |
| Operator     | `m5.large`      | 2    | 8 GB  | 20 GB             |

---

## Monthly Estimate — Testnet (Single Node per Type)

| Component        | Instance      | Compute/mo | Storage/mo | Total/mo |
|-----------------|---------------|-----------|-----------|---------|
| Validator        | `m5.xlarge`   | ~$140      | ~$50       | ~$190   |
| Horizon API      | `m5.large`    | ~$70       | ~$20       | ~$90    |
| Soroban RPC      | `c5.2xlarge`  | ~$248      | ~$30       | ~$278   |
| Operator         | `m5.large`    | ~$70       | ~$2        | ~$72    |
| **Testnet Total**|               | **~$528**  | **~$102**  | **~$630** |

Storage cost uses EBS gp3 at **$0.08/GB-month**.
Compute uses on-demand rates: `m5.large` $0.096/hr, `m5.xlarge` $0.192/hr,
`c5.2xlarge` $0.340/hr (730 hrs/month).

---

## Monthly Estimate — Mainnet HA (Production)

A production mainnet deployment typically runs 3 validators, 2 Horizon replicas,
2 Soroban RPC nodes, and 2 operator replicas for high availability.

| Component           | Count | Instance      | Compute/mo | Storage/mo | Total/mo  |
|--------------------|-------|---------------|-----------|-----------|----------|
| Validators          | 3     | `m5.xlarge`   | ~$421      | ~$150      | ~$571    |
| Horizon API         | 2     | `m5.large`    | ~$140      | ~$40       | ~$180    |
| Soroban RPC         | 2     | `c5.2xlarge`  | ~$497      | ~$60       | ~$557    |
| Operator            | 2     | `m5.large`    | ~$140      | ~$4        | ~$144    |
| PostgreSQL (Horizon)| 1     | `db.m5.large` | ~$125      | ~$25       | ~$150    |
| **Mainnet HA Total**|       |               | **~$1,323**| **~$279**  | **~$1,602** |

PostgreSQL uses RDS `db.m5.large` on-demand (~$0.171/hr) with 200 GB gp3 storage.

---

## Cost Reduction Options

| Strategy                  | Typical Saving | Notes                                      |
|--------------------------|---------------|--------------------------------------------|
| 1-year Reserved Instances | 30–40%        | Validators and Horizon are good candidates |
| Spot Instances            | 60–90%        | Indexers and analytics only (not validators) |
| EBS gp3 vs gp2            | ~20%          | gp3 is cheaper and faster; use by default  |
| Right-sizing              | 10–30%        | Use operator recommendations after 14 days |
| Savings Plans             | 20–30%        | Compute Savings Plans cover EC2 + Fargate  |

See [Cost Optimization Guide](cost-optimization-guide.md) for configuration details.

---

## Local NVMe vs EBS

For validators with heavy ledger I/O, local NVMe storage eliminates EBS costs
entirely and reduces sync lag. See the
[High-Performance Local Storage](../README.md#-high-performance-local-storage-nvme)
section in the README for benchmark data and configuration.

---

## Assumptions

- Region: `us-east-1` on-demand pricing.
- Storage: EBS gp3 at $0.08/GB-month; no snapshot costs included.
- Network: intra-region traffic excluded; inter-region egress ~$0.02/GB.
- Prices are approximate and subject to change; verify at [calculator.aws](https://calculator.aws).
