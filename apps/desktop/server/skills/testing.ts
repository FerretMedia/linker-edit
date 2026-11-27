import { crc32, deflateRawSync } from "node:zlib";

/** A zip as zip tools write one; `stored` leaves the files uncompressed. */
export function zip(files: Record<string, string | Uint8Array>, options: { stored?: boolean; claimedSize?: number } = {}): Uint8Array {
  const locals: Buffer[] = [];
  const entries: Buffer[] = [];
  let offset = 0;
  for (const [path, content] of Object.entries(files)) {
    const data = Buffer.from(content);
    const packed = options.stored ? data : deflateRawSync(data);
    const name = Buffer.from(path);
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(options.stored ? 0 : 8, 8);
    local.writeUInt32LE(crc32(data), 14);
    local.writeUInt32LE(packed.length, 18);
    local.writeUInt32LE(data.length, 22);
    local.writeUInt16LE(name.length, 26);
    const entry = Buffer.alloc(46);
    entry.writeUInt32LE(0x02014b50, 0);
    entry.writeUInt16LE(options.stored ? 0 : 8, 10);
    entry.writeUInt32LE(crc32(data), 16);
    entry.writeUInt32LE(packed.length, 20);
    entry.writeUInt32LE(options.claimedSize ?? data.length, 24);
    entry.writeUInt16LE(name.length, 28);
    entry.writeUInt32LE(offset, 42);
    locals.push(local, name, packed);
    entries.push(entry, name);
    offset += local.length + name.length + packed.length;
  }
  const directory = Buffer.concat(entries);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(Object.keys(files).length, 8);
  end.writeUInt16LE(Object.keys(files).length, 10);
  end.writeUInt32LE(directory.length, 12);
  end.writeUInt32LE(offset, 16);
  return new Uint8Array(Buffer.concat([...locals, directory, end]));
}
