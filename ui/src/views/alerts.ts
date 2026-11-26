// Alertas: items sin README, número sin el prefijo de la plantilla, ids repetidos en varias carpetas, planes
// incompletos, items activos sin movimiento o sin estado, fechas vencidas y avisos de git.
// Desestimar: se guarda cada alerta (`<item>|<tipo>`) con una huella de lo que la disparó, por carpeta de
// trabajos, en la config. Si la huella cambia (otra carpeta duplicada, más actividad…), la alerta vuelve.

import { api } from "../api";
import { onAction, onChange, openDetail, refresh, registerView, render, setAlertCounter } from "../app";
import { confirmDialog, daysUntil, esc, fmtAgo, promptDialog, toast } from "../dom";
import { errorText, t } from "../i18n";
import { ico } from "../icons";
import { S, effectiveStatus, isClosed, matchesSearch, metaText, stateDef, statusOf, titleOf, wf } from "../state";
import { noSearchResults, searchBox } from "../search";
import type { Item } from "../types";
import { gitAlerts, gitDesyncs, gitUncommitted } from "../git";
import { sessionsOf } from "../sessions";

const KINDS = ["noReadme", "noPrefix", "dupes", "plan", "stale", "noStatus", "overdue", "gitDesync", "gitDirty"] as const;
type Kind = (typeof KINDS)[number];
export type AlertKind = Kind;

interface Alert {
  kind: Kind;
  item: Item;
  detail: string;
  /** Carpetas del mismo id (solo `dupes`). */
  names?: string[];
}

const DAY = 86400;
/** Días sin cambios para avisar de un item activo, y días de antigüedad para avisar de uno sin estado. */
const STALE_DAYS = 14;
const NO_STATUS_DAYS = 30;

// ---------- desestimadas ----------

let dismissed: Record<string, string> = {};
let dismissedRoot: string | null = null;
let showDismissed = false;

/** Carga las desestimadas de la carpeta abierta (una vez por carpeta) y vuelve a dibujar. */
function ensureDismissed(): void {
  if (!S.root || dismissedRoot === S.root) return;
  const root = S.root;
  dismissedRoot = root;
  dismissed = {};
  api
    .alertsDismissed()
    .then((map) => {
      if (S.root !== root) return;
      dismissed = map;
      render();
    })
    .catch(() => {
      dismissedRoot = null;
    });
}

const keyOf = (a: Alert) => `${a.item.name}|${a.kind}`;

/** Lo que disparó la alerta: si cambia, una alerta desestimada vuelve. */
function fingerprint(a: Alert): string {
  const it = a.item;
  switch (a.kind) {
    case "dupes":
      return (a.names ?? []).join(",");
    case "plan":
      return it.planGaps.join(",");
    case "stale":
      return String(Math.floor(it.last));
    case "overdue":
      return metaText(it, wf().fields.due);
    case "gitDesync":
      return JSON.stringify(gitDesyncs(it));
    case "gitDirty":
      return JSON.stringify(gitUncommitted(it).map((d) => [d.repo, d.branch, d.count]));
    default:
      return "";
  }
}

const isDismissed = (a: Alert) => dismissed[keyOf(a)] === fingerprint(a);

// ---------- cálculo ----------

/** Prefijo fijo de la plantilla de carpeta antes del id (`wi` en `wi{id}-{slug}`); vacío si no tiene. */
function idPrefix(): string {
  const tpl = wf().folders.newItemTemplate ?? "";
  const at = tpl.indexOf("{id}");
  return at > 0 ? tpl.slice(0, at) : "";
}

function allAlerts(): Alert[] {
  const out: Alert[] = [];
  const seen = new Set<string>();
  const now = Date.now() / 1000;
  const prefix = idPrefix().toLowerCase();
  for (const it of S.items) {
    const status = effectiveStatus(it);
    const base = it.name.split("/").pop()!;
    if (it.kind === "item" && !it.hasReadme) out.push({ kind: "noReadme", item: it, detail: "" });
    // El id salió del nombre de la carpeta pero falta el prefijo de la plantilla (`123456-x` en vez de `wi123456-x`).
    if (it.kind === "item" && prefix && it.ids.some((id) => base.startsWith(id)) && !base.toLowerCase().startsWith(prefix))
      out.push({ kind: "noPrefix", item: it, detail: `${idPrefix()}${base}` });
    // Un id repetido cuenta una vez; dentro de un mismo grupo no es duplicado.
    const dupes = (it.dupes ?? []).filter((d) => d.split("/")[0] !== it.name.split("/")[0]);
    if (dupes.length) {
      const names = [it.name, ...dupes].sort();
      const key = names.join("|");
      if (!seen.has(key)) {
        seen.add(key);
        out.push({ kind: "dupes", item: it, detail: dupes.join(", "), names });
      }
    }
    // Plan a medias después de una sesión de planificación que ya no está trabajando.
    if (it.planGaps.length) {
      const plans = sessionsOf(it.name).filter((s) => s.plan);
      if (plans.length && !plans.some((s) => s.status === "starting" || s.status === "working"))
        out.push({ kind: "plan", item: it, detail: it.planGaps.map((g) => t(`detail.gap.${g}`)).join(" · ") });
    }
    const category = stateDef(status)?.category;
    if (it.kind === "item" && (category === "active" || category === "review") && now - it.last > STALE_DAYS * DAY)
      out.push({ kind: "stale", item: it, detail: fmtAgo(it.last) });
    if (it.kind === "item" && it.hasReadme && !statusOf(it) && now - it.last < NO_STATUS_DAYS * DAY)
      out.push({ kind: "noStatus", item: it, detail: fmtAgo(it.last) });
    const due = metaText(it, wf().fields.due);
    if (due && daysUntil(due) < 0 && !isClosed(status)) out.push({ kind: "overdue", item: it, detail: due });
  }
  out.push(...gitAlerts());
  // Varias alertas de git del mismo item y tipo son una sola (se desestiman juntas).
  const byKey = new Map<string, Alert>();
  for (const a of out) {
    const prev = byKey.get(keyOf(a));
    if (prev) prev.detail = `${prev.detail} · ${a.detail}`;
    else byKey.set(keyOf(a), { ...a });
  }
  return [...byKey.values()];
}

/** Alertas vigentes (sin las desestimadas): lo que cuenta el badge. */
export function activeAlerts(): Alert[] {
  ensureDismissed();
  return allAlerts().filter((a) => !isDismissed(a));
}

// ---------- vista ----------

function fixButton(a: Alert): string {
  const name = esc(a.item.name);
  if (a.kind === "noReadme")
    return `<button class="btn" data-action="create-readme" data-name="${name}">${ico("filePlus")}${esc(t("alerts.createReadme"))}</button>`;
  if (a.kind === "noPrefix")
    return `<button class="btn" data-action="alert-rename" data-name="${name}" data-to="${esc(a.detail)}">${ico("edit")}${esc(t("alerts.rename"))}</button>`;
  // Agrupar solo carpetas sueltas: dentro de un grupo no cuentan como duplicado.
  if (a.kind === "dupes" && !a.names!.some((n) => n.includes("/")))
    return `<button class="btn" data-action="alert-group" data-names="${esc(a.names!.join("|"))}" title="${esc(t("alerts.groupHint"))}">${ico("folderIn")}${esc(t("alerts.group"))}</button>`;
  return "";
}

function row(a: Alert, off: boolean): string {
  const name = esc(a.item.name);
  const toggle = off
    ? `<button class="btn" data-action="alert-restore" data-key="${esc(keyOf(a))}">${ico("undo")}${esc(t("alerts.restore"))}</button>`
    : `<button class="icon-btn" data-action="alert-dismiss" data-key="${esc(keyOf(a))}" title="${esc(t("alerts.dismissHint"))}" aria-label="${esc(t("alerts.dismiss"))}">${ico("x")}</button>`;
  const detail = a.kind === "noPrefix" ? t("alerts.renameTo", { name: a.detail }) : a.detail;
  return `<li class="link-row" data-action="open-item" data-name="${name}"><span class="grow"><b>${esc(titleOf(a.item))}</b> <span class="muted">${name}</span></span><span class="muted">${esc(detail)}</span>${off ? "" : fixButton(a)}${toggle}</li>`;
}

function blocks(list: Alert[], off: boolean): string {
  return KINDS.map((k) => {
    const of = list.filter((a) => a.kind === k);
    if (!of.length) return "";
    const all = off
      ? `<button class="btn ghost" data-action="alert-restore-all" data-kind="${k}">${esc(t("alerts.restoreAll"))}</button>`
      : `<button class="btn ghost" data-action="alert-dismiss-all" data-kind="${k}">${esc(t("alerts.dismissAll"))}</button>`;
    return `<section class="card"><h2>${ico(off ? "info" : "alert")}<span class="grow">${esc(t(`alerts.${k}`))} <span class="muted">${of.length}</span></span>${all}</h2>
      ${off ? "" : `<p class="muted">${esc(t(`alerts.why.${k}`))}</p>`}<ul class="list">${of.map((a) => row(a, off)).join("")}</ul></section>`;
  }).join("");
}

function renderAlerts(): string {
  ensureDismissed();
  // La búsqueda global filtra la lista (no el contador del menú).
  const list = allAlerts().filter((a) => (a.names ?? [a.item.name]).some((n) => matchesSearch(S.items.find((x) => x.name === n) ?? a.item)));
  const on = list.filter((a) => !isDismissed(a));
  const off = list.filter(isDismissed);
  const toggle = off.length
    ? `<label class="check"><input type="checkbox" data-change="alerts-show-dismissed" ${showDismissed ? "checked" : ""}>${esc(t("alerts.showDismissed", { n: off.length }))}</label>`
    : "";
  const dismissedBlock = showDismissed && off.length ? `<h2 class="muted">${esc(t("alerts.dismissedTitle"))}</h2>${blocks(off, true)}` : "";
  return `<div class="page"><div class="toolbar"><h1>${esc(t("nav.alerts"))}</h1>${searchBox()}<span class="spacer"></span>${toggle}</div>
    ${blocks(on, false) || `<p class="empty">${noSearchResults() || esc(t("alerts.none"))}</p>`}${dismissedBlock}</div>`;
}

registerView("alerts", renderAlerts);
setAlertCounter(() => activeAlerts().length);

async function setDismissed(alerts: Alert[], on: boolean): Promise<void> {
  if (!alerts.length) return;
  try {
    dismissed = await api.alertsDismiss(alerts.map((a) => ({ key: keyOf(a), fp: on ? fingerprint(a) : "" })), on);
  } catch (err) {
    toast(errorText(err), undefined, "error");
  }
  render();
}

const findAlert = (key: string) => allAlerts().find((a) => keyOf(a) === key);

/** ¿Está desestimada la alerta `kind` del item? Hoy la usa también para vencidas que solo tienen la fecha del
 *  tracker (no figuran en Alertas: la huella queda vacía). */
export function isAlertDismissed(item: Item, kind: Kind): boolean {
  ensureDismissed();
  return isDismissed({ kind, item, detail: "" });
}

/** Desestima la alerta `kind` del item (desde Hoy, «Necesita tu atención»). */
export async function dismissAlert(item: Item, kind: Kind): Promise<void> {
  await setDismissed([findAlert(`${item.name}|${kind}`) ?? { kind, item, detail: "" }], true);
}

onAction("alert-dismiss", async (el, ev) => {
  ev.stopPropagation();
  const a = findAlert(el.dataset.key!);
  if (a) await setDismissed([a], true);
});
onAction("alert-restore", async (el, ev) => {
  ev.stopPropagation();
  const a = findAlert(el.dataset.key!);
  if (a) await setDismissed([a], false);
});
onAction("alert-dismiss-all", async (el) => {
  const kind = el.dataset.kind!;
  const of = allAlerts().filter((a) => a.kind === kind && !isDismissed(a));
  if (!(await confirmDialog(t("alerts.confirmDismissAll", { n: of.length, kind: t(`alerts.${kind}`) }), t("alerts.dismiss")))) return;
  await setDismissed(of, true);
});
onAction("alert-restore-all", async (el) => {
  await setDismissed(allAlerts().filter((a) => a.kind === el.dataset.kind && isDismissed(a)), false);
});
onChange("alerts-show-dismissed", (el) => {
  showDismissed = (el as HTMLInputElement).checked;
  render();
});

onAction("create-readme", async (el, ev) => {
  ev.stopPropagation();
  // Escribir un campo crea el README (`# carpeta` + frontmatter) por la escritura única.
  const first = wf().states.find((s) => s.category === "backlog") ?? wf().states[0];
  await api.updateFields(el.dataset.name!, { [wf().fields.status]: first.id });
  await refresh();
  await openDetail(el.dataset.name!);
});
onAction("alert-rename", async (el, ev) => {
  ev.stopPropagation();
  const name = el.dataset.name!;
  const next = await promptDialog(t("alerts.renamePrompt"), el.dataset.to!);
  if (!next || next === name.split("/").pop()) return;
  try {
    await api.renameFolder(name, next);
  } catch (err) {
    toast(errorText(err), undefined, "error");
  }
  await refresh();
});
onAction("alert-group", async (el, ev) => {
  ev.stopPropagation();
  const names = el.dataset.names!.split("|");
  const first = S.items.find((i) => i.name === names[0]);
  const title = await promptDialog(t("alerts.groupPrompt"), first ? titleOf(first) : "");
  if (!title) return;
  try {
    await api.createGroup(title, names);
    toast(t("alerts.grouped", { n: names.length }));
  } catch (err) {
    toast(errorText(err), undefined, "error");
  }
  await refresh();
});
