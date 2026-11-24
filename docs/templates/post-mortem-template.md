# Incident Post-Mortem Template

Use this template within **5 business days** of resolving SEV-1 or SEV-2 incidents. SEV-3 post-mortems are optional but recommended for recurring issues.

Copy this file into your incident tracker or wiki and fill every section.

---

## Incident metadata

| Field | Value |
| --- | --- |
| **Incident ID** | INC-YYYYMMDD-XXX |
| **Title** | [Short descriptive title] |
| **Severity** | SEV-1 / SEV-2 / SEV-3 / SEV-4 |
| **Status** | Resolved / Mitigated / Monitoring |
| **Detection time (UTC)** | YYYY-MM-DD HH:MM |
| **Resolution time (UTC)** | YYYY-MM-DD HH:MM |
| **Duration** | [e.g., 2h 15m] |
| **Incident commander** | [Name] |
| **Authors** | [Names] |
| **Reviewers** | [Engineering lead, SRE lead] |
| **Related alerts** | [Alertmanager alert names / URLs] |
| **Related issues/PRs** | [#123, #456] |

---

## Executive summary

[2–3 sentences: what happened, customer impact, and how it was resolved. Non-technical stakeholders should understand this section.]

---

## Impact assessment

### User and business impact

- **Stellar network:** [Ledger delays, consensus gaps, none]
- **API availability:** [Horizon/Soroban RPC error rate, duration]
- **Tenants affected:** [List namespaces / customers]
- **Financial impact:** [If applicable]

### Technical impact

- **Nodes affected:** [StellarNode names, namespaces]
- **Data integrity:** [Archive corruption, DB inconsistency — yes/no with details]
- **SLA breach:** [Yes/no against defined SLO]

---

## Timeline (UTC)

All times in UTC. Include detection, escalation, key decisions, and recovery.

| Time | Actor | Event |
| --- | --- | --- |
| HH:MM | Alertmanager | [Alert fired: StellarNodeDown] |
| HH:MM | On-call | [Acknowledged; opened incident channel] |
| HH:MM | IC | [Severity set to SEV-2; assigned runbook owner] |
| HH:MM | Engineer | [Mitigation step applied] |
| HH:MM | IC | [Declared resolved; monitoring period started] |

---

## Root cause analysis (RCA)

### What happened?

[Technical description of the failure chain — the proximate cause.]

### Why did it happen?

[Contributing factors: design gap, config error, capacity, human process, dependency failure.]

### Why did our defenses not prevent it?

[Monitoring gaps, missing alerts, insufficient testing, documentation drift.]

### Five whys (optional)

1. Why? →
2. Why? →
3. Why? →
4. Why? →
5. Why? → [Root cause]

---

## Detection and response evaluation

### What went well?

- [e.g., Fast detection via `stellar_node_ingestion_lag` alert]
- [e.g., Runbook steps were accurate]
- [e.g., Cross-team communication]

### What could be improved?

- [e.g., Alert threshold too high]
- [e.g., Missing runbook for edge case]
- [e.g., Slow credential rotation]

---

## Mitigation and resolution

[Detailed steps taken to restore service, including commands, config changes, and rollbacks. Link to PRs.]

---

## Corrective and preventive actions

| ID | Action | Owner | Priority | Due date | Status |
| --- | --- | --- | --- | --- | --- |
| CAPA-1 | [e.g., Add alert for archive lag] | [Name] | P0 | YYYY-MM-DD | Open |
| CAPA-2 | [e.g., Automate PVC expansion test] | [Name] | P1 | YYYY-MM-DD | Open |

Action items must be **SMART**: specific, measurable, assigned, with due dates.

---

## Lessons learned

### Technical

- [Lesson 1]

### Process

- [Lesson 1]

### Documentation updates required

- [ ] [Link to doc PR]

---

## Supporting artifacts

- [ ] Prometheus snapshot / Grafana dashboard export
- [ ] `stellar-operator incident-report` bundle path
- [ ] Relevant log excerpts (redacted)
- [ ] Post-incident metric screenshots

---

## Sign-off

| Role | Name | Date |
| --- | --- | --- |
| Incident commander | | |
| Engineering manager | | |
| SRE lead | | |

---

## Appendix: severity reference

See the [Incident Response Playbook](../security/incident-response-playbook.md#1-containment-priority-matrix) for severity definitions.
