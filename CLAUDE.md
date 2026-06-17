# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Check

```bash
cargo check              # Fast compile check
cargo clippy             # Lint
cargo build              # Full build
cargo run                # Build & run
codegraph sync           # Re-index codebase after structural changes (new files, new types, new fns)
```

**After any code change that adds, removes, or renames files, structs, enums, functions, or modules, you MUST run `codegraph sync`** to keep the `.codegraph/` index current. Future Claude sessions rely on this index for fast code exploration.

No tests exist yet.

## Architecture

Saad Agent is a CLI AI coding assistant powered by DeepSeek via the `rig` crate (Rust AI framework). It streams multi-turn agent responses to the terminal with Claude Code-style phase indicators.

```
main.rs            Entry point: tracing init, agent build, main REPL loop
config.rs          Constants, env-var getters (DEEPSEEK_API_KEY, SAAD_MAX_TURNS, etc.)
memory.rs          ConversationMemory — AI-summary compaction when messages exceed limit
permission.rs      4-level permission system (per-op/session/permanent/deny) backed by TOML config
stream_handler.rs  Drives the stream → StreamDisplay state machine for CLI rendering
command.rs         Slash-command handlers (/help, /clear, /save, /load, /history, /exit, /effort)
error.rs           Unified AgentError enum (Io + Other)
ui.rs              Terminal rendering: welcome card, spinner, StreamDisplay (phase headers, tool calls, token stats)
tool/cmd.rs        ExecuteCommand tool — executes system commands (PowerShell on Windows, sh on Unix)
tool/fs.rs         ReadFile, GetFileLines, WriteFile, EditFile tools — all with path-traversal protection
```

### Key data flow

```
User input → main loop → build_context() (summary + messages) → agent.stream_chat()
    → stream_handler::process_stream() → StreamDisplay (phases: Reasoning → ToolCall → Answer → Token stats)
    → memory.extend() (only Text content kept) → memory.compact() (AI summary if > max_messages)
```

### ConversationMemory

Wraps `Vec<Message>` + summary. On overflow calls a lightweight `CompletionModel` to summarize the oldest half. Summary injected as System message via `build_context()`. Persisted to `.saad-agent/history.json` as `{summary, messages}`.

### Permission system

Three entry points share one global `AtomicU8` level:
- `confirm_execution(cmd)` — before running shell commands
- `confirm_file_write(path)` — before WriteFile/EditFile
- `confirm_cross_directory(detail)` — path-escape detection (ReadFile/WriteFile/EditFile)

Levels: Prompt (dialoguer::Select with 4 options) → SessionAllowAll (memory) → PermanentAllowAll (TOML file).

### StreamDisplay phases

`StreamPhase` state machine: `Idle → Reasoning → ToolCall → Answer`. Phase headers use `console::measure_text_width()` for CJK-safe separator lines. Tool results render up to 10 indented lines. Token usage printed per-turn in Claude Code CLI style (`⏺ Turn 1 · 1.2k input · 0.3k output`).

### Path safety (fs tools)

`resolve_safe_path` canonicalizes the path and verifies it's within `current_dir()` subtree. If not, triggers `permission::confirm_cross_directory`. `resolve_safe_path_for_write` handles non-existent files by walking up to verify parent directories are safe.

## Configuration

All config lives in `config.rs` with env-var overrides:

| Env | Default | Notes |
|-----|---------|-------|
| `DEEPSEEK_API_KEY` | — (required) | |
| `DEEPSEEK_MODEL` | `deepseek-v4-flash` | |
| `SAAD_MAX_TURNS` | `100` | Agent turn limit |
| `SAAD_MAX_TOKENS` | `384000` | |
| `SAAD_MAX_HISTORY` | `20` | Messages before compaction |
| `SAAD_EFFORT` | `normal` | `concise` \| `normal` \| `elaborate` |

Effort level can also be changed at runtime via `/effort concise|normal|elaborate`.

## Notes

- Windows: PowerShell 5.1 is detected at startup; if PS < 7, the preamble warns AI not to use `&&`/`||` chaining.
- GBK encoding: `cmd.rs` has `encoding_rs::GBK` fallback when UTF-8 decode fails on Windows.
- The `.saad-agent/` directory stores `permission.toml`, `history.json` — relative to CWD.
