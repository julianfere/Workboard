"""Helpers compartidos de los generadores de golden files.

Los golden se generan con la v1 en Python (la referencia de comportamiento) y los consume el port en
Rust. La v1 se importa desde `WORKBOARD_V1` (por defecto `_v1/` en la raíz del repo, que no se publica).
"""

import json
import os
import shutil
import sys
import tempfile
from pathlib import Path

GOLDEN = Path(__file__).resolve().parent.parent
REPO = GOLDEN.parent.parent
FIXTURES = GOLDEN / "fixtures"
CASES = GOLDEN / "cases"
V1 = Path(os.environ.get("WORKBOARD_V1") or REPO / "_v1").resolve()

ROOT_TOKEN = "<ROOT>"
CONFIG_TOKEN = "<CONFIG>"


def load_v1():
    if not (V1 / "core.py").is_file():
        sys.exit(f"No encuentro la v1 en {V1} (definí WORKBOARD_V1)")
    if str(V1) not in sys.path:
        sys.path.insert(0, str(V1))


def write_cases(name, data):
    CASES.mkdir(parents=True, exist_ok=True)
    path = CASES / f"{name}.json"
    text = json.dumps(data, ensure_ascii=False, indent=1) + "\n"
    path.write_text(text, encoding="utf-8", newline="\n")
    n = len(data) if isinstance(data, list) else len(data.get("cases", data))
    print(f"  {path.relative_to(REPO).as_posix()}: {n} casos")


def call(fn, *args, **kwargs):
    """Resultado o error de una llamada, en una forma comparable desde Rust."""
    try:
        return {"ok": fn(*args, **kwargs)}
    except Exception as exc:  # noqa: BLE001 — el tipo de error es parte del contrato
        return {"error": type(exc).__name__, "message": str(exc)}


def scrub(value, replacements):
    """Reemplaza rutas absolutas por tokens en cualquier estructura JSON."""
    if isinstance(value, str):
        for old, new in replacements:
            value = value.replace(old, new)
        return value
    if isinstance(value, list):
        return [scrub(v, replacements) for v in value]
    if isinstance(value, tuple):
        return [scrub(v, replacements) for v in value]
    if isinstance(value, dict):
        return {scrub(k, replacements): scrub(v, replacements) for k, v in value.items()}
    return value


def path_variants(p):
    """Formas en que una ruta puede aparecer en la salida (Windows: \\ y /, y en minúsculas por normcase)."""
    s = str(Path(p).resolve())
    out = set()
    for v in (s, os.path.normcase(s)):
        out |= {v, v.replace("\\", "/"), v.replace("\\", "\\\\")}
    return sorted(out, key=len, reverse=True)


def drop_keys(value, keys):
    """Saca campos que dependen del reloj o del filesystem (mtime, ctime…)."""
    if isinstance(value, list):
        return [drop_keys(v, keys) for v in value]
    if isinstance(value, dict):
        return {k: drop_keys(v, keys) for k, v in value.items() if k not in keys}
    return value


class TempCopy:
    """Copia temporal de un fixture: las operaciones de escritura nunca tocan los golden."""

    def __init__(self, src):
        self.src = Path(src)

    def __enter__(self):
        # resolve(): %TEMP% puede venir con nombres cortos de Windows (USUARI~1) y la v1 resuelve la raíz.
        self.tmp = Path(tempfile.mkdtemp(prefix="wb-golden-")).resolve()
        self.path = self.tmp / self.src.name
        shutil.copytree(self.src, self.path)
        return self.path

    def __exit__(self, *exc):
        shutil.rmtree(self.tmp, ignore_errors=True)
