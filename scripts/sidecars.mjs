// Compila wb-mcp y wb-hook y los deja en src-tauri/binaries/<nombre>-<target>[.exe], que es como Tauri
// espera los `externalBin`: los copia junto al ejecutable de la app (sin el sufijo), donde la app los busca.
// Uso: node scripts/sidecars.mjs [--release]. El target sale de TAURI_ENV_TARGET_TRIPLE (lo define
// `tauri build --target …`) o del host de rustc.
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const release = process.argv.includes("--release");
const host = execFileSync("rustc", ["-vV"], { encoding: "utf8" }).match(/^host: (\S+)$/m)[1];
const target = process.env.TAURI_ENV_TARGET_TRIPLE || host;
const args = ["build", "-p", "wb-mcp", "-p", "wb-hook"];
if (release) args.push("--release");
if (target !== host) args.push("--target", target);
execFileSync("cargo", args, { cwd: root, stdio: "inherit" });

const profile = release ? "release" : "debug";
const out = target !== host ? join(root, "target", target, profile) : join(root, "target", profile);
const ext = target.includes("windows") ? ".exe" : "";
const dest = join(root, "src-tauri", "binaries");
mkdirSync(dest, { recursive: true });
for (const name of ["wb-mcp", "wb-hook"]) {
  copyFileSync(join(out, name + ext), join(dest, `${name}-${target}${ext}`));
}
console.log(`sidecars ready for ${target} (${profile})`);
