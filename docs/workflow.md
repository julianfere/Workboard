# Workflow presets

Everything that is specific to *how you work* lives in a workflow preset, not in Workboard's code:
the states and what each one means, the item types, the environments, when a period starts, how folders
are named, which frontmatter keys hold which data, the titles of the README sections Workboard reads, how
files are categorized and how items are recognized in git branches and commits.

Workboard ships two presets (`crates/wb-core/presets/`):

| Id | For |
|---|---|
| `default` | English, general purpose: `PROJ-123-slug` or `123-slug` folders, `backlog → planned → in-progress → review → testing → done`, monthly periods, `dev`/`staging`/`prod`. |
| `classic-es` | The original (Spanish) flow Workboard was born with: `wi123456-slug` folders, periods from the 20th to the 19th, `dev`/`qa`/`prod`. It reproduces the first version byte for byte and is what the golden tests run against. |

Pick one in **Settings → Workflow → Preset**. The choice is stored in `settings.json` (`"workflow": "<id>"`)
and used by the app, the agent sessions and the `workboard` MCP server. Plugins can add more presets (see
[Adding presets](#adding-presets-with-a-plugin)).

> Changing the preset does not rewrite any README. If the new preset uses different frontmatter keys or
> state ids, existing items simply show up without status until you edit them.

## Format

A preset is a JSON object. Unknown keys are rejected (a typo fails loudly instead of being ignored).
Texts shown to the user are *labels*: `{ "en": "…", "es": "…" }`. The UI picks the current language, then
English, then the id.

```json
{
  "id": "my-flow",
  "name": { "en": "My flow" },
  "states": [ … ],
  "types": [ … ],
  "environments": [ … ],
  "period": { "startDay": 1 },
  "folders": { … },
  "fields": { … },
  "tasks": { "heading": "Tasks", "maxLen": 300 },
  "deployKeywords": ["deploy", "release"],
  "notes": { … },
  "inboxTitle": "Inbox",
  "plan": { … },
  "sections": { … },
  "files": { … },
  "git": { … },
  "alerts": { … }
}
```

Every key below is required unless marked *optional*.

### `id`, `name`

`id` identifies the preset in `settings.json`; `name` is what **Settings** shows.

### `states`

The states an item can be in, **in workflow order**. Each has an `id` (what is written in the README),
`labels` and a `category`:

```json
"states": [
  { "id": "backlog",     "category": "backlog",   "labels": { "en": "Backlog" } },
  { "id": "planned",     "category": "planned",   "labels": { "en": "Planned" } },
  { "id": "in-progress", "category": "active",    "labels": { "en": "In progress" } },
  { "id": "review",      "category": "review",    "labels": { "en": "In review" } },
  { "id": "testing",     "category": "review",    "labels": { "en": "Testing" } },
  { "id": "done",        "category": "done",      "labels": { "en": "Done" } },
  { "id": "cancelled",   "category": "cancelled", "labels": { "en": "Cancelled" } }
]
```

The category says what a state is *for*, independently of its name. Workboard never looks at state ids,
only at categories and order:

| Category | Used for |
|---|---|
| `backlog` | The initial state of items created from a tracker (the first `backlog` state, or the first state). |
| `planned` | **Plan** moves the item to the first `planned` state. Items in a `planned` state are checked for an incomplete plan (see [`plan`](#plan)). |
| `active` | Work in progress (Today view, ordering). |
| `review` | Coded, in review or testing. |
| `done`, `cancelled` | **Closed**. Closed items are hidden from the MCP's `workboard_list_items` unless asked for; moving an item to a closed state checks its open tasks; a session whose items are all closed is closed automatically. |
| `unknown` | Outside the scale (for example "we don't know where this is"). |

Order matters too. `progress` (*optional*) is the state's step on the workflow scale; states with the same
`progress` tie (in `classic-es`, `diag` ties with `en-curso`, as in v1). Without it, the state's position is
used. Session metrics measure advance and rework with it, and the tracker sync ranks a state by
`(category, progress)` with categories in the order backlog < planned < active < review < done; a proposed
state with a lower rank than the current one is a **move back** and is left unchecked. `cancelled` and
`unknown` states are off the scale: they never count as moving back.

At least one state is required; ids must be unique.

### `mainPath`

*Optional.* The states of the normal path, in order: the item detail shows them as steps (an item in a state
off the path, like a "diagnosis" side state, shows it as an extra step at the end) and its "Move to" button
goes to the next one. Without it, every state that is not `cancelled` or `unknown` is on the path. Each entry
must be a known state, without repeats.

```json
"mainPath": ["backlog", "planned", "in-progress", "review", "testing", "done"]
```

### `types`

Kinds of item (bug, feature…). `aliases` (*optional*) are other names that map to this type, compared
case-insensitively: tracker sync and **New item** use them to translate a tracker's work item type
(`User Story`, `Product Backlog Item`…) into a local type. `icon` (*optional*) is the icon shown next to the
id in the **Items** grid (a name from `ui/src/icons.ts`: `bug`, `story`, `task`, `wrench`…).

```json
"types": [
  { "id": "bug", "labels": { "en": "Bug" }, "icon": "bug" },
  { "id": "feature", "labels": { "en": "Feature" }, "aliases": ["user story", "product backlog item", "epic"], "icon": "story" }
]
```

A type in the README that is not in this list is ignored.

### `environments`

Where work gets deployed, in promotion order (promoting to an environment starts from the previous one).
Same shape as `types`. Items record where they already are in the `environments` field, scripts are marked
as "ran in" an environment, and the git rules map each environment to remote branches.

```json
"environments": [
  { "id": "dev", "labels": { "en": "Dev" } },
  { "id": "staging", "labels": { "en": "Staging" } },
  { "id": "prod", "labels": { "en": "Production" } }
]
```

### `period`

```json
"period": { "startDay": 1 }
```

Hours and agreed dates are grouped by period. A period is named `YYYY-MM` and starts on `startDay` (1 to
28) of that month: with `1` it is the calendar month; with `20` the period `2026-11` runs from November 20
to December 19. A date before `startDay` belongs to the previous month's period. **Assigned this period**
uses the same boundaries.

### `folders`

```json
"folders": {
  "itemPattern": "^(?P<id>[A-Za-z][A-Za-z0-9]*-\\d+|\\d+)(?:-|$)",
  "itemIdPattern": "^([A-Za-z][A-Za-z0-9]*-\\d+|\\d+)$",
  "newItemTemplate": "{id}-{slug}",
  "groupPrefix": "grp-",
  "deletedDir": "_deleted",
  "ignorePrefixes": ["_", "."]
}
```

| Key | Meaning |
|---|---|
| `itemPattern` | Regex applied to a folder name to find the item's id. Needs a named group `id`; an optional `id2` lets one folder hold two ids (`wi123456-654321-slug` in `classic-es`). A folder that does not match is still an item, just without an id. |
| `itemIdPattern` | Regex every id typed by hand (frontmatter `id`, **New item**) must match. |
| `newItemTemplate` | Suggested folder name for a new item: `{id}` and `{slug}` (from the title). |
| `groupPrefix` | Folders starting with this (case-insensitive) are **groups**. |
| `deletedDir` | Where **Delete** moves folders (inside the work folder, so it can be undone). |
| `ignorePrefixes` | Folders starting with any of these are not scanned (`_deleted`, `.git`, `_templates`…). |

The id of an item is the one in its frontmatter (`fields.itemIds`, a value or a list) if present, otherwise
the one(s) in its folder name. Two folders with the same id raise an alert.

**Groups.** A group is a folder `grp-<slug>/` at the root with its own `README.md` and one sub-folder per
item, one level deep. Shared files go in the group folder; files of a single item go in that item's folder.
An item inside a group is named by its relative path (`grp-loyalty/SHOP-110-points`). Sessions on a group
can write every item in it.

### `fields`

Which frontmatter key holds each piece of data Workboard understands. Keys use only `a-z` and `_`, and
must be distinct.

```json
"fields": {
  "status": "status",
  "type": "type",
  "environments": "environments",
  "itemIds": "id",
  "notes": "notes",
  "branch": "branch",
  "title": "title",
  "due": "due",
  "period": "period",
  "hours": "hours",
  "sessions": "sessions",
  "extra": []
}
```

| Field | Value |
|---|---|
| `status` | A state id. |
| `type` | A type id. |
| `environments` | List of environment ids where the work already is: `[dev, staging]`. |
| `itemIds` | Id or list of ids, overriding the folder name. |
| `notes` | One line: what is left. |
| `branch` | Branch name (no spaces). |
| `title` | Overrides the README's first `# heading`. |
| `due` | Agreed date, `YYYY-MM-DD`. Overrides the tracker's. |
| `period` | `YYYY-MM`. If missing, derived from `due`. |
| `hours` | Agreed hours (a number). Overrides the tracker's estimate. |
| `sessions` | Agent sessions that worked on the item. Written by Workboard; not editable. |
| `extra` | *Optional.* More single-line text fields: `[{ "key": "release_note", "noSpaces": false }]`. Each one is also a column of the **Items** grid. Each may have `labels` (display name, default the key), `icon` (a name from the app's icon set) and `link`: with `link: "tracker"` the value is a tracker work item id, and with an `https://…{value}` template it is the part that goes in the URL; the grid cell and the item detail (header and side panel) show it as a link. |

All of them except `sessions` are editable from the UI, validated the same way by the UI, the MCP and
plugins, and an empty value deletes the key. The frontmatter is a subset of YAML: `key: value` and
`key: [a, b]` only.

### `tasks`

```json
"tasks": { "heading": "Tasks", "maxLen": 300 }
```

The task section of each README (`## Tasks`, matched case-insensitively). Workboard reads and writes only
its top-level `- [ ]` / `- [x]` lines; anything else in the section is left alone. `maxLen` caps a task's
length.

### `deployKeywords`

*Optional.* Lowercase words that identify the "how to deploy" section of a README (`## How to deploy`,
`## Release steps`…). Its text is shown in the item's detail and returned by `workboard_get_item`.

### `notes`

```json
"notes": { "itemsKey": "items", "authorKey": "author", "createdKey": "created", "untitled": "Untitled note" }
```

Personal notes live outside the work folder (by default `<config>/notes`; any folder, such as an Obsidian
vault, can be set in **Settings**). These are the frontmatter keys of a note: the items it is linked to
(`items: [SHOP-101]`), who wrote it (`author: claude` for agents) and when; `untitled` is the title of a
note without one.

### `inboxTitle`

Title of the inbox: tasks not tied to any item, kept in `<config>/inbox.md`.

### `plan`

```json
"plan": { "heading": "Plan", "placeholder": "*(filled in by the planning session)*", "minChars": 40 }
```

**Plan** on an item moves it to the first `planned` state, adds a `## Plan` section with `placeholder` if
the README has none, and opens an agent session in plan mode. An item in a `planned` state has an
**incomplete plan** when the section, without the placeholder, has fewer than `minChars` characters, or when
it has no open task. That raises an alert and, in the planning session, sends the agent back to finish
(see [agents](agents.md#plan)).

### `sections`

```json
"sections": { "description": "Description", "criteria": "Acceptance criteria" }
```

Titles of the sections Workboard writes when it creates an item from a tracker work item: its description
and acceptance criteria (converted from HTML to Markdown) go under `## Description` and
`## Acceptance criteria`.

### `files`

How each file of an item is categorized in the **Files** tab and whether it can be marked as "ran in" an
environment.

```json
"files": {
  "kinds": ["ROLLBACK", "MIGRATION", "FIX", "SEED", "DIAG", "VERIF", "PLAN", "HANDOFF", "QA"],
  "categories": [
    { "id": "script", "runnable": true, "extensions": [".py", ".ps1", ".sh"], "labels": { "en": "Scripts" } },
    { "id": "migration", "runnable": true, "kinds": ["MIGRATION", "SEED", "ROLLBACK"], "labels": { "en": "Migrations" } },
    { "id": "diagnostic", "runnable": true, "kinds": ["DIAG", "VERIF"], "labels": { "en": "Diagnostics" } },
    { "id": "doc", "runnable": false, "kinds": ["PLAN", "HANDOFF", "QA"], "extensions": [".md", ".pdf"], "labels": { "en": "Documentation" } },
    { "id": "sql", "runnable": true, "extensions": [".sql"], "labels": { "en": "SQL" } }
  ],
  "fallbackCategory": { "id": "other", "runnable": false, "labels": { "en": "Other" } }
}
```

- `kinds`: words looked for in the file name, upper-cased, in order. A word must not be glued to other
  letters: `MIGRATION_add_index.sql` and `wi1-MIGRATION.sql` are migrations, `MIGRATIONS.sql` is not.
- `categories`: the **first** category whose `kinds` contains the file's kind, or whose `extensions`
  contains its extension, wins. So `MIGRATION_x.sql` is a migration, not plain SQL, and `PLAN_x.md` is
  documentation. Every kind used must be listed in `kinds`.
- `runnable`: files in this category can be marked as ran in dev/staging/prod (stored in
  `<config>/runs.json`; the MCP tool `workboard_mark_ran` writes the same marks).
- `fallbackCategory`: everything else.
- `openFirst` (*optional*): category ids, in order; the **Files** tab of the item detail opens the newest file of the first one the item has (else its first text file).
- `exampleKind` (*optional*): the kind the agent guide uses in its example file name
  (`DIAG_WI<number>_<what it checks>.sql`); it must be listed in `kinds`. Default: the first kind.

Agents are told about these prefixes in their guide, so the files they create land in the right category.

### `git`

*Optional.* Rules of the read-only Git panel; no branch or remote name is hard-coded. Without it, the
`idPattern`, `ignorePattern` and `promotePrefix` below are used, with no `envRefs` and no `envStates`
(`envRefs` and `envStates` are optional on their own too). The `default` preset's values:

```json
"git": {
  "idPattern": "(?:^|[^A-Za-z0-9])#?(?P<id>[A-Za-z][A-Za-z0-9]*-\\d+|\\d{2,})(?:[^0-9]|$)",
  "ignorePattern": "(?i)\\b(?:merged\\s+)?(?:pr|pull\\s+request)\\s*#?\\d+",
  "promotePrefix": "promote/",
  "envRefs": {
    "dev": ["origin/develop", "origin/development", "origin/dev"],
    "staging": ["origin/staging"],
    "prod": ["origin/main", "origin/master"]
  },
  "envStates": { "staging": "testing", "prod": "done" }
}
```

| Key | Meaning |
|---|---|
| `idPattern` | Regex with a named group `id` that finds item ids in branch names and commit messages. The text around the group may consume one boundary character. A branch whose name contains an item's id belongs to that item (one branch can carry several ids). |
| `ignorePattern` | Removed from a commit message before looking for ids, so `Merged PR 482` does not count as item 482. |
| `promotePrefix` | Prefix of the branches the promotion commands create. Branches with this prefix are copies of other work, not an item's own branches. |
| `envRefs` | For each environment, candidate remote branches; the first one that exists in a repo is that environment's branch. Used to detect repos next to the work folder (a sibling repo with none of these branches is ignored) and as the default environments of each repo. Can be overridden in **Settings** without editing the preset. Keys must be environment ids. |
| `envStates` | The state an item should be in once its work is in an environment in **all** its repos. The tracker sync proposes it as an unchecked "per git" row. An environment without an entry proposes nothing. Keys must be environments, values states. |

How the Git panel uses these rules is described in the [README](../README.md#git-panel-read-only).

### `alerts`

*Optional* (and so is each key). Workflow-specific rules of the **Alerts** view.

```json
"alerts": {
  "finalEnvKinds": ["MIGRATION", "FIX"],
  "staleStates": ["in-progress", "review", "testing"]
}
```

| Key | Meaning |
|---|---|
| `finalEnvKinds` | File kinds (from `files.kinds`) that write data and must be run in the **last** environment. An item that has one of these files, has a state, is not closed and does not list the last environment in its environments field raises the "not run in the final environment" alert. Empty: no such alert. |
| `staleStates` | States in which an item without changes for 14 days raises the "no movement" alert. Empty: every state of category `active` or `review`. |

## Folder conventions in short

```
work/                              the folder you open in Workboard
├── SHOP-101-checkout-timeout/     an item: id SHOP-101 (from itemPattern)
│   ├── README.md                  frontmatter + "# Title" + "## Tasks"
│   ├── DIAG_SHOP-101_slow.sql     category "diagnostic" (kind DIAG)
│   └── MIGRATION_SHOP-101_ix.sql  category "migration", runnable
├── cleanup-feature-flags/         an item without id
├── grp-loyalty-program/           a group (groupPrefix)
│   ├── README.md                  the group's own README
│   ├── SHOP-110-points-balance/   items of the group, one level
│   └── SHOP-111-points-expiry-job/
├── _deleted/                      deletedDir: ignored, restorable from the app
└── .obsidian/                     ignored (ignorePrefixes)
```

`examples/demo-work/` is a complete example for the `default` preset.

## Adding presets with a plugin

A plugin lists preset files in its manifest (`"presets": ["presets/my-flow.json"]`). While the plugin is
enabled, its presets appear in **Settings → Workflow** next to the built-in ones; an id that clashes with a
built-in preset is ignored. The same preset is resolved by the app, the sessions and the MCP server. See
[plugins](plugins.md).

If you change a built-in preset in a fork, change both `default.json` and `classic-es.json`: the tests load
every preset and the golden tests run on `classic-es`.
