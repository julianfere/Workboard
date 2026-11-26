// Controlador: carga de datos, navegación, refresco en vivo y acciones. Cada vista exporta `render()` y
// registra sus acciones (`data-action="nombre"` en el HTML) con `onAction`.

import { api, inTauri } from "./api";
import { $, esc, toast } from "./dom";
import { errorText, setLang, t } from "./i18n";
import { ico } from "./icons";
import { setPref } from "./prefs";
import { folderName, S, type ViewId } from "./state";
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

/** La vista del detalle y lo que pregunta antes de cerrarlo (cambios sin guardar). */
export function registerDetail(fn: () => string, leave: () => Promise<boolean>): void {
  detailView = fn;
  detailLeave = leave;
}
let detailLeave: () => Promise<boolean> = async () => true;

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
  // La barra de título nativa sigue el tema elegido (`null` = la del sistema).
  if (inTauri) void nativeWindow().then((w) => w.setTheme(theme === "system" ? null : theme)).catch(() => {});
}

/** La ventana de Tauri (import dinámico: en el navegador no hace falta). */
async function nativeWindow() {
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  return getCurrentWindow();
}

let lastTitle = "";
/** Título de la pestaña y de la ventana nativa (Tauri no copia `document.title` a la ventana). */
function syncTitle(): void {
  const title = S.root ? `${folderName(S.root)} · ${t("app.name")}` : t("app.name");
  if (title === lastTitle) return;
  lastTitle = title;
  document.title = title;
  if (inTauri) void nativeWindow().then((w) => w.setTitle(title)).catch(() => {});
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
    if (S.detail && !S.items.some((i) => i.name === S.detail)) {
      // La carpeta abierta desapareció (la movieron, renombraron o borraron por fuera).
      S.detail = null;
      S.detailData = null;
      toast(t("detail.gone"), undefined, "error");
    } else if (S.detail) await loadDetail(S.detail, false);
  } catch (err) {
    S.loadError = errorText(err);
  }
  render();
}

const fsHooks: (() => Promise<unknown>)[] = [];

/** Lo que se relee además de los items cuando cambia algo en disco (bandeja, notas, sesiones). */
export function onFsRefresh(fn: () => Promise<unknown>): void {
  fsHooks.push(fn);
}

/** Refresco completo (cambio en disco o `F5`): items y detalle, más bandeja, notas y sesiones, como la v1.
 *  Las sesiones terminadas se cierran solas al leer la lista (`sessions`, aviso con Deshacer). */
export async function refreshAll(): Promise<void> {
  if (!S.settings?.root) return;
  await Promise.all(fsHooks.map((fn) => fn().catch(() => {})));
  await refresh();
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
      if (sig !== S.signature) await refreshAll();
    } catch {
      /* la carpeta pudo desaparecer: lo informa el próximo refresh */
    }
  }, 2000);
}

// ---------- navegación ----------

export function goView(view: ViewId): void {
  if (S.detail) {
    void detailLeave().then((ok) => ok && showView(view));
    return;
  }
  showView(view);
}

function showView(view: ViewId): void {
  // En modo discreto (clase `discreet` en `<html>`, ver `discreet.ts`) la vista Horas no está.
  S.view = view === "hours" && document.documentElement.classList.contains("discreet") ? "today" : view;
  // Se abre en la última vista usada (pref `view`, como la v1).
  setPref("view", S.view);
  S.detail = null;
  S.detailData = null;
  render();
  $("#main")?.scrollTo(0, 0);
}

/** Abre el detalle de un item en el panel lateral, sobre la vista actual. Al cambiar de item se queda en la
 *  pestaña Archivos si estaba ahí (como la v1); si no, vuelve al Resumen. */
export async function openDetail(name: string): Promise<void> {
  if (S.detail && S.detail !== name && !(await detailLeave())) return;
  if (S.detail !== name) {
    if (S.detailTab !== "files") S.detailTab = "summary";
    S.fieldErrors = {};
    S.fieldDrafts = {};
  }
  await loadPanelWidth();
  await loadDetail(name);
}

/** Cierra el detalle sin preguntar. */
export function closeDetail(): void {
  S.detail = null;
  S.detailData = null;
  render();
}

/** Cierra el detalle si no hay cambios sin guardar (o si se aceptan descartarlos). */
export async function requestCloseDetail(): Promise<void> {
  if (S.detail && (await detailLeave())) closeDetail();
}

// ---------- panel del detalle: ancho (arrastrando el borde), maximizado y guardado en `ui_prefs` ----------

const PANEL_PREF = "detail";
const PANEL_MIN = 560;
const PANEL_NARROW = 900;
const panel = { width: 1100, max: false, loaded: false };

async function loadPanelWidth(): Promise<void> {
  if (panel.loaded) return;
  panel.loaded = true;
  try {
    const p = (await api.uiPrefs())[PANEL_PREF] as { width?: number; max?: boolean } | undefined;
    if (typeof p?.width === "number" && p.width >= PANEL_MIN) panel.width = Math.round(p.width);
    panel.max = p?.max === true;
  } catch {
    /* sin preferencias: el ancho por defecto */
  }
}

function savePanelWidth(): void {
  void api.setUiPref(PANEL_PREF, { width: panel.width, max: panel.max }).catch(() => {});
}

/** Ancho real: el guardado, sin pasar el espacio disponible (deja ver un poco de la vista de atrás). */
function applyPanelWidth(el: HTMLElement): void {
  const room = (el.parentElement?.clientWidth ?? innerWidth) - 40;
  const w = Math.max(Math.min(PANEL_MIN, room), Math.min(panel.width, room));
  el.classList.toggle("max", panel.max);
  el.style.width = panel.max ? "" : `${w}px`;
  el.classList.toggle("narrow", !panel.max && w < PANEL_NARROW);
}

export function togglePanelMax(): void {
  panel.max = !panel.max;
  savePanelWidth();
  const el = $("#dpanel");
  if (el) applyPanelWidth(el);
}

let drag: { x: number; w: number } | null = null;
document.addEventListener("mousedown", (ev) => {
  const el = $("#dpanel");
  if (!el || !(ev.target as HTMLElement).closest(".dp-resizer")) return;
  ev.preventDefault();
  if (panel.max) {
    panel.max = false;
    panel.width = Math.round(el.getBoundingClientRect().width);
  }
  drag = { x: ev.clientX, w: el.getBoundingClientRect().width };
  document.body.classList.add("resizing");
});
document.addEventListener("mousemove", (ev) => {
  const el = $("#dpanel");
  if (!drag || !el) return;
  const room = (el.parentElement?.clientWidth ?? innerWidth) - 40;
  panel.width = Math.max(PANEL_MIN, Math.min(room, Math.round(drag.w - (ev.clientX - drag.x))));
  applyPanelWidth(el);
});
document.addEventListener("mouseup", () => {
  if (!drag) return;
  drag = null;
  document.body.classList.remove("resizing");
  savePanelWidth();
});
window.addEventListener("resize", () => {
  const el = $("#dpanel");
  if (el && S.detail) applyPanelWidth(el);
});

/** Arma el panel del detalle (o lo esconde). Mantiene el scroll de sus columnas si sigue el mismo item. */
function renderPanel(): void {
  const el = $("#dpanel");
  const scrim = $("#dscrim");
  if (!el || !scrim) return;
  const open = !!(S.detail && S.detailData && detailView);
  scrim.hidden = !open;
  el.classList.toggle("open", open);
  el.setAttribute("aria-hidden", String(!open));
  if (!open) {
    el.innerHTML = "";
    delete el.dataset.item;
    return;
  }
  const same = el.dataset.item === S.detail && el.dataset.tab === S.detailTab;
  const scroll = same ? [...el.querySelectorAll<HTMLElement>("[data-keep-scroll]")].map((x) => [x.dataset.keepScroll!, x.scrollTop] as const) : [];
  el.innerHTML = detailView!();
  el.dataset.item = S.detail!;
  el.dataset.tab = S.detailTab;
  for (const [k, top] of scroll) {
    const box = el.querySelector<HTMLElement>(`[data-keep-scroll="${k}"]`);
    if (box) box.scrollTop = top;
  }
  applyPanelWidth(el);
}

// ---------- render ----------

function renderNav(): string {
  const projectName = S.root ? folderName(S.root) : t("project.none");
  const item = (id: ViewId, icon: string, key: string) => {
    const badge = navCounters[id] ? `<span class="nav-count${id === "tasks" ? " neutral" : ""}" id="${id}-count"></span>` : "";
    return `<button class="nav-item${S.view === id ? " sel" : ""}${id === "hours" ? " hrs" : ""}" data-action="nav" data-view="${id}">${ico(icon)}<span>${esc(t(`nav.${id}`))}</span>${badge}<span class="nav-key">${key}</span></button>`;
  };
  return `<button class="brand" data-action="project-menu" title="${esc(S.root)}" aria-haspopup="menu"><span class="brand-icon">${ico("folder")}</span><span class="brand-name">${esc(projectName)}</span>${ico("chevronDown", "chev")}</button>
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
  } else {
    const view = VIEWS[S.view];
    main.innerHTML = view
      ? view()
      : `<div class="page"><h1>${esc(t(`nav.${S.view}`))}</h1><p class="empty">${esc(t("view.placeholder", { view: t(`nav.${S.view}`) }))}</p></div>`;
  }
  renderPanel();
  for (const [id, fn] of Object.entries(navCounters)) {
    const badge = $(`#${id}-count`);
    const count = S.items.length ? fn() : 0;
    if (badge) badge.textContent = count ? String(count) : "";
  }
  syncTitle();
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

/** Corre una acción registrada sin un elemento en la página (paleta de comandos). `data` = su `dataset`. */
export function runAction(name: string, data: Record<string, string> = {}): void {
  const fn = ACTIONS[name];
  if (!fn) return;
  const el = document.createElement("button");
  Object.assign(el.dataset, data);
  void run(fn, el, { target: el, preventDefault() {}, stopPropagation() {} } as unknown as Event);
}

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
  onAction("detail-close", () => requestCloseDetail());
  onAction("detail-max", () => togglePanelMax());
  // Click en el fondo del panel: cierra el detalle.
  document.addEventListener("click", (ev) => {
    if ((ev.target as HTMLElement).id === "dscrim") void requestCloseDetail();
  });
}
