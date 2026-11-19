// Sesiones de agentes: la lista de la barra lateral, la barra del panel de terminales (pestañas, acciones,
// aviso de sesión larga y de otro item en el pedido), los modales y el sondeo del estado en vivo.

import { api } from "./api";
import { onAction, onAfterRender, refresh, registerNavExtra, render } from "./app";
import { $, confirmDialog, esc, promptDialog, toast } from "./dom";
import { errorText, t } from "./i18n";
import { ico } from "./icons";
import { inSession, itemByName, S, titleOf } from "./state";
import { attach, closeTab, setTermChrome, showTab, syncPanel, termSize, toggleTerminal } from "./terminal";
import type { AgentsInfo, Session } from "./types";
import { openRoot } from "./views/settings";

const RECENT_SECS = 48 * 3600;
/** Avisos de sesión larga ocultados (por sesión: el nivel con el que se ocultó). */
const hiddenWeight = new Map<string, string>();
/** Avisos de otro item ya vistos (por sesión: ts del aviso). */
const seenOther = new Map<string, number>();
let showEarlier = false;
let lastStatus = new Map<string, string>();
let lastFullLoad = 0;

const all = () => [...S.sessions, ...S.otherSessions];
export const sessionById = (id: string) => all().find((s) => s.id === id);
/** Sesiones (no cerradas) que tienen el item. */
export const sessionsOf = (name: string) => S.sessions.filter((s) => inSession(s, name));

function epoch(local: string): number {
  const ms = Date.parse(local);
  return Number.isNaN(ms) ? 0 : ms / 1000;
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
  showTab(s);
  render();
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

function row(s: Session): string {
  const active = S.term.open && S.term.active === s.id ? " sel" : "";
  const sub = s.project ? s.project : s.items.length > 1 ? t("session.items", { n: s.items.length }) : "";
  return `<div class="sess-row${active}" role="button" tabindex="0" data-action="sess-open" data-id="${esc(s.id)}" title="${esc(`${s.name} · ${statusLabel(s)}`)}">
    <span class="sdot ${s.status}" aria-label="${esc(statusLabel(s))}"></span>
    <span class="sess-name">${esc(s.name)}${sub ? ` <span class="sess-sub">${esc(sub)}</span>` : ""}</span>
    ${weightBadge(s)}
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
  const others = S.otherSessions;
  return `<div class="sess-nav">
    <div class="sess-title"><span>${esc(t("session.title"))}</span>
      <button class="icon-btn" data-action="term-toggle" title="${esc(t("term.toggle"))} (Ctrl+J)">${ico("terminal")}</button>
      <button class="icon-btn" data-action="sess-new" title="${esc(t("session.new"))}">${ico("plus")}</button></div>
    <div class="sess-list">
      ${top.map(row).join("") || `<p class="sess-empty">${esc(t("session.none"))}</p>`}
      ${earlier.length ? `<button class="sess-more" data-action="sess-earlier">${ico(showEarlier ? "chevronDown" : "chevronRight")}${esc(t("session.earlier", { n: earlier.length }))}</button>${showEarlier ? earlier.map(row).join("") : ""}` : ""}
      ${others.length ? `<div class="sess-group">${esc(t("session.otherProjects"))}</div>${others.map(row).join("")}` : ""}
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
  const w = s.weight;
  if (w && w.level !== "normal" && s.state !== "closed" && hiddenWeight.get(s.id) !== w.level) {
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
      return `<div class="term-tab${sel}" role="tab" data-action="term-tab" data-id="${esc(id)}"><span class="sdot ${s?.status ?? "paused"}"></span><span class="term-tab-name">${esc(s?.name ?? id)}</span>
        <button class="term-tab-x" data-action="term-tab-close" data-id="${esc(id)}" title="${esc(t("term.closeTab"))}">${ico("x")}</button></div>`;
    })
    .join("");
  const s = S.term.active ? sessionById(S.term.active) : undefined;
  const id = esc(s?.id ?? "");
  const actions = s
    ? `${s.running ? `<button class="icon-btn" data-action="sess-pause" data-id="${id}" title="${esc(t("session.pause"))}">${ico("minus")}</button>` : ""}
       <button class="icon-btn" data-action="sess-rename" data-id="${id}" title="${esc(t("session.rename"))}">${ico("edit")}</button>
       ${s.state !== "closed" ? `<button class="icon-btn" data-action="sess-close" data-id="${id}" title="${esc(t("session.close"))}">${ico("trash")}</button>` : ""}`
    : "";
  const info = s ? `<span class="term-info">${esc(statusLabel(s))}${s.statusDetail ? ` · ${esc(s.statusDetail)}` : ""}</span>` : "";
  head.innerHTML = `<div class="term-tabs" role="tablist">${tabsHtml}</div><span class="grow"></span>${info}${actions}
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
  const first = preselected.length === 1 ? itemByName(preselected[0]) : undefined;
  const back = document.createElement("div");
  back.className = "modal-back";
  back.innerHTML = `<form class="modal" role="dialog" aria-modal="true"><h2>${esc(t("session.new"))}</h2>
    <label>${esc(t("session.name"))}<input name="name" value="${esc(first ? (first.ids[0] ? `${first.ids[0]} · ${titleOf(first)}` : titleOf(first)) : "")}"></label>
    <div class="field-label">${esc(t("session.itemsLabel"))}</div><div class="check-list">${options || `<p class="empty">${esc(t("items.empty"))}</p>`}</div>
    <div class="row2"><label>${esc(t("session.agent"))}<select name="agent">${agents}</select></label>
    <label>${esc(t("session.modeLabel"))}<select name="mode">${modes}</select></label></div>
    <label>${esc(t("session.model"))}<input name="model" placeholder="${esc(t("session.modelHint"))}"></label>
    <p class="error" data-error></p>
    <div class="modal-actions"><button type="button" class="btn" data-cancel>${esc(t("common.cancel"))}</button><button class="btn primary">${ico("play")}${esc(t("session.start"))}</button></div></form>`;
  document.body.append(back);
  const form = back.querySelector("form")!;
  const value = (n: string) => (form.elements.namedItem(n) as HTMLInputElement).value;
  const close = () => back.remove();
  back.querySelector<HTMLElement>("[data-cancel]")!.onclick = close;
  back.addEventListener("keydown", (e) => e.key === "Escape" && close());
  form.onsubmit = async (e) => {
    e.preventDefault();
    const picked = [...form.querySelectorAll<HTMLInputElement>('input[name="item"]:checked')].map((i) => i.value);
    try {
      const { cols, rows } = termSize();
      const s = await api.sessionCreate({ items: picked, name: value("name"), mode: value("mode"), agent: value("agent"), model: value("model"), cols, rows });
      close();
      await refresh();
      await launched(s);
    } catch (err) {
      back.querySelector("[data-error]")!.textContent = errorText(err);
    }
  };
  form.querySelector<HTMLInputElement>('input[name="name"]')!.focus();
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
onAction("term-toggle", () => toggleTerminal());
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
  await api.sessionPause(el.dataset.id!);
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
  if (s?.weight) hiddenWeight.set(s.id, s.weight.level);
  render();
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
