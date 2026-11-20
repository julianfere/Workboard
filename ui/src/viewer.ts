// Visor de archivos de un item (como el de la v1): texto con SQL coloreado, Markdown, imágenes (click para
// verlas en tamaño real), PDF con el visor del sistema web, diagramas draw.io y Mermaid ("Ver código" alterna
// con el fuente). Todo local: el contenido llega por `read_file` / `read_media` (base64, con límite) y las
// imágenes y PDF se muestran desde un `blob:` que se libera al cerrar. "Abrir con la app" sigue disponible.

import { api } from "./api";
import { diagramKind, drawio, mermaid, mount, type DiagramPage } from "./diagrams";
import { esc, fmtSize, toast } from "./dom";
import { highlightSql } from "./highlight";
import { errorText, t } from "./i18n";
import { ico } from "./icons";
import { renderMarkdown } from "./markdown";
import type { FileEntry } from "./types";

interface ViewerState {
  f: FileEntry;
  body: HTMLElement;
  bar: HTMLElement;
  text?: string;
  /** Diagramas: mostrar el fuente en vez del dibujo. */
  source: boolean;
  pages?: DiagramPage[];
  page: number;
  url?: string;
  closed: boolean;
}

const msg = (key: string) => (key.startsWith("viewer.") ? t(key) : key);

function base64ToBlob(data: string, mime: string): Blob {
  const bin = atob(data);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return new Blob([bytes], { type: mime });
}

function toolbar(v: ViewerState): string {
  const kind = v.f.text ? diagramKind(v.f.name) : "";
  return `${kind ? `<button class="btn" data-v="source">${ico("code")}${esc(t(v.source ? "viewer.viewDiagram" : "viewer.viewSource"))}</button>` : ""}
    ${v.f.text ? `<button class="btn" data-v="copy">${ico("copy")}${esc(t("viewer.copy"))}</button>` : ""}
    <button class="btn" data-v="open">${ico("external")}${esc(t("viewer.openWith"))}</button>
    <button class="icon-btn" data-v="reveal" title="${esc(t("detail.reveal"))}">${ico("folder")}</button>
    <button class="icon-btn" data-v="close" title="${esc(t("common.close"))}">${ico("x")}</button>`;
}

function showPage(v: ViewerState, i: number): void {
  const p = v.pages?.[i];
  if (!p) return;
  v.page = i;
  const tabs =
    v.pages!.length > 1
      ? `<div class="diag-pages" role="tablist">${v.pages!.map((x, j) => `<button class="tab${j === i ? " sel" : ""}" data-v="page" data-page="${j}">${esc(x.name)}</button>`).join("")}</div>`
      : "";
  v.body.innerHTML = tabs + (p.error ? `<p class="warn">${ico("alert")}${esc(msg(p.error))}</p>` : `<div class="diagram" data-v="zoom" title="${esc(t("viewer.zoomHint"))}"></div>`);
  if (p.svg) mount(v.body.querySelector<HTMLElement>(".diagram")!, p.svg);
}

async function showDiagram(v: ViewerState): Promise<void> {
  const text = v.text ?? "";
  if (v.source) {
    v.body.innerHTML = `<pre class="code">${esc(text)}</pre>`;
    return;
  }
  if (v.pages) return showPage(v, v.page);
  v.body.innerHTML = `<p class="muted">${esc(t("viewer.drawing"))}</p>`;
  try {
    const pages =
      diagramKind(v.f.name) === "mermaid" ? [{ name: "", svg: await mermaid(text) }] : await drawio(text, (n) => t("viewer.page", { n }));
    if (v.closed) return;
    v.pages = pages;
    if (!v.source) showPage(v, 0);
  } catch (e) {
    if (v.closed || v.source) return;
    const m = (e as Error).message;
    const text2 = diagramKind(v.f.name) === "mermaid" ? t("viewer.mermaid.error", { msg: m }) : msg(m);
    v.body.innerHTML = `<p class="warn">${ico("alert")}${esc(text2)}</p><pre class="code">${esc(text)}</pre>`;
  }
}

async function load(v: ViewerState): Promise<void> {
  const f = v.f;
  v.body.innerHTML = `<p class="muted">${esc(t("common.loading"))}</p>`;
  try {
    if (f.media) {
      const m = await api.readMedia(f.path);
      if (v.closed) return;
      v.url = URL.createObjectURL(base64ToBlob(m.data, m.mime));
      v.body.innerHTML =
        m.mime === "application/pdf"
          ? `<iframe class="pdf" src="${esc(v.url)}" title="${esc(f.name)}"></iframe>`
          : `<div class="viewer-img" data-v="zoom" title="${esc(t("viewer.zoomHint"))}"><img src="${esc(v.url)}" alt="${esc(f.name)}"></div>`;
      return;
    }
    if (!f.text) {
      v.body.innerHTML = `<p class="empty">${esc(t("viewer.binary", { size: fmtSize(f.size) }))}</p>`;
      return;
    }
    v.text = await api.readFile(f.path);
    if (v.closed) return;
    const low = f.name.toLowerCase();
    if (diagramKind(low)) return showDiagram(v);
    v.body.innerHTML = low.endsWith(".md")
      ? `<div class="md">${renderMarkdown(v.text.replace(/^---\r?\n[\s\S]*?\r?\n---\r?\n/, ""))}</div>`
      : `<pre class="code">${low.endsWith(".sql") ? highlightSql(v.text) : esc(v.text)}</pre>`;
  } catch (err) {
    if (!v.closed) v.body.innerHTML = `<p class="warn">${ico("alert")}${esc(errorText(err))}</p>`;
  }
}

/** Abre el visor de un archivo del item (modal). */
export function openFileViewer(f: FileEntry): void {
  const back = document.createElement("div");
  back.className = "modal-back";
  const dir = f.path.split("/").slice(0, -1).join("/");
  back.innerHTML = `<div class="modal viewer" role="dialog" aria-modal="true" aria-label="${esc(f.name)}">
    <div class="viewer-head"><div class="grow"><h2>${esc(f.name)}</h2><div class="muted">${esc(dir)} · ${esc(fmtSize(f.size))}</div></div><div class="viewer-bar"></div></div>
    <div class="viewer-body"></div></div>`;
  document.body.append(back);
  const v: ViewerState = {
    f,
    body: back.querySelector<HTMLElement>(".viewer-body")!,
    bar: back.querySelector<HTMLElement>(".viewer-bar")!,
    source: false,
    page: 0,
    closed: false,
  };
  v.bar.innerHTML = toolbar(v);

  const close = () => {
    if (v.closed) return;
    v.closed = true;
    if (v.url) URL.revokeObjectURL(v.url);
    back.remove();
  };
  back.addEventListener("click", async (e) => {
    if (e.target === back) return close();
    const el = (e.target as HTMLElement).closest<HTMLElement>("[data-v]");
    if (!el || !back.contains(el)) return;
    switch (el.dataset.v) {
      case "close":
        return close();
      case "source":
        v.source = !v.source;
        v.bar.innerHTML = toolbar(v);
        return showDiagram(v);
      case "page":
        return showPage(v, Number(el.dataset.page));
      case "zoom":
        el.classList.toggle("full");
        return;
      case "copy":
        try {
          await navigator.clipboard.writeText(v.text ?? "");
          toast(t("viewer.copied"));
        } catch (err) {
          toast(errorText(err), undefined, "error");
        }
        return;
      case "open":
      case "reveal":
        try {
          await (el.dataset.v === "open" ? api.openPath(f.path) : api.reveal(f.path));
        } catch (err) {
          toast(errorText(err), undefined, "error");
        }
        return;
    }
  });
  back.tabIndex = -1;
  back.addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      e.stopPropagation();
      close();
    }
  });
  back.focus();
  void load(v);
}
