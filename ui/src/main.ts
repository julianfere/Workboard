// Shell de la app: navegación, tema, idioma y la vista de Ajustes con el flujo de trabajo. Las demás
// vistas se portan en las próximas fases.

import "./styles.css";
import { api } from "./api";
import { errorText, label, LANGS, setLang, t } from "./i18n";
import { ico } from "./icons";
import type { AppInfo, PresetSummary, Settings, Theme, Workflow } from "./types";

const VIEWS = [
  { id: "today", icon: "sun" },
  { id: "items", icon: "list" },
  { id: "tasks", icon: "tasks" },
  { id: "hours", icon: "clock" },
  { id: "alerts", icon: "alert" },
  { id: "notes", icon: "note" },
  { id: "metrics", icon: "layers" },
  { id: "tools", icon: "wrench" },
] as const;
type ViewId = (typeof VIEWS)[number]["id"] | "settings";

const S = {
  view: "settings" as ViewId,
  settings: null as Settings | null,
  workflow: null as Workflow | null,
  presets: [] as PresetSummary[],
  info: null as AppInfo | null,
  error: "",
};

const $ = <T extends HTMLElement>(sel: string) => document.querySelector(sel) as T;
const esc = (s: string) => s.replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);

// ---------- tema ----------

const darkQuery = window.matchMedia("(prefers-color-scheme: dark)");

function applyTheme(theme: Theme): void {
  const dark = theme === "dark" || (theme === "system" && darkQuery.matches);
  document.documentElement.dataset.theme = dark ? "dark" : "light";
}
darkQuery.addEventListener("change", () => S.settings && applyTheme(S.settings.theme));

// ---------- render ----------

function renderNav(): void {
  const item = (id: ViewId, icon: string, key: string) =>
    `<button class="nav-item${S.view === id ? " sel" : ""}" data-view="${id}">${ico(icon)}<span>${esc(t(`nav.${id}`))}</span><span class="nav-key">${key}</span></button>`;
  $("#sidenav").innerHTML =
    `<div class="brand"><span class="brand-icon">${ico("layers")}</span>${esc(t("app.name"))}</div>` +
    VIEWS.map((v, i) => item(v.id, v.icon, String(i + 1))).join("") +
    `<div class="nav-spacer"></div>` +
    item("settings", "settings", "");
}

function renderMain(): void {
  const main = $("#main");
  if (S.view !== "settings") {
    main.innerHTML = `<div class="page"><h1>${esc(t(`nav.${S.view}`))}</h1><p class="placeholder">${esc(
      t("view.placeholder", { view: t(`nav.${S.view}`) }),
    )}</p></div>`;
    return;
  }
  main.innerHTML = `<div class="page"><h1>${esc(t("settings.title"))}</h1>
    ${S.error ? `<p class="error">${esc(S.error)}</p>` : ""}
    ${settingsCard()}${workflowCard()}${aboutCard()}</div>`;
}

function options(values: [string, string][], selected: string): string {
  return values
    .map(([v, text]) => `<option value="${esc(v)}"${v === selected ? " selected" : ""}>${esc(text)}</option>`)
    .join("");
}

function settingsCard(): string {
  const s = S.settings!;
  const langs: [string, string][] = [["", "Auto"], ...LANGS.map((l): [string, string] => [l, l === "en" ? "English" : "Español"])];
  const themes: [string, string][] = (["system", "light", "dark"] as const).map((th) => [th, t(`settings.theme.${th}`)]);
  const presets: [string, string][] = S.presets.map((p) => [p.id, label(p.name, p.id)]);
  return `<section class="card"><h2>${esc(t("settings.appearance"))}</h2>
    <div class="row"><label for="set-lang">${esc(t("settings.language"))}</label><select id="set-lang">${options(langs, s.lang ?? "")}</select></div>
    <div class="row"><label for="set-theme">${esc(t("settings.theme"))}</label><select id="set-theme">${options(themes, s.theme)}</select></div>
    <div class="row"><label for="set-workflow">${esc(t("settings.workflow.preset"))}</label><select id="set-workflow">${options(presets, s.workflow)}</select></div>
  </section>`;
}

function workflowCard(): string {
  const wf = S.workflow;
  if (!wf) return "";
  const chips = (items: { id: string; labels: Record<string, string>; category?: string }[]) =>
    `<div class="chips">${items
      .map((it) => `<span class="chip" title="${esc(it.id)}">${it.category ? `<span class="dot ${it.category}"></span>` : ""}${esc(label(it.labels, it.id))}</span>`)
      .join("")}</div>`;
  return `<section class="card"><h2>${esc(t("settings.workflow"))} · ${esc(label(wf.name, wf.id))}</h2>
    <div class="muted">${esc(t("workflow.states"))}</div>${chips(wf.states)}
    <div class="muted">${esc(t("workflow.types"))}</div>${chips(wf.types)}
    <div class="muted">${esc(t("workflow.environments"))}</div>${chips(wf.environments)}
    <p class="muted">${esc(t("workflow.period", { day: wf.period.startDay }))}</p>
  </section>`;
}

function aboutCard(): string {
  const i = S.info;
  if (!i) return "";
  return `<section class="card"><h2>${esc(t("settings.about"))}</h2>
    <p class="muted">${esc(t("about.version", { version: i.version, os: i.os }))}<br><code>${esc(i.configDir)}</code></p></section>`;
}

function render(): void {
  renderNav();
  renderMain();
}

// ---------- acciones ----------

async function updateSettings(patch: Partial<Settings>): Promise<void> {
  try {
    S.settings = await api.setSettings({ ...S.settings!, ...patch });
    S.error = "";
    if ("workflow" in patch) S.workflow = await api.workflowPreset(S.settings.workflow);
    setLang(S.settings.lang);
    applyTheme(S.settings.theme);
  } catch (err) {
    S.error = errorText(err);
  }
  render();
}

function goView(view: ViewId): void {
  S.view = view;
  render();
}

document.addEventListener("click", (ev) => {
  const btn = (ev.target as HTMLElement).closest<HTMLElement>("[data-view]");
  if (btn) goView(btn.dataset.view as ViewId);
});

document.addEventListener("change", (ev) => {
  const el = ev.target as HTMLSelectElement;
  if (el.id === "set-lang") updateSettings({ lang: el.value || null });
  else if (el.id === "set-theme") updateSettings({ theme: el.value as Theme });
  else if (el.id === "set-workflow") updateSettings({ workflow: el.value });
});

document.addEventListener("keydown", (ev) => {
  const target = ev.target as HTMLElement;
  if (ev.ctrlKey || ev.metaKey || ev.altKey || target.matches("input, textarea, select")) return;
  const n = Number(ev.key);
  if (n >= 1 && n <= VIEWS.length) goView(VIEWS[n - 1].id);
});

// ---------- arranque ----------

async function boot(): Promise<void> {
  try {
    [S.settings, S.presets, S.info] = await Promise.all([api.getSettings(), api.workflowPresets(), api.appInfo()]);
    setLang(S.settings.lang);
    applyTheme(S.settings.theme);
    S.workflow = await api.workflowPreset(S.settings.workflow);
  } catch (err) {
    S.error = errorText(err);
    S.settings ??= { lang: null, theme: "system", workflow: "default" };
  }
  render();
  document.title = t("app.name");
}

boot();
