# Keel Desktop

**A local AI agent workspace for macOS, Linux, and Windows.**  
Download a GGUF model, point it at a real folder on your computer, and let the agent read, write, query, run commands, and preview what it builds—with zero subscriptions and explicit approvals.

Built with **Tauri 2**, **Rust**, and **llama.cpp** (`llama-server`), carrying the **Keel** small-model harness and dark-mode workspace from Keel Mobile onto the desktop without iOS memory or sandbox restrictions.

## What is included

- **One-click local models & `llama-server` lifecycle:** Pinned GGUF catalog (`Bonsai 2` 4B/9B/27B, `LFM3` 1.7B, `MiniCPM5` 2B, `Qwen3.8` 27B) with SHA-256 verification, custom `.gguf` import/URL download, or connect to any OpenAI-compatible local server (`http://127.0.0.1:8080/v1`, Ollama, LM Studio).
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
- **Harness-agnostic pass-through:** Use the built-in Keel Harness by default, or switch to an external CLI harness (`opencode`, `keel`, etc.) when installed.

## Repository layout

```text
crates/keel-core   Local agent loop, tools, SQL sandbox, model catalog/runtime, stdio MCP, sessions
src-tauri          Tauri 2 desktop shell, IPC commands, event streaming, brand icons
ui                 Zero-dependency dark-mode frontend + built-in live viewers
scripts            Local check and verification scripts
```

## Develop & test

### Prerequisites

- Rust (`1.85+`, `clippy`, `rustfmt`)
- Node.js (`20+`, for `ui/tests/*.test.mjs`)
- On Linux: `libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev`
- Optional for local inference: `llama-server` on `PATH` (or configure its path in **Models**)

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
