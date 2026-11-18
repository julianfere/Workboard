"""Arma `fixtures/work/`: una carpeta `_work` ficticia que cubre los casos que maneja la v1.

Se versiona tal cual (ver .gitattributes: sin conversión de fin de línea). Git no guarda carpetas
vacías, así que cada carpeta tiene al menos un archivo.
"""

import shutil

from common import FIXTURES

WORK = FIXTURES / "work"

LF = "\n"


def w(rel, data):
    p = WORK / rel
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_bytes(data if isinstance(data, bytes) else data.encode("utf-8"))


def build():
    if WORK.exists():
        shutil.rmtree(WORK)

    # WI completo: frontmatter variado, pendientes con sub-ítems y texto suelto, deploy, archivos de cada tipo.
    w("wi111111-login-con-sso/README.md", LF.join([
        "---",
        "estado: en-curso",
        "tipo: us",
        "ambientes: [dev]",
        'notas: "falta: revisar el callback"',
        "rama: feature/111111-sso",
        "pactada: 2026-11-25",
        "horas: 12,5",
        "# comentario: se ignora",
        "---",
        "",
        "# Login con SSO",
        "",
        "Texto libre.",
        "",
        "## Pendientes",
        "",
        "- [ ] Configurar el proveedor",
        "  - [ ] sub-ítem: no es tarea",
        "- [x] Crear la tabla de sesiones",
        "Texto suelto dentro de la sección.",
        "- [ ]   Probar   con   espacios  ",
        "",
        "## Deploy",
        "",
        "1. Correr la migración.",
        "### Detalle",
        "Sigue dentro del deploy.",
        "",
        "## Notas",
        "",
        "- [ ] fuera de Pendientes: no es tarea",
        "",
    ]))
    w("wi111111-login-con-sso/MIGRATION_WI111111_sesiones.sql", "CREATE TABLE sesiones (id INT);\n")
    w("wi111111-login-con-sso/ROLLBACK_WI111111_sesiones.sql", "DROP TABLE sesiones;\n")
    w("wi111111-login-con-sso/DIAG_WI111111_usuarios.sql", "SELECT 1;\n")
    w("wi111111-login-con-sso/gen_diag.py", "print('genera un DIAG')\n")
    w("wi111111-login-con-sso/consulta.sql", "SELECT 2;\n")
    w("wi111111-login-con-sso/docs/PLAN_sso.md", "# Plan\n")
    w("wi111111-login-con-sso/docs/flujo.drawio", "<mxfile/>\n")
    w("wi111111-login-con-sso/captura.png", bytes.fromhex("89504e470d0a1a0a0000000d49484452"))
    w("wi111111-login-con-sso/datos.bin", b"\x00\x01\x02")
    w("wi111111-login-con-sso/.oculto/secreto.txt", "no se escanea\n")

    # Dos WI en el nombre, README con CRLF y BOM.
    w("wi222222-333333-pagos/README.md", ("﻿---\r\nestado: qa\r\ntipo: bug\r\nambientes: [dev, qa]\r\n---\r\n\r\n"
                                          "# Pagos duplicados\r\n\r\n## Pendientes\r\n\r\n- [ ] Verificar en QA\r\n").encode("utf-8"))
    w("wi222222-333333-pagos/FIX_WI222222_duplicados.sql", "UPDATE pagos SET x = 1;\n")

    # Mismo número que otra carpeta (dupes) y README en cp1252.
    w("wi111111-copia-vieja/README.md", "---\nestado: descartado\n---\n\n# Copia vieja de sesión\n".encode("cp1252"))

    # Sin número en el nombre: wi en el frontmatter (lista) y tipo fix.
    w("arreglo-sin-wi/README.md", "---\ntipo: fix\nwi: [444444, 555555]\nestado: codeado\n---\n# Arreglo sin work item\n")
    w("arreglo-sin-wi/SEED_datos.sql", "INSERT INTO t VALUES (1);\n")

    # Plan vacío (planGaps) y con el texto de relleno.
    w("wi666666-plan-pendiente/README.md", LF.join([
        "---", "estado: plan", "---", "# Plan pendiente", "", "## Plan", "",
        "*(lo completa la sesión de planificación)*", "", "## Pendientes", "", "- [x] hecho", "",
    ]))

    # Sin README.
    w("wi777777-sin-readme/notas.txt", "solo un archivo\n")

    # Grupo con README propio, archivo compartido y dos WI.
    w("grp-checkout/README.md", "---\nestado: en-curso\nnotas: grupo del checkout\n---\n# Checkout\n\n## Pendientes\n\n- [ ] Coordinar deploy\n")
    w("grp-checkout/VERIF_checkout.sql", "SELECT 3;\n")
    w("grp-checkout/wi444444-carrito/README.md", "---\nestado: diag\ntipo: task\n---\n# Carrito\n")
    w("grp-checkout/wi444444-carrito/script.ps1", "Write-Host hola\n")
    w("grp-checkout/wi555555-envio/README.md", "---\nestado: pendiente\ntipo: raro\n---\n# Envío\n")
    w("grp-checkout/_privado/x.txt", "dentro del grupo pero ignorado\n")

    # Ignorados en la raíz.
    w("_deleted/wi888888-eliminado/README.md", "---\nestado: qa\n---\n# Eliminado\n")
    w("_deleted/grp-viejo/README.md", "# Grupo eliminado\n")
    w("_interno/x.txt", "ignorado\n")
    w(".config/x.txt", "ignorado\n")
    w("suelto.md", "archivo en la raíz: no es un WI\n")

    print(f"  {WORK} armado")


if __name__ == "__main__":
    build()
