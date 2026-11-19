// Trabajos: la grilla de items con búsqueda, filtros, orden, agrupado, selección y acciones en lote.

import { api } from "../api";
import { onAction, onChange, openDetail, refresh, registerView, render } from "../app";
import { $, confirmDialog, daysUntil, esc, fmtAgo, promptDialog, toast } from "../dom";
import { label, t } from "../i18n";
import { ico } from "../icons";
import { S, effectiveStatus, isClosed, metaText, stateDef, stateLabel, stateRank, titleOf, typeLabel, wf } from "../state";
import type { Item } from "../types";

const selected = new Set<string>();

export function statusChip(status: string): string {
  if (!status) return `<span class="chip muted">${esc(t("items.noStatus"))}</span>`;
  const cat = stateDef(status)?.category ?? "unknown";
  return `<span class="chip"><span class="dot ${cat}"></span>${esc(stateLabel(status))}</span>`;
}

export function dueBadge(it: Item): string {
  const due = metaText(it, wf().fields.due);
  if (!due) return "";
  const days = daysUntil(due);
  const overdue = days < 0 && !isClosed(effectiveStatus(it));
  return `<span class="due${overdue ? " overdue" : days <= 7 ? " soon" : ""}" title="${esc(due)}">${esc(due)}</span>`;
}

function matches(it: Item): boolean {
  const f = S.filter;
  const status = effectiveStatus(it);
  if (f.status && status !== f.status) return false;
  if (f.hideClosed && !f.status && isClosed(status)) return false;
  if (f.text) {
    const hay = [it.name, it.title, it.ids.join(" "), metaText(it, wf().fields.notes), ...it.tasks.map((x) => x.text)]
      .join(" ")
      .toLowerCase();
    if (!hay.includes(f.text.toLowerCase())) return false;
  }
  return true;
}

const SORTS: Record<string, (a: Item, b: Item) => number> = {
  status: (a, b) => stateRank(effectiveStatus(a)) - stateRank(effectiveStatus(b)),
  id: (a, b) => (a.ids[0] ?? "").localeCompare(b.ids[0] ?? "", undefined, { numeric: true }),
  title: (a, b) => titleOf(a).localeCompare(titleOf(b)),
  due: (a, b) => (metaText(a, wf().fields.due) || "9999").localeCompare(metaText(b, wf().fields.due) || "9999"),
  last: (a, b) => a.last - b.last,
};

function sorted(items: Item[]): Item[] {
  const cmp = SORTS[S.sort.key] ?? SORTS.last;
  return [...items].sort((a, b) => cmp(a, b) * S.sort.dir || a.name.localeCompare(b.name));
}

function row(it: Item, nested = false): string {
  const open = it.tasks.filter((x) => !x.done).length;
  const notes = metaText(it, wf().fields.notes);
  const isGroup = it.kind === "group";
  return `<tr class="row${nested ? " nested" : ""}${isGroup ? " group" : ""}${selected.has(it.name) ? " sel" : ""}" data-action="open-item" data-name="${esc(it.name)}">
    <td class="c-check"><input type="checkbox" data-action="select-item" data-name="${esc(it.name)}"${selected.has(it.name) ? " checked" : ""} aria-label="${esc(t("items.select"))}"></td>
    <td class="c-status">${statusChip(effectiveStatus(it))}</td>
    <td class="c-id">${isGroup ? ico("layers") : esc(it.ids.join(", ") || "—")}</td>
    <td class="c-type">${it.type ? esc(typeLabel(it.type)) : ""}</td>
    <td class="c-title"><div class="title">${esc(titleOf(it))} ${dueBadge(it)}</div><div class="sub">${esc(it.name)}</div></td>
    <td class="c-notes">${esc(notes)}</td>
    <td class="c-tasks">${it.tasks.length ? `${open}/${it.tasks.length}` : ""}</td>
    <td class="c-last">${esc(fmtAgo(it.last))}</td>
  </tr>`;
}

function rows(items: Item[]): string {
  // Los items de un grupo van debajo de su grupo (si el grupo pasa el filtro, o ellos mismos).
  const visible = items.filter(matches);
  const topLevel = sorted(visible.filter((it) => !it.group || !visible.some((g) => g.name === it.group)));
  return topLevel
    .map((it) => {
      if (it.kind !== "group") return row(it);
      const members = sorted(visible.filter((m) => m.group === it.name));
      return row(it) + members.map((m) => row(m, true)).join("");
    })
    .join("");
}

function grouped(): string {
  const by = S.filter.groupBy;
  if (by === "none") return rows(S.items);
  const keyOf = (it: Item) => (by === "status" ? effectiveStatus(it) : it.type);
  const keys = by === "status" ? ["", ...wf().states.map((s) => s.id)] : ["", ...wf().types.map((x) => x.id)];
  return keys
    .map((k) => {
      const body = rows(S.items.filter((it) => keyOf(it) === k));
      if (!body) return "";
      const title = k ? (by === "status" ? stateLabel(k) : typeLabel(k)) : t(by === "status" ? "items.noStatus" : "items.noType");
      return `<tr class="group-head"><td colspan="8">${esc(title)}</td></tr>${body}`;
    })
    .join("");
}

function header(key: string, text: string, cls: string): string {
  const active = S.sort.key === key;
  const arrow = active ? ico(S.sort.dir > 0 ? "chevronDown" : "chevronDown", S.sort.dir > 0 ? "flip" : "") : "";
  return `<th class="${cls}"><button class="th" data-action="sort" data-key="${key}">${esc(text)}${arrow}</button></th>`;
}

function bulkBar(): string {
  if (!selected.size) return "";
  const states = wf().states.map((s) => `<option value="${esc(s.id)}">${esc(label(s.labels, s.id))}</option>`).join("");
  return `<div class="bulk"><span>${esc(t("items.selected", { n: selected.size }))}</span>
    <select data-change="bulk-status" aria-label="${esc(t("items.setStatus"))}"><option value="">${esc(t("items.setStatus"))}</option>${states}</select>
    <button class="btn" data-action="bulk-group">${ico("layers")}${esc(t("items.group"))}</button>
    <button class="btn danger" data-action="bulk-trash">${ico("trash")}${esc(t("items.delete"))}</button>
    <button class="link" data-action="bulk-clear">${esc(t("items.clearSelection"))}</button></div>`;
}

function renderItems(): string {
  const f = S.filter;
  const states = wf().states.map((s) => `<option value="${esc(s.id)}"${f.status === s.id ? " selected" : ""}>${esc(label(s.labels, s.id))}</option>`).join("");
  const groupOpts = (["none", "status", "type"] as const)
    .map((g) => `<option value="${g}"${f.groupBy === g ? " selected" : ""}>${esc(t(`items.groupBy.${g}`))}</option>`)
    .join("");
  const body = grouped();
  return `<div class="page wide">
    <div class="toolbar"><h1>${esc(t("nav.items"))}</h1>
      <div class="search">${ico("search")}<input id="items-search" type="search" placeholder="${esc(t("items.search"))}" value="${esc(f.text)}" data-change="filter-text"></div>
      <select data-change="filter-status" aria-label="${esc(t("items.status"))}"><option value="">${esc(t("items.allStates"))}</option>${states}</select>
      <label class="check"><input type="checkbox" data-change="filter-closed"${f.hideClosed ? " checked" : ""}>${esc(t("items.hideClosed"))}</label>
      <select data-change="group-by" aria-label="${esc(t("items.groupBy"))}">${groupOpts}</select>
      <span class="spacer"></span>
      ${S.deletedCount ? `<button class="btn ghost" data-action="show-deleted" title="${esc(t("items.deleted"))}">${ico("trash")}${S.deletedCount}</button>` : ""}
      <button class="btn primary" data-action="new-item">${ico("plus")}${esc(t("items.new"))}</button>
    </div>
    ${S.loadError ? `<p class="error">${esc(S.loadError)}</p>` : ""}
    ${bulkBar()}
    <table class="grid"><thead><tr><th class="c-check"></th>${header("status", t("items.status"), "c-status")}${header("id", t("items.id"), "c-id")}<th class="c-type">${esc(t("items.type"))}</th>${header("title", t("items.title"), "c-title")}<th class="c-notes">${esc(t("items.notes"))}</th><th class="c-tasks">${esc(t("items.tasks"))}</th>${header("last", t("items.activity"), "c-last")}</tr></thead>
    <tbody>${body || `<tr><td colspan="8" class="empty">${esc(t(S.items.length ? "items.noMatches" : "items.empty"))}</td></tr>`}</tbody></table>
  </div>`;
}

// ---------- nuevo item ----------

function newItemModal(): void {
  const types = wf().types.map((x) => `<option value="${esc(x.id)}">${esc(label(x.labels, x.id))}</option>`).join("");
  const back = document.createElement("div");
  back.className = "modal-back";
  back.innerHTML = `<form class="modal" role="dialog" aria-modal="true"><h2>${esc(t("items.new"))}</h2>
    <label>${esc(t("newItem.id"))}<input name="id" placeholder="${esc(t("newItem.idHint"))}"></label>
    <label>${esc(t("items.title"))}<input name="title" required></label>
    <label>${esc(t("items.type"))}<select name="type"><option value=""></option>${types}</select></label>
    <label>${esc(t("newItem.folder"))}<input name="folder" required></label>
    <p class="error" data-error></p>
    <div class="modal-actions"><button type="button" class="btn" data-cancel>${esc(t("common.cancel"))}</button><button class="btn primary">${esc(t("newItem.create"))}</button></div></form>`;
  document.body.append(back);
  const form = back.querySelector("form")!;
  const field = (n: string) => form.elements.namedItem(n) as HTMLInputElement;
  let touched = false;
  const suggest = () => {
    if (touched) return;
    const slug = field("title").value.normalize("NFKD").replace(/[^\x00-\x7f]/g, "").toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "").slice(0, 40).replace(/-$/, "");
    const id = field("id").value.trim();
    const tpl = id ? wf().folders.newItemTemplate : "{slug}";
    field("folder").value = tpl.replace("{id}", id).replace("{slug}", slug).replace(/^-+|-+$/g, "");
  };
  field("title").oninput = suggest;
  field("id").oninput = suggest;
  field("folder").oninput = () => (touched = true);
  const close = () => back.remove();
  back.querySelector<HTMLElement>("[data-cancel]")!.onclick = close;
  back.addEventListener("keydown", (e) => e.key === "Escape" && close());
  form.onsubmit = async (e) => {
    e.preventDefault();
    try {
      const it = await api.createItem(field("folder").value.trim(), field("title").value, (form.elements.namedItem("type") as HTMLSelectElement).value);
      close();
      await refresh();
      await openDetail(it.name);
    } catch (err) {
      const { errorText } = await import("../i18n");
      back.querySelector("[data-error]")!.textContent = errorText(err);
    }
  };
  field("id").focus();
}

// ---------- eliminados ----------

async function deletedModal(): Promise<void> {
  const list = await api.listDeleted();
  const back = document.createElement("div");
  back.className = "modal-back";
  back.innerHTML = `<div class="modal" role="dialog" aria-modal="true"><h2>${esc(t("items.deleted"))}</h2>
    <ul class="list">${list
      .map((d) => `<li><span class="grow"><b>${esc(d.title || d.base)}</b> <span class="muted">${esc(d.base)} · ${esc(fmtAgo(d.deleted))}</span></span>
        <button class="btn" data-restore="${esc(d.path)}" data-base="${esc(d.base)}">${esc(t("items.restore"))}</button></li>`)
      .join("") || `<li class="empty">${esc(t("items.noDeleted"))}</li>`}</ul>
    <div class="modal-actions"><button class="btn" data-close>${esc(t("common.close"))}</button></div></div>`;
  document.body.append(back);
  back.querySelector<HTMLElement>("[data-close]")!.onclick = () => back.remove();
  back.querySelectorAll<HTMLElement>("[data-restore]").forEach((b) => {
    b.onclick = async () => {
      await api.untrash(b.dataset.restore!, b.dataset.base!);
      back.remove();
      await refresh();
    };
  });
}

// ---------- acciones ----------

registerView("items", renderItems);

onAction("open-item", (el, ev) => {
  if ((ev.target as HTMLElement).closest("input")) return;
  return openDetail(el.dataset.name!);
});
onAction("select-item", (el, ev) => {
  ev.stopPropagation();
  const name = el.dataset.name!;
  if (selected.has(name)) selected.delete(name);
  else selected.add(name);
  render();
});
onAction("bulk-clear", () => {
  selected.clear();
  render();
});
onAction("sort", (el) => {
  const key = el.dataset.key!;
  S.sort = S.sort.key === key ? { key, dir: -S.sort.dir } : { key, dir: key === "last" ? -1 : 1 };
  render();
});
onAction("new-item", () => newItemModal());
onAction("show-deleted", () => deletedModal());
onAction("bulk-group", async () => {
  const title = await promptDialog(t("items.groupTitle"));
  if (!title) return;
  const res = await api.createGroup(title, [...selected].filter((n) => !n.includes("/")));
  selected.clear();
  await refresh();
  toast(t("items.grouped", { n: res.members.length }));
});
onAction("bulk-trash", async () => {
  const names = [...selected];
  if (!(await confirmDialog(t("items.confirmDelete", { n: names.length }), t("items.delete")))) return;
  const moved: [string, string][] = [];
  for (const n of names) moved.push([await api.trash(n), n]);
  selected.clear();
  await refresh();
  toast(t("items.deletedN", { n: moved.length }), {
    label: t("common.undo"),
    run: async () => {
      for (const [deleted, name] of moved) await api.untrash(deleted, name);
      await refresh();
    },
  });
});

onChange("bulk-status", async (el) => {
  const status = (el as HTMLSelectElement).value;
  if (!status) return;
  for (const n of selected) await api.updateFields(n, { [wf().fields.status]: status });
  selected.clear();
  await refresh();
});
onChange("filter-text", (el) => {
  S.filter.text = (el as HTMLInputElement).value;
  render();
  $<HTMLInputElement>("#items-search")?.focus();
});
onChange("filter-status", (el) => {
  S.filter.status = (el as HTMLSelectElement).value;
  render();
});
onChange("filter-closed", (el) => {
  S.filter.hideClosed = (el as HTMLInputElement).checked;
  render();
});
onChange("group-by", (el) => {
  S.filter.groupBy = (el as HTMLSelectElement).value as typeof S.filter.groupBy;
  render();
});

// Búsqueda en vivo (sin esperar el `change`).
document.addEventListener("input", (ev) => {
  const el = ev.target as HTMLInputElement;
  if (el.id !== "items-search") return;
  S.filter.text = el.value;
  const pos = el.selectionStart;
  render();
  const input = $<HTMLInputElement>("#items-search");
  input?.focus();
  input?.setSelectionRange(pos, pos);
});
