// Renders crates/app/icons/icon.svg to the app's icons, next to it:
//
// - icon.png, 1024 px: the Linux AppImage's icon;
// - icon.ico: the Windows exe's (crates/app/build.rs) and the installer's;
// - icon.icns: the macOS app's.
//
// Run it after changing the SVG:
//
//   node scripts/render-icon.ts
//
// Chromium draws it, so the logo's glow filter comes out as it does in the browser. The .ico and .icns hold PNGs,
// which Windows Vista and macOS 10.7 and later read.
import { chromium } from "@playwright/test";
import * as fs from "node:fs";
import * as path from "node:path";

const dir = path.join(import.meta.dirname, "..", "..", "..", "crates", "app", "icons");
const svg = fs.readFileSync(path.join(dir, "icon.svg"), "utf8");

const ICO_SIZES = [16, 24, 32, 48, 64, 128, 256];
// The icns types that hold a PNG of one size.
const ICNS_TYPES: [string, number][] = [
  ["icp4", 16],
  ["icp5", 32],
  ["icp6", 64],
  ["ic07", 128],
  ["ic08", 256],
  ["ic09", 512],
  ["ic10", 1024],
];

const browser = await chromium.launch();
const pngs = new Map<number, Buffer>();
try {
  const page = await browser.newPage();
  for (const size of new Set([...ICO_SIZES, ...ICNS_TYPES.map(([, size]) => size)])) {
    await page.setViewportSize({ width: size, height: size });
    await page.setContent(`<style>html,body{margin:0;background:transparent}svg{display:block;width:${size}px;height:${size}px}</style>${svg}`);
    pngs.set(size, await page.locator("svg").first().screenshot({ omitBackground: true }));
  }
} finally {
  await browser.close();
}

fs.writeFileSync(path.join(dir, "icon.png"), pngs.get(1024)!);

// ICONDIR, then one ICONDIRENTRY per image, then the images. 256 is written as 0.
const header = Buffer.alloc(6 + 16 * ICO_SIZES.length);
header.writeUInt16LE(0, 0);
header.writeUInt16LE(1, 2);
header.writeUInt16LE(ICO_SIZES.length, 4);
let offset = header.length;
ICO_SIZES.forEach((size, i) => {
  const png = pngs.get(size)!;
  const entry = 6 + 16 * i;
  header.writeUInt8(size >= 256 ? 0 : size, entry);
  header.writeUInt8(size >= 256 ? 0 : size, entry + 1);
  header.writeUInt8(0, entry + 2);
  header.writeUInt8(0, entry + 3);
  header.writeUInt16LE(1, entry + 4);
  header.writeUInt16LE(32, entry + 6);
  header.writeUInt32LE(png.length, entry + 8);
  header.writeUInt32LE(offset, entry + 12);
  offset += png.length;
});
fs.writeFileSync(path.join(dir, "icon.ico"), Buffer.concat([header, ...ICO_SIZES.map((size) => pngs.get(size)!)]));

// "icns" and the file's length, then per image its type and length (both counting their 8-byte header).
const entries = ICNS_TYPES.map(([type, size]) => {
  const png = pngs.get(size)!;
  const head = Buffer.alloc(8);
  head.write(type, 0, "ascii");
  head.writeUInt32BE(png.length + 8, 4);
  return Buffer.concat([head, png]);
});
const icnsHead = Buffer.alloc(8);
icnsHead.write("icns", 0, "ascii");
icnsHead.writeUInt32BE(8 + entries.reduce((sum, entry) => sum + entry.length, 0), 4);
fs.writeFileSync(path.join(dir, "icon.icns"), Buffer.concat([icnsHead, ...entries]));

console.log(`Wrote icon.png, icon.ico and icon.icns in ${dir}`);
