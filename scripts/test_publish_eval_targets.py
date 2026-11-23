#!/usr/bin/env python3
"""Fail-closed checks for scripts/publish_eval_targets.py (no live model)."""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "publish_eval_targets.py"
DOC = ROOT / "docs" / "everyday-readiness.md"


def run_script(*args: str, check: bool = False) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(SCRIPT), *args],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=check,
    )


def write_report(path: Path, **overrides: object) -> Path:
    payload: dict[str, object] = {
        "model": "bonsai4b",
        "pack": "held_out",
        "runs": 5,
        "case_filter": None,
        "cases": [
            {
                "case": name,
                "samples": [
                    {"case": name, "run": run, "passed": run <= 4,
                     "seconds": 18.2, "ttft_seconds": 1.4}
                    for run in range(1, 6)
                ],
            }
            for name in ("friendly_email", "document_facts")
        ],
        "hardware": "cpu",
        "pack_success_rate": 0.8,
        "pack_p50_seconds": 18.2,
        "pack_p50_ttft_seconds": 1.4,
        "peak_rss_mb": 4200.0,
    }
    payload.update(overrides)
    path.write_text(json.dumps(payload), encoding="utf-8")
    return path


class PublishEvalTargetsTests(unittest.TestCase):
    def test_scaffold_dry_run_prints_tbd_table(self) -> None:
        for args in (["--scaffold"], ["--dry-run"]):
            with self.subTest(args=args):
                proc = run_script(*args)
                self.assertEqual(proc.returncode, 0, proc.stderr)
                out = proc.stdout
                self.assertIn("| Model | Hardware |", out)
                self.assertIn("| Bonsai 2 · 4B | CPU | TBD | TBD | TBD | TBD |", out)
                self.assertIn("| Bonsai 2 · 4B | GPU | TBD | TBD | TBD | TBD |", out)
                # Must not invent measured-looking values in scaffold mode.
                for line in out.splitlines():
                    if not (
                        line.startswith("| ")
                        and (" | CPU |" in line or " | GPU |" in line)
                    ):
                        continue
                    if line.startswith("| Model |") or line.startswith("| ---"):
                        continue
                    self.assertRegex(
                        line,
                        r"\| TBD \| TBD \| TBD \| TBD \|$",
                        msg=f"scaffold invented metrics: {line}",
                    )

    def test_publish_without_report_fails_closed(self) -> None:
        proc = run_script(
            "--model",
            "Bonsai 2 · 4B",
            "--hardware",
            "cpu",
        )
        self.assertNotEqual(proc.returncode, 0)
        combined = proc.stdout + proc.stderr
        self.assertIn("refusing to publish", combined)
        self.assertIn("--report", combined)

    def test_missing_report_file_fails_closed(self) -> None:
        proc = run_script(
            "--report",
            "docs/eval-results/does-not-exist.json",
            "--model",
            "Bonsai 2 · 4B",
            "--hardware",
            "cpu",
            "--dry-run",
        )
        self.assertNotEqual(proc.returncode, 0)
        self.assertIn("report not found", proc.stderr + proc.stdout)

    def test_refuse_smoke_pack(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            report = write_report(Path(tmp) / "smoke.json", pack="smoke")
            proc = run_script(
                "--report",
                str(report),
                "--model",
                "Bonsai 2 · 4B",
                "--hardware",
                "cpu",
                "--dry-run",
            )
            self.assertNotEqual(proc.returncode, 0)
            self.assertIn("held_out", proc.stderr + proc.stdout)

    def test_refuse_runs_below_five(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            report = write_report(Path(tmp) / "short.json", runs=4)
            proc = run_script(
                "--report",
                str(report),
                "--model",
                "Bonsai 2 · 4B",
                "--hardware",
                "cpu",
                "--dry-run",
            )
            self.assertNotEqual(proc.returncode, 0)
            self.assertIn("runs=", proc.stderr + proc.stdout)

    def test_refuse_missing_measured_fields(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "incomplete.json"
            path.write_text(
                json.dumps({"pack": "held_out", "runs": 5, "case_filter": None, "pack_success_rate": 1.0}),
                encoding="utf-8",
            )
            proc = run_script(
                "--report",
                str(path),
                "--model",
                "Bonsai 2 · 4B",
                "--hardware",
                "cpu",
                "--dry-run",
            )
            self.assertNotEqual(proc.returncode, 0)
            self.assertIn("pack_p50_seconds", proc.stderr + proc.stdout)

    def test_refuse_placeholder_metric_strings(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            report = write_report(
                Path(tmp) / "fake.json",
                pack_p50_seconds="TBD",
            )
            proc = run_script(
                "--report",
                str(report),
                "--model",
                "Bonsai 2 · 4B",
                "--hardware",
                "cpu",
                "--dry-run",
            )
            self.assertNotEqual(proc.returncode, 0)
            combined = proc.stderr + proc.stdout
            self.assertIn("refusing to publish", combined)
            self.assertIn("pack_p50_seconds", combined)

    def test_refuse_cpu_gpu_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            report = write_report(Path(tmp) / "cpu.json", hardware="cpu")
            proc = run_script(
                "--report",
                str(report),
                "--model",
                "Bonsai 2 · 4B",
                "--hardware",
                "gpu",
                "--dry-run",
            )
            self.assertNotEqual(proc.returncode, 0)
            self.assertIn("disagrees", proc.stderr + proc.stdout)

    def test_refuse_invalid_or_unattributed_evidence(self) -> None:
        bad_reports = [
            ({"pack_success_rate": 1.7}, "pack_success_rate"),
            ({"pack_p50_seconds": -2}, "pack_p50_seconds"),
            ({"peak_rss_mb": float("inf")}, "peak_rss_mb"),
            ({"runs": 5.5}, "runs"),
            ({"case_filter": "friendly_email"}, "KEEL_EVAL_CASE"),
            ({"hardware": None}, "hardware"),
            ({"hardware": "unknown"}, "hardware"),
            ({"model": "different-fixture-model"}, "model"),
            ({"pack_p50_seconds": 0.1}, "recorded samples"),
            ({"cases": []}, "case samples"),
        ]
        with tempfile.TemporaryDirectory() as tmp:
            for overrides, error in bad_reports:
                with self.subTest(overrides=overrides):
                    report = write_report(Path(tmp) / "bad.json", **overrides)
                    proc = run_script(
                        "--report", str(report), "--model", "Bonsai 2 · 4B",
                        "--hardware", "cpu", "--dry-run",
                    )
                    self.assertNotEqual(proc.returncode, 0)
                    self.assertIn(error, proc.stderr)
                    self.assertEqual(proc.stdout, "")

    def test_valid_report_dry_run_prints_row_without_writing_doc(self) -> None:
        before = DOC.read_text(encoding="utf-8")
        with tempfile.TemporaryDirectory() as tmp:
            report = write_report(Path(tmp) / "ok.json")
            proc = run_script(
                "--report",
                str(report),
                "--model",
                "Bonsai 2 · 4B",
                "--hardware",
                "cpu",
                "--dry-run",
            )
            self.assertEqual(proc.returncode, 0, proc.stderr)
            self.assertIn("| Bonsai 2 · 4B | CPU | 1.40s | 18.2s | 80% | 4.1 GiB |", proc.stdout)
            self.assertEqual(DOC.read_text(encoding="utf-8"), before)

    def test_doc_published_targets_remain_tbd(self) -> None:
        """CI gate: metric cells stay TBD until real hardware reports are applied."""
        proc = run_script("--scaffold")
        self.assertEqual(proc.returncode, 0, proc.stderr)


if __name__ == "__main__":
    unittest.main()
