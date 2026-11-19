// Tipos de lo que devuelve el backend (serde camelCase de los structs de Rust).

export type Labels = Record<string, string>;

export type StateCategory = "backlog" | "planned" | "active" | "review" | "done" | "cancelled" | "unknown";

export interface StateDef {
  id: string;
  category: StateCategory;
  labels: Labels;
}

export interface Choice {
  id: string;
  labels: Labels;
}

export interface FileCategory {
  id: string;
  runnable: boolean;
  kinds?: string[];
  extensions?: string[];
  labels: Labels;
}

export interface FieldKeys {
  status: string;
  type: string;
  environments: string;
  itemIds: string;
  notes: string;
  branch: string;
  title: string;
  due: string;
  period: string;
  hours: string;
  sessions: string;
  extra: { key: string; noSpaces?: boolean }[];
}

export interface Workflow {
  id: string;
  name: Labels;
  states: StateDef[];
  types: Choice[];
  environments: Choice[];
  period: { startDay: number };
  folders: { itemPattern: string; itemIdPattern: string; newItemTemplate: string; groupPrefix: string; deletedDir: string; ignorePrefixes: string[] };
  fields: FieldKeys;
  tasks: { heading: string; maxLen: number };
  plan: { heading: string; placeholder: string; minChars: number };
  files: { kinds: string[]; categories: FileCategory[]; fallbackCategory: FileCategory };
}

export type MetaValue = string | string[];

export interface Task {
  text: string;
  done: boolean;
}

export interface FileEntry {
  path: string;
  name: string;
  size: number;
  mtime: number;
  kind: string | null;
  cat: string;
  text: boolean;
  media: string | null;
}

export interface Item {
  name: string;
  kind: "item" | "group";
  group: string;
  members: string[];
  ids: string[];
  title: string;
  type: string;
  meta: Record<string, MetaValue>;
  hasReadme: boolean;
  deploy: string;
  tasks: Task[];
  planGaps: ("plan" | "tasks")[];
  created: number;
  last: number;
  size: number;
  files: FileEntry[];
  kinds: string[];
  dupes?: string[];
}

export interface ItemList {
  root: string;
  items: Item[];
  deleted: number;
  signature: [number, number];
}

export interface ItemDetail {
  item: Item;
  readme: string;
  runs: Record<string, Record<string, number>>;
}

export interface Version {
  id: string;
  ts: number;
  size: number;
}

export interface DeletedEntry {
  path: string;
  base: string;
  title: string;
  status: string;
  deleted: number;
  group: boolean;
}

export interface Note {
  id: string;
  title: string;
  items: string[];
  author: string;
  mtime: number;
  excerpt: string;
  body?: string;
}

export interface PresetSummary {
  id: string;
  name: Labels;
}

export type Theme = "system" | "light" | "dark";

export interface Settings {
  lang: string | null;
  theme: Theme;
  workflow: string;
  root: string | null;
  recents: string[];
  notesFolder: string | null;
}

export interface AppInfo {
  version: string;
  os: string;
  configDir: string;
}

/** Error de un comando: código = clave de traducción. */
export interface ApiError {
  code: string;
  params: Record<string, string>;
  conflict?: boolean;
}
