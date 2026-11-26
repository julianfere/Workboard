# Golden files

Comportamiento de referencia de la v1 (Python), capturado como datos para que el port en Rust lo
reproduzca. Cada caso es entrada → salida esperada (o error esperado). Los tests de Rust leen estos JSON y
comparan; no ejecutan Python.

```
gen/              generadores (Python, importan la v1 desde WORKBOARD_V1 o ../workboard-v1)
fixtures/work/    una carpeta _work ficticia (se versiona tal cual: CRLF, BOM y cp1252 a propósito)
fixtures/repos/   app.fi: repo git sintético en formato `git fast-import` (SHAs deterministas)
cases/*.json      los casos
KNOWN_ISSUES.md   bugs de la v1 que los golden registran tal cual
```

Regenerar: `py tests/golden/gen/gen_all.py`. Es reproducible: dos corridas dan los mismos bytes. Si un
cambio de la v1 cambia un golden, se revisa el diff antes de commitear.

## Casos

| Archivo | Qué cubre |
|---|---|
| `frontmatter.json` | `split_frontmatter`, `join_frontmatter`, ida y vuelta, `read_text` (utf-8, BOM, cp1252, inválido) |
| `tasks.json` | `## Pendientes`: parse, add, set, remove, edit (con `expect`), close, clear_done, `clean_task` |
| `validate.json` | `validate_fields`: cada campo editable, normalización y mensajes de error |
| `misc.json` | categorías de archivo, períodos 20→19, horas, slugify, regex de carpetas de WI, tipos de Azure, `html_to_text`, `extract_section`, `plan_gaps`, constantes |
| `workspace_scan.json` | `list_items` sobre `fixtures/work`, `list_deleted`, `folder_of`, `safe_path`, `read_file`, `read_media`, `get_meta` |
| `workspace_ops.json` | 40 operaciones de escritura en secuencia (árbol y README después de cada una), versiones, `state_log`, rotación de backups |
| `git.json` | `overview` (estado por entorno), `promotion` a QA y PROD, `branch_diff`, `detect_repos`, `wi_nums`, trailer de cherry-pick, `remote_web_url` |
| `mcp.json` | cada herramienta del MCP en modo solo lectura, escritura global y sesión, con sus efectos (README, eventos, runs, notas) e `instructions` |

## Convenciones para quien los consume

- **Errores**: `{"error": "<TipoDeExcepción>", "message": "..."}`. El tipo es contrato; el mensaje es la
  referencia en español (con i18n puede cambiar de forma, no de sentido).
- **Tokens**: `<ROOT>` = raíz `_work` (o la carpeta temporal del caso), `<CONFIG>` = carpeta de config,
  `<TODAY>` = fecha del día (`AAAA-MM-DD`), `<HH:MM>` = hora.
- **Campos fuera**: `created`, `last`, `mtime`, `deleted`, `ts`, `computedAt`, `seconds`, `lastFetch`
  dependen del reloj y se sacan.
- **Orden por tiempo**: lo que la v1 ordena por mtime (`list_wis`: más reciente primero; archivos de
  `get_wi`: más nuevo primero; `list_deleted`: último eliminado primero) en el golden va **ordenado por
  ruta**. El test tiene que ordenar igual antes de comparar, y probar el orden por tiempo aparte.
- **Operaciones** (`workspace_ops.json`): corren en orden sobre una copia de `fixtures/work`, con la
  carpeta de backups en `<config>/backups`. Cada paso depende de los anteriores.
- **Repo git**: recrearlo como dice el docstring de `gen/build_repo.py` (init, fast-import, remote
  `origin`, checkout de `Feature/111111-login` y un archivo sin trackear). Entornos: DEV =
  `origin/development`, QA = `origin/qa`, PROD = `origin/main`.
- **MCP**: las llamadas de cada escenario corren en orden sobre una copia nueva de `fixtures/work`, con
  `ado_cache.json` = `ado_cache_items` y, en modo sesión, el registro de `session_id` con los WI
  `wi111111-login-con-sso` y `grp-checkout`.

## Qué no cubren todavía

Sesiones (`sessions.py`: hooks, registro, `recommend`, `other_wis`), `timelog.py`, `sessionstats.py`,
`harness.py` (argv y contexto por agente), `notes.py` fuera de `add_note`, `tools.py`, Azure (`ado.py`) y
la UI. Se agregan en la fase que porta cada módulo.
