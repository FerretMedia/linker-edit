#!/usr/bin/env node
// `npx linker-edit@latest`: sets up davinci-resolve-mcp when it's missing, starts the server and opens it.
// `npx linker-edit@latest setup` only does the setup.
import { spawn } from "node:child_process";
import { loadConfigOrExit } from "./config.ts";
import { setupResolveMcp } from "./setup.ts";
import { VERSION } from "./updates.ts";

const command = process.argv[2];

if (command === "--version") {
  console.log(VERSION);
  process.exit(0);
}
if (command !== undefined && command !== "setup") {
  console.error("Usage: linker-edit [setup | --version]");
  process.exit(1);
}

const ready = await setupResolveMcp(loadConfigOrExit().resolveMcp);
if (command === "setup") process.exit(ready ? 0 : 1);

// Starts anyway when setup failed: the Resolve status in the app says what's missing.
const { server, url } = await import("./index.ts");
if (server.listening) openBrowser(url);
else server.once("listening", () => openBrowser(url));

function openBrowser(url: string) {
  const [command, args] =
    process.platform === "win32"
      ? ["cmd", ["/c", "start", "", url]]
      : [process.platform === "darwin" ? "open" : "xdg-open", [url]];
  spawn(command, args, { stdio: "ignore", detached: true, windowsHide: true })
    .on("error", () => console.log(`Open ${url} in your browser.`))
    .unref();
}
