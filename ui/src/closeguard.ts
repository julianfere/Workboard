// Cerrar la app con cambios sin guardar: cada módulo registra una función que devuelve el aviso (o null).
// En la app se intercepta el cierre de la ventana y se confirma con el diálogo propio; en el navegador, el
// aviso estándar de `beforeunload` (el texto lo pone el navegador).

import { inTauri } from "./api";
import { confirmDialog } from "./dom";
import { t } from "./i18n";

const guards: (() => string | null)[] = [];
let installed = false;

/** Primer aviso pendiente de los módulos registrados (null = se puede cerrar). */
function pending(): string | null {
  for (const g of guards) {
    const msg = g();
    if (msg) return msg;
  }
  return null;
}

export function registerCloseGuard(fn: () => string | null): void {
  guards.push(fn);
  if (installed) return;
  installed = true;
  window.addEventListener("beforeunload", (ev) => {
    if (!pending()) return;
    ev.preventDefault();
    ev.returnValue = "";
  });
  if (!inTauri) return;
  void import("@tauri-apps/api/window").then(({ getCurrentWindow }) => {
    const win = getCurrentWindow();
    void win.onCloseRequested(async (ev) => {
      const msg = pending();
      if (!msg) return; // sin preventDefault: la ventana se cierra
      ev.preventDefault();
      if (await confirmDialog(msg, t("close.discard"))) await win.destroy();
    });
  });
}
