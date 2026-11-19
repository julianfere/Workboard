// Única puerta al backend: `wb_api::Api::dispatch`. Dentro de Tauri va por el comando `api`; en un
// navegador (desarrollo, Playwright) por HTTP al `wb-devserver` (`npm run dev` lo reenvía).

import { invoke } from "@tauri-apps/api/core";
import type {
  ApiError,
  AppInfo,
  DeletedEntry,
  Item,
  ItemDetail,
  ItemList,
  Note,
  PresetSummary,
  Settings,
  Task,
  Version,
  Workflow,
} from "./types";

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
};

/** Elegir una carpeta: diálogo nativo en la app; en el navegador no hay (se escribe la ruta). */
export async function pickFolder(): Promise<string | null> {
  if (!inTauri) return null;
  const { open } = await import("@tauri-apps/plugin-dialog");
  const res = await open({ directory: true, multiple: false });
  return typeof res === "string" ? res : null;
}
