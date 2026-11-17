import zlib from "node:zlib";

// Codecs for the hex blobs inside Resolve's project XML. The layouts come from reverse engineering
// sample files, much of it first mapped by davinci-resolve-mcp's drp-format library (MIT, see NOTICE).

export interface VersionStamp {
  /** The Resolve that wrote the file, like "21.0.4.0005". */
  dbAppVer: string;
  /** The project format: 14 for Resolve 19.1, 17 for Resolve 21. */
  dbPrjVer: number;
}

/** The `<!--DbAppVer="…" DbPrjVer="…"-->` comment at the top of each XML entry. */
export function versionStamp(xml: string): VersionStamp | null {
  const match = /^\uFEFF?(?:<\?[^?]*\?>\s*)?<!--\s*DbAppVer="([^"]*)"\s+DbPrjVer="(\d+)"\s*-->/.exec(xml);
  return match ? { dbAppVer: match[1]!, dbPrjVer: Number(match[2]) } : null;
}

/**
 * A sized blob, like `<Clip>` and version 2 `<FieldsBlob>`: `<u32 version> <u32 length> <marker> <payload>`,
 * big-endian, where length counts the marker and payload. Marker 0x80 is a raw payload, 0x81 a zstd frame.
 */
export interface SizedBlob {
  version: number;
  compressed: boolean;
  data: Buffer;
}

export function decodeSized(hex: string): SizedBlob {
  const bytes = Buffer.from(hex, "hex");
  if (bytes.length < 9 || bytes.readUInt32BE(4) !== bytes.length - 8 || (bytes[8] !== 0x80 && bytes[8] !== 0x81)) {
    throw new Error("Not a sized blob.");
  }
  const compressed = bytes[8] === 0x81;
  const payload = bytes.subarray(9);
  return { version: bytes.readUInt32BE(0), compressed, data: compressed ? zlib.zstdDecompressSync(payload) : payload };
}

export function encodeSized(blob: SizedBlob): string {
  // A one-shot frame with the content size in its header and no checksum, like the frames Resolve writes.
  const payload = blob.compressed ? zlib.zstdCompressSync(blob.data, { pledgedSrcSize: blob.data.length }) : blob.data;
  const header = Buffer.alloc(9);
  header.writeUInt32BE(blob.version, 0);
  header.writeUInt32BE(payload.length + 1, 4);
  header[8] = blob.compressed ? 0x81 : 0x80;
  return Buffer.concat([header, payload]).toString("hex");
}

// Protobuf, top level only: enough to read and replace fields without knowing the schema.

export interface ProtoField {
  field: number;
  wire: number;
  /** The whole field, tag included, to copy it unchanged. */
  bytes: Buffer;
  /** A length-delimited field's payload. */
  payload: Buffer | null;
  /** A varint field's value. */
  varint: bigint | null;
}

export function readProto(buffer: Buffer): ProtoField[] {
  const fields: ProtoField[] = [];
  let at = 0;
  const varint = () => {
    let value = 0n;
    for (let shift = 0n; ; shift += 7n) {
      if (at >= buffer.length || shift > 63n) throw new Error("Not a protobuf message.");
      const byte = buffer[at++]!;
      value |= BigInt(byte & 0x7f) << shift;
      if (!(byte & 0x80)) return value;
    }
  };
  while (at < buffer.length) {
    const start = at;
    const tag = varint();
    const field = Number(tag >> 3n);
    const wire = Number(tag & 7n);
    let payload: Buffer | null = null;
    let value: bigint | null = null;
    if (wire === 0) value = varint();
    else if (wire === 1) at += 8;
    else if (wire === 5) at += 4;
    else if (wire === 2) {
      const length = Number(varint());
      payload = buffer.subarray(at, at + length);
      at += length;
    } else throw new Error(`Protobuf wire type ${wire} isn't supported.`);
    if (at > buffer.length || field === 0) throw new Error("Not a protobuf message.");
    fields.push({ field, wire, bytes: buffer.subarray(start, at), payload, varint: value });
  }
  return fields;
}

function encodeVarint(value: bigint): Buffer {
  const bytes: number[] = [];
  let rest = value;
  while (rest > 0x7fn) {
    bytes.push(Number(rest & 0x7fn) | 0x80);
    rest >>= 7n;
  }
  bytes.push(Number(rest));
  return Buffer.from(bytes);
}

function protoString(field: number, value: string): Buffer {
  const payload = Buffer.from(value, "utf8");
  return Buffer.concat([encodeVarint(BigInt((field << 3) | 2)), encodeVarint(BigInt(payload.length)), payload]);
}

/**
 * The media file a media pool `<Clip>` blob names. The blob is a sized blob around a protobuf whose field 1 is
 * the folder and field 2 the file name.
 */
export function clipPath(blob: string): string {
  const fields = readProto(decodeSized(blob).data);
  const folder = fields.find((field) => field.field === 1)?.payload?.toString("utf8") ?? "";
  const file = fields.find((field) => field.field === 2)?.payload?.toString("utf8") ?? "";
  if (!folder) return file;
  const separator = folder.includes("\\") && !folder.includes("/") ? "\\" : "/";
  return folder.endsWith(separator) ? folder + file : folder + separator + file;
}

/**
 * The `<Clip>` blob pointing at `path` instead: fields 1 and 2 rewritten, every other field (codec, dates,
 * the file name repeated in field 6) copied as it was, and the blob re-compressed with its length updated.
 */
export function withClipPath(blob: string, path: string): string {
  const cut = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  const file = path.slice(cut + 1);
  // Resolve stores the folder without a trailing separator; a root like "/" or "C:\" keeps its own.
  let folder = cut < 0 ? "" : path.slice(0, cut);
  if (cut >= 0 && (folder === "" || folder.endsWith(":"))) folder = path.slice(0, cut + 1);
  if (!file) throw new Error(`"${path}" doesn't name a file.`);

  const sized = decodeSized(blob);
  const fields = readProto(sized.data);
  if (!fields.some((field) => field.field === 1 && field.wire === 2) || !fields.some((field) => field.field === 2 && field.wire === 2)) {
    throw new Error("The media's <Clip> blob has no folder and file name to rewrite.");
  }
  const data = Buffer.concat(
    fields.map((field) => (field.field === 1 ? protoString(1, folder) : field.field === 2 ? protoString(2, file) : field.bytes)),
  );
  return encodeSized({ ...sized, data });
}

/**
 * A keyed dictionary, like the media pool's `<Time>` and `<Geometry>`: `<u32 1> <u32 count>`, then per entry
 * `<u32 key bytes> <UTF-16BE key> <u32 type> <u8 subtype> <value>`, all big-endian.
 */
export type KeyedValue = string | number | bigint | boolean | Buffer | null;

export function readKeyedDict(hex: string): Map<string, KeyedValue> {
  const bytes = Buffer.from(hex, "hex");
  if (bytes.length < 8 || bytes.readUInt32BE(0) !== 1) throw new Error("Not a keyed dictionary.");
  let at = 8;
  const take = (length: number) => {
    if (at + length > bytes.length) throw new Error("The keyed dictionary is cut short.");
    at += length;
    return bytes.subarray(at - length, at);
  };
  const u32 = () => take(4).readUInt32BE(0);
  const utf16 = () => Buffer.from(take(u32())).swap16().toString("utf16le");

  const entries = new Map<string, KeyedValue>();
  for (let count = bytes.readUInt32BE(4); count > 0; count--) {
    const key = utf16();
    const type = u32();
    take(1); // subtype
    if (type === 0x0a) entries.set(key, utf16());
    else if (type === 0x0c) {
      const length = u32();
      entries.set(key, length === 0xffffffff ? null : take(length));
    } else if (type === 0x01) entries.set(key, take(1)[0] !== 0);
    else if (type === 0x02) entries.set(key, take(4).readInt32BE(0));
    else if (type === 0x03) entries.set(key, take(4).readUInt32BE(0));
    else if (type === 0x04 || type === 0x05) entries.set(key, take(8).readBigInt64BE(0));
    else if (type === 0x06) entries.set(key, take(8).readDoubleBE(0));
    else if (type === 0x10) entries.set(key, take(9));
    else throw new Error(`Keyed dictionary value type ${type} isn't supported.`);
  }
  return entries;
}

/** `<FrameRate>` and `<MediaFrameRate>`: a little-endian double, then 8 zero bytes. */
export function frameRate(bytes: Buffer): number | null {
  return bytes.length === 16 ? bytes.readDoubleLE(0) : null;
}

/** `<Resolution>`: width and height as big-endian u32s at bytes 4 and 12. */
export function resolution(bytes: Buffer): { width: number; height: number } | null {
  return bytes.length === 16 ? { width: bytes.readUInt32BE(4), height: bytes.readUInt32BE(12) } : null;
}

/** `<MediaExtents>`: start and duration in seconds, little-endian doubles. */
export function extents(bytes: Buffer): { start: number; duration: number } | null {
  return bytes.length === 16 ? { start: bytes.readDoubleLE(0), duration: bytes.readDoubleLE(8) } : null;
}

/**
 * A timeline clip's `<In>`: the source in point in timeline frames, as `<frames>|<hex little-endian double of
 * frames / 1000>`. Empty means 0.
 */
export function decodeIn(value: string): number {
  if (value === "") return 0;
  const match = /^(-?\d+)\|([0-9a-f]{16})$/i.exec(value);
  if (!match) throw new Error(`Unknown <In> value "${value}".`);
  return Number(match[1]);
}

export function encodeIn(frames: number): string {
  if (frames === 0) return "";
  const double = Buffer.alloc(8);
  double.writeDoubleLE(frames / 1000);
  return `${frames}|${double.toString("hex")}`;
}
