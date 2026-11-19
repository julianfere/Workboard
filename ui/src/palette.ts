// Paleta `Ctrl+K`: buscar items y ejecutar comandos. `>` al principio = solo comandos.

import { goView, NAV, onAction, openDetail } from "./app";
import { esc } from "./dom";
import { t } from "./i18n";
import { ico } from "./icons";
import { S, effectiveStatus, titleOf } from "./state";
import { pluginCommands } from "./plugins";
import { statusChip } from "./views/items";

interface Entry {
  icon: string;
  text: string;
  hint: string;
  run: () => void;
  html?: string;
}

function commands(): Entry[] {
  const out: Entry[] = NAV.map((v, i) => ({ icon: v.icon, text: t(`nav.${v.id}`), hint: String(i + 1), run: () => goView(v.id) }));
  out.push({ icon: "settings", text: t("nav.settings"), hint: "", run: () => goView("settings") });
  out.push({ icon: "plus", text: t("items.new"), hint: "", run: () => document.querySelector<HTMLElement>('[data-action="new-item"]')?.click() ?? (goView("items"), setTimeout(() => document.querySelector<HTMLElement>('[data-action="new-item"]')?.click())) });
  out.push({ icon: "folder", text: t("project.open"), hint: "", run: () => document.querySelector<HTMLElement>('[data-action="project-menu"]')?.click() });
  out.push(...pluginCommands());
  return out;
}

function entries(q: string): Entry[] {
  const onlyCommands = q.startsWith(">");
  const query = (onlyCommands ? q.slice(1) : q).trim().toLowerCase();
  const cmds = commands().filter((c) => !query || c.text.toLowerCase().includes(query));
  if (onlyCommands) return cmds;
  const items = S.items
    .filter((it) => !query || [it.name, it.title, it.ids.join(" ")].join(" ").toLowerCase().includes(query))
    .slice(0, 20)
    .map((it) => ({ icon: "file", text: titleOf(it), hint: it.ids.join(", ") || it.name, run: () => openDetail(it.name), html: statusChip(effectiveStatus(it)) }));
  return query ? [...items, ...cmds] : [...cmds, ...items];
}

export function openPalette(): void {
  if (document.querySelector(".palette")) return;
  const back = document.createElement("div");
  back.className = "modal-back";
  back.innerHTML = `<div class="modal palette" role="dialog" aria-modal="true" aria-label="${esc(t("palette.open"))}">
    <div class="search big">${ico("search")}<input id="palette-input" placeholder="${esc(t("palette.placeholder"))}" autocomplete="off"></div><ul class="palette-list" role="listbox"></ul></div>`;
  document.body.append(back);
  const input = back.querySelector<HTMLInputElement>("#palette-input")!;
  const list = back.querySelector<HTMLElement>(".palette-list")!;
  let current: Entry[] = [];
  let sel = 0;
  const draw = () => {
    current = entries(input.value);
    sel = Math.min(sel, Math.max(current.length - 1, 0));
    list.innerHTML = current
      .map((e, i) => `<li role="option" class="${i === sel ? "sel" : ""}" data-i="${i}">${ico(e.icon)}${e.html ?? ""}<span class="grow">${esc(e.text)}</span><span class="muted">${esc(e.hint)}</span></li>`)
      .join("") || `<li class="empty">${esc(t("palette.none"))}</li>`;
    list.querySelector(".sel")?.scrollIntoView({ block: "nearest" });
  };
  const close = () => back.remove();
  const pick = (i: number) => {
    const e = current[i];
    close();
    e?.run();
  };
  input.oninput = () => {
    sel = 0;
    draw();
  };
  input.onkeydown = (e) => {
    if (e.key === "ArrowDown") { sel = Math.min(sel + 1, current.length - 1); draw(); e.preventDefault(); }
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
