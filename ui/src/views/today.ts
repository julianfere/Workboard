// Hoy: saludo, el período actual (días, horas pactadas y trabajadas, cerrados), lo que necesita tu atención
// (sesiones que te necesitan, fechas vencidas, alertas importantes), lo que está en curso, fechas pactadas
// cercanas y las próximas tareas de lo que está en curso o pactado para el período.

import { api } from "../api";
import { onAction, onAfterRender, registerView, render, setNavCounter } from "../app";
import { daysUntil, esc, toast, todayIso } from "../dom";
import { hoursHidden } from "../discreet";
import { errorText, lang, t } from "../i18n";
import { ico } from "../icons";
import { byUrgency, dueOf, fmtDay, hoursOf, isOverdue, lateTag, periodOf, priorityBadge, priorityOf } from "../planning";
import { S, effectiveStatus, isClosed, itemByName, metaText, stateDef, stateLabel, titleOf, wf } from "../state";
import { curPeriod, fmtH, periodRange, periodShift, periodStart, timeReady, workedIn, workedTotal } from "../stats";
import type { Item } from "../types";
import { activeAlerts, alertItemCount, alertTitle, dismissAlert, isAlertDismissed, type AlertKind } from "./alerts";
import { statusChip } from "./items";

/** Período (`AAAA-MM`) en que cae una fecha, según el día de inicio del flujo. */
export function periodKey(iso: string): string {
  const [y, m, d] = iso.split("-").map(Number);
  let year = y;
  let month = m;
  if (d < wf().period.startDay) {
    month -= 1;
    if (month === 0) {
      month = 12;
      year -= 1;
    }
  }
  return `${year}-${String(month).padStart(2, "0")}`;
}

const DAY_MS = 864e5;
/** Grupos y sueltos, sin los miembros de un grupo (v1 `!i.group`, app.js:2827). */
const units = () => S.items.filter((it) => !it.group);
const category = (it: Item) => stateDef(effectiveStatus(it))?.category;
const idsOf = (it: Item) => it.ids.join(", ") || it.name.split("/").pop()!;
const notesOf = (it: Item) => metaText(it, wf().fields.notes);

/** Cuándo vence una fecha, relativo a hoy (`n` = días; negativo = vencida). */
const when = (n: number) =>
  n < 0 ? (n === -1 ? t("today.due.yesterday") : t("today.due.ago", { n: -n })) : n === 0 ? t("today.due.today") : n === 1 ? t("today.due.tomorrow") : t("today.due.in", { n });

// ---------- necesita tu atención ----------

/** Alertas que piden acción ahora (las demás quedan en Alertas). Las vencidas van aparte. */
const IMPORTANT: AlertKind[] = ["finalEnv", "gitDesync", "gitDirty", "stale", "plan"];

interface Attention {
  tone: "danger" | "warn";
  icon: "alert" | "clock";
  title: string;
  sub: string;
  /** Atributos del botón de la acción. */
  action: string;
  label: string;
  /** Item y tipo de alerta a desestimar (las sesiones no se desestiman). */
  dismiss?: { name: string; kind: AlertKind };
}

export function attentionList(): Attention[] {
  const out: Attention[] = [];
  for (const s of [...S.sessions, ...S.otherSessions].filter((x) => x.status === "needsYou")) {
    out.push({
      tone: "danger",
      icon: "alert",
      title: t("today.att.session", { name: s.name }),
      sub: s.statusDetail || (s.project ? t("today.att.project", { name: s.project }) : t("today.att.agent")),
      action: `data-action="sess-open" data-id="${esc(s.id)}"`,
      label: t("today.att.open"),
    });
  }
  const late = units()
    .filter((it) => isOverdue(it) && !isAlertDismissed(it, "overdue"))
    .sort((a, b) => dueOf(a).date.localeCompare(dueOf(b).date));
  for (const it of late) {
    const date = dueOf(it).date;
    out.push({
      tone: "warn",
      icon: "clock",
      title: t("today.att.late", { item: `${idsOf(it)} ${titleOf(it)}`, date: fmtDay(date) }),
      // Como la v1 (app.js:2836): el estado y las notas.
      sub: [effectiveStatus(it) ? stateLabel(effectiveStatus(it)) : when(daysUntil(date)), notesOf(it)].filter(Boolean).join(" · "),
      action: `data-action="open-item" data-name="${esc(it.name)}"`,
      label: t("today.att.view"),
      dismiss: { name: it.name, kind: "overdue" },
    });
  }
  for (const a of activeAlerts().filter((x) => IMPORTANT.includes(x.kind))) {
    out.push({
      tone: a.kind === "finalEnv" ? "danger" : "warn",
      icon: "alert",
      title: alertTitle(a.kind),
      sub: `${idsOf(a.item)} ${titleOf(a.item)}${a.detail ? ` · ${a.detail}` : ""}`,
      action: `data-action="open-item" data-name="${esc(a.item.name)}"`,
      label: t("today.att.view"),
      dismiss: { name: a.item.name, kind: a.kind },
    });
  }
  return out;
}

// ---------- próximas tareas ----------

/** Tareas abiertas de lo que está en curso (o planificado) o pactado para el período actual, por urgencia. */
export function todayTasks(): { it: Item; text: string; i: number }[] {
  const k = curPeriod();
  const rel = (it: Item) => {
    const group = it.group ? itemByName(it.group) : undefined;
    const c = category(it);
    return periodOf(it).key === k || (!!group && periodOf(group).key === k) || c === "active" || c === "review" || c === "planned";
  };
  return S.items
    .filter((it) => !isClosed(effectiveStatus(it)) && rel(it))
    .sort(byUrgency)
    .flatMap((it) => it.tasks.flatMap((task, i) => (task.done ? [] : [{ it, text: task.text, i }])));
}

// ---------- render ----------

const bar = (pct: number, cls = "") => `<span class="h-bar ${cls}"><i style="width:${Math.max(0, Math.min(100, pct))}%"></i></span>`;
const kpi = (label: string, value: string, sub: string, cls = "", hours = false) =>
  `<div class="kpi${hours ? " hrs" : ""}"><span class="k-label">${esc(label)}</span><span class="k-val">${esc(value)}</span><span class="k-sub ${cls}">${esc(sub)}</span></div>`;

function periodCard(): string {
  const k = curPeriod();
  const day = (iso: string) => new Date(`${iso}T00:00:00`).getTime();
  const start = day(periodStart(k));
  const end = day(periodStart(periodShift(k, 1)));
  const total = Math.round((end - start) / DAY_MS);
  const left = Math.max(0, Math.round((end - day(todayIso())) / DAY_MS));
  const elapsed = total - left;
  const planned = units().filter((it) => periodOf(it).key === k);
  const withHours = planned.filter((it) => hoursOf(it).hours !== null);
  const plan = withHours.reduce((n, it) => n + (hoursOf(it).hours ?? 0), 0);
  // Como en Horas: lo trabajado en lo pactado (se compara con lo pactado) y todo lo trabajado en el período.
  const report = timeReady();
  const worked = report ? planned.reduce((n, it) => n + workedIn(it, k), 0) : null;
  const workedAll = report ? units().reduce((n, it) => n + workedIn(it, k), 0) : null;
  const closed = planned.filter((it) => isClosed(effectiveStatus(it))).length;
  const workedSub =
    worked === null ? "" : `${plan ? `${t("today.ofPlanned", { pct: Math.round((worked / plan) * 100) })} · ` : ""}${t("today.inPeriodAll", { h: fmtH(workedAll) })}`;
  return `<section class="card today-period">
    <div class="tp-main"><span class="k-label">${esc(t("today.periodTitle", { range: periodRange(k) }))}</span>
      <span class="tp-title">${esc(left > 1 ? t("today.daysLeft", { n: left }) : left === 1 ? t("today.daysLeftOne") : t("today.lastDay"))} · ${esc(t("today.plannedN", { n: planned.length }))}</span>
      ${bar(total ? (elapsed / total) * 100 : 0, "wide")}<span class="muted sm">${esc(t("today.elapsed", { n: elapsed, total }))}</span></div>
    <div class="tp-kpis">
      ${kpi(t("today.kpi.planned"), plan ? fmtH(plan) : "—", t("today.kpi.withHours", { n: withHours.length }), "", true)}
      ${kpi(t("today.kpi.worked"), worked === null ? "…" : fmtH(worked), workedSub, plan && worked !== null && worked > plan ? "bad" : "", true)}
      ${kpi(t("today.kpi.closed"), t("today.kpi.closedN", { n: closed, total: planned.length }), t("today.kpi.closedSub"))}
    </div>
    <button class="link hrs" data-action="nav" data-view="hours">${esc(t("today.seeHours"))}</button>
  </section>`;
}

function attentionCard(): string {
  const att = attentionList();
  const alerts = alertItemCount();
  const rows = att
    .slice(0, 6)
    .map(
      (a, n) => `<li><span class="att-ico ${a.tone}">${ico(a.icon)}</span>
        <span class="grow att-main"><span class="att-t">${esc(a.title)}</span><span class="muted sm">${esc(a.sub)}</span></span>
        <button class="btn" ${a.action}>${esc(a.label)}</button>
        ${a.dismiss ? `<button class="icon-btn" data-action="today-dismiss" data-n="${n}" title="${esc(t("alerts.dismissHint"))}" aria-label="${esc(t("alerts.dismiss"))}">${ico("x")}</button>` : ""}</li>`,
    )
    .join("");
  return `<section class="card"><h2><span class="grow">${esc(t("today.attention"))}${att.length ? ` <span class="nav-count">${att.length}</span>` : ""}</span>
      <button class="link" data-action="nav" data-view="alerts">${esc(alerts ? t("today.allAlertsN", { n: alerts }) : t("today.allAlerts"))}</button></h2>
    ${rows ? `<ul class="list">${rows}</ul>` : `<p class="empty">${ico("check")} ${esc(t("today.att.none"))}</p>`}
    ${att.length > 6 ? `<button class="link" data-action="nav" data-view="alerts">${esc(t("today.att.more", { n: att.length - 6 }))}</button>` : ""}</section>`;
}

function wipCard(): string {
  const report = timeReady();
  const wip = units()
    .filter((it) => ["active", "review"].includes(category(it) ?? ""))
    .sort(byUrgency);
  const rows = wip
    .slice(0, 7)
    .map((it) => {
      const plan = hoursOf(it).hours;
      const real = report ? workedTotal(it) : 0;
      // Como la v1 (app.js:2899): horas contra lo pactado o, sin horas (u ocultas), las notas.
      const notes = notesOf(it);
      const sub =
        plan && !hoursHidden()
          ? `<span class="muted sm hrs">${bar((real / plan) * 100, real > plan ? "over" : "")} ${esc(t("today.hoursOf", { worked: fmtH(real), planned: fmtH(plan) }))}</span>`
          : notes
            ? `<span class="muted sm">${esc(notes)}</span>`
            : "";
      const id = it.kind === "group" ? t("today.group") : it.ids.join(", ") || t("today.noId");
      const due = dueOf(it).date;
      return `<li class="link-row" data-action="open-item" data-name="${esc(it.name)}"><span class="grow att-main"><span class="att-t"><span class="muted">${esc(id)}</span> ${esc(titleOf(it))}${lateTag(it)}</span>${sub}</span>
        ${priorityBadge(priorityOf(it))}<span class="wip-side">${statusChip(effectiveStatus(it))}${due ? `<span class="muted sm">${esc(t("today.dueOn", { date: fmtDay(due) }))}</span>` : ""}</span></li>`;
    })
    .join("");
  return `<section class="card"><h2><span class="grow">${esc(t("today.inProgress"))}</span><button class="link" data-action="nav" data-view="items">${esc(t("today.allItems"))}</button></h2>
    ${rows ? `<ul class="list">${rows}</ul>` : `<p class="empty">${esc(t("today.nothingActive"))}</p>`}</section>`;
}

function datesCard(): string {
  const soon = units()
    .filter((it) => !isClosed(effectiveStatus(it)) && dueOf(it).date && daysUntil(dueOf(it).date) <= 14)
    .sort((a, b) => dueOf(a).date.localeCompare(dueOf(b).date))
    .slice(0, 6);
  const rows = soon
    .map((it) => {
      const { date, src } = dueOf(it);
      const [y, m, d] = date.split("-").map(Number);
      const n = daysUntil(date);
      const month = new Date(y, m - 1, d).toLocaleDateString(lang(), { month: "short" }).replace(".", "");
      const prio = priorityOf(it);
      const extra = [prio !== null ? t("today.due.priority", { n: prio }) : "", src === "local" ? t("today.due.own") : ""].filter(Boolean);
      return `<li class="link-row" data-action="open-item" data-name="${esc(it.name)}"><span class="date-box${n < 0 ? " late" : ""}"><b>${d}</b><span>${esc(month)}</span></span>
        <span class="grow att-main"><span class="att-t">${esc(titleOf(it))}</span><span class="sm ${n < 0 ? "bad" : "muted"}">${esc([when(n), ...extra].join(" · "))}</span></span></li>`;
    })
    .join("");
  return `<section class="card"><h2><span class="grow">${esc(t("today.dates"))}</span><span class="muted sm">${esc(t("today.datesSub"))}</span></h2>
    ${rows ? `<ul class="list">${rows}</ul>` : `<p class="empty">${esc(t("today.noDates"))}</p>`}</section>`;
}

// Lo escrito en el campo de la bandeja sobrevive al refresco de la vista (se rearma con cada cambio en disco).
let inboxDraft = "";
let inboxFocus: { start: number | null; end: number | null } | null = null;

function captureInbox(): void {
  const el = document.querySelector<HTMLInputElement>("#today-inbox");
  inboxFocus = null;
  if (!el) return;
  inboxDraft = el.value;
  if (document.activeElement === el) inboxFocus = { start: el.selectionStart, end: el.selectionEnd };
}

onAfterRender(() => {
  const el = document.querySelector<HTMLInputElement>("#today-inbox");
  if (!el || !inboxFocus) return;
  el.focus();
  el.setSelectionRange(inboxFocus.start, inboxFocus.end);
});

/** Fila de tarea completa, como la de Pendientes sin «Mover» (v1 app.js:2914): tildar, editar y borrar usan las
 *  acciones de Pendientes. */
function taskRow(it: Item, text: string, i: number): string {
  const at = `data-name="${esc(it.name)}" data-index="${i}"`;
  return `<li class="task"><input type="checkbox" data-change="tk-toggle" ${at} aria-label="${esc(text)}">
    <span class="grow" data-action="tk-edit" ${at}>${esc(text)}</span>
    <button class="icon-btn" data-action="tk-edit" ${at} title="${esc(t("common.edit"))}">${ico("edit")}</button>
    <button class="icon-btn" data-action="tk-remove" ${at} title="${esc(t("common.delete"))}">${ico("trash")}</button>
    <button class="link muted sm" data-action="open-item" data-name="${esc(it.name)}" title="${esc(titleOf(it))}">${esc(idsOf(it))}</button></li>`;
}

function tasksCard(): string {
  const rows = todayTasks()
    .slice(0, 7)
    .map(({ it, text, i }) => taskRow(it, text, i))
    .join("");
  return `<section class="card"><h2><span class="grow">${esc(t("today.nextTasks"))}</span><span class="muted sm">${esc(t("today.nextTasksSub"))}</span></h2>
    ${rows ? `<ul class="tasks">${rows}</ul>` : `<p class="empty">${esc(t("today.noTasks"))}</p>`}
    <div class="add-row"><input id="today-inbox" placeholder="${esc(t("today.inboxAdd"))}" value="${esc(inboxDraft)}" data-enter="today-inbox"><button class="btn" data-action="today-inbox">${ico("inbox")}${esc(t("tasks.inbox"))}</button></div></section>`;
}

function renderToday(): string {
  captureInbox();
  const now = new Date();
  const date = now.toLocaleDateString(lang(), { weekday: "long", day: "numeric", month: "long" });
  const h = now.getHours();
  const hello = t(h < 13 ? "today.hello.morning" : h < 20 ? "today.hello.afternoon" : "today.hello.evening");
  return `<div class="page today"><div class="today-head"><span class="muted">${esc(date.charAt(0).toUpperCase() + date.slice(1))}</span><h1>${esc(hello)}</h1></div>
    ${periodCard()}
    <div class="today-grid"><div>${attentionCard()}${tasksCard()}</div><div>${wipCard()}${datesCard()}</div></div>
  </div>`;
}

registerView("today", renderToday);
setNavCounter("today", () => attentionList().length);

onAction("today-inbox", async () => {
  const input = document.querySelector<HTMLInputElement>("#today-inbox")!;
  if (!input.value.trim()) return;
  try {
    S.inbox = await api.inboxAdd(input.value);
  } catch (err) {
    toast(errorText(err), undefined, "error");
    return;
  }
  input.value = "";
  render();
});
onAction("today-dismiss", async (el, ev) => {
  ev.stopPropagation();
  const a = attentionList().slice(0, 6)[Number(el.dataset.n)];
  const it = a?.dismiss && itemByName(a.dismiss.name);
  if (it) await dismissAlert(it, a.dismiss!.kind);
});
