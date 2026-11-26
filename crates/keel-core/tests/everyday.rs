use keel_core::{
    agent::AgentEvent,
    apps::AppPreset,
    documents::{self, Document},
    projects::{self, Project},
    recording::TurnRecorder,
    sessions::{self, ChatSession, DesktopConfig, UiMessage},
    streaming::{SseLines, TextRouter},
};
use std::io::Write;

fn temp() -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("keel_everyday_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    root
}
fn empty_session() -> ChatSession {
    ChatSession {
        id: "test-session".into(),
        title: "Test".into(),
        project_id: None,
        pinned: false,
        app_preset: AppPreset::Chat,
        workspace_path: None,
        keel_session_log: None,
        code_checkout: None,
        applied_code_patch: None,
        last_code_test: None,
        messages: vec![],
        staged_edits: vec![],
        updated_at: 0,
    }
}
fn user(content: &str) -> UiMessage {
    UiMessage {
        id: "user".into(),
        role: "user".into(),
        content: content.into(),
        thinking: None,
        tool_calls: vec![],
        attachments: vec![],
        status: "complete".into(),
    }
}
fn document(text: &str) -> Document {
    Document {
        id: "source".into(),
        name: "brief.txt".into(),
        kind: "txt".into(),
        text: text.into(),
        truncated: false,
        image_data_url: None,
    }
}

#[test]
fn documents_extract_text_docx_and_pdf_without_external_programs() {
    let root = temp();
    std::fs::write(root.join("notes.txt"), "Café budget: €4800").unwrap();
    assert_eq!(
        documents::import_document(&root.join("notes.txt"))
            .unwrap()
            .text,
        "Café budget: €4800"
    );
    let file = std::fs::File::create(root.join("brief.docx")).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    zip.start_file(
        "word/document.xml",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    zip.write_all(b"<w:document xmlns:w=\"w\"><w:body><w:p><w:r><w:t>Project Harbor &amp; Co</w:t></w:r></w:p><w:p><w:r><w:t>Budget 4800</w:t></w:r></w:p></w:body></w:document>").unwrap();
    zip.finish().unwrap();
    let doc = documents::import_document(&root.join("brief.docx")).unwrap();
    assert!(doc.text.contains("Harbor & Co\nBudget 4800"));
    assert_eq!(
        keel_core::tools::read_workspace_file(&root, "brief.docx", Some(2), Some(2)).unwrap(),
        "   2: Budget 4800\n"
    );
    // Small, valid text PDF with a built-in Helvetica font.
    let stream = "BT /F1 12 Tf 50 700 Td (Project Harbor budget 4800) Tj ET";
    let objects = ["<< /Type /Catalog /Pages 2 0 R >>".to_string(), "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(), "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".into(), "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(), format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len())];
    let mut pdf = "%PDF-1.4\n".to_string();
    let mut offsets = vec![0];
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.push_str(&format!("{} 0 obj\n{}\nendobj\n", index + 1, object));
    }
    let xref = pdf.len();
    pdf.push_str("xref\n0 6\n0000000000 65535 f \n");
    for offset in offsets.iter().skip(1) {
        pdf.push_str(&format!("{offset:010} 00000 n \n"));
    }
    pdf.push_str(&format!(
        "trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF"
    ));
    std::fs::write(root.join("brief.pdf"), pdf).unwrap();
    assert!(documents::import_document(&root.join("brief.pdf"))
        .unwrap()
        .text
        .contains("Harbor budget 4800"));
    assert!(
        keel_core::tools::read_workspace_file(&root, "brief.pdf", None, None)
            .unwrap()
            .contains("Harbor budget 4800")
    );
    std::fs::write(root.join("binary.txt"), [0xff, 0xfe, 0x00]).unwrap();
    assert!(documents::import_document(&root.join("binary.txt")).is_err());
    std::fs::write(root.join("blob.bin"), [0x01, 0x02, 0x03]).unwrap();
    assert!(documents::import_document(&root.join("blob.bin")).is_err());
    assert!(documents::is_image_extension("PNG"));
    assert!(documents::is_image_extension("jpeg"));
    assert!(!documents::is_image_extension("pdf"));
    let from_bytes = documents::import_document_bytes("pasted.md", b"# Notes\nReady").unwrap();
    assert_eq!(from_bytes.name, "pasted.md");
    assert_eq!(from_bytes.text, "# Notes\nReady");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn image_ocr_imports_and_reads_from_workspace() {
    let root = temp();
    // Render crisp 24-bit BMP containing bold letters "OK" so OCR works offline without external generators.
    let glyphs: [&[&str]; 2] = [
        &[
            " #### ", "#    #", "#    #", "#    #", "#    #", "#    #", " #### ",
        ],
        &[
            "#    #", "#   # ", "#  #  ", "###   ", "#  #  ", "#   # ", "#    #",
        ],
    ];
    let scale = 6usize;
    let pad = 28usize;
    let width = pad * 2 + (2 * 6 + 3) * scale;
    let height = pad * 2 + 7 * scale;
    let row_stride = (width * 3).div_ceil(4) * 4;
    let img_size = row_stride * height;
    let file_size = 54 + img_size;
    let mut bmp = Vec::with_capacity(file_size);
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&(file_size as u32).to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    bmp.extend_from_slice(&54u32.to_le_bytes());
    bmp.extend_from_slice(&40u32.to_le_bytes());
    bmp.extend_from_slice(&(width as i32).to_le_bytes());
    bmp.extend_from_slice(&(height as i32).to_le_bytes());
    bmp.extend_from_slice(&1u16.to_le_bytes());
    bmp.extend_from_slice(&24u16.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    bmp.extend_from_slice(&(img_size as u32).to_le_bytes());
    bmp.extend_from_slice(&2835i32.to_le_bytes());
    bmp.extend_from_slice(&2835i32.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    let mut pixels = vec![255u8; img_size];
    for (gi, glyph) in glyphs.iter().enumerate() {
        for (ry, row) in glyph.iter().enumerate() {
            for (rx, ch) in row.chars().enumerate() {
                if ch == '#' {
                    for dy in 0..scale {
                        for dx in 0..scale {
                            let x = pad + (gi * 9 + rx) * scale + dx;
                            let y = pad + ry * scale + dy;
                            let bmp_y = height - 1 - y;
                            let idx = bmp_y * row_stride + x * 3;
                            pixels[idx] = 0;
                            pixels[idx + 1] = 0;
                            pixels[idx + 2] = 0;
                        }
                    }
                }
            }
        }
    }
    bmp.extend_from_slice(&pixels);
    let img_path = root.join("receipt.bmp");
    std::fs::write(&img_path, bmp).unwrap();

    let doc = documents::import_document(&img_path).unwrap();
    assert_eq!(doc.kind, "bmp");
    assert!(doc
        .image_data_url
        .as_deref()
        .is_some_and(|u| u.starts_with("data:image/bmp;base64,")));
    if documents::ocr_available() {
        assert!(doc.text.contains("OK"), "OCR text was: {:?}", doc.text);
        let ws_text =
            keel_core::tools::read_workspace_file(&root, "receipt.bmp", None, None).unwrap();
        assert!(ws_text.contains("OK"), "workspace OCR was: {ws_text:?}");
    } else {
        assert!(
            doc.text.contains("Image pixels attached"),
            "expected vision fallback caption, got {:?}",
            doc.text
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn chat_history_attaches_image_pixels_for_vision() {
    let mut session = empty_session();
    let mut message = user("What is in this picture?");
    message.attachments = vec![Document {
        id: "shot".into(),
        name: "shot.png".into(),
        kind: "png".into(),
        text: "(Image pixels attached for vision. No OCR text was extracted.)".into(),
        truncated: false,
        image_data_url: Some("data:image/png;base64,aaa".into()),
    }];
    session.messages.push(message);
    let text_only = keel_core::context::history(&session, None, 8192);
    assert!(text_only.last().unwrap().image_urls.is_empty());
    let with_vision = keel_core::context::history_with_vision(&session, None, 8192, true);
    let last = with_vision.last().unwrap();
    assert_eq!(
        last.image_urls,
        vec!["data:image/png;base64,aaa".to_string()]
    );
    let api = last.to_api_value();
    assert_eq!(api["content"][1]["type"], "image_url");
    assert!(last.content.contains("Image pixels attached"));
}

#[test]
fn project_and_attachment_context_survives_followups_without_leaking_other_projects() {
    let root = temp();
    let project = Project {
        id: "harbor".into(),
        name: "Harbor".into(),
        instructions: "Write for local cafes".into(),
        workspace_path: None,
        documents: vec![document("The approved budget is 4800 euros.")],
    };
    projects::save_project(&root, &project).unwrap();
    let mut session = empty_session();
    session.project_id = Some(project.id.clone());
    let mut message = user("What is the launch date?");
    message.attachments = vec![Document {
        id: "dates".into(),
        name: "dates.txt".into(),
        text: "Launch: 14 May 2027".into(),
        ..document("")
    }];
    session.messages.push(message);
    session.messages.push(UiMessage {
        role: "assistant".into(),
        content: "14 May".into(),
        ..user("")
    });
    session.messages.push(UiMessage {
        id: "followup".into(),
        ..user("And the budget?")
    });
    let history = keel_core::context::history(&session, Some(&project), 8192);
    let latest = &history.last().unwrap().content;
    assert!(latest.contains("4800") && latest.contains("14 May") && latest.contains("brief.txt"));
    let other = keel_core::context::history(&empty_session(), None, 8192);
    assert!(other.is_empty());
    let cfg = DesktopConfig {
        personal_context: "I prefer vegetarian meals".into(),
        ..Default::default()
    };
    assert!(keel_core::context::instructions(&cfg, Some(&project)).contains("vegetarian"));
    assert!(keel_core::context::instructions(&cfg, Some(&project)).contains("local cafes"));
    assert!(!keel_core::context::instructions(&cfg, None).contains("local cafes"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn excerpts_are_bounded_unicode_safe_and_find_relevant_later_sections() {
    let text = format!(
        "{}{}{}",
        "irrelevant café ".repeat(200),
        "Launch budget is 4800 euros. ".repeat(30),
        "other material ".repeat(200)
    );
    let (snippet, partial) = documents::excerpt(&text, "What is the launch budget?", 2200);
    assert!(partial && snippet.chars().count() <= 2200);
    assert!(snippet.contains("4800"));

    // Short domain terms like Q4 and SQL must not be dropped by keyword filtering.
    let short_term_doc = format!(
        "{}\nQ4 SQL tax total is 9123 EUR.\n{}",
        "general filler paragraph line.\n".repeat(80),
        "trailing notes paragraph line.\n".repeat(80)
    );
    let (short_snip, short_partial) = documents::excerpt(&short_term_doc, "Q4 SQL tax?", 2200);
    assert!(short_partial && short_snip.contains("9123 EUR"));
}

#[test]
fn byte_streams_preserve_multilingual_text_and_split_reasoning_tags() {
    let text = "data: {\"text\":\"Hyvää päivää ☕ 日本語\"}\r\n\r\ndata: [DONE]";
    let mut decoder = SseLines::default();
    let mut lines = vec![];
    for byte in text.as_bytes() {
        lines.extend(decoder.push(&[*byte]).unwrap());
    }
    lines.extend(decoder.finish().unwrap());
    assert!(lines[0].contains("Hyvää päivää ☕ 日本語"));
    assert_eq!(lines[1], "[DONE]");
    let mut router = TextRouter::default();
    let mut output = vec![];
    for chunk in [
        "Hi <thi",
        "nk>quiet",
        " thoughts</thi",
        "nk> there <tool_",
        "call>{\"secret\":1}</tool_call>",
    ] {
        output.extend(router.push(chunk));
    }
    output.extend(router.finish());
    let visible: String = output
        .iter()
        .filter(|s| !s.0)
        .map(|s| s.1.as_str())
        .collect();
    let thoughts: String = output
        .iter()
        .filter(|s| s.0)
        .map(|s| s.1.as_str())
        .collect();
    assert_eq!(visible, "Hi  there ");
    assert_eq!(thoughts, "quiet thoughts");

    // Literal <think> inside a fenced code block stays visible instead of entering reasoning mode.
    let mut fence_router = TextRouter::default();
    let mut fence_out = vec![];
    for chunk in ["Example:\n```html\n<thi", "nk>demo</think>\n```\nDone"] {
        fence_out.extend(fence_router.push(chunk));
    }
    fence_out.extend(fence_router.finish());
    let fence_vis: String = fence_out
        .iter()
        .filter(|s| !s.0)
        .map(|s| s.1.as_str())
        .collect();
    let fence_think: String = fence_out
        .iter()
        .filter(|s| s.0)
        .map(|s| s.1.as_str())
        .collect();
    assert!(fence_vis.contains("<think>demo</think>"));
    assert!(fence_think.is_empty());
}

#[test]
fn interrupted_turns_keep_partial_output_and_staged_edits() {
    let root = temp();
    let mut session = empty_session();
    session.messages.push(user("Write a document"));
    let mut recorder = TurnRecorder::new(&root, session).unwrap();
    recorder
        .record(&AgentEvent::ContentDelta {
            text: "Here is a draft".into(),
        })
        .unwrap();
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let edit = keel_core::tools::build_staged_edit(
        &workspace,
        keel_core::tools::StagedEditKind::Create,
        "brief.md",
        None,
        Some("# Brief"),
        None,
    )
    .unwrap();
    recorder
        .record(&AgentEvent::StagedEditAdded { edit })
        .unwrap();
    drop(recorder); // A process exit loses in-memory state.
    sessions::recover_interrupted(&root).unwrap();
    let restored = sessions::load_session(&root, "test-session").unwrap();
    assert_eq!(restored.messages.last().unwrap().status, "failed");
    assert!(restored
        .messages
        .last()
        .unwrap()
        .content
        .contains("Here is a draft"));
    assert_eq!(restored.staged_edits.len(), 1);
    assert!(!workspace.join("brief.md").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn record_ids_cannot_escape_storage_and_legacy_messages_load() {
    let root = temp();
    assert!(sessions::load_session(&root, "../../secret").is_err());
    assert!(projects::load_project(&root, "/secret").is_err());
    let legacy = r#"{"id":"legacy","title":"Old conversation","app_preset":"general","workspace_path":null,"messages":[{"id":"m","role":"user","content":"Hello","thinking":null,"tool_calls":[]}],"staged_edits":[],"updated_at":1}"#;
    let session: ChatSession = serde_json::from_str(legacy).unwrap();
    assert!(session.project_id.is_none() && session.messages[0].attachments.is_empty());
    assert_eq!(session.messages[0].status, "complete");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn durable_memory_requires_an_explicit_request_and_is_applied_to_followup_context() {
    let mut config = DesktopConfig::default();
    assert!(!keel_core::context::remember_requested(&mut config, "I am vegetarian").unwrap());
    assert!(
        keel_core::context::remember_requested(&mut config, "Remember that I am vegetarian")
            .unwrap()
    );
    assert!(
        !keel_core::context::remember_requested(&mut config, "Remember that I am vegetarian")
            .unwrap()
    );
    let mut session = empty_session();
    session.messages.push(user("Suggest lunch"));
    let history = keel_core::context::history_for_turn(&session, &config, None, 8192);
    assert!(
        history[0].content.contains("I am vegetarian")
            && history[0].content.contains("Suggest lunch")
    );
}

#[cfg(unix)]
#[test]
fn nested_creates_searches_and_csv_mounts_stay_inside_the_workspace() {
    use std::os::unix::fs::symlink;
    let root = temp();
    let workspace = root.join("workspace");
    let outside = root.join("outside");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("private.csv"), "secret\nOUTSIDE_MARKER\n").unwrap();
    symlink(&outside, workspace.join("linked")).unwrap();
    symlink(outside.join("private.csv"), workspace.join("private.csv")).unwrap();
    symlink(outside.join("missing.txt"), workspace.join("broken.txt")).unwrap();
    assert!(keel_core::tools::confine_path(&workspace, "linked/new/nested/file.txt").is_err());
    assert!(keel_core::tools::confine_path(&workspace, "broken.txt").is_err());
    assert!(
        !keel_core::tools::search_workspace_files(&workspace, "OUTSIDE_MARKER")
            .unwrap()
            .contains("private.csv")
    );
    assert!(keel_core::sql_sandbox::run_sql_in_workspace(
        Some(&workspace),
        "SELECT * FROM private"
    )
    .is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn csv_queries_treat_quoted_newlines_as_one_record() {
    let root = temp();
    std::fs::write(
        root.join("notes.csv"),
        "name,note,amount\nA,\"first line\nsecond line\",10\nB,\"quoted \"\"word\"\"\",20\n",
    )
    .unwrap();
    let result = keel_core::sql_sandbox::run_sql_in_workspace(
        Some(&root),
        "SELECT COUNT(*) AS rows, SUM(amount) AS total FROM notes",
    )
    .unwrap();
    assert!(result.contains("2 | 30"), "{result}");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn continue_history_keeps_partial_interrupted_answer_without_footer_or_pure_errors() {
    let mut session = empty_session();
    session.messages.push(user("Write a 5-point outline."));
    session.messages.push(UiMessage {
        id: "stopped-partial".into(),
        role: "assistant".into(),
        content: "1. Introduction\n2. Background\n\nReply stopped. Your partial answer and proposed edits have been kept.".into(),
        status: "failed".into(),
        ..user("")
    });
    session.messages.push(UiMessage {
        id: "pure-error".into(),
        role: "assistant".into(),
        content: "Connection refused".into(),
        status: "failed".into(),
        ..user("")
    });
    session.messages.push(UiMessage {
        id: "continue-turn".into(),
        ..user("Continue where you left off.")
    });

    let history = keel_core::context::history(&session, None, 8192);
    assert_eq!(history.len(), 3);
    assert_eq!(history[0].role, "user");
    assert_eq!(history[1].role, "assistant");
    assert_eq!(history[1].content, "1. Introduction\n2. Background");
    assert_eq!(history[2].role, "user");
    assert_eq!(history[2].content, "Continue where you left off.");
}
