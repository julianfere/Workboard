// Helpers de DOM y formato, sin dependencias.

import { lang, t } from "./i18n";

export const $ = <T extends HTMLElement = HTMLElement>(sel: string, root: ParentNode = document) =>
  root.querySelector(sel) as T | null;

export const esc = (s: unknown) =>
  String(s ?? "").replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);

/** `a.b` → `a.b` atributo seguro. */
export const attr = esc;

export function fmtSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

/** Fecha relativa corta ("hace 3 h", "2 days ago"…). `ts` en segundos. */
export function fmtAgo(ts: number): string {
  if (!ts) return "";
  const rtf = new Intl.RelativeTimeFormat(lang(), { numeric: "auto" });
  const diff = ts - Date.now() / 1000;
  const abs = Math.abs(diff);
  if (abs < 3600) return rtf.format(Math.round(diff / 60), "minute");
  if (abs < 86400) return rtf.format(Math.round(diff / 3600), "hour");
  if (abs < 86400 * 30) return rtf.format(Math.round(diff / 86400), "day");
  return fmtDate(ts);
}

export function fmtDate(ts: number): string {
  return new Date(ts * 1000).toLocaleDateString(lang(), { year: "numeric", month: "short", day: "numeric" });
}

/** `AAAA-MM-DD` local de hoy. */
export function todayIso(): string {
  const d = new Date();
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

/** Días entre hoy y una fecha `AAAA-MM-DD` (negativo = pasada). */
export function daysUntil(iso: string): number {
  const [y, m, d] = iso.split("-").map(Number);
  const target = new Date(y, m - 1, d).getTime();
  const now = new Date();
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  return Math.round((target - today) / 86400000);
}

let toastTimer = 0;

/** Aviso abajo a la derecha. `action` = botón opcional (Deshacer…). */
export function toast(text: string, action?: { label: string; run: () => void }, kind: "info" | "error" = "info"): void {
  let el = $("#toast");
  if (!el) {
    el = document.createElement("div");
    el.id = "toast";
    el.setAttribute("role", "status");
    document.body.append(el);
  }
  el.className = `toast ${kind}`;
  el.innerHTML = `<span>${esc(text)}</span>${action ? `<button class="link" data-toast-action>${esc(action.label)}</button>` : ""}`;
  el.hidden = false;
  const btn = $("[data-toast-action]", el);
  if (btn && action) btn.onclick = () => { el!.hidden = true; action.run(); };
  clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => (el!.hidden = true), action ? 12000 : 4000);
}

/** Diálogo de confirmación propio (`Enter` acepta, `Esc` cancela). */
export function confirmDialog(text: string, okLabel = t("common.ok")): Promise<boolean> {
  return new Promise((resolve) => {
    const back = document.createElement("div");
    back.className = "modal-back";
    back.innerHTML = `<div class="modal small" role="dialog" aria-modal="true"><p>${esc(text)}</p>
      <div class="modal-actions"><button class="btn" data-no>${esc(t("common.cancel"))}</button>
      <button class="btn primary" data-yes>${esc(okLabel)}</button></div></div>`;
    document.body.append(back);
    const done = (v: boolean) => { back.remove(); document.removeEventListener("keydown", onKey, true); resolve(v); };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") { e.stopPropagation(); done(false); }
      if (e.key === "Enter") { e.stopPropagation(); done(true); }
    };
    document.addEventListener("keydown", onKey, true);
    $("[data-yes]", back)!.onclick = () => done(true);
    $("[data-no]", back)!.onclick = () => done(false);
    back.onclick = (e) => {
      if (e.target === back) done(false);
    };
    $<HTMLButtonElement>("[data-yes]", back)!.focus();
  });
}

/** Pide un texto en un diálogo propio (`window.prompt` no es confiable dentro del WebView). */
export function promptDialog(text: string, value = ""): Promise<string | null> {
  return new Promise((resolve) => {
    const back = document.createElement("div");
    back.className = "modal-back";
    back.innerHTML = `<form class="modal small" role="dialog" aria-modal="true"><label>${esc(text)}<input name="v" value="${esc(value)}"></label>
      <div class="modal-actions"><button type="button" class="btn" data-no>${esc(t("common.cancel"))}</button>
      <button class="btn primary">${esc(t("common.ok"))}</button></div></form>`;
    document.body.append(back);
    const form = back.querySelector("form")!;
    const input = form.elements.namedItem("v") as HTMLInputElement;
    const done = (v: string | null) => { back.remove(); resolve(v); };
    form.onsubmit = (e) => { e.preventDefault(); done(input.value.trim() || null); };
    $("[data-no]", back)!.onclick = () => done(null);
    back.addEventListener("keydown", (e) => e.key === "Escape" && done(null));
    input.focus();
    input.select();
  });
}
