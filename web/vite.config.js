import { realpathSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { defineConfig } from "vite";

const here = dirname(fileURLToPath(import.meta.url));
const vendor = (...parts) => resolve(here, "src", "vendor", "input-bindings", ...parts);

// Bun links git dependencies from its global cache: let the dev server serve the renderer
// package's own files (the imported-character WASM bridge) from there.
const renderer = realpathSync(resolve(here, "node_modules", "@moritzbrantner", "three-d-renderer"));
// The same for the game-audio package's wasm director (#67, 67c).
const gameAudio = realpathSync(resolve(here, "node_modules", "@moritzbrantner", "game-audio"));

export default defineConfig({
  base: "/arpg/",
  server: {
    fs: { allow: [here, renderer, gameAudio] },
  },
  // Pre-bundling would move the wasm-bindgen glue away from the `.wasm` it loads by URL.
  optimizeDeps: { exclude: ["@moritzbrantner/game-audio"] },
  oxc: {
    jsx: { runtime: "automatic" },
  },
  resolve: {
    alias: [
      {
        find: "@moritzbrantner/input-bindings-react/styles.css",
        replacement: vendor("packages", "input-bindings-react", "src", "styles.css"),
      },
      {
        find: "@moritzbrantner/input-bindings-react",
        replacement: vendor("packages", "input-bindings-react", "src", "index.tsx"),
      },
      {
        find: "@moritzbrantner/input-bindings-runtime",
        replacement: vendor("packages", "input-bindings-runtime", "src", "index.ts"),
      },
      {
        find: "@moritzbrantner/input-bindings-web",
        replacement: vendor("packages", "input-bindings-web", "src", "index.ts"),
      },
      {
        find: "@moritzbrantner/input-bindings",
        replacement: vendor("packages", "input-bindings", "src", "public.ts"),
      },
    ],
  },
});
