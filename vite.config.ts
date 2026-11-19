import { defineConfig } from "vite";

// La UI vive en ui/ y lee los catálogos de locales/ y los presets de crates/wb-core/presets/ (los mismos
// archivos que usa el backend). Sin CDNs: todo se empaqueta y la app funciona offline.
export default defineConfig({
  root: "ui",
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    fs: { allow: [".."] },
    // En el navegador la API va al wb-devserver (cargo run -p wb-api --bin wb-devserver).
    proxy: { "/api": "http://127.0.0.1:8766" },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "es2022",
  },
});
