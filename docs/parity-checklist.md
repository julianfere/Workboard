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
| [x] | Después de mover: sesiones, marcas, backups y notas siguen a la carpeta (notas vinculadas por carpeta, también las de adentro de un grupo) | 3 / 6 | parcial (backups) |
| [x] | Nuevo item con o sin WI (crea carpeta + README; con id trae título, tipo, descripción y criterios del tracker) | 3 / 7 | workspace_ops (create_folder), wb-api/tests/tracker |
| [x] | Plan incompleto (`plan_gaps`) | 3 | misc |
| [x] | Bandeja de tareas sin WI (`inbox.md` en la config) | 3 | tasks (mismas funciones) |
| [x] | Watcher: refresco en vivo del filesystem y de `.git` de los repos. Un cambio en disco (o `F5`) relee items, detalle, bandeja, notas (salvo una en edición) y sesiones; las terminadas se cierran solas con Deshacer (`refreshAll`). El detalle conserva lo que estás escribiendo y avisa si el README cambió en disco | 4 | — |

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
| [x] | Vistas: Hoy, Trabajos, Pendientes, Horas, Alertas, Notas, Métricas, Tools, Ajustes. **Hoy** completo: saludo y fecha, tarjeta del período (días que quedan, avance, horas pactadas y trabajadas, cerrados N de M), «Necesita tu atención» (sesiones que te necesitan, también de otros proyectos; vencidas; alertas de git, sin movimiento y plan incompleto) con acción y Desestimar (el mismo de Alertas), En curso con horas trabajadas vs pactadas, fechas pactadas, próximas tareas de lo en curso o pactado para el período. **Pendientes** completo: filtros de estado, sesión, ambiente corrido y fecha pactada (guardados en `ui_prefs`, clave `tasks`), «Mostrar hechas», bloques por período pactado con sesión abierta y «N de M abiertas»; editar, borrar (con deshacer) y «Mover…» tareas de items y bandeja (`move_task`). Contadores del menú en Hoy (atención), Pendientes (abiertas + bandeja) y Alertas. **Ajustes**: abrir la raíz en el explorador o VS Code y la línea de atajos. Sin portar a propósito: densidad global (la grilla la tiene por vista) e índice de secciones | 4 | wb-api tests::move_task_between_items_and_inbox |
| [x] | Paleta `Ctrl+K` (buscar trabajos, tareas, archivos; comandos; acciones del trabajo de arriba): varias palabras (`matchesText`), Recientes sin texto, secciones con encabezado y pie ↑↓ / Enter. Acciones: siguiente estado (el que sigue en el preset sin cancelado/desconocido: en `classic-es`, En curso → Diagnóstico, la v1 saltaba a Codeado), sesión, tarea, pactada, Promover, VS Code (`open_in_editor`), carpeta. ReleaseManual va al plugin; el comando de modo discreto, con esa feature | 4 | — |
| [x] | Atajos (`Ctrl+J`, `/`, `1`…`6`, `Ctrl+S`, `Esc`, `F5`, `Supr`, `Alt+1`…`9`; también `Ctrl+A` en la grilla) | 4 | — |
| [x] | Selección múltiple, barra y menú contextual en bulk; deshacer de 12 s | 4 | — |
| [x] | Grilla: columnas configurables, orden, ancho, agrupar hasta 3 niveles, filtros con chips, densidad | 4 | — |
| [x] | Vistas guardadas (pestañas) con Guardar / Descartar | 4 | — |
| [x] | Exportar CSV (`;`, BOM) | 4 | — |
| [x] | Detalle a página completa: pasos de estado (hechos marcados, «Pasar a…», deshacer), guardado automático solo de lo tocado con indicador y foco devuelto, conflicto con disco (el refresco no pisa lo que escribís; aviso con «Aplicar mi cambio» / «Descartar»), migas al grupo, vencida, prioridad y estado del tracker, alertas del item, Sesión de IA, VS Code; planificación con período ±6, pistas del tracker / suma de sus items, «Quitar» y horas trabajadas; grupo con estado «el del item menos avanzado» y «Disolver grupo…»; carpeta que desaparece cierra el detalle con aviso. Sin plegar el panel de campos (opcional) | 4 | wb-api detail::tests |
| [x] | Pestañas del detalle: Resumen («Qué falta» arriba, «Scripts para correr» con marcas), README, Archivos (en un grupo también los de sus items con su dueño, «nuevo» / «modificado», categorías plegables recordadas, «Corrido en…» en el visor), Git, Cómo deployar (con ayuda según `deployKeywords`), Notas (crear, vincular, desvincular, «También en»), Versiones; contadores en las pestañas | 4 | wb-api detail::tests |
| [x] | Visor: SQL coloreado, Markdown, imágenes, PDF, draw.io (visor local), Mermaid; offline | 4 | viewer_reads (límites y rutas) |
| [x] | Alertas desestimables (sin README, número sin prefijo, duplicados, plan incompleto, git; también activos sin movimiento, sin estado y fecha vencida): por fila o todas por tipo, con huella de lo que la disparó (vuelve si cambia), por raíz en `<config>/alerts_off.json`; el badge no cuenta las desestimadas. Falta «MIGRATION / FIX sin correr en PROD» (necesita un dato nuevo en el preset) y «A _deleted» por carpeta en duplicados | 4 | wb-api alerts::tests |
| [x] | Proyectos recientes con alias (`settings.json` → `aliases`, `set_alias`; se migran de la v1); menú del nombre del proyecto con recientes («(no existe)», el actual resaltado, renombrar, quitar con Deshacer) y «Abrir carpeta…»; título de la ventana nativa con `setTitle`; `workboard <carpeta>` la abre. Título nativo y argumento sin probar en la ventana | 4 | wb-api settings / lib tests |
| [x] | Tema claro / oscuro / sistema (también desde la paleta); sin colores fijos fuera de los tokens (`--on-accent`, `--backdrop`); fechas con el idioma de la app; barra de título nativa con `setTheme` (sin probar en la ventana); i18n en / es | 2 / 4 | — |
| [x] | Prioridad, pactada, vencidas, agrupar por período, planificación local con marca **local** | 4 | misc (períodos) |
| [x] | Modo discreto (`Ctrl+Shift+H` o Ajustes → Apariencia): oculta horas pactadas y trabajadas (grilla, detalle, Hoy, planificación en lote) y la vista Horas; se guarda en `ui_prefs` (`hideHours`) | 4 | — |

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
| [x] | Carpeta configurable (vault de Obsidian: sin `.obsidian/` ni carpetas ocultas, con subcarpetas; el bloque de frontmatter ajeno se conserva al guardar, cambiar título o vincular; lee listas en bloque), vínculo de items (`notes.itemsKey` del preset) y autor | 3 / 4 | mcp (add_note), wb-core notes::tests |
| [x] | Cambiar título sin renombrar archivo; borrar a la papelera del sistema (crate `trash`; si falla, a `<config>/notes-deleted/`); filtros (texto, item, sin vincular) y orden (fecha, título) | 4 | wb-core notes::tests, wb-api notes::tests |

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
