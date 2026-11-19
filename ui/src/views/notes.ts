// Notas personales: lista con filtro, editor y nota nueva. Las notas viven fuera de los proyectos.

import { api } from "../api";
import { onAction, registerView, render } from "../app";
import { esc, fmtAgo, promptDialog, toast } from "../dom";
import { errorText, t } from "../i18n";
import { ico } from "../icons";
import { renderMarkdown } from "../markdown";
import { S } from "../state";
import type { ApiError } from "../types";

let editing = false;

export async function loadNotes(): Promise<void> {
  try {
    S.notes = await api.notes();
  } catch (err) {
    toast(errorText(err), undefined, "error");
  }
}

function renderNotes(): string {
  const q = S.noteFilter.toLowerCase();
  const list = S.notes
    .filter((n) => !q || `${n.title} ${n.excerpt} ${n.items.join(" ")}`.toLowerCase().includes(q))
    .map(
      (n) => `<li class="note${S.note?.id === n.id ? " sel" : ""}" data-action="note-open" data-id="${esc(n.id)}">
        <b>${esc(n.title)}</b><span class="muted">${esc(fmtAgo(n.mtime))}${n.items.length ? ` · ${esc(n.items.join(", "))}` : ""}</span>
        <span class="excerpt">${esc(n.excerpt)}</span></li>`,
    )
    .join("");
  const note = S.note;
  const editor = note
    ? `<section class="card note-editor"><div class="toolbar"><h2 class="grow">${esc(note.title)}</h2>
        ${editing
          ? `<button class="btn primary" data-action="note-save">${esc(t("common.save"))}</button><button class="btn" data-action="note-cancel">${esc(t("common.cancel"))}</button>`
          : `<button class="btn" data-action="note-edit">${ico("edit")}${esc(t("common.edit"))}</button>`}</div>
        ${editing ? `<textarea id="note-body" spellcheck="true">${esc(note.body ?? "")}</textarea>` : `<div class="md">${renderMarkdown(note.body ?? "")}</div>`}</section>`
    : `<p class="empty">${esc(t("notes.pick"))}</p>`;
  return `<div class="page wide"><div class="toolbar"><h1>${esc(t("nav.notes"))}</h1>
      <div class="search">${ico("search")}<input id="notes-search" type="search" placeholder="${esc(t("notes.search"))}" value="${esc(S.noteFilter)}"></div>
      <span class="spacer"></span><button class="btn primary" data-action="note-new">${ico("plus")}${esc(t("notes.new"))}</button></div>
    <div class="notes-grid"><ul class="notes-list">${list || `<li class="empty">${esc(t("notes.none"))}</li>`}</ul>${editor}</div></div>`;
}

async function save(): Promise<void> {
  const note = S.note!;
  const body = document.querySelector<HTMLTextAreaElement>("#note-body")!.value;
  try {
    S.note = await api.noteSave(note.id, body, note.mtime);
    editing = false;
    await loadNotes();
    render();
  } catch (err) {
    toast(errorText(err), (err as ApiError).code === "core.note.changed" ? { label: t("common.reload"), run: () => open(note.id) } : undefined, "error");
  }
}

async function open(id: string): Promise<void> {
  S.note = await api.noteRead(id);
  editing = false;
  render();
}

registerView("notes", renderNotes);

onAction("note-open", (el) => open(el.dataset.id!));
onAction("note-edit", () => {
  editing = true;
  render();
  document.querySelector<HTMLTextAreaElement>("#note-body")?.focus();
});
onAction("note-cancel", () => {
  editing = false;
  render();
});
onAction("note-save", () => save());
onAction("note-new", async () => {
  const title = await promptDialog(t("notes.title"));
  if (!title) return;
  const note = await api.noteCreate(title, []);
  await loadNotes();
  await open(note.id);
  editing = true;
  render();
});

document.addEventListener("keydown", (ev) => {
  if ((ev.ctrlKey || ev.metaKey) && ev.key.toLowerCase() === "s" && editing && S.view === "notes") {
    ev.preventDefault();
    save();
  }
});
document.addEventListener("input", (ev) => {
  const el = ev.target as HTMLInputElement;
  if (el.id !== "notes-search") return;
  S.noteFilter = el.value;
  render();
  const input = document.querySelector<HTMLInputElement>("#notes-search");
  input?.focus();
  input?.setSelectionRange(el.value.length, el.value.length);
});
