"""Casos del panel Git y la promoción sobre el repo de `fixtures/repos/app.fi`.

Captura la salida de `GitPanel.overview`, `promotion` (QA y PROD), `branch_diff`, `detect_repos` y
helpers sin estado. Los comandos de promoción los arma la UI a partir de esta salida.
"""

import os
import shutil
import subprocess
import tempfile
from pathlib import Path

from build_repo import restore
from common import ROOT_TOKEN, call, drop_keys, load_v1, path_variants, scrub, write_cases

load_v1()
import gitpanel  # noqa: E402

ENVS = {"DEV": "origin/development", "QA": "origin/qa", "PROD": "origin/main"}
WIS = ["111111", "222222", "333333", "444444", "555555", "666666", "777777", "999999"]
VOLATILE = {"computedAt", "seconds", "lastFetch"}


def norm(value, root):
    return drop_keys(scrub(value, [(v, ROOT_TOKEN) for v in path_variants(root)]), VOLATILE)


def helper_cases():
    texts = ["111111 login", "Merged PR 900001: 111111 login", "PR #123456 y 654321", "pull request 222222",
             "1234567 no es", "12345 tampoco", "a111111b", "wi-111111/222222", "", None,
             "fix (cherry picked from commit 7238ab4) 333333"]
    nums = [{"text": t, "wis": sorted(gitpanel.wi_nums(t))} for t in texts]
    trailers = [{"text": t, "shas": gitpanel.CP_TRAILER.findall(t)} for t in
                ["x\n\n(cherry picked from commit 7238ab4c0ffee0000000000000000000000000000)",
                 "(cherry picked from commit abc12)", "(cherry picked from commit ABCDEF1)", "(cherry picked from commit abcdef1)"]]
    urls = ["https://dev.azure.com/org/proj/_git/app", "https://user@dev.azure.com/org/proj/_git/app.git",
            "https://org.visualstudio.com/proj/_git/app", "git@ssh.dev.azure.com:v3/org/proj/app",
            "https://github.com/acme/app.git", "http://server/_git/app"]
    web = []
    with tempfile.TemporaryDirectory() as d:
        repo = Path(d)
        subprocess.run(["git", "init", "-q", str(repo)], check=True)
        for i, u in enumerate(urls):
            subprocess.run(["git", "-C", str(repo), "remote", "add", f"r{i}", u], check=True)
            web.append({"url": u, "web": gitpanel.remote_web_url(repo, f"r{i}")})
        web.append({"url": None, "remote": "noexiste", "web": gitpanel.remote_web_url(repo, "noexiste")})
    return {"wi_nums": nums, "cp_trailer": trailers, "remote_web_url": web}


def repo_cases():
    tmp = Path(tempfile.mkdtemp(prefix="wb-git-")).resolve()
    try:
        project = tmp / "proyecto"
        (project / "_work").mkdir(parents=True)
        repo = restore(project / "app")
        # Un repo sin ramas de entorno (se descarta) y una carpeta que no es repo.
        subprocess.run(["git", "init", "-q", str(project / "vault")], check=True)
        (project / "docs").mkdir()
        repos = [{"name": "app", "path": str(repo), "envs": ENVS}]
        panel = gitpanel.GitPanel()
        out = {
            "detect_repos": gitpanel.detect_repos(project / "_work"),
            "default_envs": gitpanel.default_envs(repo),
            "overview": panel.overview(repos, WIS),
            "overview_one": panel.overview(repos, ["555555"]),
            "overview_missing_env": gitpanel.GitPanel().overview(
                [{"name": "app", "path": str(repo), "envs": dict(ENVS, QA="origin/no-existe")}], ["111111"]),
            "overview_not_repo": gitpanel.GitPanel().overview([{"name": "docs", "path": str(project / "docs"), "envs": ENVS}], WIS),
            "promotion_qa": gitpanel.promotion(panel, repos, WIS, "QA"),
            "promotion_prod": gitpanel.promotion(panel, repos, WIS, "PROD"),
            "promotion_qa_one": gitpanel.promotion(panel, repos, ["444444"], "QA"),
            "promotion_qa_stacked": gitpanel.promotion(panel, repos, ["555555"], "QA"),
            "promotion_qa_mention_only": gitpanel.promotion(panel, repos, ["333333"], "QA"),
            "promotion_missing_target": gitpanel.promotion(
                panel, [{"name": "app", "path": str(repo), "envs": dict(ENVS, QA="origin/no-existe")}], ["111111"], "QA"),
        }
        tips = {b["name"]: b["tip"]["sha"] for b in out["overview"]["byWi"].get("111111", [])}
        out["branch_diff"] = call(panel.branch_diff, repos[0], tips.get("Feature/111111-login"))
        ff_tip = next((b["tip"]["sha"] for b in out["overview"]["byWi"].get("999999", [])), None)
        out["branch_diff_ff"] = call(panel.branch_diff, repos[0], ff_tip)
        # Mismo cálculo con el caché en disco caliente: tiene que dar lo mismo.
        cache = tmp / "git_cache.json"
        gitpanel.GitPanel(cache).overview(repos, WIS)
        warm = gitpanel.GitPanel(cache).overview(repos, WIS)
        out["overview_warm_equals_cold"] = norm(warm, tmp) == norm(out["overview"], tmp)
        return norm(out, tmp)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def main():
    data = repo_cases()
    data["helpers"] = helper_cases()
    write_cases("git", data)


if __name__ == "__main__":
    main()
