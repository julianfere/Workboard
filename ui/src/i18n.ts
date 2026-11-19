// Traducciones: los mismos catálogos que el backend (locales/*.json). Inglés es la referencia: lo que
// falta en otro idioma sale en inglés y lo que falta en todos sale como la clave.

import en from "../../locales/en.json";
import es from "../../locales/es.json";
import type { ApiError, Labels } from "./types";

const CATALOGS: Record<string, Record<string, string>> = { en, es };
export const LANGS = Object.keys(CATALOGS);
const FALLBACK = "en";

let current = FALLBACK;

/** Idioma soportado más cercano a una etiqueta (`es-AR` → `es`). */
export function resolveLang(tag: string | null | undefined): string {
  const base = (tag ?? "").split(/[-_]/)[0].toLowerCase();
  return LANGS.includes(base) ? base : FALLBACK;
}

export function setLang(tag: string | null | undefined): void {
  current = resolveLang(tag ?? navigator.language);
  document.documentElement.lang = current;
}

export function lang(): string {
  return current;
}

export function t(key: string, params: Record<string, string | number> = {}): string {
  const text = CATALOGS[current]?.[key] ?? CATALOGS[FALLBACK][key] ?? key;
  return text.replace(/\{(\w+)\}/g, (m, name: string) => (name in params ? String(params[name]) : m));
}

/** Texto de un objeto `{en, es}` del flujo (estados, tipos…). */
export function label(labels: Labels, id: string): string {
  return labels[current] ?? labels[FALLBACK] ?? id;
}

export function errorText(err: unknown): string {
  const e = err as Partial<ApiError>;
  if (e && typeof e.code === "string") return t(e.code, e.params ?? {});
  return t("error.generic", { message: String(err) });
}
