# CLAUDE.md

Workboard (versión nueva): port a **Rust + Tauri 2** de la app Python (la "v1"), genérico y extensible.
Cada trabajo es una carpeta con un `README.md`; su estado vive en el frontmatter y sus tareas en una
sección de checklist. La v1 sigue en uso y es la referencia de comportamiento.

## Comandos

| Qué | Comando |
|---|---|
| App | `npm install` y `npm run tauri dev` (compila antes `wb-mcp` y `wb-hook`, que van junto al exe) |
| UI en el navegador contra el backend real | `cargo run -p wb-api --bin wb-devserver -- <carpeta>` + `npm run dev` → `http://localhost:1420` (o `http://127.0.0.1:8766` con la UI compilada) |
| Tests | `cargo test --workspace` |
| Lint (lo mismo que el CI) | `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings` y `npm run build` |
| Golden files | `py tests/golden/gen/gen_all.py` (necesita la v1 en `_v1/` o `WORKBOARD_V1`) |

En Git Bash `cargo` está en `~/.cargo/bin` (agregarlo al PATH si no aparece).
**Al probar, siempre `WORKBOARD_CONFIG_DIR` apuntando a una carpeta temporal**: si no, se pisa la
config real.

## Arquitectura

- `crates/wb-core` — lógica sin UI: `workflow` (presets en `crates/wb-core/presets/*.json`), `frontmatter`
  (parser/escritor propio, decodificación utf-8/BOM/cp1252), `tasks` (sección de tareas), `validate`,
  `text` (slug, secciones, `plan_gaps`), `workspace` (escaneo, rutas seguras), `write` (**`write_readme` es
  la única escritura de README**: reintento por mtime, backup de 20 copias, escritura atómica, `state_log`;
  mover, eliminar, renombrar, versiones), `records` (`runs.json`, bandeja), `notes`. `py.rs` replica la
  semántica de texto de Python de la que depende el formato. Errores: `CoreError { kind, code, params }`.
- `crates/wb-mcp` — MCP `workboard` (stdio, `rmcp`; el transporte va detrás de la feature `server`). `tools.rs`: tools
  promovidas en `<config>/tools/<nombre>/tool.json` (claves de la v1), congeladas por sha256; `run_tool` las corre solo
  dentro de una sesión, en la carpeta del item (300 s). `lib.rs` tiene la lógica de cada herramienta (se prueba
  sin JSON-RPC) y `main.rs` el transporte. Modos: `WORKBOARD_SESSION` (solo los items de esa sesión),
  `WORKBOARD_MCP_WRITE=1` o solo lectura. Lee `settings.json` (preset, idioma, `root`, `notesFolder`) y
  `tracker_cache.json` de la carpeta de config. Herramientas y respuestas en inglés.
- `crates/wb-sessions` — sesiones de agentes: `registry` (`<config>/sessions/<raíz>.json`, lo lee también el MCP),
  `agents` (línea de comando de Claude Code, Copilot CLI y el comando propio; `.cmd` de npm → `node <script>`),
  `pty` (portable-pty/ConPTY, salida agrupada cada 8 ms, búfer de 512 KB para reconectar), `transcript` (peso de la
  sesión y recomendación nueva / traspaso / compactar) y `SessionManager` (ciclo de vida, receptor de hooks HTTP con
  token, contexto `<config>/ctx/<id>/AGENTS.md`, settings de Claude con hooks y permisos, `activity.jsonl`, aviso de
  otro item en el pedido, validación del plan en `Stop`). Textos para el agente: claves `agent.*`. También
  `timelog` (horas por item y día: tiempo activo de `activity.jsonl`, UserPromptSubmit → Stop/Notification/exit, más
  los turnos del transcript anteriores al registro; caché `<config>/time_cache.json`), `stats` (métricas por sesión:
  tokens una vez por `message.id`, herramientas, pedidos, `st: needsYou`, scripts, avance de sus items con backups +
  `state_log.jsonl`; caché `<config>/stats_cache.json`), `candidates` (scripts para promover como tools) y `reflect`
  (evidencia `<config>/reflect/<stamp>.md` y la sesión en modo plan). `WORKBOARD_CLAUDE_PROJECTS` cambia la carpeta de
  transcripts (tests).
- `crates/wb-git` — panel Git **solo lectura**: `cmd::git` rechaza todo subcomando fuera de `READ_ONLY` (`for-each-ref`,
  `rev-list`, `cherry`, `merge-base`, `diff`, `diff-tree`, `log`, `patch-id`, `status`) y siempre usa
  `--no-optional-locks`. `GitPanel::overview` lee cada repo en paralelo (remotes de `.git/config`) y calcula el estado
  de cada rama por entorno (merged / equivalent / content / mentioned / pending) sobre un grafo en memoria
  (`graph::RepoGraph`); lo demás va a git en lote (`diff-tree --stdin`, patch-ids), cacheado por SHA en
  `<config>/git_cache.json`. `promotion` devuelve los commits que faltan en el entorno destino (dudosos destildados,
  códigos `promo.*`); la UI arma los comandos de cherry-pick o merge y nunca se ejecutan. `branch_diff` (para Revisar),
  `web_url` (Azure, GitHub, GitLab), `watch_signature`. Cómo se reconoce un item en ramas y commits, el prefijo de
  promoción y las ramas candidatas por ambiente salen del preset (`Workflow.git`).
- `crates/wb-tracker` — trackers externos (Azure DevOps primero) detrás del trait `Tracker`: estados, campos, work items
  en lotes de 200, asignados a `@Me` (POST WIQL de solo lectura), detalle y `set_state` (única escritura: PATCH de
  `System.State`). El HTTP pasa por `Transport` (`UreqTransport`: ureq + rustls con certificados del sistema; los tests
  usan respuestas fijas). El cliente saca el token de todo error. Heurísticas de campos (fecha y horas pactadas) y
  `html_to_text` en `azure.rs`.
- `crates/wb-hook` — el hook del CLI (`wb-hook <Evento>`): postea a la app y reimprime la respuesta. Sin dependencias.
- `crates/wb-i18n` — `t(lang, key, params)` sobre `locales/*.json` (los mismos archivos que usa la UI).
- `crates/wb-platform` — todo lo que depende del SO (`config_dir`, `open_path`, `reveal_path`, `open_url` solo https,
  `kill_tree`, `which`, `script_command` + `run_captured` con timeout, `downloads_dir`, `hide_console`) y `secrets`
  (almacén de credenciales del sistema con `keyring`; `WORKBOARD_SECRET_SERVICE` cambia el servicio). Los tokens
  nunca van a archivos. Fuera de este crate no hay `cfg(windows)`.
- `crates/wb-api` — la API de la app: `Api::dispatch(cmd, args)` (un `match` con todos los comandos, cada uno
  un wrapper fino sobre `wb-core`), `settings.json` (idioma, tema, preset, raíz, recientes, carpeta de notas) y
  `ApiError { code, params, conflict }`. `sessions.rs`: comandos `session_*`, `plan_item`, agentes; el
  `SessionManager` se levanta a pedido y busca `wb-mcp`/`wb-hook` junto al exe (o `WORKBOARD_MCP_BIN`/`WORKBOARD_HOOK_BIN`). `git.rs`: comandos `git_*` y `review_item` (Revisar: paquete `<config>/review/<stamp>-<id>/REVIEW.md` + un `.diff` por rama, sesión en modo plan con `--model`); repos de cada carpeta de trabajos en
  `settings.json` → `git.repos[<raíz>]` (sin entrada = repos hermanos detectados) y ramas candidatas en `git.envRefs`.
  `tracker.rs`: comandos `tracker_*`; `<config>/trackers.json` (conexión por raíz; estados, mapeo y campos por
  proyecto) y `<config>/tracker_cache.json` (última lectura por raíz, la lee el MCP); vista previa del sync en Rust
  (completar / tipo / avanza / retrocede; retroceder = categoría menor o antes en el preset), aplicar escribe solo
  estado y tipo; la fila «según git» (`git::env_reach`) propone el estado de `Workflow.git.envStates` cuando el trabajo ya está en ese ambiente en todos los repos del item. `stats.rs`: `time_report`, `stats_report`, `tool_*`, `reflect_items`, `save_export`/`reveal_export`
  (CSV a Descargas con BOM). Los `*_command` se encadenan en el fallback de `dispatch`. Binario `wb-devserver`: la misma API por HTTP (`POST /api/<cmd>`,
  header `X-Workboard: 1`) para desarrollar la UI en el navegador contra el backend real; `--fake-tracker <archivo.json>`
  responde las llamadas al tracker sin red.
- `src-tauri/` — la ventana: un único comando `api(cmd, args)` (async: un overview de git lento no congela la ventana) que llama a `Api::dispatch`, `session_attach`
  (salida de una terminal por un `Channel` de bytes) y el plugin de diálogo para elegir carpeta. Cerrar la
  ventana mata las terminales. Un comando nuevo se agrega solo en `wb-api` (y su wrapper en `ui/src/api.ts`).
- `ui/` — TypeScript + Vite, sin frameworks. `api.ts`: única puerta al backend (`invoke` en Tauri, `fetch` al
  devserver en el navegador). `app.ts`: carga, navegación, refresco en vivo (cada 2 s compara la firma del
  árbol, como la v1) y registro de acciones (`data-action` / `data-change` / `data-enter`). Una vista por
  archivo en `ui/src/views/` (`render()` + sus acciones). `state.ts`: estado global y helpers del flujo
  (`metaText(item, wf().fields.status)`…: nunca claves del frontmatter escritas a mano). `markdown.ts`:
  renderer propio que escapa todo el HTML. Diálogos propios (`confirmDialog`, `promptDialog`), no los del navegador;
  el clic en el fondo de un modal se maneja con un bloque (`{ if (e.target === back) close(); }`): devolver `false`
  cancela los clics en checkboxes. Por feature: `sessions.ts` + `terminal.ts` (xterm), `git.ts` (pestaña Git,
  promoción, alertas, sondea `git_signature` cada 3 s), `views/tracker.ts`, `views/hours.ts`, `views/metrics.ts`,
  `views/tools.ts` (datos compartidos en `stats.ts`; horas y fecha pactadas: las del README y, si faltan, las del tracker) y `reflect.ts`.
- `tests/golden/` — comportamiento de la v1 capturado como JSON (ver su README y `KNOWN_ISSUES.md`).
- `spike/` — experimentos descartables (terminal, MCP con `rmcp`, hook). No son parte del workspace.
- `_v1/` — copia local de la v1 (ignorada por git, no se publica).

## Convenciones (no romper)

- **Nada del flujo de trabajo en el código**: estados, tipos, ambientes, período, nombres de carpeta,
  claves del frontmatter, títulos de sección y categorías de archivo salen del `Workflow`. Si hace falta
  un dato nuevo, se agrega al preset (y a los dos presets incluidos).
- **Paridad con la v1 por golden files**: cada módulo portado lee los casos de `tests/golden/cases/` y
  los reproduce con el preset `classic-es`. Una diferencia buscada se documenta en `KNOWN_ISSUES.md`.
- **Textos**: todo texto visible va en `locales/en.json` y `locales/es.json` (un test exige las mismas
  claves y parámetros). El backend nunca arma texto para la UI: devuelve códigos.
- **Colores**: solo variables de `ui/src/styles.css` (claro y oscuro). Iconos con `ico()` de
  `ui/src/icons.ts`; sin emojis ni glyphs como iconos.
- UI offline: sin CDNs ni recursos remotos.
- Commits convencionales (`feat(core): …`, `test(golden): …`), uno por paso lógico, en inglés.

## Restricciones heredadas de la v1

- Dentro de la carpeta de trabajos la app solo escribe el frontmatter y las líneas de tareas del
  README, más crear/mover/renombrar carpetas a pedido (nunca borra contenido: eliminar = mover a la
  carpeta de eliminados). Todo lo demás va a la carpeta de config.
- Git: solo comandos de lectura. La promoción **genera** comandos, no los corre.
- Trackers (Azure, etc.): lectura salvo acciones de escritura explícitas del usuario, con credenciales
  aparte. Los secretos nunca van a logs ni a mensajes de error.

## Plan

Fases: 0 golden ✓ · 1 spike ✓ · 2 esqueleto ✓ · 3 core + MCP ✓ · 4 UI base ✓ · 5 git ✓ ·
6 sesiones y agentes ✓ · 7 trackers ✓ · 8 plugins + plugin privado · 9 multiplataforma, docs y release ·
10 cutover desde la v1. El checklist de paridad está en `docs/parity-checklist.md`.
