#!/usr/bin/env node
// `npx linker-edit@latest`: starts the server and opens the app window. `--browser` opens the app in the browser
// instead of a window. Nothing needs setting up: Linker-Edit drives Resolve through the MCP server Resolve comes with.
// `--serve` only starts the server: the installed desktop app runs it this way and opens its own window.
import { spawn } from "node:child_process";
import * as fs from "node:fs";
import * as path from "node:path";
import { APP_ROOT } from "./config.ts";
import { VERSION } from "./updates.ts";

const command = process.argv[2];

if (command === "--version") {
  console.log(VERSION);
  process.exit(0);
}
if (command !== undefined && command !== "setup" && command !== "--browser" && command !== "--serve") {
  console.error("Usage: linker-edit [--browser | --serve | --version]");
  process.exit(1);
}

// Kept for older instructions; there is nothing to set up any more.
if (command === "setup") {
  console.log("Nothing to set up: Linker-Edit uses the MCP server that comes with DaVinci Resolve Studio 21.1.2 and newer.");
  process.exit(0);
}

const { server, url } = await import("./index.ts");
const open = () => {
  if (command === "--serve") process.send?.({ type: "ready", url });
  else if (command === "--browser") openBrowser(url);
  else void openWindow(url);
};
if (server.listening) open();
else server.once("listening", open);

/** Opens the desktop window; closing it stops Linker-Edit. Falls back to the browser when Electron can't start. */
async function openWindow(url: string) {
  const main = path.join(APP_ROOT, "dist", "desktop.js");
  let electron: string;
  try {
    // The electron package's main export is the path to its binary.
    electron = (await import("electron")).default as unknown as string;
    if (!fs.existsSync(main)) throw new Error(`${main} is missing. Run \`npm run build\`.`);
  } catch (error) {
    console.log(`Can't open the app window (${error instanceof Error ? error.message : String(error)}).`);
    openBrowser(url);
    return;
  }
  const env: NodeJS.ProcessEnv = { ...process.env, LINKER_EDIT_URL: url };
  delete env.ELECTRON_RUN_AS_NODE;
  spawn(electron, [main], { env, stdio: "inherit" })
    .on("error", (error) => {
      console.log(`Can't open the app window (${error.message}).`);
      openBrowser(url);
    })
    .on("exit", (code) => {
      if (code === 0) process.exit(0);
      console.log("The app window closed unexpectedly. Linker-Edit keeps running.");
      openBrowser(url);
    });
}

function openBrowser(url: string) {
  const [command, args] =
    process.platform === "win32"
      ? ["cmd", ["/c", "start", "", url]]
      : [process.platform === "darwin" ? "open" : "xdg-open", [url]];
  spawn(command, args, { stdio: "ignore", detached: true, windowsHide: true })
    .on("error", () => console.log(`Open ${url} in your browser.`))
    .unref();
}
