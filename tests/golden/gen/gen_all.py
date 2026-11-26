"""Regenera todos los golden files desde la v1:  py tests/golden/gen/gen_all.py

Requiere la v1 en `WORKBOARD_V1` (o `../workboard-v1`), Python 3.12 con `mcp` 2.x y git en el PATH.
"""

import build_repo
import build_work
import gen_core
import gen_git
import gen_mcp
import gen_workspace

if __name__ == "__main__":
    print("fixtures:")
    build_work.build()
    build_repo.build()
    print("casos:")
    gen_core.main()
    gen_workspace.main()
    gen_git.main()
    gen_mcp.main()
