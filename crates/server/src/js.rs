// What the TypeScript server got from JavaScript and Node themselves, where Rust's own differs and a file, a cache
// key, a message or a tool's answer would come out different: JSON.stringify, Number(), String(number),
// Math.round, toFixed, slicing by UTF-16 units, localeCompare (ICU), Node's path functions, mtimeMs and
// toISOString. And promise chains: jobs run one at a time, in the order they were queued.

use std::cmp::Ordering;
use std::fs::Metadata;
use std::future::Future;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, LazyLock};
use std::time::UNIX_EPOCH;

use icu_collator::options::{CollatorOptions, Strength};
use icu_collator::preferences::CollationNumericOrdering;
use icu_collator::{Collator, CollatorBorrowed, CollatorPreferences};
use serde::{Serialize, Serializer};
use serde_json::Value;
use serde_json::ser::{CompactFormatter, Formatter, PrettyFormatter};
use tokio::sync::watch;
use tokio::task::JoinHandle;

// --- JSON ---

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

/// A number as JSON.stringify writes it, as a Value: 40 rather than 40.0, null for NaN and infinities.
pub fn number_value(value: f64) -> Value {
    if !value.is_finite() {
        Value::Null
    } else if value.fract() == 0.0 && value.abs() <= 9_007_199_254_740_991.0 {
        Value::from(value as i64)
    } else {
        Value::from(value)
    }
}

/// For `#[serde(serialize_with = "js::num")]` on an f64.
pub fn num<S: Serializer>(value: &f64, serializer: S) -> Result<S::Ok, S::Error> {
    number_value(*value).serialize(serializer)
}

/// For `#[serde(serialize_with = "js::opt_num")]` on an Option<f64>.
pub fn opt_num<S: Serializer>(value: &Option<f64>, serializer: S) -> Result<S::Ok, S::Error> {
    value
        .map(number_value)
        .unwrap_or(Value::Null)
        .serialize(serializer)
}

/// For `#[serde(serialize_with = "js::nums")]` on a list of f64.
pub fn nums<S: Serializer, T: AsRef<[f64]>>(value: &T, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.collect_seq(value.as_ref().iter().map(|value| number_value(*value)))
}

// --- Numbers ---

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

/// Number(text): 0 for nothing, NaN when it isn't a number.
pub fn parse_number(text: &str) -> f64 {
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

/// Number(value), with None for undefined: null is 0, a string is read as a number, anything else is NaN.
pub fn number(value: Option<&Value>) -> f64 {
    match value {
        None => f64::NAN,
        Some(Value::Null) => 0.0,
        Some(Value::Bool(value)) => f64::from(u8::from(*value)),
        Some(Value::Number(value)) => value.as_f64().unwrap_or(f64::NAN),
        Some(Value::String(text)) => parse_number(text),
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

/// String(value), with None for undefined: "null" for null, as a template literal writes it.
pub fn string(value: Option<&Value>) -> String {
    match value {
        None => "undefined".into(),
        Some(Value::Null) => "null".into(),
        Some(value) => to_string(value),
    }
}

/// A Value as JSON.parse would have made it: whole numbers are integers, so Python's 10.0 equals json!(10).
pub fn whole_numbers(value: Value) -> Value {
    match value {
        Value::Number(number) if number.is_f64() => {
            number.as_f64().map_or(Value::Number(number), number_value)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(whole_numbers).collect()),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| (key, whole_numbers(value)))
                .collect(),
        ),
        other => other,
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

/// Math.round: halves go up, also below zero (-2.5 is -2), unlike f64::round.
pub fn math_round(value: f64) -> f64 {
    let floor = value.floor();
    if value - floor >= 0.5 {
        floor + 1.0
    } else {
        floor
    }
}

/// `Math.round(value * 10 ** digits) / 10 ** digits`.
pub fn round_to(value: f64, digits: i32) -> f64 {
    let scale = 10f64.powi(digits);
    math_round(value * scale) / scale
}

/// Number.prototype.toFixed: an exact half goes away from zero, where Rust's formatting rounds it to even.
pub fn to_fixed(value: f64, digits: usize) -> String {
    let exact = format!("{:.40}", value.abs());
    let tie = exact
        .split_once('.')
        .map(|(_, fraction)| {
            let rest = &fraction[digits.min(fraction.len())..];
            rest.starts_with('5') && rest[1..].bytes().all(|byte| byte == b'0')
        })
        .unwrap_or(false);
    if !tie {
        return format!("{value:.digits$}");
    }
    let scale = 10f64.powi(digits as i32);
    let away = (value.abs() * scale).floor() + 1.0;
    let sign = if value < 0.0 { "-" } else { "" };
    format!("{sign}{:.digits$}", away / scale)
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

// --- Text ---

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

/// JavaScript's whitespace and line terminators (\s): Rust's char::is_whitespace also has U+0085 and lacks the
/// byte order mark.
pub fn is_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

/// text.trim().
pub fn trim(text: &str) -> &str {
    text.trim_matches(is_space)
}

/// text.trimEnd().
pub fn trim_end(text: &str) -> &str {
    text.trim_end_matches(is_space)
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

/// a.localeCompare(b, undefined, { numeric: true, sensitivity: "base" }): "Clip 9" before "Clip 10", Equal when
/// only case or accents differ.
pub fn locale_compare_numeric(a: &str, b: &str) -> Ordering {
    NUMERIC_BASE.compare(a, b)
}

// --- Dates ---

/// new Date().toISOString().
pub fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// new Date(ms).toISOString(): 2026-11-16T10:00:00.000Z.
pub fn iso_from_ms(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
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

// --- Paths and files ---

/// Node's path.isAbsolute: on Windows a path from the drive's root ("/Music", "\Music") counts too.
pub fn is_absolute(path: &str) -> bool {
    if cfg!(windows) {
        let bytes = path.as_bytes();
        path.starts_with(['/', '\\'])
            || (bytes.len() > 2
                && bytes[0].is_ascii_alphabetic()
                && bytes[1] == b':'
                && matches!(bytes[2], b'/' | b'\\'))
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
pub fn resolve(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    let absolute = if path.as_os_str().is_empty() {
        std::env::current_dir().unwrap_or_default()
    } else {
        std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
    };
    normalize(&absolute)
}

/// path.extname: from the last dot of the last part, "" when the name starts with its only dot or has none.
pub fn extname(file: &str) -> &str {
    let name = file
        .rsplit(['/', std::path::MAIN_SEPARATOR])
        .next()
        .unwrap_or(file);
    match name.rfind('.') {
        Some(index) if name[..index].chars().any(|c| c != '.') => &name[index..],
        _ => "",
    }
}

/// path.parse(file).name: the file's name without its extension.
pub fn parsed_name(file: &Path) -> String {
    let base = file
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = extname(&base).len();
    base[..base.len() - ext].to_string()
}

/// fs.Stats.mtimeMs: milliseconds since 1970, with the fraction Node gives.
pub fn mtime_ms(meta: &Metadata) -> f64 {
    let Ok(modified) = meta.modified() else {
        return 0.0;
    };
    match modified.duration_since(UNIX_EPOCH) {
        Ok(since) => since.as_secs() as f64 * 1e3 + f64::from(since.subsec_nanos()) / 1e6,
        Err(before) => {
            let before = before.duration();
            -(before.as_secs() as f64 * 1e3 + f64::from(before.subsec_nanos()) / 1e6)
        }
    }
}

// --- Child processes ---

/// Node's windowsHide: no console window for the child process on Windows.
pub fn hide_window(command: &mut tokio::process::Command) {
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    #[cfg(not(windows))]
    let _ = command;
}

// --- One at a time ---

/// Jobs that run one at a time, in the order they were queued: the TypeScript's `queue = queue.then(job)`. Each job
/// takes its place when queued and runs in a task of its own, so it runs whether or not anyone waits for it.
#[derive(Clone)]
pub struct Serial {
    next: Arc<AtomicU64>,
    serving: Arc<watch::Sender<u64>>,
}

impl Default for Serial {
    fn default() -> Self {
        Self {
            next: Arc::new(AtomicU64::new(0)),
            serving: Arc::new(watch::channel(0).0),
        }
    }
}

/// Lets the next job go when the running one ends, also when it panics.
struct Turn(Arc<watch::Sender<u64>>);

impl Drop for Turn {
    fn drop(&mut self) {
        self.0.send_modify(|serving| *serving += 1);
    }
}

impl Serial {
    /// Queues a job. Needs a tokio runtime. Aborting the returned handle before the job ran stalls the queue.
    pub fn spawn<F, T>(&self, job: F) -> JoinHandle<T>
    where
        F: Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        let ticket = self.next.fetch_add(1, AtomicOrdering::SeqCst);
        let mut serving = self.serving.subscribe();
        let sender = self.serving.clone();
        tokio::spawn(async move {
            let _ = serving.wait_for(|now| *now == ticket).await;
            let _turn = Turn(sender);
            job.await
        })
    }
}

/// The result of a job that several callers wait for, like one promise awaited twice.
#[derive(Clone)]
pub struct Shared<T: Clone> {
    result: watch::Receiver<Option<T>>,
}

impl<T: Clone + Send + Sync + 'static> Shared<T> {
    /// Runs the job in the queue and shares what it gives.
    pub fn queue<F>(serial: &Serial, job: F) -> Self
    where
        F: Future<Output = T> + Send + 'static,
    {
        let (sender, result) = watch::channel(None);
        serial.spawn(async move {
            let value = job.await;
            sender.send_replace(Some(value));
        });
        Self { result }
    }

    /// A result that is already there, or comes from elsewhere.
    pub fn from_receiver(result: watch::Receiver<Option<T>>) -> Self {
        Self { result }
    }

    /// What the job gave; None when it panicked.
    pub async fn get(&self) -> Option<T> {
        let mut result = self.result.clone();
        result
            .wait_for(Option::is_some)
            .await
            .ok()
            .and_then(|value| value.clone())
    }
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
            (-35.0, "-35"),
            (3.0, "3"),
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
        assert_eq!(number_value(30.0).to_string(), "30");
        assert_eq!(number_value(1.25).to_string(), "1.25");
        assert_eq!(number_value(f64::NAN), Value::Null);
        assert_eq!(
            whole_numbers(json!({ "a": [1.0, 2.5, 3] })).to_string(),
            r#"{"a":[1,2.5,3]}"#
        );
        assert_eq!(string(None), "undefined");
        assert_eq!(string(Some(&json!(null))), "null");
        assert_eq!(string(Some(&json!([1.0, null, "a"]))), "1,,a");
        assert_eq!(string(Some(&json!({}))), "[object Object]");
        // A file time read back from a cache is the same to the last bit (serde_json's float_roundtrip).
        let mtime = 1_794_962_131_877.482_7;
        assert_eq!(
            serde_json::from_str::<f64>(&number_value(mtime).to_string()).unwrap(),
            mtime
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
        assert_eq!(parse_number("5."), 5.0);
        assert_eq!(parse_number("1e3"), 1000.0);
        assert!(parse_number("1.2.3").is_nan());
    }

    #[test]
    fn rounds_like_javascript() {
        assert_eq!(math_round(-2.5), -2.0);
        assert_eq!(math_round(2.5), 3.0);
        assert_eq!(math_round(0.499_999_999_999_999_94), 0.0);
        assert_eq!(round_to(-14.25, 1), -14.2);
        assert_eq!(to_fixed(0.125, 2), "0.13");
        assert_eq!(to_fixed(-0.125, 2), "-0.13");
        assert_eq!(to_fixed(1.005, 2), "1.00");
        assert_eq!(to_fixed(0.5, 2), "0.50");
    }

    #[test]
    fn slices_by_utf16_units() {
        assert_eq!(slice("abc", 2), "ab");
        assert_eq!(slice("a🎬b", 2), "a");
        assert_eq!(slice("a🎬b", 3), "a🎬");
        assert_eq!(slice("ab\u{1F600}c", 3), "ab");
        assert_eq!(length("a🎬"), 3);
        assert_eq!(trim("\u{feff} a\u{3000}\n"), "a");
        assert_eq!(trim("\u{85}a"), "\u{85}a");
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
        let mut names = vec![
            "b.mp4",
            "Upbeat.wav",
            "a.MOV",
            "broken.mp4",
            "Calm",
            "_x",
            "10",
            "9",
        ];
        names.sort_by(|a, b| locale_compare(a, b));
        assert_eq!(
            names,
            [
                "_x",
                "10",
                "9",
                "a.MOV",
                "b.mp4",
                "broken.mp4",
                "Calm",
                "Upbeat.wav"
            ]
        );
        // Node's order for these, from ICU.
        let mut names = vec![
            "a.wav",
            "a-b.wav",
            "#1.wav",
            "a_b.wav",
            "(old).wav",
            "-dash.wav",
            "a b.wav",
            "~x.wav",
        ];
        names.sort_by(|a, b| locale_compare(a, b));
        assert_eq!(
            names,
            [
                "-dash.wav",
                "(old).wav",
                "#1.wav",
                "~x.wav",
                "a b.wav",
                "a_b.wav",
                "a-b.wav",
                "a.wav"
            ]
        );
        assert_eq!(locale_compare("a", "A"), Ordering::Less);
        assert_eq!(
            locale_compare_numeric("Whoosh 2", "whoosh 10"),
            Ordering::Less
        );
        assert_eq!(locale_compare_numeric("Café", "cafe"), Ordering::Equal);
    }

    #[test]
    fn handles_paths_like_node() {
        assert_eq!(
            join(Path::new("/home/me"), "Videos/../Music"),
            PathBuf::from(format!("{0}home{0}me{0}Music", std::path::MAIN_SEPARATOR))
        );
        assert!(is_absolute("/Music"));
        assert!(!is_absolute("Music/Song.wav"));
        assert_eq!(is_absolute(r"C:\Music"), cfg!(windows));
        assert!(!is_absolute("C:Music"));
        if cfg!(windows) {
            assert_eq!(
                resolve(Path::new("C:/a/b/../c/")),
                PathBuf::from("C:\\a\\c")
            );
        }
        let here = std::env::current_dir().unwrap();
        assert_eq!(resolve("a/./b/../c/"), here.join("a").join("c"));
        assert_eq!(extname("Song.MP3"), ".MP3");
        assert_eq!(extname(".hidden"), "");
        assert_eq!(extname("a.b/c"), "");
        assert_eq!(parsed_name(Path::new("dir/Reel.final.mp4")), "Reel.final");
        assert_eq!(iso_from_ms(0), "1970-01-01T00:00:00.000Z");
    }
}
