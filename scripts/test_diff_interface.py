#!/usr/bin/env python3
"""Tests for scripts/diff-interface.sh covering missing WASM, missing snapshot, diff failure, and match."""
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPTS_DIR = Path(__file__).resolve().parent
DIFF_SCRIPT = SCRIPTS_DIR / "diff-interface.sh"


class TestDiffInterface(unittest.TestCase):
    def setUp(self):
        self.tmpdir = tempfile.TemporaryDirectory()
        self.tmppath = Path(self.tmpdir.name)

        # Set up mock repository structure
        self.scripts_dir = self.tmppath / "scripts"
        self.scripts_dir.mkdir(parents=True, exist_ok=True)
        shutil.copy(DIFF_SCRIPT, self.scripts_dir / "diff-interface.sh")
        (self.scripts_dir / "diff-interface.sh").chmod(0o755)

        self.docs_dir = self.tmppath / "docs"
        self.docs_dir.mkdir(parents=True, exist_ok=True)

        self.wasm_dir = self.tmppath / "target" / "wasm32v1-none" / "release"
        self.wasm_dir.mkdir(parents=True, exist_ok=True)
        self.wasm_file = self.wasm_dir / "tricklepay_stream.wasm"

        # Create bin directory for stub stellar CLI
        self.bin_dir = self.tmppath / "bin"
        self.bin_dir.mkdir(parents=True, exist_ok=True)
        self._create_stellar_stub()

    def tearDown(self):
        self.tmpdir.cleanup()

    def _create_stellar_stub(self):
        stellar_stub = self.bin_dir / "stellar"
        stellar_stub.write_text(
            """#!/usr/bin/env bash
set -euo pipefail

# Check for contract info interface command
if [[ "${1:-}" == "contract" && "${2:-}" == "info" && "${3:-}" == "interface" ]]; then
  if [[ -n "${STUB_STELLAR_INTERFACE_FILE:-}" && -f "${STUB_STELLAR_INTERFACE_FILE}" ]]; then
    cat "${STUB_STELLAR_INTERFACE_FILE}"
  else
    printf "%s\n" "${STUB_STELLAR_INTERFACE_OUTPUT:-// default stub interface}"
  fi
  exit 0
fi

echo "unknown stub command: $*" >&2
exit 1
""",
            encoding="utf-8",
        )
        stellar_stub.chmod(0o755)

    def _run_script(self, env_overrides: dict = None) -> subprocess.CompletedProcess:
        env = os.environ.copy()
        env["PATH"] = f"{self.bin_dir}:{env.get('PATH', '')}"
        if env_overrides:
            env.update(env_overrides)

        script_path = str(self.scripts_dir / "diff-interface.sh")
        return subprocess.run(
            [script_path],
            cwd=str(self.tmppath),
            capture_output=True,
            text=True,
            env=env,
        )

    def test_missing_wasm_artifact_fails(self):
        """diff-interface.sh must fail with exit code 1 when the WASM file is missing."""
        # Create expected docs/interface.txt, but leave WASM missing
        (self.docs_dir / "interface.txt").write_text("pub trait Contract {}", encoding="utf-8")
        if self.wasm_file.exists():
            self.wasm_file.unlink()

        result = self._run_script()
        self.assertEqual(result.returncode, 1)
        self.assertIn("missing Wasm artifact:", result.stderr)
        self.assertIn("run 'make wasm' before checking the interface", result.stderr)

    def test_missing_interface_snapshot_fails(self):
        """diff-interface.sh must fail with exit code 1 when docs/interface.txt is missing."""
        # Create WASM file, but leave docs/interface.txt missing
        self.wasm_file.write_bytes(b"\x00asm\x01\x00\x00\x00")
        snapshot_file = self.docs_dir / "interface.txt"
        if snapshot_file.exists():
            snapshot_file.unlink()

        result = self._run_script()
        self.assertEqual(result.returncode, 1)
        self.assertIn("missing interface snapshot:", result.stderr)

    def test_changed_interface_fails_with_diff(self):
        """diff-interface.sh must fail with exit code 1 when interface has changed."""
        self.wasm_file.write_bytes(b"\x00asm\x01\x00\x00\x00")
        (self.docs_dir / "interface.txt").write_text(
            "pub trait Contract {\n    fn original_fn();\n}\n", encoding="utf-8"
        )

        mock_interface = self.tmppath / "new_interface.txt"
        mock_interface.write_text("pub trait Contract {\n    fn modified_fn();\n}\n", encoding="utf-8")

        result = self._run_script(env_overrides={"STUB_STELLAR_INTERFACE_FILE": str(mock_interface)})
        self.assertEqual(result.returncode, 1)
        self.assertIn("contract interface changed; regenerate docs/interface.txt", result.stderr)

    def test_matching_interface_succeeds(self):
        """diff-interface.sh must exit 0 and print 'Interface matches.' when interfaces match."""
        self.wasm_file.write_bytes(b"\x00asm\x01\x00\x00\x00")
        interface_content = "pub trait Contract {\n    fn matching_fn();\n}\n"
        (self.docs_dir / "interface.txt").write_text(interface_content, encoding="utf-8")

        mock_interface = self.tmppath / "matching_interface.txt"
        mock_interface.write_text(interface_content, encoding="utf-8")

        result = self._run_script(env_overrides={"STUB_STELLAR_INTERFACE_FILE": str(mock_interface)})
        self.assertEqual(result.returncode, 0)
        self.assertIn("Interface matches.", result.stdout)


if __name__ == "__main__":
    unittest.main()
