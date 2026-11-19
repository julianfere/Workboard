// Reflexionar: una sesión en modo plan lee las sesiones de unos items (métricas y transcripts) y propone
// mejoras para las instrucciones, skills, tools o la app. Las deja en una nota; no cambia nada.

import { api } from "./api";
import { confirmDialog, toast } from "./dom";
import { t } from "./i18n";
import { launched } from "./sessions";
import { S, inSession, stateDef } from "./state";
import { termSize } from "./terminal";

/** ¿Tiene sentido reflexionar sobre el item? Terminado o en revisión y con alguna sesión. */
export function canReflect(name: string, status: string): boolean {
  const cat = stateDef(status)?.category ?? "";
  return ["review", "done", "cancelled"].includes(cat) && S.sessions.some((s) => inSession(s, name));
}

export const hasSessions = (names: string[]) => names.some((n) => S.sessions.some((s) => inSession(s, n)));

export async function reflectItems(names: string[]): Promise<void> {
  const text = names.length === 1 ? t("reflect.confirmOne") : t("reflect.confirmMany", { n: names.length });
  if (!(await confirmDialog(text, t("reflect.action")))) return;
  const { cols, rows } = termSize();
  const out = await api.reflectItems(names, cols, rows);
  await launched(out.session);
  toast(t("reflect.started"));
}
