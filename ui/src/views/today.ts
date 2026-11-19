// Hoy: el período actual, lo que está en curso, fechas pactadas cercanas o vencidas y próximas tareas.

import { api } from "../api";
import { onAction, onChange, refresh, registerView } from "../app";
import { daysUntil, esc, todayIso } from "../dom";
import { t } from "../i18n";
import { S, effectiveStatus, isClosed, metaText, stateDef, titleOf, wf } from "../state";
import type { Item } from "../types";
import { dueBadge, statusChip } from "./items";

/** Período (`AAAA-MM`) en que cae una fecha, según el día de inicio del flujo. */
export function periodKey(iso: string): string {
  const [y, m, d] = iso.split("-").map(Number);
  let year = y;
  let month = m;
  if (d < wf().period.startDay) {
    month -= 1;
    if (month === 0) {
      month = 12;
      year -= 1;
    }
  }
  return `${year}-${String(month).padStart(2, "0")}`;
}

function line(it: Item, extra = ""): string {
  return `<li class="link-row" data-action="open-item" data-name="${esc(it.name)}">${statusChip(effectiveStatus(it))}<span class="grow">${esc(titleOf(it))}</span>${extra}<span class="muted">${esc(it.ids.join(", "))}</span></li>`;
}

function renderToday(): string {
  const open = S.items.filter((it) => it.kind === "item" && !isClosed(effectiveStatus(it)));
  const cat = (it: Item) => stateDef(effectiveStatus(it))?.category;
  const active = open.filter((it) => cat(it) === "active" || cat(it) === "review");
  const dueKey = wf().fields.due;
  const dated = open
    .filter((it) => metaText(it, dueKey) && daysUntil(metaText(it, dueKey)) <= 14)
    .sort((a, b) => metaText(a, dueKey).localeCompare(metaText(b, dueKey)));
  const period = periodKey(todayIso());
  const inPeriod = open.filter((it) => (metaText(it, wf().fields.period) || (metaText(it, dueKey) ? periodKey(metaText(it, dueKey)) : "")) === period);
  const next = active
    .flatMap((it) => it.tasks.map((task, i) => ({ it, task, i })).filter(({ task }) => !task.done).slice(0, 2))
    .slice(0, 10);
  const nextTasks = next
    .map(({ it, task, i }) => `<li class="task"><input type="checkbox" data-change="today-task" data-name="${esc(it.name)}" data-index="${i}" aria-label="${esc(task.text)}"><span class="grow">${esc(task.text)}</span><span class="muted">${esc(titleOf(it))}</span></li>`)
    .join("");
  const card = (title: string, body: string, empty: string) =>
    `<section class="card"><h2>${esc(title)}</h2>${body ? `<ul class="list">${body}</ul>` : `<p class="empty">${esc(empty)}</p>`}</section>`;
  return `<div class="page"><h1>${esc(t("nav.today"))}</h1>
    <div class="stats">
      <div class="stat"><b>${open.length}</b><span>${esc(t("today.open"))}</span></div>
      <div class="stat"><b>${active.length}</b><span>${esc(t("today.active"))}</span></div>
      <div class="stat"><b>${inPeriod.length}</b><span>${esc(t("today.period", { period }))}</span></div>
      <div class="stat${dated.some((it) => daysUntil(metaText(it, dueKey)) < 0) ? " bad" : ""}"><b>${dated.filter((it) => daysUntil(metaText(it, dueKey)) < 0).length}</b><span>${esc(t("today.overdue"))}</span></div>
    </div>
    ${card(t("today.inProgress"), active.map((it) => line(it)).join(""), t("today.nothingActive"))}
    ${card(t("today.dates"), dated.map((it) => line(it, dueBadge(it))).join(""), t("today.noDates"))}
    <section class="card"><h2>${esc(t("today.nextTasks"))}</h2>${nextTasks ? `<ul class="tasks">${nextTasks}</ul>` : `<p class="empty">${esc(t("today.noTasks"))}</p>`}
      <div class="add-row"><input id="today-inbox" placeholder="${esc(t("tasks.inboxAdd"))}" data-enter="today-inbox"><button class="btn" data-action="today-inbox">${esc(t("tasks.inbox"))}</button></div></section>
  </div>`;
}

registerView("today", renderToday);

onChange("today-task", async (el) => {
  const it = S.items.find((x) => x.name === el.dataset.name)!;
  const i = Number(el.dataset.index);
  await api.setTask(it.name, i, true, it.tasks[i].text);
  await refresh();
});
onAction("today-inbox", async () => {
  const input = document.querySelector<HTMLInputElement>("#today-inbox")!;
  if (!input.value.trim()) return;
  S.inbox = await api.inboxAdd(input.value);
  input.value = "";
});
