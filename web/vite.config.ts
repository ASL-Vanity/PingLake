import { defineConfig, loadEnv } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig(({ mode }) => ({
  plugins: [react()],
  build: {
    outDir: "dist",
    emptyOutDir: true,
    sourcemap: false,
  },
  server: {
    host: "127.0.0.1",
    port: 4173,
    proxy: {
      "/api": loadEnv(mode, ".", "PINGLAKE_").PINGLAKE_DEV_HUB_URL || "http://127.0.0.1:8090",
    },
  },
}));
