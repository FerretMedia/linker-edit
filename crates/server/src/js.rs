// What the TypeScript server got from JavaScript and Node themselves, for the ports that must write the same bytes
// (the edit history, the project and collection files) and the same messages: JSON.stringify, Number(),
// String(number), slicing by UTF-16 units, localeCompare and Node's path functions.

use std::cmp::Ordering;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

use icu_collator::options::{CollatorOptions, Strength};
use icu_collator::preferences::CollationNumericOrdering;
use icu_collator::{Collator, CollatorBorrowed, CollatorPreferences};
use serde::Serialize;
use serde_json::Value;
use serde_json::ser::{CompactFormatter, Formatter, PrettyFormatter};

/// Writes numbers as JavaScript does: 2 rather than 2.0, 1e+21 rather than 1e21.
struct JsNumbers<F>(F);

macro_rules! delegate {
    ($($name:ident($($arg:ident: $ty:ty),*);)*) => {
        $(fn $name<W: ?Sized + io::Write>(&mut self, writer: &mut W $(, $arg: $ty)*) -> io::Result<()> {
            self.0.$name(writer $(, $arg)*)
        })*
    };
}

impl<F: Formatter> Formatter for JsNumbers<F> {
    fn write_f64<W: ?Sized + io::Write>(&mut self, writer: &mut W, value: f64) -> io::Result<()> {
        writer.write_all(number_to_string(value).as_bytes())
    }

    fn write_f32<W: ?Sized + io::Write>(&mut self, writer: &mut W, value: f32) -> io::Result<()> {
        self.write_f64(writer, f64::from(value))
    }

    delegate! {
        begin_array();
        end_array();
        begin_array_value(first: bool);
        end_array_value();
        begin_object();
        end_object();
        begin_object_key(first: bool);
        end_object_key();
        begin_object_value();
        end_object_value();
    }
}

fn write<F: Formatter>(value: &impl Serialize, formatter: F) -> String {
    let mut out = Vec::new();
    let mut serializer = serde_json::Serializer::with_formatter(&mut out, JsNumbers(formatter));
    // Only maps with non-string keys fail, and none are written here.
    value
        .serialize(&mut serializer)
        .expect("a value that serializes to JSON");
    String::from_utf8(out).expect("JSON is UTF-8")
}

/// JSON.stringify(value).
pub fn stringify(value: &impl Serialize) -> String {
    write(value, CompactFormatter)
}

/// JSON.stringify(value, null, 2).
pub fn stringify_pretty(value: &impl Serialize) -> String {
    write(value, PrettyFormatter::with_indent(b"  "))
}

/// String(value) for a number, as ECMAScript's Number::toString writes it.
pub fn number_to_string(value: f64) -> String {
    if value.is_nan() {
        return "NaN".into();
    }
    if value == 0.0 {
        return "0".into();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    if value < 0.0 {
        return format!("-{}", number_to_string(-value));
    }
    // The shortest digits that read back as the same number, and where the point goes.
    let scientific = format!("{value:e}");
    let (mantissa, exponent) = scientific.split_once('e').expect("{:e} has an exponent");
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let k = digits.len() as i32;
    let n = exponent.parse::<i32>().expect("a whole exponent") + 1;
    if k <= n && n <= 21 {
        format!("{digits}{}", "0".repeat((n - k) as usize))
    } else if 0 < n && n <= 21 {
        format!("{}.{}", &digits[..n as usize], &digits[n as usize..])
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", "0".repeat((-n) as usize))
    } else {
        let sign = if n - 1 < 0 { '-' } else { '+' };
        let rest = if k > 1 {
            format!(".{}", &digits[1..])
        } else {
            String::new()
        };
        format!("{}{rest}e{sign}{}", &digits[..1], (n - 1).abs())
    }
}

/// Number(value), with None for undefined: null is 0, a string is read as a number, anything else is NaN.
pub fn number(value: Option<&Value>) -> f64 {
    match value {
        None => f64::NAN,
        Some(Value::Null) => 0.0,
        Some(Value::Bool(value)) => f64::from(u8::from(*value)),
        Some(Value::Number(value)) => value.as_f64().unwrap_or(f64::NAN),
        Some(Value::String(text)) => {
            let text = trim(text);
            if text.is_empty() {
                return 0.0;
            }
            match text {
                "Infinity" | "+Infinity" => f64::INFINITY,
                "-Infinity" => f64::NEG_INFINITY,
                // Rust reads "inf" and "nan", JavaScript doesn't.
                _ if text
                    .chars()
                    .any(|c| c.is_ascii_alphabetic() && !matches!(c, 'e' | 'E')) =>
                {
                    f64::NAN
                }
                _ => text.parse().unwrap_or(f64::NAN),
            }
        }
        Some(Value::Array(_) | Value::Object(_)) => f64::NAN,
    }
}

/// String(value) for what a Resolve script reports: "" for null.
pub fn to_string(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        Value::Number(_) => number_to_string(number(Some(value))),
        Value::Bool(value) => value.to_string(),
        Value::Array(items) => items.iter().map(to_string).collect::<Vec<_>>().join(","),
        Value::Object(_) => "[object Object]".into(),
    }
}

/// Boolean(value).
pub fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|n| n != 0.0 && !n.is_nan()),
        Value::String(text) => !text.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// Math.round, which rounds halves up: -2.5 is -2.
pub fn round(value: f64) -> f64 {
    let floor = value.floor();
    if value - floor >= 0.5 {
        floor + 1.0
    } else {
        floor
    }
}

/// text.length: UTF-16 code units.
pub fn length(text: &str) -> usize {
    text.encode_utf16().count()
}

/// text.slice(0, end), counted in UTF-16 code units. A character that would be cut in half is left out.
pub fn slice(text: &str, end: usize) -> &str {
    let mut units = 0;
    for (index, c) in text.char_indices() {
        units += c.len_utf16();
        if units > end {
            return &text[..index];
        }
    }
    text
}

/// text.trim(), which also takes off the byte order mark.
pub fn trim(text: &str) -> &str {
    text.trim_matches(|c: char| c.is_whitespace() || c == '\u{FEFF}')
}

/// n.toLocaleString("en-US") for a whole number: 20,000.
pub fn group_thousands(value: usize) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (index, c) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// new Date().toISOString().
pub fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Date.now().
pub fn now_millis() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// !Number.isNaN(Date.parse(text)) for the ISO 8601 forms Linker-Edit writes, and dates and times without a zone.
pub fn is_date(text: &str) -> bool {
    chrono::DateTime::parse_from_rfc3339(text).is_ok()
        || chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f").is_ok()
        || chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M").is_ok()
        || chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").is_ok()
}

fn collator(numeric: bool, strength: Option<Strength>) -> CollatorBorrowed<'static> {
    let mut prefs = CollatorPreferences::default();
    if numeric {
        prefs.numeric_ordering = Some(CollationNumericOrdering::True);
    }
    let mut options = CollatorOptions::default();
    options.strength = strength;
    Collator::try_new(prefs, options).expect("the collation data is compiled in")
}

static LOCALE_COMPARE: LazyLock<CollatorBorrowed<'static>> =
    LazyLock::new(|| collator(false, None));
static NUMERIC_BASE: LazyLock<CollatorBorrowed<'static>> =
    LazyLock::new(|| collator(true, Some(Strength::Primary)));

/// a.localeCompare(b) in Node with its default en-US locale: the CLDR root order.
pub fn locale_compare(a: &str, b: &str) -> Ordering {
    LOCALE_COMPARE.compare(a, b)
}

/// a.localeCompare(b, undefined, { numeric: true, sensitivity: "base" }): "Clip 9" before "Clip 10", case ignored.
pub fn locale_compare_numeric(a: &str, b: &str) -> Ordering {
    NUMERIC_BASE.compare(a, b)
}

/// Node's path.isAbsolute: on Windows also \\x and /x, which are on the current drive.
pub fn is_absolute(path: &str) -> bool {
    if cfg!(windows) {
        let bytes = path.as_bytes();
        let separator = |b: u8| b == b'/' || b == b'\\';
        bytes.first().is_some_and(|&b| separator(b))
            || (bytes.len() > 2
                && bytes[0].is_ascii_alphabetic()
                && bytes[1] == b':'
                && separator(bytes[2]))
    } else {
        path.starts_with('/')
    }
}

/// Node's path.normalize without the trailing separator: . and .. worked out, the platform's separators.
pub fn normalize(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    let text = if cfg!(windows) {
        text.replace('/', "\\")
    } else {
        text.into_owned()
    };
    let mut prefix = None;
    let mut root = false;
    let mut parts: Vec<String> = Vec::new();
    for component in Path::new(&text).components() {
        match component {
            Component::Prefix(value) => {
                prefix = Some(value.as_os_str().to_string_lossy().into_owned())
            }
            Component::RootDir => root = true,
            Component::CurDir => {}
            Component::ParentDir => {
                if parts.last().is_some_and(|last| last != "..") {
                    parts.pop();
                } else if !root {
                    parts.push("..".into());
                }
            }
            Component::Normal(value) => parts.push(value.to_string_lossy().into_owned()),
        }
    }
    let separator = std::path::MAIN_SEPARATOR_STR;
    let mut out = prefix.unwrap_or_default();
    if root {
        out.push_str(separator);
    }
    out.push_str(&parts.join(separator));
    if out.is_empty() {
        out.push('.');
    }
    PathBuf::from(out)
}

/// Node's path.join(a, b).
pub fn join(a: &Path, b: &str) -> PathBuf {
    normalize(&a.join(b))
}

/// Node's path.resolve(path): absolute, normalized, with no trailing separator.
pub fn resolve(path: &Path) -> PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    normalize(&absolute)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn writes_numbers_like_javascript() {
        for (value, text) in [
            (2.0, "2"),
            (1.5, "1.5"),
            (-0.5, "-0.5"),
            (0.1 + 0.2, "0.30000000000000004"),
            (1e21, "1e+21"),
            (123456789012345680000.0, "123456789012345680000"),
            (0.000001, "0.000001"),
            (1e-7, "1e-7"),
            (1.5e-10, "1.5e-10"),
            (23.976, "23.976"),
        ] {
            assert_eq!(number_to_string(value), text);
        }
        assert_eq!(
            stringify(&json!({ "a": [1.0, 2.5, null], "b": "é\n" })),
            r#"{"a":[1,2.5,null],"b":"é\n"}"#
        );
        assert_eq!(
            stringify_pretty(&json!({ "a": [], "b": { "c": 24.0 } })),
            "{\n  \"a\": [],\n  \"b\": {\n    \"c\": 24\n  }\n}"
        );
    }

    #[test]
    fn reads_numbers_like_javascript() {
        assert!(number(None).is_nan());
        assert_eq!(number(Some(&json!(null))), 0.0);
        assert_eq!(number(Some(&json!(" 23.976 "))), 23.976);
        assert_eq!(number(Some(&json!(""))), 0.0);
        assert!(number(Some(&json!("inf"))).is_nan());
        assert!(number(Some(&json!("24fps"))).is_nan());
        assert_eq!(number(Some(&json!(true))), 1.0);
        assert_eq!(round(-2.5), -2.0);
        assert_eq!(round(2.5), 3.0);
        assert_eq!(round(0.49999999999999994), 0.0);
    }

    #[test]
    fn slices_by_utf16_units() {
        assert_eq!(slice("abc", 2), "ab");
        assert_eq!(slice("a🎬b", 2), "a");
        assert_eq!(slice("a🎬b", 3), "a🎬");
        assert_eq!(length("a🎬"), 3);
        assert_eq!(group_thousands(20_000), "20,000");
        assert_eq!(group_thousands(999), "999");
        assert_eq!(group_thousands(1_234_567), "1,234,567");
    }

    #[test]
    fn compares_like_node() {
        let mut names = vec!["b", "B", "a", "_x", "10", "9", "Á"];
        names.sort_by(|a, b| locale_compare(a, b));
        assert_eq!(names, ["_x", "10", "9", "a", "Á", "b", "B"]);
        let mut clips = vec!["Clip 10", "clip 9", "b-roll", ".git"];
        clips.sort_by(|a, b| locale_compare_numeric(a, b));
        assert_eq!(clips, [".git", "b-roll", "clip 9", "Clip 10"]);
    }

    #[test]
    fn handles_paths_like_node() {
        assert_eq!(
            join(Path::new("/home/me"), "Videos/../Music"),
            PathBuf::from(format!("{0}home{0}me{0}Music", std::path::MAIN_SEPARATOR))
        );
        assert!(is_absolute("/x"));
        assert!(!is_absolute("x"));
        if cfg!(windows) {
            assert!(is_absolute("C:\\x"));
            assert!(!is_absolute("C:x"));
            assert_eq!(
                resolve(Path::new("C:/a/b/../c/")),
                PathBuf::from("C:\\a\\c")
            );
        }
    }
}
