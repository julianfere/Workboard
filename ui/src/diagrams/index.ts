// Diagramas del visor de archivos y de los bloques ```mermaid del Markdown, todo local y a pedido:
// - draw.io (.drawio/.dio): visor oficial 24.7.17 en `ui/public/vendor/drawio/` (se carga la primera vez) que
//   exporta cada página a SVG. Si no carga o falla en una página, queda el renderer propio (`drawio-fallback`).
// - Mermaid (.mmd/.mermaid y bloques del Markdown): paquete npm `mermaid`, importado a pedido (chunk aparte).
// Errores: `Error(message)` con una clave `viewer.*` (o el texto de Mermaid), la traduce quien muestra.
// Nada sale a internet: el modelo se limpia antes de dibujar (fuentes e imágenes remotas) y el SVG después.

import { t } from "../i18n";

export interface DiagramPage {
  name: string;
  svg?: string;
  /** Clave `viewer.*` del problema de esa página. */
  error?: string;
}

const DRAWIO = `${import.meta.env.BASE_URL}vendor/drawio`;

export function diagramKind(name: string): "drawio" | "mermaid" | "" {
  const low = name.toLowerCase();
  return /\.(drawio|dio)$/.test(low) ? "drawio" : /\.(mmd|mermaid)$/.test(low) ? "mermaid" : "";
}

// ---------- limpieza: nada remoto ----------

const SCHEME = /^\s*(https?:|ftp:|\/\/|javascript:)/i;

/** Una URL que sale de la app: otro origen (las del visor de draw.io son absolutas al propio origen). */
function remote(url: string): boolean {
  if (!SCHEME.test(url)) return false;
  try {
    const u = new URL(url.trim(), location.href);
    return u.protocol === "javascript:" || u.origin !== location.origin;
  } catch {
    return true;
  }
}

/** Saca del estilo de cada celda lo que haría pedir algo a internet (fuentes web, imágenes remotas). */
function sanitizeModel(model: Element): void {
  for (const el of [model, ...model.querySelectorAll("*")]) {
    const style = el.getAttribute("style");
    if (style) {
      const kept = style.split(";").filter((kv) => {
        const [k, ...rest] = kv.split("=");
        const raw = rest.join("=");
        let v = raw;
        try {
          v = decodeURIComponent(raw);
        } catch {
          /* queda el valor tal cual */
        }
        return k !== "fontSource" && !(k === "image" && remote(v));
      });
      el.setAttribute("style", kept.join(";"));
    }
    for (const attr of ["link", "image"]) if (remote(el.getAttribute(attr) ?? "")) el.removeAttribute(attr);
  }
}

/** SVG listo para insertar: sin scripts, handlers ni referencias remotas. Se arma en un `<template>` (documento
 *  inerte: no corre nada ni pide recursos) con el parser de HTML, que tolera las etiquetas HTML de los textos. */
export function sanitizeSvg(svg: string): string {
  const tpl = document.createElement("template");
  tpl.innerHTML = svg;
  const root = tpl.content;
  root.querySelectorAll("script, iframe, object, embed, link, meta").forEach((e) => e.remove());
  for (const el of root.querySelectorAll("*")) {
    for (const a of [...el.attributes]) {
      const n = a.name.toLowerCase();
      if (n.startsWith("on")) el.removeAttribute(a.name);
      else if ((n === "href" || n.endsWith(":href") || n === "src" || n === "srcset") && remote(a.value)) el.removeAttribute(a.name);
      else if (n === "style" && /url\(\s*['"]?\s*(https?:|\/\/)/i.test(a.value)) el.removeAttribute(a.name);
    }
    if (el.localName === "a") el.removeAttribute("target");
  }
  root.querySelectorAll("style").forEach((s) => {
    s.textContent = (s.textContent ?? "").replace(/@import[^;]*;?/gi, "").replace(/url\(\s*['"]?\s*(https?:|\/\/)[^)]*\)/gi, "none");
  });
  return root.querySelector("svg") ? tpl.innerHTML : "";
}

// ---------- draw.io ----------

async function inflate(b64: string): Promise<string> {
  const bytes = Uint8Array.from(atob(b64.replace(/\s+/g, "")), (c) => c.charCodeAt(0));
  const stream = new Blob([bytes]).stream().pipeThrough(new DecompressionStream("deflate-raw"));
  return decodeURIComponent(await new Response(stream).text());
}

const childNamed = (el: Element, name: string) => [...el.children].find((e) => e.nodeName === name);

interface ModelPage {
  name: string;
  model?: Element;
  error?: string;
}

async function pagesOf(xml: string, pageName: (n: number) => string): Promise<ModelPage[]> {
  const doc = new DOMParser().parseFromString(xml, "text/xml");
  if (doc.getElementsByTagName("parsererror").length) throw new Error("viewer.drawio.invalid");
  const root = doc.documentElement;
  if (root.nodeName === "mxGraphModel") return [{ name: "", model: root }];
  const diagrams = [...root.getElementsByTagName("diagram")];
  if (!diagrams.length) throw new Error("viewer.drawio.noDiagram");
  const pages: ModelPage[] = [];
  for (const d of diagrams) {
    const name = d.getAttribute("name") || pageName(pages.length + 1);
    let model = childNamed(d, "mxGraphModel");
    try {
      if (!model && d.textContent?.trim()) {
        const inner = new DOMParser().parseFromString(await inflate(d.textContent), "text/xml");
        model = inner.documentElement.nodeName === "mxGraphModel" ? inner.documentElement : inner.getElementsByTagName("mxGraphModel")[0];
      }
      pages.push(model ? { name, model } : { name, error: "viewer.diagram.empty" });
    } catch {
      pages.push({ name, error: "viewer.drawio.inflate" });
    }
  }
  return pages;
}

/* eslint-disable @typescript-eslint/no-explicit-any */
type Win = Window & Record<string, any>;
let drawioLoading: Promise<void> | null = null;

function loadDrawio(): Promise<void> {
  return (drawioLoading ||= new Promise<void>((ok, ko) => {
    const base = new URL(DRAWIO, location.href).href;
    Object.assign(window as Win, {
      STENCIL_PATH: `${DRAWIO}/stencils`,
      SHAPES_PATH: `${DRAWIO}/shapes`,
      STYLE_PATH: `${DRAWIO}/styles`,
      GRAPH_IMAGE_PATH: `${DRAWIO}/img`,
      IMAGE_PATH: `${DRAWIO}/img`,
      mxBasePath: `${DRAWIO}/mxgraph`,
      mxImageBasePath: `${DRAWIO}/mxgraph/images`,
      mxLoadResources: false,
      mxLoadStylesheets: false,
      DRAWIO_LOG_URL: "",
      PROXY_URL: "",
      // Los íconos de imagen (Azure, etc.) vienen como "img/lib/…" y el visor los resuelve contra esta base.
      DRAWIO_BASE_URL: base,
      DRAWIO_VIEWER_URL: base,
      // Stub vacío: sin MathJax (las fórmulas se ven como texto) y sin pedirlo a internet.
      DRAW_MATH_URL: `${DRAWIO}/math/es5`,
    });
    const s = document.createElement("script");
    s.src = `${DRAWIO}/viewer-static.min.js`;
    s.onload = () => ((window as Win).GraphViewer ? ok() : ko(new Error("viewer.drawio.load")));
    s.onerror = () => {
      drawioLoading = null;
      s.remove();
      ko(new Error("viewer.drawio.load"));
    };
    document.head.append(s);
  }));
}

function renderOfficial(model: Element): string {
  const w = window as Win;
  const div = document.createElement("div");
  div.style.cssText = "position:absolute;left:-20000px;top:0;width:1600px;visibility:hidden";
  document.body.append(div);
  let viewer: any = null;
  try {
    viewer = new w.GraphViewer(div, model, { lightbox: false, nav: false, tooltips: false, border: 12, "auto-fit": false, "check-visible-state": false });
    const bgAttr = model.getAttribute("background");
    const bg = !bgAttr || bgAttr === "none" ? "#ffffff" : bgAttr;
    const b = viewer.graph.getGraphBounds();
    if (!b.width && !b.height) throw new Error("viewer.diagram.empty");
    // getSvg(background, scale, border, nocrop, crisp, ignoreSelection, showText, imgExport, linkTarget,
    //        hasShadow, incExtFonts, theme, exportType, cells, noCssClass, disableLinks)
    const svg = viewer.graph.getSvg(bg, 1, 12, false, null, true, null, null, null, null, null, null, null, null, null, true);
    return new XMLSerializer().serializeToString(svg);
  } finally {
    try {
      viewer?.graph?.destroy();
    } catch {
      /* el visor puede haber quedado a medio armar */
    }
    div.remove();
  }
}
/* eslint-enable @typescript-eslint/no-explicit-any */

export async function drawio(xml: string, pageName: (n: number) => string): Promise<DiagramPage[]> {
  const pages = await pagesOf(xml, pageName);
  let official = true;
  try {
    await loadDrawio();
  } catch {
    official = false;
  }
  const out: DiagramPage[] = [];
  for (const p of pages) {
    if (!p.model) {
      out.push({ name: p.name, error: p.error });
      continue;
    }
    sanitizeModel(p.model);
    if (official) {
      try {
        out.push({ name: p.name, svg: sanitizeSvg(renderOfficial(p.model)) });
        continue;
      } catch (e) {
        if ((e as Error).message === "viewer.diagram.empty") {
          out.push({ name: p.name, error: "viewer.diagram.empty" });
          continue;
        }
        console.warn("draw.io:", e);
      }
    }
    try {
      const { renderPage } = await import("./drawio-fallback");
      out.push({ name: p.name, svg: sanitizeSvg(renderPage(p.model)) });
    } catch (e) {
      out.push({ name: p.name, error: (e as Error).message.startsWith("viewer.") ? (e as Error).message : "viewer.drawio.render" });
    }
  }
  return out;
}

// ---------- Mermaid ----------

type Mermaid = typeof import("mermaid").default;
let mermaidLoading: Promise<Mermaid> | null = null;
let mermaidTheme = "";
let seq = 0;
const mermaidCache = new Map<string, string>();

function loadMermaid(): Promise<Mermaid> {
  return (mermaidLoading ||= import("mermaid").then(
    (m) => m.default,
    (e) => {
      mermaidLoading = null;
      throw e;
    },
  ));
}

/** SVG de un diagrama Mermaid (cacheado por tema y fuente). Error: el mensaje de Mermaid, recortado. */
export async function mermaid(src: string): Promise<string> {
  const theme = document.documentElement.dataset.theme === "dark" ? "dark" : "default";
  const key = `${theme}\n${src}`;
  const hit = mermaidCache.get(key);
  if (hit) return hit;
  const m = await loadMermaid();
  if (theme !== mermaidTheme) {
    m.initialize({ startOnLoad: false, theme, securityLevel: "strict", fontFamily: getComputedStyle(document.body).fontFamily });
    mermaidTheme = theme;
  }
  const id = `wbmm${++seq}`;
  try {
    await m.parse(src);
    const svg = sanitizeSvg((await m.render(id, src)).svg);
    mermaidCache.set(key, svg);
    return svg;
  } catch (e) {
    document.getElementById(id)?.remove();
    document.getElementById(`d${id}`)?.remove();
    throw new Error(((e as Error)?.message || String(e)).split("\n").slice(0, 4).join("\n"));
  }
}

/** Inserta el SVG con su tamaño real (Mermaid lo manda con width=100%; el CSS lo ajusta al ancho). */
export function mount(el: HTMLElement, svg: string): void {
  el.innerHTML = svg;
  const s = el.querySelector("svg");
  const vb = s?.viewBox?.baseVal;
  if (s && vb && vb.width) {
    s.setAttribute("width", String(Math.ceil(vb.width)));
    s.setAttribute("height", String(Math.ceil(vb.height)));
    s.style.maxWidth = "";
  }
}

// ---------- bloques ```mermaid del Markdown ----------
// renderMarkdown deja un bloque con el código; cuando aparece en el DOM se reemplaza por el diagrama.

let observing = false;
let queued = false;

async function hydrate(): Promise<void> {
  queued = false;
  for (const el of document.querySelectorAll<HTMLElement>(".mmd-block:not([data-done])")) {
    if (el.dataset.done) continue; // otra pasada (por una mutación mientras esta esperaba) ya lo tomó
    el.dataset.done = "1";
    try {
      const svg = await mermaid(el.dataset.mmd ?? "");
      if (!el.isConnected) continue;
      const box = document.createElement("div");
      box.className = "diagram";
      mount(box, svg);
      el.replaceChildren(box);
    } catch (e) {
      const p = document.createElement("p");
      p.className = "warn";
      p.textContent = t("viewer.mermaid.error", { msg: (e as Error).message });
      el.prepend(p);
    }
  }
}

export function watchMermaidBlocks(): void {
  if (observing) return;
  observing = true;
  const kick = () => {
    if (!queued && document.querySelector(".mmd-block:not([data-done])")) {
      queued = true;
      requestAnimationFrame(() => void hydrate());
    }
  };
  new MutationObserver(kick).observe(document.documentElement, { childList: true, subtree: true });
  kick();
}
