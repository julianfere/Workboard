// Búsqueda global: un mismo texto filtra Trabajos, Alertas y Pendientes (como la v1). El texto vive en
// `S.search` y la coincidencia en `matchesSearch` (`state.ts`); acá, la caja y el aviso «Limpiar búsqueda».

import { onAction, render } from "./app";
import { $, esc } from "./dom";
import { t } from "./i18n";
import { ico } from "./icons";
import { S } from "./state";

const ID = "global-search";

/** La caja de búsqueda de la barra de cada vista (siempre con el mismo texto). */
export function searchBox(): string {
  return `<div class="search">${ico("search")}<input id="${ID}" type="search" placeholder="${esc(t("items.search"))}" value="${esc(S.search)}"></div>`;
}

/** «Nada coincide con «…». Limpiar búsqueda» (o "" si no hay búsqueda). */
export function noSearchResults(): string {
  if (!S.search.trim()) return "";
  return `${esc(t("search.none", { q: S.search.trim() }))} <button class="link" data-action="search-clear">${esc(t("search.clear"))}</button>`;
}

/** Pone el foco en la búsqueda (`/`). */
export function focusSearch(): void {
  $<HTMLInputElement>(`#${ID}`)?.focus();
}

onAction("search-clear", () => {
  S.search = "";
  render();
});

// En vivo (sin esperar el `change`), conservando el cursor.
document.addEventListener("input", (ev) => {
  const el = ev.target as HTMLInputElement;
  if (el.id !== ID) return;
  S.search = el.value;
  const pos = el.selectionStart;
  render();
  const input = $<HTMLInputElement>(`#${ID}`);
  input?.focus();
  input?.setSelectionRange(pos, pos);
});
