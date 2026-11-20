// Notas personales: lista con búsqueda, filtro por item y orden; editor, título (sin renombrar el archivo),
// vínculos con items y borrar a la papelera del sistema. Las notas viven fuera de los proyectos (la carpeta
// puede ser una de un vault de Obsidian).

import { api } from "../api";
import { onAction, onChange, registerView, render } from "../app";
import { confirmDialog, esc, fmtAgo, promptDialog, toast } from "../dom";
import { errorText, lang, t } from "../i18n";
import { ico } from "../icons";
import { renderMarkdown } from "../markdown";
import { S, titleOf } from "../state";
import { launched } from "../sessions";
import { termSize } from "../terminal";
import type { ApiError, Item, Note } from "../types";

/** Nota de Reflexionar (por el título que le pide al agente) vinculada a algún item. */
const isLearnings = (n: Note) => n.items.length > 0 && n.title.startsWith(t("agent.reflect.noteTitle", { label: "" }).trim());

/** Filtro por item: "" = todas, `NO_ITEMS` = sin vínculo, si no la clave del item. */
const NO_ITEMS = "\u0000none";

let editing = false;
/** ¿Hay una nota en edición? (el refresco por cambios en disco no la relee). */
export const noteEditing = () => editing;
let folder: { folder: string; isDefault: boolean } | null = null;

function pref(key: string, fallback: string): string {
  try {
    return localStorage.getItem(`wb.notes.${key}`) ?? fallback;
  } catch {
    return fallback;
  }
}
function setPref(key: string, value: string): void {
  try {
    localStorage.setItem(`wb.notes.${key}`, value);
  } catch {
    // sin almacenamiento: queda solo en memoria
  }
}
let itemFilter = pref("item", "");
let sortBy = pref("sort", "mtime") === "title" ? "title" : "mtime";

export async function loadNotes(): Promise<void> {
  try {
    [S.notes, folder] = await Promise.all([api.notes(), api.notesFolder()]);
    if (S.note && !S.notes.some((n) => n.id === S.note!.id)) S.note = null;
  } catch (err) {
    toast(errorText(err), undefined, "error");
  }
}

/** Cómo se nombra un item dentro de las notas: su id, o la carpeta si no tiene (como el backend). */
const itemKey = (it: Item) => it.ids[0] ?? it.name;
const itemOfKey = (key: string) => S.items.find((i) => i.name === key || i.ids.includes(key));

function chip(key: string, unlink: boolean): string {
  const it = itemOfKey(key);
  const text = it ? `${key} · ${titleOf(it)}` : key;
  const x = unlink
    ? `<button class="icon-btn" data-action="note-unlink" data-key="${esc(key)}" title="${esc(t("notes.unlink"))}" aria-label="${esc(t("notes.unlink"))}">${ico("x")}</button>`
    : "";
  return `<span class="chip" title="${esc(text)}">${esc(unlink ? text : key)}${x}</span>`;
}

function visible(): Note[] {
  const q = S.noteFilter.toLowerCase();
  return S.notes
    .filter((n) => !q || `${n.title} ${n.excerpt} ${n.items.join(" ")} ${n.author}`.toLowerCase().includes(q))
    .filter((n) => !itemFilter || (itemFilter === NO_ITEMS ? !n.items.length : n.items.includes(itemFilter)))
    .sort(sortBy === "title" ? (a, b) => a.title.localeCompare(b.title, lang()) : (a, b) => b.mtime - a.mtime);
}

function toolbar(): string {
  const keys = [...new Set(S.notes.flatMap((n) => n.items))].sort((a, b) => a.localeCompare(b, lang()));
  if (itemFilter && itemFilter !== NO_ITEMS && !keys.includes(itemFilter)) keys.push(itemFilter);
  const opt = (v: string, text: string, sel: boolean) => `<option value="${esc(v)}"${sel ? " selected" : ""}>${esc(text)}</option>`;
  const items = [opt("", t("notes.allItems"), !itemFilter), opt(NO_ITEMS, t("notes.noItems"), itemFilter === NO_ITEMS), ...keys.map((k) => opt(k, k, itemFilter === k))];
  return `<div class="toolbar"><h1>${esc(t("nav.notes"))}</h1>
      ${folder ? `<span class="muted" title="${esc(t("notes.folderHint"))}">${esc(folder.folder)}</span>` : ""}
      <span class="spacer"></span>
      <select data-change="notes-item" aria-label="${esc(t("notes.filterItem"))}">${items.join("")}</select>
      <select data-change="notes-sort" aria-label="${esc(t("notes.sort"))}">${opt("mtime", t("notes.sortRecent"), sortBy === "mtime")}${opt("title", t("notes.sortTitle"), sortBy === "title")}</select>
      <div class="search">${ico("search")}<input id="notes-search" type="search" placeholder="${esc(t("notes.search"))}" value="${esc(S.noteFilter)}"></div>
      <button class="btn primary" data-action="note-new">${ico("plus")}${esc(t("notes.new"))}</button></div>`;
}

function listHtml(): string {
  const list = visible()
    .map(
      (n) => `<li class="note${S.note?.id === n.id ? " sel" : ""}" data-action="note-open" data-id="${esc(n.id)}">
        <b>${esc(n.title)}</b><span class="muted">${esc(fmtAgo(n.mtime))}${n.author ? ` · ${esc(n.author)}` : ""}</span>
        ${n.items.length ? `<span class="note-items">${n.items.map((k) => chip(k, false)).join("")}</span>` : ""}
        <span class="excerpt">${esc(n.excerpt)}</span></li>`,
    )
    .join("");
  const empty = S.notes.length ? t("notes.noMatch") : t("notes.none");
  return list || `<li class="empty">${esc(empty)}</li>`;
}

function linkPicker(note: Note): string {
  const options = S.items
    .filter((it) => !note.items.includes(itemKey(it)))
    .map((it) => `<option value="${esc(itemKey(it))}">${esc(`${itemKey(it)} · ${titleOf(it)}`)}</option>`)
    .join("");
  return options ? `<select data-change="note-link" aria-label="${esc(t("notes.link"))}"><option value="">${esc(t("notes.link"))}</option>${options}</select>` : "";
}

function editorHtml(): string {
  const note = S.note;
  if (!note) return `<p class="empty">${esc(t("notes.pick"))}</p>`;
  const actions = editing
    ? `<button class="btn primary" data-action="note-save">${esc(t("common.save"))}</button><button class="btn" data-action="note-cancel">${esc(t("common.cancel"))}</button>`
    : `${isLearnings(note) ? `<button class="btn" data-action="note-apply" title="${esc(t("learn.hint"))}">${ico("sparkle")}${esc(t("learn.action"))}</button>` : ""}
       <button class="btn" data-action="note-edit">${ico("edit")}${esc(t("common.edit"))}</button>
       <button class="icon-btn" data-action="note-reveal" title="${esc(t("notes.reveal"))}" aria-label="${esc(t("notes.reveal"))}">${ico("folder")}</button>
       <button class="icon-btn" data-action="note-delete" title="${esc(t("notes.deleteHint"))}" aria-label="${esc(t("notes.delete"))}">${ico("trash")}</button>`;
  return `<section class="card note-editor">
      <div class="toolbar"><h2 class="grow">${esc(note.title)}
        <button class="icon-btn" data-action="note-title" title="${esc(t("notes.retitleHint"))}" aria-label="${esc(t("notes.retitle"))}">${ico("edit")}</button></h2>${actions}</div>
      <div class="note-links"><span class="muted">${esc(t("notes.linkedTo"))}</span>${note.items.map((k) => chip(k, true)).join("") || `<span class="muted">${esc(t("notes.noLinks"))}</span>`}${linkPicker(note)}
        ${note.author ? `<span class="spacer"></span><span class="muted">${esc(t("notes.author", { author: note.author }))}</span>` : ""}</div>
      ${editing ? `<textarea id="note-body" spellcheck="true">${esc(note.body ?? "")}</textarea>` : `<div class="md">${renderMarkdown(note.body ?? "")}</div>`}</section>`;
}

function renderNotes(): string {
  return `<div class="page wide">${toolbar()}<div class="notes-grid"><ul class="notes-list">${listHtml()}</ul>${editorHtml()}</div></div>`;
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

/** Después de un cambio que devuelve la nota: la deja abierta y recarga la lista. */
async function updated(note: Note): Promise<void> {
  S.note = { ...note, body: note.body ?? S.note?.body };
  await loadNotes();
  render();
}

registerView("notes", renderNotes);

onAction("note-open", (el) => open(el.dataset.id!));
onAction("note-apply", async () => {
  const note = S.note;
  if (!note) return;
  const which = await promptDialog(t("learn.which"), "", true);
  if (which === null) return;
  try {
    const { cols, rows } = termSize();
    const out = await api.applyLearnings(note.id, which, cols, rows);
    await launched(out.session);
    toast(t("learn.started"));
  } catch (err) {
    toast(errorText(err as ApiError), undefined, "error");
  }
});
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
  const note = await api.noteCreate(title, itemFilter && itemFilter !== NO_ITEMS ? [itemFilter] : []);
  await loadNotes();
  await open(note.id);
  editing = true;
  render();
});
onAction("note-title", async () => {
  const note = S.note;
  if (!note) return;
  if (editing) return toast(t("notes.saveFirst"), undefined, "error");
  const title = await promptDialog(t("notes.retitlePrompt"), note.title);
  if (!title || title === note.title) return;
  try {
    await updated(await api.noteSetTitle(note.id, title, note.mtime));
    toast(t("notes.retitled"));
  } catch (err) {
    toast(errorText(err), (err as ApiError).code === "core.note.changed" ? { label: t("common.reload"), run: () => open(note.id) } : undefined, "error");
  }
});
onChange("note-link", async (el) => {
  const key = (el as HTMLSelectElement).value;
  if (!S.note || !key) return;
  if (editing) return toast(t("notes.saveFirst"), undefined, "error");
  try {
    await updated(await api.noteLink(S.note.id, key, true));
  } catch (err) {
    toast(errorText(err), undefined, "error");
  }
});
onAction("note-unlink", async (el, ev) => {
  ev.stopPropagation();
  if (!S.note) return;
  if (editing) return toast(t("notes.saveFirst"), undefined, "error");
  try {
    await updated(await api.noteLink(S.note.id, el.dataset.key!, false));
  } catch (err) {
    toast(errorText(err), undefined, "error");
  }
});
onAction("note-reveal", async () => {
  if (S.note) await api.noteReveal(S.note.id);
});
onAction("note-delete", async () => {
  const note = S.note;
  if (!note) return;
  if (!(await confirmDialog(t("notes.confirmDelete", { title: note.title }), t("notes.delete")))) return;
  try {
    const out = await api.noteDelete(note.id);
    S.note = null;
    editing = false;
    await loadNotes();
    render();
    toast(out.trash ? t("notes.deleted") : t("notes.movedTo", { path: out.movedTo ?? "" }));
  } catch (err) {
    toast(errorText(err), undefined, "error");
  }
});
onChange("notes-item", (el) => {
  itemFilter = (el as HTMLSelectElement).value;
  setPref("item", itemFilter);
  render();
});
onChange("notes-sort", (el) => {
  sortBy = (el as HTMLSelectElement).value === "title" ? "title" : "mtime";
  setPref("sort", sortBy);
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
