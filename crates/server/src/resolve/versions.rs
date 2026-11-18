// Full access without mistakes: the project is backed up before agents change it, agents work on "AI - "
// timelines, and an AI timeline only becomes the main one when the user says so, keeping the old one.

use std::path::{Component, Path, PathBuf};

use chrono::NaiveDateTime;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::connection::ResolveConnection;
use crate::projects::store::STATE_DIR;

/// Where a project's backups go, in its working folder. Kept out of the edit history.
pub const BACKUPS_DIR: &str = "backups";
/// How many backups a project folder keeps; the oldest go first.
pub const KEEP_BACKUPS: usize = 30;

const AI_PREFIX: &str = "AI - ";

// Windows file names can't hold ":", and neither need timeline names.
fn file_stamp(date: NaiveDateTime) -> String {
    date.format("%Y-%m-%d %H-%M-%S").to_string()
}

fn name_stamp(date: NaiveDateTime) -> String {
    date.format("%Y-%m-%d %H.%M").to_string()
}

/// "C:\a\b" as Resolve's scripts take paths: with forward slashes.
fn forward_slashes(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Exports the open Resolve project into the folder's backups/, and keeps the newest KEEP_BACKUPS. Returns the
/// backup's path relative to dir. `now` is local time: `chrono::Local::now().naive_local()`.
pub async fn backup_project(
    connection: &dyn ResolveConnection,
    dir: &Path,
    now: NaiveDateTime,
) -> anyhow::Result<PathBuf> {
    #[derive(Deserialize)]
    struct Exported {
        file: String,
    }
    let backups = dir.join(BACKUPS_DIR);
    std::fs::create_dir_all(&backups)?;
    let ignore = backups.join(".gitignore");
    if !ignore.exists() {
        std::fs::write(&ignore, "*\n")?;
    }
    let Exported { file } = connection
        .run_script(
            "backup_project",
            json!({ "dir": forward_slashes(&backups), "stamp": file_stamp(now) }),
            60,
        )
        .await?;
    let file = PathBuf::from(file);
    if !file.exists() {
        anyhow::bail!(
            "Resolve said it exported the project, but there is no {}.",
            file.display()
        );
    }
    prune_backups(&backups);
    Ok(relative(dir, &file))
}

/// path.relative for a file inside dir, with the platform's separators.
fn relative(dir: &Path, file: &Path) -> PathBuf {
    match file.strip_prefix(dir) {
        Ok(inside) => inside
            .components()
            .filter(|part| matches!(part, Component::Normal(_)))
            .collect(),
        Err(_) => file.to_path_buf(),
    }
}

fn prune_backups(backups: &Path) {
    let Ok(entries) = std::fs::read_dir(backups) else {
        return;
    };
    let mut files: Vec<(PathBuf, std::time::SystemTime)> = entries
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".drp"))
        .filter_map(|entry| Some((entry.path(), entry.metadata().ok()?.modified().ok()?)))
        .collect();
    files.sort_by_key(|(_, time)| std::cmp::Reverse(*time));
    for (file, _) in files.into_iter().skip(KEEP_BACKUPS) {
        let _ = std::fs::remove_file(file);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelineRef {
    pub id: String,
    pub name: String,
}

/// For each AI timeline, by id: the main timeline it's a new version of.
pub type Origins = IndexMap<String, TimelineRef>;

fn origins_file(dir: &Path) -> PathBuf {
    dir.join(STATE_DIR).join("timeline-origins.json")
}

pub fn read_origins(dir: &Path) -> Origins {
    std::fs::read_to_string(origins_file(dir))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn write_origins(dir: &Path, origins: &Origins) -> anyhow::Result<()> {
    let file = origins_file(dir);
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(
        file,
        format!("{}\n", serde_json::to_string_pretty(origins)?),
    )?;
    Ok(())
}

/// Remembers what a new AI timeline is a version of. A version of an AI timeline replaces what that one
/// would have; one of a timeline nothing is known about replaces nothing.
pub fn record_origin(dir: &Path, ai_id: &str, source: &TimelineRef) -> anyhow::Result<()> {
    let mut origins = read_origins(dir);
    let main = match origins.get(&source.id) {
        Some(main) => main.clone(),
        None if source.name.starts_with(AI_PREFIX) => return Ok(()),
        None => source.clone(),
    };
    origins.insert(ai_id.to_string(), main);
    write_origins(dir, &origins)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Applied {
    pub main: TimelineRef,
    pub previous: TimelineRef,
    /// The AI timeline's name before it took the main one's.
    pub was: String,
}

/// Makes an approved AI timeline the main one: the main one is renamed "<name> (before AI <date>)" and the
/// AI one takes its name. The user starts this; back the project up first. `now` is local time.
pub async fn apply_timeline(
    connection: &dyn ResolveConnection,
    dir: &Path,
    ai_id: &str,
    now: NaiveDateTime,
) -> anyhow::Result<Applied> {
    let origins = read_origins(dir);
    let Some(main) = origins.get(ai_id).cloned() else {
        anyhow::bail!("Linker-Edit doesn't know which timeline this one is a new version of.");
    };
    let applied: Applied = connection
        .run_script(
            "apply_timeline",
            json!({ "timeline": ai_id, "main": main.id, "stamp": name_stamp(now) }),
            30,
        )
        .await?;
    // Other versions of the same timeline now replace the new main one.
    let next: Origins = origins
        .into_iter()
        .filter(|(id, _)| id != ai_id)
        .map(|(id, origin)| {
            let origin = if origin.id == main.id {
                TimelineRef {
                    id: ai_id.to_string(),
                    name: applied.main.name.clone(),
                }
            } else {
                origin
            };
            (id, origin)
        })
        .collect();
    write_origins(dir, &next)?;
    Ok(applied)
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::sync::Arc;
    use std::time::{Duration, SystemTime};

    use chrono::NaiveDate;
    use serde_json::{Value, json};

    use super::*;
    use crate::resolve::run::{ToolContext, run_resolve_tool};
    use crate::resolve::testing::{
        FAKE_RESOLVE_MODULE, PythonConnection, has_python, logged_calls, python_connection,
    };

    fn now() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 11, 12)
            .unwrap()
            .and_hms_opt(9, 30, 15)
            .unwrap()
    }

    struct Setup {
        _dir: tempfile::TempDir,
        state: PathBuf,
        connection: Arc<PythonConnection>,
        workspace: PathBuf,
    }

    impl Setup {
        fn calls(&self) -> Vec<Value> {
            logged_calls(&self.state)
        }
    }

    fn setup(extra: Value) -> Setup {
        let dir = tempfile::Builder::new()
            .prefix("linker-edit-versions-")
            .tempdir()
            .unwrap();
        let state = dir.path().join("state.json");
        let mut value = json!({
            "project": {
                "name": "Doc: final",
                "id": "doc-id",
                "current": "Edit",
                "timelines": [
                    { "id": "t1", "name": "Edit", "video": [[]], "audio": [] },
                    { "id": "t2", "name": "AI - Edit tight", "video": [[]], "audio": [] },
                    { "id": "t3", "name": "AI - Other", "video": [[]], "audio": [] }
                ]
            },
            "media": { "name": "Master", "clips": [], "folders": [] }
        });
        if let Value::Object(extra) = extra {
            value.as_object_mut().unwrap().extend(extra);
        }
        std::fs::write(&state, value.to_string()).unwrap();
        let connection = python_connection(
            FAKE_RESOLVE_MODULE,
            &[("FAKE_RESOLVE", state.to_str().unwrap())],
        );
        let workspace = dir.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        Setup {
            _dir: dir,
            state,
            connection,
            workspace,
        }
    }

    fn origin(id: &str, name: &str) -> TimelineRef {
        TimelineRef {
            id: id.into(),
            name: name.into(),
        }
    }

    fn origins_json(dir: &Path) -> Value {
        serde_json::to_value(read_origins(dir)).unwrap()
    }

    #[tokio::test]
    async fn saves_and_exports_the_open_project_into_backups_kept_out_of_git() {
        if !has_python() {
            return;
        }
        let s = setup(json!({}));
        let file = backup_project(s.connection.as_ref(), &s.workspace, now())
            .await
            .unwrap();
        assert_eq!(
            file,
            Path::new("backups").join("Doc- final 2026-11-12 09-30-15.drp")
        );
        assert_eq!(
            file.to_string_lossy(),
            Path::new("backups")
                .join("Doc- final 2026-11-12 09-30-15.drp")
                .to_string_lossy()
        );
        assert!(s.workspace.join(&file).exists());
        assert_eq!(
            std::fs::read_to_string(s.workspace.join("backups/.gitignore")).unwrap(),
            "*\n"
        );
        assert_eq!(
            s.calls(),
            vec![
                json!(["SaveProject"]),
                json!([
                    "ExportProject",
                    "Doc: final",
                    "Doc- final 2026-11-12 09-30-15.drp",
                    false
                ])
            ]
        );
    }

    #[tokio::test]
    async fn fails_when_resolve_cant_export() {
        if !has_python() {
            return;
        }
        let s = setup(json!({ "export_fails": true }));
        let error = backup_project(s.connection.as_ref(), &s.workspace, now())
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains(r#"couldn't export "Doc: final""#),
            "{error}"
        );
    }

    #[tokio::test]
    async fn keeps_the_newest_backups() {
        if !has_python() {
            return;
        }
        let s = setup(json!({}));
        let backups = s.workspace.join("backups");
        std::fs::create_dir(&backups).unwrap();
        let start = SystemTime::UNIX_EPOCH + Duration::from_secs(1_767_225_600); // 2026-01-01
        for i in 0..KEEP_BACKUPS + 2 {
            let old = backups.join(format!("old {i}.drp"));
            std::fs::write(&old, "drp").unwrap();
            let file = File::options().write(true).open(&old).unwrap();
            file.set_modified(start + Duration::from_secs(60 * i as u64))
                .unwrap();
        }
        backup_project(s.connection.as_ref(), &s.workspace, now())
            .await
            .unwrap();
        let left: Vec<String> = std::fs::read_dir(&backups)
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".drp"))
            .collect();
        assert_eq!(left.len(), KEEP_BACKUPS);
        assert!(left.contains(&"Doc- final 2026-11-12 09-30-15.drp".to_string()));
        assert!(!left.contains(&"old 0.drp".to_string()));
        assert!(!left.contains(&"old 2.drp".to_string()));
        assert!(left.contains(&"old 3.drp".to_string()));
    }

    #[tokio::test]
    async fn remembers_what_a_copy_is_a_version_of_through_copies_of_copies() {
        if !has_python() {
            return;
        }
        let s = setup(json!({}));
        let context = ToolContext::new(&s.workspace);
        run_resolve_tool(
            s.connection.clone(),
            "duplicate_timeline",
            json!({ "timeline": "Edit", "name": "Tight" }),
            &context,
        )
        .await
        .unwrap();
        let origins = read_origins(&s.workspace);
        assert_eq!(
            origins.values().cloned().collect::<Vec<_>>(),
            vec![origin("t1", "Edit")]
        );
        let copy = origins.keys().next().unwrap().clone();
        record_origin(&s.workspace, "t4", &origin(&copy, "AI - Tight")).unwrap();
        assert_eq!(
            read_origins(&s.workspace).get("t4"),
            Some(&origin("t1", "Edit"))
        );
        // A copy of an AI timeline nothing is known about replaces nothing.
        record_origin(&s.workspace, "t5", &origin("t3", "AI - Other")).unwrap();
        assert_eq!(read_origins(&s.workspace).get("t5"), None);
    }

    #[tokio::test]
    async fn remembers_what_a_new_timeline_replaces_and_refuses_one_that_doesnt_exist() {
        if !has_python() {
            return;
        }
        let s = setup(json!({}));
        let context = ToolContext::new(&s.workspace);
        let made = run_resolve_tool(
            s.connection.clone(),
            "create_timeline",
            json!({ "name": "Silences cut", "replaces": "Edit" }),
            &context,
        )
        .await
        .unwrap();
        let made: Value = serde_json::from_str(&made.text).unwrap();
        assert_eq!(made["replaces"], json!({ "id": "t1", "name": "Edit" }));
        assert_eq!(
            read_origins(&s.workspace).get(made["id"].as_str().unwrap()),
            Some(&origin("t1", "Edit"))
        );
        let error = run_resolve_tool(
            s.connection.clone(),
            "create_timeline",
            json!({ "name": "X", "replaces": "Nope" }),
            &context,
        )
        .await
        .unwrap_err();
        assert!(
            error.to_string().contains(r#"no timeline "Nope""#),
            "{error}"
        );
    }

    #[tokio::test]
    async fn makes_an_approved_ai_timeline_the_main_one_keeping_the_old_one_renamed() {
        if !has_python() {
            return;
        }
        let s = setup(json!({}));
        record_origin(&s.workspace, "t2", &origin("t1", "Edit")).unwrap();
        record_origin(&s.workspace, "t9", &origin("t1", "Edit")).unwrap();
        let applied = apply_timeline(s.connection.as_ref(), &s.workspace, "t2", now())
            .await
            .unwrap();
        assert_eq!(
            applied,
            Applied {
                main: origin("t2", "Edit"),
                previous: origin("t1", "Edit (before AI 2026-11-12 09.30)"),
                was: "AI - Edit tight".into()
            }
        );
        assert_eq!(
            s.calls(),
            vec![
                json!(["SetName", "Edit", "Edit (before AI 2026-11-12 09.30)"]),
                json!(["SetName", "AI - Edit tight", "Edit"]),
                json!(["SetCurrentTimeline", "Edit"])
            ]
        );
        // The applied one is the main timeline now; other versions of it replace the new main one.
        assert_eq!(
            origins_json(&s.workspace),
            json!({ "t9": { "id": "t2", "name": "Edit" } })
        );
    }

    #[tokio::test]
    async fn only_applies_an_ai_timeline_it_knows_the_main_timeline_of() {
        if !has_python() {
            return;
        }
        let s = setup(json!({}));
        let error = apply_timeline(s.connection.as_ref(), &s.workspace, "t2", now())
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("doesn't know which timeline"),
            "{error}"
        );
        record_origin(&s.workspace, "t1", &origin("t3", "Edit")).unwrap();
        let error = apply_timeline(s.connection.as_ref(), &s.workspace, "t1", now())
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("isn't an AI timeline"),
            "{error}"
        );
        assert_eq!(s.calls(), Vec::<Value>::new());
    }

    #[tokio::test]
    async fn puts_the_main_timelines_name_back_when_the_ai_timeline_cant_take_it() {
        if !has_python() {
            return;
        }
        let s = setup(json!({ "refuse_rename": ["t2"] }));
        record_origin(&s.workspace, "t2", &origin("t1", "Edit")).unwrap();
        let error = apply_timeline(s.connection.as_ref(), &s.workspace, "t2", now())
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains(r#"refused to rename "AI - Edit tight""#),
            "{error}"
        );
        assert_eq!(
            s.calls(),
            vec![
                json!(["SetName", "Edit", "Edit (before AI 2026-11-12 09.30)"]),
                json!(["SetName", "AI - Edit tight", "Edit"]),
                json!(["SetName", "Edit (before AI 2026-11-12 09.30)", "Edit"])
            ]
        );
        // Nothing changed, so it can be tried again.
        assert_eq!(
            origins_json(&s.workspace),
            json!({ "t2": { "id": "t1", "name": "Edit" } })
        );
    }
}
