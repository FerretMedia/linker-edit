import adapter from "@sveltejs/adapter-static";
import { sveltekit } from "@sveltejs/kit/vite";
import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [
    tailwindcss(),
    sveltekit({
      // A single-page app: server/index.ts serves build/ and owns all state over /ws.
      adapter: adapter({ fallback: "index.html" }),
    }),
  ],
  server: {
    port: 5317,
    proxy: { "/ws": { target: "ws://127.0.0.1:4317", ws: true }, "/media": "http://127.0.0.1:4317" },
  },
});
