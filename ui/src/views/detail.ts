// Detalle de un item: estado por pasos, campos con guardado automático, tareas, archivos con visor,
// README y versiones anteriores.

import { api } from "../api";
import { closeDetail, loadDetail, onAction, onChange, refresh, registerDetail, render } from "../app";
import { confirmDialog, esc, fmtAgo, fmtSize, promptDialog, toast } from "../dom";
import { errorText, label, t } from "../i18n";
import { ico } from "../icons";
import { renderMarkdown } from "../markdown";
import { canReflect, reflectItems } from "../reflect";
import { pluginItemCard } from "../plugins";
import { canReview, openReview } from "../review";
import { addToSessionModal, launched, loadSessions, newSessionModal, planItem, sessionById } from "../sessions";
import { S, effectiveStatus, inSession, itemByName, metaList, metaText, stateDef, titleOf, wf } from "../state";
import { termSize } from "../terminal";
import { openFileViewer } from "../viewer";
import type { ApiError, FileEntry, Item } from "../types";
import { statusChip } from "./items";
import { localTag, overridesTracker } from "../planning";
import { trackerCard } from "./tracker";
import { itemIds, openPromotion, renderGitTab } from "../git";

const item = (): Item => S.detailData!.item;

function steps(it: Item): string {
  const current = effectiveStatus(it);
  return `<div class="steps" role="group" aria-label="${esc(t("detail.status"))}">${wf()
    .states.map(
      (s) => `<button class="step${s.id === current ? " current" : ""}" data-action="set-status" data-status="${esc(s.id)}">
        <span class="dot ${s.category}"></span>${esc(label(s.labels, s.id))}</button>`,
    )
    .join("")}</div>`;
}

function fieldError(key: string): string {
  return S.fieldErrors[key] ? `<div class="field-error">${esc(S.fieldErrors[key])}</div>` : "";
}

function textField(key: string, labelText: string, type = "text", placeholder = "", tag = ""): string {
  const value = key in S.fieldDrafts ? S.fieldDrafts[key] : metaText(item(), key);
  return `<label class="field"><span>${esc(labelText)}${tag}</span><input type="${type}" value="${esc(value)}" data-change="field" data-key="${esc(key)}" placeholder="${esc(placeholder)}"${S.fieldErrors[key] ? ' aria-invalid="true"' : ""}>${fieldError(key)}</label>`;
}

function fieldsCard(it: Item): string {
  const f = wf().fields;
  const types = wf().types.map((x) => `<option value="${esc(x.id)}"${metaText(it, f.type) === x.id ? " selected" : ""}>${esc(label(x.labels, x.id))}</option>`).join("");
  const envs = metaList(it, f.environments);
  const envBoxes = wf()
    .environments.map(
      (e) => `<label class="check"><input type="checkbox" data-change="env" data-env="${esc(e.id)}"${envs.includes(e.id) ? " checked" : ""}>${esc(label(e.labels, e.id))}</label>`,
    )
    .join("");
  return `<section class="card"><h2>${esc(t("detail.fields"))}</h2>
    <label class="field">${esc(t("items.type"))}<select data-change="field" data-key="${esc(f.type)}"><option value=""></option>${types}</select>${fieldError(f.type)}</label>
    <div class="field">${esc(t("detail.environments"))}<div class="checks">${envBoxes}</div>${fieldError(f.environments)}</div>
    ${textField(f.notes, t("items.notes"), "text", t("detail.notesHint"))}
    ${textField(f.branch, t("detail.branch"))}
    ${textField(f.title, t("items.title"), "text", it.title)}
    <h3>${esc(t("detail.planning"))}</h3>
    ${textField(f.due, t("detail.due"), "date", "", overridesTracker(it, "due") ? localTag() : "")}
    ${textField(f.period, t("detail.period"), "text", t("detail.periodHint"), overridesTracker(it, "period") ? localTag() : "")}
    ${textField(f.hours, t("detail.hours"), "text", "", overridesTracker(it, "hours") ? localTag() : "")}
    ${f.extra.map((x) => textField(x.key, x.key)).join("")}
    ${textField(f.itemIds, t("detail.ids"), "text", it.ids.join(", "))}
  </section>`;
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
    <div class="add-row"><input id="task-new" placeholder="${esc(t("detail.addTask"))}" data-enter="task-add"><button class="btn" data-action="task-add">${ico("plus")}</button></div>
  </section>`;
}

function membersCard(it: Item): string {
  if (it.kind !== "group") return "";
  const rows = it.members
    .map((m) => itemByName(m))
    .filter(Boolean)
    .map((m) => `<li data-action="open-item" data-name="${esc(m!.name)}" class="link-row">${statusChip(effectiveStatus(m!))}<span class="grow">${esc(titleOf(m!))}</span><span class="muted">${esc(m!.ids.join(", "))}</span></li>`)
    .join("");
  return `<section class="card"><h2>${esc(t("detail.members"))}</h2><ul class="list">${rows || `<li class="empty">—</li>`}</ul></section>`;
}

const SESSIONS_SHOWN = 6;
let allSessions = false;

/** Se puede planear un item que todavía no arrancó (o que ya está en planificación). */
function canPlan(it: Item): boolean {
  if (it.kind !== "item") return false;
  const cat = stateDef(effectiveStatus(it))?.category ?? "unknown";
  return ["backlog", "planned", "unknown"].includes(cat);
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

function summaryTab(it: Item): string {
  const deploy = it.deploy ? `<section class="card"><h2>${esc(t("detail.deploy"))}</h2><div class="md">${renderMarkdown(it.deploy)}</div></section>` : "";
  return `<div class="detail-grid"><div>${tasksCard(it)}${membersCard(it)}${deploy}</div><div>${sessionsCard(it)}${pluginItemCard(it)}${trackerCard(it)}${fieldsCard(it)}</div></div>`;
}

function readmeTab(): string {
  const text = S.detailData!.readme;
  return text
    ? `<section class="card md">${renderMarkdown(text.replace(/^---[\s\S]*?\n---\s*\n/, ""))}</section>`
    : `<p class="empty">${esc(t("detail.noReadme"))}</p>`;
}

function fileRow(f: FileEntry, runnable: boolean): string {
  const marks = S.detailData!.runs[f.path] ?? {};
  const envs = runnable
    ? wf()
        .environments.map(
          (e) => `<button class="env${marks[e.id] ? " on" : ""}" data-action="toggle-ran" data-path="${esc(f.path)}" data-env="${esc(e.id)}" title="${esc(t("detail.ranIn", { env: label(e.labels, e.id) }))}">${esc(label(e.labels, e.id))}</button>`,
        )
        .join("")
    : "";
  return `<li class="file"><span class="grow"><button class="link" data-action="view-file" data-path="${esc(f.path)}">${esc(f.name)}</button>
      <span class="muted">${esc(f.path.split("/").slice(1, -1).join("/"))}</span></span>
    <span class="envs">${envs}</span><span class="muted size">${esc(fmtSize(f.size))}</span><span class="muted">${esc(fmtAgo(f.mtime))}</span>
    <button class="icon-btn" data-action="reveal" data-path="${esc(f.path)}" title="${esc(t("detail.reveal"))}">${ico("folder")}</button></li>`;
}

function filesTab(it: Item): string {
  const cats = [...wf().files.categories, wf().files.fallbackCategory];
  const blocks = cats
    .map((c) => {
      const files = it.files.filter((f) => f.cat === c.id).sort((a, b) => b.mtime - a.mtime);
      if (!files.length) return "";
      return `<section class="card"><h2>${esc(label(c.labels, c.id))} <span class="muted">${files.length}</span></h2><ul class="files">${files.map((f) => fileRow(f, c.runnable)).join("")}</ul></section>`;
    })
    .join("");
  return blocks || `<p class="empty">${esc(t("detail.noFiles"))}</p>`;
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
        .map((v) => `<li><span class="grow">${esc(new Date(v.ts * 1000).toLocaleString())}</span><span class="muted">${esc(fmtSize(v.size))}</span>
          <button class="btn" data-action="view-version" data-id="${esc(v.id)}">${esc(t("detail.view"))}</button>
          <button class="btn" data-action="restore-version" data-id="${esc(v.id)}">${esc(t("detail.restore"))}</button></li>`)
        .join("")}</ul></section>`
    : `<p class="empty">${esc(t("detail.noVersions"))}</p>`;
}

function renderDetail(): string {
  const it = item();
  const tabs = (["summary", "readme", "files", "versions", "git"] as const)
    .map((tab) => `<button class="tab${S.detailTab === tab ? " sel" : ""}" data-action="detail-tab" data-tab="${tab}">${esc(t(`detail.tab.${tab}`))}${tab === "files" ? ` <span class="muted">${it.files.length}</span>` : ""}</button>`)
    .join("");
  const body = { summary: summaryTab, readme: readmeTab, files: filesTab, versions: versionsTab, git: renderGitTab }[S.detailTab](it);
  const inGroup = it.group ? `<button class="btn" data-action="ungroup">${ico("folderOut")}${esc(t("detail.ungroup"))}</button>` : it.kind === "item" ? `<button class="btn" data-action="to-group">${ico("folderIn")}${esc(t("detail.toGroup"))}</button>` : "";
  return `<div class="page wide detail">
    <div class="detail-head"><button class="icon-btn" data-action="back" title="${esc(t("common.back"))}">${ico("chevronLeft")}</button>
      <div class="grow"><h1>${esc(titleOf(it))}</h1><div class="muted">${esc(it.name)}${it.ids.length ? ` · ${esc(it.ids.join(", "))}` : ""}${it.dupes?.length ? ` · <span class="warn-text">${esc(t("alerts.dupes"))}</span>` : ""}</div></div>
      <button class="btn" data-action="reveal" data-path="${esc(it.name)}">${ico("folder")}${esc(t("detail.openFolder"))}</button>
      ${inGroup}
      ${itemIds(it).length ? `<button class="btn" data-action="promote-item" title="${esc(t("promo.buttonHint"))}">${ico("promote")}${esc(t("promo.button"))}</button>` : ""}
      <button class="btn" data-action="rename">${ico("edit")}${esc(t("detail.rename"))}</button>
      <button class="btn danger" data-action="trash-item">${ico("trash")}${esc(t("items.delete"))}</button>
    </div>
    ${steps(it)}
    <div class="tabs" role="tablist">${tabs}</div>
    ${body}
  </div>`;
}

// ---------- guardado ----------

async function save(fields: Record<string, unknown>): Promise<void> {
  const name = item().name;
  try {
    await api.updateFields(name, fields);
    for (const k of Object.keys(fields)) {
      delete S.fieldErrors[k];
      delete S.fieldDrafts[k];
    }
    await refresh();
  } catch (err) {
    const e = err as ApiError;
    const key = e.params?.key;
    if (key) {
      S.fieldErrors[key] = errorText(err);
      if (typeof fields[key] === "string") S.fieldDrafts[key] = fields[key] as string;
    }
    else toast(errorText(err), undefined, "error");
    render();
  }
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

registerDetail(renderDetail);

onAction("back", () => closeDetail());
onAction("promote-item", () => openPromotion([item().name]));
onAction("detail-tab", (el) => {
  S.detailTab = el.dataset.tab as typeof S.detailTab;
  versions = null;
  render();
});
onAction("set-status", (el) => save({ [wf().fields.status]: el.dataset.status }));
onChange("field", (el) => save({ [el.dataset.key!]: (el as HTMLInputElement).value }));
onChange("env", () => {
  const envs = [...document.querySelectorAll<HTMLInputElement>('[data-change="env"]')].filter((b) => b.checked).map((b) => b.dataset.env!);
  return save({ [wf().fields.environments]: envs });
});
onChange("task-toggle", (el) => {
  const i = Number(el.dataset.index);
  const task = item().tasks[i];
  return taskOp(() => api.setTask(item().name, i, (el as HTMLInputElement).checked, task.text));
});
onAction("task-add", async () => {
  const input = document.querySelector<HTMLInputElement>("#task-new")!;
  if (!input.value.trim()) return;
  await taskOp(() => api.addTask(item().name, input.value));
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
  await taskOp(() => api.removeTask(item().name, i, task.text));
  toast(t("detail.taskRemoved"), { label: t("common.undo"), run: () => taskOp(() => api.addTask(item().name, task.text)) });
});
onAction("toggle-ran", async (el) => {
  const path = el.dataset.path!;
  const on = !el.classList.contains("on");
  await api.markRan(path, el.dataset.env!, on);
  await loadDetail(item().name);
});
onAction("view-file", (el) => {
  const f = item().files.find((x) => x.path === el.dataset.path);
  if (f) openFileViewer(f);
});
onAction("reveal", async (el) => {
  await api.reveal(el.dataset.path!);
});
onAction("view-version", async (el) => viewer(el.dataset.id!, `<pre class="code">${esc(await api.readVersion(item().name, el.dataset.id!))}</pre>`));
onAction("restore-version", async (el) => {
  if (!(await confirmDialog(t("detail.confirmRestore"), t("detail.restore")))) return;
  await api.restoreVersion(item().name, el.dataset.id!);
  versions = null;
  await refresh();
  toast(t("detail.restored"));
});
onAction("rename", async () => {
  const it = item();
  const base = it.name.split("/").pop()!;
  const next = await promptDialog(t("detail.renameTo"), base);
  if (!next || next === base) return;
  const name = await api.renameFolder(it.name, next);
  S.detail = name;
  await refresh();
});
onAction("trash-item", async () => {
  const it = item();
  if (!(await confirmDialog(t("items.confirmDelete", { n: 1 }), t("items.delete")))) return;
  const deleted = await api.trash(it.name);
  closeDetail();
  await refresh();
  toast(t("items.deletedN", { n: 1 }), { label: t("common.undo"), run: async () => { await api.untrash(deleted, it.name); await refresh(); } });
});
onAction("ungroup", async () => {
  const name = await api.moveFolder(item().name, "");
  S.detail = name;
  await refresh();
});
onAction("to-group", async () => {
  const groups = S.items.filter((i) => i.kind === "group");
  if (!groups.length) return toast(t("detail.noGroups"));
  const choice = await promptDialog(t("detail.whichGroup", { groups: groups.map((g) => g.name).join(", ") }), groups[0].name);
  if (!choice) return;
  const name = await api.moveFolder(item().name, choice);
  S.detail = name;
  await refresh();
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
  if (!(await confirmDialog(t("session.planConfirm", { name: titleOf(it) }), t("session.plan")))) return;
  await planItem(it.name);
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
