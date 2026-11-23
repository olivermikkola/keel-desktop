#!/usr/bin/env python3
"""Apply a measured evaluate_local JSON report into docs/everyday-readiness.md.

Does not invent numbers. Requires:
  --report PATH.json   from KEEL_EVAL_OUT / tee
  --model LABEL        row label matching the Published targets table (e.g. "Bonsai 2 · 4B")
  --hardware cpu|gpu   row hardware column

Only fills Success rate and p50 latency (and TTFT / Peak RSS when present in the report).
Leaves other cells untouched. Refuses GPU fills when hardware=cpu metadata disagrees.

Example:
  python3 scripts/publish_eval_targets.py \\
    --report docs/eval-results/2026-11-23-bonsai4b-cpu.json \\
    --model "Bonsai 2 · 4B" --hardware cpu --dry-run
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path


DOC = Path("docs/everyday-readiness.md")


def load_report(path: Path) -> dict:
    data = json.loads(path.read_text(encoding="utf-8"))
    if data.get("pack") != "held_out":
        raise SystemExit(
            f"refusing to publish: pack is {data.get('pack')!r}; need held_out"
        )
    runs = int(data.get("runs") or 0)
    if runs < 5:
        raise SystemExit(
            f"refusing to publish: runs={runs}; need KEEL_EVAL_RUNS≥5 for targets"
        )
    return data


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
    replacement = (
        f"{match.group(1)} {ttft} | {p50} | {success} | {rss} |"
    )
    return pattern.sub(replacement, doc, count=1)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--model", required=True, help="Exact Model column text")
    parser.add_argument("--hardware", required=True, choices=("cpu", "gpu", "metal"))
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
            "Print the would-be Published targets row only; do not write the doc. "
            "Still requires a real held_out report (runs≥5); never invents TTFT/p50."
        ),
    )
    args = parser.parse_args()

    report = load_report(args.report)
    report_hw = (report.get("hardware") or "").lower()
    hw_col = "GPU" if args.hardware in ("gpu", "metal") else "CPU"
    if report_hw and report_hw not in (args.hardware, "metal" if args.hardware == "gpu" else args.hardware):
        # soft check: allow missing hardware in older reports
        if report_hw == "cpu" and args.hardware != "cpu":
            raise SystemExit(
                f"report hardware={report_hw!r} disagrees with --hardware {args.hardware}"
            )
        if report_hw in ("gpu", "metal") and args.hardware == "cpu":
            raise SystemExit(
                f"report hardware={report_hw!r} disagrees with --hardware {args.hardware}"
            )

    cells = {
        "p50": fmt_seconds(report.get("pack_p50_seconds")),
        "success": fmt_rate(report.get("pack_success_rate")),
    }
    if "pack_p50_ttft_seconds" in report:
        cells["ttft"] = fmt_seconds(report.get("pack_p50_ttft_seconds"))
    if "peak_rss_mb" in report:
        cells["rss"] = fmt_rss(report.get("peak_rss_mb"))

    text = args.doc.read_text(encoding="utf-8")
    patched = patch_row(text, args.model, hw_col, cells)
    if args.dry_run:
        for line in patched.splitlines():
            if args.model in line and f"| {hw_col} |" in line:
                print(line)
                break
        return 0
    args.doc.write_text(patched, encoding="utf-8")
    print(f"updated {args.doc} for {args.model} / {hw_col}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
