# Keel as a local AI hub

Keel has three ways to work with the same assistant and model library:

- **Chat**: questions, explanations, brainstorming, and writing. No workspace tools, MCP tools, or external coding harness. Works without a folder.
- **Work**: tasks over a selected folder, including documents, CSV analysis, websites, slides, and study material. Inspect files, stage changes, review, then preview the result. No built-in shell tool.
- **Code**: repository exploration, changes, commands, tests, and diff review. The built-in harness requests approval for shell commands. Installed external CLI harnesses can be selected in Code and use their own execution/approval policy.

A thread keeps its mode through its persisted `app_preset`. Existing `general` threads open in Work; Websites, Docs, Sheets, Study, and Slides remain specialized Work threads. Switching modes resumes a recent matching thread or creates one. Creating a new thread in a mode uses that mode. Navigation is held during a text generation so its events cannot appear in another thread.

## Optional capabilities

The Tools library shows what works today and what is planned. Text models are shared by all three modes. Download only a text model you need, import an existing GGUF, or explicitly connect an existing OpenAI-compatible model server. Media tools do not require a local text model.

Images use an optional loopback AUTOMATIC1111 / Forge API server. Keel does not install that runtime or its model weights. A user who does not use Images downloads neither. An existing local server can be reused without duplicating its models in Keel.

The image adapter uses `GET /sdapi/v1/sd-models` for checkpoint discovery and `POST /sdapi/v1/txt2img` for one PNG. Endpoint configuration is explicit and restricted to HTTP loopback. Redirects are disabled. This adapter does not configure hosted inference or send prompts to a cloud fallback.

Image jobs have running, succeeded, failed, and cancelled states. Their request, chosen checkpoint, output folder, and artifact path are kept in Keel's data directory. An app restart marks interrupted jobs failed and never replays them. Only one request is active at a time; **Stop waiting** cancels Keel's request, not the server's global queue. The server may still finish its computation. The UI labels this explicitly.

Successful images are written as new `assets/keel-<job-id>.png` files in the selected workspace. Clicking Generate authorizes creation of this new artifact; existing workspace files are not overwritten. In Work, refer to that relative path when building a website or deck. Both the sandboxed preview and standalone **Export HTML** embed local PNG references as data URLs so exported pages stay self-contained.

## 0.5.0 upgrades

1. **Web search in Chat & Work:** `web_search` is an explicit tool (also available in Code). It requires the same user approval path as shell/MCP. Results leave the machine over the network; there is no silent cloud chat fallback.
2. **Vision in Chat:** image attachments keep their pixels (`image_data_url`) and Chat sends OpenAI-compatible multimodal `image_url` parts to the local or connected model. OCR remains an optional caption/fallback when Tesseract is installed or when a text-only model rejects images.
3. **Keep the model loaded:** the graphics-or-processor choice from Settings reaches the runner, and a later message reuses that process when the model, context, and that choice are unchanged.
4. **Check this model:** Settings can run three short questions (a saved preference, a number in a note, and a question the note does not answer). A model that fails is not offered as the one for this computer; it stays installed and can still be started by hand.

## 0.6.0 direction

1. **One coding engine:** Code mode can use the Keel CLI (streaming events, GUI approvals, shared JSONL journals). The built-in `keel-core` harness is the default. Select a compatible Keel CLI in Settings. Chat, Work, and Images stay Desktop-owned.
2. **Model packs & scheduling:** VRAM-aware detection, verified capability packs, and unload/scheduling on top of keep-alive. Curate Lightweight / Balanced / Powerful from measurements.
3. **Reliability evidence:** expand Check this model / `evaluate_local` into a held-out suite with published latency and success targets per recommended model (including GPU/Metal numbers).
4. **Hub expansion:** optional audio STT/TTS packs; explicit opt-in remote providers with secure credentials and location/cost labels — never a silent cloud fallback. Video waits until image/audio jobs are dependable.

## Next milestones

1. **Reliable everyday use:** PDF/DOCX/text attachments, search/rename/pin/export, reusable projects, saved preferences, a file Library, durable partial replies, guided runtime/model setup, web search, Chat vision, keep-alive, and model check are implemented. Next: broader real-model evaluations, native interaction tests, richer Code context via the Keel CLI, and better CPU/GPU latency.
2. **Model packs:** capability-specific runtime adapters, verified manifests, resumable/cancellable downloads, removal, measured hardware requirements, and resource scheduling/unloading. Curate Lightweight / Balanced / Powerful choices where real measurements support them; parameter counts alone are insufficient.
3. **Audio:** separate speech-to-text and text-to-speech adapters with optional packs and audio artifacts. Do not force music generation into a speech model's interface.
4. **Video:** asynchronous generation with engine-specific requirements, progress, cancellation semantics, and playable artifacts after image/audio jobs are dependable.
5. **Explicit remote providers:** opt-in endpoint/auth configuration, secure credential storage, clear processing-location/cost labels, and no silent cloud fallback. A model still has to run somewhere when a user skips a local download.

Images generation still needs a separately installed image server. System RAM detection is implemented for model sizing. Keel notices the graphics chip and keeps the chosen model loaded between messages, using the graphics-or-processor choice from Settings. GPU/VRAM detection, audio/video backends, automatic media runtime installation, and hosted-provider authentication are not implemented by this milestone.

## Verification

`./scripts/check.sh` checks formatting, workspace Clippy, Rust tests, and UI unit tests. Rust integration tests use local HTTP fixtures to verify the actual text-tool loop and image API adapter without downloading models or contacting live systems. They cover Chat tool rejection, Work staging/approval, image saving, malformed responses, cancellation, and restart recovery.

For repeatable frontend checks, install Playwright (`pip install playwright==1.56.0` and `python -m playwright install chromium`) and run `python scripts/ui-smoke.py`. It serves the real frontend with a test-only mock of Tauri IPC, checks the mode and image flows, and writes screenshots to a temporary folder (or `KEEL_SMOKE_OUTPUT`). This verifies frontend behavior, not native file dialogs, GPU inference quality, or a production image-server installation.
