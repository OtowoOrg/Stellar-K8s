#!/usr/bin/env python3
"""Threshold sharding for Stellar validator seeds — see `docs/security/key-sharding.md`.

This script splits a Stellar ed25519 secret seed (`S...`) into `n` Shamir
Secret Sharing shares with a `k`-of-`n` reconstruction threshold, and recovers
the seed from any `k` of them. Recovery is proven correct end to end by
re-deriving the ed25519 public key from the reconstructed seed and comparing it
with the public key recorded inside every share: if the two differ, the shares
did not come from one coherent ceremony and nothing is emitted.

Design constraints (the full protocol lives in the runbook):

* **Standard library only.** The GF(256) Shamir implementation below is
  self-contained, so the script runs on an air-gapped host with no package
  installs and no edits to `requirements.txt`. `pip install shamirs` is *not*
  required (and is deliberately avoided — a supply-chain install inside a key
  ceremony is an unnecessary risk).
* **No plaintext at rest.** The seed is read from stdin or a `0600` file and is
  written only to stdout unless `--out-file` is passed; `--out-file` refuses a
  target that is not on a `tmpfs`/`ramfs` mount unless
  `--allow-plaintext-disk-write` is given explicitly.
* **No secret in argv.** Prefer stdin / `--seed-file`. `--seed` exists for
  tests and warns, because a command line is world-readable via `ps`.
* **Best-effort hygiene.** Core dumps are disabled and the process attempts
  `mlockall(MCL_CURRENT)` before key material is touched; secret buffers are
  zeroed before exit.

Subcommands
-----------
  generate   Create a fresh 32-byte Stellar seed (air-gapped host only).
  derive     Print the public key + fingerprint for a seed.
  split      Split a seed into `n` shares with threshold `k`.
  combine    Recover a seed from >= k shares and print its public key.
  verify     Like combine, but redacted: never prints the recovered seed.
  selftest   RFC 8032 + GF(256) + StrKey + end-to-end split/recover checks.

Exit codes
----------
  0  success / all checks passed
  1  usage, parse, or integrity (checksum) failure
  2  reconstructed seed does not match the public key recorded in the shares
"""

from __future__ import annotations

import argparse
import base64
import binascii
import ctypes
import dataclasses
import hashlib
import os
import secrets
import sys
from collections.abc import Callable, Iterable, Sequence
from dataclasses import dataclass
from pathlib import Path

PROG = "shamir-split.py"
SCHEME = "shamir-gf256"
FORMAT_VERSION = 1

# Stellar StrKey version bytes (18 << 3 == 0x90 for seeds, 6 << 3 == 0x30 for
# accounts) — same constants as src/controller/security/vault.rs.
SEED_VERSION_BYTE = 18 << 3
ACCOUNT_VERSION_BYTE = 6 << 3
STRKEY_PAYLOAD_LEN = 32
DEFAULT_SEED_ENV = "STELLAR_CORE_SEED"
SHARE_HEADER = "-----BEGIN STELLAR SSS SHARE-----"
SHARE_FOOTER = "-----END STELLAR SSS SHARE-----"
DEFAULT_FILE_PREFIX = "stellar-validator-seed"

EXIT_OK = 0
EXIT_ERROR = 1
EXIT_MISMATCH = 2

# Recorded per share so a new split over the same identity can be created later.
SHARE_KEY_ORDER = (
    "version",
    "scheme",
    "threshold",
    "total",
    "index",
    "label",
    "public-key",
    "fingerprint",
    "share",
    "crc16",
)
_SHARE_KEYS = frozenset(SHARE_KEY_ORDER)


class MismatchError(ValueError):
    """The share set does not reconstruct the identity it claims to."""


# ---------------------------------------------------------------------------
# Process hygiene
# ---------------------------------------------------------------------------

_MCL_CURRENT = 1


def harden_process() -> list[str]:
    """Best-effort hardening before key material is read.

    Returns human-readable notes so callers can surface what actually happened
    instead of silently assuming a protection is in place.
    """
    notes: list[str] = []

    try:  # A core dump would write the seed to disk.
        import resource

        resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
        notes.append("core dumps disabled (RLIMIT_CORE=0)")
    except (ImportError, OSError, ValueError) as exc:  # pragma: no cover - platform dependent
        notes.append(f"could not disable core dumps: {exc}")

    # MCL_CURRENT only: MCL_FUTURE can make later allocations fail and is not
    # worth the operational risk inside a ceremony.
    for libname in ("libc.so.6", "libSystem.B.dylib"):
        try:
            libc = ctypes.CDLL(libname, use_errno=True)
            if libc.mlockall(_MCL_CURRENT) == 0:
                notes.append("mlockall(MCL_CURRENT) succeeded")
            else:
                notes.append(
                    f"mlockall denied (errno={ctypes.get_errno()}); prefer the "
                    "Vault-backed assembly path for long-lived material"
                )
            break
        except (OSError, AttributeError, TypeError) as exc:
            notes.append(f"mlockall unavailable via {libname}: {exc}")
            continue
    else:  # pragma: no cover - platform dependent
        notes.append("mlockall unavailable on this platform")

    return notes


def zero(buf: bytearray) -> None:
    """Overwrite a mutable secret buffer in place.

    CPython does not optimise this away, but note that an immutable `bytes` copy
    cannot be wiped — that is exactly why the plaintext seed must live in tmpfs
    (or, better, only ever inside Vault Transit) for the duration of a ceremony.
    """
    for i in range(len(buf)):
        buf[i] = 0


# ---------------------------------------------------------------------------
# CRC16-XModem, base32 and Stellar StrKey
# ---------------------------------------------------------------------------


def crc16_xmodem(data: bytes) -> int:
    """CRC-16/XMODEM: poly 0x1021, init 0x0000, no reflection, xorout 0."""
    crc = 0
    for byte in data:
        crc ^= byte << 8
        for _ in range(8):
            if crc & 0x8000:
                crc = ((crc << 1) ^ 0x1021) & 0xFFFF
            else:
                crc = (crc << 1) & 0xFFFF
    return crc


def encode_strkey(version_byte: int, payload: bytes) -> str:
    if len(payload) != STRKEY_PAYLOAD_LEN:
        raise ValueError("StrKey payload must be 32 bytes")
    body = bytes([version_byte]) + payload
    checksum = crc16_xmodem(body).to_bytes(2, "little")
    return base64.b32encode(body + checksum).decode("ascii").rstrip("=")


def decode_strkey(strkey: str, expected_version: int) -> bytes:
    text = strkey.strip()
    padding = "=" * ((8 - len(text) % 8) % 8)
    try:
        raw = base64.b32decode(text.upper() + padding, casefold=True)
    except (binascii.Error, ValueError) as exc:
        raise ValueError(f"invalid StrKey base32: {exc}") from exc
    if len(raw) != STRKEY_PAYLOAD_LEN + 3:
        raise ValueError(
            f"invalid StrKey length: expected {STRKEY_PAYLOAD_LEN + 3} bytes, got {len(raw)}"
        )
    if raw[0] != expected_version:
        raise ValueError(
            f"invalid StrKey version byte: 0x{raw[0]:02x} (expected 0x{expected_version:02x})"
        )
    checksum = int.from_bytes(raw[-2:], "little")
    if crc16_xmodem(raw[:-2]) != checksum:
        raise ValueError("invalid StrKey checksum")
    return raw[1:-2]


def seed_fingerprint(seed_strkey: str) -> str:
    """SHA-256 of the seed StrKey — the audit-safe handle used by the operator.

    Mirrors `seed_fingerprint()` in `src/controller/security/vault.rs`. The seed
    is 256 bits of CSPRNG output, so publishing its digest does not weaken it:
    it only enables correlated audit logs and share-set verification.
    """
    if len(seed_strkey) != 56 or not seed_strkey.startswith("S"):
        raise ValueError("fingerprint requires a Stellar secret seed StrKey")
    return hashlib.sha256(seed_strkey.encode("ascii")).hexdigest()


# ---------------------------------------------------------------------------
# GF(256) arithmetic — AES polynomial x^8 + x^4 + x^3 + x + 1 (0x11B), g = 0x03
# ---------------------------------------------------------------------------

_GF_POLY_LOW = 0x1B  # 0x11B without the implicit x^8 term


def _gf_mul_slow(a: int, b: int) -> int:
    """Carry-less multiply modulo the AES polynomial. Used to build the tables."""
    product = 0
    for _ in range(8):
        if b & 1:
            product ^= a
        carry = a & 0x80
        a = (a << 1) & 0xFF
        if carry:
            a ^= _GF_POLY_LOW
        b >>= 1
    return product


def _build_tables() -> tuple[list[int], list[int]]:
    exp = [0] * 512
    log = [0] * 256
    value = 1
    for i in range(255):
        exp[i] = value
        log[value] = i
        value = _gf_mul_slow(value, 0x03)  # 0x03 is primitive for 0x11B
    if value != 1:
        raise RuntimeError("GF(256) table build failed: 0x03 is not primitive for 0x11B")
    for i in range(255, 512):
        exp[i] = exp[i - 255]
    return exp, log


_GF_EXP, _GF_LOG = _build_tables()


def gf_mul(a: int, b: int) -> int:
    if a == 0 or b == 0:
        return 0
    return _GF_EXP[_GF_LOG[a] + _GF_LOG[b]]


def gf_div(a: int, b: int) -> int:
    """a / b in GF(256). Raises ZeroDivisionError when b == 0."""
    if b == 0:
        raise ZeroDivisionError("division by zero in GF(256)")
    if a == 0:
        return 0
    return _GF_EXP[(_GF_LOG[a] - _GF_LOG[b]) % 255]


def _eval_poly(coefficients: Sequence[int], x: int) -> int:
    """Horner evaluation with the highest-degree coefficient last."""
    acc = coefficients[-1]
    for coefficient in reversed(coefficients[:-1]):
        acc = gf_mul(acc, x) ^ coefficient
    return acc


# ---------------------------------------------------------------------------
# Shamir Secret Sharing over GF(256)
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class Share:
    index: int  # 1..255: the non-zero x coordinate
    threshold: int
    total: int
    payload: bytes
    label: str = ""
    public_key: str = ""
    fingerprint: str = ""


def validate_threshold(threshold: int, total: int) -> None:
    if not 2 <= threshold <= total:
        raise ValueError(f"threshold must satisfy 2 <= k <= n (got k={threshold}, n={total})")
    if total > 255:
        raise ValueError("total shares must be <= 255 (GF(256) x coordinates are non-zero)")


def split_secret(
    secret: bytes,
    threshold: int,
    total: int,
    randbytes: Callable[[int], bytes] = secrets.token_bytes,
) -> list[Share]:
    """Split `secret` into `total` shares, any `threshold` of which recover it.

    Each byte of the secret is shared independently with a random degree
    `threshold - 1` polynomial over GF(256), so `threshold - 1` shares reveal
    nothing at all about the secret — not even its length pattern.
    """
    validate_threshold(threshold, total)
    if not secret:
        raise ValueError("refusing to split an empty secret")

    # coefficients[i] = [a_0 = secret byte, a_1, ..., a_{k-1}]
    coefficients = [[byte, *randbytes(threshold - 1)] for byte in secret]

    shares: list[Share] = []
    for x in range(1, total + 1):
        payload = bytes(_eval_poly(poly, x) for poly in coefficients)
        shares.append(Share(index=x, threshold=threshold, total=total, payload=payload))
    return shares


def combine_shares(shares: Sequence[Share]) -> bytes:
    """Recover the secret from >= threshold shares via Lagrange at x = 0."""
    if not shares:
        raise ValueError("no shares supplied")
    threshold = shares[0].threshold
    if len(shares) < threshold:
        raise ValueError(f"need at least {threshold} shares to reconstruct, got {len(shares)}")

    indexes = [share.index for share in shares]
    if len(set(indexes)) != len(indexes):
        raise ValueError("duplicate share indexes supplied")
    if any(index == 0 for index in indexes):
        raise ValueError("share index 0 is invalid (x = 0 holds the secret)")

    length = len(shares[0].payload)
    if any(len(share.payload) != length for share in shares):
        raise ValueError("shares have inconsistent payload lengths")

    recovered = bytearray(length)
    try:
        for i, share in enumerate(shares):
            numerator = 1
            denominator = 1
            for j, other in enumerate(shares):
                if i == j:
                    continue
                numerator = gf_mul(numerator, other.index)
                denominator = gf_mul(denominator, share.index ^ other.index)  # - is XOR in GF(2^m)
            factor = gf_div(numerator, denominator)
            if factor == 0:
                raise ValueError("degenerate share set: a Lagrange factor was zero")
            for byte_index in range(length):
                recovered[byte_index] ^= gf_mul(share.payload[byte_index], factor)
        return bytes(recovered)
    finally:
        zero(recovered)


# ---------------------------------------------------------------------------
# Ed25519 public key derivation (RFC 8032 reference arithmetic, stdlib only)
# ---------------------------------------------------------------------------

_P = 2**255 - 19
_D = (-121665 * pow(121666, _P - 2, _P)) % _P
_SQRT_M1 = pow(2, (_P - 1) // 4, _P)


def _recover_x(y: int, sign: int) -> int | None:
    xx = (y * y - 1) * pow(_D * y * y + 1, _P - 2, _P) % _P
    x = pow(xx, (_P + 3) // 8, _P)
    if (x * x - xx) % _P != 0:
        x = x * _SQRT_M1 % _P
    if (x * x - xx) % _P != 0:
        return None
    if x & 1 != sign:
        x = _P - x
    return x


_BY = 4 * pow(5, _P - 2, _P) % _P
_BX = _recover_x(_BY, 0)
if _BX is None:  # pragma: no cover - impossible for the canonical base point
    raise RuntimeError("could not recover the Ed25519 base point x coordinate")
_BASE = (_BX, _BY, 1, _BX * _BY % _P)
_NEUTRAL = (0, 1, 1, 0)


def _point_add(p: tuple[int, int, int, int], q: tuple[int, int, int, int]):
    a = (p[1] - p[0]) * (q[1] - q[0]) % _P
    b = (p[1] + p[0]) * (q[1] + q[0]) % _P
    c = 2 * p[3] * q[3] * _D % _P
    d = 2 * p[2] * q[2] % _P
    e, f, g, h = b - a, d - c, d + c, b + a
    return (e * f % _P, g * h % _P, f * g % _P, e * h % _P)


def _point_mul(scalar: int, point: tuple[int, int, int, int]):
    result = _NEUTRAL
    addend = point
    while scalar > 0:
        if scalar & 1:
            result = _point_add(result, addend)
        addend = _point_add(addend, addend)
        scalar >>= 1
    return result


def _point_compress(point: tuple[int, int, int, int]) -> bytes:
    inverse_z = pow(point[2], _P - 2, _P)
    x = point[0] * inverse_z % _P
    y = point[1] * inverse_z % _P
    return (y | ((x & 1) << 255)).to_bytes(32, "little")


def ed25519_public_key(seed: bytes) -> bytes:
    """RFC 8032 §5.1.5: A = [clamp(SHA-512(seed)[:32])] * B."""
    if len(seed) != STRKEY_PAYLOAD_LEN:
        raise ValueError("ed25519 seed must be 32 bytes")
    digest = hashlib.sha512(seed).digest()
    scalar = int.from_bytes(digest[:32], "little")
    scalar &= (1 << 254) - 8
    scalar |= 1 << 254
    return _point_compress(_point_mul(scalar, _BASE))


# ---------------------------------------------------------------------------
# Share record (PEM-like, transcription friendly)
# ---------------------------------------------------------------------------


def _share_preimage(threshold: int, total: int, index: int, payload: bytes) -> bytes:
    return b"SSS1" + bytes([threshold, total, index]) + payload


def render_share(share: Share) -> str:
    payload = base64.b32encode(share.payload).decode("ascii").rstrip("=")
    crc = crc16_xmodem(_share_preimage(share.threshold, share.total, share.index, share.payload))
    fields = {
        "version": str(FORMAT_VERSION),
        "scheme": SCHEME,
        "threshold": str(share.threshold),
        "total": str(share.total),
        "index": str(share.index),
        "label": share.label,
        "public-key": share.public_key,
        "fingerprint": share.fingerprint,
        "share": payload,
        "crc16": f"{crc:04x}",
    }
    body = "\n".join(f"{key}: {fields[key]}" for key in SHARE_KEY_ORDER)
    return f"{SHARE_HEADER}\n{body}\n{SHARE_FOOTER}\n"


def parse_share(text: str, source: str = "<memory>") -> Share:
    fields: dict[str, str] = {}
    for line in text.splitlines():
        stripped = line.strip()
        if not stripped or stripped in (SHARE_HEADER, SHARE_FOOTER) or stripped.startswith("#"):
            continue
        if ":" not in stripped:
            continue
        key, _, value = stripped.partition(":")
        key = key.strip().lower()
        if key in _SHARE_KEYS:
            fields[key] = value.strip()

    missing = [key for key in SHARE_KEY_ORDER if key not in fields]
    if missing:
        raise ValueError(f"{source}: missing share field(s): {', '.join(missing)}")
    if fields["version"] != str(FORMAT_VERSION):
        raise ValueError(f"{source}: unsupported share version {fields['version']!r}")
    if fields["scheme"] != SCHEME:
        raise ValueError(f"{source}: unsupported scheme {fields['scheme']!r}")

    try:
        threshold = int(fields["threshold"])
        total = int(fields["total"])
        index = int(fields["index"])
    except ValueError as exc:
        raise ValueError(f"{source}: threshold/total/index must be integers") from exc
    validate_threshold(threshold, total)
    if not 1 <= index <= total:
        raise ValueError(f"{source}: index {index} is outside 1..{total}")

    payload_text = fields["share"].replace("-", "").replace(" ", "")
    padding = "=" * ((8 - len(payload_text) % 8) % 8)
    try:
        payload = base64.b32decode(payload_text.upper() + padding, casefold=True)
    except (binascii.Error, ValueError) as exc:
        raise ValueError(f"{source}: share payload is not valid base32: {exc}") from exc
    if len(payload) != STRKEY_PAYLOAD_LEN:
        raise ValueError(
            f"{source}: share payload must be {STRKEY_PAYLOAD_LEN} bytes, got {len(payload)}"
        )

    try:
        recorded_crc = int(fields["crc16"], 16)
    except ValueError as exc:
        raise ValueError(f"{source}: crc16 must be hexadecimal") from exc
    actual_crc = crc16_xmodem(_share_preimage(threshold, total, index, payload))
    if recorded_crc != actual_crc:
        raise ValueError(
            f"{source}: crc16 mismatch (recorded {recorded_crc:04x}, computed {actual_crc:04x}) "
            "— the share was transcribed or stored incorrectly"
        )

    public_key = fields["public-key"]
    if not public_key.startswith("G") or len(public_key) != 56:
        raise ValueError(f"{source}: public-key is not a Stellar account id")
    try:
        decode_strkey(public_key, ACCOUNT_VERSION_BYTE)
    except ValueError as exc:
        raise ValueError(f"{source}: public-key fails StrKey validation: {exc}") from exc

    fingerprint = fields["fingerprint"].lower()
    if len(fingerprint) != 64 or any(c not in "0123456789abcdef" for c in fingerprint):
        raise ValueError(f"{source}: fingerprint must be a 64-character SHA-256 hex digest")

    return Share(
        index=index,
        threshold=threshold,
        total=total,
        payload=payload,
        label=fields["label"],
        public_key=public_key,
        fingerprint=fingerprint,
    )


def _split_share_blocks(text: str) -> list[str]:
    blocks: list[str] = []
    current: list[str] = []
    inside = False
    for line in text.splitlines():
        if line.strip() == SHARE_HEADER:
            inside = True
            current = [line]
        elif line.strip() == SHARE_FOOTER and inside:
            current.append(line)
            blocks.append("\n".join(current))
            current = []
            inside = False
        elif inside:
            current.append(line)
    return blocks


def load_share_sources(paths: Iterable[str]) -> list[Share]:
    """Load shares from files, or from stdin when the path is `-`."""
    shares: list[Share] = []
    for path in paths:
        if path == "-":
            blocks = _split_share_blocks(sys.stdin.read())
            if not blocks:
                raise ValueError("<stdin>: no share block found")
            for offset, block in enumerate(blocks, start=1):
                shares.append(parse_share(block, f"<stdin>#{offset}"))
            continue
        source = Path(path)
        try:
            text = source.read_text(encoding="ascii")
        except FileNotFoundError as exc:
            raise ValueError(f"{path}: no such share file") from exc
        blocks = _split_share_blocks(text)
        if blocks:
            for offset, block in enumerate(blocks, start=1):
                origin = f"{path}#{offset}" if len(blocks) > 1 else path
                shares.append(parse_share(block, origin))
        else:
            shares.append(parse_share(text, path))
    return shares


# ---------------------------------------------------------------------------
# Seed input / plaintext output
# ---------------------------------------------------------------------------


def extract_seed_token(raw_text: str) -> str:
    """Return the seed StrKey from raw text, or `""` when none is present.

    Accepts both a bare StrKey and the labelled record emitted by `generate`
    (`seed: S…` / `public-key: G…` / `fingerprint: …`), so that the documented
    `generate | split` pipeline round-trips without a temporary plaintext file.
    """
    for line in raw_text.splitlines():
        candidate = line.strip()
        if not candidate or candidate.startswith("#"):
            continue
        if ":" in candidate:
            label, _, value = candidate.partition(":")
            if label.strip().lower() != "seed":
                continue  # e.g. "public-key:" / "fingerprint:" lines
            candidate = value.strip()
            if not candidate:
                continue
        return candidate.split()[0]
    return ""


def read_seed(args: argparse.Namespace) -> tuple[str, bytearray]:
    """Return (seed StrKey, raw seed bytes as a mutable, zeroable buffer)."""
    if args.seed is not None:
        print(
            f"{PROG}: warning: --seed puts the secret in this process's argv, which "
            "is readable by other local users via `ps`; prefer stdin or --seed-file",
            file=sys.stderr,
        )
        raw_text = args.seed
    elif args.seed_file is not None:
        path = Path(args.seed_file)
        mode = path.stat().st_mode & 0o777
        if mode & 0o077:
            print(
                f"{PROG}: warning: {path} is mode {mode:04o}; restrict it to 0600",
                file=sys.stderr,
            )
        raw_text = path.read_text(encoding="ascii")
    elif args.seed_env:
        value = os.environ.get(DEFAULT_SEED_ENV)
        if value is None:
            raise ValueError(f"environment variable {DEFAULT_SEED_ENV} is not set")
        raw_text = value
    else:
        raw_text = sys.stdin.read()

    seed = extract_seed_token(raw_text)
    if not seed:
        raise ValueError("no seed supplied (expected a Stellar secret seed on stdin)")

    raw = bytearray(decode_strkey(seed, SEED_VERSION_BYTE))
    return seed, raw


def is_tmpfs(path: Path) -> bool | None:
    """True/False when it can be determined, None when /proc/mounts is absent."""
    mounts = Path("/proc/mounts")
    if not mounts.exists():
        return None
    try:
        target = str(path.resolve())
    except OSError:
        return None
    best: tuple[int, str] | None = None
    for line in mounts.read_text(encoding="utf-8", errors="replace").splitlines():
        parts = line.split()
        if len(parts) < 3:
            continue
        mount_point, fstype = parts[1], parts[2]
        prefix = mount_point.rstrip("/")
        matches = target == mount_point or prefix == "" or target.startswith(prefix + "/")
        if matches and (best is None or len(mount_point) > best[0]):
            best = (len(mount_point), fstype)
    if best is None:
        return False
    return best[1] in ("tmpfs", "ramfs")


def write_plaintext(destination: str, text: str, allow_disk: bool) -> None:
    """Write plaintext seed material, refusing anything that is not RAM-backed."""
    if destination == "-":
        sys.stdout.write(text if text.endswith("\n") else text + "\n")
        return

    path = Path(destination)
    verdict = is_tmpfs(path if path.exists() else path.parent)
    if verdict is not True and not allow_disk:
        detail = (
            "is not on a tmpfs/ramfs mount"
            if verdict is False
            else "could not be confirmed as RAM-backed (no /proc/mounts)"
        )
        raise ValueError(
            f"refusing to write plaintext seed to {destination}: {detail}. "
            "Use /dev/shm, or pass --allow-plaintext-disk-write if you accept "
            "that the seed will persist on this machine."
        )

    fd = os.open(destination, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    try:
        os.write(fd, text.encode("ascii"))
        os.fsync(fd)
    finally:
        os.close(fd)


def write_share_files(out_dir: Path, shares: Sequence[Share], prefix: str) -> list[Path]:
    out_dir.mkdir(parents=True, exist_ok=True)
    written: list[Path] = []
    for share in shares:
        target = out_dir / f"{prefix}-{share.index}.share"
        fd = os.open(target, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        try:
            os.write(fd, render_share(share).encode("ascii"))
        finally:
            os.close(fd)
        written.append(target)
    return written


# ---------------------------------------------------------------------------
# Subcommands
# ---------------------------------------------------------------------------


def cmd_generate(args: argparse.Namespace) -> int:
    harden_process()
    raw = bytearray(secrets.token_bytes(STRKEY_PAYLOAD_LEN))
    try:
        seed = encode_strkey(SEED_VERSION_BYTE, raw)
        public = encode_strkey(ACCOUNT_VERSION_BYTE, ed25519_public_key(bytes(raw)))
        write_plaintext(
            args.out_file or "-",
            f"seed: {seed}\npublic-key: {public}\nfingerprint: {seed_fingerprint(seed)}\n",
            args.allow_plaintext_disk_write,
        )
        print(
            f"{PROG}: generated a fresh validator seed. Equivalent to "
            "`stellar-core gen-seed`; run it only on the air-gapped host. "
            "See docs/security/key-sharding.md.",
            file=sys.stderr,
        )
    finally:
        zero(raw)
    return EXIT_OK


def cmd_derive(args: argparse.Namespace) -> int:
    harden_process()
    seed, raw = read_seed(args)
    try:
        public = encode_strkey(ACCOUNT_VERSION_BYTE, ed25519_public_key(bytes(raw)))
    finally:
        zero(raw)
    sys.stdout.write(f"public-key: {public}\nfingerprint: {seed_fingerprint(seed)}\n")
    return EXIT_OK


def cmd_split(args: argparse.Namespace) -> int:
    harden_process()
    seed, raw = read_seed(args)
    try:
        public = encode_strkey(ACCOUNT_VERSION_BYTE, ed25519_public_key(bytes(raw)))
        fingerprint = seed_fingerprint(seed)
        shares = split_secret(bytes(raw), args.threshold, args.shares)
    finally:
        zero(raw)

    labels = [f"{args.label}-{share.index}" for share in shares]
    if args.labels:
        custom = [item.strip() for item in args.labels.split(",") if item.strip()]
        if len(custom) != len(shares):
            raise ValueError(f"--labels must list exactly {len(shares)} names, got {len(custom)}")
        labels = custom

    shares = [
        dataclasses.replace(
            share, label=labels[share.index - 1], public_key=public, fingerprint=fingerprint
        )
        for share in shares
    ]

    if args.out_dir:
        for share, path in zip(shares, write_share_files(Path(args.out_dir), shares, args.file_prefix)):
            print(f"{PROG}: wrote share {share.index}/{share.total} -> {path}", file=sys.stderr)
    else:
        for share in shares:
            sys.stdout.write(render_share(share))
            sys.stdout.write("\n")

    print(
        f"{PROG}: {args.threshold}-of-{args.shares} split complete "
        f"public-key={public} fingerprint={fingerprint} — store one share per custodian; "
        f"fewer than {args.threshold} shares reveal nothing",
        file=sys.stderr,
    )
    return EXIT_OK


def _reconstruct(shares: Sequence[Share]) -> tuple[str, str, bytearray]:
    """Validate a share set and return (seed, public_key, raw seed buffer)."""
    if not shares:
        raise ValueError("no shares supplied")

    identities = {share.public_key for share in shares}
    if len(identities) != 1:
        raise MismatchError(
            "shares carry different public keys — they belong to different ceremonies: "
            + ", ".join(sorted(identities))
        )
    fingerprints = {share.fingerprint for share in shares}
    if len(fingerprints) != 1:
        raise MismatchError("shares carry different seed fingerprints — mixed ceremonies")
    thresholds = {share.threshold for share in shares}
    if len(thresholds) != 1:
        raise ValueError(f"shares disagree on the threshold: {sorted(thresholds)}")
    if len(shares) < shares[0].threshold:
        raise ValueError(
            f"need at least {shares[0].threshold} shares, got {len(shares)}"
        )

    recovered = combine_shares(shares)
    seed = encode_strkey(SEED_VERSION_BYTE, recovered)
    derived = encode_strkey(ACCOUNT_VERSION_BYTE, ed25519_public_key(recovered))
    if derived != shares[0].public_key:
        raise MismatchError(
            "reconstructed seed derives a different public key than the shares record: "
            f"derived={derived} recorded={shares[0].public_key}"
        )
    actual_fingerprint = seed_fingerprint(seed)
    if actual_fingerprint != shares[0].fingerprint:
        raise MismatchError(
            "reconstructed seed fingerprint does not match the shares record: "
            f"derived={actual_fingerprint} recorded={shares[0].fingerprint}"
        )
    return seed, derived, bytearray(recovered)


def cmd_combine(args: argparse.Namespace) -> int:
    harden_process()
    shares = load_share_sources(args.shares)
    seed, public, raw = _reconstruct(shares)
    try:
        if args.out_file:
            write_plaintext(
                args.out_file,
                f"seed: {seed}\n",
                args.allow_plaintext_disk_write,
            )
        elif not args.redact:
            write_plaintext(
                "-",
                f"seed: {seed}\npublic-key: {public}\nfingerprint: {seed_fingerprint(seed)}\n",
                args.allow_plaintext_disk_write,
            )
            print(
                f"{PROG}: reconstructed the seed from {len(shares)} shares — keep it in "
                "memory only (tmpfs), then wipe the shell history",
                file=sys.stderr,
            )
        if args.redact:
            sys.stdout.write(
                f"public-key: {public}\n"
                f"fingerprint: {seed_fingerprint(seed)}\n"
                f"shares-used: {len(shares)}\n"
                f"threshold: {shares[0].threshold}\n"
            )
    finally:
        zero(raw)
    return EXIT_OK


# ---------------------------------------------------------------------------
# Self test
# ---------------------------------------------------------------------------

# RFC 8032 §7.1 TEST 1 — deterministic proof that the Ed25519 arithmetic is right.
_RFC8032_TEST1_SEED = bytes.fromhex(
    "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60"
)
_RFC8032_TEST1_PUBLIC = bytes.fromhex(
    "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
)

# StrKey vector from stellar/js-stellar-base `test/unit/strkey_test.js`. It must
# decode *and* re-encode byte-identically; any drift in the base32 alphabet, the
# version byte, or CRC-16/XMODEM breaks the round trip.
_STRKEY_REFERENCE_ACCOUNT = "GBPXXOA5N4JYPESHAADMQKBPWZWQDQ64ZV6ZL2S3LAGW4SY7NTCMWIVL"

# Keypair.master("Test SDF Network ; September 2015"): the canonical test-network
# root key. The secret half is *derived* from the public passphrase, so no secret
# literal appears in this file, while the public half is an externally published
# constant (live on Stellar testnet Horizon, sequence 154 at time of writing).
_TESTNET_PASSPHRASE = b"Test SDF Network ; September 2015"
_TESTNET_MASTER_ACCOUNT = "GBRPYHIL2CI3FNQ4BXLFMNDLFJUNPU2HY3ZMFSHONUCEOASW7QC7OX2H"


def _deterministic_randbytes(counter: list[int]) -> Callable[[int], bytes]:
    def randbytes(n: int) -> bytes:
        counter[0] += 1
        digest = hashlib.sha256(b"shamir-split-selftest" + counter[0].to_bytes(4, "big")).digest()
        return digest[:n]

    return randbytes


class _SelfTest:
    def __init__(self) -> None:
        self.failures = 0

    def check(self, condition: bool, description: str) -> None:
        if not condition:
            self.failures += 1
        print(f"  [{'PASS' if condition else 'FAIL'}] {description}")

    def rejects(self, description: str, call: Callable[[], object]) -> None:
        try:
            call()
        except (ValueError, ZeroDivisionError):
            self.check(True, description)
        else:
            self.check(False, description)

    def rejects_as_mismatch(self, description: str, call: Callable[[], object]) -> None:
        """Assert the call fails with the exit-2 class (`MismatchError`)."""
        try:
            call()
        except MismatchError:
            self.check(True, description)
        except ValueError as exc:
            self.check(False, f"{description} (raised {type(exc).__name__}, want MismatchError)")
        else:
            self.check(False, description)


def cmd_selftest(_args: argparse.Namespace) -> int:
    test = _SelfTest()
    print(f"{PROG}: self test")

    print("Stellar StrKey codec")
    raw = bytes(range(32))
    seed_strkey = encode_strkey(SEED_VERSION_BYTE, raw)
    public_strkey = encode_strkey(ACCOUNT_VERSION_BYTE, raw)
    test.check(seed_strkey.startswith("S") and len(seed_strkey) == 56, "seed is an S-address, 56 chars")
    test.check(public_strkey.startswith("G") and len(public_strkey) == 56, "account is a G-address, 56 chars")
    test.check(decode_strkey(seed_strkey, SEED_VERSION_BYTE) == raw, "seed StrKey round-trips")
    decoded_reference = decode_strkey(_STRKEY_REFERENCE_ACCOUNT, ACCOUNT_VERSION_BYTE)
    test.check(
        encode_strkey(ACCOUNT_VERSION_BYTE, decoded_reference) == _STRKEY_REFERENCE_ACCOUNT,
        "js-stellar-base reference vector decodes and re-encodes byte-identically",
    )
    corrupted = seed_strkey[:-1] + ("A" if seed_strkey[-1] != "A" else "B")
    test.rejects("corrupted StrKey checksum is rejected", lambda: decode_strkey(corrupted, SEED_VERSION_BYTE))

    print("Ed25519 (RFC 8032 §7.1 TEST 1)")
    test.check(
        ed25519_public_key(_RFC8032_TEST1_SEED) == _RFC8032_TEST1_PUBLIC,
        "public key matches the published RFC 8032 vector",
    )

    print("GF(256) field axioms")
    mismatches = 0
    for a in (1, 2, 3, 0x1B, 0x53, 0xFF):
        for b in (1, 2, 0x11, 0x80, 0xFF):
            if gf_mul(a, b) != _gf_mul_slow(a, b):
                mismatches += 1
            if gf_mul(a, gf_div(b, a)) != b:
                mismatches += 1
    test.check(mismatches == 0, "log/exp tables agree with the schoolbook multiply")
    test.check(_GF_EXP[255] == 1 and _GF_LOG[0x03] == 1, "generator 0x03 has multiplicative order 255")
    test.rejects("division by zero is refused", lambda: gf_div(1, 0))

    print("Shamir split/recover (3-of-5, every 3-subset)")
    secret = hashlib.sha256(b"selftest-secret").digest()
    shares = split_secret(secret, 3, 5, randbytes=_deterministic_randbytes([0]))
    subsets = 0
    recovered_ok = True
    for i in range(5):
        for j in range(i + 1, 5):
            for k in range(j + 1, 5):
                subsets += 1
                if combine_shares([shares[i], shares[j], shares[k]]) != secret:
                    recovered_ok = False
    test.check(recovered_ok and subsets == 10, f"all {subsets} 3-share subsets recover the secret")
    test.check(
        combine_shares([shares[2], shares[0], shares[4]]) == combine_shares([shares[0], shares[2], shares[4]]),
        "recovery is independent of share order",
    )
    test.rejects("fewer than threshold shares is refused", lambda: combine_shares(shares[:2]))
    test.rejects(
        "duplicate share indexes are refused",
        lambda: combine_shares([shares[0], shares[0], shares[1]]),
    )
    test.rejects(
        "a single share leaks no secret (nothing to interpolate)",
        lambda: combine_shares([shares[0]]),
    )

    print("Share record integrity")
    expected_public = encode_strkey(ACCOUNT_VERSION_BYTE, ed25519_public_key(secret))
    labelled = [
        dataclasses.replace(
            share,
            public_key=expected_public,
            fingerprint=seed_fingerprint(encode_strkey(SEED_VERSION_BYTE, secret)),
        )
        for share in shares
    ]
    rendered = render_share(labelled[0])
    reparsed = parse_share(rendered, "selftest")
    test.check(reparsed.payload == labelled[0].payload, "share record round-trips")
    test.check(
        reparsed.public_key == expected_public and reparsed.fingerprint == labelled[0].fingerprint,
        "public key and fingerprint survive the round trip",
    )
    test.rejects(
        "tampered metadata is caught by crc16",
        lambda: parse_share(rendered.replace("index: 1", "index: 2", 1), "selftest"),
    )
    test.rejects(
        "a tampered checksum field is caught",
        lambda: parse_share(
            rendered.replace(
                f"crc16: {crc16_xmodem(_share_preimage(3, 5, 1, labelled[0].payload)):04x}",
                "crc16: 0000",
                1,
            ),
            "selftest",
        ),
    )
    test.rejects(
        "a truncated share payload is caught",
        lambda: parse_share(rendered.replace("share: ", "share: A", 1), "selftest"),
    )

    print("End-to-end: split -> recover -> identical ed25519 public key")
    shares = split_secret(secret, 3, 5, randbytes=_deterministic_randbytes([0]))
    labelled = [
        dataclasses.replace(
            share,
            public_key=expected_public,
            fingerprint=seed_fingerprint(encode_strkey(SEED_VERSION_BYTE, secret)),
        )
        for share in shares
    ]
    recovered = combine_shares([labelled[4], labelled[0], labelled[3]])
    test.check(recovered == secret, "recovered bytes are byte-identical to the seed")
    test.check(
        encode_strkey(ACCOUNT_VERSION_BYTE, ed25519_public_key(recovered)) == expected_public,
        f"recovered public key equals the original ({expected_public})",
    )
    seed_strkey = encode_strkey(SEED_VERSION_BYTE, recovered)
    test.check(
        seed_fingerprint(seed_strkey)
        == seed_fingerprint(encode_strkey(SEED_VERSION_BYTE, secret)),
        "recovered seed fingerprint equals the original",
    )
    test.check(
        _reconstruct([labelled[0], labelled[2], labelled[4]])[1] == expected_public,
        "_reconstruct() verifies the recovered identity",
    )
    test.rejects(
        "an incoherent share set (mixed public keys) is refused",
        lambda: _reconstruct(
            [labelled[0], labelled[1], dataclasses.replace(labelled[2], public_key=public_strkey)]
        ),
    )
    test.rejects_as_mismatch(
        "a mixed-ceremony share set fails with the exit-2 (MismatchError) class",
        lambda: _reconstruct(
            [labelled[0], labelled[1], dataclasses.replace(labelled[2], public_key=public_strkey)]
        ),
    )
    test.rejects_as_mismatch(
        "shares whose recorded fingerprint disagrees fail with MismatchError",
        lambda: _reconstruct(
            [labelled[0], labelled[1], dataclasses.replace(labelled[2], fingerprint="0" * 64)]
        ),
    )
    test.check(
        extract_seed_token(seed_strkey) == seed_strkey,
        "a bare seed StrKey parses as itself",
    )
    test.check(
        extract_seed_token(
            f"seed: {seed_strkey}\npublic-key: {expected_public}\nfingerprint: {'a' * 64}\n"
        )
        == seed_strkey,
        "the generate record round-trips through extract_seed_token (generate | split)",
    )
    test.check(
        extract_seed_token("public-key: GAAZI4TCR3TY5OJHCTJC2A4QSY6CJWJH5IAJTGKIN2ER7LBNVKOCCWN\n") == "",
        "a record with no seed line is rejected rather than misread",
    )

    print("End-to-end: canonical test-network master key")
    master_seed_raw = hashlib.sha256(_TESTNET_PASSPHRASE).digest()
    master_seed = encode_strkey(SEED_VERSION_BYTE, master_seed_raw)
    master_public = encode_strkey(ACCOUNT_VERSION_BYTE, ed25519_public_key(master_seed_raw))
    test.check(
        master_public == _TESTNET_MASTER_ACCOUNT,
        f"SHA-256(passphrase) derives the published testnet master account {_TESTNET_MASTER_ACCOUNT}",
    )
    master_shares = split_secret(master_seed_raw, 3, 5, randbytes=_deterministic_randbytes([7]))
    master_labelled = [
        dataclasses.replace(share, public_key=master_public, fingerprint=seed_fingerprint(master_seed))
        for share in master_shares
    ]
    recovered_seed, recovered_public, recovered_raw = _reconstruct(
        [master_labelled[1], master_labelled[3], master_labelled[4]]
    )
    try:
        test.check(recovered_seed == master_seed, "3-of-5 recovery returns the master seed StrKey")
        test.check(
            recovered_public == _TESTNET_MASTER_ACCOUNT,
            "recovered public key equals the published master account",
        )
        test.check(
            seed_fingerprint(recovered_seed) == seed_fingerprint(master_seed),
            "recovered fingerprint equals the original",
        )
    finally:
        zero(recovered_raw)

    verdict = "all checks passed" if test.failures == 0 else f"{test.failures} check(s) FAILED"
    print(f"{PROG}: {verdict}")
    return EXIT_OK if test.failures == 0 else EXIT_ERROR


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def add_seed_input(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--seed", help="seed StrKey (warns: visible to `ps`)")
    parser.add_argument("--seed-file", help="file containing the seed StrKey (prefer mode 0600)")
    parser.add_argument(
        "--seed-env",
        action="store_true",
        help=f"read the seed from ${DEFAULT_SEED_ENV} instead of stdin",
    )


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog=PROG,
        description=(
            "k-of-n Shamir sharding for Stellar validator seeds. "
            "See docs/security/key-sharding.md for the full ceremony."
        ),
    )
    sub = parser.add_subparsers(dest="command", required=True)

    generate = sub.add_parser("generate", help="create a fresh seed (air-gapped host only)")
    generate.add_argument("--out-file", help="write to this file instead of stdout")
    generate.add_argument("--allow-plaintext-disk-write", action="store_true")
    generate.set_defaults(func=cmd_generate)

    derive = sub.add_parser("derive", help="print the public key + fingerprint for a seed")
    add_seed_input(derive)
    derive.set_defaults(func=cmd_derive)

    split = sub.add_parser("split", help="split a seed into n shares with threshold k")
    split.add_argument("-k", "--threshold", type=int, required=True, help="minimum shares to recover")
    split.add_argument("-n", "--shares", type=int, required=True, help="total number of shares")
    add_seed_input(split)
    split.add_argument("--out-dir", help="write one share file per custodian into this directory")
    split.add_argument("--file-prefix", default=DEFAULT_FILE_PREFIX, help="share file name prefix")
    split.add_argument("--label", default="custodian", help="default custodian label prefix")
    split.add_argument("--labels", help="comma-separated custodian labels, one per share")
    split.set_defaults(func=cmd_split)

    combine = sub.add_parser("combine", help="recover a seed from >= k shares")
    combine.add_argument("shares", nargs="+", help="share files, or '-' for stdin")
    combine.add_argument("--out-file", help="write the seed to this file instead of stdout")
    combine.add_argument("--redact", action="store_true", help="audit output only; never print the seed")
    combine.add_argument("--allow-plaintext-disk-write", action="store_true")
    combine.set_defaults(func=cmd_combine, redact=False)

    verify = sub.add_parser("verify", help="combine + compare the public key, without printing the seed")
    verify.add_argument("shares", nargs="+", help="share files, or '-' for stdin")
    verify.add_argument("--out-file", help="write the seed to this file (tmpfs only)")
    verify.add_argument("--allow-plaintext-disk-write", action="store_true")
    verify.set_defaults(func=cmd_combine, redact=True)

    selftest = sub.add_parser("selftest", help="run the built-in cryptographic self test")
    selftest.set_defaults(func=cmd_selftest)

    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        return args.func(args)
    except MismatchError as exc:
        print(f"{PROG}: error: {exc}", file=sys.stderr)
        return EXIT_MISMATCH
    except ValueError as exc:
        print(f"{PROG}: error: {exc}", file=sys.stderr)
        return EXIT_ERROR
    except KeyboardInterrupt:
        print(f"{PROG}: interrupted", file=sys.stderr)
        return EXIT_ERROR


if __name__ == "__main__":
    sys.exit(main())
