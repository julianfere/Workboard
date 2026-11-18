"""Arma `fixtures/repos/app.fi`: un repo git sintético con los casos que distingue el panel Git y la
promoción de la v1, exportado con `git fast-export` (SHAs deterministas: fechas y autores fijos).

Entornos (remote `origin`, sin fetch real: los "push" son `update-ref` de refs/remotes):
  DEV = origin/development · QA = origin/qa · PROD = origin/main

Casos:
  111111  rama con 2 commits mergeada a DEV con PR ("Merged PR 900001"), falta en QA; además la rama
          local tiene un commit sin pushear.
  222222  rama mergeada a DEV y pasada a QA con cherry-pick -x (equivalent / alreadyCp).
  333333  commit directo en DEV que menciona el WI, sin rama (commits mencionados DEV → QA).
  444444  rama con un merge desde development en el medio (dev merge).
  555555  rama apilada sobre la de 111111 (commits compartidos).
  666666  rama con 2 commits llevada a QA como squash (contenido igual).
  777777  rama con un commit que menciona solo otro WI (otherwi).
  999999  rama mergeada a DEV por fast-forward (sin commits propios).
  promote/cp-qa-111111-…  rama de una promoción anterior: se ignora.

Para recrearlo (lo mismo hace `gen_git.py` y deben hacer los tests):
  git init -q <dir> && git -C <dir> fast-import --quiet < app.fi
  git -C <dir> config remote.origin.url https://example.com/acme/app.git
  git -C <dir> config remote.origin.fetch +refs/heads/*:refs/remotes/origin/*
  git -C <dir> checkout -q Feature/111111-login       (rama actual)
  echo x > <dir>/pendiente.txt                         (un archivo sin trackear: dirty)
"""

import os
import shutil
import subprocess
import tempfile
from pathlib import Path

from common import FIXTURES

OUT = FIXTURES / "repos" / "app.fi"
REMOTE_URL = "https://example.com/acme/app.git"
BASE_TS = 1_790_000_000  # 2026-09-21 aprox.


class Repo:
    def __init__(self, path):
        self.path = path
        self.tick = 0
        self.env = dict(os.environ, GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull, HOME=str(path),
                        GIT_AUTHOR_NAME="Ana", GIT_AUTHOR_EMAIL="ana@example.com",
                        GIT_COMMITTER_NAME="Ana", GIT_COMMITTER_EMAIL="ana@example.com")

    def git(self, *args, author=None):
        env = dict(self.env)
        if author:
            env["GIT_AUTHOR_NAME"] = env["GIT_COMMITTER_NAME"] = author
            env["GIT_AUTHOR_EMAIL"] = env["GIT_COMMITTER_EMAIL"] = f"{author.lower()}@example.com"
        self.tick += 1
        stamp = f"{BASE_TS + self.tick * 3600} +0000"
        env["GIT_AUTHOR_DATE"] = env["GIT_COMMITTER_DATE"] = stamp
        res = subprocess.run(["git", "-C", str(self.path), *args], env=env, capture_output=True, text=True)
        if res.returncode != 0:
            raise RuntimeError(f"git {' '.join(args)}: {res.stderr}")
        return res.stdout.strip()

    def write(self, rel, text):
        p = self.path / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text, encoding="utf-8", newline="\n")

    def commit(self, msg, files, author=None):
        for rel, text in files.items():
            self.write(rel, text)
        self.git("add", "-A")
        self.git("commit", "-q", "-m", msg, author=author)
        return self.git("rev-parse", "HEAD")

    def push(self, branch, remote_branch=None):
        self.git("update-ref", f"refs/remotes/origin/{remote_branch or branch}", self.git("rev-parse", branch))


def build():
    tmp = Path(tempfile.mkdtemp(prefix="wb-repo-")).resolve()
    try:
        r = Repo(tmp)
        r.git("init", "-q", "-b", "main")
        r.git("config", "core.autocrlf", "false")
        r.git("config", "commit.gpgsign", "false")
        r.commit("initial", {"README.md": "# app\n", "src/app.txt": "v1\n", "src/config.txt": "a=1\n"})
        r.git("branch", "development")
        r.git("branch", "qa")
        for b in ("main", "development", "qa"):
            r.push(b)

        # 111111: dos commits en una rama, merge a DEV por PR.
        r.git("checkout", "-q", "-b", "Feature/111111-login", "development")
        r.commit("111111 login: formulario", {"src/login.txt": "form\n"})
        r.commit("111111 login: api", {"src/login_api.txt": "api\n"})
        r.push("Feature/111111-login")
        # 555555: apilada sobre 111111.
        r.git("checkout", "-q", "-b", "Bug/555555-envio", "Feature/111111-login")
        r.commit("555555 envio: costo", {"src/envio.txt": "costo\n"}, author="Beto")
        r.push("Bug/555555-envio")
        r.git("checkout", "-q", "development")
        r.git("merge", "-q", "--no-ff", "-m", "Merged PR 900001: 111111 login", "Feature/111111-login")

        # 222222: rama → DEV, y cherry-pick -x a QA.
        r.git("checkout", "-q", "-b", "Feature/222222-pagos", "development")
        p1 = r.commit("222222 pagos: no duplicar", {"src/pagos.txt": "unico\n"})
        r.push("Feature/222222-pagos")
        r.git("checkout", "-q", "development")
        r.git("merge", "-q", "--no-ff", "-m", "Merged PR 900002: 222222 pagos", "Feature/222222-pagos")
        r.git("checkout", "-q", "qa")
        r.git("cherry-pick", "-x", p1)

        # 333333: commit directo en DEV, sin rama.
        r.git("checkout", "-q", "development")
        r.commit("333333 ajuste de config", {"src/config.txt": "a=2\n"}, author="Caro")

        # 444444: rama con merge desde development en el medio.
        r.git("checkout", "-q", "-b", "Feature/444444-carrito", "development")
        r.commit("444444 carrito: modelo", {"src/carrito.txt": "modelo\n"})
        r.git("checkout", "-q", "development")
        r.commit("otro trabajo en development", {"src/otro.txt": "x\n"}, author="Caro")
        r.git("checkout", "-q", "Feature/444444-carrito")
        r.git("merge", "-q", "--no-ff", "-m", "Merge branch 'development' into Feature/444444-carrito", "development")
        r.commit("444444 carrito: vista", {"src/carrito_vista.txt": "vista\n"})
        r.push("Feature/444444-carrito")
        r.git("checkout", "-q", "development")
        r.git("merge", "-q", "--no-ff", "-m", "Merged PR 900004: 444444 carrito", "Feature/444444-carrito")

        # 666666: dos commits; a QA llega como squash (mismo contenido).
        r.git("checkout", "-q", "-b", "Feature/666666-reportes", "development")
        r.commit("666666 reportes: base", {"src/reportes.txt": "base\n"})
        r.commit("666666 reportes: filtros", {"src/reportes.txt": "base\nfiltros\n"})
        r.push("Feature/666666-reportes")
        r.git("checkout", "-q", "development")
        r.git("merge", "-q", "--no-ff", "-m", "Merged PR 900006: 666666 reportes", "Feature/666666-reportes")
        # Squash a mano: solo el cambio de la rama (un `merge --squash` traería todo development).
        r.git("checkout", "-q", "qa")
        r.commit("666666 reportes (squash)", {"src/reportes.txt": "base\nfiltros\n"})

        # 777777: un commit que menciona solo otro WI.
        r.git("checkout", "-q", "-b", "Feature/777777-busqueda", "development")
        r.commit("777777 busqueda: indice", {"src/busqueda.txt": "indice\n"})
        r.commit("888888 arreglo ajeno", {"src/ajeno.txt": "ajeno\n"}, author="Dani")
        r.push("Feature/777777-busqueda")

        # 999999: fast-forward a DEV (sin commits propios fuera de la línea principal).
        r.git("checkout", "-q", "-b", "Feature/999999-ff", "development")
        r.commit("999999 ff", {"src/ff.txt": "ff\n"})
        r.push("Feature/999999-ff")
        r.git("checkout", "-q", "development")
        r.git("merge", "-q", "--ff-only", "Feature/999999-ff")

        # Una promoción anterior (se ignora) y los push de los entornos.
        r.git("branch", "promote/cp-qa-111111-20260101-1200", "qa")
        r.push("promote/cp-qa-111111-20260101-1200")
        for b in ("development", "qa"):
            r.push(b)

        # 111111: commit local sin pushear.
        r.git("checkout", "-q", "Feature/111111-login")
        r.commit("111111 login: tests (sin pushear)", {"src/login_test.txt": "test\n"})

        OUT.parent.mkdir(parents=True, exist_ok=True)
        stream = subprocess.run(["git", "-C", str(tmp), "fast-export", "--all", "--signed-tags=strip", "--reencode=yes"],
                                capture_output=True, check=True).stdout
        OUT.write_bytes(stream)
        print(f"  {OUT} ({len(stream)} bytes)")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def restore(dest):
    """Recrea el repo del fixture en `dest` (carpeta vacía o inexistente). Devuelve la ruta."""
    dest = Path(dest)
    dest.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull)

    def g(*args, **kw):
        return subprocess.run(["git", "-C", str(dest), *args], env=env, check=True, capture_output=True, **kw)

    g("init", "-q")
    g("config", "core.autocrlf", "false")
    g("fast-import", "--quiet", input=OUT.read_bytes())
    g("config", "remote.origin.url", REMOTE_URL)
    g("config", "remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*")
    g("checkout", "-q", "Feature/111111-login")
    (dest / "pendiente.txt").write_text("x\n", encoding="utf-8")
    return dest


if __name__ == "__main__":
    build()
