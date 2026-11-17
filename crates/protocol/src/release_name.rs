// Releases are named by their day, like Linker's: 2026.11.07 is the release of 7 November 2026 (UTC). That's the name
// on tags, GitHub releases, installers and in the app. A day has one release; releasing again that day replaces it.
//
// npm and the app updater need a version of three numbers without leading zeros that sorts in release order, so
// underneath each release is YYYY.MDD.HHMM, the UTC time it was made: 2026.1107.1430. A release that replaces the
// day's earlier one has a later time, so installed apps still update to it.

use std::sync::LazyLock;

use regex::Regex;

static VERSION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([0-9]{4})\.([0-9]{1,2})([0-9]{2})\.([0-9]+)$").unwrap());

/// The release name for a version: 2026.1107.1430 → 2026.11.07. Anything else, like a checkout's 0.0.0, comes back as
/// it is.
pub fn release_name(version: &str) -> String {
    let Some(parts) = VERSION.captures(version) else {
        return version.to_string();
    };
    format!("{}.{:0>2}.{}", &parts[1], &parts[2], &parts[3])
}

/// Where every release is, to download one by hand.
pub const RELEASES_URL: &str = "https://github.com/FerretMedia/linker-edit/releases/latest";

/// Whether version a comes after version b; both are YYYY.MDD.HHMM.
pub fn is_newer(a: &str, b: &str) -> bool {
    let x: Vec<f64> = a.split('.').map(js_number).collect();
    let y: Vec<f64> = b.split('.').map(js_number).collect();
    for i in 0..3 {
        let diff = x.get(i).copied().unwrap_or(0.0) - y.get(i).copied().unwrap_or(0.0);
        // NaN, from a part that isn't a number, isn't newer, as in JavaScript.
        if diff != 0.0 {
            return diff > 0.0;
        }
    }
    false
}

/// JavaScript's Number(text) for the parts of a version: blank is 0, anything else not a number is NaN.
fn js_number(text: &str) -> f64 {
    let text = text.trim();
    if text.is_empty() {
        return 0.0;
    }
    text.parse().unwrap_or(f64::NAN)
}

#[cfg(test)]
mod tests {
    use super::*;

    // From scripts/release-version.test.ts.
    #[test]
    fn names_a_release_by_its_day() {
        assert_eq!(release_name("2026.1107.0"), "2026.11.07");
        assert_eq!(release_name("2026.1111.1430"), "2026.11.11");
        assert_eq!(release_name("2027.105.2359"), "2027.01.05");
    }

    #[test]
    fn leaves_other_versions_as_they_are() {
        assert_eq!(release_name("0.0.0"), "0.0.0");
        assert_eq!(release_name("1.2.3"), "1.2.3");
    }

    // From server/updates.test.ts.
    #[test]
    fn compares_numerically_not_as_text() {
        assert!(is_newer("2026.1108.0", "2026.205.3"));
        assert!(is_newer("2026.205.10", "2026.205.9"));
        assert!(!is_newer("2026.205.0", "2026.205.0"));
        assert!(!is_newer("0.0.0", "2026.205.0"));
    }

    #[test]
    fn a_later_release_the_same_day_or_month_is_newer() {
        assert!(is_newer("2026.1108.1430", "2026.1108.959"));
        assert!(is_newer("2026.201.0", "2026.131.2359"));
        assert!(is_newer("2026.1001.0", "2026.930.2359"));
        assert!(!is_newer("2026.x.0", "2026.1.0"));
    }
}
