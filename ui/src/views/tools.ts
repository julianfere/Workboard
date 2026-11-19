// Tools: arriba las promovidas (el MCP las ofrece a los agentes con workboard_run_tool); abajo los
// candidatos, scripts que escribieron las sesiones fuera de los repos y que se corrieron varias veces o
// se escribieron en más de una sesión. Promover copia el script a la carpeta de config.

import { api } from "../api";
import { onAction, registerView, render } from "../app";
import { $, confirmDialog, esc, fmtDate, fmtSize, promptDialog, toast } from "../dom";
import { errorText, t } from "../i18n";
import { ico } from "../icons";
import { S } from "../state";
import type { ToolCandidate, ToolInfo, ToolsOverview } from "../types";

const V = { data: null as ToolsOverview | null, loading: false, root: "" };

async function load(force = false): Promise<void> {
  if (V.root !== S.root) {
    V.root = S.root;
    V.data = null;
  }
  if (V.loading || (V.data && !force)) return;
  V.loading = true;
  try {
    V.data = await api.toolsOverview();
  } catch (err) {
    toast(errorText(err), undefined, "error");
  } finally {
    V.loading = false;
  }
  render();
}

function toolRow(x: ToolInfo): string {
  const n = esc(x.name);
  return `<tr><td><b class="mono">${n}</b>${x.ok ? "" : ` <span class="chip warn-text" title="${esc(t("tools.changedHint"))}">${esc(t("tools.changed"))}</span>`}</td>
    <td>${esc(x.description || "—")}</td><td class="mono small">${esc(x.usage)}</td><td class="muted small">${esc(x.origin.items.join(", ") || "—")}</td>
    <td class="actions">${x.ok ? "" : `<button class="btn" data-action="tool-repromote" data-name="${n}">${esc(t("tools.accept"))}</button>`}
      <button class="icon-btn" data-action="tool-edit" data-name="${n}" title="${esc(t("common.edit"))}" aria-label="${esc(t("common.edit"))}">${ico("edit")}</button>
      <button class="icon-btn" data-action="tool-remove" data-name="${n}" title="${esc(t("tools.remove"))}" aria-label="${esc(t("tools.remove"))}">${ico("trash")}</button></td></tr>`;
}

function candidateRow(x: ToolCandidate, i: number): string {
  return `<tr><td><b class="mono">${esc(x.name)}</b><div class="muted small mono ellipsis" title="${esc(x.path)}">${esc(x.path)}</div></td>
    <td class="num">${x.runs}</td><td class="small">${esc(x.sessions.map((s) => s.name || s.id.slice(0, 8)).join(", "))}</td>
    <td class="muted small">${esc(x.items.join(", ") || "—")}</td><td class="muted small">${x.last ? esc(fmtDate(x.last)) : "—"}</td>
    <td class="muted small num">${esc(fmtSize(x.bytes))}</td>
    <td class="actions"><button class="btn" data-action="tool-promote" data-index="${i}">${esc(t("tools.promote"))}</button></td></tr>`;
}

function renderTools(): string {
  if (V.root !== S.root || (!V.data && !V.loading)) void load();
  const head = `<div class="toolbar"><h1>${esc(t("nav.tools"))}</h1><span class="muted">${esc(t("tools.subtitle"))}</span><span class="spacer"></span>
    <button class="icon-btn" data-action="tools-refresh" title="${esc(t("tools.refresh"))}" aria-label="${esc(t("tools.refresh"))}">${ico("refresh")}</button></div>`;
  if (!V.data) return `<div class="page wide">${head}<p class="empty">${esc(t("tools.loading"))}</p></div>`;
  const { tools, candidates } = V.data;
  const h = (k: string, cls = "") => `<th class="${cls}">${esc(t(`tools.col.${k}`))}</th>`;
  return `<div class="page wide">${head}
    <section class="card"><h2>${ico("wrench")}${esc(t("tools.promoted"))} <span class="muted">${tools.length}</span></h2>
      <p class="muted small">${esc(t("tools.promotedHint"))}</p>
      ${tools.length ? `<table class="grid h-table"><thead><tr>${h("name")}${h("description")}${h("usage")}${h("origin")}<th></th></tr></thead><tbody>${tools.map(toolRow).join("")}</tbody></table>` : `<p class="empty">${esc(t("tools.nonePromoted"))}</p>`}</section>
    <section class="card"><h2>${ico("code")}${esc(t("tools.candidates"))} <span class="muted">${candidates.length}</span></h2>
      <p class="muted small">${esc(t("tools.candidatesHint"))}</p>
      ${candidates.length ? `<table class="grid h-table"><thead><tr>${h("script")}${h("runs", "num")}${h("sessions")}${h("items")}${h("last")}${h("size", "num")}<th></th></tr></thead><tbody>${candidates.map(candidateRow).join("")}</tbody></table>` : `<p class="empty">${esc(t("tools.noCandidates"))}</p>`}</section></div>`;
}

registerView("tools", renderTools);

/** Ver el código y promover (o aceptar el cambio de una tool ya promovida). */
async function promoteModal(path: string, file: string, suggested: string, existing?: ToolInfo): Promise<void> {
  let pv: { code: string; description: string; usage: string };
  try {
    pv = await api.toolPreview(path);
  } catch (err) {
    toast(errorText(err), undefined, "error");
    return;
  }
  const back = document.createElement("div");
  back.className = "modal-back";
  back.innerHTML = `<form class="modal wide" role="dialog" aria-modal="true"><h2>${esc(t("tools.promoteTitle", { name: file }))}</h2>
    <p class="muted small mono">${esc(path)}</p>
    <label>${esc(t("tools.name"))}<input name="name" value="${esc(existing?.name ?? suggested)}"${existing ? " readonly" : ""}></label>
    <label>${esc(t("tools.description"))}<input name="description" value="${esc(existing?.description || pv.description)}" placeholder="${esc(t("tools.descriptionHint"))}"></label>
    <label>${esc(t("tools.usage"))}<input name="usage" class="mono" value="${esc(existing?.usage || pv.usage)}" placeholder="${esc(t("tools.usageHint"))}"></label>
    <pre class="code tool-code">${esc(pv.code)}</pre>
    <p class="error" data-error></p>
    <div class="modal-actions"><span class="muted small grow">${esc(t("tools.copyNote"))}</span><button type="button" class="btn" data-cancel>${esc(t("common.cancel"))}</button>
      <button class="btn primary">${esc(t("tools.promote"))}</button></div></form>`;
  document.body.append(back);
  const form = back.querySelector("form")!;
  const value = (n: string) => (form.elements.namedItem(n) as HTMLInputElement).value.trim();
  const close = () => back.remove();
  $("[data-cancel]", back)!.onclick = close;
  back.addEventListener("keydown", (e) => e.key === "Escape" && close());
  form.onsubmit = async (e) => {
    e.preventDefault();
    try {
      const out = await api.toolPromote(path, value("name"), value("description"), value("usage"), !!existing);
      close();
      toast(t("tools.promotedDone", { name: out.name }));
      await load(true);
    } catch (err) {
      $("[data-error]", back)!.textContent = errorText(err);
    }
  };
  (form.elements.namedItem(existing ? "description" : "name") as HTMLInputElement).focus();
}

const toolByName = (name: string) => V.data?.tools.find((x) => x.name === name);

onAction("tools-refresh", () => load(true));
onAction("tool-promote", (el) => {
  const c = V.data?.candidates[Number(el.dataset.index)];
  if (c) void promoteModal(c.path, c.name, c.suggested);
});
onAction("tool-repromote", (el) => {
  const x = toolByName(el.dataset.name!);
  if (x) void promoteModal(x.path, x.file, x.name, x);
});
onAction("tool-edit", async (el) => {
  const x = toolByName(el.dataset.name!);
  if (!x) return;
  const description = await promptDialog(t("tools.editDescription", { name: x.name }), x.description);
  if (description === null) return;
  const usage = await promptDialog(t("tools.editUsage", { name: x.name }), x.usage);
  await api.toolUpdate(x.name, description, usage ?? x.usage);
  await load(true);
});
onAction("tool-remove", async (el) => {
  const x = toolByName(el.dataset.name!);
  if (!x || !(await confirmDialog(t("tools.removeConfirm", { name: x.name }), t("tools.remove")))) return;
  await api.toolRemove(x.name);
  toast(t("tools.removed", { name: x.name }));
  await load(true);
});
