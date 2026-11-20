// Markdown mínimo para los README y las notas: títulos, párrafos, listas (con casillas), citas, código,
// tablas simples, énfasis y links. Todo el texto se escapa: el HTML del archivo nunca se interpreta.
// Bloques ```sql coloreados y ```mermaid dibujados a pedido (ver `diagrams`).

import { watchMermaidBlocks } from "./diagrams";
import { esc } from "./dom";
import { highlightSql } from "./highlight";
import { ico } from "./icons";

/** Bloque de código: SQL coloreado; Mermaid queda como código hasta que `diagrams` lo dibuja. */
function fencedBlock(lang: string, code: string): string {
  if (lang === "mermaid") {
    watchMermaidBlocks();
    return `<div class="mmd-block" data-mmd="${esc(code)}"><pre class="code"><code>${esc(code)}</code></pre></div>`;
  }
  const body = lang === "sql" ? highlightSql(code) : esc(code);
  return `<pre class="code" data-lang="${esc(lang)}"><code>${body}</code></pre>`;
}

function inline(s: string): string {
  let out = esc(s);
  out = out.replace(/`([^`]+)`/g, (_, c) => `<code>${c}</code>`);
  out = out.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
  out = out.replace(/(^|[^*])\*([^*\s][^*]*)\*/g, "$1<em>$2</em>");
  out = out.replace(/\[([^\]]+)\]\((https?:\/\/[^)\s]+)\)/g, '<a href="$2" target="_blank" rel="noreferrer">$1</a>');
  return out;
}

export function renderMarkdown(src: string): string {
  const lines = src.replace(/\r\n?/g, "\n").split("\n");
  const out: string[] = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    const fence = line.match(/^```(\w*)/);
    if (fence) {
      const code: string[] = [];
      i++;
      while (i < lines.length && !lines[i].startsWith("```")) code.push(lines[i++]);
      i++;
      out.push(fencedBlock(fence[1].toLowerCase(), code.join("\n")));
      continue;
    }
    const h = line.match(/^(#{1,6})\s+(.*)/);
    if (h) {
      out.push(`<h${h[1].length}>${inline(h[2])}</h${h[1].length}>`);
      i++;
      continue;
    }
    if (/^\s*[-*+]\s|^\s*\d+[.)]\s/.test(line)) {
      const ordered = /^\s*\d/.test(line);
      const items: string[] = [];
      while (i < lines.length && /^\s*([-*+]|\d+[.)])\s/.test(lines[i])) {
        const raw = lines[i].replace(/^\s*([-*+]|\d+[.)])\s/, "");
        const sub = lines[i].match(/^\s*/)![0].length > 0;
        const box = raw.match(/^\[( |x|X)\]\s?(.*)/);
        const cls = [sub ? "sub" : "", box ? "task" : "", box && box[1] !== " " ? "done" : ""].filter(Boolean).join(" ");
        const attr = cls ? ` class="${cls}"` : "";
        items.push(
          box
            ? `<li${attr}><span class="box">${box[1] === " " ? "" : ico("check")}</span>${inline(box[2])}</li>`
            : `<li${attr}>${inline(raw)}</li>`,
        );
        i++;
      }
      out.push(ordered ? `<ol>${items.join("")}</ol>` : `<ul>${items.join("")}</ul>`);
      continue;
    }
    if (line.startsWith(">")) {
      const quote: string[] = [];
      while (i < lines.length && lines[i].startsWith(">")) quote.push(lines[i++].replace(/^>\s?/, ""));
      out.push(`<blockquote>${inline(quote.join(" "))}</blockquote>`);
      continue;
    }
    if (line.includes("|") && /^\s*\|?[\s:-]+\|[\s|:-]*$/.test(lines[i + 1] ?? "")) {
      const cells = (l: string) => l.replace(/^\s*\||\|\s*$/g, "").split("|").map((c) => inline(c.trim()));
      const head = cells(line);
      i += 2;
      const rows: string[][] = [];
      while (i < lines.length && lines[i].includes("|")) rows.push(cells(lines[i++]));
      out.push(
        `<table><thead><tr>${head.map((c) => `<th>${c}</th>`).join("")}</tr></thead><tbody>${rows
          .map((r) => `<tr>${r.map((c) => `<td>${c}</td>`).join("")}</tr>`)
          .join("")}</tbody></table>`,
      );
      continue;
    }
    if (!line.trim()) {
      i++;
      continue;
    }
    const para: string[] = [];
    while (i < lines.length && lines[i].trim() && !/^(#{1,6}\s|```|>|\s*([-*+]|\d+[.)])\s)/.test(lines[i])) para.push(lines[i++]);
    out.push(`<p>${inline(para.join(" "))}</p>`);
  }
  return out.join("\n");
}
