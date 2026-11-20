// Controlador: carga de datos, navegación, refresco en vivo y acciones. Cada vista exporta `render()` y
// registra sus acciones (`data-action="nombre"` en el HTML) con `onAction`.

import { api } from "./api";
import { $, esc, toast } from "./dom";
import { errorText, setLang, t } from "./i18n";
import { ico } from "./icons";
import { S, type ViewId } from "./state";
import type { Theme } from "./types";

type Handler = (el: HTMLElement, ev: Event) => void | Promise<void>;
const ACTIONS: Record<string, Handler> = {};
const CHANGES: Record<string, Handler> = {};
const VIEWS: Partial<Record<ViewId, () => string>> = {};
let detailView: (() => string) | null = null;
let projectView: (() => string) | null = null;

export function onAction(name: string, fn: Handler): void {
  ACTIONS[name] = fn;
}

/** Cambio de un input/select (`data-change="nombre"`). */
export function onChange(name: string, fn: Handler): void {
  CHANGES[name] = fn;
}

export function registerView(id: ViewId, fn: () => string): void {
  VIEWS[id] = fn;
}

export function registerDetail(fn: () => string): void {
  detailView = fn;
}

export function registerProject(fn: () => string): void {
  projectView = fn;
}

let navExtra: () => string = () => `<div class="nav-spacer"></div>`;
const afterRenders: (() => void)[] = [];

/** Lo que va entre las secciones y el pie del menú (la lista de sesiones). */
export function registerNavExtra(fn: () => string): void {
  navExtra = fn;
}

/** Corre después de cada render (partes de la ventana que no se rearman con innerHTML, como las terminales). */
export function onAfterRender(fn: () => void): void {
  afterRenders.push(fn);
}

export const NAV: { id: ViewId; icon: string }[] = [
  { id: "today", icon: "sun" },
  { id: "items", icon: "list" },
  { id: "tasks", icon: "tasks" },
  { id: "hours", icon: "clock" },
  { id: "alerts", icon: "alert" },
  { id: "notes", icon: "note" },
  { id: "metrics", icon: "layers" },
  { id: "tools", icon: "wrench" },
];

// ---------- tema ----------

const darkQuery = window.matchMedia("(prefers-color-scheme: dark)");

export function applyTheme(theme: Theme): void {
  const dark = theme === "dark" || (theme === "system" && darkQuery.matches);
  document.documentElement.dataset.theme = dark ? "dark" : "light";
}
darkQuery.addEventListener("change", () => S.settings && applyTheme(S.settings.theme));

// ---------- datos ----------

/** Relee la carpeta de trabajos (y el detalle abierto). */
export async function refresh(): Promise<void> {
  if (!S.settings?.root) return;
  try {
    const list = await api.listItems();
    S.items = list.items;
    S.root = list.root;
    S.deletedCount = list.deleted;
    S.signature = list.signature.join(":");
    S.loadError = "";
    if (S.detail) await loadDetail(S.detail, false);
  } catch (err) {
    S.loadError = errorText(err);
  }
  render();
}

export async function loadDetail(name: string, rerender = true): Promise<void> {
  try {
    S.detailData = await api.getItem(name);
    S.detail = name;
  } catch (err) {
    S.detail = null;
    S.detailData = null;
    toast(errorText(err), undefined, "error");
  }
  if (rerender) render();
}

export async function reloadSettings(): Promise<void> {
  S.settings = await api.getSettings();
  S.workflow = await api.workflow();
  setLang(S.settings.lang);
  applyTheme(S.settings.theme);
}

/** Refresco en vivo: cada 2 s compara la firma del árbol (lo mismo que el watcher de la v1). */
export function startPolling(): void {
  window.setInterval(async () => {
    if (document.hidden || !S.settings?.root || document.querySelector(".modal-back")) return;
    try {
      const sig = (await api.signature()).join(":");
      if (sig !== S.signature) await refresh();
    } catch {
      /* la carpeta pudo desaparecer: lo informa el próximo refresh */
    }
  }, 2000);
}

// ---------- navegación ----------

export function goView(view: ViewId): void {
  // En modo discreto (clase `discreet` en `<html>`, ver `discreet.ts`) la vista Horas no está.
  S.view = view === "hours" && document.documentElement.classList.contains("discreet") ? "today" : view;
  S.detail = null;
  S.detailData = null;
  render();
  $("#main")?.scrollTo(0, 0);
}

export async function openDetail(name: string): Promise<void> {
  S.detailTab = "summary";
  S.fieldErrors = {};
  S.fieldDrafts = {};
  await loadDetail(name);
  $("#main")?.scrollTo(0, 0);
}

export function closeDetail(): void {
  S.detail = null;
  S.detailData = null;
  render();
}

// ---------- render ----------

function renderNav(): string {
  const projectName = S.root ? S.root.split(/[\\/]/).pop() : t("project.none");
  const item = (id: ViewId, icon: string, key: string) => {
    const badge = navCounters[id] ? `<span class="nav-count${id === "tasks" ? " neutral" : ""}" id="${id}-count"></span>` : "";
    return `<button class="nav-item${S.view === id && !S.detail ? " sel" : ""}${id === "hours" ? " hrs" : ""}" data-action="nav" data-view="${id}">${ico(icon)}<span>${esc(t(`nav.${id}`))}</span>${badge}<span class="nav-key">${key}</span></button>`;
  };
  return `<button class="brand" data-action="project-menu" title="${esc(S.root)}"><span class="brand-icon">${ico("folder")}</span><span class="brand-name">${esc(projectName)}</span>${ico("chevronDown", "chev")}</button>
    ${NAV.map((v, i) => item(v.id, v.icon, String(i + 1))).join("")}
    ${navExtra()}
    <button class="nav-item" data-action="palette">${ico("search")}<span>${esc(t("palette.open"))}</span><span class="nav-key">Ctrl K</span></button>
    ${item("settings", "settings", "")}`;
}

export function render(): void {
  const nav = $("#sidenav")!;
  const main = $("#main")!;
  nav.innerHTML = renderNav();
  if (!S.settings?.root && S.view !== "settings") {
    main.innerHTML = projectView ? projectView() : "";
  } else if (S.detail && S.detailData && detailView) {
    main.innerHTML = detailView();
  } else {
    const view = VIEWS[S.view];
    main.innerHTML = view
      ? view()
      : `<div class="page"><h1>${esc(t(`nav.${S.view}`))}</h1><p class="empty">${esc(t("view.placeholder", { view: t(`nav.${S.view}`) }))}</p></div>`;
  }
  for (const [id, fn] of Object.entries(navCounters)) {
    const badge = $(`#${id}-count`);
    const count = S.items.length ? fn() : 0;
    if (badge) badge.textContent = count ? String(count) : "";
  }
  document.title = S.root ? `${S.root.split(/[\\/]/).pop()} · ${t("app.name")}` : t("app.name");
  for (const fn of afterRenders) fn();
}

/** Contadores del menú (alertas, «necesita tu atención» en Hoy, tareas abiertas en Pendientes). */
const navCounters: Partial<Record<ViewId, () => number>> = {};
export function setNavCounter(id: ViewId, fn: () => number): void {
  navCounters[id] = fn;
}
export function setAlertCounter(fn: () => number): void {
  setNavCounter("alerts", fn);
}

// ---------- eventos ----------

async function run(fn: Handler, el: HTMLElement, ev: Event): Promise<void> {
  try {
    await fn(el, ev);
  } catch (err) {
    toast(errorText(err), undefined, "error");
  }
}

export function installEvents(): void {
  document.addEventListener("click", (ev) => {
    const el = (ev.target as HTMLElement).closest<HTMLElement>("[data-action]");
    if (el && ACTIONS[el.dataset.action!]) {
      ev.preventDefault();
      run(ACTIONS[el.dataset.action!], el, ev);
    }
  });
  document.addEventListener("change", (ev) => {
    const el = (ev.target as HTMLElement).closest<HTMLElement>("[data-change]");
    if (el && CHANGES[el.dataset.change!]) run(CHANGES[el.dataset.change!], el, ev);
  });
  document.addEventListener("keydown", (ev) => {
    const el = ev.target as HTMLElement;
    if (ev.key === "Enter" && el.dataset.enter && ACTIONS[el.dataset.enter]) {
      ev.preventDefault();
      run(ACTIONS[el.dataset.enter], el, ev);
    }
  });
  onAction("nav", (el) => goView(el.dataset.view as ViewId));
}
