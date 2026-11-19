// Horas: pactadas (frontmatter) contra trabajadas (tiempo activo de las sesiones de agentes, repartido
// entre los items de cada sesión), por período o en total. Desglose por sesión y por período de cada item.

import { onAction, openDetail, registerView, render } from "../app";
import { esc } from "../dom";
import { t } from "../i18n";
import { ico } from "../icons";
import { S, effectiveStatus, isClosed, itemByName, stateLabel, titleOf, typeLabel } from "../state";
import {
  D,
  csvNum,
  curPeriod,
  exportCsv,
  fmtDateTime,
  fmtH,
  itemPeriod,
  loadTime,
  periodRange,
  periodShift,
  plannedHours,
  sessionsWithTime,
  timeReady,
  toCsv,
  workedIn,
  workedTotal,
} from "../stats";
import type { Item } from "../types";
import { agentLabel } from "../sessions";
import { statusChip } from "./items";
import { periodKey } from "./today";

const V = { all: false, period: "", sort: "dev" as "dev" | "plan" | "inP" | "total" | "title" };

interface Row {
  it: Item;
  plan: number | null;
  inP: number;
  total: number;
}

const units = () => S.items.filter((i) => !i.group);
const ratio = (r: { total: number; plan: number | null }) => (r.plan ? r.total / r.plan : null);

function rows(k: string | null): { planned: Row[]; other: Row[] } {
  const row = (it: Item): Row => ({ it, plan: plannedHours(it), inP: k ? workedIn(it, k) : workedTotal(it), total: workedTotal(it) });
  if (k === null) {
    return { planned: units().filter((i) => plannedHours(i) !== null || workedTotal(i) > 0).map(row), other: [] };
  }
  const inPeriod = (i: Item) => workedIn(i, k) > 0;
  return {
    planned: units().filter((i) => itemPeriod(i) === k).map(row),
    other: units().filter((i) => itemPeriod(i) !== k && inPeriod(i)).map(row),
  };
}

function bar(real: number, plan: number | null): string {
  const p = plan ? real / plan : null;
  if (p === null) return `<span class="muted">—</span>`;
  const cls = p > 1 ? "over" : p > 0.8 ? "warn" : "";
  return `<span class="h-bar ${cls}" title="${esc(t("hours.ofPlanned", { pct: Math.round(p * 100) }))}"><i style="width:${Math.min(100, p * 100)}%"></i></span> <span class="h-dev ${cls}">${Math.round(p * 100)}%</span>`;
}

function table(list: Row[], k: string | null, withPeriod: boolean): string {
  const key: Record<typeof V.sort, (r: Row) => number | string> = {
    dev: (r) => -(ratio(r) ?? -1),
    plan: (r) => -(r.plan ?? -1),
    inP: (r) => -r.inP,
    total: (r) => -r.total,
    title: (r) => titleOf(r.it).toLowerCase(),
  };
  const sorted = [...list].sort((a, b) => (key[V.sort](a) > key[V.sort](b) ? 1 : key[V.sort](a) < key[V.sort](b) ? -1 : 0));
  const th = (id: typeof V.sort, label: string, cls = "") =>
    `<th class="${cls}"><button class="th" data-action="hours-sort" data-key="${id}">${esc(label)}${V.sort === id ? ico("chevronDown") : ""}</button></th>`;
  const tot = sorted.reduce((a, r) => ({ plan: a.plan + (r.plan ?? 0), inP: a.inP + r.inP, total: a.total + r.total }), { plan: 0, inP: 0, total: 0 });
  const body = sorted
    .map((r) => {
      const i = r.it;
      const per = itemPeriod(i);
      const ns = sessionsWithTime(i).length;
      return `<tr class="row" data-action="hours-item" data-name="${esc(i.name)}"><td>${statusChip(effectiveStatus(i))}</td>
        <td class="muted">${i.kind === "group" ? ico("layers") : esc(i.ids.join(", ") || "—")}</td>
        <td class="h-title">${esc(titleOf(i))}</td>
        ${withPeriod ? `<td class="muted">${per ? esc(periodRange(per)) : esc(t("hours.unplanned"))}</td>` : ""}
        <td class="num">${fmtH(r.plan)}</td>${k !== null ? `<td class="num">${r.inP ? fmtH(r.inP) : "—"}</td>` : ""}
        <td class="num">${r.total ? fmtH(r.total) : "—"}</td><td>${bar(r.total, r.plan)}</td><td class="num">${ns || "—"}</td></tr>`;
    })
    .join("");
  return `<table class="grid h-table"><thead><tr><th>${esc(t("hours.col.status"))}</th><th>${esc(t("hours.col.id"))}</th>${th("title", t("hours.col.title"))}
      ${withPeriod ? `<th>${esc(t("hours.col.period"))}</th>` : ""}${th("plan", t("hours.col.planned"), "num")}${k !== null ? th("inP", t("hours.col.inPeriod"), "num") : ""}
      ${th("total", t("hours.col.total"), "num")}${th("dev", t("hours.col.progress"))}<th class="num">${esc(t("hours.col.sessions"))}</th></tr></thead>
    <tbody>${body}</tbody>
    <tfoot><tr><td colspan="${withPeriod ? 4 : 3}">${esc(t("hours.totalRow", { n: sorted.length }))}</td><td class="num">${fmtH(tot.plan)}</td>
      ${k !== null ? `<td class="num">${fmtH(tot.inP)}</td>` : ""}<td class="num">${fmtH(tot.total)}</td><td>${bar(tot.total, tot.plan || null)}</td><td></td></tr></tfoot></table>`;
}

function renderHours(): string {
  const time = timeReady();
  if (!V.period) V.period = curPeriod();
  const k = V.all ? null : V.period;
  const head = `<div class="toolbar"><h1>${esc(t("nav.hours"))}</h1>
    <div class="seg"><button class="${V.all ? "" : "on"}" data-action="hours-mode" data-all="0">${esc(t("hours.byPeriod"))}</button><button class="${V.all ? "on" : ""}" data-action="hours-mode" data-all="1">${esc(t("hours.all"))}</button></div>
    ${V.all ? "" : `<span class="h-nav"><button class="icon-btn" data-action="hours-period" data-step="-1" title="${esc(t("hours.prev"))}" aria-label="${esc(t("hours.prev"))}">${ico("chevronLeft")}</button>
      <b>${esc(periodRange(V.period))}</b>
      <button class="icon-btn" data-action="hours-period" data-step="1" title="${esc(t("hours.next"))}" aria-label="${esc(t("hours.next"))}">${ico("chevronRight")}</button>
      ${V.period !== curPeriod() ? `<button class="btn" data-action="hours-period" data-step="0">${esc(t("hours.current"))}</button>` : ""}</span>`}
    <span class="spacer"></span><span class="muted">${esc(D.timeLoading ? t("hours.loading") : t("hours.workedIs"))}</span>
    <button class="btn" data-action="hours-export">${ico("download")}${esc(t("hours.export"))}</button>
    <button class="icon-btn" data-action="hours-refresh" title="${esc(t("hours.refresh"))}" aria-label="${esc(t("hours.refresh"))}">${ico("refresh")}</button></div>`;
  if (!time) return `<div class="page wide">${head}<p class="empty">${esc(t("hours.loading"))}</p></div>`;
  const { planned, other } = rows(k);
  const sum = (list: Row[], f: (r: Row) => number | null) => list.reduce((n, r) => n + (f(r) ?? 0), 0);
  const plan = sum(planned, (r) => r.plan);
  const workedP = sum(planned, (r) => r.inP);
  const workedAll = workedP + sum(other, (r) => r.inP);
  const open = planned.filter((r) => !isClosed(effectiveStatus(r.it)));
  const noHours = planned.filter((r) => r.plan === null).length;
  const stat = (label: string, val: string, sub = "") => `<div class="stat"><span>${esc(label)}</span><b>${esc(val)}</b>${sub ? `<span>${esc(sub)}</span>` : ""}</div>`;
  const stats = `<div class="stats">
    ${stat(t(V.all ? "hours.kpi.planned" : "hours.kpi.plannedPeriod"), fmtH(plan), t("hours.kpi.items", { n: planned.length }) + (noHours ? ` · ${t("hours.kpi.noHours", { n: noHours })}` : ""))}
    ${stat(t(V.all ? "hours.kpi.worked" : "hours.kpi.workedPlanned"), fmtH(workedP), plan ? t("hours.ofPlanned", { pct: Math.round((workedP / plan) * 100) }) : "")}
    ${V.all ? "" : stat(t("hours.kpi.workedPeriod"), fmtH(workedAll), other.length ? t("hours.kpi.otherItems", { h: fmtH(sum(other, (r) => r.inP)), n: other.length }) : t("hours.kpi.allPlanned"))}
    ${stat(t("hours.kpi.openPlanned"), fmtH(sum(open, (r) => r.plan)), t("hours.kpi.open", { n: open.length }))}</div>`;
  const legend = `<p class="muted small">${esc(t("hours.legend"))}</p>`;
  if (!planned.length && !other.length) {
    return `<div class="page wide">${head}${stats}<p class="empty">${esc(t(V.all ? "hours.emptyAll" : "hours.emptyPeriod"))}</p></div>`;
  }
  return `<div class="page wide">${head}${stats}
    <section class="card"><h2>${ico("clock")}${esc(t(V.all ? "hours.withHours" : "hours.plannedForPeriod"))} <span class="muted">${planned.length}</span></h2>${legend}
      ${planned.length ? table(planned, k, V.all) : `<p class="empty">${esc(t("hours.nonePlanned"))}</p>`}</section>
    ${other.length ? `<section class="card"><h2>${esc(t("hours.otherWorked"))} <span class="muted">${other.length}</span></h2>${table(other, k, true)}</section>` : ""}</div>`;
}

registerView("hours", renderHours);

onAction("hours-mode", (el) => {
  V.all = el.dataset.all === "1";
  render();
});
onAction("hours-period", (el) => {
  const step = Number(el.dataset.step);
  V.period = step === 0 ? curPeriod() : periodShift(V.period || curPeriod(), step);
  render();
});
onAction("hours-sort", (el, ev) => {
  ev.stopPropagation();
  V.sort = el.dataset.key as typeof V.sort;
  render();
});
onAction("hours-refresh", () => loadTime(true));
onAction("hours-export", async () => {
  const k = V.all ? null : V.period || curPeriod();
  const { planned, other } = rows(k);
  const out: (string | number | null)[][] = [
    ["group", "folder", "ids", "type", "status", "title", "period", "planned", "inPeriod", "total", "progress"].map((c) => t(`hours.csv.${c}`)),
  ];
  for (const [g, r] of [...planned.map((r) => ["planned", r] as const), ...other.map((r) => ["other", r] as const)]) {
    const i = r.it;
    const per = itemPeriod(i);
    out.push([
      t(g === "planned" ? "hours.csv.groupPlanned" : "hours.csv.other"),
      i.name,
      i.ids.join(" "),
      i.kind === "group" ? t("hours.group") : i.type ? typeLabel(i.type) : "",
      stateLabel(effectiveStatus(i)),
      titleOf(i),
      per ? periodRange(per) : "",
      csvNum(r.plan),
      k ? csvNum(r.inP) : "",
      csvNum(r.total),
      r.plan ? Math.round((r.total / r.plan) * 100) : "",
    ]);
  }
  await exportCsv(`${t("nav.hours")}-${k ?? t("hours.all")}.csv`, toCsv(out));
});

// ---------- desglose de un item ----------

onAction("hours-item", (el) => {
  const it = itemByName(el.dataset.name!);
  if (!it || !D.time) return;
  const list = sessionsWithTime(it);
  const h = plannedHours(it);
  const real = workedTotal(it);
  const byPeriod = new Map<string, number>();
  for (const e of [it, ...it.members.map(itemByName)].filter(Boolean).map((x) => D.time!.byItem[x!.name]).filter(Boolean)) {
    for (const [d, sec] of Object.entries(e.days)) {
      const k = periodKey(d);
      byPeriod.set(k, (byPeriod.get(k) ?? 0) + sec);
    }
  }
  const sessions = new Map(D.time.sessions.map((s) => [s.id, s]));
  const rowsHtml = list
    .map((x) => {
      const s = sessions.get(x.id);
      return `<tr><td>${esc(s?.name ?? x.id.slice(0, 8))}</td><td>${esc(s ? agentLabel({ id: s.agent }) : "")}</td><td class="muted">${esc(s?.source ? t(`hours.source.${s.source}`) : "")}</td>
        <td class="muted">${esc(fmtDateTime(s?.from))}</td><td class="muted">${esc(fmtDateTime(s?.to))}</td>
        <td class="num">${fmtH(x.secs / 3600)}</td><td class="num">${s ? fmtH(s.active / 3600) : ""}</td></tr>`;
    })
    .join("");
  const periods = [...byPeriod.entries()]
    .sort((a, b) => a[0].localeCompare(b[0]))
    .map(([k, sec]) => `<tr><td>${esc(periodRange(k))}</td><td class="num">${fmtH(sec / 3600)}</td></tr>`)
    .join("");
  const back = document.createElement("div");
  back.className = "modal-back";
  back.innerHTML = `<div class="modal wide" role="dialog" aria-modal="true"><h2>${esc(t("hours.detailTitle", { title: titleOf(it) }))}</h2>
    <p class="muted">${esc(t("hours.detailSub", { planned: fmtH(h), worked: fmtH(real) }))}${h ? ` (${Math.round((real / h) * 100)}%)` : ""}</p>
    <h3>${esc(t("hours.bySession"))}</h3>
    <table class="grid h-table"><thead><tr><th>${esc(t("hours.col.session"))}</th><th>${esc(t("hours.col.agent"))}</th><th>${esc(t("hours.col.source"))}</th><th>${esc(t("hours.col.from"))}</th><th>${esc(t("hours.col.to"))}</th><th class="num">${esc(t("hours.col.forItem"))}</th><th class="num">${esc(t("hours.col.sessionTotal"))}</th></tr></thead>
    <tbody>${rowsHtml || `<tr><td colspan="7" class="muted">${esc(t("hours.noSessions"))}</td></tr>`}</tbody></table>
    ${periods ? `<h3>${esc(t("hours.byPeriodTitle"))}</h3><table class="grid h-table"><tbody>${periods}</tbody></table>` : ""}
    <div class="modal-actions"><button class="btn" data-open>${esc(t("hours.openDetail"))}</button><span class="spacer"></span><button class="btn primary" data-close>${esc(t("common.close"))}</button></div></div>`;
  document.body.append(back);
  const close = () => back.remove();
  back.querySelector<HTMLElement>("[data-close]")!.onclick = close;
  back.querySelector<HTMLElement>("[data-open]")!.onclick = () => {
    close();
    void openDetail(it.name);
  };
  back.onclick = (e) => e.target === back && close();
  back.addEventListener("keydown", (e) => e.key === "Escape" && close());
  back.querySelector<HTMLElement>("[data-close]")!.focus();
});
