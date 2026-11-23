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

const SAVED_PREFERENCE_CONTEXT: &str = "I am vegetarian and prefer concise answers.";

const MEAT_WORDS: &[&str] = &["chicken", "beef", "pork", "salmon", "tuna", "turkey"];

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
    pub samples: Vec<CaseSample>,
}

const CASES: &[EvalCase] = &[
    EvalCase {
        name: "friendly_email",
        packs: &[PACK_SMOKE],
        preset: AppPreset::Chat,
        prompt: "Draft a friendly two-sentence email declining a meeting tomorrow and proposing Friday instead.",
        setup: EvalSetup::Plain,
    },
    EvalCase {
        name: "document_facts",
        packs: &[PACK_SMOKE],
        preset: AppPreset::Chat,
        prompt: "What are the budget and launch date in the attached brief? Cite the filename and keep the answer short.",
        setup: EvalSetup::DocumentBrief,
    },
    EvalCase {
        name: "csv_total",
        packs: &[PACK_SMOKE],
        preset: AppPreset::General,
        prompt: "Use run_sql to query sales.csv and report the sum of its revenue column. Give the exact total.",
        setup: EvalSetup::WorkspaceTools,
    },
    EvalCase {
        name: "code_fix",
        packs: &[PACK_SMOKE],
        preset: AppPreset::Code,
        prompt: "Read calculator.py and stage a fix so add(2, 3) returns 5. Use file tools and propose the edit; do not run shell commands. I will apply and test it after review.",
        setup: EvalSetup::WorkspaceTools,
    },
    EvalCase {
        name: "saved_preference",
        packs: &[PACK_SMOKE],
        preset: AppPreset::Chat,
        prompt: "Suggest one quick lunch for me in one sentence.",
        setup: EvalSetup::SavedPreference,
    },
];

/// All registered cases (currently the smoke pack).
pub fn all_cases() -> &'static [EvalCase] {
    CASES
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
    Ok(())
}

/// History + instructions the example CLI feeds into `run_agent_turn`.
pub fn prepare_turn(case: &EvalCase) -> (Vec<ChatTurnMessage>, String) {
    match case.setup {
        EvalSetup::Plain | EvalSetup::WorkspaceTools => {
            (vec![ChatTurnMessage::text("user", case.prompt)], String::new())
        }
        EvalSetup::DocumentBrief => {
            let session = ChatSession {
                id: "fixture".into(),
                title: "Brief".into(),
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
                messages: vec![UiMessage {
                    id: "user".into(),
                    role: "user".into(),
                    content: case.prompt.into(),
                    thinking: None,
                    tool_calls: vec![],
                    status: "complete".into(),
                    attachments: vec![Document {
                        id: "brief".into(),
                        name: "harbor-brief.txt".into(),
                        kind: "txt".into(),
                        text: "Project Harbor. Approved budget: 4,800 euros. Launch date: 14 May 2027. The audience is local cafes.".into(),
                        truncated: false,
                        image_data_url: None,
                    }],
                }],
            };
            (crate::context::history(&session, None, 8192), String::new())
        }
        EvalSetup::SavedPreference => {
            let config = DesktopConfig {
                personal_context: SAVED_PREFERENCE_CONTEXT.into(),
                ..Default::default()
            };
            let session = ChatSession {
                id: "fixture".into(),
                title: "Preference".into(),
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
                messages: vec![UiMessage {
                    id: "user".into(),
                    role: "user".into(),
                    content: case.prompt.into(),
                    thinking: None,
                    tool_calls: vec![],
                    status: "complete".into(),
                    attachments: vec![],
                }],
            };
            let history = crate::context::history_for_turn(&session, &config, None, 8192);
            let instructions = crate::context::instructions(&config, None);
            (history, instructions)
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
        "saved_preference" => Ok(grade_saved_preference(input.answer)),
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
    answer.contains("100")
        && tool_logs
            .iter()
            .any(|t| t.name == "run_sql" && !t.is_error)
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
            let case_samples: Vec<CaseSample> = samples
                .iter()
                .filter(|s| s.case == case)
                .cloned()
                .collect();
            let runs = case_samples.len();
            let passed = case_samples.iter().filter(|s| s.passed).count();
            let seconds: Vec<f64> = case_samples.iter().map(|s| s.seconds).collect();
            CaseAggregate {
                case,
                runs,
                passed,
                success_rate: success_rate(passed, runs),
                p50_seconds: p50_seconds(&seconds),
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
        let names: Vec<_> = cases_for_pack(PACK_SMOKE)
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
                "saved_preference"
            ]
        );
        assert_eq!(all_cases().len(), 5);
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
        assert!(grade_csv_total("The total is 100.", std::slice::from_ref(&ok_log)));
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
                answer: None,
                error: None,
            },
            CaseSample {
                case: "friendly_email".into(),
                run: 2,
                passed: false,
                seconds: 20.0,
                answer: None,
                error: None,
            },
            CaseSample {
                case: "friendly_email".into(),
                run: 3,
                passed: true,
                seconds: 30.0,
                answer: None,
                error: None,
            },
            CaseSample {
                case: "csv_total".into(),
                run: 1,
                passed: true,
                seconds: 5.0,
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
        assert_eq!(agg[1].case, "csv_total");
        assert!((agg[1].success_rate - 1.0).abs() < 1e-9);
    }

    #[test]
    fn prepare_turn_injects_brief_and_preference() {
        let brief = case_by_name("document_facts").unwrap();
        let (history, instructions) = prepare_turn(brief);
        assert!(instructions.is_empty());
        assert!(!history.is_empty());

        let pref = case_by_name("saved_preference").unwrap();
        let (_history, instructions) = prepare_turn(pref);
        assert!(instructions.to_lowercase().contains("vegetarian"));
    }
}
