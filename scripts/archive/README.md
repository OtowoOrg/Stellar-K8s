# scripts/archive

This directory holds historical and one-off scripts that were used during initial
repository setup and issue management. They are **not** part of the normal development
or operational workflow.

## Contents

- `create_batch_*.sh` — GitHub issue creation scripts used to seed the issue tracker
  in bulk during project bootstrapping. These are no longer needed for day-to-day work.
- `create_labels.sh` — One-time script to create GitHub issue labels.
- `create_wave_issues.sh` — Wave-based issue creation script.
- `run_batches.sh` — Coordinator for the batch creation scripts above.
- `update_epic_issues.sh` / `update_wave_issues.sh` — Historical scripts for updating
  issue metadata in bulk.

## Why archived instead of deleted?

These scripts document the project's issue taxonomy and the structure used when
bootstrapping the tracker. They may be useful as reference if similar bulk operations
are needed in the future.

If a script here is no longer useful even as reference, feel free to delete it.

## Active scripts

Operational scripts live in the parent `scripts/` directory. See the main
[README.md](../../README.md) and [DEVELOPMENT.md](../../DEVELOPMENT.md) for usage.
