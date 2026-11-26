// Alertas (como la v1): ids repetidos en varias carpetas, scripts que escriben sin correr en el último ambiente,
// items activos sin movimiento o sin estado, número sin el formato de la plantilla, sin README, avisos de git,
// planes incompletos y (de la v2) fechas vencidas. Son de cada item: un grupo no tiene alertas propias.
// Desestimar: se guarda cada alerta (`<item>|<tipo>`) con una huella de lo que la disparó, por carpeta de
// trabajos, en la config. Si la huella cambia (otra carpeta duplicada, más actividad…), la alerta vuelve.

import { api } from "../api";
import { onAction, onChange, refresh, registerView, render, setAlertCounter } from "../app";
import { confirmDialog, daysUntil, esc, fmtAgo, promptDialog, toast } from "../dom";
import { errorText, t } from "../i18n";
import { ico } from "../icons";
import { S, effectiveStatus, envLabel, isClosed, matchesSearch, metaList, metaText, stateDef, statusOf, titleOf, typeLabel, wf } from "../state";
import { noSearchResults, searchBox } from "../search";
import type { Item } from "../types";
import { gitAlerts, gitDesyncs, gitUncommitted } from "../git";
import { sessionsOf } from "../sessions";
import { statusChip } from "./items";

// En el orden de la v1 (app.js:137); `overdue` es de la v2.
const KINDS = ["dupes", "finalEnv", "stale", "noStatus", "noPrefix", "noReadme", "gitDesync", "gitDirty", "plan", "overdue"] as const;
type Kind = (typeof KINDS)[number];
export type AlertKind = Kind;
/** Tipos que no suman en el contador del menú (la v1 no cuenta «sin prefijo» ni «sin README»). */
const NO_COUNT: readonly Kind[] = ["noPrefix", "noReadme", "overdue"];

interface Alert {
  kind: Kind;
  item: Item;
  detail: string;
  /** Carpetas del mismo id, ordenadas, incluida la del item (solo `dupes`): el conjunto agrupa la vista. */
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
const clusterKey = (a: Alert) => (a.names ?? []).join("|");

/** Lo que disparó la alerta: si cambia, una alerta desestimada vuelve (v1 app.js:171). */
function fingerprint(a: Alert): string {
  const it = a.item;
  switch (a.kind) {
    case "dupes":
      return (a.names ?? []).filter((n) => n !== it.name).join(",");
    case "finalEnv":
      return finalEnvFiles(it).join(",");
    case "plan":
      return it.planGaps.join(",");
    case "stale":
      return String(Math.floor(it.last));
    case "overdue":
      return metaText(it, wf().fields.due);
    case "gitDesync":
      return JSON.stringify(gitDesyncs(it));
    case "gitDirty":
      return JSON.stringify(gitUncommitted(it));
    default:
      return "";
  }
}

const isDismissed = (a: Alert) => dismissed[keyOf(a)] === fingerprint(a);

// ---------- reglas del flujo ----------

/** Prefijo fijo de la plantilla de carpeta antes del id (`wi` en `wi{id}-{slug}`); vacío si no tiene. */
function idPrefix(): string {
  const tpl = wf().folders.newItemTemplate ?? "";
  const at = tpl.indexOf("{id}");
  return at > 0 ? tpl.slice(0, at) : "";
}

/** El grupo `id` de `folders.itemPattern` anclado al principio (`^\d{6}` en classic-es). La v1 avisa de una carpeta
 *  sin id que empieza con un número así (`1234567-x`, `123456_x`): no tiene el formato de la plantilla. */
let idStartCache: [string, RegExp | null] | null = null;
export function idStart(pattern: string): RegExp | null {
  if (idStartCache?.[0] === pattern) return idStartCache[1];
  let re: RegExp | null = null;
  const at = pattern.indexOf("(?P<id>");
  let depth = 0;
  for (let i = at; at >= 0 && i < pattern.length; i++) {
    if (pattern[i] === "\\") i++;
    else if (pattern[i] === "(") depth++;
    else if (pattern[i] === ")" && --depth === 0) {
      try {
        re = new RegExp(`^(?:${pattern.slice(at + "(?P<id>".length, i)})`, pattern.startsWith("(?i)") ? "i" : "");
      } catch {
        re = null;
      }
      break;
    }
  }
  idStartCache = [pattern, re];
  return re;
}

/** Archivos del item de los tipos que tienen que correrse en el último ambiente (`alerts.finalEnvKinds`). */
function finalEnvFiles(it: Item): string[] {
  const kinds = wf().alerts?.finalEnvKinds ?? [];
  return it.files
    .filter((f) => f.kind && kinds.includes(f.kind))
    .map((f) => f.name)
    .sort();
}

/** Estados vigilados por «sin movimiento»: los del preset o, si no dice, los de categoría active/review. */
function isStaleState(status: string): boolean {
  const list = wf().alerts?.staleStates ?? [];
  if (list.length) return list.includes(status);
  const category = stateDef(status)?.category;
  return category === "active" || category === "review";
}

/** Último ambiente del flujo (PROD en classic-es). */
const finalEnv = () => wf().environments[wf().environments.length - 1]?.id ?? "";

/** Título de un tipo de alerta. */
export function alertTitle(kind: Kind): string {
  if (kind === "finalEnv") return t("alerts.finalEnv", { kinds: (wf().alerts?.finalEnvKinds ?? []).join(" / "), env: envLabel(finalEnv()) });
  return t(`alerts.${kind}`);
}

// ---------- cálculo ----------

function allAlerts(): Alert[] {
  const out: Alert[] = [];
  const now = Date.now() / 1000;
  const startsLikeId = idStart(wf().folders.itemPattern);
  const kinds = wf().alerts?.finalEnvKinds ?? [];
  const final = finalEnv();
  for (const it of S.items) {
    if (it.kind !== "item") continue; // las alertas son de cada item (v1 app.js:149)
    const status = effectiveStatus(it);
    const base = it.name.split("/").pop()!;
    // Mismo id en varias carpetas; las del mismo grupo son partes de un trabajo, no un duplicado.
    const dupes = it.dupes ?? [];
    if (dupes.length && !(it.group && dupes.every((d) => d.startsWith(`${it.group}/`))))
      out.push({ kind: "dupes", item: it, detail: dupes.join(", "), names: [it.name, ...dupes].sort() });
    // Scripts que escriben datos y todavía no figuran en el último ambiente (v1: MIGRATION / FIX sin PROD).
    if (status && !isClosed(status) && final && it.kinds.some((k) => kinds.includes(k)) && !metaList(it, wf().fields.environments).includes(final))
      out.push({ kind: "finalEnv", item: it, detail: finalEnvFiles(it).join(", ") });
    if (status && isStaleState(status) && now - it.last > STALE_DAYS * DAY) out.push({ kind: "stale", item: it, detail: "" });
    if (!statusOf(it) && now - it.last < NO_STATUS_DAYS * DAY) out.push({ kind: "noStatus", item: it, detail: "" });
    if (!it.ids.length && startsLikeId?.test(it.name)) out.push({ kind: "noPrefix", item: it, detail: `${idPrefix()}${base}` });
    if (!it.hasReadme) out.push({ kind: "noReadme", item: it, detail: "" });
    // Plan a medias después de una sesión de planificación que ya no está trabajando.
    if (it.planGaps.length) {
      const plans = sessionsOf(it.name).filter((s) => s.plan);
      if (plans.length && !plans.some((s) => s.status === "starting" || s.status === "working"))
        out.push({ kind: "plan", item: it, detail: it.planGaps.map((g) => t(`detail.gap.${g}`)).join(" · ") });
    }
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

/** Alertas vigentes (sin las desestimadas). */
export function activeAlerts(): Alert[] {
  ensureDismissed();
  return allAlerts().filter((a) => !isDismissed(a));
}

/** Contador del menú (v1 app.js:275): items con alguna alerta que cuenta, más cada conjunto de duplicados. */
export function alertCount(): number {
  const keys = new Set<string>();
  for (const a of activeAlerts()) {
    if (NO_COUNT.includes(a.kind)) continue;
    keys.add(a.kind === "dupes" ? `dupes:${clusterKey(a)}` : a.item.name);
  }
  return keys.size;
}

/** «Todas las alertas (N)» de Hoy (v1 app.js:2860): items con alguna alerta que cuenta. */
export function alertItemCount(): number {
  return new Set(activeAlerts().filter((a) => !NO_COUNT.includes(a.kind)).map((a) => a.item.name)).size;
}

// ---------- vista ----------

function fixButton(a: Alert): string {
  const name = esc(a.item.name);
  if (a.kind === "noReadme")
    return `<button class="btn" data-action="create-readme" data-name="${name}">${ico("filePlus")}${esc(t("alerts.createReadme"))}</button>`;
  if (a.kind === "noPrefix")
    return `<button class="btn" data-action="alert-rename" data-name="${name}" data-to="${esc(a.detail)}">${ico("edit")}${esc(t("alerts.rename"))}</button>`;
  return "";
}

/** Fila como la v1 (app.js:807): estado, tipo y #id, título y carpeta, hace cuánto (+ el detalle de la v2). */
function row(a: Alert, off: boolean): string {
  const it = a.item;
  const name = esc(it.name);
  const toggle = off
    ? `<button class="btn" data-action="alert-restore" data-key="${esc(keyOf(a))}">${ico("undo")}${esc(t("alerts.restore"))}</button>`
    : `<button class="icon-btn" data-action="alert-dismiss" data-key="${esc(keyOf(a))}" title="${esc(t("alerts.dismissHint"))}" aria-label="${esc(t("alerts.dismiss"))}">${ico("x")}</button>`;
  const detail = a.kind === "noPrefix" ? t("alerts.renameTo", { name: a.detail }) : a.detail;
  const ids = it.ids.map((id) => `#${id}`).join(" ");
  return `<li class="link-row alert-row" data-action="open-item" data-name="${name}"><span class="ar-state">${statusChip(effectiveStatus(it))}</span>
    <span class="ar-id muted sm">${esc(it.type ? typeLabel(it.type) : "")} ${esc(ids)}</span>
    <span class="grow ar-name"><b>${esc(titleOf(it))}</b><span class="muted sm">${name}${detail ? ` · ${esc(detail)}` : ""}</span></span>
    <span class="muted sm">${esc(fmtAgo(it.last))}</span>${off ? "" : fixButton(a)}${toggle}</li>`;
}

/** Duplicados como conjuntos (v1 app.js:816): ids en común, cada carpeta con sus archivos, y acciones. */
function dupBlock(list: Alert[], off: boolean): string {
  const clusters = new Map<string, Alert[]>();
  for (const a of list) {
    const k = clusterKey(a);
    if (!clusters.has(k)) clusters.set(k, []);
    clusters.get(k)!.push(a);
  }
  return [...clusters.values()]
    .map((alerts) => {
      const names = alerts[0].names!;
      const items = names.map((n) => S.items.find((i) => i.name === n)).filter(Boolean) as Item[];
      const common = [...new Set(items.flatMap((i) => i.ids))].filter((id) => items.every((i) => i.ids.includes(id)));
      const keys = esc(alerts.map(keyOf).join("\n"));
      const head = off
        ? `<button class="btn" data-action="alert-restore-set" data-keys="${keys}">${ico("undo")}${esc(t("alerts.restore"))}</button>`
        : `${names.some((n) => n.includes("/")) ? "" : `<button class="btn" data-action="alert-group" data-names="${esc(names.join("|"))}" title="${esc(t("alerts.groupHint"))}">${ico("folderIn")}${esc(t("alerts.group"))}</button>`}
          <button class="btn" data-action="alert-dismiss-set" data-keys="${keys}" title="${esc(t("alerts.intentionalHint"))}">${esc(t("alerts.intentional"))}</button>`;
      const rows = items
        .map(
          (it) => `<li class="link-row alert-row" data-action="open-item" data-name="${esc(it.name)}"><span class="ar-state">${statusChip(effectiveStatus(it))}</span>
          <span class="grow ar-name"><span>${esc(it.name)}</span><span class="muted sm">${esc(it.title)}</span></span>
          <span class="muted sm">${esc(t("alerts.filesN", { n: it.files.length }))}</span><span class="muted sm">${esc(fmtAgo(it.last))}</span>
          ${off ? "" : `<button class="btn" data-action="alert-trash" data-name="${esc(it.name)}" title="${esc(t("alerts.trashHint"))}">${ico("trash")}${esc(t("alerts.trash"))}</button>`}</li>`,
        )
        .join("");
      return `<div class="dup-cluster"><div class="dc-head"><b>${esc(common.length ? common.map((id) => `#${id}`).join(" ") : t("alerts.dupNoId"))}</b>
        <span class="muted grow">${esc(t("alerts.foldersN", { n: items.length }))}</span>${head}</div><ul class="list">${rows}</ul></div>`;
    })
    .join("");
}

function blocks(list: Alert[], off: boolean): string {
  return KINDS.map((k) => {
    const of = list.filter((a) => a.kind === k).sort((x, y) => y.item.last - x.item.last);
    if (!of.length) return "";
    const n = k === "dupes" ? new Set(of.map(clusterKey)).size : of.length;
    const counts = !NO_COUNT.includes(k);
    const all = off
      ? `<button class="btn ghost" data-action="alert-restore-all" data-kind="${k}">${esc(t("alerts.restoreAll"))}</button>`
      : `<button class="btn ghost" data-action="alert-dismiss-all" data-kind="${k}">${esc(t("alerts.dismissAll"))}</button>`;
    const body = k === "dupes" ? dupBlock(of, off) : `<ul class="list">${of.map((a) => row(a, off)).join("")}</ul>`;
    return `<section class="card"><h2><span class="${counts && !off ? "alert-ico on" : "alert-ico"}">${ico(counts ? "alert" : "info")}</span><span class="grow">${esc(alertTitle(k))} <span class="muted">${n}</span></span>${all}</h2>
      ${off ? "" : `<p class="muted">${esc(t(`alerts.why.${k}`, { env: envLabel(finalEnv()) }))}</p>`}${body}</section>`;
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
setAlertCounter(alertCount);

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

const alertsOfKeys = (el: HTMLElement) => {
  const keys = el.dataset.keys!.split("\n");
  return allAlerts().filter((a) => keys.includes(keyOf(a)));
};

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
// Duplicados: «Es intencional» desestima el conjunto; vuelve si aparece otra carpeta con el mismo id.
onAction("alert-dismiss-set", async (el, ev) => {
  ev.stopPropagation();
  await setDismissed(alertsOfKeys(el), true);
});
onAction("alert-restore-set", async (el, ev) => {
  ev.stopPropagation();
  await setDismissed(alertsOfKeys(el), false);
});
onAction("alert-dismiss-all", async (el) => {
  const kind = el.dataset.kind as Kind;
  const of = allAlerts().filter((a) => a.kind === kind && !isDismissed(a));
  if (!(await confirmDialog(t("alerts.confirmDismissAll", { n: of.length, kind: alertTitle(kind) }), t("alerts.dismiss")))) return;
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
  // Como la v1 (app.js:3373): la escritura única sin campos crea solo `# carpeta`, sin estado y sin abrir el detalle.
  try {
    await api.updateFields(el.dataset.name!, {});
    toast(t("alerts.readmeCreated"));
  } catch (err) {
    toast(errorText(err), undefined, "error");
  }
  await refresh();
});
// Duplicados: mover una carpeta a la de eliminados, con Deshacer (v1 app.js:825).
onAction("alert-trash", async (el, ev) => {
  ev.stopPropagation();
  const name = el.dataset.name!;
  if (!(await confirmDialog(t("items.confirmDelete", { n: 1 }), t("items.delete")))) return;
  let deleted: string;
  try {
    deleted = await api.trash(name);
  } catch (err) {
    toast(errorText(err), undefined, "error");
    return;
  }
  await refresh();
  toast(t("items.deletedN", { n: 1 }), {
    label: t("common.undo"),
    run: async () => {
      await api.untrash(deleted, name);
      await refresh();
    },
  });
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
