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
| — (hard-coded in the v1: `/go ` for Claude, `harness.py` `PLAN_PREFIX`) | `settings.json` → `agents.planPrefix.claude` = `/go ` | Usually *already there* (it is the v2 default). An empty `planPrefix: {}` saved by an earlier v2 counts as another value: kept, `--force` sets `/go `. |
| `git[<root>]` = `[{name, path, envs}]` | `settings.json` → `git.repos[<root>]` | The root key goes from lower case to the path as written; environment keys (`DEV`) to the workflow ids (`dev`). |
| `ado.conn[<root>]` = `{org, project}` | `trackers.json` → `connections[<root key>]` = `{kind: "azure-devops", org, project}` | Org URL normalized as in the v2. |
| `ado.procs["org\|project"]` | `trackers.json` → `projects["azure-devops\|org\|project"]` | `states` (plus `suggested` from the Azure category), `fetchedAt`, `map`, `mapSaved`, `dateFields`, `numFields`, `hasPriority`, `hasEffort`; `pactadaField` → `dueField`, `horasField` → `hoursField` (`""` = "none" is kept). Mapped states the preset does not have are left out and reported. |
| `ado.pats[org]` (DPAPI, base64) | credential store, `azure-devops:read:<org>` | Decrypted with DPAPI (current user). |
| `ado.wpats[org]` (DPAPI, base64) | credential store, `azure-devops:write:<org>` | Same. This is the write token of **Change status in the tracker** and also the one ReleaseManual (private plugin, `host/tracker/writeToken`) uses, as in the v1. |
| `ado.rm["org\|project"]` | `plugin-data/private.json` → `rm["<org>\|<project>"]` | ReleaseManual settings for the private plugin (`private`): `type`, `repo`, `base`, `prefix`, `scriptPath`, `commitMsg`, `link`, `form`, `formFetchedAt`, `visible`, `last` (same format; other keys left out). Key: normalized org URL and project, lower case (what the plugin builds from the folder connection). Other keys of the file are kept. Written even if the plugin is not installed yet: it picks it up once enabled. |
| `runs.json` | `runs.json` | Same format; root keys recomputed; marks merged per file and environment. |
| `state_log.jsonl` | `state_log.jsonl` | Same format; rows merged without duplicates, sorted by `ts`. |
| `mcp_events.jsonl` | `mcp_events.jsonl` | Same. |
| `tools/<name>/` (`tool.json` + script) | `tools/<name>/` | Copied as is (same `tool.json` keys and sha256). A script that no longer matches its hash is copied and reported: it does not run until promoted again, as in the v1. |
| `inbox.md` | `inbox.md` | Same format. |
| `backups/` | `backups/` | Same layout (`<root>-<sha1[:12]>/<item>/…`); files added without overwriting. Keeps **Versions** and the status history of Metrics. |
| `sessions/<root>.json` | `sessions/<root>.json` | Same file name (same root key). Each session goes from the v1 keys (`nombre`, `raiz`, `wis`, `estado`, `modo`, `agente`, `modelo`, `creada`, `ultimaActividad`, `forkDe`) to the v2 ones; `cerrada` → `closed`, anything else → `paused` (resume them with **Resume**: same id, same Claude Code transcript). Merged by id. |
| `sessions/<id>.activity.jsonl`, `<id>.events.jsonl` | same names | Merged without duplicates. The v2 reads the v1 rows (`wis`), so Hours and Metrics keep the history. |
| `prefs["pinnedSess@<root>"]`, `prefs["keepSess@<root>"]` | `sessions/<root>.json` → `pinned`, `keepOpen` of each session | Pinned sessions and sessions reopened by hand (they do not close by themselves). |

### Window preferences (`prefs`)

The v1 kept the view state in `config.json` → `prefs` (written by `setPref` in `ui/app.js`); the v2 keeps it in
`ui_prefs.json` (one key per view, read by the UI) and `alerts_off.json`. Ids are translated; states, types and
environments are checked against the preset. Values the v2 does not have are left out and counted in the report.

| v1 `prefs` | v2 | How |
|---|---|---|
| `views`, `activeView` and the current `stateFilter`, `hideClosed`, `typeFilter`, `pactFilter`, `period`, `groupBy`, `gridCols`, `gridSort`, `density` | `ui_prefs.json` → `grid` = `{views, active, current, flipped: {}, open: {}}` (`grid.ts`) | Each view and the current values become a `Snap`: `stateFilter` → `status` (`none` → `""`), `typeFilter` → `type`, `pactFilter` → `due`, `period` → `activity`, `groupBy` (`week-last` → `weekLast`, `week-created` → `weekCreated`, `periodo` → `period`, `state` → `status`, `tipo` → `type`, `azure` → `tracker`, `prio` → `priority`; `none` → `[]`), `gridCols` (`state` → `status`, `wi` → `id`, `tipo` → `type`, `session` → `sessions`, `notas` → `notes`, `azure` → `tracker`, `rama` → `branch`, `prio` → `priority`, `pactada` → `due`, `horas` → `hours`, `real` → `worked`, `rm` → `x:release_manual` if the preset has that field; widths kept), `gridSort` (`asc` → `1`, `desc` → `-1`), `density` (`compacta` → `compact`). Factory view ids `mia`, `activos`, `qa`, `todo` → `mine`, `active`, `review`, `all` (names kept). Without `views` the v2 factory views are used. |
| `tasksState`, `tasksSess`, `tasksEnv`, `tasksPact`, `tasksShowDone` | `ui_prefs.json` → `tasks` = `{status, session, env, due, showDone}` | Environment to the workflow id (`QA` → `qa`). |
| `detailWidth` (px or `"max"`), `detailWidthRestore` | `ui_prefs.json` → `detail` = `{width, max}` | At least 560 px, as the v2 panel. |
| `hideHours` | `ui_prefs.json` → `hideHours` | Discreet mode. |
| `view` | `ui_prefs.json` → `view` | Last view: `hoy` → `today`, `backlog` → `items`, the others keep their id. |
| `hoursAll`, `hoursSort`, `hoursPeriod@<root>` (period start, ms) | `ui_prefs.json` → `hours` = `{all, sort, period: {<root>: "YYYY-MM"}}` | The period code is the year and month of its start (local time), as the v1 `periodCode`. |
| `mxRange`, `mxTipo`, `mxAll` | `ui_prefs.json` → `metrics` = `{range, type, all}` | Type checked against the preset. |
| `heavyOff@<root>` = `{<session>: "larga" \| "alta"}` | `ui_prefs.json` → `sessionWeightHidden` = `{<session>: "long" \| "heavy"}` | Hidden long-session warnings; merged per session from all roots. |
| `alertsOff@<root>` = `{"<item>\|<code>": fingerprint}` | `alerts_off.json` → `{<root key>: {"<item>\|<kind>": fingerprint}}` (`alerts.rs`, `alerts.ts`) | Merged per alert. Codes: `dup` → `dupes`, `migration` → `finalEnv`, `stale`, `nostate` → `noStatus`, `noprefix` → `noPrefix`, `noreadme` → `noReadme`, `gitdesync` → `gitDesync`, `gituncommitted` → `gitDirty`, `planincomplete` → `plan`. Fingerprints are recomputed as the v2 does: plan gaps `pendientes` → `tasks`; git desyncs with the workflow environment ids in workflow order; the rest are already equal. An alert that cannot be translated is left out (reported): it shows again and can be dismissed again. |

## What is not migrated

| v1 | Why |
|---|---|
| `ado_cache.json`, `git_cache.json`, `time_cache.json`, `stats_cache.json` | Caches: rebuilt on the next sync, git overview, Hours or Metrics. |
| `workboard.log` | The v1 error log. |
| `webview/` | Window data of the v1 (pywebview). |
| `ctx/`, `claude/` | Per-session context and Claude Code settings: rebuilt when a session starts or resumes. |
| `reflect/`, `review/` | Reflect and Review packs already generated; the sessions that used them keep their transcripts. |
| `notas-borradas/`, `tools/_removed/` | Deleted notes and removed tools. |
| Other `prefs`: `openGroups@`/`closedGrps@`/`periodFlip@`/`subFlip@` (open groups), `dockH`, `detailSide`, `detailTab`, `closedCats`, `promoTarget`/`promoMode`/`promoShell`, `planMode`, `reviewModel`, `noteSort`/`noteWi`, `alertsShowOff`, `sessMore@` | The v2 does not keep them, or keeps them elsewhere (promotion and notes choices in the window storage) or with other keys (open groups use the v2 group keys). They go back to their defaults. |

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
