// Única puerta al backend: `wb_api::Api::dispatch`. Dentro de Tauri va por el comando `api`; en un
// navegador (desarrollo, Playwright) por HTTP al `wb-devserver` (`npm run dev` lo reenvía).

import { invoke } from "@tauri-apps/api/core";
import type {
  AgentsInfo,
  ApiError,
  AppInfo,
  DeletedEntry,
  Item,
  ItemDetail,
  ItemList,
  Note,
  PresetSummary,
  Session,
  SessionList,
  SessionStats,
  Settings,
  Task,
  TimeReport,
  ToolInfo,
  ToolsOverview,
  Version,
  Workflow,
} from "./types";
import type { AssignedList, SyncPreview, TrackerConfig, TrackerFailure, TrackerItem } from "./types";
import type { BranchDiff, GitConfig, GitOverview, Promotion, RepoConfig } from "./types";

export const inTauri = "__TAURI_INTERNALS__" in window;

function toApiError(err: unknown): ApiError {
  const e = err as Partial<ApiError>;
  if (e && typeof e.code === "string") return { code: e.code, params: e.params ?? {}, conflict: e.conflict };
  return { code: "error.generic", params: { message: String(err) } };
}

export async function call<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  if (inTauri) {
    try {
      return await invoke<T>("api", { cmd, args });
    } catch (err) {
      throw toApiError(err);
    }
  }
  let res: Response;
  try {
    res = await fetch(`/api/${cmd}`, {
      method: "POST",
      headers: { "Content-Type": "application/json", "X-Workboard": "1" },
      body: JSON.stringify(args),
    });
  } catch (err) {
    throw toApiError({ code: "error.devserver", params: { message: String(err) } });
  }
  const out = (await res.json()) as { ok: boolean; data?: T; error?: ApiError };
  if (!out.ok) throw toApiError(out.error);
  return out.data as T;
}

export const api = {
  appInfo: () => call<AppInfo>("app_info"),
  workflowPresets: () => call<PresetSummary[]>("workflow_presets"),
  workflow: () => call<Workflow>("workflow"),
  getSettings: () => call<Settings>("get_settings"),
  setSettings: (settings: Settings) => call<Settings>("set_settings", { settings }),
  openRoot: (path: string) => call<Settings>("open_root", { path }),
  forgetRecent: (path: string) => call<Settings>("forget_recent", { path }),

  signature: () => call<[number, number]>("signature"),
  listItems: () => call<ItemList>("list_items"),
  getItem: (name: string) => call<ItemDetail>("get_item", { name }),
  updateFields: (name: string, fields: Record<string, unknown>) => call<Item>("update_fields", { name, fields }),
  addTask: (name: string, text: string) => call<Item>("add_task", { name, text }),
  setTask: (name: string, index: number, done: boolean, expect: string) => call<Item>("set_task", { name, index, done, expect }),
  editTask: (name: string, index: number, text: string, expect: string) => call<Item>("edit_task", { name, index, text, expect }),
  removeTask: (name: string, index: number, expect: string) => call<Item>("remove_task", { name, index, expect }),
  readFile: (path: string) => call<string>("read_file", { path }),
  readMedia: (path: string) => call<{ mime: string; data: string }>("read_media", { path }),
  listVersions: (name: string) => call<Version[]>("list_versions", { name }),
  readVersion: (name: string, id: string) => call<string>("read_version", { name, id }),
  restoreVersion: (name: string, id: string) => call<Item>("restore_version", { name, id }),
  createItem: (folder: string, title: string, type: string) => call<Item>("create_item", { folder, title, type }),
  createGroup: (title: string, members: string[]) => call<{ group: string; members: string[] }>("create_group", { title, members }),
  moveFolder: (name: string, dest: string) => call<string>("move_folder", { name, dest }),
  renameFolder: (name: string, base: string) => call<string>("rename_folder", { name, base }),
  trash: (name: string) => call<string>("trash", { name }),
  untrash: (deleted: string, name: string) => call<string>("untrash", { deleted, name }),
  listDeleted: () => call<DeletedEntry[]>("list_deleted"),
  markRan: (path: string, env: string, on: boolean) => call<Record<string, number>>("mark_ran", { path, env, on }),
  reveal: (path: string) => call<null>("reveal", { path }),
  openPath: (path: string) => call<null>("open_path", { path }),

  inbox: () => call<Task[]>("inbox"),
  inboxAdd: (text: string) => call<Task[]>("inbox_add", { text }),
  inboxSet: (index: number, done: boolean, expect: string) => call<Task[]>("inbox_set", { index, done, expect }),
  inboxRemove: (index: number, expect: string) => call<Task[]>("inbox_remove", { index, expect }),
  inboxClearDone: () => call<Task[]>("inbox_clear_done"),
  notes: () => call<Note[]>("notes"),
  noteRead: (id: string) => call<Note>("note_read", { id }),
  noteSave: (id: string, body: string, mtime: number) => call<Note>("note_save", { id, body, mtime }),
  noteCreate: (title: string, items: string[]) => call<Note>("note_create", { title, items }),

  sessionsRev: () => call<number>("sessions_rev"),
  sessions: () => call<SessionList>("sessions"),
  sessionAgents: () => call<AgentsInfo>("session_agents"),
  setAgents: (cfg: Partial<Pick<AgentsInfo, "default" | "custom" | "planPrefix">>) => call<AgentsInfo>("set_agents", cfg),
  sessionCreate: (opts: { items: string[]; name: string; mode: string; agent: string; model: string; cols: number; rows: number }) =>
    call<Session>("session_create", opts),
  sessionResume: (id: string, cols: number, rows: number) => call<Session>("session_resume", { id, cols, rows }),
  sessionPause: (id: string) => call<null>("session_pause", { id }),
  sessionClose: (id: string) => call<null>("session_close", { id }),
  sessionReopen: (id: string) => call<null>("session_reopen", { id }),
  sessionRename: (id: string, name: string) => call<string>("session_rename", { id, name }),
  sessionPin: (id: string, pinned: boolean) => call<null>("session_pin", { id, pinned }),
  sessionAddItems: (id: string, items: string[]) => call<Session>("session_add_items", { id, items }),
  sessionRemoveItem: (id: string, item: string) => call<Session>("session_remove_item", { id, item }),
  sessionSplit: (id: string, item: string, cols: number, rows: number) => call<Session>("session_split", { id, item, cols, rows }),
  sessionContinue: (id: string, cols: number, rows: number) => call<Session>("session_continue", { id, cols, rows }),
  sessionPrompt: (id: string, kind: "handoff" | "compact") => call<null>("session_prompt", { id, kind }),
  sessionRead: (id: string, offset: number) => call<{ offset: number; data: string; exited: boolean; code: number | null }>("session_read", { id, offset }),
  sessionWrite: (id: string, data: string) => call<null>("session_write", { id, data }),
  sessionResize: (id: string, cols: number, rows: number) => call<null>("session_resize", { id, cols, rows }),
  planItem: (name: string, cols: number, rows: number) => call<{ folder: string; session: Session }>("plan_item", { name, cols, rows }),

  // ---------- horas, métricas, tools y Reflexionar (fase 6) ----------
  timeReport: () => call<TimeReport>("time_report"),
  statsReport: () => call<{ sessions: SessionStats[] }>("stats_report"),
  toolsOverview: () => call<ToolsOverview>("tools_overview"),
  toolPreview: (path: string) => call<{ code: string; description: string; usage: string }>("tool_preview", { path }),
  toolPromote: (path: string, name: string, description: string, usage: string, replace: boolean) =>
    call<ToolInfo>("tool_promote", { path, name, description, usage, replace }),
  toolUpdate: (name: string, description: string, usage: string) => call<ToolInfo>("tool_update", { name, description, usage }),
  toolRemove: (name: string) => call<null>("tool_remove", { name }),
  reflectItems: (names: string[], cols: number, rows: number) => call<{ session: Session; evidence: string }>("reflect_items", { names, cols, rows }),
  saveExport: (name: string, text: string) => call<string>("save_export", { name, text }),
  revealExport: (path: string) => call<null>("reveal_export", { path }),
};

/** Elegir una carpeta: diálogo nativo en la app; en el navegador no hay (se escribe la ruta). */
export async function pickFolder(): Promise<string | null> {
  if (!inTauri) return null;
  const { open } = await import("@tauri-apps/plugin-dialog");
  const res = await open({ directory: true, multiple: false });
  return typeof res === "string" ? res : null;
}

// ---------- trackers (Azure DevOps): comandos `tracker_*` ----------

export const trackerApi = {
  config: () => call<TrackerConfig>("tracker_config"),
  connect: (kind: string, org: string, project: string, secret: string) => call<TrackerConfig>("tracker_connect", { kind, org, project, secret }),
  fetch: () => call<TrackerConfig>("tracker_fetch"),
  setMap: (map: Record<string, string>) => call<TrackerConfig>("tracker_set_map", { map }),
  setField: (which: "due" | "hours", ref: string) => call<TrackerConfig>("tracker_set_field", { which, ref }),
  clearSecret: (write: boolean) => call<TrackerConfig>("tracker_clear_secret", { write }),
  disconnect: () => call<TrackerConfig>("tracker_disconnect"),
  sync: () => call<TrackerConfig>("tracker_sync"),
  preview: (includeDiffer: boolean, only: string[] | null) => call<SyncPreview>("tracker_preview", { includeDiffer, only }),
  apply: (changes: { name: string; status: string | null; type: string | null }[]) =>
    call<{ done: string[]; errors: TrackerFailure[] }>("tracker_apply", { changes }),
  assigned: (period: string) => call<AssignedList>("tracker_assigned", { period }),
  importItems: (ids: string[]) => call<{ done: string[]; skipped: string[]; errors: TrackerFailure[] }>("tracker_import", { ids }),
  item: (id: string) => call<TrackerItem>("tracker_item", { id }),
  createItem: (data: { id: string; title: string; type: string; folder: string; description: string; criteria: string; trackerType: string; trackerState: string }) =>
    call<Item>("tracker_create_item", data),
  pushStates: (changes: { id: string; state: string }[], secret: string | null) =>
    call<{ done: { id: string; state: string }[]; errors: TrackerFailure[] }>("tracker_push_states", { changes, secret }),
  open: (id: string) => call<null>("tracker_open", { id }),
};

// ---------- git ----------

/** Panel Git (solo lectura) y datos de la promoción. */
export const gitApi = {
  config: () => call<GitConfig>("git_config"),
  /** `repos: null` = volver a los detectados; `envRefs: null` = las ramas del flujo. */
  setConfig: (cfg: { repos?: RepoConfig[] | null; envRefs?: Record<string, string[]> | null }) => call<GitConfig>("set_git_config", cfg),
  signature: () => call<string>("git_signature"),
  overview: (refresh: boolean) => call<GitOverview>("git_overview", { refresh }),
  promotion: (ids: string[], target: string) => call<Promotion>("git_promotion", { ids, target }),
  branchDiff: (name: string) => call<BranchDiff[]>("git_branch_diff", { name }),
  review: (name: string, model: string, cols: number, rows: number) =>
    call<{ session: Session; packet: string }>("review_item", { name, model, cols, rows }),
};
