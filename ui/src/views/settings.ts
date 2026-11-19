// Ajustes (proyecto, flujo, idioma, tema, notas, acerca de) y el selector de carpeta de trabajos.

import { api, inTauri, pickFolder } from "../api";
import { applyTheme, onAction, onChange, refresh, registerProject, registerView, reloadSettings, render } from "../app";
import { esc, toast } from "../dom";
import { errorText, label, LANGS, setLang, t } from "../i18n";
import { ico } from "../icons";
import { S, wf } from "../state";
import type { Settings, Theme } from "../types";
import { loadInbox } from "./tasks";
import { loadNotes } from "./notes";

function options(values: [string, string][], selected: string): string {
  return values.map(([v, text]) => `<option value="${esc(v)}"${v === selected ? " selected" : ""}>${esc(text)}</option>`).join("");
}

function projectPicker(): string {
  const recents = (S.settings?.recents ?? [])
    .map((r) => `<li class="link-row" data-action="open-recent" data-path="${esc(r)}">${ico("folder")}<span class="grow">${esc(r.split(/[\\/]/).pop())} <span class="muted">${esc(r)}</span></span>
      <button class="icon-btn" data-action="forget-recent" data-path="${esc(r)}" title="${esc(t("project.forget"))}">${ico("x")}</button></li>`)
    .join("");
  return `<section class="card project"><h2>${esc(t("project.open"))}</h2><p class="muted">${esc(t("project.explain"))}</p>
    <div class="add-row"><input id="root-path" placeholder="${esc(t("project.pathHint"))}" data-enter="open-path">
      ${inTauri ? `<button class="btn" data-action="browse-root">${ico("folder")}${esc(t("project.browse"))}</button>` : ""}
      <button class="btn primary" data-action="open-path">${esc(t("project.openBtn"))}</button></div>
    ${recents ? `<h3>${esc(t("project.recents"))}</h3><ul class="list">${recents}</ul>` : ""}</section>`;
}

function renderProject(): string {
  return `<div class="page"><h1>${esc(t("app.name"))}</h1>${S.loadError ? `<p class="error">${esc(S.loadError)}</p>` : ""}${projectPicker()}</div>`;
}

function renderSettings(): string {
  const s = S.settings!;
  const langs: [string, string][] = [["", t("settings.auto")], ...LANGS.map((l): [string, string] => [l, l === "en" ? "English" : "Español"])];
  const themes: [string, string][] = (["system", "light", "dark"] as const).map((th) => [th, t(`settings.theme.${th}`)]);
  const presets: [string, string][] = S.presets.map((p) => [p.id, label(p.name, p.id)]);
  const w = wf();
  const chips = (items: { id: string; labels: Record<string, string>; category?: string }[]) =>
    `<div class="chips">${items.map((it) => `<span class="chip" title="${esc(it.id)}">${it.category ? `<span class="dot ${it.category}"></span>` : ""}${esc(label(it.labels, it.id))}</span>`).join("")}</div>`;
  return `<div class="page"><h1>${esc(t("settings.title"))}</h1>
    ${projectPicker()}
    <section class="card"><h2>${esc(t("settings.appearance"))}</h2>
      <div class="set-row"><label for="set-lang">${esc(t("settings.language"))}</label><select id="set-lang" data-change="set-lang">${options(langs, s.lang ?? "")}</select></div>
      <div class="set-row"><label for="set-theme">${esc(t("settings.theme"))}</label><select id="set-theme" data-change="set-theme">${options(themes, s.theme)}</select></div>
    </section>
    <section class="card"><h2>${esc(t("settings.workflow"))}</h2>
      <div class="set-row"><label for="set-workflow">${esc(t("settings.workflow.preset"))}</label><select id="set-workflow" data-change="set-workflow">${options(presets, s.workflow)}</select></div>
      <div class="muted">${esc(t("workflow.states"))}</div>${chips(w.states)}
      <div class="muted">${esc(t("workflow.types"))}</div>${chips(w.types)}
      <div class="muted">${esc(t("workflow.environments"))}</div>${chips(w.environments)}
      <p class="muted">${esc(t("workflow.period", { day: w.period.startDay }))}</p></section>
    <section class="card"><h2>${esc(t("nav.notes"))}</h2>
      <div class="set-row"><label for="set-notes">${esc(t("settings.notesFolder"))}</label><input id="set-notes" data-change="set-notes" value="${esc(s.notesFolder ?? "")}" placeholder="${esc(t("settings.notesDefault"))}"></div></section>
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
  } catch (err) {
    S.loadError = errorText(err);
    render();
  }
}

registerView("settings", renderSettings);
registerProject(renderProject);

onChange("set-lang", (el) => update({ lang: (el as HTMLSelectElement).value || null }));
onChange("set-theme", (el) => update({ theme: (el as HTMLSelectElement).value as Theme }));
onChange("set-workflow", (el) => update({ workflow: (el as HTMLSelectElement).value }));
onChange("set-notes", (el) => update({ notesFolder: (el as HTMLInputElement).value.trim() || null }));
onAction("open-path", () => {
  const v = document.querySelector<HTMLInputElement>("#root-path")!.value.trim();
  if (v) return openRoot(v);
});
onAction("browse-root", async () => {
  const p = await pickFolder();
  if (p) await openRoot(p);
});
onAction("open-recent", (el, ev) => {
  if ((ev.target as HTMLElement).closest('[data-action="forget-recent"]')) return;
  return openRoot(el.dataset.path!);
});
onAction("forget-recent", async (el, ev) => {
  ev.stopPropagation();
  S.settings = await api.forgetRecent(el.dataset.path!);
  render();
});
onAction("project-menu", () => {
  S.view = "settings";
  S.detail = null;
  render();
  document.querySelector<HTMLInputElement>("#root-path")?.focus();
});
