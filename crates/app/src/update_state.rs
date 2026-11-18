//! How the desktop app's update state moves from one status to the next, like T3 Code's updateMachine.ts: plain
//! functions with no Velopack in them, so they're tested on their own. updater.rs calls them as Velopack reports
//! what it's doing.

use std::sync::LazyLock;

use linker_edit_protocol::{DesktopUpdateState, DesktopUpdateStatus};
use regex::Regex;
use serde::{Deserialize, Serialize};

pub fn initial(current_version: &str, not_applied: Option<String>) -> DesktopUpdateState {
    DesktopUpdateState {
        status: DesktopUpdateStatus::Idle,
        current_version: current_version.to_string(),
        available_version: None,
        downloaded_version: None,
        percent: None,
        notes: vec![],
        checked_at: None,
        error: None,
        download_url: None,
        not_applied,
    }
}

pub fn on_check_start(state: &DesktopUpdateState) -> DesktopUpdateState {
    // A downloaded update stays ready to install while it looks for a newer one.
    if state.downloaded_version.is_some() {
        return DesktopUpdateState {
            error: None,
            ..state.clone()
        };
    }
    DesktopUpdateState {
        status: DesktopUpdateStatus::Checking,
        percent: None,
        error: None,
        ..state.clone()
    }
}

pub fn on_check_failed(state: &DesktopUpdateState, message: &str, now: &str) -> DesktopUpdateState {
    if state.downloaded_version.is_some() {
        return DesktopUpdateState {
            status: DesktopUpdateStatus::Downloaded,
            percent: Some(100.0),
            checked_at: Some(now.into()),
            ..state.clone()
        };
    }
    DesktopUpdateState {
        status: DesktopUpdateStatus::Error,
        percent: None,
        error: Some(message.into()),
        checked_at: Some(now.into()),
        ..state.clone()
    }
}

/// `download_url` is set when the app can't install it by itself and the user downloads it by hand.
pub fn on_available(
    state: &DesktopUpdateState,
    version: &str,
    notes: Vec<String>,
    now: &str,
    download_url: Option<String>,
) -> DesktopUpdateState {
    if state.downloaded_version.as_deref() == Some(version) {
        return DesktopUpdateState {
            status: DesktopUpdateStatus::Downloaded,
            available_version: Some(version.into()),
            percent: Some(100.0),
            checked_at: Some(now.into()),
            error: None,
            ..state.clone()
        };
    }
    DesktopUpdateState {
        status: DesktopUpdateStatus::Available,
        available_version: Some(version.into()),
        downloaded_version: None,
        percent: None,
        notes,
        checked_at: Some(now.into()),
        error: None,
        download_url,
        ..state.clone()
    }
}

pub fn on_not_available(state: &DesktopUpdateState, now: &str) -> DesktopUpdateState {
    if state.downloaded_version.is_some() {
        return DesktopUpdateState {
            status: DesktopUpdateStatus::Downloaded,
            percent: Some(100.0),
            checked_at: Some(now.into()),
            error: None,
            ..state.clone()
        };
    }
    DesktopUpdateState {
        status: DesktopUpdateStatus::UpToDate,
        available_version: None,
        percent: None,
        notes: vec![],
        checked_at: Some(now.into()),
        error: None,
        download_url: None,
        ..state.clone()
    }
}

pub fn on_progress(state: &DesktopUpdateState, percent: f64) -> DesktopUpdateState {
    DesktopUpdateState {
        status: DesktopUpdateStatus::Downloading,
        percent: Some(percent.clamp(0.0, 100.0).floor()),
        error: None,
        ..state.clone()
    }
}

pub fn on_downloaded(state: &DesktopUpdateState, version: &str) -> DesktopUpdateState {
    DesktopUpdateState {
        status: DesktopUpdateStatus::Downloaded,
        available_version: Some(version.into()),
        downloaded_version: Some(version.into()),
        percent: Some(100.0),
        error: None,
        ..state.clone()
    }
}

pub fn on_download_failed(state: &DesktopUpdateState, message: &str) -> DesktopUpdateState {
    // With a release found, "available" lets the next check download it again.
    DesktopUpdateState {
        status: if state.available_version.is_some() {
            DesktopUpdateStatus::Available
        } else {
            DesktopUpdateStatus::Error
        },
        percent: None,
        error: Some(message.into()),
        ..state.clone()
    }
}

/// The installer didn't start; the update is still downloaded, so the user can try again.
pub fn on_install_failed(state: &DesktopUpdateState, message: &str) -> DesktopUpdateState {
    DesktopUpdateState {
        status: DesktopUpdateStatus::Downloaded,
        error: Some(message.into()),
        ..state.clone()
    }
}

/// Written just before installing, read when the app starts again: see [`not_applied_version`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallRecord {
    pub from: String,
    pub to: String,
}

/// The version an install was meant to bring, when the app started again still on the version it was installed
/// from: the installer failed or was stopped. None when the install worked, or the record is unreadable.
pub fn not_applied_version(record: Option<&InstallRecord>, current: &str) -> Option<String> {
    record
        .filter(|record| record.from == current && record.to != current)
        .map(|record| record.to.clone())
}

const MAX_NOTES: usize = 6;
const MAX_NOTE_LENGTH: usize = 140;

static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]*>").unwrap());
static ENTITY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)&(#x[0-9a-f]+|#[0-9]+|[a-z]+);").unwrap());
static SPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
static ITEM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<li[^>]*>(.*?)</li>").unwrap());
static PARAGRAPH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<p[^>]*>(.*?)</p>").unwrap());
static BY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r" by @[\w-]+(?:\[bot\])? in \S+$").unwrap());
static IN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r" in #[0-9]+$").unwrap());
static SKIPPED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(full changelog|new contributors)\b").unwrap());
static HTML: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<[a-z]").unwrap());

fn plain_text(html: &str) -> String {
    let text = TAG.replace_all(html, "");
    let text = ENTITY.replace_all(&text, |caps: &regex::Captures| {
        let name = &caps[1];
        let code = if let Some(hex) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
            u32::from_str_radix(hex, 16).ok()
        } else if let Some(decimal) = name.strip_prefix('#') {
            decimal.parse().ok()
        } else {
            match name.to_ascii_lowercase().as_str() {
                "amp" => Some('&' as u32),
                "lt" => Some('<' as u32),
                "gt" => Some('>' as u32),
                "quot" => Some('"' as u32),
                "apos" => Some('\'' as u32),
                "nbsp" => Some(' ' as u32),
                _ => None,
            }
        };
        code.and_then(char::from_u32)
            .map_or_else(|| caps[0].to_string(), String::from)
    });
    SPACE.replace_all(&text, " ").trim().to_string()
}

fn shorten(line: &str) -> String {
    if line.chars().count() <= MAX_NOTE_LENGTH {
        return line.to_string();
    }
    let kept: String = line.chars().take(MAX_NOTE_LENGTH - 1).collect();
    format!("{}…", kept.trim_end())
}

/// A few short lines of what's new, from a release's notes as Velopack has them: the GitHub release's generated
/// notes (a list of the merged pull requests, "Title by @user in #12"), packed into the release as HTML.
pub fn release_note_lines(html: &str) -> Vec<String> {
    let items: Vec<String> = ITEM
        .captures_iter(html)
        .map(|caps| plain_text(&caps[1]))
        .collect();
    let lines: Vec<String> = if items.is_empty() {
        PARAGRAPH
            .captures_iter(html)
            .map(|caps| plain_text(&caps[1]))
            .collect()
    } else {
        items
            .iter()
            .map(|item| IN.replace(&BY.replace(item, ""), "").into_owned())
            .collect()
    };
    let kept: Vec<String> = lines
        .into_iter()
        .filter(|line| !line.is_empty() && !SKIPPED.is_match(line))
        .collect();
    // Without any HTML, the notes are plain text, one line each.
    let plain = if !kept.is_empty() || HTML.is_match(html) {
        kept
    } else {
        html.split('\n')
            .map(plain_text)
            .filter(|line| !line.is_empty())
            .collect()
    };
    plain
        .iter()
        .take(MAX_NOTES)
        .map(|line| shorten(line))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: &str = "2026-11-12T10:00:00.000Z";

    fn start() -> DesktopUpdateState {
        initial("2026.1111.2", None)
    }

    #[test]
    fn goes_from_checking_to_downloaded() {
        let state = on_check_start(&start());
        assert_eq!(state.status, DesktopUpdateStatus::Checking);
        let state = on_available(
            &state,
            "2026.1112.0",
            vec!["Faster previews".into()],
            NOW,
            None,
        );
        assert_eq!(state.status, DesktopUpdateStatus::Available);
        assert_eq!(state.available_version.as_deref(), Some("2026.1112.0"));
        assert_eq!(state.notes, ["Faster previews"]);
        assert_eq!(state.checked_at.as_deref(), Some(NOW));
        let state = on_progress(&state, 41.7);
        assert_eq!(
            (state.status, state.percent),
            (DesktopUpdateStatus::Downloading, Some(41.0))
        );
        let state = on_downloaded(&state, "2026.1112.0");
        assert_eq!(state.status, DesktopUpdateStatus::Downloaded);
        assert_eq!(state.downloaded_version.as_deref(), Some("2026.1112.0"));
        assert_eq!(state.percent, Some(100.0));
    }

    #[test]
    fn says_up_to_date_when_theres_nothing_new() {
        let state = on_not_available(&on_check_start(&start()), NOW);
        assert_eq!(state.status, DesktopUpdateStatus::UpToDate);
        assert_eq!(state.available_version, None);
        assert_eq!(state.checked_at.as_deref(), Some(NOW));
    }

    #[test]
    fn keeps_a_downloaded_update_through_later_checks_failed_or_not() {
        let downloaded = on_downloaded(
            &on_available(&start(), "2026.1112.0", vec![], NOW, None),
            "2026.1112.0",
        );
        assert_eq!(
            on_check_start(&downloaded).status,
            DesktopUpdateStatus::Downloaded
        );
        assert_eq!(
            on_not_available(&downloaded, NOW).status,
            DesktopUpdateStatus::Downloaded
        );
        let failed = on_check_failed(&downloaded, "offline", NOW);
        assert_eq!(
            (failed.status, failed.error),
            (DesktopUpdateStatus::Downloaded, None)
        );
        assert_eq!(
            on_available(&downloaded, "2026.1112.0", vec![], NOW, None).status,
            DesktopUpdateStatus::Downloaded
        );
    }

    #[test]
    fn offers_a_newer_release_than_the_one_downloaded() {
        let downloaded = on_downloaded(
            &on_available(&start(), "2026.1112.0", vec![], NOW, None),
            "2026.1112.0",
        );
        let newer = on_available(&downloaded, "2026.1112.1", vec![], NOW, None);
        assert_eq!(newer.status, DesktopUpdateStatus::Available);
        assert_eq!(newer.downloaded_version, None);
    }

    #[test]
    fn reports_a_failed_check_and_a_failed_download_with_the_release_still_on_offer() {
        let failed = on_check_failed(&on_check_start(&start()), "offline", NOW);
        assert_eq!(
            (failed.status, failed.error.as_deref()),
            (DesktopUpdateStatus::Error, Some("offline"))
        );
        let available = on_available(&start(), "2026.1112.0", vec![], NOW, None);
        let failed = on_download_failed(&on_progress(&available, 10.0), "reset");
        assert_eq!(
            (failed.status, failed.error.as_deref(), failed.percent),
            (DesktopUpdateStatus::Available, Some("reset"), None)
        );
        assert_eq!(
            on_download_failed(&start(), "reset").status,
            DesktopUpdateStatus::Error
        );
    }

    #[test]
    fn keeps_the_update_after_an_installer_that_didnt_start() {
        let downloaded = on_downloaded(&start(), "2026.1112.0");
        let failed = on_install_failed(&downloaded, "spawn EACCES");
        assert_eq!(
            (failed.status, failed.error.as_deref()),
            (DesktopUpdateStatus::Downloaded, Some("spawn EACCES"))
        );
    }

    #[test]
    fn carries_a_download_link_where_the_app_cant_install_by_itself() {
        let available = on_available(
            &start(),
            "2026.1112.0",
            vec![],
            NOW,
            Some("https://example.com/releases".into()),
        );
        assert_eq!(
            available.download_url.as_deref(),
            Some("https://example.com/releases")
        );
    }

    #[test]
    fn finds_an_install_that_came_back_on_the_old_version() {
        let record = InstallRecord {
            from: "2026.1111.2".into(),
            to: "2026.1112.0".into(),
        };
        assert_eq!(
            not_applied_version(Some(&record), "2026.1111.2").as_deref(),
            Some("2026.1112.0")
        );
        assert_eq!(not_applied_version(Some(&record), "2026.1112.0"), None);
        assert_eq!(not_applied_version(None, "2026.1111.2"), None);
    }

    // As GitHub generates them; Velopack has them as HTML from the markdown the release was packed with.
    const GENERATED: &str = r#"<h2>What's Changed</h2>
<ul>
<li>Title bar built into the window, like T3 Code by <a class="user-mention notranslate" href="https://github.com/TheTrueFerret">@TheTrueFerret</a> in <a class="issue-link js-issue-link" href="https://github.com/FerretMedia/linker-edit/pull/7">#7</a></li>
<li>Name releases by their day: 2026.11.11, like Linker&#39;s by <a href="https://github.com/TheTrueFerret">@TheTrueFerret</a> in <a href="https://github.com/FerretMedia/linker-edit/pull/8">#8</a></li>
</ul>
<p><strong>Full Changelog</strong>: <a class="commit-link" href="https://github.com/FerretMedia/linker-edit/compare/v1...v2"><tt>v1...v2</tt></a></p>"#;

    #[test]
    fn lists_the_pull_request_titles() {
        assert_eq!(
            release_note_lines(GENERATED),
            [
                "Title bar built into the window, like T3 Code",
                "Name releases by their day: 2026.11.11, like Linker's",
            ]
        );
        // The same notes as Velopack turns GitHub's markdown into HTML, with the pull request's address.
        assert_eq!(
            release_note_lines(
                "<ul>\n<li>Faster previews by @someone in https://github.com/FerretMedia/linker-edit/pull/9</li>\n</ul>"
            ),
            ["Faster previews"]
        );
    }

    #[test]
    fn reads_paragraphs_when_theres_no_list_without_the_changelog_link() {
        assert_eq!(
            release_note_lines(
                "<p>Our own Resolve MCP: 27 tools</p><p><strong>Full Changelog</strong>: v1...v2</p>"
            ),
            ["Our own Resolve MCP: 27 tools"]
        );
    }

    #[test]
    fn takes_plain_text() {
        assert_eq!(release_note_lines("First\nSecond"), ["First", "Second"]);
        assert!(release_note_lines("").is_empty());
    }

    #[test]
    fn keeps_it_short() {
        let many: String = (0..10)
            .map(|i| format!("<li>Change {i} {}</li>", "x".repeat(200)))
            .collect();
        let lines = release_note_lines(&format!("<ul>{many}</ul>"));
        assert_eq!(lines.len(), 6);
        assert!(lines[0].chars().count() <= 140);
        assert!(lines[0].ends_with('…'));
    }
}
