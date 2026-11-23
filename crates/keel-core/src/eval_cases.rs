//! Held-out local eval cases for `evaluate_local`.
//!
//! This is the expanding reliability suite (v0.6 pillar C). It is separate from
//! Settings **Check this model** (`model_check`), which stays a short demotion gate.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::apps::AppPreset;
use crate::compaction::ChatTurnMessage;
use crate::documents::Document;
use crate::sessions::{ChatSession, DesktopConfig, UiMessage, UiToolLog};
use crate::tools::{StagedEdit, StagedEditKind, StagedEditStatus};

/// Pack tag for the current five everyday smoke cases.
pub const PACK_SMOKE: &str = "smoke";

/// Larger held-out pack: smoke cases plus extra reliability stubs.
///
/// Run with `KEEL_EVAL_PACK=held_out`. Cases that need native UI or multi-turn
/// recovery are listed in [`HELD_OUT_TODOS`] until fixtures exist.
pub const PACK_HELD_OUT: &str = "held_out";

/// Planned held-out case ids that are not yet runnable (need richer harness).
pub const HELD_OUT_TODOS: &[&str] = &[
    // Needs a long multi-turn session that triggers compaction mid-run.
    "long_session_compaction",
    // Needs native UI interruption / restart while a turn is in flight.
    "native_recovery_mid_turn",
];

const SAVED_PREFERENCE_CONTEXT: &str = "I am vegetarian and prefer concise answers.";

const MEAT_WORDS: &[&str] = &["chicken", "beef", "pork", "salmon", "tuna", "turkey"];

const BURIED_CODEWORD: &str = "CEDAR";

/// How the example CLI prepares history / workspace for a case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvalSetup {
    /// User prompt only.
    Plain,
    /// Chat turn with an attached brief document.
    DocumentBrief,
    /// Needs a disposable workspace with CSV + Python fixtures.
    WorkspaceTools,
    /// Injects a saved vegetarian preference into instructions/history.
    SavedPreference,
    /// Saved vegetarian preference plus a casual chat claim that contradicts it.
    PreferenceConflict,
    /// Multi-turn history with a buried fact the model must recall.
    BuriedFact,
}

/// One registered eval case.
#[derive(Debug, Clone, Copy)]
pub struct EvalCase {
    pub name: &'static str,
    pub packs: &'static [&'static str],
    pub preset: AppPreset,
    pub prompt: &'static str,
    pub setup: EvalSetup,
}

/// Inputs the graders need after a turn finishes.
#[derive(Debug)]
pub struct GradeInput<'a> {
    pub answer: &'a str,
    pub tool_logs: &'a [UiToolLog],
    pub staged_edits: &'a mut [StagedEdit],
    pub workspace: Option<&'a Path>,
}

/// One case sample from a single run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseSample {
    pub case: String,
    pub run: usize,
    pub passed: bool,
    pub seconds: f64,
    /// Seconds until the first visible answer `ContentDelta`, when observed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttft_seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Aggregated metrics across `KEEL_EVAL_RUNS` for one case.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseAggregate {
    pub case: String,
    pub runs: usize,
    pub passed: usize,
    pub success_rate: f64,
    pub p50_seconds: f64,
    /// Median TTFT across samples that recorded one; omitted when none did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub p50_ttft_seconds: Option<f64>,
    pub samples: Vec<CaseSample>,
}

const BOTH_PACKS: &[&str] = &[PACK_SMOKE, PACK_HELD_OUT];
const HELD_OUT_ONLY: &[&str] = &[PACK_HELD_OUT];

const CASES: &[EvalCase] = &[
    // --- smoke (also included in held_out) ---
    EvalCase {
        name: "friendly_email",
        packs: BOTH_PACKS,
        preset: AppPreset::Chat,
        prompt: "Draft a friendly two-sentence email declining a meeting tomorrow and proposing Friday instead.",
        setup: EvalSetup::Plain,
    },
    EvalCase {
        name: "document_facts",
        packs: BOTH_PACKS,
        preset: AppPreset::Chat,
        prompt: "What are the budget and launch date in the attached brief? Cite the filename and keep the answer short.",
        setup: EvalSetup::DocumentBrief,
    },
    EvalCase {
        name: "csv_total",
        packs: BOTH_PACKS,
        preset: AppPreset::General,
        prompt: "Use run_sql to query sales.csv and report the sum of its revenue column. Give the exact total.",
        setup: EvalSetup::WorkspaceTools,
    },
    EvalCase {
        name: "code_fix",
        packs: BOTH_PACKS,
        preset: AppPreset::Code,
        prompt: "Read calculator.py and stage a fix so add(2, 3) returns 5. Use file tools and propose the edit; do not run shell commands. I will apply and test it after review.",
        setup: EvalSetup::WorkspaceTools,
    },
    EvalCase {
        name: "saved_preference",
        packs: BOTH_PACKS,
        preset: AppPreset::Chat,
        prompt: "Suggest one quick lunch for me in one sentence.",
        setup: EvalSetup::SavedPreference,
    },
    // --- held_out stubs (extra reliability coverage) ---
    EvalCase {
        name: "preference_conflict",
        packs: HELD_OUT_ONLY,
        preset: AppPreset::Chat,
        prompt: "Suggest one quick lunch for me in one sentence.",
        setup: EvalSetup::PreferenceConflict,
    },
    EvalCase {
        name: "csv_region_filter",
        packs: HELD_OUT_ONLY,
        preset: AppPreset::General,
        prompt: "Use run_sql on sales.csv to report only the West region revenue. Give the exact number.",
        setup: EvalSetup::WorkspaceTools,
    },
    EvalCase {
        name: "multi_file_code",
        packs: HELD_OUT_ONLY,
        preset: AppPreset::Code,
        prompt: "Read util.py and main.py. Stage a fix in util.py so that running main.py prints 5. Use file tools and propose the edit; do not run shell commands. I will apply and test it after review.",
        setup: EvalSetup::WorkspaceTools,
    },
    EvalCase {
        name: "buried_fact_recall",
        packs: HELD_OUT_ONLY,
        preset: AppPreset::Chat,
        prompt: "What is the project codeword from earlier in this conversation? Answer with only the codeword.",
        setup: EvalSetup::BuriedFact,
    },
    EvalCase {
        name: "unknown_from_brief",
        packs: HELD_OUT_ONLY,
        preset: AppPreset::Chat,
        prompt: "According to the attached brief, what is the legal counsel's name? Keep the answer short.",
        setup: EvalSetup::DocumentBrief,
    },
];

/// All registered runnable cases (smoke + held_out stubs).
pub fn all_cases() -> &'static [EvalCase] {
    CASES
}

/// Pack tags known to the registry.
pub fn known_packs() -> &'static [&'static str] {
    &[PACK_SMOKE, PACK_HELD_OUT]
}

/// Cases that carry the given pack tag.
pub fn cases_for_pack(pack: &str) -> Vec<&'static EvalCase> {
    CASES
        .iter()
        .filter(|case| case.packs.contains(&pack))
        .collect()
}

/// Look up a case by name.
pub fn case_by_name(name: &str) -> Option<&'static EvalCase> {
    CASES.iter().find(|case| case.name == name)
}

/// Write disposable CSV/code fixtures used by workspace cases.
pub fn write_workspace_fixtures(root: &Path) -> Result<()> {
    std::fs::create_dir_all(root)?;
    std::fs::write(
        root.join("sales.csv"),
        "region,revenue\nNorth,19\nSouth,31\nWest,50\n",
    )?;
    std::fs::write(
        root.join("calculator.py"),
        "def add(a, b):\n    return a - b\n",
    )?;
    // Multi-file coding stub: broken helper imported by main.py.
    std::fs::write(root.join("util.py"), "def double(n):\n    return n - n\n")?;
    std::fs::write(
        root.join("main.py"),
        "from util import double\n\nif __name__ == \"__main__\":\n    print(double(2) + 1)\n",
    )?;
    Ok(())
}

fn empty_session(case: &EvalCase, title: &str, messages: Vec<UiMessage>) -> ChatSession {
    ChatSession {
        id: "fixture".into(),
        title: title.into(),
        app_preset: case.preset,
        workspace_path: None,
        keel_session_log: None,
        code_checkout: None,
        applied_code_patch: None,
        last_code_test: None,
        project_id: None,
        pinned: false,
        staged_edits: vec![],
        updated_at: 0,
        messages,
    }
}

fn ui_text(id: &str, role: &str, content: &str) -> UiMessage {
    UiMessage {
        id: id.into(),
        role: role.into(),
        content: content.into(),
        thinking: None,
        tool_calls: vec![],
        status: "complete".into(),
        attachments: vec![],
    }
}

fn harbor_brief_attachment() -> Document {
    Document {
        id: "brief".into(),
        name: "harbor-brief.txt".into(),
        kind: "txt".into(),
        text: "Project Harbor. Approved budget: 4,800 euros. Launch date: 14 May 2027. The audience is local cafes.".into(),
        truncated: false,
        image_data_url: None,
    }
}

/// History + instructions the example CLI feeds into `run_agent_turn`.
pub fn prepare_turn(case: &EvalCase) -> (Vec<ChatTurnMessage>, String) {
    match case.setup {
        EvalSetup::Plain | EvalSetup::WorkspaceTools => (
            vec![ChatTurnMessage::text("user", case.prompt)],
            String::new(),
        ),
        EvalSetup::DocumentBrief => {
            let mut user = ui_text("user", "user", case.prompt);
            user.attachments = vec![harbor_brief_attachment()];
            let session = empty_session(case, "Brief", vec![user]);
            (crate::context::history(&session, None, 8192), String::new())
        }
        EvalSetup::SavedPreference => {
            let config = DesktopConfig {
                personal_context: SAVED_PREFERENCE_CONTEXT.into(),
                ..Default::default()
            };
            let session = empty_session(
                case,
                "Preference",
                vec![ui_text("user", "user", case.prompt)],
            );
            let history = crate::context::history_for_turn(&session, &config, None, 8192);
            let instructions = crate::context::instructions(&config, None);
            (history, instructions)
        }
        EvalSetup::PreferenceConflict => {
            // Saved prefs stay vegetarian; a casual chat claim must not override them.
            let config = DesktopConfig {
                personal_context: SAVED_PREFERENCE_CONTEXT.into(),
                ..Default::default()
            };
            let session = empty_session(
                case,
                "Preference conflict",
                vec![
                    ui_text(
                        "u1",
                        "user",
                        "By the way I eat chicken all the time now, just so you know.",
                    ),
                    ui_text(
                        "a1",
                        "assistant",
                        "Noted in chat. Your Saved preferences still say you are vegetarian.",
                    ),
                    ui_text("user", "user", case.prompt),
                ],
            );
            let history = crate::context::history_for_turn(&session, &config, None, 8192);
            let instructions = crate::context::instructions(&config, None);
            (history, instructions)
        }
        EvalSetup::BuriedFact => {
            let session = empty_session(
                case,
                "Buried fact",
                vec![
                    ui_text(
                        "u1",
                        "user",
                        &format!(
                            "For our internal notes, the project codeword is {BURIED_CODEWORD}. Do not bring it up unless asked."
                        ),
                    ),
                    ui_text(
                        "a1",
                        "assistant",
                        "Understood. I will keep that codeword for later.",
                    ),
                    ui_text("u2", "user", "Draft a one-line status update about the cafe launch."),
                    ui_text(
                        "a2",
                        "assistant",
                        "Cafe launch prep is on track for the May date.",
                    ),
                    ui_text("u3", "user", "Thanks. Keep answers short."),
                    ui_text("a3", "assistant", "Will do."),
                    ui_text("user", "user", case.prompt),
                ],
            );
            (crate::context::history(&session, None, 8192), String::new())
        }
    }
}

/// Dispatch to the grader for `case.name`.
pub fn grade(case: &EvalCase, input: GradeInput<'_>) -> Result<bool> {
    match case.name {
        "friendly_email" => Ok(grade_friendly_email(input.answer)),
        "document_facts" => Ok(grade_document_facts(input.answer)),
        "csv_total" => Ok(grade_csv_total(input.answer, input.tool_logs)),
        "code_fix" => {
            let Some(root) = input.workspace else {
                bail!("code_fix requires a workspace root");
            };
            grade_code_fix(root, input.staged_edits)
        }
        "saved_preference" | "preference_conflict" => Ok(grade_saved_preference(input.answer)),
        "csv_region_filter" => Ok(grade_csv_region_filter(input.answer, input.tool_logs)),
        "multi_file_code" => {
            let Some(root) = input.workspace else {
                bail!("multi_file_code requires a workspace root");
            };
            grade_multi_file_code(root, input.staged_edits)
        }
        "buried_fact_recall" => Ok(grade_buried_fact_recall(input.answer)),
        "unknown_from_brief" => Ok(grade_unknown_from_brief(input.answer)),
        other => bail!("unknown eval case: {other}"),
    }
}

pub fn grade_friendly_email(answer: &str) -> bool {
    let answer = answer.to_lowercase();
    answer.contains("friday") && answer.contains("tomorrow") && !answer.contains("<think>")
}

pub fn grade_document_facts(answer: &str) -> bool {
    let answer = answer.to_lowercase();
    (answer.contains("4,800") || answer.contains("4800"))
        && answer.contains("14")
        && answer.contains("may")
        && answer.contains("harbor-brief.txt")
}

pub fn grade_csv_total(answer: &str, tool_logs: &[UiToolLog]) -> bool {
    let answer = answer.to_lowercase();
    answer.contains("100") && tool_logs.iter().any(|t| t.name == "run_sql" && !t.is_error)
}

pub fn grade_saved_preference(answer: &str) -> bool {
    let answer = answer.to_lowercase();
    !MEAT_WORDS.iter().any(|word| answer.contains(word)) && !answer.trim().is_empty()
}

/// Applies staged edits in `workspace`, then checks `add(2, 3) == 5` with Python.
pub fn grade_code_fix(workspace: &Path, staged_edits: &mut [StagedEdit]) -> Result<bool> {
    let has_edit = !staged_edits.is_empty();
    for edit in staged_edits.iter_mut() {
        crate::tools::apply_staged_edit(workspace, edit)?;
    }
    let ok = has_edit
        && std::process::Command::new("python3")
            .arg("-c")
            .arg("from calculator import add; assert add(2,3)==5 and add(-1,1)==0")
            .current_dir(workspace)
            .status()?
            .success();
    Ok(ok)
}

pub fn grade_csv_region_filter(answer: &str, tool_logs: &[UiToolLog]) -> bool {
    let answer = answer.to_lowercase();
    answer.contains("50") && tool_logs.iter().any(|t| t.name == "run_sql" && !t.is_error)
}

/// Applies staged edits, then checks `python3 main.py` prints `5`.
pub fn grade_multi_file_code(workspace: &Path, staged_edits: &mut [StagedEdit]) -> Result<bool> {
    let touches_util = staged_edits.iter().any(|e| {
        e.path.ends_with("util.py")
            || e.new_path
                .as_deref()
                .is_some_and(|p| p.ends_with("util.py"))
    });
    if !touches_util {
        return Ok(false);
    }
    for edit in staged_edits.iter_mut() {
        crate::tools::apply_staged_edit(workspace, edit)?;
    }
    let output = std::process::Command::new("python3")
        .arg("main.py")
        .current_dir(workspace)
        .output()?;
    Ok(output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "5")
}

pub fn grade_buried_fact_recall(answer: &str) -> bool {
    answer.to_uppercase().contains(BURIED_CODEWORD)
}

/// Passes when the model refuses to invent a counsel name not in the brief.
pub fn grade_unknown_from_brief(answer: &str) -> bool {
    let answer = answer.to_lowercase();
    let hedges = [
        "not",
        "doesn't",
        "does not",
        "don't",
        "do not",
        "unknown",
        "unspecified",
        "isn't",
        "is not",
        "no name",
        "cannot",
        "can't",
        "missing",
        "absent",
    ];
    hedges.iter().any(|h| answer.contains(h)) && !answer.contains("<think>")
}

/// Median of `seconds` samples (average of the two middle values when even).
pub fn p50_seconds(seconds: &[f64]) -> f64 {
    if seconds.is_empty() {
        return 0.0;
    }
    let mut sorted = seconds.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        sorted[mid]
    } else {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    }
}

pub fn success_rate(passed: usize, runs: usize) -> f64 {
    if runs == 0 {
        0.0
    } else {
        passed as f64 / runs as f64
    }
}

/// Group samples by case name and compute success rate + p50 latency.
pub fn aggregate_samples(samples: &[CaseSample]) -> Vec<CaseAggregate> {
    let mut order: Vec<String> = Vec::new();
    for sample in samples {
        if !order.iter().any(|name| name == &sample.case) {
            order.push(sample.case.clone());
        }
    }
    order
        .into_iter()
        .map(|case| {
            let case_samples: Vec<CaseSample> =
                samples.iter().filter(|s| s.case == case).cloned().collect();
            let runs = case_samples.len();
            let passed = case_samples.iter().filter(|s| s.passed).count();
            let seconds: Vec<f64> = case_samples.iter().map(|s| s.seconds).collect();
            let ttfts: Vec<f64> = case_samples.iter().filter_map(|s| s.ttft_seconds).collect();
            CaseAggregate {
                case,
                runs,
                passed,
                success_rate: success_rate(passed, runs),
                p50_seconds: p50_seconds(&seconds),
                p50_ttft_seconds: if ttfts.is_empty() {
                    None
                } else {
                    Some(p50_seconds(&ttfts))
                },
                samples: case_samples,
            }
        })
        .collect()
}

/// Helper for unit tests: a pending edit that rewrites a relative path.
pub fn fixture_edit(path: &str, old: &str, new: &str) -> StagedEdit {
    StagedEdit {
        id: "fixture".into(),
        kind: StagedEditKind::Edit,
        path: path.into(),
        new_path: None,
        summary: "fixture".into(),
        old_content: Some(old.into()),
        new_content: Some(new.into()),
        diff: String::new(),
        added_lines: 0,
        removed_lines: 0,
        status: StagedEditStatus::Pending,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn smoke_pack_has_the_five_everyday_cases() {
        let names: Vec<_> = cases_for_pack(PACK_SMOKE).iter().map(|c| c.name).collect();
        assert_eq!(
            names,
            [
                "friendly_email",
                "document_facts",
                "csv_total",
                "code_fix",
                "saved_preference"
            ]
        );
    }

    #[test]
    fn held_out_pack_includes_smoke_plus_stubs() {
        let names: Vec<_> = cases_for_pack(PACK_HELD_OUT)
            .iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(
            names,
            [
                "friendly_email",
                "document_facts",
                "csv_total",
                "code_fix",
                "saved_preference",
                "preference_conflict",
                "csv_region_filter",
                "multi_file_code",
                "buried_fact_recall",
                "unknown_from_brief",
            ]
        );
        assert_eq!(all_cases().len(), 10);
        assert_eq!(known_packs(), &[PACK_SMOKE, PACK_HELD_OUT]);
        // Planned cases stay documented and distinct from runnable names.
        for todo in HELD_OUT_TODOS {
            assert!(
                case_by_name(todo).is_none(),
                "todo case {todo} must not be runnable yet"
            );
            assert!(!names.contains(todo));
        }
    }

    #[test]
    fn friendly_email_grader_fixture_answers() {
        assert!(grade_friendly_email(
            "Sorry I cannot make tomorrow. Friday works better for me."
        ));
        assert!(!grade_friendly_email("Let's meet on Friday."));
        assert!(!grade_friendly_email(
            "<think>planning</think>Sorry about tomorrow; try Friday."
        ));
    }

    #[test]
    fn document_facts_grader_fixture_answers() {
        assert!(grade_document_facts(
            "Budget 4,800 euros; launch 14 May 2027 (harbor-brief.txt)."
        ));
        assert!(grade_document_facts(
            "From harbor-brief.txt: budget 4800, launch 14 May."
        ));
        assert!(!grade_document_facts("Budget is 4800 euros on 14 May."));
        assert!(!grade_document_facts(
            "Budget unknown; see harbor-brief.txt for May."
        ));
    }

    #[test]
    fn csv_total_grader_requires_answer_and_successful_sql() {
        let ok_log = UiToolLog {
            name: "run_sql".into(),
            arguments: json!({}),
            result: Some("100".into()),
            is_error: false,
            edit_id: None,
        };
        let bad_log = UiToolLog {
            name: "run_sql".into(),
            arguments: json!({}),
            result: Some("error".into()),
            is_error: true,
            edit_id: None,
        };
        assert!(grade_csv_total(
            "The total is 100.",
            std::slice::from_ref(&ok_log)
        ));
        assert!(!grade_csv_total("The total is 100.", &[bad_log]));
        assert!(!grade_csv_total("The total is 99.", &[ok_log]));
        assert!(!grade_csv_total("The total is 100.", &[]));
    }

    #[test]
    fn saved_preference_grader_rejects_meat() {
        assert!(grade_saved_preference("A chickpea wrap."));
        assert!(!grade_saved_preference("Grilled chicken salad."));
        assert!(!grade_saved_preference("   "));
        assert!(!grade_saved_preference("Salmon bowl."));
    }

    #[test]
    fn code_fix_grader_applies_edit_and_checks_python() {
        let root = std::env::temp_dir().join(format!("keel_eval_grade_{}", uuid::Uuid::new_v4()));
        write_workspace_fixtures(&root).unwrap();
        let old = "def add(a, b):\n    return a - b\n";
        let new = "def add(a, b):\n    return a + b\n";
        let mut edits = vec![fixture_edit("calculator.py", old, new)];
        assert!(grade_code_fix(&root, &mut edits).unwrap());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn code_fix_grader_fails_without_edits() {
        let root = std::env::temp_dir().join(format!("keel_eval_grade_{}", uuid::Uuid::new_v4()));
        write_workspace_fixtures(&root).unwrap();
        let mut edits: Vec<StagedEdit> = vec![];
        assert!(!grade_code_fix(&root, &mut edits).unwrap());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn aggregation_computes_success_rate_and_p50() {
        let samples = vec![
            CaseSample {
                case: "friendly_email".into(),
                run: 1,
                passed: true,
                seconds: 10.0,
                ttft_seconds: Some(1.0),
                answer: None,
                error: None,
            },
            CaseSample {
                case: "friendly_email".into(),
                run: 2,
                passed: false,
                seconds: 20.0,
                ttft_seconds: Some(3.0),
                answer: None,
                error: None,
            },
            CaseSample {
                case: "friendly_email".into(),
                run: 3,
                passed: true,
                seconds: 30.0,
                ttft_seconds: Some(5.0),
                answer: None,
                error: None,
            },
            CaseSample {
                case: "csv_total".into(),
                run: 1,
                passed: true,
                seconds: 5.0,
                ttft_seconds: None,
                answer: None,
                error: None,
            },
        ];
        let agg = aggregate_samples(&samples);
        assert_eq!(agg.len(), 2);
        assert_eq!(agg[0].case, "friendly_email");
        assert_eq!(agg[0].passed, 2);
        assert_eq!(agg[0].runs, 3);
        assert!((agg[0].success_rate - 2.0 / 3.0).abs() < 1e-9);
        assert!((agg[0].p50_seconds - 20.0).abs() < 1e-9);
        assert!((agg[0].p50_ttft_seconds.unwrap() - 3.0).abs() < 1e-9);
        assert_eq!(agg[1].case, "csv_total");
        assert!((agg[1].success_rate - 1.0).abs() < 1e-9);
        assert!(agg[1].p50_ttft_seconds.is_none());
    }

    #[test]
    fn p50_handles_empty_even_and_odd() {
        assert_eq!(p50_seconds(&[]), 0.0);
        assert!((p50_seconds(&[3.0]) - 3.0).abs() < 1e-9);
        assert!((p50_seconds(&[2.0, 8.0]) - 5.0).abs() < 1e-9);
        assert!((p50_seconds(&[1.0, 2.0, 100.0, 200.0]) - 51.0).abs() < 1e-9);
        assert_eq!(success_rate(0, 0), 0.0);
        assert!((success_rate(1, 4) - 0.25).abs() < 1e-9);
    }

    #[test]
    fn held_out_graders_cover_stubs() {
        let ok_log = UiToolLog {
            name: "run_sql".into(),
            arguments: json!({}),
            result: Some("50".into()),
            is_error: false,
            edit_id: None,
        };
        assert!(grade_csv_region_filter(
            "West revenue is 50.",
            std::slice::from_ref(&ok_log)
        ));
        assert!(!grade_csv_region_filter("West revenue is 50.", &[]));
        assert!(grade_buried_fact_recall("CEDAR"));
        assert!(grade_buried_fact_recall("The codeword is cedar."));
        assert!(!grade_buried_fact_recall("I forget."));
        assert!(grade_unknown_from_brief(
            "The brief does not name legal counsel."
        ));
        assert!(grade_unknown_from_brief(
            "Unknown — not in harbor-brief.txt."
        ));
        assert!(!grade_unknown_from_brief("Legal counsel is Alex Rivera."));
        assert!(grade_saved_preference("A chickpea wrap."));
        // Conflict case reuses the vegetarian grader.
        assert!(!grade_saved_preference("Chicken sandwich."));
    }

    #[test]
    fn multi_file_code_grader_applies_util_edit() {
        let root = std::env::temp_dir().join(format!("keel_eval_multi_{}", uuid::Uuid::new_v4()));
        write_workspace_fixtures(&root).unwrap();
        let old = "def double(n):\n    return n - n\n";
        let new = "def double(n):\n    return n + n\n";
        let mut edits = vec![fixture_edit("util.py", old, new)];
        assert!(grade_multi_file_code(&root, &mut edits).unwrap());
        let mut wrong = vec![fixture_edit("calculator.py", "x", "y")];
        assert!(!grade_multi_file_code(&root, &mut wrong).unwrap());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn prepare_turn_injects_brief_preference_conflict_and_buried_fact() {
        let brief = case_by_name("document_facts").unwrap();
        let (history, instructions) = prepare_turn(brief);
        assert!(instructions.is_empty());
        assert!(!history.is_empty());

        let pref = case_by_name("saved_preference").unwrap();
        let (_history, instructions) = prepare_turn(pref);
        assert!(instructions.to_lowercase().contains("vegetarian"));

        let conflict = case_by_name("preference_conflict").unwrap();
        let (history, instructions) = prepare_turn(conflict);
        assert!(instructions.to_lowercase().contains("vegetarian"));
        let blob = format!("{history:?}").to_lowercase();
        assert!(blob.contains("chicken"));

        let buried = case_by_name("buried_fact_recall").unwrap();
        let (history, _) = prepare_turn(buried);
        let blob = format!("{history:?}");
        assert!(blob.contains(BURIED_CODEWORD));
        assert!(history.len() >= 4);
    }

    #[test]
    fn every_registered_case_has_a_grader() {
        for case in all_cases() {
            // Empty workspace / tools: graders must not panic; unknown names must not exist.
            let root =
                std::env::temp_dir().join(format!("keel_eval_dispatch_{}", uuid::Uuid::new_v4()));
            write_workspace_fixtures(&root).unwrap();
            let mut edits: Vec<StagedEdit> = vec![];
            let result = grade(
                case,
                GradeInput {
                    answer: "",
                    tool_logs: &[],
                    staged_edits: &mut edits,
                    workspace: Some(root.as_path()),
                },
            );
            assert!(result.is_ok(), "grader missing for {}", case.name);
            let _ = std::fs::remove_dir_all(&root);
        }
    }
}
