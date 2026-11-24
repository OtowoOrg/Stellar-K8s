#!/usr/bin/env python3
"""Static analysis gate for unsafe shell patterns (issue #1049).

Scans the repository's shell scripts for patterns that are known to cause
silent data loss, command injection, or non-deterministic CI behaviour --
the classes of defect that ``shellcheck -S error`` deliberately does not
report because they are *stylistic* to a linter but *operational* to an
operator repository.

The checker is intentionally standalone (stdlib + PyYAML only) so that it
runs identically on a developer laptop and in CI, without needing a Go or
Haskell toolchain the way ``kubeconform``/``shellcheck`` do.

Usage:
    scripts/check-shell-safety.py [PATH ...]
    scripts/check-shell-safety.py --format json
    scripts/check-shell-safety.py --list-rules
    scripts/check-shell-safety.py --strict          # warnings fail too

Exit codes: 0 = gate passed, 1 = gate failed, 2 = bad invocation.

Suppressions
------------
A finding can be waived with an inline pragma on the offending line or on
the line directly above it::

    rm -rf $BUILD_DIR  # shell-safety: allow SH002 -- path is validated above

A whole file can waive a rule with a pragma anywhere in its header::

    # shell-safety: disable-file SH009 -- fixed paths are the point of this fixture

Every suppression requires a ``--`` reason; a bare ``allow`` is itself an
error (rule SH000) so waivers cannot be added silently.
"""

from __future__ import annotations

import argparse
import fnmatch
import json
import os
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Iterable, Iterator, Sequence

try:
    import yaml
except ImportError:  # pragma: no cover - PyYAML is a documented dependency
    yaml = None

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_CONFIG = REPO_ROOT / "config" / "shell-safety.yaml"
DEFAULT_TARGETS = ("scripts",)

ERROR = "error"
WARNING = "warning"

# ---------------------------------------------------------------------------
# Rule definitions
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class Rule:
    """A single unsafe-pattern rule."""

    rule_id: str
    severity: str
    summary: str
    remedy: str


RULES: dict[str, Rule] = {
    r.rule_id: r
    for r in (
        Rule(
            "SH000",
            ERROR,
            "suppression pragma without a '--' reason",
            "Write '# shell-safety: allow SH123 -- why this is safe'.",
        ),
        Rule(
            "SH001",
            ERROR,
            "script does not enable strict mode",
            "Add 'set -euo pipefail' near the top of the script.",
        ),
        Rule(
            "SH002",
            ERROR,
            "unquoted expansion passed to a destructive command",
            'Quote the expansion: rm -rf "$dir" (and prefer "${dir:?}").',
        ),
        Rule(
            "SH003",
            ERROR,
            "recursive delete of an expansion with no empty-value guard",
            'Use "${dir:?refusing to delete empty path}" so an unset value aborts.',
        ),
        Rule(
            "SH004",
            ERROR,
            "eval on interpolated data",
            "Replace eval with an array command, a function, or a case dispatch.",
        ),
        Rule(
            "SH005",
            ERROR,
            "remote content piped straight into an interpreter",
            "Download to a temp file, verify a checksum/signature, then execute.",
        ),
        Rule(
            "SH006",
            ERROR,
            "TLS verification disabled on a network fetch",
            "Drop -k/--insecure/--no-check-certificate and fix the trust store.",
        ),
        Rule(
            "SH007",
            ERROR,
            "world-writable permission bits",
            "Grant the narrowest mode that works, e.g. chmod 755 or chmod 600.",
        ),
        Rule(
            "SH008",
            WARNING,
            "predictable temporary path instead of mktemp",
            'Use "$(mktemp -d)" / "$(mktemp)" and clean it up in a trap.',
        ),
        Rule(
            "SH009",
            WARNING,
            "mktemp result never cleaned up by a trap",
            "Add: trap 'rm -rf \"$tmp\"' EXIT",
        ),
        Rule(
            "SH010",
            ERROR,
            "cd whose failure is not handled",
            'Use cd "$dir" || exit 1 (or run under set -e with an explicit guard).',
        ),
        Rule(
            "SH011",
            WARNING,
            "backtick command substitution",
            "Use $(...) which nests correctly and is easier to quote.",
        ),
        Rule(
            "SH012",
            ERROR,
            "unquoted \"$@\"/$* argument forwarding",
            'Forward arguments as "$@" so empty and spaced arguments survive.',
        ),
        Rule(
            "SH013",
            WARNING,
            "iterating over the output of ls/find",
            "Use a glob, or find -print0 piped into 'while IFS= read -r -d \"\"'.",
        ),
        Rule(
            "SH014",
            ERROR,
            "unquoted expansion in a [ ... ] test",
            'Quote it: [ "$x" = y ], or use [[ ... ]] which does not word-split.',
        ),
        Rule(
            "SH015",
            ERROR,
            "curl/wget without a failure exit status",
            "Add curl -fsSL (the -f is what turns HTTP 4xx/5xx into an error).",
        ),
    )
}

DESTRUCTIVE_COMMANDS = ("rm", "rmdir", "mv", "cp", "chmod", "chown", "chgrp", "dd", "shred", "truncate")

# ---------------------------------------------------------------------------
# Findings
# ---------------------------------------------------------------------------


@dataclass
class Finding:
    """One rule violation at one location."""

    path: str
    line: int
    rule_id: str
    severity: str
    message: str
    evidence: str

    def as_dict(self) -> dict:
        return {
            "path": self.path,
            "line": self.line,
            "rule": self.rule_id,
            "severity": self.severity,
            "message": self.message,
            "evidence": self.evidence,
        }


# ---------------------------------------------------------------------------
# Source preparation
# ---------------------------------------------------------------------------

_SUPPRESS_LINE_RE = re.compile(r"#\s*shell-safety:\s*allow\s+(?P<ids>[A-Z0-9, ]+?)(?P<reason>\s*--.*)?$")
_SUPPRESS_FILE_RE = re.compile(r"#\s*shell-safety:\s*disable-file\s+(?P<ids>[A-Z0-9, ]+?)(?P<reason>\s*--.*)?$")
_HEREDOC_RE = re.compile(r"<<-?\s*(?P<q>['\"]?)(?P<tag>[A-Za-z_][A-Za-z0-9_]*)(?P=q)")


def _split_code_and_comment(line: str) -> tuple[str, str]:
    """Split a shell line into (code, comment), honouring quotes.

    A ``#`` only opens a comment at the start of a word, so ``foo#bar`` and
    ``"a # b"`` are left intact.
    """
    in_single = in_double = False
    prev = ""
    for idx, ch in enumerate(line):
        if ch == "'" and not in_double and prev != "\\":
            in_single = not in_single
        elif ch == '"' and not in_single and prev != "\\":
            in_double = not in_double
        elif ch == "#" and not in_single and not in_double:
            if idx == 0 or line[idx - 1] in " \t":
                return line[:idx], line[idx:]
        prev = "" if (ch == "\\" and prev == "\\") else ch
    return line, ""


def _blank_single_quotes(code: str) -> str:
    """Replace single-quoted spans with spaces.

    Single quotes suppress every expansion, so their contents can never be
    the source of an injection or word-splitting bug.
    """
    out: list[str] = []
    in_single = False
    for ch in code:
        if ch == "'":
            in_single = not in_single
            out.append("'")
        else:
            out.append(" " if in_single else ch)
    return "".join(out)


@dataclass
class ShellLine:
    """One physical line of a script, pre-classified for the rule engine."""

    number: int
    raw: str
    code: str  # comment stripped
    scrubbed: str  # comment stripped + single-quoted spans blanked
    comment: str
    in_heredoc: bool = False


@dataclass
class ShellScript:
    """A parsed shell script ready for rule evaluation."""

    path: Path
    display: str
    lines: list[ShellLine] = field(default_factory=list)
    file_suppressions: set[str] = field(default_factory=set)
    line_suppressions: dict[int, set[str]] = field(default_factory=dict)
    pragma_errors: list[tuple[int, str]] = field(default_factory=list)

    @property
    def code_lines(self) -> Iterator[ShellLine]:
        return (ln for ln in self.lines if not ln.in_heredoc)


def _parse_pragmas(script: ShellScript) -> None:
    """Collect inline/file suppressions and flag reason-less pragmas."""
    for ln in script.lines:
        for regex, is_file in ((_SUPPRESS_FILE_RE, True), (_SUPPRESS_LINE_RE, False)):
            match = regex.search(ln.comment)
            if not match:
                continue
            ids = {part.strip() for part in match.group("ids").split(",") if part.strip()}
            if not (match.group("reason") or "").strip():
                script.pragma_errors.append((ln.number, ln.raw.strip()))
                continue
            if is_file:
                script.file_suppressions |= ids
            else:
                # A pragma on its own line waives the *next* code line;
                # a trailing pragma waives its own line.
                target = ln.number if ln.code.strip() else ln.number + 1
                script.line_suppressions.setdefault(target, set()).update(ids)


def parse_script(path: Path, display: str | None = None) -> ShellScript:
    """Read a script and pre-classify each line."""
    text = path.read_text(encoding="utf-8", errors="replace")
    script = ShellScript(path=path, display=display or str(path))

    pending_heredocs: list[str] = []
    active_heredoc: str | None = None

    for number, raw in enumerate(text.splitlines(), start=1):
        if active_heredoc is not None:
            script.lines.append(ShellLine(number, raw, "", "", "", in_heredoc=True))
            if raw.strip() == active_heredoc:
                active_heredoc = pending_heredocs.pop(0) if pending_heredocs else None
            continue

        code, comment = _split_code_and_comment(raw)
        script.lines.append(ShellLine(number, raw, code, _blank_single_quotes(code), comment))

        tags = [m.group("tag") for m in _HEREDOC_RE.finditer(code)]
        if tags:
            active_heredoc = tags[0]
            pending_heredocs = tags[1:]

    _parse_pragmas(script)
    return script


# ---------------------------------------------------------------------------
# Rule engine
# ---------------------------------------------------------------------------

_STRICT_MODE_RE = re.compile(r"^\s*set\s+-[a-zA-Z]*e[a-zA-Z]*[ou]?")
_UNQUOTED_VAR = r"(?<![\"'\w])\$(?:\{[A-Za-z_][A-Za-z0-9_]*(?:\[[^]]*\])?\}|[A-Za-z_][A-Za-z0-9_]*)"
# Any expansion, quoted or not -- quoting stops word-splitting but does not
# stop an empty value from turning `rm -rf "$dir"/` into `rm -rf /`.
_ANY_VAR = re.compile(r"\$(?:\{[^{}]*\}|[A-Za-z_][A-Za-z0-9_]*)")
_DESTRUCTIVE_RE = re.compile(
    r"(?:^|[;&|]|\bthen\b|\bdo\b|\belse\b|\{)\s*(?:sudo\s+)?(?P<cmd>" + "|".join(DESTRUCTIVE_COMMANDS) + r")\s+(?P<args>[^;&|)]*)"
)
_RM_RECURSIVE_RE = re.compile(r"\brm\s+(?:-[a-zA-Z]*\s+)*-{1,2}[a-zA-Z-]*(?:r|R|recursive)[a-zA-Z]*\b")
_EVAL_RE = re.compile(r"(?:^|[;&|]\s*|\bthen\s+|\bdo\s+)eval\s+(?P<args>.+)")
_PIPE_TO_SHELL_RE = re.compile(
    r"\b(?:curl|wget)\b[^|;&]*\|\s*(?:sudo\s+)?(?:/usr/bin/env\s+)?(?:ba|z|k|da)?sh\b"
)
_INSECURE_TLS_RE = re.compile(r"\b(?:curl|wget)\b[^;&|]*?(?:\s(?:-k|--insecure|--no-check-certificate)\b)")
_WORLD_WRITABLE_RE = re.compile(r"\bchmod\b[^;&|]*?(?:\s(?:[0-7]?[0-7]{2}[2367]|a\+rwx|o\+w|ugo\+rwx)\b)")
_PREDICTABLE_TMP_RE = re.compile(r"(?<![\w/])/tmp/(?![\"'\s]*\$)[A-Za-z0-9_.-]+")
_MKTEMP_ASSIGN_RE = re.compile(r"(?P<var>[A-Za-z_][A-Za-z0-9_]*)=\W*\$\((\s*mktemp\b[^)]*)\)")
_CD_RE = re.compile(r"(?:^|[;&|]\s*|\bthen\s+|\bdo\s+|\(\s*)cd\s+(?P<rest>[^;&|)]+)")
_BACKTICK_RE = re.compile(r"(?<!\\)`[^`]*(?<!\\)`")
_ARGS_FORWARD_RE = re.compile(r"\$(?:@|\*)")
_FUNC_START_RE = re.compile(r"^\s*(?:function\s+)?(?P<name>teardown|teardown_file|cleanup|_cleanup|finish)\s*\(\s*\)")
_LS_LOOP_RE = re.compile(r"\bfor\s+\w+\s+in\s+[^;]*\$\(\s*(?:ls|find)\b")
_TEST_RE = re.compile(r"(?:^|[;&|]|\bif\b|\bwhile\b|\belif\b|&&|\|\|)\s*(?:!\s*)?\[\s(?P<body>[^]]*)\]")
_NETWORK_FETCH_RE = re.compile(r"(?:^|[;&|(]|\$\(|\bthen\b|\bdo\b)\s*(?:sudo\s+)?(?P<cmd>curl|wget)\s+(?P<args>[^;&|)]*)")

# Expansions that are safe to leave unquoted in a [ ... ] test because the
# shell guarantees they are a single word.
_SAFE_TEST_VARS = {"$?", "$$", "$#"}


def _double_quote_spans(text: str) -> list[tuple[int, int]]:
    """Return the ``(start, end)`` offsets of every double-quoted span."""
    spans: list[tuple[int, int]] = []
    in_double = False
    prev = ""
    start = 0
    for idx, ch in enumerate(text):
        if ch == '"' and prev != "\\":
            if in_double:
                spans.append((start, idx))
            else:
                start = idx
            in_double = not in_double
        prev = ch
    if in_double:
        spans.append((start, len(text)))
    return spans


def _unquoted_positions(text: str):
    """Build a predicate telling whether an offset sits outside double quotes."""
    spans = _double_quote_spans(text)
    return lambda pos: not any(a <= pos <= b for a, b in spans)


def _iter_var_matches(text: str) -> Iterator[re.Match]:
    """Yield unquoted-looking expansions, skipping double-quoted spans."""
    unquoted = _unquoted_positions(text)
    for match in re.finditer(_UNQUOTED_VAR, text):
        if unquoted(match.start()):
            yield match


def _has_empty_guard(expansion: str) -> bool:
    """True if the expansion cannot come out empty.

    ``${x:?msg}`` aborts on an unset/empty value. ``${x:-default}`` only
    helps when the default is itself non-empty -- ``${x:-}`` is exactly the
    bug this rule exists to catch.
    """
    if ":?" in expansion:
        return True
    fallback = re.search(r":-(?P<default>[^}]*)\}", expansion)
    return bool(fallback and fallback.group("default").strip())


class RuleEngine:
    """Applies every rule to a parsed script."""

    def __init__(self, script: ShellScript) -> None:
        self.script = script
        self.findings: list[Finding] = []
        self.strict_mode = any(_STRICT_MODE_RE.match(ln.code) for ln in script.code_lines)

    # -- helpers ----------------------------------------------------------

    def _report(self, line: ShellLine, rule_id: str, detail: str = "") -> None:
        rule = RULES[rule_id]
        message = rule.summary if not detail else f"{rule.summary}: {detail}"
        self.findings.append(
            Finding(
                path=self.script.display,
                line=line.number,
                rule_id=rule_id,
                severity=rule.severity,
                message=message,
                evidence=line.raw.strip()[:200],
            )
        )

    # -- rules ------------------------------------------------------------

    def _check_strict_mode(self) -> None:
        if self.strict_mode:
            return
        if not self.script.lines:
            return
        shebang = self.script.lines[0].raw
        if not shebang.startswith("#!"):
            # No shebang: a sourced fragment, whose caller owns strict mode.
            return
        if not re.search(r"\b(?:ba|z|k|da)?sh\b", shebang):
            # e.g. `#!/usr/bin/env bats` — bats drives failure detection
            # itself, and `set -e` at file scope breaks its runner.
            return
        if "lib" in self.script.path.parts:
            # scripts/lib/* are sourced helpers; they must not impose `set -e`
            # on whichever script pulls them in.
            return
        self._report(self.script.lines[0], "SH001")

    def _check_line(self, ln: ShellLine) -> None:
        code, scrubbed = ln.code, ln.scrubbed
        if not code.strip():
            return

        # SH002 / SH003 — destructive commands
        for match in _DESTRUCTIVE_RE.finditer(scrubbed):
            args = match.group("args")
            unguarded = [m.group(0) for m in _iter_var_matches(args) if not _has_empty_guard(m.group(0))]
            if unguarded:
                self._report(ln, "SH002", f"{match.group('cmd')} {' '.join(unguarded)}")
            # SH003 targets the form that turns into a root delete when the
            # variable is empty: `rm -rf "$dir"/build` becomes `rm -rf /build`.
            # A bare `rm -rf "$dir"` with an empty value is a harmless no-op,
            # so flagging it would only produce churn.
            if match.group("cmd") == "rm" and _RM_RECURSIVE_RE.search(scrubbed):
                for expansion in _ANY_VAR.finditer(args):
                    if _has_empty_guard(expansion.group(0)):
                        continue
                    tail = args[expansion.end() :].lstrip('"')
                    if tail.startswith("/"):
                        self._report(ln, "SH003", expansion.group(0) + tail[:20])
                        break

        # SH004 — eval on interpolated data
        eval_match = _EVAL_RE.search(scrubbed)
        if eval_match and ("$" in eval_match.group("args") or "`" in eval_match.group("args")):
            self._report(ln, "SH004", eval_match.group("args").strip()[:80])

        # SH005 — remote content into an interpreter
        if _PIPE_TO_SHELL_RE.search(scrubbed):
            self._report(ln, "SH005")

        # SH006 — TLS verification disabled
        if _INSECURE_TLS_RE.search(scrubbed):
            self._report(ln, "SH006")

        # SH007 — world-writable bits
        if _WORLD_WRITABLE_RE.search(scrubbed):
            self._report(ln, "SH007")

        # SH008 — predictable temp paths
        if "mktemp" not in scrubbed:
            tmp = _PREDICTABLE_TMP_RE.search(scrubbed)
            if tmp:
                self._report(ln, "SH008", tmp.group(0))

        # SH010 — unchecked cd. Under `set -e` a failed cd already aborts the
        # script, so this only matters for scripts without strict mode.
        if not self.strict_mode:
            cd_match = _CD_RE.search(scrubbed)
            if cd_match:
                rest = cd_match.group("rest")
                tail = scrubbed[cd_match.end("rest") :]
                guarded = "||" in rest or "||" in tail or "&&" in tail
                if not guarded and not rest.strip().startswith("-"):
                    self._report(ln, "SH010", f"cd {rest.strip()[:60]}")

        # SH011 — backticks (escaped ones are markdown, not substitution)
        if _BACKTICK_RE.search(scrubbed):
            self._report(ln, "SH011")

        # SH012 — argument forwarding that is not wrapped in double quotes
        unquoted = _unquoted_positions(scrubbed)
        for match in _ARGS_FORWARD_RE.finditer(scrubbed):
            if unquoted(match.start()):
                self._report(ln, "SH012", match.group(0))
                break

        # SH013 — looping over ls/find output
        if _LS_LOOP_RE.search(scrubbed):
            self._report(ln, "SH013")

        # SH014 — unquoted expansion inside [ ... ]
        for match in _TEST_RE.finditer(scrubbed):
            body = match.group("body")
            for var in _iter_var_matches(body):
                if var.group(0) not in _SAFE_TEST_VARS:
                    self._report(ln, "SH014", var.group(0))
                    break

        # SH015 — network fetch without a failing exit status
        for match in _NETWORK_FETCH_RE.finditer(scrubbed):
            args = match.group("args")
            if match.group("cmd") == "curl":
                flags = re.findall(r"(?:^|\s)-(?!-)([a-zA-Z]+)", args)
                if not any("f" in group for group in flags) and "--fail" not in args:
                    self._report(ln, "SH015", "curl without -f/--fail")
            elif "-O-" not in args and "--tries" not in args and "-q" not in args.split():
                # wget already exits non-zero on HTTP errors unless output is
                # piped; only the piped form needs an explicit guard.
                continue

    def _cleanup_surface(self) -> str:
        """Text of every construct that can remove a temp path on exit.

        This is `trap` lines plus the bodies of conventional cleanup
        functions (including bats' `teardown`). It reads `code`, not
        `scrubbed`, because a trap action is almost always single-quoted --
        blanking those spans would hide the very reference we look for.
        """
        parts: list[str] = []
        depth = 0
        in_cleanup = False
        for ln in self.script.code_lines:
            if "trap" in ln.code or re.search(r"\brm\s+-[a-zA-Z]*[rf]", ln.code):
                parts.append(ln.code)
            if not in_cleanup and _FUNC_START_RE.match(ln.code):
                in_cleanup, depth = True, 0
            if in_cleanup:
                parts.append(ln.code)
                depth += ln.code.count("{") - ln.code.count("}")
                if depth <= 0 and "}" in ln.code:
                    in_cleanup = False
        return " ".join(parts)

    def _check_mktemp_cleanup(self) -> None:
        surface = self._cleanup_surface()
        for ln in self.script.code_lines:
            match = _MKTEMP_ASSIGN_RE.search(ln.code)
            if not match:
                continue
            var = match.group("var")
            if not re.search(rf"\${{?{re.escape(var)}\b", surface):
                self._report(ln, "SH009", f"${var}")

    def _check_pragmas(self) -> None:
        for number, raw in self.script.pragma_errors:
            line = self.script.lines[number - 1]
            self._report(line, "SH000", raw[:80])

    # -- driver -----------------------------------------------------------

    def run(self) -> list[Finding]:
        self._check_pragmas()
        self._check_strict_mode()
        for ln in self.script.code_lines:
            self._check_line(ln)
        self._check_mktemp_cleanup()
        return self._filter_suppressed()

    def _filter_suppressed(self) -> list[Finding]:
        kept = []
        for finding in self.findings:
            if finding.rule_id in self.script.file_suppressions:
                continue
            if finding.rule_id in self.script.line_suppressions.get(finding.line, ()):
                continue
            kept.append(finding)
        return kept


# ---------------------------------------------------------------------------
# Discovery & configuration
# ---------------------------------------------------------------------------


@dataclass
class Config:
    """Runtime configuration, optionally loaded from YAML."""

    exclude: list[str] = field(default_factory=list)
    severity_overrides: dict[str, str] = field(default_factory=dict)

    @classmethod
    def load(cls, path: Path) -> "Config":
        if not path.is_file():
            return cls()
        if yaml is None:
            raise SystemExit("PyYAML is required to read a shell-safety config file")
        data = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
        overrides = {str(k): str(v) for k, v in (data.get("severity_overrides") or {}).items()}
        unknown = set(overrides) - set(RULES)
        if unknown:
            raise SystemExit(f"unknown rule id(s) in severity_overrides: {', '.join(sorted(unknown))}")
        bad = {k: v for k, v in overrides.items() if v not in (ERROR, WARNING, "off")}
        if bad:
            raise SystemExit(f"severity must be one of error/warning/off: {bad}")
        return cls(exclude=[str(p) for p in (data.get("exclude") or [])], severity_overrides=overrides)


def _is_shell_file(path: Path) -> bool:
    if path.suffix in (".sh", ".bash", ".bats"):
        return True
    if path.suffix:
        return False
    try:
        with path.open("rb") as handle:
            first = handle.readline(128)
    except OSError:
        return False
    return first.startswith(b"#!") and (b"sh" in first or b"bash" in first)


def discover(targets: Sequence[str], config: Config) -> list[Path]:
    """Collect shell scripts under ``targets``, honouring exclusions."""
    found: list[Path] = []
    for target in targets:
        base = Path(target)
        if not base.is_absolute():
            base = REPO_ROOT / base
        if base.is_file():
            candidates: Iterable[Path] = [base]
        elif base.is_dir():
            candidates = (p for p in sorted(base.rglob("*")) if p.is_file())
        else:
            raise SystemExit(f"no such path: {target}")
        for path in candidates:
            rel = _relative(path)
            if any(fnmatch.fnmatch(rel, pattern) for pattern in config.exclude):
                continue
            if ".git" in path.parts or "target" in path.parts:
                continue
            if _is_shell_file(path):
                found.append(path)
    return sorted(set(found))


def _relative(path: Path) -> str:
    try:
        return str(path.resolve().relative_to(REPO_ROOT))
    except ValueError:
        return str(path)


def analyze(paths: Sequence[Path], config: Config) -> list[Finding]:
    """Run every rule over every script and apply severity overrides."""
    findings: list[Finding] = []
    for path in paths:
        script = parse_script(path, display=_relative(path))
        for finding in RuleEngine(script).run():
            override = config.severity_overrides.get(finding.rule_id)
            if override == "off":
                continue
            if override:
                finding.severity = override
            findings.append(finding)
    findings.sort(key=lambda f: (f.path, f.line, f.rule_id))
    return findings


# ---------------------------------------------------------------------------
# Reporting
# ---------------------------------------------------------------------------


def _render_text(findings: Sequence[Finding], scanned: int, strict: bool) -> str:
    lines: list[str] = []
    annotate = os.environ.get("GITHUB_ACTIONS") == "true"
    errors = [f for f in findings if f.severity == ERROR]
    warnings = [f for f in findings if f.severity == WARNING]

    lines.append(f"→ Shell safety gate: scanned {scanned} script(s)")
    lines.append("")
    if not findings:
        lines.append("✓ No unsafe shell patterns detected")
    for finding in findings:
        marker = "✗" if finding.severity == ERROR else "⚠"
        lines.append(f"  {marker} {finding.path}:{finding.line} [{finding.rule_id}] {finding.message}")
        lines.append(f"      {finding.evidence}")
        lines.append(f"      fix: {RULES[finding.rule_id].remedy}")
        if annotate:
            level = "error" if finding.severity == ERROR else "warning"
            lines.append(
                f"::{level} file={finding.path},line={finding.line},"
                f"title={finding.rule_id}::{finding.message}"
            )
    lines.append("")
    lines.append("━" * 60)
    lines.append(f"Shell Safety Summary:  errors: {len(errors)}   warnings: {len(warnings)}")
    lines.append("━" * 60)
    if errors or (strict and warnings):
        lines.append("")
        lines.append("❌ Shell safety gate FAILED")
    else:
        lines.append("")
        lines.append("✅ Shell safety gate passed")
    return "\n".join(lines)


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Static analysis gate for unsafe shell patterns")
    parser.add_argument("paths", nargs="*", default=None, help="files or directories to scan (default: scripts/)")
    parser.add_argument("--config", default=str(DEFAULT_CONFIG), help="YAML config with exclusions/severities")
    parser.add_argument("--format", choices=("text", "json"), default="text")
    parser.add_argument("--strict", action="store_true", help="treat warnings as failures")
    parser.add_argument("--list-rules", action="store_true", help="print the rule catalogue and exit")
    args = parser.parse_args(argv)

    if args.list_rules:
        for rule in RULES.values():
            print(f"{rule.rule_id}  {rule.severity:<7}  {rule.summary}")
            print(f"          fix: {rule.remedy}")
        return 0

    config = Config.load(Path(args.config))
    targets = args.paths or list(DEFAULT_TARGETS)
    paths = discover(targets, config)
    findings = analyze(paths, config)

    if args.format == "json":
        print(
            json.dumps(
                {
                    "scanned": len(paths),
                    "findings": [f.as_dict() for f in findings],
                    "errors": sum(1 for f in findings if f.severity == ERROR),
                    "warnings": sum(1 for f in findings if f.severity == WARNING),
                },
                indent=2,
            )
        )
    else:
        print(_render_text(findings, len(paths), args.strict))

    failed = any(f.severity == ERROR for f in findings)
    if args.strict:
        failed = failed or bool(findings)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
