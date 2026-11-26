// Arranque: carga ajustes y datos, instala eventos y atajos, y empieza el refresco en vivo.

import "./styles.css";
import { api } from "./api";
import { applyTheme, closeDetail, goView, installEvents, NAV, onFsRefresh, refresh, refreshAll, render, startPolling } from "./app";
import { esc } from "./dom";
import { errorText, setLang, t } from "./i18n";
import { loadDiscreet, toggleDiscreet } from "./discreet";
import { openPalette } from "./palette";
import "./projects";
import { loadSessions, startSessionPolling, toggleDock } from "./sessions";
import { S } from "./state";
import { installTerminalPanel, retheme } from "./terminal";
import "./views/alerts";
import "./views/detail";
import "./views/hours";
import { clearSelection, deleteSelection, selectAllVisible, selectionSize, switchSavedView } from "./views/items";
import "./views/metrics";
import { loadNotes, noteEditing } from "./views/notes";
import "./views/settings";
import { loadInbox } from "./views/tasks";
import "./views/today";
import "./views/tools";
import { loadPlugins } from "./plugins";

installEvents();
// Un cambio en disco relee también la bandeja, las notas (no mientras se edita una) y las sesiones.
onFsRefresh(loadInbox);
onFsRefresh(async () => {
  if (!noteEditing()) await loadNotes();
});
onFsRefresh(loadSessions);
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
    toggleDock();
    return;
  }
  // Dentro de la terminal las teclas son del agente.
  if (target.closest("#term-body")) return;
  if (document.querySelector(".modal-back")) return;
  // `Ctrl+Shift+H`: modo discreto (oculta horas pactadas y trabajadas).
  if ((ev.ctrlKey || ev.metaKey) && ev.shiftKey && !ev.altKey && ev.key.toLowerCase() === "h") {
    ev.preventDefault();
    void toggleDiscreet();
    return;
  }
  if (ev.key === "F5") {
    ev.preventDefault();
    void refreshAll();
    return;
  }
  const ctrl = ev.ctrlKey || ev.metaKey;
  // El detalle se guarda al cambiar cada campo: `Ctrl+S` confirma el que se está editando.
  if (ctrl && ev.key.toLowerCase() === "s" && S.detail) {
    ev.preventDefault();
    if (typing) target.blur();
    return;
  }
  const inGrid = S.view === "items" && !S.detail;
  // `Alt+1`…`Alt+9`: vistas guardadas de Trabajos.
  if (ev.altKey && !ctrl && inGrid && /^Digit[1-9]$/.test(ev.code)) {
    ev.preventDefault();
    switchSavedView(Number(ev.code.slice(5)) - 1);
    return;
  }
  if (typing) return;
  if (ctrl && !ev.altKey && ev.key.toLowerCase() === "a" && inGrid) {
    ev.preventDefault();
    selectAllVisible();
    return;
  }
  if (ev.ctrlKey || ev.metaKey || ev.altKey) return;
  if (ev.key === "Delete" && inGrid && selectionSize()) {
    ev.preventDefault();
    void deleteSelection();
    return;
  }
  if (ev.key === "Escape" && S.detail) closeDetail();
  else if (ev.key === "Escape" && inGrid) clearSelection();
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
    [S.settings, S.presets, S.info, S.workflow] = await Promise.all([api.getSettings(), api.workflowPresets(), api.appInfo(), api.workflow(), loadPlugins(), loadDiscreet()]);
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
