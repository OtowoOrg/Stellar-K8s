#!/usr/bin/env python3
"""Unit tests for scripts/check-shell-safety.py (issue #1049).

Each test drives the checker over a small synthetic script so that both the
positive case (the unsafe pattern is reported) and the negative case (the
safe spelling is *not* reported) are pinned down. Rules that only fire on
genuinely dangerous code are worthless if they also fire on the safe form,
so every rule below is tested in both directions.
"""

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path

_SPEC = importlib.util.spec_from_file_location(
    "check_shell_safety",
    Path(__file__).resolve().parent.parent / "check-shell-safety.py",
)
gate = importlib.util.module_from_spec(_SPEC)
# @dataclass resolves annotations through sys.modules, so the module has to be
# registered before it is executed.
sys.modules["check_shell_safety"] = gate
_SPEC.loader.exec_module(gate)

STRICT_HEADER = "#!/usr/bin/env bash\nset -euo pipefail\n"


def scan(body: str, *, header: str = STRICT_HEADER, config: gate.Config | None = None):
    """Analyse a script body and return its findings."""
    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "fixture.sh"
        path.write_text(header + body, encoding="utf-8")
        return gate.analyze([path], config or gate.Config())


def rules(findings) -> set:
    return {f.rule_id for f in findings}


class StrictModeTest(unittest.TestCase):
    def test_missing_strict_mode_is_reported(self):
        findings = scan('echo hi\n', header="#!/usr/bin/env bash\n")
        self.assertIn("SH001", rules(findings))

    def test_strict_mode_satisfies_the_rule(self):
        self.assertNotIn("SH001", rules(scan("echo hi\n")))

    def test_sourced_library_without_shebang_is_exempt(self):
        findings = scan("helper() { echo hi; }\n", header="# shared helpers\n")
        self.assertNotIn("SH001", rules(findings))

    def test_bats_shebang_is_exempt(self):
        # bats drives failure detection itself; `set -e` at file scope breaks it.
        findings = scan("@test 'x' { true; }\n", header="#!/usr/bin/env bats\n")
        self.assertNotIn("SH001", rules(findings))

    def test_scripts_lib_helpers_are_exempt(self):
        with tempfile.TemporaryDirectory() as tmp:
            lib = Path(tmp) / "lib"
            lib.mkdir()
            path = lib / "helpers.sh"
            path.write_text("#!/usr/bin/env bash\nhelper() { echo hi; }\n", encoding="utf-8")
            self.assertNotIn("SH001", rules(gate.analyze([path], gate.Config())))


class DestructiveCommandTest(unittest.TestCase):
    def test_unquoted_expansion_in_rm_is_reported(self):
        self.assertIn("SH002", rules(scan("rm -rf $BUILD_DIR\n")))

    def test_quoted_expansion_is_accepted(self):
        self.assertNotIn("SH002", rules(scan('rm -rf "$BUILD_DIR"\n')))

    def test_recursive_delete_with_path_suffix_is_reported(self):
        # An empty $BUILD_DIR turns this into `rm -rf /artifacts`.
        self.assertIn("SH003", rules(scan('rm -rf "$BUILD_DIR"/artifacts\n')))

    def test_recursive_delete_of_a_nested_path_is_reported(self):
        self.assertIn("SH003", rules(scan('rm -rf "${BUILD_DIR}/artifacts"\n')))

    def test_bare_recursive_delete_is_not_flagged(self):
        # `rm -rf ""` is a harmless no-op, so flagging it would be pure churn.
        self.assertNotIn("SH003", rules(scan('rm -rf "$BUILD_DIR"\n')))

    def test_empty_guard_clears_the_recursive_delete_rule(self):
        self.assertNotIn("SH003", rules(scan('rm -rf "${BUILD_DIR:?}"/artifacts\n')))

    def test_empty_default_does_not_count_as_a_guard(self):
        self.assertIn("SH003", rules(scan('rm -rf "${BUILD_DIR:-}"/artifacts\n')))

    def test_non_empty_default_counts_as_a_guard(self):
        self.assertNotIn("SH003", rules(scan('rm -rf "${BUILD_DIR:-./target}"/artifacts\n')))

    def test_literal_path_is_not_flagged(self):
        self.assertEqual(rules(scan("rm -rf ./target\n")) & {"SH002", "SH003"}, set())

    def test_other_destructive_commands_are_covered(self):
        self.assertIn("SH002", rules(scan("chown -R root $TARGET\n")))


class EvalAndRemoteCodeTest(unittest.TestCase):
    def test_eval_on_interpolated_data_is_reported(self):
        self.assertIn("SH004", rules(scan('eval "$USER_INPUT"\n')))

    def test_eval_on_a_literal_is_not_reported(self):
        self.assertNotIn("SH004", rules(scan("eval 'echo static'\n")))

    def test_curl_piped_into_bash_is_reported(self):
        self.assertIn("SH005", rules(scan("curl -fsSL https://example.com/i.sh | bash\n")))

    def test_curl_to_a_file_is_not_reported(self):
        self.assertNotIn("SH005", rules(scan('curl -fsSL https://example.com/i.sh -o "$tmp"\n')))

    def test_insecure_tls_is_reported(self):
        self.assertIn("SH006", rules(scan("curl -k -fsSL https://example.com\n")))

    def test_verified_tls_is_not_reported(self):
        self.assertNotIn("SH006", rules(scan("curl -fsSL https://example.com\n")))

    def test_curl_without_fail_flag_is_reported(self):
        self.assertIn("SH015", rules(scan('curl -sSL https://example.com -o "$out"\n')))

    def test_curl_with_fail_flag_is_accepted(self):
        self.assertNotIn("SH015", rules(scan('curl -fsSL https://example.com -o "$out"\n')))


class PermissionsAndTempFileTest(unittest.TestCase):
    def test_world_writable_octal_is_reported(self):
        self.assertIn("SH007", rules(scan('chmod 777 "$dir"\n')))

    def test_world_writable_symbolic_is_reported(self):
        self.assertIn("SH007", rules(scan('chmod a+rwx "$dir"\n')))

    def test_narrow_mode_is_accepted(self):
        self.assertNotIn("SH007", rules(scan('chmod 755 "$dir"\n')))

    def test_predictable_temp_path_is_reported(self):
        self.assertIn("SH008", rules(scan('echo hi > /tmp/build.log\n')))

    def test_mktemp_is_accepted(self):
        findings = rules(scan('log="$(mktemp)"\ntrap \'rm -f "$log"\' EXIT\n'))
        self.assertNotIn("SH008", findings)
        self.assertNotIn("SH009", findings)

    def test_mktemp_without_cleanup_is_reported(self):
        self.assertIn("SH009", rules(scan('tmp="$(mktemp -d)"\necho "$tmp"\n')))

    def test_bats_teardown_counts_as_cleanup(self):
        body = 'setup() {\n  TEST_DIR="$(mktemp -d)"\n}\n\nteardown() {\n  rm -rf "${TEST_DIR}"\n}\n'
        self.assertNotIn("SH009", rules(scan(body)))

    def test_single_quoted_trap_body_counts_as_cleanup(self):
        body = 'TMPFILE="$(mktemp)"\ntrap \'rm -f "$TMPFILE"\' EXIT\n'
        self.assertNotIn("SH009", rules(scan(body)))


class QuotingTest(unittest.TestCase):
    def test_unchecked_cd_is_reported_without_strict_mode(self):
        findings = scan('cd "$dir"\n', header="#!/usr/bin/env bash\n")
        self.assertIn("SH010", rules(findings))

    def test_cd_under_strict_mode_is_accepted(self):
        # `set -e` already aborts on a failed cd, so flagging it would be noise.
        self.assertNotIn("SH010", rules(scan('cd "$dir"\n')))

    def test_guarded_cd_is_accepted(self):
        findings = scan('cd "$dir" || exit 1\n', header="#!/usr/bin/env bash\n")
        self.assertNotIn("SH010", rules(findings))

    def test_backticks_are_reported(self):
        self.assertIn("SH011", rules(scan("now=`date`\n")))

    def test_dollar_parens_is_accepted(self):
        self.assertNotIn("SH011", rules(scan('now="$(date)"\n')))

    def test_escaped_backticks_in_a_string_are_not_substitution(self):
        self.assertNotIn("SH011", rules(scan('echo "use \\`make\\` here"\n')))

    def test_unquoted_argument_forwarding_is_reported(self):
        self.assertIn("SH012", rules(scan("run_tool $@\n")))

    def test_quoted_argument_forwarding_is_accepted(self):
        self.assertNotIn("SH012", rules(scan('run_tool "$@"\n')))

    def test_star_inside_a_quoted_message_is_accepted(self):
        self.assertNotIn("SH012", rules(scan('log() { echo "── $* ──"; }\n')))

    def test_looping_over_ls_is_reported(self):
        self.assertIn("SH013", rules(scan("for f in $(ls .); do echo \"$f\"; done\n")))

    def test_glob_loop_is_accepted(self):
        self.assertNotIn("SH013", rules(scan('for f in ./*.txt; do echo "$f"; done\n')))

    def test_unquoted_test_operand_is_reported(self):
        self.assertIn("SH014", rules(scan('if [ $mode = fast ]; then echo y; fi\n')))

    def test_quoted_test_operand_is_accepted(self):
        self.assertNotIn("SH014", rules(scan('if [ "$mode" = fast ]; then echo y; fi\n')))

    def test_double_bracket_test_is_accepted(self):
        self.assertNotIn("SH014", rules(scan('if [[ $mode == fast ]]; then echo y; fi\n')))


class ContextHandlingTest(unittest.TestCase):
    def test_comments_are_ignored(self):
        self.assertEqual(rules(scan("# rm -rf $HOME\necho ok\n")), set())

    def test_heredoc_bodies_are_ignored(self):
        body = 'cat <<EOF\nrm -rf $HOME\nEOF\n'
        self.assertEqual(rules(scan(body)), set())

    def test_hash_inside_a_string_does_not_start_a_comment(self):
        self.assertIn("SH002", rules(scan('rm -rf $dir  # trailing\n')))

    def test_single_quoted_text_is_inert(self):
        self.assertNotIn("SH002", rules(scan("echo 'rm -rf $HOME'\n")))


class SuppressionTest(unittest.TestCase):
    def test_inline_pragma_with_reason_suppresses(self):
        body = 'rm -rf $dir  # shell-safety: allow SH002,SH003 -- validated above\n'
        self.assertEqual(rules(scan(body)) & {"SH002", "SH003"}, set())

    def test_pragma_without_reason_is_itself_an_error(self):
        body = "rm -rf $dir  # shell-safety: allow SH002\n"
        found = rules(scan(body))
        self.assertIn("SH000", found)
        self.assertIn("SH002", found, "a reason-less pragma must not suppress anything")

    def test_standalone_pragma_applies_to_the_next_line(self):
        body = "# shell-safety: allow SH002 -- fixture\nrm -rf $dir\n"
        self.assertNotIn("SH002", rules(scan(body)))

    def test_file_level_pragma_suppresses_everywhere(self):
        body = "# shell-safety: disable-file SH011 -- legacy fixture\na=`date`\nb=`date`\n"
        self.assertNotIn("SH011", rules(scan(body)))


class ConfigTest(unittest.TestCase):
    def test_severity_override_downgrades(self):
        config = gate.Config(severity_overrides={"SH002": gate.WARNING})
        findings = scan("rm -rf $dir\n", config=config)
        sh002 = [f for f in findings if f.rule_id == "SH002"]
        self.assertTrue(sh002)
        self.assertEqual(sh002[0].severity, gate.WARNING)

    def test_severity_override_off_removes_the_finding(self):
        config = gate.Config(severity_overrides={"SH002": "off"})
        self.assertNotIn("SH002", rules(scan("rm -rf $dir\n", config=config)))

    def test_repo_config_file_parses(self):
        config = gate.Config.load(gate.DEFAULT_CONFIG)
        self.assertIsInstance(config.exclude, list)
        self.assertIsInstance(config.severity_overrides, dict)


class ExitCodeTest(unittest.TestCase):
    def _run(self, body, *extra):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "fixture.sh"
            path.write_text(STRICT_HEADER + body, encoding="utf-8")
            return gate.main([str(path), "--format", "json", *extra])

    def test_clean_script_exits_zero(self):
        self.assertEqual(self._run('echo "ok"\n'), 0)

    def test_error_finding_exits_one(self):
        self.assertEqual(self._run("rm -rf $dir\n"), 1)

    def test_warning_alone_passes_by_default(self):
        self.assertEqual(self._run("echo hi > /tmp/fixed.log\n"), 0)

    def test_warning_fails_under_strict(self):
        self.assertEqual(self._run("echo hi > /tmp/fixed.log\n", "--strict"), 1)


class RepositoryGateTest(unittest.TestCase):
    """The repository itself must stay clean at `error` severity."""

    def test_repository_scripts_have_no_error_findings(self):
        config = gate.Config.load(gate.DEFAULT_CONFIG)
        paths = gate.discover(["scripts"], config)
        self.assertTrue(paths, "expected to discover shell scripts under scripts/")
        errors = [f for f in gate.analyze(paths, config) if f.severity == gate.ERROR]
        self.assertEqual(
            errors,
            [],
            "unsafe shell patterns introduced:\n"
            + "\n".join(f"{f.path}:{f.line} [{f.rule_id}] {f.message}" for f in errors),
        )


if __name__ == "__main__":
    unittest.main()
