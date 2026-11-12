import { defineConfig } from "vitest/config";

// Separate from vite.config.ts: unit tests run in Node and don't need the SvelteKit plugin.
export default defineConfig({
  test: {
    include: ["server/**/*.test.ts", "src/**/*.test.ts", "scripts/**/*.test.ts", "desktop/**/*.test.ts"],
    // The edit history tests run git many times, which on Windows can pass 5 seconds while every file runs at once.
    testTimeout: 20_000,
  },
});
