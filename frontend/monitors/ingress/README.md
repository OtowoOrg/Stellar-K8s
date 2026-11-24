# Stellar-K8s: Public Ingress & SSL/TLS Certificate Expiration Monitor

A real-time SSL/TLS health and certificate expiration tracking dashboard for Stellar-K8s cluster ingresses and Gateway API `HTTPRoute` endpoints.

## Context & Impact

Stellar-K8s clusters expose public endpoints for Horizon REST and Soroban RPC APIs across multiple environments (Mainnet, Testnet, Futurenet, Regional clusters). Expiration of SSL/TLS certificates causes instant disruption for wallets, indexers, RPC clients, and validators.

This monitoring dashboard provides:

1. **Searchable & Sortable Data Table**: Complete visibility into all exposed Ingress and HTTPRoute endpoints across clusters.
2. **Urgency Color-Coding**:
    - 🔴 **Critical / Expired (Red)**: `< 7 days` remaining or expired (`<= 0 days`).
    - 🟡 **Warning / Expiring Soon (Yellow / Amber)**: `< 30 days` remaining (7–30 days).
    - 🟢 **Healthy (Green)**: `> 30 days` remaining.
3. **Priority Default Sorting**: Automatically sorts certificates by `daysRemaining` ascending so expiring and expired certificates appear first at the top of the interface.
4. **Certificate Renewal Triggering**: One-click "Trigger Certificate Renewal" button with live state feedback (Idle ➔ Renewing ➔ Renewed) and batch emergency renewal for all critical routes.
5. **Detailed X.509 Diagnostics**: Side inspector drawer displaying Issuer, Subject Alternative Names (SANs), Serial Number, Signature Algorithm, Valid From/Until, Secret References, and Kubernetes annotations.
6. **High Performance**: Smooth 60fps search and sorting tested for fleets of 50+ exposed routes.

---

## Directory Structure

```
frontend/
├── components/
│   ├── cert_table.tsx            # Core reusable CertTable component
│   └── cert_table.test.ts        # Fleet & sorting unit tests
└── monitors/
    └── ingress/
        ├── index.html            # Vite HTML entry point
        ├── package.json          # Node scripts & dependencies
        ├── tsconfig.json         # TypeScript configuration
        ├── vite.config.ts        # Vite configuration with API proxy
        ├── README.md             # Documentation (this file)
        └── src/
            ├── App.tsx           # Dashboard application shell
            ├── main.tsx          # React bootstrap mount
            ├── types.ts          # Complete TypeScript types & interfaces
            ├── certUtils.ts      # Expiration calculation, urgency & sorting utilities
            ├── certUtils.test.ts # Unit tests for date formatting & filters
            ├── mockData.ts       # 50+ route realistic mock fleet
            ├── styles.css        # Dark theme styling with glowing status rings
            └── components/
                ├── MetricStrip.tsx   # Top KPI metrics strip
                ├── AlertBanner.tsx   # Critical alert banner
                └── DetailDrawer.tsx  # X.509 Inspector drawer
```

---

## Running Locally

From `frontend/monitors/ingress`:

```bash
# 1. Install dependencies
npm install

# 2. Start development server
npm run dev
```

Open the Vite URL printed by the command (default: `http://localhost:5175`).

### Data Sources

- **Mock Payload (50+ Routes Fleet)**: Default mode containing expired (-2d), critical (1d, 3d, 5d, 6d), warning (8d, 12d, 15d, 19d, 22d, 28d), and healthy (>30d) routes across Horizon, Soroban RPC, and Validator ingresses.
- **Cluster API**: Proxies to `/api/v1/certificates/ingress` on the running operator.

---

## Running Unit Tests

```bash
npm test
```

Executes test suites covering:

- Positive, zero, and negative (expired) days remaining calculations.
- Urgency tier classification (Red `< 7d`, Yellow `< 30d`, Green `> 30d`).
- Full-text multi-attribute search filtering (Hostname, Issuer, SANs, Namespaces).
- Column sorting and default expiry prioritization.
- 50+ routes fleet generation and sub-5ms search throughput.
- Certificate renewal simulation and state transitions.

---

## Production Build

```bash
npm run build
```
