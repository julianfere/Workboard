# Agent sessions and the `workboard` MCP server

Workboard runs AI coding agents in an embedded terminal, one **session** per piece of work. Every session
gets the same things, whatever the agent:

- a **context file** with its items (folder, status, summary, open tasks), the Workboard guide and the
  promoted tools;
- the **`workboard` MCP server**, through which it reads and updates the state of its items;
- for Claude Code, **hooks** that tell Workboard whether the agent is working, needs you or is idle, and
  **permissions** to edit inside its items' folders without asking.

Sessions always start in the **parent of the work folder** (where the repos usually are), so the agent
sees the code and the work folder side by side.

## Agents

Pick the default agent in **Settings → AI agents**; **New session** lets you choose another one each time.
Workboard looks for each CLI on the `PATH` (and, on Windows, in the usual Node install folders, so npm
packages are found even when the app was started from the desktop). npm's `.cmd` shims are launched as
`node <script>`, because a pseudo-terminal cannot run `.cmd` files.

| Agent | Executable | Hooks (live status) | Split (fork) | Long-session weight |
|---|---|---|---|---|
| Claude Code | `claude` | yes | yes | yes (reads the transcript) |
| GitHub Copilot CLI | `copilot` | no | no | no |
| Custom command | anything | no | no | no |

### Claude Code

Launched as:

```
claude --session-id <id> -n <name>            (new; --resume <id> to resume;
                                               --resume <from> --fork-session --session-id <id> to split)
       --append-system-prompt-file <config>/ctx/<id>/AGENTS.md
       --mcp-config <config>/agents/mcp-<id>.json
       --settings <config>/claude/settings-<id>.json
       [--permission-mode plan|acceptEdits] [--model <model>] [<first prompt>]
```

### GitHub Copilot CLI

```
copilot --session-id <id> [-n <name>]
        --additional-mcp-config @<config>/agents/mcp-<id>.json
        --allow-all-mcp-server-instructions --allow-tool workboard
        [--model <model>] [--plan | --allow-tool write] [-i <first prompt>]
```

`--session-id` creates the session or resumes it. The context folder is added to
`COPILOT_CUSTOM_INSTRUCTIONS_DIRS`, so Copilot loads the `AGENTS.md` in it.

### Custom command

Any CLI that can load an MCP configuration. In **Settings → AI agents** set a name, the command (on the
`PATH` or a full path), its arguments and, optionally, different arguments to resume. Arguments are a
template with placeholders:

| Placeholder | Value |
|---|---|
| `{session_id}` | The session's UUID. |
| `{name}` | The session's name. |
| `{prompt}` | The first prompt. If empty, it is dropped together with the flag right before it (`-i {prompt}` disappears). |
| `{context_file}` | Path of the context file (`AGENTS.md`). |
| `{context_dir}` | Its folder. |
| `{mcp_config}` | Path of the MCP configuration (`{"mcpServers": {"workboard": …}}`). |
| `{cwd}` | Working directory (the parent of the work folder). |

Quotes group words (`--title "{name}"`). The process also gets `WORKBOARD_CONTEXT_FILE` and
`WORKBOARD_MCP_CONFIG`. Example for a CLI that takes an MCP file and an instructions file:

```
--mcp-config "{mcp_config}" --instructions "{context_file}" -i {prompt}
```

### Modes

A session runs in `default` (the agent asks for permissions), `plan` (read only, proposes a plan) or
`acceptEdits` mode. For Claude Code that is `--permission-mode`; for Copilot, `--plan` or
`--allow-tool write`. **Settings → AI agents → Plan starts with** sets, per agent, a prefix for the first
prompt of planning sessions (a skill such as `/plan `, for example).

## What Workboard writes per session

Everything goes to the configuration folder (`<config>`, see the [README](../README.md#configuration)),
never to the work folder, except one frontmatter field.

| File | Content |
|---|---|
| `<config>/sessions/<root>.json` | Registry of the sessions of a work folder (name, items, agent, mode, state, pinned…). Written only by the app; the MCP server reads it to know which items a session may write. |
| `<config>/ctx/<id>/AGENTS.md` | Context: the session's items with folder, status, summary and open tasks; the Workboard guide (how to use the MCP tools, file prefixes, rules); the promoted tools. |
| `<config>/agents/mcp-<id>.json` | MCP configuration: the `wb-mcp` executable with `WORKBOARD_SESSION`, `WORKBOARD_ROOT` and `WORKBOARD_CONFIG_DIR`. |
| `<config>/claude/settings-<id>.json` | Claude Code only: hooks and permissions (below). |
| `<config>/sessions/<id>.activity.jsonl` | One line per hook event, launch, exit and change of items. Source of the Hours and Metrics views. |
| `<config>/sessions/<id>.events.jsonl` | One line per write the agent made through the MCP server. |
| README frontmatter `sessions` | The session id is added to each of its items (traceability). |

### Claude Code settings

```json
{
  "hooks": {
    "SessionStart":     [{ "hooks": [{ "type": "command", "command": "\"…/wb-hook\" SessionStart", "timeout": 5 }] }],
    "UserPromptSubmit": [ … ], "PostToolUse": [{ "matcher": "*", … }],
    "Notification": [ … ], "Stop": [ … ], "SessionEnd": [ … ]
  },
  "permissions": {
    "allow": [
      "mcp__workboard",
      "Edit(<work>/SHOP-101-checkout-timeout/**)", "Write(<work>/SHOP-101-checkout-timeout/**)",
      "Read(<config>/reflect/**)", "Read(<config>/review/**)",
      "Read(<config>/sessions/**)", "Read(<config>/tools/**)",
      "Read(~/.claude/projects/**)"
    ]
  }
}
```

The agent can use the MCP server and edit files in its own items' folders without being asked; anything
else goes through Claude Code's normal permission prompts. The extra `Read` rules let Reflect and Review
sessions read their evidence.

## Hooks

`wb-hook <Event>` (installed next to the app) reads the event's JSON from stdin, posts it to
`127.0.0.1:$WORKBOARD_HOOK_PORT/hook/<Event>` with `$WORKBOARD_HOOK_TOKEN` and prints the answer. It never
fails: without the app, it exits 0 silently. What the hooks do:

| Event | Effect |
|---|---|
| `SessionStart`, `Stop` | The session is **idle**. |
| `UserPromptSubmit` | **Working.** Workboard adds to the turn's context anything queued for the agent (items added or moved) and a warning if the prompt names the id of an item in the work folder that is **not** in this session (the UI shows it too). |
| `PostToolUse` | Working again (after a permission prompt); edits are used to know which repos the session touched. |
| `Notification` | **Needs you** (a permission or a question), or idle. |
| `SessionEnd` | Ended. |
| `Stop` in a planning session | The plan check (below). |

The sessions sidebar shows that live status, and sessions whose items are all closed are closed
automatically (with undo).

## The `workboard` MCP server

`wb-mcp` speaks MCP over stdio. Its `instructions` repeat the guide and the session context, so a client
that ignores the context file still knows how to work. Tool names and answers are in English; error
messages follow the app's language.

### Modes

| Mode | When | Can write |
|---|---|---|
| Session | `WORKBOARD_SESSION=<id>` (sessions launched by Workboard) | Only the items of that session. A group in the session allows every item of the group. |
| Write | `WORKBOARD_MCP_WRITE=1` | Any item of the work folder. |
| Read-only | Neither | Nothing. |

The work folder is `WORKBOARD_ROOT` or the one last opened in the app; the workflow and language come from
`settings.json`. Every write is appended to `<config>/sessions/<id>.events.jsonl` (or
`<config>/mcp_events.jsonl` outside a session). The server exposes no git and no tracker tools.

To use it outside Workboard (read-only), register the `wb-mcp` executable that ships next to the app as a
stdio server in your agent, for example `claude mcp add workboard -- <path>/wb-mcp`.

### Tools

`item` is always an item id (`SHOP-101`) or its folder name (`grp-loyalty/SHOP-110-points-balance`).

| Tool | Parameters | What it does |
|---|---|---|
| `workboard_session` | — | Mode, work folder and the session's items with status, tasks and folders. Agents are told to call it first. |
| `workboard_list_items` | `search` (text in folder, id, title or notes), `status`, `include_closed` (default false) | Finds items. Closed items (done/cancelled categories) are left out unless asked for. |
| `workboard_get_item` | `item` | Frontmatter, tasks with their index, files (and where each script already ran), the deploy section, and planning data: priority, agreed date, period and agreed hours, local or from the tracker cache. |
| `workboard_update_item` | `item`, and any of `status`, `type`, `environments` (full list), `notes` (one line: what is left), `branch`, `due` (`YYYY-MM-DD`), `hours` | Changes only the fields passed; `""` clears one. Validated like the UI. The agent is told to set date and hours only when the user gives them. |
| `workboard_add_task` | `item`, `text` | Adds an open task (creates the section if needed). |
| `workboard_complete_task` | `item`, `task` (index, exact text or a fragment that matches one task), `done` (default true; false reopens) | Checks or unchecks a task. |
| `workboard_mark_ran` | `item`, `file` (name or path inside the item), `environment`, `ran` (default true; false removes the mark) | Records that the **user** ran a script in an environment. Only when the user says so. |
| `workboard_add_note` | `item`, `title`, `text` | Creates a personal note linked to the item, in the notes folder (outside the project), authored by the agent. |
| `workboard_list_tools` | — | The promoted tools: name, description, usage, interpreter and whether its hash is still valid. |
| `workboard_run_tool` | `name`, `args` (list), `item` (default: the session's first item) | Runs a promoted tool in the item's folder. Only inside a session, only on the session's items, 300 s timeout. |

All README writes go through the same writer as the app: retry if the file changed meanwhile, a backup of
the previous version, atomic write.

## Planning, reviewing and learning

### Plan

**Plan** on an item moves it to the workflow's first `planned` state, adds a `## Plan` section with a
placeholder if there is none, and opens a session in plan mode with a prompt that asks the agent to fill
the plan and load the tasks through the MCP server.

When the agent finishes a turn (`Stop` hook), Workboard checks every item of the session that is in a
`planned` state: the plan section must have at least `plan.minChars` characters besides the placeholder,
and there must be at least one open task. If only one of the two is missing, the turn is blocked and the
agent is sent back with what is missing (at most twice). If both are missing the agent is probably asking
you something, so it is not blocked. Items with an incomplete plan also show in **Alerts**.

### Reflect

**Reflect** (on one or more items) builds an evidence file in `<config>/reflect/<stamp>.md`: what happened
with each item, the sessions that worked on it with their metrics, the current instructions, and where to
read the details (transcripts are not copied in). Then it opens a plan-mode session that reads it and proposes improvements: to the Workboard guide, the project's
`CLAUDE.md`/`AGENTS.md`, a skill, the app, or a script worth promoting. The result is saved as a
«Learnings» note with numbered candidates marked accept / review / discard. Nothing is changed.

### Apply candidates

From a learnings note, **Apply candidates…** opens a normal session (it asks permission for every change)
on the items linked to the note, applying the candidates you pick by number, or the ones marked accept.

### Review

**Review** on an item with an id asks another model to review its branches. Workboard builds a packet in
`<config>/review/<stamp>-<id>/`: `REVIEW.md` (the README path, each repo and branch with its commit range
and stat) and one `.diff` per branch with the item's own commits, made with read-only git. Then it opens a
plan-mode session, optionally with a different `--model`, that reviews the diff against the README's
acceptance criteria, leaves the verdict in a «Review» note and adds blocking findings as tasks.

## Long sessions

For Claude Code sessions Workboard reads the end of the transcript (`~/.claude/projects/*/<id>.jsonl`) to
know the size of the context of the last answer. Above 180k tokens the session is marked **long**, above
350k **very long**, and Workboard recommends a way out:

| Recommendation | When |
|---|---|
| New session | The README was updated after the last code edit: what matters is saved. |
| Handoff, then new session | An item changed status in this session, or there has been no activity for 3 hours or more, and there is work the README does not reflect yet. |
| Compact | You are in the middle of something. |

The buttons **Ask for handoff** (sends a fixed prompt asking the agent to update each item's summary and load what is
left as tasks, so a new session can pick it up from the README),
**Compact** (`/compact`) and **Continue in a new session** (closes this one and opens another on the same
items) act on it. Prompts are written to the terminal only when the agent is idle.

## Other session actions

Rename, pin, add items to a session, remove an item, **Split** (moves one item to a new session; with
Claude Code the new one inherits the conversation), pause, close and reopen. When folders are moved,
renamed or grouped, the sessions follow their items and the agent is told about the move in its next turn.

## Tools

Sessions write scripts. The **Tools** view lists **candidates**: scripts written by sessions outside git
repositories that ran three times or more, or were written in more than one session. **Promote** copies
the script to `<config>/tools/<name>/` with a `tool.json` (description, usage, origin and SHA-256).

The copy is frozen: if the file changes, `workboard_run_tool` refuses to run it until you accept the change
in the Tools view. Sessions see the promoted tools in their context and through `workboard_list_tools`, and
are told to check there before writing a script for something repetitive. Interpreters by extension:
`.py` python, `.ps1` powershell, `.sh` bash, `.js`/`.mjs`/`.cjs` node, `.bat` cmd.
