import zlib from "node:zlib";

// Just enough ZIP for Resolve's .drp/.drt: stored or deflated entries, no ZIP64, no encryption.
// Writing copies every entry that wasn't changed byte for byte, so only edited entries differ.

const LOCAL = 0x04034b50;
const CENTRAL = 0x02014b50;
const END = 0x06054b50;
const DESCRIPTOR = 0x08074b50;
const HAS_DESCRIPTOR = 0x0008;

export interface ZipEntry {
  name: string;
  method: number;
  crc: number;
  /** The entry as stored: local header, data and any data descriptor. */
  local: Buffer;
  /** Its central directory record. */
  central: Buffer;
  /** Where the compressed data sits inside `local`. */
  dataStart: number;
  dataSize: number;
}

export interface Zip {
  entries: ZipEntry[];
  /** The end of central directory record, comment included. */
  end: Buffer;
}

export function readZip(buffer: Buffer): Zip {
  const endAt = findEnd(buffer);
  const end = buffer.subarray(endAt);
  const count = end.readUInt16LE(10);
  let at = end.readUInt32LE(16);
  if (end.readUInt16LE(4) !== 0 || count !== end.readUInt16LE(8) || count === 0xffff || at === 0xffffffff) {
    throw new Error("Not a .drp or .drt: the ZIP is split or ZIP64, which Resolve doesn't write.");
  }
  const entries: ZipEntry[] = [];
  for (let i = 0; i < count; i++) {
    if (buffer.readUInt32LE(at) !== CENTRAL) throw new Error("Not a .drp or .drt: the ZIP's central directory is damaged.");
    const centralLength = 46 + buffer.readUInt16LE(at + 28) + buffer.readUInt16LE(at + 30) + buffer.readUInt16LE(at + 32);
    const central = buffer.subarray(at, at + centralLength);
    at += centralLength;

    const flags = central.readUInt16LE(8);
    const method = central.readUInt16LE(10);
    const dataSize = central.readUInt32LE(20);
    const offset = central.readUInt32LE(42);
    const name = central.toString("utf8", 46, 46 + central.readUInt16LE(28));
    if (flags & 0x0001) throw new Error(`Not a .drp or .drt: ${name} is encrypted.`);
    if (method !== 0 && method !== 8) throw new Error(`Not a .drp or .drt: ${name} uses ZIP method ${method}.`);
    if (dataSize === 0xffffffff || offset === 0xffffffff) throw new Error("Not a .drp or .drt: ZIP64 entries aren't supported.");
    if (buffer.readUInt32LE(offset) !== LOCAL) throw new Error(`Not a .drp or .drt: ${name} has no local header.`);

    const dataStart = 30 + buffer.readUInt16LE(offset + 26) + buffer.readUInt16LE(offset + 28);
    let length = dataStart + dataSize;
    if (flags & HAS_DESCRIPTOR) length += buffer.readUInt32LE(offset + length) === DESCRIPTOR ? 16 : 12;
    entries.push({
      name,
      method,
      crc: central.readUInt32LE(16),
      local: buffer.subarray(offset, offset + length),
      central,
      dataStart,
      dataSize,
    });
  }
  return { entries, end };
}

function findEnd(buffer: Buffer): number {
  for (let i = buffer.length - 22; i >= Math.max(0, buffer.length - 22 - 0xffff); i--) {
    if (buffer.readUInt32LE(i) === END) return i;
  }
  throw new Error("Not a .drp or .drt: it isn't a ZIP file.");
}

/** The entry's uncompressed bytes, checked against its CRC. */
export function entryData(entry: ZipEntry): Buffer {
  const raw = entry.local.subarray(entry.dataStart, entry.dataStart + entry.dataSize);
  const data = entry.method === 8 ? zlib.inflateRawSync(raw) : Buffer.from(raw);
  if (zlib.crc32(data) >>> 0 !== entry.crc) throw new Error(`The file is damaged: ${entry.name} fails its CRC check.`);
  return data;
}

/** The archive with `changes` (entry name → new contents) applied, every other entry copied as it was. */
export function writeZip(zip: Zip, changes: ReadonlyMap<string, Buffer> = new Map()): Buffer {
  for (const name of changes.keys()) {
    if (!zip.entries.some((entry) => entry.name === name)) throw new Error(`The file has no entry ${name}.`);
  }
  const locals: Buffer[] = [];
  const centrals: Buffer[] = [];
  let offset = 0;
  for (const entry of zip.entries) {
    const data = changes.get(entry.name);
    const central = Buffer.from(entry.central);
    let local = entry.local;
    if (data) {
      const stored = entry.method === 8 ? zlib.deflateRawSync(data) : data;
      const crc = zlib.crc32(data) >>> 0;
      // Keep the original header (version, times, name, extra field) but put the sizes in it
      // instead of a trailing data descriptor.
      const header = Buffer.from(entry.local.subarray(0, entry.dataStart));
      const flags = header.readUInt16LE(6) & ~HAS_DESCRIPTOR;
      for (const [record, at] of [
        [header, 0],
        [central, 2],
      ] as const) {
        record.writeUInt16LE(flags, 6 + at);
        record.writeUInt32LE(crc, 14 + at);
        record.writeUInt32LE(stored.length, 18 + at);
        record.writeUInt32LE(data.length, 22 + at);
      }
      local = Buffer.concat([header, stored]);
    }
    central.writeUInt32LE(offset, 42);
    locals.push(local);
    centrals.push(central);
    offset += local.length;
  }
  const directory = Buffer.concat(centrals);
  const end = Buffer.from(zip.end);
  end.writeUInt32LE(directory.length, 12);
  end.writeUInt32LE(offset, 16);
  return Buffer.concat([...locals, directory, end]);
}
