// Renders desktop/resources/icon.svg to the 1024 px icon.png electron-builder makes the app icons from.
// Run it after changing the SVG:
//
//   node scripts/render-icon.ts
//
// Chromium draws it, so the logo's glow filter comes out as it does in the browser.
import { chromium } from "@playwright/test";
import * as fs from "node:fs";
import * as path from "node:path";

const dir = path.join(import.meta.dirname, "..", "desktop", "resources");
const svg = fs.readFileSync(path.join(dir, "icon.svg"), "utf8");

const browser = await chromium.launch();
try {
  const page = await browser.newPage({ viewport: { width: 1024, height: 1024 } });
  await page.setContent(`<style>html,body{margin:0;background:transparent}svg{display:block;width:1024px;height:1024px}</style>${svg}`);
  await page.locator("svg").first().screenshot({ path: path.join(dir, "icon.png"), omitBackground: true });
} finally {
  await browser.close();
}
console.log("Wrote desktop/resources/icon.png");
