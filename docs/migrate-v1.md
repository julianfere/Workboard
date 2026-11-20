# Migrating from the v1

`workboard migrate-v1` copies the settings of the Python app (the "v1", `%APPDATA%\Workboard`) to the
configuration folder of this version (`%APPDATA%\Workboard2`, or `WORKBOARD_CONFIG_DIR`). The work folders
themselves need nothing: both versions read the same `README.md` files.

```
workboard migrate-v1 [--from <v1 settings folder>] [--to <settings folder>] [--preset <id>] [--dry-run] [--force]
```

| Option | Default | |
|---|---|---|
| `--from` | `%APPDATA%\Workboard` | The v1 configuration folder. `WORKBOARD_CONFIG_DIR` is **not** used for it (that one is this version's). |
| `--to` | the configuration folder (`WORKBOARD_CONFIG_DIR` or `%APPDATA%\Workboard2`) | Where to migrate. |
| `--preset` | `classic-es` | Workflow preset. The v1 flow is `classic-es`; a preset from a plugin (e.g. `my-flow`) works if the plugin is installed and enabled in the destination first. |
| `--dry-run` | | Shows the plan and writes nothing (no files, no credential store). |
| `--force` | | Replaces values the destination already has. |

Rules:

- **The v1 folder is only read.** Nothing there changes; the v1 keeps working.
- **Idempotent.** Running it again changes nothing. A value the destination already has with the same
  content is reported as *already there*; with another content it is *kept* (and replaced only with
  `--force`). Logs and folders (run marks, status history, session activity, backups, notes) are merged:
  only what is missing is added.
- **Tokens never show up.** The PATs are decrypted with DPAPI (same Windows user that saved them) and
  go straight to the credential store; the report only names the organization.
- Output: one line per thing (*migrated*, *would migrate*, *already there*, *kept*, *failed*), the list of
  what is not migrated and a summary. Exit code 1 if something failed, 2 for bad arguments.
- Only Windows: on other systems the command says there is no v1 to migrate.

In a release build the app is a GUI program, so Windows does not attach the console it was started from.
Pipe the output to see it and to wait for it: `workboard.exe migrate-v1 --dry-run | more` in `cmd`, or
`& .\workboard.exe migrate-v1 --dry-run | Out-Host` in PowerShell. Development builds print normally.

## What is migrated

v1 = `%APPDATA%\Workboard`, v2 = the destination. Keys are those of the v1 `config.json` unless noted.

| v1 | v2 | How |
|---|---|---|
| `root` | `settings.json` → `root` | Canonical path. |
| `recents` | `settings.json` → `recents` | Added after the existing ones (no duplicates, case-insensitive, up to 12); the open folder goes first. |
| `aliases` (key: lower-case path) | `settings.json` → `aliases[<path>]` | Folder display names. The key goes to the path as written in the recents; an existing different name is kept unless `--force`. |
| — (the v1 was Spanish only) | `settings.json` → `lang` = `es` | Only if not set. |
| `prefs.theme` (`light`/`dark`) | `settings.json` → `theme` | `system` is the v2 default, nothing to do. |
| — | `settings.json` → `workflow` = `--preset` | The v2 default (`default`) counts as not chosen. The preset must resolve (included or from an enabled plugin). |
| `notesFolder` | `settings.json` → `notesFolder` | The folder stays where it is (it can be an Obsidian vault). |
| `notas/` (when there was no `notesFolder`) | `notes/` (or the v2 `notesFolder`) | Files copied, recursive, without overwriting. Same note format. |
| `agents.default`, `agents.custom` | `settings.json` → `agents.default`, `agents.custom` | Same keys (`label`, `command`, `args`, `resumeArgs`) and placeholders. |
| `git[<root>]` = `[{name, path, envs}]` | `settings.json` → `git.repos[<root>]` | The root key goes from lower case to the path as written; environment keys (`DEV`) to the workflow ids (`dev`). |
| `ado.conn[<root>]` = `{org, project}` | `trackers.json` → `connections[<root key>]` = `{kind: "azure-devops", org, project}` | Org URL normalized as in the v2. |
| `ado.procs["org\|project"]` | `trackers.json` → `projects["azure-devops\|org\|project"]` | `states` (plus `suggested` from the Azure category), `fetchedAt`, `map`, `mapSaved`, `dateFields`, `numFields`, `hasPriority`, `hasEffort`; `pactadaField` → `dueField`, `horasField` → `hoursField` (`""` = "none" is kept). Mapped states the preset does not have are left out and reported. |
| `ado.pats[org]` (DPAPI, base64) | credential store, `azure-devops:read:<org>` | Decrypted with DPAPI (current user). |
| `ado.wpats[org]` (DPAPI, base64) | credential store, `azure-devops:write:<org>` | Same. |
| `runs.json` | `runs.json` | Same format; root keys recomputed; marks merged per file and environment. |
| `state_log.jsonl` | `state_log.jsonl` | Same format; rows merged without duplicates, sorted by `ts`. |
| `mcp_events.jsonl` | `mcp_events.jsonl` | Same. |
| `tools/<name>/` (`tool.json` + script) | `tools/<name>/` | Copied as is (same `tool.json` keys and sha256). A script that no longer matches its hash is copied and reported: it does not run until promoted again, as in the v1. |
| `inbox.md` | `inbox.md` | Same format. |
| `backups/` | `backups/` | Same layout (`<root>-<sha1[:12]>/<item>/…`); files added without overwriting. Keeps **Versions** and the status history of Metrics. |
| `sessions/<root>.json` | `sessions/<root>.json` | Same file name (same root key). Each session goes from the v1 keys (`nombre`, `raiz`, `wis`, `estado`, `modo`, `agente`, `modelo`, `creada`, `ultimaActividad`, `forkDe`) to the v2 ones; `cerrada` → `closed`, anything else → `paused` (resume them with **Resume**: same id, same Claude Code transcript). Merged by id. |
| `sessions/<id>.activity.jsonl`, `<id>.events.jsonl` | same names | Merged without duplicates. The v2 reads the v1 rows (`wis`), so Hours and Metrics keep the history. |

## What is not migrated

| v1 | Why |
|---|---|
| `ado_cache.json`, `git_cache.json`, `time_cache.json`, `stats_cache.json` | Caches: rebuilt on the next sync, git overview, Hours or Metrics. |
| `workboard.log` | The v1 error log. |
| `webview/` | Window data of the v1 (pywebview). |
| `ctx/`, `claude/` | Per-session context and Claude Code settings: rebuilt when a session starts or resumes. |
| `reflect/`, `review/` | Reflect and Review packs already generated; the sessions that used them keep their transcripts. |
| `notas-borradas/`, `tools/_removed/` | Deleted notes and removed tools. |
| `prefs` other than `theme` | View state (filters, columns, sizes, active view, pinned sessions): the v2 keeps it in the window. |
| `ado.rm` | ReleaseManual settings. In the v2 ReleaseManual belongs to a plugin, with its own data (`plugin-data/<id>.json`) and its own write token; set it up from the plugin. |

## Code

- `crates/wb-api/src/migrate.rs`: the logic (`run` with the credential store and the DPAPI function as
  parameters, `render`, `parse_args`, `cli`). Report = codes `migrate.*` + parameters, translated with
  `locales/*.json`.
- `crates/wb-platform/src/dpapi.rs`: DPAPI through the `windows-dpapi` crate (safe wrapper; no `unsafe` in
  the workspace), and `wb_platform::v1_config_dir`.
- `src-tauri/src/main.rs`: `migrate-v1` is handled before the window opens.
- Tests: `crates/wb-api/tests/migrate.rs` builds a v1 folder in a temporary directory (with tokens
  encrypted with DPAPI at test time; outside Windows a fake cipher) and checks the result, the dry run,
  idempotency, `--force`, merges, bad tokens and that the report never contains a token.
