// Revisar: una sesión en modo plan, con el modelo que se elija, revisa el diff de las ramas de un item contra
// los criterios del README. El veredicto queda en una nota y los hallazgos bloqueantes, como tareas.

import { gitApi } from "./api";
import { $, esc, toast } from "./dom";
import { errorText, t } from "./i18n";
import { launched } from "./sessions";
import { stateDef } from "./state";
import { termSize } from "./terminal";
import type { Item } from "./types";

const MODEL_KEY = "wb.reviewModel";
const MODELS = ["sonnet", "opus", "haiku"];

/** Un item (no grupo) con id, en revisión: el id es lo que encuentra sus ramas. */
export function canReview(it: Item, status: string): boolean {
  return it.kind === "item" && it.ids.length > 0 && stateDef(status)?.category === "review";
}

function lastModel(): string {
  try {
    return localStorage.getItem(MODEL_KEY) ?? "sonnet";
  } catch {
    return "sonnet";
  }
}

export function openReview(it: Item): void {
  const back = document.createElement("div");
  back.className = "modal-back";
  back.innerHTML = `<form class="modal" role="dialog" aria-modal="true"><h2>${esc(t("review.title", { id: it.ids[0] }))}</h2>
    <p>${esc(t("review.intro"))}</p>
    <label>${esc(t("review.model"))}<input name="model" list="review-models" value="${esc(lastModel())}" placeholder="${esc(t("review.modelHint"))}" spellcheck="false"></label>
    <datalist id="review-models">${MODELS.map((m) => `<option value="${m}">`).join("")}</datalist>
    <p class="muted small">${esc(t("review.modelTip"))}</p>
    <div class="modal-actions"><button type="button" class="btn" data-no>${esc(t("common.cancel"))}</button>
      <button class="btn primary" data-go>${esc(t("review.action"))}</button></div></form>`;
  document.body.append(back);
  const form = back.querySelector("form")!;
  const input = form.elements.namedItem("model") as HTMLInputElement;
  const go = $<HTMLButtonElement>("[data-go]", back)!;
  const close = () => back.remove();
  $("[data-no]", back)!.onclick = close;
  back.addEventListener("click", (e) => {
    if (e.target === back) close();
  });
  back.addEventListener("keydown", (e) => {
    if (e.key === "Escape") close();
  });
  form.onsubmit = async (e) => {
    e.preventDefault();
    const model = input.value.trim();
    try {
      localStorage.setItem(MODEL_KEY, model);
    } catch {
      /* sin almacenamiento: la próxima vez vuelve a sonnet */
    }
    go.disabled = true;
    go.textContent = t("review.building");
    try {
      const { cols, rows } = termSize();
      const out = await gitApi.review(it.name, model, cols, rows);
      close();
      await launched(out.session);
      toast(t("review.started"));
    } catch (err) {
      go.disabled = false;
      go.textContent = t("review.action");
      toast(errorText(err), undefined, "error");
    }
  };
  input.focus();
  input.select();
}
