// Trabajos: la grilla de items con vistas guardadas (pestañas), búsqueda, filtros con chips, agrupación en
// hasta 3 niveles, columnas configurables (orden, ancho), densidad, exportar a CSV, selección múltiple con
// barra y menú contextual en lote, y deshacer. El modelo (columnas, filtros, vistas) está en `grid.ts`.

import { api } from "../api";
import { onAction, onAfterRender, onChange, openDetail, refresh, registerView, render } from "../app";
import { $, confirmDialog, daysUntil, esc, fmtAgo, promptDialog, toast } from "../dom";
import { errorText, label, t } from "../i18n";
import { ico } from "../icons";
import { hasSessions, reflectItems } from "../reflect";
import { addToSessionModal, newSessionModal } from "../sessions";
import { S, effectiveStatus, isClosed, itemByName, metaList, metaText, titleOf, wf } from "../state";
import { exportCsv, toCsv } from "../stats";
import { dueOf } from "../planning";
import type { Item, MetaValue, TrackerItem } from "../types";
import { createTrackedItem, fetchTrackerItem, openAssigned, trackerBulk, trackerConnected, trackerToolbar } from "./tracker";
import { openPromotion } from "../git";
import { pluginBulkButtons } from "../plugins";
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
  cycleSort,
  deleteView,
  discardView,
  ensurePrefs,
  exportRows,
  filterChips,
  folderOpen,
  groupOpen,
  groupTree,
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
  toggleColumn,
  toggleFlip,
  update,
  viewDirty,
  visibleColumns,
  visibleRows,
} from "./grid";

export { switchSavedView };

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
      <div class="search">${ico("search")}<input id="items-search" type="search" placeholder="${esc(t("items.search"))}" value="${esc(G.text)}"></div>
      <button class="btn${chips.length ? " has" : ""}" data-action="grid-pop" data-kind="filters" aria-haspopup="true">${ico("filter")}${esc(t("grid.filters"))}${chips.length ? `<b class="fcount">${chips.length}</b>` : ""}${ico("chevronDown")}</button>
      ${groupSelects()}
      <span class="spacer"></span>
      ${S.deletedCount ? `<button class="btn ghost" data-action="show-deleted" title="${esc(t("items.deleted"))}">${ico("trash")}${S.deletedCount}</button>` : ""}
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
  return `<tr class="row ${cls}${sel ? " sel" : ""}" data-action="grid-row" data-name="${esc(it.name)}">
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
  const open = groupOpen(g);
  const count = g.rows.reduce((n, r) => n + (r.item.kind === "group" ? r.shown.length || 1 : 1), 0);
  const head = `<tr class="group-head d${depth}${g.cls ? ` ${g.cls}` : ""}${open ? " open" : ""}" data-action="grid-group" data-key="${esc(g.key)}">
    <td colspan="${cols.length + 1}"><span class="gh" style="padding-left:${depth * 20}px">${ico("chevronRight", "tw")}<span class="gt">${g.title}</span>${g.sub ? `<span class="gs">${g.sub}</span>` : ""}<span class="gc">${count}</span></span></td></tr>`;
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
      : `${esc(t("items.noMatches"))} <button class="link" data-action="grid-chip" data-k="all">${esc(t("grid.clearFilters"))}</button>`;
    return `<p class="empty">${empty}</p>`;
  }
  const body = groupTree(rows, snap().groupBy)
    .map((g) => groupHtml(g, 0, cols))
    .join("");
  const allSel = G.order.length > 0 && G.order.every((n) => G.selected.has(n));
  const minW = 34 + cols.reduce((n, c) => n + (c.width || 260), 0);
  const head = cols
    .map((c) => {
      const arrow = sort?.id === c.id ? ico("chevronDown", sort.dir > 0 ? "flip" : "") : "";
      return `<th class="gc-${c.id}" aria-sort="${sort?.id === c.id ? (sort.dir > 0 ? "ascending" : "descending") : "none"}"><button class="th" data-action="grid-sort" data-col="${c.id}">${esc(t(c.label))}${arrow}</button><span class="col-rs" data-rs="${c.id}" title="${esc(t("grid.resize"))}"></span></th>`;
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
  const states = wf().states.map((s) => `<option value="${esc(s.id)}">${esc(label(s.labels, s.id))}</option>`).join("");
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
      (c) => `<div class="menu-check"><label class="grow"><input type="checkbox" data-change="grid-col-on" data-col="${c.id}"${on.has(c.id) ? " checked" : ""}${c.fixed ? " disabled" : ""}> ${esc(t(c.label))}</label>
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

/** Menú de la selección: lo que no aplica no aparece. */
function ctxMenu(): string {
  const names = selectedNames();
  if (!names.length) return "";
  const items = names.map(itemByName).filter(Boolean) as Item[];
  const f = wf().fields;
  const one = items.length === 1 ? items[0] : null;
  const out: string[] = [head(one ? titleOf(one) : t("items.selected", { n: items.length }))];
  out.push(menuItem("grid-bulk-session", "terminal", t("grid.bulk.newSession")), menuItem("grid-bulk-addsession", "plus", t("grid.bulk.addSession")));
  if (hasSessions(names)) out.push(menuItem("bulk-reflect", "sparkle", t("reflect.action")));
  out.push(sepHtml, head(t("items.status")));
  for (const st of wf().states) out.push(menuItem("grid-bulk-field", "state", label(st.labels, st.id), { key: f.status, value: st.id }, `<span class="dot ${st.category}"></span>`));
  out.push(sepHtml, head(t("items.type")));
  for (const ty of wf().types) out.push(menuItem("grid-bulk-field", "tag", label(ty.labels, ty.id), { key: f.type, value: ty.id }));
  out.push(menuItem("grid-bulk-field", "eraser", t("items.noType"), { key: f.type, value: "" }));
  if (wf().environments.length) {
    out.push(sepHtml, head(t("grid.col.envs")));
    for (const e of wf().environments) {
      const all = items.every((it) => metaList(it, f.environments).includes(e.id));
      out.push(menuItem("grid-bulk-env", all ? "check" : "plus", label(e.labels, e.id), { env: e.id }, all ? `<span class="muted">${esc(t("grid.bulk.envRemove"))}</span>` : ""));
    }
  }
  out.push(sepHtml, head(t("detail.planning")), menuItem("grid-bulk-plan", "calendar", t("grid.bulk.planningEdit")));
  if (items.some((it) => metaText(it, f.due) || metaText(it, f.period) || metaText(it, f.hours)))
    out.push(menuItem("grid-bulk-revert", "undo", t(trackerConnected() ? "grid.bulk.revertTracker" : "grid.bulk.clearPlanning")));
  out.push(sepHtml, head(t("grid.bulk.edit")), menuItem("grid-bulk-text", "edit", t("items.notes"), { key: f.notes }), menuItem("grid-bulk-text", "branch", t("grid.col.branch"), { key: f.branch }));
  if (one) out.push(menuItem("grid-bulk-rename", "edit", t("grid.bulk.rename")));
  const loose = items.filter((it) => it.kind === "item" && !it.group);
  const inGroup = items.filter((it) => it.group);
  const groups = S.items.filter((it) => it.kind === "group");
  out.push(sepHtml, head(t("items.group")));
  if (loose.length) out.push(menuItem("bulk-group", "layers", t("grid.bulk.groupNew")));
  if (groups.length && items.some((it) => it.kind === "item")) out.push(menuItem("grid-bulk-move", "folderIn", t("grid.bulk.moveTo")));
  if (inGroup.length) out.push(menuItem("grid-bulk-ungroup", "folderOut", t("grid.bulk.ungroup")));
  out.push(sepHtml, head(t("grid.bulk.git")), menuItem("bulk-promote", "promote", t("promo.button")));
  const trk = trackerBulk(names);
  const plugins = pluginBulkButtons(names);
  if (trk || plugins) out.push(sepHtml, head(t("grid.bulk.integrations")), `<div class="menu-btns">${trk}${plugins}</div>`);
  out.push(sepHtml, head(t("grid.bulk.openIn")));
  if (one) out.push(menuItem("grid-bulk-reveal", "folder", t("grid.bulk.reveal")));
  out.push(menuItem("grid-bulk-copy", "copy", t("grid.bulk.copyNames"), { what: "names" }));
  if (items.some((it) => it.ids.length)) out.push(menuItem("grid-bulk-copy", "hash", t("grid.bulk.copyIds"), { what: "ids" }));
  out.push(sepHtml, menuItem("bulk-trash", "trash", t("items.delete")));
  return out.join("");
}

function popHtml(): string {
  if (!pop) return "";
  if (pop.kind === "filters") return filtersMenu();
  if (pop.kind === "columns") return columnsMenu();
  if (pop.kind === "view") return viewMenu(pop.arg);
  return ctxMenu();
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
  if ((ev.target as HTMLElement).closest("#grid-pop .menu-item, #grid-pop .menu-btns .btn")) closePop();
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

function newItemModal(): void {
  const types = wf().types.map((x) => `<option value="${esc(x.id)}">${esc(label(x.labels, x.id))}</option>`).join("");
  const back = document.createElement("div");
  back.className = "modal-back";
  back.innerHTML = `<form class="modal" role="dialog" aria-modal="true"><h2>${esc(t("items.new"))}</h2>
    ${trackerConnected() ? `<p class="muted"><button type="button" class="link" data-assigned>${ico("calendar")} ${esc(t("tracker.assigned.fromNewItem"))}</button></p>` : ""}
    <label>${esc(t("newItem.id"))}<span class="add-row"><input name="id" placeholder="${esc(t("newItem.idHint"))}"><button type="button" class="btn" data-fetch title="${esc(t("tracker.fetchHint"))}">${ico("cloud")}${esc(t("tracker.fetch"))}</button></span></label>
    <p class="muted" data-fetched></p>
    <label>${esc(t("items.title"))}<input name="title" required></label>
    <label>${esc(t("items.type"))}<select name="type"><option value=""></option>${types}</select></label>
    <label>${esc(t("newItem.folder"))}<input name="folder" required></label>
    <p class="error" data-error></p>
    <div class="modal-actions"><button type="button" class="btn" data-cancel>${esc(t("common.cancel"))}</button><button class="btn primary">${esc(t("newItem.create"))}</button></div></form>`;
  document.body.append(back);
  const form = back.querySelector("form")!;
  const field = (n: string) => form.elements.namedItem(n) as HTMLInputElement;
  let touched = false;
  const suggest = () => {
    if (touched) return;
    const slug = field("title").value.normalize("NFKD").replace(/[^\x00-\x7f]/g, "").toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "").slice(0, 40).replace(/-$/, "");
    const id = field("id").value.trim();
    const tpl = id ? wf().folders.newItemTemplate : "{slug}";
    field("folder").value = tpl.replace("{id}", id).replace("{slug}", slug).replace(/^-+|-+$/g, "");
  };
  field("title").oninput = suggest;
  field("folder").oninput = () => (touched = true);
  const close = () => back.remove();
  // Con id: el tracker completa título, tipo y carpeta (y la descripción y los criterios del README).
  let fetched: TrackerItem | null = null;
  const info = back.querySelector<HTMLElement>("[data-fetched]")!;
  field("id").oninput = () => {
    if (fetched && fetched.id !== field("id").value.trim().replace(/^#/, "")) {
      fetched = null;
      info.textContent = "";
    }
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
        (form.elements.namedItem("type") as HTMLSelectElement).value = d.localType;
        if (!touched) field("folder").value = d.folder;
      }
      const have = d.existing.length ? ` · ${t("tracker.alreadyIn", { folders: d.existing.join(", ") })}` : "";
      info.textContent = (d.fromTracker ? t("tracker.fetched", { type: d.type, state: d.state, source: d.source }) : t("tracker.fetchOffline")) + have;
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
    try {
      const id = field("id").value.trim();
      const type = (form.elements.namedItem("type") as HTMLSelectElement).value;
      const it = id
        ? await createTrackedItem({ id, title: field("title").value, type, folder: field("folder").value.trim(), fetched })
        : await api.createItem(field("folder").value.trim(), field("title").value, type);
      close();
      await refresh();
      await openDetail(it.name);
    } catch (err) {
      back.querySelector("[data-error]")!.textContent = errorText(err);
    }
  };
  field("id").focus();
}

// ---------- eliminados ----------

async function deletedModal(): Promise<void> {
  const list = await api.listDeleted();
  const back = document.createElement("div");
  back.className = "modal-back";
  back.innerHTML = `<div class="modal" role="dialog" aria-modal="true"><h2>${esc(t("items.deleted"))}</h2>
    <ul class="list">${list
      .map((d) => `<li><span class="grow"><b>${esc(d.title || d.base)}</b> <span class="muted">${esc(d.base)} · ${esc(fmtAgo(d.deleted))}</span></span>
        <button class="btn" data-restore="${esc(d.path)}" data-base="${esc(d.base)}">${esc(t("items.restore"))}</button></li>`)
      .join("") || `<li class="empty">${esc(t("items.noDeleted"))}</li>`}</ul>
    <div class="modal-actions"><button class="btn" data-close>${esc(t("common.close"))}</button></div></div>`;
  document.body.append(back);
  back.querySelector<HTMLElement>("[data-close]")!.onclick = () => back.remove();
  back.querySelectorAll<HTMLElement>("[data-restore]").forEach((b) => {
    b.onclick = async () => {
      await api.untrash(b.dataset.restore!, b.dataset.base!);
      back.remove();
      await refresh();
    };
  });
}

// ---------- acciones en lote (con deshacer) ----------

type Before = [string, Record<string, MetaValue>];

/** Escribe campos en varios items y ofrece deshacer (vuelve cada campo a lo que tenía). */
async function applyFields(names: string[], fieldsFor: (it: Item) => Record<string, MetaValue> | null, what: string): Promise<void> {
  const before: Before[] = [];
  const errors: string[] = [];
  for (const n of names) {
    const it = itemByName(n);
    const fields = it && fieldsFor(it);
    if (!it || !fields) continue;
    const prev = Object.fromEntries(Object.keys(fields).map((k) => [k, it.meta[k] ?? ""]));
    try {
      await api.updateFields(n, fields);
      before.push([n, prev]);
    } catch (err) {
      errors.push(`${titleOf(it)}: ${errorText(err)}`);
    }
  }
  await refresh();
  if (errors.length) toast(t("grid.bulk.errors", { n: errors.length, list: errors.join(" · ") }), undefined, "error");
  if (before.length) toast(t("grid.bulk.done", { what, n: before.length }), { label: t("common.undo"), run: () => void restoreFields(before) });
}

async function restoreFields(before: Before[]): Promise<void> {
  let n = 0;
  for (const [name, fields] of before) {
    const clean = Object.fromEntries(Object.entries(fields).map(([k, v]) => [k, Array.isArray(v) && !v.length ? "" : v]));
    try {
      await api.updateFields(name, clean);
      n++;
    } catch (err) {
      toast(errorText(err), undefined, "error");
    }
  }
  await refresh();
  toast(t("grid.undone", { n }));
}

/** Mueve carpetas (a un grupo o a la raíz) con deshacer. */
async function moveItems(names: string[], dest: string, what: string): Promise<void> {
  const moved: [string, string, string][] = [];
  for (const n of names) {
    const old = itemByName(n)?.group ?? "";
    const now = await api.moveFolder(n, dest);
    if (now !== n) moved.push([now, n, old]);
  }
  G.selected.clear();
  await refresh();
  toast(t("grid.bulk.done", { what, n: moved.length }), {
    label: t("common.undo"),
    run: async () => {
      for (const [now, , old] of moved) await api.moveFolder(now, old);
      await refresh();
      toast(t("grid.undone", { n: moved.length }));
    },
  });
}

async function trashItems(names: string[]): Promise<void> {
  if (!(await confirmDialog(t("items.confirmDelete", { n: names.length }), t("items.delete")))) return;
  const moved: [string, string][] = [];
  for (const n of names) moved.push([await api.trash(n), n]);
  G.selected.clear();
  await refresh();
  toast(t("items.deletedN", { n: moved.length }), {
    label: t("common.undo"),
    run: async () => {
      for (const [deleted, name] of moved) await api.untrash(deleted, name);
      await refresh();
    },
  });
}

/** Planificación en lote: fecha pactada, período y horas; vacío = no tocar. */
function planningModal(names: string[]): void {
  const f = wf().fields;
  const back = document.createElement("div");
  back.className = "modal-back";
  back.innerHTML = `<form class="modal small" role="dialog" aria-modal="true"><h2>${esc(t("grid.bulk.planning"))}</h2>
    <p class="muted">${esc(t("grid.plan.intro", { n: names.length }))}</p>
    <label>${esc(t("detail.due"))}<input name="due" type="date"></label>
    <label>${esc(t("detail.period"))}<input name="period" placeholder="${esc(t("detail.periodHint"))}"></label>
    <label>${esc(t("detail.hours"))}<input name="hours" inputmode="decimal"></label>
    <p class="error" data-error></p>
    <div class="modal-actions"><button type="button" class="btn" data-cancel>${esc(t("common.cancel"))}</button><button class="btn primary">${esc(t("common.ok"))}</button></div></form>`;
  document.body.append(back);
  const form = back.querySelector("form")!;
  const val = (n: string) => (form.elements.namedItem(n) as HTMLInputElement).value.trim();
  const close = () => back.remove();
  back.querySelector<HTMLElement>("[data-cancel]")!.onclick = close;
  back.addEventListener("keydown", (e) => e.key === "Escape" && close());
  back.addEventListener("click", (e) => {
    if (e.target === back) close();
  });
  form.onsubmit = async (e) => {
    e.preventDefault();
    const fields: Record<string, string> = {};
    if (val("due")) fields[f.due] = val("due");
    if (val("period")) fields[f.period] = val("period");
    if (val("hours")) fields[f.hours] = val("hours");
    if (!Object.keys(fields).length) return close();
    // Se valida con el primero: si un valor no sirve, no se escribe nada.
    try {
      await api.updateFields(names[0], fields);
    } catch (err) {
      back.querySelector("[data-error]")!.textContent = errorText(err);
      return;
    }
    close();
    const first = itemByName(names[0]);
    // El primero ya quedó escrito: se reescribe con lo de antes en el deshacer, como los demás.
    const prevFirst: Before | null = first ? [names[0], Object.fromEntries(Object.keys(fields).map((k) => [k, first.meta[k] ?? ""]))] : null;
    await applyFieldsAfterFirst(names, fields, prevFirst);
  };
  (form.elements.namedItem("due") as HTMLInputElement).focus();
}

async function applyFieldsAfterFirst(names: string[], fields: Record<string, string>, prevFirst: Before | null): Promise<void> {
  const before: Before[] = prevFirst ? [prevFirst] : [];
  const errors: string[] = [];
  for (const n of names.slice(1)) {
    const it = itemByName(n);
    if (!it) continue;
    const prev = Object.fromEntries(Object.keys(fields).map((k) => [k, it.meta[k] ?? ""]));
    try {
      await api.updateFields(n, fields);
      before.push([n, prev]);
    } catch (err) {
      errors.push(`${titleOf(it)}: ${errorText(err)}`);
    }
  }
  await refresh();
  if (errors.length) toast(t("grid.bulk.errors", { n: errors.length, list: errors.join(" · ") }), undefined, "error");
  if (before.length) toast(t("grid.bulk.done", { what: t("grid.bulk.planning"), n: before.length }), { label: t("common.undo"), run: () => void restoreFields(before) });
}

/** Elegir el grupo de destino. */
function moveToGroupModal(names: string[]): void {
  const groups = S.items.filter((it) => it.kind === "group");
  const back = document.createElement("div");
  back.className = "modal-back";
  back.innerHTML = `<form class="modal small" role="dialog" aria-modal="true"><h2>${esc(t("grid.bulk.moveTo"))}</h2>
    <label>${esc(t("items.group"))}<select name="dest">${groups.map((g) => `<option value="${esc(g.name)}">${esc(titleOf(g))}</option>`).join("")}</select></label>
    <div class="modal-actions"><button type="button" class="btn" data-cancel>${esc(t("common.cancel"))}</button><button class="btn primary">${esc(t("common.ok"))}</button></div></form>`;
  document.body.append(back);
  const form = back.querySelector("form")!;
  const close = () => back.remove();
  back.querySelector<HTMLElement>("[data-cancel]")!.onclick = close;
  back.addEventListener("keydown", (e) => e.key === "Escape" && close());
  form.onsubmit = async (e) => {
    e.preventDefault();
    const dest = (form.elements.namedItem("dest") as HTMLSelectElement).value;
    close();
    const its = names.filter((n) => itemByName(n)?.kind === "item" && itemByName(n)?.group !== dest);
    await moveItems(its, dest, t("grid.bulk.moveTo"));
  };
  (form.elements.namedItem("dest") as HTMLSelectElement).focus();
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
onAction("grid-group", (el) => toggleFlip(el.dataset.key!));
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
onAction("bulk-group", async () => {
  const names = selectedNames().filter((n) => !itemByName(n)?.group && itemByName(n)?.kind === "item");
  if (!names.length) return;
  const title = await promptDialog(t("items.groupTitle"));
  if (!title) return;
  const res = await api.createGroup(title, names);
  G.selected.clear();
  await refresh();
  toast(t("items.grouped", { n: res.members.length }), {
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
onAction("grid-bulk-field", (el) => {
  const key = el.dataset.key!;
  const value = el.dataset.value ?? "";
  const shown = key === wf().fields.status ? (value ? label(wf().states.find((s) => s.id === value)!.labels, value) : "") : t("items.type");
  return applyFields(selectedNames(), (it) => (metaText(it, key) === value ? null : { [key]: value }), shown || t("items.type"));
});
onAction("grid-bulk-env", (el) => {
  const env = el.dataset.env!;
  const key = wf().fields.environments;
  const items = selectedNames().map(itemByName).filter(Boolean) as Item[];
  const remove = items.every((it) => metaList(it, key).includes(env));
  return applyFields(
    items.map((i) => i.name),
    (it) => {
      const list = metaList(it, key);
      if (remove) return { [key]: list.filter((e) => e !== env) };
      return list.includes(env) ? null : { [key]: [...list, env] };
    },
    t("grid.col.envs"),
  );
});
onAction("grid-bulk-plan", () => planningModal(selectedNames()));
onAction("grid-bulk-revert", () => {
  const f = wf().fields;
  const keys = [f.due, f.period, f.hours];
  return applyFields(
    selectedNames(),
    (it) => (keys.some((k) => metaText(it, k)) ? Object.fromEntries(keys.map((k) => [k, ""])) : null),
    t("detail.planning"),
  );
});
onAction("grid-bulk-text", async (el) => {
  const key = el.dataset.key!;
  const names = selectedNames();
  const first = itemByName(names[0]);
  const shown = key === wf().fields.notes ? t("items.notes") : t("grid.col.branch");
  const value = await promptDialog(shown, first ? metaText(first, key) : "", true);
  if (value === null) return;
  await applyFields(names, (it) => (metaText(it, key) === value ? null : { [key]: value }), shown);
});
onAction("grid-bulk-rename", async () => {
  const name = selectedNames()[0];
  if (!name) return;
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
onAction("grid-bulk-move", () => moveToGroupModal(selectedNames()));
onAction("grid-bulk-ungroup", () => moveItems(selectedNames().filter((n) => itemByName(n)?.group), "", t("grid.bulk.ungroup")));
onAction("grid-bulk-session", () => newSessionModal(selectedNames()));
onAction("grid-bulk-addsession", () => addToSessionModal(selectedNames()));
onAction("grid-bulk-reveal", async () => {
  await api.reveal(selectedNames()[0]);
});
onAction("grid-bulk-copy", async (el) => {
  const items = selectedNames().map(itemByName).filter(Boolean) as Item[];
  const text = el.dataset.what === "ids" ? [...new Set(items.flatMap((i) => i.ids))].join("\n") : items.map((i) => i.name).join("\n");
  await navigator.clipboard.writeText(text);
  toast(t("grid.bulk.copied"));
});

onChange("bulk-status", async (el) => {
  const status = (el as HTMLSelectElement).value;
  if (!status) return;
  const key = wf().fields.status;
  await applyFields(selectedNames(), (it) => (metaText(it, key) === status ? null : { [key]: status }), label(wf().states.find((s) => s.id === status)!.labels, status));
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

/** Pone el foco en la búsqueda de la grilla (`/`). */
export function focusSearch(): void {
  $<HTMLInputElement>("#items-search")?.focus();
}

// Búsqueda en vivo (sin esperar el `change`).
document.addEventListener("input", (ev) => {
  const el = ev.target as HTMLInputElement;
  if (el.id !== "items-search") return;
  G.text = el.value;
  const pos = el.selectionStart;
  render();
  const input = $<HTMLInputElement>("#items-search");
  input?.focus();
  input?.setSelectionRange(pos, pos);
});

