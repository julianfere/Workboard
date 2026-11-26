// Comandos de la promoción (ui/src/promo.ts): `npm test` (node --test, sin dependencias).
import assert from "node:assert/strict";
import { test } from "node:test";

import { branchRef, promoBranch, promoCommands, type PromoOpts } from "../src/promo.ts";
import type { PromoRepo, PromoRow } from "../src/types.ts";

const es = JSON.parse(await import("node:fs").then((fs) => fs.readFileSync(new URL("../../locales/es.json", import.meta.url), "utf8")));
const fill = (s: string, p: Record<string, unknown> = {}) => s.replace(/\{(\w+)\}/g, (_, k) => String(p[k]));

function opts(o: Partial<PromoOpts> = {}): PromoOpts {
  return {
    mode: "cherry",
    shell: "bash",
    target: "qa",
    targetLabel: "QA",
    ids: ["111111"],
    stamp: "20261126-0930",
    prefix: "promote/",
    isOn: (c) => c.on,
    text: {
      more: (n) => fill(es["promo.branchMore"], { n }),
      conflictCherry: es["promo.cmd.conflict"],
      conflictMerge: es["promo.cmd.conflictMerge"],
      pr: es["promo.cmd.pr"],
      failed: es["promo.cmd.failed"],
    },
    ...o,
  };
}

const row = (short: string, extra: Partial<PromoRow> = {}): PromoRow => ({
  sha: `${short}000`, short, ts: 0, author: "Ana", parents: [], subject: `WI 111111 ${short}`, branch: "Feature/111111-login",
  ids: ["111111"], merge: false, flags: [], on: true, ...extra,
});

function repo(locations: string[]): PromoRepo {
  return {
    name: "app", path: String.raw`C:\src\app`, targetRef: "origin/qa", remote: "origin", targetBranch: "qa", webUrl: null, forge: null,
    branches: [{ name: "Feature/111111-login", ids: ["111111"], locations, commits: [row("aaa1111")], notes: [], warnings: [], alreadyCp: 0 }],
    commits: [row("aaa1111"), row("bbb2222", { merge: true }), row("ccc3333"), row("ddd4444", { on: false })],
  };
}

test("branch name keeps the selection order and the v1 suffix", () => {
  assert.equal(promoBranch(opts({ ids: ["333333", "111111"] })), "promote/cp-qa-333333-111111-20261126-0930");
  assert.equal(
    promoBranch(opts({ ids: ["555555", "222222", "444444", "111111", "333333"] })),
    "promote/cp-qa-555555-222222-444444-y2mas-20261126-0930",
  );
  assert.equal(promoBranch(opts({ ids: ["1", "2", "3", "4"], text: { ...opts().text, more: (n) => `and${n}more` } })), "promote/cp-qa-1-2-3-and1more-20261126-0930");
});

test("merge prefers origin, then the target remote, then another remote, then local", () => {
  const b = (locations: string[]) => ({ name: "Feature/1-x", locations });
  assert.equal(branchRef(b(["local", "fork", "origin", "upstream"]), "upstream"), "origin/Feature/1-x");
  assert.equal(branchRef(b(["local", "fork", "upstream"]), "upstream"), "upstream/Feature/1-x");
  assert.equal(branchRef(b(["local", "fork"]), "upstream"), "fork/Feature/1-x");
  assert.equal(branchRef(b(["local"]), "upstream"), "Feature/1-x");
});

test("cherry-pick commands group runs, split merges and carry the conflict comment", () => {
  const out = promoCommands(repo(["local", "origin"]), opts());
  assert.equal(
    out,
    [
      "# app → QA (origin/qa)",
      "#   aaa1111  #111111 · WI 111111 aaa1111",
      "#   bbb2222  #111111 · WI 111111 bbb2222",
      "#   ccc3333  #111111 · WI 111111 ccc3333",
      `# ${es["promo.cmd.conflict"]}`,
      [
        'cd "C:/src/app"',
        "git fetch --all --prune",
        "git switch -c promote/cp-qa-111111-20261126-0930 origin/qa",
        "git cherry-pick -x aaa1111",
        "git cherry-pick -x -m 1 bbb2222",
        "git cherry-pick -x ccc3333",
        "git push -u origin promote/cp-qa-111111-20261126-0930",
      ].join(" && \\\n  "),
    ].join("\n"),
  );
});

test("merge commands use origin/<branch> and also carry a conflict comment", () => {
  const r = { ...repo(["local", "upstream", "origin"]), remote: "upstream" };
  const out = promoCommands(r, opts({ mode: "merge" }))!;
  assert.match(out, /git merge --no-ff origin\/Feature\/111111-login/);
  assert.ok(out.includes(`# ${es["promo.cmd.conflictMerge"]}`), out);
  assert.ok(!out.includes(es["promo.cmd.conflict"]), out);
  assert.match(out, /git push -u upstream promote\/cp-qa-111111-20261126-0930$/);
});

test("powershell stops at the first failure", () => {
  const out = promoCommands(repo(["origin"]), opts({ shell: "ps", mode: "merge" }))!;
  assert.ok(out.includes(String.raw`  Set-Location "C:\src\app"`), out);
  assert.ok(out.includes(`  git merge --no-ff origin/Feature/111111-login; if ($LASTEXITCODE -ne 0) { Write-Warning "${es["promo.cmd.failed"]}: git merge"; return }`), out);
});
