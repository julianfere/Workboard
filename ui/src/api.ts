// Única puerta al backend. Dentro de Tauri usa `invoke`; en un navegador común (desarrollo con
// `npm run dev`, Playwright) usa un mock con los mismos presets, para trabajar la UI sin compilar Rust.

import { invoke } from "@tauri-apps/api/core";
import classicEs from "../../crates/wb-core/presets/classic-es.json";
import defaultPreset from "../../crates/wb-core/presets/default.json";
import type { ApiError, AppInfo, PresetSummary, Settings, Workflow } from "./types";

const inTauri = "__TAURI_INTERNALS__" in window;

export async function call<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  if (!inTauri) return mock(cmd, args) as T;
  try {
    return await invoke<T>(cmd, args);
  } catch (err) {
    throw toApiError(err);
  }
}

function toApiError(err: unknown): ApiError {
  const e = err as Partial<ApiError>;
  if (e && typeof e.code === "string") return { code: e.code, params: e.params ?? {} };
  return { code: "error.generic", params: { message: String(err) } };
}

export const api = {
  appInfo: () => call<AppInfo>("app_info"),
  workflowPresets: () => call<PresetSummary[]>("workflow_presets"),
  workflowPreset: (id: string) => call<Workflow>("workflow_preset", { id }),
  getSettings: () => call<Settings>("get_settings"),
  setSettings: (settings: Settings) => call<Settings>("set_settings", { settings }),
};

// ---------- mock para el navegador ----------

const PRESETS: Record<string, Workflow> = {
  default: defaultPreset as Workflow,
  "classic-es": classicEs as Workflow,
};
const SETTINGS_KEY = "workboard.dev.settings";

function mock(cmd: string, args: Record<string, unknown>): unknown {
  switch (cmd) {
    case "app_info":
      return { version: "dev", os: "browser", configDir: "(localStorage)" } satisfies AppInfo;
    case "workflow_presets":
      return Object.values(PRESETS).map((p) => ({ id: p.id, name: p.name }));
    case "workflow_preset": {
      const preset = PRESETS[args.id as string];
      if (!preset) throw { code: "error.unknownPreset", params: { id: String(args.id) } } satisfies ApiError;
      return structuredClone(preset);
    }
    case "get_settings": {
      const saved = localStorage.getItem(SETTINGS_KEY);
      return saved ? (JSON.parse(saved) as Settings) : ({ lang: null, theme: "system", workflow: "default" } satisfies Settings);
    }
    case "set_settings": {
      const settings = args.settings as Settings;
      if (!PRESETS[settings.workflow]) throw { code: "error.unknownPreset", params: { id: settings.workflow } } satisfies ApiError;
      localStorage.setItem(SETTINGS_KEY, JSON.stringify(settings));
      return settings;
    }
    default:
      throw { code: "error.generic", params: { message: `unknown command ${cmd}` } } satisfies ApiError;
  }
}
