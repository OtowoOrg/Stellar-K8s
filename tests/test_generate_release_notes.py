import os
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path


class GenerateReleaseNotesScriptTest(unittest.TestCase):
    def test_script_invokes_git_cliff_with_expected_arguments(self) -> None:
        repo_root = Path(__file__).resolve().parents[1]
        script_path = repo_root / "scripts" / "generate-release-notes.sh"

        with tempfile.TemporaryDirectory() as tmpdir:
            tmp_path = Path(tmpdir)
            output_path = tmp_path / "changelog.md"
            config_path = tmp_path / "cliff.toml"
            config_path.write_text("[changelog]\n", encoding="utf-8")

            stub_dir = tmp_path / "bin"
            stub_dir.mkdir()
            stub_path = stub_dir / "git-cliff"
            stub_path.write_text(
                "#!/usr/bin/env bash\n"
                "set -euo pipefail\n"
                "printf '%s\n' \"$*\" > \"$OUTPUT_FILE\"\n"
                "printf 'stub invoked\n'\n",
                encoding="utf-8",
            )
            stub_path.chmod(stub_path.stat().st_mode | stat.S_IEXEC)

            env = os.environ.copy()
            env["PATH"] = f"{stub_dir}:{env['PATH']}"
            env["OUTPUT_FILE"] = str(output_path)

            subprocess.run(
                [
                    "bash",
                    str(script_path),
                    "--output",
                    str(output_path),
                    "--config",
                    str(config_path),
                    "--latest",
                    "--strip-header",
                ],
                cwd=repo_root,
                check=True,
                env=env,
                capture_output=True,
                text=True,
            )

            self.assertTrue(output_path.exists())
            content = output_path.read_text(encoding="utf-8")
            self.assertIn("--config", content)
            self.assertIn("--latest", content)
            self.assertIn("--strip", content)
            self.assertIn("header", content)


if __name__ == "__main__":
    unittest.main()
