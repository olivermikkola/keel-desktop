#!/usr/bin/env python3
"""Fail-closed checks for scripts/verify_v06_taggable.sh (no live model / no tagging)."""

from __future__ import annotations

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "verify_v06_taggable.sh"


def run_verify(env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    merged = os.environ.copy()
    if env:
        merged.update(env)
    return subprocess.run(
        ["bash", str(SCRIPT)],
        cwd=ROOT,
        capture_output=True,
        text=True,
        env=merged,
    )


def write_fixture(root: Path, *, cargo_version: str = "0.7.1") -> None:
    """Minimal tree the taggable gate reads."""
    (root / "Cargo.toml").write_text(
        f'[workspace.package]\nversion = "{cargo_version}"\n',
        encoding="utf-8",
    )
    tauri = root / "src-tauri"
    tauri.mkdir(parents=True)
    (tauri / "tauri.conf.json").write_text(
        '{"version": "0.7.1"}\n',
        encoding="utf-8",
    )
    ui = root / "ui"
    ui.mkdir(parents=True)
    (ui / "index.html").write_text("Keel Desktop 0.7.1\n", encoding="utf-8")
    wf = root / ".github" / "workflows"
    wf.mkdir(parents=True)
    (wf / "release.yml").write_text(
        'on:\n  workflow_dispatch:\n    inputs:\n      tag:\n        default: "v0.7.1"\n',
        encoding="utf-8",
    )
    (wf / "eval-targets-evidence.yml").write_text("name: Eval targets evidence\n", encoding="utf-8")
    docs = root / "docs" / "releases"
    docs.mkdir(parents=True)
    (docs / "v0.7.1.md").write_text("# v0.7.1\n", encoding="utf-8")
    scripts = root / "scripts"
    scripts.mkdir(parents=True)
    (scripts / "publish_eval_targets.py").write_text("# stub\n", encoding="utf-8")
    # git rev-parse is optional in the script; init so branch/sha lines stay quiet.
    subprocess.run(["git", "init", "-q"], cwd=root, check=False)


class VerifyV06TaggableTests(unittest.TestCase):
    def test_current_checkout_passes(self) -> None:
        proc = run_verify()
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertIn("Version gate passed", proc.stdout)

    def test_wrong_cargo_version_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            fixture = Path(tmp)
            write_fixture(fixture, cargo_version="0.5.0")
            proc = run_verify(env={"KEEL_TAGGABLE_ROOT": str(fixture)})
            self.assertNotEqual(proc.returncode, 0, proc.stdout)
            combined = proc.stdout + proc.stderr
            self.assertIn("FAIL", combined)
            self.assertIn("Cargo.toml", combined)
            self.assertIn("Not taggable yet", combined)

    def test_missing_release_notes_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            fixture = Path(tmp)
            write_fixture(fixture)
            (fixture / "docs" / "releases" / "v0.7.1.md").unlink()
            proc = run_verify(env={"KEEL_TAGGABLE_ROOT": str(fixture)})
            self.assertNotEqual(proc.returncode, 0, proc.stdout)
            self.assertIn("docs/releases/v0.7.1.md", proc.stdout + proc.stderr)


if __name__ == "__main__":
    unittest.main()
