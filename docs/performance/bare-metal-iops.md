# Bare-Metal NVMe IOPS Tuning & Deployment Guide

Stellar Core spends much of each ledger close writing to disk: bucket list
merges, ledger state updates, and database commits. On most network-attached
cloud block storage, disk writes are the bottleneck for ingestion and catchup.
This guide covers tuning physical (or instance-store) NVMe drives for Stellar
Core and checking the result with a repeatable `fio` benchmark.

It covers three areas:

1. **Kernel I/O scheduler** choice for NVMe (`none`, the multi-queue successor
   of `noop`, vs `mq-deadline`).
2. **XFS and ext4** creation and mount flags (`noatime`, `discard`, and related
   flags) for write speed and drive endurance.
3. **`fio` benchmarking** with
   [`examples/scripts/run-fio-benchmark.sh`](../../examples/scripts/run-fio-benchmark.sh)
   and the IOPS targets for mainnet participation.

Related documents:

- [Capacity Planning](../operations/capacity-planning.md): how much disk you
  need. This guide covers how fast the disk must be.
- [Resource Limits](../resource-limits.md): CPU and memory for each node type.
- [Proactive Disk Scaling](../proactive-disk-scaling.md): only for network
  volumes. Local NVMe cannot be expanded online.

> **Production safety.** Every setting in this guide keeps the kernel's
> data-integrity guarantees in place. Nothing here turns off write barriers,
> journaling, or `fsync`. Settings that do are listed under
> [§4.5 Never use](#45-options-that-must-never-be-used) and must not be used
> on a validator.

---

## Contents

- [1. Workload profiles: validator vs archive](#1-workload-profiles-validator-vs-archive)
- [2. Baseline IOPS targets](#2-baseline-iops-targets)
- [3. Kernel I/O scheduler](#3-kernel-io-scheduler)
- [4. Filesystem: XFS and ext4](#4-filesystem-xfs-and-ext4)
- [5. Benchmarking with fio](#5-benchmarking-with-fio)
- [6. Reference baseline: AWS i3.large](#6-reference-baseline-aws-i3large)
- [7. Deploying on Stellar-K8s with local NVMe](#7-deploying-on-stellar-k8s-with-local-nvme)
- [8. Production checklist](#8-production-checklist)

---

## 1. Workload profiles: validator vs archive

Validators and archive nodes put very different loads on the disk, so tune and
benchmark them separately.

| | **Validator node** (heavy IOPS) | **Archive node** (heavy storage) |
|---|---|---|
| Typical config | `nodeType: Validator`, `historyMode: Recent` | `historyMode: Full`, history archive publisher, full-history Horizon/RPC backing store |
| What hits the disk | Bucket list merges, ledger-state reads/writes, DB commits on every ledger close (about every 5s) | Sequential catchup replay, writing history checkpoints (millions of small `.xdr.gz` files), serving history reads |
| Dominant pattern | Small random I/O plus latency-sensitive `fdatasync` | Large sequential I/O plus metadata-heavy small-file creation |
| Metric that matters most | 4k random IOPS and p99 sync latency | Sequential MiB/s, capacity, and inode headroom |
| Failure symptom | Ledger apply time grows, node falls behind and loses sync | Catchup takes days, history publishing lags, disk or inodes fill up |
| Capacity | About 100Gi (see [capacity planning](../operations/capacity-planning.md#11-validator-stellar-core)) | 1.5Ti or more and growing (see [capacity planning](../operations/capacity-planning.md#3-storage-growth-model)) |
| Recommended filesystem | XFS (ext4 is fine) | **XFS** (dynamic inode allocation) |

**If a node does both** (for example a Tier 1 validator that also publishes a
full history archive), it must meet the **validator** IOPS and latency targets
*and* the **archive** throughput and capacity targets. Where possible, put the
database/buckets and the history archive on separate NVMe drives.

---

## 2. Baseline IOPS targets

The benchmark script checks these values (they are defined in the `TARGETS`
array in the script). **Minimum** is the lowest value that still gives smooth
mainnet participation. **Recommended** leaves headroom for traffic spikes,
catchup after downtime, and drive wear.

The validator random-IOPS minimum is taken from the
[official Stellar validator prerequisites](https://developers.stellar.org/docs/validators/admin-guide/prerequisites)
(NVMe SSD, 10,000 IOPS). This matches
[capacity planning §1.1](../operations/capacity-planning.md#11-validator-stellar-core).

### Validator

| Job | What it models | Metric | Minimum | Recommended |
|---|---|---|---:|---:|
| `rand-write-4k` | Bucket and DB page writes | IOPS | ≥ 10,000 | ≥ 30,000 |
| `rand-read-4k` | Ledger-state lookups | IOPS | ≥ 10,000 | ≥ 50,000 |
| `mixed-70r30w-4k` | Applying a ledger close | IOPS (read + write) | ≥ 10,000 | ≥ 30,000 |
| `sync-write-4k` | DB commit / WAL `fdatasync` | p99 latency | ≤ 2,000 µs | ≤ 1,000 µs |
| `seq-write-1m` | Bucket merges | MiB/s | ≥ 200 | ≥ 500 |

### Archive

| Job | What it models | Metric | Minimum | Recommended |
|---|---|---|---:|---:|
| `rand-write-4k` | Checkpoint file and index writes | IOPS | ≥ 5,000 | ≥ 15,000 |
| `rand-read-4k` | Serving history lookups | IOPS | ≥ 5,000 | ≥ 20,000 |
| `seq-write-1m` | Catchup replay, archive publishing | MiB/s | ≥ 300 | ≥ 1,000 |
| `seq-read-1m` | Catchup and history serving | MiB/s | ≥ 500 | ≥ 1,500 |

**Why the sync latency target matters.** Random IOPS at a high queue depth show
how much work the drive can do in parallel. Stellar Core's commit path, however,
waits on one `fdatasync()` at a time. Datacenter NVMe drives with power-loss
protection (PLP) finish these in tens of microseconds. Consumer drives without
PLP must flush their volatile cache and often take several milliseconds. A
drive can pass every IOPS test and still fail `sync-write-4k`. **Use drives
with PLP for validators.**

---

## 3. Kernel I/O scheduler

### 3.1 `noop` vs `none` vs `mq-deadline`

NVMe devices use the multi-queue block layer (`blk-mq`). The legacy
single-queue schedulers, including `noop`, `deadline` and `cfq`, were **removed
in Linux 5.0**. On current kernels:

| Scheduler | What it is | Use for Stellar |
|---|---|---|
| `none` | The blk-mq equivalent of `noop`: requests go straight to the device's hardware queues without reordering. | **Default for both profiles.** NVMe drives run many hardware queues and do their own internal scheduling, so host-side reordering only costs CPU. |
| `mq-deadline` | The blk-mq port of `deadline`: it gives each read and write an expiry time and prefers reads. | **Only for archive nodes whose single drive also serves other latency-sensitive reads** (for example history serving while a catchup writes heavily). It stops large sequential writes from starving reads, but it has one global dispatch lock, which lowers peak IOPS on fast NVMe. |
| `kyber`, `bfq` | Latency-target / fairness schedulers | Not recommended. `bfq` in particular adds heavy per-I/O CPU cost on NVMe. |

Most distributions already use `none` for NVMe. Check it anyway: some cloud
images and tuning profiles (for example `tuned` profiles) change it.

### 3.2 Inspect and set at runtime

```bash
# The active scheduler is shown in [brackets]
cat /sys/block/nvme0n1/queue/scheduler
# [none] mq-deadline kyber bfq

# Change it for testing. This is not persistent and is safe to do while mounted.
echo none | sudo tee /sys/block/nvme0n1/queue/scheduler
```

Change the scheduler on a live node only while it is drained or out of
consensus. Changing it is safe, but it briefly pauses the queue.

### 3.3 Persist with udev

The scheduler is a per-device setting, so persist it with a udev rule rather
than the `elevator=` kernel parameter (ignored by blk-mq on current kernels):

```bash
sudo tee /etc/udev/rules.d/60-stellar-nvme-scheduler.rules <<'EOF'
# Stellar Core NVMe: no host-side I/O scheduling (blk-mq "none").
ACTION=="add|change", KERNEL=="nvme[0-9]*n[0-9]*", ENV{DEVTYPE}=="disk", ATTR{queue/scheduler}="none"
EOF

sudo udevadm control --reload-rules
sudo udevadm trigger --subsystem-match=block --action=change
cat /sys/block/nvme*n*/queue/scheduler
```

For an archive drive where `mq-deadline` benchmarked better, give that device
its own rule (match on `ENV{ID_SERIAL}` rather than the kernel name, because
`nvmeXnY` numbering can change between boots).

### 3.4 Other queue settings

Leave all other block-queue settings at the kernel defaults unless a benchmark
shows a clear improvement. There are two exceptions worth knowing about:

| Setting | Guidance |
|---|---|
| `queue/read_ahead_kb` | Keep the default (usually `128`) for validators. For archive drives that mostly serve sequential history reads, `1024` can help buffered reads. The benchmark script uses `O_DIRECT` and bypasses read-ahead, so measure this change with application-level catchup time instead. |
| Kernel parameter `nvme_core.default_ps_max_latency_us=0` | Turns off NVMe Autonomous Power State Transitions (APST). Set it **only** if you see periodic multi-millisecond latency spikes or controller resets on consumer-grade drives. Datacenter drives rarely enable APST. |

---

## 4. Filesystem: XFS and ext4

### 4.1 Before you format

> ⚠️ `mkfs` and `nvme format` destroy all data on the target device.
> Double-check the device name with `lsblk -o NAME,MODEL,SERIAL,SIZE,MOUNTPOINT`.

- **Use a dedicated drive for Stellar data.** Keep the OS and logs off it so
  they don't compete for the queue.
- **Use the whole device or a 1 MiB-aligned partition.** `parted -a optimal`
  and modern `fdisk` both align correctly by default.
- **Optional: 4K LBA format.** Some NVMe drives ship with 512-byte sectors but
  support 4K. Check with `sudo nvme id-ns -H /dev/nvme0n1 | grep 'LBA Format'`.
  Switching (`nvme format --lbaf=<n>`) is destructive and drive-specific, so
  benchmark before and after. Instance-store NVMe (AWS `i3`) does not expose
  this option.

### 4.2 XFS (recommended)

XFS allocates inodes dynamically and handles parallel allocation well. That
suits the millions of small files in a history archive and large bucket files.

```bash
# Defaults are correct for NVMe. mkfs.xfs TRIMs the whole device first by default.
sudo mkfs.xfs -L stellar-data /dev/nvme0n1
sudo mkdir -p /mnt/stellar-data
```

`/etc/fstab`:

```fstab
# Validator (heavy IOPS)
LABEL=stellar-data  /mnt/stellar-data  xfs  defaults,noatime,inode64,logbsize=256k  0 0

# Archive (heavy storage), same flags. Keep inode64 so inodes can live anywhere on large volumes.
# LABEL=stellar-data  /mnt/stellar-data  xfs  defaults,noatime,inode64,logbsize=256k  0 0
```

### 4.3 ext4

ext4 is fine for validators. For archive nodes, note that ext4 sets its inode
count at `mkfs` time, so **don't** use `-T largefile`/`largefile4`: a full
history archive can run out of inodes long before it runs out of space.

```bash
# Initialise inode tables and journal now, so the background lazy init
# doesn't distort benchmarks or early production I/O.
sudo mkfs.ext4 -L stellar-data -m 1 -E lazy_itable_init=0,lazy_journal_init=0 /dev/nvme0n1
```

`/etc/fstab`:

```fstab
LABEL=stellar-data  /mnt/stellar-data  ext4  defaults,noatime,errors=remount-ro  0 2
```

### 4.4 Mount flags explained

| Flag | FS | Effect | Profile |
|---|---|---|---|
| `noatime` | both | Stops writing an access-time update on every read. This removes a metadata write for each file read, which saves both IOPS and drive wear. Also implies `nodiratime`. | **Both, always** |
| `inode64` | XFS | Lets inodes be placed anywhere on the volume (the default on current kernels; set explicitly for clarity). | Both |
| `logbsize=256k` | XFS | Uses bigger in-memory log buffers, so metadata-heavy writes (bucket merges, checkpoint files) need fewer log I/Os. | Both |
| `errors=remount-ro` | ext4 | On corruption, remount read-only instead of carrying on. Stellar Core then stops rather than writing bad state. | Both |
| `discard` | both | Online TRIM: tells the drive about freed blocks right away. See [§4.6](#46-trim-discard-vs-fstrim). | Optional |
| `-m 1` (mkfs) | ext4 | Reserves 1% for root instead of 5%. On a 2 TB drive that frees about 80 GB. | Both |

### 4.5 Options that must never be used

These options trade crash-safety for benchmark numbers. After a power loss or
kernel panic they can leave the ledger database or bucket files **silently
corrupted**. That leads to a forced full re-catchup, and for validators it
risks externalising bad state.

| Option | Why it is unsafe |
|---|---|
| `nobarrier` / `barrier=0` | Turns off cache flushes, so `fsync` no longer means the data is durable. (Removed from XFS in Linux 4.19. Still accepted by ext4.) |
| `data=writeback` (ext4) | File data can reach disk after the metadata that points to it, so files can contain stale blocks after a crash. |
| `journal_async_commit` (ext4) | Weakens journal commit ordering and cannot be used with the default `data=ordered` mode. It brings no real benefit on NVMe. |
| `commit=` values above 5 (ext4) | Widens the window of data lost on a crash. It does not raise NVMe throughput. |

### 4.6 TRIM: `discard` vs `fstrim`

TRIM keeps the drive's free-block pool large. That keeps write speed stable and
reduces write amplification, which **extends drive endurance** (TBW/DWPD). How
TRIM is issued matters less than making sure it runs at all.

| Method | Pros | Cons | Recommendation |
|---|---|---|---|
| Periodic `fstrim` (`fstrim.timer`) | Batches TRIM into one weekly pass, well away from the hot write path | Freed space is reported to the drive with a delay of up to a week | **Default for validators**, whose p99 latency matters most |
| `discard` mount flag | The drive always has an up-to-date view of free space | Some controllers stall I/O while processing TRIM, so bucket-merge deletions can add latency to later writes | Acceptable for archive nodes, and for validators **only if** `run-fio-benchmark.sh` shows no regression with it on |

```bash
# Periodic TRIM (ships with util-linux on systemd distros)
sudo systemctl enable --now fstrim.timer
systemctl list-timers fstrim.timer

# Confirm the device supports TRIM (non-zero DISC-GRAN / DISC-MAX)
lsblk --discard /dev/nvme0n1
```

Don't enable both methods; pick one. For endurance, also watch
`percentage_used` in `sudo nvme smart-log /dev/nvme0n1` and plan to replace
the drive before it reaches 100%.

---

## 5. Benchmarking with fio

### 5.1 The script

[`examples/scripts/run-fio-benchmark.sh`](../../examples/scripts/run-fio-benchmark.sh)
runs the fio jobs from [§2](#2-baseline-iops-targets) against a directory on
the filesystem under test. It then prints a Markdown report with PASS, WARN or
FAIL for each target.

Safety properties:

- It writes to a **single scratch file** (`stellar-fio-benchmark.dat`) inside
  `--target-dir` and deletes it on exit, including on Ctrl-C.
- It refuses to run against a raw block device, refuses to overwrite an
  existing scratch file, and requires free space of `--size` plus 10%.
- It records the kernel, fio version, scheduler, filesystem and mount options
  next to the results, so runs can be compared.

Prerequisites:

```bash
# Debian/Ubuntu
sudo apt-get install -y fio jq
# RHEL/Amazon Linux/Fedora
sudo dnf install -y fio jq
```

### 5.2 Usage

```bash
# Validator targets only
./examples/scripts/run-fio-benchmark.sh --target-dir /mnt/stellar-data --profile validator

# Archive targets, with a larger working set and a longer run
./examples/scripts/run-fio-benchmark.sh -d /mnt/stellar-data -p archive -s 16G -r 120

# Use as a gate in node provisioning (exit code 2 if any minimum is missed)
./examples/scripts/run-fio-benchmark.sh -d /mnt/stellar-data --strict
```

| Flag | Default | Notes |
|---|---|---|
| `-d, --target-dir` | (required) | Must be on the NVMe filesystem that will hold Stellar data. |
| `-p, --profile` | `all` | `validator`, `archive`, or `all` |
| `-s, --size` | `4G` | Working-set size. Increase it to go beyond the drive's DRAM/SLC cache (see §5.4). |
| `-r, --runtime` / `--ramp` | `60` / `10` s | Measurement time and warm-up time per job |
| `-e, --ioengine` | `libaio` | `io_uring` on kernel 5.1 or later can show higher IOPS at the same CPU cost |
| `-j, --numjobs` | `min(nproc, 4)` | Parallel workers for the random tests |
| `-l, --label` | none | Tags the report, for A/B comparisons (for example `"sched=mq-deadline"`) |
| `--strict` | off | Exit code 2 if any minimum target is missed |

Output is written to `./fio-results-<timestamp>/`: one raw fio JSON file per
job plus `summary.md`.

### 5.3 Jobs and equivalent raw `fio` commands

All jobs use a shared scratch file, `--time_based --ramp_time --group_reporting`,
and `--direct=1` (bypassing the page cache) unless noted. You can reproduce
them by hand:

```bash
F=/mnt/stellar-data/stellar-fio-benchmark.dat
COMMON="--filename=$F --size=4G --time_based --runtime=60 --ramp_time=10 --group_reporting --randrepeat=0 --norandommap"

# rand-write-4k: bucket/DB page writes
fio --name=rand-write-4k $COMMON --rw=randwrite --bs=4k --iodepth=32 --numjobs=4 --ioengine=libaio --direct=1
# rand-read-4k: ledger-state lookups
fio --name=rand-read-4k  $COMMON --rw=randread  --bs=4k --iodepth=32 --numjobs=4 --ioengine=libaio --direct=1
# mixed-70r30w-4k: ledger close apply
fio --name=mixed-70r30w-4k $COMMON --rw=randrw --rwmixread=70 --bs=4k --iodepth=16 --numjobs=4 --ioengine=libaio --direct=1
# sync-write-4k: DB commit path, buffered write + fdatasync after every write
fio --name=sync-write-4k $COMMON --rw=write --bs=4k --iodepth=1 --numjobs=1 --ioengine=sync --direct=0 --fdatasync=1
# seq-write-1m: bucket merges, history publish
fio --name=seq-write-1m  $COMMON --rw=write --bs=1M --iodepth=8 --numjobs=1 --ioengine=libaio --direct=1
# seq-read-1m: catchup, history serving
fio --name=seq-read-1m   $COMMON --rw=read  --bs=1M --iodepth=8 --numjobs=1 --ioengine=libaio --direct=1

rm -f "$F"
```

For `sync-write-4k`, the script reports the p99 latency of the `fdatasync()`
call itself (fio's `sync.lat_ns`), because that is what a database commit
waits on.

### 5.4 Getting trustworthy numbers

- **Test the real mount.** Run against the filesystem, mount flags and
  scheduler you will use in production. Raw-device numbers overstate what
  Stellar Core will see.
- **Beat the cache.** Many drives absorb short bursts in DRAM or SLC cache.
  Use `--size` of at least 4× the drive's cache (16G–64G for most datacenter
  NVMe drives), and on archive drives also try `-r 300` to see
  steady-state sequential write speed.
- **Steady state vs fresh-out-of-box.** A new or freshly TRIMmed SSD writes
  faster than a full one. For a validator you plan to run for years, run the
  benchmark again after the drive has been in service. The baseline in §6 is
  fresh-out-of-box.
- **Keep the host quiet.** Stop Stellar Core (or drain the pod) before
  benchmarking. The benchmark saturates the drive.
- **A/B one change at a time.** Change the scheduler **or** a mount flag,
  rerun with `--label`, and compare the two `summary.md` files. Run each
  configuration at least twice. Differences under about 5% are noise.

### 5.5 When a target fails

| Failing job | Likely cause | Check or fix |
|---|---|---|
| `rand-*-4k` far below spec | Wrong scheduler, filesystem on a slow device, drive throttling | `cat /sys/block/*/queue/scheduler`; `lsblk`; `sudo nvme smart-log` (temperature, `critical_warning`) |
| `sync-write-4k` p99 in milliseconds | Drive has no PLP, or the drive or RAID controller cache is in write-through mode | Use a PLP drive; check the controller cache policy |
| `seq-write-1m` drops off with long runtimes | SLC cache exhausted (consumer or QLC drive) | Use a TLC datacenter drive; don't use QLC for validators |
| Periodic latency spikes | Online `discard`, APST power-state changes | Switch to `fstrim.timer` (§4.6); see `nvme_core.default_ps_max_latency_us` (§3.4) |
| Everything around 3,000 IOPS | You are benchmarking a network volume (EBS `gp3` default), not local NVMe | Check the `Filesystem` row in `summary.md` |

---

## 6. Reference baseline: AWS i3.large

AWS `i3.large` is a widely available, reproducible reference point for
instance-store NVMe. Use it to sanity-check the script and your expectations
before testing your own hardware.

| | |
|---|---|
| Instance | `i3.large`: 2 vCPU, 15.25 GiB RAM |
| Storage | 1 × 475 GB NVMe SSD (instance store, **erased on stop/terminate**) |
| AWS-published peak (4 KiB, saturated queue depth) | 100,125 random read IOPS / 35,000 random write IOPS ([AWS docs: SSD instance store volumes](https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/ssd-instance-store.html)) |

Note that `i3.large` has only 2 vCPUs, so fio runs 2 workers by default. It
can become CPU-bound before the drive saturates, so treat its random-read
result as a lower bound for the drive.

### 6.1 Reproduce

```bash
# Amazon Linux 2023 on i3.large. The instance-store NVMe is the drive with no mountpoint.
lsblk -o NAME,MODEL,SIZE,MOUNTPOINT
sudo dnf install -y fio jq xfsprogs

sudo mkfs.xfs -L stellar-data /dev/nvme0n1
sudo mkdir -p /mnt/stellar-data
sudo mount -o noatime,inode64,logbsize=256k LABEL=stellar-data /mnt/stellar-data
echo none | sudo tee /sys/block/nvme0n1/queue/scheduler
sudo chown "$USER" /mnt/stellar-data

./examples/scripts/run-fio-benchmark.sh -d /mnt/stellar-data -p all -s 16G --label "i3.large baseline"
```

### 6.2 Measured output

<!--
  VALIDATION RUN: replace the block below with the verbatim contents of
  fio-results-*/summary.md produced by the command in §6.1 on an i3.large.
  Do not edit the numbers by hand.
-->

```text
PENDING: paste the verbatim summary.md from the §6.1 validation run here.
```

**How to read it.** Because the script uses a fixed queue depth rather than
saturating the queue, expect results at or below AWS's published peaks. Compare
the measured `rand-write-4k` against the validator minimum of 10,000 IOPS. If your own hardware scores well below
`i3.large` on `rand-write-4k` or `sync-write-4k`, it is a weaker validator host
than a two-vCPU cloud instance, and you should revisit §3 and §4 before going to
mainnet.

---

## 7. Deploying on Stellar-K8s with local NVMe

After the drive is tuned and passes the benchmark, expose it to the operator
with `spec.storage.mode: Local`.

### 7.1 How mount flags reach the pod

A Kubernetes `local` PersistentVolume **bind-mounts a directory that is already
mounted on the host**. The PV's `mountOptions` field is not applied to `local`
volumes. The flags from §4 therefore take effect only through the host's
`/etc/fstab` (or your provisioning tool: cloud-init, Ansible, a MachineConfig,
and so on). The scheduler comes from the host's udev rule (§3.3).

### 7.2 StorageClass and PersistentVolume

```yaml
apiVersion: storage.k8s.io/v1
kind: StorageClass
metadata:
  name: local-nvme
provisioner: kubernetes.io/no-provisioner
volumeBindingMode: WaitForFirstConsumer   # bind only once the pod is scheduled
reclaimPolicy: Retain
---
apiVersion: v1
kind: PersistentVolume
metadata:
  name: stellar-nvme-node-a
spec:
  capacity:
    storage: 400Gi                        # ≤ usable size of the host filesystem
  accessModes: ["ReadWriteOnce"]
  persistentVolumeReclaimPolicy: Retain
  storageClassName: local-nvme
  volumeMode: Filesystem
  local:
    path: /mnt/stellar-data               # the tuned mount from §4
  nodeAffinity:
    required:
      nodeSelectorTerms:
        - matchExpressions:
            - key: kubernetes.io/hostname
              operator: In
              values: ["node-a"]
```

To manage many hosts, the
[local static provisioner](https://github.com/kubernetes-sigs/sig-storage-local-static-provisioner)
can create these PVs automatically from mounts under a discovery directory.

### 7.3 StellarNode

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: validator-mainnet
  namespace: stellar
spec:
  nodeType: Validator
  network: mainnet
  historyMode: Recent
  storage:
    mode: Local
    storageClass: local-nvme
    size: "400Gi"
    retentionPolicy: Retain               # never auto-delete a local ledger DB
    nodeAffinity:
      requiredDuringSchedulingIgnoredDuringExecution:
        nodeSelectorTerms:
          - matchExpressions:
              - key: stellar.org/storage
                operator: In
                values: ["nvme-tuned"]
```

Label nodes only after they pass the benchmark:

```bash
kubectl label node node-a stellar.org/storage=nvme-tuned
```

Notes:

- If `storageClass` is left empty in `Local` mode, the operator falls back to
  an existing `local-path` or `local-storage` StorageClass. Setting it
  explicitly avoids landing on an untuned default path.
- Local volumes **cannot be expanded online**, and `diskScaling` does not
  apply to them. Size archive nodes with growth in mind
  ([capacity planning §3](../operations/capacity-planning.md#3-storage-growth-model)).
- Local data is tied to one host. Pair local NVMe with
  [volume snapshots / backups](../volume-snapshots.md) and the
  [disaster recovery runbook](../operations/disaster-recovery.md).

---

## 8. Production checklist

Before admitting a bare-metal NVMe host to mainnet:

- [ ] Dedicated NVMe drive for Stellar data. Validators use a drive with power-loss protection.
- [ ] Scheduler is `none` (or a measured `mq-deadline` for archive-only drives), persisted with a udev rule (§3.3).
- [ ] XFS (or ext4 for validators only) created with the §4 commands.
- [ ] Mounted with `noatime` through `/etc/fstab`. No option from §4.5 present (`findmnt /mnt/stellar-data`).
- [ ] TRIM enabled with exactly one method: `fstrim.timer` or `discard` (§4.6).
- [ ] `run-fio-benchmark.sh --strict` passes for the node's profile, and `summary.md` is kept with the host's records.
- [ ] Settings survive a reboot: reboot, check the scheduler and mount flags again, and rerun the benchmark.
- [ ] `nvme smart-log` shows `critical_warning: 0`, and drive wear (`percentage_used`) is monitored.
- [ ] Node labelled and a `StellarNode` with `storage.mode: Local` and `retentionPolicy: Retain` deployed (§7).

**Rollback.** Every change in this guide can be undone. Delete the udev rule
and reboot (or `echo mq-deadline > .../scheduler`) to restore the previous
scheduler. Remove mount flags from `/etc/fstab` and remount. Disable
`fstrim.timer`. None of these changes alter data already on disk.
