"""Casos de `Workspace` sobre `fixtures/work`: escaneo y una secuencia de operaciones de escritura.

Cada operación corre sobre una copia temporal. Después de cada paso se guarda el resultado y el estado
del árbol (rutas + contenido de los README que cambiaron), sin tiempos ni rutas absolutas.
"""

import os
import tempfile
from pathlib import Path

from common import FIXTURES, ROOT_TOKEN, TempCopy, call, drop_keys, load_v1, path_variants, scrub, write_cases

load_v1()
import core  # noqa: E402

TIME_KEYS = {"created", "last", "mtime", "deleted", "ts"}
WORK = FIXTURES / "work"


def tree(root):
    out = []
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames.sort()
        for f in sorted(filenames):
            out.append((Path(dirpath) / f).relative_to(root).as_posix())
    return out


def readmes(root):
    return {p: (root / p).read_bytes().decode("utf-8", "replace") for p in tree(root) if p.endswith("README.md")}


def norm(value, root):
    return drop_keys(scrub(value, [(v, ROOT_TOKEN) for v in path_variants(root)]), TIME_KEYS)


def scan_cases():
    ws = core.Workspace(WORK)
    items = sorted(ws.list_items(), key=lambda it: it["name"])
    folder_of = [{"name": n, **call(lambda n=n: ws.folder_of(n).relative_to(ws.root).as_posix())} for n in
                 ["wi111111-login-con-sso", "grp-checkout", "grp-checkout/wi444444-carrito", "grp-checkout/_privado",
                  "_deleted/wi888888-eliminado", "_interno", ".config", "suelto.md", "no-existe", "../x", "",
                  "wi111111-login-con-sso/docs", "grp-checkout/wi444444-carrito/../wi555555-envio"]]
    safe = [{"rel": r, **call(lambda r=r: ws.safe_path(r).relative_to(ws.root).as_posix())} for r in
            ["", ".", "a/b", "../x", "a/../../x", "a/../b", "C:/Windows" if os.name == "nt" else "/etc"]]
    read_file = [{"rel": r, **call(ws.read_file, r)} for r in
                 ["wi111111-login-con-sso/consulta.sql", "wi111111-copia-vieja/README.md", "no/existe.txt", "../x", "wi111111-login-con-sso"]]
    media = [{"rel": r, **call(ws.read_media, r)} for r in ["wi111111-login-con-sso/captura.png", "wi111111-login-con-sso/consulta.sql"]]
    get_meta = [{"name": n, **call(ws.get_meta, n)} for n in ["wi222222-333333-pagos", "wi777777-sin-readme", "grp-checkout"]]
    # list_deleted ordena por fecha de borrado (mtime): en el golden va por ruta (ver README).
    deleted = sorted(ws.list_deleted(), key=lambda d: d["path"])
    return norm({"list_items": items, "list_deleted": deleted, "folder_of": folder_of, "safe_path": safe,
                 "read_file": read_file, "read_media": media, "get_meta": get_meta}, ws.root)


# (descripción, nombre del método, args)
OPS = [
    ("cambia estado y notas con caracteres que van entre comillas", "update_meta",
     ["wi111111-login-con-sso", {"estado": "codeado", "notas": "ok: listo", "ambientes": ["dev", "qa"]}]),
    ("borra un campo con \"\"", "update_meta", ["wi111111-login-con-sso", {"horas": ""}]),
    ("pasar a prod tilda los pendientes abiertos", "update_meta", ["wi111111-login-con-sso", {"estado": "prod"}]),
    ("de prod a descartado no vuelve a tocar el cuerpo", "update_meta", ["wi111111-login-con-sso", {"estado": "descartado"}]),
    ("clave inválida", "update_meta", ["wi111111-login-con-sso", {"Estado": "qa"}]),
    ("sin cambios no escribe", "update_meta", ["wi222222-333333-pagos", {"estado": "qa"}]),
    ("README con CRLF y BOM: reescribe en LF", "update_meta", ["wi222222-333333-pagos", {"notas": "n"}]),
    ("README cp1252: reescribe en UTF-8", "update_meta", ["wi111111-copia-vieja", {"tipo": "bug"}]),
    ("sin README: lo crea con # nombre", "add_task", ["wi777777-sin-readme", "primera tarea"]),
    ("agrega a una sección existente", "add_task", ["grp-checkout", "  otra   tarea "]),
    ("tarea vacía", "add_task", ["grp-checkout", "   "]),
    ("tilda con expect correcto", "set_task", ["grp-checkout", 1, True, "otra tarea"]),
    ("tilda con expect que no coincide", "set_task", ["grp-checkout", 0, True, "no es esta"]),
    ("índice fuera de rango", "set_task", ["grp-checkout", 9, True]),
    ("edita conservando hecha", "edit_task", ["grp-checkout", 1, "otra tarea editada"]),
    ("borra", "remove_task", ["grp-checkout", 0, "Coordinar deploy"]),
    ("WI de un grupo", "update_meta", ["grp-checkout/wi444444-carrito", {"estado": "codeado"}]),
    ("mueve un WI a un grupo", "move_folder", ["wi666666-plan-pendiente", "grp-checkout"]),
    ("lo saca a la raíz", "move_folder", ["grp-checkout/wi666666-plan-pendiente", ""]),
    ("un grupo no entra en otro", "move_folder", ["grp-checkout", "grp-checkout"]),
    ("destino que no es grupo", "move_folder", ["wi666666-plan-pendiente", "wi111111-login-con-sso"]),
    ("mover a donde ya está", "move_folder", ["wi666666-plan-pendiente", ""]),
    ("eliminar a _deleted", "trash", ["wi222222-333333-pagos"]),
    ("restaurar", "untrash", ["_deleted/wi222222-333333-pagos", "wi222222-333333-pagos"]),
    ("restaurar algo que no está en _deleted", "untrash", ["wi111111-login-con-sso", "x"]),
    ("restaurar sobre una carpeta que existe", "untrash", ["_deleted/wi888888-eliminado", "wi111111-login-con-sso"]),
    ("restaurar dentro de un grupo", "untrash", ["_deleted/wi888888-eliminado", "grp-checkout/wi888888-eliminado"]),
    ("renombrar WI", "rename_folder", ["wi666666-plan-pendiente", "wi666666-plan-listo"]),
    ("un WI no puede empezar con grp-", "rename_folder", ["wi666666-plan-listo", "grp-x"]),
    ("un grupo tiene que seguir siendo grupo", "rename_folder", ["grp-checkout", "checkout"]),
    ("nombre inválido", "rename_folder", ["wi666666-plan-listo", "con espacio"]),
    ("nombre que empieza con _", "rename_folder", ["wi666666-plan-listo", "_oculto"]),
    ("renombrar a uno que existe", "rename_folder", ["wi666666-plan-listo", "arreglo-sin-wi"]),
    ("solo cambia mayúsculas", "rename_folder", ["arreglo-sin-wi", "Arreglo-Sin-WI"]),
    ("renombrar grupo", "rename_folder", ["grp-checkout", "grp-checkout-v2"]),
    ("crear carpeta", "create_folder", ["wi999999-nuevo"]),
    ("crear carpeta que existe", "create_folder", ["wi999999-nuevo"]),
    ("crear con ruta", "create_folder", ["a/b"]),
    ("eliminar dos veces el mismo nombre", "trash", ["wi999999-nuevo"]),
]


def op_cases():
    steps = []
    with TempCopy(WORK) as root, tempfile.TemporaryDirectory() as cfg:
        ws = core.Workspace(root, Path(cfg) / "backups")
        before = readmes(root)
        for desc, method, args in OPS:
            res = call(getattr(ws, method), *args)
            if "ok" in res and isinstance(res["ok"], Path):
                res["ok"] = res["ok"].relative_to(root).as_posix()
            after = readmes(root)
            changed = {p: t for p, t in after.items() if before.get(p) != t}
            steps.append({"desc": desc, "op": method, "args": args, **res, "tree": tree(root), "readmes_changed": changed})
            before = after
        # Segunda eliminación de un nombre que ya está en _deleted: queda con sufijo de fecha.
        ws.create_folder("wi999999-nuevo")
        res = call(ws.trash, "wi999999-nuevo")
        steps.append({"desc": "eliminar un nombre que ya está en _deleted agrega -AAAAMMDD-HHMMSS", "op": "trash",
                      "args": ["wi999999-nuevo"], "ok_pattern": r"^_deleted/wi999999-nuevo-\d{8}-\d{6}$",
                      "matches": bool(__import__("re").fullmatch(r"_deleted/wi999999-nuevo-\d{8}-\d{6}", res.get("ok", "")))})
        # Versiones: cada escritura dejó una copia (últimas 20) y restaurar vuelve al texto anterior.
        name = "wi111111-login-con-sso"
        versions = ws.list_versions(name)
        oldest = versions[-1]["id"]
        original = ws.read_version(name, oldest)
        ws.restore_version(name, oldest)
        restored = (root / name / "README.md").read_bytes().decode("utf-8")
        versions_info = {"count_before_restore": len(versions), "oldest_equals_fixture": original == (WORK / name / "README.md").read_text(encoding="utf-8"),
                         "restored_equals_oldest": restored == original, "count_after_restore": len(ws.list_versions(name)),
                         "invalid_id": call(ws.read_version, name, "../x"), "missing_id": call(ws.read_version, name, "20000101-000000-000000000")}
        state_log = (Path(cfg) / "state_log.jsonl").read_text(encoding="utf-8").splitlines()
        import json
        log = [drop_keys(json.loads(l), {"ts", "root"}) for l in state_log]
        return norm({"steps": steps, "versions": versions_info, "state_log": log}, root)


def backup_rotation_case():
    """21 escrituras = 21 copias previas: quedan BACKUP_KEEP y se pierde la del README original."""
    with TempCopy(WORK) as root, tempfile.TemporaryDirectory() as cfg:
        ws = core.Workspace(root, Path(cfg) / "backups")
        for i in range(core.BACKUP_KEEP + 1):
            ws.update_meta("grp-checkout", {"notas": f"v{i}"})
        vs = ws.list_versions("grp-checkout")
        oldest = ws.read_version("grp-checkout", vs[-1]["id"])
        return {"writes": core.BACKUP_KEEP + 1, "kept": len(vs), "oldest_notas": core.split_frontmatter(oldest)[0].get("notas")}


def main():
    write_cases("workspace_scan", scan_cases())
    data = op_cases()
    data["backup_rotation"] = backup_rotation_case()
    write_cases("workspace_ops", data)


if __name__ == "__main__":
    main()
