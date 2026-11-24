# API Deprecation Timeline and Stakeholder Reporting

Epic: #1528 · Module: `src/api_gateway/deprecation_timeline.rs`

`build_report` turns the gateway's `VersioningConfig` (deprecated versions,
sunset versions, sunset dates) into a timeline. It covers every deprecated and
sunset version, so no API is left out.

## Adoption from runtime usage

Adoption comes from the gateway's own request telemetry (`RequestEvent`), not
from what consumers report about themselves. Each API key (or `anonymous`)
gets one of these statuses:

| Status       | Traffic seen in the window          |
|--------------|-------------------------------------|
| `migrated`   | successor version only              |
| `inProgress` | both deprecated and successor       |
| `notStarted` | deprecated version only             |

Request counts in the report are exact tallies of the events, so they match
the raw usage data.

## Reminders

`due_reminders` uses `ReminderPolicy` (default: 90, 60, 30, 14, 7 and 1 days
before sunset). Each `(api, interval)` pair fires once. The caller stores the
pairs it has already sent. After a missed run, the next run sends only the
tightest interval that has not been sent. `Reminder::message()` gives the text
body for Slack or email, listing the consumers that still use the deprecated
version.

## Export

- JSON: `serde_json::to_string(&report)`
- CSV: `render_csv(&report)` (one row per API and consumer)
- HTML: `render_html(&report)` (self-contained timeline with adoption bars and
  expandable consumer lists)
