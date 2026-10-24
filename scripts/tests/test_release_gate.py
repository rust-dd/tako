import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


REPOSITORY = Path(__file__).resolve().parents[2]

FAKE_TOOL = '''#!/usr/bin/env python3
import json
import os
from pathlib import Path
import sys

name = Path(sys.argv[0]).name
args = sys.argv[1:]
with open(os.environ["TAKO_GATE_LOG"], "a", encoding="utf-8") as log:
    log.write(json.dumps({"tool": name, "args": args, "cwd": os.getcwd()}) + "\\n")
if name == "curl":
    print(json.dumps({"crate": {"newest_version": "0.0.0"}}))
if name == "cargo" and args[0] == "test" and "--features" in args:
    features = args[args.index("--features") + 1].split(",")
    if "compio" not in features and os.environ.get("TAKO_FAIL_TOKIO") == "1":
        sys.exit(73)
'''


def profile(arguments):
    if "--all-features" in arguments:
        return "all-features"
    if "--features" in arguments:
        return "tokio-rich"
    return "default"


class ReleaseGateTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.log = self.directory / "calls.jsonl"
        tools = self.directory / "bin"
        tools.mkdir()
        for name in ("cargo", "curl"):
            executable = tools / name
            executable.write_text(FAKE_TOOL, encoding="utf-8")
            executable.chmod(0o755)
        self.environment = {
            **os.environ,
            "PATH": f"{tools}{os.pathsep}{os.environ['PATH']}",
            "TAKO_GATE_LOG": str(self.log),
            "TAKO_FAIL_TOKIO": "0",
            "CARGO_INCREMENTAL": "0",
        }

    def calls(self):
        if not self.log.exists():
            return []
        return [json.loads(line) for line in self.log.read_text().splitlines()]

    def run_script(self, path, *arguments, cwd=REPOSITORY):
        return subprocess.run(
            ["bash", str(REPOSITORY / path), *arguments],
            cwd=cwd,
            env=self.environment,
            capture_output=True,
            text=True,
            timeout=10,
            check=False,
        )

    def assert_no_registry_or_publish(self):
        for call in self.calls():
            self.assertNotEqual(call["tool"], "curl", "registry accessed before the gate finished")
            self.assertNotEqual(call["args"][0], "publish", "publish ran before the gate finished")

    def test_tokio_only_test_failure_stops_release_before_registry_access(self):
        self.environment["TAKO_FAIL_TOKIO"] = "1"
        result = self.run_script("publish.sh")
        self.assert_no_registry_or_publish()
        self.assertEqual(result.returncode, 73, result.stdout + result.stderr)
        tests = [call for call in self.calls() if call["args"][0] == "test"]
        self.assertEqual([profile(call["args"]) for call in tests], ["default", "tokio-rich"])

    def test_gate_only_checks_all_profiles_without_registry_access_or_publish(self):
        result = self.run_script("publish.sh", "--gate-only")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assert_no_registry_or_publish()
        calls = self.calls()
        self.assertTrue(any(call["args"] == ["+nightly", "fmt", "--all", "--", "--check"] for call in calls))
        for action in ("test", "clippy"):
            selected = [call for call in calls if call["args"][0] == action]
            self.assertEqual([profile(call["args"]) for call in selected], ["default", "tokio-rich", "all-features"])
        rich = [call["args"] for call in calls if "--features" in call["args"]]
        feature_lists = [args[args.index("--features") + 1] for args in rich]
        self.assertEqual(feature_lists[0], feature_lists[1])
        features = set(feature_lists[0].split(","))
        required = {
            "ws", "sse", "proxy-protocol", "udp", "per-thread", "http3",
            "webtransport", "async-graphql", "validator", "garde", "queue-cron",
            "socket-activation", "vsock", "utoipa-yaml",
        }
        self.assertTrue(required.issubset(features))
        self.assertFalse(any("compio" in feature for feature in features))
        for call in calls:
            if call["args"][0] == "clippy":
                self.assertIn("--tests", call["args"])
                self.assertIn("--examples", call["args"])
                self.assertEqual(call["args"][-3:], ["--", "-D", "warnings"])

    def test_gate_only_cannot_skip_the_gate(self):
        result = self.run_script("publish.sh", "--gate-only", "--skip-gate")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.calls(), [])

    def test_helper_runs_from_another_directory(self):
        result = self.run_script("scripts/check-workspace.sh", "test", "tokio-rich", cwd=self.directory)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(len(self.calls()), 1)
        self.assertEqual(Path(self.calls()[0]["cwd"]).resolve(), REPOSITORY.resolve())
        self.assertEqual(profile(self.calls()[0]["args"]), "tokio-rich")

    def test_helper_rejects_unknown_actions_and_profiles(self):
        for arguments in (("publish", "default"), ("test", "unknown"), ("test",)):
            with self.subTest(arguments=arguments):
                result = self.run_script("scripts/check-workspace.sh", *arguments)
                self.assertEqual(result.returncode, 2)
                self.assertEqual(self.calls(), [])


if __name__ == "__main__":
    unittest.main()
