// Estado de la UI y helpers del flujo de trabajo (qué clave del frontmatter es cada dato).

import { label } from "./i18n";
import type { AppInfo, Item, ItemDetail, MetaValue, Note, PresetSummary, Session, Settings, StateDef, Task, TrackerWorkItem, Workflow } from "./types";

export type DetailTab = "summary" | "readme" | "files" | "git" | "deploy" | "notes" | "versions";

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
  detailTab: "summary" as DetailTab,
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
  /** Búsqueda global (Trabajos, Alertas y Pendientes; no se guarda). */
  search: "",
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

/** Lo último leído del tracker (la caché del sync). Lo conecta `views/grid.ts` con la config del tracker, para que
 *  este módulo no dependa de las vistas. */
export const trackerLookup = {
  item: (_id: string): TrackerWorkItem | undefined => undefined,
  /** Tipo del tracker → tipo del flujo ("" = sin equivalente). */
  localType: (_type: string): string => "",
};

/** El work item del tracker de un item (el de su primer id, como la v1); los grupos no tienen. */
export const trackerItemOf = (it: Item): TrackerWorkItem | undefined => (it.kind === "group" || !it.ids.length ? undefined : trackerLookup.item(it.ids[0]));

/** Título para mostrar: el del tracker (último sync), si no el del README, si no la carpeta (v1 `displayTitle`). */
export const titleOf = (it: Item) => trackerItemOf(it)?.title || it.title || it.name.split("/").pop()!;

/** Tipo del flujo según el tracker ("" si no hay datos o no tiene equivalente). */
export const trackerTypeOf = (it: Item) => {
  const w = trackerItemOf(it);
  return w?.type ? trackerLookup.localType(w.type) : "";
};

/** Tipo efectivo: el del README; si falta, el del tracker (v1 `tipoOf`). */
export const typeOf = (it: Item) => it.type || trackerTypeOf(it);

/** Ícono del tipo (sale del flujo). */
export const typeIcon = (id: string) => wf().types.find((x) => x.id === id)?.icon ?? "";

/** Búsqueda global: todas las palabras en carpeta, títulos (README y tracker), ids, resumen, rama, estado (local y del
 *  tracker), archivos o tareas (v1 `matches`). */
export function matchesSearch(it: Item, q = S.search): boolean {
  const words = q.toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return true;
  const f = wf().fields;
  const w = trackerItemOf(it);
  const hay = [it.name, it.title, w?.title, w?.state, it.ids.join(" "), metaText(it, f.notes), metaText(it, f.branch), stateLabel(effectiveStatus(it)), ...it.files.map((x) => x.name), ...it.tasks.map((x) => x.text)]
    .join(" ")
    .toLowerCase();
  return words.every((x) => hay.includes(x));
}

/** Nombre para mostrar de una carpeta de trabajos: su alias (`settings.aliases`) o el nombre de la carpeta. */
export function folderName(path: string): string {
  const low = path.toLowerCase();
  const alias = Object.entries(S.settings?.aliases ?? {}).find(([k]) => k.toLowerCase() === low)?.[1];
  return alias || path.split(/[\\/]/).filter(Boolean).pop() || path;
}
