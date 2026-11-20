#!/usr/bin/env python3
"""Tests for scripts/deploy.sh and scripts/verify-deployment.sh using stub CLI binaries."""
import hashlib
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPTS_DIR = Path(__file__).resolve().parent
DEPLOY_SCRIPT = SCRIPTS_DIR / "deploy.sh"
VERIFY_SCRIPT = SCRIPTS_DIR / "verify-deployment.sh"

DUMMY_CONTRACT_ID = "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
LOCAL_WASM_CONTENT = b"\x00asm\x01\x00\x00\x00local_dummy_wasm_content"
MISMATCHED_WASM_CONTENT = b"\x00asm\x01\x00\x00\x00remote_mismatched_wasm"


def compute_sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


class TestDeploymentScripts(unittest.TestCase):
    def setUp(self):
        self.tmpdir = tempfile.TemporaryDirectory()
        self.tmppath = Path(self.tmpdir.name)

        # Initialize mock repo structure
        scripts_dir = self.tmppath / "scripts"
        scripts_dir.mkdir(parents=True, exist_ok=True)
        shutil.copy(DEPLOY_SCRIPT, scripts_dir / "deploy.sh")
        shutil.copy(VERIFY_SCRIPT, scripts_dir / "verify-deployment.sh")

        # Make sure copied scripts are executable
        (scripts_dir / "deploy.sh").chmod(0o755)
        (scripts_dir / "verify-deployment.sh").chmod(0o755)

        # Initialize git repository so git rev-parse --show-toplevel works
        subprocess.run(["git", "init"], cwd=str(self.tmppath), check=True, capture_output=True)
        subprocess.run(
            ["git", "config", "user.name", "Test"], cwd=str(self.tmppath), check=True, capture_output=True
        )
        subprocess.run(
            ["git", "config", "user.email", "test@example.com"],
            cwd=str(self.tmppath),
            check=True,
            capture_output=True,
        )

        # Create bin directory for CLI stubs
        self.bin_dir = self.tmppath / "bin"
        self.bin_dir.mkdir(parents=True, exist_ok=True)

        # Create fixture files
        self.fixture_local_wasm = self.tmppath / "fixture_local.wasm"
        self.fixture_local_wasm.write_bytes(LOCAL_WASM_CONTENT)

        self.fixture_mismatch_wasm = self.tmppath / "fixture_mismatch.wasm"
        self.fixture_mismatch_wasm.write_bytes(MISMATCHED_WASM_CONTENT)

        self._create_stubs()

    def tearDown(self):
        self.tmpdir.cleanup()

    def _create_stubs(self):
        # 1. Stub cargo executable
        cargo_stub = self.bin_dir / "cargo"
        cargo_stub.write_text(
            """#!/usr/bin/env bash
set -euo pipefail

if [[ "${STUB_CARGO_FAIL:-0}" == "1" ]]; then
  echo "error: simulated cargo build failure" >&2
  exit 101
fi

# Simulate building release WASM artifact
TARGET_DIR="target/wasm32v1-none/release"
mkdir -p "$TARGET_DIR"
cp "${FIXTURE_LOCAL_WASM}" "$TARGET_DIR/tricklepay_stream.wasm"
exit 0
""",
            encoding="utf-8",
        )
        cargo_stub.chmod(0o755)

        # 2. Stub stellar executable
        stellar_stub = self.bin_dir / "stellar"
        stellar_stub.write_text(
            """#!/usr/bin/env bash
set -euo pipefail

if [[ "${STUB_STELLAR_FAIL:-0}" == "1" ]]; then
  echo "error: simulated stellar cli failure" >&2
  exit 1
fi

cmd="${1:-}"
sub="${2:-}"

if [[ "$cmd" == "contract" && "$sub" == "deploy" ]]; then
  # Output simulated contract ID
  echo "${STUB_DEPLOY_CONTRACT_ID:-CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA}"
  exit 0
elif [[ "$cmd" == "contract" && "$sub" == "fetch" ]]; then
  # Parse --out-file argument
  out_file=""
  while [[ $# -gt 0 ]]; do
    if [[ "$1" == "--out-file" ]]; then
      out_file="$2"
      shift 2
    else
      shift
    fi
  done

  if [[ -z "$out_file" ]]; then
    echo "error: missing --out-file argument" >&2
    exit 2
  fi

  if [[ "${STUB_STELLAR_FETCH_MISMATCH:-0}" == "1" ]]; then
    cp "${FIXTURE_MISMATCH_WASM}" "$out_file"
  else
    cp "${FIXTURE_LOCAL_WASM}" "$out_file"
  fi
  exit 0
fi

echo "unknown stub command: $*" >&2
exit 1
""",
            encoding="utf-8",
        )
        stellar_stub.chmod(0o755)

    def _run_script(self, script_name: str, args: list, env_overrides: dict = None) -> subprocess.CompletedProcess:
        env = os.environ.copy()
        # Prepend stub bin directory to PATH
        env["PATH"] = f"{self.bin_dir}:{env.get('PATH', '')}"
        env["FIXTURE_LOCAL_WASM"] = str(self.fixture_local_wasm)
        env["FIXTURE_MISMATCH_WASM"] = str(self.fixture_mismatch_wasm)
        env["STUB_DEPLOY_CONTRACT_ID"] = DUMMY_CONTRACT_ID

        if env_overrides:
            env.update(env_overrides)

        script_path = str(self.tmppath / "scripts" / script_name)
        return subprocess.run(
            [script_path] + args,
            cwd=str(self.tmppath),
            capture_output=True,
            text=True,
            env=env,
        )

    # =========================================================================
    # deploy.sh test cases
    # =========================================================================

    def test_deploy_usage_no_args(self):
        """deploy.sh without identity argument must exit with code 1 and show usage."""
        result = self._run_script("deploy.sh", [])
        self.assertEqual(result.returncode, 1)
        self.assertIn("usage:", result.stderr.lower())
        self.assertIn("<identity-name>", result.stderr)

    def test_deploy_failing_build_aborts(self):
        """deploy.sh must fail immediately and not deploy if cargo build fails."""
        result = self._run_script("deploy.sh", ["alice"], env_overrides={"STUB_CARGO_FAIL": "1"})
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("simulated cargo build failure", result.stderr)
        self.assertNotIn("Deployment Successful!", result.stdout)

    def test_deploy_success_match_and_hash(self):
        """deploy.sh with valid identity builds, deploys, and outputs contract ID and sha256 hash."""
        expected_hash = compute_sha256(LOCAL_WASM_CONTENT)
        result = self._run_script("deploy.sh", ["alice"])

        self.assertEqual(result.returncode, 0)
        self.assertIn("Building optimized WASM...", result.stdout)
        self.assertIn("Deploying to testnet as 'alice'...", result.stdout)
        self.assertIn("Deployment Successful!", result.stdout)
        self.assertIn(f"Contract ID : {DUMMY_CONTRACT_ID}", result.stdout)
        self.assertIn(f"WASM Hash   : sha256:{expected_hash}", result.stdout)

    def test_deploy_custom_network(self):
        """deploy.sh honors the NETWORK environment variable."""
        result = self._run_script("deploy.sh", ["carol"], env_overrides={"NETWORK": "futurenet"})
        self.assertEqual(result.returncode, 0)
        self.assertIn("Deploying to futurenet as 'carol'...", result.stdout)

    # =========================================================================
    # verify-deployment.sh test cases
    # =========================================================================

    def test_verify_bad_arguments_exit_2(self):
        """verify-deployment.sh with invalid argument counts must exit with code 2."""
        # 0 arguments
        res_0 = self._run_script("verify-deployment.sh", [])
        self.assertEqual(res_0.returncode, 2)
        self.assertIn("Usage:", res_0.stderr)

        # 1 argument
        res_1 = self._run_script("verify-deployment.sh", ["C12345"])
        self.assertEqual(res_1.returncode, 2)
        self.assertIn("Usage:", res_1.stderr)

        # 3 arguments
        res_3 = self._run_script("verify-deployment.sh", ["C12345", "testnet", "extra"])
        self.assertEqual(res_3.returncode, 2)
        self.assertIn("Usage:", res_3.stderr)

    def test_verify_failing_build_aborts(self):
        """verify-deployment.sh must exit non-zero if local cargo build fails."""
        result = self._run_script(
            "verify-deployment.sh",
            ["C12345", "testnet"],
            env_overrides={"STUB_CARGO_FAIL": "1"},
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("simulated cargo build failure", result.stderr)

    def test_verify_matching_deployment_exit_0(self):
        """verify-deployment.sh exits 0 when local WASM hash matches deployed WASM hash."""
        expected_hash = compute_sha256(LOCAL_WASM_CONTENT)
        result = self._run_script("verify-deployment.sh", ["C12345", "testnet"])

        self.assertEqual(result.returncode, 0)
        self.assertIn(f"local:    {expected_hash}", result.stdout)
        self.assertIn(f"deployed: {expected_hash}", result.stdout)
        self.assertIn("Deployment matches the local build.", result.stdout)

    def test_verify_mismatching_deployment_exit_1(self):
        """verify-deployment.sh exits 1 when local WASM hash differs from deployed WASM hash."""
        local_hash = compute_sha256(LOCAL_WASM_CONTENT)
        deployed_hash = compute_sha256(MISMATCHED_WASM_CONTENT)
        result = self._run_script(
            "verify-deployment.sh",
            ["C12345", "testnet"],
            env_overrides={"STUB_STELLAR_FETCH_MISMATCH": "1"},
        )

        self.assertEqual(result.returncode, 1)
        self.assertIn(f"local:    {local_hash}", result.stdout)
        self.assertIn(f"deployed: {deployed_hash}", result.stdout)
        self.assertIn("Deployment does not match the local build.", result.stderr)


if __name__ == "__main__":
    unittest.main()
