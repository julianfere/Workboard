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
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "es2022",
  },
});
