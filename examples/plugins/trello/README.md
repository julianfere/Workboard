# Trello tracker plugin (example)

Connects a Workboard work folder to a Trello board as a **tracker** (see `docs/plugins.md`, "Trackers").
Python 3.8+, standard library only. Files: `manifest.json`, `plugin.py` (thin entry on the SDK),
`trello_api.py` (all the Trello logic, testable without the SDK or network), `workboard_plugin.py` (copy of
`sdk/python/workboard_plugin.py`) and `tests/`.

> **Not tested against the real Trello API.** It follows the Trello REST API v1 documentation
> (https://developer.atlassian.com/cloud/trello/rest/), but it was developed without credentials: the
> tests use a fake API. Please report what differs.

## Install

Copy this folder into `<config>/plugins/trello/` (Windows: `%APPDATA%\Workboard2\plugins\trello\`), enable it in
**Settings → Plugins** and connect the work folder in **Settings → Tracker**. On Windows, if `python` is not on
the `PATH`, change `command` to `py` in `manifest.json`.

- **Board**: its id, short link (`AbCd1234`) or URL (`https://trello.com/b/AbCd1234/name`). Stored as the short link.
- **Workspace** (optional): name or URL; if given, the board must belong to it.
- **Credential**: `API key:token`. Get the key from a Power-Up at https://trello.com/power-ups/admin and a
  token at `https://trello.com/1/authorize?expiration=never&scope=read,write&response_type=token&key=<key>`.
  It is sent in the `Authorization: OAuth …` header (never in the URL) and removed from every error text.

## Mapping

| Workboard | Trello |
|---|---|
| work item id | the card's board-local number `idShort` (folders like `123-fix-login`; `idPattern` `^[0-9]+$`) |
| states | the board's open lists, in board order; `category` is `list`, `types` empty |
| suggested category | guessed from the list name (case and accents ignored): backlog/to do/por hacer → `backlog`; doing/in progress/en curso → `active`; review/QA/testing/revisión → `review`; done/hecho/terminado → `done`; cancelled/cancelado → `cancelled`; else `null` |
| type | name of the card's first named label, or `""` (labels are not listed as types) |
| title, description | card name, card description (already Markdown) |
| criteria | the card's checklists as a Markdown checklist (`- [x] …`), a bold title per checklist when there are several |
| assigned | full names of the card members, comma separated |
| changed | `dateLastActivity` |
| due | card `due` (ISO, as Trello gives it), or a date custom field when chosen |
| hours (estimate) | the number custom field chosen as hours field; `completed` and `priority` are always `null` |
| fields | `due` (date, role `due`) plus the board's custom fields (`number`, `date`, `text`, `checkbox`, `list`; ref = custom field id). A number field named like hours/horas/estimate gets role `hours` |
| assigned to me | open cards with the token's member, due date (day part, UTC) in `[start, end)`, list not excluded, ordered by due date |
| change state | moves the card to the open list with that exact name (error if none; first one if repeated) |

Each call fetches the board's cards once (`/boards/{id}/cards/all`, archived cards included), not one request
per id. The item link (`itemUrl`) opens the board: Trello card URLs use the card's own short link, which is not
the id Workboard keeps.

Errors: rejected key/token → `tracker.auth`; missing board, workspace or card → `tracker.notFound`; anything
else is shown as a message.

## Tests

```
py -m unittest discover -s examples/plugins/trello/tests
```

`tests/fake_trello.py` is the fake API. `tests/run_plugin.py` runs the real protocol loop of `plugin.py` with that
fake (it imports `plugin` and replaces its `HTTP`); production runs `plugin.py` directly, so nothing in the
environment can swap the transport.

---

## Resumen (es)

Plugin de tracker para Trello. **No se probó contra la API real** (no había credenciales): sigue la
documentación de la API REST v1 y los tests usan una API falsa.

- Id del trabajo = número de la tarjeta en el tablero (`idShort`). Estados = listas abiertas del tablero (la
  categoría sugerida sale del nombre: backlog/por hacer, en curso, revisión, hecho, cancelado). Tipo = primera
  etiqueta con nombre. Criterios = checklists en Markdown. Fecha pactada = `due` de la tarjeta o un campo
  personalizado de fecha; horas = un campo personalizado numérico.
- Cambiar estado mueve la tarjeta a la lista abierta con ese nombre.
- Credencial `API key:token` (https://trello.com/power-ups/admin y la URL de autorización de arriba); viaja en el
  header `Authorization`, nunca en la URL, y se borra de todo mensaje de error.
- El link del trabajo abre el tablero (la URL de una tarjeta usa su propio short link, no el número).
