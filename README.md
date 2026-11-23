# Keel Desktop

[![CI](https://github.com/olivermikkola/keel-desktop/actions/workflows/ci.yml/badge.svg)](https://github.com/olivermikkola/keel-desktop/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/olivermikkola/keel-desktop)](https://github.com/olivermikkola/keel-desktop/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

**Your everyday AI assistant — running on your own computer.**
Chat, work through documents and data, build websites and slides, generate
images, and write code. The model runs locally on your machine; nothing has
to leave it. No account, no subscription, no cloud fallback.

Keel Desktop is the everyday app of the Keel family. Chat and Work use Desktop's
local model runtime. Code can use that runtime or the separate
Keel CLI on supported setups. [Keel product vision](https://github.com/olivermikkola/keel/blob/main/docs/VISION.md)

Built with **Tauri 2**, **Rust**, and **llama.cpp** (`llama-server`), carrying the **Keel** small-model harness and workspace (Ink, Slate, and warm Paper light themes) from Keel Mobile onto the desktop without iOS memory or sandbox restrictions.

## Get started

1. Download the installer for your platform. For unreleased changes after the latest tag, build from `main` using the instructions below. [MoE Systems downloads](https://moesystems.com/keel-desktop/) · [GitHub releases](https://github.com/olivermikkola/keel-desktop/releases)
2. Open Keel and choose **Set up my AI**. Guided setup recommends a model for your RAM and installs the runner and model. You can pause setup and resume it later, connect Ollama, LM Studio, or another OpenAI-compatible server, or import your own `.gguf` in **Models**.
3. Ask a question in **Chat**, open a folder in **Work** (or click **Try with sample files** for a ready-to-use folder), or choose a Git repository in **Code**.

![Keel Desktop Work mode showing sample prompts and a budget document preview](https://moesystems.com/assets/keel-desktop-work.webp)

*Work mode with sample documents.*

## What you can do

### Everyday assistant — Chat & Work

- **Document chat:** Drag and drop files, paste images from the clipboard, or attach text-based PDFs, DOCX, text, Markdown, CSV, and code. Inspect the extracted source and ask follow-up questions. Large documents use labeled excerpts. Run OCR on scanned PDFs before importing them. In Chat, attached images are sent as pixels so a vision-capable model can describe what it sees.
- **Web search:** Chat and Work can look up current public facts with `web_search` after you approve the request. Search leaves your machine over the network; answers still come from your local (or connected) model.
- **Offline image text:** Optional local Tesseract OCR supplies a text caption for images and workspace files when pixels are not enough. You can install Tesseract with `sudo apt install tesseract-ocr`, `sudo dnf install tesseract`, or `brew install tesseract`.
- **Projects:** Reusable instructions, reference documents, an optional working folder, and conversations grouped around the same work.
- **Saved preferences:** Editable local context for future conversations. Add it in the sidebar or explicitly ask “Remember that …”; ordinary conversation is not automatically mined for memories.
- **Conversation tools:** Search message text and titles, rename, pin, export to Markdown, edit a prompt in a copy, retry a text answer, keep drafts, undo a deletion, and navigate with keyboard shortcuts (`Ctrl/Cmd+N/K/B/,/1/2/3`, `Esc` to stop, `?` shortcut reference in Settings). Interrupted replies keep partial answers and proposed edits.
- **Library:** Find approved documents, websites, code, and generated PNGs across your workspaces.
- **Readable replies & themes:** Smooth incremental streaming with live thinking timer, tables, numbered steps, quotes, task lists, web links, and code blocks with copy controls. Direct answers are the default; reasoning is optional and persisted. Choose **Ink**, **Slate**, or warm **Paper** light theme in Settings.
- **Three modes, one model:** Chat answers without workspace tools. Work handles files, documents, and data with staged edits. Code adds repository tools, approved shell commands, and diff review. Existing threads keep their history and map into the appropriate mode.

### Built-in apps & live viewers

- **Websites** — live sandboxed HTML/CSS/JS preview with Print/PDF and standalone **Export HTML**
- **Sheets** — `run_sql` with CSV files auto-mounted as typed SQLite tables (`INTEGER`/`REAL`/`TEXT`) plus a read-only `files` table, row/column counts, and quick table filtering
- **Docs** — Markdown writer + rendered viewer
- **Study** — interactive `Q:`/`A:` flashcards and multiple-choice quizzes with live score tracking and reset
- **Slides** — HTML presentation preview with slide navigation, Print/PDF export, and standalone **Export HTML**

### Build software — Code

- **Small-model tuned Rust harness (`keel-core`):** streaming SSE responses with reasoning separation and text-format tool-call fallback parsing; repeat-tool guard, active failure circuit breaker, and tool-schema-aware context-window compaction; workspace tools confined safely to the selected folder (`list_files`, `read_file`, `search_files`, `propose_edit`, `create_file`, `delete_file`, `rename_file`, `run_sql`, `run_shell`).
- **Staged diff review & approvals:** Review proposed edits file-by-file in the interactive Diff viewer (approve or reject individually or all at once, or toggle auto-apply for workspace edits), while shell commands and mutating MCP calls prompt for `Allow once`, `Allow for this chat`, or `Deny`.
- **Local `stdio` MCP servers:** Configure standard MCP servers globally in Keel's OS config directory (`mcp.json`) or per-workspace in `.keel/mcp.json`. Workspace MCP servers are gated behind an explicit folder-trust toggle in **Connectors** so untrusted repos cannot auto-spawn commands.
- **Isolated checkout** runs edits and tests in a separate Git worktree based on the selected repository's HEAD. In Code, choose **Start isolated checkout**, run a test, review the patch, and choose **Apply patch to repo** when it is ready. You can undo the applied patch from the same panel.
- **Guided GitHub publishing** can commit reviewed changes on a feature branch, push the branch, and open a pull request. A failing recorded test blocks publishing. Opening a PR requires a GitHub remote and an authenticated GitHub CLI (`gh`).
- **Keel CLI in Code** streams tool results, requests approvals, and keeps resumable CLI journals. Install a current `keel` on PATH and select **Keel CLI** under Settings → Developer options. Finish the current turn before resuming a journal in the terminal. On macOS, Code shows the available command boundary and disables agent commands when it cannot confine them. New installs use the built-in engine by default.

### Models & images

- **Local models & `llama-server` lifecycle:** GGUF catalog (`Bonsai 2` 4B/9B/27B, `LFM3` 1.7B, `MiniCPM5` 2B) with RAM fit indicators, storage usage bar, configurable context window, and pinned revisions + SHA-256 verification for the 4B/9B/LFM3/MiniCPM entries; the 27B entry currently follows `main` without a checksum. Includes custom `.gguf` import/URL download, or connect to any OpenAI-compatible local server (`http://127.0.0.1:8080/v1`, Ollama, LM Studio). Guided setup notices the computer (RAM and graphics chip) before recommending a model. Settings can choose Automatic, Graphics chip, or Processor; that choice reaches the runner, and a later message reuses the process when the model, context, and choice are unchanged. **Check this model** runs three short questions; a model that fails is not offered as the one for this computer (it stays installed and can still be started by hand). The status line shows how long the last answer took.
- **Optional Images:** Connect a local AUTOMATIC1111 / Forge server, choose its checkpoint, generate PNGs into your workspace, and use them in website/slide previews. Job history, failure states, and stop-waiting controls are included. Audio STT/TTS packs ship in Settings (try-run + artifacts); video remains planned.

See [everyday assistant validation](docs/everyday-readiness.md) for tested flows and real-model limitations.

## Optional image setup

1. Start your local AUTOMATIC1111 or Forge server with its `--api` flag and PNG output. Its models stay on that server.
2. Open **Tools → Images**, enter the loopback URL (usually `http://127.0.0.1:7860`), and click **Connect**.
3. Choose a checkpoint and output folder, write a prompt, and generate. PNGs save to `assets/keel-<id>.png`.
4. Open **Work → Websites** or **Slides** using the same folder and ask Keel to use the generated asset path.

No image model or server is needed for Chat, Work, or Code. **Stop waiting** cancels Keel's request; the image server may still finish computing. Hosted inference is not configured for this adapter.

See [the AI hub architecture and roadmap](docs/ai-hub.md) for current behavior, optional packs, and the next audio/video milestones.

## Install

### Download prebuilt desktop installers (macOS, Linux, Windows)

Grab the latest installer from [**GitHub Releases**](https://github.com/olivermikkola/keel-desktop/releases):

- **macOS:** `.dmg` (Apple Silicon `aarch64` or Intel `x86_64`) — drag **Keel Desktop** into Applications
- **Fedora / RHEL:** `.rpm` — `sudo dnf install ./Keel.Desktop-*.x86_64.rpm` (pulls in everything Keel's model engine needs, including `openssl3-libs` on Fedora 45+)
- **Ubuntu / Debian:** `.deb` — `sudo apt install ./Keel.Desktop_*_amd64.deb`
- **Other Linux:** `.AppImage` (needs FUSE 2: `fuse-libs` on Fedora, `libfuse2` on Ubuntu). If the model engine is missing a system library, Keel tells you which one and the command that installs it.
- **Windows:** `.exe` (NSIS setup) or `.msi`

### Install locally on Linux (from source)

Creates `~/.local/bin/keel-desktop`, `~/.local/bin/keel-desktop-cli`, and a desktop launcher entry so **Keel Desktop** appears in your application menu:

```sh
./scripts/install-linux.sh
```

## Develop & test

### Prerequisites

- Rust (stable, tested with `1.100`, plus `clippy` and `rustfmt`)
- Node.js (`20+`, for `ui/tests/*.test.mjs`)
- On Linux: `libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev`
- Local inference: guided setup installs a pinned `llama-server`; an existing executable on `PATH` or a custom path in Settings can also be used.

### Run checks

```sh
./scripts/check.sh
```

### Run the desktop app in dev mode

```sh
cargo run -p keel-desktop
```

### Headless CLI smoke test (no GUI window)

```sh
cargo run -p keel-core --bin keel-desktop-cli -- --help
```

`keel-desktop-cli` exposes model information and one-shot prompts through
Desktop's local engine. For a terminal coding session, use the Keel CLI.
[Keel CLI](https://github.com/olivermikkola/keel)

## Repository layout

```text
crates/keel-core   Local agent loop, tools, SQL sandbox, model catalog/runtime, stdio MCP, sessions
src-tauri          Tauri 2 desktop shell, IPC commands, event streaming, brand icons
ui                 Zero-dependency dark-mode frontend + built-in live viewers
scripts            Local check and verification scripts
```

## The Keel family

One engine, focused surfaces — the canonical positioning is the
[Keel product vision](https://github.com/olivermikkola/keel/blob/main/docs/VISION.md).

| Product | What it is |
| --- | --- |
| [Keel](https://github.com/olivermikkola/keel) | CLI + TUI coding agent for small local models — Code mode's canonical engine |
| **Keel Desktop** (this repo) | Everyday local-AI app: Chat, Work, Images, Code |
| [Keel Mobile](https://moesystems.com/keel-mobile/) | Standalone on-device iPhone agent — the model runs on the phone |
| [Keel Mobile for Android](https://github.com/olivermikkola/keel-mobile-android) | On-device Android agent, source-build pre-alpha |
| [Keel Console](https://github.com/olivermikkola/keel-xbox) | Living-room app for Xbox / TV |
| [Keel Arena](https://github.com/olivermikkola/keel-arena) | Public benchmarks and scorecards — the proof layer |

## License

MIT — see [LICENSE](LICENSE).
