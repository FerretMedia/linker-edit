use serde::Serialize;

use crate::blobs::version_stamp;
use crate::read::DrpFile;

/// The project formats seen in sample files, for messages. Resolve opens older formats (upgrading them) but
/// refuses newer ones.
fn format_name(prj_ver: u32) -> Option<&'static str> {
    match prj_ver {
        14 => Some("Resolve 19.1"),
        17 => Some("Resolve 21"),
        _ => None,
    }
}

/// `{ ok: true }`, or `{ ok: false, message }` saying why the file can't be opened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VersionCheck {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl VersionCheck {
    fn ok() -> Self {
        Self {
            ok: true,
            message: None,
        }
    }

    fn refused(message: String) -> Self {
        Self {
            ok: false,
            message: Some(message),
        }
    }
}

/// Whether a Resolve that reads project format `target_prj_ver` (its DbPrjVer) can open this file.
pub fn check_version(file: &DrpFile, target_prj_ver: u32) -> VersionCheck {
    let label = if file.project.name.is_empty() {
        "This file".to_string()
    } else {
        format!("\"{}\"", file.project.name)
    };
    let stamps: Vec<_> = file
        .xml
        .iter()
        .map(|(entry, text)| (entry, version_stamp(text)))
        .collect();
    let unstamped = stamps.iter().find(|(_, stamp)| stamp.is_none());
    if unstamped.is_some() || stamps.is_empty() {
        let place = unstamped
            .map(|(entry, _)| format!(" in {entry}"))
            .unwrap_or_default();
        return VersionCheck::refused(format!(
            "{label} has no version stamp{place}, so it can't be checked."
        ));
    }
    let stamps: Vec<_> = stamps.into_iter().filter_map(|(_, stamp)| stamp).collect();
    let mut formats: Vec<u32> = stamps.iter().map(|stamp| stamp.db_prj_ver).collect();
    formats.sort_unstable();
    formats.dedup();
    if formats.len() > 1 {
        let formats: Vec<String> = formats.iter().map(u32::to_string).collect();
        return VersionCheck::refused(format!(
            "{label} mixes project formats {}, so it wasn't saved by one Resolve.",
            formats.join(" and ")
        ));
    }
    let stamp = &stamps[0];
    if stamp.db_prj_ver <= target_prj_ver {
        return VersionCheck::ok();
    }
    let app = stamp
        .db_app_ver
        .split('.')
        .take(3)
        .collect::<Vec<_>>()
        .join(".");
    let target = format_name(target_prj_ver)
        .map(|name| format!(" ({name})"))
        .unwrap_or_default();
    VersionCheck::refused(format!(
        "{label} was saved by DaVinci Resolve {app} in project format {}, but the Resolve opening it reads format \
         {target_prj_ver}{target}. Resolve can't open projects from a newer version: update it to {app} or later.",
        stamp.db_prj_ver
    ))
}
