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
  /** Otros nombres (por ejemplo, tipos de un tracker que corresponden a este). */
  aliases?: string[];
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
  /** Títulos de las secciones que se completan al crear un item desde un tracker. */
  sections: { description: string; criteria: string };
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
  agents: AgentSettings;
}

export interface CustomAgent {
  label: string;
  command: string;
  args: string;
  resumeArgs: string;
}

export interface AgentSettings {
  default: string;
  custom: CustomAgent;
  planPrefix: Record<string, string>;
}

// ---------- sesiones de agentes ----------

export type LiveStatus = "starting" | "working" | "needsYou" | "idle" | "ended" | "running" | "paused" | "closed";

export interface Recommendation {
  action: "new" | "handoff" | "compact";
  reason: string;
  /** Hay código que el README no refleja (si no, lo trabajado solo está en la conversación). */
  code: boolean;
  hours?: number;
}

export interface Weight {
  ctx: number;
  prompts: number;
  level: "normal" | "long" | "heavy";
  lastTs: number;
  lastEdit: number;
  lastReadme: number;
  recommend?: Recommendation;
}

export interface Session {
  id: string;
  name: string;
  root: string;
  cwd: string;
  items: string[];
  state: "active" | "paused" | "closed";
  mode: string;
  agent: string;
  plan: boolean;
  model?: string;
  created: string;
  lastActivity: string;
  forkOf: string | null;
  repos: string[];
  pinned: boolean;
  keepOpen: boolean;
  running: boolean;
  status: LiveStatus;
  statusDetail: string;
  hooks: boolean;
  weight: Weight | null;
  otherItems: { ts: number; items: { id: string; name: string }[] } | null;
  /** Solo en las de otros proyectos. */
  project?: string;
}

export interface SessionList {
  rev: number;
  sessions: Session[];
  others: Session[];
  autoClosed: string[];
}

export interface AgentInfo {
  id: string;
  label?: string;
  found: boolean;
  hooks: boolean;
  fork: boolean;
}

export interface AgentsInfo {
  agents: AgentInfo[];
  default: string;
  custom: CustomAgent;
  planPrefix: Record<string, string>;
  placeholders: string[];
  modes: string[];
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

// ---------- trackers (Azure DevOps) ----------

export interface TrackerState {
  name: string;
  color: string;
  /** Categoría propia del tracker (Proposed, InProgress…). */
  category: string;
  types: string[];
  /** Categoría del flujo que le corresponde. */
  suggested: StateCategory | null;
}

export interface TrackerField {
  ref: string;
  name: string;
}

export interface TrackerWorkItem {
  state: string;
  title: string;
  type: string;
  assigned: string;
  changed: string;
  priority: number | null;
  due: string;
  estimate: number | null;
  completed: number | null;
}

export interface TrackerCache {
  source: string;
  kind: string;
  org: string;
  project: string;
  syncedAt: number | null;
  requested: string[];
  items: Record<string, TrackerWorkItem>;
}

export interface TrackerConfig {
  kinds: string[];
  connected: boolean;
  kind: string;
  source: string;
  org: string;
  project: string;
  hasSecret: boolean;
  hasWriteSecret: boolean;
  states: TrackerState[];
  statesFetchedAt: number | null;
  /** Estado del tracker → id de estado del flujo. */
  map: Record<string, string>;
  mapSaved: boolean;
  suggested: Record<string, string>;
  fieldsFetched: boolean;
  dateFields: TrackerField[];
  numFields: TrackerField[];
  hasPriority: boolean;
  hasEffort: boolean;
  dueField: string;
  hoursField: string;
  /** Tipo del tracker → tipo del flujo. */
  localTypes: Record<string, string>;
  cache: TrackerCache | null;
}

export interface SyncRow {
  key: string;
  name: string;
  ids: string[];
  tracker: { id: string; state: string; title: string; type: string };
  current: string;
  status: string;
  type: string;
  kind: "fill" | "type" | "differ" | "back";
  on: boolean;
}

export interface SyncPreview {
  rows: SyncRow[];
  unmapped: { state: string; items: string[] }[];
  notFound: string[];
  equal: number;
  differ: number;
  fill: number;
  types: number;
  read: number;
  requested: number;
  syncedAt: number | null;
}

export interface AssignedRow extends TrackerWorkItem {
  id: string;
  localType: string;
  existing: string[];
}

export interface AssignedList {
  period: string;
  start: string;
  end: string;
  field: string;
  items: AssignedRow[];
}

export interface TrackerItem {
  id: string;
  title: string;
  type: string;
  localType: string;
  state: string;
  description: string;
  criteria: string;
  folder: string;
  existing: string[];
  fromTracker: boolean;
  source: string;
}

export interface TrackerFailure {
  id?: string;
  name?: string;
  error: ApiError;
}
