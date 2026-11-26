// Panel de terminales (abajo, `Ctrl+J`): una pestaña por sesión abierta con su xterm. Las terminales viven
// fuera del render por innerHTML (se crean una vez y se ocultan); lo que se rearma es la barra de pestañas y
// el aviso de arriba. En la app la salida llega por un canal de Tauri; en el navegador, consultando.

import { Channel, invoke } from "@tauri-apps/api/core";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { api, inTauri } from "./api";
import { $, esc } from "./dom";
import { t } from "./i18n";
import { S } from "./state";
import type { Session } from "./types";

interface Tab {
  id: string;
  term: Terminal;
  fit: FitAddon;
  el: HTMLElement;
  /** Sube con cada conexión: lo que llega de una conexión vieja se descarta. */
  gen: number;
  /** Conectada a un proceso vivo. */
  live: boolean;
  /** Cuándo terminó el último proceso (ms): la lista de sesiones puede tardar en enterarse. */
  endedAt: number;
  writes: Promise<unknown>;
}

const tabs = new Map<string, Tab>();
const HEIGHT_KEY = "wb.termHeight";
let chrome: () => void = () => {};

/** Barra de pestañas y aviso (los arma sessions.ts, que conoce las acciones). */
export function setTermChrome(fn: () => void): void {
  chrome = fn;
}

function cssVar(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

function theme() {
  return {
    background: cssVar("--term-bg"),
    foreground: cssVar("--term-fg"),
    cursor: cssVar("--term-cursor"),
    selectionBackground: cssVar("--term-sel"),
  };
}

/** Se vuelve a leer el tema (claro/oscuro) en todas las terminales. */
export function retheme(): void {
  for (const tab of tabs.values()) tab.term.options.theme = theme();
}

function storedHeight(): number | null {
  try {
    const v = Number(localStorage.getItem(HEIGHT_KEY));
    return v > 80 ? v : null;
  } catch {
    return null;
  }
}

/** Arma el panel la primera vez (se llama al arrancar). */
export function installTerminalPanel(): void {
  const panel = $("#term")!;
  panel.innerHTML = `<div class="term-drag" title="${esc(t("term.resize"))}"></div><div class="term-head" id="term-head"></div><div id="term-banner"></div><div class="term-body" id="term-body"></div>`;
  const h = storedHeight();
  if (h) panel.style.height = `${h}px`;
  const drag = $(".term-drag", panel)!;
  drag.onpointerdown = (e) => {
    e.preventDefault();
    drag.setPointerCapture(e.pointerId);
    const startY = e.clientY;
    const startH = panel.getBoundingClientRect().height;
    const max = window.innerHeight - 120;
    drag.onpointermove = (m) => {
      panel.style.height = `${Math.max(120, Math.min(max, startH + startY - m.clientY))}px`;
    };
    drag.onpointerup = () => {
      drag.onpointermove = null;
      try {
        localStorage.setItem(HEIGHT_KEY, String(Math.round(panel.getBoundingClientRect().height)));
      } catch {
        /* sin almacenamiento: la altura vuelve a la de siempre */
      }
    };
  };
  new ResizeObserver(() => fitActive()).observe($("#term-body")!);
}

function fitActive(): void {
  const tab = S.term.active ? tabs.get(S.term.active) : undefined;
  if (!tab || !S.term.open || tab.el.hidden) return;
  try {
    tab.fit.fit();
  } catch {
    /* panel sin tamaño todavía */
  }
}

/** Tamaño para lanzar una sesión: el de la terminal visible, o una estimación del panel. */
export function termSize(): { cols: number; rows: number } {
  const tab = S.term.active ? tabs.get(S.term.active) : undefined;
  if (tab && tab.term.cols > 20) return { cols: tab.term.cols, rows: tab.term.rows };
  const body = $("#term-body");
  const w = body?.clientWidth || window.innerWidth - 260;
  const h = body?.clientHeight || window.innerHeight * 0.4;
  return { cols: Math.max(80, Math.floor(w / 8)), rows: Math.max(20, Math.floor(h / 17)) };
}

function ensureTab(id: string): Tab {
  const existing = tabs.get(id);
  if (existing) return existing;
  const el = document.createElement("div");
  el.className = "term-view";
  el.hidden = true;
  $("#term-body")!.append(el);
  const term = new Terminal({
    fontFamily: cssVar("--mono") || "monospace",
    fontSize: 13,
    scrollback: 10000,
    allowProposedApi: true,
    theme: theme(),
  });
  const fit = new FitAddon();
  term.loadAddon(fit);
  term.open(el);
  const tab: Tab = { id, term, fit, el, gen: 0, live: false, endedAt: 0, writes: Promise.resolve() };
  // En orden: un pegado largo o tipear rápido no se puede desordenar.
  term.onData((d) => {
    if (tab.live) tab.writes = tab.writes.then(() => api.sessionWrite(id, d)).catch(() => {});
  });
  // Ctrl+C con selección copia (sin selección va al proceso, como siempre). Ctrl+V no se manda como tecla:
  // el webview dispara su "paste" nativo y xterm lo escribe (con bracketed paste), sin permisos de portapapeles.
  term.attachCustomKeyEventHandler((ev) => {
    if (ev.type !== "keydown" || !ev.ctrlKey || ev.altKey || ev.metaKey || ev.shiftKey) return true;
    const key = ev.key.toLowerCase();
    if (key === "c" && term.hasSelection()) {
      copyText(term.getSelection());
      term.clearSelection();
      ev.preventDefault();
      return false;
    }
    return key !== "v";
  });
  term.onResize(({ cols, rows }) => {
    if (tab.live) api.sessionResize(id, cols, rows).catch(() => {});
  });
  tabs.set(id, tab);
  return tab;
}

/** Copia al portapapeles: la API del navegador (anda en el webview de Tauri sin permisos, dentro de una
 *  tecla) y, si falla, el `execCommand` de siempre. */
function copyText(text: string): void {
  const fallback = () => {
    const prev = document.activeElement as HTMLElement | null;
    const area = document.createElement("textarea");
    area.value = text;
    area.style.position = "fixed";
    area.style.opacity = "0";
    document.body.append(area);
    area.select();
    try {
      document.execCommand("copy");
    } catch {
      /* sin portapapeles */
    }
    area.remove();
    prev?.focus();
  };
  if (navigator.clipboard?.writeText) navigator.clipboard.writeText(text).catch(fallback);
  else fallback();
}

function ended(tab: Tab, gen: number): void {
  if (gen !== tab.gen || !tab.live) return;
  tab.live = false;
  tab.endedAt = Date.now();
  tab.term.write(`\r\n\x1b[2m[${t("term.ended")}]\x1b[0m\r\n`);
  chrome();
}

/** Conecta la pestaña al proceso de la sesión (redibuja lo guardado y sigue lo nuevo). */
export async function attach(id: string): Promise<void> {
  const tab = ensureTab(id);
  const gen = ++tab.gen;
  tab.term.reset();
  tab.live = true;
  if (inTauri) {
    const onData = new Channel<ArrayBuffer>();
    onData.onmessage = (data) => gen === tab.gen && tab.term.write(new Uint8Array(data));
    const onExit = new Channel<{ code: number | null }>();
    onExit.onmessage = () => ended(tab, gen);
    try {
      await invoke("session_attach", { id, onData, onExit });
    } catch {
      tab.live = false;
    }
  } else {
    poll(tab, gen, 0);
  }
  fitActive();
  if (tab.live) api.sessionResize(id, tab.term.cols, tab.term.rows).catch(() => {});
}

/** Servidor de desarrollo: sin canal, se pide la salida nueva cada 80 ms. */
function poll(tab: Tab, gen: number, offset: number): void {
  if (gen !== tab.gen) return;
  api
    .sessionRead(tab.id, offset)
    .then((r) => {
      if (gen !== tab.gen) return;
      if (r.data) tab.term.write(Uint8Array.from(atob(r.data), (c) => c.charCodeAt(0)));
      if (r.exited) ended(tab, gen);
      else window.setTimeout(() => poll(tab, gen, r.offset), 80);
    })
    .catch(() => ended(tab, gen));
}

export function isLive(id: string): boolean {
  return tabs.get(id)?.live ?? false;
}

/** Muestra la pestaña de una sesión (la crea si hace falta) y conecta si el proceso corre. */
export function showTab(s: Session): void {
  if (!S.term.tabs.includes(s.id)) S.term.tabs.push(s.id);
  S.term.active = s.id;
  S.term.open = true;
  const tab = ensureTab(s.id);
  syncPanel();
  tab.term.focus();
}

export function closeTab(id: string): void {
  const tab = tabs.get(id);
  if (tab) {
    tab.gen++;
    tab.term.dispose();
    tab.el.remove();
    tabs.delete(id);
  }
  S.term.tabs = S.term.tabs.filter((x) => x !== id);
  if (S.term.active === id) S.term.active = S.term.tabs[S.term.tabs.length - 1] ?? null;
  if (!S.term.tabs.length) S.term.open = false;
  syncPanel();
}

export function toggleTerminal(force?: boolean): void {
  S.term.open = force ?? !S.term.open;
  syncPanel();
  if (S.term.open && S.term.active) tabs.get(S.term.active)?.term.focus();
}

/** Visibilidad del panel y de cada terminal; conecta las pestañas cuya sesión volvió a correr. */
export function syncPanel(): void {
  const panel = $("#term");
  if (!panel) return;
  panel.hidden = !S.term.open;
  // Maximizado solo mientras el panel se ve.
  if (!S.term.open) $("#content")?.classList.remove("term-max");
  for (const [id, tab] of tabs) {
    tab.el.hidden = id !== S.term.active;
    const s = S.sessions.find((x) => x.id === id) ?? S.otherSessions.find((x) => x.id === id);
    if (s?.running && !tab.live && Date.now() - tab.endedAt > 3000) attach(id);
  }
  chrome();
  requestAnimationFrame(fitActive);
}
