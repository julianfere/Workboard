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
  "permissions": ["items.write", "files.read", "secrets", "tracker"],
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
- `permissions`: what the plugin may ask Workboard (see *Host methods*). Anything else is refused.
- `fields`: extra frontmatter keys the plugin may write with `host/updateItem` (single-line text). The
  workflow's own editable fields are always allowed with `items.write`.
- `presets`: workflow presets (same format as `crates/wb-core/presets/*.json`). They appear in
  **Settings → Workflow** while the plugin is enabled. An id that clashes with a built-in preset is ignored.
- `actions[].placements`: `item` (detail of one item), `items` (bulk bar, one or more selected items),
  `palette` (command palette, no items) and `settings` (a button in the plugin's row in Settings).
- `actions[].when` (optional, only for `item`/`items`): `kind` (`item` or `group`), `hasIds` (boolean),
  `categories` (state categories, e.g. `["active", "review"]`). Every selected item must match.

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
Settings). An `ItemRef` is `{ "name", "title", "ids", "kind", "type", "status", "folder" }`;
`folder` is the absolute path, `name` the path relative to the root (`grp-x/wi1-foo`).

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

## Example

`examples/plugins/hello/` is a minimal plugin in Python (copy the folder into `<config>/plugins/`; on
Windows, if `python` is not on the `PATH`, change `command` to `py`). Plugins written in Rust can use
`wb_plugins::sdk::serve`, which handles the framing and `initialize`; the tests in
`crates/wb-plugins/tests/rpc.rs` and `crates/wb-api/tests/plugins.rs` use it.
