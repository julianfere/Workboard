// Archivos del detalle: en un grupo también los de sus items (con la etiqueta del dueño), bloques por
// categoría del flujo que se pliegan (se recuerda por categoría), marcas «nuevo» / «modificado» respecto de las
// de ejecutado y los botones «corrido en <ambiente>» (en la lista, en «Scripts para correr» y en el visor).
// La pestaña muestra la lista y el visor lado a lado (como la v1): abre el primer archivo de las categorías
// `files.openFirst` del flujo (o el primero de texto) y el visor sobrevive a los re-render de la página.

import { api } from "../api";
import { loadDetail, onAction, onAfterRender, render } from "../app";
import { esc, fmtAgo, fmtDateTimeFull, fmtSize } from "../dom";
import { label, t } from "../i18n";
import { ico } from "../icons";
import { S, itemByName, wf } from "../state";
import type { FileCategory, FileEntry, Item } from "../types";
import { inlineFileViewer, type InlineViewer, type RunToggles } from "../viewer";

/** Los archivos que se ven en el detalle: los propios y, en un grupo, los de sus items. */
export function allFiles(it: Item): FileEntry[] {
  if (it.kind !== "group") return it.files;
  const members = it.members.map(itemByName).filter((m): m is Item => !!m);
  return [...it.files, ...members.flatMap((m) => m.files)];
}

/** En un grupo, el item dueño del archivo (`null` = del grupo). */
function ownerOf(it: Item, f: FileEntry): Item | null {
  if (it.kind !== "group") return null;
  return it.members.map(itemByName).find((m): m is Item => !!m && f.path.startsWith(`${m.name}/`)) ?? null;
}

const categories = (): FileCategory[] => [...wf().files.categories, wf().files.fallbackCategory];
const categoryOf = (f: FileEntry) => categories().find((c) => c.id === f.cat);
const isRunnable = (f: FileEntry) => !!categoryOf(f)?.runnable;
const marksOf = (f: FileEntry): Record<string, number> => S.detailData?.runs[f.path] ?? {};

/** «new» = script sin marcas; «changed» = cambió después de la última marca; «ran» = al día. */
export function runState(f: FileEntry): "" | "new" | "changed" | "ran" {
  if (!isRunnable(f)) return "";
  const marks = Object.values(marksOf(f));
  if (!marks.length) return "new";
  return f.mtime > Math.max(...marks) + 2 ? "changed" : "ran";
}

export const newCount = (it: Item) => allFiles(it).filter((f) => runState(f) === "new").length;

export function newBadge(n: number): string {
  if (!n) return "";
  return `<span class="run-badge new" title="${esc(t("detail.run.newHint"))}">${esc(n === 1 ? t("detail.run.newOne") : t("detail.run.newMany", { n }))}</span>`;
}

function stateBadge(f: FileEntry): string {
  const st = runState(f);
  if (st === "new") return `<span class="run-badge new" title="${esc(t("detail.run.newHint"))}">${esc(t("detail.run.new"))}</span>`;
  if (st === "changed") return `<span class="run-badge changed" title="${esc(t("detail.run.changedHint"))}">${esc(t("detail.run.changed"))}</span>`;
  return "";
}

/** Botones de ambiente de un script (click = marcar o desmarcar). */
function envToggles(f: FileEntry): string {
  const marks = marksOf(f);
  return wf()
    .environments.map((e) => {
      const name = label(e.labels, e.id);
      const at = marks[e.id];
      const title = at ? t("detail.run.on", { env: name, date: fmtDateTimeFull(at) }) : t("detail.run.mark", { env: name });
      return `<button class="env${at ? " on" : ""}" data-action="toggle-ran" data-path="${esc(f.path)}" data-env="${esc(e.id)}" aria-pressed="${at ? "true" : "false"}" title="${esc(title)}">${esc(name)}</button>`;
    })
    .join("");
}

function ownerChip(it: Item, f: FileEntry): string {
  if (it.kind !== "group") return "";
  const o = ownerOf(it, f);
  const text = o ? (o.ids[0] ? `#${o.ids[0]}` : o.name.split("/").pop()!) : t("detail.groupFile");
  return `<span class="chip f-owner" title="${esc(o ? o.name : t("detail.groupFileHint"))}">${esc(text)}</span>`;
}

/** Ruta del archivo dentro de su dueño (el item, o el grupo para lo propio del grupo). */
function relPath(it: Item, f: FileEntry): string {
  const base = (ownerOf(it, f)?.name ?? it.name).length + 1;
  return f.path.slice(base);
}

function fileRow(it: Item, f: FileEntry): string {
  const runnable = isRunnable(f);
  const rel = relPath(it, f);
  const dir = rel.split("/").slice(0, -1).join("/");
  return `<li class="file ${runState(f)}${f.path === shownFile(it)?.path ? " sel" : ""}"><span class="grow">${ownerChip(it, f)}<button class="link" data-action="view-file" data-path="${esc(f.path)}">${esc(f.name)}</button>
      ${dir ? `<span class="muted">${esc(dir)}</span>` : ""}${stateBadge(f)}</span>
    <span class="envs">${runnable ? envToggles(f) : ""}</span><span class="muted size">${esc(fmtSize(f.size))}</span><span class="muted">${esc(fmtAgo(f.mtime))}</span>
    <button class="icon-btn" data-action="reveal" data-path="${esc(f.path)}" title="${esc(t("detail.reveal"))}">${ico("folder")}</button></li>`;
}

// ---------- categorías plegadas (por categoría, en este navegador / ventana) ----------

const CLOSED_KEY = "wb.detail.closedCats";

function closedCats(): Set<string> {
  try {
    return new Set(JSON.parse(localStorage.getItem(CLOSED_KEY) ?? "[]") as string[]);
  } catch {
    return new Set();
  }
}

function setClosedCats(set: Set<string>): void {
  try {
    localStorage.setItem(CLOSED_KEY, JSON.stringify([...set]));
  } catch {
    // sin almacenamiento: no se recuerda
  }
}

// ---------- archivo abierto en el visor ----------

/** Archivo elegido (se mantiene al cambiar de item si existe en el otro; si no, el de siempre). */
let picked: string | null = null;
let viewer: InlineViewer | null = null;

/** Elige el archivo que muestra la pestaña Archivos (paleta, «Scripts para correr»). */
export function selectFile(path: string | null): void {
  picked = path;
}

/** El archivo del visor: el elegido o el primero de `files.openFirst`, o el primero de texto, o el primero. */
function shownFile(it: Item): FileEntry | null {
  const files = allFiles(it);
  const own = picked ? files.find((f) => f.path === picked) : undefined;
  if (own) return own;
  const newest = (list: FileEntry[]) => [...list].sort((a, b) => b.mtime - a.mtime)[0];
  for (const cat of wf().files.openFirst ?? []) {
    const f = newest(files.filter((x) => x.cat === cat));
    if (f) return f;
  }
  const order = categories().map((c) => c.id);
  const sorted = [...files].sort((a, b) => order.indexOf(a.cat) - order.indexOf(b.cat) || b.mtime - a.mtime);
  return sorted.find((f) => f.text) ?? sorted[0] ?? null;
}

function runTogglesOf(f: FileEntry): RunToggles | undefined {
  if (!isRunnable(f)) return undefined;
  const name = S.detail!;
  return {
    envs: wf().environments.map((e) => ({ id: e.id, label: label(e.labels, e.id) })),
    marks: { ...marksOf(f) },
    toggle: async (env, on) => {
      const marks = await api.markRan(f.path, env, on);
      if (S.detail === name) await loadDetail(name);
      return marks;
    },
  };
}

// Después de cada render: el visor vuelve a su lugar (o se arma, o se descarta si ya no se ve).
onAfterRender(() => {
  const slot = document.querySelector<HTMLElement>("#viewer-slot");
  const path = slot?.dataset.path ?? "";
  // Un archivo que cambió en disco (tamaño o fecha) se vuelve a leer.
  const stamp = slot?.dataset.stamp ?? "";
  if (viewer && (viewer.path !== path || viewer.el.dataset.stamp !== stamp)) {
    viewer.dispose();
    viewer = null;
  }
  if (!slot || !path || !S.detailData) return;
  if (!viewer) {
    const f = allFiles(S.detailData.item).find((x) => x.path === path);
    if (!f) return;
    viewer = inlineFileViewer(f, runTogglesOf(f));
    viewer.el.dataset.stamp = stamp;
  } else viewer.setMarks(S.detailData.runs[path] ?? {});
  if (viewer.el.parentElement !== slot) slot.append(viewer.el);
});

export function filesTab(it: Item): string {
  const files = allFiles(it);
  const closed = closedCats();
  const blocks = categories()
    .map((c) => {
      const of = files.filter((f) => f.cat === c.id).sort((a, b) => b.mtime - a.mtime);
      if (!of.length) return "";
      const open = !closed.has(c.id);
      const n = of.filter((f) => runState(f) === "new").length;
      return `<section class="card file-cat${open ? " open" : ""}"><h2><button class="link cat-toggle" data-action="files-cat" data-cat="${esc(c.id)}" aria-expanded="${open}" title="${esc(t("detail.files.toggle"))}">${ico(open ? "chevronDown" : "chevronRight")}${esc(label(c.labels, c.id))}</button>
        <span class="muted">${of.length}</span>${newBadge(n)}</h2>${open ? `<ul class="files">${of.map((f) => fileRow(it, f)).join("")}</ul>` : ""}</section>`;
    })
    .join("");
  if (!blocks) return `<p class="empty">${esc(t("detail.noFiles"))}</p>`;
  const f = shownFile(it);
  return `<div class="files-split"><div class="file-list">${blocks}</div>
    <div class="viewer-slot" id="viewer-slot" data-path="${esc(f?.path ?? "")}" data-stamp="${esc(f ? `${f.size}:${f.mtime}` : "")}"></div></div>`;
}

/** «Scripts para correr» del Resumen: los archivos de categorías que se corren, con sus marcas. */
export function scriptsCard(it: Item): string {
  const order = categories().map((c) => c.id);
  const files = allFiles(it)
    .filter(isRunnable)
    .sort((a, b) => order.indexOf(a.cat) - order.indexOf(b.cat) || b.mtime - a.mtime);
  if (!files.length) return "";
  const rows = files
    .map(
      (f) => `<li class="file ${runState(f)}"><span class="grow">${ownerChip(it, f)}<span class="chip kind">${esc(f.kind ?? label(categoryOf(f)!.labels, f.cat))}</span>
        <button class="link" data-action="view-file" data-path="${esc(f.path)}">${esc(f.name)}</button>${runState(f) === "changed" ? stateBadge(f) : ""}</span>
        <span class="envs">${envToggles(f)}</span></li>`,
    )
    .join("");
  return `<section class="card"><h2><span class="grow">${esc(t("detail.scripts"))}</span><span class="muted sm">${esc(t("detail.scripts.hint"))}</span></h2><ul class="files">${rows}</ul></section>`;
}

// ---------- acciones ----------

onAction("files-cat", (el) => {
  const set = closedCats();
  const cat = el.dataset.cat!;
  if (set.has(cat)) set.delete(cat);
  else set.add(cat);
  setClosedCats(set);
  render();
});

onAction("toggle-ran", async (el) => {
  const path = el.dataset.path!;
  const on = !el.classList.contains("on");
  await api.markRan(path, el.dataset.env!, on);
  await loadDetail(S.detail!);
});

// Ver un archivo: lo muestra en la pestaña Archivos (desde «Scripts para correr» también).
onAction("view-file", (el) => {
  picked = el.dataset.path ?? null;
  S.detailTab = "files";
  render();
});
