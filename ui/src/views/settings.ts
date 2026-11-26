// Ajustes (proyecto, flujo, idioma, tema, notas, acerca de) y el selector de carpeta de trabajos.

import { api, inTauri, pickFolder } from "../api";
import { applyTheme, onAction, onChange, refresh, registerProject, registerView, reloadSettings, render } from "../app";
import { esc, toast } from "../dom";
import { errorText, label, LANGS, setLang, t } from "../i18n";
import { ico } from "../icons";
import { S, wf } from "../state";
import type { Settings, Theme } from "../types";
import { agentLabel, agentsInfo, loadSessions } from "../sessions";
import type { AgentsInfo } from "../types";
import { loadInbox } from "./tasks";
import { hoursHidden } from "../discreet";
import { loadNotes, setNotesFolder } from "./notes";
import { trackerSettingsCard } from "./tracker";
import { gitSettingsCard } from "../git";
import { pluginsSettingsCard } from "../plugins";
import { loadRecents, recentRows } from "../projects";

function options(values: [string, string][], selected: string): string {
  return values.map(([v, text]) => `<option value="${esc(v)}"${v === selected ? " selected" : ""}>${esc(text)}</option>`).join("");
}

function projectPicker(): string {
  const recents = S.settings?.recents.length ? recentRows(false) : "";
  return `<section class="card project"><h2>${esc(t("project.open"))}</h2><p class="muted">${esc(t("project.explain"))}</p>
    <div class="add-row"><input id="root-path" placeholder="${esc(t("project.pathHint"))}" data-enter="open-path">
      ${inTauri ? `<button class="btn" data-action="browse-root">${ico("folder")}${esc(t("project.browse"))}</button>` : ""}
      <button class="btn primary" data-action="open-path">${esc(t("project.openBtn"))}</button></div>
    ${S.settings?.root ? `<div class="add-row"><span class="muted root-path"><code>${esc(S.settings.root)}</code></span>
      <button class="btn" data-action="root-open">${ico("folder")}${esc(t("detail.openFolder"))}</button>
      <button class="btn" data-action="root-vscode">${ico("code")}${esc(t("detail.vscode"))}</button></div>` : ""}
    ${recents ? `<h3>${esc(t("project.recents"))}</h3><ul class="list">${recents}</ul>` : ""}</section>`;
}

function renderProject(): string {
  return `<div class="page"><h1>${esc(t("app.name"))}</h1>${S.loadError ? `<p class="error">${esc(S.loadError)}</p>` : ""}${projectPicker()}</div>`;
}

let agents: AgentsInfo | null = null;
let agentsLoading = false;

/** Agentes de las sesiones: el de por defecto, el comando propio y con qué arranca Planear. */
function agentsCard(): string {
  if (!agents) {
    if (!agentsLoading) {
      agentsLoading = true;
      agentsInfo(true)
        .then((a) => (agents = a))
        .catch(() => {})
        .finally(() => {
          agentsLoading = false;
          render();
        });
    }
    return `<section class="card"><h2>${esc(t("settings.agents"))}</h2><p class="muted">…</p></section>`;
  }
  const a = agents;
  const defaults: [string, string][] = a.agents.map((x) => [x.id, `${agentLabel(x)}${x.found ? "" : ` (${t("agent.notFound")})`}`]);
  const c = a.custom;
  const input = (key: string, value: string, hint = "") =>
    `<input data-change="set-agent-custom" data-key="${key}" value="${esc(value)}" placeholder="${esc(hint)}">`;
  const prefixes = a.agents
    .filter((x) => x.id !== "custom")
    .map((x) => `<div class="set-row"><label>${esc(t("settings.planPrefix", { agent: agentLabel(x) }))}</label><input data-change="set-plan-prefix" data-agent="${esc(x.id)}" value="${esc(a.planPrefix[x.id] ?? "")}" placeholder="${esc(t("settings.planPrefixHint"))}"></div>`)
    .join("");
  return `<section class="card"><h2>${esc(t("settings.agents"))}</h2>
    <div class="set-row"><label for="set-agent">${esc(t("settings.defaultAgent"))}</label><select id="set-agent" data-change="set-agent-default">${options(defaults, a.default)}</select></div>
    ${prefixes}
    <h3>${esc(t("agent.custom"))}</h3>
    <div class="set-row"><label>${esc(t("settings.customLabel"))}</label>${input("label", c.label)}</div>
    <div class="set-row"><label>${esc(t("settings.customCommand"))}</label>${input("command", c.command, t("settings.customCommandHint"))}</div>
    <div class="set-row"><label>${esc(t("settings.customArgs"))}</label>${input("args", c.args)}</div>
    <div class="set-row"><label>${esc(t("settings.customResumeArgs"))}</label>${input("resumeArgs", c.resumeArgs)}</div>
    <p class="muted">${esc(t("settings.placeholders"))} ${a.placeholders.map((p) => `<code>{${esc(p)}}</code>`).join(" ")}</p></section>`;
}

async function saveAgents(patch: Parameters<typeof api.setAgents>[0]): Promise<void> {
  try {
    agents = await api.setAgents(patch);
    S.settings = await api.getSettings();
  } catch (err) {
    toast(errorText(err), undefined, "error");
  }
  render();
}

function renderSettings(): string {
  const s = S.settings!;
  const langs: [string, string][] = [["", t("settings.auto")], ...LANGS.map((l): [string, string] => [l, l === "en" ? "English" : "Español"])];
  const themes: [string, string][] = (["system", "light", "dark"] as const).map((th) => [th, t(`settings.theme.${th}`)]);
  const presets: [string, string][] = S.presets.map((p) => [p.id, p.plugin ? t("plugin.presetFrom", { name: label(p.name, p.id), plugin: p.plugin }) : label(p.name, p.id)]);
  const w = wf();
  const chips = (items: { id: string; labels: Record<string, string>; category?: string }[]) =>
    `<div class="chips">${items.map((it) => `<span class="chip" title="${esc(it.id)}">${it.category ? `<span class="dot ${it.category}"></span>` : ""}${esc(label(it.labels, it.id))}</span>`).join("")}</div>`;
  return `<div class="page"><h1>${esc(t("settings.title"))}</h1>
    ${projectPicker()}
    <section class="card"><h2>${esc(t("settings.appearance"))}</h2>
      <div class="set-row"><label for="set-lang">${esc(t("settings.language"))}</label><select id="set-lang" data-change="set-lang">${options(langs, s.lang ?? "")}</select></div>
      <div class="set-row"><label for="set-theme">${esc(t("settings.theme"))}</label><select id="set-theme" data-change="set-theme">${options(themes, s.theme)}</select></div>
      <p class="muted">${esc(t("settings.shortcuts"))}</p>
      <div class="set-row"><label for="set-discreet">${esc(t("discreet.label"))}</label><label class="check"><input type="checkbox" id="set-discreet" data-change="set-discreet"${hoursHidden() ? " checked" : ""}>${esc(t("discreet.hint"))}</label></div>
    </section>
    <section class="card"><h2>${esc(t("settings.workflow"))}</h2>
      <div class="set-row"><label for="set-workflow">${esc(t("settings.workflow.preset"))}</label><select id="set-workflow" data-change="set-workflow">${options(presets, s.workflow)}</select></div>
      <div class="muted">${esc(t("workflow.states"))}</div>${chips(w.states)}
      <div class="muted">${esc(t("workflow.types"))}</div>${chips(w.types)}
      <div class="muted">${esc(t("workflow.environments"))}</div>${chips(w.environments)}
      <p class="muted">${esc(t("workflow.period", { day: w.period.startDay }))}</p></section>
    <section class="card"><h2>${esc(t("nav.notes"))}</h2>
      <div class="set-row"><label for="set-notes">${esc(t("settings.notesFolder"))}</label><div class="set-inline"><input id="set-notes" data-change="set-notes" value="${esc(s.notesFolder ?? "")}" placeholder="${esc(t("settings.notesDefault"))}">
        ${inTauri ? `<button class="btn" data-action="notes-folder" title="${esc(t("notes.changeFolderHint"))}">${ico("folder")}${esc(t("project.browse"))}</button>` : ""}
        ${s.notesFolder ? `<button class="btn ghost" data-action="notes-folder-reset">${esc(t("notes.useDefault"))}</button>` : ""}</div></div></section>
    ${trackerSettingsCard()}
    ${agentsCard()}
    ${gitSettingsCard()}
    ${pluginsSettingsCard()}
    ${S.info ? `<section class="card"><h2>${esc(t("settings.about"))}</h2><p class="muted">${esc(t("about.version", { version: S.info.version, os: S.info.os }))}<br><code>${esc(S.info.configDir)}</code></p></section>` : ""}
  </div>`;
}

async function update(patch: Partial<Settings>): Promise<void> {
  try {
    S.settings = await api.setSettings({ ...S.settings!, ...patch });
    if ("workflow" in patch) S.workflow = await api.workflow();
    setLang(S.settings.lang);
    applyTheme(S.settings.theme);
    if ("workflow" in patch) await refresh();
    if ("notesFolder" in patch) await loadNotes();
  } catch (err) {
    toast(errorText(err), undefined, "error");
  }
  render();
}

export async function openRoot(path: string): Promise<void> {
  try {
    await api.openRoot(path);
    await reloadSettings();
    S.detail = null;
    S.view = "items";
    await Promise.all([loadInbox(), loadNotes()]);
    await refresh();
    await loadSessions();
  } catch (err) {
    S.loadError = errorText(err);
    render();
  }
  await loadRecents();
}

registerView("settings", renderSettings);
registerProject(renderProject);

onChange("set-lang", (el) => update({ lang: (el as HTMLSelectElement).value || null }));
onChange("set-theme", (el) => update({ theme: (el as HTMLSelectElement).value as Theme }));
onChange("set-workflow", (el) => update({ workflow: (el as HTMLSelectElement).value }));
// La carpeta de notas pasa por `setNotesFolder`: confirma si la nota abierta tiene cambios sin guardar.
onChange("set-notes", (el) => setNotesFolder((el as HTMLInputElement).value.trim() || null));
onAction("open-path", () => {
  const v = document.querySelector<HTMLInputElement>("#root-path")!.value.trim();
  if (v) return openRoot(v);
});
onAction("root-open", () => void api.openPath("").catch((err) => toast(errorText(err), undefined, "error")));
onAction("root-vscode", () => void api.openInVscode("").catch((err) => toast(errorText(err), undefined, "error")));
onAction("browse-root", async () => {
  const p = await pickFolder();
  if (p) await openRoot(p);
});
onChange("set-agent-default", (el) => saveAgents({ default: (el as HTMLSelectElement).value }));
onChange("set-agent-custom", (el) => {
  const custom = { ...(agents?.custom ?? { label: "", command: "", args: "", resumeArgs: "" }) };
  (custom as Record<string, string>)[el.dataset.key!] = (el as HTMLInputElement).value;
  return saveAgents({ custom });
});
onChange("set-plan-prefix", (el) => {
  const planPrefix = { ...(agents?.planPrefix ?? {}), [el.dataset.agent!]: (el as HTMLInputElement).value };
  return saveAgents({ planPrefix });
});
