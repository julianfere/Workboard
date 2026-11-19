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

// ---------- git ----------

/** Texto traducible que arma el backend: clave de `locales/*.json` y parámetros. */
export interface Msg {
  code: string;
  params: Record<string, string>;
}

export interface RepoConfig {
  name: string;
  path: string;
  /** Ambiente → rama remota (`origin/main`). */
  envs: Record<string, string>;
  /** Se compara con los otros repos (alerta de repos desparejos). */
  compare: boolean;
}

export interface GitConfig {
  repos: RepoConfig[];
  detected: RepoConfig[];
  /** La carpeta tiene repos configurados a mano (si no, son los detectados). */
  custom: boolean;
  envRefs: Record<string, string[]>;
  customEnvRefs: boolean;
}

export type GitState = "merged" | "equivalent" | "content" | "mentioned" | "pending";

export interface EnvState {
  ahead: number | null;
  state: GitState;
}

export interface GitTip {
  sha: string;
  ts: number;
  author: string;
  subject: string;
  loc: string;
}

export interface GitBranch {
  repo: string;
  name: string;
  locations: string[];
  tip: GitTip;
  envs: Record<string, EnvState>;
  own: number;
  files: number;
  mergedInto: string | null;
  ids: string[];
  current: boolean;
}

export interface GitMention {
  sha: string;
  ts: number;
  subject: string;
}

export interface GitRepoInfo {
  name: string;
  path?: string;
  envs?: Record<string, string>;
  missingEnvs?: string[];
  current?: string;
  dirty?: string[];
  dirtyCount?: number;
  lastFetch?: number | null;
  compare?: boolean;
  error?: Msg;
}

export interface GitOverview {
  repos: GitRepoInfo[];
  byItem: Record<string, GitBranch[]>;
  /** id → repo → ambiente → commit. */
  mentions: Record<string, Record<string, Record<string, GitMention>>>;
  computedAt: number;
  seconds: number;
}

export interface PromoFlag extends Msg {
  kind: "otherItem" | "shared" | "likely" | "merge";
}

export interface PromoRow {
  sha: string;
  short: string;
  ts: number;
  author: string;
  parents: string[];
  subject: string;
  branch: string;
  ids: string[];
  merge: boolean;
  flags: PromoFlag[];
  on: boolean;
}

export interface PromoBranch {
  name: string;
  ids: string[];
  locations: string[];
  commits: PromoRow[];
  notes: Msg[];
  warnings: Msg[];
  alreadyCp: number;
  fromEnv?: string;
}

export interface PromoRepo {
  name: string;
  path?: string;
  targetRef?: string;
  remote?: string;
  targetBranch?: string;
  webUrl?: string | null;
  forge?: "azure" | "github" | "gitlab" | null;
  lastFetch?: number | null;
  branches?: PromoBranch[];
  commits?: PromoRow[];
  sourceEnv?: string | null;
  error?: Msg;
}

export interface Promotion {
  target: string;
  ids: string[];
  repos: PromoRepo[];
}

export interface BranchDiff {
  repo: string;
  path: string;
  branch: string;
  diff: { base: string; tip: string; commits: number; stat: string; diff: string; cut: boolean };
}
