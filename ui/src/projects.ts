// Carpetas de trabajos: el menú del nombre del proyecto (recientes, renombrar, abrir otra), los nombres para
// mostrar (alias, solo en la app) y «Quitar de recientes» con Deshacer. La lista también la usa Ajustes.

import { api, inTauri, pickFolder } from "./api";
import { onAction, onAfterRender, reloadSettings, render } from "./app";
import { $, esc, promptDialog, toast } from "./dom";
import { errorText, t } from "./i18n";
import { ico } from "./icons";
import { folderName, S } from "./state";
import type { RecentFolder } from "./types";
import { openRoot } from "./views/settings";

/** Recientes con su estado en disco (se piden al abrir el menú o Ajustes). */
let recents: RecentFolder[] | null = null;
let loading = false;

/** Relee los recientes y vuelve a dibujar. */
export async function loadRecents(): Promise<void> {
  try {
    recents = await api.recentFolders();
  } catch {
    recents = [];
  }
  render();
}

/** Filas de los recientes (`menu` = dentro del menú del proyecto). */
export function recentRows(menu: boolean): string {
  if (!recents) {
    if (!loading) {
      loading = true;
      void loadRecents().finally(() => (loading = false));
    }
    return "";
  }
  return recents
    .map((r) => {
      const missing = r.exists ? "" : ` <span class="flag">${esc(t("projects.missing"))}</span>`;
      const cls = `${menu ? "menu-item proj-row" : "link-row proj-row"}${r.current ? " cur" : ""}${r.exists ? "" : " missing"}`;
      const tag = menu ? "div" : "li";
      return `<${tag} class="${cls}" data-action="proj-open" data-path="${esc(r.path)}" title="${esc(r.path)}" role="${menu ? "menuitem" : "button"}">${ico("folder")}
        <span class="proj-main"><span class="proj-name">${esc(r.name)}${missing}</span><span class="proj-path muted">${esc(r.path)}</span></span>
        <button class="icon-btn" data-action="proj-rename" data-path="${esc(r.path)}" title="${esc(t("projects.renameHint"))}" aria-label="${esc(t("projects.rename"))}">${ico("edit")}</button>
        <button class="icon-btn" data-action="proj-forget" data-path="${esc(r.path)}" title="${esc(t("projects.forget"))}" aria-label="${esc(t("projects.forget"))}">${ico("x")}</button></${tag}>`;
    })
    .join("");
}

// ---------- menú del nombre del proyecto ----------

function menuHtml(): string {
  const rows = recentRows(true);
  const name = S.root ? folderName(S.root) : "";
  return `<div class="menu-head">${esc(t("projects.recents"))}</div>
    ${rows || `<div class="menu-item muted">${esc(recents ? t("projects.noRecents") : t("common.loading"))}</div>`}
    <div class="menu-sep"></div>
    ${S.root ? `<button class="menu-item" data-action="proj-rename" data-path="${esc(S.root)}" role="menuitem">${ico("edit")}<span class="proj-main"><span>${esc(t("projects.renameCurrent", { name }))}</span><span class="proj-path muted">${esc(t("projects.renameHint"))}</span></span></button>` : ""}
    <button class="menu-item" data-action="proj-pick" role="menuitem">${ico("folder")}<span>${esc(t("projects.openFolder"))}</span></button>`;
}

function menuEl(): HTMLElement | null {
  return $("#proj-pop");
}

/** Dibuja el menú si está abierto (después de cada render: la lista llega sola). */
function showMenu(): void {
  const el = menuEl();
  if (!el || el.hidden) return;
  el.innerHTML = menuHtml();
  const brand = $(".brand");
  if (brand) {
    const r = brand.getBoundingClientRect();
    el.style.left = `${Math.max(8, r.left)}px`;
    el.style.top = `${Math.min(r.bottom + 4, innerHeight - el.offsetHeight - 8)}px`;
  }
}

export function openProjectMenu(): void {
  let el = menuEl();
  if (!el) {
    el = document.createElement("div");
    el.id = "proj-pop";
    el.className = "menu proj-menu";
    el.setAttribute("role", "menu");
    el.setAttribute("aria-label", t("projects.menu"));
    document.body.append(el);
  }
  el.hidden = false;
  recents = null;
  showMenu();
}

export function closeProjectMenu(): void {
  const el = menuEl();
  if (el) el.hidden = true;
}

const menuOpen = () => !!menuEl() && !menuEl()!.hidden;

document.addEventListener("mousedown", (ev) => {
  const target = ev.target as HTMLElement;
  if (menuOpen() && !target.closest("#proj-pop") && !target.closest('[data-action="project-menu"]')) closeProjectMenu();
});
document.addEventListener(
  "keydown",
  (ev) => {
    if (ev.key === "Escape" && menuOpen()) {
      ev.stopPropagation();
      closeProjectMenu();
    }
  },
  true,
);

// ---------- acciones ----------

/** Abrir otra carpeta: el diálogo del sistema en la app; en el navegador, el campo de ruta de Ajustes. */
export async function pickProject(): Promise<void> {
  if (inTauri) {
    const p = await pickFolder();
    if (p) await openRoot(p);
    return;
  }
  S.view = "settings";
  S.detail = null;
  render();
  $<HTMLInputElement>("#root-path")?.focus();
}

async function renameFolder(path: string): Promise<void> {
  const name = await promptDialog(t("projects.renamePrompt"), folderName(path), true);
  if (name === null) return;
  try {
    await api.setAlias(path, name);
    await reloadSettings();
    await loadRecents();
    toast(name.trim() ? t("projects.renamed", { name: folderName(path) }) : t("projects.renameCleared"));
  } catch (err) {
    toast(errorText(err), undefined, "error");
  }
}

async function forget(path: string): Promise<void> {
  const pos = S.settings?.recents.findIndex((r) => r.toLowerCase() === path.toLowerCase()) ?? -1;
  const name = folderName(path);
  S.settings = await api.forgetRecent(path);
  await loadRecents();
  toast(t("projects.forgotten", { name }), {
    label: t("common.undo"),
    run: async () => {
      S.settings = await api.restoreRecent(path, Math.max(pos, 0));
      await loadRecents();
    },
  });
}

onAction("project-menu", () => (menuOpen() ? closeProjectMenu() : openProjectMenu()));
onAction("proj-open", (el) => {
  closeProjectMenu();
  return openRoot(el.dataset.path!).then(() => loadRecents());
});
onAction("proj-rename", (el, ev) => {
  ev.stopPropagation();
  closeProjectMenu();
  return renameFolder(el.dataset.path!);
});
onAction("proj-forget", (el, ev) => {
  ev.stopPropagation();
  return forget(el.dataset.path!);
});
onAction("proj-pick", () => {
  closeProjectMenu();
  return pickProject();
});

onAfterRender(showMenu);
