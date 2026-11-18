// What the TypeScript server got from JavaScript itself, where Rust's own differs and a file, a cache key or a
// tool's answer would come out different: Math.round, toFixed, numbers in JSON, path.resolve, localeCompare,
// mtimeMs, toISOString. And promise chains: jobs run one at a time, in the order they were queued.

use std::cmp::Ordering;
use std::fs::Metadata;
use std::future::Future;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::time::UNIX_EPOCH;

use serde::{Serialize, Serializer};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::watch;
use tokio::task::JoinHandle;

// --- Numbers ---

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

/// Number(text) for the numbers a log shows: NaN when it isn't one, 0 for nothing.
pub fn parse_number(text: &str) -> f64 {
    let text = trim(text);
    if text.is_empty() {
        return 0.0;
    }
    if text.bytes().any(|byte| byte.is_ascii_alphabetic()) {
        return f64::NAN;
    }
    text.parse().unwrap_or(f64::NAN)
}

/// A number as JSON.stringify writes it: 40 rather than 40.0, null for NaN and infinities.
pub fn number(value: f64) -> Value {
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
    number(*value).serialize(serializer)
}

/// For `#[serde(serialize_with = "js::opt_num")]` on an Option<f64>.
pub fn opt_num<S: Serializer>(value: &Option<f64>, serializer: S) -> Result<S::Ok, S::Error> {
    value
        .map(number)
        .unwrap_or(Value::Null)
        .serialize(serializer)
}

/// For `#[serde(serialize_with = "js::nums")]` on a list of f64.
pub fn nums<S: Serializer, T: AsRef<[f64]>>(value: &T, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.collect_seq(value.as_ref().iter().map(|value| number(*value)))
}

// --- Text ---

/// String.prototype.trim, which also takes the byte order mark.
pub fn trim(text: &str) -> &str {
    text.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
}

/// `text.slice(0, max)`: max counts UTF-16 units. A character split in two is left out whole.
pub fn slice(text: &str, max: usize) -> &str {
    let mut units = 0;
    for (index, c) in text.char_indices() {
        units += c.len_utf16();
        if units > max {
            return &text[..index];
        }
    }
    text
}

/// sha256 of the text, in hex.
pub fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// `name.localeCompare(other)`, close to ICU's root collation: spaces and punctuation before digits before letters,
/// then accents, then lower case before upper case.
pub fn locale_compare(a: &str, b: &str) -> Ordering {
    collate(a, b, false).then_with(|| tie_break(a, b))
}

/// `a.localeCompare(b, undefined, { numeric: true, sensitivity: "base" })`: numbers by their value, and Equal
/// when only case or accents differ.
pub fn locale_compare_numeric_base(a: &str, b: &str) -> Ordering {
    collate(a, b, true)
}

/// The kind first (space, punctuation, digits, letters), then the character or number.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Element(u8, Key);

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Char(u32),
    Number(String),
}

// ICU's order of the ASCII punctuation and symbols; others go after them, by code point.
const PUNCTUATION: &str = r#"_-,;:!?.'"()[]{}@*/\&#%`^+<=>|~$"#;

fn punctuation_weight(c: char) -> u32 {
    PUNCTUATION
        .find(c)
        .map_or(PUNCTUATION.len() as u32 + u32::from(c), |index| {
            index as u32
        })
}

fn elements(text: &str, numeric: bool) -> Vec<Element> {
    use unicode_normalization::UnicodeNormalization;
    use unicode_normalization::char::is_combining_mark;
    let mut out = Vec::new();
    let mut chars = text.nfd().filter(|c| !is_combining_mark(*c)).peekable();
    while let Some(c) = chars.next() {
        if numeric && c.is_ascii_digit() {
            let mut digits = String::from(c);
            while let Some(next) = chars.next_if(char::is_ascii_digit) {
                digits.push(next);
            }
            let value = digits.trim_start_matches('0');
            // Longer numbers are bigger: pad so the text compares as the number.
            out.push(Element(2, Key::Number(format!("{value:0>40}"))));
            continue;
        }
        let class = if c.is_whitespace() {
            0
        } else if c.is_numeric() {
            2
        } else if c.is_alphabetic() {
            3
        } else {
            1
        };
        let key = if class == 1 {
            punctuation_weight(c)
        } else {
            u32::from(c.to_lowercase().next().unwrap_or(c))
        };
        out.push(Element(class, Key::Char(key)));
    }
    out
}

fn collate(a: &str, b: &str, numeric: bool) -> Ordering {
    elements(a, numeric).cmp(&elements(b, numeric))
}

/// What ICU looks at after the letters: accents, then case (lower first), then the code points.
fn tie_break(a: &str, b: &str) -> Ordering {
    use unicode_normalization::UnicodeNormalization;
    let accents = |text: &str| -> Vec<char> {
        text.nfd()
            .filter(|c| unicode_normalization::char::is_combining_mark(*c))
            .collect()
    };
    let case = |text: &str| -> Vec<bool> { text.chars().map(char::is_uppercase).collect() };
    accents(a)
        .cmp(&accents(b))
        .then_with(|| case(a).cmp(&case(b)))
        .then_with(|| a.cmp(b))
}

// --- Paths and files ---

/// path.resolve: absolute, with "." and ".." worked out and no separator at the end.
pub fn resolve(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    let absolute = if path.as_os_str().is_empty() {
        std::env::current_dir().unwrap_or_default()
    } else {
        std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
    };
    let mut out = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(out.components().next_back(), Some(Component::Normal(_))) {
                    out.pop();
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// path.isAbsolute: on Windows a path from the drive's root ("/Music", "\Music") counts too.
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

/// `new Date().toISOString()`.
pub fn iso_now() -> String {
    chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

/// `new Date(ms).toISOString()`.
pub fn iso_from_ms(ms: f64) -> String {
    chrono::DateTime::from_timestamp_millis(ms.trunc() as i64)
        .unwrap_or_default()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
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
    use super::*;

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
        assert_eq!(number(30.0).to_string(), "30");
        assert_eq!(number(1.25).to_string(), "1.25");
        // A file time read back from a cache is the same to the last bit (serde_json's float_roundtrip).
        let mtime = 1_794_962_131_877.482_7;
        assert_eq!(
            serde_json::from_str::<f64>(&number(mtime).to_string()).unwrap(),
            mtime
        );
        assert_eq!(parse_number("5."), 5.0);
        assert!(parse_number("1.2.3").is_nan());
    }

    #[test]
    fn sorts_names_like_localecompare() {
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
            locale_compare_numeric_base("Whoosh 2", "whoosh 10"),
            Ordering::Less
        );
        assert_eq!(locale_compare_numeric_base("Café", "cafe"), Ordering::Equal);
    }

    #[test]
    fn works_out_paths_like_node() {
        assert!(is_absolute("/Music"));
        assert!(!is_absolute("Music/Song.wav"));
        assert_eq!(is_absolute(r"C:\Music"), cfg!(windows));
        assert!(!is_absolute("C:Music"));
        assert_eq!(extname("Song.MP3"), ".MP3");
        assert_eq!(extname(".hidden"), "");
        assert_eq!(extname("a.b/c"), "");
        assert_eq!(parsed_name(Path::new("dir/Reel.final.mp4")), "Reel.final");
        let here = std::env::current_dir().unwrap();
        assert_eq!(resolve("a/./b/../c/"), here.join("a").join("c"));
        assert_eq!(slice("ab\u{1F600}c", 3), "ab");
    }
}
