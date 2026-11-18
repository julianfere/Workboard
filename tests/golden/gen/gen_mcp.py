"""Casos del MCP `workboard` de la v1: cada herramienta llamada directo (sin JSON-RPC) en tres modos.

`mcp_server` lee WORKBOARD_SESSION / WORKBOARD_MCP_WRITE / la carpeta de config al importarse, así que
cada escenario corre en un subproceso con su entorno, sobre una copia de `fixtures/work`.
"""

import json
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from common import CONFIG_TOKEN, FIXTURES, ROOT_TOKEN, TempCopy, drop_keys, path_variants, scrub, write_cases

SESSION_ID = "11111111-2222-3333-4444-555555555555"
TODAY_TOKEN = "<TODAY>"

READ_CALLS = [
    ["workboard_session", {}],
    ["workboard_list_wis", {}],
    ["workboard_list_wis", {"incluir_cerrados": True}],
    ["workboard_list_wis", {"estado": "descartado"}],
    ["workboard_list_wis", {"buscar": "CHECKOUT"}],
    ["workboard_list_wis", {"buscar": "444444"}],
    ["workboard_get_wi", {"wi": "wi111111-login-con-sso"}],
    ["workboard_get_wi", {"wi": "111111"}],
    ["workboard_get_wi", {"wi": "#222222"}],
    ["workboard_get_wi", {"wi": "333333"}],
    ["workboard_get_wi", {"wi": "grp-checkout"}],
    ["workboard_get_wi", {"wi": "444444"}],
    ["workboard_get_wi", {"wi": "000000"}],
    ["workboard_set_estado", {"wi": "222222", "estado": "prod"}],
    ["workboard_list_tools", {}],
    ["workboard_run_tool", {"nombre": "x"}],
]

WRITE_CALLS = [
    ["workboard_session", {}],
    ["workboard_set_estado", {"wi": "222222", "estado": "prod", "notas": "pasó a prod"}],
    ["workboard_set_estado", {"wi": "222222"}],
    ["workboard_set_estado", {"wi": "222222", "estado": "inventado"}],
    ["workboard_add_task", {"wi": "wi777777-sin-readme", "texto": "  crear   README "}],
]

SESSION_CALLS = [
    ["workboard_session", {}],
    ["workboard_set_estado", {"wi": "wi111111-login-con-sso", "estado": "codeado", "ambientes": ["qa", "dev"], "horas": "8,5"}],
    ["workboard_set_estado", {"wi": "222222", "estado": "qa"}],
    ["workboard_set_estado", {"wi": "444444", "estado": "codeado"}],
    ["workboard_set_estado", {"wi": "grp-checkout", "pactada": "2026-12-01"}],
    ["workboard_add_task", {"wi": "wi111111-login-con-sso", "texto": "Documentar"}],
    ["workboard_add_task", {"wi": "wi111111-login-con-sso", "texto": " "}],
    ["workboard_complete_task", {"wi": "wi111111-login-con-sso", "tarea": "0"}],
    ["workboard_complete_task", {"wi": "wi111111-login-con-sso", "tarea": "Documentar"}],
    ["workboard_complete_task", {"wi": "wi111111-login-con-sso", "tarea": "tabla", "hecha": False}],
    ["workboard_complete_task", {"wi": "wi111111-login-con-sso", "tarea": "r"}],
    ["workboard_complete_task", {"wi": "wi111111-login-con-sso", "tarea": "no existe"}],
    ["workboard_complete_task", {"wi": "wi111111-login-con-sso", "tarea": "99"}],
    ["workboard_marcar_ejecutado", {"wi": "111111-login", "archivo": "MIGRATION_WI111111_sesiones.sql", "ambiente": "QA"}],
    ["workboard_marcar_ejecutado", {"wi": "wi111111-login-con-sso", "archivo": "docs\\PLAN_sso.md", "ambiente": "dev"}],
    ["workboard_marcar_ejecutado", {"wi": "wi111111-login-con-sso", "archivo": "migration_wi111111_sesiones.sql", "ambiente": "prod"}],
    ["workboard_marcar_ejecutado", {"wi": "wi111111-login-con-sso", "archivo": "MIGRATION_WI111111_sesiones.sql", "ambiente": "qa", "ejecutado": False}],
    ["workboard_marcar_ejecutado", {"wi": "wi111111-login-con-sso", "archivo": "x.sql", "ambiente": "uat"}],
    ["workboard_marcar_ejecutado", {"wi": "wi111111-login-con-sso", "archivo": ".sql", "ambiente": "dev"}],
    ["workboard_marcar_ejecutado", {"wi": "wi111111-login-con-sso", "archivo": "consulta.sql", "ambiente": "uat"}],
    ["workboard_get_wi", {"wi": "wi111111-login-con-sso"}],
    ["workboard_add_note", {"wi": "grp-checkout/wi444444-carrito", "titulo": "Hallazgo: el carrito", "texto": "Detalle\n\ncon líneas"}],
    ["workboard_add_note", {"wi": "grp-checkout/wi444444-carrito", "titulo": "Hallazgo: el carrito", "texto": ""}],
    ["workboard_add_note", {"wi": "444444", "titulo": "ambiguo", "texto": "x"}],
    ["workboard_list_tools", {}],
    ["workboard_run_tool", {"nombre": "no-existe", "wi": "wi111111-login-con-sso"}],
    # Bug de la v1: sin `wi`, `_session_wis()` devuelve un set y `wis[0]` falla (ver KNOWN_ISSUES.md).
    ["workboard_run_tool", {"nombre": "no-existe"}],
]

ADO_CACHE_ITEMS = {
    "111111": {"state": "Active", "priority": 2, "pactada": "2026-12-15T00:00:00Z", "estimate": 20},
    "222222": {"state": "Resolved", "priority": 1, "pactada": "2026-11-30", "estimate": 4},
    "333333": {"state": "Resolved", "priority": 3, "estimate": 6},
    "444444": {"state": "New", "priority": "", "estimate": 0},
}


def child(spec_path):
    spec = json.loads(Path(spec_path).read_text(encoding="utf-8"))
    sys.path.insert(0, spec["v1"])
    import mcp_server
    from mcp.server.mcpserver.exceptions import ToolError
    out = {"instructions": mcp_server._instructions(), "calls": []}
    for name, kwargs in spec["calls"]:
        try:
            res = {"ok": getattr(mcp_server, name)(**kwargs)}
        except ToolError as exc:
            res = {"error": "ToolError", "message": str(exc)}
        except Exception as exc:  # noqa: BLE001
            res = {"error": type(exc).__name__, "message": str(exc)}
        out["calls"].append({"tool": name, "args": kwargs, **res})
    sys.stdout.buffer.write(json.dumps(out, ensure_ascii=False, default=str).encode("utf-8"))


def scenario(mode, calls):
    from common import V1
    with TempCopy(FIXTURES / "work") as root, tempfile.TemporaryDirectory() as cfg_raw:
        cfg = Path(cfg_raw).resolve()
        (cfg / "ado_cache.json").write_text(json.dumps({os.path.normcase(str(root)): {"items": ADO_CACHE_ITEMS}}), encoding="utf-8")
        env = dict(os.environ, WORKBOARD_ROOT=str(root), WORKBOARD_CONFIG_DIR=str(cfg), PYTHONIOENCODING="utf-8")
        env.pop("WORKBOARD_SESSION", None)
        env.pop("WORKBOARD_MCP_WRITE", None)
        if mode == "write":
            env["WORKBOARD_MCP_WRITE"] = "1"
        if mode == "session":
            env["WORKBOARD_SESSION"] = SESSION_ID
            (cfg / "sessions").mkdir()
            reg = {"sessions": [{"id": SESSION_ID, "nombre": "Sesión de prueba", "raiz": str(root), "agente": "claude",
                                 "wis": ["wi111111-login-con-sso", "grp-checkout"], "estado": "activa"}]}
            (cfg / "sessions" / "work-test.json").write_text(json.dumps(reg, ensure_ascii=False), encoding="utf-8")
        spec = cfg / "spec.json"
        spec.write_text(json.dumps({"v1": str(V1), "calls": calls}, ensure_ascii=False), encoding="utf-8")
        res = subprocess.run([sys.executable, __file__, "--child", str(spec)], env=env, capture_output=True, cwd=str(Path(__file__).parent))
        if res.returncode != 0:
            raise RuntimeError(res.stderr.decode("utf-8", "replace"))
        out = json.loads(res.stdout.decode("utf-8"))
        # El orden real depende de los mtime (list_wis: actividad reciente primero; archivos: más nuevo
        # primero) y en una copia recién hecha empatan: el golden ordena por nombre (ver README).
        for c in out["calls"]:
            ok = c.get("ok")
            if not isinstance(ok, dict):
                continue
            if isinstance(ok.get("wis"), list) and ok["wis"] and isinstance(ok["wis"][0], dict):
                ok["wis"].sort(key=lambda w: w["carpeta"])
            if isinstance(ok.get("archivos"), list):
                ok["archivos"].sort(key=lambda f: f["ruta"])
        # Efectos: README que cambiaron, runs.json, eventos y notas.
        changed = {}
        for p in sorted(root.rglob("README.md")):
            rel = p.relative_to(root).as_posix()
            src = FIXTURES / "work" / rel
            text = p.read_bytes()
            if not src.exists() or src.read_bytes() != text:
                changed[rel] = text.decode("utf-8", "replace")
        events_file = cfg / "sessions" / f"{SESSION_ID}.events.jsonl" if mode == "session" else cfg / "mcp_events.jsonl"
        events = [drop_keys(json.loads(l), {"ts"}) for l in events_file.read_text(encoding="utf-8").splitlines()] if events_file.exists() else []
        runs = json.loads((cfg / "runs.json").read_text(encoding="utf-8")) if (cfg / "runs.json").exists() else {}
        runs = {k: {f: sorted(envs) for f, envs in v.items()} for k, v in runs.items()}  # sin timestamps
        notes_dir = cfg / "notas"
        notes = {p.name: p.read_text(encoding="utf-8") for p in sorted(notes_dir.glob("*.md"))} if notes_dir.exists() else {}
        out.update({"readmes_changed": changed, "events": events, "runs": runs, "notes": notes})
        today = time.strftime("%Y-%m-%d")
        reps = [(v, ROOT_TOKEN) for v in path_variants(root)] + [(v, CONFIG_TOKEN) for v in path_variants(cfg)]
        reps += [(os.path.normcase(str(root)), ROOT_TOKEN), (today, TODAY_TOKEN)]
        out = scrub(out, reps)
        # Hora de las marcas de ejecutado ("<TODAY> 16:37"): depende del reloj.
        import re
        return json.loads(re.sub(rf"{TODAY_TOKEN} \d\d:\d\d", f"{TODAY_TOKEN} <HH:MM>", json.dumps(out, ensure_ascii=False)))


def main():
    data = {"session_id": SESSION_ID, "ado_cache_items": ADO_CACHE_ITEMS,
            "read_only": scenario("read", READ_CALLS),
            "write_all": scenario("write", WRITE_CALLS),
            "session": scenario("session", SESSION_CALLS)}
    write_cases("mcp", data)


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] == "--child":
        child(sys.argv[2])
    else:
        main()
