import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [react()],
  server: {
    port: 3120,
    strictPort: true,
    proxy: {
      "/api": {
        target: process.env.BUSINESS_CORE_DEV_URL ?? "http://127.0.0.1:3110",
        changeOrigin: false,
      },
    },
  },
});
