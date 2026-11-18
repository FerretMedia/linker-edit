//! src/lib/settings.ts: the settings page's sections, and Resolve's state in one line.
//!
//! The browser keeps the open section in the address ("#settings/remote") so Back closes the
//! page; the window has no address, so `sectionFromHash`/`settingsHash` have no port: the
//! workspace holds the open section.

use linker_edit_protocol::ResolveStatus;

use crate::icons::IconName;
use crate::labels::COLLECTIONS_TITLE;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SettingsSection {
    #[default]
    General,
    Workspaces,
    Resolve,
    Transcription,
    Remote,
    Account,
    About,
}

impl SettingsSection {
    /// SETTINGS_SECTIONS, in its order.
    pub const ALL: [SettingsSection; 7] = [
        SettingsSection::General,
        SettingsSection::Workspaces,
        SettingsSection::Resolve,
        SettingsSection::Transcription,
        SettingsSection::Remote,
        SettingsSection::Account,
        SettingsSection::About,
    ];

    /// The id the TS uses, which is also the element id of its tab here.
    pub fn id(self) -> &'static str {
        match self {
            SettingsSection::General => "general",
            SettingsSection::Workspaces => "workspaces",
            SettingsSection::Resolve => "resolve",
            SettingsSection::Transcription => "transcription",
            SettingsSection::Remote => "remote",
            SettingsSection::Account => "account",
            SettingsSection::About => "about",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SettingsSection::General => "General",
            SettingsSection::Workspaces => COLLECTIONS_TITLE,
            SettingsSection::Resolve => "Resolve",
            SettingsSection::Transcription => "Transcription",
            SettingsSection::Remote => "Remote access",
            SettingsSection::Account => "Linker account",
            SettingsSection::About => "About & updates",
        }
    }

    /// SettingsPage.svelte's ICONS.
    pub fn icon(self) -> IconName {
        match self {
            SettingsSection::General => IconName::Settings2,
            SettingsSection::Workspaces => IconName::Layers,
            SettingsSection::Resolve => IconName::Clapperboard,
            SettingsSection::Transcription => IconName::AudioLines,
            SettingsSection::Remote => IconName::Globe,
            SettingsSection::Account => IconName::UserRound,
            SettingsSection::About => IconName::Info,
        }
    }
}

/// The dot before Resolve's status line: settings.ts's `tone` class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusTone {
    Success,
    Warning,
    Destructive,
    Muted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusLine {
    pub tone: StatusTone,
    pub title: String,
    pub detail: String,
}

/// Resolve's state in one line, as the sidebar and the settings page show it.
pub fn resolve_status_line(resolve: &ResolveStatus) -> StatusLine {
    let line = |tone, title: &str, detail: &str| StatusLine {
        tone,
        title: title.into(),
        detail: detail.into(),
    };
    match resolve {
        ResolveStatus::Checking => line(StatusTone::Warning, "Checking Resolve…", ""),
        ResolveStatus::Connected {
            product,
            version,
            project,
            timeline,
            ..
        } => {
            let version = version.split('.').take(3).collect::<Vec<_>>().join(".");
            let detail = match (project, timeline) {
                (Some(project), Some(timeline)) => format!("{project} · {timeline}"),
                (Some(project), None) => project.clone(),
                (None, _) => "No project open".into(),
            };
            StatusLine {
                tone: StatusTone::Success,
                title: format!("{product} {version}"),
                detail,
            }
        }
        ResolveStatus::NotRunning => line(
            StatusTone::Muted,
            "Resolve is not running",
            "Start Resolve, then refresh.",
        ),
        ResolveStatus::Blocked { hint } => {
            line(StatusTone::Warning, "Resolve needs attention", hint)
        }
        ResolveStatus::Unreachable { hint } => {
            line(StatusTone::Destructive, "Resolve not reachable", hint)
        }
        ResolveStatus::SetupNeeded { hint } => line(StatusTone::Destructive, "Setup needed", hint),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // settings.test.ts
    #[test]
    fn names_the_version_without_its_build_number() {
        let line = resolve_status_line(&ResolveStatus::Connected {
            product: "DaVinci Resolve Studio".into(),
            version: "21.1.2.5".into(),
            project: Some("Promo".into()),
            project_id: None,
            database: None,
            timeline: Some("Cut 1".into()),
        });
        assert_eq!(
            line,
            StatusLine {
                tone: StatusTone::Success,
                title: "DaVinci Resolve Studio 21.1.2".into(),
                detail: "Promo · Cut 1".into(),
            }
        );
    }
}
