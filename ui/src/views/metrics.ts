// Métricas: cada sesión es un dato (sesiones como evals). Cuánto pidió del usuario (mensajes, pedidos,
// interrupciones), cuántos tokens usó y si sus items avanzaron o retrocedieron mientras trabajaba,
// agrupado por configuración (agente · modelo · modo · Planear) para comparar con datos.

import { onAction, onChange, registerView, render } from "../app";
import { esc } from "../dom";
import { t } from "../i18n";
import { ico } from "../icons";
import { agentLabel, openSession, sessionById } from "../sessions";
import { itemByName, stateDef, stateLabel, typeLabel, wf } from "../state";
import { D, exportCsv, fmtDateTime, fmtK, fmtMin, fmtNum, loadStats, loadTime, timeReady, toCsv } from "../stats";
import type { SessionStats } from "../types";

const V = { range: "90" as "30" | "90" | "all", type: "", all: false };
const SHOWN = 40;

interface Row extends SessionStats {
  cfg: string;
  active: number | null;
}

const mainModel = (r: SessionStats) => {
  const e = Object.entries(r.models).sort((a, b) => b[1] - a[1])[0];
  return (e ? e[0] : r.model ?? "").replace(/^claude-/, "");
};

function config(r: SessionStats): string {
  return [agentLabel({ id: r.agent }), mainModel(r), t(`session.mode.${r.mode}`), r.plan ? t("metrics.planTag") : ""].filter(Boolean).join(" · ");
}

/** Tipo de los items de la sesión (el primero que tenga). */
function typeOf(r: SessionStats): string {
  for (const w of r.items) {
    const it = itemByName(w);
    if (!it) continue;
    const types = it.kind === "group" ? it.members.map((m) => itemByName(m)?.type ?? "") : [it.type];
    const found = types.find(Boolean);
    if (found) return found;
  }
  return "";
}

const advanced = (r: SessionStats) => Object.values(r.outcome).some((o) => o.advance > 0);
const reworked = (r: SessionStats) => Object.values(r.outcome).some((o) => o.setbacks > 0);

function rows(): Row[] {
  const since = V.range === "all" ? 0 : Date.now() / 1000 - Number(V.range) * 86400;
  const active = new Map((D.time?.sessions ?? []).map((s) => [s.id, s.active]));
  return (D.stats ?? [])
    .filter((r) => (r.to ?? r.from ?? 0) >= since && (!V.type || typeOf(r) === V.type) && (r.prompts || r.tokens))
    .map((r) => ({ ...r, cfg: config(r), active: active.get(r.id) ?? null }));
}

function avg(xs: Row[], f: (r: Row) => number | null): number | null {
  const v = xs.map(f).filter((x): x is number => x !== null && x !== undefined);
  return v.length ? v.reduce((a, b) => a + b, 0) / v.length : null;
}

function groups(list: Row[]) {
  const by = new Map<string, Row[]>();
  for (const r of list) by.set(r.cfg, [...(by.get(r.cfg) ?? []), r]);
  return [...by.entries()]
    .map(([cfg, xs]) => ({
      cfg,
      n: xs.length,
      adv: xs.filter(advanced).length / xs.length,
      back: xs.filter(reworked).length,
      prompts: avg(xs, (r) => r.prompts),
      asks: avg(xs, (r) => r.asks),
      inter: avg(xs, (r) => r.interrupts),
      out: avg(xs, (r) => r.tokens?.output ?? null),
      ctx: avg(xs, (r) => (r.tokens ? r.tokens.input + r.tokens.cacheRead + r.tokens.cacheWrite : null)),
      active: avg(xs, (r) => r.active),
    }))
    .sort((a, b) => b.n - a.n);
}

const num = (x: number | null) => (x === null ? "—" : fmtNum(x));
const pct = (x: number | null) => (x === null ? "—" : `${Math.round(x * 100)} %`);

function stateTag(id: string | null): string {
  const cat = id ? (stateDef(id)?.category ?? "unknown") : "unknown";
  return `<span class="chip"><span class="dot ${cat}"></span>${esc(id ? stateLabel(id) : t("items.noStatus"))}</span>`;
}

function result(r: SessionStats): string {
  const parts = Object.values(r.outcome)
    .filter((o) => o.start || o.end)
    .map((o) => `${stateTag(o.start)} ${ico("chevronRight")} ${stateTag(o.end)}${o.setbacks ? ` <span class="chip warn-text" title="${esc(t("metrics.reworkHint"))}">${esc(t("metrics.rework"))}</span>` : ""}`);
  return parts.length ? parts.join("<br>") : `<span class="muted">${esc(t("metrics.noChange"))}</span>`;
}

function renderMetrics(): string {
  if (!D.stats && !D.statsLoading) void loadStats();
  timeReady();
  const ranges = (["30", "90", "all"] as const)
    .map((r) => `<button class="${V.range === r ? "on" : ""}" data-action="metrics-range" data-range="${r}">${esc(t(`metrics.range.${r}`))}</button>`)
    .join("");
  const types = wf().types.map((x) => `<option value="${esc(x.id)}"${V.type === x.id ? " selected" : ""}>${esc(typeLabel(x.id))}</option>`).join("");
  const head = `<div class="toolbar"><h1>${esc(t("nav.metrics"))}</h1><span class="muted">${esc(t("metrics.subtitle"))}</span>
    <div class="seg">${ranges}</div>
    <select data-change="metrics-type" aria-label="${esc(t("metrics.type"))}"><option value="">${esc(t("metrics.allTypes"))}</option>${types}</select>
    <span class="spacer"></span>
    <button class="btn" data-action="metrics-export">${ico("download")}${esc(t("metrics.export"))}</button>
    <button class="icon-btn" data-action="metrics-refresh" title="${esc(t("metrics.refresh"))}" aria-label="${esc(t("metrics.refresh"))}">${ico("refresh")}</button></div>`;
  if (!D.stats) return `<div class="page wide">${head}<p class="empty">${esc(t("metrics.loading"))}</p></div>`;
  const list = rows();
  if (!list.length) return `<div class="page wide">${head}<p class="empty">${esc(t("metrics.empty"))}</p></div>`;
  const gs = groups(list);
  const tot = (f: (r: Row) => number | null | undefined) => list.reduce((n, r) => n + (f(r) ?? 0), 0);
  const withTokens = list.filter((r) => r.tokens).length;
  const stat = (label: string, val: string, sub = "") => `<div class="stat"><span>${esc(label)}</span><b>${esc(val)}</b>${sub ? `<span>${esc(sub)}</span>` : ""}</div>`;
  const stats = `<div class="stats">
    ${stat(t("metrics.kpi.sessions"), String(list.length), t("metrics.kpi.configs", { n: gs.length }))}
    ${stat(t("metrics.kpi.advanced"), pct(list.filter(advanced).length / list.length), t("metrics.kpi.rework", { n: list.filter(reworked).length }))}
    ${stat(t("metrics.kpi.prompts"), num(tot((r) => r.prompts) / list.length), t("metrics.kpi.asks", { n: num(tot((r) => r.asks) / list.length) }))}
    ${stat(t("metrics.kpi.output"), fmtK(tot((r) => r.tokens?.output)), t("metrics.kpi.perSession", { n: fmtK(withTokens ? tot((r) => r.tokens?.output) / withTokens : null) }))}</div>`;
  const cfgRows = gs
    .map((g) => `<tr><td><b>${esc(g.cfg)}</b></td><td class="num">${g.n}</td><td class="num">${pct(g.adv)}</td><td class="num">${g.back || "—"}</td>
      <td class="num">${num(g.prompts)}</td><td class="num">${num(g.asks)}</td><td class="num">${num(g.inter)}</td><td class="num">${fmtK(g.out)}</td><td class="num">${fmtK(g.ctx)}</td><td class="num">${fmtMin(g.active)}</td></tr>`)
    .join("");
  const shown = [...list].sort((a, b) => (b.to ?? 0) - (a.to ?? 0)).slice(0, V.all ? list.length : SHOWN);
  const sessRows = shown
    .map((r) => {
      const open = sessionById(r.id);
      return `<tr class="${open ? "row" : ""}"${open ? ` data-action="metrics-open" data-id="${esc(r.id)}"` : ""}><td><b>${esc(r.name || r.id.slice(0, 8))}</b><div class="muted small">${esc(fmtDateTime(r.to))}</div></td>
        <td class="small">${esc(r.cfg)}</td><td class="num">${fmtMin(r.active)}</td><td class="num">${r.prompts}</td><td class="num">${r.asks || "—"}</td>
        <td class="num">${fmtK(r.tokens?.output)}</td><td class="num">${r.scripts.written.length || "—"}</td><td class="small">${result(r)}</td></tr>`;
    })
    .join("");
  const c = (k: string, cls = "num") => `<th class="${cls}">${esc(t(`metrics.col.${k}`))}</th>`;
  return `<div class="page wide">${head}${stats}
    <section class="card"><h2>${ico("layers")}${esc(t("metrics.byConfig"))}</h2><p class="muted small">${esc(t("metrics.byConfigHint"))}</p>
      <table class="grid h-table"><thead><tr>${c("config", "")}${c("sessions")}${c("advanced")}${c("rework")}${c("prompts")}${c("asks")}${c("interrupts")}${c("output")}${c("context")}${c("active")}</tr></thead>
      <tbody>${cfgRows}</tbody></table></section>
    <section class="card"><h2>${ico("terminal")}${esc(t("metrics.sessions"))} <span class="muted">${list.length}</span></h2>
      <table class="grid h-table"><thead><tr>${c("session", "")}${c("config", "")}${c("active")}${c("prompts")}${c("asks")}${c("output")}${c("scripts")}${c("items", "")}</tr></thead>
      <tbody>${sessRows}</tbody></table>
      ${list.length > shown.length ? `<button class="link" data-action="metrics-all">${esc(t("metrics.showAll", { n: list.length }))}</button>` : ""}</section></div>`;
}

registerView("metrics", renderMetrics);

onAction("metrics-range", (el) => {
  V.range = el.dataset.range as typeof V.range;
  render();
});
onChange("metrics-type", (el) => {
  V.type = (el as HTMLSelectElement).value;
  render();
});
onAction("metrics-all", () => {
  V.all = true;
  render();
});
onAction("metrics-refresh", async () => {
  await Promise.all([loadStats(true), loadTime(true)]);
});
onAction("metrics-open", (el) => openSession(el.dataset.id!));
onAction("metrics-export", async () => {
  const cols = ["session", "id", "config", "agent", "model", "mode", "plan", "type", "items", "from", "to", "activeMin", "prompts", "asks", "interrupts", "input", "output", "cacheRead", "cacheWrite", "scripts", "advanced", "rework"];
  const d = (ts: number | null) => (ts ? new Date(ts * 1000).toISOString().slice(0, 16).replace("T", " ") : "");
  const yes = (b: boolean) => (b ? t("metrics.csv.yes") : "");
  const out: (string | number | null | undefined)[][] = [cols.map((c) => t(`metrics.csv.${c}`))];
  for (const r of rows()) {
    out.push([r.name, r.id, r.cfg, r.agent, mainModel(r), r.mode, yes(r.plan), typeOf(r) ? typeLabel(typeOf(r)) : "", r.items.join(" "), d(r.from), d(r.to),
      r.active === null ? "" : Math.round(r.active / 60), r.prompts, r.asks, r.interrupts,
      r.tokens?.input, r.tokens?.output, r.tokens?.cacheRead, r.tokens?.cacheWrite, r.scripts.written.length, yes(advanced(r)), yes(reworked(r))]);
  }
  await exportCsv(`${t("nav.metrics")}-${V.range}.csv`, toCsv(out));
});
