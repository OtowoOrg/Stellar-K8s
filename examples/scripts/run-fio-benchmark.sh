#!/usr/bin/env bash
# run-fio-benchmark.sh
#
# Storage benchmark for Stellar Core hosts running on bare-metal / instance-store NVMe.
# Runs a fixed set of fio jobs that model Stellar Core disk access patterns and checks
# the results against the validator and archive targets documented in
# docs/performance/bare-metal-iops.md.
#
# Safety: the script only ever writes to a single scratch file inside --target-dir and
# removes it on exit. It refuses to run against a raw block device.
#
# Usage:
#   ./run-fio-benchmark.sh --target-dir /mnt/stellar-data [options]
#
# Requirements: Linux, fio >= 3.5, jq, coreutils, util-linux (findmnt, lsblk).

set -euo pipefail

SCRIPT_NAME="$(basename "$0")"
readonly SCRIPT_NAME
readonly TEST_FILE_NAME="stellar-fio-benchmark.dat"

# ----------------------------------------------------------------------------
# Defaults
# ----------------------------------------------------------------------------
TARGET_DIR=""
PROFILE="all"          # validator | archive | all
SIZE="4G"              # size of the scratch file (fio units, base 1024)
RUNTIME=60             # seconds per job
RAMP=10                # warm-up seconds per job, excluded from results
IOENGINE="libaio"
NUMJOBS=""             # default: min(nproc, 4)
OUTPUT_DIR=""
LABEL=""
STRICT=0

# ----------------------------------------------------------------------------
# Targets (keep in sync with docs/performance/bare-metal-iops.md, section 2)
# Format: profile:job:metric:comparator:minimum:recommended
#   metric: iops | bw_mib | p99_us
#   comparator: ge (higher is better) | le (lower is better)
# ----------------------------------------------------------------------------
readonly TARGETS=(
  "validator:rand-write-4k:iops:ge:10000:30000"
  "validator:rand-read-4k:iops:ge:10000:50000"
  "validator:mixed-70r30w-4k:iops:ge:10000:30000"
  "validator:sync-write-4k:p99_us:le:2000:1000"
  "validator:seq-write-1m:bw_mib:ge:200:500"
  "archive:rand-write-4k:iops:ge:5000:15000"
  "archive:rand-read-4k:iops:ge:5000:20000"
  "archive:seq-write-1m:bw_mib:ge:300:1000"
  "archive:seq-read-1m:bw_mib:ge:500:1500"
)

usage() {
  cat <<EOF
Usage: ${SCRIPT_NAME} --target-dir DIR [options]

Benchmarks the filesystem backing DIR with fio jobs that model Stellar Core I/O and
reports PASS/WARN/FAIL against validator and/or archive node targets.

Required:
  -d, --target-dir DIR   Directory on the NVMe filesystem under test (must be writable).
                         A single scratch file (${TEST_FILE_NAME}) is created and removed.

Options:
  -p, --profile NAME     validator | archive | all            (default: ${PROFILE})
  -s, --size SIZE        Scratch file size, fio units e.g. 4G  (default: ${SIZE})
  -r, --runtime SEC      Measured seconds per job              (default: ${RUNTIME})
      --ramp SEC         Warm-up seconds per job               (default: ${RAMP})
  -e, --ioengine NAME    fio ioengine: libaio | io_uring       (default: ${IOENGINE})
  -j, --numjobs N        Parallel jobs for random tests        (default: min(nproc,4))
  -o, --output-dir DIR   Where to write JSON + summary         (default: ./fio-results-<ts>)
  -l, --label TEXT       Free-text tag stored in the summary (e.g. "sched=none,xfs")
      --strict           Exit 2 if any minimum target is missed
  -h, --help             Show this help

Examples:
  ${SCRIPT_NAME} -d /mnt/stellar-data -p validator
  ${SCRIPT_NAME} -d /mnt/stellar-data -p archive -s 16G -r 120 --label "mq-deadline"
EOF
}

log()  { printf '[%s] %s\n' "$(date -u +%H:%M:%S)" "$*" >&2; }
warn() { printf '[%s] WARNING: %s\n' "$(date -u +%H:%M:%S)" "$*" >&2; }
die()  { printf '[%s] ERROR: %s\n' "$(date -u +%H:%M:%S)" "$*" >&2; exit 1; }

# ----------------------------------------------------------------------------
# Argument parsing
# ----------------------------------------------------------------------------
while [[ $# -gt 0 ]]; do
  case "$1" in
    -d|--target-dir) TARGET_DIR="${2:-}"; shift 2 ;;
    -p|--profile)    PROFILE="${2:-}"; shift 2 ;;
    -s|--size)       SIZE="${2:-}"; shift 2 ;;
    -r|--runtime)    RUNTIME="${2:-}"; shift 2 ;;
    --ramp)          RAMP="${2:-}"; shift 2 ;;
    -e|--ioengine)   IOENGINE="${2:-}"; shift 2 ;;
    -j|--numjobs)    NUMJOBS="${2:-}"; shift 2 ;;
    -o|--output-dir) OUTPUT_DIR="${2:-}"; shift 2 ;;
    -l|--label)      LABEL="${2:-}"; shift 2 ;;
    --strict)        STRICT=1; shift ;;
    -h|--help)       usage; exit 0 ;;
    *) usage >&2; die "Unknown argument: $1" ;;
  esac
done

# ----------------------------------------------------------------------------
# Validation
# ----------------------------------------------------------------------------
[[ "$(uname -s)" == "Linux" ]] || die "This benchmark targets Linux hosts only."
[[ -n "$TARGET_DIR" ]] || { usage >&2; die "--target-dir is required."; }
[[ ! -b "$TARGET_DIR" ]] || die "Refusing to benchmark a raw block device; pass a directory on a mounted filesystem."
[[ -d "$TARGET_DIR" ]] || die "Target directory does not exist: $TARGET_DIR"
[[ -w "$TARGET_DIR" ]] || die "Target directory is not writable: $TARGET_DIR"

case "$PROFILE" in validator|archive|all) ;; *) die "Invalid --profile: $PROFILE" ;; esac
case "$IOENGINE" in libaio|io_uring) ;; *) die "Invalid --ioengine: $IOENGINE (use libaio or io_uring)" ;; esac
[[ "$RUNTIME" =~ ^[1-9][0-9]*$ ]] || die "--runtime must be a positive integer"
[[ "$RAMP" =~ ^[0-9]+$ ]] || die "--ramp must be a non-negative integer"

for cmd in fio jq findmnt lsblk df awk; do
  command -v "$cmd" >/dev/null 2>&1 || die "Required command not found: $cmd"
done

if [[ -z "$NUMJOBS" ]]; then
  NUMJOBS="$(nproc)"
  if (( NUMJOBS > 4 )); then NUMJOBS=4; fi
fi
[[ "$NUMJOBS" =~ ^[1-9][0-9]*$ ]] || die "--numjobs must be a positive integer"

# Convert an fio-style size (4G, 512M, 1T; base 1024) to bytes.
size_to_bytes() {
  local value="$1" num unit
  [[ "$value" =~ ^([0-9]+)([KkMmGgTt]?)[iI]?[bB]?$ ]] || return 1
  num="${BASH_REMATCH[1]}"; unit="${BASH_REMATCH[2]^^}"
  case "$unit" in
    "") echo "$num" ;;
    K)  echo $(( num * 1024 )) ;;
    M)  echo $(( num * 1024 ** 2 )) ;;
    G)  echo $(( num * 1024 ** 3 )) ;;
    T)  echo $(( num * 1024 ** 4 )) ;;
  esac
}

SIZE_BYTES="$(size_to_bytes "$SIZE")" || die "Invalid --size: $SIZE"
(( SIZE_BYTES >= 1024 ** 3 )) || die "--size must be at least 1G to get meaningful results"

FREE_BYTES=$(( $(df -Pk "$TARGET_DIR" | awk 'NR==2 {print $4}') * 1024 ))
# Leave 10% headroom so the benchmark cannot fill the filesystem.
(( FREE_BYTES > SIZE_BYTES + SIZE_BYTES / 10 )) \
  || die "Not enough free space in $TARGET_DIR (need > $SIZE plus 10% headroom)."

TEST_FILE="${TARGET_DIR%/}/${TEST_FILE_NAME}"
[[ ! -e "$TEST_FILE" ]] || die "$TEST_FILE already exists; remove it or pick another directory."

OUTPUT_DIR="${OUTPUT_DIR:-./fio-results-$(date -u +%Y%m%dT%H%M%SZ)}"
mkdir -p "$OUTPUT_DIR"

# shellcheck disable=SC2329  # invoked via trap
cleanup() { rm -f "$TEST_FILE"; }
trap cleanup EXIT
trap 'die "Interrupted"' INT TERM

# ----------------------------------------------------------------------------
# Environment capture (best effort; recorded alongside results)
# ----------------------------------------------------------------------------
MNT_SOURCE="$(findmnt -no SOURCE -T "$TARGET_DIR" || echo unknown)"
MNT_FSTYPE="$(findmnt -no FSTYPE -T "$TARGET_DIR" || echo unknown)"
MNT_OPTS="$(findmnt -no OPTIONS -T "$TARGET_DIR" || echo unknown)"

# Resolve the whole-disk device (e.g. nvme0n1) behind partitions / LVM / md.
DISK_NAME="$(lsblk -rnso NAME,TYPE "$MNT_SOURCE" 2>/dev/null | awk '$2=="disk"{print $1; exit}')"
DISK_NAME="${DISK_NAME:-$(basename "$MNT_SOURCE")}"
SYSFS_QUEUE="/sys/block/${DISK_NAME}/queue"
read_sysfs() { [[ -r "$1" ]] && cat "$1" || echo "n/a"; }
SCHEDULER="$(read_sysfs "$SYSFS_QUEUE/scheduler")"
READ_AHEAD_KB="$(read_sysfs "$SYSFS_QUEUE/read_ahead_kb")"
DISK_MODEL="$(read_sysfs "/sys/block/${DISK_NAME}/device/model" | xargs)"

case "$MNT_FSTYPE" in
  tmpfs|overlay|nfs|nfs4|fuse*) warn "Target is on '$MNT_FSTYPE'; results will not reflect NVMe performance." ;;
esac
[[ "$DISK_NAME" == nvme* ]] || warn "Backing disk '$DISK_NAME' does not look like an NVMe device."

FIO_VERSION="$(fio --version)"

log "Target:     $TARGET_DIR ($MNT_SOURCE, $MNT_FSTYPE)"
log "Disk:       $DISK_NAME ${DISK_MODEL:+($DISK_MODEL)}  scheduler: $SCHEDULER"
log "fio:        $FIO_VERSION  engine=$IOENGINE numjobs=$NUMJOBS size=$SIZE runtime=${RUNTIME}s ramp=${RAMP}s"
log "Results:    $OUTPUT_DIR"

# ----------------------------------------------------------------------------
# Job definitions
# Format: name|description|fio arguments
# ----------------------------------------------------------------------------
readonly COMMON_ARGS=(
  "--filename=$TEST_FILE" "--size=$SIZE" "--time_based" "--runtime=$RUNTIME"
  "--ramp_time=$RAMP" "--group_reporting" "--output-format=json" "--randrepeat=0"
  "--norandommap"
)

declare -A JOB_DESC JOB_ARGS
JOB_ORDER=()
add_job() {
  JOB_ORDER+=("$1"); JOB_DESC["$1"]="$2"; JOB_ARGS["$1"]="$3"
}

add_job "rand-write-4k" "Bucket/DB page writes (4k random write, QD32)" \
  "--rw=randwrite --bs=4k --iodepth=32 --numjobs=$NUMJOBS --ioengine=$IOENGINE --direct=1"
add_job "rand-read-4k" "Ledger-state lookups (4k random read, QD32)" \
  "--rw=randread --bs=4k --iodepth=32 --numjobs=$NUMJOBS --ioengine=$IOENGINE --direct=1"
add_job "mixed-70r30w-4k" "Ledger close apply (70/30 random read/write, QD16)" \
  "--rw=randrw --rwmixread=70 --bs=4k --iodepth=16 --numjobs=$NUMJOBS --ioengine=$IOENGINE --direct=1"
add_job "sync-write-4k" "DB commit / WAL (4k write + fdatasync, QD1)" \
  "--rw=write --bs=4k --iodepth=1 --numjobs=1 --ioengine=sync --direct=0 --fdatasync=1"
add_job "seq-write-1m" "Bucket merges, history publish (1M sequential write, QD8)" \
  "--rw=write --bs=1M --iodepth=8 --numjobs=1 --ioengine=$IOENGINE --direct=1"
add_job "seq-read-1m" "Catchup / history serving (1M sequential read, QD8)" \
  "--rw=read --bs=1M --iodepth=8 --numjobs=1 --ioengine=$IOENGINE --direct=1"

# Select jobs required by the chosen profile.
declare -A WANTED=()
for t in "${TARGETS[@]}"; do
  IFS=: read -r t_profile t_job _ <<<"$t"
  if [[ "$PROFILE" == "all" || "$PROFILE" == "$t_profile" ]]; then
    WANTED["$t_job"]=1
  fi
done

# ----------------------------------------------------------------------------
# Run
# ----------------------------------------------------------------------------
declare -A R_IOPS R_BW R_P99
for job in "${JOB_ORDER[@]}"; do
  [[ -n "${WANTED[$job]:-}" ]] || continue
  out="$OUTPUT_DIR/$job.json"
  log "Running $job — ${JOB_DESC[$job]}"
  # shellcheck disable=SC2086  # JOB_ARGS entries are intentionally word-split.
  fio --name="$job" "${COMMON_ARGS[@]}" ${JOB_ARGS[$job]} --output="$out" \
    || die "fio failed for job $job (see $out)"

  # Aggregate read+write so mixed jobs report total IOPS / bandwidth.
  R_IOPS["$job"]="$(jq -r '.jobs[0] | (.read.iops + .write.iops)' "$out")"
  R_BW["$job"]="$(jq -r '.jobs[0] | ((.read.bw_bytes // (.read.bw * 1024)) + (.write.bw_bytes // (.write.bw * 1024))) / 1048576' "$out")"
  if [[ "$job" == "sync-write-4k" ]]; then
    # Latency of the fdatasync() call itself: what a DB commit waits on.
    R_P99["$job"]="$(jq -r '.jobs[0].sync.lat_ns.percentile["99.000000"] // .jobs[0].write.clat_ns.percentile["99.000000"] // 0' "$out")"
  else
    R_P99["$job"]="$(jq -r '.jobs[0] | [.read.clat_ns.percentile["99.000000"] // 0, .write.clat_ns.percentile["99.000000"] // 0] | max' "$out")"
  fi
  R_P99["$job"]="$(awk -v ns="${R_P99[$job]}" 'BEGIN { printf "%.0f", ns / 1000 }')"
done

# ----------------------------------------------------------------------------
# Report
# ----------------------------------------------------------------------------
SUMMARY="$OUTPUT_DIR/summary.md"
fmt() { awk -v v="$1" 'BEGIN { printf "%.0f", v }'; }

{
  echo "# fio benchmark summary"
  echo
  echo "| Field | Value |"
  echo "|---|---|"
  echo "| Date (UTC) | $(date -u +%Y-%m-%dT%H:%M:%SZ) |"
  if [[ -n "$LABEL" ]]; then echo "| Label | $LABEL |"; fi
  echo "| Host | $(hostname) |"
  echo "| Kernel | $(uname -r) |"
  echo "| fio | $FIO_VERSION ($IOENGINE, numjobs=$NUMJOBS, size=$SIZE, runtime=${RUNTIME}s, ramp=${RAMP}s) |"
  echo "| Device | $DISK_NAME ${DISK_MODEL:+($DISK_MODEL)} |"
  echo "| Scheduler | $SCHEDULER |"
  echo "| read_ahead_kb | $READ_AHEAD_KB |"
  echo "| Filesystem | $MNT_FSTYPE on $MNT_SOURCE |"
  echo "| Mount options | $MNT_OPTS |"
  echo
  echo "## Raw results"
  echo
  echo "| Job | Pattern | IOPS | MiB/s | p99 latency (µs) |"
  echo "|---|---|---:|---:|---:|"
  for job in "${JOB_ORDER[@]}"; do
    [[ -n "${R_IOPS[$job]:-}" ]] || continue
    echo "| $job | ${JOB_DESC[$job]} | $(fmt "${R_IOPS[$job]}") | $(fmt "${R_BW[$job]}") | ${R_P99[$job]} |"
  done
  echo
  echo "## Target evaluation"
  echo
  echo "| Profile | Job | Metric | Result | Minimum | Recommended | Status |"
  echo "|---|---|---|---:|---:|---:|---|"
} >"$SUMMARY"

FAILURES=0
for t in "${TARGETS[@]}"; do
  IFS=: read -r t_profile t_job t_metric t_cmp t_min t_rec <<<"$t"
  [[ "$PROFILE" == "all" || "$PROFILE" == "$t_profile" ]] || continue

  case "$t_metric" in
    iops)   value="$(fmt "${R_IOPS[$t_job]}")"; unit="IOPS" ;;
    bw_mib) value="$(fmt "${R_BW[$t_job]}")"; unit="MiB/s" ;;
    p99_us) value="${R_P99[$t_job]}"; unit="p99 µs" ;;
  esac

  if [[ "$t_cmp" == "ge" ]]; then
    if   (( value >= t_rec )); then status="PASS"
    elif (( value >= t_min )); then status="WARN (below recommended)"
    else status="FAIL"; FAILURES=$((FAILURES + 1)); fi
    op="≥"
  else
    if   (( value <= t_rec )); then status="PASS"
    elif (( value <= t_min )); then status="WARN (above recommended)"
    else status="FAIL"; FAILURES=$((FAILURES + 1)); fi
    op="≤"
  fi
  echo "| $t_profile | $t_job | $unit | $value | $op $t_min | $op $t_rec | $status |" >>"$SUMMARY"
done

echo >&2
cat "$SUMMARY"
echo >&2
log "Summary written to $SUMMARY (raw fio JSON alongside it)."

if (( FAILURES > 0 )); then
  warn "$FAILURES minimum target(s) missed. See docs/performance/bare-metal-iops.md for tuning steps."
  if (( STRICT == 1 )); then exit 2; fi
fi
exit 0
