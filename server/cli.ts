#!/usr/bin/env node
// `npx linker-edit@latest`: sets up davinci-resolve-mcp when it's missing, starts the server and opens the app window.
// `npx linker-edit@latest setup` only does the setup; `--browser` opens the app in the browser instead of a window.
import { spawn } from "node:child_process";
import * as fs from "node:fs";
import * as path from "node:path";
import { APP_ROOT, loadConfigOrExit } from "./config.ts";
import { setupResolveMcp } from "./setup.ts";
import { VERSION } from "./updates.ts";

const command = process.argv[2];

if (command === "--version") {
  console.log(VERSION);
  process.exit(0);
}
if (command !== undefined && command !== "setup" && command !== "--browser") {
  console.error("Usage: linker-edit [setup | --browser | --version]");
  process.exit(1);
}

const ready = await setupResolveMcp(loadConfigOrExit().resolveMcp);
if (command === "setup") process.exit(ready ? 0 : 1);

// Starts anyway when setup failed: the Resolve status in the app says what's missing.
const { server, url } = await import("./index.ts");
const open = () => (command === "--browser" ? openBrowser(url) : void openWindow(url));
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
