//! Codecs for the hex blobs inside Resolve's project XML. The layouts come from reverse engineering
//! sample files, much of it first mapped by davinci-resolve-mcp's drp-format library (MIT, see NOTICE).

use std::sync::LazyLock;

use indexmap::IndexMap;
use regex::Regex;

use crate::{Error, Result, fail};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionStamp {
    /// The Resolve that wrote the file, like "21.0.4.0005".
    pub db_app_ver: String,
    /// The project format: 14 for Resolve 19.1, 17 for Resolve 21.
    pub db_prj_ver: u32,
}

static STAMP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"^\x{FEFF}?(?:<\?[^?]*\?>\s*)?<!--\s*DbAppVer="([^"]*)"\s+DbPrjVer="([0-9]+)"\s*-->"#,
    )
    .unwrap()
});

/// The `<!--DbAppVer="…" DbPrjVer="…"-->` comment at the top of each XML entry.
pub fn version_stamp(xml: &str) -> Option<VersionStamp> {
    let captures = STAMP.captures(xml)?;
    Some(VersionStamp {
        db_app_ver: captures[1].to_string(),
        db_prj_ver: captures[2].parse().ok()?,
    })
}

/// Bytes from hex the way Node's `Buffer.from(hex, "hex")` reads them: up to the first pair that isn't hex.
pub fn from_hex(hex: &str) -> Vec<u8> {
    hex.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map_while(|&[high, low]| {
            let digit = |byte: u8| (byte as char).to_digit(16);
            Some((digit(high)? * 16 + digit(low)?) as u8)
        })
        .collect()
}

pub fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn u32_be(bytes: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(bytes[at..at + 4].try_into().expect("four bytes"))
}

/// A sized blob, like `<Clip>` and version 2 `<FieldsBlob>`: `<u32 version> <u32 length> <marker> <payload>`,
/// big-endian, where length counts the marker and payload. Marker 0x80 is a raw payload, 0x81 a zstd frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SizedBlob {
    pub version: u32,
    pub compressed: bool,
    pub data: Vec<u8>,
}

pub fn decode_sized(hex: &str) -> Result<SizedBlob> {
    let bytes = from_hex(hex);
    if bytes.len() < 9
        || u32_be(&bytes, 4) as usize != bytes.len() - 8
        || (bytes[8] != 0x80 && bytes[8] != 0x81)
    {
        return fail("Not a sized blob.");
    }
    let compressed = bytes[8] == 0x81;
    let payload = &bytes[9..];
    let data = if compressed {
        zstd::decode_all(payload)
            .map_err(|error| Error(format!("The blob's zstd frame is damaged: {error}.")))?
    } else {
        payload.to_vec()
    };
    Ok(SizedBlob {
        version: u32_be(&bytes, 0),
        compressed,
        data,
    })
}

pub fn encode_sized(blob: &SizedBlob) -> String {
    // A one-shot frame with the content size in its header and no checksum, like the frames Resolve writes.
    let payload = if blob.compressed {
        zstd::bulk::compress(&blob.data, 3).expect("zstd compresses any input")
    } else {
        blob.data.clone()
    };
    let mut bytes = Vec::with_capacity(9 + payload.len());
    bytes.extend_from_slice(&blob.version.to_be_bytes());
    bytes.extend_from_slice(&(payload.len() as u32 + 1).to_be_bytes());
    bytes.push(if blob.compressed { 0x81 } else { 0x80 });
    bytes.extend_from_slice(&payload);
    to_hex(&bytes)
}

// Protobuf, top level only: enough to read and replace fields without knowing the schema.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtoField {
    pub field: u64,
    pub wire: u8,
    /// The whole field, tag included, to copy it unchanged.
    pub bytes: Vec<u8>,
    /// A length-delimited field's payload.
    pub payload: Option<Vec<u8>>,
    /// A varint field's value.
    pub varint: Option<u64>,
}

pub fn read_proto(buffer: &[u8]) -> Result<Vec<ProtoField>> {
    let not_proto = || Error("Not a protobuf message.".into());
    let mut fields = Vec::new();
    let mut at = 0usize;
    let varint = |at: &mut usize| -> Result<u64> {
        let mut value = 0u64;
        let mut shift = 0u32;
        loop {
            if *at >= buffer.len() || shift > 63 {
                return Err(not_proto());
            }
            let byte = buffer[*at];
            *at += 1;
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
            shift += 7;
        }
    };
    while at < buffer.len() {
        let start = at;
        let tag = varint(&mut at)?;
        let field = tag >> 3;
        let wire = (tag & 7) as u8;
        let mut payload = None;
        let mut value = None;
        match wire {
            0 => value = Some(varint(&mut at)?),
            1 => at += 8,
            5 => at += 4,
            2 => {
                let length = usize::try_from(varint(&mut at)?).map_err(|_| not_proto())?;
                let end = at.checked_add(length).ok_or_else(not_proto)?;
                payload = Some(buffer[at.min(buffer.len())..end.min(buffer.len())].to_vec());
                at = end;
            }
            _ => return fail(format!("Protobuf wire type {wire} isn't supported.")),
        }
        if at > buffer.len() || field == 0 {
            return Err(not_proto());
        }
        fields.push(ProtoField {
            field,
            wire,
            bytes: buffer[start..at].to_vec(),
            payload,
            varint: value,
        });
    }
    Ok(fields)
}

fn encode_varint(mut value: u64, out: &mut Vec<u8>) {
    while value > 0x7f {
        out.push((value & 0x7f) as u8 | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn proto_string(field: u64, value: &str) -> Vec<u8> {
    let mut out = Vec::new();
    encode_varint((field << 3) | 2, &mut out);
    encode_varint(value.len() as u64, &mut out);
    out.extend_from_slice(value.as_bytes());
    out
}

fn proto_text(fields: &[ProtoField], field: u64) -> String {
    fields
        .iter()
        .find(|candidate| candidate.field == field)
        .and_then(|found| found.payload.as_deref())
        .map(|payload| String::from_utf8_lossy(payload).into_owned())
        .unwrap_or_default()
}

/// The media file a media pool `<Clip>` blob names. The blob is a sized blob around a protobuf whose field 1 is
/// the folder and field 2 the file name.
pub fn clip_path(blob: &str) -> Result<String> {
    let fields = read_proto(&decode_sized(blob)?.data)?;
    let folder = proto_text(&fields, 1);
    let file = proto_text(&fields, 2);
    if folder.is_empty() {
        return Ok(file);
    }
    let separator = if folder.contains('\\') && !folder.contains('/') {
        "\\"
    } else {
        "/"
    };
    Ok(if folder.ends_with(separator) {
        folder + &file
    } else {
        folder + separator + &file
    })
}

/// The `<Clip>` blob pointing at `path` instead: fields 1 and 2 rewritten, every other field (codec, dates,
/// the file name repeated in field 6) copied as it was, and the blob re-compressed with its length updated.
pub fn with_clip_path(blob: &str, path: &str) -> Result<String> {
    let cut = path.rfind(['/', '\\']);
    let file = &path[cut.map_or(0, |cut| cut + 1)..];
    // Resolve stores the folder without a trailing separator; a root like "/" or "C:\" keeps its own.
    let folder = match cut {
        None => "",
        Some(cut) if path[..cut].is_empty() || path[..cut].ends_with(':') => &path[..=cut],
        Some(cut) => &path[..cut],
    };
    if file.is_empty() {
        return fail(format!("\"{path}\" doesn't name a file."));
    }

    let sized = decode_sized(blob)?;
    let fields = read_proto(&sized.data)?;
    let has = |field: u64| {
        fields
            .iter()
            .any(|candidate| candidate.field == field && candidate.wire == 2)
    };
    if !has(1) || !has(2) {
        return fail("The media's <Clip> blob has no folder and file name to rewrite.");
    }
    let data = fields
        .iter()
        .flat_map(|field| match field.field {
            1 => proto_string(1, folder),
            2 => proto_string(2, file),
            _ => field.bytes.clone(),
        })
        .collect();
    Ok(encode_sized(&SizedBlob { data, ..sized }))
}

/// A value in a keyed dictionary, by its type byte.
#[derive(Debug, Clone, PartialEq)]
pub enum KeyedValue {
    String(String),
    Bytes(Vec<u8>),
    Null,
    Bool(bool),
    Int32(i32),
    UInt32(u32),
    Int64(i64),
    Double(f64),
}

/// A keyed dictionary, like the media pool's `<Time>` and `<Geometry>`: `<u32 1> <u32 count>`, then per entry
/// `<u32 key bytes> <UTF-16BE key> <u32 type> <u8 subtype> <value>`, all big-endian.
pub fn read_keyed_dict(hex: &str) -> Result<IndexMap<String, KeyedValue>> {
    let bytes = from_hex(hex);
    if bytes.len() < 8 || u32_be(&bytes, 0) != 1 {
        return fail("Not a keyed dictionary.");
    }
    let mut at = 8usize;
    let mut take = |length: usize| -> Result<&[u8]> {
        if at + length > bytes.len() {
            return fail("The keyed dictionary is cut short.");
        }
        at += length;
        Ok(&bytes[at - length..at])
    };
    fn u32_of(slice: &[u8]) -> u32 {
        u32_be(slice, 0)
    }
    fn utf16(slice: &[u8]) -> Result<String> {
        if !slice.len().is_multiple_of(2) {
            return fail("The keyed dictionary has a broken string.");
        }
        let units: Vec<u16> = slice
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&pair| u16::from_be_bytes(pair))
            .collect();
        Ok(String::from_utf16_lossy(&units))
    }

    let mut entries = IndexMap::new();
    for _ in 0..u32_be(&bytes, 4) {
        let key_length = u32_of(take(4)?) as usize;
        let key = utf16(take(key_length)?)?;
        let kind = u32_of(take(4)?);
        take(1)?; // subtype
        let value = match kind {
            0x0a => {
                let length = u32_of(take(4)?) as usize;
                KeyedValue::String(utf16(take(length)?)?)
            }
            0x0c => match u32_of(take(4)?) {
                0xffffffff => KeyedValue::Null,
                length => KeyedValue::Bytes(take(length as usize)?.to_vec()),
            },
            0x01 => KeyedValue::Bool(take(1)?[0] != 0),
            0x02 => KeyedValue::Int32(i32::from_be_bytes(take(4)?.try_into().expect("four bytes"))),
            0x03 => KeyedValue::UInt32(u32_of(take(4)?)),
            0x04 | 0x05 => KeyedValue::Int64(i64::from_be_bytes(
                take(8)?.try_into().expect("eight bytes"),
            )),
            0x06 => KeyedValue::Double(f64::from_be_bytes(
                take(8)?.try_into().expect("eight bytes"),
            )),
            0x10 => KeyedValue::Bytes(take(9)?.to_vec()),
            _ => {
                return fail(format!(
                    "Keyed dictionary value type {kind} isn't supported."
                ));
            }
        };
        entries.insert(key, value);
    }
    Ok(entries)
}

/// `<FrameRate>` and `<MediaFrameRate>`: a little-endian double, then 8 zero bytes.
pub fn frame_rate(bytes: &[u8]) -> Option<f64> {
    (bytes.len() == 16).then(|| f64::from_le_bytes(bytes[0..8].try_into().expect("eight bytes")))
}

/// `<Resolution>`: width and height as big-endian u32s at bytes 4 and 12.
pub fn resolution(bytes: &[u8]) -> Option<(u32, u32)> {
    (bytes.len() == 16).then(|| (u32_be(bytes, 4), u32_be(bytes, 12)))
}

/// `<MediaExtents>`: start and duration in seconds, little-endian doubles.
pub fn extents(bytes: &[u8]) -> Option<(f64, f64)> {
    (bytes.len() == 16).then(|| {
        let double =
            |at: usize| f64::from_le_bytes(bytes[at..at + 8].try_into().expect("eight bytes"));
        (double(0), double(8))
    })
}

static IN_POINT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(-?[0-9]+)\|([0-9a-fA-F]{16})$").unwrap());

/// A timeline clip's `<In>`: the source in point in timeline frames, as `<frames>|<hex little-endian double of
/// frames / 1000>`. Empty means 0.
pub fn decode_in(value: &str) -> Result<i64> {
    if value.is_empty() {
        return Ok(0);
    }
    IN_POINT
        .captures(value)
        .and_then(|captures| captures[1].parse().ok())
        .ok_or_else(|| Error(format!("Unknown <In> value \"{value}\".")))
}

pub fn encode_in(frames: i64) -> String {
    if frames == 0 {
        return String::new();
    }
    format!(
        "{frames}|{}",
        to_hex(&(frames as f64 / 1000.0).to_le_bytes())
    )
}
