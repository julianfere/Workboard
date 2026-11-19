# CLAUDE.md

Workboard (versión nueva): port a **Rust + Tauri 2** de la app Python (la "v1"), genérico y extensible.
Cada trabajo es una carpeta con un `README.md`; su estado vive en el frontmatter y sus tareas en una
sección de checklist. La v1 sigue en uso y es la referencia de comportamiento.

## Comandos

| Qué | Comando |
|---|---|
| App | `npm install` y `npm run tauri dev` |
| Solo UI en el navegador (backend mock) | `npm run dev` → `http://localhost:1420` (Playwright) |
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
- `src-tauri/` — la app. Cada `#[tauri::command]` es un wrapper fino sobre los crates y devuelve
  `ApiResult<T>`; los errores son `ApiError { code, params }` (code = clave de traducción).
- `ui/` — TypeScript + Vite, sin frameworks. `ui/src/api.ts` es la única puerta al backend: dentro de
  Tauri usa `invoke`, en el navegador un mock con los mismos presets (agregar cada comando nuevo a los dos).
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

Fases: 0 golden ✓ · 1 spike ✓ · 2 esqueleto ✓ · 3 core + MCP ✓ · 4 UI base · 5 git ·
6 sesiones y agentes · 7 trackers · 8 plugins + plugin privado · 9 multiplataforma, docs y release ·
10 cutover desde la v1. El checklist de paridad está en `docs/parity-checklist.md`.
