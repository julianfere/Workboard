// Planificación de cada item: prioridad (del tracker), fecha pactada, período y horas pactadas. Lo cargado en
// el frontmatter pisa lo del tracker; un grupo sin valor propio toma el de sus items abiertos (la prioridad
// más alta, la fecha más temprana). Las claves del frontmatter salen del flujo (`wf().fields`).

import { daysUntil, esc, todayIso } from "./dom";
import { lang, t } from "./i18n";
import { effectiveStatus, isClosed, itemByName, metaText, wf } from "./state";
import { curPeriod, plannedHours, periodRange } from "./stats";
import type { Item } from "./types";
import { periodKey } from "./views/today";
import { TR, trackerConnected, trackerDue, trackerEstimate } from "./views/tracker";

/** De dónde sale un valor: el README, el tracker o los items del grupo. */
export type Source = "local" | "tracker" | "group" | "";

const isoDate = /^\d{4}-\d{2}-\d{2}$/;
const periodCode = /^\d{4}-\d{2}$/;

function members(it: Item): Item[] {
  return it.kind === "group" ? (it.members.map(itemByName).filter(Boolean) as Item[]) : [];
}

/** Los items del grupo que siguen abiertos (o todos, si están todos cerrados). */
function openMembers(it: Item): Item[] {
  const all = members(it);
  const open = all.filter((m) => !isClosed(effectiveStatus(m)));
  return open.length ? open : all;
}

/** Prioridad del tracker (1 = la más alta); en un grupo, la más alta de sus items abiertos. */
export function priorityOf(it: Item): number | null {
  if (it.kind === "group") {
    const ps = openMembers(it).map(priorityOf).filter((p): p is number => p !== null);
    return ps.length ? Math.min(...ps) : null;
  }
  if (!trackerConnected()) return null;
  const ps = it.ids.map((id) => TR.cfg?.cache?.items[id]?.priority).filter((p): p is number => typeof p === "number");
  return ps.length ? Math.min(...ps) : null;
}

/** Fecha pactada (`AAAA-MM-DD`) y de dónde sale. */
export function dueOf(it: Item): { date: string; src: Source } {
  const own = metaText(it, wf().fields.due);
  if (isoDate.test(own)) return { date: own, src: "local" };
  if (it.kind === "group") {
    const ds = openMembers(it).map((m) => dueOf(m).date).filter(Boolean).sort();
    return ds.length ? { date: ds[0], src: "group" } : { date: "", src: "" };
  }
  const tr = trackerDue(it.ids);
  return tr ? { date: tr, src: "tracker" } : { date: "", src: "" };
}

/** Vencida: la fecha pactada ya pasó y el item no está cerrado. */
export function isOverdue(it: Item): boolean {
  const { date } = dueOf(it);
  return !!date && date < todayIso() && !isClosed(effectiveStatus(it));
}

/** Período (`AAAA-MM`): el fijado en el README (`manual`) o el de la fecha pactada. "" = sin pactar. */
export function periodOf(it: Item): { key: string; manual: boolean } {
  const own = metaText(it, wf().fields.period);
  if (periodCode.test(own)) return { key: own, manual: true };
  if (it.kind === "group" && !metaText(it, wf().fields.due)) {
    const ks = openMembers(it).map((m) => periodOf(m).key).filter(Boolean).sort();
    if (ks.length) return { key: ks[0], manual: false };
  }
  const { date } = dueOf(it);
  return { key: date ? periodKey(date) : "", manual: false };
}

/** Horas pactadas (las del README, las del tracker o la suma del grupo) y de dónde salen. */
export function hoursOf(it: Item): { hours: number | null; src: Source } {
  const h = plannedHours(it);
  if (h === null) return { hours: null, src: "" };
  const own = parseFloat(metaText(it, wf().fields.hours).replace(",", "."));
  return { hours: h, src: Number.isFinite(own) ? "local" : it.kind === "group" ? "group" : "tracker" };
}

/** ¿El tracker tiene un valor propio que lo cargado en el README está pisando? */
export function overridesTracker(it: Item, what: "due" | "hours" | "period"): boolean {
  if (it.kind !== "item" || !trackerConnected()) return false;
  const own = metaText(it, what === "due" ? wf().fields.due : what === "hours" ? wf().fields.hours : wf().fields.period);
  if (!own) return false;
  return what === "hours" ? trackerEstimate(it.ids) !== null : !!trackerDue(it.ids);
}

/** Cuántos períodos hay entre el actual y `k` (0 = actual, <0 = pasados). */
export function periodDiff(k: string): number {
  const [ay, am] = curPeriod().split("-").map(Number);
  const [by, bm] = k.split("-").map(Number);
  return (by - ay) * 12 + bm - am;
}

export function periodRel(k: string): string {
  const n = periodDiff(k);
  if (n === 0) return t("grid.period.current");
  if (n === -1) return t("grid.period.previous");
  if (n === 1) return t("grid.period.next");
  return n < 0 ? t("grid.period.ago", { n: -n }) : t("grid.period.in", { n });
}

/** Orden por urgencia: prioridad, fecha pactada y actividad. */
export function byUrgency(a: Item, b: Item): number {
  const pa = priorityOf(a) ?? 99;
  const pb = priorityOf(b) ?? 99;
  if (pa !== pb) return pa - pb;
  const da = dueOf(a).date || "9999";
  const db = dueOf(b).date || "9999";
  if (da !== db) return da < db ? -1 : 1;
  return b.last - a.last;
}

// ---------- marcas ----------

export const fmtDay = (iso: string) => {
  const [y, m, d] = iso.split("-").map(Number);
  return new Date(y, m - 1, d).toLocaleDateString(lang(), { day: "2-digit", month: "2-digit", year: "2-digit" });
};

/** Marca «local»: el valor sale del README y pisa al del tracker. */
export const localTag = () => `<span class="src-tag local" title="${esc(t("grid.local.hint"))}">${esc(t("grid.local"))}</span>`;

/** Marca «vencida» junto al título. */
export function lateTag(it: Item): string {
  if (!isOverdue(it)) return "";
  const days = -daysUntil(dueOf(it).date);
  return `<span class="tag-late" title="${esc(t("grid.overdue.hint", { date: fmtDay(dueOf(it).date), n: days }))}">${esc(t("grid.overdue"))}</span>`;
}

export function priorityBadge(p: number | null): string {
  if (p === null) return "";
  return `<span class="prio p${Math.min(Math.max(p, 1), 4)}" title="${esc(t("grid.priority.hint", { n: p }))}">P${p}</span>`;
}

/** Celda de la fecha pactada: la fecha (o el período fijado) con sus marcas. */
export function dueCell(it: Item): string {
  const due = dueOf(it);
  const per = periodOf(it);
  if (!due.date && !per.key) return "";
  const lines = [
    due.date ? t(`grid.due.src.${due.src || "local"}`, { date: fmtDay(due.date) }) : "",
    per.key ? t(per.manual ? "grid.period.manual" : "grid.period.of", { range: periodRange(per.key), rel: periodRel(per.key) }) : "",
  ].filter(Boolean);
  const local = (due.src === "local" && overridesTracker(it, "due")) || (per.manual && (overridesTracker(it, "period") || !!due.date));
  const text = due.date ? esc(fmtDay(due.date)) : `<span class="muted">${esc(per.key)}</span>`;
  return `<span class="pact" title="${esc(lines.join("\n"))}">${text}${local ? localTag() : ""}${lateTag(it)}</span>`;
}

/** Cantidad de horas con su marca. */
export function hoursCell(it: Item, fmt: (h: number) => string): string {
  const { hours, src } = hoursOf(it);
  if (hours === null) return "";
  const local = src === "local" && overridesTracker(it, "hours");
  return `<span class="num" title="${esc(t(`grid.hours.src.${src || "local"}`))}">${esc(fmt(hours))}${local ? localTag() : ""}</span>`;
}

/** Filtro por fecha pactada: hasta el período actual (los sin fecha entran), solo el actual o vencidas. */
export function passesDue(it: Item, f: string): boolean {
  if (!f) return true;
  if (f === "late") return isOverdue(it);
  const k = periodOf(it).key;
  if (f === "cur") return k === curPeriod();
  return !k || k <= curPeriod();
}

