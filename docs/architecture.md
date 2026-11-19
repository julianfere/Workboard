# Architecture

Workboard is a Rust workspace plus a TypeScript UI in a Tauri 2 window. All logic is in Rust crates that
know nothing about the window; the window is a thin shell that forwards commands.

```
ui/ (TypeScript)  ──invoke("api", {cmd, args})──▶  src-tauri  ──▶  wb-api::Api::dispatch(cmd, args)
                  ◀── {ok, data} / {ok:false, error:{code, params}} ──┘          │
                                                                                 ├─ wb-core     work folder, README writes
                                                                                 ├─ wb-git      read-only git
                                                                                 ├─ wb-tracker  Azure DevOps
                                                                                 ├─ wb-sessions agent sessions (pty, hooks)
                                                                                 └─ wb-plugins  plugin processes

agent CLI ──stdio──▶ wb-mcp (workboard MCP server) ──▶ wb-core      (same files, read on every call)
agent CLI ──hook───▶ wb-hook ──HTTP 127.0.0.1 + token──▶ wb-sessions
```

## Crates

| Crate | Responsibility |
|---|---|
| `wb-core` | Everything about the work folder, without UI. `workflow` (presets in `presets/*.json`), `frontmatter` (own parser/writer for the `k: v` / `k: [a, b]` subset; reads UTF-8, BOM and cp1252, writes UTF-8 + LF), `tasks` (the task section), `validate` (editable fields), `text` (slug, sections, `plan_gaps`), `workspace` (scan, safe paths: nothing outside the root), `write` (**`write_readme` is the only way a README is written**: mtime check with retry, backup of the last 20 versions, atomic write, `state_log.jsonl`; plus move, delete to `_deleted`, rename, versions), `records` (`runs.json`, inbox), `notes`. `py.rs` replicates the Python text semantics the file format depends on. Errors are `CoreError { kind, code, params }`. |
| `wb-mcp` | The `workboard` MCP server (stdio, `rmcp`). `lib.rs` holds the logic of each tool, testable without JSON-RPC; `main.rs` the transport (behind the `server` feature); `context.rs` the guide and session context; `tools.rs` promoted tools frozen by SHA-256. |
| `wb-sessions` | Agent sessions: `registry` (`<config>/sessions/<root>.json`), `agents` (command lines of Claude Code, Copilot CLI and the custom command), `pty` (portable-pty / ConPTY, output batched every 8 ms, 512 KB buffer to reattach), `transcript` (session weight and recommendation), `SessionManager` (lifecycle, hook receiver, context and settings files, `activity.jsonl`, plan check), `timelog` (hours), `stats` (metrics), `candidates` (scripts to promote), `reflect` (evidence). |
| `wb-git` | The read-only Git panel. `cmd::git` refuses any subcommand outside `READ_ONLY` and always passes `--no-optional-locks`. Branch state per environment on an in-memory graph, batched `diff-tree --stdin` and patch-ids cached by SHA, repos in parallel. `promotion` returns the commits missing in a target environment. |
| `wb-tracker` | External trackers behind the `Tracker` trait; HTTP behind `Transport`. Azure DevOps in `azure.rs`, `html_to_text` in `html.rs`. |
| `wb-plugins` | Plugin manifests, enabled list, presets they contribute, `resolve_workflow` (built-in or plugin preset; used by the app, sessions and MCP), the JSON-RPC runner (`rpc::call`) and `sdk::serve` for plugins written in Rust. Protocol in [plugins.md](plugins.md). |
| `wb-hook` | The CLI hook binary. No dependencies (it runs on every tool call, so startup time matters). |
| `wb-i18n` | `t(lang, key, params)` over `locales/*.json`, the same files the UI loads. |
| `wb-platform` | Everything OS-specific: `config_dir`, opening files and https URLs, revealing in the file manager, `kill_tree`, `which`, running scripts with a timeout, the downloads folder, hiding console windows, and `secrets` (system credential store via `keyring`). There is no `cfg(windows)` outside this crate. |
| `wb-api` | The app API. `Api::dispatch(cmd, args)` is one `match` with every command, each a thin wrapper over the crates above; `settings.rs` (`settings.json`); feature modules `sessions.rs`, `git.rs`, `tracker.rs`, `stats.rs`, `plugins.rs`, chained in the fallback of `dispatch`. Errors are `ApiError { code, params, conflict }`. Binary `wb-devserver`. |

Outside the workspace: `src-tauri/` (the window), `ui/` (the UI), `tests/golden/` (reference behavior),
`spike/` (throwaway experiments, not built).

## Data flow

1. The UI calls `api(cmd, args)` from `ui/src/api.ts`, the only door to the backend. In Tauri it is
   `invoke("api", …)`; in a browser it is a `fetch` to the dev server. Every command works in both.
2. `src-tauri` has a single async command `api` that runs `Api::dispatch` off the UI thread (a slow git
   overview never freezes the window), plus `session_attach` (terminal output over a byte `Channel`) and the
   dialog plugin to pick a folder. Closing the window kills the terminals.
3. `Api::dispatch` validates arguments, calls the crate and returns JSON, or an `ApiError` with a code and
   parameters. **The backend never builds UI text**: the UI translates codes with the locale files.
4. Nothing in the work folder is cached: each call scans what it needs, so external edits (your editor, an
   agent, git) are always seen. The UI polls a cheap signature of the tree every 2 s (and of the repos'
   `.git` files every 3 s) and reloads when it changes.

A new command is added in `wb-api` (and its wrapper in `ui/src/api.ts`); `src-tauri` does not change.

## Configuration folder

`<config>` is `WORKBOARD_CONFIG_DIR` or the OS default (`%APPDATA%\Workboard2`,
`~/Library/Application Support/Workboard2`, `~/.config/Workboard2`).

| Path | Content |
|---|---|
| `settings.json` | Language, theme, workflow preset, open folder and recents, notes folder, agents, git repos per folder. Read by the MCP server too. |
| `backups/` | Previous versions of each README (last 20), used by **Versions** and the metrics. |
| `state_log.jsonl` | Every status change. |
| `runs.json` | Scripts marked as ran per environment. |
| `inbox.md` | Tasks without an item. |
| `notes/` | Personal notes (default; can be any folder). |
| `sessions/<root>.json` | Session registry per work folder. |
| `sessions/<id>.activity.jsonl`, `sessions/<id>.events.jsonl` | Hook activity and MCP writes per session. |
| `mcp_events.jsonl` | MCP writes outside a session. |
| `ctx/<id>/AGENTS.md`, `agents/mcp-<id>.json`, `claude/settings-<id>.json` | Per-session context, MCP config and Claude Code settings. |
| `reflect/`, `review/` | Evidence and review packets. |
| `tools/<name>/` | Promoted tools with `tool.json`. |
| `time_cache.json`, `stats_cache.json` | Caches of the Hours and Metrics views. |
| `git_cache.json` | Git results cached by SHA. |
| `trackers.json`, `tracker_cache.json` | Tracker connections, mapping and fields; last read per folder. |
| `plugins/`, `plugins.json`, `plugin-data/`, `plugins.log` | Installed plugins, enabled list, their data and stderr. |

Tokens are never in this folder: they are in the system credential store.

## Dev server

`wb-devserver` serves the same API over HTTP so the UI can be developed in a browser (devtools, Playwright)
against the real backend:

```sh
WORKBOARD_CONFIG_DIR=/tmp/wb-dev cargo run -p wb-api --bin wb-devserver -- examples/demo-work [--port 8766]
npm run dev          # Vite on http://localhost:1420 forwards /api to the dev server
```

`POST /api/<cmd>` with the arguments as a JSON object and the header `X-Workboard: 1` (it forces a CORS
preflight, so no web page can post to it). Answers `{"ok": true, "data": …}` or
`{"ok": false, "error": {code, params}}`. `GET` serves `ui/dist` when the UI is built.
`--fake-tracker <file.json>` answers tracker calls from a file and keeps secrets in memory.

```sh
curl -X POST -H "X-Workboard: 1" -d "{}" http://127.0.0.1:8766/api/list_items
```

There is no file watcher in the dev server beyond the UI's own polling.

## Testing

`cargo test --workspace` runs everything. CI runs `cargo fmt --check`, `clippy -D warnings` and the tests on
Windows, macOS and Linux, and type-checks and builds the UI (`npm run build`).

- **Golden files.** Workboard is a port of a Python app. Its behavior was captured as data in
  `tests/golden/cases/*.json` (input → expected output) over a fictional work folder
  (`tests/golden/fixtures/work`, with CRLF, BOM and cp1252 on purpose) and a synthetic git repo in
  `git fast-import` format (deterministic SHAs). The Rust tests read those cases and compare, using the
  `classic-es` preset. Intentional differences are documented in `tests/golden/KNOWN_ISSUES.md`.
  Regenerating needs the original app (`py tests/golden/gen/gen_all.py`).
- **Fake transports.** Tracker tests (`crates/wb-tracker/tests`, `crates/wb-api/tests/tracker.rs`) plug a
  `Transport` that answers fixed JSON and an in-memory `SecretStore`: no network, no real credentials.
- **The self-executable plugin.** `crates/wb-plugins/tests/rpc.rs` and `crates/wb-api/tests/plugins.rs` copy
  the test binary itself into a plugin folder. When the runner launches it, it sees
  `WORKBOARD_PLUGIN_PROTOCOL` and acts as the plugin (`sdk::serve`), with the behavior chosen by a
  `mode.txt` next to it (normal, silent to test timeouts, garbage output…). No extra binaries to build.
  Session tests (`crates/wb-sessions/tests/manager.rs`) run a real pty with a custom agent that just echoes
  (`cmd /c echo` or `sh -c echo`), so no agent CLI is needed.
- `WORKBOARD_CONFIG_DIR`, `WORKBOARD_SECRET_SERVICE` and `WORKBOARD_CLAUDE_PROJECTS` point config,
  credentials and transcripts at temporary locations.

## Conventions

- **No workflow in code.** States, types, environments, period, folder names, frontmatter keys, section
  titles and file categories come from the `Workflow`. A new piece of workflow data goes into the preset
  format (and both built-in presets). Code reasons about state *categories*, never state ids.
- **All UI text in locales.** Every visible string is in `locales/en.json` and `locales/es.json`; a test
  requires both to have the same keys and parameters. The backend returns codes, not sentences.
- **Colors as CSS variables** in `ui/src/styles.css` (light and dark). Icons with `ico()` from
  `ui/src/icons.ts`; no emojis or glyphs as icons.
- **Offline UI**: no CDNs or remote resources.
- **Read-only git**: only the subcommands in `wb_git::cmd::READ_ONLY`. Promotion generates commands
  (cherry-pick or merge, for PowerShell or Bash, plus a pull request URL); the user runs them.
- **The work folder is the user's.** Workboard writes only README frontmatter and task lines there, and
  creates, moves or renames folders when asked. Deleting moves to `_deleted`. Everything else goes to the
  configuration folder.
- **Secrets** only in the credential store, never in files, logs or errors.
- UI code: no framework; one file per view in `ui/src/views/`; own dialogs instead of the browser's;
  workflow-aware helpers (`metaText(item, wf().fields.status)`) instead of hard-coded frontmatter keys.
