# Spike Rust + Tauri (2026-11-18)

Decide si el port de Workboard a Rust + Tauri 2 es viable. **Veredicto: pasa.**

## Terminal embebida — `spike/terminal`

Tauri 2.12 + `portable-pty` 0.9 (ConPTY) + xterm.js (el mismo vendorizado de la v1). La salida va al
front por un `tauri::ipc::Channel` con bytes crudos; no hace falta WebSocket.

| Criterio | Resultado |
|---|---|
| TUI de Claude Code (colores, bordes, ancho, barra de estado) | ✓ |
| Resize, acentos/ñ, Esc, flechas, Ctrl+C, pegar multilínea, scroll | ✓ (prueba manual del usuario) |
| Cerrar la ventana mata el árbol (`taskkill /PID /T`) | ✓ |
| Consultas de terminal de Claude (`ESC[6n`, foco, `ESC[>0q`, `ESC[?u`, OSC 7501) | ✓ xterm contesta las necesarias |
| Throughput sin ventana (`dir /s` System32, 2,7 MB) | 9,2–18,9 s (pywinpty en la v1: 15,8 s; sin pty: <2 s) |
| Mismo caso en la ventana | 40 s → 30 s al agrupar la salida en lotes de 8 ms |
| Exe release | 9 MB |

Aprendizajes:
- **ConPTY arranca con `ESC[6n`** (posición del cursor) y no dibuja nada hasta recibir respuesta. En la
  app la contesta xterm.js; cualquier consumidor sin terminal (tests, bench) tiene que contestar
  `ESC[1;1R`.
- ConPTY entrega de a ~65 bytes: **no mandar una lectura = un mensaje IPC**. Agrupar en un hilo aparte
  (8 ms o 256 KB) bajó un 25 % el caso masivo sin latencia perceptible al tipear.
- El techo es ConPTY (conhost), no el lenguaje. Para salidas enormes, lo que queda es el render de
  xterm (probar el addon WebGL en la app real).
- En Windows el lector no ve EOF hasta que se suelta el master: al salir el hijo, soltar el master.

## MCP — `spike/mcp` (`spike-mcp`)

`rmcp` 3.11 (SDK oficial) por stdio, con `#[tool_router]`/`#[tool_handler]` y JSON Schema por
`schemars`. `initialize` (con `instructions`), `tools/list` y `tools/call` andan; `claude -p
--mcp-config … --strict-mcp-config` lo usó sin cambios. Exe release: 3,3 MB.

## Hook — `spike/mcp` (`hook`)

Std puro, POST HTTP a mano. Tiempo por invocación medido desde Git Bash (20 corridas):

| | ms |
|---|---|
| `cmd /c exit` (piso de lanzar un proceso) | 104 |
| **hook Rust** | **76** |
| `py wb_hook.py` (v1) | 369 |
| `Workboard.exe --hook` (v1 congelada) | 666 |

Exe: 155 KB.

## Pendiente para la app real
- Medir render de xterm con el addon WebGL.
- Respuesta a `ESC[>0q` (XTVERSION) por si algún agente la necesita.
- macOS/Linux: repetir terminal y MCP cuando haya CI.
