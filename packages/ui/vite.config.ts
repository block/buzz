import { copyFileSync } from "node:fs";
import { defineConfig } from "vite";
export default defineConfig({
  plugins: [
    {
      name: "license-notices",
      apply: "build",
      closeBundle() {
        copyFileSync("THIRD_PARTY_NOTICES.md", "dist/THIRD_PARTY_NOTICES.md");
        copyFileSync("../../LICENSE", "dist/LICENSE");
      },
    },
  ],
  build: {
    lib: {
      entry: "src/index.ts",
      formats: ["es"],
      fileName: "index",
      cssFileName: "ui",
    },
    rollupOptions: {
      external: [
        /^react($|\/)/,
        /^react-dom($|\/)/,
        /^@base-ui\//,
        /^lucide-react/,
        /^@daypicker\//,
        /^react-resizable-panels/,
      ],
    },
  },
});
