// Plugins: botones de sus acciones (detalle, barra de selección, paleta y Ajustes) y el modal genérico que
// dibuja cada paso que devuelve el plugin (formulario, resultado). La lógica vive en el plugin: acá solo se
// muestra lo que manda y se le devuelve lo que el usuario completó.

import { api, pluginApi } from "./api";
import { onAction, onChange, refresh, render } from "./app";
import { $, confirmDialog, esc, toast } from "./dom";
import { errorText, t } from "./i18n";
import { ico } from "./icons";
import { launched } from "./sessions";
import { S, itemByName, stateDef, statusOf } from "./state";
import { termSize } from "./terminal";
import type { Item, PluginAction, PluginButton, PluginField, PluginInfo, PluginList, PluginStep } from "./types";

const P: { list: PluginList | null; loading: boolean } = { list: null, loading: false };

export async function loadPlugins(): Promise<void> {
  P.loading = true;
  try {
    P.list = await pluginApi.list();
  } catch {
    P.list = null;
  } finally {
    P.loading = false;
  }
}

/** Acciones habilitadas para un lugar (y, si hay items, que todos cumplan su `when`). */
export function pluginActions(placement: PluginAction["placements"][number], items: Item[] = []): { plugin: PluginInfo; action: PluginAction }[] {
  const out: { plugin: PluginInfo; action: PluginAction }[] = [];
  for (const plugin of P.list?.plugins ?? []) {
    if (!plugin.enabled || !plugin.id) continue;
    for (const action of plugin.actions ?? []) {
      if (action.placements.includes(placement) && items.every((it) => matches(action, it))) out.push({ plugin, action });
    }
  }
  return out;
}

function matches(action: PluginAction, it: Item): boolean {
  const w = action.when;
  if (!w) return true;
  if (w.kind && it.kind !== w.kind) return false;
  if (w.hasIds != null && it.ids.length > 0 !== w.hasIds) return false;
  if (w.categories?.length && !w.categories.includes(stateDef(statusOf(it))?.category ?? "")) return false;
  return true;
}

function button(plugin: PluginInfo, action: PluginAction, items: string[], placement: string): string {
  const title = action.hint ? `${action.hint} · ${plugin.name}` : (plugin.name ?? "");
  return `<button class="btn" data-action="plugin-run" data-plugin="${esc(plugin.id!)}" data-act="${esc(action.id)}" data-placement="${placement}" data-items="${esc(JSON.stringify(items))}" title="${esc(title)}">${ico(action.icon ?? "puzzle") || ico("puzzle")}${esc(action.label)}</button>`;
}

/** Botones para la barra de selección. */
export function pluginBulkButtons(names: string[]): string {
  const items = names.map(itemByName).filter((x): x is Item => !!x);
  return pluginActions("items", items).map(({ plugin, action }) => button(plugin, action, names, "items")).join("");
}

/** Tarjeta del detalle con las acciones de los plugins (nada si no hay). */
export function pluginItemCard(it: Item): string {
  const list = pluginActions("item", [it]);
  if (!list.length) return "";
  return `<section class="card"><h2>${ico("puzzle")}${esc(t("plugin.actions"))}</h2>
    <div class="card-actions">${list.map(({ plugin, action }) => button(plugin, action, [it.name], "item")).join("")}</div></section>`;
}

/** Comandos para la paleta. */
export function pluginCommands(): { icon: string; text: string; hint: string; run: () => void }[] {
  return pluginActions("palette").map(({ plugin, action }) => ({
    icon: action.icon && ico(action.icon) ? action.icon : "puzzle",
    text: action.label,
    hint: plugin.name ?? "",
    run: () => runAction(plugin.id!, action.id, [], "palette"),
  }));
}

// ---------- Ajustes ----------

const PERMISSION_KEYS: Record<string, string> = {
  "items.write": "plugin.perm.itemsWrite",
  "files.read": "plugin.perm.filesRead",
  secrets: "plugin.perm.secrets",
  tracker: "plugin.perm.tracker",
  "tracker.write": "plugin.perm.trackerWrite",
};

export function pluginsSettingsCard(): string {
  if (!P.list) {
    if (!P.loading) loadPlugins().then(render);
    return `<section class="card"><h2>${ico("puzzle")}${esc(t("plugin.title"))}</h2><p class="muted">…</p></section>`;
  }
  const rows = P.list.plugins
    .map((p) => {
      if (!p.id) {
        return `<li><span class="grow"><b>${esc(p.folder)}</b> <span class="flag">${esc(t("plugin.invalid", { error: p.error ?? "" }))}</span></span></li>`;
      }
      const perms = (p.permissions ?? []).map((x) => t(PERMISSION_KEYS[x] ?? x));
      const settings = p.enabled ? (p.actions ?? []).filter((a) => a.placements.includes("settings")).map((a) => button(p, a, [], "settings")).join("") : "";
      const problems = [p.command ? t("plugin.noCommand", { detail: p.command }) : "", ...(p.presetErrors ?? [])].filter(Boolean);
      return `<li class="plugin-row"><div class="grow">
          <div><b>${esc(p.name ?? p.id)}</b> <span class="muted">${esc(p.version ?? "")} · ${esc(p.folder)}</span></div>
          ${p.description ? `<div class="muted">${esc(p.description)}</div>` : ""}
          <div class="muted small">${esc(perms.length ? t("plugin.permissions", { list: perms.join(", ") }) : t("plugin.noPermissions"))}${p.presets?.length ? ` · ${esc(t("plugin.presets", { n: p.presets.length }))}` : ""}</div>
          ${problems.map((x) => `<div class="flag">${esc(x)}</div>`).join("")}
          ${settings ? `<div class="card-actions">${settings}</div>` : ""}</div>
        <label class="check"><input type="checkbox" data-change="plugin-enable" data-id="${esc(p.id)}"${p.enabled ? " checked" : ""}>${esc(t("plugin.enabled"))}</label></li>`;
    })
    .join("");
  return `<section class="card"><h2>${ico("puzzle")}${esc(t("plugin.title"))}</h2>
    <p class="muted">${esc(t("plugin.intro"))}</p>
    ${rows ? `<ul class="list">${rows}</ul>` : `<p class="empty">${esc(t("plugin.none"))}</p>`}
    <div class="card-actions"><button class="btn" data-action="plugins-folder">${ico("folder")}${esc(t("plugin.openFolder"))}</button>
      <button class="btn" data-action="plugins-reload">${ico("refresh")}${esc(t("plugin.reload"))}</button></div></section>`;
}

export async function setPluginEnabled(id: string, on: boolean): Promise<void> {
  if (on) {
    const p = P.list?.plugins.find((x) => x.id === id);
    const perms = (p?.permissions ?? []).map((x) => t(PERMISSION_KEYS[x] ?? x));
    const text = t("plugin.enableConfirm", { name: p?.name ?? id, list: perms.length ? perms.join(", ") : t("plugin.noPermissionsShort") });
    if (!(await confirmDialog(text, t("plugin.enable")))) return render();
  }
  try {
    P.list = await pluginApi.enable(id, on);
  } catch (err) {
    toast(errorText(err), undefined, "error");
  }
  render();
}

// ---------- modal de una acción ----------

interface Run {
  plugin: string;
  action: string;
  items: string[];
  placement: string;
  step: PluginStep | null;
  back: HTMLElement;
  busy: boolean;
}

let run: Run | null = null;

export async function runAction(plugin: string, action: string, items: string[], placement: string): Promise<void> {
  closeRun();
  const back = document.createElement("div");
  back.className = "modal-back";
  back.innerHTML = `<div class="modal plugin-modal" role="dialog" aria-modal="true"><p class="muted">${esc(t("plugin.working"))}</p></div>`;
  document.body.append(back);
  back.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && !run?.busy) closeRun();
  });
  run = { plugin, action, items, placement, step: null, back, busy: false };
  await step(null);
}

function closeRun(): void {
  run?.back.remove();
  run = null;
}

/** Corre un paso (`button` null = abrir) y dibuja lo que vuelve. */
async function step(buttonId: string | null, values: Record<string, unknown> = {}): Promise<void> {
  const r = run;
  if (!r) return;
  r.busy = true;
  r.back.querySelectorAll<HTMLButtonElement>("button").forEach((b) => (b.disabled = true));
  const state = r.step && "state" in r.step ? r.step.state : undefined;
  try {
    const { cols, rows } = termSize();
    const out = await pluginApi.action({
      plugin: r.plugin, action: r.action, items: r.items, placement: r.placement, cols, rows,
      ...(buttonId === null ? {} : { button: buttonId, values, state: state ?? null }),
    });
    if (run !== r) return;
    r.busy = false;
    await show(out);
  } catch (err) {
    if (run !== r) return;
    r.busy = false;
    if (!r.step) {
      closeRun();
      toast(errorText(err), undefined, "error");
      return;
    }
    r.back.querySelectorAll<HTMLButtonElement>("button").forEach((b) => (b.disabled = false));
    const box = $(".plugin-error", r.back);
    if (box) {
      box.textContent = errorText(err);
      box.hidden = false;
    } else toast(errorText(err), undefined, "error");
  }
}

async function show(s: PluginStep): Promise<void> {
  const r = run!;
  if (s.type === "session") {
    closeRun();
    await launched(s.session);
    toast(s.message || t("plugin.sessionStarted"));
    return;
  }
  if (s.type === "close") {
    closeRun();
    if (s.message) toast(s.message);
    await refresh();
    return;
  }
  r.step = s;
  const modal = $(".plugin-modal", r.back)!;
  if (s.type === "result") {
    modal.innerHTML = `<h2>${esc(s.title ?? "")}</h2>${s.message ? `<p>${esc(s.message)}</p>` : ""}
      ${s.steps?.length ? `<ul class="plugin-steps">${s.steps.map((x) => `<li class="${x.ok ? "ok" : "err"}">${ico(x.ok ? "check" : "x")}<div>${esc(x.label)}${x.error ? `<div class="flag">${esc(x.error)}</div>` : ""}</div></li>`).join("")}</ul>` : ""}
      <div class="plugin-error flag" hidden></div>
      <div class="modal-actions">${(s.links ?? []).map((l) => `<button class="btn" data-url="${esc(l.url)}">${ico("external")}${esc(l.label)}</button>`).join("")}
        <span class="spacer"></span>${buttons(s.buttons ?? [])}<button class="btn primary" data-close>${esc(t("common.close"))}</button></div>`;
    wire(modal, s.buttons ?? [], []);
    await refresh();
    return;
  }
  modal.innerHTML = `<form novalidate><h2>${esc(s.title ?? "")}</h2>${s.intro ? `<p class="muted">${esc(s.intro)}</p>` : ""}
    <div class="plugin-error flag"${s.error ? "" : " hidden"}>${esc(s.error ?? "")}</div>
    <div class="plugin-fields">${s.fields.map(field).join("")}</div>
    <div class="modal-actions"><span class="spacer"></span><button type="button" class="btn" data-close>${esc(t("common.cancel"))}</button>${buttons(s.buttons ?? [])}</div></form>`;
  wire(modal, s.buttons ?? [], s.fields);
  modal.querySelector<HTMLElement>("input:not([type=checkbox]), textarea, select")?.focus();
}

function buttons(list: PluginButton[]): string {
  return list.map((b) => `<button type="button" class="btn${b.primary ? " primary" : ""}${b.danger ? " danger" : ""}" data-btn="${esc(b.id)}">${esc(b.label)}</button>`).join("");
}

function field(f: PluginField): string {
  if (f.type === "info") return `<p class="muted plugin-info${f.wide === false ? "" : " wide"}">${esc(f.text ?? f.label ?? "")}</p>`;
  const id = esc(f.id ?? "");
  const v = f.value ?? "";
  const attrs = `data-field="${id}" data-type="${f.type}"${f.refresh ? " data-refresh" : ""}${f.placeholder ? ` placeholder="${esc(f.placeholder)}"` : ""}`;
  let control: string;
  if (f.type === "textarea") control = `<textarea ${attrs} rows="${f.rows ?? 4}" spellcheck="false">${esc(String(v))}</textarea>`;
  else if (f.type === "select")
    control = `<select ${attrs}>${(f.options ?? []).map((o) => `<option value="${esc(o.value)}"${String(v) === o.value ? " selected" : ""}>${esc(o.label)}</option>`).join("")}</select>`;
  else if (f.type === "checkbox")
    return `<label class="check${f.wide ? " wide" : ""}" title="${esc(f.hint ?? "")}"><input type="checkbox" ${attrs}${v === true ? " checked" : ""}>${esc(f.label ?? "")}</label>`;
  else {
    const type = f.type === "number" ? "number" : f.type === "password" ? "password" : "text";
    const value = f.type === "password" ? "" : String(v);
    control = `<input type="${type}" ${attrs} value="${esc(value)}"${f.type === "password" ? ' autocomplete="off"' : ""} spellcheck="false">`;
  }
  return `<label class="plugin-field${f.wide ? " wide" : ""}">${esc(f.label ?? "")}${f.required ? " *" : ""}${control}${f.hint ? `<span class="muted small">${esc(f.hint)}</span>` : ""}</label>`;
}

function values(modal: HTMLElement): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  modal.querySelectorAll<HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement>("[data-field]").forEach((el) => {
    const id = el.dataset.field!;
    const type = el.dataset.type;
    if (type === "checkbox") out[id] = (el as HTMLInputElement).checked;
    else if (type === "number") out[id] = el.value.trim() === "" ? null : Number(el.value);
    else if (type === "password") {
      if (el.value) out[id] = el.value;
    } else out[id] = el.value;
  });
  return out;
}

function wire(modal: HTMLElement, list: PluginButton[], fields: PluginField[]): void {
  modal.querySelectorAll<HTMLElement>("[data-close]").forEach((b) => (b.onclick = () => closeRun()));
  modal.querySelectorAll<HTMLElement>("[data-url]").forEach((b) => {
    b.onclick = () => pluginApi.openUrl(b.dataset.url!).catch((err) => toast(errorText(err), undefined, "error"));
  });
  modal.querySelectorAll<HTMLElement>("[data-refresh]").forEach((el) => {
    el.addEventListener("change", () => step("$refresh", values(modal)));
  });
  modal.querySelector("form")?.addEventListener("submit", (e) => {
    e.preventDefault();
    const primary = list.find((b) => b.primary);
    if (primary) press(modal, primary, fields);
  });
  modal.querySelectorAll<HTMLElement>("[data-btn]").forEach((b) => {
    const def = list.find((x) => x.id === b.dataset.btn)!;
    b.onclick = () => press(modal, def, fields);
  });
}

async function press(modal: HTMLElement, b: PluginButton, fields: PluginField[]): Promise<void> {
  const v = values(modal);
  if (b.validate !== false) {
    const missing = fields.find((f) => f.required && f.id && (v[f.id] === "" || v[f.id] === null || v[f.id] === undefined || v[f.id] === false));
    if (missing) {
      const box = $(".plugin-error", modal)!;
      box.textContent = t("plugin.required", { field: missing.label ?? missing.id ?? "" });
      box.hidden = false;
      modal.querySelector<HTMLElement>(`[data-field="${CSS.escape(missing.id!)}"]`)?.focus();
      return;
    }
  }
  if (b.confirm && !(await confirmDialog(b.confirm, b.label))) return;
  await step(b.id, v);
}

onAction("plugin-run", (el) => runAction(el.dataset.plugin!, el.dataset.act!, JSON.parse(el.dataset.items || "[]") as string[], el.dataset.placement ?? "item"));
onChange("plugin-enable", async (el) => {
  await setPluginEnabled(el.dataset.id!, (el as HTMLInputElement).checked);
  // Un plugin aporta (o deja de aportar) presets de flujo.
  S.presets = await api.workflowPresets();
  render();
});
onAction("plugins-folder", () => {
  pluginApi.openFolder().catch((err) => toast(errorText(err), undefined, "error"));
});
onAction("plugins-reload", async () => {
  await loadPlugins();
  S.presets = await api.workflowPresets();
  render();
});
