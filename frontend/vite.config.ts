import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig } from "vite";

// https://vite.dev/config/
export default defineConfig({
  plugins: [svelte()],
  server: {
    proxy: {
      "/ws": { target: "ws://127.0.0.1:3000", ws: true },
    },
  },
  base: "./",
});
