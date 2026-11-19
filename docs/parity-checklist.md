# Checklist de paridad con la v1

Todo lo que hace la v1 (Python + pywebview) y tiene que hacer la nueva versión antes del cutover. `[x]` = hecho,
`[~]` = hecho en parte (ver notas de la fase). Sale de
las secciones del README de la v1. **Fase** = la del plan en que se porta. **Golden** = archivo de
`tests/golden/cases/` que fija el comportamiento (— = se prueba a mano o con Playwright).

Lo que es propio de un flujo de trabajo concreto (ReleaseManual, ramas de entorno propias, el flujo
Feature/Bug de Implementar) va al plugin privado y se marca **(privado)**.

## Datos y archivos (core)

| | Función | Fase | Golden |
|---|---|---|---|
| [x] | Escaneo de la raíz en cada llamada (sin caché); `_` y `.` se ignoran | 3 | workspace_scan |
| [x] | Frontmatter propio (`k: v`, `k: [a, b]`), comillas al escribir, campos vacíos no se escriben | 3 | frontmatter |
| [x] | Lectura utf-8 / BOM / cp1252; escritura siempre UTF-8 + LF | 3 | frontmatter, workspace_ops |
| [x] | WI por nombre `wiNNNNNN[-NNNNNN]-slug` o campo `wi`; duplicados entre carpetas | 3 | misc, workspace_scan |
| [x] | Grupos `grp-*` (un nivel, README propio, lo compartido en la carpeta del grupo) | 3 | workspace_scan |
| [x] | Escritura única de README: chequeo de mtime + reintento, backup (20), escritura atómica | 3 | workspace_ops |
| [x] | Versiones anteriores del README: listar, ver, restaurar | 3 | workspace_ops |
| [x] | `state_log.jsonl` con cada cambio de `estado` | 3 | workspace_ops |
| [x] | `## Pendientes`: agregar, tildar, editar, borrar (con `expect`), limpiar hechas; nada más de la sección | 3 | tasks, workspace_ops |
| [x] | Pasar a PROD / descartado tilda los pendientes abiertos | 3 | workspace_ops |
| [x] | Validación de campos editables (estado, tipo, ambientes, wi, pactada, periodo, horas, textos) | 3 | validate |
| [x] | Períodos configurables (hoy del 20 al 19) | 3 | misc |
| [x] | Categorías de archivo por prefijo/extensión; scripts "corridos en DEV/QA/PROD" (`runs.json`) | 3 | misc, mcp |
| [x] | Mover a grupo / sacar / disolver, eliminar a `_deleted` con deshacer, restaurar, renombrar carpeta | 3 | workspace_ops |
| [~] | Después de mover: sesiones, marcas, backups y notas siguen a la carpeta (falta notas) | 3 / 6 | parcial (backups) |
| [x] | Nuevo item con o sin WI (crea carpeta + README; con id trae título, tipo, descripción y criterios del tracker) | 3 / 7 | workspace_ops (create_folder), wb-api/tests/tracker |
| [x] | Plan incompleto (`plan_gaps`) | 3 | misc |
| [x] | Bandeja de tareas sin WI (`inbox.md` en la config) | 3 | tasks (mismas funciones) |
| [~] | Watcher: refresco en vivo del filesystem y de `.git` de los repos | 4 | — |

## MCP `workboard`

| | Función | Fase | Golden |
|---|---|---|---|
| [x] | Modos: sesión (solo sus WI, grupos incluyen sus WI), escritura global, solo lectura | 3 | mcp |
| [x] | `session`, `list_wis`, `get_wi`, `set_estado`, `add_task`, `complete_task`, `marcar_ejecutado`, `add_note`, `list_tools`, `run_tool` | 3 | mcp |
| [x] | `instructions` con la guía + contexto de la sesión | 3 | mcp |
| [x] | Eventos por sesión (`<id>.events.jsonl`) o globales | 3 | mcp |
| [x] | Planificación: prioridad, pactada, período y horas (local pisa a Azure) | 3 | mcp |
| [x] | Nombres de herramientas y respuestas en inglés (sin alias: las sesiones de la v1 usan el MCP de la v1) | 3 | mcp |

## UI

| | Función | Fase | Golden |
|---|---|---|---|
| [~] | Vistas: Hoy, Trabajos, Pendientes, Horas, Alertas, Notas, Métricas, Tools, Ajustes | 4 | — |
| [~] | Paleta `Ctrl+K` (buscar trabajos, tareas, archivos; comandos; acciones del trabajo de arriba) | 4 | — |
| [ ] | Atajos (`Ctrl+J`, `/`, `1`…`6`, `Ctrl+S`, `Esc`, `F5`, `Supr`, `Alt+1`…`9`) | 4 | — |
| [ ] | Selección múltiple, barra y menú contextual en bulk; deshacer de 12 s | 4 | — |
| [ ] | Grilla: columnas configurables, orden, ancho, agrupar hasta 3 niveles, filtros con chips, densidad | 4 | — |
| [ ] | Vistas guardadas (pestañas) con Guardar / Descartar | 4 | — |
| [ ] | Exportar CSV (`;`, BOM) | 4 | — |
| [~] | Detalle a página completa: pasos de estado, guardado automático solo de lo tocado, conflicto con disco | 4 | — |
| [~] | Pestañas del detalle: Resumen, README, Archivos, Git, Cómo deployar, Notas | 4 | — |
| [ ] | Visor: SQL coloreado, Markdown, imágenes, PDF, draw.io (visor local), Mermaid; offline | 4 | — |
| [ ] | Alertas desestimables (sin README, número sin prefijo, duplicados, plan incompleto, git) | 4 | — |
| [~] | Proyectos recientes con alias; cambiar de proyecto | 4 | — |
| [~] | Tema claro / oscuro / sistema; variables de color; i18n en / es | 2 / 4 | — |
| [ ] | Prioridad, pactada, vencidas, agrupar por período, planificación local con marca **local** | 4 | misc (períodos) |

## Git

| | Función | Fase | Golden |
|---|---|---|---|
| [x] | Solo lectura (`for-each-ref`, `rev-list`, `cherry`, `merge-base`, `diff`, `log`, `status --no-optional-locks`) | 5 | — |
| [x] | Repos hermanos de `_work` detectados + configurados a mano; entornos por repo configurables | 5 | git |
| [x] | Estado por entorno de las ramas de cada WI: merged, equivalent, content, mentioned, pending | 5 | git |
| [x] | Rama actual, archivos sin commitear, último fetch; alertas front/back desparejos y sin commitear | 5 | git (parcial) |
| [x] | Caché por SHA en disco (`git_cache.json`), grafo en memoria, repos en paralelo | 5 | git (warm = cold) |
| [x] | Promoción: cherry-pick (dudosos destildados con motivo) o merge `--no-ff`; comandos PowerShell / Git Bash; URL de PR | 5 | git (datos; los comandos los arma la UI) |
| [x] | Diff de los commits propios para **Revisar** | 5 | git (branch_diff) |

## Sesiones y agentes

| | Función | Fase | Golden |
|---|---|---|---|
| [x] | Terminal embebida (pty), pestañas, `Ctrl+J`; cerrar mata el árbol | 6 | wb-sessions/tests |
| [x] | Agentes: Claude Code, Copilot CLI, comando propio con placeholders | 6 | agents (unit) |
| [x] | Contexto (`AGENTS.md`), guía, config del MCP y permisos por sesión | 6 | wb-sessions/tests |
| [x] | Hooks (trabajando / te necesita / reposo) y `activity.jsonl` | 6 | wb-sessions/tests |
| [x] | Barra lateral: fijadas, corriendo, recientes, anteriores, cerradas, otros proyectos; cierre automático | 6 | — |
| [x] | Renombrar, quitar WI, Separar (fork), sumar a sesión, reabrir | 6 | wb-sessions/tests |
| [x] | Sesión larga: peso por transcript, recomendación (nueva / traspaso / compactar) | 6 | transcript (unit) |
| [x] | Otro WI en el pedido (`UserPromptSubmit`) | 6 | wb-sessions/tests |
| [x] | Planear (modo plan + validación en `Stop`); Implementar Feature/Bug **(privado: plugin en `../workboard-private`)** | 6 / 8 | wb-api, workboard-private tests/stdio |
| [x] | Reflexionar (sesión en modo plan con evidencia) y Revisar (paquete con el diff de cada rama, sesión en modo plan con el modelo elegido) (y «Aplicar candidatos…» desde la nota de aprendizajes) | 6 | wb-sessions/tests, wb-api git::tests y tests::apply_learnings |
| [x] | Horas trabajadas por WI y período (activity + transcript) | 6 | timelog (unit), wb-sessions/tests |
| [x] | Métricas por sesión y por configuración; exportar | 6 | stats (unit), wb-sessions/tests |
| [x] | Tools: candidatos, promover, congeladas por hash, `run_tool` | 6 | wb-mcp tools (unit), candidates (unit) |

## Notas

| | Función | Fase | Golden |
|---|---|---|---|
| [ ] | Carpeta configurable (vault de Obsidian), vínculo `wis: [...]`, autor | 3 / 4 | mcp (add_note) |
| [ ] | Cambiar título sin renombrar archivo; borrar a la papelera del sistema; filtros y orden | 4 | — |

## Azure DevOps

| | Función | Fase | Golden |
|---|---|---|---|
| [x] | Conexión por raíz, PAT de solo lectura en el almacén de credenciales del SO; mapeo de estados con sugerencias por categoría | 7 | wb-tracker/tests, wb-api/tests/tracker |
| [x] | Sync con previsualización (checkboxes, retrocesos destildados) y fila «según git» (destildada; el estado de cada ambiente sale de `git.envStates` del preset) | 7 | wb-api tracker::tests, git::tests |
| [x] | Campos elegibles de fecha pactada y horas; prioridad (la vista Horas usa horas y fecha del tracker si el README no las tiene) | 7 | wb-tracker (catalog) |
| [x] | Asignados del período (WIQL) e importación | 7 | wb-api/tests/tracker |
| [x] | Cambiar estado en Azure (PAT de escritura aparte, solo `System.State`) | 7 | wb-api/tests/tracker |
| [x] | ReleaseManual **(privado: plugin en `../workboard-private`, más un preset con las ramas de la empresa)** | 8 | workboard-private tests/stdio |
| [x] | Plugins: manifiesto, acciones con formulario genérico, presets aportados, permisos del host | 8 | wb-plugins tests/rpc, wb-api tests/plugins |
