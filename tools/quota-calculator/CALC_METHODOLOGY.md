# Quota Calculator — Calculation Methodology

This document explains the calculation methodology used by the `quota-calculator` tool. All cost formulas are anchored to Google Cloud Platform (GCP) sample rates (USD). The provided numbers are sample default values and should be updated to match current GCP pricing or your billing agreement.

- CPU & memory:
  - Total vCPU = replicas * vCPU per pod * (1 + bufferPercent).
  - Total memory (GB) = replicas * memory per pod * (1 + bufferPercent).
  - Node recommendation selects a node type (e.g. `n2-standard-4`) and computes how many nodes required to fit CPU and memory.

- Storage growth:
  - Compounded monthly growth: month_n = month_{n-1} * (1 + monthlyGrowthPercent).
  - Average monthly storage used is used to estimate ongoing monthly storage cost.

- Network (egress):
  - Monthly egress (GB) = rps * avgResponseKb * seconds_per_month / (1024*1024).
  - Peak throughput (approx, Mbps) = rps * avgResponseKb * 8 / 1024.

- Cost estimation (GCP anchors, sample defaults):
  - Compute cost = vCPU-hours * price_per_vCPU_hour + memory_GB-hours * price_per_GB_hour.
  - Storage cost = avg_GB * price_per_GB_month.
  - Network cost = egress_GB_month * price_per_GB_egress.

Notes:
- The tool assumes steady-state usage for compute (24/7) when computing vCPU and memory hours. For bursty workloads, scale vCPU-hours proportionally to utilization.
- Prices are configurable in the library; replace `DEFAULT_PRICES` with current GCP on-demand or committed rates as needed.
