// Sesiones de agentes: la lista de la barra lateral, la barra del panel de terminales (pestañas, acciones,
// aviso de sesión larga y de otro item en el pedido), los modales y el sondeo del estado en vivo.

import { api } from "./api";
import { goView, onAction, onAfterRender, refresh, registerNavExtra, render } from "./app";
import { $, confirmDialog, esc, fmtAgo, promptDialog, toast } from "./dom";
import { errorText, t } from "./i18n";
import { ico } from "./icons";
import { effectiveStatus, inSession, isClosed, itemByName, S, titleOf, wf } from "./state";
import { attach, closeTab, setTermChrome, showTab, syncPanel, termSize, toggleTerminal } from "./terminal";
import type { AgentsInfo, Item, Session } from "./types";
import { openRoot } from "./views/settings";

const RECENT_SECS = 48 * 3600;
/** Avisos de sesión larga ocultados (por sesión: el nivel con el que se ocultó). Se guardan en las
 *  preferencias de la UI, así siguen ocultos después de reiniciar. */
const WEIGHT_PREF = "sessionWeightHidden";
let hiddenWeight: Record<string, string> = {};
let hiddenWeightLoaded = false;
/** Avisos de otro item ya vistos (por sesión: ts del aviso). */
const seenOther = new Map<string, number>();
let showEarlier = false;
let lastStatus = new Map<string, string>();
let lastFullLoad = 0;

const all = () => [...S.sessions, ...S.otherSessions];
export const sessionById = (id: string) => all().find((s) => s.id === id);
/** Sesiones (no cerradas) que tienen el item. */
export const sessionsOf = (name: string) => S.sessions.filter((s) => inSession(s, name));

/** Antes de mover, renombrar o eliminar carpetas: si una sesión corriendo trabaja en alguna (o adentro, o
 *  en su grupo), se confirma, porque con un archivo abierto el sistema puede no dejar moverla (v1
 *  `busyGuard`). Devuelve `true` si hay que frenar. */
export async function busyGuard(names: string[]): Promise<boolean> {
  const touches = (w: string, n: string) => w === n || w.startsWith(`${n}/`) || n.startsWith(`${w}/`);
  const running = S.sessions.filter((s) => s.running && s.items.some((w) => names.some((n) => touches(w, n))));
  if (!running.length) return false;
  return !(await confirmDialog(t("session.busyMove", { names: running.map((s) => `«${s.name}»`).join(", "), n: running.length }), t("session.busyGoOn")));
}

function epoch(local: string): number {
  const ms = Date.parse(local);
  return Number.isNaN(ms) ? 0 : ms / 1000;
}

const lastTs = (s: Session) => epoch(s.lastActivity || s.created);
const agentOf = (s: Session) => s.agent || "claude";

/** Nombre del agente de una sesión (el del comando propio, si tiene). */
function sessionAgentLabel(s: Session): string {
  return agentLabel({ id: agentOf(s), label: agentsCache?.custom?.label });
}

/** El aviso de sesión larga que corresponde mostrar: ocultar «larga» lo oculta hasta que pase a «muy larga»;
 *  ocultar «muy larga» lo oculta del todo (como la v1). */
function weightShown(s: Session): string {
  const level = s.weight?.level;
  if (!level || level === "normal") return "";
  const off = hiddenWeight[s.id];
  return off === level || off === "heavy" ? "" : level;
}

async function loadHiddenWeight(): Promise<void> {
  if (hiddenWeightLoaded) return;
  hiddenWeightLoaded = true;
  try {
    const v = (await api.uiPrefs())[WEIGHT_PREF];
    if (v && typeof v === "object") hiddenWeight = { ...(v as Record<string, string>), ...hiddenWeight };
  } catch {
    hiddenWeightLoaded = false;
  }
}

/** Otras sesiones corriendo que editaron un repo que también editó esta: pueden pisarse (todas corren en la
 *  raíz). */
function conflicts(s: Session | undefined): { repo: string; other: Session }[] {
  if (!s?.running) return [];
  const pool = s.project ? S.otherSessions.filter((o) => o.root === s.root) : S.sessions;
  return pool
    .filter((o) => o.id !== s.id && o.running && o.state !== "closed")
    .flatMap((o) => o.repos.filter((r) => s.repos.includes(r)).map((repo) => ({ repo, other: o })));
}

// ---------- datos ----------

export async function loadSessions(): Promise<void> {
  if (!S.settings?.root) return;
  try {
    const list = await api.sessions();
    S.sessions = list.sessions;
    S.otherSessions = list.others;
    S.sessionsRev = list.rev;
    lastFullLoad = Date.now();
    await loadHiddenWeight();
    if (!agentsCache) agentsInfo().catch(() => {});
    notifyChanges();
    if (list.autoClosed.length) {
      const names = list.autoClosed.map((id) => sessionById(id)?.name ?? id).join(", ");
      toast(t("session.autoClosed", { names }), {
        label: t("common.undo"),
        run: async () => {
          for (const id of list.autoClosed) await api.sessionReopen(id);
          await loadSessions();
        },
      });
    }
  } catch {
    /* sin carpeta abierta o el manager no arrancó: la lista queda como estaba */
  }
  render();
}

/** Avisa cuando una sesión pasa a "te necesita" (también las de otros proyectos). */
function notifyChanges(): void {
  const now = new Map(all().map((s) => [s.id, s.status]));
  for (const s of all()) {
    if (s.status === "needsYou" && lastStatus.get(s.id) !== "needsYou" && lastStatus.size) {
      toast(t("session.needsYou", { name: s.name }), { label: t("session.go"), run: () => openSession(s.id) });
    }
  }
  lastStatus = now;
}

/** El estado en vivo cambia por hooks: se consulta un contador cada segundo y la lista solo si cambió
 *  (o cada 15 s, para el peso de las sesiones largas). */
export function startSessionPolling(): void {
  window.setInterval(async () => {
    if (document.hidden || !S.settings?.root) return;
    try {
      const rev = await api.sessionsRev();
      if (rev !== S.sessionsRev || Date.now() - lastFullLoad > 15000) await loadSessions();
    } catch {
      /* el manager todavía no arrancó */
    }
  }, 1000);
}

// ---------- abrir y lanzar ----------

export async function openSession(id: string): Promise<void> {
  let s = sessionById(id);
  if (s?.project && s.root !== S.root) {
    await openRoot(s.root);
    await loadSessions();
    s = sessionById(id);
  }
  if (!s) return;
  // Abrir una sesión pausada la reanuda y conecta (como la v1).
  if (!s.running && s.state !== "closed") {
    try {
      await resume(s.id);
    } catch (err) {
      toast(errorText(err), undefined, "error");
      return;
    }
    s = sessionById(id) ?? s;
  }
  showTab(s);
  render();
}

/** `Ctrl+J`: muestra u oculta el panel; sin pestañas abre la última sesión usada (o avisa que no hay). */
export function toggleDock(): void {
  if (S.term.tabs.length) return toggleTerminal();
  const last = S.sessions.filter((s) => s.state !== "closed").sort((a, b) => lastTs(b) - lastTs(a))[0];
  if (last) void openSession(last.id);
  else toast(t("session.noneOpen"), undefined, "error");
}

async function resume(id: string): Promise<void> {
  const { cols, rows } = termSize();
  await api.sessionResume(id, cols, rows);
  await attach(id);
  await loadSessions();
}

/** Después de lanzar una sesión: la lista se actualiza y se abre su pestaña conectada. */
export async function launched(s: Session): Promise<void> {
  await loadSessions();
  const fresh = sessionById(s.id) ?? s;
  showTab(fresh);
  await attach(s.id);
  render();
}

/** Planear un item: estado de planificación, sección de plan y sesión en modo plan. */
export async function planItem(name: string): Promise<void> {
  const { cols, rows } = termSize();
  const out = await api.planItem(name, cols, rows);
  await refresh();
  await launched(out.session);
}

// ---------- barra lateral ----------

function statusLabel(s: Session): string {
  return t(`session.status.${s.status}`);
}

function weightBadge(s: Session): string {
  const level = s.weight?.level;
  if (!level || level === "normal") return "";
  return `<span class="sess-weight ${level}" title="${esc(t(`session.weight.${level}`, { k: Math.round((s.weight?.ctx ?? 0) / 1000) }))}">${esc(t(`session.weight.short.${level}`))}</span>`;
}

/** Tooltip de una fila: items, estado, hace cuánto y repos editados (otro proyecto: proyecto, items y estado). */
function rowTip(s: Session): string {
  const status = `${statusLabel(s)}${s.statusDetail ? `: ${s.statusDetail}` : ""}`;
  if (s.project) return [s.project, s.items.join(", "), status].filter(Boolean).join(" · ");
  const parts = [s.items.join(", "), status, fmtAgo(lastTs(s))];
  if (s.repos.length) parts.push(t("session.edited", { repos: s.repos.join(", ") }));
  return parts.filter(Boolean).join(" · ");
}

function agentTag(s: Session): string {
  const agent = agentOf(s);
  if (agent === "claude") return "";
  const label = agent === "copilot" || agent === "custom" ? t(`session.agentTag.${agent}`) : agent;
  return `<span class="sess-agent">${esc(label)}</span>`;
}

function row(s: Session): string {
  const active = S.term.open && S.term.active === s.id ? " sel" : "";
  const meta = s.project
    ? `<span class="sess-sub">${esc(s.project)}</span>`
    : `${agentTag(s)}<span class="sess-sub" title="${esc(t("session.items", { n: s.items.length }))}">${esc(t("session.itemCount", { n: s.items.length }))}</span>`;
  return `<div class="sess-row${active}" role="button" tabindex="0" data-action="sess-open" data-id="${esc(s.id)}" title="${esc(rowTip(s))}">
    <span class="sdot ${s.status}" aria-label="${esc(statusLabel(s))}"></span>
    <span class="sess-name">${esc(s.name)}</span>
    ${weightBadge(s)}${meta}
    ${s.project ? "" : `<button class="sess-pin${s.pinned ? " on" : ""}" data-action="sess-pin" data-id="${esc(s.id)}" title="${esc(t(s.pinned ? "session.unpin" : "session.pin"))}">${ico("pin")}</button>`}
  </div>`;
}

function sessionsNav(): string {
  if (!S.settings?.root) return `<div class="nav-spacer"></div>`;
  const open = S.sessions.filter((s) => s.state !== "closed");
  const now = Date.now() / 1000;
  const top: Session[] = [];
  const earlier: Session[] = [];
  for (const s of open) {
    const recent = now - epoch(s.lastActivity || s.created) < RECENT_SECS;
    if (s.pinned || s.running || S.term.tabs.includes(s.id) || recent) top.push(s);
    else earlier.push(s);
  }
  top.sort((a, b) => Number(b.pinned) - Number(a.pinned));
  const closed = S.sessions.length - open.length;
  // Otros proyectos: primero las que te necesitan, con el contador en el título.
  const others = [...S.otherSessions].sort((a, b) => Number(b.status === "needsYou") - Number(a.status === "needsYou"));
  const need = others.filter((s) => s.status === "needsYou").length;
  const needBadge = need ? ` <span class="sess-count" title="${esc(t("session.needCount", { n: need }))}">${need}</span>` : "";
  return `<div class="sess-nav">
    <div class="sess-title"><span>${esc(t("session.title"))}</span>
      <button class="icon-btn" data-action="term-toggle" title="${esc(t("term.toggle"))} (Ctrl+J)">${ico("terminal")}</button>
      <button class="icon-btn" data-action="sess-new" title="${esc(t("session.new"))}">${ico("plus")}</button></div>
    <div class="sess-list">
      ${top.map(row).join("") || `<p class="sess-empty">${esc(t("session.none"))}</p>`}
      ${earlier.length ? `<button class="sess-more" data-action="sess-earlier">${ico(showEarlier ? "chevronDown" : "chevronRight")}${esc(t("session.earlier", { n: earlier.length }))}</button>${showEarlier ? earlier.map(row).join("") : ""}` : ""}
      ${others.length ? `<div class="sess-group">${esc(t("session.otherProjects"))}${needBadge}</div>${others.map(row).join("")}` : ""}
      ${closed ? `<button class="sess-more" data-action="sess-closed">${ico("check")}${esc(t("session.closed", { n: closed }))}</button>` : ""}
    </div></div>`;
}

// ---------- barra del panel ----------

function recommendText(s: Session): string {
  const r = s.weight?.recommend;
  if (!r) return "";
  return t(r.reason, { why: t(r.code ? "session.rec.why.code" : "session.rec.why.chat"), hours: r.hours ?? 0 });
}

function banner(s: Session | undefined): string {
  if (!s) return "";
  const id = esc(s.id);
  const parts: string[] = [];
  if (s.state === "closed") {
    parts.push(`<div class="term-note">${ico("info")}<span class="grow">${esc(t("session.isClosed"))}</span>
      <button class="btn" data-action="sess-reopen" data-id="${id}">${esc(t("session.reopen"))}</button></div>`);
  } else if (!s.running) {
    parts.push(`<div class="term-note">${ico("info")}<span class="grow">${esc(t("session.isPaused"))}</span>
      <button class="btn primary" data-action="sess-resume" data-id="${id}">${ico("play")}${esc(t("session.resume"))}</button></div>`);
  }
  const conf = conflicts(s);
  if (conf.length) {
    const list = [...new Set(conf.map((c) => c.repo))]
      .map((repo) => t("session.conflictRepo", { repo, names: conf.filter((c) => c.repo === repo).map((c) => c.other.name).join("», «") }))
      .join(" · ");
    parts.unshift(`<div class="term-note conflict">${ico("alert")}<span class="grow">${esc(t("session.conflict", { list }))}</span></div>`);
  }
  const w = s.weight;
  if (w && weightShown(s) && s.state !== "closed") {
    const rec = w.recommend?.action;
    const btn = (action: string, kind: string, label: string) =>
      `<button class="btn${rec === kind ? " primary" : ""}" data-action="${action}" data-id="${id}">${esc(label)}</button>`;
    parts.push(`<div class="term-note ${w.level}">${ico("alert")}<span class="grow"><b>${esc(t(`session.weight.${w.level}`, { k: Math.round(w.ctx / 1000) }))}</b> ${esc(recommendText(s))}</span>
      ${btn("sess-handoff", "handoff", t("session.handoff"))}${btn("sess-continue", "new", t("session.continue"))}${btn("sess-compact", "compact", t("session.compact"))}
      <button class="icon-btn" data-action="sess-hide-weight" data-id="${id}" title="${esc(t("common.hide"))}">${ico("x")}</button></div>`);
  }
  const o = s.otherItems;
  if (o && (seenOther.get(s.id) ?? 0) < o.ts && s.state !== "closed") {
    const adds = o.items
      .map((x) => `<button class="btn" data-action="sess-add-other" data-id="${id}" data-item="${esc(x.name)}">${ico("plus")}${esc(x.id)}</button>`)
      .join("");
    parts.push(`<div class="term-note">${ico("info")}<span class="grow">${esc(t("session.otherItem", { list: o.items.map((x) => `${x.id} (${x.name})`).join(", ") }))}</span>${adds}
      <button class="icon-btn" data-action="sess-hide-other" data-id="${id}" title="${esc(t("common.hide"))}">${ico("x")}</button></div>`);
  }
  return parts.join("");
}

function renderChrome(): void {
  const head = $("#term-head");
  if (!head) return;
  const tabsHtml = S.term.tabs
    .map((id) => {
      const s = sessionById(id);
      const sel = id === S.term.active ? " sel" : "";
      const st = s ? `${agentOf(s) !== "claude" ? `${sessionAgentLabel(s)} · ` : ""}${statusLabel(s)}` : "";
      const warn = conflicts(s).length ? `<span class="term-tab-warn" title="${esc(t("session.conflictTab"))}">${ico("alert")}</span>` : "";
      return `<div class="term-tab${sel}" role="tab" data-action="term-tab" data-id="${esc(id)}" title="${esc(s?.items.join(", ") ?? "")}"><span class="sdot ${s?.status ?? "paused"}"></span><span class="term-tab-name">${esc(s?.name ?? id)}</span>${st ? `<span class="term-tab-st">${esc(st)}</span>` : ""}${warn}
        <button class="term-tab-x" data-action="term-tab-close" data-id="${esc(id)}" title="${esc(t("term.closeTab"))}">${ico("x")}</button></div>`;
    })
    .join("");
  const s = S.term.active ? sessionById(S.term.active) : undefined;
  const id = esc(s?.id ?? "");
  const actions = s
    ? `${s.state !== "closed" ? `<button class="icon-btn" data-action="sess-add-items" data-id="${id}" title="${esc(t("session.addItems"))}">${ico("plus")}</button>` : ""}
       ${s.running ? `<button class="icon-btn" data-action="sess-pause" data-id="${id}" title="${esc(t("session.pause"))}">${ico("minus")}</button>` : ""}
       <button class="icon-btn" data-action="sess-rename" data-id="${id}" title="${esc(t("session.rename"))}">${ico("edit")}</button>
       ${s.state !== "closed" ? `<button class="icon-btn" data-action="sess-close" data-id="${id}" title="${esc(t("session.close"))}">${ico("trash")}</button>` : ""}`
    : "";
  const info = s ? `<span class="term-info">${esc(statusLabel(s))}${s.statusDetail ? ` · ${esc(s.statusDetail)}` : ""}</span>` : "";
  const maxed = $("#content")?.classList.contains("term-max") ?? false;
  head.innerHTML = `<div class="term-tabs" role="tablist">${tabsHtml}</div><span class="grow"></span>${info}${actions}
    <button class="icon-btn" data-action="term-max" title="${esc(t(maxed ? "term.restore" : "term.maximize"))}">${ico(maxed ? "minimize" : "maximize")}</button>
    <button class="icon-btn" data-action="term-toggle" title="${esc(t("term.hide"))} (Ctrl+J)">${ico("chevronDown")}</button>`;
  $("#term-banner")!.innerHTML = banner(s);
}

// ---------- modales ----------

let agentsCache: AgentsInfo | null = null;

export async function agentsInfo(force = false): Promise<AgentsInfo> {
  if (!agentsCache || force) agentsCache = await api.sessionAgents();
  return agentsCache;
}

export function agentLabel(a: { id: string; label?: string }): string {
  return a.id === "custom" ? a.label || t("agent.custom") : t(`agent.${a.id}`);
}

/** Nombre del agente por defecto, si ya se leyó la lista (si no, "" y se la pide para la próxima). */
export function defaultAgentLabel(): string {
  if (!agentsCache) {
    agentsInfo().catch(() => {});
    return "";
  }
  return agentLabel({ id: agentsCache.default, label: agentsCache.custom?.label });
}

/** Nombre por defecto (como la v1): un item «WI 123 · título», un grupo su título, varios «WI a + WI b». */
function defaultName(names: string[]): string {
  const items = names.map(itemByName).filter((i): i is Item => !!i);
  const label = (i: Item) => (i.kind === "group" || !i.ids[0] ? titleOf(i) : t("session.itemLabel", { id: i.ids[0] }));
  if (items.length === 1) {
    const i = items[0];
    return (i.kind === "group" || !i.ids[0] ? titleOf(i) : `${label(i)} · ${titleOf(i)}`).slice(0, 70);
  }
  return items.map(label).join(" + ").slice(0, 70);
}

/** Varios items sueltos (sin grupo): se ofrece agruparlos en una carpeta antes de empezar. */
function canGroup(names: string[]): boolean {
  return names.length > 1 && names.every((n) => {
    const it = itemByName(n);
    return !!it && it.kind !== "group" && !it.group;
  });
}

/** Nueva sesión con los items elegidos (se pueden cambiar en el modal). */
export async function newSessionModal(preselected: string[] = []): Promise<void> {
  const info = await agentsInfo(true);
  const items = S.items.filter((it) => it.kind === "group" || !it.group);
  const chosen = new Set(preselected);
  const options = items
    .map((it) => `<label class="check-row"><input type="checkbox" name="item" value="${esc(it.name)}"${chosen.has(it.name) ? " checked" : ""}><span>${esc(titleOf(it))} <span class="muted">${esc(it.name)}</span></span></label>`)
    .join("");
  const agents = info.agents
    .map((a) => `<option value="${esc(a.id)}"${a.id === info.default ? " selected" : ""}${a.found ? "" : " disabled"}>${esc(agentLabel(a))}${a.found ? "" : ` (${esc(t("agent.notFound"))})`}</option>`)
    .join("");
  const modes = info.modes.map((m) => `<option value="${esc(m)}">${esc(t(`session.mode.${m}`))}</option>`).join("");
  const back = document.createElement("div");
  back.className = "modal-back";
  back.innerHTML = `<form class="modal" role="dialog" aria-modal="true"><h2>${esc(t("session.new"))}</h2>
    <label>${esc(t("session.name"))}<input name="name" value="${esc(defaultName(preselected))}"></label>
    <div class="field-label">${esc(t("session.itemsLabel"))}</div><div class="check-list">${options || `<p class="empty">${esc(t("items.empty"))}</p>`}</div>
    <div data-group-opt></div>
    <div class="row2"><label>${esc(t("session.agent"))}<select name="agent">${agents}</select></label>
    <label>${esc(t("session.modeLabel"))}<select name="mode">${modes}</select></label></div>
    <label>${esc(t("session.model"))}<input name="model" placeholder="${esc(t("session.modelHint"))}"></label>
    <p class="muted sm"><button type="button" class="link" data-agents-cfg>${esc(t("session.customCfg"))}</button></p>
    <p class="error" data-error></p>
    <div class="modal-actions"><button type="button" class="btn" data-cancel>${esc(t("common.cancel"))}</button><button class="btn primary">${ico("play")}${esc(t("session.start"))}</button></div></form>`;
  document.body.append(back);
  const form = back.querySelector("form")!;
  const value = (n: string) => (form.elements.namedItem(n) as HTMLInputElement).value;
  const nameInput = form.querySelector<HTMLInputElement>('input[name="name"]')!;
  const picked = () => [...form.querySelectorAll<HTMLInputElement>('input[name="item"]:checked')].map((i) => i.value);
  const close = () => back.remove();
  // El nombre sigue a la selección hasta que se escribe uno; la opción de agrupar, mientras no se toque.
  let nameTouched = false;
  let groupTouched: boolean | null = null;
  const groupOpt = back.querySelector<HTMLElement>("[data-group-opt]")!;
  const syncGroup = () => {
    const names = picked();
    if (!canGroup(names)) {
      groupOpt.innerHTML = "";
      return;
    }
    const checked = groupTouched ?? !names.some((n) => sessionsOf(n).some((s) => s.running));
    groupOpt.innerHTML = `<label class="check-row"><input type="checkbox" name="group"${checked ? " checked" : ""}><span>${esc(t("session.groupFirst", { n: names.length, prefix: wf().folders.groupPrefix }))} <span class="muted">${esc(t("session.groupFirstHint"))}</span></span></label>`;
  };
  syncGroup();
  nameInput.addEventListener("input", () => (nameTouched = true));
  form.addEventListener("change", (e) => {
    const el = e.target as HTMLInputElement;
    if (el.name === "group") groupTouched = el.checked;
    if (el.name !== "item") return;
    if (!nameTouched) nameInput.value = defaultName(picked());
    syncGroup();
  });
  back.querySelector<HTMLElement>("[data-cancel]")!.onclick = close;
  back.querySelector<HTMLElement>("[data-agents-cfg]")!.onclick = () => {
    close();
    goView("settings");
  };
  back.addEventListener("keydown", (e) => e.key === "Escape" && close());
  form.onsubmit = async (e) => {
    e.preventDefault();
    let names = picked();
    const name = value("name").trim();
    try {
      if (canGroup(names) && form.querySelector<HTMLInputElement>('input[name="group"]')?.checked) {
        const g = await api.createGroup(name || names.join(" + "), names);
        names = [g.group];
      }
      const { cols, rows } = termSize();
      const s = await api.sessionCreate({ items: names, name, mode: value("mode"), agent: value("agent"), model: value("model"), cols, rows });
      close();
      await refresh();
      await launched(s);
    } catch (err) {
      back.querySelector("[data-error]")!.textContent = errorText(err);
    }
  };
  nameInput.focus();
  nameInput.select();
}

/** «Sumar trabajos…» a una sesión (desde el panel): buscar y elegir items abiertos que no son de la sesión. */
function addItemsModal(sid: string): void {
  const s = sessionById(sid);
  if (!s) return;
  const chosen = new Set<string>();
  const back = document.createElement("div");
  back.className = "modal-back";
  back.innerHTML = `<div class="modal" role="dialog" aria-modal="true"><h2>${esc(t("session.addItemsTitle", { name: s.name }))}</h2>
    <p class="muted">${esc(t(s.hooks ? "session.addItemsHint" : "session.addItemsHintNoHooks"))}</p>
    <input type="search" data-q placeholder="${esc(t("session.addItemsSearch"))}">
    <div class="check-list" data-list></div>
    <div class="modal-actions"><span class="muted grow" data-count></span><button class="btn" data-close>${esc(t("common.cancel"))}</button>
      <button class="btn primary" data-ok disabled>${esc(t("session.addItemsOk"))}</button></div></div>`;
  document.body.append(back);
  const q = back.querySelector<HTMLInputElement>("[data-q]")!;
  const list = back.querySelector<HTMLElement>("[data-list]")!;
  const ok = back.querySelector<HTMLButtonElement>("[data-ok]")!;
  const count = back.querySelector<HTMLElement>("[data-count]")!;
  const draw = () => {
    const needle = q.value.trim().toLowerCase();
    const cand = S.items
      .filter((it) => !inSession(s, it.name) && !isClosed(effectiveStatus(it)))
      .filter((it) => !needle || [it.name, it.title, it.ids.join(" ")].join(" ").toLowerCase().includes(needle))
      .sort((a, b) => b.last - a.last)
      .slice(0, 80);
    list.innerHTML =
      cand
        .map((it) => `<label class="check-row"><input type="checkbox" value="${esc(it.name)}"${chosen.has(it.name) ? " checked" : ""}><span>${esc(it.ids.map((x) => `#${x}`).join(" "))} ${esc(titleOf(it))} <span class="muted">${esc(it.name)}</span></span></label>`)
        .join("") || `<p class="empty">${esc(t("session.addItemsNone"))}</p>`;
  };
  const sync = () => {
    ok.disabled = !chosen.size;
    count.textContent = chosen.size ? t("session.addItemsCount", { n: chosen.size }) : "";
  };
  draw();
  q.oninput = draw;
  list.addEventListener("change", (e) => {
    const el = e.target as HTMLInputElement;
    if (el.checked) chosen.add(el.value);
    else chosen.delete(el.value);
    sync();
  });
  back.querySelector<HTMLElement>("[data-close]")!.onclick = () => back.remove();
  back.addEventListener("keydown", (e) => e.key === "Escape" && back.remove());
  ok.onclick = async () => {
    back.remove();
    try {
      await api.sessionAddItems(sid, [...chosen]);
      await refresh();
      await loadSessions();
      toast(t(s.hooks ? "session.added" : "session.addedNoHooks"));
    } catch (err) {
      toast(errorText(err), undefined, "error");
    }
  };
  q.focus();
}

/** Elegir una sesión abierta para sumarle items. */
export function addToSessionModal(names: string[]): void {
  const open = S.sessions.filter((s) => s.state !== "closed" && !names.every((n) => inSession(s, n)));
  const back = document.createElement("div");
  back.className = "modal-back";
  back.innerHTML = `<div class="modal" role="dialog" aria-modal="true"><h2>${esc(t("session.addTo"))}</h2>
    <ul class="list">${open.map((s) => `<li class="link-row" data-pick="${esc(s.id)}"><span class="sdot ${s.status}"></span><span class="grow">${esc(s.name)} <span class="muted">${esc(s.items.join(", "))}</span></span></li>`).join("") || `<li class="empty">${esc(t("session.none"))}</li>`}</ul>
    <div class="modal-actions"><button class="btn" data-close>${esc(t("common.cancel"))}</button></div></div>`;
  document.body.append(back);
  back.querySelector<HTMLElement>("[data-close]")!.onclick = () => back.remove();
  back.addEventListener("keydown", (e) => e.key === "Escape" && back.remove());
  back.querySelectorAll<HTMLElement>("[data-pick]").forEach((li) => {
    li.onclick = async () => {
      back.remove();
      try {
        await api.sessionAddItems(li.dataset.pick!, names);
        await refresh();
        await loadSessions();
        toast(t("session.added"));
      } catch (err) {
        toast(errorText(err), undefined, "error");
      }
    };
  });
}

function closedModal(): void {
  const closed = S.sessions.filter((s) => s.state === "closed");
  const back = document.createElement("div");
  back.className = "modal-back";
  back.innerHTML = `<div class="modal" role="dialog" aria-modal="true"><h2>${esc(t("session.closedTitle"))}</h2>
    <ul class="list">${closed.map((s) => `<li><span class="grow">${esc(s.name)} <span class="muted">${esc(s.items.join(", "))}</span></span>
      <button class="btn" data-reopen="${esc(s.id)}">${esc(t("session.reopen"))}</button></li>`).join("") || `<li class="empty">${esc(t("session.none"))}</li>`}</ul>
    <div class="modal-actions"><button class="btn" data-close>${esc(t("common.close"))}</button></div></div>`;
  document.body.append(back);
  back.querySelector<HTMLElement>("[data-close]")!.onclick = () => back.remove();
  back.addEventListener("keydown", (e) => e.key === "Escape" && back.remove());
  back.querySelectorAll<HTMLElement>("[data-reopen]").forEach((b) => {
    b.onclick = async () => {
      await api.sessionReopen(b.dataset.reopen!);
      b.closest("li")?.remove();
      await loadSessions();
    };
  });
}

// ---------- acciones ----------

registerNavExtra(sessionsNav);
setTermChrome(renderChrome);
onAfterRender(syncPanel);

const reload = () => loadSessions();

onAction("sess-open", (el) => openSession(el.dataset.id!));
onAction("sess-new", () => newSessionModal(S.detail ? [S.detail] : []));
onAction("sess-pin", async (el, ev) => {
  ev.stopPropagation();
  const s = sessionById(el.dataset.id!);
  if (s) await api.sessionPin(s.id, !s.pinned);
  await reload();
});
onAction("sess-earlier", () => {
  showEarlier = !showEarlier;
  render();
});
onAction("sess-closed", () => closedModal());
onAction("term-toggle", () => toggleDock());
onAction("term-max", () => {
  $("#content")?.classList.toggle("term-max");
  syncPanel();
});
onAction("sess-add-items", (el) => addItemsModal(el.dataset.id!));
onAction("term-tab", (el, ev) => {
  if ((ev.target as HTMLElement).closest(".term-tab-x")) return;
  const s = sessionById(el.dataset.id!);
  if (s) showTab(s);
  render();
});
onAction("term-tab-close", (el, ev) => {
  ev.stopPropagation();
  closeTab(el.dataset.id!);
  render();
});
onAction("sess-resume", async (el) => resume(el.dataset.id!));
onAction("sess-reopen", async (el) => {
  await api.sessionReopen(el.dataset.id!);
  await reload();
});
onAction("sess-pause", async (el) => {
  const s = sessionById(el.dataset.id!);
  if (!s || !(await confirmDialog(t("session.pauseConfirm", { name: s.name }), t("session.pauseOk")))) return;
  await api.sessionPause(s.id);
  await reload();
});
onAction("sess-close", async (el) => {
  const s = sessionById(el.dataset.id!);
  if (!s || !(await confirmDialog(t("session.closeConfirm", { name: s.name }), t("session.close")))) return;
  await api.sessionClose(s.id);
  closeTab(s.id);
  await reload();
  toast(t("session.closedOne", { name: s.name }), { label: t("common.undo"), run: async () => { await api.sessionReopen(s.id); await reload(); } });
});
onAction("sess-rename", async (el) => {
  const s = sessionById(el.dataset.id!);
  if (!s) return;
  const name = await promptDialog(t("session.rename"), s.name);
  if (!name) return;
  await api.sessionRename(s.id, name);
  await reload();
});
onAction("sess-handoff", async (el) => {
  await api.sessionPrompt(el.dataset.id!, "handoff");
  toast(t("session.handoffSent"));
});
onAction("sess-compact", async (el) => {
  await api.sessionPrompt(el.dataset.id!, "compact");
  toast(t("session.compacting"));
});
onAction("sess-continue", async (el) => {
  const s = sessionById(el.dataset.id!);
  if (!s || !(await confirmDialog(t("session.continueConfirm", { name: s.name }), t("session.continue")))) return;
  const { cols, rows } = termSize();
  const next = await api.sessionContinue(s.id, cols, rows);
  closeTab(s.id);
  await launched(next);
});
onAction("sess-hide-weight", (el) => {
  const s = sessionById(el.dataset.id!);
  if (!s?.weight) return;
  hiddenWeight = { ...hiddenWeight, [s.id]: s.weight.level };
  render();
  api.setUiPref(WEIGHT_PREF, hiddenWeight).catch(() => {});
});
onAction("sess-hide-other", (el) => {
  const s = sessionById(el.dataset.id!);
  if (s?.otherItems) seenOther.set(s.id, s.otherItems.ts);
  render();
});
onAction("sess-add-other", async (el) => {
  await api.sessionAddItems(el.dataset.id!, [el.dataset.item!]);
  const s = sessionById(el.dataset.id!);
  if (s?.otherItems) seenOther.set(s.id, s.otherItems.ts);
  await refresh();
  await reload();
  toast(t("session.added"));
});
