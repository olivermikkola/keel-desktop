# Everyday assistant — 0.5.0

The target is an assistant a person can use for recurring writing, document, data, and coding work without managing model-server commands. 0.4.0 made those workflows concrete; 0.5.0 adds approved web search, Chat vision, keep-alive, and a model check. It does not establish parity with Claude Pro, or justify a blanket recommendation to cancel a subscription.

## Ready for testing

- Guided local setup: a recommendation based on total RAM, a pinned/checksummed model-runner package, model download/verification, RAM-aware configurable context window, and managed startup. An existing compatible server can be connected instead. The bundled packages cover macOS Intel/Apple Silicon, Windows CPU x64/ARM64, and Linux x64/ARM64; runtime compatibility and performance still depend on the user's hardware and operating system.
- Keep-alive & model check: the graphics-or-processor choice reaches the runner, and a later message reuses that process when the model, context, and that choice are unchanged. Settings **Check this model** runs three short questions; a model that fails is not offered as the one for this computer (it stays installed and can still be started by hand). The status line shows how long the last answer took.
- Chat with attachments: drag-and-drop, clipboard image paste, or file picker import for pure-Rust PDF/DOCX extraction, text/CSV/code import, optional offline image text extraction via local Tesseract OCR, source inspection, and source context retained for follow-up questions. Files are limited to 20 MB and extracted text to 300,000 characters. Scanned PDFs need OCR. In Chat, image attachments also send image pixels as multimodal `image_url` parts when the model accepts them; OCR text is a caption/fallback. The model receives bounded excerpts when full source text will not fit; the prompt labels those excerpts explicitly.
- Web search: Chat and Work (and Code) can call `web_search` after explicit user approval. Results come from the public web over the network; Keel does not fall back to a cloud chat model.
- Projects: instructions, reusable reference documents, optional working folders, and related conversations. Project documents are not added to unrelated conversations.
- User-controlled preferences: explicit “Remember that …” requests or edits in Saved preferences. These are included in future model context and can be edited or cleared locally. This does not guarantee every model will obey them.
- Conversation management: content/title search, rename, pin, export Markdown, fork-before-edit, retry text-only answers, draft retention, and delete/undo. Deleted conversations move to the app's local trash directory; undo restores them. They are not securely erased by deletion.
- Library: approved workspace files and generated PNGs, linked back into Work. Files remain in their original folders.
- Recovery: a durable assistant draft records partial output and staged edits. Cancellation, server errors, and app restarts retain that state. File review waits until generation ends, preventing concurrent snapshot writes from losing an approval. Shell/MCP approvals are installed before the UI is notified and grants apply to the current thread/folder.
- Streaming: UTF-8 survives arbitrary byte boundaries; split reasoning and tool tags do not leak into the visible reply. Oversized tool output is explicitly shortened to fit context. Older conversation turns can be compacted; oversized current requests fail with a useful explanation instead of silently truncating the user's request.

## Verification available in the repository

- `cargo test -p keel-core`: unit and HTTP integration coverage, including file confinement, actual tool dispatch, staging/apply, PDF/DOCX/text extraction, source scope, explicit memory, UTF-8/tag fragmentation, interruption recovery, checked download resume, checksum failures, and archive path traversal.
- `node --test ui/tests/*.test.mjs`: state and Markdown rendering checks, including tables, code copying markup, and executable-link rejection.
- Linux CI also starts a real Tauri window under Xvfb with `--ui-smoke-test`, using a temporary app data directory. The actual frontend must bootstrap through native IPC, create its initial Chat session, and render its composer. This complements the fixture-based flow checks; it does not operate native file dialogs.
- `python scripts/ui-smoke.py`: real frontend with a test-only Tauri IPC fixture. Exercises guided setup, attachments/source preview, preferences, projects, thread rename/pin/search, Chat/Work/Code, file review, Library, Images, and sandboxed asset previews. Native pickers and GPU inference are not simulated as real interactions by this test.
- `KEEL_EVAL_MODEL_PATH=/path/model.gguf cargo run -p keel-core --example check_managed_setup`: an opt-in real download/start/stop check. It downloads and verifies the pinned runner, loads a supplied GGUF, checks its model alias, and stops it. Passed locally on Linux with Bonsai 4B and runner b12428.
- `KEEL_EVAL_URL=http://127.0.0.1:19181/v1 KEEL_EVAL_MODEL=model-name cargo run -p keel-core --example evaluate_local`: opt-in task checks using the actual agent engine and a running model. Case definitions and graders live in `keel_core::eval_cases`. Packs: `smoke` (five everyday cases) and `held_out` (smoke plus preference-conflict, CSV filter, multi-file code, buried-fact recall, and unknown-from-brief stubs; longer native/long-session cases remain TODOs in the registry). `KEEL_EVAL_CASE` restricts a run to one case; `KEEL_EVAL_PACK` selects a pack (default `smoke`); `KEEL_EVAL_RUNS` repeats each case and reports success rate plus p50 latency (default `1`). It creates disposable CSV/code fixtures; code checks apply staged edits in that fixture and check results with Python. Review answers as well as heuristic pass flags.

## Published targets

Formal targets for recommended model × hardware combinations. Fill a cell only after a measured run on that model × hardware with the held-out pack (`KEEL_EVAL_PACK=held_out`) and enough repeats (`KEEL_EVAL_RUNS`, typically ≥5). Columns:

| Column | Meaning |
| --- | --- |
| TTFT | Time to first visible answer token (not yet emitted by `evaluate_local`; leave TBD until instrumented) |
| p50 latency | Median wall-clock seconds per case from the JSON report (`p50_seconds`), then summarize across the pack as agreed when publishing |
| Success rate | Pack-level fraction of passed case samples (`passed / runs` aggregated across cases) |
| Peak RSS | Peak resident set size of the model server during the run (measure with OS tools; not in the JSON report yet) |

**Every cell below is TBD** until measured on the intended hardware. Do not treat the informal CPU samples in the next section as these targets. **GPU/Metal is unmeasured** — do not invent or infer GPU numbers from CPU runs.

| Model | Hardware | TTFT | p50 latency | Success rate | Peak RSS |
| --- | --- | --- | --- | --- | --- |
| LFM3 · 1.7B | CPU | TBD | TBD | TBD | TBD |
| LFM3 · 1.7B | GPU | TBD | TBD | TBD | TBD |
| MiniCPM5 · 2B | CPU | TBD | TBD | TBD | TBD |
| MiniCPM5 · 2B | GPU | TBD | TBD | TBD | TBD |
| Bonsai 2 · 4B | CPU | TBD | TBD | TBD | TBD |
| Bonsai 2 · 4B | GPU | TBD | TBD | TBD | TBD |
| Bonsai 2 · 9B | CPU | TBD | TBD | TBD | TBD |
| Bonsai 2 · 9B | GPU | TBD | TBD | TBD | TBD |
| Bonsai 2 · 27B | CPU | TBD | TBD | TBD | TBD |
| Bonsai 2 · 27B | GPU | TBD | TBD | TBD | TBD |

## Continuous evidence

Re-run against a deliberately started local OpenAI-compatible server:

```bash
# Smoke (default): five everyday cases, one pass each
KEEL_EVAL_URL=http://127.0.0.1:19181/v1 KEEL_EVAL_MODEL=bonsai4b \
  cargo run -p keel-core --example evaluate_local

# Held-out pack with repeats (use for published-target measurements)
KEEL_EVAL_URL=http://127.0.0.1:19181/v1 KEEL_EVAL_MODEL=bonsai4b \
  KEEL_EVAL_PACK=held_out KEEL_EVAL_RUNS=5 \
  cargo run -p keel-core --example evaluate_local | tee eval-held-out.json
```

Stdout is pretty-printed JSON shaped like:

```json
{
  "model": "bonsai4b",
  "pack": "held_out",
  "runs": 5,
  "cases": [
    {
      "case": "friendly_email",
      "runs": 5,
      "passed": 5,
      "success_rate": 1.0,
      "p50_seconds": 12.4,
      "samples": []
    }
  ],
  "note": "…"
}
```

Where results should land:

1. **Machine-readable:** keep the JSON next to the measurement notes (for example `docs/eval-results/<date>-<model>-<cpu|gpu>.json` locally or in a PR). Do not commit secrets or host-private paths.
2. **Human summary:** when a cell in Published targets is filled, cite the JSON (date, model id, runner build, `run_where`, pack, `KEEL_EVAL_RUNS`) in the PR or release notes. Update only measured cells; leave the rest TBD.
3. **Informal samples:** one-off development runs may stay under Observed real-model results below; they do not count as published targets.

Unit tests for graders and aggregation: `cargo test -p keel-core --lib eval`. Real-model runs stay opt-in (no CI GPU requirement).

## Observed real-model results

These are separate from the Published targets table above. Single CPU runs during development (Bonsai 4B Q2_0, 8K context, llama.cpp b12428; these are task samples, not representative benchmarks):

| Task | Result | Approximate elapsed time |
| --- | --- | --- |
| Two-sentence scheduling email | Passed content check | 15 seconds |
| Budget/date from an attached brief, with filename | Passed | 15 seconds |
| Sum CSV revenue through actual SQL tool calls | Passed | 73 seconds after narrowing the read-only tool profile; earlier run 95 seconds |
| Read Python source, stage an addition fix, apply and test | Passed | 113 seconds; no tool errors in the corrected run |
| Respect a saved vegetarian preference | Failed on the tested 4B prompt variants | 13–15 seconds |
| Same saved-preference check, Bonsai 9B Q2_0 | Passed | 39 seconds |

The saved-preference failure is material: persisting and supplying memory correctly does not make a small model follow it reliably. The larger recommendation passed this sample; it has not been proven reliable across a broad memory suite. CPU latency is also a limitation. GPU/Metal performance has not been measured here.

One additional email experiment showed why the reasoning switch needed a backend fix: a 256-token response used approximately 40 seconds reasoning without producing the requested email. Passing `enable_thinking: false` to the chat template produced the two-sentence answer in approximately 9 seconds in that experiment. Default direct answers now use the actual template option, not just a prompt instruction.

## Remaining bar for recommending a switch

1. Measure the held-out pack (`KEEL_EVAL_PACK=held_out`, `KEEL_EVAL_RUNS`≥5) on intended user hardware, then expand remaining TODOs (long-session compaction, native mid-turn recovery) and stronger project-document / multi-file coverage.
2. Fill Published targets (TTFT, p50 latency, success rate, peak RSS) per recommended model/hardware combination from those runs. Do not infer GPU results from CPU tests.
3. Test with people doing the recurring tasks they currently use Claude for, and compare completed work, correction effort, and time—not just UI appearance or a few successful prompts.
4. Address the failures those evaluations find. Hosted provider authentication and automatic audio/video packs remain separate unimplemented capabilities.
