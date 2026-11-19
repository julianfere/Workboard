// Arranque: carga ajustes y datos, instala eventos y atajos, y empieza el refresco en vivo.

import "./styles.css";
import { api } from "./api";
import { applyTheme, closeDetail, goView, installEvents, NAV, refresh, render, startPolling } from "./app";
import { esc } from "./dom";
import { errorText, setLang, t } from "./i18n";
import { openPalette } from "./palette";
import { loadSessions, startSessionPolling } from "./sessions";
import { S } from "./state";
import { installTerminalPanel, retheme, toggleTerminal } from "./terminal";
import "./views/alerts";
import "./views/detail";
import "./views/items";
import { loadNotes } from "./views/notes";
import "./views/settings";
import { loadInbox } from "./views/tasks";
import "./views/today";

installEvents();
installTerminalPanel();
// La terminal no lee variables CSS: se le vuelve a pasar el tema cuando cambia.
new MutationObserver(retheme).observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });

document.addEventListener("keydown", (ev) => {
  const target = ev.target as HTMLElement;
  const typing = target.matches("input, textarea, select");
  if ((ev.ctrlKey || ev.metaKey) && ev.key.toLowerCase() === "k") {
    ev.preventDefault();
    openPalette();
    return;
  }
  if ((ev.ctrlKey || ev.metaKey) && ev.key.toLowerCase() === "j") {
    ev.preventDefault();
    toggleTerminal();
    return;
  }
  // Dentro de la terminal las teclas son del agente.
  if (target.closest("#term-body")) return;
  if (document.querySelector(".modal-back")) return;
  if (ev.key === "F5") {
    ev.preventDefault();
    refresh();
    return;
  }
  if (typing || ev.ctrlKey || ev.metaKey || ev.altKey) return;
  if (ev.key === "Escape" && S.detail) closeDetail();
  else if (ev.key === "/") {
    const search = document.querySelector<HTMLInputElement>("#items-search, #notes-search");
    if (search) {
      ev.preventDefault();
      search.focus();
    }
  } else {
    const n = Number(ev.key);
    if (n >= 1 && n <= NAV.length) goView(NAV[n - 1].id);
  }
});

async function boot(): Promise<void> {
  try {
    [S.settings, S.presets, S.info, S.workflow] = await Promise.all([api.getSettings(), api.workflowPresets(), api.appInfo(), api.workflow()]);
    setLang(S.settings.lang);
    applyTheme(S.settings.theme);
    if (S.settings.root) {
      await Promise.all([refresh(), loadInbox(), loadNotes(), loadSessions()]);
    }
  } catch (err) {
    S.loadError = errorText(err);
    setLang(null);
  }
  if (!S.settings) {
    document.querySelector("#main")!.innerHTML = `<div class="page"><h1>${esc(t("app.name"))}</h1><p class="error">${esc(S.loadError)}</p></div>`;
    return;
  }
  render();
  startPolling();
  startSessionPolling();
}

boot();
