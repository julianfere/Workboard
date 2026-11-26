// Trackers (Azure DevOps): conexión y mapeo en Ajustes, sincronización con previsualización, asignados del
// período, datos del tracker en el detalle, cambiar el estado en el tracker y «Nuevo item» con id.
// Todo lee salvo «Cambiar estado en el tracker», que escribe con la credencial de escritura.

import { trackerApi } from "../api";
import { goView, onAction, onChange, refresh, render } from "../app";
import { $, confirmDialog, esc, fmtDate, toast } from "../dom";
import { errorText, label, lang, t } from "../i18n";
import { ico } from "../icons";
import { S, envLabel, itemByName, stateDef, stateLabel, statusOf, titleOf, typeLabel, wf } from "../state";
import type { ApiError, AssignedList, Item, NewItemResult, SyncPreview, SyncRow, TrackerConfig, TrackerItem, TrackerWorkItem } from "../types";

// ---------- estado ----------

export const TR = {
  cfg: null as TrackerConfig | null,
  /** Raíz para la que se cargó la config ("" = sin cargar). */
  loadedFor: "",
  loading: false,
  /** Mapeo en edición en Ajustes (se guarda con el botón). */
  draftMap: null as Record<string, string> | null,
  /** Estados que aparecieron al volver a traerlos. */
  newStates: new Set<string>(),
};

/** Relee la config (y la caché) del tracker de la carpeta abierta. */
export async function loadTracker(): Promise<void> {
  if (!S.root) return;
  TR.loading = true;
  try {
    TR.cfg = await trackerApi.config();
    TR.loadedFor = S.root;
  } catch {
    TR.cfg = null;
  } finally {
    TR.loading = false;
  }
}

/** Carga la config la primera vez que una vista la necesita y vuelve a renderizar. */
function ensureLoaded(): TrackerConfig | null {
  if (S.root && TR.loadedFor !== S.root && !TR.loading) {
    TR.loadedFor = S.root;
    TR.cfg = null;
    TR.draftMap = null;
    loadTracker().then(render);
  }
  return TR.loadedFor === S.root ? TR.cfg : null;
}

export const trackerConnected = () => !!ensureLoaded()?.connected;

const trackerState = (name: string) => TR.cfg?.states.find((s) => s.name === name);
const trackerDot = (name: string) => `<span class="dot ${trackerState(name)?.suggested ?? "unknown"}"></span>`;
const trackerItems = (it: Item): [string, TrackerWorkItem][] =>
  it.ids.map((id): [string, TrackerWorkItem | undefined] => [id, TR.cfg?.cache?.items[id]]).filter((x): x is [string, TrackerWorkItem] => !!x[1]);
/** Horas estimadas en el tracker de unos ids (0 = sin cargar). Cada id cuenta una vez. */
export function trackerEstimate(ids: string[]): number | null {
  const cfg = ensureLoaded();
  const hs = [...new Set(ids)].map((id) => cfg?.cache?.items[id]?.estimate ?? 0).filter((h) => h > 0);
  return hs.length ? hs.reduce((a, b) => a + b, 0) : null;
}

/** La fecha pactada más temprana en el tracker de unos ids, como `AAAA-MM-DD` local ("" = ninguna). */
export function trackerDue(ids: string[]): string {
  const cfg = ensureLoaded();
  const days = ids
    .map((id) => new Date(cfg?.cache?.items[id]?.due || "x"))
    .filter((d) => !isNaN(d.getTime()))
    .map((d) => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`)
    .sort();
  return days[0] ?? "";
}

const fmtIso = (iso: string) => {
  const d = new Date(iso);
  return isNaN(d.getTime()) ? "" : d.toLocaleDateString(lang(), { year: "numeric", month: "short", day: "numeric" });
};
const fmtHours = (h: number | null) => (h === null || h === undefined ? "" : `${Math.round(h * 10) / 10} h`);
const failures = (errors: { error: ApiError; id?: string; name?: string }[]) =>
  errors.map((e) => `${e.id ? `#${e.id}` : e.name}: ${errorText(e.error)}`).join(" · ");

// ---------- modal propio (tablas anchas, se re-renderiza en el lugar) ----------

let modal: { back: HTMLElement; view: () => string } | null = null;

function openModal(view: () => string): void {
  closeModal();
  const back = document.createElement("div");
  back.className = "modal-back";
  back.addEventListener("keydown", (e) => e.key === "Escape" && closeModal());
  // Con `addEventListener`: un `onclick` que devuelve `false` cancelaría los clicks de los checkboxes.
  back.addEventListener("click", (e) => {
    if (e.target === back) closeModal();
  });
  document.body.append(back);
  modal = { back, view };
  rerender();
  back.querySelector<HTMLElement>("input, select, button")?.focus();
}

function rerender(): void {
  if (!modal) return;
  const keep = modal.back.querySelector(".trk-body")?.scrollTop ?? 0;
  modal.back.innerHTML = `<div class="modal trk-modal" role="dialog" aria-modal="true">${modal.view()}</div>`;
  const body = modal.back.querySelector(".trk-body");
  if (body) body.scrollTop = keep;
}

function closeModal(): void {
  modal?.back.remove();
  modal = null;
}

function busy(el: HTMLElement, text: string): () => void {
  const btn = el as HTMLButtonElement;
  const html = btn.innerHTML;
  btn.disabled = true;
  btn.textContent = text;
  return () => {
    btn.disabled = false;
    btn.innerHTML = html;
  };
}

// ---------- grilla y detalle ----------

/** Estado en el tracker, al lado del título en la grilla. */
export function trackerBadge(it: Item): string {
  if (!trackerConnected()) return "";
  const list = trackerItems(it);
  if (!list.length) return "";
  const [, w] = list[0];
  return `<span class="chip trk-chip" title="${esc(`${TR.cfg!.source} · ${w.type} · ${w.state}\n${w.title}`)}">${trackerDot(w.state)}${esc(w.state)}</span>`;
}

/** Botones de la barra de la grilla: asignados del período (con conexión) y sincronizar, siempre (sin
 * configurar abre el asistente, como «Sincronizar Azure» de la v1). */
export function trackerToolbar(): string {
  if (!S.root) return "";
  const cfg = ensureLoaded();
  const source = cfg?.source ?? "Azure DevOps";
  return `${cfg?.connected ? `<button class="btn" data-action="trk-assigned" title="${esc(t("tracker.assigned.title"))}">${ico("calendar")}${esc(t("tracker.assigned.short"))}</button>` : ""}
    <button class="btn" data-action="trk-sync" title="${esc(t("tracker.sync.hint", { source }))}">${ico("cloud")}${esc(t("tracker.sync.button"))}</button>`;
}

/** Acción en lote: cambiar el estado en el tracker de los seleccionados. */
export function trackerBulk(names: string[]): string {
  if (!trackerConnected()) return "";
  return `<button class="btn" data-action="trk-push" data-names="${esc(JSON.stringify(names))}">${ico("cloud")}${esc(t("tracker.push.button", { source: TR.cfg!.source }))}</button>`;
}

/** Tarjeta del detalle con lo último leído del tracker. */
export function trackerCard(it: Item): string {
  const cfg = ensureLoaded();
  if (!cfg?.connected || !it.ids.length || it.kind !== "item") return "";
  const rows = it.ids
    .map((id) => {
      const w = cfg.cache?.items[id];
      const open = `<button class="icon-btn" data-action="trk-open" data-id="${esc(id)}" title="${esc(t("tracker.open", { source: cfg.source }))}">${ico("external")}</button>`;
      if (!w) {
        const requested = cfg.cache?.requested.includes(id);
        return `<li><span class="grow">#${esc(id)} <span class="muted">${esc(t(requested ? "tracker.notFound" : "tracker.notSynced"))}</span></span>${open}</li>`;
      }
      // Las horas llevan `hrs` (el modo discreto las oculta).
      const hrs = (text: string) => `<span class="hrs">${esc(text)}</span>`;
      const facts = [
        w.type && esc(w.type),
        w.assigned && esc(t("tracker.assignedTo", { name: w.assigned })),
        w.priority !== null && esc(t("tracker.priority", { n: w.priority })),
        w.due && esc(t("tracker.due", { date: fmtIso(w.due) })),
        w.estimate !== null && hrs(t("tracker.estimate", { hours: fmtHours(w.estimate) })),
        w.completed !== null && hrs(t("tracker.completed", { hours: fmtHours(w.completed) })),
        w.changed && esc(t("tracker.changed", { date: fmtIso(w.changed) })),
        // A qué estado del flujo mapea su estado (v1 app.js:1149).
        esc(t("tracker.mapsTo", { state: cfg.map[w.state] ? stateLabel(cfg.map[w.state]) : "—" })),
      ].filter((f): f is string => !!f);
      return `<li><span class="grow"><span class="chip">${trackerDot(w.state)}${esc(w.state)}</span> #${esc(id)} ${esc(w.title)}
        <div class="muted tr-facts">${facts.map((f) => (f.startsWith("<span") ? f : `<span>${f}</span>`)).join("")}</div></span>${open}</li>`;
    })
    .join("");
  const synced = cfg.cache?.syncedAt ? t("tracker.syncedAt", { date: fmtDate(cfg.cache.syncedAt) }) : t("tracker.neverSynced");
  return `<section class="card"><h2>${ico("cloud")}${esc(cfg.source)}</h2><ul class="list">${rows}</ul>
    <p class="muted">${esc(synced)}</p>
    <div class="card-actions"><button class="btn" data-action="trk-sync" data-only="${esc(JSON.stringify([it.name]))}">${ico("refresh")}${esc(t("tracker.sync.button"))}</button>
      <button class="btn" data-action="trk-push" data-names="${esc(JSON.stringify([it.name]))}">${ico("state")}${esc(t("tracker.push.button", { source: cfg.source }))}</button></div></section>`;
}

// ---------- Ajustes: conexión, campos y mapeo ----------

function options(values: [string, string][], selected: string): string {
  return values.map(([v, text]) => `<option value="${esc(v)}"${v === selected ? " selected" : ""}>${esc(text)}</option>`).join("");
}

function fieldSelect(which: "due" | "hours", cfg: TrackerConfig): string {
  const list = which === "due" ? cfg.dateFields : cfg.numFields;
  const current = which === "due" ? cfg.dueField : cfg.hoursField;
  const opts: [string, string][] = [["", t("tracker.field.none")], ...list.map((f): [string, string] => [f.ref, `${f.name} (${f.ref})`])];
  return `<select data-change="trk-field" data-which="${which}">${options(opts, current)}</select>`;
}

function stateOptions(selected: string): string {
  const opts: [string, string][] = [["", t("tracker.map.unmapped")], ...wf().states.map((s): [string, string] => [s.id, label(s.labels, s.id)])];
  return options(opts, selected);
}

function initialMap(cfg: TrackerConfig): Record<string, string> {
  return cfg.mapSaved ? { ...cfg.map } : { ...cfg.suggested, ...cfg.map };
}

function mappingTable(cfg: TrackerConfig): string {
  const map = (TR.draftMap ??= initialMap(cfg));
  const counts: Record<string, number> = {};
  for (const w of Object.values(cfg.cache?.items ?? {})) counts[w.state] = (counts[w.state] ?? 0) + 1;
  const extra = Object.keys(counts)
    .filter((n) => !cfg.states.some((s) => s.name === n))
    .map((name) => ({ name, color: "", category: "", types: [] as string[], suggested: null }));
  const rows = [...cfg.states, ...extra]
    .map((s) => {
      const v = map[s.name] ?? "";
      const sug = !cfg.mapSaved && v && v === cfg.suggested[s.name];
      return `<tr><td><span class="chip">${trackerDot(s.name)}${esc(s.name)}</span></td>
        <td class="muted">${esc(s.category ? t(`tracker.category.${s.category}`) : t("tracker.map.outside"))}</td>
        <td class="muted" title="${esc(s.types.join(", "))}">${esc(s.types.slice(0, 3).join(", "))}${s.types.length > 3 ? ` +${s.types.length - 3}` : ""}</td>
        <td class="num muted">${counts[s.name] ?? ""}</td>
        <td><select data-change="trk-map" data-state="${esc(s.name)}">${stateOptions(v)}</select>
          ${sug ? `<span class="trk-tag">${esc(t("tracker.map.suggestedTag"))}</span>` : ""}${TR.newStates.has(s.name) ? `<span class="trk-tag">${esc(t("tracker.map.newTag"))}</span>` : ""}</td></tr>`;
    })
    .join("");
  return `<table class="grid trk-table"><thead><tr><th>${esc(t("tracker.map.state", { source: cfg.source }))}</th><th>${esc(t("tracker.map.category"))}</th>
    <th>${esc(t("tracker.map.types"))}</th><th class="num">${esc(t("tracker.map.inFolders"))}</th><th>${esc(t("tracker.map.local"))}</th></tr></thead><tbody>${rows}</tbody></table>`;
}

export function trackerSettingsCard(): string {
  if (!S.root) return "";
  const cfg = ensureLoaded();
  if (!cfg) return `<section class="card"><h2>${ico("cloud")}${esc(t("tracker.title"))}</h2><p class="muted">${esc(t("common.loading"))}</p></section>`;
  const kinds: [string, string][] = cfg.kinds.map((k) => [k, t(`tracker.kind.${k}`)]);
  const conn = `<div class="set-row"><label for="trk-kind">${esc(t("tracker.kind"))}</label><select id="trk-kind">${options(kinds, cfg.kind)}</select></div>
    <div class="set-row"><label for="trk-org">${esc(t("tracker.org"))}</label><input id="trk-org" value="${esc(cfg.org)}" placeholder="https://dev.azure.com/my-org"></div>
    <div class="set-row"><label for="trk-project">${esc(t("tracker.project"))}</label><input id="trk-project" value="${esc(cfg.project)}"></div>
    <div class="set-row"><label for="trk-secret">${esc(t("tracker.secret"))}</label><input id="trk-secret" type="password" autocomplete="off" placeholder="${esc(t(cfg.hasSecret ? "tracker.secretSaved" : "tracker.secretHint"))}"></div>
    <p class="muted">${esc(t("tracker.secretHelp"))}</p>
    <div class="card-actions"><button class="btn primary" data-action="trk-connect">${ico("cloud")}${esc(t("tracker.connect"))}</button>
      ${cfg.connected && cfg.hasSecret ? `<button class="btn" data-action="trk-clear-secret">${esc(t("tracker.clearSecret"))}</button>` : ""}
      ${cfg.connected && cfg.hasWriteSecret ? `<button class="btn" data-action="trk-clear-secret" data-write="1">${esc(t("tracker.clearWriteSecret"))}</button>` : ""}
      ${cfg.connected ? `<button class="btn danger" data-action="trk-disconnect">${esc(t("tracker.disconnect"))}</button>` : ""}</div>`;
  if (!cfg.connected || !cfg.states.length) {
    return `<section class="card"><h2>${ico("cloud")}${esc(t("tracker.title"))}</h2><p class="muted">${esc(t("tracker.readOnly"))}</p>${conn}</section>`;
  }
  const fetched = cfg.statesFetchedAt ? fmtDate(cfg.statesFetchedAt) : "";
  return `<section class="card"><h2>${ico("cloud")}${esc(cfg.source)} <span class="muted">${esc(cfg.org)} / ${esc(cfg.project)}</span></h2>
    <p class="muted">${esc(t("tracker.readOnly"))}</p>${conn}
    <h3>${esc(t("tracker.fields"))}</h3>
    <div class="set-row"><label>${esc(t("tracker.field.due"))}</label>${fieldSelect("due", cfg)}</div>
    <div class="set-row"><label>${esc(t("tracker.field.hours"))}</label>${fieldSelect("hours", cfg)}</div>
    <p class="muted">${esc(t("tracker.field.help"))}${cfg.hasPriority ? "" : ` ${t("tracker.field.noPriority")}`}</p>
    <h3>${esc(t("tracker.map.title"))}</h3>
    <p class="muted">${esc(t("tracker.map.help", { n: cfg.states.length, date: fetched }))} ${cfg.mapSaved ? "" : esc(t("tracker.map.reviewSuggestions"))}
      <button class="link" data-action="trk-refetch">${esc(t("tracker.refetch"))}</button></p>
    ${mappingTable(cfg)}
    <div class="card-actions"><button class="btn primary" data-action="trk-save-map">${esc(t("tracker.map.save"))}</button>
      ${cfg.mapSaved ? `<button class="btn" data-action="trk-sync">${ico("refresh")}${esc(t("tracker.sync.button"))}</button>` : ""}</div></section>`;
}

onAction("trk-connect", async (el) => {
  const done = busy(el, t("tracker.connecting"));
  try {
    const v = (id: string) => $<HTMLInputElement>(`#${id}`)!.value;
    await trackerApi.connect(v("trk-kind"), v("trk-org"), v("trk-project"), v("trk-secret"));
    const before = new Set(TR.cfg?.states.map((s) => s.name) ?? []);
    TR.cfg = await trackerApi.fetch();
    TR.newStates = before.size ? new Set(TR.cfg.states.filter((s) => !before.has(s.name)).map((s) => s.name)) : new Set();
    TR.draftMap = null;
    toast(t("tracker.connected", { n: TR.cfg.states.length }));
  } catch (err) {
    toast(errorText(err), undefined, "error");
    await loadTracker();
  } finally {
    done();
  }
  render();
});
onAction("trk-refetch", async () => {
  const before = new Set(TR.cfg?.states.map((s) => s.name) ?? []);
  TR.cfg = await trackerApi.fetch();
  TR.newStates = new Set(TR.cfg.states.filter((s) => !before.has(s.name)).map((s) => s.name));
  // Los estados nuevos se sugieren aunque el mapeo ya esté guardado.
  const map = (TR.draftMap ??= initialMap(TR.cfg));
  for (const s of TR.newStates) if (!(s in map) && TR.cfg.suggested[s]) map[s] = TR.cfg.suggested[s];
  toast(t("tracker.refetched", { n: TR.cfg.states.length, added: TR.newStates.size }));
  render();
  rerender();
});
onChange("trk-map", (el) => {
  const map = (TR.draftMap ??= initialMap(TR.cfg!));
  const v = (el as HTMLSelectElement).value;
  if (v) map[el.dataset.state!] = v;
  else delete map[el.dataset.state!];
});
onAction("trk-save-map", async () => {
  TR.cfg = await trackerApi.setMap(TR.draftMap ?? initialMap(TR.cfg!));
  TR.draftMap = null;
  TR.newStates.clear();
  toast(t("tracker.map.saved"));
  render();
});
// Como la v1: la fecha pactada se relee al momento solo desde el modal del sync (si no, en el próximo
// sync); las horas releen siempre todos los work items.
onChange("trk-field", async (el) => {
  const sel = el as HTMLSelectElement;
  const which = el.dataset.which as "due" | "hours";
  try {
    TR.cfg = await trackerApi.setField(which, sel.value);
    const inSync = !!sync && modal?.view === syncView;
    if (which === "due" && !inSync) {
      toast(t(sel.value ? "tracker.field.dueSaved" : "tracker.field.dueNone"));
    } else {
      sel.disabled = true;
      toast(t("tracker.rereading"));
      TR.cfg = await trackerApi.sync();
      if (inSync) await loadPreview();
      const n = Object.values(TR.cfg.cache?.items ?? {}).filter((w) => (w.estimate ?? 0) > 0).length;
      if (which === "due") toast(t(sel.value ? "tracker.field.dueUpdated" : "tracker.field.dueNone"));
      else toast(sel.value ? t("tracker.field.hoursRead", { n }) : t("tracker.field.hoursNone", { source: TR.cfg.source }));
    }
  } catch (err) {
    toast(errorText(err), undefined, "error");
    sel.disabled = false;
  }
  render();
  rerender();
});
onAction("trk-clear-secret", async (el) => {
  const write = !!el.dataset.write;
  if (!(await confirmDialog(t(write ? "tracker.clearWriteSecretConfirm" : "tracker.clearSecretConfirm"), t("common.delete")))) return;
  TR.cfg = await trackerApi.clearSecret(write);
  render();
  rerender();
});
onAction("trk-disconnect", async () => {
  if (!(await confirmDialog(t("tracker.disconnectConfirm", { org: TR.cfg!.org, project: TR.cfg!.project }), t("tracker.disconnect")))) return;
  TR.cfg = await trackerApi.disconnect();
  TR.draftMap = null;
  if (wizard) {
    wizard = null;
    closeModal();
  }
  toast(t("tracker.disconnected"));
  render();
});
onAction("trk-open", (el) => trackerApi.open(el.dataset.id!).then(() => undefined));

// ---------- asistente: 1 conexión · 2 mapeo (como la v1) ----------

let wizard: { step: 1 | 2; thenSync: boolean; only: string[] | null } | null = null;

/** Abre el asistente de conexión y mapeo. Con `thenSync`, guardar el mapeo sincroniza. */
export function openWizard(step: 1 | 2, thenSync = false, only: string[] | null = null): void {
  wizard = { step: step === 2 && TR.cfg?.states.length ? 2 : 1, thenSync, only };
  TR.draftMap = null;
  openModal(wizardView);
}

const wizardButton = () => `<button class="btn ghost" data-action="trk-wizard">${ico("settings")}${esc(t("tracker.wizard.open"))}</button>`;

function wizardView(): string {
  const cfg = TR.cfg!;
  const w = wizard!;
  const can2 = cfg.states.length > 0;
  const steps = `<div class="trk-steps"><button class="chip${w.step === 1 ? " on" : ""}" data-action="trk-wz-step" data-step="1">1 · ${esc(t("tracker.wizard.connection"))}</button>
    <button class="chip${w.step === 2 ? " on" : ""}" data-action="trk-wz-step" data-step="2"${can2 ? "" : " disabled"}>2 · ${esc(t("tracker.map.title"))}</button></div>`;
  if (w.step === 1) {
    const kinds: [string, string][] = cfg.kinds.map((k) => [k, t(`tracker.kind.${k}`)]);
    return `<h2>${ico("cloud")}${esc(cfg.source)}</h2>${steps}
      <p class="muted">${esc(t("tracker.readOnly"))}</p>
      <div class="trk-body">
        <div class="set-row"><label for="trkw-kind">${esc(t("tracker.kind"))}</label><select id="trkw-kind">${options(kinds, cfg.kind)}</select></div>
        <div class="set-row"><label for="trkw-org">${esc(t("tracker.org"))}</label><input id="trkw-org" value="${esc(cfg.org)}" placeholder="https://dev.azure.com/my-org"></div>
        <div class="set-row"><label for="trkw-project">${esc(t("tracker.project"))}</label><input id="trkw-project" value="${esc(cfg.project)}"></div>
        <div class="set-row"><label for="trkw-secret">${esc(t("tracker.secret"))}</label><input id="trkw-secret" type="password" autocomplete="off" placeholder="${esc(t(cfg.hasSecret ? "tracker.secretSaved" : "tracker.secretHint"))}"></div>
        <p class="muted">${esc(t("tracker.secretHelp"))}
          ${cfg.connected && cfg.hasSecret ? `<button class="link" data-action="trk-clear-secret">${esc(t("tracker.clearSecret"))}</button>` : ""}
          ${cfg.connected ? ` · <button class="link" data-action="trk-disconnect">${esc(t("tracker.disconnect"))}</button>` : ""}</p>
      </div>
      <div class="modal-actions"><span class="spacer"></span><button class="btn" data-action="trk-close">${esc(t("common.cancel"))}</button>
        <button class="btn primary" data-action="trk-wz-connect">${ico("cloud")}${esc(t("tracker.connect"))}</button></div>`;
  }
  const fetched = cfg.statesFetchedAt ? fmtDate(cfg.statesFetchedAt) : "";
  return `<h2>${ico("cloud")}${esc(cfg.source)} <span class="muted">${esc(cfg.org)} / ${esc(cfg.project)}</span></h2>${steps}
    <p class="muted">${esc(t("tracker.map.help", { n: cfg.states.length, date: fetched }))} <button class="link" data-action="trk-refetch">${esc(t("tracker.refetch"))}</button></p>
    <div class="trk-body">
      <div class="set-row"><label>${esc(t("tracker.field.due"))}</label>${fieldSelect("due", cfg)}</div>
      <div class="set-row"><label>${esc(t("tracker.field.hours"))}</label>${fieldSelect("hours", cfg)}</div>
      <p class="muted">${esc(t("tracker.field.help"))}${cfg.hasPriority ? "" : ` ${esc(t("tracker.field.noPriority"))}`}</p>
      ${cfg.mapSaved ? "" : `<p class="muted">${esc(t("tracker.map.reviewSuggestions"))}</p>`}
      ${mappingTable(cfg)}
    </div>
    <div class="modal-actions"><button class="btn ghost" data-action="trk-wz-step" data-step="1">${esc(t("tracker.wizard.back"))}</button><span class="spacer"></span>
      <button class="btn" data-action="trk-close">${esc(t("common.cancel"))}</button>
      <button class="btn primary" data-action="trk-wz-save">${esc(t(w.thenSync ? "tracker.map.saveAndSync" : "tracker.map.save"))}</button></div>`;
}

onAction("trk-wizard", async () => {
  try {
    TR.cfg = await trackerApi.config();
  } catch (err) {
    toast(errorText(err), undefined, "error");
    return;
  }
  openWizard(TR.cfg.states.length ? 2 : 1);
});
onAction("trk-wz-step", (el) => {
  if (!wizard) return;
  wizard.step = el.dataset.step === "2" && TR.cfg?.states.length ? 2 : 1;
  rerender();
});
onAction("trk-wz-connect", async (el) => {
  const done = busy(el, t("tracker.connecting"));
  try {
    const v = (id: string) => $<HTMLInputElement>(`#${id}`)!.value;
    await trackerApi.connect(v("trkw-kind"), v("trkw-org"), v("trkw-project"), v("trkw-secret"));
    const before = new Set(TR.cfg?.states.map((s) => s.name) ?? []);
    TR.cfg = await trackerApi.fetch();
    TR.newStates = before.size ? new Set(TR.cfg.states.filter((s) => !before.has(s.name)).map((s) => s.name)) : new Set();
    TR.draftMap = null;
    if (wizard) wizard.step = 2;
    toast(t("tracker.connected", { n: TR.cfg.states.length }));
  } catch (err) {
    toast(errorText(err), undefined, "error");
    done();
    return;
  }
  render();
  rerender();
});
onAction("trk-wz-save", async (el) => {
  const done = busy(el, t("tracker.saving"));
  try {
    TR.cfg = await trackerApi.setMap(TR.draftMap ?? initialMap(TR.cfg!));
  } catch (err) {
    toast(errorText(err), undefined, "error");
    done();
    return;
  }
  TR.draftMap = null;
  TR.newStates.clear();
  const w = wizard;
  wizard = null;
  closeModal();
  render();
  toast(t("tracker.map.saved"));
  if (w?.thenSync) await startSync(w.only);
});

// ---------- sincronizar con previsualización ----------

let sync: { preview: SyncPreview | null; includeDiffer: boolean; toggled: Set<string>; only: string[] | null } | null = null;

const rowChecked = (r: SyncRow) => (sync!.toggled.has(r.key) ? !r.on : r.on);

async function loadPreview(): Promise<void> {
  if (!sync) return;
  sync.preview = await trackerApi.preview(sync.includeDiffer, sync.only);
}

/** Errores que se arreglan en la conexión: reabren el asistente (la v1 miraba PAT/organización/proyecto). */
const SETUP_ERRORS = new Set([
  "tracker.noSecret",
  "tracker.noSecretFor",
  "tracker.notConnected",
  "tracker.auth",
  "tracker.loginPage",
  "tracker.orgNotFound",
  "tracker.projectNotFound",
  "tracker.badOrg",
  "tracker.noOrg",
  "tracker.noProject",
]);

/** Sincronizar (todo o solo unas carpetas). Sin configurar abre el asistente y sincroniza al guardar. */
async function startSync(only: string[] | null): Promise<void> {
  if (!S.root) return;
  let cfg: TrackerConfig;
  try {
    cfg = TR.cfg = await trackerApi.config();
  } catch (err) {
    toast(errorText(err), undefined, "error");
    return;
  }
  if (!cfg.connected || !cfg.hasSecret) return openWizard(1, true, only);
  if (!cfg.states.length || !cfg.mapSaved) return openWizard(2, true, only);
  toast(t("tracker.sync.reading", { source: cfg.source }));
  try {
    TR.cfg = await trackerApi.sync();
  } catch (err) {
    toast(errorText(err), undefined, "error");
    if (SETUP_ERRORS.has((err as ApiError)?.code)) openWizard(1, true, only);
    return;
  }
  sync = { preview: null, includeDiffer: false, toggled: new Set(), only };
  await loadPreview();
  render();
  openModal(syncView);
}

function syncView(): string {
  const cfg = TR.cfg!;
  const p = sync!.preview!;
  const rows = p.rows;
  const picked = rows.filter(rowChecked);
  const change = (r: SyncRow) =>
    [
      r.status ? `<span class="chip">${stateChip(r.current)} → <span class="dot ${stateDef(r.status)?.category ?? "unknown"}"></span><b>${esc(stateLabel(r.status))}</b></span>` : "",
      r.type ? `<span class="chip">${esc(typeLabel(r.type))}</span>` : "",
      r.why ? `<span class="muted small">${esc(t("tracker.sync.gitWhy", { env: envLabel(r.why.env), repos: r.why.repos.join(", ") }))}</span>` : "",
    ]
      .filter(Boolean)
      .join(" ");
  const unmapped = p.unmapped.length
    ? `<h3>${esc(t("tracker.sync.unmapped", { source: cfg.source }))}</h3><p class="muted">${esc(t("tracker.sync.unmappedHelp"))}</p>
      <div class="trk-unmapped">${p.unmapped
        .map((u) => `<span class="chip">${trackerDot(u.state)}${esc(u.state)}</span><select data-change="trk-sync-map" data-state="${esc(u.state)}">${stateOptions(cfg.suggested[u.state] ?? "")}</select><span class="muted" title="${esc(u.items.join("\n"))}">${esc(t("tracker.sync.nItems", { n: u.items.length }))}</span>`)
        .join("")}</div>`
    : "";
  const table = rows.length
    ? `<table class="grid trk-table"><thead><tr><th><input type="checkbox" data-change="trk-sync-all"${picked.length === rows.length ? " checked" : ""} aria-label="${esc(t("tracker.sync.all"))}"></th>
        <th>${esc(t("tracker.sync.folder"))}</th><th>${esc(t("tracker.sync.ids"))}</th><th>${esc(cfg.source)}</th><th>${esc(t("tracker.sync.change"))}</th><th></th></tr></thead><tbody>
      ${rows
        .map((r) => `<tr class="${rowChecked(r) ? "" : "trk-off"}"><td><input type="checkbox" data-change="trk-sync-row" data-key="${esc(r.key)}"${rowChecked(r) ? " checked" : ""}></td>
          <td>${esc(r.name)}</td><td class="muted">#${esc(r.ids.join(", "))}</td>
          <td title="${esc(r.tracker.title)}"><span class="chip">${trackerDot(r.tracker.state)}${esc(r.tracker.state)}</span> <span class="muted">${esc(r.tracker.type)} · ${esc(r.tracker.title)}</span></td>
          <td>${change(r)}</td><td><span class="trk-kind k-${r.kind}">${esc(t(`tracker.sync.kind.${r.kind}`))}</span></td></tr>`)
        .join("")}</tbody></table>`
    : `<p class="empty">${esc(t(p.differ && !sync!.includeDiffer ? "tracker.sync.nothingEmpty" : "tracker.sync.nothing"))}</p>`;
  return `<h2>${esc(t("tracker.sync.title", { source: cfg.source }))}</h2>
    <p class="muted">${sync!.only ? esc(t("tracker.sync.only", { n: sync!.only.length })) + " " : ""}${esc(t("tracker.sync.summary", { read: p.read, requested: p.requested, date: p.syncedAt ? fmtDate(p.syncedAt) : "" }))}</p>
    <div class="trk-body">
      <div class="set-row"><label>${esc(t("tracker.field.due"))}</label>${fieldSelect("due", cfg)}</div>
      <div class="set-row"><label>${esc(t("tracker.field.hours"))}</label>${fieldSelect("hours", cfg)}</div>
      <div class="chips">
        <span class="chip">${esc(t("tracker.sync.pill.fill"))} <b>${p.fill}</b></span>
        <span class="chip">${esc(t("tracker.sync.pill.types"))} <b>${p.types}</b></span>
        <span class="chip">${esc(t("tracker.sync.pill.differ"))} <b>${p.differ}</b></span>
        <span class="chip">${esc(t("tracker.sync.pill.equal"))} <b>${p.equal}</b></span>
        <span class="chip">${esc(t("tracker.sync.pill.unmapped"))} <b>${new Set(p.unmapped.flatMap((u) => u.items)).size}</b></span>
        <span class="chip" title="${esc(p.notFound.join("\n"))}">${esc(t("tracker.sync.pill.notFound", { source: cfg.source }))} <b>${p.notFound.length}</b></span>
      </div>
      ${unmapped}
      <h3><label class="check"><input type="checkbox" data-change="trk-sync-differ"${sync!.includeDiffer ? " checked" : ""}>${esc(t("tracker.sync.includeDiffer", { n: p.differ }))}</label></h3>
      ${sync!.includeDiffer ? `<p class="muted">${esc(t("tracker.sync.backHelp"))}</p>` : ""}
      ${table}
    </div>
    <div class="modal-actions">${wizardButton()}<span class="spacer"></span><button class="btn" data-action="trk-close">${esc(t("common.close"))}</button>
      <button class="btn primary" data-action="trk-sync-apply"${picked.length ? "" : " disabled"}>${esc(t("tracker.sync.apply", { n: picked.length }))}</button></div>`;
}

function stateChip(id: string): string {
  if (!id) return `<span class="muted">${esc(t("items.noStatus"))}</span>`;
  return `<span class="dot ${stateDef(id)?.category ?? "unknown"}"></span>${esc(stateLabel(id))}`;
}

onAction("trk-close", () => closeModal());
onAction("trk-sync", (el) => startSync(el.dataset.only ? (JSON.parse(el.dataset.only) as string[]) : null));

/** Sincronizar solo esas carpetas (para el menú de la selección). */
export function syncSelection(names: string[]): Promise<void> {
  return startSync(names.length ? names : null);
}
onChange("trk-sync-row", (el) => {
  const key = el.dataset.key!;
  if (sync!.toggled.has(key)) sync!.toggled.delete(key);
  else sync!.toggled.add(key);
  rerender();
});
onChange("trk-sync-all", (el) => {
  const on = (el as HTMLInputElement).checked;
  sync!.toggled = new Set(sync!.preview!.rows.filter((r) => r.on !== on).map((r) => r.key));
  rerender();
});
onChange("trk-sync-differ", async (el) => {
  sync!.includeDiffer = (el as HTMLInputElement).checked;
  await loadPreview();
  rerender();
});
onChange("trk-sync-map", async (el) => {
  const v = (el as HTMLSelectElement).value;
  if (!v) return;
  TR.cfg = await trackerApi.setMap({ ...TR.cfg!.map, [el.dataset.state!]: v });
  await loadPreview();
  rerender();
});
onAction("trk-sync-apply", async (el) => {
  const picked = sync!.preview!.rows.filter(rowChecked);
  const over = picked.filter((r) => r.status && r.current);
  if (over.length && !(await confirmDialog(t(over.some((r) => r.kind === "back") ? "tracker.sync.overwriteBack" : "tracker.sync.overwrite", { n: over.length }), t("tracker.sync.applyShort")))) return;
  const done = busy(el, t("tracker.applying"));
  try {
    const res = await trackerApi.apply(picked.map((r) => ({ name: r.name, status: r.status || null, type: r.type || null })));
    closeModal();
    sync = null;
    await refresh();
    toast(t("tracker.sync.applied", { n: res.done.length }) + (res.errors.length ? ` · ${failures(res.errors)}` : ""), undefined, res.errors.length ? "error" : "info");
  } finally {
    done();
  }
});

// ---------- asignados del período ----------

let assigned: { period: string; list: AssignedList | null; error: string; loading: boolean; picked: Set<string> } | null = null;

/** Clave `AAAA-MM` del período de una fecha (como `Workflow::period_key`). */
function periodKey(d: Date): string {
  let y = d.getFullYear();
  let m = d.getMonth() + 1;
  if (d.getDate() < wf().period.startDay) {
    m -= 1;
    if (m === 0) {
      m = 12;
      y -= 1;
    }
  }
  return `${y}-${String(m).padStart(2, "0")}`;
}

function shiftPeriod(key: string, n: number): string {
  const [y, m] = key.split("-").map(Number);
  const d = new Date(y, m - 1 + n, 1);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}`;
}

function periodLabel(key: string, n: number): string {
  const [y, m] = key.split("-").map(Number);
  const day = wf().period.startDay;
  const start = new Date(y, m - 1, day);
  const end = new Date(y, m, day - 1);
  const f = (d: Date) => d.toLocaleDateString(lang(), { day: "2-digit", month: "2-digit" });
  const rel = n === 0 ? t("tracker.period.current") : n === -1 ? t("tracker.period.previous") : n === 1 ? t("tracker.period.next") : t("tracker.period.inN", { n });
  return `${f(start)} – ${f(end)} · ${rel}`;
}

export function openAssigned(): void {
  assigned = { period: periodKey(new Date()), list: null, error: "", loading: false, picked: new Set() };
  openModal(assignedView);
  loadAssigned();
}

async function loadAssigned(): Promise<void> {
  const a = assigned!;
  a.loading = true;
  a.error = "";
  rerender();
  try {
    a.list = await trackerApi.assigned(a.period);
    a.picked = new Set(a.list.items.filter((r) => !r.existing.length).map((r) => r.id));
  } catch (err) {
    a.list = null;
    a.error = errorText(err);
  }
  a.loading = false;
  rerender();
}

function assignedView(): string {
  const a = assigned!;
  const cur = periodKey(new Date());
  const opts = [-1, 0, 1, 2]
    .map((n) => {
      const k = shiftPeriod(cur, n);
      return `<option value="${k}"${k === a.period ? " selected" : ""}>${esc(periodLabel(k, n))}</option>`;
    })
    .join("");
  const rows = a.list?.items ?? [];
  const free = rows.filter((r) => !r.existing.length);
  const field = TR.cfg?.dateFields.find((f) => f.ref === a.list?.field)?.name ?? a.list?.field ?? "";
  const list = a.loading
    ? `<p class="empty">${esc(t("common.loading"))}</p>`
    : a.error
      ? `<p class="error">${esc(a.error)}</p><button class="btn" data-action="trk-to-settings">${ico("settings")}${esc(t("tracker.title"))}</button>`
      : !rows.length
        ? `<p class="empty">${esc(t("tracker.assigned.none"))}</p>`
        : `<ul class="list">${rows
            .map((r) => {
              const has = r.existing.length > 0;
              const meta = [r.state, r.priority !== null ? `P${r.priority}` : "", r.due ? fmtIso(r.due) : ""].filter(Boolean).join(" · ");
              return `<li title="${esc(has ? t("tracker.assigned.have", { folders: r.existing.join(", ") }) : r.title)}"><label class="check grow">
                <input type="checkbox" data-change="trk-as-pick" data-id="${esc(r.id)}"${has ? " disabled" : a.picked.has(r.id) ? " checked" : ""}>
                <span class="chip">#${esc(r.id)}</span>${r.localType ? `<span class="chip">${esc(typeLabel(r.localType))}</span>` : ""}<span class="grow">${esc(r.title)}</span></label>
                <span class="muted">${trackerDot(r.state)} ${esc(meta)}${has ? ` · ${esc(t("tracker.assigned.loaded"))}` : ""}</span></li>`;
            })
            .join("")}</ul>`;
  return `<h2>${esc(t("tracker.assigned.title"))}</h2><p class="muted">${esc(t("tracker.assigned.help", { source: TR.cfg?.source ?? "" }))}</p>
    <div class="trk-body">
      <div class="add-row"><select data-change="trk-as-period"${a.loading ? " disabled" : ""}>${opts}</select>
        <button class="btn" data-action="trk-as-reload"${a.loading ? " disabled" : ""}>${ico("refresh")}${esc(t("tracker.assigned.query"))}</button></div>
      ${rows.length && !a.loading ? `<p class="muted">${esc(t("tracker.assigned.count", { n: rows.length, field, have: rows.length - free.length }))}${free.length ? ` <button class="link" data-action="trk-as-all">${esc(t(a.picked.size === free.length ? "tracker.assigned.noneBtn" : "tracker.assigned.allBtn"))}</button>` : ""}</p>` : ""}
      ${list}
    </div>
    <div class="modal-actions"><button class="btn" data-action="trk-close">${esc(t("common.cancel"))}</button>
      <button class="btn primary" data-action="trk-as-import"${a.picked.size && !a.loading ? "" : " disabled"}>${ico("plus")}${esc(t("tracker.assigned.import", { n: a.picked.size }))}</button></div>`;
}

onAction("trk-assigned", () => openAssigned());
onAction("trk-to-settings", () => {
  closeModal();
  goView("settings");
});
onAction("trk-as-reload", () => loadAssigned());
onChange("trk-as-period", (el) => {
  assigned!.period = (el as HTMLSelectElement).value;
  return loadAssigned();
});
onChange("trk-as-pick", (el) => {
  const id = el.dataset.id!;
  if ((el as HTMLInputElement).checked) assigned!.picked.add(id);
  else assigned!.picked.delete(id);
  rerender();
});
onAction("trk-as-all", () => {
  const free = (assigned!.list?.items ?? []).filter((r) => !r.existing.length).map((r) => r.id);
  assigned!.picked = assigned!.picked.size === free.length ? new Set() : new Set(free);
  rerender();
});
onAction("trk-as-import", async (el) => {
  const done = busy(el, t("tracker.importing"));
  try {
    const res = await trackerApi.importItems([...assigned!.picked]);
    closeModal();
    assigned = null;
    await loadTracker();
    await refresh();
    const extra = [res.skipped.length ? t("tracker.assigned.skipped", { n: res.skipped.length }) : "", res.errors.length ? failures(res.errors) : ""].filter(Boolean).join(" · ");
    toast(t("tracker.assigned.imported", { n: res.done.length }) + (extra ? ` · ${extra}` : ""), undefined, res.errors.length ? "error" : "info");
  } finally {
    done();
  }
});

// ---------- cambiar el estado en el tracker (ESCRIBE en el tracker) ----------

interface PushRow {
  id: string;
  item: Item;
  w: TrackerWorkItem | null;
  cands: string[];
  target: string;
  on: boolean;
}

let push: { rows: PushRow[] } | null = null;

const statesFor = (type: string | undefined) => (TR.cfg?.states ?? []).filter((s) => !type || !s.types.length || s.types.includes(type));

export async function openPush(names: string[]): Promise<void> {
  TR.cfg = await trackerApi.config();
  const cfg = TR.cfg;
  if (!cfg.connected) {
    toast(t("tracker.push.setupConn", { source: cfg.source }), undefined, "error");
    return openWizard(1);
  }
  if (!cfg.states.length || !cfg.mapSaved) {
    toast(t("tracker.push.setupMap", { source: cfg.source }), undefined, "error");
    return openWizard(cfg.states.length ? 2 : 1);
  }
  const expand = (n: string): Item[] => {
    const it = itemByName(n);
    if (!it) return [];
    return it.kind === "group" ? it.members.map(itemByName).filter((m): m is Item => !!m) : [it];
  };
  const seen = new Set<string>();
  const rows: PushRow[] = [];
  for (const it of names.flatMap(expand)) {
    for (const id of it.ids) {
      if (seen.has(id)) continue;
      seen.add(id);
      const w = cfg.cache?.items[id] ?? null;
      const cands = statesFor(w?.type).filter((s) => cfg.map[s.name] === statusOf(it)).map((s) => s.name);
      const target = w && cands.includes(w.state) ? w.state : (cands[0] ?? w?.state ?? "");
      rows.push({ id, item: it, w, cands, target, on: !!target && target !== w?.state });
    }
  }
  if (!rows.length) {
    toast(t("tracker.push.noIds"), undefined, "error");
    return;
  }
  push = { rows };
  openModal(pushView);
}

const pushPicked = () => push!.rows.filter((r) => r.on && r.target && r.target !== r.w?.state);

function pushView(): string {
  const cfg = TR.cfg!;
  const n = pushPicked().length;
  const opts = (r: PushRow) =>
    statesFor(r.w?.type)
      .map((s) => `<option value="${esc(s.name)}"${s.name === r.target ? " selected" : ""}>${esc(s.name)}${r.cands.includes(s.name) ? ` · ${t("tracker.push.byMap")}` : ""}${s.name === r.w?.state ? ` · ${t("tracker.push.current")}` : ""}</option>`)
      .join("");
  const rows = push!.rows
    .map((r, i) => `<tr class="${r.on && r.target !== r.w?.state ? "" : "trk-off"}"><td><input type="checkbox" data-change="trk-push-on" data-i="${i}"${r.on ? " checked" : ""}></td>
      <td>${esc(titleOf(r.item))}<div class="muted">${esc(r.item.name)}</div></td><td class="muted">#${esc(r.id)}</td>
      <td>${stateChip(statusOf(r.item))}</td>
      <td>${r.w ? `<span class="chip">${trackerDot(r.w.state)}${esc(r.w.state)}</span> <span class="muted">${esc(r.w.type)}</span>` : `<span class="muted">${esc(t("tracker.notSynced"))}</span>`}</td>
      <td><select data-change="trk-push-target" data-i="${i}">${r.target ? "" : `<option value="" selected>—</option>`}${opts(r)}</select>${r.cands.length ? "" : ` <span class="muted">${esc(t("tracker.push.noMap"))}</span>`}</td></tr>`)
    .join("");
  return `<h2>${esc(t("tracker.push.title", { source: cfg.source }))}</h2>
    <p class="warn">${ico("alert")}${esc(t("tracker.push.warning", { source: cfg.source, org: cfg.org, project: cfg.project }))}</p>
    <div class="trk-body">
      ${cfg.hasWriteSecret ? "" : `<div class="set-row"><label for="trk-write-secret">${esc(t("tracker.writeSecret"))}</label><input id="trk-write-secret" type="password" autocomplete="off" placeholder="${esc(t("tracker.writeSecretHint"))}"></div>
        <p class="muted">${esc(t("tracker.writeSecretHelp"))}</p>`}
      <table class="grid trk-table"><thead><tr><th></th><th>${esc(t("tracker.sync.folder"))}</th><th>${esc(t("tracker.sync.ids"))}</th><th>${esc(t("tracker.push.local"))}</th>
        <th>${esc(t("tracker.push.now", { source: cfg.source }))}</th><th>${esc(t("tracker.push.new", { source: cfg.source }))}</th></tr></thead><tbody>${rows}</tbody></table>
    </div>
    <div class="modal-actions">${wizardButton()}<span class="spacer"></span><button class="btn" data-action="trk-close">${esc(t("common.cancel"))}</button>
      <button class="btn primary" data-action="trk-push-go"${n ? "" : " disabled"}>${esc(t("tracker.push.go", { n, source: cfg.source }))}</button></div>`;
}

/** Re-renderiza sin perder la credencial que se está escribiendo. */
function rerenderKeepingSecret(): void {
  const secret = $<HTMLInputElement>("#trk-write-secret")?.value ?? "";
  rerender();
  const input = $<HTMLInputElement>("#trk-write-secret");
  if (input) input.value = secret;
}

onAction("trk-push", (el) => openPush(JSON.parse(el.dataset.names!) as string[]));
onChange("trk-push-on", (el) => {
  push!.rows[Number(el.dataset.i)].on = (el as HTMLInputElement).checked;
  rerenderKeepingSecret();
});
onChange("trk-push-target", (el) => {
  const r = push!.rows[Number(el.dataset.i)];
  r.target = (el as HTMLSelectElement).value;
  r.on = !!r.target && r.target !== r.w?.state;
  rerenderKeepingSecret();
});
onAction("trk-push-go", async (el) => {
  const picked = pushPicked();
  const secret = $<HTMLInputElement>("#trk-write-secret")?.value.trim() ?? "";
  if (!TR.cfg!.hasWriteSecret && !secret) {
    toast(t("tracker.noWriteSecret", { source: TR.cfg!.source }), undefined, "error");
    return;
  }
  const list = picked.map((r) => `#${r.id}: ${r.w?.state ?? "?"} → ${r.target}`).join(", ");
  if (!(await confirmDialog(t("tracker.push.confirm", { n: picked.length, source: TR.cfg!.source, list }), t("tracker.push.confirmOk")))) return;
  const done = busy(el, t("tracker.push.writing"));
  try {
    const res = await trackerApi.pushStates(picked.map((r) => ({ id: r.id, state: r.target })), secret || null);
    closeModal();
    push = null;
    await loadTracker();
    render();
    toast(t("tracker.push.done", { n: res.done.length, source: TR.cfg?.source ?? "" }) + (res.errors.length ? ` · ${failures(res.errors)}` : ""), undefined, res.errors.length ? "error" : "info");
  } finally {
    done();
  }
});

// ---------- nuevo item con id del tracker ----------

/** Datos del tracker para el id (sin conexión o con la credencial rechazada vuelve vacío, con `error`:
 * se completa a mano, como la v1). */
export function fetchTrackerItem(id: string): Promise<TrackerItem> {
  return trackerApi.item(id);
}

export function createTrackedItem(data: {
  id: string;
  title: string;
  type: string;
  folder: string;
  description: string;
  criteria: string;
  fetched: TrackerItem | null;
  plan: boolean;
  cols: number;
  rows: number;
}): Promise<NewItemResult> {
  const f = data.fetched;
  return trackerApi.createItem({
    id: data.id,
    title: data.title,
    type: data.type,
    folder: data.folder,
    description: data.description,
    criteria: data.criteria,
    trackerType: f?.type ?? "",
    trackerState: f?.state ?? "",
    plan: data.plan,
    cols: data.cols,
    rows: data.rows,
  });
}
