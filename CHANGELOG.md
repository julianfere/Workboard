# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - Unreleased

First public release: a rewrite in Rust + Tauri 2 of a Python desktop app, with its behavior pinned by
golden files captured from the original.

### Added

#### Work folders

- Each sub-folder of the open folder is an item; status, type, environments, ids, notes, branch, title,
  agreed date, period and hours live in the README frontmatter (`k: v` and `k: [a, b]`), tasks in a
  checklist section.
- Groups (`grp-*` folders, one level, with their own README); `_` and `.` folders ignored; item ids from
  the folder name or the frontmatter; duplicated ids detected.
- One safe README writer for the app, the MCP server and plugins: mtime check with retry, backup of the
  last 20 versions, atomic write, UTF-8 + LF; reads UTF-8, BOM and cp1252.
- README versions: list, view, restore. Status changes logged to `state_log.jsonl`.
- Tasks: add, check, edit, remove (with a guard against stale indexes), clear done; moving to a closed
  state checks the open ones. Inbox for tasks without an item.
- New item (with or without an id), group, move into or out of a group, dissolve, rename, delete to
  `_deleted` with undo and restore.
- File categories by name prefix and extension; scripts marked as ran per environment.
- Field validation shared by the UI, the MCP server and plugins.
- Incomplete-plan detection; personal notes linked to items (any folder, e.g. an Obsidian vault).

#### Workflow

- Workflow presets: states with categories, types with aliases, environments, period start day, folder
  patterns, frontmatter keys, section titles, file categories and git rules. Built-in `default` (English)
  and `classic-es` (the original flow). Selectable in Settings.

#### UI

- Views: Today, Work, Tasks, Hours, Alerts, Notes, Metrics, Tools, Settings; item detail with Summary,
  README, Files, Versions and Git tabs and automatic saving of the edited fields.
- Item grid with configurable columns, filters, grouping and saved views, CSV export, a context menu with
  hover submenus and bulk actions with undo; file viewer (SQL, Markdown, images, PDF, draw.io, Mermaid);
  dismissable alerts; notes management; keyboard shortcuts.
- Live refresh of the work folder and of the repos' `.git`; command palette (`Ctrl+K`); recent folders;
  light, dark and system themes; English and Spanish.

#### Agent sessions

- Embedded terminal (ConPTY / pty) with tabs; closing kills the process tree.
- Agents: Claude Code, GitHub Copilot CLI and a custom command with placeholders.
- Per-session context file, MCP configuration, and Claude Code settings with hooks and permissions scoped to
  the session's folders.
- Live status from hooks (working / needs you / idle), activity log, automatic closing of sessions whose
  items are all closed.
- Sidebar with pinned, running, recent, older and closed sessions and sessions of other folders; rename,
  add or remove items, split (fork), reopen.
- Long-session weight from the transcript and a recommendation: new session, handoff or compact.
- Warning when a prompt names an item that is not in the session.
- Plan: planning session in plan mode, with the plan checked when the agent stops.
- Reflect (evidence-based improvement proposals saved as a note), Apply candidates, and Review (another
  model reviews the branch diff against the acceptance criteria).
- Hours per item and period; metrics per session; CSV export.
- Tools: candidates from session scripts, promotion, frozen by SHA-256.

#### MCP server `workboard`

- Tools: `workboard_session`, `workboard_list_items`, `workboard_get_item`, `workboard_update_item`,
  `workboard_add_task`, `workboard_complete_task`, `workboard_mark_ran`, `workboard_add_note`,
  `workboard_list_tools`, `workboard_run_tool`.
- Session mode (writes only the session's items), global write mode, read-only mode; instructions with the
  guide and the session context; an event log per session.

#### Git (read-only)

- Repos next to the work folder detected by their environment branches, or configured by hand.
- Per-environment state of each item's branches: merged, equivalent, content, mentioned, pending; current
  branch, uncommitted files, last fetch; alerts for repos out of step and uncommitted changes.
- Results cached by SHA, in-memory commit graph, repos read in parallel.
- Promotion: cherry-pick or `merge --no-ff` commands for PowerShell or Bash plus the pull request URL
  (Azure DevOps, GitHub, GitLab); never executed by the app.
- Branch diff for Review.

#### Trackers

- Azure DevOps: connection per work folder; read token in the system credential store; state mapping
  suggested from the tracker's categories; due date and hours fields chosen or guessed; priority.
- Sync with a preview (fills, type, advances, moves back, per git); applying writes only status and type.
- Assigned this period, import, and New item filled from a work item.
- Change state in Azure DevOps with a separate write token (only `System.State`).
- Trackers from plugins: a plugin can add a tracker kind (Jira, Trello…) that works like the built-in one
  (connect, states, mapping, sync, assigned, import, change state); per-kind connection form, ids filtered
  by `idPattern`, credential scrubbed from errors and logs, configuration kept while the plugin is missing.
  Reference plugin for Trello boards in `examples/plugins/trello`.

#### Plugins

- Plugins as separate executables speaking JSON-RPC over stdio: manifest with permissions, actions in the
  item detail, bulk bar, palette and Settings, generic forms and results, sessions, contributed presets,
  host methods for items, files, secrets, data and the tracker connection. Example plugin in
  `examples/plugins/hello`.
- Plugin SDKs for Python and Node (`sdk/`, single files without dependencies): actions, trackers, step
  builders, host requests and errors. `workboard plugin new <id>` creates a ready-to-enable plugin and
  `workboard plugin check <folder>` validates it and runs `initialize`.

#### Platform and release

- Windows, macOS and Linux; configuration in `%APPDATA%\Workboard2`,
  `~/Library/Application Support/Workboard2` or `~/.config/Workboard2` (`WORKBOARD_CONFIG_DIR` overrides).
- Dev server with the same API over HTTP, with a fake tracker for offline UI work.
- Documentation and a demo work folder (`examples/demo-work`).

### Known gaps

- The Trello reference plugin is tested only against a fake API, not against Trello itself.
- On Windows, the installed app is a windowed program: `workboard migrate-v1` and `workboard plugin`
  print nothing in an interactive console unless the output is redirected (for example `| Out-Host`).
