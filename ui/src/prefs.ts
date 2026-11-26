// Preferencias chicas de la UI (`<config>/ui_prefs.json`, como el `set_pref` de la v1): se leen una vez al
// arrancar y se guardan en segundo plano. Claves: `view` (última vista), `hours` y `metrics`.

import { api } from "./api";
import { toast } from "./dom";
import { errorText, t } from "./i18n";

let cache: Record<string, unknown> = {};
let errorShown = false;

export async function loadPrefs(): Promise<void> {
  try {
    cache = await api.uiPrefs();
  } catch {
    cache = {};
  }
}

export function pref<T>(key: string): T | undefined {
  return cache[key] as T | undefined;
}

/** Guarda una preferencia (`null` la borra). Un error se avisa una sola vez, como la v1. */
export function setPref(key: string, value: unknown): void {
  if (value === null) delete cache[key];
  else cache[key] = value;
  api.setUiPref(key, value).catch((err) => {
    if (errorShown) return;
    errorShown = true;
    toast(t("prefs.saveFailed", { message: errorText(err) }), undefined, "error");
  });
}
