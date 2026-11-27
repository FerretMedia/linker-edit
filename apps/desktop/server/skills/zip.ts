import { inflateRawSync } from "node:zlib";

/** The files in a zip: stored and deflated ones, which is what Claude's skill exports and every zip tool make. */
export interface Unzipped {
  files: { path: string; bytes: Uint8Array }[];
  /** Files it can't unpack, and why. */
  skipped: { path: string; reason: string }[];
}

const END = 0x06054b50;
const ENTRY = 0x02014b50;
const LOCAL = 0x04034b50;

/**
 * Reads a zip from its central directory. Throws when it isn't one, or holds more than `entries` files or unpacks
 * to more than `bytes`, which is how a zip bomb looks.
 */
export function unzip(zip: Uint8Array, limits: { entries: number; bytes: number }): Unzipped {
  const view = new DataView(zip.buffer, zip.byteOffset, zip.byteLength);
  const u16 = (at: number) => view.getUint16(at, true);
  const u32 = (at: number) => view.getUint32(at, true);
  const fits = (at: number, length: number) => at >= 0 && at + length <= zip.byteLength;

  // The end record is in the last 22 bytes, or before a comment of up to 64 KB.
  let end = zip.byteLength - 22;
  while (end >= Math.max(0, zip.byteLength - 22 - 0xffff) && u32(end) !== END) end -= 1;
  if (end < 0 || !fits(end, 22) || u32(end) !== END) throw new Error("It isn't a zip file.");
  const count = u16(end + 10);
  let at = u32(end + 16);
  if (count === 0xffff || at === 0xffffffff) throw new Error("It's a Zip64 file, which can't be read here.");
  if (count > limits.entries) throw new Error(`It holds more than ${limits.entries.toLocaleString("en-US")} files.`);

  const files: Unzipped["files"] = [];
  const skipped: Unzipped["skipped"] = [];
  let unpacked = 0;
  for (let index = 0; index < count; index += 1) {
    if (!fits(at, 46) || u32(at) !== ENTRY) throw new Error("It's damaged.");
    const flags = u16(at + 8);
    const method = u16(at + 10);
    const packedSize = u32(at + 20);
    const size = u32(at + 24);
    const nameLength = u16(at + 28);
    const local = u32(at + 42);
    const path = new TextDecoder().decode(zip.subarray(at + 46, at + 46 + nameLength));
    at += 46 + nameLength + u16(at + 30) + u16(at + 32);
    if (path.endsWith("/")) continue;
    if (flags & 1) {
      skipped.push({ path, reason: "It's encrypted." });
      continue;
    }
    if (method !== 0 && method !== 8) {
      skipped.push({ path, reason: "It's packed in a way that can't be read here." });
      continue;
    }
    if (!fits(local, 30) || u32(local) !== LOCAL) throw new Error("It's damaged.");
    const start = local + 30 + u16(local + 26) + u16(local + 28);
    if (!fits(start, packedSize)) throw new Error("It's damaged.");
    const packed = zip.subarray(start, start + packedSize);
    const left = limits.bytes - unpacked;
    if (size > left) throw new Error(`It unpacks to more than ${Math.round(limits.bytes / 1024 / 1024)} MB.`);
    let bytes: Uint8Array;
    try {
      // The size it states can lie, so inflating stops at what is left.
      bytes = method === 0 ? packed : inflateRawSync(packed, { maxOutputLength: Math.max(1, left) });
    } catch {
      throw new Error(`It's damaged, or unpacks to more than ${Math.round(limits.bytes / 1024 / 1024)} MB.`);
    }
    unpacked += bytes.byteLength;
    if (unpacked > limits.bytes) throw new Error(`It unpacks to more than ${Math.round(limits.bytes / 1024 / 1024)} MB.`);
    files.push({ path, bytes });
  }
  return { files, skipped };
}
