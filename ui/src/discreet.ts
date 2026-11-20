// Modo discreto (`Ctrl+Shift+H` o Ajustes → Apariencia): oculta las horas pactadas y trabajadas de toda la UI y
// la vista Horas. Se guarda en las prefs de la UI (`hideHours`). Se resuelve con una clase en `<html>`: lo que
// muestra horas lleva la clase `hrs` (o, en el detalle, el campo de horas del flujo, por una regla armada acá).

import { api } from "./api";
import { onAfterRender, onChange, render } from "./app";
import { toast } from "./dom";
import { errorText, t } from "./i18n";
import { S } from "./state";

const PREF = "hideHours";
let hidden = false;
let ruleKey = "";

export const hoursHidden = () => hidden;

/** La regla del campo de horas del detalle depende de la clave del flujo (`wf().fields.hours`). */
function syncRule(): void {
  const key = S.workflow?.fields.hours ?? "";
  if (key === ruleKey) return;
  ruleKey = key;
  let style = document.querySelector<HTMLStyleElement>("#discreet-rule");
  if (!style) {
    style = document.createElement("style");
    style.id = "discreet-rule";
    document.head.append(style);
  }
  style.textContent = key ? `.discreet label.field:has(> input[data-key="${CSS.escape(key)}"]) { display: none; }` : "";
}

function apply(): void {
  document.documentElement.classList.toggle("discreet", hidden);
  syncRule();
}

export async function loadDiscreet(): Promise<void> {
  try {
    hidden = (await api.uiPrefs())[PREF] === true;
  } catch {
    hidden = false;
  }
  apply();
}

export async function setDiscreet(on: boolean): Promise<void> {
  hidden = on;
  apply();
  if (hidden && S.view === "hours" && !S.detail) S.view = "today";
  render();
  toast(t(hidden ? "discreet.on" : "discreet.off"));
  try {
    await api.setUiPref(PREF, hidden ? true : null);
  } catch (err) {
    toast(errorText(err), undefined, "error");
  }
}

export const toggleDiscreet = () => setDiscreet(!hidden);

onAfterRender(syncRule);
// Ajustes → Apariencia.
onChange("set-discreet", (el) => setDiscreet((el as HTMLInputElement).checked));
