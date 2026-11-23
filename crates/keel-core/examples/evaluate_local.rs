//! Opt-in real-model checks. Run against a server you deliberately start locally.
//!
//! Case definitions and graders live in `keel_core::eval_cases`. This binary is a thin CLI.
//!
//! Env:
//! - `KEEL_EVAL_URL` (default `http://127.0.0.1:19181/v1`)
//! - `KEEL_EVAL_MODEL` (default `bonsai4b`)
//! - `KEEL_EVAL_CASE` — run only this case name
//! - `KEEL_EVAL_PACK` — pack tag filter (`smoke` default, or `held_out`)
//! - `KEEL_EVAL_RUNS` — repeat each case N times and aggregate (default `1`)
//! - `KEEL_EVAL_OUT` — optional path; write the same JSON report there (for `scripts/publish_eval_targets.py`)
//! - `KEEL_EVAL_HARDWARE` — optional label (`cpu` / `gpu` / `metal`) recorded in the report
//! - `KEEL_EVAL_PEAK_RSS_MB` — optional peak RSS of the model server (MiB), recorded in the report
//!
//! Stdout is a JSON report with per-case `success_rate`, `p50_seconds`, and `p50_ttft_seconds` when available.
//! See `docs/everyday-readiness.md` § Continuous evidence for where to archive it.
use anyhow::{Context, Result};
use keel_core::{
    agent::{run_agent_turn, AgentEvent, ApprovalDecision, TurnParams},
    apps::AppPreset,
    eval_cases::{
        aggregate_samples, case_by_name, cases_for_pack, grade, write_workspace_fixtures,
        CaseSample, EvalSetup, GradeInput, PACK_SMOKE,
    },
    mcp::McpConfigFile,
};
use serde_json::json;
use std::{collections::HashSet, time::Instant};

#[tokio::main]
async fn main() -> Result<()> {
    let url = std::env::var("KEEL_EVAL_URL").unwrap_or_else(|_| "http://127.0.0.1:19181/v1".into());
    let model = std::env::var("KEEL_EVAL_MODEL").unwrap_or_else(|_| "bonsai4b".into());
    let pack = std::env::var("KEEL_EVAL_PACK").unwrap_or_else(|_| PACK_SMOKE.into());
    let runs: usize = std::env::var("KEEL_EVAL_RUNS")
        .ok()
        .map(|v| v.parse())
        .transpose()
        .context("KEEL_EVAL_RUNS must be a positive integer")?
        .unwrap_or(1)
        .max(1);
    let selected_case = std::env::var("KEEL_EVAL_CASE").ok();
    let out_path = std::env::var("KEEL_EVAL_OUT").ok();
    let hardware = std::env::var("KEEL_EVAL_HARDWARE").ok();
    let peak_rss_mb = std::env::var("KEEL_EVAL_PEAK_RSS_MB")
        .ok()
        .map(|v| v.parse::<f64>())
        .transpose()
        .context("KEEL_EVAL_PEAK_RSS_MB must be a number")?;

    let cases: Vec<_> = if let Some(name) = selected_case.as_deref() {
        vec![case_by_name(name).with_context(|| format!("unknown KEEL_EVAL_CASE: {name}"))?]
    } else {
        cases_for_pack(&pack)
    };
    if cases.is_empty() {
        anyhow::bail!("no eval cases matched pack {pack:?}");
    }

    let root = std::env::temp_dir().join(format!("keel_eval_{}", uuid::Uuid::new_v4()));
    write_workspace_fixtures(&root)?;

    let mut samples = Vec::new();
    for run in 1..=runs {
        for case in &cases {
            if case.setup == EvalSetup::WorkspaceTools {
                write_workspace_fixtures(&root)?;
            }
            let (history, custom_instructions) = keel_core::eval_cases::prepare_turn(case);
            let start = Instant::now();
            let mut first_content_at: Option<f64> = None;
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(240),
                run_agent_turn(
                    TurnParams {
                        base_url: url.clone(),
                        model_name: model.clone(),
                        context_size: 8192,
                        app_preset: case.preset,
                        custom_instructions,
                        workspace_root: if case.preset == AppPreset::Chat {
                            None
                        } else {
                            Some(root.clone())
                        },
                        auto_apply_edits: false,
                        enable_thinking: false,
                        history,
                        mcp_config: McpConfigFile::default(),
                        mcp_tools: vec![],
                        max_steps: 6,
                        api_key: None,
                    },
                    &mut HashSet::new(),
                    |event| {
                        if first_content_at.is_none() {
                            if let AgentEvent::ContentDelta { text } = &event {
                                if !text.is_empty() {
                                    first_content_at = Some(start.elapsed().as_secs_f64());
                                }
                            }
                        }
                    },
                    |_, _, _, _| async { ApprovalDecision::Deny },
                ),
            )
            .await;
            let seconds = start.elapsed().as_secs_f64();
            let sample = match result {
                Ok(Ok(mut outcome)) => {
                    let passed = grade(
                        case,
                        GradeInput {
                            answer: &outcome.content,
                            tool_logs: &outcome.tool_logs,
                            staged_edits: &mut outcome.staged_edits,
                            workspace: Some(root.as_path()),
                        },
                    )
                    .unwrap_or(false);
                    CaseSample {
                        case: case.name.into(),
                        run,
                        passed,
                        seconds,
                        ttft_seconds: first_content_at,
                        answer: Some(outcome.content),
                        error: None,
                    }
                }
                Ok(Err(error)) => CaseSample {
                    case: case.name.into(),
                    run,
                    passed: false,
                    seconds,
                    ttft_seconds: first_content_at,
                    answer: None,
                    error: Some(error.to_string()),
                },
                Err(_) => CaseSample {
                    case: case.name.into(),
                    run,
                    passed: false,
                    seconds,
                    ttft_seconds: first_content_at,
                    answer: None,
                    error: Some("Timed out after 240 seconds".into()),
                },
            };
            eprintln!(
                "{} run {}: {} ({:.1}s{})",
                sample.case,
                run,
                if sample.passed { "PASS" } else { "FAIL" },
                seconds,
                sample
                    .ttft_seconds
                    .map(|t| format!(", ttft {t:.2}s"))
                    .unwrap_or_default()
            );
            samples.push(sample);
        }
    }

    let aggregates = aggregate_samples(&samples);
    let pack_passed: usize = aggregates.iter().map(|a| a.passed).sum();
    let pack_runs: usize = aggregates.iter().map(|a| a.runs).sum();
    let pack_latencies: Vec<f64> = aggregates.iter().map(|a| a.p50_seconds).collect();
    let pack_ttfts: Vec<f64> = aggregates
        .iter()
        .filter_map(|a| a.p50_ttft_seconds)
        .collect();
    let mut report = json!({
        "model": model,
        "pack": pack,
        "runs": runs,
        "cases": aggregates,
        "pack_success_rate": if pack_runs == 0 {
            0.0
        } else {
            pack_passed as f64 / pack_runs as f64
        },
        "pack_p50_seconds": keel_core::eval_cases::p50_seconds(&pack_latencies),
        "note": "Small opt-in task checks, not a comparison with Claude or a broad quality benchmark. Read answers as well as pass flags."
    });
    if let Some(hw) = &hardware {
        report["hardware"] = json!(hw);
    }
    if let Some(rss) = peak_rss_mb {
        report["peak_rss_mb"] = json!(rss);
    }
    if !pack_ttfts.is_empty() {
        report["pack_p50_ttft_seconds"] = json!(keel_core::eval_cases::p50_seconds(&pack_ttfts));
    }
    let pretty = serde_json::to_string_pretty(&report)?;
    println!("{pretty}");
    if let Some(path) = out_path {
        if let Some(parent) = std::path::Path::new(&path).parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, &pretty).with_context(|| format!("write KEEL_EVAL_OUT={path}"))?;
        eprintln!("wrote {path}");
    }
    let _ = std::fs::remove_dir_all(root);
    Ok(())
}
