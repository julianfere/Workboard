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

export interface Workflow {
  id: string;
  name: Labels;
  states: StateDef[];
  types: Choice[];
  environments: Choice[];
  period: { startDay: number };
  folders: { itemPattern: string; itemIdPattern: string; groupPrefix: string; deletedDir: string; ignorePrefixes: string[] };
  fields: Record<string, unknown>;
  tasks: { heading: string; maxLen: number };
  plan: { heading: string; placeholder: string; minChars: number };
  files: { kinds: string[]; categories: FileCategory[]; fallbackCategory: FileCategory };
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
}
