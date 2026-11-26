// Detalle de un item, en un panel lateral sobre la vista (como la v1: fondo, ancho que se arrastra y se
// recuerda, maximizable; ver `app.ts`). Cabecera (migas, meta, alertas, acciones), estado por pasos, campos con guardado
// automático de lo tocado (el refresco en vivo no pisa lo que estás escribiendo; si el README cambió en disco
// mientras editabas, un aviso deja aplicar tu cambio sobre lo nuevo o descartarlo), tareas, archivos con visor,
// cómo deployar, notas, README, Git y versiones anteriores. A la derecha, siempre a la vista, el panel de
// planificación, detalles, tracker, resumen git e info.

import { api, pluginApi, trackerApi } from "../api";
import { closeDetail, onAction, onAfterRender, onChange, refresh, registerDetail, render, requestCloseDetail } from "../app";
import { confirmDialog, esc, fmtAgo, fmtDate, fmtDateTimeFull, fmtSize, promptDialog, selectDialog, toast } from "../dom";
import { errorText, label, t } from "../i18n";
import { ico } from "../icons";
import { renderMarkdown } from "../markdown";
import { canReflect, reflectItems } from "../reflect";
import { pluginItemCard } from "../plugins";
import { canReview, openReview } from "../review";
import { addToSessionModal, launched, loadSessions, newSessionModal, planItem, sessionById } from "../sessions";
import { S, effectiveStatus, envLabel, inSession, itemByName, metaList, metaText, stateDef, stateLabel, statusOf, titleOf, wf, type DetailTab } from "../state";
import { termSize } from "../terminal";
import type { ApiError, Item, StateCategory, StateDef, Workflow } from "../types";
import { statusChip } from "./items";
import { dueOf, fmtDay, hoursOf, lateTag, localTag, overridesTracker, periodOf, periodRel, priorityBadge, priorityOf } from "../planning";
import { curPeriod, fmtH, periodRange, periodShift, sessionsWithTime, timeReady, workedTotal } from "../stats";
import { TR, trackerBadge, trackerCard, trackerConnected, trackerDue, trackerEstimate } from "./tracker";
import { G, OK_STATES, gitDesyncs, gitInfo, gitUncommitted, itemIds, openPromotion, renderGitTab } from "../git";
import { activeAlerts } from "./alerts";
import { allFiles, filesTab, newBadge, newCount, scriptsCard } from "./detail-files";
import { ensureNotes, notesOf, notesTab } from "./detail-notes";

const item = (): Item => S.detailData!.item;
const isGroup = (it: Item) => it.kind === "group";

// ---------- guardado: lo tocado, el indicador y el conflicto con disco ----------

type SaveState = "" | "pending" | "saving" | "ok" | "err";
let saveState: SaveState = "";
/** README cuando empezaste a escribir: si cambia en disco antes de guardar, hay conflicto. */
let baseline: string | null = null;
let diskChanged = false;
/** Lo escrito en «agregar tarea» (no se pierde con el refresco). */
let taskDraft = "";
/** Item del último render: al abrir otro se limpia el estado de edición. */
let shownName: string | null = null;
/** Foco a devolver después del render. */
let pendingFocus: { sel: string; start: number | null; end: number | null } | null = null;

const drafts = () => Object.keys(S.fieldDrafts);
const dirty = () => drafts().length > 0;

function resetEditing(): void {
  saveState = "";
  baseline = null;
  diskChanged = false;
  taskDraft = "";
}

function saveStateHtml(): string {
  if (!saveState) return `<span class="save-state" id="save-state"></span>`;
  return `<span class="save-state ${saveState}" id="save-state" role="status">${esc(t(`detail.saveState.${saveState}`))}</span>`;
}

function setSaveState(st: SaveState): void {
  saveState = st;
  const el = document.querySelector("#save-state");
  if (el) el.outerHTML = saveStateHtml();
}

/** Nombre de un campo del frontmatter para el aviso de conflicto. */
function fieldName(key: string): string {
  const f = wf().fields;
  const names: Record<string, string> = {
    [f.notes]: t("detail.whatsMissing"),
    [f.branch]: t("detail.branch"),
    [f.title]: t("items.title"),
    [f.due]: t("detail.due"),
    [f.period]: t("detail.period"),
    [f.hours]: t("detail.hours"),
    [f.itemIds]: t("detail.ids"),
  };
  return names[key] ?? key;
}

function conflictBanner(): string {
  if (!diskChanged) return "";
  const what = drafts().map(fieldName).join(", ");
  return `<div class="d-warn conflict" role="alert"><div class="grow"><b>${esc(t("detail.conflict.title"))}</b>
      ${what ? `<span>${esc(t("detail.conflict.fields", { fields: what }))}</span>` : ""}
      <span class="muted">${esc(t("detail.conflict.why"))}</span></div>
    <button class="btn primary" data-action="detail-apply">${esc(t("detail.conflict.apply"))}</button>
    <button class="btn" data-action="detail-discard">${esc(t("detail.conflict.discard"))}</button></div>`;
}

/** Selector estable del elemento con foco, para devolvérselo después de rearmar el HTML. */
function captureFocus(): void {
  const a = document.activeElement as HTMLInputElement | null;
  pendingFocus = null;
  if (!a || !a.closest("#dpanel .detail")) return;
  let sel = "";
  if (a.dataset.change && a.dataset.key) sel = `[data-change="${a.dataset.change}"][data-key="${CSS.escape(a.dataset.key)}"]`;
  else if (a.id) sel = `#${CSS.escape(a.id)}`;
  if (!sel) return;
  let start: number | null = null;
  let end: number | null = null;
  try {
    start = a.selectionStart;
    end = a.selectionEnd;
  } catch {
    // inputs sin selección (fecha, select)
  }
  pendingFocus = { sel, start, end };
}

onAfterRender(() => {
  const f = pendingFocus;
  pendingFocus = null;
  if (!f || !S.detail) return;
  const el = document.querySelector<HTMLInputElement>(`#dpanel .detail ${f.sel}`);
  if (!el) return;
  el.focus();
  try {
    if (f.start !== null && f.end !== null) el.setSelectionRange(f.start, f.end);
  } catch {
    // no admite selección
  }
});

// Mientras escribís: el valor queda como borrador (el refresco lo respeta) y se marca «sin guardar».
document.addEventListener("input", (ev) => {
  const el = ev.target as HTMLInputElement;
  if (!S.detail || !el.closest?.("#dpanel .detail")) return;
  if (el.id === "task-new") {
    taskDraft = el.value;
    return;
  }
  if (el.dataset.change !== "field" || !el.dataset.key || el.tagName === "SELECT") return;
  if (!dirty()) baseline = S.detailData?.readme ?? null;
  S.fieldDrafts[el.dataset.key] = el.value;
  if (saveState !== "pending") setSaveState("pending");
});

// Al salir de un campo que volvió a su valor (escribiste y borraste), deja de estar pendiente.
document.addEventListener("focusout", (ev) => {
  const el = ev.target as HTMLInputElement;
  const key = el.dataset?.key;
  if (!S.detailData || el.dataset?.change !== "field" || !key || !(key in S.fieldDrafts) || S.fieldErrors[key]) return;
  if (S.fieldDrafts[key] !== metaText(item(), key)) return;
  delete S.fieldDrafts[key];
  if (!dirty()) {
    baseline = null;
    if (saveState === "pending") setSaveState("");
    if (diskChanged) {
      diskChanged = false;
      render();
    }
  }
});

/** Guarda solo los campos pasados. Devuelve si se guardó. */
async function save(fields: Record<string, unknown>): Promise<boolean> {
  const name = item().name;
  setSaveState("saving");
  try {
    await api.updateFields(name, fields);
    for (const k of Object.keys(fields)) {
      delete S.fieldErrors[k];
      delete S.fieldDrafts[k];
    }
    // Lo que guardamos recién no es un cambio en disco: la base se vuelve a tomar después de releer.
    baseline = null;
    if (!dirty()) diskChanged = false;
    saveState = "ok";
    await refresh();
    // Lo que quedó escrito en otro campo sigue pendiente; lo guardado recién no es un cambio en disco.
    if (dirty() && !diskChanged) baseline = S.detailData?.readme ?? null;
    return true;
  } catch (err) {
    const e = err as ApiError;
    const key = e.params?.key;
    if (key && key in fields) {
      S.fieldErrors[key] = errorText(err);
      if (typeof fields[key] === "string") S.fieldDrafts[key] = fields[key] as string;
      pendingFocus = { sel: `[data-change="field"][data-key="${CSS.escape(key)}"]`, start: null, end: null };
    } else toast(errorText(err), e.conflict ? { label: t("common.reload"), run: () => refresh() } : undefined, "error");
    saveState = "err";
    render();
    return false;
  }
}

// ---------- estado por pasos ----------

const isLinear = (s: StateDef) => s.category !== "cancelled" && s.category !== "unknown";
/** Orden de las categorías del camino (descartado y desconocido quedan afuera). */
const CAT_ORDER: StateCategory[] = ["backlog", "planned", "active", "review", "done"];

/** Camino principal del flujo (`mainPath` del preset; sin él, los estados que no son descartados ni desconocidos). */
function mainPath(): StateDef[] {
  const ids = wf().mainPath?.length ? wf().mainPath! : wf().states.filter(isLinear).map((s) => s.id);
  return ids.map(stateDef).filter((s): s is StateDef => !!s);
}

/** Estado siguiente: el que sigue en el camino principal; desde un estado fuera del camino (como «diagnóstico»),
 *  el primero del camino de una categoría posterior (la v1: diag → codeado); sin estado, el primero. */
function nextState(current: string): StateDef | null {
  const line = mainPath();
  if (!current) return line[0] ?? null;
  const i = line.findIndex((s) => s.id === current);
  if (i >= 0) return line[i + 1] ?? null;
  const rank = CAT_ORDER.indexOf(stateDef(current)?.category ?? "unknown");
  return rank < 0 ? null : (line.find((s) => CAT_ORDER.indexOf(s.category) > rank) ?? null);
}

/** Pasos: el camino principal y, al final, el estado actual si es otro (v1 app.js:1025). */
function steps(it: Item): string {
  const current = statusOf(it);
  const line = mainPath();
  const cur = line.findIndex((s) => s.id === current);
  const shown = cur >= 0 || !current ? line.map((s) => s.id) : [...line.map((s) => s.id), current];
  return `<div class="steps" role="group" aria-label="${esc(t("detail.status"))}">${shown
    .map((id, i) => {
      const on = id === current;
      const done = cur >= 0 && i < cur;
      const title = on ? t("detail.step.current") : t("detail.moveTo", { status: stateLabel(id) });
      return `<button class="step${on ? " current" : ""}${done ? " done" : ""}" data-action="set-status" data-status="${esc(id)}" aria-pressed="${on}" title="${esc(title)}">
        ${done ? ico("check") : `<span class="dot ${stateDef(id)?.category ?? "unknown"}"></span>`}${esc(stateLabel(id))}</button>`;
    })
    .join("")}</div>`;
}

async function setStatus(status: string): Promise<void> {
  const it = item();
  const before = statusOf(it);
  if (status === before) return;
  const key = wf().fields.status;
  if (!(await save({ [key]: status }))) return;
  const name = it.name;
  toast(t("detail.statusChanged", { title: titleOf(it), status: status ? stateLabel(status) : t("items.noStatus") }), {
    label: t("common.undo"),
    run: async () => {
      await api.updateFields(name, { [key]: before });
      await refresh();
    },
  });
}

// ---------- campos ----------

function fieldError(key: string): string {
  return S.fieldErrors[key] ? `<div class="field-error">${esc(S.fieldErrors[key])}</div>` : "";
}

const valueOf = (it: Item, key: string) => (key in S.fieldDrafts ? S.fieldDrafts[key] : metaText(it, key));

function input(key: string, type = "text", placeholder = "", extra = ""): string {
  return `<input type="${type}" value="${esc(valueOf(item(), key))}" data-change="field" data-key="${esc(key)}" placeholder="${esc(placeholder)}"${S.fieldErrors[key] ? ' aria-invalid="true"' : ""}${extra}>`;
}

function textField(key: string, labelText: string, type = "text", placeholder = "", tag = "", hint = ""): string {
  return `<label class="field"><span>${esc(labelText)}${tag}</span>${input(key, type, placeholder)}${hint ? `<span class="hint">${hint}</span>` : ""}${fieldError(key)}</label>`;
}

const clearLink = (it: Item, key: string) =>
  metaText(it, key) ? ` <button class="link" data-action="field-clear" data-key="${esc(key)}">${esc(t("detail.clear"))}</button>` : "";

/** Período: el fijado o «según la fecha»; las opciones son los 6 anteriores y siguientes al actual. */
function periodSelect(it: Item): string {
  const key = wf().fields.period;
  const own = metaText(it, key);
  const derived = periodOf({ ...it, meta: { ...it.meta, [key]: "" } }).key;
  const cur = curPeriod();
  const keys = [...new Set([...Array.from({ length: 13 }, (_, i) => periodShift(cur, i - 6)), ...(/^\d{4}-\d{2}$/.test(own) ? [own] : [])])].sort();
  const auto = derived ? t("detail.period.byDate", { range: periodRange(derived), rel: periodRel(derived) }) : t("detail.period.noDate");
  return `<label class="field"><span>${esc(t("detail.period"))}${overridesTracker(it, "period") ? localTag() : ""}</span>
    <select data-change="field" data-key="${esc(key)}"><option value="">${esc(auto)}</option>${keys
      .map((k) => `<option value="${esc(k)}"${own === k ? " selected" : ""}>${esc(`${periodRange(k)} · ${periodRel(k)}`)}</option>`)
      .join("")}</select>${fieldError(key)}</label>`;
}

function planningCard(it: Item): string {
  const f = wf().fields;
  const source = TR.cfg?.source ?? "";
  // Fecha pactada: la del tracker (o la de sus items, en un grupo) como pista.
  const trDue = !isGroup(it) && trackerConnected() ? trackerDue(it.ids) : "";
  const due = dueOf(it);
  const dueHint = trDue
    ? t("detail.hint.tracker", { source, value: fmtDay(trDue) })
    : isGroup(it) && due.src === "group"
      ? t("detail.hint.fromItems", { value: fmtDay(due.date) })
      : "";
  // Horas pactadas: la estimación del tracker o la suma de sus items.
  const trHours = !isGroup(it) && trackerConnected() ? trackerEstimate(it.ids) : null;
  const hours = hoursOf(it);
  const hoursHint = trHours !== null
    ? t("detail.hint.tracker", { source, value: fmtH(trHours) })
    : isGroup(it) && hours.src === "group"
      ? t("detail.hint.sumItems", { value: fmtH(hours.hours) })
      : "";
  const time = timeReady();
  const real = time ? workedTotal(it) : null;
  const nSess = time ? sessionsWithTime(it).length : 0;
  const worked = !time
    ? esc(t("detail.calculating"))
    : `<b>${esc(fmtH(real))}</b>${hours.hours ? ` · ${esc(t("detail.worked.pct", { pct: Math.round(((real ?? 0) / hours.hours) * 100) }))}` : ""}${nSess ? ` · <button class="link" data-action="hours-item" data-name="${esc(it.name)}" title="${esc(t("detail.worked.hoursHint"))}">${esc(t("detail.worked.sessions", { n: nSess }))}</button>` : ""}`;
  return `<section class="card"><h2>${ico("calendar")}${esc(t("detail.planning"))}</h2>
    ${textField(f.due, t("detail.due"), "date", "", overridesTracker(it, "due") ? localTag() : "", `${esc(dueHint)}${clearLink(it, f.due)}`)}
    ${textField(f.hours, t("detail.hours"), "text", trHours !== null ? String(trHours) : "", overridesTracker(it, "hours") ? localTag() : "", `${esc(hoursHint)}${clearLink(it, f.hours)}`)}
    ${periodSelect(it)}
    <div class="kv"><span>${esc(t("detail.worked"))}</span><span>${worked}</span></div>
  </section>`;
}

function fieldsCard(it: Item): string {
  const f = wf().fields;
  if (isGroup(it)) {
    const own = statusOf(it);
    const derived = effectiveStatus({ ...it, meta: { ...it.meta, [f.status]: "" } });
    const auto = derived ? t("detail.group.derived", { status: stateLabel(derived) }) : t("detail.group.derivedNone");
    const opts = wf().states.map((s) => `<option value="${esc(s.id)}"${own === s.id ? " selected" : ""}>${esc(label(s.labels, s.id))}</option>`).join("");
    return `<section class="card"><h2>${esc(t("detail.fields"))}</h2>
      <label class="field">${esc(t("detail.status"))}<select data-change="field" data-key="${esc(f.status)}"><option value="">${esc(auto)}</option>${opts}</select>${fieldError(f.status)}</label>
      ${textField(f.title, t("items.title"), "text", it.title || it.name)}
      ${textField(f.branch, t("detail.branch"))}
      ${f.extra.map((x) => textField(x.key, extraName(x))).join("")}
    </section>`;
  }
  // Estado: todos los del flujo, «Sin estado» y el que tenga el README si no es del flujo (v1 app.js:1134).
  const own = statusOf(it);
  const states = [...wf().states.map((x) => x.id), ...(own && !stateDef(own) ? [own] : [])]
    .map((id) => `<option value="${esc(id)}"${own === id ? " selected" : ""}>${esc(stateLabel(id))}</option>`)
    .join("");
  const types = wf().types.map((x) => `<option value="${esc(x.id)}"${metaText(it, f.type) === x.id ? " selected" : ""}>${esc(label(x.labels, x.id))}</option>`).join("");
  // Sin tipo propio: el del tracker como pista (v1 app.js:1135).
  const trType = metaText(it, f.type) ? "" : trackerType(it);
  const noType = trType ? t("detail.noTypeTracker", { source: TR.cfg?.source ?? "", type: trType }) : t("detail.noType");
  const envs = metaList(it, f.environments);
  const envBoxes = wf()
    .environments.map(
      (e) => `<label class="check"><input type="checkbox" data-change="env" data-env="${esc(e.id)}"${envs.includes(e.id) ? " checked" : ""}>${esc(label(e.labels, e.id))}</label>`,
    )
    .join("");
  return `<section class="card"><h2>${esc(t("detail.fields"))}</h2>
    <label class="field">${esc(t("detail.status"))}<select data-change="status-select"><option value="">${esc(t("detail.noStatus"))}</option>${states}</select>${fieldError(f.status)}</label>
    <label class="field">${esc(t("items.type"))}<select data-change="field" data-key="${esc(f.type)}"><option value="">${esc(noType)}</option>${types}</select>${fieldError(f.type)}</label>
    <div class="field">${esc(t("detail.environments"))}<div class="checks">${envBoxes}</div>${fieldError(f.environments)}</div>
    ${textField(f.branch, t("detail.branch"))}
    ${textField(f.title, t("items.title"), "text", it.title)}
    ${f.extra.map((x) => textField(x.key, extraName(x))).join("")}
    ${textField(f.itemIds, t("detail.ids"), "text", it.ids.join(", "))}
  </section>`;
}

/** Tipo del tracker del item (el del flujo si está mapeado), para la pista de «Sin tipo». */
function trackerType(it: Item): string {
  const w = it.ids.map((id) => TR.cfg?.cache?.items[id]).find((x) => x?.type);
  if (!w || !trackerConnected()) return "";
  const local = TR.cfg?.localTypes[w.type];
  const def = local ? wf().types.find((x) => x.id === local) : undefined;
  return def ? label(def.labels, def.id) : w.type;
}

// ---------- campos extra con link (los declara el preset: `fields.extra[].link`) ----------

type Extra = Workflow["fields"]["extra"][number];

const extraName = (x: Extra) => (x.labels ? label(x.labels, x.key) : x.key);

/** Links de los campos extra con valor (`link: "tracker"` → el work item; plantilla https → la página). */
function extraLinks(it: Item): string[] {
  return wf()
    .fields.extra.filter((x) => x.link && metaText(it, x.key).trim())
    .map((x) => {
      const v = metaText(it, x.key).trim();
      const text = x.link === "tracker" ? `${extraName(x)} #${v}` : `${extraName(x)} ${v}`;
      return `<button class="link" data-action="extra-link" data-key="${esc(x.key)}" title="${esc(t("detail.extraOpen", { name: extraName(x) }))}">${ico(x.icon && ico(x.icon) ? x.icon : "external")}${esc(text)}</button>`;
    });
}

/** «Qué falta» (el resumen de una línea), destacado arriba del Resumen. */
function whatsMissing(): string {
  const key = wf().fields.notes;
  return `<section class="card sum-note"><label class="field"><span>${esc(t("detail.whatsMissing"))}</span>${input(key, "text", t("detail.notesHint"), ' id="sum-notes"')}${fieldError(key)}</label></section>`;
}

function tasksCard(it: Item): string {
  const rows = it.tasks
    .map(
      (task, i) => `<li class="task${task.done ? " done" : ""}">
        <input type="checkbox" data-change="task-toggle" data-index="${i}"${task.done ? " checked" : ""} aria-label="${esc(task.text)}">
        <span class="grow" data-action="task-edit" data-index="${i}">${esc(task.text)}</span>
        <button class="icon-btn" data-action="task-edit" data-index="${i}" title="${esc(t("common.edit"))}">${ico("edit")}</button>
        <button class="icon-btn" data-action="task-remove" data-index="${i}" title="${esc(t("common.delete"))}">${ico("trash")}</button></li>`,
    )
    .join("");
  const gaps = it.planGaps.length
    ? `<p class="warn">${ico("alert")}${esc(it.planGaps.map((g) => t(`detail.gap.${g}`)).join(" · "))}</p>`
    : "";
  return `<section class="card"><h2>${esc(t("items.tasks"))}</h2>${gaps}
    <ul class="tasks">${rows || `<li class="empty">${esc(t("detail.noTasks"))}</li>`}</ul>
    <div class="add-row"><input id="task-new" value="${esc(taskDraft)}" placeholder="${esc(t("detail.addTask"))}" data-enter="task-add"><button class="btn" data-action="task-add">${ico("plus")}</button></div>
  </section>`;
}

function membersCard(it: Item): string {
  if (!isGroup(it)) return "";
  const rows = it.members
    .map((m) => itemByName(m))
    .filter(Boolean)
    .map((m) => `<li data-action="open-item" data-name="${esc(m!.name)}" class="link-row">${statusChip(effectiveStatus(m!))}<span class="grow">${esc(titleOf(m!))}</span><span class="muted">${esc(m!.ids.join(", "))}</span></li>`)
    .join("");
  return `<section class="card"><h2>${esc(t("detail.members"))} <span class="muted">${it.members.length}</span></h2><ul class="list">${rows || `<li class="empty">—</li>`}</ul></section>`;
}

const SESSIONS_SHOWN = 6;
let allSessions = false;

/** Planear es por item (no grupo), desde cualquier estado: si ya pasó la planificación, se confirma. */
const canPlan = (it: Item) => it.kind === "item";

/** El item ya pasó la planificación (o está cerrado): planear lo vuelve atrás (v1 app.js:4069). */
function pastPlanning(it: Item): boolean {
  const cat = stateDef(statusOf(it))?.category;
  return cat === "active" || cat === "review" || cat === "done" || cat === "cancelled";
}

function sessionsCard(it: Item): string {
  const list = S.sessions.filter((s) => inSession(s, it.name));
  const shown = allSessions ? list : list.slice(0, SESSIONS_SHOWN);
  const rows = shown
    .map((s) => {
      const own = s.items.includes(it.name) && s.items.length > 1;
      const id = esc(s.id);
      return `<li class="link-row" data-action="sess-open" data-id="${id}"><span class="sdot ${s.status}"></span>
        <span class="grow">${esc(s.name)} <span class="muted">${esc(t(`session.status.${s.status}`))}${s.items.length > 1 ? ` · ${esc(t("session.items", { n: s.items.length }))}` : ""}</span></span>
        ${s.state === "closed" ? `<button class="btn" data-action="sess-reopen" data-id="${id}">${esc(t("session.reopen"))}</button>` : ""}
        ${own && s.state !== "closed" ? `<button class="icon-btn" data-action="sess-split" data-id="${id}" title="${esc(t("session.split"))}">${ico("columns")}</button>
          <button class="icon-btn" data-action="sess-remove-item" data-id="${id}" title="${esc(t("session.removeItem"))}">${ico("minus")}</button>` : ""}
        <button class="icon-btn" data-action="sess-rename" data-id="${id}" title="${esc(t("session.rename"))}">${ico("edit")}</button></li>`;
    })
    .join("");
  const more = list.length > SESSIONS_SHOWN && !allSessions ? `<button class="link" data-action="sess-show-all">${esc(t("session.showAll", { n: list.length }))}</button>` : "";
  return `<section class="card"><h2>${ico("terminal")}${esc(t("session.title"))}</h2>
    ${rows ? `<ul class="list">${rows}</ul>${more}` : `<p class="empty">${esc(t("session.noneForItem"))}</p>`}
    <div class="card-actions"><button class="btn" data-action="detail-new-session">${ico("plus")}${esc(t("session.new"))}</button>
      ${S.sessions.some((s) => s.state !== "closed" && !inSession(s, it.name)) ? `<button class="btn" data-action="detail-add-to-session">${ico("folderIn")}${esc(t("session.addTo"))}</button>` : ""}
      ${canPlan(it) ? `<button class="btn" data-action="detail-plan">${ico("plan")}${esc(t("session.plan"))}</button>` : ""}
      ${canReview(it, effectiveStatus(it)) ? `<button class="btn" data-action="detail-review" title="${esc(t("review.hint"))}">${ico("check")}${esc(t("review.action"))}</button>` : ""}
      ${canReflect(it.name, effectiveStatus(it)) ? `<button class="btn" data-action="detail-reflect" title="${esc(t("reflect.hint"))}">${ico("sparkle")}${esc(t("reflect.action"))}</button>` : ""}</div></section>`;
}

// ---------- pestañas ----------

function summaryTab(it: Item): string {
  return `${whatsMissing()}${tasksCard(it)}${scriptsCard(it)}${membersCard(it)}${sessionsCard(it)}`;
}

// ---------- panel derecho (a la vista en todas las pestañas, como la v1) ----------

type EnvSum = { state: "ok" | "partial" | "no"; ok: string[]; all: string[] };

/** Por ambiente: ok si todos los repos del item ya tienen el trabajo, parcial si algunos, no si ninguno
 *  (v1 `gitEnvSummary`). `null` = sin ramas ni commits del item. */
function gitEnvSummary(it: Item): Record<string, EnvSum> | null {
  const info = gitInfo(it);
  if (!info || !G.data || (!info.branches.length && !Object.keys(info.mentions).length)) return null;
  const repos = G.data.repos.filter((r) => !r.error && (info.branches.some((b) => b.repo === r.name) || info.mentions[r.name]));
  const envs = wf().environments.map((e) => e.id).filter((env) => repos.some((r) => r.envs?.[env]));
  const out: Record<string, EnvSum> = {};
  for (const env of envs) {
    const all = repos.filter((r) => r.envs?.[env]).map((r) => r.name);
    const ok = all.filter((name) => info.branches.some((b) => b.repo === name && OK_STATES.has(b.envs[env]?.state ?? "")) || !!info.mentions[name]?.[env]);
    out[env] = { state: ok.length === all.length ? "ok" : ok.length ? "partial" : "no", ok, all };
  }
  return out;
}

function gitCard(it: Item): string {
  if (it.kind !== "item" || !it.ids.length) return "";
  const title = `<h2>${ico("branch")}${esc(t("detail.tab.git"))}</h2>`;
  if (!G.data) return `<section class="card side-git">${title}<p class="muted">${esc(t(G.loading ? "detail.git.loading" : "detail.git.noData"))}</p></section>`;
  const sum = gitEnvSummary(it);
  if (!sum) return `<section class="card side-git">${title}<p class="muted">${esc(t("detail.git.none"))}</p></section>`;
  const icon = { ok: "check", partial: "clock", no: "minus" } as const;
  const rows = Object.entries(sum)
    .map(([env, x]) => {
      const text = x.state === "ok" ? t("detail.git.allRepos") : x.state === "partial" ? t("detail.git.someRepos", { ok: x.ok.length, all: x.all.length }) : t("detail.git.pending");
      return `<div class="kv"><span>${esc(envLabel(env))}</span><span class="genv ${x.state}" title="${esc(x.ok.join(", ") || t("detail.git.noRepos"))}">${ico(icon[x.state])}${esc(text)}</span></div>`;
    })
    .join("");
  return `<section class="card side-git">${title}${rows}
    <button class="link see-more" data-action="detail-tab" data-tab="git">${esc(t("detail.git.seeBranches"))}${ico("chevronRight")}</button></section>`;
}

function infoCard(it: Item): string {
  const group = isGroup(it);
  const files = group
    ? t("detail.info.groupFiles", { n: allFiles(it).length, own: it.files.length })
    : t("detail.info.files", { n: it.files.length, size: fmtSize(it.size) });
  return `<section class="card side-info"><h2>${ico("info")}${esc(t("detail.info"))}</h2>
    ${group ? "" : `<div class="kv"><span>${esc(t("detail.info.created"))}</span><span>${esc(it.created ? fmtDate(it.created) : "—")}</span></div>`}
    <div class="kv"><span>${esc(t("detail.info.last"))}</span><span title="${esc(fmtDateTimeFull(it.last))}">${esc(fmtAgo(it.last))}</span></div>
    <div class="kv"><span>${esc(t("detail.info.filesLabel"))}</span><span>${esc(files)}</span></div>
    ${it.kinds.length ? `<div class="kinds">${it.kinds.map((k) => `<span class="chip kind">${esc(k)}</span>`).join("")}</div>` : ""}
    <div class="side-links">${it.hasReadme ? `<button class="link" data-action="detail-tab" data-tab="versions">${esc(t("detail.info.versions"))}</button>` : ""}
      <button class="link" data-action="rename">${esc(t("detail.info.rename"))}</button>
      ${group ? `<button class="link" data-action="dissolve-group" title="${esc(t("detail.dissolveHint"))}">${esc(t("detail.dissolve"))}</button>` : ""}</div></section>`;
}

function sidePanel(it: Item): string {
  const links = extraLinks(it);
  const linkCard = links.length ? `<section class="card side-links-card"><div class="side-links">${links.join("")}</div></section>` : "";
  return `${pluginItemCard(it)}${planningCard(it)}${fieldsCard(it)}${trackerCard(it)}${linkCard}${gitCard(it)}${infoCard(it)}`;
}

function readmeTab(): string {
  const text = S.detailData!.readme;
  return text
    ? `<section class="card md">${renderMarkdown(text.replace(/^---[\s\S]*?\n---\s*\n/, ""))}</section>`
    : `<p class="empty">${esc(t("detail.noReadme"))}</p>`;
}

function deployTab(it: Item): string {
  if (it.deploy) return `<section class="card md">${renderMarkdown(it.deploy)}</section>`;
  const words = (wf().deployKeywords ?? []).map((w) => `«${w}»`).join(", ");
  return `<div class="empty"><p>${esc(t("detail.noDeploy"))}</p>${words ? `<p>${esc(t("detail.noDeployHint", { words }))}</p>` : ""}</div>`;
}

let versions: { id: string; ts: number; size: number }[] | null = null;

function versionsTab(): string {
  if (!versions) {
    api.listVersions(item().name).then((v) => {
      versions = v;
      render();
    });
    return `<p class="muted">${esc(t("common.loading"))}</p>`;
  }
  return versions.length
    ? `<section class="card"><ul class="list">${versions
        .map((v) => `<li><span class="grow">${esc(fmtDateTimeFull(v.ts))}</span><span class="muted">${esc(fmtSize(v.size))}</span>
          <button class="btn" data-action="view-version" data-id="${esc(v.id)}">${esc(t("detail.view"))}</button>
          <button class="btn" data-action="restore-version" data-id="${esc(v.id)}">${esc(t("detail.restore"))}</button></li>`)
        .join("")}</ul></section>`
    : `<p class="empty">${esc(t("detail.noVersions"))}</p>`;
}

const cnt = (n: number | string) => ` <span class="cnt">${esc(n)}</span>`;

/** Lo que va al lado del nombre de cada pestaña. */
function tabExtra(it: Item, tab: DetailTab): string {
  switch (tab) {
    case "summary": {
      const open = it.tasks.filter((x) => !x.done).length;
      return open ? cnt(open) : "";
    }
    case "files":
      return cnt(allFiles(it).length) + newBadge(newCount(it));
    case "git": {
      const g = gitInfo(it);
      if (!g) return "";
      const warn = gitDesyncs(it).length || gitUncommitted(it).length;
      return cnt(g.branches.length) + (warn ? ` <span class="flag" title="${esc(t("detail.gitWarn"))}">${ico("alert")}</span>` : "");
    }
    case "deploy":
      return it.deploy ? "" : cnt("—");
    case "notes":
      return cnt(notesOf(it).length);
    default:
      return "";
  }
}

const TABS: DetailTab[] = ["summary", "readme", "files", "git", "deploy", "notes", "versions"];

// ---------- cabecera ----------

function head(it: Item): string {
  const parent = it.group ? itemByName(it.group) : null;
  const status = statusOf(it);
  const next = isGroup(it) ? null : nextState(status);
  const typeText = it.type ? label(wf().types.find((x) => x.id === it.type)?.labels ?? {}, it.type) : "";
  const meta = [
    isGroup(it) ? esc(t("detail.group.label", { n: it.members.length })) : typeText ? esc(typeText) : "",
    it.ids.length
      ? trackerConnected()
        ? it.ids.map((x) => `<button class="link mono" data-action="trk-open" data-id="${esc(x)}" title="${esc(t("tracker.open", { source: TR.cfg?.source ?? "" }))}">#${esc(x)}</button>`).join(" ")
        : esc(it.ids.map((x) => `#${x}`).join(" "))
      : "",
    trackerBadge(it),
    priorityBadge(priorityOf(it)),
    ...extraLinks(it),
    `<span class="mono">${esc(it.name)}</span>`,
    it.dupes?.length ? `<span class="warn-text">${esc(t("alerts.dupes"))}</span>` : "",
  ].filter(Boolean);
  const alerts = [...new Set(activeAlerts().filter((a) => a.item.name === it.name).map((a) => a.kind))];
  const inGroup = it.group
    ? `<button class="btn" data-action="ungroup">${ico("folderOut")}${esc(t("detail.ungroup"))}</button>`
    : it.kind === "item"
      ? `<button class="btn" data-action="to-group">${ico("folderIn")}${esc(t("detail.toGroup"))}</button>`
      : "";
  const maxed = !!document.querySelector("#dpanel.max");
  const maxText = t(maxed ? "detail.restoreWidth" : "detail.maximize");
  return `<div class="detail-top"><button class="link" data-action="back" title="${esc(t("detail.backHint"))}">${ico("chevronLeft")}${esc(t(`nav.${S.view}`))}</button>
      ${parent ? `<span class="muted">/</span><button class="link" data-action="open-item" data-name="${esc(parent.name)}">${ico("folder")}${esc(titleOf(parent))}</button>` : ""}
      <span class="spacer"></span>${saveStateHtml()}
      <button class="icon-btn" data-action="detail-max" title="${esc(maxText)}" aria-label="${esc(maxText)}">${ico(maxed ? "minimize" : "maximize")}</button>
      <button class="icon-btn" data-action="back" title="${esc(t("detail.closeHint"))}" aria-label="${esc(t("common.close"))}">${ico("x")}</button></div>
    <div class="detail-head"><div class="grow"><div class="detail-meta muted">${meta.join(`<span class="sep">·</span>`)}</div>
      <h1>${esc(titleOf(it))}${lateTag(it)}</h1></div></div>
    <div class="detail-row">${isGroup(it) ? `<span class="muted">${esc(t("detail.status"))}: ${statusChip(effectiveStatus(it))}</span>` : steps(it)}
      <span class="spacer"></span>
      <div class="detail-acts">
        ${!isGroup(it) && ["backlog", "planned"].includes(stateDef(status)?.category ?? "backlog") ? `<button class="btn" data-action="detail-plan" title="${esc(t("detail.planHint"))}">${ico("plan")}${esc(t("session.plan"))}</button>` : ""}
        <button class="btn" data-action="detail-new-session" title="${esc(t("detail.aiSessionHint"))}">${ico("play")}${esc(t("detail.aiSession"))}</button>
        <button class="icon-btn" data-action="open-folder" title="${esc(t("detail.openFolder"))}" aria-label="${esc(t("detail.openFolder"))}">${ico("folder")}</button>
        <button class="icon-btn" data-action="open-vscode" title="${esc(t("detail.vscode"))}" aria-label="${esc(t("detail.vscode"))}">${ico("code")}</button>
        ${itemIds(it).length ? `<button class="btn" data-action="promote-item" title="${esc(t("promo.buttonHint"))}">${ico("promote")}${esc(t("promo.button"))}</button>` : ""}
        ${inGroup}
        <button class="btn" data-action="rename">${ico("edit")}${esc(t("detail.rename"))}</button>
        <button class="btn danger" data-action="trash-item">${ico("trash")}${esc(t("items.delete"))}</button>
        ${next ? `<button class="btn primary" data-action="set-status" data-status="${esc(next.id)}">${ico("chevronRight")}${esc(t("detail.moveTo", { status: label(next.labels, next.id) }))}</button>` : ""}
      </div></div>
    ${conflictBanner()}
    ${alerts.length ? `<div class="d-warn" role="note">${ico("alert")}<span>${esc(alerts.map((k) => t(`alerts.${k}`)).join(" · "))}</span>${it.dupes?.length ? `<span class="muted">${esc(it.dupes.join(", "))}</span>` : ""}</div>` : ""}`;
}

function renderDetail(): string {
  const it = item();
  if (shownName !== it.name) {
    shownName = it.name;
    resetEditing();
    versions = null;
  }
  // Si el README cambió en disco mientras había algo sin guardar, no se pisa: se avisa.
  if (dirty() && baseline !== null && S.detailData!.readme !== baseline) diskChanged = true;
  if (!pendingFocus) captureFocus();
  ensureNotes();
  const tabs = TABS.map(
    (tab) => `<button class="tab${S.detailTab === tab ? " sel" : ""}" role="tab" aria-selected="${S.detailTab === tab}" data-action="detail-tab" data-tab="${tab}">${esc(t(`detail.tab.${tab}`))}${tabExtra(it, tab)}</button>`,
  ).join("");
  const body = {
    summary: summaryTab,
    readme: readmeTab,
    files: filesTab,
    git: renderGitTab,
    deploy: deployTab,
    notes: notesTab,
    versions: versionsTab,
  }[S.detailTab](it);
  return `<div class="dp-resizer" title="${esc(t("detail.resizeHint"))}" aria-hidden="true"></div>
    <div class="detail"><div class="dp-head">${head(it)}</div>
      <div class="d-main"><div class="d-content"><div class="tabs" role="tablist">${tabs}</div><div class="d-body${S.detailTab === "files" ? " files-body" : ""}" data-keep-scroll="body">${body}</div></div>
        <aside class="d-side" data-keep-scroll="side">${sidePanel(it)}</aside></div></div>`;
}

async function taskOp(fn: () => Promise<unknown>): Promise<void> {
  try {
    await fn();
  } catch (err) {
    const e = err as ApiError;
    toast(errorText(err), e.conflict ? { label: t("common.reload"), run: () => refresh() } : undefined, "error");
  }
  await refresh();
}

function viewer(title: string, html: string): void {
  const back = document.createElement("div");
  back.className = "modal-back";
  back.innerHTML = `<div class="modal viewer" role="dialog" aria-modal="true"><div class="viewer-head"><h2>${esc(title)}</h2><button class="icon-btn" data-close title="${esc(t("common.close"))}">${ico("x")}</button></div><div class="viewer-body">${html}</div></div>`;
  document.body.append(back);
  const close = () => back.remove();
  back.querySelector<HTMLElement>("[data-close]")!.onclick = close;
  back.onclick = (e) => {
    if (e.target === back) close();
  };
  back.tabIndex = -1;
  back.focus();
  back.addEventListener("keydown", (e) => e.key === "Escape" && close());
}

/** Antes de salir del detalle con algo escrito sin guardar. */
async function leaveOk(): Promise<boolean> {
  if (!dirty()) return true;
  if (!(await confirmDialog(t("detail.discardConfirm"), t("detail.conflict.discard")))) return false;
  S.fieldDrafts = {};
  S.fieldErrors = {};
  resetEditing();
  return true;
}

registerDetail(renderDetail, leaveOk);

onAction("back", () => requestCloseDetail());
onChange("status-select", (el) => setStatus((el as HTMLSelectElement).value));
onAction("extra-link", async (el) => {
  const x = wf().fields.extra.find((f) => f.key === el.dataset.key);
  const value = x ? metaText(item(), x.key).trim() : "";
  if (!x || !value) return;
  if (x.link === "tracker") await trackerApi.open(value);
  else await pluginApi.openUrl(x.link!.replace("{value}", encodeURIComponent(value)));
});
onAction("promote-item", () => openPromotion([item().name]));
onAction("detail-tab", (el) => {
  S.detailTab = el.dataset.tab as DetailTab;
  versions = null;
  if (S.detailTab === "notes") ensureNotes(true);
  render();
});
onAction("set-status", (el) => setStatus(el.dataset.status ?? ""));
onChange("field", async (el) => {
  const key = el.dataset.key!;
  const value = (el as HTMLInputElement).value;
  // Con el README cambiado en disco, lo escrito espera a que elijas en el aviso.
  if (diskChanged && el.tagName !== "SELECT") {
    S.fieldDrafts[key] = value;
    render();
    return;
  }
  await save({ [key]: value });
});
onAction("field-clear", async (el) => {
  await save({ [el.dataset.key!]: "" });
});
onChange("env", async () => {
  const envs = [...document.querySelectorAll<HTMLInputElement>('[data-change="env"]')].filter((b) => b.checked).map((b) => b.dataset.env!);
  await save({ [wf().fields.environments]: envs });
});
onAction("detail-apply", async () => {
  diskChanged = false;
  baseline = null;
  render();
  if (dirty()) await save({ ...S.fieldDrafts });
});
onAction("detail-discard", () => {
  S.fieldDrafts = {};
  S.fieldErrors = {};
  resetEditing();
  render();
});
onChange("task-toggle", (el) => {
  const i = Number(el.dataset.index);
  const task = item().tasks[i];
  return taskOp(() => api.setTask(item().name, i, (el as HTMLInputElement).checked, task.text));
});
onAction("task-add", async () => {
  const input = document.querySelector<HTMLInputElement>("#task-new")!;
  if (!input.value.trim()) return;
  const text = input.value;
  taskDraft = "";
  await taskOp(() => api.addTask(item().name, text));
  document.querySelector<HTMLInputElement>("#task-new")?.focus();
});
onAction("task-edit", async (el) => {
  const i = Number(el.dataset.index);
  const task = item().tasks[i];
  const text = await promptDialog(t("common.edit"), task.text);
  if (text && text !== task.text) await taskOp(() => api.editTask(item().name, i, text, task.text));
});
onAction("task-remove", async (el) => {
  const i = Number(el.dataset.index);
  const task = item().tasks[i];
  const name = item().name;
  await taskOp(() => api.removeTask(name, i, task.text));
  // Deshacer la vuelve a agregar (al final) con el mismo tildado.
  toast(t("detail.taskRemoved"), {
    label: t("common.undo"),
    run: () =>
      taskOp(async () => {
        const after = await api.addTask(name, task.text);
        if (task.done) await api.setTask(name, after.tasks.length - 1, true, task.text);
      }),
  });
});
onAction("reveal", async (el) => {
  await api.reveal(el.dataset.path!);
});
onAction("open-folder", async () => {
  await api.openPath(item().name);
});
onAction("open-vscode", async () => {
  await api.openInVscode(item().name);
});
onAction("view-version", async (el) => viewer(el.dataset.id!, `<pre class="code">${esc(await api.readVersion(item().name, el.dataset.id!))}</pre>`));
onAction("restore-version", async (el) => {
  if (!(await confirmDialog(t("detail.confirmRestore"), t("detail.restore")))) return;
  await api.restoreVersion(item().name, el.dataset.id!);
  versions = null;
  await refresh();
  toast(t("detail.restored"));
});
/** Sigue al item abierto cuando su carpeta cambia de nombre o de lugar. */
async function follow(from: string, to: string): Promise<void> {
  if (S.detail === from) {
    S.detail = to;
    if (shownName === from) shownName = to;
  }
  await refresh();
}

const baseOf = (name: string) => name.split("/").pop()!;

onAction("rename", async () => {
  const it = item();
  const base = baseOf(it.name);
  const next = await promptDialog(t(isGroup(it) ? "detail.renameGroupTo" : "detail.renameTo"), base);
  if (!next || next === base) return;
  const name = await api.renameFolder(it.name, next);
  await follow(it.name, name);
  toast(t("detail.renamed", { name: baseOf(name) }), {
    label: t("common.undo"),
    run: async () => {
      try {
        await follow(name, await api.renameFolder(name, base));
        toast(t("detail.renameUndone"));
      } catch (err) {
        toast(errorText(err), undefined, "error");
      }
    },
  });
});
onAction("trash-item", async () => {
  const it = item();
  if (!(await confirmDialog(t("items.confirmDelete", { n: 1 }), t("items.delete")))) return;
  const deleted = await api.trash(it.name);
  closeDetail();
  await refresh();
  toast(t("items.deletedN", { n: 1 }), { label: t("common.undo"), run: async () => { await api.untrash(deleted, it.name); await refresh(); } });
});
onAction("dissolve-group", async () => {
  const it = item();
  if (!(await confirmDialog(t("detail.dissolveConfirm", { n: it.members.length }), t("detail.dissolve")))) return;
  const out = await api.dissolveGroup(it.name);
  closeDetail();
  await refresh();
  toast(t("detail.dissolved"), {
    label: t("common.undo"),
    run: async () => {
      const group = await api.untrash(out.deleted, it.name);
      for (const m of out.members) await api.moveFolder(m, group);
      await refresh();
    },
  });
});
/** Mueve el item abierto a un grupo (o a la raíz con `""`), con aviso y Deshacer (v1 app.js:2088). */
async function moveTo(dest: string): Promise<void> {
  const it = item();
  const from = it.group;
  const name = await api.moveFolder(it.name, dest);
  await follow(it.name, name);
  const group = dest ? itemByName(dest) : null;
  toast(dest ? t("detail.movedTo", { group: group ? titleOf(group) : baseOf(dest) }) : t("detail.ungrouped"), {
    label: t("common.undo"),
    run: async () => {
      try {
        await follow(name, await api.moveFolder(name, from));
        toast(t("detail.moveUndone"));
      } catch (err) {
        toast(errorText(err), undefined, "error");
      }
    },
  });
}

onAction("ungroup", () => moveTo(""));
onAction("to-group", async () => {
  const groups = S.items.filter((i) => i.kind === "group" && i.name !== item().group);
  if (!groups.length) return toast(t("detail.noGroups"));
  const choice = await selectDialog(
    t("detail.whichGroup", { title: titleOf(item()) }),
    groups.map((g) => ({ value: g.name, label: g.title && g.title !== baseOf(g.name) ? `${titleOf(g)} · ${baseOf(g.name)}` : baseOf(g.name) })),
    t("detail.toGroup"),
  );
  if (choice) await moveTo(choice);
});

// ---------- sesiones del item ----------

onAction("sess-show-all", () => {
  allSessions = true;
  render();
});
onAction("detail-new-session", () => newSessionModal([item().name]));
onAction("detail-add-to-session", () => addToSessionModal([item().name]));
onAction("detail-plan", async () => {
  const it = item();
  if (dirty()) return toast(t("detail.saveFirst"), undefined, "error");
  const planned = wf().states.find((x) => x.category === "planned");
  if (pastPlanning(it)) {
    const text = t("detail.replanConfirm", { name: titleOf(it), status: stateLabel(statusOf(it)), plan: planned ? label(planned.labels, planned.id) : "" });
    if (!(await confirmDialog(text, t("detail.replan")))) return;
  }
  const open = S.sessions.filter((x) => x.state !== "closed" && inSession(x, it.name)).length;
  if (open && !(await confirmDialog(t("detail.planOpenSessions", { name: titleOf(it), n: open }), t("detail.planAnother")))) return;
  await planItem(it.name);
  toast(t("detail.planStarted", { name: titleOf(it), plan: planned ? label(planned.labels, planned.id) : "" }));
});
onAction("detail-reflect", () => reflectItems([item().name]));
onAction("detail-review", () => openReview(item()));
onAction("sess-remove-item", async (el, ev) => {
  ev.stopPropagation();
  const s = sessionById(el.dataset.id!);
  if (!s || !(await confirmDialog(t("session.removeConfirm", { item: item().name, name: s.name }), t("session.removeItem")))) return;
  await api.sessionRemoveItem(s.id, item().name);
  await loadSessions();
});
onAction("sess-split", async (el, ev) => {
  ev.stopPropagation();
  const s = sessionById(el.dataset.id!);
  if (!s || !(await confirmDialog(t("session.splitConfirm", { item: item().name, name: s.name }), t("session.split")))) return;
  const { cols, rows } = termSize();
  await launched(await api.sessionSplit(s.id, item().name, cols, rows));
});
