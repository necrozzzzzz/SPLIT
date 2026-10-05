import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

const host = process.env.TAURI_DEV_HOST;
const edition = process.env.SPLIT_EDITION ?? "borderless";

if (edition !== "borderless" && edition !== "fullscreen") {
  throw new Error(
    `Unsupported SPLIT_EDITION=${JSON.stringify(edition)}; expected "borderless" or "fullscreen"`,
  );
}

export default defineConfig({
  plugins: [react()],
  define: {
    __SPLIT_EDITION__: JSON.stringify(edition),
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
});
