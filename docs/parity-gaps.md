# Brechas de paridad v1 → v2

Relevamiento del 2026-11-25: la v1 (`_v1/`) contra la v2 (este repo + `../workboard-private`), leyendo el código de
los dos lados. El checklist de `docs/parity-checklist.md` tiene filas gruesas y las marca todas hechas; esto baja al
detalle. Cada US es una rama `us/<id>`; al cerrarla se tilda acá.

Lo que **coincide** y no hay que tocar: frontmatter, tareas, `write_readme`, mover/eliminar/restaurar/versiones,
bandeja, `runs.json`, algoritmo de estado git y comandos de promoción, consultas y reglas del sync de Azure, pasos de
ReleaseManual, horas, umbrales de peso de sesión.

Referencias: `v1 app.js:N` = `_v1/ui/app.js`.

## Decisiones tomadas

- La v2 se comporta como la v1 salvo lo listado en «Intencional» abajo o en `tests/golden/KNOWN_ISSUES.md`.
- Todo dato del flujo sale del preset (nada de `"en-curso"` o `"MIGRATION"` en el código): si hace falta, se agrega al
  preset y a los dos presets incluidos (y a `classic-tx` del privado).
- Intencional (no se toca): quitar «Elegir otro archivo» y «Copiar #id» de ReleaseManual; mandar solo campos
  visibles; GitHub/GitLab; draw.io sin recursos remotos; preset `default` en una instalación nueva.
- **Pendiente de decisión del usuario**: nombres de herramientas/parámetros/respuestas del MCP en inglés (rompe
  skills que usen los de la v1).

## US-01 Notas y Markdown

- [ ] **(alta)** El editor de notas pierde lo escrito: `notes.ts:123` rearma el `<textarea>` desde `note.body` en cada
      `render()` (las sesiones llaman a `render()` cada ≤15 s). Guardar el borrador en el estado, aviso «● cambios sin
      guardar» y confirmar al abrir otra nota / cambiar de vista / cerrar (v1 app.js:3655, 3727, 3782, 3789).
- [ ] Editar directo con pestañas Editar / Vista previa (v1 app.js:3773-3776).
- [ ] Abrir en Obsidian y abrir con la app predeterminada (v1 `open_note`, `app.py:404-414`, app.js:3778, 3847).
- [ ] Menú contextual de la lista: Abrir, Cambiar título, Mostrar en carpeta, Borrar (v1 app.js:3713, 3850-3858).
- [ ] Nota nueva desde el detalle se abre en edición con el cursor al final (v1 app.js:3808).
- [ ] Carpeta de notas con selector de carpeta y «Usar la predeterminada» (v1 app.js:3742, 3841).
- [ ] Links del Markdown abren en el navegador del sistema (`open_url`, solo https/http y `mailto:`), no en la ventana
      (v1 app.js:3257, 3378; v2 `markdown.ts:25` sin manejador).
- [ ] Markdown: `---` → `<hr>` y `[[wikilink]]` (v1 app.js:3258, 3273). Mermaid dentro de Markdown con zoom al clic
      (v1 diagrams.js:541).

## US-02 Terminal y panel de sesiones (UI)

- [ ] **(alta)** Ctrl+C con selección copia y Ctrl+V pega (v1 `attachCustomKeyEventHandler`, app.js:4491-4496).
- [ ] Aviso de conflicto: dos sesiones editando el mismo repo (banner + ícono en la pestaña; v1 app.js:4547,
      4557-4559, 4759-4763). `Session.repos` ya existe.
- [ ] Clic en una sesión pausada la reanuda y conecta (v1 app.js:4582-4593).
- [ ] Pausar pide confirmación (v1 app.js:4611). Quitar un item de la sesión con Deshacer (v1 app.js:4437).
- [ ] Ctrl+J sin pestañas: abre la última sesión o avisa «No hay sesiones abiertas» (v1 app.js:4595-4600).
- [ ] Panel: maximizar/restaurar (v1 app.js:4555, 4603); pestaña con agente y estado (v1 app.js:4546); «Sumar item…» a
      la sesión activa (v1 app.js:4551, 4716-4734).
- [ ] Lista: tooltip con items, hace cuánto y repos editados; etiqueta Copilot/Propio; conteo de items; «Otros
      proyectos» ordena primero «te necesita» y muestra el contador (v1 app.js:4376-4377, 4392-4395).
- [ ] Nueva sesión: nombre por defecto «WI a + WI b», link «Configurar un comando propio…», opción «Agrupar estos N en
      una carpeta grp-…» (v1 app.js:4632, 4643-4666).
- [ ] Toast «Compactando…»; recordar entre reinicios el aviso de sesión larga ocultado (v1 app.js:4360, 4371).

## US-03 Sesiones, MCP y métricas (backend)

- [ ] **(alta)** Planear con Claude empieza con `/go ` como la v1 (`harness.py:31, 263`): default del agente Claude, no
      vacío. Migración: ver US-11.
- [ ] Métricas: contar «pedidos» también con el `st` de la v1 (`te-necesita`) para las sesiones migradas
      (`stats.rs:398`). Anotar `st` en todos los eventos como la v1 (`sessions.py:434`).
- [ ] Avance/retrabajo en stats y «avanza/retrocede» del sync: diag y en-curso empatan como en la v1
      (`sessionstats.py:28`, `STATE_RANK` app.js:1397). Rango por categoría del preset, no por posición.
- [ ] Guía del agente (`mcp.guide`): recuperar lo que tenía la v1 (`harness.py:129-144`): tipos, qué muestra
      `get_item` (prioridad, pactada, período, horas), un archivo por item cuando hay varios sin grupo, ejemplo
      `DIAG_…sql`, «la ruta está en el contexto de la sesión». Intro de las `instructions` (`mcp_server.py:178-190`).
- [ ] `run_tool` sin `item`: el primero de la sesión, no el alfabético (`wb-mcp/src/lib.rs:646`). Evento con
      `nombre args → código`; salida recortada con «…(recortado)».
- [ ] Contexto: listar solo tools vigentes; línea aparte «README del grupo: …» (`harness.py:164, 194`).
- [ ] Nombre de un proyecto en una carpeta `_work`/tipo trabajos: el de la carpeta que la contiene o su alias, en
      recientes, título de ventana y sesiones de otros proyectos (v1 `app.py:66-73, 246`).
- [ ] Comando propio vacío = borrar la config; etiqueta por defecto «Comando propio».
- [ ] Promover tool: descripción opcional; reemplazar acepta otra ruta si la tool existe (v1 `app.py:623-630`).

## US-04 Alertas y Hoy

- [ ] **(alta)** Alerta «MIGRATION / FIX sin correr en PROD» (v1 app.js:139, 153, 174) en Alertas y en «Necesita tu
      atención» de Hoy. Categorías de archivo y ambiente final desde el preset.
- [ ] Desparejos de git: solo comparar repos que tienen QA… en términos de preset: los repos que tienen el ambiente
      intermedio; un repo sin ese ambiente no genera alertas en PROD (v1 app.js:2421; v2 `git.ts:125`).
- [ ] «Crear README» no pone estado ni abre el detalle (v1 app.js:1927).
- [ ] Duplicados como cluster: cantidad de archivos por carpeta, «A _deleted» por carpeta con Deshacer, «Es
      intencional» (v1 app.js:816-827).
- [ ] Contador del menú como la v1 (items únicos, solo tipos que cuentan; app.js:275). Filas con estado, tipo, #id,
      título, carpeta, hace cuánto (app.js:807).
- [ ] Reglas: stale solo en estados activos de la v1 (sin diag; app.js:154); «sin estado» también sin README
      (app.js:155); «sin prefijo» como la v1 (app.js:156). Alertas git por item, no por grupo (app.js:149). Huella de
      «sin commitear» con la lista de archivos (app.js:173).
- [ ] Hoy cuenta grupos y sueltos sin sus miembros (v1 `!i.group`, app.js:2827).
- [ ] Hoy «En curso»: «Pactada <fecha>» y notas sin horas; vencidas con estado + notas (app.js:2836, 2899).
      «Próximas tareas» con la fila de tarea completa (app.js:2914). El campo de la bandeja no se borra al refrescar.

## US-05 Grilla de Trabajos y búsqueda

- [ ] Columna Git (✓ / ◐ / ● por ambiente, repos, ordenable; v1 app.js:398, 781-803).
- [ ] Columnas Alertas (chips) y ReleaseManual (link al WI; columna aportada por el plugin o campo con link).
- [ ] Columna Sesión con el punto de estado y el nombre (v1 `sessionCell`, app.js:396).
- [ ] Título y tipo desde Azure cuando el README no los tiene (v1 `displayTitle` app.js:432, `tipoOf` app.js:112) en
      grilla, detalle, pendientes y alertas; pista «(Azure: X)» en el select de tipo.
- [ ] Columnas por defecto, vistas de fábrica («Mi vista», «Activos», «En QA», «Todo») y agrupación «Semana ·
      actividad» como la v1 (app.js:415, 467-477).
- [ ] Grupos abiertos al entrar: los dos primeros, recordar los abiertos, único grupo forzado (app.js:622-629).
- [ ] Barra de colores por estado en el encabezado de grupo, franja de estado por fila, ícono del tipo en ID
      (app.js:392, 632, 699).
- [ ] Búsqueda global (Alertas y Pendientes también; app.js:836, 846, 910) que incluye estado y título de Azure
      (app.js:333). «Limpiar búsqueda» cuando no hay resultados (app.js:605).
- [ ] Botón «Releer (F5)»; Eliminados siempre visible (app.js:597-598).

## US-06 Menú contextual, lote y eliminados

- [ ] Menú: Abrir detalle, Planear con…, Revisar con otro modelo…, acciones de plugins (Implementar), Disolver grupo,
      VS Code, Crear README faltante (N), Borrar resumen (v1 `buildMenu`, app.js:1894-1987).
- [ ] Estado «Sin estado» en el menú, la barra de lote y el detalle (app.js:1907, 1134).
- [ ] Ambientes en lote con «Agregar» / «Quitar» (Quitar solo los que tiene alguno; app.js:1970).
- [ ] Planificación en lote: atajos de período (anterior, actual, +1, +2, otro, según la fecha); vacío = quitar
      (app.js:1930-1942).
- [ ] Agrupar propone el título «WI a + WI b»; «Mover a grupo» como submenú (app.js:2069).
- [ ] Azure: «Sincronizar estados (N)» y «Abrir en Azure (N)» (máx. 10) de la selección (app.js:1947-1952).
- [ ] Explorer: hasta 5 carpetas.
- [ ] Aviso antes de eliminar/agrupar/mover/renombrar si una sesión trabaja en esas carpetas (`busyGuard`, app.js:2037).
- [ ] Eliminar en lote: saltear miembros de un grupo que también se elimina, errores por carpeta, Deshacer de lo hecho
      (v1 `app.py:818-832`).
- [ ] Deshacer en lote restaura sin validar (v1 `restore_meta`, `app.py:1491`).
- [ ] Ambientes en lote conserva los desconocidos (v1 `app.py:1476`).
- [ ] Restaurar de eliminados con el nombre original (sin sufijo de fecha) y el modal queda abierto (app.js:2142).

## US-07 Nuevo item y grupo

- [ ] «Crear y planear» en un paso: crea en el estado de planificación y abre la sesión de Planear (v1 `app.py:482-516`,
      app.js:3966, 3991-4003). Desde Nuevo item con WI y sin WI.
- [ ] Descripción y criterios editables (con WI: los de Azure; sin WI: «Qué hay que hacer / contexto»).
- [ ] Sin id: título obligatorio, tipo por defecto el de «arreglo» del preset, carpeta `<tipo>-<slug>`.
- [ ] README nuevo con línea de origen («> Arreglo del proyecto (sin work item)» / «> {tipo} #id · Azure: estado»),
      sección de plan con el texto de relleno y sección de pendientes (títulos del preset; v1 `app.py:506-512`). También
      los importados de Asignados (`tracker.rs:1011`).
- [ ] «Traer» con PAT inválido sigue en manual (`fromAzure: false`, v1 `app.py:476`).
- [ ] Crear grupo: título obligatorio, al menos un item, README con «> Grupo de WI: #ids», «## Cómo deployar» y
      pendientes, errores por carpeta sin cortar; se puede reagrupar items de otro grupo (v1 `app.py:852-871`).

## US-08 Detalle del item

- [x] Panel lateral sobre la grilla con fondo, ancho ajustable arrastrando, maximizable, ancho guardado
      (v1 app.js:739-779, 995-1005).
- [x] Pasos: camino principal + estado actual si es otro (app.js:1025, 2820).
- [x] Select de estado con todos los valores y «Sin estado»; pista del tipo de Azure (app.js:1134-1135).
- [x] Mover a grupo con selector, no prompt de texto; renombrar y sacar del grupo con aviso y Deshacer
      (app.js:2088, 2102).
- [x] Bloques: Info (creada, última actividad, archivos y tamaño, chips de scripts), resumen Git con «Ver ramas y
      commits →», Azure «Mapea a», link a Horas desde «Trabajadas», link «ReleaseManual #N» en cabecera y panel
      (app.js:1038, 1104, 1131, 1149-1162).
- [x] Archivos: lista y visor lado a lado, abre el primer MIGRATION/FIX, la pestaña se mantiene al cambiar de item
      (app.js:999, 1215).
- [x] Borrar una tarea y Deshacer respeta el tildado (app.js:887).
- [x] Planear desde cualquier estado con confirmación «Volver a planear»; pregunta si ya hay sesión (app.js:4063-4082).
- [ ] Implementar (plugin) solo en el estado de plan y confirma si hay sesión (app.js:1914, 4216).

## US-09 Git

- [ ] Botón para copiar el nombre de la rama de promoción (app.js:2628, 2650).
- [ ] Merge de ramas prefiere `origin/<rama>`, después el remote destino (app.js:2555).
- [ ] Nombre de la rama: orden de selección y sufijo como la v1 (`y{n}mas`) (app.js:2533, 2551).
- [ ] Comentario de conflicto también en modo merge (app.js:2581).
- [ ] Revisar fuerza un overview nuevo (v1 `app.py:725`); marca «… (diff recortado)» (`gitpanel.py:360`).
- [ ] Repo sin `[remote]` en `.git/config`: fallback (v1 `git remote`, `gitpanel.py:386`).
- [ ] «Repos y entornos» desde la pestaña Git abre un modal encima (app.js:2488).

## US-10 Azure DevOps

- [ ] Un solo PAT de escritura por organización para la app y ReleaseManual (v1 `ado.wpats[org]`). El plugin lo pide al
      host en vez de guardar el suyo.
- [ ] Fecha pactada: adivinar con la lista de la v1 (sin «target date»/«due date»; `app.py:52`) y volver a intentar en
      cada sync si no adivinó (`app.py:1349`); horas sin «hours».
- [ ] Cambiar el campo de fecha en Ajustes no resincroniza (solo desde el modal de sync).
- [ ] Sync sin configurar abre el asistente y sincroniza al guardar; errores de PAT/org/proyecto reabren el asistente;
      botón «Sincronizar Azure» siempre visible (app.js:1545-1560, index.html:42).
- [ ] Botón «Conexión y mapeo» en el pie del modal de sync y del de cambiar estado (app.js:4268).
- [ ] Tarjeta Azure del detalle: «Mapea a: <estado>» (app.js:1149).
- [ ] Organización: aceptar lo que aceptaba la v1 (`ado.py:64-74`).
- [ ] Entidades HTML completas en descripciones (v1 `html.unescape`).
- [ ] Mensajes de error con los textos de la v1 (`ado.py:113-127`, `app.py:1337`).
- [ ] ReleaseManual: orden de scripts como la v1; desde un grupo usa el grupo (privado `release.rs:388`, `lib.rs:47`).

## US-11 Migración desde la v1

- [ ] `plan_prefix` de Claude (`/go `).
- [ ] PAT de escritura para ReleaseManual (o el compartido de US-10).
- [ ] Vistas guardadas, columnas, orden y filtros (`views`, `activeView`, `gridCols`, `gridSort`) → `ui_prefs.json`;
      alertas desestimadas (`alertsOff:<raíz>`) → `alerts_off.json`. Corregir `docs/migrate-v1.md`.
- [ ] Configuración de ReleaseManual (`ado.rm[org|proyecto]`) → datos del plugin.

## US-12 Horas, métricas, paleta, ajustes y general

- [ ] Horas: clic en la fila abre el detalle y el número de sesiones el desglose; etiqueta de origen local/Azure,
      vencido, «· actual/anterior», chip de grupo «N WI» (app.js:2726-2741). CSV con «Fecha pactada» y «Origen».
- [ ] Persistir período, modo y orden de Horas (por carpeta) y rango/tipo/«ver todas» de Métricas.
- [ ] Paleta: Ctrl+K cierra, ↑↓ dan la vuelta, «Nueva sesión» usa la selección de la grilla (app.js:3113, 3664).
- [ ] Escape con foco en un campo cierra el detalle o limpia la búsqueda (app.js:3693).
- [ ] Ajustes: navegación por secciones (app.js:3198, 3345).
- [ ] Abrir en la última vista usada (pref `view`, por defecto Hoy; app.js:211).
- [ ] Exportar CSV con diálogo «Guardar como» (v1 `app.py:1006`).
- [ ] `mark_ran` valida que el archivo exista dentro de la raíz (v1 `app.py:787`).
- [ ] Log de errores de la API con rotación (v1 `workboard.log`, 1 MB).
