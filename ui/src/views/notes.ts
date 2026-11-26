// Notas personales: lista con búsqueda, filtro por item, orden y menú contextual; editor con pestañas Editar /
// Vista previa (como la v1), título (sin renombrar el archivo), vínculos con items, abrir en Obsidian o con la
// app, y borrar a la papelera del sistema. Las notas viven fuera de los proyectos (la carpeta puede ser una de
// un vault de Obsidian).
//
// Lo que se escribe queda como borrador en este módulo (no en el `<textarea>`): cada `render()` (las sesiones
// y el refresco en vivo lo llaman seguido) rearma el editor desde el borrador y le devuelve el foco, la
// selección y el scroll. Abrir otra nota, cambiar la carpeta o cerrar la app piden confirmar si hay cambios.

import { api, inTauri, pickFolder } from "../api";
import { goView, onAction, onAfterRender, onChange, registerView, render } from "../app";
import { registerCloseGuard } from "../closeguard";
import { confirmDialog, esc, fmtAgo, fmtDateTimeFull, promptDialog, toast } from "../dom";
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

/** Cuerpo editado de la nota abierta (`S.note`); null = sin cambios. */
let draft: string | null = null;
/** ¿Hay cambios sin guardar? (el refresco por cambios en disco no relee las notas mientras tanto). */
export const noteEditing = () => draft !== null;
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
let mode: "edit" | "view" = pref("mode", "edit") === "view" ? "view" : "edit";

export async function loadNotes(): Promise<void> {
  try {
    [S.notes, folder] = await Promise.all([api.notes(), api.notesFolder()]);
    if (S.note && !draft && !S.notes.some((n) => n.id === S.note!.id)) S.note = null;
  } catch (err) {
    toast(errorText(err), undefined, "error");
  }
}

/** Si hay cambios sin guardar, pide descartarlos. `true` = se puede seguir (y el borrador se descarta). */
async function discardOk(): Promise<boolean> {
  if (draft === null) return true;
  if (!(await confirmDialog(t("notes.discardConfirm"), t("notes.discard")))) return false;
  draft = null;
  return true;
}

registerCloseGuard(() => (draft !== null && S.note ? t("notes.closeConfirm", { title: S.note.title }) : null));

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
      <button class="btn ghost" data-action="notes-folder" title="${esc(t("notes.changeFolderHint"))}">${ico("folder")}${esc(t("notes.changeFolder"))}</button>
      ${folder && !folder.isDefault ? `<button class="btn ghost" data-action="notes-folder-reset">${esc(t("notes.useDefault"))}</button>` : ""}
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

const statusHtml = (note: Note) =>
  draft !== null ? `<span class="unsaved-dot"></span>${esc(t("notes.unsaved"))}` : esc(t("notes.savedAt", { when: fmtDateTimeFull(note.mtime) }));

function editorHtml(): string {
  const note = S.note;
  if (!note) return `<p class="empty">${esc(t("notes.pick"))}</p>`;
  const body = draft ?? note.body ?? "";
  const tab = (m: "edit" | "view", text: string) =>
    `<button class="tab${mode === m ? " sel" : ""}" data-action="note-mode" data-mode="${m}" aria-pressed="${mode === m}">${esc(text)}</button>`;
  const dirty = draft !== null;
  return `<section class="card note-editor">
      <div class="toolbar"><h2 class="grow">${esc(note.title)}
        <button class="icon-btn" data-action="note-title" title="${esc(t("notes.retitleHint"))}" aria-label="${esc(t("notes.retitle"))}">${ico("edit")}</button></h2></div>
      <div class="note-links"><span class="muted">${esc(t("notes.linkedTo"))}</span>${note.items.map((k) => chip(k, true)).join("") || `<span class="muted">${esc(t("notes.noLinks"))}</span>`}${linkPicker(note)}
        ${note.author ? `<span class="spacer"></span><span class="muted">${esc(t("notes.author", { author: note.author }))}</span>` : ""}</div>
      <div class="tabs">${tab("edit", t("common.edit"))}${tab("view", t("notes.tabPreview"))}</div>
      ${mode === "edit" ? `<textarea id="note-body" spellcheck="false" aria-label="${esc(t("notes.content"))}">${esc(body)}</textarea>` : `<div class="md note-preview">${renderMarkdown(body)}</div>`}
      <div class="note-foot">
        <button class="btn primary" data-action="note-save"${dirty ? "" : " disabled"}>${esc(t("common.save"))}</button>
        <span class="muted note-status" id="note-status">${statusHtml(note)}</span>
        <button class="btn ghost" data-action="note-discard"${dirty ? "" : " hidden"}>${esc(t("notes.discardChanges"))}</button>
        ${isLearnings(note) ? `<button class="btn" data-action="note-apply" title="${esc(t("learn.hint"))}">${ico("sparkle")}${esc(t("learn.action"))}</button>` : ""}
        <span class="spacer"></span>
        <button class="btn ghost" data-action="note-open-ext" data-mode="obsidian" title="${esc(t("notes.openObsidianHint"))}">${esc(t("notes.openObsidian"))}</button>
        <button class="btn ghost" data-action="note-open-ext" data-mode="app" title="${esc(t("notes.openAppHint"))}">${ico("external")}${esc(t("notes.openApp"))}</button>
        <button class="btn ghost" data-action="note-reveal">${ico("folder")}${esc(t("notes.reveal"))}</button>
        <button class="btn ghost danger" data-action="note-delete" title="${esc(t("notes.deleteHint"))}">${ico("trash")}${esc(t("notes.delete"))}</button>
      </div></section>`;
}

// ---------- foco a través de los render ----------

/** Foco y selección del campo que se estaba usando, para devolvérselos después de rearmar el HTML. */
let keep: { id: string; start: number; end: number } | null = null;
/** Scroll del editor (de la nota `id`), con o sin foco. */
let scroll: { id: string; top: number } | null = null;
/** Después del próximo render: foco en el editor con el cursor al final (nota nueva). */
let focusEnd = false;

function captureFocus(): void {
  const a = document.activeElement as HTMLTextAreaElement | HTMLInputElement | null;
  keep = a && (a.id === "note-body" || a.id === "notes-search") ? { id: a.id, start: a.selectionStart ?? 0, end: a.selectionEnd ?? 0 } : null;
  const ta = document.querySelector<HTMLTextAreaElement>("#note-body");
  scroll = ta && S.note ? { id: S.note.id, top: ta.scrollTop } : null;
}

onAfterRender(() => {
  const k = keep;
  const sc = scroll;
  keep = scroll = null;
  if (S.view !== "notes" || S.detail) return;
  const ta = document.querySelector<HTMLTextAreaElement>("#note-body");
  if (focusEnd && ta) {
    focusEnd = false;
    ta.focus();
    ta.setSelectionRange(ta.value.length, ta.value.length);
    ta.scrollTop = ta.scrollHeight;
    return;
  }
  if (k) {
    const el = document.querySelector<HTMLInputElement | HTMLTextAreaElement>(`#${k.id}`);
    el?.focus();
    el?.setSelectionRange(k.start, k.end);
  }
  if (ta && sc && sc.id === S.note?.id) ta.scrollTop = sc.top;
});

function renderNotes(): string {
  captureFocus();
  return `<div class="page wide">${toolbar()}<div class="notes-grid"><ul class="notes-list">${listHtml()}</ul>${editorHtml()}</div></div>`;
}

// ---------- acciones ----------

async function save(): Promise<void> {
  const note = S.note;
  if (!note || draft === null) return;
  try {
    S.note = await api.noteSave(note.id, draft, note.mtime);
    draft = null;
    await loadNotes();
    render();
    toast(t("notes.saved"));
  } catch (err) {
    toast(errorText(err), (err as ApiError).code === "core.note.changed" ? { label: t("common.reload"), run: () => void reload(note.id) } : undefined, "error");
  }
}

/** Relee la nota del disco descartando el borrador (la acción «Recargar» de un conflicto). */
async function reload(id: string): Promise<void> {
  draft = null;
  S.note = await api.noteRead(id);
  render();
}

/** Abre una nota (confirma si la abierta tiene cambios). Volver a abrir la misma no pierde el borrador. */
async function open(id: string): Promise<boolean> {
  if (S.note?.id === id && draft !== null) return true;
  if (!(await discardOk())) return false;
  S.note = await api.noteRead(id);
  return true;
}

/** Abrir una nota desde otra vista (pestaña Notas del detalle). */
export async function openNote(id: string, { edit = false } = {}): Promise<void> {
  if (!(await open(id))) return;
  if (edit) {
    mode = "edit";
    focusEnd = true;
  }
  goView("notes");
}

/** Crea una nota y la abre en edición con el cursor al final (como la v1). */
export async function newNote(items: string[]): Promise<void> {
  if (!(await discardOk())) return;
  const title = await promptDialog(t("notes.title"));
  if (!title) return;
  const note = await api.noteCreate(title, items);
  await loadNotes();
  await openNote(note.id, { edit: true });
}

/** Cambia la carpeta de notas (`null` = la predeterminada). Confirma si hay cambios sin guardar. */
export async function setNotesFolder(path: string | null): Promise<void> {
  if (!(await discardOk())) return render(); // el campo de Ajustes vuelve a la carpeta actual
  try {
    S.settings = await api.setSettings({ ...S.settings!, notesFolder: path });
    S.note = null;
    await loadNotes();
  } catch (err) {
    toast(errorText(err), undefined, "error");
  }
  render();
}

/** Después de un cambio que devuelve la nota: la deja abierta (con el borrador, si hay) y recarga la lista. */
async function updated(note: Note): Promise<void> {
  if (S.note?.id === note.id) S.note = { ...note, body: note.body ?? S.note?.body };
  await loadNotes();
  render();
}

const noteOf = (el: HTMLElement): Note | null => (el.dataset.id ? (S.note?.id === el.dataset.id ? S.note : S.notes.find((n) => n.id === el.dataset.id) ?? null) : S.note);

registerView("notes", renderNotes);

onAction("note-open", async (el) => {
  if (await open(el.dataset.id!)) render();
});
onAction("note-mode", (el) => {
  mode = el.dataset.mode === "view" ? "view" : "edit";
  setPref("mode", mode);
  render();
  if (mode === "edit") document.querySelector<HTMLTextAreaElement>("#note-body")?.focus();
});
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
onAction("note-save", () => save());
onAction("note-discard", async () => {
  if (!(await discardOk())) return;
  render();
});
onAction("note-new", () => newNote(itemFilter && itemFilter !== NO_ITEMS ? [itemFilter] : []));
onAction("note-title", async (el) => {
  const note = noteOf(el);
  if (!note) return;
  if (S.note?.id === note.id && draft !== null) return toast(t("notes.saveFirst"), undefined, "error");
  const title = await promptDialog(t("notes.retitlePrompt"), note.title);
  if (!title || title === note.title) return;
  try {
    await updated(await api.noteSetTitle(note.id, title, note.mtime));
    toast(t("notes.retitled"));
  } catch (err) {
    toast(errorText(err), (err as ApiError).code === "core.note.changed" && S.note?.id === note.id ? { label: t("common.reload"), run: () => void reload(note.id) } : undefined, "error");
  }
});
// Vincular y desvincular tocan solo el frontmatter: el borrador del cuerpo se conserva (como la v1).
onChange("note-link", async (el) => {
  const key = (el as HTMLSelectElement).value;
  if (!S.note || !key) return;
  try {
    await updated(await api.noteLink(S.note.id, key, true));
  } catch (err) {
    toast(errorText(err), undefined, "error");
  }
});
onAction("note-unlink", async (el, ev) => {
  ev.stopPropagation();
  if (!S.note) return;
  try {
    await updated(await api.noteLink(S.note.id, el.dataset.key!, false));
  } catch (err) {
    toast(errorText(err), undefined, "error");
  }
});
onAction("note-reveal", async (el) => {
  const note = noteOf(el);
  if (note) await api.noteReveal(note.id);
});
onAction("note-open-ext", async (el) => {
  if (S.note) await api.noteOpen(S.note.id, el.dataset.mode === "obsidian" ? "obsidian" : "app");
});
onAction("note-delete", async (el) => {
  const note = noteOf(el);
  if (!note) return;
  const isOpen = S.note?.id === note.id;
  const text = t("notes.confirmDelete", { title: note.title }) + (isOpen && draft !== null ? ` ${t("notes.unsavedLost")}` : "");
  if (!(await confirmDialog(text, t("notes.delete")))) return;
  try {
    const out = await api.noteDelete(note.id);
    if (isOpen) {
      S.note = null;
      draft = null;
    }
    await loadNotes();
    render();
    toast(out.trash ? t("notes.deleted") : t("notes.movedTo", { path: out.movedTo ?? "" }));
  } catch (err) {
    toast(errorText(err), undefined, "error");
  }
});
onAction("notes-folder", async () => {
  const path = inTauri ? await pickFolder() : await promptDialog(t("notes.folderPrompt"), folder?.folder ?? "");
  if (path) await setNotesFolder(path);
});
onAction("notes-folder-reset", () => setNotesFolder(null));
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

// ---------- menú contextual de la lista ----------

function closeMenu(): void {
  document.querySelector("#note-menu")?.remove();
}

function showMenu(note: Note, x: number, y: number): void {
  closeMenu();
  const id = esc(note.id);
  const item = (action: string, icon: string, text: string) =>
    `<button class="menu-item" data-action="${action}" data-id="${id}">${ico(icon)}<span>${esc(text)}</span></button>`;
  const el = document.createElement("div");
  el.id = "note-menu";
  el.className = "menu";
  el.setAttribute("role", "menu");
  el.innerHTML = `<div class="menu-head">${esc(note.title)}</div>
    ${item("note-open", "note", t("notes.open"))}${item("note-title", "edit", `${t("notes.retitle")}…`)}${item("note-reveal", "folder", t("notes.reveal"))}
    <div class="menu-sep"></div>${item("note-delete", "trash", `${t("notes.delete")}…`)}`;
  document.body.append(el);
  el.style.left = `${Math.max(8, Math.min(x, innerWidth - el.offsetWidth - 8))}px`;
  el.style.top = `${Math.max(8, Math.min(y, innerHeight - el.offsetHeight - 8))}px`;
  el.querySelector<HTMLElement>(".menu-item")?.focus();
}

document.addEventListener("contextmenu", (ev) => {
  if (S.view !== "notes" || S.detail) return;
  const row = (ev.target as HTMLElement).closest<HTMLElement>("li.note[data-id]");
  const note = row && S.notes.find((n) => n.id === row.dataset.id);
  if (!note) return;
  ev.preventDefault();
  showMenu(note, ev.clientX, ev.clientY);
});
document.addEventListener("mousedown", (ev) => {
  if (!(ev.target as HTMLElement).closest("#note-menu")) closeMenu();
});
// Una acción del menú lo cierra (después de correr: el registro de acciones va antes).
document.addEventListener("click", (ev) => {
  if ((ev.target as HTMLElement).closest("#note-menu .menu-item")) closeMenu();
});
window.addEventListener("blur", closeMenu);
window.addEventListener("resize", closeMenu);
document.addEventListener("scroll", closeMenu, true);

// ---------- teclado y escritura ----------

document.addEventListener("keydown", (ev) => {
  if (ev.key === "Escape" && document.querySelector("#note-menu")) {
    ev.stopPropagation();
    closeMenu();
    return;
  }
  if ((ev.ctrlKey || ev.metaKey) && ev.key.toLowerCase() === "s" && S.view === "notes" && !S.detail && !document.querySelector(".modal-back")) {
    ev.preventDefault();
    void save();
  }
}, true);
document.addEventListener("input", (ev) => {
  const el = ev.target as HTMLInputElement;
  if (el.id === "note-body" && S.note) {
    const was = draft !== null;
    draft = el.value;
    if (was) return;
    // Primera tecla: se marca «sin guardar» sin rearmar el editor.
    const btn = document.querySelector<HTMLButtonElement>('[data-action="note-save"]');
    if (btn) btn.disabled = false;
    const status = document.querySelector("#note-status");
    if (status) status.innerHTML = statusHtml(S.note);
    const discard = document.querySelector<HTMLElement>('[data-action="note-discard"]');
    if (discard) discard.hidden = false;
    return;
  }
  if (el.id !== "notes-search") return;
  S.noteFilter = el.value;
  render(); // `captureFocus` devuelve el foco y el cursor a la búsqueda
});
