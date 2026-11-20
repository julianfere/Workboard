// Estado de la UI y helpers del flujo de trabajo (qué clave del frontmatter es cada dato).

import { label } from "./i18n";
import type { AppInfo, Item, ItemDetail, MetaValue, Note, PresetSummary, Session, Settings, StateDef, Task, Workflow } from "./types";

export type ViewId = "today" | "items" | "tasks" | "hours" | "alerts" | "notes" | "metrics" | "tools" | "settings";

export const S = {
  view: "items" as ViewId,
  settings: null as Settings | null,
  workflow: null as Workflow | null,
  presets: [] as PresetSummary[],
  info: null as AppInfo | null,
  root: "",
  items: [] as Item[],
  deletedCount: 0,
  signature: "",
  loadError: "",
  /** Detalle abierto (nombre del item) y sus datos. */
  detail: null as string | null,
  detailData: null as ItemDetail | null,
  detailTab: "summary" as "summary" | "readme" | "files" | "versions" | "git",
  /** Errores de validación por clave del frontmatter. */
  fieldErrors: {} as Record<string, string>,
  /** Lo que escribió el usuario en un campo que no pasó la validación (no se pierde al re-renderizar). */
  fieldDrafts: {} as Record<string, string>,
  inbox: [] as Task[],
  notes: [] as Note[],
  note: null as Note | null,
  noteFilter: "",
  /** Sesiones de la carpeta abierta y las que corren en otros proyectos. */
  sessions: [] as Session[],
  otherSessions: [] as Session[],
  sessionsRev: 0,
  /** Panel de terminales: visible, pestañas abiertas (ids) y la activa. */
  term: { open: false, tabs: [] as string[], active: null as string | null },
};

/** ¿El item es de la sesión (o de un grupo de la sesión)? */
export const inSession = (s: Session, name: string) => s.items.some((w) => name === w || name.startsWith(`${w}/`));

export function wf(): Workflow {
  return S.workflow!;
}

export function metaText(it: Item, key: string): string {
  const v: MetaValue | undefined = it.meta[key];
  return typeof v === "string" ? v : "";
}

export function metaList(it: Item, key: string): string[] {
  const v = it.meta[key];
  return Array.isArray(v) ? v : typeof v === "string" && v ? [v] : [];
}

export const statusOf = (it: Item) => metaText(it, wf().fields.status);
export const stateDef = (id: string): StateDef | undefined => wf().states.find((s) => s.id === id);
export const stateLabel = (id: string) => (stateDef(id) ? label(stateDef(id)!.labels, id) : id);
export const stateRank = (id: string) => {
  const i = wf().states.findIndex((s) => s.id === id);
  return i < 0 ? wf().states.length : i;
};
export const isClosed = (id: string) => ["done", "cancelled"].includes(stateDef(id)?.category ?? "");
export const typeLabel = (id: string) => {
  const t = wf().types.find((x) => x.id === id);
  return t ? label(t.labels, id) : id;
};
export const envLabel = (id: string) => {
  const e = wf().environments.find((x) => x.id === id);
  return e ? label(e.labels, id) : id;
};

/** Estado efectivo: el del README; en un grupo sin estado, el del item menos avanzado. */
export function effectiveStatus(it: Item): string {
  const own = statusOf(it);
  if (own || it.kind !== "group") return own;
  const members = it.members.map((m) => S.items.find((x) => x.name === m)).filter(Boolean) as Item[];
  const open = members.map(statusOf).filter((s) => s && !isClosed(s));
  return open.sort((a, b) => stateRank(a) - stateRank(b))[0] ?? "";
}

export const itemByName = (name: string) => S.items.find((i) => i.name === name);

/** Título para mostrar: el del README o la carpeta. */
export const titleOf = (it: Item) => it.title || it.name.split("/").pop()!;
