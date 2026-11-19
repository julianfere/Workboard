// Alertas: items sin README, ids repetidos en varias carpetas, planes incompletos y fechas vencidas.

import { api } from "../api";
import { onAction, openDetail, refresh, registerView, setAlertCounter } from "../app";
import { daysUntil, esc } from "../dom";
import { t } from "../i18n";
import { S, effectiveStatus, isClosed, metaText, titleOf, wf } from "../state";
import type { Item } from "../types";

interface Alert {
  kind: "noReadme" | "dupes" | "plan" | "overdue";
  item: Item;
  detail: string;
}

function alerts(): Alert[] {
  const out: Alert[] = [];
  const seen = new Set<string>();
  for (const it of S.items) {
    const status = effectiveStatus(it);
    if (it.kind === "item" && !it.hasReadme) out.push({ kind: "noReadme", item: it, detail: "" });
    // Un id repetido cuenta una vez; dentro de un mismo grupo no es duplicado.
    const dupes = (it.dupes ?? []).filter((d) => d.split("/")[0] !== it.name.split("/")[0]);
    if (dupes.length) {
      const key = [it.name, ...dupes].sort().join("|");
      if (!seen.has(key)) {
        seen.add(key);
        out.push({ kind: "dupes", item: it, detail: dupes.join(", ") });
      }
    }
    if (it.planGaps.length) out.push({ kind: "plan", item: it, detail: it.planGaps.map((g) => t(`detail.gap.${g}`)).join(" · ") });
    const due = metaText(it, wf().fields.due);
    if (due && daysUntil(due) < 0 && !isClosed(status)) out.push({ kind: "overdue", item: it, detail: due });
  }
  return out;
}

function renderAlerts(): string {
  const list = alerts();
  const kinds = ["noReadme", "dupes", "plan", "overdue"] as const;
  const blocks = kinds
    .map((k) => {
      const rows = list
        .filter((a) => a.kind === k)
        .map((a) => `<li class="link-row" data-action="open-item" data-name="${esc(a.item.name)}"><span class="grow"><b>${esc(titleOf(a.item))}</b> <span class="muted">${esc(a.item.name)}</span></span><span class="muted">${esc(a.detail)}</span>
          ${k === "noReadme" ? `<button class="btn" data-action="create-readme" data-name="${esc(a.item.name)}">${esc(t("alerts.createReadme"))}</button>` : ""}</li>`)
        .join("");
      return rows ? `<section class="card"><h2>${esc(t(`alerts.${k}`))}</h2><ul class="list">${rows}</ul></section>` : "";
    })
    .join("");
  return `<div class="page"><h1>${esc(t("nav.alerts"))}</h1>${blocks || `<p class="empty">${esc(t("alerts.none"))}</p>`}</div>`;
}

registerView("alerts", renderAlerts);
setAlertCounter(() => alerts().length);

onAction("create-readme", async (el, ev) => {
  ev.stopPropagation();
  // Escribir un campo crea el README (`# carpeta` + frontmatter) por la escritura única.
  const first = wf().states.find((s) => s.category === "backlog") ?? wf().states[0];
  await api.updateFields(el.dataset.name!, { [wf().fields.status]: first.id });
  await refresh();
  await openDetail(el.dataset.name!);
});
