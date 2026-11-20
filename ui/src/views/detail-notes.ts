// Pestaña Notas del detalle: las notas vinculadas al item (por su id o su carpeta), crear una ya vinculada,
// vincular una existente y desvincular. Abrir una nota lleva a la vista Notas.

import { api } from "../api";
import { goView, onAction, render } from "../app";
import { esc, fmtAgo, promptDialog } from "../dom";
import { t } from "../i18n";
import { ico } from "../icons";
import { S, titleOf } from "../state";
import type { Item, Note } from "../types";
import { loadNotes } from "./notes";

let loadedFor: string | null = null;

/** Carga las notas una vez por carpeta abierta (o a pedido, al entrar a la pestaña). */
export function ensureNotes(force = false): void {
  if (!force && loadedFor === S.root) return;
  loadedFor = S.root;
  void loadNotes().then(render);
}

/** Cómo se nombra un item dentro de las notas: su id, o la carpeta si no tiene (como el backend). */
const itemKey = (it: Item) => it.ids[0] ?? it.name;
const ownKeys = (it: Item) => new Set([it.name, ...it.ids]);

export const notesOf = (it: Item): Note[] => {
  const own = ownKeys(it);
  return S.notes.filter((n) => n.items.some((k) => own.has(k))).sort((a, b) => b.mtime - a.mtime);
};

function chip(key: string): string {
  const it = S.items.find((i) => i.name === key || i.ids.includes(key));
  return it
    ? `<button class="chip link" data-action="open-item" data-name="${esc(it.name)}" title="${esc(titleOf(it))}">${esc(key)}</button>`
    : `<span class="chip">${esc(key)}</span>`;
}

export function notesTab(it: Item): string {
  const own = ownKeys(it);
  const cards = notesOf(it)
    .map((n) => {
      const others = n.items.filter((k) => !own.has(k));
      const id = esc(n.id);
      return `<section class="card wn-card"><div class="wn-top"><button class="link wn-title" data-action="detail-note-open" data-id="${id}">${esc(n.title)}</button>
          <span class="muted">${n.author ? `${esc(n.author)} · ` : ""}${esc(fmtAgo(n.mtime))}</span></div>
        ${n.excerpt ? `<p class="excerpt">${esc(n.excerpt)}</p>` : ""}
        <div class="wn-foot">${others.length ? `<span class="muted">${esc(t("detail.notes.alsoIn"))}</span>${others.map(chip).join("")}` : ""}
          <span class="spacer"></span><button class="link" data-action="detail-note-open" data-id="${id}">${esc(t("detail.notes.open"))}</button>
          <button class="link" data-action="detail-note-unlink" data-id="${id}">${esc(t("detail.notes.unlink"))}</button></div></section>`;
    })
    .join("");
  return `<div class="wn-actions"><button class="btn primary" data-action="detail-note-new">${ico("plus")}${esc(t("detail.notes.new"))}</button>
      <button class="btn" data-action="detail-note-link">${ico("note")}${esc(t("detail.notes.link"))}</button></div>
    ${cards || `<p class="empty">${esc(t("detail.notes.none"))}</p>`}`;
}

/** Modal con las notas que todavía no están vinculadas al item. */
function pickNote(it: Item): Promise<string | null> {
  const own = ownKeys(it);
  const cand = S.notes.filter((n) => !n.items.some((k) => own.has(k))).sort((a, b) => b.mtime - a.mtime);
  return new Promise((resolve) => {
    const back = document.createElement("div");
    back.className = "modal-back";
    back.innerHTML = `<div class="modal" role="dialog" aria-modal="true"><h2>${esc(t("detail.notes.pickTitle", { item: titleOf(it) }))}</h2>
      <input type="search" class="pick-q" placeholder="${esc(t("notes.search"))}">
      <ul class="list pick-list">${cand.map((n) => `<li class="link-row" data-pick="${esc(n.id)}" data-q="${esc(`${n.title} ${n.excerpt}`.toLowerCase())}" tabindex="0"><span class="grow">${esc(n.title)}</span><span class="muted">${esc(fmtAgo(n.mtime))}</span></li>`).join("") || `<li class="empty">${esc(t("detail.notes.noOthers"))}</li>`}</ul>
      <div class="modal-actions"><button class="btn" data-cancel>${esc(t("common.cancel"))}</button></div></div>`;
    document.body.append(back);
    const done = (id: string | null) => {
      back.remove();
      resolve(id);
    };
    back.addEventListener("click", (e) => {
      if (e.target === back) return done(null);
      const el = e.target as HTMLElement;
      if (el.closest("[data-cancel]")) return done(null);
      const row = el.closest<HTMLElement>("[data-pick]");
      if (row) done(row.dataset.pick!);
    });
    back.addEventListener("keydown", (e) => {
      if (e.key === "Escape") done(null);
      const row = (e.target as HTMLElement).closest<HTMLElement>("[data-pick]");
      if (e.key === "Enter" && row) done(row.dataset.pick!);
    });
    const q = back.querySelector<HTMLInputElement>(".pick-q")!;
    q.addEventListener("input", () => {
      const v = q.value.toLowerCase();
      back.querySelectorAll<HTMLElement>("[data-pick]").forEach((r) => (r.hidden = !r.dataset.q!.includes(v)));
    });
    q.focus();
  });
}

async function openNote(id: string): Promise<void> {
  S.note = await api.noteRead(id);
  goView("notes");
}

const current = () => S.detailData!.item;

onAction("detail-note-open", (el) => openNote(el.dataset.id!));
onAction("detail-note-new", async () => {
  const it = current();
  const title = await promptDialog(t("notes.title"));
  if (!title) return;
  const note = await api.noteCreate(title, [itemKey(it)]);
  await loadNotes();
  await openNote(note.id);
});
onAction("detail-note-link", async () => {
  const it = current();
  await loadNotes();
  const id = await pickNote(it);
  if (!id) return;
  await api.noteLink(id, itemKey(it), true);
  await loadNotes();
  render();
});
onAction("detail-note-unlink", async (el) => {
  const it = current();
  const note = S.notes.find((n) => n.id === el.dataset.id);
  if (!note) return;
  const own = ownKeys(it);
  for (const key of note.items.filter((k) => own.has(k))) await api.noteLink(note.id, key, false);
  await loadNotes();
  render();
});
