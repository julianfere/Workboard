// Trabajos: la grilla de items con vistas guardadas (pestañas), búsqueda, filtros con chips, agrupación en
// hasta 3 niveles, columnas configurables (orden, ancho), densidad, exportar a CSV, selección múltiple con
// barra y menú contextual en lote, y deshacer. El modelo (columnas, filtros, vistas) está en `grid.ts`.

import { api, trackerApi } from "../api";
import { onAction, onAfterRender, onChange, openDetail, refresh, refreshAll, registerView, render } from "../app";
import { noSearchResults, searchBox } from "../search";
import { $, confirmDialog, daysUntil, esc, fmtAgo, promptDialog, toast } from "../dom";
import { errorText, label, t } from "../i18n";
import { ico } from "../icons";
import { hasSessions, reflectItems } from "../reflect";
import { addToSessionModal, busyGuard, defaultAgentLabel, launched, newSessionModal } from "../sessions";
import { termSize } from "../terminal";
import { S, effectiveStatus, envLabel, isClosed, itemByName, metaList, metaText, stateDef, stateLabel, titleOf, wf } from "../state";
import { curPeriod, exportCsv, periodRange, periodShift, toCsv } from "../stats";
import { dueOf, periodRel } from "../planning";
import { hoursHidden } from "../discreet";
import { canReview, openReview } from "../review";
import { dissolveGroup, planWithConfirm } from "./detail";
import type { Item, MetaBefore, MetaValue, TrackerFailure, TrackerItem } from "../types";
import { TR, createTrackedItem, fetchTrackerItem, openAssigned, syncSelection, trackerBulk, trackerConnected, trackerToolbar } from "./tracker";
import { openPromotion } from "../git";
import { pluginBulkButtons, pluginMenuItems } from "../plugins";
import {
  ACTIVITY_WEEKS,
  DUE_FILTERS,
  G,
  GROUP_KEYS,
  GROUP_MAX,
  type Group,
  type Row,
  activeView,
  availableColumns,
  clearChip,
  colLabel,
  cycleSort,
  distBar,
  deleteView,
  discardView,
  ensurePrefs,
  exportRows,
  filterChips,
  folderOpen,
  gridGroups,
  groupOpen,
  matchesText,
  moveColumn,
  moveView,
  newView,
  renameView,
  resetColumns,
  saveView,
  setColumnWidth,
  setGroupLevel,
  snap,
  sortRows,
  statusChipHtml,
  statusCounts,
  switchSavedView,
  rowCategory,
  toggleColumn,
  toggleFlip,
  toggleGroup,
  update,
  viewDirty,
  visibleColumns,
  visibleRows,
} from "./grid";

export { switchSavedView };

/** Límites de la v1 para abrir varias cosas a la vez: carpetas en el explorador y work items en el tracker. */
const EXPLORER_MAX = 5;
const TRACKER_OPEN_MAX = 10;

export const statusChip = statusChipHtml;

/** Fecha pactada (la del README o la del tracker) como marca, para las listas de Hoy y Pendientes. */
export function dueBadge(it: Item): string {
  const due = dueOf(it).date;
  if (!due) return "";
  const days = daysUntil(due);
  const overdue = days < 0 && !isClosed(effectiveStatus(it));
  return `<span class="due${overdue ? " overdue" : days <= 7 ? " soon" : ""}" title="${esc(due)}">${esc(due)}</span>`;
}

// ---------- selección (exportada para los atajos de teclado) ----------

const selectedNames = () => [...G.selected].filter((n) => itemByName(n));

/** Marca o desmarca una fila. */
export function toggleSelect(name: string): void {
  if (G.selected.has(name)) G.selected.delete(name);
  else G.selected.add(name);
  G.anchor = name;
  render();
}

/** Marca las filas visibles entre la última marcada y `name` (`Shift+click`). */
export function selectRange(name: string): void {
  const a = G.order.indexOf(G.anchor);
  const b = G.order.indexOf(name);
  if (a < 0 || b < 0) return toggleSelect(name);
  for (const n of G.order.slice(Math.min(a, b), Math.max(a, b) + 1)) G.selected.add(n);
  render();
}

/** Marca todo lo que se ve en la grilla (`Ctrl+A`). */
export function selectAllVisible(): void {
  for (const n of G.order) G.selected.add(n);
  render();
}

/** Limpia la selección (`Esc`). */
export function clearSelection(): void {
  if (!G.selected.size) return;
  G.selected.clear();
  render();
}

export const selectionSize = () => selectedNames().length;

/** Manda la selección a eliminados, con deshacer (`Supr`). */
export async function deleteSelection(): Promise<void> {
  const names = selectedNames();
  if (names.length) await trashItems(names);
}

// ---------- barra: pestañas, filtros, agrupar ----------

function tabs(): string {
  const av = activeView();
  const dirty = viewDirty();
  const total = S.items.filter((i) => i.kind === "item").length;
  const visible = visibleRows().reduce((n, r) => n + (r.item.kind === "group" ? r.shown.length : 1), 0);
  return `<div class="vtabs" role="tablist">${G.prefs.views
    .map(
      (v, i) =>
        `<button class="vtab${av?.id === v.id ? " on" : ""}" role="tab" aria-selected="${av?.id === v.id}" data-action="grid-view" data-id="${esc(v.id)}" data-index="${i}" title="${esc(t("grid.view.hint"))}">${esc(v.name)}${av?.id === v.id && dirty ? `<span class="vdirty" title="${esc(t("grid.view.unsaved"))}"></span>` : ""}</button>`,
    )
    .join("")}
    <button class="vtab add" data-action="grid-view-new" title="${esc(t("grid.view.newHint"))}">${ico("plus")}${esc(t("grid.view.new"))}</button>
    <span class="spacer"></span><span class="muted">${esc(t("grid.count", { n: visible, total }))}</span></div>`;
}

function groupSelects(): string {
  const lv = snap().groupBy;
  const sel = (i: number, v: string) => {
    const used = new Set(lv.filter((_, j) => j !== i));
    const empty = i === 0 ? t("items.groupBy.none") : i < lv.length ? t("grid.groupBy.remove") : t("grid.groupBy.add");
    const opts = GROUP_KEYS.filter((k) => !used.has(k) && (k !== "tracker" || trackerConnected()))
      .map((k) => `<option value="${k}"${k === v ? " selected" : ""}>${esc(t(`grid.groupBy.${k}`))}</option>`)
      .join("");
    return `<select data-change="grid-group" data-level="${i}" aria-label="${esc(t("grid.groupBy.level", { n: i + 1 }))}"><option value="">${esc(empty)}</option>${opts}</select>`;
  };
  const sep = `<span class="glevel-sep">${ico("chevronRight")}</span>`;
  const parts = lv.map((v, i) => (i ? sep : "") + sel(i, v));
  if (!lv.length) parts.push(sel(0, ""));
  else if (lv.length < GROUP_MAX) parts.push(sep + sel(lv.length, ""));
  return `<span class="glevels" title="${esc(t("grid.groupBy.hint", { n: GROUP_MAX }))}">${esc(t("items.groupBy"))} ${parts.join("")}</span>`;
}

function toolbar(): string {
  const chips = filterChips();
  const density = snap().density;
  const dirty = viewDirty();
  return `<div class="toolbar"><h1>${esc(t("nav.items"))}</h1>
      ${searchBox()}
      <button class="btn${chips.length ? " has" : ""}" data-action="grid-pop" data-kind="filters" aria-haspopup="true">${ico("filter")}${esc(t("grid.filters"))}${chips.length ? `<b class="fcount">${chips.length}</b>` : ""}${ico("chevronDown")}</button>
      ${groupSelects()}
      <span class="spacer"></span>
      <button class="btn ghost" data-action="show-deleted" title="${esc(t("grid.deletedHint"))}" aria-label="${esc(t("items.deleted"))}">${ico("trash")}${S.deletedCount ? S.deletedCount : ""}</button>
      <button class="icon-btn" data-action="grid-reload" title="${esc(t("grid.reload"))}" aria-label="${esc(t("grid.reload"))}">${ico("refresh")}</button>
      ${trackerToolbar()}
      <button class="btn primary" data-action="new-item">${ico("plus")}${esc(t("items.new"))}</button>
    </div>
    <div class="toolbar sub">
      <div class="fchips">${chips.map(([k, l]) => `<span class="chip fchip">${esc(l)}<button data-action="grid-chip" data-k="${k}" title="${esc(t("grid.chip.remove"))}" aria-label="${esc(t("grid.chip.remove"))}">${ico("x")}</button></span>`).join("")}
        ${chips.length > 1 ? `<button class="link" data-action="grid-chip" data-k="all">${esc(t("grid.clearFilters"))}</button>` : ""}</div>
      <span class="spacer"></span>
      ${dirty ? `<span class="vsave"><span class="muted">${esc(t("grid.view.unsaved"))}</span><button class="btn" data-action="grid-view-discard">${esc(t("grid.view.discard"))}</button><button class="btn primary" data-action="grid-view-save">${esc(t("grid.view.save"))}</button></span>` : ""}
      <div class="seg" role="group" aria-label="${esc(t("grid.density"))}">${(["comfortable", "compact"] as const)
        .map((d) => `<button class="${density === d ? "on" : ""}" data-action="grid-density" data-d="${d}">${esc(t(`grid.density.${d}`))}</button>`)
        .join("")}</div>
      <button class="btn" data-action="grid-pop" data-kind="columns" aria-haspopup="true">${ico("columns")}${esc(t("grid.columns"))}</button>
      <button class="icon-btn" data-action="grid-export" title="${esc(t("grid.export"))}" aria-label="${esc(t("grid.export"))}">${ico("download")}</button>
    </div>`;
}

// ---------- grilla ----------

function rowHtml(it: Item, cols: ReturnType<typeof visibleColumns>, cls = ""): string {
  G.order.push(it.name);
  const sel = G.selected.has(it.name);
  return `<tr class="row cat-${rowCategory(it)} ${cls}${sel ? " sel" : ""}" data-action="grid-row" data-name="${esc(it.name)}">
    <td class="gc-check"><input type="checkbox" data-action="select-item" data-name="${esc(it.name)}"${sel ? " checked" : ""} aria-label="${esc(t("items.select"))}"></td>
    ${cols.map((c) => `<td class="gc-${c.id}">${c.render(it)}</td>`).join("")}</tr>`;
}

function rowsHtml(rows: Row[], cols: ReturnType<typeof visibleColumns>): string {
  return sortRows(rows, (r) => r.item)
    .map((r) => {
      if (r.item.kind !== "group") return rowHtml(r.item, cols);
      const open = folderOpen(r.item.name);
      const kids = open ? sortRows(r.shown, (x) => x).map((m) => rowHtml(m, cols, "nested")).join("") : "";
      return rowHtml(r.item, cols, `group${open ? " open" : ""}`) + kids;
    })
    .join("");
}

function groupHtml(g: Group, depth: number, cols: ReturnType<typeof visibleColumns>): string {
  if (!g.title && !depth && !g.children) return rowsHtml(g.rows, cols);
  const open = groupOpen(g, depth);
  const count = g.rows.reduce((n, r) => n + (r.item.kind === "group" ? r.shown.length || 1 : 1), 0);
  const head = `<tr class="group-head d${depth}${g.cls ? ` ${g.cls}` : ""}${open ? " open" : ""}" data-action="grid-group" data-key="${esc(g.key)}" data-depth="${depth}">
    <td colspan="${cols.length + 1}"><span class="gh" style="padding-left:${depth * 20}px">${ico("chevronRight", "tw")}<span class="gt">${g.title}</span>${g.sub ? `<span class="gs">${g.sub}</span>` : ""}<span class="gc">${count}</span>${distBar(g)}</span></td></tr>`;
  if (!open) return head;
  return head + (g.children ? g.children.map((c) => groupHtml(c, depth + 1, cols)).join("") : rowsHtml(g.rows, cols));
}

function table(): string {
  const cols = visibleColumns();
  const rows = visibleRows();
  G.order = [];
  const sort = snap().sort;
  if (!rows.length) {
    const empty = !S.items.length
      ? esc(t("items.empty"))
      : S.search.trim() && !S.items.some((it) => matchesText(it))
        ? noSearchResults()
        : `${esc(t("items.noMatches"))} <button class="link" data-action="grid-chip" data-k="all">${esc(t("grid.clearFilters"))}</button>`;
    return `<p class="empty">${empty}</p>`;
  }
  const body = gridGroups(rows)
    .map((g) => groupHtml(g, 0, cols))
    .join("");
  const allSel = G.order.length > 0 && G.order.every((n) => G.selected.has(n));
  const minW = 34 + cols.reduce((n, c) => n + (c.width || 260), 0);
  const head = cols
    .map((c) => {
      const arrow = sort?.id === c.id ? ico("chevronDown", sort.dir > 0 ? "flip" : "") : "";
      return `<th class="gc-${c.id}" aria-sort="${sort?.id === c.id ? (sort.dir > 0 ? "ascending" : "descending") : "none"}"><button class="th" data-action="grid-sort" data-col="${c.id}">${esc(colLabel(c))}${arrow}</button><span class="col-rs" data-rs="${c.id}" title="${esc(t("grid.resize"))}"></span></th>`;
    })
    .join("");
  return `<div class="grid-wrap"><table class="grid items-grid ${snap().density}" style="min-width:${minW}px">
    <colgroup><col style="width:34px">${cols.map((c) => `<col data-col="${c.id}"${c.width ? ` style="width:${c.width}px"` : ""}>`).join("")}</colgroup>
    <thead><tr><th class="gc-check"><input type="checkbox" data-change="grid-select-all"${allSel ? " checked" : ""} aria-label="${esc(t("grid.selectAll"))}"></th>${head}</tr></thead>
    <tbody>${body}</tbody></table></div>`;
}

function bulkBar(): string {
  const names = selectedNames();
  if (!names.length) return "";
  const states = [`<option value="!">${esc(t("items.noStatus"))}</option>`, ...wf().states.map((s) => `<option value="=${esc(s.id)}">${esc(label(s.labels, s.id))}</option>`)].join("");
  return `<div class="bulk" role="toolbar" aria-label="${esc(t("items.selected", { n: names.length }))}"><b>${esc(t("items.selected", { n: names.length }))}</b>
    <select data-change="bulk-status" aria-label="${esc(t("items.setStatus"))}"><option value="">${esc(t("items.setStatus"))}</option>${states}</select>
    <button class="btn" data-action="grid-bulk-plan">${ico("calendar")}${esc(t("grid.bulk.planning"))}</button>
    <button class="btn" data-action="bulk-group">${ico("layers")}${esc(t("items.group"))}</button>
    ${trackerBulk(names)}
    ${pluginBulkButtons(names)}
    ${hasSessions(names) ? `<button class="btn" data-action="bulk-reflect" title="${esc(t("reflect.hint"))}">${ico("sparkle")}${esc(t("reflect.action"))}</button>` : ""}
    <button class="btn" data-action="bulk-promote" title="${esc(t("promo.buttonHint"))}">${ico("promote")}${esc(t("promo.button"))}</button>
    <button class="btn" data-action="grid-pop" data-kind="ctx">${esc(t("grid.bulk.more"))}${ico("chevronDown")}</button>
    <button class="btn danger" data-action="bulk-trash">${ico("trash")}${esc(t("items.delete"))}</button>
    <span class="spacer"></span>
    <button class="link" data-action="bulk-clear">${esc(t("items.clearSelection"))}</button></div>`;
}

function renderItems(): string {
  ensurePrefs();
  return `<div class="page wide items-page">
    ${tabs()}
    ${toolbar()}
    ${S.loadError ? `<p class="error">${esc(S.loadError)}</p>` : ""}
    ${table()}
    ${bulkBar()}
  </div>`;
}

// ---------- menús (filtros, columnas, pestaña, selección) ----------

type PopKind = "filters" | "columns" | "view" | "ctx";
let pop: { kind: PopKind; x: number; y: number; arg: string; right: boolean } | null = null;

const menuItem = (action: string, icon: string, text: string, data: Record<string, string> = {}, extra = "") =>
  `<button class="menu-item" data-action="${action}"${Object.entries(data)
    .map(([k, v]) => ` data-${k}="${esc(v)}"`)
    .join("")}>${ico(icon)}<span>${esc(text)}</span>${extra}</button>`;
const head = (text: string, hint = "") => `<div class="menu-head">${esc(text)}${hint ? ` <span class="muted">· ${esc(hint)}</span>` : ""}</div>`;
const sepHtml = `<div class="menu-sep"></div>`;

function filtersMenu(): string {
  const s = snap();
  const counts = statusCounts();
  const states = wf()
    .states.filter((st) => counts[st.id] || s.status.includes(st.id))
    .map(
      (st) =>
        `<label class="menu-check"><input type="checkbox" data-change="grid-f-status" data-id="${esc(st.id)}"${s.status.includes(st.id) ? " checked" : ""}><span class="dot ${st.category}"></span><span class="grow">${esc(label(st.labels, st.id))}</span><span class="muted">${counts[st.id] ?? 0}</span></label>`,
    )
    .join("");
  const opt = (v: string, text: string, cur: string) => `<option value="${esc(v)}"${v === cur ? " selected" : ""}>${esc(text)}</option>`;
  const types = [opt("", t("grid.filter.allTypes"), s.type), ...wf().types.map((x) => opt(x.id, label(x.labels, x.id), s.type)), opt("none", t("items.noType"), s.type)].join("");
  const dues = DUE_FILTERS.map((d) => opt(d, t(d ? `grid.due.${d}` : "grid.due.any"), s.due)).join("");
  const acts = ACTIVITY_WEEKS.map((w) => opt(w, w === "all" ? t("grid.activity.all") : t("grid.activity.weeks", { n: w }), s.activity)).join("");
  return `${head(t("items.status"), t(s.hideClosed ? "grid.filter.statusHintClosed" : "grid.filter.statusHint"))}${states}
    ${sepHtml}<label class="menu-check"><input type="checkbox" data-change="grid-f-closed"${s.hideClosed ? " checked" : ""}><span>${esc(t("items.hideClosed"))}</span></label>
    <label class="menu-row"><span>${esc(t("items.type"))}</span><select data-change="grid-f-type">${types}</select></label>
    <label class="menu-row" title="${esc(t("grid.filter.dueHint"))}"><span>${esc(t("grid.col.due"))}</span><select data-change="grid-f-due">${dues}</select></label>
    <label class="menu-row"><span>${esc(t(snap().groupBy.includes("weekCreated") ? "grid.col.created" : "items.activity"))}</span><select data-change="grid-f-activity">${acts}</select></label>
    ${sepHtml}<button class="link menu-link" data-action="grid-chip" data-k="all">${esc(t("grid.clearFilters"))}</button>`;
}

function columnsMenu(): string {
  const visible = visibleColumns();
  const on = new Set(visible.map((c) => c.id));
  const ordered = [...visible, ...availableColumns().filter((c) => !on.has(c.id))];
  return `${head(t("grid.columns"), t("grid.columnsHint"))}${ordered
    .map(
      (c) => `<div class="menu-check"><label class="grow"><input type="checkbox" data-change="grid-col-on" data-col="${c.id}"${on.has(c.id) ? " checked" : ""}${c.fixed ? " disabled" : ""}> ${esc(colLabel(c))}</label>
      ${on.has(c.id) ? `<button class="icon-btn sm" data-action="grid-col-move" data-col="${c.id}" data-d="-1" title="${esc(t("grid.moveUp"))}" aria-label="${esc(t("grid.moveUp"))}">${ico("chevronDown", "flip")}</button><button class="icon-btn sm" data-action="grid-col-move" data-col="${c.id}" data-d="1" title="${esc(t("grid.moveDown"))}" aria-label="${esc(t("grid.moveDown"))}">${ico("chevronDown")}</button>` : ""}</div>`,
    )
    .join("")}${sepHtml}<button class="link menu-link" data-action="grid-col-reset">${esc(t("grid.columnsReset"))}</button>`;
}

function viewMenu(id: string): string {
  const v = G.prefs.views.find((x) => x.id === id);
  if (!v) return "";
  return `${head(v.name)}${menuItem("grid-view-rename", "edit", t("grid.view.rename"), { id })}
    ${menuItem("grid-view-move", "chevronLeft", t("grid.view.left"), { id, d: "-1" })}
    ${menuItem("grid-view-move", "chevronRight", t("grid.view.right"), { id, d: "1" })}
    ${sepHtml}${menuItem("grid-view-delete", "trash", t("grid.view.delete"), { id })}`;
}

/** Submenús abiertos del menú de la selección (se abren en el lugar, como acordeón). */
const subOpen = new Set<string>();

/** Entrada que abre o cierra un submenú; `body` son sus entradas (nada = no se muestra). */
function subMenu(key: string, icon: string, text: string, body: string): string {
  if (!body) return "";
  const open = subOpen.has(key);
  return `<button class="menu-item has-sub${open ? " open" : ""}" data-action="grid-sub" data-k="${esc(key)}" aria-expanded="${open}">${ico(icon)}<span>${esc(text)}</span>${ico("chevronRight", "sub-chev")}</button>${open ? `<div class="menu-sub">${body}</div>` : ""}`;
}

const hint = (text: string) => `<span class="muted">${esc(text)}</span>`;
const disabledItem = (icon: string, text: string, why: string) =>
  `<button class="menu-item" disabled title="${esc(why)}">${ico(icon)}<span>${esc(text)}</span>${hint(why)}</button>`;

/** Items de la selección con los grupos abiertos en sus miembros (para lo que va por id del tracker). */
function expandGroups(items: Item[]): Item[] {
  const out = new Map<string, Item>();
  for (const it of items) {
    const list = it.kind === "group" ? (it.members.map(itemByName).filter(Boolean) as Item[]) : [it];
    for (const m of list) out.set(m.name, m);
  }
  return [...out.values()];
}

/** Planificación de la selección: fecha pactada, período (atajos) y horas; vacío = quitar (v1 app.js:1930). */
function planSection(items: Item[]): string {
  const f = wf().fields;
  const one = items.length === 1 ? items[0] : null;
  const cur = curPeriod();
  const periods = [-1, 0, 1, 2]
    .map((d) => periodShift(cur, d))
    .map((k) => menuItem("grid-bulk-set", "calendar", `${periodRange(k)} · ${periodRel(k)}`, { key: f.period, value: k, what: t("grid.bulk.setTo", { what: t("detail.period"), value: periodRange(k) }) }, one && metaText(one, f.period) === k ? ico("check") : ""))
    .join("");
  const byDate = items.some((it) => metaText(it, f.period)) ? sepHtml + menuItem("grid-bulk-set", "eraser", t("grid.bulk.periodByDate"), { key: f.period, value: "", what: t("detail.period") }) : "";
  const out = [
    menuItem("grid-bulk-text", "calendar", `${t("detail.due")}…`, { key: f.due }),
    subMenu("period", "layers", t("detail.period"), periods + menuItem("grid-bulk-text", "edit", t("grid.bulk.periodOther"), { key: f.period }) + byDate),
  ];
  if (!hoursHidden()) out.push(menuItem("grid-bulk-text", "clock", `${t("detail.hours")}…`, { key: f.hours }));
  if (items.some((it) => metaText(it, f.due) || metaText(it, f.period) || metaText(it, f.hours)))
    out.push(menuItem("grid-bulk-revert", "undo", t(trackerConnected() ? "grid.bulk.revertTracker" : "grid.bulk.clearPlanning")));
  return out.join("");
}

/** Menú de la selección (v1 `buildMenu`, app.js:1894): lo que no aplica no aparece; lo que tiene un límite
 *  se deshabilita con el motivo. `only = "plan"` muestra solo la planificación (botón de la barra). */
function ctxMenu(only = ""): string {
  const names = selectedNames();
  if (!names.length) return "";
  const items = names.map(itemByName).filter(Boolean) as Item[];
  const n = items.length;
  const f = wf().fields;
  const one = n === 1 ? items[0] : null;
  if (only === "plan") return head(t("grid.bulk.planning")) + planSection(items);
  const out: string[] = [head(one ? titleOf(one) : t("items.selected", { n }))];
  if (one) out.push(menuItem("grid-bulk-open", "panel", t("grid.bulk.openDetail")));

  // Sesión de IA (y las acciones de los plugins, como Implementar)
  out.push(sepHtml, head(t("grid.bulk.ai")), menuItem("grid-bulk-session", "terminal", t("grid.bulk.newSession")));
  if (S.sessions.some((s) => s.state !== "closed")) out.push(menuItem("grid-bulk-addsession", "plus", t("grid.bulk.addSession")));
  const agent = defaultAgentLabel();
  if (one?.kind === "item") out.push(menuItem("grid-bulk-planone", "plan", agent ? t("grid.bulk.planWith", { agent }) : `${t("session.plan")}…`));
  if (one && canReview(one, effectiveStatus(one))) out.push(menuItem("grid-bulk-review", "check", t("grid.bulk.review")));
  if (hasSessions(names)) out.push(menuItem("bulk-reflect", "sparkle", t("reflect.action")));
  out.push(pluginMenuItems(names));

  // Estado, tipo, ambientes
  out.push(sepHtml);
  const status = [{ id: "", text: t("items.noStatus"), cat: "unknown" }, ...wf().states.map((st) => ({ id: st.id, text: label(st.labels, st.id), cat: st.category as string }))]
    .map((s) => menuItem("grid-bulk-set", "state", s.text, { key: f.status, value: s.id, what: t("grid.bulk.setTo", { what: t("items.status"), value: s.text }) }, `<span class="dot ${s.cat}"></span>`))
    .join("");
  out.push(subMenu("status", "state", t("items.status"), status));
  const types = [...wf().types.map((ty) => ({ id: ty.id, text: label(ty.labels, ty.id) })), { id: "", text: t("items.noType") }]
    .map((ty) => menuItem("grid-bulk-set", ty.id ? "tag" : "eraser", ty.text, { key: f.type, value: ty.id, what: t("grid.bulk.setTo", { what: t("items.type"), value: ty.text }) }))
    .join("");
  out.push(subMenu("type", "tag", t("items.type"), types));
  const envs = wf().environments;
  if (envs.length) {
    const withEnv = (id: string) => items.some((it) => metaList(it, f.environments).includes(id));
    const envItem = (mode: string) => (e: (typeof envs)[number]) => menuItem("grid-bulk-env", mode === "add" ? "plus" : "minus", label(e.labels, e.id), { env: e.id, mode });
    const add = subMenu("envs-add", "plus", t("grid.bulk.envAdd"), envs.map(envItem("add")).join(""));
    const remove = subMenu("envs-remove", "minus", t("grid.bulk.envRemove"), envs.filter((e) => withEnv(e.id)).map(envItem("remove")).join(""));
    out.push(subMenu("envs", "layers", t("grid.col.envs"), add + remove));
  }

  // Planificación y edición
  out.push(subMenu("plan", "calendar", t("grid.bulk.planning"), planSection(items)));
  const noReadme = items.filter((it) => !it.hasReadme);
  const edit = [
    menuItem("grid-bulk-text", "branch", `${t("grid.bulk.setBranch")}…`, { key: f.branch }),
    menuItem("grid-bulk-text", "edit", `${t("grid.bulk.setNotes")}…`, { key: f.notes }),
    items.some((it) => metaText(it, f.notes)) ? menuItem("grid-bulk-set", "eraser", t("grid.bulk.clearNotes"), { key: f.notes, value: "", what: t("items.notes") }) : "",
    one ? menuItem("grid-bulk-rename", "edit", `${t("grid.bulk.rename")}…`) : "",
    noReadme.length ? menuItem("grid-bulk-readme", "filePlus", t("grid.bulk.createReadme", { n: noReadme.length })) : "",
  ].join("");
  out.push(subMenu("edit", "edit", t("grid.bulk.edit"), edit));

  // Grupo
  const loose = items.filter((it) => it.kind === "item");
  const inGroup = loose.filter((it) => it.group);
  const groups = S.items.filter((it) => it.kind === "group" && loose.some((i) => i.group !== it.name));
  const grp = [
    loose.length ? menuItem("bulk-group", "folder", loose.length > 1 ? t("grid.bulk.groupNewN", { n: loose.length }) : t("grid.bulk.groupNew")) : "",
    loose.length && groups.length ? subMenu("move", "folderIn", t("grid.bulk.moveTo"), groups.map((g) => menuItem("grid-bulk-move", "folder", titleOf(g), { dest: g.name })).join("")) : "",
    inGroup.length ? menuItem("grid-bulk-ungroup", "folderOut", inGroup.length > 1 ? t("grid.bulk.ungroupN", { n: inGroup.length }) : t("grid.bulk.ungroup")) : "",
    one?.kind === "group" ? menuItem("grid-bulk-dissolve", "folderOut", t("detail.dissolve")) : "",
  ].join("");
  out.push(subMenu("group", "folder", t("items.group"), grp));

  // Git y tracker
  const withIds = expandGroups(items).filter((it) => it.ids.length);
  const ids = [...new Set(withIds.flatMap((it) => it.ids))];
  out.push(sepHtml, menuItem("bulk-promote", "promote", t("promo.button")));
  if (withIds.length) {
    const source = TR.cfg?.source ?? "";
    const openMany = ids.length > 1 ? t("grid.bulk.openTrackerN", { source, n: ids.length }) : t("grid.bulk.openTracker", { source });
    const trk = [
      menuItem("grid-bulk-sync", "refresh", withIds.length > 1 ? t("grid.bulk.syncN", { n: withIds.length }) : t("grid.bulk.sync")),
      trackerConnected() ? menuItem("trk-push", "cloud", `${t("tracker.push.button", { source })}…`, { names: JSON.stringify(names) }) : "",
      !trackerConnected() ? "" : ids.length > TRACKER_OPEN_MAX ? disabledItem("external", openMany, t("grid.bulk.max", { n: TRACKER_OPEN_MAX })) : menuItem("grid-bulk-tracker-open", "external", openMany, { ids: JSON.stringify(ids) }),
    ].join("");
    out.push(subMenu("tracker", "cloud", source || t("grid.bulk.tracker"), trk));
  }

  // Abrir y copiar
  const openIn = [
    n > EXPLORER_MAX ? disabledItem("folder", t("grid.bulk.explorer"), t("grid.bulk.max", { n: EXPLORER_MAX })) : menuItem("grid-bulk-explorer", "folder", t("grid.bulk.explorer")),
    one ? menuItem("grid-bulk-vscode", "code", t("detail.vscode")) : "",
  ].join("");
  const copy = menuItem("grid-bulk-copy", "copy", t("grid.bulk.copyNames"), { what: "names" }) + (ids.length ? menuItem("grid-bulk-copy", "hash", t("grid.bulk.copyIds"), { what: "ids" }) : "");
  out.push(sepHtml, subMenu("open", "external", t("grid.bulk.openIn"), openIn), subMenu("copy", "copy", t("grid.bulk.copy"), copy));
  out.push(sepHtml, menuItem("bulk-trash", "trash", n > 1 ? t("grid.bulk.deleteN", { n }) : `${t("items.delete")}…`, {}, hint(t("grid.bulk.keyDel"))));
  out.push(menuItem("bulk-clear", "x", t("items.clearSelection"), {}, hint("Esc")));
  return out.join("");
}

function popHtml(): string {
  if (!pop) return "";
  if (pop.kind === "filters") return filtersMenu();
  if (pop.kind === "columns") return columnsMenu();
  if (pop.kind === "view") return viewMenu(pop.arg);
  return ctxMenu(pop.arg);
}

function showPop(): void {
  let el = $("#grid-pop");
  if (!pop || S.view !== "items" || S.detail) {
    if (el) el.hidden = true;
    pop = null;
    return;
  }
  if (!el) {
    el = document.createElement("div");
    el.id = "grid-pop";
    el.className = "menu";
    el.setAttribute("role", "menu");
    document.body.append(el);
  }
  const html = popHtml();
  if (!html) {
    closePop();
    return;
  }
  const keep = el.scrollTop;
  el.innerHTML = html;
  el.hidden = false;
  el.scrollTop = keep;
  const w = el.offsetWidth;
  const h = el.offsetHeight;
  const x = pop.right ? pop.x - w : pop.x;
  el.style.left = `${Math.max(8, Math.min(x, innerWidth - w - 8))}px`;
  el.style.top = `${Math.max(8, Math.min(pop.y, innerHeight - h - 8))}px`;
}

function openPop(kind: PopKind, x: number, y: number, arg = "", right = false): void {
  pop = { kind, x, y, arg, right };
  subOpen.clear();
  showPop();
}

function closePop(): void {
  pop = null;
  const el = $("#grid-pop");
  if (el) el.hidden = true;
}

onAfterRender(showPop);
document.addEventListener("mousedown", (ev) => {
  const target = ev.target as HTMLElement;
  if (pop && !target.closest("#grid-pop") && !target.closest('[data-action="grid-pop"]')) closePop();
});
document.addEventListener("keydown", (ev) => {
  if (ev.key === "Escape" && pop) {
    ev.stopPropagation();
    closePop();
  }
}, true);
// Una acción del menú lo cierra (después de correr: el registro de acciones va antes).
document.addEventListener("click", (ev) => {
  const hit = (ev.target as HTMLElement).closest<HTMLElement>("#grid-pop .menu-item, #grid-pop .menu-btns .btn");
  if (hit && !hit.classList.contains("has-sub")) closePop();
});
// Click derecho: en una fila, menú de la selección; en una pestaña, el de la vista.
document.addEventListener("contextmenu", (ev) => {
  if (S.view !== "items" || S.detail) return;
  const target = ev.target as HTMLElement;
  const tab = target.closest<HTMLElement>('.vtab[data-action="grid-view"]');
  const row = target.closest<HTMLElement>('tr.row[data-action="grid-row"]');
  if (!tab && !row) return;
  ev.preventDefault();
  if (tab) return openPop("view", ev.clientX, ev.clientY, tab.dataset.id!);
  const name = row!.dataset.name!;
  if (!G.selected.has(name)) {
    G.selected.clear();
    G.selected.add(name);
    G.anchor = name;
    render();
  }
  openPop("ctx", ev.clientX, ev.clientY);
});

// ---------- ancho de columnas (arrastrar el borde del encabezado) ----------

let drag: { id: string; x: number; w: number; col: HTMLElement | null } | null = null;
document.addEventListener("mousedown", (ev) => {
  const rs = (ev.target as HTMLElement).closest<HTMLElement>(".col-rs");
  if (!rs) return;
  ev.preventDefault();
  ev.stopPropagation();
  const th = rs.parentElement!;
  drag = { id: rs.dataset.rs!, x: ev.clientX, w: th.getBoundingClientRect().width, col: document.querySelector<HTMLElement>(`col[data-col="${rs.dataset.rs}"]`) };
  document.body.classList.add("resizing");
});
document.addEventListener("mousemove", (ev) => {
  if (!drag?.col) return;
  drag.col.style.width = `${Math.max(60, Math.round(drag.w + ev.clientX - drag.x))}px`;
});
document.addEventListener("mouseup", (ev) => {
  if (!drag) return;
  const { id, w, x } = drag;
  drag = null;
  document.body.classList.remove("resizing");
  if (ev.clientX !== x) setColumnWidth(id, w + ev.clientX - x);
});
// El click que termina un arrastre no ordena.
document.addEventListener("click", (ev) => {
  if ((ev.target as HTMLElement).closest(".col-rs")) {
    ev.stopPropagation();
    ev.preventDefault();
  }
}, true);

// ---------- nuevo item ----------

/** Slug como `wb_core::text::slugify` (y la v1): minúsculas ASCII, guiones, 40 caracteres. */
const slugOf = (text: string): string =>
  text.normalize("NFKD").replace(/[^\x00-\x7f]/g, "").toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 40).replace(/-+$/, "") || "wi";

/** Nuevo item (v1 «Nuevo item»): con id del tracker (Traer completa título, tipo, descripción y criterios) o sin
 * id (el arreglo del proyecto: tipo y carpeta del preset). «Crear y planear» lo crea en el estado de
 * planificación y abre la sesión de Planear; «Solo cargar» lo deja en el estado inicial. */
function newItemModal(): void {
  const rules = wf().newItem;
  const types = wf().types.map((x) => `<option value="${esc(x.id)}">${esc(label(x.labels, x.id))}</option>`).join("");
  const back = document.createElement("div");
  back.className = "modal-back";
  back.innerHTML = `<form class="modal" role="dialog" aria-modal="true"><h2>${esc(t("items.new"))}</h2>
    ${trackerConnected() ? `<p class="muted"><button type="button" class="link" data-assigned>${ico("calendar")} ${esc(t("tracker.assigned.fromNewItem"))}</button></p>` : ""}
    <label>${esc(t("newItem.id"))}<span class="add-row"><input name="id" placeholder="${esc(t("newItem.idHint"))}"><button type="button" class="btn" data-fetch title="${esc(t("tracker.fetchHint"))}">${ico("cloud")}${esc(t("tracker.fetch"))}</button></span></label>
    <p class="muted" data-fetched></p>
    <label>${esc(t("items.title"))}<input name="title"></label>
    <label>${esc(t("items.type"))}<select name="type"><option value=""></option>${types}</select></label>
    <label>${esc(t("newItem.folder"))}<input name="folder"></label>
    <label><span data-desc-label></span><textarea name="description" rows="6"></textarea></label>
    <label data-criteria>${esc(t("newItem.criteria"))}<textarea name="criteria" rows="4"></textarea></label>
    <p class="error" data-error></p>
    <div class="modal-actions"><button type="button" class="btn" data-cancel>${esc(t("common.cancel"))}</button>
      <button type="submit" class="btn" data-mode="pending" title="${esc(t("newItem.pendingHint"))}">${ico("plus")}${esc(t("newItem.pending"))}</button>
      <button type="submit" class="btn primary" data-mode="plan" title="${esc(t("newItem.planHint"))}">${ico("plan")}${esc(t("newItem.plan"))}</button></div></form>`;
  document.body.append(back);
  const form = back.querySelector("form")!;
  const field = (n: string) => form.elements.namedItem(n) as HTMLInputElement;
  const typeSel = form.elements.namedItem("type") as HTMLSelectElement;
  const idOf = () => field("id").value.trim().replace(/^#/, "");
  let touched = false;
  let typeTouched = false;
  // Sin id: el tipo «arreglo» del preset, la descripción como «qué hay que hacer» y sin criterios.
  const syncMode = () => {
    const id = idOf();
    if (!typeTouched) typeSel.value = id ? (fetched?.localType ?? "") : rules.defaultType;
    back.querySelector("[data-desc-label]")!.textContent = t(id ? "newItem.description" : "newItem.fixContext");
    back.querySelector<HTMLElement>("[data-criteria]")!.hidden = !id;
  };
  const suggest = () => {
    if (touched) return;
    const title = field("title").value.trim();
    const id = idOf();
    const slug = title ? slugOf(title) : "";
    const tpl = id ? wf().folders.newItemTemplate : title ? rules.folderTemplate || "{slug}" : "";
    field("folder").value = tpl.replace("{id}", id).replace("{type}", typeSel.value).replace("{slug}", slug).replace(/^-+|-+$/g, "");
  };
  field("title").oninput = suggest;
  field("folder").oninput = () => (touched = true);
  typeSel.onchange = () => {
    typeTouched = true;
    suggest();
  };
  const close = () => back.remove();
  // Con id: el tracker completa título, tipo y carpeta, y la descripción y los criterios (editables).
  let fetched: TrackerItem | null = null;
  const info = back.querySelector<HTMLElement>("[data-fetched]")!;
  field("id").oninput = () => {
    if (fetched && fetched.id !== idOf()) {
      fetched = null;
      info.textContent = "";
    }
    syncMode();
    suggest();
  };
  back.querySelector<HTMLElement>("[data-fetch]")!.onclick = async () => {
    const id = field("id").value.trim();
    if (!id) return;
    info.textContent = t("common.loading");
    try {
      const d = await fetchTrackerItem(id);
      fetched = d.fromTracker ? d : null;
      if (d.fromTracker) {
        field("title").value = d.title;
        typeSel.value = d.localType;
        typeTouched = false;
        (form.elements.namedItem("description") as HTMLTextAreaElement).value = d.description;
        (form.elements.namedItem("criteria") as HTMLTextAreaElement).value = d.criteria;
        if (!touched) field("folder").value = d.folder;
      }
      const have = d.existing.length ? ` · ${t("tracker.alreadyIn", { folders: d.existing.join(", ") })}` : "";
      const manual = d.error ? t("tracker.fetchManual", { reason: errorText(d.error) }) : t("tracker.fetchOffline");
      info.textContent = (d.fromTracker ? t("tracker.fetched", { type: d.type, state: d.state, source: d.source }) : manual) + have;
    } catch (err) {
      fetched = null;
      info.textContent = errorText(err);
    }
  };
  back.querySelector<HTMLElement>("[data-assigned]")?.addEventListener("click", () => {
    close();
    openAssigned();
  });
  back.querySelector<HTMLElement>("[data-cancel]")!.onclick = close;
  back.addEventListener("keydown", (e) => e.key === "Escape" && close());
  form.onsubmit = async (e) => {
    e.preventDefault();
    const btn = (e.submitter as HTMLButtonElement | null) ?? form.querySelector<HTMLButtonElement>("[data-mode=plan]")!;
    const plan = btn.dataset.mode === "plan";
    const error = back.querySelector<HTMLElement>("[data-error]")!;
    const id = idOf();
    const title = field("title").value.trim();
    if (!id && !title) {
      error.textContent = t("newItem.titleRequired");
      field("title").focus();
      return;
    }
    const text = (n: string) => (form.elements.namedItem(n) as HTMLTextAreaElement).value;
    const { cols, rows } = termSize();
    form.querySelectorAll<HTMLButtonElement>("[type=submit]").forEach((b) => (b.disabled = true));
    try {
      const base = { title, type: typeSel.value, folder: field("folder").value.trim(), description: text("description"), plan, cols, rows };
      const out = id
        ? await createTrackedItem({ ...base, id, criteria: text("criteria"), fetched })
        : await api.createItem(base);
      close();
      await refresh();
      toast(t(out.session ? "newItem.createdPlan" : "newItem.created", { folder: out.item.name }));
      if (out.session) await launched(out.session);
      else await openDetail(out.item.name);
    } catch (err) {
      form.querySelectorAll<HTMLButtonElement>("[type=submit]").forEach((b) => (b.disabled = false));
      error.textContent = errorText(err);
    }
  };
  syncMode();
  field("id").focus();
}

// ---------- eliminados ----------

/** Eliminados: restaurar vuelve la carpeta con su nombre original (sin el sufijo de fecha) y el modal queda
 *  abierto con la lista al día (v1 app.js:2128-2149). */
async function deletedModal(): Promise<void> {
  const back = document.createElement("div");
  back.className = "modal-back";
  const close = () => back.remove();
  const draw = async () => {
    const list = await api.listDeleted();
    back.innerHTML = `<div class="modal" role="dialog" aria-modal="true"><h2>${esc(t("items.deleted"))}</h2>
    <p class="muted">${esc(t("items.deletedIntro", { dir: wf().folders.deletedDir }))}</p>
    <ul class="list">${list
      .map((d) => `<li>${d.group ? ico("folder") : `<span class="dot ${stateDef(d.status)?.category ?? "unknown"}"></span>`}<span class="grow"><b>${esc(d.title || d.base)}</b> <span class="muted">${esc(d.base)} · ${esc(fmtAgo(d.deleted))}${d.status ? ` · ${esc(stateLabel(d.status))}` : ""}</span></span>
        <button class="btn" data-restore="${esc(d.path)}" data-base="${esc(d.original)}">${ico("undo")}${esc(t("items.restore"))}</button></li>`)
      .join("") || `<li class="empty">${esc(t("items.noDeleted"))}</li>`}</ul>
    <div class="modal-actions"><button class="btn" data-close>${esc(t("common.close"))}</button></div></div>`;
    back.querySelector<HTMLElement>("[data-close]")!.onclick = close;
    back.querySelectorAll<HTMLElement>("[data-restore]").forEach((b) => {
      b.onclick = async () => {
        const res = await api.restoreDeleted([{ to: b.dataset.restore!, name: b.dataset.base! }]);
        if (res.errors.length) return toast(failures(res.errors), undefined, "error");
        await refresh();
        toast(t("items.restored", { name: b.dataset.base! }));
        await draw();
      };
    });
  };
  back.addEventListener("click", (e) => {
    if (e.target === back) close();
  });
  back.addEventListener("keydown", (e) => e.key === "Escape" && close());
  await draw();
  document.body.append(back);
}

// ---------- acciones en lote (con deshacer) ----------

const failures = (errs: TrackerFailure[]) => errs.map((e) => `${e.name ?? e.id}: ${errorText(e.error)}`).join(" · ");

/** Toast de una acción en lote: «qué · N carpetas», los errores por carpeta y Deshacer si se hizo algo. */
function bulkToast(what: string, done: number, errors: TrackerFailure[], undo: (() => Promise<void>) | null): void {
  const errs = errors.length ? ` · ${t("grid.bulk.errors", { n: errors.length, list: failures(errors) })}` : "";
  toast(t("grid.bulk.done", { what, n: done }) + errs, done && undo ? { label: t("common.undo"), run: () => void undo().catch((err) => toast(errorText(err), undefined, "error")) } : undefined, errors.length ? "error" : "info");
}

/** Mismos campos (y ambientes a agregar o quitar) en varias carpetas, en el backend, con deshacer (v1
 *  `bulkApply`). Un valor inválido no escribe nada; un error de una carpeta no corta el resto. */
async function bulkApply(names: string[], fields: Record<string, MetaValue>, what: string, envs: { add?: string[]; remove?: string[] } = {}, undo = true): Promise<void> {
  if (!names.length) return;
  try {
    const res = await api.bulkUpdate(names, fields, envs.add ?? [], envs.remove ?? []);
    await refresh();
    bulkToast(what, res.done.length, res.errors, undo && res.before.length ? () => undoBulk(res.before) : null);
  } catch (err) {
    toast(errorText(err), undefined, "error");
  }
}

/** Deshace un `bulkApply`: los valores previos se escriben tal cual (sin validar). */
async function undoBulk(before: MetaBefore[]): Promise<void> {
  const res = await api.restoreMeta(before);
  await refresh();
  toast(t("grid.undone", { n: res.done.length }) + (res.errors.length ? ` · ${failures(res.errors)}` : ""), undefined, res.errors.length ? "error" : "info");
}

/** Mueve carpetas a un grupo (o a la raíz con `""`) con deshacer: cada una vuelve a donde estaba. */
async function moveItems(names: string[], dest: string, what: string): Promise<void> {
  if (!names.length || (await busyGuard(names))) return;
  const moved: [string, string][] = [];
  const errors: TrackerFailure[] = [];
  for (const n of names) {
    try {
      const now = await api.moveFolder(n, dest);
      if (now !== n) moved.push([now, itemByName(n)?.group ?? ""]);
    } catch (err) {
      errors.push({ name: n, error: err as TrackerFailure["error"] });
    }
  }
  G.selected.clear();
  await refresh();
  bulkToast(what, moved.length, errors, async () => {
    for (const [now, old] of moved) await api.moveFolder(now, old);
    await refresh();
    toast(t("grid.undone", { n: moved.length }));
  });
}

/** Manda la selección a eliminados (los items de un grupo que también se elimina van con él), con deshacer. */
async function trashItems(names: string[]): Promise<void> {
  if (await busyGuard(names)) return;
  const groups = names.filter((n) => itemByName(n)?.kind === "group").length;
  const text = t("items.confirmDelete", { n: names.length }) + (groups ? `\n${t("items.confirmDeleteGroups")}` : "");
  if (!(await confirmDialog(text, t("items.delete")))) return;
  const res = await api.deleteItems(names);
  G.selected.clear();
  await refresh();
  bulkToast(t("items.delete"), res.done.length, res.errors, async () => {
    const back = await api.restoreDeleted(res.done);
    await refresh();
    toast(t("grid.undone", { n: back.done.length }) + (back.errors.length ? ` · ${failures(back.errors)}` : ""), undefined, back.errors.length ? "error" : "info");
  });
}

/** Valor de un campo para la selección (vacío = quitar). Con una sola carpeta, se propone el que tiene. */
async function askField(key: string, names: string[]): Promise<void> {
  const f = wf().fields;
  const shown: Record<string, string> = { [f.due]: t("detail.due"), [f.period]: t("detail.period"), [f.hours]: t("detail.hours"), [f.notes]: t("items.notes"), [f.branch]: t("grid.col.branch") };
  const hints: Record<string, string> = { [f.due]: "grid.bulk.askDue", [f.period]: "grid.bulk.askPeriod", [f.hours]: "grid.bulk.askHours" };
  const one = names.length === 1 ? itemByName(names[0]) : null;
  const what = shown[key] ?? key;
  const text = t(hints[key] ?? "grid.bulk.askText", { what, n: names.length, day: wf().period.startDay });
  const value = await promptDialog(text, one ? metaText(one, key) : "", true);
  if (value === null) return;
  await bulkApply(names, { [key]: value.trim() }, what);
}

// ---------- acciones ----------

registerView("items", renderItems);

onAction("open-item", (el, ev) => {
  if ((ev.target as HTMLElement).closest("input")) return;
  return openDetail(el.dataset.name!);
});
onAction("grid-row", (el, ev) => {
  const e = ev as MouseEvent;
  const target = e.target as HTMLElement;
  if (target.closest("input, button")) return;
  const name = el.dataset.name!;
  if (e.shiftKey) return selectRange(name);
  if (e.ctrlKey || e.metaKey) return toggleSelect(name);
  return openDetail(name);
});
onAction("select-item", (el, ev) => {
  ev.stopPropagation();
  if ((ev as MouseEvent).shiftKey) return selectRange(el.dataset.name!);
  toggleSelect(el.dataset.name!);
});
onAction("bulk-clear", () => clearSelection());
onAction("grid-sort", (el) => cycleSort(el.dataset.col!));
onAction("grid-group", (el) => toggleGroup(el.dataset.key!, Number(el.dataset.depth)));
onAction("grid-reload", () => refreshAll());
onAction("grid-folder", (el, ev) => {
  ev.stopPropagation();
  toggleFlip(`f:${el.dataset.name}`);
});
onAction("grid-pop", (el) => {
  const kind = el.dataset.kind as PopKind;
  if (pop?.kind === kind) return closePop();
  const r = el.getBoundingClientRect();
  if (kind === "ctx") return openPop(kind, r.left, r.top - 8 - Math.min(480, innerHeight / 2));
  openPop(kind, kind === "columns" ? r.right : r.left, r.bottom + 4, "", kind === "columns");
});
onAction("grid-chip", (el) => clearChip(el.dataset.k!));
onAction("grid-density", (el) => update((s) => (s.density = el.dataset.d === "compact" ? "compact" : "comfortable")));
onAction("grid-col-move", (el) => moveColumn(el.dataset.col!, Number(el.dataset.d)));
onAction("grid-col-reset", () => resetColumns());
onAction("grid-export", async () => {
  const name = `${t("nav.items")}${activeView() ? `-${activeView()!.name}` : ""}.csv`;
  await exportCsv(name, toCsv(exportRows()));
});
onAction("grid-view", (el) => switchSavedView(el.dataset.id!));
onAction("grid-view-save", () => {
  saveView();
  toast(t("grid.view.saved", { name: activeView()?.name ?? "" }));
});
onAction("grid-view-discard", () => discardView());
onAction("grid-view-new", async () => {
  const name = await promptDialog(t("grid.view.newPrompt"));
  if (name) newView(name);
});
onAction("grid-view-rename", async (el) => {
  const v = G.prefs.views.find((x) => x.id === el.dataset.id);
  if (!v) return;
  const name = await promptDialog(t("grid.view.rename"), v.name);
  if (name) renameView(v.id, name);
});
onAction("grid-view-move", (el) => moveView(el.dataset.id!, Number(el.dataset.d)));
onAction("grid-view-delete", async (el) => {
  const v = G.prefs.views.find((x) => x.id === el.dataset.id);
  if (!v) return;
  const last = G.prefs.views.length === 1;
  if (await confirmDialog(t(last ? "grid.view.confirmDeleteLast" : "grid.view.confirmDelete", { name: v.name }), t("grid.view.delete"))) deleteView(v.id);
});

onAction("new-item", () => newItemModal());
onAction("bulk-promote", () => openPromotion(selectedNames()));
onAction("show-deleted", () => deletedModal());
onAction("bulk-reflect", () => reflectItems(selectedNames()));
onAction("grid-sub", (el) => {
  const k = el.dataset.k!;
  if (subOpen.has(k)) subOpen.delete(k);
  else subOpen.add(k);
  showPop();
});
onAction("bulk-group", async () => {
  // Los de otro grupo también (se reagrupan, como la v1); los grupos no se anidan.
  const items = selectedNames().map(itemByName).filter((it): it is Item => it?.kind === "item");
  if (!items.length) return toast(t("group.pickItems"), undefined, "error");
  const names = items.map((it) => it.name);
  if (await busyGuard(names)) return;
  // Título propuesto como la v1: «WI a + WI b» (o el nombre de la carpeta si no tiene id).
  const suggested = items.map((it) => (it.ids[0] ? t("session.itemLabel", { id: it.ids[0] }) : it.name.split("/").pop()!)).join(" + ").slice(0, 60);
  const title = await promptDialog(t("items.groupTitle"), suggested);
  if (title === null) return;
  if (!title.trim()) return toast(t("group.titleRequired"), undefined, "error");
  const res = await api.createGroup(title, names);
  G.selected.clear();
  await refresh();
  const errors = res.errors.length ? ` · ${t("grid.bulk.errors", { n: res.errors.length, list: failures(res.errors) })}` : "";
  toast(t("items.grouped", { n: res.members.length }) + errors, {
    label: t("common.undo"),
    run: async () => {
      for (const m of res.members) await api.moveFolder(m, "");
      await api.trash(res.group);
      await refresh();
      toast(t("grid.undone", { n: res.members.length }));
    },
  });
});
onAction("bulk-trash", () => deleteSelection());
onAction("grid-bulk-set", (el) => bulkApply(selectedNames(), { [el.dataset.key!]: el.dataset.value ?? "" }, el.dataset.what ?? ""));
onAction("grid-bulk-env", (el) => {
  const env = el.dataset.env!;
  const add = el.dataset.mode === "add";
  return bulkApply(selectedNames(), {}, t(add ? "grid.bulk.envAdded" : "grid.bulk.envRemoved", { env: envLabel(env) }), add ? { add: [env] } : { remove: [env] });
});
onAction("grid-bulk-plan", (el) => {
  const r = el.getBoundingClientRect();
  openPop("ctx", r.left, r.top - 8 - Math.min(320, innerHeight / 2), "plan");
});
onAction("grid-bulk-revert", () => {
  const f = wf().fields;
  return bulkApply(selectedNames(), { [f.due]: "", [f.period]: "", [f.hours]: "" }, t("detail.planning"));
});
onAction("grid-bulk-text", (el) => askField(el.dataset.key!, selectedNames()));
onAction("grid-bulk-readme", () => {
  const names = selectedNames().filter((n) => itemByName(n)?.hasReadme === false);
  return bulkApply(names, {}, t("grid.bulk.readmeCreated"), {}, false);
});
onAction("grid-bulk-rename", async () => {
  const name = selectedNames()[0];
  if (!name || (await busyGuard([name]))) return;
  const base = name.split("/").pop()!;
  const next = await promptDialog(t("grid.bulk.rename"), base);
  if (!next || next === base) return;
  const now = await api.renameFolder(name, next);
  G.selected.clear();
  await refresh();
  toast(t("grid.bulk.renamed", { name: now.split("/").pop()! }), {
    label: t("common.undo"),
    run: async () => {
      await api.renameFolder(now, base);
      await refresh();
    },
  });
});
onAction("grid-bulk-move", (el) => {
  const dest = el.dataset.dest!;
  const group = itemByName(dest);
  const names = selectedNames().filter((n) => itemByName(n)?.kind === "item" && itemByName(n)?.group !== dest);
  return moveItems(names, dest, t("grid.bulk.movedTo", { group: group ? titleOf(group) : dest }));
});
onAction("grid-bulk-ungroup", () => moveItems(selectedNames().filter((n) => itemByName(n)?.group), "", t("grid.bulk.ungroup")));
onAction("grid-bulk-dissolve", async () => {
  const it = itemByName(selectedNames()[0]);
  if (it?.kind !== "group") return;
  G.selected.clear();
  await dissolveGroup(it);
});
onAction("grid-bulk-open", () => openDetail(selectedNames()[0]));
onAction("grid-bulk-planone", () => {
  const it = itemByName(selectedNames()[0]);
  if (it) return planWithConfirm(it);
});
onAction("grid-bulk-review", () => {
  const it = itemByName(selectedNames()[0]);
  if (it) openReview(it);
});
onAction("grid-bulk-sync", () => {
  const items = expandGroups(selectedNames().map(itemByName).filter(Boolean) as Item[]);
  return syncSelection(items.filter((it) => it.ids.length).map((it) => it.name));
});
onAction("grid-bulk-tracker-open", async (el) => {
  const ids = JSON.parse(el.dataset.ids || "[]") as string[];
  for (const id of ids.slice(0, TRACKER_OPEN_MAX)) await trackerApi.open(id).catch((err) => toast(errorText(err), undefined, "error"));
});
onAction("grid-bulk-session", () => newSessionModal(selectedNames()));
onAction("grid-bulk-addsession", () => addToSessionModal(selectedNames()));
onAction("grid-bulk-explorer", async () => {
  for (const n of selectedNames().slice(0, EXPLORER_MAX)) await api.openPath(n).catch((err) => toast(errorText(err), undefined, "error"));
});
onAction("grid-bulk-vscode", async () => {
  await api.openInVscode(selectedNames()[0]).catch((err) => toast(errorText(err), undefined, "error"));
});
onAction("grid-bulk-copy", async (el) => {
  const items = selectedNames().map(itemByName).filter(Boolean) as Item[];
  const ids = el.dataset.what === "ids";
  const list = ids ? [...new Set(expandGroups(items).flatMap((i) => i.ids))] : items.map((i) => i.name);
  try {
    await navigator.clipboard.writeText(list.join(ids ? ", " : "\n"));
    toast(t(ids ? "grid.bulk.copiedIds" : "grid.bulk.copiedNames", { n: list.length }));
  } catch {
    toast(t("grid.bulk.copyFailed"), undefined, "error");
  }
});

onChange("bulk-status", async (el) => {
  const v = (el as HTMLSelectElement).value;
  if (!v) return;
  // "=" + id cambia al estado; "!" es «Sin estado» (borra la clave).
  await bulkApply(selectedNames(), { [wf().fields.status]: v === "!" ? "" : v.slice(1) }, t("items.status"));
});
onChange("grid-select-all", (el) => {
  if ((el as HTMLInputElement).checked) selectAllVisible();
  else {
    for (const n of G.order) G.selected.delete(n);
    render();
  }
});
onChange("grid-group", (el) => setGroupLevel(Number(el.dataset.level), (el as HTMLSelectElement).value));
onChange("grid-f-status", (el) => {
  const id = el.dataset.id!;
  update((s) => (s.status = (el as HTMLInputElement).checked ? [...s.status, id] : s.status.filter((x) => x !== id)));
});
onChange("grid-f-closed", (el) => update((s) => (s.hideClosed = (el as HTMLInputElement).checked)));
onChange("grid-f-type", (el) => update((s) => (s.type = (el as HTMLSelectElement).value)));
onChange("grid-f-due", (el) => update((s) => (s.due = (el as HTMLSelectElement).value as typeof s.due)));
onChange("grid-f-activity", (el) => update((s) => (s.activity = (el as HTMLSelectElement).value)));
onChange("grid-col-on", (el) => toggleColumn(el.dataset.col!, (el as HTMLInputElement).checked));
