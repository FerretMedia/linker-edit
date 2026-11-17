// Read-only .drp/.drt inspector. No dependencies (Node >= 23.8 for zstd).
// Usage: node drp-dump.mjs <file.drp|file.drt> [--blobs]
import fs from 'node:fs';
import zlib from 'node:zlib';

const file = process.argv[2];
const showBlobs = process.argv.includes('--blobs');
const buf = fs.readFileSync(file);

// ---- minimal zip reader (central directory) ----
function readZip(b) {
  let eocd = -1;
  for (let i = b.length - 22; i >= Math.max(0, b.length - 70000); i--) {
    if (b.readUInt32LE(i) === 0x06054b50) { eocd = i; break; }
  }
  if (eocd < 0) throw new Error('not a zip');
  const n = b.readUInt16LE(eocd + 10);
  let p = b.readUInt32LE(eocd + 16);
  const out = {};
  for (let k = 0; k < n; k++) {
    const method = b.readUInt16LE(p + 10), crc = b.readUInt32LE(p + 16);
    const csize = b.readUInt32LE(p + 20), nlen = b.readUInt16LE(p + 28);
    const elen = b.readUInt16LE(p + 30), clen = b.readUInt16LE(p + 32);
    const off = b.readUInt32LE(p + 42);
    const name = b.toString('utf8', p + 46, p + 46 + nlen);
    p += 46 + nlen + elen + clen;
    if (name.endsWith('/')) continue;
    const ds = off + 30 + b.readUInt16LE(off + 26) + b.readUInt16LE(off + 28);
    const raw = b.subarray(ds, ds + csize);
    const data = method === 8 ? zlib.inflateRawSync(raw) : Buffer.from(raw);
    const ok = zlib.crc32 ? zlib.crc32(data) === crc : 'n/a';
    out[name] = { data: data.toString('utf8'), method, crcOk: ok };
  }
  return out;
}

// ---- blob decoding ----
// "00000002 <u32 rawLen> 81 <zstd frame>"  or bare "81 <zstd>"  or "<u32 len> 789c <zlib>"
function decodeBlob(hex) {
  const b = Buffer.from(hex, 'hex');
  try {
    if (b.length > 9 && b.readUInt32BE(0) === 2 && b[8] === 0x81) return zlib.zstdDecompressSync(b.subarray(9));
    if (b[0] === 0x81 && b.readUInt32LE(1) === 0xfd2fb528) return zlib.zstdDecompressSync(b.subarray(1));
    if (b.length > 6 && b[4] === 0x78) return zlib.inflateSync(b.subarray(4));
  } catch (e) { return null; }
  return b; // uncompressed keyed dict (00000001 <count> ...) or raw
}
// Printable runs: UTF-16BE (keyed-dict keys) and ASCII/UTF-8 (protobuf strings, paths)
function strings(b, min = 4) {
  const s = new Set();
  const u16 = b.toString('utf16le', 1).replace(/[^\x20-\x7e]+/g, '\n'); // BE via offset 1 approx
  const be = Buffer.from(b); be.swap16 && (be.length % 2 === 0) && be.swap16();
  for (const t of [be.toString('utf16le'), b.toString('latin1')]) {
    for (const m of t.matchAll(/[\x20-\x7e]{4,}/g)) if (m[0].length >= min) s.add(m[0]);
  }
  return [...s];
}

const tag = (x, t) => (x.match(new RegExp(`<${t}>([^<]*)</${t}>`)) || [])[1];
const all = (x, re) => [...x.matchAll(re)];
const leDouble = (hex, i = 0) => Buffer.from(hex, 'hex').readDoubleLE(i);

const z = readZip(buf);
console.log(`# ${file}`);
for (const [n, e] of Object.entries(z)) console.log(`  entry ${n}  ${e.data.length}B  method=${e.method} crcOk=${e.crcOk}`);
const proj = z['project.xml'].data;
console.log('header:', (proj.match(/<!--(.*?)-->/) || [])[1], ' ProjectName:', tag(proj, 'ProjectName'), ' ProjectVersion:', tag(proj, 'ProjectVersion'));

// Media pool
const mp = Object.entries(z).filter(([n]) => n.startsWith('MediaPool/'));
for (const [n, { data }] of mp) {
  console.log(`\n## ${n}`);
  for (const m of all(data, /<(Sm2Mp\w+) DbId="([^"]+)">\s*<FieldsBlob[^>]*>[^<]*<\/FieldsBlob>\s*<Name>([^<]*)<\/Name>/g)) {
    const [, kind, id, name] = m;
    const start = m.index, end = data.indexOf(`</${kind}>`, start);
    const chunk = data.slice(start, end);
    const clip = tag(chunk, 'Clip');
    let path = '';
    if (clip) {
      // Clip blob = protobuf: field 1 = directory, field 2 = file name (length-prefixed strings)
      const d = decodeBlob(clip);
      if (!d) path = '(undecodable)';
      else {
        const p = [];
        for (let i = 0; i < d.length && p.length < 2; ) { // walk top-level fields 1 and 2
          const key = d[i], len = d[i + 1];
          if ((key === 0x0a || key === 0x12) && len < 0x80) { p.push(d.toString('utf8', i + 2, i + 2 + len)); i += 2 + len; } else i++;
        }
        path = p.join(p[0] && p[0].includes('\\') ? '\\' : '/');
      }
    }
    const seq = kind === 'Sm2MpFolder' ? null : (chunk.match(/<Sm2Sequence DbId="([^"]+)"/) || [])[1];
    const fr = tag(chunk, 'FrameRate'), res = tag(chunk, 'Resolution');
    const extra = seq ? ` seq=${seq} fps=${fr ? leDouble(fr) : '?'} res=${res ? `${parseInt(res.slice(0, 16), 16)}x${parseInt(res.slice(16), 16)}` : '?'}` : '';
    console.log(`  ${kind.padEnd(20)} ${id}  "${name}"  ${path}${extra}`);
  }
}

// Timelines (SeqContainer/*.xml)
for (const [n, { data }] of Object.entries(z).filter(([n]) => n.startsWith('SeqContainer/'))) {
  console.log(`\n## ${n}`);
  for (const t of all(data, /<Sm2TiTrack DbId="([^"]+)">([\s\S]*?)<\/Sm2TiTrack>/g)) {
    const body = t[2];
    console.log(`  track type=${tag(body, 'Type')} seq=${tag(body, 'Sequence')} name="${tag(body, 'UserDefinedName') || ''}"`);
    for (const c of all(body, /<(Sm2Ti\w+) DbId="([^"]+)">([\s\S]*?)<\/\1>/g)) {
      const x = c[3];
      const fps = tag(x, 'MediaFrameRate');
      console.log(`    ${c[1].padEnd(18)} "${tag(x, 'Name')}" start=${tag(x, 'Start')} dur=${tag(x, 'Duration')} in=${tag(x, 'In') || ''} mediaRef=${tag(x, 'MediaRef') || ''} fps=${fps ? leDouble(fps) : ''} path=${tag(x, 'MediaFilePath') || ''} fx=${(tag(x, 'EffectFiltersBA') || '').length / 2}B`);
    }
  }
}

if (showBlobs) {
  console.log('\n## blob samples');
  const seen = new Set();
  for (const [n, { data }] of Object.entries(z)) {
    for (const m of all(data, /<(\w+)>([0-9a-f]{40,})<\/\1>/g)) {
      if (seen.has(m[1])) continue; seen.add(m[1]);
      const d = decodeBlob(m[2]);
      console.log(`  <${m[1]}> in ${n}: ${m[2].length / 2}B hdr=${m[2].slice(0, 20)} -> ${d ? d.length + 'B' : 'fail'}: ${d ? strings(d).slice(0, 8).join(' | ').slice(0, 220) : ''}`);
    }
  }
}
