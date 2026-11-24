# Uptime SLA Reporting and Delegation Reward Tracking

## Uptime SLA (`src/sla`)

`SlaTracker` accumulates probe results (`record(ts, up)`) and computes
time-weighted uptime over a rolling window, so irregular probe intervals do not
skew the result.

- `SlaConfig { target_percent, window_secs }` is passed on each evaluation, so a
  changed `slaTarget` takes effect on the next reconcile without a restart.
- `check_breach` returns an `SlaBreach` when window uptime is below target;
  call it on every probe cycle to alert within one probe interval.
- `monthly_report` returns per-UTC-month uptime, whether the target was met and
  the month-over-month trend in percentage points.

## Delegation rewards (`src/delegation`)

`DelegationTracker` holds the delegated-stake snapshot (`update_stake`, refreshed
from network data each epoch) and an append-only reward ledger.

- `distribute(epoch, total_reward, commission_bps)`: the validator keeps the
  commission, the rest is split pro rata to stake in integer stroops. Rounding
  remainders go to the validator so the ledger sums exactly.
- `history` / `epoch_entries` query the ledger.
- `statement_csv` / `statement_json` export a delegator statement.

Both modules are pure logic with unit tests; wiring them into the reconciler
(CRD field for `slaTarget`, probe feed, status history) is follow-up work.
