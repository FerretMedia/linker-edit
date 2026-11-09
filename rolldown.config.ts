import { defineConfig } from "rolldown";
import pkg from "./package.json" with { type: "json" };

const external = Object.keys(pkg.dependencies).map((name) => new RegExp(`^${name}(/|$)`));

// Node won't strip TypeScript types inside node_modules, so the npm package runs these bundles.
export default defineConfig([
  {
    input: "server/cli.ts",
    platform: "node",
    external,
    output: { file: "dist/linker-edit.js", format: "esm", codeSplitting: false },
  },
  // The agents' "linker" MCP server runs as its own process.
  {
    input: "server/transcribe/mcpServer.ts",
    platform: "node",
    external,
    output: { file: "dist/linker-mcp.js", format: "esm", codeSplitting: false },
  },
]);
