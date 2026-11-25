#!/usr/bin/env python3
# Copyright 2024 Stellar-K8s Contributors
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.
"""
check-suspicious-code.py

Flags code that could violate GitHub's Acceptable Use Policies and get the
repository or the owning account restricted:

  * crypto-mining           miners, mining pools, stratum URLs
  * backdoor / reverse-shell /dev/tcp shells, nc -e, socat exec, mkfifo pipes
  * obfuscated-exec         eval/exec of base64/hex-decoded payloads
  * remote-exec             curl|wget piped straight into a shell
  * exfiltration            paste sites, request catchers, Discord/Telegram
                            webhooks, tunnelling services
  * actions-abuse           risky workflow patterns (pull_request_target +
                            PR head checkout, miners/long sleeps in CI)
  * github-spam             bulk/looped star, follow, fork, or issue/comment
                            creation through the GitHub API or gh CLI
  * committed-binary        executables / archives committed to the repo

Usage:
  scripts/check-suspicious-code.py                 # scan all tracked files
  scripts/check-suspicious-code.py --base <ref>    # scan files changed vs <ref>
  scripts/check-suspicious-code.py --report        # never fail, just report

Suppress a reviewed false positive by adding `suspicious-code: allow` on the
same line, or list a path glob in .suspicious-code-allowlist.

Exit codes: 0 = clean, 1 = findings.
"""

from __future__ import annotations

import argparse
import fnmatch
import os
import re
import subprocess
import sys
from dataclasses import dataclass

ROOT = subprocess.run(
    ["git", "rev-parse", "--show-toplevel"], capture_output=True, text=True, check=True
).stdout.strip()

SELF_PATHS = {
    "scripts/check-suspicious-code.py",
    ".github/workflows/suspicious-code-scan.yml",
    ".suspicious-code-allowlist",
}

SKIP_DIRS = ("target/", "node_modules/", "vendor/", ".git/")
MAX_TEXT_BYTES = 2_000_000
INLINE_ALLOW = "suspicious-code: allow"

I = re.IGNORECASE

# (category, description, regex, path filter or None)
RULES: list[tuple[str, str, re.Pattern[str], re.Pattern[str] | None]] = [
    # --- crypto mining -----------------------------------------------------
    ("crypto-mining", "Known crypto-miner binary or library",
     re.compile(r"\b(xmrig|xmr-stak|cpuminer|minerd|cgminer|bfgminer|ethminer|nbminer|t-rex|lolminer|phoenixminer|nanominer|coinhive|cryptoloot|coin-hive)\b", I), None),
    ("crypto-mining", "Mining pool / stratum URL",
     re.compile(r"stratum\+(tcp|ssl|tls)://|\b(pool\.)?(minexmr|supportxmr|nanopool|2miners|f2pool|nicehash|hashvault|moneroocean|herominers)\.(com|org|net|pro)\b", I), None),
    ("crypto-mining", "CryptoNight / RandomX mining algorithm reference",
     re.compile(r"\b(cryptonight|randomx)\b.*\b(hash|mine|miner|threads)\b", I), None),

    # --- backdoors / reverse shells ----------------------------------------
    ("backdoor", "Reverse shell via /dev/tcp or /dev/udp",
     re.compile(r"/dev/(tcp|udp)/[\w.\-$]+/\d+", I), None),
    ("backdoor", "netcat executing a shell",
     re.compile(r"\b(nc|ncat|netcat)\b[^\n|;]*\s-(e|c)\s*\S*(sh|bash|cmd)", I), None),
    ("backdoor", "socat exec reverse shell",
     re.compile(r"\bsocat\b[^\n]*\bexec:[^\n]*(sh|bash)", I), None),
    ("backdoor", "mkfifo shell pipe",
     re.compile(r"mkfifo\s+\S+\s*;[^\n]*\b(nc|ncat|sh|bash)\b", I), None),
    ("backdoor", "Interactive shell over a socket (python/perl/ruby/php)",
     re.compile(r"socket\.[^\n]{0,80}(subprocess|pty\.spawn|os\.dup2)|pty\.spawn\(\s*['\"]/bin/(ba)?sh|fsockopen\([^\n]*\)[^\n]*(exec|system|shell_exec)", I), None),

    # --- obfuscated execution ----------------------------------------------
    ("obfuscated-exec", "eval/exec of base64- or hex-decoded payload",
     re.compile(r"\b(eval|exec|Function)\s*\(\s*(atob|base64\.b64decode|b64decode|Buffer\.from|bytes\.fromhex|codecs\.decode|unhexlify|zlib\.decompress|marshal\.loads)", I), None),
    ("obfuscated-exec", "base64-decoded data piped into a shell",
     re.compile(r"base64\s+(-d|--decode|-D)\b[^\n]*\|\s*(sudo\s+)?(ba|z|da)?sh\b", I), None),
    ("obfuscated-exec", "Very long base64 blob (possible packed payload)",
     re.compile(r"['\"][A-Za-z0-9+/]{400,}={0,2}['\"]"), re.compile(r"\.(py|js|ts|sh|bash|rb|php|ps1|pl|ya?ml)$")),
    ("obfuscated-exec", "PowerShell encoded command",
     re.compile(r"powershell[^\n]*\s-(enc|encodedcommand|e)\s+[A-Za-z0-9+/=]{20,}", I), None),

    # --- remote code execution --------------------------------------------
    ("remote-exec", "Script from a raw IP / plain-HTTP host piped into a shell",
     re.compile(r"\b(curl|wget)\b[^\n|]*(https?://\d{1,3}(\.\d{1,3}){3}|http://)[^\n|]*\|\s*(sudo\s+)?(ba|z|da)?sh\b", I), None),

    # --- exfiltration endpoints --------------------------------------------
    ("exfiltration", "Request-catcher / paste / tunnelling endpoint",
     re.compile(r"\b(webhook\.site|requestbin\.(com|net)|pipedream\.net|ngrok\.io|ngrok-free\.app|serveo\.net|localtunnel\.me|interact\.sh|oast\.(fun|me|pro|live)|burpcollaborator\.net|canarytokens\.com|pastebin\.com/raw|transfer\.sh|hastebin\.com)\b", I), None),
    ("exfiltration", "Discord / Telegram bot webhook",
     re.compile(r"discord(app)?\.com/api/webhooks/\d+|api\.telegram\.org/bot\d+:", I), None),
    ("exfiltration", "Environment or secrets sent to a remote host",
     re.compile(r"\b(curl|wget|fetch|requests\.(post|get))\b[^\n]*(\$\{\{\s*secrets\.|\benv\b\s*\||printenv|os\.environ|process\.env\b|~/\.ssh|id_rsa|\.aws/credentials)", I), None),

    # --- GitHub Actions abuse ----------------------------------------------
    ("actions-abuse", "Workflow sleeps for a very long time (resource abuse)",
     re.compile(r"\bsleep\s+(\d{5,}|infinity)\b", I), re.compile(r"^\.github/workflows/")),
    ("actions-abuse", "Infinite loop in a workflow run step",
     re.compile(r"while\s+(true|:|\[\s*1\s*\])\s*;\s*do", I), re.compile(r"^\.github/workflows/")),
    ("actions-abuse", "Checks out untrusted PR head code (pwn-request pattern)",
     re.compile(r"ref:\s*\$\{\{\s*github\.event\.pull_request\.head\.(sha|ref)\s*\}\}", I), re.compile(r"^\.github/workflows/")),

    # --- GitHub platform spam / inauthentic activity -----------------------
    ("github-spam", "Programmatic starring / following / forking via API",
     re.compile(r"(PUT|-X\s*PUT|\.put\()[^\n]*(/user/starred/|/user/following/)|/repos/[^\s'\"]+/forks\b[^\n]*(POST|-X\s*POST)|gh\s+api\s+(-X\s*PUT\s+)?/?user/(starred|following)/", I), None),
    ("github-spam", "gh CLI issue/PR/comment creation inside a loop",
     re.compile(r"(for|while)\b[^\n]*;\s*do[^\n]*\bgh\s+(issue|pr)\s+(create|comment)\b", I), None),
]

LOOP_RE = re.compile(r"^\s*(for\b|while\b|\w+\s*\.forEach|.*\bin\s+range\()", I)
BULK_CREATE_RE = re.compile(
    r"\bgh\s+(issue|pr)\s+(create|comment)\b|gh\s+api[^\n]*/(issues|comments|pulls)\b[^\n]*(POST|-f\s)|\.(create_issue|create_comment|create_pull)\(|/repos/[^\s'\"]+/(issues|pulls)(/\d+/comments)?['\"]?[^\n]*(POST|requests\.post)",
    I,
)

BINARY_EXTS = {
    ".exe", ".dll", ".scr", ".msi", ".bat", ".com", ".vbs", ".jar", ".apk",
    ".dmg", ".pkg", ".deb", ".rpm", ".so", ".dylib", ".bin", ".elf",
}
MAGIC = {
    b"MZ": "Windows PE executable",
    b"\x7fELF": "ELF executable",
    b"\xcf\xfa\xed\xfe": "Mach-O executable",
    b"\xfe\xed\xfa\xcf": "Mach-O executable",
    b"\xca\xfe\xba\xbe": "Mach-O universal / Java class",
}


@dataclass
class Finding:
    path: str
    line: int
    category: str
    message: str
    snippet: str


def tracked_files(base: str | None) -> list[str]:
    if base:
        out = subprocess.run(
            ["git", "diff", "--name-only", "--diff-filter=ACMR", f"{base}...HEAD"],
            capture_output=True, text=True, cwd=ROOT, check=True,
        ).stdout
    else:
        out = subprocess.run(
            ["git", "ls-files"], capture_output=True, text=True, cwd=ROOT, check=True
        ).stdout
    return [p for p in out.splitlines() if p]


def load_allowlist() -> list[str]:
    path = os.path.join(ROOT, ".suspicious-code-allowlist")
    if not os.path.exists(path):
        return []
    with open(path, encoding="utf-8") as fh:
        return [ln.strip() for ln in fh if ln.strip() and not ln.startswith("#")]


def scan_file(rel: str) -> list[Finding]:
    full = os.path.join(ROOT, rel)
    if not os.path.isfile(full) or os.path.islink(full):
        return []
    findings: list[Finding] = []

    with open(full, "rb") as fh:
        head = fh.read(8)
    ext = os.path.splitext(rel)[1].lower()
    magic = next((d for m, d in MAGIC.items() if head.startswith(m)), None)
    if magic or ext in BINARY_EXTS:
        findings.append(Finding(rel, 0, "committed-binary",
                                magic or f"Executable file type '{ext}'", ""))
        return findings

    if os.path.getsize(full) > MAX_TEXT_BYTES:
        return findings
    try:
        with open(full, encoding="utf-8") as fh:
            lines = fh.read().splitlines()
    except (UnicodeDecodeError, OSError):
        return findings

    for idx, line in enumerate(lines, 1):
        if INLINE_ALLOW in line:
            continue
        for category, desc, rx, path_filter in RULES:
            if path_filter and not path_filter.search(rel):
                continue
            if rx.search(line):
                findings.append(Finding(rel, idx, category, desc, line.strip()[:160]))

    # Bulk GitHub object creation: a create call within a few lines after a loop.
    for idx, line in enumerate(lines, 1):
        if not LOOP_RE.search(line):
            continue
        window = lines[idx - 1: idx + 8]
        for off, wline in enumerate(window):
            if INLINE_ALLOW in wline:
                continue
            if BULK_CREATE_RE.search(wline):
                findings.append(Finding(
                    rel, idx + off, "github-spam",
                    "Bulk issue/PR/comment creation in a loop (GitHub may treat as spam)",
                    wline.strip()[:160]))
                break

    # De-duplicate identical (line, category) hits.
    seen, unique = set(), []
    for f in findings:
        key = (f.line, f.category)
        if key not in seen:
            seen.add(key)
            unique.append(f)
    return unique


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--base", help="only scan files changed since this git ref")
    ap.add_argument("--report", action="store_true", help="report only, always exit 0")
    args = ap.parse_args()

    allow = load_allowlist()
    gha = os.environ.get("GITHUB_ACTIONS") == "true"
    findings: list[Finding] = []

    for rel in tracked_files(args.base):
        if rel in SELF_PATHS or rel.startswith(SKIP_DIRS):
            continue
        if any(fnmatch.fnmatch(rel, pat) for pat in allow):
            continue
        findings.extend(scan_file(rel))

    if not findings:
        print("✅ No suspicious code found.")
        return 0

    by_cat: dict[str, int] = {}
    for f in findings:
        by_cat[f.category] = by_cat.get(f.category, 0) + 1
        loc = f"{f.path}:{f.line}" if f.line else f.path
        print(f"[{f.category}] {loc}: {f.message}")
        if f.snippet:
            print(f"    {f.snippet}")
        if gha:
            level = "warning" if args.report else "error"
            print(f"::{level} file={f.path},line={max(f.line, 1)},title={f.category}::{f.message}")

    print("\nSummary:")
    for cat, n in sorted(by_cat.items()):
        print(f"  {cat:18} {n}")
    print(f"  {'total':18} {len(findings)}")
    print(f"\nReview each hit. If it is a legitimate use, add '{INLINE_ALLOW}' "
          "on that line or add the path to .suspicious-code-allowlist.")

    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as fh:
            fh.write("## Suspicious code scan\n\n| Category | Location | Finding |\n|---|---|---|\n")
            for f in findings:
                loc = f"{f.path}:{f.line}" if f.line else f.path
                fh.write(f"| {f.category} | `{loc}` | {f.message} |\n")

    return 0 if args.report else 1


if __name__ == "__main__":
    sys.exit(main())
