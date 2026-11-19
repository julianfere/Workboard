# Workboard

A desktop workspace for the work you do with AI coding agents. Each piece of work is a folder with a
`README.md`; its status lives in the README frontmatter and its tasks in a checklist section. Workboard
shows all of them, keeps them up to date as files change, and runs agent sessions (Claude Code, GitHub
Copilot CLI or any CLI with MCP) that read and update that state through an MCP server.

> **Status: early port.** This is a rewrite in Rust + Tauri of a Python app used daily. The parity
> checklist (`docs/`) and the reference behavior (`tests/golden/`) drive the port. Not usable yet.

## Principles

- **Files are the source of truth.** Workboard only writes the frontmatter and the task lines of each
  README; everything else (settings, caches, backups) lives in its own config folder.
- **Read-only by default.** Git is read-only; promotions only generate the commands for you to run.
  Task trackers are read-only unless you explicitly push a change.
- **Configurable workflow.** States, types, environments, periods and folder conventions come from a
  preset (`crates/wb-core/presets/`), not from code.
- **Offline UI**, no CDNs.

## Development

Requirements: Rust (stable), Node 22, and on Linux the WebKitGTK dev packages.

| What | Command |
|---|---|
| Run the app | `npm install` then `npm run tauri dev` |
| UI only, in a browser (mocked backend) | `npm run dev` → <http://localhost:1420> |
| Tests | `cargo test --workspace` |
| Lint | `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings` |
| Regenerate golden files (needs the v1) | `py tests/golden/gen/gen_all.py` |

Set `WORKBOARD_CONFIG_DIR` to a temporary folder when testing, so you don't touch your real settings.

## Layout

```
crates/wb-core      workflow presets, work folders, frontmatter, tasks, safe README writes
crates/wb-mcp       the `workboard` MCP server agents use
crates/wb-i18n      translations, shared with the UI (locales/*.json)
crates/wb-platform  config folder, opening files, process trees
src-tauri/          the desktop app and its commands
ui/                 the UI (TypeScript + Vite)
tests/golden/       reference behavior captured from the v1
spike/              throwaway experiments (terminal, MCP, hook)
docs/               parity checklist
```

## License

MIT
