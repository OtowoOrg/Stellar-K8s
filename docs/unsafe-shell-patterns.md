# Unsafe Shell Pattern Gate

> **TL;DR** — Every `*.sh` file in the repository (except `scripts/archive/`)
> is scanned for a deny-list of dangerous shell patterns. CI fails on any
> match. Reviewed exceptions can be allowlisted with an inline comment.

---

## Overview

`scripts/check-unsafe-shell.sh` is a static analysis gate that complements
[ShellCheck](https://www.shellcheck.net/): ShellCheck focuses on
correctness and portability bugs, while this gate focuses on **security**
anti-patterns that a correctness linter does not flag.

It detects:

| Pattern | Why it matters |
|---|---|
| `eval` of a dynamic string | Arbitrary code execution if the input isn't fully trusted |
| `curl \| sh`, `wget \| bash`, `sh -c "$(curl ...)"` | Executes remote content with no integrity check |
| `chmod 777`, `chmod a+w`, `chmod o+w` | World-writable files/directories are a privilege-escalation vector |
| `StrictHostKeyChecking=no` | Disables SSH host-key verification, enabling MITM |
| `curl -k`/`--insecure`, `wget --no-check-certificate` | Disables TLS certificate verification |
| Unquoted `rm -rf $VAR` | Word-splitting/empty-variable footgun that can delete unintended paths |

The scan is a plain-text regex pass (see `PATTERN_NAMES`/`PATTERN_REGEXES` in
the script) — it does not execute the scripts it inspects.

## Running it

```bash
# Whole repository (same as CI)
make check-unsafe-shell

# A specific path
scripts/check-unsafe-shell.sh scripts/my-script.sh
```

It is also part of `make security-scan` / `make security-all`, runs as a
step in the CI `lint` job (`.github/workflows/ci.yml`), and as a local
pre-commit hook (`.pre-commit-config.yaml`) on any staged `*.sh` file.

## Suppressing a reviewed exception

Some patterns are legitimate — for example the official rustup and Homebrew
installers both pipe a fetched script into a shell. Rather than silently
skip these, annotate the exact line with a reason:

```bash
# unsafe-shell-allow: official rustup installer, HTTPS with TLS 1.2 pinned
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
```

The annotation may also go on the same line as the flagged command:

```bash
curl -s https://internal-mirror.example.com/bootstrap.sh | bash # unsafe-shell-allow: internal mirror, checksum verified upstream
```

Allowlisted lines are not silent: `check-unsafe-shell.sh` prints every
allowlisted match and its reason on every run, so exceptions stay auditable
in CI logs and can be grepped for (`unsafe-shell-allow:`) during review.

## Tests

`scripts/tests/check-unsafe-shell.bats` covers every pattern (true positive),
comment-only mentions (true negative), and both allowlist-annotation forms.
Run with:

```bash
make test-shell   # or: bats scripts/tests/check-unsafe-shell.bats
```
