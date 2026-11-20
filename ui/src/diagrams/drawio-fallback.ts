// @ts-nocheck -- direct port of the v1 fallback renderer (ui/diagrams.js); kept close to the original so
// both stay comparable. Only used when the official draw.io viewer cannot load or fails on a page.
//
// Renderer propio de draw.io: figuras comunes, flechas y texto; las figuras de librerías salen como
// rectángulos. Devuelve el SVG de una página (`<mxGraphModel>`).

const n2 = (v) => Math.round(v * 100) / 100;
const escX = (s) => String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
const child = (el, name) => [...el.children].find((e) => e.nodeName === name);

const SHAPES = new Set(["ellipse", "doubleEllipse", "rhombus", "text", "swimlane", "triangle", "image", "line", "cylinder", "cylinder3",
  "hexagon", "cloud", "actor", "umlActor", "process", "document", "note", "parallelogram", "trapezoid", "card", "partialRectangle", "label"]);
// Algunas figuras de la librería de flowchart, a su equivalente básico.
const ALIAS = { "mxgraph.flowchart.decision": "rhombus", "mxgraph.flowchart.terminator": "terminator", "mxgraph.flowchart.start_1": "ellipse",
  "mxgraph.flowchart.start_2": "ellipse", "mxgraph.flowchart.on-page_reference": "ellipse", "mxgraph.flowchart.database": "cylinder",
  "mxgraph.flowchart.document": "document", "mxgraph.flowchart.process": "process", "mxgraph.flowchart.data": "parallelogram",
  "mxgraph.flowchart.predefined_process": "process", "mxgraph.flowchart.manual_operation": "trapezoid", "mxgraph.basic.rect": "rectangle" };

function parseStyle(s) {
  const st = {}, names = [];
  for (const kv of (s || "").split(";")) {
    if (!kv) continue;
    const i = kv.indexOf("=");
    if (i < 0) names.push(kv); else st[kv.slice(0, i)] = kv.slice(i + 1);
  }
  st.shape = ALIAS[st.shape] || st.shape || names.find(x => SHAPES.has(x)) || "";
  st._names = names;
  return st;
}
function col(v, def) {
  if (v == null || v === "" || v === "default" || v === "inherit") return def;
  const m = /^light-dark\(\s*([^,]+),/.exec(v);
  return m ? m[1].trim() : v;
}

function geom(g) {
  if (!g) return null;
  const num = (e, a) => +(e.getAttribute(a) || 0), pt = e => ({ x: num(e, "x"), y: num(e, "y") });
  const geo = { x: num(g, "x"), y: num(g, "y"), w: num(g, "width"), h: num(g, "height"), rel: g.getAttribute("relative") === "1", points: [] };
  for (const e of g.children) {
    const as = e.getAttribute("as");
    if (e.nodeName === "mxPoint") { if (as === "sourcePoint") geo.sp = pt(e); else if (as === "targetPoint") geo.tp = pt(e); else if (as === "offset") geo.off = pt(e); }
    else if (e.nodeName === "Array" && as === "points") geo.points = [...e.children].filter(p => p.nodeName === "mxPoint").map(pt);
  }
  return geo;
}

function readModel(model) {
  const root = child(model, "root") || model, cells = new Map(), order = [];
  for (const el of root.children) {
    let c = el, value = el.getAttribute("value");
    if (el.nodeName !== "mxCell") { // <object>/<UserObject label="…"> envuelve al mxCell
      c = child(el, "mxCell"); if (!c) continue;
      value = el.getAttribute("label") || "";
      if (el.getAttribute("placeholders") === "1") value = value.replace(/%(\w+)%/g, (m, k) => el.getAttribute(k) ?? m);
    }
    const cell = { id: el.getAttribute("id"), parent: c.getAttribute("parent"), source: c.getAttribute("source"), target: c.getAttribute("target"),
      vertex: c.getAttribute("vertex") === "1", edge: c.getAttribute("edge") === "1", hidden: c.getAttribute("visible") === "0",
      collapsed: c.getAttribute("collapsed") === "1", value: value || "", st: parseStyle(c.getAttribute("style")), geo: geom(child(c, "mxGeometry")) };
    cells.set(cell.id, cell); order.push(cell);
  }
  return { cells, order };
}

// Caja absoluta de un vértice (la geometría de un hijo es relativa a su contenedor).
function boxOf(c, M, depth = 0) {
  if (c.box !== undefined) return c.box;
  c.box = null;
  if (!c.vertex || !c.geo || depth > 50) return null;
  const p = M.cells.get(c.parent), g = c.geo;
  if (g.rel) {
    if (!p || p.edge) return null; // texto de una flecha: se ubica sobre la flecha
    const pb = boxOf(p, M, depth + 1); if (!pb) return null;
    const off = g.off || { x: 0, y: 0 };
    return c.box = { x: pb.x + g.x * pb.w + off.x, y: pb.y + g.y * pb.h + off.y, w: g.w, h: g.h };
  }
  const pb = p && p.vertex ? boxOf(p, M, depth + 1) : null;
  return c.box = { x: (pb ? pb.x : 0) + g.x, y: (pb ? pb.y : 0) + g.y, w: g.w, h: g.h };
}
function visible(c, M) {
  for (let p = c, d = 0; p && d < 50; p = M.cells.get(p.parent), d++) {
    if (p.hidden || (p !== c && p.collapsed)) return false;
  }
  return true;
}

/* ---------- flechas ---------- */
const center = b => ({ x: b.x + b.w / 2, y: b.y + b.h / 2 });
const horiz = d => d === "E" || d === "W";
function boundary(b, toward, shape) {
  const c = center(b), dx = toward.x - c.x, dy = toward.y - c.y, hw = b.w / 2, hh = b.h / 2;
  if ((!dx && !dy) || !hw || !hh) return c;
  let t;
  if (shape === "ellipse" || shape === "doubleEllipse") t = 1 / Math.sqrt(dx * dx / (hw * hw) + dy * dy / (hh * hh));
  else if (shape === "rhombus") t = 1 / (Math.abs(dx) / hw + Math.abs(dy) / hh);
  else t = Math.min(dx ? hw / Math.abs(dx) : Infinity, dy ? hh / Math.abs(dy) : Infinity);
  return { x: c.x + dx * t, y: c.y + dy * t };
}
// Puerto fijo (exitX/exitY, entryX/entryY) y hacia qué lado sale.
function fixedPort(b, st, pre) {
  if (!b || st[pre + "X"] == null || st[pre + "Y"] == null) return null;
  const x = +st[pre + "X"], y = +st[pre + "Y"];
  const p = { x: b.x + b.w * x + (+st[pre + "Dx"] || 0), y: b.y + b.h * y + (+st[pre + "Dy"] || 0) };
  const d = x <= 0 ? "W" : x >= 1 ? "E" : y <= 0 ? "N" : y >= 1 ? "S" : Math.abs(x - .5) > Math.abs(y - .5) ? (x < .5 ? "W" : "E") : (y < .5 ? "N" : "S");
  return { p, d };
}
// Lado de `b` que mira hacia `q`; si q cae dentro del rango de un lado, sale derecho hacia él.
function portToward(b, q) {
  const c = center(b);
  if (q.x >= b.x && q.x <= b.x + b.w && (q.y < b.y || q.y > b.y + b.h)) return q.y < b.y ? { p: { x: q.x, y: b.y }, d: "N" } : { p: { x: q.x, y: b.y + b.h }, d: "S" };
  if (q.y >= b.y && q.y <= b.y + b.h && (q.x < b.x || q.x > b.x + b.w)) return q.x < b.x ? { p: { x: b.x, y: q.y }, d: "W" } : { p: { x: b.x + b.w, y: q.y }, d: "E" };
  const dx = (q.x - c.x) / (b.w || 1), dy = (q.y - c.y) / (b.h || 1);
  if (Math.abs(dx) >= Math.abs(dy)) return dx > 0 ? { p: { x: b.x + b.w, y: c.y }, d: "E" } : { p: { x: b.x, y: c.y }, d: "W" };
  return dy > 0 ? { p: { x: c.x, y: b.y + b.h }, d: "S" } : { p: { x: c.x, y: b.y }, d: "N" };
}
function autoPorts(sb, tb) {
  const ox0 = Math.max(sb.x, tb.x), ox1 = Math.min(sb.x + sb.w, tb.x + tb.w), oy0 = Math.max(sb.y, tb.y), oy1 = Math.min(sb.y + sb.h, tb.y + tb.h);
  if (ox1 > ox0 && (tb.y >= sb.y + sb.h || sb.y >= tb.y + tb.h)) {
    const x = (ox0 + ox1) / 2, dn = tb.y > sb.y;
    return [{ p: { x, y: dn ? sb.y + sb.h : sb.y }, d: dn ? "S" : "N" }, { p: { x, y: dn ? tb.y : tb.y + tb.h }, d: dn ? "N" : "S" }];
  }
  if (oy1 > oy0 && (tb.x >= sb.x + sb.w || sb.x >= tb.x + tb.w)) {
    const y = (oy0 + oy1) / 2, r = tb.x > sb.x;
    return [{ p: { x: r ? sb.x + sb.w : sb.x, y }, d: r ? "E" : "W" }, { p: { x: r ? tb.x : tb.x + tb.w, y }, d: r ? "W" : "E" }];
  }
  const s = portToward(sb, center(tb)), t = portToward(tb, center(sb));
  return [s, t];
}
function elbow(a, b) {
  const A = a.p, B = b.p;
  if (Math.abs(A.x - B.x) < .5 || Math.abs(A.y - B.y) < .5) return [];
  if (horiz(a.d) && horiz(b.d)) { const mx = (A.x + B.x) / 2; return [{ x: mx, y: A.y }, { x: mx, y: B.y }]; }
  if (!horiz(a.d) && !horiz(b.d)) { const my = (A.y + B.y) / 2; return [{ x: A.x, y: my }, { x: B.x, y: my }]; }
  return horiz(a.d) ? [{ x: B.x, y: A.y }] : [{ x: A.x, y: B.y }];
}
function orthThrough(pts, firstHoriz) {
  const out = [pts[0]];
  let h = firstHoriz;
  for (let i = 1; i < pts.length; i++) {
    const a = out[out.length - 1], b = pts[i];
    if (Math.abs(a.x - b.x) > .5 && Math.abs(a.y - b.y) > .5) out.push(h ? { x: b.x, y: a.y } : { x: a.x, y: b.y });
    else h = Math.abs(a.y - b.y) > .5;
    out.push(b);
  }
  return out;
}
function route(c, M) {
  const g = c.geo || { points: [] }, st = c.st;
  const ep = M.cells.get(c.parent), o = ep && ep.vertex && ep.box ? ep.box : { x: 0, y: 0 };
  const abs = p => p && { x: p.x + o.x, y: p.y + o.y };
  const S = M.cells.get(c.source), T = M.cells.get(c.target);
  const sb = S && S.box && S.box.w + S.box.h > 0 ? S.box : null, tb = T && T.box && T.box.w + T.box.h > 0 ? T.box : null;
  const sp = abs(g.sp), tp = abs(g.tp), P = g.points.map(abs);
  if ((!sb && !sp) || (!tb && !tp)) return null;
  const orth = /orthogonal|elbow|entityRelation|segment|isometric/i.test(st.edgeStyle || "");
  if (orth && sb && tb) {
    const fs = fixedPort(sb, st, "exit"), ft = fixedPort(tb, st, "entry");
    if (!P.length) {
      const [as, at] = autoPorts(sb, tb);
      const s = fs || (ft ? portToward(sb, ft.p) : as), t = ft || (fs ? portToward(tb, fs.p) : at);
      return [s.p, ...elbow(s, t), t.p];
    }
    const s = fs || portToward(sb, P[0]), t = ft || portToward(tb, P[P.length - 1]);
    const mid = orthThrough([s.p, ...P], horiz(s.d));
    const last = mid[mid.length - 1];
    if (Math.abs(last.x - t.p.x) > .5 && Math.abs(last.y - t.p.y) > .5) mid.push(horiz(t.d) ? { x: last.x, y: t.p.y } : { x: t.p.x, y: last.y });
    return [...mid, t.p];
  }
  const s = (sb && fixedPort(sb, st, "exit")?.p) || (sb ? boundary(sb, P[0] || (tb ? center(tb) : tp), S.st.shape) : sp);
  const t = (tb && fixedPort(tb, st, "entry")?.p) || (tb ? boundary(tb, P[P.length - 1] || s, T.st.shape) : tp);
  return [s, ...P, t];
}
function pointAlong(pts, frac) {
  const lens = [];
  let total = 0;
  for (let i = 1; i < pts.length; i++) { const l = Math.hypot(pts[i].x - pts[i - 1].x, pts[i].y - pts[i - 1].y); lens.push(l); total += l; }
  let d = total * Math.min(1, Math.max(0, frac));
  for (let i = 0; i < lens.length; i++) {
    if (d <= lens[i] || i === lens.length - 1) {
      const r = lens[i] ? Math.min(1, d / lens[i]) : 0, a = pts[i], b = pts[i + 1];
      return { x: a.x + (b.x - a.x) * r, y: a.y + (b.y - a.y) * r };
    }
    d -= lens[i];
  }
  return pts[0];
}
// Punta de flecha en `tip`, llegando desde `from`. Devuelve el SVG y cuánto acortar la línea.
function marker(type, filled, tip, from, size, stroke, sw) {
  if (!type || type === "none") return { svg: "", cut: 0 };
  const dx = tip.x - from.x, dy = tip.y - from.y, len = Math.hypot(dx, dy) || 1, ux = dx / len, uy = dy / len, nx = -uy, ny = ux;
  const thin = /Thin$/.test(type), L = size + 2, W = size * (thin ? .4 : .6);
  const at = (a, b) => `${n2(tip.x - ux * a + nx * b)},${n2(tip.y - uy * a + ny * b)}`;
  const paint = `stroke="${escX(stroke)}" stroke-width="${sw}" stroke-linejoin="round"`;
  const fill = filled ? escX(stroke) : "#ffffff";
  const line = (a, b) => `<polyline points="${a} ${b}" fill="none" ${paint}/>`;
  switch (type.replace(/Thin$/, "")) {
    case "block": return { svg: `<polygon points="${at(0, 0)} ${at(L, W)} ${at(L, -W)}" fill="${fill}" ${paint}/>`, cut: L };
    case "open": case "openAsync": return { svg: `<polyline points="${at(L, W)} ${at(0, 0)} ${at(L, -W)}" fill="none" ${paint}/>`, cut: 0 };
    case "oval": { const r = size / 2, cx = tip.x - ux * r, cy = tip.y - uy * r; return { svg: `<circle cx="${n2(cx)}" cy="${n2(cy)}" r="${n2(r)}" fill="${fill}" ${paint}/>`, cut: size }; }
    case "diamond": return { svg: `<polygon points="${at(0, 0)} ${at(L * .8, W)} ${at(L * 1.6, 0)} ${at(L * .8, -W)}" fill="${fill}" ${paint}/>`, cut: L * 1.6 };
    case "dash": return { svg: line(at(0, W), at(0, -W)), cut: 0 };
    case "ERmany": case "ERzeroToMany": case "ERoneToMany": {
      let s = line(at(L, 0), at(0, W)) + line(at(L, 0), at(0, -W));
      if (type === "ERoneToMany") s += line(at(L + 3, W), at(L + 3, -W));
      if (type === "ERzeroToMany") s += `<circle cx="${n2(tip.x - ux * (L + 6))}" cy="${n2(tip.y - uy * (L + 6))}" r="3.5" fill="#ffffff" ${paint}/>`;
      return { svg: s, cut: 0 };
    }
    case "ERone": case "ERmandOne": case "ERzeroToOne": {
      let s = line(at(L * .6, W), at(L * .6, -W));
      if (type === "ERmandOne") s += line(at(L, W), at(L, -W));
      if (type === "ERzeroToOne") s += `<circle cx="${n2(tip.x - ux * (L + 5))}" cy="${n2(tip.y - uy * (L + 5))}" r="3.5" fill="#ffffff" ${paint}/>`;
      return { svg: s, cut: 0 };
    }
    default: // classic y lo que no se conoce
      return { svg: `<polygon points="${at(0, 0)} ${at(L, W)} ${at(L * .7, 0)} ${at(L, -W)}" fill="${fill}" ${paint}/>`, cut: L * .7 };
  }
}
const dashArr = (st, sw) => st.dashPattern ? st.dashPattern.trim().split(/\s+/).map(v => n2(+v * sw)).join(",") : `${3 * sw},${3 * sw}`;

/* ---------- texto ---------- */
const measureCtx = document.createElement("canvas").getContext("2d");
function htmlToText(v) {
  const body = new DOMParser().parseFromString(`<body>${v}</body>`, "text/html").body;
  body.querySelectorAll("div,p,li,tr,h1,h2,h3,h4,h5,h6").forEach(b => { if (b.lastChild?.nodeName === "BR") b.lastChild.remove(); if (b.previousSibling) b.before("\n"); });
  body.querySelectorAll("br").forEach(b => b.replaceWith("\n"));
  return body.textContent.replace(/ /g, " ").replace(/^\n+|\s+$/g, "");
}
function fontOf(st, defSize) {
  const fs = +st.fontSize || defSize, fst = +st.fontStyle || 0, fam = st.fontFamily || "Helvetica";
  return { fs, bold: fst & 1, italic: fst & 2, under: fst & 4, fam: `${/\s/.test(fam) ? `"${fam}"` : fam}, Helvetica, Arial, sans-serif` };
}
function wrapLines(text, f, maxW) {
  measureCtx.font = `${f.italic ? "italic " : ""}${f.bold ? "bold " : ""}${f.fs}px ${f.fam}`;
  const w = s => measureCtx.measureText(s).width, lines = [];
  for (const para of text.split("\n")) {
    if (!maxW || w(para) <= maxW) { lines.push(para); continue; }
    let line = "";
    for (const word of para.split(" ")) {
      const test = line ? line + " " + word : word;
      if (line && w(test) > maxW) { lines.push(line); line = word; } else line = test;
    }
    lines.push(line);
  }
  return { lines, width: Math.max(0, ...lines.map(w)) };
}

/* ---------- una página ---------- */
export function renderPage(model: Element): string {
  const M = readModel(model), parts = [], bb = { x0: Infinity, y0: Infinity, x1: -Infinity, y1: -Infinity };
  const grow = (x, y, w = 0, h = 0) => { bb.x0 = Math.min(bb.x0, x); bb.y0 = Math.min(bb.y0, y); bb.x1 = Math.max(bb.x1, x + w); bb.y1 = Math.max(bb.y1, y + h); };

  function label(text, box, st, o = {}) {
    if (!text || st.noLabel === "1") return "";
    const f = fontOf(st, o.fs || 12), color = col(st.fontColor, "#000000");
    if (color === "none") return "";
    const sp = +(st.spacing ?? 2), sL = sp + (+st.spacingLeft || 0), sR = sp + (+st.spacingRight || 0), sT = sp + (+st.spacingTop || 0), sB = sp + (+st.spacingBottom || 0);
    const wrap = st.whiteSpace === "wrap" && box.w > 0 && !o.rot;
    const { lines, width } = wrapLines(text, f, wrap ? box.w - sL - sR : 0);
    const lh = f.fs * 1.2, total = lines.length * lh;
    const align = o.rot ? "center" : st.align || "center", va = o.rot ? "middle" : st.verticalAlign || "middle";
    const x = align === "left" ? box.x + sL : align === "right" ? box.x + box.w - sR : box.x + box.w / 2 + (sL - sR) / 2;
    const y0 = va === "top" ? box.y + sT : va === "bottom" ? box.y + box.h - sB - total : box.y + box.h / 2 - total / 2 + (sT - sB) / 2;
    const anchor = align === "left" ? "start" : align === "right" ? "end" : "middle";
    const bx = anchor === "start" ? x : anchor === "end" ? x - width : x - width / 2;
    let out = "";
    const bg = col(st.labelBackgroundColor, o.bg || "none"), bd = col(st.labelBorderColor, "none");
    if (bg !== "none" || bd !== "none") out += `<rect x="${n2(bx - 2)}" y="${n2(y0 - 1)}" width="${n2(width + 4)}" height="${n2(total + 2)}" fill="${escX(bg)}" stroke="${escX(bd)}"/>`;
    if (!o.rot) grow(bx - 2, y0 - 1, width + 4, total + 2);
    out += `<text font-family="${escX(f.fam)}" font-size="${f.fs}" fill="${escX(color)}" text-anchor="${anchor}"${f.bold ? ` font-weight="bold"` : ""}${f.italic ? ` font-style="italic"` : ""}${f.under ? ` text-decoration="underline"` : ""} xml:space="preserve">`
      + lines.map((l, i) => `<tspan x="${n2(x)}" y="${n2(y0 + i * lh + f.fs * .95)}">${escX(l)}</tspan>`).join("") + `</text>`;
    if (o.rot) { const c = center(box); out = `<g transform="rotate(-90 ${n2(c.x)} ${n2(c.y)})">${out}</g>`; }
    return out;
  }
  const textOf = c => !c.value ? "" : c.st.html === "1" ? htmlToText(c.value) : c.value;

  function shape(c, b) {
    const st = c.st, names = st._names, bare = names.includes("text") || names.includes("edgeLabel") || names.includes("group") || st.shape === "text";
    const fill = col(st.fillColor, bare ? "none" : "#ffffff"), stroke = col(st.strokeColor, bare ? "none" : "#000000"), sw = +st.strokeWidth || 1;
    let a = `fill="${escX(fill)}" stroke="${escX(stroke)}" stroke-width="${sw}"`;
    if (st.dashed === "1") a += ` stroke-dasharray="${dashArr(st, sw)}"`;
    if (st.opacity) a += ` opacity="${+st.opacity / 100}"`;
    if (st.fillOpacity) a += ` fill-opacity="${+st.fillOpacity / 100}"`;
    if (st.strokeOpacity) a += ` stroke-opacity="${+st.strokeOpacity / 100}"`;
    const { x, y, w, h } = b, cx = x + w / 2, cy = y + h / 2;
    const poly = (...p) => `<polygon points="${p.map(q => `${n2(q[0])},${n2(q[1])}`).join(" ")}" ${a}/>`;
    const path = d => `<path d="${d}" ${a}/>`;
    const sizeOf = (def, rel = true) => { const s = st.size != null ? +st.size : def; return rel && st.fixedSize !== "1" && s <= 1 ? s * w : s; };
    let s;
    switch (st.shape) {
      case "ellipse": s = `<ellipse cx="${n2(cx)}" cy="${n2(cy)}" rx="${n2(w / 2)}" ry="${n2(h / 2)}" ${a}/>`; break;
      case "doubleEllipse": s = `<ellipse cx="${n2(cx)}" cy="${n2(cy)}" rx="${n2(w / 2)}" ry="${n2(h / 2)}" ${a}/><ellipse cx="${n2(cx)}" cy="${n2(cy)}" rx="${n2(Math.max(0, w / 2 - 4))}" ry="${n2(Math.max(0, h / 2 - 4))}" ${a.replace(/fill="[^"]*"/, 'fill="none"')}/>`; break;
      case "rhombus": s = poly([cx, y], [x + w, cy], [cx, y + h], [x, cy]); break;
      case "triangle": {
        const d = st.direction || "east";
        s = d === "north" ? poly([cx, y], [x + w, y + h], [x, y + h]) : d === "south" ? poly([x, y], [x + w, y], [cx, y + h])
          : d === "west" ? poly([x + w, y], [x + w, y + h], [x, cy]) : poly([x, y], [x + w, cy], [x, y + h]);
        break;
      }
      case "hexagon": { const k = Math.min(sizeOf(.25), w / 2); s = poly([x + k, y], [x + w - k, y], [x + w, cy], [x + w - k, y + h], [x + k, y + h], [x, cy]); break; }
      case "parallelogram": { const k = Math.min(sizeOf(.2), w / 2); s = poly([x + k, y], [x + w, y], [x + w - k, y + h], [x, y + h]); break; }
      case "trapezoid": { const k = Math.min(sizeOf(.2), w / 2); s = poly([x + k, y], [x + w - k, y], [x + w, y + h], [x, y + h]); break; }
      case "cylinder": case "cylinder3": {
        const r = Math.min(h / 3, st.shape === "cylinder3" ? (+st.size || 15) : h * .15), rx = n2(w / 2);
        s = path(`M${n2(x)},${n2(y + r)} A${rx},${n2(r)} 0 0 1 ${n2(x + w)},${n2(y + r)} V${n2(y + h - r)} A${rx},${n2(r)} 0 0 1 ${n2(x)},${n2(y + h - r)} Z`)
          + `<path d="M${n2(x)},${n2(y + r)} A${rx},${n2(r)} 0 0 0 ${n2(x + w)},${n2(y + r)}" ${a.replace(/fill="[^"]*"/, 'fill="none"')}/>`;
        break;
      }
      case "document": {
        const dy = h * (st.size != null ? +st.size : .3) / 2;
        s = path(`M${n2(x)},${n2(y)} H${n2(x + w)} V${n2(y + h - dy)} Q${n2(x + w * .75)},${n2(y + h - dy * 2)} ${n2(cx)},${n2(y + h - dy)} T${n2(x)},${n2(y + h - dy)} Z`);
        break;
      }
      case "process": { const k = Math.min(sizeOf(.1), w / 2); s = `<rect x="${n2(x)}" y="${n2(y)}" width="${n2(w)}" height="${n2(h)}" ${a}/><path d="M${n2(x + k)},${n2(y)} V${n2(y + h)} M${n2(x + w - k)},${n2(y)} V${n2(y + h)}" ${a}/>`; break; }
      case "note": { const k = Math.min(+st.size || 30, w / 2, h / 2); s = path(`M${n2(x)},${n2(y)} H${n2(x + w - k)} L${n2(x + w)},${n2(y + k)} V${n2(y + h)} H${n2(x)} Z`) + path(`M${n2(x + w - k)},${n2(y)} V${n2(y + k)} H${n2(x + w)}`); break; }
      case "card": { const k = Math.min(+st.size || 30, w / 2, h / 2); s = poly([x + k, y], [x + w, y], [x + w, y + h], [x, y + h], [x, y + k]); break; }
      case "terminator": s = `<rect x="${n2(x)}" y="${n2(y)}" width="${n2(w)}" height="${n2(h)}" rx="${n2(Math.min(w, h) / 2)}" ${a}/>`; break;
      case "umlActor": case "actor": {
        const r = Math.min(w / 2, h / 8);
        s = `<circle cx="${n2(cx)}" cy="${n2(y + r)}" r="${n2(r)}" ${a}/><path d="M${n2(cx)},${n2(y + 2 * r)} V${n2(y + h * .65)} M${n2(x)},${n2(y + h * .35)} H${n2(x + w)} M${n2(x)},${n2(y + h)} L${n2(cx)},${n2(y + h * .65)} L${n2(x + w)},${n2(y + h)}" ${a.replace(/fill="[^"]*"/, 'fill="none"')}/>`;
        break;
      }
      case "line": s = `<line x1="${n2(x)}" y1="${n2(cy)}" x2="${n2(x + w)}" y2="${n2(cy)}" ${a}/>`; break;
      case "image": case "label": {
        let src = st.image || "";
        if (/^data:[^,;]+,/.test(src) && !/;base64,/.test(src)) src = src.replace(",", ";base64,"); // draw.io no guarda el ";base64" (choca con el separador de estilos)
        const frame = (fill !== "none" || stroke !== "none") && st.shape === "label" ? `<rect x="${n2(x)}" y="${n2(y)}" width="${n2(w)}" height="${n2(h)}" ${a}/>` : "";
        s = /^data:image\//.test(src) ? frame + `<image href="${escX(src)}" x="${n2(x)}" y="${n2(y)}" width="${n2(w)}" height="${n2(h)}" preserveAspectRatio="xMidYMid meet"/>`
          : st.shape === "label" ? frame : `<rect x="${n2(x)}" y="${n2(y)}" width="${n2(w)}" height="${n2(h)}" fill="none" stroke="#a1a1aa" stroke-dasharray="4,3"/>`;
        break;
      }
      case "partialRectangle": {
        const side = (k, d) => st[k] === "0" ? "" : d;
        s = `<rect x="${n2(x)}" y="${n2(y)}" width="${n2(w)}" height="${n2(h)}" ${a.replace(/stroke="[^"]*"/, 'stroke="none"')}/>`
          + `<path d="${side("top", `M${n2(x)},${n2(y)} H${n2(x + w)} `)}${side("right", `M${n2(x + w)},${n2(y)} V${n2(y + h)} `)}${side("bottom", `M${n2(x)},${n2(y + h)} H${n2(x + w)} `)}${side("left", `M${n2(x)},${n2(y)} V${n2(y + h)}`)}" ${a.replace(/fill="[^"]*"/, 'fill="none"')}/>`;
        break;
      }
      default: {
        const rnd = st.rounded === "1" ? (st.absoluteArcSize === "1" ? (+st.arcSize || 20) / 2 : Math.min(w, h) * (+st.arcSize || 15) / 100) : 0;
        s = `<rect x="${n2(x)}" y="${n2(y)}" width="${n2(w)}" height="${n2(h)}"${rnd ? ` rx="${n2(rnd)}"` : ""} ${a}/>`;
      }
    }
    return st.rotation && +st.rotation ? `<g transform="rotate(${+st.rotation} ${n2(cx)} ${n2(cy)})">${s}</g>` : s;
  }
  function swimlane(c, b) {
    const st = c.st, ss = +(st.startSize ?? 23), hor = st.horizontal !== "0", sw = +st.strokeWidth || 1;
    const fill = col(st.fillColor, "#ffffff"), body = col(st.swimlaneFillColor, "none"), stroke = col(st.strokeColor, "#000000");
    const head = hor ? { x: b.x, y: b.y, w: b.w, h: Math.min(ss, b.h) } : { x: b.x, y: b.y, w: Math.min(ss, b.w), h: b.h };
    const dash = st.dashed === "1" ? ` stroke-dasharray="${dashArr(st, sw)}"` : "";
    const r = q => `x="${n2(q.x)}" y="${n2(q.y)}" width="${n2(q.w)}" height="${n2(q.h)}"`;
    return `<rect ${r(b)} fill="${escX(body)}" stroke="${escX(stroke)}" stroke-width="${sw}"${dash}/><rect ${r(head)} fill="${escX(fill)}" stroke="${escX(stroke)}" stroke-width="${sw}"${dash}/>`
      + label(textOf(c), head, st, { rot: !hor });
  }
  function edge(c) {
    const st = c.st, pts = c.poly.map(p => ({ ...p })), stroke = col(st.strokeColor, "#000000"), sw = +st.strokeWidth || 1;
    let out = "";
    if (stroke !== "none") {
      const size = (k, def) => +st[k] || def;
      const end = marker(st.endArrow ?? "classic", st.endFill !== "0", pts[pts.length - 1], pts[pts.length - 2], size("endSize", 6), stroke, sw);
      const start = marker(st.startArrow ?? "none", st.startFill !== "0", pts[0], pts[1], size("startSize", 6), stroke, sw);
      const cut = (i, j, d) => { const a = pts[i], b = pts[j], l = Math.hypot(a.x - b.x, a.y - b.y); if (d && l > d) { a.x += (b.x - a.x) * d / l; a.y += (b.y - a.y) * d / l; } };
      cut(pts.length - 1, pts.length - 2, end.cut); cut(0, 1, start.cut);
      const dash = st.dashed === "1" ? ` stroke-dasharray="${dashArr(st, sw)}"` : "", op = st.opacity ? ` opacity="${+st.opacity / 100}"` : "";
      out += `<g${op}><polyline points="${pts.map(p => `${n2(p.x)},${n2(p.y)}`).join(" ")}" fill="none" stroke="${escX(stroke)}" stroke-width="${sw}" stroke-linejoin="round"${dash}/>${end.svg}${start.svg}</g>`;
    }
    const t = textOf(c);
    if (t) {
      const g = c.geo || {}, p = pointAlong(c.poly, g.rel !== false && g.x ? (g.x + 1) / 2 : .5), off = g.off || { x: 0, y: 0 };
      out += label(t, { x: p.x + off.x, y: p.y + off.y, w: 0, h: 0 }, st, { fs: 11, bg: "#ffffff" });
    }
    c.poly.forEach(p => grow(p.x, p.y));
    return out;
  }

  for (const c of M.order) if (c.vertex) boxOf(c, M);
  for (const c of M.order) if (c.edge) c.poly = route(c, M);
  for (const c of M.order) {
    if (!visible(c, M)) continue;
    if (c.edge) { if (c.poly && c.poly.length > 1) parts.push(edge(c)); continue; }
    if (!c.vertex) continue;
    const st = c.st;
    let b = c.box;
    if (!b) { // texto pegado a una flecha
      const e = M.cells.get(c.parent);
      if (!e?.poly || !c.geo) continue;
      const p = pointAlong(e.poly, (c.geo.x + 1) / 2), off = c.geo.off || { x: 0, y: 0 };
      b = { x: p.x + off.x - c.geo.w / 2, y: p.y + off.y - c.geo.h / 2, w: c.geo.w, h: c.geo.h };
    }
    const isLbl = st._names.includes("edgeLabel");
    if (st.shape === "swimlane") parts.push(swimlane(c, b));
    else {
      parts.push(shape(c, b));
      const lb = { ...b };
      if (st.labelPosition === "left") lb.x -= b.w; else if (st.labelPosition === "right") lb.x += b.w;
      if (st.verticalLabelPosition === "top") lb.y -= b.h; else if (st.verticalLabelPosition === "bottom") lb.y += b.h;
      parts.push(label(textOf(c), lb, st, isLbl ? { fs: 11, bg: "#ffffff" } : {}));
    }
    if (b.w || b.h) grow(b.x, b.y, b.w, b.h);
  }
  if (!parts.length || bb.x0 === Infinity) throw new Error("viewer.diagram.empty");
  const pad = 12, x = bb.x0 - pad, y = bb.y0 - pad, w = bb.x1 - bb.x0 + 2 * pad, h = bb.y1 - bb.y0 + 2 * pad;
  const bgAttr = model.getAttribute("background"), bg = !bgAttr || bgAttr === "none" ? "#ffffff" : bgAttr;
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${Math.ceil(w)}" height="${Math.ceil(h)}" viewBox="${n2(x)} ${n2(y)} ${n2(w)} ${n2(h)}">`
    + `<rect x="${n2(x)}" y="${n2(y)}" width="${n2(w)}" height="${n2(h)}" fill="${escX(bg)}"/>${parts.join("")}</svg>`;
}

