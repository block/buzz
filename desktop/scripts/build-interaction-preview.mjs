import { build } from "vite";

// Separate output and entry point keep every demo transport out of app bundles.
await build({
  configFile: "vite.config.ts",
  mode: "e2e",
  build: {
    outDir: "dist-interaction-preview",
    rolldownOptions: { input: "interaction-preview/index.html" },
  },
});
