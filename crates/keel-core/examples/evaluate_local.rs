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
//!
//! Stdout is a JSON report with per-case `success_rate` and `p50_seconds`.
//! See `docs/everyday-readiness.md` § Continuous evidence for where to archive it.
use anyhow::{Context, Result};
use keel_core::{
    agent::{run_agent_turn, ApprovalDecision, TurnParams},
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
                    },
                    &mut HashSet::new(),
                    |_| {},
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
                        answer: Some(outcome.content),
                        error: None,
                    }
                }
                Ok(Err(error)) => CaseSample {
                    case: case.name.into(),
                    run,
                    passed: false,
                    seconds,
                    answer: None,
                    error: Some(error.to_string()),
                },
                Err(_) => CaseSample {
                    case: case.name.into(),
                    run,
                    passed: false,
                    seconds,
                    answer: None,
                    error: Some("Timed out after 240 seconds".into()),
                },
            };
            eprintln!(
                "{} run {}: {} ({:.1}s)",
                sample.case,
                run,
                if sample.passed { "PASS" } else { "FAIL" },
                seconds
            );
            samples.push(sample);
        }
    }

    let aggregates = aggregate_samples(&samples);
    let report = json!({
        "model": model,
        "pack": pack,
        "runs": runs,
        "cases": aggregates,
        "note": "Small opt-in task checks, not a comparison with Claude or a broad quality benchmark. Read answers as well as pass flags."
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    let _ = std::fs::remove_dir_all(root);
    Ok(())
}
