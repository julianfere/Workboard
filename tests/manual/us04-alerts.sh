#!/usr/bin/env bash
# Arma una carpeta de trabajos (preset classic-es) para ver a mano las alertas y Hoy (US-04).
#   bash tests/manual/us04-alerts.sh <carpeta-nueva>
#   WORKBOARD_CONFIG_DIR=$(mktemp -d) cargo run -p wb-api --bin wb-devserver -- <carpeta-nueva> --port 8804
# (en la UI: Ajustes → preset classic-es). Qué se espera ver está en cada carpeta, abajo.
set -euo pipefail
root=${1:?uso: us04-alerts.sh <carpeta-nueva>}
mkdir -p "$root"
cd "$root"
today=$(date +%F)
yesterday=$(date -d "-3 days" +%F 2>/dev/null || date -v-3d +%F)

item() { # carpeta, frontmatter (líneas), título
  mkdir -p "$1"
  printf -- "---\n%s\n---\n# %s\n" "$2" "$3" >"$1/README.md"
}
old() { # pone la carpeta 30 días atrás (en Windows también la fecha de creación, que cuenta como actividad)
  find "$1" -exec touch -d "30 days ago" {} + 2>/dev/null || find "$1" -exec touch -t "$(date -v-30d +%Y%m%d%H%M)" {} +
  if command -v powershell.exe >/dev/null; then
    powershell.exe -NoProfile -Command "Get-ChildItem -Recurse -Force '$(cygpath -w "$PWD/$1")' | ForEach-Object { \$_.CreationTime = (Get-Date).AddDays(-30) }; (Get-Item '$(cygpath -w "$PWD/$1")').CreationTime = (Get-Date).AddDays(-30)"
  fi
}

# «MIGRATION / FIX sin correr en PROD» (y en «Necesita tu atención», en rojo).
item wi100001-migracion $'estado: en-curso\ntipo: us\nambientes: [qa]\nnotas: falta correr en prod' "Migración pendiente"
echo "select 1;" >wi100001-migracion/MIGRATION_add_col.sql
# Sin alerta: ya figura en PROD.
item wi100002-fix-en-prod $'estado: qa\nambientes: [qa, prod]' "Fix ya corrido"
echo "update x;" >wi100002-fix-en-prod/FIX_datos.sql
# Sin alerta: cerrado.
item wi100003-fix-cerrado $'estado: prod' "Fix cerrado"
echo "update y;" >wi100003-fix-cerrado/FIX_viejo.sql
# Sin alerta: SEED no está en alerts.finalEnvKinds (como la v1).
item wi100004-seed $'estado: en-curso' "Solo seed"
echo "insert;" >wi100004-seed/SEED_base.sql
# «Activas sin movimiento» (en-curso hace 30 días); el diagnóstico quieto no avisa.
item wi100005-viejo $'estado: en-curso' "Trabajo quieto"
old wi100005-viejo
item wi100006-diag-viejo $'estado: diag' "Diagnóstico quieto"
old wi100006-diag-viejo
# Sin README: «Sin README» + «Sin estado» («Crear README» crea solo el título, sin estado ni abrir el detalle).
mkdir -p wi100007-sin-readme && echo "notas" >wi100007-sin-readme/notas.txt
# «Número sin el formato de carpeta»: sin id y empieza como uno.
item 1234567-sin-prefijo $'estado: pendiente' "Carpeta mal nombrada"
# Duplicados: un conjunto con «Agrupar», «Es intencional» y «A eliminados» por carpeta.
item wi100008-parte-a $'estado: en-curso' "Parte A"
echo a >wi100008-parte-a/a.txt
item wi100008-parte-b $'estado: pendiente' "Parte B"
# En un grupo: mismo id, no es duplicado. Hoy cuenta el grupo y no sus miembros.
item grp-pagos $'estado: en-curso\nnotas: grupo de pagos' "Pagos"
item grp-pagos/wi100009-api $'estado: en-curso' "API de pagos"
item grp-pagos/wi100009-front $'estado: codeado' "Front de pagos"
# Vencida: en «Necesita tu atención» con estado + notas; en curso con «Pactada …» y notas (sin horas).
item wi100010-vencida "estado: codeado
pactada: $yesterday
notas: esperando QA" "Entrega vencida"
printf "\n## Pendientes\n\n- [ ] Revisar con QA\n- [ ] Avisar al cliente\n" >>wi100010-vencida/README.md
echo "Listo: $root (hoy $today)"
