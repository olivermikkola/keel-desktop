# Keel Desktop

**Chat, get work done, and build software with local AI on macOS, Linux, and Windows.**
Use a shared text model across Chat, Work, and Code. Download a GGUF or connect an existing model server. Add optional image generation when you need it; media models are never required for the assistant.

Built with **Tauri 2**, **Rust**, and **llama.cpp** (`llama-server`), carrying the **Keel** small-model harness and dark-mode workspace from Keel Mobile onto the desktop without iOS memory or sandbox restrictions.

## Start here

Open Keel and choose **Get started** or **Models → Guided setup**. Keel recommends a text model for your RAM, downloads the checksum-verified model runner and weights, and starts the assistant. No terminal command or separate llama.cpp installation is needed on supported macOS, Windows, and Linux builds. Models can take several gigabytes; setup shows the sizes before you choose it.

Already using Ollama or LM Studio? Choose **Connect existing server** in guided setup instead. No duplicate model download is required. You can also import GGUF files in Models.

## What is included

- **Document chat:** Attach PDFs, DOCX, text, Markdown, CSV, or code files. Inspect the extracted source and ask follow-up questions. Large documents use labeled excerpts; scanned PDFs need OCR before import.
- **Projects:** Reusable instructions, reference documents, an optional working folder, and conversations grouped around the same work.
- **Saved preferences:** Editable local context for future conversations. Add it in the sidebar or explicitly ask “Remember that …”; ordinary conversation is not automatically mined for memories.
- **Conversation tools:** Search message text and titles, rename, pin, export to Markdown, edit a prompt in a copy, retry a text answer, keep drafts, and undo a deletion. Interrupted replies keep partial answers and proposed edits.
- **Library:** Find approved documents, websites, code, and generated PNGs across your workspaces.
- **Readable replies:** Tables, numbered steps, quotes, web links, and code blocks with copy controls. Direct answers are the default; reasoning is optional.
- **Chat / Work / Code:** Chat answers without workspace tools. Work handles files, documents, and data with staged edits. Code adds repository tools, approved shell commands, and diff review. Existing threads keep their history and map into the appropriate mode.
- **Optional Images:** Connect a local AUTOMATIC1111 / Forge server, choose its checkpoint, generate PNGs into your workspace, and use them in website/slide previews. Job history, failure states, and stop-waiting controls are included. Audio and video are marked as planned.
- **Local models & `llama-server` lifecycle:** GGUF catalog (`Bonsai 2` 4B/9B/27B, `LFM3` 1.7B, `MiniCPM5` 2B), with pinned revisions and SHA-256 verification for the 4B/9B/LFM3/MiniCPM entries; the 27B entry currently follows `main` without a checksum. Includes custom `.gguf` import/URL download, or connect to any OpenAI-compatible local server (`http://127.0.0.1:8080/v1`, Ollama, LM Studio).
- **Small-model tuned Rust harness (`keel-core`):**
  - Streaming SSE responses with `<think>` reasoning separation and `<tool_call>` text fallback parsing
  - Repeat-tool guard, failure circuit breaker, and context-window compaction
  - Real workspace tools confined safely to the selected folder (`list_files`, `read_file`, `search_files`, `propose_edit`, `create_file`, `delete_file`, `rename_file`, `run_sql`, `run_shell`)
- **Staged Diff Review & Approvals:** Review proposed edits file-by-file before applying them (or toggle auto-apply for workspace edits), while shell commands and mutating MCP calls prompt for `Allow once`, `Always allow`, or `Deny`.
- **Built-in Apps & Live Viewers:**
  - **Websites** (live sandboxed HTML/CSS/JS preview)
  - **Sheets** (`run_sql` with CSV files auto-mounted as SQLite tables + table viewer)
  - **Docs** (Markdown writer + rendered viewer)
  - **Study** (interactive `Q:`/`A:` flashcards and multiple-choice quizzes)
  - **Slides** (HTML presentation preview with slide navigation & print/PDF export)
- **Local `stdio` MCP servers:** Configure standard MCP servers in `.keel/mcp.json` or `~/.config/keel-desktop/mcp.json`.
- **Harness-agnostic pass-through:** Use the built-in Keel Harness by default, or switch in **Code** to an external CLI harness (`opencode`, `keel`, etc.) when installed.

See [everyday assistant validation](docs/everyday-readiness.md) for tested flows and real-model limitations.

## Optional image setup

1. Start your local AUTOMATIC1111 or Forge server with its `--api` flag and PNG output. Its models stay in that server.
2. Open **Tools → Images**, enter the loopback URL (usually `http://127.0.0.1:7860`), and click **Connect**.
3. Choose a checkpoint and output folder, write a prompt, and generate. PNGs save to `assets/keel-<id>.png`.
4. Open **Work → Websites** or **Slides** using the same folder and ask Keel to use the generated asset path.

No image model or server is needed for Chat, Work, or Code. **Stop waiting** cancels Keel's request; the image server may still finish computing. Hosted inference is not configured by this adapter.

See [the AI hub architecture and roadmap](docs/ai-hub.md) for current behavior, optional packs, and the next audio/video milestones.

## Repository layout

```text
crates/keel-core   Local agent loop, tools, SQL sandbox, model catalog/runtime, stdio MCP, sessions
src-tauri          Tauri 2 desktop shell, IPC commands, event streaming, brand icons
ui                 Zero-dependency dark-mode frontend + built-in live viewers
scripts            Local check and verification scripts
```

## Install

### Download prebuilt desktop installers (macOS, Linux, Windows)

Grab the latest installer from [**GitHub Releases**](https://github.com/olivermikkola/keel-desktop/releases):

- **macOS:** `.dmg` (Apple Silicon `aarch64` or Intel `x86_64`) — drag **Keel Desktop** into Applications
- **Linux:** `.deb`, `.AppImage`, or `.rpm`
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

## License

MIT — see [LICENSE](LICENSE).
