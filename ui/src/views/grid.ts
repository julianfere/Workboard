// Modelo de la grilla de Trabajos: columnas, filtros, agrupación en hasta 3 niveles, orden, densidad y
// vistas guardadas (pestañas). Todo se guarda en las preferencias de la UI (`ui_prefs.json`, clave `grid`).
// Estados, tipos, ambientes y claves del frontmatter salen del flujo; nada del flujo está escrito acá.

import { api } from "../api";
import { render } from "../app";
import { esc, fmtAgo, fmtDate } from "../dom";
import { lang, t } from "../i18n";
import { ico } from "../icons";
import { sessionsOf } from "../sessions";
import { S, effectiveStatus, envLabel, isClosed, matchesSearch, metaList, metaText, stateDef, stateLabel, stateRank, titleOf, trackerLookup, typeIcon, typeLabel, typeOf, wf } from "../state";
import { label } from "../i18n";
import { gitEnvSummary, gitPending } from "../git";
import { activeAlerts, type AlertKind, alertTitle } from "./alerts";
import { hoursHidden } from "../discreet";
import { fmtH, periodRange, timeReady, workedTotal } from "../stats";
import { byUrgency, dueCell, dueOf, hoursCell, hoursOf, isOverdue, lateTag, passesDue, periodDiff, periodOf, periodRel, priorityBadge, priorityOf } from "../planning";
import type { ExtraField, Item, Session } from "../types";
import { TR, trackerBadge, trackerConnected } from "./tracker";

// La caché del tracker de la carpeta abierta, para los helpers de `state.ts` (título y tipo del tracker).
trackerLookup.item = (id) => (TR.loadedFor === S.root ? TR.cfg?.cache?.items[id] : undefined);
trackerLookup.localType = (type) => (TR.loadedFor === S.root ? (TR.cfg?.localTypes[type] ?? "") : "");

// ---------- tipos ----------

export type Density = "comfortable" | "compact";
export type DueFilter = "" | "upto" | "cur" | "late";
export type GroupKey = "status" | "type" | "period" | "priority" | "tracker" | "weekLast" | "weekCreated";

export interface ColPref {
  id: string;
  w?: number;
}

/** Lo que guarda una vista: filtros, agrupación, columnas, orden y densidad. */
export interface Snap {
  status: string[];
  hideClosed: boolean;
  type: string;
  due: DueFilter;
  activity: string;
  groupBy: GroupKey[];
  columns: ColPref[] | null;
  sort: { id: string; dir: 1 | -1 } | null;
  density: Density;
}

export interface SavedView extends Snap {
  id: string;
  name: string;
}

interface GridPrefs {
  views: SavedView[];
  active: string;
  current: Snap;
  /** Grupos cuyo estado (abierto / cerrado) difiere del de por defecto, por carpeta de trabajos. */
  flipped: Record<string, string[]>;
  /** Grupos de primer nivel abiertos (salvo por período), por carpeta de trabajos: sin entrada, los dos primeros. */
  open: Record<string, string[]>;
}

export const GROUP_KEYS: GroupKey[] = ["status", "type", "period", "priority", "tracker", "weekLast", "weekCreated"];
export const GROUP_MAX = 3;
export const ACTIVITY_WEEKS = ["all", "2", "4", "8", "12"];
export const DUE_FILTERS: DueFilter[] = ["", "upto", "cur", "late"];

const DEFAULT_SNAP: Snap = {
  status: [],
  hideClosed: true,
  type: "",
  due: "",
  activity: "all",
  groupBy: ["weekLast"],
  columns: null,
  sort: null,
  density: "comfortable",
};

// ---------- estado ----------

export const G = {
  loaded: false,
  loading: false,
  prefs: {
    views: [] as SavedView[],
    active: "",
    current: { ...DEFAULT_SNAP } as Snap,
    flipped: {} as Record<string, string[]>,
    open: {} as Record<string, string[]>,
  } as GridPrefs,
  selected: new Set<string>(),
  /** Última fila marcada (para `Shift+click`). */
  anchor: "",
  /** Filas visibles en el orden de la grilla (para rangos y «seleccionar todo»). */
  order: [] as string[],
  /** Claves de los grupos de primer nivel que se ven (para recordar cuáles quedan abiertos). */
  topKeys: [] as string[],
};

export const snap = (): Snap => G.prefs.current;

function clean(raw: Partial<Snap> | undefined): Snap {
  const s = { ...DEFAULT_SNAP, ...(raw ?? {}) };
  return {
    status: Array.isArray(s.status) ? s.status.filter((x) => typeof x === "string") : [],
    hideClosed: s.hideClosed !== false,
    type: typeof s.type === "string" ? s.type : "",
    due: DUE_FILTERS.includes(s.due) ? s.due : "",
    activity: ACTIVITY_WEEKS.includes(s.activity) ? s.activity : "all",
    groupBy: (Array.isArray(s.groupBy) ? s.groupBy : []).filter((g, i, a) => GROUP_KEYS.includes(g) && a.indexOf(g) === i).slice(0, GROUP_MAX),
    columns: Array.isArray(s.columns) ? s.columns.filter((c) => c && typeof c.id === "string") : null,
    sort: s.sort && typeof s.sort.id === "string" ? { id: s.sort.id, dir: s.sort.dir === 1 ? 1 : -1 } : null,
    density: s.density === "compact" ? "compact" : "comfortable",
  };
}

const sameSnap = (a: Snap, b: Snap) => JSON.stringify(clean(a)) === JSON.stringify(clean(b));

/** Vistas de fábrica, las de la v1 (Mi vista, Activos, En QA, Todo) más «Por período». La de revisión es el último
 *  estado de la categoría `review` del flujo (`qa` en classic-es: «En QA»). */
function defaultViews(): SavedView[] {
  const review = [...wf().states].reverse().find((s) => s.category === "review");
  return [
    { id: "mine", name: t("grid.view.mine"), ...DEFAULT_SNAP },
    { id: "active", name: t("grid.view.active"), ...DEFAULT_SNAP, status: [], hideClosed: true },
    ...(review ? [{ id: "review", name: stateLabel(review.id), ...DEFAULT_SNAP, status: [review.id], groupBy: [] }] : []),
    { id: "all", name: t("grid.view.all"), ...DEFAULT_SNAP, hideClosed: false, groupBy: ["status"] as GroupKey[] },
    { id: "period", name: t("grid.view.period"), ...DEFAULT_SNAP, groupBy: ["period"] as GroupKey[] },
  ];
}

/** Lee las preferencias la primera vez que se muestra la grilla. */
export function ensurePrefs(): void {
  if (G.loaded || G.loading) return;
  G.loading = true;
  api
    .uiPrefs()
    .then((all) => {
      const raw = (all.grid ?? {}) as Partial<GridPrefs>;
      const views = Array.isArray(raw.views) ? raw.views.filter((v) => v && typeof v.id === "string").map((v) => ({ ...clean(v), id: v.id, name: String(v.name ?? v.id) })) : [];
      G.prefs.views = views.length ? views : defaultViews();
      G.prefs.active = G.prefs.views.some((v) => v.id === raw.active) ? raw.active! : G.prefs.views[0].id;
      G.prefs.current = raw.current ? clean(raw.current) : clean(G.prefs.views.find((v) => v.id === G.prefs.active));
      G.prefs.flipped = raw.flipped && typeof raw.flipped === "object" ? raw.flipped : {};
      G.prefs.open = raw.open && typeof raw.open === "object" ? raw.open : {};
    })
    .catch(() => {
      G.prefs.views = defaultViews();
      G.prefs.active = G.prefs.views[0].id;
    })
    .finally(() => {
      G.loaded = true;
      G.loading = false;
      render();
    });
}

let saveTimer = 0;
/** Guarda las preferencias (agrupa los cambios seguidos, como arrastrar un ancho). */
export function savePrefs(): void {
  clearTimeout(saveTimer);
  saveTimer = window.setTimeout(() => void api.setUiPref("grid", G.prefs).catch(() => undefined), 300);
}

/** Cambia algo de lo que muestra la grilla, guarda y vuelve a dibujar. */
export function update(fn: (s: Snap) => void): void {
  fn(G.prefs.current);
  G.prefs.current = clean(G.prefs.current);
  savePrefs();
  render();
}

// ---------- vistas guardadas ----------

export const activeView = () => G.prefs.views.find((v) => v.id === G.prefs.active) ?? null;
export const viewDirty = () => {
  const v = activeView();
  return !!v && !sameSnap(v, snap());
};

/** Cambia a una vista guardada (por id o por posición, desde 0). */
export function switchSavedView(which: string | number): void {
  const v = typeof which === "number" ? G.prefs.views[which] : G.prefs.views.find((x) => x.id === which);
  if (!v) return;
  G.prefs.active = v.id;
  G.prefs.current = clean(v);
  savePrefs();
  render();
}

export function saveView(): void {
  const cur = activeView();
  if (!cur) return;
  G.prefs.views = G.prefs.views.map((v) => (v.id === cur.id ? { ...v, ...clean(snap()) } : v));
  savePrefs();
  render();
}

export function discardView(): void {
  const cur = activeView();
  if (cur) switchSavedView(cur.id);
}

export function newView(name: string): void {
  const id = `v${Date.now().toString(36)}`;
  G.prefs.views = [...G.prefs.views, { id, name, ...clean(snap()) }];
  G.prefs.active = id;
  savePrefs();
  render();
}

export function renameView(id: string, name: string): void {
  G.prefs.views = G.prefs.views.map((v) => (v.id === id ? { ...v, name } : v));
  savePrefs();
  render();
}

export function moveView(id: string, d: number): void {
  const v = [...G.prefs.views];
  const i = v.findIndex((x) => x.id === id);
  const j = i + d;
  if (i < 0 || j < 0 || j >= v.length) return;
  [v[i], v[j]] = [v[j], v[i]];
  G.prefs.views = v;
  savePrefs();
  render();
}

/** Borra una vista; si era la última vuelven las de fábrica. */
export function deleteView(id: string): void {
  const rest = G.prefs.views.filter((x) => x.id !== id);
  G.prefs.views = rest.length ? rest : defaultViews();
  if (G.prefs.active === id || !rest.length) {
    switchSavedView(G.prefs.views[0].id);
    return;
  }
  savePrefs();
  render();
}

// ---------- columnas ----------

export interface Column {
  id: string;
  /** Clave de i18n del encabezado (o el texto, con `raw`). */
  label: string;
  /** `label` ya es el texto (columnas de campos extra del flujo). */
  raw?: boolean;
  /** Ancho por defecto en px (0 = flexible). */
  w: number;
  /** No se puede ocultar. */
  fixed?: boolean;
  /** El primer click ordena ascendente. */
  asc?: boolean;
  /** Solo si hay tracker conectado. */
  tracker?: boolean;
  render: (it: Item) => string;
  /** Valor para ordenar (null = sin valor: siempre al final). */
  sort: (it: Item) => number | string | null;
}

const kindsOf = (it: Item) => it.kinds.map((k) => `<span class="chip kind">${esc(k)}</span>`).join("");
const openSessions = (it: Item) => sessionsOf(it.name).filter((s) => s.state !== "closed").length;
const muted = (s: string) => (s ? `<span class="muted">${esc(s)}</span>` : "");

/** Ícono del tipo (el del flujo) con el nombre de tooltip; con `withLabel` (o sin ícono), también el nombre. */
function typeBadge(type: string, withLabel = false): string {
  if (!type) return "";
  const icon = typeIcon(type);
  return `<span class="tbadge" title="${esc(typeLabel(type))}">${icon ? ico(icon) : ""}${withLabel || !icon ? `<span>${esc(typeLabel(type))}</span>` : ""}</span>`;
}

function idCell(it: Item): string {
  if (it.kind === "group") {
    const tip = it.members
      .map((m) => S.items.find((x) => x.name === m))
      .map((m) => (m ? m.ids.join(" ") || m.name.split("/").pop() : ""))
      .join(" · ");
    return `<span class="id-cell" title="${esc(tip)}">${ico("layers")}<span>${esc(t("grid.groupItems", { n: it.members.length }))}</span></span>`;
  }
  const ids = it.ids.length ? esc(it.ids.join(" ")) : `<span class="muted">${esc(t("grid.noId"))}</span>`;
  return `<span class="id-cell">${typeBadge(typeOf(it))}<span class="wi-id">${ids}</span></span>`;
}

/** Sesión abierta del item (la que corre primero): punto de estado y nombre; click la abre (v1 `sessionCell`). */
function sessionCell(it: Item): string {
  const s: Session | undefined = sessionsOf(it.name)
    .filter((x) => x.state !== "closed")
    .sort((a, b) => Number(b.running) - Number(a.running))[0];
  if (!s) return "";
  return `<button class="sess-cell" data-action="sess-open" data-id="${esc(s.id)}" title="${esc(`${t(`session.status.${s.status}`)} · ${t("grid.sessionOpen")}`)}"><span class="sdot ${esc(s.status)}"></span><span>${esc(s.name)}</span></button>`;
}

/** Git por ambiente: completo (check), parcial (medio círculo) o ninguno (punto), con los repos (v1 `gitCell`). */
function gitCell(it: Item): string {
  const sum = gitEnvSummary(it);
  if (!sum) return gitPending() && it.ids.length ? `<span class="muted">…</span>` : "";
  return sum
    .map((s) => {
      const tip = t("grid.git.tip", { env: envLabel(s.env), n: s.ok.length, total: s.all.length, repos: s.ok.join(", ") || t("grid.git.none") });
      const mark = s.state === "ok" ? ico("check") : `<span class="gmark"></span>`;
      return `<span class="genv ${s.state}" title="${esc(tip)}">${esc(envLabel(s.env))}${mark}</span>`;
    })
    .join("");
}

const gitRank = (it: Item) => gitEnvSummary(it)?.reduce((n, s) => n + (s.state === "ok" ? 2 : s.state === "partial" ? 1 : 0), 0) ?? null;

// Alertas vigentes por item: se calculan una vez por render (`queueMicrotask` limpia la caché).
let alertCache: Map<string, AlertKind[]> | null = null;
function alertsOf(it: Item): AlertKind[] {
  if (!alertCache) {
    const map = new Map<string, AlertKind[]>();
    for (const a of activeAlerts()) for (const n of a.names ?? [a.item.name]) map.set(n, [...(map.get(n) ?? []), a.kind]);
    alertCache = map;
    queueMicrotask(() => (alertCache = null));
  }
  return alertCache.get(it.name) ?? [];
}

const alertsCell = (it: Item) =>
  alertsOf(it)
    .map((k) => `<span class="chip alert-chip" title="${esc(alertTitle(k))}">${esc(t(`grid.alertShort.${k}`))}</span>`)
    .join("");

/** Columna de un campo extra del flujo: el valor, o un link si el flujo dice qué es (`link`). */
function extraColumn(f: ExtraField): Column {
  const value = (it: Item) => metaText(it, f.key);
  return {
    id: `x:${f.key}`,
    label: f.labels ? label(f.labels, f.key) : f.key,
    raw: true,
    w: 130,
    render: (it) => {
      const v = value(it);
      if (!v) return "";
      const icon = f.icon ? ico(f.icon) : "";
      if (f.link === "tracker") return `<button class="link xlink" data-action="trk-open" data-id="${esc(v)}" title="${esc(t("grid.openTracker"))}">${icon}#${esc(v)}</button>`;
      return `<span class="xval">${icon}${esc(v)}</span>`;
    },
    sort: (it) => value(it) || null,
  };
}

function titleCell(it: Item): string {
  const compact = snap().density === "compact";
  const showNotes = !visibleColumns().some((c) => c.id === "notes");
  const notes = metaText(it, wf().fields.notes);
  const toggle =
    it.kind === "group"
      ? `<button class="grp-tw${folderOpen(it.name) ? " open" : ""}" data-action="grid-folder" data-name="${esc(it.name)}" aria-label="${esc(t("grid.toggleGroup"))}">${ico("chevronRight")}</button>`
      : "";
  const sub = compact ? "" : `<div class="sub">${esc(showNotes && notes ? notes : it.name)}</div>`;
  return `<div class="title">${toggle}<span class="t-main">${esc(titleOf(it))}</span>${lateTag(it)}${trackerBadge(it)}</div>${sub}`;
}

export const COLUMNS: Column[] = [
  { id: "status", label: "items.status", w: 150, render: (it) => statusChipHtml(effectiveStatus(it)), sort: (it) => stateRank(effectiveStatus(it)) },
  { id: "id", label: "items.id", w: 120, render: idCell, sort: (it) => it.ids[0] ?? null },
  { id: "type", label: "items.type", w: 130, render: (it) => typeBadge(typeOf(it), true), sort: (it) => (typeOf(it) ? wf().types.findIndex((x) => x.id === typeOf(it)) : null) },
  { id: "title", label: "items.title", w: 0, fixed: true, render: titleCell, sort: (it) => titleOf(it).toLowerCase() },
  { id: "folder", label: "grid.col.folder", w: 200, render: (it) => muted(it.name.split("/").pop()!), sort: (it) => it.name.toLowerCase() },
  { id: "notes", label: "items.notes", w: 260, render: (it) => muted(metaText(it, wf().fields.notes)), sort: (it) => metaText(it, wf().fields.notes) || null },
  { id: "tracker", label: "grid.col.tracker", w: 130, tracker: true, render: (it) => trackerBadge(it), sort: (it) => trackerStateOf(it) || null },
  { id: "priority", label: "grid.col.priority", w: 90, asc: true, render: (it) => priorityBadge(priorityOf(it)), sort: priorityOf },
  { id: "due", label: "grid.col.due", w: 160, asc: true, render: dueCell, sort: (it) => dueOf(it).date || null },
  { id: "hours", label: "grid.col.hours", w: 110, render: (it) => hoursCell(it, fmtH), sort: (it) => hoursOf(it).hours },
  { id: "worked", label: "grid.col.worked", w: 110, render: workedCell, sort: (it) => (timeReady() ? workedTotal(it) || null : null) },
  { id: "envs", label: "grid.col.envs", w: 130, render: (it) => metaList(it, wf().fields.environments).map((e) => `<span class="chip env">${esc(envLabel(e))}</span>`).join(""), sort: (it) => metaList(it, wf().fields.environments).length || null },
  { id: "branch", label: "grid.col.branch", w: 180, render: (it) => muted(metaText(it, wf().fields.branch)), sort: (it) => metaText(it, wf().fields.branch) || null },
  { id: "kinds", label: "grid.col.kinds", w: 170, render: kindsOf, sort: (it) => it.kinds.join(",") || null },
  { id: "tasks", label: "items.tasks", w: 90, render: (it) => muted(it.tasks.length ? `${it.tasks.filter((x) => !x.done).length}/${it.tasks.length}` : ""), sort: (it) => it.tasks.filter((x) => !x.done).length || null },
  { id: "sessions", label: "grid.col.sessions", w: 180, render: sessionCell, sort: (it) => openSessions(it) || null },
  { id: "git", label: "grid.col.git", w: 190, render: gitCell, sort: gitRank },
  { id: "alerts", label: "grid.col.alerts", w: 170, render: alertsCell, sort: (it) => alertsOf(it).length || null },
  { id: "files", label: "grid.col.files", w: 80, render: (it) => muted(String(it.files.length)), sort: (it) => it.files.length },
  { id: "last", label: "items.activity", w: 120, render: (it) => `<span class="muted" title="${esc(fmtDate(it.last))}">${esc(fmtAgo(it.last))}</span>`, sort: (it) => it.last },
  { id: "created", label: "grid.col.created", w: 120, render: (it) => muted(it.created ? fmtDate(it.created) : ""), sort: (it) => it.created || null },
];

/** Las de la v1: estado, id, título, sesión, tracker, git, scripts, ambientes, alertas y actividad. */
const DEFAULT_COLUMNS = ["status", "id", "title", "sessions", "tracker", "git", "kinds", "envs", "alerts", "last"];

/** Todas las columnas: las fijas más una por campo extra del flujo. */
const allColumns = (): Column[] => [...COLUMNS, ...wf().fields.extra.map(extraColumn)];
const colById = (id: string): Column | undefined => COLUMNS.find((c) => c.id === id) ?? allColumns().find((c) => c.id === id);

/** Encabezado de una columna. */
export const colLabel = (c: Column) => (c.raw ? c.label : t(c.label));

function workedCell(it: Item): string {
  const report = timeReady();
  if (!report) return `<span class="muted">…</span>`;
  const h = workedTotal(it);
  if (!h) return "";
  const planned = hoursOf(it).hours;
  return `<span class="num${planned && h > planned ? " over" : ""}">${esc(fmtH(h))}</span>`;
}

function trackerStateOf(it: Item): string {
  return it.ids.map((id) => TR.cfg?.cache?.items[id]?.state).find(Boolean) ?? "";
}

/** Estado como chip (el mismo de toda la app). */
export function statusChipHtml(status: string): string {
  if (!status) return `<span class="chip muted">${esc(t("items.noStatus"))}</span>`;
  const cat = stateDef(status)?.category ?? "unknown";
  return `<span class="chip"><span class="dot ${cat}"></span>${esc(stateLabel(status))}</span>`;
}

/** Columnas que se pueden mostrar ahora (la del tracker, solo con tracker). */
export const availableColumns = () => allColumns().filter((c) => (!c.tracker || trackerConnected()) && !hiddenHours(c.id));

/** Columnas de horas: en modo discreto no se muestran (siguen guardadas en la vista). */
const HOUR_COLUMNS = ["hours", "worked"];
const hiddenHours = (id: string) => hoursHidden() && HOUR_COLUMNS.includes(id);

/** Columnas visibles, en orden, con su ancho. El título siempre está. */
export function visibleColumns(): (Column & { width: number })[] {
  const saved: ColPref[] = snap().columns ?? DEFAULT_COLUMNS.map((id) => ({ id }));
  const list = saved.filter((c) => colById(c.id) && (!colById(c.id)!.tracker || trackerConnected()) && !hiddenHours(c.id));
  if (!list.some((c) => c.id === "title")) list.splice(Math.min(2, list.length), 0, { id: "title" });
  return list.map((c) => {
    const col = colById(c.id)!;
    return { ...col, width: c.w ?? col.w };
  });
}

function setColumns(cols: ColPref[]): void {
  update((s) => (s.columns = cols.map((c) => (c.w && c.w !== colById(c.id)?.w ? { id: c.id, w: c.w } : { id: c.id }))));
}

const currentCols = (): ColPref[] => (snap().columns ?? DEFAULT_COLUMNS.map((id) => ({ id }))).filter((c) => colById(c.id));

export function toggleColumn(id: string, on: boolean): void {
  const cols = currentCols();
  if (colById(id)?.fixed) return;
  setColumns(on ? (cols.some((c) => c.id === id) ? cols : [...cols, { id }]) : cols.filter((c) => c.id !== id));
}

export function moveColumn(id: string, d: number): void {
  const cols = currentCols();
  const i = cols.findIndex((c) => c.id === id);
  const j = i + d;
  if (i < 0 || j < 0 || j >= cols.length) return;
  [cols[i], cols[j]] = [cols[j], cols[i]];
  setColumns(cols);
}

export function setColumnWidth(id: string, w: number): void {
  const cols = currentCols();
  if (!cols.some((c) => c.id === id)) return;
  setColumns(cols.map((c) => (c.id === id ? { ...c, w: Math.max(60, Math.round(w)) } : c)));
}

export function resetColumns(): void {
  update((s) => {
    s.columns = null;
    s.sort = null;
  });
}

/** Click en un encabezado: primer sentido, el otro y sin orden. */
export function cycleSort(id: string): void {
  const col = colById(id);
  if (!col) return;
  const first: 1 | -1 = col.asc ? 1 : -1;
  update((s) => {
    if (!s.sort || s.sort.id !== id) s.sort = { id, dir: first };
    else if (s.sort.dir === first) s.sort = { id, dir: (-first) as 1 | -1 };
    else s.sort = null;
  });
}

// ---------- filtros ----------

/** Búsqueda global (la misma de Alertas y Pendientes; ver `matchesSearch`). */
export const matchesText = (it: Item, q = S.search): boolean => matchesSearch(it, q);

const WEEK = 7 * 86400;

/** Lunes 00:00 (local) de la semana de `ts` (segundos). */
function weekStart(ts: number): number {
  const d = new Date(ts * 1000);
  d.setHours(0, 0, 0, 0);
  d.setDate(d.getDate() - ((d.getDay() + 6) % 7));
  return d.getTime() / 1000;
}

const activityField = (): "created" | "last" => (snap().groupBy.includes("weekCreated") ? "created" : "last");

function passesActivity(it: Item): boolean {
  const w = snap().activity;
  if (w === "all") return true;
  return it[activityField()] >= weekStart(Date.now() / 1000) - (Number(w) - 1) * WEEK;
}

export function passesFilters(it: Item, skipText = false): boolean {
  const s = snap();
  const status = effectiveStatus(it);
  if (!skipText && !matchesText(it)) return false;
  if (!passesActivity(it)) return false;
  if (s.type && (it.type || "none") !== s.type) return false;
  if (!passesDue(it, s.due)) return false;
  if (s.status.length) return s.status.includes(status);
  return !(s.hideClosed && isClosed(status));
}

export interface Row {
  item: Item;
  /** Items visibles del grupo (si la fila es un grupo de carpetas). */
  shown: Item[];
}

/** Filas de primer nivel: items sueltos y grupos con algún item visible (o que coinciden con la búsqueda). */
export function visibleRows(): Row[] {
  const out: Row[] = [];
  for (const it of S.items) {
    if (it.group) continue;
    if (it.kind !== "group") {
      if (passesFilters(it)) out.push({ item: it, shown: [] });
      continue;
    }
    const subs = it.members.map((m) => S.items.find((x) => x.name === m)).filter(Boolean) as Item[];
    const groupHit = !!S.search.trim() && matchesText(it);
    const shown = subs.filter((m) => passesFilters(m, groupHit));
    if (shown.length || (!subs.length && passesFilters(it))) out.push({ item: it, shown });
  }
  return out;
}

/** Chips de los filtros activos: `[clave, texto]`. */
export function filterChips(): [string, string][] {
  const s = snap();
  const out: [string, string][] = [];
  if (s.status.length) out.push(["status", s.status.length > 3 ? t("grid.chip.statusN", { n: s.status.length, total: wf().states.length }) : t("grid.chip.status", { list: s.status.map(stateLabel).join(", ") })]);
  else if (s.hideClosed) out.push(["hideClosed", t("grid.chip.hideClosed")]);
  if (s.type) out.push(["type", t("grid.chip.type", { type: s.type === "none" ? t("items.noType") : typeLabel(s.type) })]);
  if (s.due) out.push(["due", t("grid.chip.due", { what: t(`grid.due.${s.due}`) })]);
  if (s.activity !== "all") out.push(["activity", t(activityField() === "created" ? "grid.chip.created" : "grid.chip.activity", { n: s.activity })]);
  return out;
}

export function clearChip(k: string): void {
  update((s) => {
    if (k === "status" || k === "all") s.status = [];
    if (k === "hideClosed") s.hideClosed = false;
    if (k === "all") s.hideClosed = DEFAULT_SNAP.hideClosed;
    if (k === "type" || k === "all") s.type = "";
    if (k === "due" || k === "all") s.due = "";
    if (k === "activity" || k === "all") s.activity = "all";
  });
}

/** Cuántos items (sin grupos) hay en cada estado con la búsqueda actual, para el menú de filtros. */
export function statusCounts(): Record<string, number> {
  const out: Record<string, number> = {};
  for (const it of S.items) if (it.kind === "item" && matchesText(it) && passesActivity(it)) out[effectiveStatus(it)] = (out[effectiveStatus(it)] ?? 0) + 1;
  return out;
}

// ---------- agrupar ----------

export interface Group {
  key: string;
  title: string;
  sub?: string;
  cls?: string;
  /** Abierto por defecto. */
  open: boolean;
  rows: Row[];
  children?: Group[];
}

export function setGroupLevel(i: number, v: string): void {
  update((s) => {
    const lv = [...s.groupBy];
    if (v) lv[i] = v as GroupKey;
    else lv.splice(i, 1);
    s.groupBy = lv.filter(Boolean);
  });
}

function weekTitle(ws: number): { title: string; sub: string } {
  const n = Math.round((weekStart(Date.now() / 1000) - ws) / WEEK);
  const d = new Date(ws * 1000).toLocaleDateString(lang(), { day: "numeric", month: "short", year: "numeric" });
  const sub = n === 0 ? t("grid.week.this") : n === 1 ? t("grid.week.last") : t("grid.week.ago", { n });
  return { title: t("grid.week.of", { date: d }), sub };
}

function bucket(rows: Row[], keyOf: (r: Row) => string): Map<string, Row[]> {
  const map = new Map<string, Row[]>();
  for (const r of rows) {
    const k = keyOf(r);
    if (!map.has(k)) map.set(k, []);
    map.get(k)!.push(r);
  }
  return map;
}

function groupBy(rows: Row[], by: GroupKey): Group[] {
  const dot = (cat: string) => `<span class="dot ${esc(cat)}"></span>`;
  if (by === "status") {
    const m = bucket(rows, (r) => effectiveStatus(r.item));
    const keys = [...wf().states.map((s) => s.id), ...[...m.keys()].filter((k) => !stateDef(k))];
    return keys
      .filter((k) => m.has(k))
      .map((k) => ({ key: `s:${k}`, title: k ? `${dot(stateDef(k)?.category ?? "unknown")}${esc(stateLabel(k))}` : esc(t("items.noStatus")), open: true, rows: m.get(k)! }));
  }
  if (by === "type") {
    const m = bucket(rows, (r) => r.item.type);
    return [...wf().types.map((x) => x.id), ...[...m.keys()].filter((k) => k && !wf().types.some((x) => x.id === k)), ""]
      .filter((k) => m.has(k))
      .map((k) => ({ key: `t:${k}`, title: esc(k ? typeLabel(k) : t("items.noType")), open: true, rows: m.get(k)! }));
  }
  if (by === "priority") {
    const m = bucket(rows, (r) => String(priorityOf(r.item) ?? ""));
    return [...m.keys()]
      .sort((a, b) => (a === "" ? 1 : 0) - (b === "" ? 1 : 0) || Number(a) - Number(b))
      .map((k) => ({ key: `p:${k}`, title: k ? `${priorityBadge(Number(k))} ${esc(t("grid.priority.n", { n: k }))}` : esc(t("grid.priority.none")), open: true, rows: m.get(k)! }));
  }
  if (by === "tracker") {
    const m = bucket(rows, (r) => trackerStateOf(r.item));
    const order = (TR.cfg?.states ?? []).map((s) => s.name);
    const rank = (k: string) => (k ? (order.indexOf(k) + 1 || order.length + 1) : order.length + 2);
    return [...m.keys()]
      .sort((a, b) => rank(a) - rank(b) || a.localeCompare(b))
      .map((k) => {
        const cat = TR.cfg?.states.find((s) => s.name === k)?.suggested ?? "unknown";
        return { key: `a:${k}`, title: k ? `${dot(cat)}${esc(k)}` : esc(t("grid.tracker.none")), open: true, rows: m.get(k)! };
      });
  }
  if (by === "period") {
    // Pasados → actual → futuros (plegados) y al final los que no tienen fecha pactada.
    const m = bucket(rows, (r) => periodOf(r.item).key);
    const keys = [...m.keys()].filter(Boolean).sort();
    if (m.has("")) keys.push("");
    return keys.map((k) => {
      const list = m.get(k)!;
      if (!k) return { key: "p:none", title: esc(t("grid.period.none")), open: true, rows: list };
      const n = periodDiff(k);
      const late = list.filter((r) => isOverdue(r.item)).length;
      return {
        key: `p:${k}`,
        title: esc(t("grid.period.title", { range: periodRange(k) })),
        sub: `${esc(periodRel(k))}${late ? ` · <span class="tag-late">${esc(t("grid.overdueN", { n: late }))}</span>` : ""}`,
        cls: n === 0 ? "cur" : n > 0 ? "future" : "past",
        open: n <= 0,
        rows: list,
      };
    });
  }
  const field = by === "weekCreated" ? "created" : "last";
  const m = bucket(
    [...rows].sort((a, b) => b.item[field] - a.item[field]),
    (r) => String(weekStart(r.item[field])),
  );
  return [...m.entries()].map(([k, list]) => ({ key: `w:${k}`, ...weekTitle(Number(k)), open: true, rows: list }));
}

/** Grupos anidados, un nivel por criterio. La clave de un subgrupo incluye la de sus padres. */
export function groupTree(rows: Row[], levels: GroupKey[], parent = ""): Group[] {
  if (!levels.length) return [{ key: `${parent}all`, title: "", open: true, rows }];
  const gs = groupBy(rows, levels[0]).map((g) => ({ ...g, key: parent + g.key }));
  if (levels.length > 1) for (const g of gs) g.children = groupTree(g.rows, levels.slice(1), `${g.key}›`);
  return gs;
}

/** Orden de las filas: el de la columna elegida (sin valor, al final) o el de por defecto. */
export function sortRows<T>(list: T[], get: (x: T) => Item): T[] {
  const s = snap().sort;
  const col = s ? colById(s.id) : null;
  if (s && col) {
    return [...list].sort((a, b) => {
      const x = col.sort(get(a));
      const y = col.sort(get(b));
      if (x === null || y === null) return (x === null ? 1 : 0) - (y === null ? 1 : 0);
      return (x > y ? 1 : x < y ? -1 : 0) * s.dir || get(a).name.localeCompare(get(b).name);
    });
  }
  if (snap().groupBy.includes("period")) return [...list].sort((a, b) => byUrgency(get(a), get(b)));
  return [...list].sort((a, b) => get(b).last - get(a).last);
}

// ---------- plegado (por carpeta de trabajos) ----------

const flips = () => new Set(G.prefs.flipped[S.root] ?? []);

/** Grupos de la grilla (anota los de primer nivel para el plegado). */
export function gridGroups(rows: Row[]): Group[] {
  const gs = groupTree(rows, snap().groupBy);
  G.topKeys = gs.map((g) => g.key);
  return gs;
}

/** El primer nivel recuerda los grupos abiertos (salvo por período, que se abre según la fecha). */
const remembersTop = () => snap().groupBy.length > 0 && snap().groupBy[0] !== "period";

function topOpen(key: string): boolean {
  const saved = G.prefs.open[S.root];
  return saved ? saved.includes(key) : G.topKeys.indexOf(key) < 2;
}

/** Abierto: con búsqueda o un único grupo, siempre; en el primer nivel, los dos primeros o los que dejaste abiertos;
 *  en los demás (y por período), el de por defecto salvo que lo hayas cambiado (v1 app.js:622-629). */
export function groupOpen(g: Group, depth: number): boolean {
  if (S.search.trim() || (depth === 0 && G.topKeys.length === 1)) return true;
  if (depth === 0 && remembersTop()) return topOpen(g.key);
  return g.open !== flips().has(g.key);
}

export const folderOpen = (name: string) => !flips().has(`f:${name}`) || !!S.search.trim();

/** Plegar o desplegar un grupo de la grilla. */
export function toggleGroup(key: string, depth: number): void {
  if (depth !== 0 || !remembersTop()) return toggleFlip(key);
  const open = G.topKeys.filter(topOpen);
  G.prefs.open = { ...G.prefs.open, [S.root]: open.includes(key) ? open.filter((k) => k !== key) : [...open, key] };
  savePrefs();
  render();
}

/** Barra de colores por estado de las filas de un grupo (hasta 80). */
export function distBar(g: Group): string {
  const its = g.rows.flatMap((r) => (r.item.kind === "group" && r.shown.length ? r.shown : [r.item])).slice(0, 80);
  return `<span class="dist">${its.map((it) => `<i class="${esc(stateDef(effectiveStatus(it))?.category ?? "unknown")}" title="${esc(titleOf(it))}"></i>`).join("")}</span>`;
}

/** Categoría del estado de un item (franja de color de la fila). */
export const rowCategory = (it: Item) => stateDef(effectiveStatus(it))?.category ?? (effectiveStatus(it) ? "unknown" : "none");

export function toggleFlip(key: string): void {
  const f = flips();
  if (f.has(key)) f.delete(key);
  else f.add(key);
  G.prefs.flipped = { ...G.prefs.flipped, [S.root]: [...f] };
  savePrefs();
  render();
}

// ---------- exportar ----------

const textOf = (html: string) => {
  const d = document.createElement("div");
  d.innerHTML = html;
  // Las marcas (local, vencida) no son parte del valor; las listas de chips van separadas.
  d.querySelectorAll(".src-tag, .tag-late").forEach((x) => x.remove());
  d.querySelectorAll(".chip").forEach((x) => x.append(" "));
  return (d.textContent ?? "").replace(/\s+/g, " ").trim();
};

/** Lo que pasa los filtros (los items de los grupos incluidos) con las columnas visibles, en texto. */
export function exportRows(): string[][] {
  const cols = visibleColumns();
  const out = [[t("grid.col.folder"), t("grid.col.group"), ...cols.map(colLabel)]];
  const cell = (c: Column, it: Item) => (c.id === "title" ? titleOf(it) : textOf(c.render(it)));
  for (const r of sortRows(visibleRows(), (x) => x.item)) {
    for (const it of [r.item, ...sortRows(r.shown, (x) => x)]) out.push([it.name, it.group, ...cols.map((c) => cell(c, it))]);
  }
  return out;
}

