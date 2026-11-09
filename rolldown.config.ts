import { defineConfig } from "rolldown";
import pkg from "./package.json" with { type: "json" };

// Node won't strip TypeScript types inside node_modules, so the npm package runs this bundle.
export default defineConfig({
  input: "server/cli.ts",
  platform: "node",
  external: Object.keys(pkg.dependencies).map((name) => new RegExp(`^${name}(/|$)`)),
  output: { file: "dist/linker-edit.js", format: "esm", codeSplitting: false },
});
