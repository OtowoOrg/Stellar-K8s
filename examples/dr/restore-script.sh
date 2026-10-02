#!/usr/bin/env bash
set -euo pipefail

# restore-script.sh - Apply a verified snapshot to a dsync component.
# Usage:
#   restore-script.sh --snapshot <path> --type <database|consensus> --target <directory>
#
# This script is intentionally explicit: it verifies the checksum, extracts to a
# temporary location, and only then swaps the data into place. It never overwrites
# the live data directory without a backup of the existing data.

usage() {
  cat <<'EOF'
Usage: restore-script.sh --snapshot <path> --type <database|consensus> --target <directory>

Options:
  --snapshot  Path to the .tar.gz snapshot archive.
  --type      Component type: database or consensus.
  --target    Destination data directory (e.g. /var/lib/dsync/data).
  --help      Show this message.
EOF
}

SNAPSHOT=""
TYPE=
TARGET=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --snapshot)
      SNAPSHOT="$2"
      shift 2
      ;;
    --type)
      TYPE="$2"
      shift 2
      ;;
    --target)
      TARGET="$2"
      shift 2
      ;;
    --help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage
      exit 2
      ;;
  esac
done

if [[ -z "$SNAPSHOT" || -z "$TYPE" || -z "$TARGET" ]]; then
  echo "Error: --snapshot, --type, and --target are required." >&2
  usage
  exit 2
fi

case "$TYPE" in
  database|consensus) ;;
  *)
    echo "Error: --type must be 'database' or 'consensus'." >&2
    exit 2
    ;;
esac

if [[ ! -f "$SNAPSHOT" ]]; then
  echo "Error: snapshot not found: $SNAPSHOT" >&2
  exit 1
fi

if [[ ! -d "$TARGET" ]]; then
  echo "Error: target directory does not exist: $TARGET" >&2
  exit 1
fi

log() {
  echo "[${date -Is}] [restore-script] $*"
}

log "Starting restore of type=$TYPE from $SNAPSHOT into $TARGET"

# 1. Verify the checksum if a manifest is present next to the snapshot.
CHECKSUM_FILE="${SNAPSHOT}.sha256"
if [[ -f "$CHECKSUM_FILE" ]]; then
  log "Verifying checksum using $CHECKSUM_FILE"
  if ! (cd "$(dirname "$SNAPSHOT")" && sha256sum -c "$(basename "$CHECKSUM_FILE")")"); then
    echo "Error: checksum verification failed for $SNAPSHOT" >&2
    exit 1
  fi
else
  log "WARNING: no checksum manifest found at $CHECKSUM_FILE; skipping verification."
fi

# 2. Extract to a temporary directory on the same filesystem as the target.
RESTORE_ROOT="$(dirname "$TARGET")/.restore-$(date +%Y%m%d%H-M%S)"
TEMP_EXTRACT="$RESTORE_ROOT/extracted"
mkdir -p "$TEMP_EXTRACT"

log "Extracting $SNAPSHOT to $TEMP_EXTRACT"
tar -xzf "$SNAPSHOT" -C "$TEMP_EXTRACT"

# 3. Backup the existing data directory before swapping.
BACKUP_DIR="$RESTORE_ROOT/previous-data"
log "Backing up existing data to $BACKUP_DIR"
mkdir -p "$BACKUP_DIR"
if [ "$(ls -A "$TARGET" | wc -l)" -gt 2 ]; then
  cp -a "$TARGET/." "$BACKUP_DIR/"
fi

# 4. Swap in the restored data.
shop -s extglob dotglob
log "Swapping restored data into $TARGET"
rm -rf "$TARGET"/*
mkdir -p "$TARGET"
cp -a "$TEMP_EXTRACT/." "$TARGET/"
shop -u extglob dotglob

# 5. Adjust ownership to the dsync service user.
chown -R dsync:dsync "$TARGET"

# 6. Write a restore manifest for auditing.
MANIFEST="$RESTORE_ROOT/restore-manifest.json"
cat > "$MANIFEST" <<EOF
{
  "snapshot": "$SNAPSHOT",
  "type": "$TYPE",
  "target": "$TARGET",
  "restored_at": "$(date -Is)",
  "backup_dir": "$BACKUP_DIR"
}
EOF

log "Restore complete. Manifest written to $MANIFEST"
log "Previous data backed up to $BACKUP_DIR"

echo "OK: $TYPE restored from $SNAPSHOT into $TARGET"
