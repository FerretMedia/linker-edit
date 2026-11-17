//! Just enough ZIP for Resolve's .drp/.drt: stored or deflated entries, no ZIP64, no encryption.
//! Writing copies every entry that wasn't changed byte for byte, so only edited entries differ.

use std::io::{Read, Write};

use flate2::Compression;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use indexmap::IndexMap;

use crate::{Error, Result, fail};

const LOCAL: u32 = 0x04034b50;
const CENTRAL: u32 = 0x02014b50;
const END: u32 = 0x06054b50;
const DESCRIPTOR: u32 = 0x08074b50;
const HAS_DESCRIPTOR: u16 = 0x0008;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZipEntry {
    pub name: String,
    pub method: u16,
    pub crc: u32,
    /// The entry as stored: local header, data and any data descriptor.
    pub local: Vec<u8>,
    /// Its central directory record.
    pub central: Vec<u8>,
    /// Where the compressed data sits inside `local`.
    pub data_start: usize,
    pub data_size: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Zip {
    pub entries: Vec<ZipEntry>,
    /// The end of central directory record, comment included.
    pub end: Vec<u8>,
}

fn cut_short() -> Error {
    Error("Not a .drp or .drt: the ZIP is cut short.".into())
}

fn u16_at(bytes: &[u8], at: usize) -> Result<u16> {
    let slice = bytes.get(at..at + 2).ok_or_else(cut_short)?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn u32_at(bytes: &[u8], at: usize) -> Result<u32> {
    let slice = bytes.get(at..at + 4).ok_or_else(cut_short)?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn put_u16(bytes: &mut [u8], at: usize, value: u16) {
    bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

/// `bytes[from..to]`, clamped to the buffer like Node's `subarray`.
fn clamped(bytes: &[u8], from: usize, to: usize) -> &[u8] {
    let to = to.min(bytes.len());
    &bytes[from.min(to)..to]
}

pub fn read_zip(buffer: &[u8]) -> Result<Zip> {
    let end_at = find_end(buffer)?;
    let end = &buffer[end_at..];
    let count = u16_at(end, 10)?;
    let start = u32_at(end, 16)?;
    if u16_at(end, 4)? != 0 || count != u16_at(end, 8)? || count == 0xffff || start == 0xffffffff {
        return fail("Not a .drp or .drt: the ZIP is split or ZIP64, which Resolve doesn't write.");
    }
    let mut at = start as usize;
    let mut entries = Vec::with_capacity(count as usize);
    for _ in 0..count {
        if u32_at(buffer, at)? != CENTRAL {
            return fail("Not a .drp or .drt: the ZIP's central directory is damaged.");
        }
        let name_length = u16_at(buffer, at + 28)? as usize;
        let central_length = 46
            + name_length
            + u16_at(buffer, at + 30)? as usize
            + u16_at(buffer, at + 32)? as usize;
        let central = buffer.get(at..at + central_length).ok_or_else(cut_short)?;
        at += central_length;

        let flags = u16_at(central, 8)?;
        let method = u16_at(central, 10)?;
        let data_size = u32_at(central, 20)?;
        let offset = u32_at(central, 42)?;
        let name = String::from_utf8_lossy(&central[46..46 + name_length]).into_owned();
        if flags & 0x0001 != 0 {
            return fail(format!("Not a .drp or .drt: {name} is encrypted."));
        }
        if method != 0 && method != 8 {
            return fail(format!(
                "Not a .drp or .drt: {name} uses ZIP method {method}."
            ));
        }
        if data_size == 0xffffffff || offset == 0xffffffff {
            return fail("Not a .drp or .drt: ZIP64 entries aren't supported.");
        }
        let (data_size, offset) = (data_size as usize, offset as usize);
        if u32_at(buffer, offset)? != LOCAL {
            return fail(format!("Not a .drp or .drt: {name} has no local header."));
        }

        let data_start =
            30 + u16_at(buffer, offset + 26)? as usize + u16_at(buffer, offset + 28)? as usize;
        let mut length = data_start + data_size;
        if flags & HAS_DESCRIPTOR != 0 {
            length += if u32_at(buffer, offset + length)? == DESCRIPTOR {
                16
            } else {
                12
            };
        }
        entries.push(ZipEntry {
            name,
            method,
            crc: u32_at(central, 16)?,
            local: clamped(buffer, offset, offset + length).to_vec(),
            central: central.to_vec(),
            data_start,
            data_size,
        });
    }
    Ok(Zip {
        entries,
        end: end.to_vec(),
    })
}

fn find_end(buffer: &[u8]) -> Result<usize> {
    if buffer.len() >= 22 {
        let last = buffer.len() - 22;
        for i in (last.saturating_sub(0xffff)..=last).rev() {
            if u32_at(buffer, i)? == END {
                return Ok(i);
            }
        }
    }
    fail("Not a .drp or .drt: it isn't a ZIP file.")
}

/// The entry's uncompressed bytes, checked against its CRC.
pub fn entry_data(entry: &ZipEntry) -> Result<Vec<u8>> {
    let raw = clamped(
        &entry.local,
        entry.data_start,
        entry.data_start + entry.data_size,
    );
    let data = if entry.method == 8 {
        let mut data = Vec::new();
        DeflateDecoder::new(raw)
            .read_to_end(&mut data)
            .map_err(|_| {
                Error(format!(
                    "The file is damaged: {} can't be decompressed.",
                    entry.name
                ))
            })?;
        data
    } else {
        raw.to_vec()
    };
    if crc32fast::hash(&data) != entry.crc {
        return fail(format!(
            "The file is damaged: {} fails its CRC check.",
            entry.name
        ));
    }
    Ok(data)
}

/// The archive with `changes` (entry name → new contents) applied, every other entry copied as it was.
pub fn write_zip(zip: &Zip, changes: &IndexMap<String, Vec<u8>>) -> Result<Vec<u8>> {
    for name in changes.keys() {
        if !zip.entries.iter().any(|entry| &entry.name == name) {
            return fail(format!("The file has no entry {name}."));
        }
    }
    let mut out = Vec::new();
    let mut directory = Vec::new();
    for entry in &zip.entries {
        let mut central = entry.central.clone();
        put_u32(&mut central, 42, out.len() as u32);
        if let Some(data) = changes.get(&entry.name) {
            let stored = if entry.method == 8 {
                deflate(data)
            } else {
                data.clone()
            };
            let crc = crc32fast::hash(data);
            // Keep the original header (version, times, name, extra field) but put the sizes in it
            // instead of a trailing data descriptor.
            let mut header = entry
                .local
                .get(..entry.data_start)
                .ok_or_else(cut_short)?
                .to_vec();
            let flags = u16_at(&header, 6)? & !HAS_DESCRIPTOR;
            for (record, at) in [(&mut header, 0), (&mut central, 2)] {
                put_u16(record, 6 + at, flags);
                put_u32(record, 14 + at, crc);
                put_u32(record, 18 + at, stored.len() as u32);
                put_u32(record, 22 + at, data.len() as u32);
            }
            out.extend_from_slice(&header);
            out.extend_from_slice(&stored);
        } else {
            out.extend_from_slice(&entry.local);
        }
        directory.extend_from_slice(&central);
    }
    let mut end = zip.end.clone();
    put_u32(&mut end, 12, directory.len() as u32);
    put_u32(&mut end, 16, out.len() as u32);
    out.extend_from_slice(&directory);
    out.extend_from_slice(&end);
    Ok(out)
}

fn deflate(data: &[u8]) -> Vec<u8> {
    // zlib's default level, like Node's deflateRawSync.
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::new(6));
    encoder
        .write_all(data)
        .expect("writing to a Vec can't fail");
    encoder.finish().expect("writing to a Vec can't fail")
}
