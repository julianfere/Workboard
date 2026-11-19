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

// ---------- horas, métricas y tools (fase 6) ----------

export interface ItemTime {
  /** Segundos activos por día local (`AAAA-MM-DD`). */
  days: Record<string, number>;
  /** Segundos con la sesión abierta (agentes sin hooks). */
  open: Record<string, number>;
  /** Segundos activos por sesión. */
  sessions: Record<string, number>;
}

export interface SessionTime {
  id: string;
  name: string;
  agent: string;
  state: "active" | "paused" | "closed";
  items: string[];
  active: number;
  open: number;
  source: "" | "log" | "transcript" | "log+transcript";
  from: number | null;
  to: number | null;
}

export interface TimeReport {
  byItem: Record<string, ItemTime>;
  sessions: SessionTime[];
}

export interface Tokens {
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
}

export interface Transition {
  ts: number;
  from: string;
  to: string;
  session: string | null;
}

export interface Outcome {
  start: string | null;
  end: string | null;
  advance: number;
  setbacks: number;
  transitions: Transition[];
}

export interface WrittenScript {
  path: string;
  ts: number;
  chars: number;
}

export interface SessionStats {
  id: string;
  name: string;
  agent: string;
  mode: string;
  plan: boolean;
  model?: string | null;
  state: "active" | "paused" | "closed";
  items: string[];
  forkOf: string | null;
  from: number | null;
  to: number | null;
  models: Record<string, number>;
  tokens: Tokens | null;
  prompts: number;
  interrupts: number;
  asks: number;
  tools: Record<string, number>;
  scripts: { written: WrittenScript[]; runs: Record<string, number> };
  outcome: Record<string, Outcome>;
  transcript?: string;
}

export interface ToolInfo {
  name: string;
  file: string;
  interpreter: string;
  description: string;
  usage: string;
  origin: { path: string; sessions: string[]; items: string[] };
  sha256: string;
  created: string;
  /** El script sigue igual que cuando se promovió (si no, el MCP no lo corre). */
  ok: boolean;
  path: string;
}

export interface ToolCandidate {
  name: string;
  path: string;
  paths: string[];
  runs: number;
  sessions: { id: string; name: string }[];
  items: string[];
  last: number | null;
  bytes: number;
  suggested: string;
}

export interface ToolsOverview {
  tools: ToolInfo[];
  candidates: ToolCandidate[];
}
