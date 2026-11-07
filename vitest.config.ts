import { defineConfig } from "vitest/config";

// Separate from vite.config.ts: unit tests run in Node and don't need the SvelteKit plugin.
export default defineConfig({
  test: {
    include: ["server/**/*.test.ts", "src/**/*.test.ts"],
  },
});
