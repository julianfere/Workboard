# CLAUDE.md

Workboard (versión nueva): port a **Rust + Tauri 2** de la app Python (la "v1"), genérico y extensible.
Cada trabajo es una carpeta con un `README.md`; su estado vive en el frontmatter y sus tareas en una
sección de checklist. La v1 sigue en uso y es la referencia de comportamiento.

## Comandos

| Qué | Comando |
|---|---|
| App | `npm install` y `npm run tauri dev` |
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
- `crates/wb-mcp` — MCP `workboard` (stdio, `rmcp`). `lib.rs` tiene la lógica de cada herramienta (se prueba
  sin JSON-RPC) y `main.rs` el transporte. Modos: `WORKBOARD_SESSION` (solo los items de esa sesión),
  `WORKBOARD_MCP_WRITE=1` o solo lectura. Lee `settings.json` (preset, idioma, `root`, `notesFolder`) y
  `tracker_cache.json` de la carpeta de config. Herramientas y respuestas en inglés.
- `crates/wb-i18n` — `t(lang, key, params)` sobre `locales/*.json` (los mismos archivos que usa la UI).
- `crates/wb-platform` — todo lo que depende del SO (`config_dir`, `open_path`, `reveal_path`,
  `kill_tree`). Fuera de este crate no hay `cfg(windows)`.
- `crates/wb-api` — la API de la app: `Api::dispatch(cmd, args)` (un `match` con todos los comandos, cada uno
  un wrapper fino sobre `wb-core`), `settings.json` (idioma, tema, preset, raíz, recientes, carpeta de notas) y
  `ApiError { code, params, conflict }`. Binario `wb-devserver`: la misma API por HTTP (`POST /api/<cmd>`,
  header `X-Workboard: 1`) para desarrollar la UI en el navegador contra el backend real.
- `src-tauri/` — la ventana: un único comando `api(cmd, args)` que llama a `Api::dispatch`, más el plugin
  de diálogo para elegir carpeta. Un comando nuevo se agrega solo en `wb-api` (y su wrapper en `ui/src/api.ts`).
- `ui/` — TypeScript + Vite, sin frameworks. `api.ts`: única puerta al backend (`invoke` en Tauri, `fetch` al
  devserver en el navegador). `app.ts`: carga, navegación, refresco en vivo (cada 2 s compara la firma del
  árbol, como la v1) y registro de acciones (`data-action` / `data-change` / `data-enter`). Una vista por
  archivo en `ui/src/views/` (`render()` + sus acciones). `state.ts`: estado global y helpers del flujo
  (`metaText(item, wf().fields.status)`…: nunca claves del frontmatter escritas a mano). `markdown.ts`:
  renderer propio que escapa todo el HTML. Diálogos propios (`confirmDialog`, `promptDialog`), no los del navegador.
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

Fases: 0 golden ✓ · 1 spike ✓ · 2 esqueleto ✓ · 3 core + MCP ✓ · 4 UI base ✓ · 5 git ·
6 sesiones y agentes · 7 trackers · 8 plugins + plugin privado · 9 multiplataforma, docs y release ·
10 cutover desde la v1. El checklist de paridad está en `docs/parity-checklist.md`.
