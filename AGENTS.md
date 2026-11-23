# Keel Desktop: rules for contributors and agents

Keel Desktop is a cross-platform local AI agent workspace built with **Tauri 2** and **Rust**.

## Core principles

- **Local first.** Models run locally (`llama-server` or any OpenAI-compatible loopback endpoint). Never silently fall back to a cloud model.
- **One coding engine.** Code mode's canonical engine is the [Keel CLI](https://github.com/olivermikkola/keel) ([canonical vision](https://github.com/olivermikkola/keel/blob/main/docs/VISION.md)); the built-in `keel-core` harness is today's default. New Code-mode work should move toward driving the Keel CLI (streaming events, approvals, resumable sessions), not grow a parallel coding agent. Chat, Work, and Images remain Desktop's own surface.
- **You approve, the model asks.** File changes can be staged for diff review (`propose_edit` / `create_file` / `delete_file` / `rename_file`), and shell commands or mutating MCP tools require explicit user approval unless allowed for the session.
- **Clean public repository.** Never commit secrets, tokens, private host paths, personal config files, or model weights (`.gguf`).
- **Small, focused modules.** Keep `crates/keel-core` modular and well-tested; avoid duplicating logic between `keel-core`, `src-tauri`, and `ui/`.
- **`KEEL_HOME` (optional).** When set, Keel CLI journal paths use `$KEEL_HOME/.keel/sessions` instead of `dirs::home_dir()` (useful for tests and portable layouts). On Windows, `dirs` uses Known Folder APIs and ignores `USERPROFILE`/`HOME` overrides.

## Before opening a PR

1. Keep one topic per PR, branched off `main`.
2. Run `./scripts/check.sh` locally (formatting, clippy `-D warnings`, Rust tests, and UI tests) and ensure everything passes.
3. Include real-run command output in the PR `## Proof` section.
4. Keep the repo clean and verify CI passes on Linux, macOS, and Windows before merging.
