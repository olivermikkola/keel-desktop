# Keel Desktop: rules for contributors and agents

Keel Desktop is a cross-platform local AI agent workspace built with **Tauri 2** and **Rust**.

## Core principles

- **Local first.** Models run locally (`llama-server` or any OpenAI-compatible loopback endpoint). Never silently fall back to a cloud model.
- **You approve, the model asks.** File changes can be staged for diff review (`propose_edit` / `create_file` / `delete_file` / `rename_file`), and shell commands or mutating MCP tools require explicit user approval unless allowed for the session.
- **Clean public repository.** Never commit secrets, tokens, private host paths, personal config files, or model weights (`.gguf`).
- **Small, focused modules.** Keep `crates/keel-core` modular and well-tested; avoid duplicating logic between `keel-core`, `src-tauri`, and `ui/`.

## Before opening a PR

1. Keep one topic per PR, branched off `main`.
2. Run `./scripts/check.sh` locally (formatting, clippy `-D warnings`, Rust tests, and UI tests) and ensure everything passes.
3. Include real-run command output in the PR `## Proof` section.
4. Keep the repo clean and verify CI passes on Linux, macOS, and Windows before merging.
