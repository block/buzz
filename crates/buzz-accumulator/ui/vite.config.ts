import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// The lab talks to the daemon through this dev proxy: same origin, no CORS,
// and the daemon's HTTP API stays loopback-pure with zero UI knowledge.
export default defineConfig({
  plugins: [react()],
  server: {
    proxy: {
      "/api": {
        target: "http://127.0.0.1:4640",
        rewrite: (path) => path.replace(/^\/api/, ""),
      },
    },
  },
});
