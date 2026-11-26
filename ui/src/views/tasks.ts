// Pendientes: las tareas de los items (un bloque por item, separados por período pactado) y la bandeja de tareas
// sin item. Filtros por estado, sesión abierta, ambiente corrido y fecha pactada (se guardan en las prefs de la UI,
// clave `tasks`); «Mostrar hechas». Cada tarea se tilda, edita, borra (con deshacer) o se mueve a otro item.

import { api } from "../api";
import { onAction, onChange, refresh, registerView, render, setNavCounter } from "../app";
import { esc, promptDialog, toast } from "../dom";
import { errorText, label, t } from "../i18n";
import { ico } from "../icons";
import { byUrgency, lateTag, passesDue, periodDiff, periodOf, periodRel, priorityBadge, priorityOf } from "../planning";
import { sessionsOf } from "../sessions";
import { S, effectiveStatus, isClosed, itemByName, matchesSearch, metaList, titleOf, wf } from "../state";
import { noSearchResults, searchBox } from "../search";
import { periodRange } from "../stats";
import type { Item, Task } from "../types";
import { DUE_FILTERS, type DueFilter } from "./grid";
import { statusChip } from "./items";

export async function loadInbox(): Promise<void> {
  try {
    S.inbox = await api.inbox();
  } catch (err) {
    toast(errorText(err), undefined, "error");
  }
}

// ---------- filtros (prefs de la UI, clave `tasks`) ----------

interface Filters {
  status: string;
  /** Id de sesión, "none" = sin sesión abierta. */
  session: string;
  env: string;
  due: DueFilter;
  showDone: boolean;
}

const F: Filters = { status: "", session: "", env: "", due: "", showDone: false };
let prefsLoaded = false;

function loadFilters(): void {
  if (prefsLoaded) return;
  prefsLoaded = true;
  api
    .uiPrefs()
    .then((p) => {
      const saved = p.tasks as Partial<Filters> | undefined;
      if (saved && typeof saved === "object") {
        Object.assign(F, saved);
        if (!DUE_FILTERS.includes(F.due)) F.due = "";
        render();
      }
    })
    .catch(() => undefined);
}

function saveFilters(): void {
  void api.setUiPref("tasks", { ...F }).catch(() => undefined);
}

// ---------- datos ----------

const openCount = (it: Item) => it.tasks.filter((x) => !x.done).length;
const openSessions = (it: Item) => sessionsOf(it.name).filter((s) => s.state !== "closed");

/** Tareas abiertas de todos los items más las de la bandeja (el contador del menú). */
export const openTaskCount = () => S.items.reduce((n, it) => n + openCount(it), 0) + S.inbox.filter((x) => !x.done).length;

function visibleItems(): Item[] {
  const pk = (it: Item) => periodOf(it).key || "9999-99";
  return S.items
    .filter(
      (it) =>
        matchesSearch(it) &&
        it.tasks.some((x) => F.showDone || !x.done) &&
        (!F.status || effectiveStatus(it) === F.status) &&
        (!F.session || (F.session === "none" ? !openSessions(it).length : openSessions(it).some((s) => s.id === F.session))) &&
        (!F.env || metaList(it, wf().fields.environments).includes(F.env)) &&
        passesDue(it, F.due),
    )
    .sort((a, b) => Number(!openCount(a)) - Number(!openCount(b)) || pk(a).localeCompare(pk(b)) || byUrgency(a, b));
}

// ---------- render ----------

const opt = (v: string, text: string, cur: string) => `<option value="${esc(v)}"${v === cur ? " selected" : ""}>${esc(text)}</option>`;

function toolbar(): string {
  const states = wf().states.map((s) => opt(s.id, label(s.labels, s.id), F.status)).join("");
  const sessions = S.sessions.filter((s) => s.state !== "closed").map((s) => opt(s.id, s.name, F.session)).join("");
  const envs = wf().environments.map((e) => opt(e.id, t("tasks.filter.ranIn", { env: label(e.labels, e.id) }), F.env)).join("");
  const dues = DUE_FILTERS.map((d) => opt(d, d ? t("tasks.filter.due", { what: t(`grid.due.${d}`) }) : t("grid.due.any"), F.due)).join("");
  return `<div class="toolbar"><h1>${esc(t("nav.tasks"))}</h1>
    ${searchBox()}
    <select data-change="tasks-filter" data-key="status" aria-label="${esc(t("tasks.filter.status"))}">${opt("", t("tasks.filter.anyStatus"), F.status)}${states}</select>
    <select data-change="tasks-filter" data-key="session" aria-label="${esc(t("tasks.filter.session"))}">${opt("", t("tasks.filter.anySession"), F.session)}${opt("none", t("tasks.filter.noSession"), F.session)}${sessions}</select>
    <select data-change="tasks-filter" data-key="env" aria-label="${esc(t("tasks.filter.env"))}">${opt("", t("tasks.filter.anyEnv"), F.env)}${envs}</select>
    <select data-change="tasks-filter" data-key="due" aria-label="${esc(t("grid.col.due"))}" title="${esc(t("grid.filter.dueHint"))}">${dues}</select>
    <label class="check"><input type="checkbox" data-change="tasks-show-done"${F.showDone ? " checked" : ""}>${esc(t("tasks.showDone"))}</label></div>`;
}

/** Fila de una tarea (`name` = "" en la bandeja). */
function taskRow(name: string, task: Task, i: number): string {
  const at = `data-name="${esc(name)}" data-index="${i}"`;
  return `<li class="task${task.done ? " done" : ""}"><input type="checkbox" data-change="tk-toggle" ${at}${task.done ? " checked" : ""} aria-label="${esc(task.text)}">
    <span class="grow" data-action="tk-edit" ${at}>${esc(task.text)}</span>
    <button class="icon-btn" data-action="tk-edit" ${at} title="${esc(t("common.edit"))}">${ico("edit")}</button>
    <button class="icon-btn" data-action="tk-move" ${at} title="${esc(t("tasks.move"))}">${ico("folderIn")}</button>
    <button class="icon-btn" data-action="tk-remove" ${at} title="${esc(t("common.delete"))}">${ico("trash")}</button></li>`;
}

function itemBlock(it: Item): string {
  const sess = openSessions(it)[0];
  const tasks = it.tasks.map((task, i) => (F.showDone || !task.done ? taskRow(it.name, task, i) : "")).join("");
  return `<section class="card task-block"><h2 class="link-row" data-action="open-item" data-name="${esc(it.name)}">${statusChip(effectiveStatus(it))}
      <span class="muted">${esc(it.ids.join(", "))}</span>${priorityBadge(priorityOf(it))}<span class="grow">${esc(titleOf(it))}${lateTag(it)}</span>
      ${sess ? `<span class="muted sm sess-tag"><span class="sdot ${sess.status}"></span>${esc(sess.name)}</span>` : ""}
      <span class="muted sm">${esc(t("tasks.openOf", { n: openCount(it), total: it.tasks.length }))}</span></h2>
    <ul class="tasks">${tasks}</ul></section>`;
}

function blocks(items: Item[]): string {
  // Separados por período pactado (si hay planificación): pasados, actual, futuros, sin fecha y sin abiertas.
  const planning = S.items.some((it) => periodOf(it).key);
  let last: string | null = null;
  return items
    .map((it) => {
      const k = periodOf(it).key;
      const key = openCount(it) ? k || "none" : "done";
      let head = "";
      if (planning && key !== last) {
        last = key;
        if (key === "done") head = `<div class="period-head">${esc(t("tasks.head.done"))}</div>`;
        else if (key === "none") head = `<div class="period-head">${esc(t("grid.period.none"))}</div>`;
        else {
          const n = periodDiff(k);
          head = `<div class="period-head ${n === 0 ? "cur" : n > 0 ? "future" : "past"}">${esc(t("grid.period.title", { range: periodRange(k) }))} <span class="muted">· ${esc(periodRel(k))}</span></div>`;
        }
      }
      return head + itemBlock(it);
    })
    .join("");
}

function inboxCard(): string {
  const open = S.inbox.filter((x) => !x.done).length;
  const done = S.inbox.length - open;
  const rows = S.inbox.map((task, i) => (F.showDone || !task.done ? taskRow("", task, i) : "")).join("");
  return `<aside class="card inbox-card"><h2>${ico("inbox")}<span class="grow">${esc(t("tasks.inbox"))}</span><span class="muted">${open}</span></h2>
    <p class="muted sm">${esc(t("tasks.inboxHint"))}</p>
    <ul class="tasks">${rows || `<li class="empty">${esc(t("tasks.inboxEmpty"))}</li>`}</ul>
    <div class="add-row"><input id="inbox-new" placeholder="${esc(t("tasks.inboxAdd"))}" data-enter="inbox-add"><button class="btn" data-action="inbox-add">${ico("plus")}</button></div>
    ${done ? `<button class="link sm" data-action="inbox-clear">${esc(t("tasks.clearDoneN", { n: done }))}</button>` : ""}</aside>`;
}

function renderTasks(): string {
  loadFilters();
  const list = blocks(visibleItems());
  return `<div class="page tasks-page">${toolbar()}<div class="tasks-wrap"><div>${list || `<p class="empty">${noSearchResults() || esc(t("tasks.none"))}</p>`}</div>${inboxCard()}</div></div>`;
}

registerView("tasks", renderTasks);
setNavCounter("tasks", openTaskCount);

// ---------- acciones ----------

const tasksOf = (name: string) => (name ? (itemByName(name)?.tasks ?? []) : S.inbox);

/** Después de escribir: relee los items (o solo la bandeja). */
async function after(name: string): Promise<void> {
  if (name) await refresh();
  else {
    await loadInbox();
    render();
  }
}

async function op(name: string, fn: () => Promise<unknown>): Promise<void> {
  try {
    await fn();
  } catch (err) {
    toast(errorText(err), undefined, "error");
  }
  await after(name);
}

onChange("tasks-filter", (el) => {
  const key = el.dataset.key as "status" | "session" | "env" | "due";
  (F as unknown as Record<string, string>)[key] = (el as HTMLSelectElement).value;
  saveFilters();
  render();
});
onChange("tasks-show-done", (el) => {
  F.showDone = (el as HTMLInputElement).checked;
  saveFilters();
  render();
});

onChange("tk-toggle", async (el) => {
  const name = el.dataset.name!;
  const i = Number(el.dataset.index);
  const task = tasksOf(name)[i];
  const done = (el as HTMLInputElement).checked;
  await op(name, () => (name ? api.setTask(name, i, done, task.text) : api.inboxSet(i, done, task.text)));
});
onAction("tk-edit", async (el, ev) => {
  ev.stopPropagation();
  const name = el.dataset.name!;
  const i = Number(el.dataset.index);
  const task = tasksOf(name)[i];
  const text = await promptDialog(t("common.edit"), task.text);
  if (!text || text === task.text) return;
  await op(name, () => (name ? api.editTask(name, i, text, task.text) : api.inboxEdit(i, text, task.text)));
});
onAction("tk-remove", async (el, ev) => {
  ev.stopPropagation();
  const name = el.dataset.name!;
  const i = Number(el.dataset.index);
  const task = tasksOf(name)[i];
  await op(name, () => (name ? api.removeTask(name, i, task.text) : api.inboxRemove(i, task.text)));
  // Deshacer la vuelve a agregar (al final) con el mismo tildado.
  toast(t("tasks.removed"), {
    label: t("common.undo"),
    run: () =>
      void op(name, async () => {
        if (name) {
          const it = await api.addTask(name, task.text);
          if (task.done) await api.setTask(name, it.tasks.length - 1, true, task.text);
        } else {
          const list = await api.inboxAdd(task.text);
          if (task.done) await api.inboxSet(list.length - 1, true, task.text);
        }
      }),
  });
});
onAction("tk-move", (el, ev) => {
  ev.stopPropagation();
  const src = el.dataset.name!;
  const i = Number(el.dataset.index);
  const task = tasksOf(src)[i];
  movePicker(src, task.text, async (dst) => {
    try {
      await api.moveTask(src, i, task.text, dst);
      toast(dst ? t("tasks.movedTo", { name: titleOf(itemByName(dst)!) }) : t("tasks.movedToInbox"));
    } catch (err) {
      toast(errorText(err), undefined, "error");
    }
    await Promise.all([refresh(), loadInbox()]);
    render();
  });
});

/** Elegir el item destino (o la bandeja, si la tarea es de un item). */
function movePicker(src: string, text: string, pick: (dst: string) => void): void {
  const back = document.createElement("div");
  back.className = "modal-back";
  back.innerHTML = `<div class="modal" role="dialog" aria-modal="true"><h2>${esc(t("tasks.moveTitle", { text: text.length > 50 ? `${text.slice(0, 50)}…` : text }))}</h2>
    <input data-q placeholder="${esc(t("tasks.moveSearch"))}"><ul class="list pick-list" data-list></ul>
    <div class="modal-actions"><button class="btn" data-close>${esc(t("common.cancel"))}</button></div></div>`;
  document.body.append(back);
  const q = back.querySelector<HTMLInputElement>("[data-q]")!;
  const list = back.querySelector<HTMLElement>("[data-list]")!;
  const close = () => back.remove();
  const draw = () => {
    const s = q.value.trim().toLowerCase();
    const cand = S.items
      .filter((it) => it.name !== src && (!s || [it.name, titleOf(it), it.ids.join(" ")].join(" ").toLowerCase().includes(s)))
      .sort((a, b) => Number(isClosed(effectiveStatus(a))) - Number(isClosed(effectiveStatus(b))) || b.last - a.last)
      .slice(0, 80);
    const inbox = src && !s ? `<li class="link-row" data-pick="">${ico("inbox")}<span class="grow">${esc(t("tasks.inboxNoItem"))}</span></li>` : "";
    list.innerHTML =
      inbox +
        cand.map((it) => `<li class="link-row" data-pick="${esc(it.name)}">${statusChip(effectiveStatus(it))}<span class="grow">${esc(titleOf(it))} <span class="muted">${esc(it.ids.join(", "))}</span></span></li>`).join("") ||
      `<li class="empty">${esc(t("items.empty"))}</li>`;
  };
  draw();
  q.oninput = draw;
  q.addEventListener("keydown", (e) => {
    if (e.key === "Enter") list.querySelector<HTMLElement>("[data-pick]")?.click();
  });
  back.addEventListener("keydown", (e) => e.key === "Escape" && close());
  back.querySelector<HTMLElement>("[data-close]")!.onclick = close;
  back.onclick = (e) => {
    if (e.target === back) close();
  };
  list.onclick = (e) => {
    const row = (e.target as HTMLElement).closest<HTMLElement>("[data-pick]");
    if (!row) return;
    close();
    pick(row.dataset.pick!);
  };
  q.focus();
}

onAction("inbox-add", async () => {
  const input = document.querySelector<HTMLInputElement>("#inbox-new")!;
  if (!input.value.trim()) return;
  S.inbox = await api.inboxAdd(input.value);
  render();
  document.querySelector<HTMLInputElement>("#inbox-new")?.focus();
});
onAction("inbox-clear", async () => {
  S.inbox = await api.inboxClearDone();
  render();
});
