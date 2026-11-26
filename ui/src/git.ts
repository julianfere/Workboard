// Panel Git (solo lectura): la vista general de las ramas de cada item, la pestaña Git del detalle, las
// alertas (repos desparejos en un entorno, cambios sin commitear), los repos y entornos en Ajustes y la
// promoción: la app lee git y ARMA los comandos (cherry-pick o merge); los corre el usuario.
//
// Refresco: la vista general se calcula al abrir la carpeta y se relee cuando cambia la firma de `.git`
// de los repos (fetch, commit, checkout…) o el conjunto de ids de los items. Se sondea cada 3 s.

import { gitApi } from "./api";
import { onAction, onChange, render } from "./app";
import { esc, fmtAgo, toast } from "./dom";
import { errorText, label, t } from "./i18n";
import { ico } from "./icons";
import { S, envLabel, itemByName, titleOf, wf } from "./state";
import type { GitBranch, GitConfig, GitMention, GitOverview, GitRepoInfo, Item, Msg, PromoRepo, PromoRow, Promotion, RepoConfig } from "./types";

export const G = {
  /** Carpeta de trabajos de la vista general cargada. */
  root: "",
  data: null as GitOverview | null,
  loading: false,
  pending: false,
  error: "",
  sig: "",
  /** Ids de los items cuando se calculó (si cambian, se recalcula). */
  ids: "",
};

const STALE_FETCH = 24 * 3600;
const OK_STATES = new Set(["merged", "equivalent", "content", "mentioned"]);
const now = () => Date.now() / 1000;

/** Texto de un mensaje del backend, con los ambientes por su nombre del flujo. */
export function msgText(m: Msg): string {
  const p: Record<string, string> = { ...m.params };
  for (const k of ["env", "target", "source"]) if (p[k]) p[k] = envLabel(p[k]);
  return t(m.code, p);
}

function idsKey(): string {
  return [...new Set(S.items.flatMap((i) => i.ids))].sort().join(",");
}

export async function loadGit(refresh: boolean): Promise<void> {
  if (!S.settings?.root) return;
  if (G.loading) {
    G.pending = G.pending || refresh;
    return;
  }
  G.loading = true;
  G.error = "";
  const ids = idsKey();
  if (S.detail && S.detailTab === "git") render();
  try {
    G.data = await gitApi.overview(refresh);
    G.ids = ids;
    G.sig = await gitApi.signature();
  } catch (err) {
    G.error = errorText(err);
  }
  G.loading = false;
  if (G.pending) {
    G.pending = false;
    return loadGit(true);
  }
  render();
}

// Sondeo: carpeta nueva → carga; firma de .git o ids distintos → recalcula.
window.setInterval(async () => {
  if (document.hidden || !S.root || G.loading || document.querySelector(".modal-back")) return;
  if (G.root !== S.root) {
    Object.assign(G, { root: S.root, data: null, error: "", sig: "", ids: "" });
    return loadGit(false);
  }
  if (!G.data) return;
  try {
    const sig = await gitApi.signature();
    if (sig !== G.sig || idsKey() !== G.ids) await loadGit(true);
  } catch {
    /* la carpeta pudo cambiar: lo informa la próxima carga */
  }
}, 3000);

// ---------- datos de un item ----------

/** Ids de un item; un grupo cuenta como sus items. */
export function itemIds(it: Item): string[] {
  const ids = [...it.ids];
  if (it.kind === "group") for (const m of it.members) ids.push(...(itemByName(m)?.ids ?? []));
  return [...new Set(ids)];
}

interface GitInfo {
  branches: GitBranch[];
  /** repo → ambiente → commit que menciona el item. */
  mentions: Record<string, Record<string, GitMention>>;
}

export function gitInfo(it: Item): GitInfo | null {
  const ids = itemIds(it);
  if (!G.data || !ids.length) return null;
  const seen = new Set<string>();
  const branches: GitBranch[] = [];
  const mentions: GitInfo["mentions"] = {};
  for (const id of ids) {
    for (const b of G.data.byItem[id] ?? []) {
      const k = `${b.repo}|${b.name}`;
      if (!seen.has(k)) {
        seen.add(k);
        branches.push(b);
      }
    }
    for (const [repo, envs] of Object.entries(G.data.mentions[id] ?? {})) {
      for (const [env, c] of Object.entries(envs)) (mentions[repo] ??= {})[env] ??= c;
    }
  }
  return { branches, mentions };
}

/** Repos que se comparan entre sí (v1 app.js:2421: los que tienen QA). En términos del flujo: los que tienen
 *  configurado algún ambiente intermedio (ni el primero ni el último); un repo solo con el primero y el último
 *  (una base de datos que va por scripts) no se compara. Sin ambientes intermedios en el flujo, todos. */
function pairedRepos(repos: GitRepoInfo[]): GitRepoInfo[] {
  const envs = wf().environments.map((e) => e.id);
  const middle = envs.slice(1, -1);
  return repos.filter(
    (r) => !r.error && r.compare !== false && (!middle.length || middle.some((e) => r.envs?.[e] && !(r.missingEnvs ?? []).includes(e))),
  );
}

/** En un entorno, un repo ya tiene el trabajo y otro no (se mergean juntos). El primer ambiente del flujo
 *  no cuenta: suele recibir los arreglos después, por backflow. */
export function gitDesyncs(it: Item): { env: string; ok: string[]; missing: string[] }[] {
  const info = gitInfo(it);
  if (!info || !G.data) return [];
  const repos = pairedRepos(G.data.repos);
  const involved = repos.filter((r) => info.branches.some((b) => b.repo === r.name) || info.mentions[r.name]);
  if (involved.length < 2) return [];
  const out: { env: string; ok: string[]; missing: string[] }[] = [];
  for (const env of wf().environments.slice(1).map((e) => e.id)) {
    const ok: string[] = [];
    const missing: string[] = [];
    for (const r of involved.filter((x) => x.envs?.[env])) {
      const has = info.branches.some((b) => b.repo === r.name && OK_STATES.has(b.envs[env]?.state ?? "")) || !!info.mentions[r.name]?.[env];
      (has ? ok : missing).push(r.name);
    }
    if (ok.length && missing.length) out.push({ env, ok, missing });
  }
  return out;
}

/** Repos con archivos sin commitear cuya rama actual es del item. */
export function gitUncommitted(it: Item): { repo: string; branch: string; count: number; files: string[] }[] {
  const info = gitInfo(it);
  if (!info || !G.data) return [];
  return G.data.repos
    .filter((r) => r.dirtyCount && info.branches.some((b) => b.repo === r.name && b.current))
    .map((r) => ({ repo: r.name, branch: r.current ?? "", count: r.dirtyCount ?? 0, files: r.dirty ?? [] }));
}

/** Alertas de git de todos los items (para la vista de alertas). */
export function gitAlerts(): { kind: "gitDesync" | "gitDirty"; item: Item; detail: string }[] {
  if (!G.data) return [];
  const out: { kind: "gitDesync" | "gitDirty"; item: Item; detail: string }[] = [];
  for (const it of S.items) {
    if (it.kind !== "item") continue; // las alertas son de cada item, no del grupo (v1 app.js:149)
    for (const d of gitDesyncs(it)) out.push({ kind: "gitDesync", item: it, detail: t("git.desyncShort", { env: envLabel(d.env), missing: d.missing.join(", ") }) });
    for (const d of gitUncommitted(it)) out.push({ kind: "gitDirty", item: it, detail: t("git.dirtyShort", { repo: d.repo, n: d.count }) });
  }
  return out;
}

// ---------- pestaña Git del detalle ----------

const STATE_ICON: Record<string, string> = { merged: "check", equivalent: "copy", content: "file", mentioned: "message", pending: "clock" };

function badge(st: GitBranch["envs"][string] | undefined, env: string): string {
  if (!st) return `<span class="muted">—</span>`;
  const text = st.state === "pending" ? (st.ahead == null ? t("git.state.pending") : t("git.state.missing", { n: st.ahead })) : t(`git.state.${st.state}`);
  const tip = st.state === "pending" && st.ahead == null ? t("git.tip.pendingUnknown", { env: envLabel(env) }) : t(`git.tip.${st.state}`, { env: envLabel(env), n: st.ahead ?? 0 });
  return `<span class="gbadge ${OK_STATES.has(st.state) ? "ok" : "pend"}" title="${esc(tip)}">${ico(STATE_ICON[st.state] ?? "clock")}${esc(text)}</span>`;
}

function gitFooter(): string {
  const g = G.data;
  return `<div class="git-foot muted">${g ? esc(t("git.computed", { ago: fmtAgo(g.computedAt), seconds: g.seconds })) + " " : ""}${esc(t("git.readOnly"))}
    <button class="btn" data-action="git-refresh">${ico("refresh")}${esc(t("git.reload"))}</button>
    <button class="btn" data-action="git-settings">${ico("settings")}${esc(t("git.reposAndEnvs"))}</button></div>`;
}

export function renderGitTab(it: Item): string {
  if (!itemIds(it).length) return `<p class="empty">${esc(t("git.noIds"))}</p>`;
  if (G.error) return `<p class="error">${esc(G.error)}</p>${gitFooter()}`;
  if (!G.data) {
    if (!G.loading) queueMicrotask(() => void loadGit(false));
    return `<p class="empty">${esc(t("git.loading"))}</p>`;
  }
  const info = gitInfo(it)!;
  const banners = [
    ...gitDesyncs(it).map((d) => `<div class="git-warn">${ico("alert")}<span>${esc(t("git.desync", { env: envLabel(d.env), ok: d.ok.join(", "), missing: d.missing.join(", ") }))}</span></div>`),
    ...gitUncommitted(it).map(
      (d) => `<div class="git-warn">${ico("edit")}<span>${esc(t("git.dirty", { repo: d.repo, n: d.count, branch: d.branch }))}
        <details><summary>${esc(t("git.viewFiles"))}</summary><pre>${esc(d.files.join("\n"))}</pre></details></span></div>`,
    ),
  ].join("");
  const repos = G.data.repos
    .map((r) => {
      const brs = info.branches.filter((b) => b.repo === r.name);
      const men = info.mentions[r.name] ?? {};
      if (r.error) return `<section class="card git-repo"><div class="git-repo-head"><b>${esc(r.name)}</b><span class="error">${esc(msgText(r.error))}</span></div></section>`;
      if (!brs.length && !Object.keys(men).length) return "";
      const envs = Object.keys(r.envs ?? {});
      const age = r.lastFetch ? now() - r.lastFetch : null;
      const head = `<div class="git-repo-head"><b>${esc(r.name)}</b>
        <span class="muted">${esc(t("git.currentBranch"))} <code>${esc(r.current)}</code>${r.dirtyCount ? ` · ${esc(t("git.uncommitted", { n: r.dirtyCount }))}` : ""}</span>
        <span class="${age == null || age > STALE_FETCH ? "warn-text" : "muted"}" title="${esc(t("git.fetchHint"))}">· ${esc(t("git.lastFetch", { ago: r.lastFetch ? fmtAgo(r.lastFetch) : t("git.never") }))}</span>
        ${r.missingEnvs?.length ? `<span class="warn-text">· ${esc(t("git.missingEnvs", { envs: r.missingEnvs.map(envLabel).join(", ") }))}</span>` : ""}</div>`;
      const rows = brs
        .map(
          (b) => `<tr><td><code>${esc(b.name)}</code>${b.current ? ` <span class="chip">${esc(t("git.current"))}</span>` : ""}
            <div class="muted small">${b.locations.map(esc).join(" · ")}${b.ids.length > 1 ? ` · ${esc(b.ids.join(", "))}` : ""}</div></td>
          <td class="git-subj" title="${esc(b.tip.subject)}">${esc(b.tip.subject)}<div class="muted small">${esc(b.tip.author)} · ${esc(fmtAgo(b.tip.ts))} · ${esc(b.tip.loc)}</div></td>
          <td>${b.own}${b.files ? ` <span class="muted small">${esc(t("git.files", { n: b.files }))}</span>` : ""}</td>
          ${envs.map((e) => `<td>${badge(b.envs[e], e)}</td>`).join("")}</tr>`,
        )
        .join("");
      const table = brs.length
        ? `<table class="grid git-table"><thead><tr><th>${esc(t("git.branch"))}</th><th>${esc(t("git.lastCommit"))}</th><th title="${esc(t("git.ownHint"))}">${esc(t("git.own"))}</th>
            ${envs.map((e) => `<th>${esc(envLabel(e))} <span class="muted">${esc(r.envs![e])}</span></th>`).join("")}</tr></thead><tbody>${rows}</tbody></table>`
        : `<p class="muted">${esc(t("git.noBranches"))}</p>`;
      const mentions = Object.entries(men)
        .map(([env, c]) => `<div>${ico("message")}<b>${esc(envLabel(env))}</b> <code>${esc(c.sha)}</code> ${esc(c.subject)} <span class="muted">· ${esc(fmtAgo(c.ts))}</span></div>`)
        .join("");
      return `<section class="card git-repo">${head}${table}${mentions ? `<div class="git-men">${mentions}</div>` : ""}</section>`;
    })
    .join("");
  const empty = repos.trim() ? "" : `<p class="empty">${esc(t(G.data.repos.length ? "git.nothingFound" : "git.noRepos", { ids: itemIds(it).map((i) => `#${i}`).join(", ") }))}</p>`;
  return `${G.loading ? `<p class="muted">${esc(t("git.reloading"))}</p>` : ""}${banners}${repos}${empty}${gitFooter()}`;
}

onAction("git-refresh", () => loadGit(true));
onAction("git-settings", () => {
  S.view = "settings";
  S.detail = null;
  S.detailData = null;
  render();
  document.querySelector("#git-settings")?.scrollIntoView();
});

// ---------- repos y entornos (Ajustes) ----------

let cfg: GitConfig | null = null;
let cfgLoading = false;
let draft: RepoConfig[] | null = null;

async function loadConfig(): Promise<void> {
  cfgLoading = true;
  try {
    cfg = await gitApi.config();
    draft = cfg.repos.map((r) => ({ ...r, envs: { ...r.envs } }));
  } catch (err) {
    toast(errorText(err), undefined, "error");
  }
  cfgLoading = false;
  render();
}

export function gitSettingsCard(): string {
  if (!S.root) return "";
  if (!cfg || !draft) {
    if (!cfgLoading) void loadConfig();
    return `<section class="card" id="git-settings"><h2>${ico("branch")}${esc(t("git.settings"))}</h2><p class="muted">${esc(t("common.loading"))}</p></section>`;
  }
  const envs = wf().environments;
  const repos = draft
    .map(
      (r, i) => `<div class="git-cfg"><div class="git-cfg-head"><input class="git-cfg-name" value="${esc(r.name)}" data-change="git-cfg" data-i="${i}" data-key="name" aria-label="${esc(t("git.repoName"))}">
        <span class="muted grow">${esc(r.path)}</span>
        <label class="check"><input type="checkbox" data-change="git-cfg" data-i="${i}" data-key="compare"${r.compare ? " checked" : ""}>${esc(t("git.compare"))}</label>
        <button class="icon-btn" data-action="git-cfg-remove" data-i="${i}" title="${esc(t("common.delete"))}">${ico("trash")}</button></div>
        <div class="git-cfg-envs">${envs
          .map((e) => `<label class="field">${esc(label(e.labels, e.id))}<input value="${esc(r.envs[e.id] ?? "")}" data-change="git-cfg" data-i="${i}" data-key="env" data-env="${esc(e.id)}" placeholder="—"></label>`)
          .join("")}</div></div>`,
    )
    .join("");
  const refs = envs
    .map((e) => `<label class="field">${esc(label(e.labels, e.id))}<input value="${esc((cfg!.envRefs[e.id] ?? []).join(", "))}" data-change="git-refs" data-env="${esc(e.id)}" placeholder="origin/…"></label>`)
    .join("");
  return `<section class="card" id="git-settings"><h2>${ico("branch")}${esc(t("git.settings"))}</h2>
    <p class="muted">${esc(t(cfg.custom ? "git.settingsCustom" : "git.settingsDetected", { root: S.root.split(/[\\/]/).pop() ?? "" }))}</p>
    ${repos || `<p class="empty">${esc(t("git.noRepos"))}</p>`}
    <div class="add-row"><input id="git-add-path" placeholder="${esc(t("git.addPathHint"))}" data-enter="git-cfg-add"><button class="btn" data-action="git-cfg-add">${ico("plus")}${esc(t("git.addRepo"))}</button></div>
    <div class="card-actions"><button class="btn primary" data-action="git-cfg-save">${esc(t("git.saveRepos"))}</button>
      ${cfg.custom ? `<button class="btn" data-action="git-cfg-detect">${esc(t("git.useDetected", { n: cfg.detected.length }))}</button>` : ""}</div>
    <h3>${esc(t("git.envRefs"))}</h3><p class="muted">${esc(t("git.envRefsHint"))}</p>
    <div class="git-cfg-envs">${refs}</div>
    <div class="card-actions"><button class="btn" data-action="git-refs-save">${esc(t("git.saveEnvRefs"))}</button>
      ${cfg.customEnvRefs ? `<button class="btn" data-action="git-refs-reset">${esc(t("git.resetEnvRefs"))}</button>` : ""}</div></section>`;
}

async function saveConfig(patch: Parameters<typeof gitApi.setConfig>[0]): Promise<void> {
  cfg = await gitApi.setConfig(patch);
  draft = cfg.repos.map((r) => ({ ...r, envs: { ...r.envs } }));
  toast(t("git.saved"));
  render();
  G.data = null;
  void loadGit(true);
}

onChange("git-cfg", (el) => {
  const r = draft?.[Number(el.dataset.i)];
  if (!r) return;
  const input = el as HTMLInputElement;
  if (el.dataset.key === "name") r.name = input.value;
  else if (el.dataset.key === "compare") r.compare = input.checked;
  else r.envs[el.dataset.env!] = input.value.trim();
});
onAction("git-cfg-remove", (el) => {
  draft?.splice(Number(el.dataset.i), 1);
  render();
});
onAction("git-cfg-add", () => {
  const input = document.querySelector<HTMLInputElement>("#git-add-path");
  const path = input?.value.trim();
  if (!path || !draft) return;
  if (draft.some((r) => r.path.toLowerCase() === path.toLowerCase())) return toast(t("git.alreadyAdded"), undefined, "error");
  draft.push({ name: path.split(/[\\/]/).filter(Boolean).pop() ?? path, path, envs: {}, compare: true });
  render();
});
onAction("git-cfg-save", () => saveConfig({ repos: draft }));
onAction("git-cfg-detect", () => saveConfig({ repos: null }));
onAction("git-refs-save", () => {
  const envRefs: Record<string, string[]> = {};
  document.querySelectorAll<HTMLInputElement>('[data-change="git-refs"]').forEach((i) => {
    envRefs[i.dataset.env!] = i.value.split(",").map((x) => x.trim()).filter(Boolean);
  });
  return saveConfig({ envRefs });
});
onAction("git-refs-reset", () => saveConfig({ envRefs: null }));

// ---------- promoción ----------

type Mode = "cherry" | "merge";
type Shell = "ps" | "bash";

const PR = {
  names: [] as string[],
  ids: [] as string[],
  target: "",
  mode: "cherry" as Mode,
  shell: "ps" as Shell,
  data: null as Promotion | null,
  error: "",
  /** Commits cuyo tilde cambió el usuario. */
  toggled: new Set<string>(),
  stamp: "",
  back: null as HTMLElement | null,
};

function pref(key: string, fallback: string): string {
  try {
    return localStorage.getItem(`wb.git.${key}`) ?? fallback;
  } catch {
    return fallback;
  }
}

function setPref(key: string, value: string): void {
  try {
    localStorage.setItem(`wb.git.${key}`, value);
  } catch {
    /* sin almacenamiento: solo no se recuerda */
  }
}

/** Ambientes a los que se puede promover (todos menos el primero, del que sale el trabajo). */
const targets = () => wf().environments.slice(1).map((e) => e.id);

function stamp(): string {
  const d = new Date();
  const p2 = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}${p2(d.getMonth() + 1)}${p2(d.getDate())}-${p2(d.getHours())}${p2(d.getMinutes())}`;
}

const promoOn = (c: PromoRow) => (PR.toggled.has(c.sha) ? !c.on : c.on);

function promoBranch(): string {
  const prefix = (wf() as { git?: { promotePrefix?: string } }).git?.promotePrefix ?? "promote/";
  const ids = PR.ids.length > 3 ? [...PR.ids.slice(0, 3), `and${PR.ids.length - 3}`] : PR.ids;
  // Con la hora: volver a generar el mismo día no choca con la rama anterior.
  return `${prefix}cp-${PR.target.toLowerCase()}-${ids.join("-")}-${PR.stamp}`.replace(/[^\w./-]+/g, "-");
}

function branchRef(b: { name: string; locations: string[] }, remote: string): string {
  const loc = b.locations.includes(remote) ? remote : b.locations.find((l) => l !== "local");
  return loc ? `${loc}/${b.name}` : b.name;
}

/** Pasos de git de un repo. Los commits seguidos van en un solo `cherry-pick -x a b c`: ante un conflicto
 *  frena ahí. Los merges van aparte con `-m 1`. */
function promoSteps(r: PromoRepo): { steps: string[]; notes: string[]; branch: string } | null {
  const branch = promoBranch();
  const steps = ["git fetch --all --prune", `git switch -c ${branch} ${r.targetRef}`];
  const notes: string[] = [];
  if (PR.mode === "cherry") {
    const commits = (r.commits ?? []).filter(promoOn);
    if (!commits.length) return null;
    let run: string[] = [];
    const flush = () => {
      if (run.length) steps.push(`git cherry-pick -x ${run.join(" ")}`);
      run = [];
    };
    for (const c of commits) {
      if (c.merge) {
        flush();
        steps.push(`git cherry-pick -x -m 1 ${c.short}`);
      } else run.push(c.short);
    }
    flush();
    for (const c of commits) notes.push(`${c.short}  ${c.ids.map((i) => `#${i}`).join(" ")} · ${c.subject.replace(/[\r\n]+/g, " ").slice(0, 80)}`);
  } else {
    const brs = (r.branches ?? []).filter((b) => b.commits.length && !b.fromEnv);
    if (!brs.length) return null;
    for (const b of brs) steps.push(`git merge --no-ff ${branchRef(b, r.remote ?? "")}`);
  }
  steps.push(`git push -u ${r.remote} ${branch}`);
  return { steps, notes, branch };
}

function prUrl(r: PromoRepo, branch: string): string | null {
  if (!r.webUrl) return null;
  const src = encodeURIComponent(branch);
  const dst = encodeURIComponent(r.targetBranch ?? "");
  if (r.forge === "azure") return `${r.webUrl}/pullrequestcreate?sourceRef=${src}&targetRef=${dst}`;
  if (r.forge === "github") return `${r.webUrl}/compare/${dst.replace(/%2F/g, "/")}...${src.replace(/%2F/g, "/")}?expand=1`;
  if (r.forge === "gitlab") return `${r.webUrl}/-/merge_requests/new?merge_request[source_branch]=${src}&merge_request[target_branch]=${dst}`;
  return null;
}

function promoCommands(r: PromoRepo): string | null {
  const x = promoSteps(r);
  if (!x) return null;
  const head = [`# ${r.name} → ${envLabel(PR.target)} (${r.targetRef})`, ...x.notes.map((n) => `#   ${n}`)];
  if (PR.mode === "cherry") head.push(`# ${t("promo.cmd.conflict")}`);
  const url = prUrl(r, x.branch);
  const pr = url ? [`# ${t("promo.cmd.pr")}: ${url}`] : [];
  if (PR.shell === "bash") return [...head, [`cd "${(r.path ?? "").replace(/\\/g, "/")}"`, ...x.steps].join(" && \\\n  "), ...pr].join("\n");
  // PowerShell 5.1 no tiene &&: un bloque que corta en el primer comando que falla (nunca llega al push).
  const failed = t("promo.cmd.failed").replace(/"/g, "'");
  return [
    ...head,
    "& {",
    `  Set-Location "${r.path}"`,
    ...x.steps.map((c) => `  ${c}; if ($LASTEXITCODE -ne 0) { Write-Warning "${failed}: ${c.split(" ").slice(0, 2).join(" ")}"; return }`),
    "}",
    ...pr,
  ].join("\n");
}

const FLAG_SHORT: Record<string, string> = { otherItem: "promo.flagShort.otherItem", shared: "promo.flagShort.shared", likely: "promo.flagShort.likely", merge: "promo.flagShort.merge" };

function commitRow(c: PromoRow): string {
  const on = promoOn(c);
  return `<label class="promo-c${on ? "" : " off"}"><input type="checkbox" data-change="promo-toggle" data-sha="${esc(c.sha)}"${on ? " checked" : ""}>
      <code>${esc(c.short)}</code><span class="pc-subj" title="${esc(c.subject)}">${esc(c.subject)}</span>
      ${c.flags.map((f) => `<span class="pflag f-${esc(f.kind)}" title="${esc(msgText(f))}">${esc(t(FLAG_SHORT[f.kind] ?? f.kind))}</span>`).join("")}
      <span class="muted pc-meta">${esc(c.author)} · ${esc(fmtAgo(c.ts))}</span></label>
    ${!on && c.flags.length ? `<div class="muted promo-why">${c.flags.map((f) => esc(msgText(f))).join(" · ")}</div>` : ""}`;
}

function repoBlock(r: PromoRepo): string {
  if (r.error) return `<div class="promo-repo"><div class="pr-head"><b>${esc(r.name)}</b><span class="error">${esc(msgText(r.error))}</span></div></div>`;
  const x = promoSteps(r);
  const cmds = promoCommands(r);
  const commits = r.commits ?? [];
  const on = commits.filter(promoOn);
  const off = commits.filter((c) => !promoOn(c));
  const warnings = (r.branches ?? []).flatMap((b) => b.warnings.map((w) => `${b.fromEnv ? "" : `${b.name}: `}${msgText(w)}`));
  const fromEnv = new Set((r.branches ?? []).filter((b) => b.fromEnv).map((b) => b.name));
  const origin = (c: PromoRow) => (fromEnv.has(c.branch) ? t("promo.mentionedIn", { env: envLabel(r.sourceEnv ?? "") }) : c.branch);
  const groups = [...new Set(on.map(origin))];
  return `<div class="promo-repo">
    <div class="pr-head"><b>${esc(r.name)}</b><span class="muted">→ ${esc(r.targetRef)}</span><span class="grow"></span>
      ${cmds ? `<button class="btn" data-action="promo-copy" data-repo="${esc(r.name)}">${ico("copy")}${esc(t("promo.copyRepo"))}</button>` : ""}</div>
    ${x ? `<div class="pr-branch"><span class="muted">${esc(t("promo.newBranch"))}</span><code>${esc(x.branch)}</code><span class="muted">${esc(t("promo.from", { ref: r.targetRef ?? "" }))}</span></div>` : ""}
    <div class="pr-commits">${
      on.length
        ? groups.map((g) => `<div class="pr-src">${esc(g)}</div>${on.filter((c) => origin(c) === g).map(commitRow).join("")}`).join("")
        : `<p class="muted">${esc(t("promo.noneChecked"))}</p>`
    }</div>
    ${warnings.map((w) => `<div class="promo-note warn">${ico("alert")}${esc(w)}</div>`).join("")}
    ${off.length ? `<details class="pr-off"><summary>${esc(t("promo.notIncluded", { n: off.length }))}</summary>${off.map(commitRow).join("")}</details>` : ""}
    ${cmds ? `<details class="pr-cmds"><summary>${esc(t("promo.viewCommands"))}</summary><pre class="code promo-cmds">${esc(cmds)}</pre></details>` : ""}
  </div>`;
}

function renderPromotion(): void {
  if (!PR.back) return;
  const title = PR.ids.length === 1 ? t("promo.titleOne", { id: PR.ids[0], env: envLabel(PR.target) }) : t("promo.titleMany", { n: PR.ids.length, env: envLabel(PR.target) });
  const seg = (action: string, value: string, current: string, text: string) =>
    `<button class="btn${value === current ? " primary" : ""}" data-action="${action}" data-value="${value}">${esc(text)}</button>`;
  const controls = `<div class="promo-ctl">
      <label class="inline">${esc(t("promo.target"))} <select data-change="promo-target">${targets().map((e) => `<option value="${esc(e)}"${e === PR.target ? " selected" : ""}>${esc(envLabel(e))}</option>`).join("")}</select></label>
      <span class="seg" title="${esc(t("promo.modeHint"))}">${seg("promo-mode", "cherry", PR.mode, t("promo.mode.cherry"))}${seg("promo-mode", "merge", PR.mode, t("promo.mode.merge"))}</span>
      <span class="seg">${seg("promo-shell", "ps", PR.shell, "PowerShell")}${seg("promo-shell", "bash", PR.shell, "Git Bash")}</span>
      <span class="grow"></span><button class="btn" data-action="promo-reload" title="${esc(t("promo.reloadHint"))}">${ico("refresh")}${esc(t("promo.reload"))}</button></div>`;
  let body: string;
  if (PR.error) body = `<p class="error">${esc(PR.error)}</p>`;
  else if (!PR.data) body = `<p class="empty">${esc(t("promo.loading", { env: envLabel(PR.target) }))}</p>`;
  else {
    const repos = PR.data.repos.filter((r) => r.error || r.commits?.length);
    const idle = PR.data.repos.filter((r) => !r.error && !r.commits?.length);
    const warnings: string[] = [];
    for (const r of repos.filter((x) => !x.error && (!x.lastFetch || now() - x.lastFetch > STALE_FETCH))) {
      warnings.push(t("promo.staleFetch", { repo: r.name, ago: r.lastFetch ? fmtAgo(r.lastFetch) : t("git.never") }));
    }
    const ok = PR.data.repos.filter((r) => !r.error);
    for (const id of PR.ids) {
      const where = ok.filter((r) => (r.branches ?? []).some((b) => !b.fromEnv && b.ids.includes(id))).map((r) => r.name);
      if (where.length === 1 && ok.length > 1) warnings.push(t("promo.oneRepo", { id, repo: where[0] }));
    }
    body =
      warnings.map((w) => `<div class="promo-warn">${ico("alert")}<span>${esc(w)}</span></div>`).join("") +
      repos.map(repoBlock).join("") +
      (repos.length ? "" : `<p class="empty">${esc(t("promo.nothing", { ids: PR.ids.map((i) => `#${i}`).join(" "), env: envLabel(PR.target) }))}</p>`) +
      (idle.length ? `<p class="muted">${esc(t("promo.idle", { env: envLabel(PR.target), repos: idle.map((r) => r.name).join(", ") }))}</p>` : "");
  }
  const any = !!PR.data && PR.data.repos.some((r) => !r.error && promoCommands(r));
  PR.back.innerHTML = `<div class="modal promo" role="dialog" aria-modal="true">
    <div class="viewer-head"><h2>${esc(title)}</h2><button class="icon-btn" data-action="promo-close" title="${esc(t("common.close"))}">${ico("x")}</button></div>
    <p class="muted">${PR.names.map((n) => esc(itemByName(n) ? titleOf(itemByName(n)!) : n)).join(" · ")}</p>
    ${controls}<div class="promo-body">${body}</div>
    <div class="modal-actions"><span class="muted small grow">${esc(t("promo.footer"))}</span>
      <button class="btn" data-action="promo-close">${esc(t("common.close"))}</button>
      <button class="btn primary" data-action="promo-copy" data-repo="*"${any ? "" : " disabled"}>${ico("copy")}${esc(t("promo.copyAll"))}</button></div></div>`;
}

async function loadPromotion(): Promise<void> {
  PR.data = null;
  PR.error = "";
  renderPromotion();
  try {
    PR.data = await gitApi.promotion(PR.ids, PR.target);
  } catch (err) {
    PR.error = errorText(err);
  }
  renderPromotion();
}

function closePromotion(): void {
  PR.back?.remove();
  PR.back = null;
}

/** Abre la promoción de estos items (por nombre de carpeta). */
export async function openPromotion(names: string[]): Promise<void> {
  const items = names.map(itemByName).filter((x): x is Item => !!x);
  const ids = [...new Set(items.flatMap(itemIds))].sort();
  if (!ids.length) return toast(t("promo.noIds"), undefined, "error");
  const envs = targets();
  if (!envs.length) return toast(t("promo.noTargets"), undefined, "error");
  const target = pref("target", envs[0]);
  Object.assign(PR, {
    names: items.map((i) => i.name),
    ids,
    target: envs.includes(target) ? target : envs[0],
    mode: pref("mode", "cherry") === "merge" ? "merge" : "cherry",
    shell: pref("shell", navigator.userAgent.includes("Windows") ? "ps" : "bash") === "bash" ? "bash" : "ps",
    toggled: new Set<string>(),
    stamp: stamp(),
  });
  closePromotion();
  const back = document.createElement("div");
  back.className = "modal-back";
  back.tabIndex = -1;
  // Sin devolver `false` (cancelaría el clic en los checkboxes del modal).
  back.addEventListener("click", (e) => {
    if (e.target === back) closePromotion();
  });
  back.addEventListener("keydown", (e) => {
    if (e.key === "Escape") closePromotion();
  });
  document.body.append(back);
  PR.back = back;
  back.focus();
  await loadPromotion();
}

async function copy(text: string, ok: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
    toast(ok);
  } catch {
    toast(t("promo.copyFailed"), undefined, "error");
  }
}

onAction("promo-close", () => closePromotion());
onAction("promo-mode", (el) => {
  PR.mode = el.dataset.value === "merge" ? "merge" : "cherry";
  setPref("mode", PR.mode);
  renderPromotion();
});
onAction("promo-shell", (el) => {
  PR.shell = el.dataset.value === "bash" ? "bash" : "ps";
  setPref("shell", PR.shell);
  renderPromotion();
});
onAction("promo-reload", () => {
  PR.stamp = stamp();
  return loadPromotion();
});
onChange("promo-target", (el) => {
  PR.target = (el as HTMLSelectElement).value;
  setPref("target", PR.target);
  PR.toggled.clear();
  return loadPromotion();
});
onChange("promo-toggle", (el) => {
  const sha = el.dataset.sha!;
  const c = PR.data?.repos.flatMap((r) => r.commits ?? []).find((x) => x.sha === sha);
  if (c) (el as HTMLInputElement).checked === c.on ? PR.toggled.delete(sha) : PR.toggled.add(sha);
  renderPromotion();
});
onAction("promo-copy", (el) => {
  const which = el.dataset.repo;
  const repos = (PR.data?.repos ?? []).filter((r) => !r.error && (which === "*" || r.name === which));
  const text = repos.map(promoCommands).filter(Boolean).join("\n\n");
  if (text) return copy(text, t("promo.copied"));
});
