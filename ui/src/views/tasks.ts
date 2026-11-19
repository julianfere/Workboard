// Pendientes: las tareas abiertas de todos los items (por item) y la bandeja de tareas sin item.

import { api } from "../api";
import { onAction, onChange, refresh, registerView, render } from "../app";
import { esc, toast } from "../dom";
import { errorText, t } from "../i18n";
import { ico } from "../icons";
import { S, effectiveStatus, isClosed, titleOf } from "../state";
import { dueBadge, statusChip } from "./items";

export async function loadInbox(): Promise<void> {
  try {
    S.inbox = await api.inbox();
  } catch (err) {
    toast(errorText(err), undefined, "error");
  }
}

function inboxCard(): string {
  const rows = S.inbox
    .map(
      (task, i) => `<li class="task${task.done ? " done" : ""}"><input type="checkbox" data-change="inbox-toggle" data-index="${i}"${task.done ? " checked" : ""} aria-label="${esc(task.text)}">
        <span class="grow">${esc(task.text)}</span><button class="icon-btn" data-action="inbox-remove" data-index="${i}" title="${esc(t("common.delete"))}">${ico("trash")}</button></li>`,
    )
    .join("");
  return `<section class="card"><h2>${ico("inbox")}${esc(t("tasks.inbox"))}</h2>
    <ul class="tasks">${rows || `<li class="empty">${esc(t("tasks.inboxEmpty"))}</li>`}</ul>
    <div class="add-row"><input id="inbox-new" placeholder="${esc(t("tasks.inboxAdd"))}" data-enter="inbox-add"><button class="btn" data-action="inbox-add">${ico("plus")}</button>
    ${S.inbox.some((x) => x.done) ? `<button class="link" data-action="inbox-clear">${esc(t("tasks.clearDone"))}</button>` : ""}</div></section>`;
}

function renderTasks(): string {
  const items = S.items.filter((it) => !isClosed(effectiveStatus(it)) && it.tasks.some((x) => !x.done));
  const blocks = items
    .map((it) => {
      const tasks = it.tasks
        .map((task, i) => ({ task, i }))
        .filter(({ task }) => !task.done)
        .map(({ task, i }) => `<li class="task"><input type="checkbox" data-change="item-task-toggle" data-name="${esc(it.name)}" data-index="${i}" aria-label="${esc(task.text)}"><span class="grow">${esc(task.text)}</span></li>`)
        .join("");
      return `<section class="card"><h2 class="link-row" data-action="open-item" data-name="${esc(it.name)}">${statusChip(effectiveStatus(it))}<span class="grow">${esc(titleOf(it))}</span>${dueBadge(it)}<span class="muted">${esc(it.ids.join(", "))}</span></h2><ul class="tasks">${tasks}</ul></section>`;
    })
    .join("");
  return `<div class="page"><h1>${esc(t("nav.tasks"))}</h1>${inboxCard()}${blocks || `<p class="empty">${esc(t("tasks.none"))}</p>`}</div>`;
}

registerView("tasks", renderTasks);

onChange("item-task-toggle", async (el) => {
  const it = S.items.find((x) => x.name === el.dataset.name)!;
  const i = Number(el.dataset.index);
  await api.setTask(it.name, i, true, it.tasks[i].text);
  await refresh();
});
onChange("inbox-toggle", async (el) => {
  const i = Number(el.dataset.index);
  S.inbox = await api.inboxSet(i, (el as HTMLInputElement).checked, S.inbox[i].text);
  render();
});
onAction("inbox-add", async () => {
  const input = document.querySelector<HTMLInputElement>("#inbox-new")!;
  if (!input.value.trim()) return;
  S.inbox = await api.inboxAdd(input.value);
  render();
  document.querySelector<HTMLInputElement>("#inbox-new")?.focus();
});
onAction("inbox-remove", async (el) => {
  const i = Number(el.dataset.index);
  S.inbox = await api.inboxRemove(i, S.inbox[i].text);
  render();
});
onAction("inbox-clear", async () => {
  S.inbox = await api.inboxClearDone();
  render();
});

