// Comandos de la promoción (cherry-pick o merge) como funciones puras: sin DOM ni estado global, para
// probarlas con `node --test` (ui/tests/promo.test.ts). La app solo ARMA los comandos; los corre el usuario.

import type { PromoRepo, PromoRow } from "./types";

export type Mode = "cherry" | "merge";
export type Shell = "ps" | "bash";

/** Textos de los comandos (de los locales, ya traducidos). */
export interface PromoTexts {
  /** Sufijo del nombre de la rama cuando hay más de tres ids (`y{n}mas` en la v1). */
  more: (n: number) => string;
  conflictCherry: string;
  conflictMerge: string;
  pr: string;
  failed: string;
}

export interface PromoOpts {
  mode: Mode;
  shell: Shell;
  /** Id del ambiente destino y su nombre visible. */
  target: string;
  targetLabel: string;
  /** Ids en el orden en que se eligieron los items. */
  ids: string[];
  stamp: string;
  /** Prefijo de las ramas de promoción (`Workflow.git.promotePrefix`). */
  prefix: string;
  isOn: (c: PromoRow) => boolean;
  text: PromoTexts;
}

/** Nombre de la rama nueva: los tres primeros ids en el orden elegido y el sufijo con cuántos más (v1). */
export function promoBranch(o: Pick<PromoOpts, "prefix" | "target" | "ids" | "stamp" | "text">): string {
  const ids = o.ids.length > 3 ? [...o.ids.slice(0, 3), o.text.more(o.ids.length - 3)] : o.ids;
  // Con la hora: volver a generar el mismo día no choca con la rama anterior.
  return `${o.prefix}cp-${o.target.toLowerCase()}-${ids.join("-")}-${o.stamp}`.replace(/[^\w./-]+/g, "-");
}

/** Ref de una rama para el merge: `origin/<rama>` si está ahí, si no el remote del destino, si no otro remote
 *  (como la v1), y la local si no está en ninguno. */
export function branchRef(b: { name: string; locations: string[] }, remote: string): string {
  const loc = b.locations.includes("origin") ? "origin" : b.locations.includes(remote) ? remote : b.locations.find((l) => l !== "local");
  return loc ? `${loc}/${b.name}` : b.name;
}

/** Pasos de git de un repo. Los commits seguidos van en un solo `cherry-pick -x a b c`: ante un conflicto
 *  frena ahí. Los merges van aparte con `-m 1`. */
export function promoSteps(r: PromoRepo, o: PromoOpts): { steps: string[]; notes: string[]; branch: string } | null {
  const branch = promoBranch(o);
  const steps = ["git fetch --all --prune", `git switch -c ${branch} ${r.targetRef}`];
  const notes: string[] = [];
  if (o.mode === "cherry") {
    const commits = (r.commits ?? []).filter(o.isOn);
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

/** URL para abrir el PR de la rama nueva contra la del destino (Azure, GitHub o GitLab). */
export function prUrl(r: PromoRepo, branch: string): string | null {
  if (!r.webUrl) return null;
  const src = encodeURIComponent(branch);
  const dst = encodeURIComponent(r.targetBranch ?? "");
  if (r.forge === "azure") return `${r.webUrl}/pullrequestcreate?sourceRef=${src}&targetRef=${dst}`;
  if (r.forge === "github") return `${r.webUrl}/compare/${dst.replace(/%2F/g, "/")}...${src.replace(/%2F/g, "/")}?expand=1`;
  if (r.forge === "gitlab") return `${r.webUrl}/-/merge_requests/new?merge_request[source_branch]=${src}&merge_request[target_branch]=${dst}`;
  return null;
}

/** El bloque de comandos de un repo, en PowerShell o Git Bash. */
export function promoCommands(r: PromoRepo, o: PromoOpts): string | null {
  const x = promoSteps(r, o);
  if (!x) return null;
  // El comentario de conflicto va en los dos modos (la v1 lo ponía siempre), con los pasos de cada uno.
  const conflict = o.mode === "cherry" ? o.text.conflictCherry : o.text.conflictMerge;
  const head = [`# ${r.name} → ${o.targetLabel} (${r.targetRef})`, ...x.notes.map((n) => `#   ${n}`), `# ${conflict}`];
  const url = prUrl(r, x.branch);
  const pr = url ? [`# ${o.text.pr}: ${url}`] : [];
  if (o.shell === "bash") return [...head, [`cd "${(r.path ?? "").replace(/\\/g, "/")}"`, ...x.steps].join(" && \\\n  "), ...pr].join("\n");
  // PowerShell 5.1 no tiene &&: un bloque que corta en el primer comando que falla (nunca llega al push).
  const failed = o.text.failed.replace(/"/g, "'");
  return [
    ...head,
    "& {",
    `  Set-Location "${r.path}"`,
    ...x.steps.map((c) => `  ${c}; if ($LASTEXITCODE -ne 0) { Write-Warning "${failed}: ${c.split(" ").slice(0, 2).join(" ")}"; return }`),
    "}",
    ...pr,
  ].join("\n");
}
