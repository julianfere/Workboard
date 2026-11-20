// Paleta `Ctrl+K`: busca trabajos, tareas, archivos y comandos; con un trabajo arriba, ofrece sus acciones.
// `>` al principio = solo comandos. ↑↓ para moverse, Enter para ejecutar, Esc para cerrar (como la v1).

import { api } from "./api";
import { applyTheme, goView, NAV, onAction, openDetail, refresh, refreshAll, render, runAction } from "./app";
import { esc, promptDialog, toast } from "./dom";
import { errorText, t } from "./i18n";
import { ico } from "./icons";
import { itemIds, openPromotion } from "./git";
import { pluginCommands } from "./plugins";
import { fmtDay } from "./planning";
import { openProjectMenu } from "./projects";
import { newSessionModal, openSession, sessionsOf } from "./sessions";
import { S, effectiveStatus, isClosed, metaText, stateDef, stateLabel, titleOf, wf } from "./state";
import { toggleTerminal } from "./terminal";
import type { FileEntry, Item } from "./types";
import { openFileViewer } from "./viewer";
import { matchesText } from "./views/grid";
import { statusChip } from "./views/items";
import { trackerConnected } from "./views/tracker";

interface Entry {
  icon: string;
  text: string;
  hint: string;
  run: () => void;
  /** Lo que va antes del texto (estado del trabajo…), ya escapado. */
  html?: string;
}

interface Section {
  head: string;
  rows: Entry[];
}

/** Cuántos de cada cosa se muestran. */
const MAX = { items: 6, tasks: 4, files: 4, commands: 6, commandsQuery: 8 };

const itemHint = (it: Item) => it.ids.join(", ") || (it.kind === "group" ? t("palette.group") : it.name);
const fail = (err: unknown) => toast(errorText(err), undefined, "error");

function commands(): Entry[] {
  const go = t("palette.goTo");
  const out: Entry[] = NAV.map((v, i) => ({ icon: v.icon, text: t(`nav.${v.id}`), hint: `${go} · ${i + 1}`, run: () => goView(v.id) }));
  out.push({ icon: "settings", text: t("nav.settings"), hint: go, run: () => goView("settings") });
  out.push({ icon: "plus", text: t("items.new"), hint: "", run: () => runAction("new-item") });
  out.push({ icon: "play", text: t("palette.cmd.newSession"), hint: "", run: () => void newSessionModal(S.detail ? [S.detail] : []) });
  if (trackerConnected()) out.push({ icon: "cloud", text: t("palette.cmd.sync"), hint: "", run: () => runAction("trk-sync") });
  out.push({ icon: "trash", text: t("palette.cmd.deleted"), hint: "", run: () => runAction("show-deleted") });
  out.push({ icon: "download", text: t("palette.cmd.export"), hint: "", run: () => runAction("grid-export") });
  out.push({ icon: "refresh", text: t("palette.cmd.reload"), hint: "F5", run: () => void refreshAll() });
  const dark = document.documentElement.dataset.theme === "dark";
  out.push({ icon: dark ? "sun" : "moon", text: t(dark ? "palette.cmd.themeLight" : "palette.cmd.themeDark"), hint: "", run: () => void setTheme(dark ? "light" : "dark") });
  out.push({ icon: "terminal", text: t("palette.cmd.terminal"), hint: "Ctrl+J", run: () => toggleTerminal() });
  out.push({ icon: "folder", text: t("projects.openFolder"), hint: "", run: () => openProjectMenu() });
  out.push(...pluginCommands());
  return out;
}

async function setTheme(theme: "light" | "dark"): Promise<void> {
  try {
    S.settings = await api.setSettings({ ...S.settings!, theme });
    applyTheme(theme);
    render();
  } catch (err) {
    fail(err);
  }
}

/** Siguiente estado del flujo: el que sigue en el orden del preset, sin los de categoría cancelado ni
 *  desconocido. Sin estado = el primero; uno cerrado no tiene siguiente. */
function nextState(current: string): string | null {
  const states = wf().states;
  const open = states.filter((s) => s.category !== "cancelled" && s.category !== "unknown");
  if (!current) return open[0]?.id ?? null;
  if (isClosed(current)) return null;
  const i = states.findIndex((s) => s.id === current);
  return states.slice(i + 1).find((s) => open.includes(s))?.id ?? null;
}

function itemActions(it: Item): Entry[] {
  const out: Entry[] = [];
  const f = wf().fields;
  const next = it.kind === "group" ? null : nextState(effectiveStatus(it));
  if (next) {
    out.push({
      icon: "chevronRight",
      text: t("palette.act.next", { state: stateLabel(next) }),
      hint: t("palette.act.status"),
      html: `<span class="dot ${stateDef(next)?.category ?? "unknown"}"></span>`,
      run: () => void saveFields(it, { [f.status]: next }, t("palette.act.moved", { state: stateLabel(next) })),
    });
  }
  const open = sessionsOf(it.name).filter((s) => s.state !== "closed");
  if (open.length) out.push({ icon: "play", text: t("palette.act.openSession", { name: open[0].name }), hint: t(`session.status.${open[0].status}`), run: () => void openSession(open[0].id) });
  else out.push({ icon: "play", text: t("palette.cmd.newSession"), hint: "", run: () => void newSessionModal([it.name]) });
  out.push({ icon: "plus", text: t("palette.act.addTask"), hint: "", run: () => void addTask(it) });
  out.push({ icon: "calendar", text: t("palette.act.due"), hint: metaText(it, f.due) ? fmtDay(metaText(it, f.due)) : "", run: () => void setDue(it) });
  if (itemIds(it).length) out.push({ icon: "promote", text: t("palette.act.promote"), hint: "git", run: () => void openPromotion([it.name]) });
  out.push({ icon: "code", text: t("palette.act.editor"), hint: "", run: () => void api.openInEditor(it.name).catch(fail) });
  out.push({ icon: "folder", text: t("palette.act.folder"), hint: "", run: () => void api.reveal(it.name).catch(fail) });
  return out;
}

async function saveFields(it: Item, fields: Record<string, string>, done: string): Promise<void> {
  try {
    await api.updateFields(it.name, fields);
    await refresh();
    toast(done);
  } catch (err) {
    fail(err);
  }
}

async function addTask(it: Item): Promise<void> {
  const text = await promptDialog(t("palette.act.addTaskPrompt", { name: titleOf(it) }));
  if (!text) return;
  try {
    await api.addTask(it.name, text);
    await refresh();
    toast(t("palette.act.taskAdded"));
  } catch (err) {
    fail(err);
  }
}

async function setDue(it: Item): Promise<void> {
  const key = wf().fields.due;
  const value = await promptDialog(t("palette.act.duePrompt"), metaText(it, key), true);
  if (value === null || value === metaText(it, key)) return;
  await saveFields(it, { [key]: value }, value ? t("palette.act.dueSet", { date: value }) : t("palette.act.dueCleared"));
}

/** Abre el detalle en la pestaña Archivos con ese archivo en el visor. */
async function openFile(it: Item, file: FileEntry): Promise<void> {
  await openDetail(it.name);
  if (S.detail !== it.name) return;
  S.detailTab = "files";
  render();
  openFileViewer(file);
}

function sections(q: string): Section[] {
  const onlyCommands = q.startsWith(">");
  const query = (onlyCommands ? q.slice(1) : q).trim().toLowerCase();
  const words = query.split(/\s+/).filter(Boolean);
  const hit = (text: string) => words.every((w) => text.toLowerCase().includes(w));
  const out: Section[] = [];
  if (!onlyCommands) {
    const items = words.length
      ? S.items.filter((it) => matchesText(it, query))
      : S.items.filter((it) => !isClosed(effectiveStatus(it))).sort((a, b) => b.last - a.last);
    const top = items.slice(0, MAX.items);
    if (top.length) {
      out.push({
        head: t(words.length ? "palette.sec.items" : "palette.sec.recent"),
        rows: top.map((it) => ({ icon: it.kind === "group" ? "layers" : "file", text: titleOf(it), hint: itemHint(it), run: () => void openDetail(it.name), html: statusChip(effectiveStatus(it)) })),
      });
    }
    if (words.length && top.length) out.push({ head: t("palette.sec.actions", { name: top[0].ids.join(", ") || titleOf(top[0]) }), rows: itemActions(top[0]) });
    if (words.length) {
      const tasks: Entry[] = [];
      for (const it of S.items) {
        for (const task of it.tasks) {
          if (tasks.length < MAX.tasks && !task.done && hit(task.text)) tasks.push({ icon: "tasks", text: task.text, hint: itemHint(it), run: () => void openDetail(it.name) });
        }
      }
      if (tasks.length) out.push({ head: t("palette.sec.tasks"), rows: tasks });
      if (words.join("").length >= 3) {
        const files: Entry[] = [];
        for (const it of S.items) {
          for (const file of it.files) {
            if (files.length < MAX.files && hit(file.name)) files.push({ icon: "file", text: file.name, hint: itemHint(it), run: () => void openFile(it, file) });
          }
        }
        if (files.length) out.push({ head: t("palette.sec.files"), rows: files });
      }
    }
  }
  const cmds = commands().filter((c) => !words.length || hit(`${c.text} ${c.hint}`));
  const limit = onlyCommands ? cmds.length : words.length ? MAX.commandsQuery : MAX.commands;
  if (cmds.length) out.push({ head: t("palette.sec.commands"), rows: cmds.slice(0, limit) });
  return out;
}

export function openPalette(): void {
  if (document.querySelector(".palette")) return;
  const back = document.createElement("div");
  back.className = "modal-back";
  back.innerHTML = `<div class="modal palette" role="dialog" aria-modal="true" aria-label="${esc(t("palette.open"))}">
    <div class="search big">${ico("search")}<input id="palette-input" placeholder="${esc(t("palette.placeholder"))}" autocomplete="off" spellcheck="false" role="combobox" aria-expanded="true" aria-controls="palette-list"><kbd>Esc</kbd></div>
    <ul class="palette-list" id="palette-list" role="listbox"></ul>
    <div class="palette-foot muted"><span><kbd>↑</kbd> <kbd>↓</kbd> ${esc(t("palette.foot.move"))}</span><span><kbd>Enter</kbd> ${esc(t("palette.foot.run"))}</span><span class="grow"></span><span>${esc(t("palette.foot.commands"))}</span></div></div>`;
  document.body.append(back);
  const input = back.querySelector<HTMLInputElement>("#palette-input")!;
  const list = back.querySelector<HTMLElement>(".palette-list")!;
  let flat: Entry[] = [];
  let sel = 0;
  const draw = () => {
    const secs = sections(input.value);
    flat = secs.flatMap((s) => s.rows);
    sel = Math.min(sel, Math.max(flat.length - 1, 0));
    let n = 0;
    const row = (e: Entry) => {
      const i = n++;
      return `<li role="option" id="pal-${i}" aria-selected="${i === sel}" class="${i === sel ? "sel" : ""}" data-i="${i}">${ico(e.icon)}${e.html ?? ""}<span class="grow">${esc(e.text)}</span><span class="muted">${esc(e.hint)}</span></li>`;
    };
    list.innerHTML =
      secs.map((s) => `<li class="palette-sec" role="presentation">${esc(s.head)}</li>${s.rows.map(row).join("")}`).join("") ||
      `<li class="empty">${esc(t("palette.none"))}</li>`;
    input.setAttribute("aria-activedescendant", flat.length ? `pal-${sel}` : "");
    list.querySelector(".sel")?.scrollIntoView({ block: "nearest" });
  };
  const close = () => back.remove();
  const pick = (i: number) => {
    const e = flat[i];
    close();
    e?.run();
  };
  input.oninput = () => {
    sel = 0;
    draw();
  };
  input.onkeydown = (e) => {
    if (e.key === "ArrowDown") { sel = Math.min(sel + 1, flat.length - 1); draw(); e.preventDefault(); }
    else if (e.key === "ArrowUp") { sel = Math.max(sel - 1, 0); draw(); e.preventDefault(); }
    else if (e.key === "Enter") { pick(sel); e.preventDefault(); }
    else if (e.key === "Escape") { close(); e.stopPropagation(); }
  };
  list.onclick = (e) => {
    const li = (e.target as HTMLElement).closest<HTMLElement>("[data-i]");
    if (li) pick(Number(li.dataset.i));
  };
  back.onclick = (e) => {
    if (e.target === back) close();
  };
  draw();
  input.focus();
}

onAction("palette", () => openPalette());
