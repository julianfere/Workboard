# Plugins

A plugin adds **actions** (a button in an item's detail, in the bulk bar of the item list, in the command
palette or in Settings) and **workflow presets** to Workboard. It is a separate executable, written in any
language, that talks JSON-RPC 2.0 with Workboard over stdin/stdout.

Plugins are code you run as your own user. Workboard does not sandbox them: the permissions in the manifest
only limit what the plugin can ask *Workboard* to do (write an item's README, read its files, keep secrets).
A plugin is disabled until you enable it in **Settings → Plugins**, where its permissions are listed.

## Installing

Each plugin is a folder in `<config>/plugins/` (Windows: `%APPDATA%\Workboard2\plugins\`) with a
`manifest.json`. **Settings → Plugins** lists them, enables or disables each one and opens the folder.
Which plugins are enabled is stored in `<config>/plugins.json`.

## Manifest

```json
{
  "id": "example",
  "name": { "en": "Example", "es": "Ejemplo" },
  "version": "0.1.0",
  "protocol": 1,
  "command": "bin/example",
  "args": [],
  "timeoutSeconds": 120,
  "permissions": ["items.write", "files.read", "secrets", "tracker", "tracker.write"],
  "fields": ["release_manual"],
  "presets": ["presets/my-flow.json"],
  "actions": [
    {
      "id": "hello",
      "label": { "en": "Say hello", "es": "Saludar" },
      "hint": { "en": "Shows a form", "es": "Muestra un formulario" },
      "icon": "sparkle",
      "placements": ["item", "items", "palette", "settings"],
      "when": { "kind": "item", "hasIds": true }
    }
  ]
}
```

- `id`: lowercase letters, digits and `-`, unique. It namespaces the plugin's secrets and data.
- `name`, `label`, `hint`: a string or `{ "en": …, "es": … }`.
- `command`: path relative to the plugin folder (on Windows `.exe` is added when missing), or a program on
  the `PATH` (`python`, `node`…) followed by `args`. It runs with the plugin folder as working directory. Workboard sets `WORKBOARD_PLUGIN_PROTOCOL` (the protocol version) in its environment.
- `timeoutSeconds`: per call, 5 to 600 (default 120). Past it the process tree is killed.
- `permissions`: what the plugin may ask Workboard (see *Host methods*): `items.write`, `files.read`,
  `secrets`, `tracker` (the connection, never a token) and `tracker.write` (the tracker's write token).
  Anything else is refused; an unknown permission makes the manifest invalid, so a plugin that asks for
  `tracker.write` needs a Workboard that knows it.
- `fields`: extra frontmatter keys the plugin may write with `host/updateItem` (single-line text). The
  workflow's own editable fields are always allowed with `items.write`.
- `presets`: workflow presets (same format as `crates/wb-core/presets/*.json`). They appear in
  **Settings → Workflow** while the plugin is enabled. An id that clashes with a built-in preset is ignored.
- `actions[].placements`: `item` (detail of one item), `items` (bulk bar, one or more selected items),
  `palette` (command palette, no items) and `settings` (a button in the plugin's row in Settings).
  The grid's context menu (right click on a row, or «More» in the bulk bar) lists `items` actions too and,
  when a single row is selected, also its `item` actions (once each); `context.placement` says which one ran.
- `actions[].when` (optional, only for `item`/`items`): `kind` (`item` or `group`), `hasIds` (boolean),
  `categories` (state categories, e.g. `["active", "review"]`). Every selected item must match. State
  conditions go through categories so they follow the active preset: `["planned"]` is the plan state in
  both built-in presets.

## Protocol

Messages are JSON-RPC 2.0 objects, **one per line** (UTF-8, `\n`-terminated), on stdin/stdout. The plugin
must not write anything else to stdout; stderr is captured into Workboard's log (truncated).

Every user step is one short-lived process:

1. Workboard starts the plugin and sends `initialize`.
2. It sends one call: `action/open` or `action/submit`.
3. While handling it the plugin may send its own requests to Workboard (`host/…`) and wait for the answers.
4. When the plugin answers the call, Workboard closes stdin. The plugin should exit; after 2 seconds it is
   killed.

So a plugin keeps no memory between steps: what it needs later goes in the step's `state`, which Workboard
sends back untouched, or in its data (`host/data/set`).

### `initialize`

```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocol":1,"host":{"name":"workboard","version":"0.1.0"},"lang":"es","plugin":{"id":"example","dir":"C:\\…\\plugins\\example"}}}
```

Answer with any object (`{}`). If `protocol` is newer than the plugin understands, answer an error.

### `action/open` and `action/submit`

```json
{"method":"action/open","params":{"action":"hello","context":{…}}}
{"method":"action/submit","params":{"action":"hello","context":{…},"button":"save","values":{"name":"Ana"},"state":{…}}}
```

`context` is `{ "lang", "root", "placement", "items": [ItemRef…] }`: where the action was started from
(`item`, `items`, `palette` or `settings`) and the items it was started on (none from the palette or
Settings). An `ItemRef` is `{ "name", "title", "ids", "kind", "type", "status", "folder", "sessions" }`;
`folder` is the absolute path, `name` the path relative to the root (`grp-x/wi1-foo`) and `sessions` how
many agent sessions that are not closed include the item (for example, to ask before opening another one
with a button's `confirm`). Older hosts don't send `sessions`: treat a missing value as 0.

`action/submit` is sent when the user presses one of the form's buttons, or changes a field marked
`refresh` (then `button` is `"$refresh"`). `values` has every field of the form (`password` fields only when
typed; checkboxes as booleans; numbers as numbers).

Both answer a **step**, one of:

**Form**

```json
{
  "type": "form",
  "title": "New release",
  "intro": "Creates the work item and the branch.",
  "error": "optional message shown above the fields",
  "fields": [
    { "id": "title", "type": "text", "label": "Title", "value": "", "required": true, "wide": true },
    { "id": "flow", "type": "select", "label": "Flow", "value": "bug", "refresh": true,
      "options": [{ "value": "feature", "label": "Feature" }, { "value": "bug", "label": "Bug" }] },
    { "id": "notes", "type": "textarea", "label": "Notes", "rows": 6 },
    { "id": "token", "type": "password", "label": "Token", "placeholder": "saved (leave empty to keep it)" },
    { "id": "link", "type": "checkbox", "label": "Link the branch", "value": true },
    { "id": "n", "type": "number", "label": "Count", "value": 3 },
    { "id": "help", "type": "info", "text": "Plain text shown as a paragraph." }
  ],
  "buttons": [
    { "id": "more", "label": "More fields", "validate": false },
    { "id": "create", "label": "Create", "primary": true, "confirm": "This writes to Azure DevOps. Continue?" }
  ],
  "state": { "anything": "the plugin needs back" }
}
```

Every field may have `hint`, `placeholder` and `wide`. `required` is checked by Workboard before submitting
unless the button has `"validate": false`. `confirm` asks before submitting. A Cancel button is always added.

**Result**

```json
{
  "type": "result",
  "title": "Release #123 created",
  "ok": false,
  "message": "The work item exists: Retry completes the missing steps.",
  "steps": [{ "label": "Work item #123", "ok": true }, { "label": "Branch", "ok": false, "error": "…" }],
  "links": [{ "label": "Open work item", "url": "https://…" }],
  "buttons": [{ "id": "retry", "label": "Retry" }],
  "state": { "id": 123 }
}
```

Links must be `https://`. Buttons submit again with `state`.

**Session**: Workboard opens an agent session (default agent) and shows its terminal.

```json
{ "type": "session", "items": ["wi123-foo"], "name": "Implement 123", "mode": "acceptEdits", "prompt": "…", "model": null }
```

`mode` is `default`, `plan` or `acceptEdits`. `items` must exist in the open folder.

**Close**: `{ "type": "close", "message": "Saved" }` closes the dialog and shows the message.

## Host methods

Requests the plugin sends to Workboard while handling a call. Errors come back as JSON-RPC errors whose
`message` is a code (`plugin.denied`, `plugin.badArgument`, `core.…`).

| Method | Permission | Params → result |
|---|---|---|
| `host/workflow` | — | `{}` → the active workflow (preset JSON) |
| `host/items` | — | `{}` → `ItemRef[]` of the open folder |
| `host/item` | — | `{name}` → `ItemRef` + `meta` (frontmatter) + `files` (`[{path, name, kind, category, size}]`, `path` relative to the root) |
| `host/updateItem` | `items.write` | `{name, fields: {key: value}}` → `ItemRef`. Keys: the workflow's editable fields (validated like the UI) or the manifest's `fields`. `""` deletes. Goes through the same README writer as the app (backup, retry). |
| `host/readFile` | `files.read` | `{path}` (relative to the root, inside an item folder) → `{text}`; at most 2 MB |
| `host/secret/get` | `secrets` | `{key}` → `{value}` or `{value: null}` |
| `host/secret/set` | `secrets` | `{key, value}` → `{}`; stored in the system credential store, namespaced by plugin id |
| `host/secret/delete` | `secrets` | `{key}` → `{}` |
| `host/data/get` | — | `{}` → the plugin's JSON data (`{}` if none) |
| `host/data/set` | — | `{data}` → `{}`; at most 1 MB, in `<config>/plugin-data/<id>.json` |
| `host/tracker` | `tracker` | `{}` → `{kind, source, org, project}` of the open folder's tracker connection, or `null`. Never a token. |
| `host/tracker/writeToken/get` | `tracker.write` | `{}` → `{value}` or `{value: null}`: the write token of the open folder's tracker organization, the same one **Change state in the tracker** uses (one per organization, shared by the app and every plugin). Error `tracker.notConnected` without a connection. |
| `host/tracker/writeToken/set` | `tracker.write` | `{value}` (not empty) → `{}`; replaces that shared token. |
| `host/tracker/writeToken/delete` | `tracker.write` | `{}` → `{}`; removes it (the app then asks for it again). |

## Trackers

A plugin can also **be a tracker**: Jira, Trello, GitHub Issues… Workboard then treats it exactly like the
built-in Azure DevOps one: connect a work folder to it in **Settings → Tracker**, sync states and types,
agreed date and hours, the tracker column, the MCP cache and **Change state in the tracker** all work the
same. Only Azure DevOps is built in (`kind: "azure"`).

### Manifest

```json
"trackers": [
  {
    "kind": "trello",
    "name": "Trello",
    "org":     { "label": { "en": "Workspace", "es": "Espacio" }, "placeholder": "my-team", "required": false },
    "project": { "label": { "en": "Board", "es": "Tablero" }, "placeholder": "board id or URL" },
    "secret":  { "label": { "en": "API key:token", "es": "API key:token" }, "hint": { "en": "…", "es": "…" } },
    "writeSecret": false,
    "itemUrl": "https://trello.com/c/{id}",
    "idPattern": "^[0-9]+$"
  }
]
```

- `kind`: lowercase letters, digits and `-`, unique among the enabled plugins and different from the
  built-in ones (`azure`). It is what the folder's connection stores, so renaming it disconnects folders.
- `org` and `project`: the two connection fields (labels and placeholders). `org.required: false` hides
  nothing but lets it be empty; `project` is always required. Workboard only trims them; the plugin
  validates and normalizes them in `tracker/connect`.
- `secret`: label and hint of the credential field. `writeSecret: true` means writes need a separate
  credential (like Azure's write PAT); otherwise `tracker/setState` gets the read credential.
- `itemUrl`: page of an item; `{org}`, `{project}` and `{id}` are replaced (URL-encoded). Must be `https://`.
- `idPattern`: which ids of the work folder belong to this tracker (a regex on the id, default any id). Ids
  that don't match are skipped instead of sent.

Declaring `trackers` needs no extra permission; **Settings → Plugins** lists the trackers a plugin adds.
A connection whose plugin is disabled or missing keeps its configuration and cache; every call answers the
error `tracker.pluginMissing` until the plugin is back.

### Methods

Same framing as actions (one short-lived process per call, `initialize` first). Every call carries the
connection and the credential:

```json
{"method":"tracker/states","params":{"kind":"trello","connection":{"org":"…","project":"…"},"secret":"…"}}
```

The credential travels only on the plugin's stdin; Workboard removes it from any error text the plugin
returns. The plugin must not log it or write it anywhere.

| Method | Extra params → result |
|---|---|
| `tracker/connect` | — → `{ "org", "project" }` normalized (e.g. the board id from its URL). Called when the user connects; an error here rejects the connection. |
| `tracker/states` | — → `[{ name, color, category, types, suggested }]`: `color` hex without `#` or `""`; `category` the tracker's own; `types` the item types that use it; `suggested` one of `backlog`, `planned`, `active`, `review`, `done`, `cancelled` or `null` (seeds the state mapping and tells closed states apart). |
| `tracker/fields` | — → `[{ ref, name, type, role }]`: `type` is `date`, `number` or anything else; `role` (optional) is `due`, `hours`, `priority` or `effort` and preselects the field for that use. |
| `tracker/workItems` | `{ ids, request: { priority, due, hours, effort, text, extra } }` → `{ "<id>": { state, title, type, assigned, changed, priority, due, estimate, completed, description, criteria, fields } }`. `due`/`hours` are the refs chosen in Settings (or `null`); `text` asks for `description` and `criteria` (Markdown); `extra` lists refs to return as-is in `fields`. Missing or invisible ids are left out. |
| `tracker/assignedToMe` | `{ dueField, start, end, exclude }` → ids assigned to the credential's owner with `dueField` in `[start, end)` (ISO dates) and not in the `exclude` states, ordered by that date. |
| `tracker/detail` | `{ id }` → `{ id, title, type, state, description, criteria }` (Markdown), to create the item's folder. |
| `tracker/setState` | `{ id, state }` → `{ state, changed }`. **Writes**: only called when the user asks for it. |

Errors are JSON-RPC errors whose `message` is shown to the user (after removing the credential). Use
`tracker.auth` for a rejected credential and `tracker.notFound` for a missing project so the app can say
which; anything else is shown as is.

## SDKs

`sdk/python/workboard_plugin.py` (Python 3.8+) and `sdk/node/workboard-plugin.mjs` (Node 18+) are single
files with no dependencies: copy the one you need next to your plugin. They handle the framing,
`initialize`, errors and host requests, so a plugin is only its handlers:

```python
from workboard_plugin import Plugin, form, text, result

plugin = Plugin()

@plugin.action("greet")
def greet(ctx):                      # action/open
    return form("Hello", [text("name", "Your name", required=True)], buttons=[("greet", "Greet")])

@greet.submit
def greet_submit(ctx, button, values, state):
    ctx.host.update_item(ctx.items[0]["name"], {"notes": f"greeted {values['name']}"})
    return result("Hello", f"Hello, {values['name']}!")

@plugin.tracker("trello")
class Trello:
    def connect(self, conn, secret): ...
    def states(self, conn, secret): ...
    def fields(self, conn, secret): ...
    def work_items(self, conn, secret, ids, request): ...
    def assigned_to_me(self, conn, secret, due_field, start, end, exclude): ...
    def detail(self, conn, secret, id): ...
    def set_state(self, conn, secret, id, state): ...

plugin.run()
```

`workboard plugin new <id> [--lang python|node] [--tracker]` creates a ready-to-enable plugin folder in
`<config>/plugins/<id>/` with the manifest, the SDK file and a starting handler; `workboard plugin check
<folder>` validates the manifest and runs `initialize` plus a harmless call against it.

## Example

`examples/plugins/hello/` is a minimal plugin in Python (copy the folder into `<config>/plugins/`; on
Windows, if `python` is not on the `PATH`, change `command` to `py`). `examples/plugins/trello/` is a tracker plugin
(Trello boards) built on the Python SDK. Plugins written in Rust can use
`wb_plugins::sdk::serve`, which handles the framing and `initialize`; the tests in
`crates/wb-plugins/tests/rpc.rs` and `crates/wb-api/tests/plugins.rs` use it.
