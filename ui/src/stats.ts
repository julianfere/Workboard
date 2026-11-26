// Datos y helpers que comparten las vistas Horas, Métricas y Tools: el tiempo trabajado y las métricas de
// las sesiones (se calculan en el backend a pedido), períodos, formato y exportar a CSV.

import { api, pickSavePath } from "./api";
import { render } from "./app";
import { todayIso, toast } from "./dom";
import { errorText, lang, t } from "./i18n";
import { S, itemByName, metaText, wf } from "./state";
import type { Item, SessionStats, TimeReport } from "./types";
import { periodKey } from "./views/today";
import { trackerDue, trackerEstimate } from "./views/tracker";

export const D = {
  time: null as TimeReport | null,
  timeLoading: false,
  stats: null as SessionStats[] | null,
  statsLoading: false,
  root: "",
};

/** Al cambiar de carpeta lo calculado deja de valer. */
function checkRoot(): void {
  if (D.root !== S.root) {
    D.root = S.root;
    D.time = null;
    D.stats = null;
  }
}

export async function loadTime(force = false): Promise<void> {
  checkRoot();
  if (D.timeLoading || (D.time && !force)) return;
  D.timeLoading = true;
  try {
    D.time = await api.timeReport();
  } catch (err) {
    toast(errorText(err), undefined, "error");
  } finally {
    D.timeLoading = false;
  }
  render();
}

export async function loadStats(force = false): Promise<void> {
  checkRoot();
  if (D.statsLoading || (D.stats && !force)) return;
  D.statsLoading = true;
  try {
    D.stats = (await api.statsReport()).sessions;
  } catch (err) {
    toast(errorText(err), undefined, "error");
  } finally {
    D.statsLoading = false;
  }
  render();
}

export function timeReady(): TimeReport | null {
  checkRoot();
  if (!D.time && !D.timeLoading) void loadTime();
  return D.time;
}

// ---------- períodos (`AAAA-MM`, del día de inicio del flujo al anterior del mes siguiente) ----------

const pad = (n: number) => String(n).padStart(2, "0");

export const curPeriod = () => periodKey(todayIso());

export function periodShift(k: string, n: number): string {
  const [y, m] = k.split("-").map(Number);
  const d = new Date(y, m - 1 + n, 1);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}`;
}

/** Primer día del período (`AAAA-MM-DD`). */
export const periodStart = (k: string) => `${k}-${pad(wf().period.startDay)}`;

export function periodRange(k: string): string {
  const [y, m] = k.split("-").map(Number);
  const start = new Date(y, m - 1, wf().period.startDay);
  const end = new Date(y, m, wf().period.startDay - 1);
  const f = (d: Date, year: boolean) => d.toLocaleDateString(lang(), { day: "numeric", month: "short", ...(year ? { year: "numeric" } : {}) });
  return `${f(start, false)} – ${f(end, true)}`;
}

/** Período de un item: el propio o el de su fecha pactada, la del README o la del tracker ("" = sin pactar). */
export function itemPeriod(it: Item): string {
  const own = metaText(it, wf().fields.period);
  if (own) return own;
  const due = metaText(it, wf().fields.due) || (it.kind === "item" ? trackerDue(it.ids) : "");
  return due ? periodKey(due) : "";
}

const members = (it: Item) => (it.kind === "group" ? (it.members.map(itemByName).filter(Boolean) as Item[]) : []);

const ownHours = (it: Item): number | null => {
  const h = parseFloat(metaText(it, wf().fields.hours).replace(",", "."));
  return Number.isFinite(h) ? h : null;
};

/** Horas pactadas: las del README; si no, la estimación del tracker. Un grupo sin horas propias suma las de
 *  sus items (dos carpetas con el mismo id cuentan la estimación del tracker una vez). */
export function plannedHours(it: Item): number | null {
  const own = ownHours(it);
  if (own !== null) return own;
  if (it.kind !== "group") return trackerEstimate(it.ids);
  const subs = members(it);
  const local = subs.map(ownHours).filter((h): h is number => h !== null);
  const tracked = trackerEstimate(subs.filter((m) => ownHours(m) === null).flatMap((m) => m.ids));
  const all = tracked === null ? local : [...local, tracked];
  return all.length ? all.reduce((a, b) => a + b, 0) : null;
}

/** Entradas de tiempo del item (y de sus items, si es un grupo). */
export function timeEntries(it: Item) {
  if (!D.time) return [];
  return [it, ...members(it)].map((x) => D.time!.byItem[x.name]).filter(Boolean);
}

/** Segundos trabajados entre dos días (`AAAA-MM-DD`, el final excluido); null = sin límite. */
export function secsIn(it: Item, from: string | null, to: string | null): number {
  let n = 0;
  for (const e of timeEntries(it)) {
    for (const [d, sec] of Object.entries(e.days)) if ((from === null || d >= from) && (to === null || d < to)) n += sec;
  }
  return n;
}

export const workedIn = (it: Item, k: string) => secsIn(it, periodStart(k), periodStart(periodShift(k, 1))) / 3600;
export const workedTotal = (it: Item) => secsIn(it, null, null) / 3600;

/** Sesiones con tiempo para el item, la de más tiempo primero. */
export function sessionsWithTime(it: Item): { id: string; secs: number }[] {
  const by = new Map<string, number>();
  for (const e of timeEntries(it)) for (const [id, sec] of Object.entries(e.sessions)) by.set(id, (by.get(id) ?? 0) + sec);
  return [...by.entries()].map(([id, secs]) => ({ id, secs })).sort((a, b) => b.secs - a.secs);
}

// ---------- formato ----------

export function fmtNum(n: number, digits = 1): string {
  return n.toLocaleString(lang(), { maximumFractionDigits: digits });
}

export const fmtH = (h: number | null | undefined) => (h === null || h === undefined ? "—" : t("hours.h", { n: fmtNum(h) }));

export function fmtMin(secs: number | null | undefined): string {
  if (secs === null || secs === undefined) return "—";
  return secs < 3600 ? t("metrics.min", { n: Math.round(secs / 60) }) : t("hours.h", { n: fmtNum(secs / 3600) });
}

export function fmtK(n: number | null | undefined): string {
  if (n === null || n === undefined) return "—";
  if (n >= 1e6) return `${fmtNum(n / 1e6)} M`;
  if (n >= 1e3) return `${fmtNum(n / 1e3)} k`;
  return String(Math.round(n));
}

export function fmtDateTime(ts: number | null | undefined): string {
  if (!ts) return "";
  return new Date(ts * 1000).toLocaleString(lang(), { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
}

// ---------- exportar ----------

/** CSV con `;` (lo que espera Excel con coma decimal). */
export function toCsv(rows: (string | number | null | undefined)[][]): string {
  return rows
    .map((r) =>
      r
        .map((v) => {
          const x = String(v ?? "");
          return /[;"\n]/.test(x) ? `"${x.replace(/"/g, '""')}"` : x;
        })
        .join(";"),
    )
    .join("\r\n");
}

/** Número para el CSV, con el separador decimal del idioma. */
export const csvNum = (n: number | null | undefined) => (n === null || n === undefined ? "" : (Math.round(n * 100) / 100).toLocaleString(lang(), { useGrouping: false }));

/** Exportar con «Guardar como» en la app (en el navegador, a Descargas). Cancelar no hace nada. */
export async function exportCsv(name: string, csv: string): Promise<void> {
  try {
    const target = await pickSavePath(name);
    if (target === null) return;
    const path = await api.saveExport(name, csv, target);
    toast(t("metrics.exported", { path }), { label: t("metrics.reveal"), run: () => void api.revealExport(path) });
  } catch (err) {
    toast(errorText(err), undefined, "error");
  }
}
