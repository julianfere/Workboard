# Bugs de la v1 registrados en los golden

Los golden capturan la v1 **tal cual**, bugs incluidos. Cada bug de esta lista tiene una decisión: el
port lo replica (paridad) o lo corrige; si lo corrige, el test de Rust marca ese caso como divergencia
esperada y lo explica.

## 1. Un merge *desde* development se expande como si fuera un merge de PR

- **Dónde**: `gitpanel._mentioned_commits`.
- **Qué pasa**: el mensaje `Merge branch 'development' into Feature/444444-carrito` menciona el WI por el
  nombre de la rama. La función lo trata como un merge de PR y lo expande a los commits que trajo
  (`M^1..M`), que son commits de `development` que entraron a la rama, no trabajo del WI.
- **Efecto**: la promoción a QA de 444444 propone, **tildado**, `1811422 otro trabajo en development` (de
  otra persona y sin WI). Ver `git.json` → `promotion_qa_one` y `promotion_qa`.
- **Decisión**: pendiente.

## 2. `workboard_run_tool` sin `wi` falla dentro de una sesión

- **Dónde**: `mcp_server.workboard_run_tool`.
- **Qué pasa**: `_session_wis()` devuelve un `set` y el código hace `wis[0]`.
- **Efecto**: `'set' object is not subscriptable` en vez de correr la tool en la carpeta del primer WI de
  la sesión. Ver `mcp.json` → `session`, última llamada.
- **Decisión**: corregido en el port (Fase 3). Sin `item`, `workboard_run_tool` usa el primer item de la
  sesión; el test del MCP espera ese comportamiento en vez del error de la v1.

## 3. La URL web del remote solo se arma para Azure DevOps

- **Dónde**: `gitpanel.remote_web_url`.
- **Qué pasa**: solo reconoce remotes `https://…/_git/…` (Azure DevOps). Un remote de GitHub o GitLab da
  `None`, así que la promoción no puede ofrecer el link para abrir el PR.
- **Efecto**: `git.json` → `helpers.remote_web_url`, el caso `https://github.com/acme/app.git` espera `null`.
- **Decisión**: corregido en el port (Fase 5): `wb_git::web_url` también reconoce GitHub y GitLab (https y
  `git@host:dueño/repo`) y devuelve la forja (`forge`) para que la UI arme el link del PR de cada una. El
  test espera `https://github.com/acme/app` en ese caso; los de Azure siguen iguales a la v1.
