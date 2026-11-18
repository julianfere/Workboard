"""Casos de funciones puras de `core.py` (sin filesystem)."""

import datetime

from common import call, load_v1, write_cases

load_v1()
import core  # noqa: E402

FRONTMATTER_TEXTS = [
    "",
    "# Sin frontmatter\n",
    "---\nestado: qa\n---\n# T\n",
    "---\nestado: qa\n---\n\n\n# Líneas vacías antes del cuerpo\n",
    "---\r\nestado: qa\r\n---\r\n\r\n# CRLF\r\n",
    "---\nsin cierre\n# T\n",
    "---\n---\n# Bloque vacío\n",
    "  ---  \nestado:  en-curso  \n  ---  \ncuerpo",
    "---\nambientes: [dev, qa , prod]\nvacia: []\nuno: [ dev ]\ncomas: [a,,b, ]\n---\n",
    '---\nnotas: "con: dos puntos"\ncita: \'simple\'\nmal: "abre\nraro: "\n---\n',
    "---\nurl: http://x.y:8080/z\nk:\n: sin clave\nsin dos puntos\n# comentario: x\n  # otro: y\n---\n",
    "---\nk: v1\nk: v2\nMayus: x\nwi: 123456\n---\n",
    "---\nestado: qa\n---",
    "﻿---\nestado: qa\n---\n",
    "---\nk: v\n---\n---\nsegundo: bloque\n---\n",
]

JOIN_CASES = [
    ({}, "# Solo cuerpo\n"),
    ({"estado": "qa"}, "# T\n"),
    ({"estado": "", "tipo": None, "ambientes": [], "notas": "x"}, "cuerpo"),
    ({"notas": "con: dos puntos", "otra": "con #", "c": "con [corchete]", "q": 'con "comillas"', "s": "con 'simple'"}, ""),
    ({"notas": "multi\nlínea\r\nacá  "}, ""),
    ({"ambientes": ["dev", "qa"], "comas": ["a,b", "c"]}, ""),
    ({"horas": 12.5, "n": 3, "b": True}, ""),
    ({"wi": ["123456", "654321"]}, "\n# Cuerpo con línea vacía\n"),
    ({"k": "  espacios  "}, ""),
    ({"vacio_tras_strip": "   "}, ""),
]

TASK_BODIES = {
    "sin_seccion": "# T\n\nTexto.\n\n\n",
    "vacia": "# T\n\n## Pendientes\n\n## Otra\n",
    "completa": "# T\n\n## Pendientes\n\n- [ ] uno\n  - [ ] sub\n- [x] dos\ntexto\n- [X] tres\n-  [ ] mal formada\n- [ ]cuatro\n\n### Sub heading\n- [ ] cinco (bajo ###: sigue en la sección)\n\n## Siguiente\n- [ ] fuera\n",
    "mayusculas": "## PENDIENTES   \n- [ ] a\n# Fin\n- [ ] fuera\n",
    "sin_newline_final": "## Pendientes\n- [ ] a",
    "crlf": "# T\r\n\r\n## Pendientes\r\n- [ ] a\r\n- [x] b\r\n",
    "dos_secciones": "## Pendientes\n- [ ] a\n## Pendientes\n- [ ] b\n",
    "heading_pegado": "##Pendientes\n- [ ] a\n",
}


def frontmatter_cases():
    split = [{"text": t, **call(lambda t=t: list(core.split_frontmatter(t)))} for t in FRONTMATTER_TEXTS]
    join = [{"meta": m, "body": b, **call(core.join_frontmatter, dict(m), b)} for m, b in JOIN_CASES]
    roundtrip = []
    for t in FRONTMATTER_TEXTS:
        meta, body = core.split_frontmatter(t)
        roundtrip.append({"text": t, "result": core.join_frontmatter(meta, body)})
    return {"split": split, "join": join, "roundtrip": roundtrip, "read_text": read_text_cases()}


def task_cases():
    cases = []
    for name, body in TASK_BODIES.items():
        cases.append({"body": name, "op": "parse", **call(core.parse_tasks, body)})
        cases.append({"body": name, "op": "add", "args": ["nueva"], **call(core.add_task, body, "nueva")})
        cases.append({"body": name, "op": "close", **call(core.close_tasks, body)})
        cases.append({"body": name, "op": "clear_done", **call(lambda b=body: list(core.clear_done(b)))})
        n = len(core.parse_tasks(body))
        for i in range(-1, n + 1):
            cases.append({"body": name, "op": "set", "args": [i, True], **call(core.set_task, body, i, True)})
            cases.append({"body": name, "op": "set", "args": [i, False], **call(core.set_task, body, i, False)})
            cases.append({"body": name, "op": "remove", "args": [i], **call(core.remove_task, body, i)})
            cases.append({"body": name, "op": "edit", "args": [i, "editada"], **call(core.edit_task, body, i, "editada")})
        if n:
            first = core.parse_tasks(body)[0]["text"]
            for expect in (first, " " + first + " ", "otra"):
                cases.append({"body": name, "op": "set", "args": [0, True], "expect": expect, **call(core.set_task, body, 0, True, expect)})
                cases.append({"body": name, "op": "remove", "args": [0], "expect": expect, **call(core.remove_task, body, 0, expect)})
                cases.append({"body": name, "op": "edit", "args": [0, "e"], "expect": expect, **call(core.edit_task, body, 0, "e", expect)})
    clean = [{"text": t, **call(core.clean_task, t)} for t in
             ["  hola   mundo ", "", "   ", None, "a" * 300, "a" * 301, "tab\tnl\nx", 123]]
    return {"bodies": TASK_BODIES, "cases": cases, "clean": clean}


def validate_cases():
    inputs = [
        {"estado": "qa"}, {"estado": "QA"}, {"estado": "inventado"}, {"estado": ""}, {"estado": None},
        {"tipo": "bug"}, {"tipo": "epic"},
        {"ambientes": ["prod", "dev"]}, {"ambientes": ["dev", "dev"]}, {"ambientes": "dev"}, {"ambientes": ["uat"]}, {"ambientes": []},
        {"wi": "123456"}, {"wi": "#123456"}, {"wi": "123456, 654321"}, {"wi": "12 34567"}, {"wi": ["123456", "#7654321"]},
        {"wi": "123"}, {"wi": "12345678"}, {"wi": "abc"}, {"wi": " , "},
        {"pactada": "2026-11-25"}, {"pactada": "2026-11-25T10:00:00"}, {"pactada": " 2026-02-30"}, {"pactada": "1989-01-01"},
        {"pactada": "2101-01-01"}, {"pactada": "25/11/2026"},
        {"periodo": "2026-11"}, {"periodo": "2026-13"}, {"periodo": "2026-00"}, {"periodo": "2026-1"}, {"periodo": " 2026-01 "},
        {"horas": "12"}, {"horas": "12,5"}, {"horas": "12.50"}, {"horas": 8}, {"horas": 0.1}, {"horas": "-1"}, {"horas": "10001"},
        {"horas": "10000"}, {"horas": "x"}, {"horas": "1e3"}, {"horas": "nan"}, {"horas": "inf"},
        {"notas": "una línea"}, {"notas": "dos\nlíneas"}, {"notas": "  recorta  "}, {"notas": "x" * 300}, {"notas": "x" * 301},
        {"rama": "feature/x"}, {"rama": "con espacio"}, {"release_manual": "123"}, {"release_manual": "a b"}, {"titulo": "T: con dos puntos"},
        {"otro": "x"}, {"estado": "qa", "otro": "x"},
        {"estado": "prod", "tipo": "us", "ambientes": ["qa"], "notas": "n", "horas": "3"},
    ]
    return [{"fields": f, **call(core.validate_fields, dict(f))} for f in inputs]


def misc_cases():
    names = ["MIGRATION_x.sql", "migration_x.sql", "Wi1_MIGRATION.sql", "XMIGRATIONX.sql", "MIGRATIONS.sql", "ROLLBACK_x.sql",
             "SEED.sql", "FIX_a.sql", "PREFIX_a.sql", "DIAG_a.sql", "VERIF_a.sql", "PLAN_a.md", "HANDOFF.md", "QA_notas.txt",
             "gen_DIAG.py", "x.PS1", "a.sql", "a.md", "a.drawio", "a.pdf", "a.png", "a.bin", "sin_extension", "FIX-DIAG.sql",
             "DIAG_FIX.sql", "README.md", "a.linq", "a.mermaid", "a.SQL"]
    files = [{"name": n, "kind": core.file_kind(n), "cat": core.file_category(n, core.file_kind(n))} for n in names]
    dates = ["2026-01-01", "2026-01-19", "2026-01-20", "2026-12-19", "2026-12-20", "2026-12-31", "2024-02-29", "2026-03-20"]
    periods = [{"date": d, "period": core.period_of(datetime.date.fromisoformat(d))} for d in dates]
    horas = [{"value": v, "result": core.parse_horas(v)} for v in
             ["8", "8,5", " 8.25 ", 7, 0, "-0.5", "10000", "10000.1", "", None, "x", "1e2", "inf", "nan"]]
    slugs = [{"text": t, "slug": core.slugify(t)} for t in
             ["Login con SSO", "Ñandú: acción rápida!!", "   ", "a" * 60, "x-" * 30, "ÀÉÎÕÜ ç", "日本語", "--a--b--"]]
    slugs.append({"text": "abc def ghi", "max_len": 5, "slug": core.slugify("abc def ghi", 5)})
    folders = ["wi123456-algo", "WI123456-algo", "123456-algo", "wi123456", "wi123456-654321-dos", "wi123456-65432-x",
               "wi1234567-x", "wi12345-x", "wi123456algo", "algo-123456", "wi123456-654321", "wi123456--x"]
    wi_re = []
    for f in folders:
        m = core.WI_RE.match(f)
        wi_re.append({"folder": f, "wis": [w for w in (m.group(1), m.group(2)) if w] if m else []})
    ado = [{"type": t, "tipo": core.ado_tipo(t)} for t in ["Bug", "User Story", " product backlog item ", "Feature", "Task", "Epic", "", None]]
    html_inputs = ["", "<p>Hola</p><p>mundo</p>", "<ul><li>uno</li><li>dos</li></ul>", "a<br>b<br/>c<BR />d",
                   "<h2>Título</h2><div>x&nbsp;y &amp; z &lt;tag&gt;</div>", "<script>alert(1)</script>ok<style>.a{}</style>",
                   "<p>a</p>\n\n\n\n<p>b</p>", "<ol><li>a</li>\n\n<li>b</li></ol>", "<table><tr><td>a</td></tr></table>"]
    html = [{"html": h, "text": core.html_to_text(h)} for h in html_inputs]
    section_bodies = [
        ("## Deploy\nuno\n### Sub\ndos\n## Otra\ntres", ("deploy",)),
        ("### Cómo correr\nx\n#### y\nz\n### Fin", ("cómo correr",)),
        ("## Notas\nx\n## Despliegue en PROD\npasos\n# Fin\nno", ("despleg",)),
        ("# Deploy\nnivel 1 no cuenta", ("deploy",)),
        ("##### Deploy\nnivel 5 no cuenta", ("deploy",)),
        ("## Promoción\n\n  \n", ("promoci",)),
        ("sin headings", ("deploy",)),
    ]
    sections = [{"body": b, "keywords": list(k), "section": core.extract_section(b, k)} for b, k in section_bodies]
    plan_bodies = [
        "", "## Plan\n\n*(lo completa la sesión de planificación)*\n\n## Pendientes\n- [ ] a\n",
        "## Plan\n" + "x" * 39 + "\n", "## Plan\n" + "x" * 40 + "\n## Pendientes\n- [x] a\n",
        "## plan de trabajo\n" + "y" * 50 + "\n## Pendientes\n- [ ] a\n", "## Planificación\n" + "y" * 50 + "\n",
        "## Plan\n" + "z" * 30 + "\n### detalle\n" + "z" * 30 + "\n# Fin\n",
    ]
    plans = [{"body": b, "gaps": core.plan_gaps(b)} for b in plan_bodies]
    return {"files": files, "periods": periods, "horas": horas, "slugify": slugs, "wi_re": wi_re, "ado_tipo": ado,
            "html_to_text": html, "extract_section": sections, "plan_gaps": plans,
            "constants": {"STATES": core.STATES, "CLOSED_STATES": sorted(core.CLOSED_STATES), "ENVS": core.ENVS,
                          "TYPES": core.TYPES, "KINDS": core.KINDS, "CATEGORIES": core.CATEGORIES,
                          "RUNNABLE_CATS": sorted(core.RUNNABLE_CATS), "PERIOD_DAY": core.PERIOD_DAY,
                          "MAX_TASK_LEN": core.MAX_TASK_LEN, "MAX_HORAS": core.MAX_HORAS, "BACKUP_KEEP": core.BACKUP_KEEP,
                          "GROUP_PREFIX": core.GROUP_PREFIX, "DELETED_DIR": core.DELETED_DIR,
                          "PLAN_PLACEHOLDER": core.PLAN_PLACEHOLDER, "PLAN_MIN_CHARS": core.PLAN_MIN_CHARS,
                          "EDITABLE": list(core.EDITABLE)}}


def read_text_cases():
    samples = {"utf8": "ñandú".encode("utf-8"), "utf8_bom": "﻿ñandú".encode("utf-8"), "cp1252": "ñandú €".encode("cp1252"),
               "invalid_both": b"\x81\x8d\x8f\x90\x9d", "mixed": "ñ".encode("utf-8") + "ñ".encode("cp1252"), "empty": b""}
    import tempfile
    from pathlib import Path
    out = []
    with tempfile.TemporaryDirectory() as d:
        for name, data in samples.items():
            p = Path(d) / name
            p.write_bytes(data)
            out.append({"name": name, "bytes_hex": data.hex(), "text": core.read_text(p)})
    return out


def main():
    write_cases("frontmatter", frontmatter_cases())
    write_cases("tasks", task_cases())
    write_cases("validate", validate_cases())
    write_cases("misc", misc_cases())


if __name__ == "__main__":
    main()
