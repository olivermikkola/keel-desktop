#!/usr/bin/env python3
"""Apply a measured evaluate_local JSON report into docs/everyday-readiness.md.

Does not invent numbers. Two modes:

  Scaffold / dry-run without a report
    python3 scripts/publish_eval_targets.py --scaffold
    python3 scripts/publish_eval_targets.py --dry-run
    Prints the Published targets table. Metric cells must stay TBD until a
    real held_out measurement is applied. Never fabricates TTFT/p50/success/RSS.

  Apply (or dry-run) from a measured report
    python3 scripts/publish_eval_targets.py \\
      --report docs/eval-results/2026-11-23-bonsai4b-cpu.json \\
      --model "Bonsai 2 · 4B" --hardware cpu --dry-run
    Requires matching model/hardware and at least five full held-out runs with samples.
    Fills only cells present in the report; leaves the rest untouched.

Refuses to write the doc when --report is missing. Requires the report model and hardware to match the requested row.
"""

from __future__ import annotations

import argparse
import json
import math
import re
import statistics
import sys
from pathlib import Path
from typing import Any


DOC = Path("docs/everyday-readiness.md")

# Rows in the Published targets table (Model column · Hardware column).
PUBLISHED_MODELS = {
    "LFM3 · 1.7B": {"lfm3-1.7b"},
    "MiniCPM5 · 2B": {"minicpm5-2b"},
    "Bonsai 2 · 4B": {"bonsai-4b", "bonsai4b"},
    "Bonsai 2 · 9B": {"bonsai-9b", "bonsai9b"},
    "Bonsai 2 · 27B": {"bonsai-27b", "bonsai27b"},
}
PUBLISHED_HARDWARE = ("CPU", "GPU")

METRIC_HEADERS = ("TTFT", "p50 latency", "Success rate", "Peak RSS")


def load_report(path: Path) -> dict[str, Any]:
    if not path.is_file():
        raise SystemExit(
            f"refusing to publish: report not found: {path} "
            "(run evaluate_local with KEEL_EVAL_OUT; do not invent metrics)"
        )
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as err:
        raise SystemExit(f"refusing to publish: invalid JSON in {path}: {err}") from err
    if not isinstance(data, dict):
        raise SystemExit("refusing to publish: report root must be a JSON object")

    if data.get("pack") != "held_out":
        raise SystemExit(
            f"refusing to publish: pack is {data.get('pack')!r}; need held_out"
        )
    runs = data.get("runs")
    if type(runs) is not int or runs < 5:
        raise SystemExit(f"refusing to publish: runs={runs!r}; need at least 5 complete runs")
    if "case_filter" not in data or data["case_filter"] is not None:
        raise SystemExit("refusing to publish: rerun the complete held_out pack without KEEL_EVAL_CASE")

    rate = require_metric(data, "pack_success_rate", maximum=1)
    latency = require_metric(data, "pack_p50_seconds")
    ttft = require_metric(data, "pack_p50_ttft_seconds") if "pack_p50_ttft_seconds" in data else None
    if "peak_rss_mb" in data:
        require_metric(data, "peak_rss_mb")

    samples = validated_samples(data.get("cases"), runs)
    expected_rate = sum(sample["passed"] for sample in samples) / len(samples)
    expected_latency = statistics.median(sample["seconds"] for sample in samples)
    ttfts = [sample["ttft_seconds"] for sample in samples if "ttft_seconds" in sample]
    expected_ttft = statistics.median(ttfts) if ttfts else None
    for key, actual, expected in (
        ("pack_success_rate", rate, expected_rate),
        ("pack_p50_seconds", latency, expected_latency),
        ("pack_p50_ttft_seconds", ttft, expected_ttft),
    ):
        if actual is None and expected is None:
            continue
        if actual is None or expected is None or not math.isclose(actual, expected, abs_tol=1e-9):
            raise SystemExit(f"refusing to publish: {key} disagrees with the recorded samples")
    return data


def validated_samples(cases: object, runs: int) -> list[dict[str, Any]]:
    if not isinstance(cases, list) or len(cases) < 2:
        raise SystemExit("refusing to publish: full held_out case samples are required")

    names: set[str] = set()
    samples: list[dict[str, Any]] = []
    for case in cases:
        name = case.get("case") if isinstance(case, dict) else None
        if not isinstance(name, str) or not name or name in names:
            raise SystemExit("refusing to publish: each case needs a unique name")
        names.add(name)
        case_samples = case.get("samples")
        if not isinstance(case_samples, list) or len(case_samples) != runs:
            raise SystemExit(f"refusing to publish: {name} needs exactly {runs} run samples")
        seen: set[int] = set()
        for sample in case_samples:
            run = sample.get("run") if isinstance(sample, dict) else None
            if (
                type(run) is not int
                or not 1 <= run <= runs
                or run in seen
                or type(sample.get("passed")) is not bool
                or sample.get("case") != name
            ):
                raise SystemExit(f"refusing to publish: invalid or duplicate sample for {name}")
            seen.add(run)
            duration = require_metric(sample, "seconds")
            if "ttft_seconds" in sample:
                require_metric(sample, "ttft_seconds", maximum=duration)
            samples.append(sample)
    return samples


def require_metric(data: dict[str, Any], key: str, *, maximum: float = math.inf) -> float:
    value = data.get(key)
    number = math.nan
    if type(value) in (float, int):
        try:
            number = float(value)
        except OverflowError:
            pass
    if not math.isfinite(number) or not 0 <= number <= maximum:
        limit = f" up to {maximum:g}" if math.isfinite(maximum) else ""
        raise SystemExit(f"refusing to publish: {key} needs a finite nonnegative number{limit}")
    return number


def fmt_seconds(value: float | None) -> str:
    if value is None:
        return "TBD"
    if value >= 100:
        return f"{value:.0f}s"
    if value >= 10:
        return f"{value:.1f}s"
    return f"{value:.2f}s"


def fmt_rate(value: float | None) -> str:
    if value is None:
        return "TBD"
    return f"{value * 100:.0f}%"


def fmt_rss(mb: float | None) -> str:
    if mb is None:
        return "TBD"
    if mb >= 1024:
        return f"{mb / 1024:.1f} GiB"
    return f"{mb:.0f} MiB"


def published_targets_section(doc: str) -> str:
    """Return the markdown table under ## Published targets (header + rows)."""
    match = re.search(
        r"## Published targets\n.*?(\| Model \| Hardware \|.*?)(?:\n## |\Z)",
        doc,
        re.DOTALL,
    )
    if not match:
        raise SystemExit("no Published targets table found in doc")
    table = match.group(1).strip()
    if "| Model | Hardware |" not in table:
        raise SystemExit("Published targets table missing expected header")
    return table


def assert_scaffold_all_tbd(table: str) -> None:
    """Fail closed if any metric cell is filled without going through measurement."""
    row_re = re.compile(
        r"^\| (?P<model>.+?) \| (?P<hw>CPU|GPU) \| "
        r"(?P<ttft>[^|]+) \| (?P<p50>[^|]+) \| (?P<success>[^|]+) \| (?P<rss>[^|]+) \|$",
        re.MULTILINE,
    )
    rows = list(row_re.finditer(table))
    if not rows:
        raise SystemExit("Published targets table has no model rows")
    expected = {(m, h) for m in PUBLISHED_MODELS for h in PUBLISHED_HARDWARE}
    found = {(m.group("model").strip(), m.group("hw")) for m in rows}
    if found != expected:
        raise SystemExit(
            "Published targets rows do not match expected model×hardware scaffold: "
            f"missing={sorted(expected - found)} unexpected={sorted(found - expected)}"
        )
    invented: list[str] = []
    for match in rows:
        for label, key in zip(METRIC_HEADERS, ("ttft", "p50", "success", "rss"), strict=True):
            cell = match.group(key).strip()
            if cell != "TBD":
                invented.append(
                    f"{match.group('model').strip()} / {match.group('hw')} {label}={cell!r}"
                )
    if invented:
        raise SystemExit(
            "refusing scaffold: Published targets still contain non-TBD metric cells "
            "(do not invent numbers; fill only via measured --report). Offenders: "
            + "; ".join(invented)
        )


def patch_row(doc: str, model: str, hardware: str, cells: dict[str, str]) -> str:
    # Table rows look like: | Model | Hardware | TTFT | p50 | Success | Peak RSS |
    pattern = re.compile(
        rf"^(\| {re.escape(model)} \| {re.escape(hardware)} \|) ([^|]+) \| ([^|]+) \| ([^|]+) \| ([^|]+) \|$",
        re.MULTILINE,
    )
    match = pattern.search(doc)
    if not match:
        raise SystemExit(f"no Published targets row for {model!r} / {hardware!r}")

    ttft = cells.get("ttft", match.group(2).strip())
    p50 = cells.get("p50", match.group(3).strip())
    success = cells.get("success", match.group(4).strip())
    rss = cells.get("rss", match.group(5).strip())
    replacement = f"{match.group(1)} {ttft} | {p50} | {success} | {rss} |"
    return pattern.sub(replacement, doc, count=1)


def check_report_identity(report: dict[str, Any], model: str, hardware: str) -> None:
    if report.get("hardware") != hardware:
        raise SystemExit(
            f"refusing to publish: report hardware={report.get('hardware')!r} disagrees with --hardware {hardware}"
        )
    allowed_models = PUBLISHED_MODELS.get(model, set()) | {model}
    report_model = report.get("model")
    if not isinstance(report_model, str) or report_model not in allowed_models:
        raise SystemExit(
            f"refusing to publish: report model={report_model!r} disagrees with --model {model!r}"
        )


def run_scaffold(doc_path: Path) -> int:
    text = doc_path.read_text(encoding="utf-8")
    table = published_targets_section(text)
    assert_scaffold_all_tbd(table)
    print("# Published targets scaffold (metric cells TBD until measured)")
    print(table)
    print()
    print(
        "No metrics invented. Fill cells only after held_out runs "
        "(KEEL_EVAL_RUNS≥5) via --report. See Continuous evidence in "
        f"{doc_path}."
    )
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument(
        "--report",
        type=Path,
        help="Measured evaluate_local JSON (required to fill cells; omit for scaffold)",
    )
    parser.add_argument("--model", help="Exact Model column text")
    parser.add_argument("--hardware", choices=("cpu", "gpu", "metal"))
    parser.add_argument(
        "--doc",
        type=Path,
        default=DOC,
        help="Markdown file with the Published targets table",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help=(
            "Without --report: print the TBD scaffold table and refuse non-TBD cells. "
            "With --report: print the would-be row only; do not write the doc. "
            "Never invents TTFT/p50/success/RSS."
        ),
    )
    parser.add_argument(
        "--scaffold",
        action="store_true",
        help="Print the Published targets scaffold (all metric cells must be TBD)",
    )
    args = parser.parse_args(argv)

    if args.scaffold or (args.dry_run and args.report is None):
        if args.report is not None:
            raise SystemExit("--scaffold does not take --report (omit --report)")
        return run_scaffold(args.doc)

    if args.report is None:
        raise SystemExit(
            "refusing to publish: --report is required to fill metric cells "
            "(use --scaffold or --dry-run with no report to print the TBD table; "
            "do not invent TTFT/p50/success/RSS)"
        )
    if not args.model or not args.hardware:
        raise SystemExit(
            "refusing to publish: --model and --hardware are required with --report"
        )

    report = load_report(args.report)
    check_report_identity(report, args.model, args.hardware)
    hw_col = "GPU" if args.hardware in ("gpu", "metal") else "CPU"

    cells = {
        "p50": fmt_seconds(float(report["pack_p50_seconds"])),
        "success": fmt_rate(float(report["pack_success_rate"])),
    }
    if "pack_p50_ttft_seconds" in report:
        cells["ttft"] = fmt_seconds(float(report["pack_p50_ttft_seconds"]))
    if "peak_rss_mb" in report:
        cells["rss"] = fmt_rss(float(report["peak_rss_mb"]))

    text = args.doc.read_text(encoding="utf-8")
    patched = patch_row(text, args.model, hw_col, cells)
    if args.dry_run:
        for line in patched.splitlines():
            if args.model in line and f"| {hw_col} |" in line:
                print(line)
                break
        else:
            raise SystemExit(f"no Published targets row for {args.model!r} / {hw_col!r}")
        return 0
    args.doc.write_text(patched, encoding="utf-8")
    print(f"updated {args.doc} for {args.model} / {hw_col}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
