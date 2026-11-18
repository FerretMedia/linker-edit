// The library's tools on the agents' "linker" MCP server: finding files in the library, and bringing them into
// the open Resolve project through import_media, into a "Library/<Kind>" bin.

use std::path::Path;

use anyhow::bail;
use async_trait::async_trait;
use linker_edit_protocol::{LIBRARY_KINDS, LibraryFile, LibraryKind, LibraryQuery};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::folders::{library_kind, path_key};
use crate::js;

pub const LIBRARY_SEARCH: &str = "library_search";
pub const LIBRARY_IMPORT: &str = "library_import";

/// The tools' definitions, as the MCP server lists them.
pub fn library_tools() -> Vec<Value> {
    let kinds: Vec<Value> = LIBRARY_KINDS
        .iter()
        .map(|kind| serde_json::to_value(kind).unwrap_or_default())
        .collect();
    vec![
        json!({
            "name": LIBRARY_SEARCH,
            "description": "Find music, sound effects, graphics, LUTs and footage in the user's library: folders of files they reuse. Searches file names, tags and notes; filter by kind, tags and length. Results come from the project's own library first, then the universal one, starred files first. Each has its absolute path, length in seconds, channels, loudness (LUFS), tags, notes, the folder's note (licensing: respect it) and where it was used before.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "enum": kinds, "description": "music, sfx (sound effects), graphics, luts, footage or other." },
                    "query": { "type": "string", "description": "Words to find, such as \"upbeat\" or \"whoosh\". Leave out to list everything of a kind." },
                    "tags": { "type": "array", "items": { "type": "string" }, "description": "Only files with all of these tags." },
                    "min_seconds": { "type": "number" },
                    "max_seconds": { "type": "number" },
                    "limit": { "type": "integer", "description": "At most this many results. Defaults to 30." },
                },
            },
            "annotations": { "readOnlyHint": true },
        }),
        json!({
            "name": LIBRARY_IMPORT,
            "description": "Bring library files (paths from library_search) into the open Resolve project's media pool, into a \"Library/<Kind>\" bin such as \"Library/Music\". A file already in the media pool is reused, not imported twice. Returns each file's media pool id for add_clips. Notes the project and timeline it was used in. LUTs aren't media: pass their path to set_color.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "files": { "type": "array", "items": { "type": "string" }, "description": "Absolute paths, as library_search returns them." },
                    "bin": { "type": "string", "description": "Another bin to import into, such as \"Library/Music/Intro\". Defaults to \"Library/<Kind>\"." },
                    "timeline": { "type": "string", "description": "The timeline you'll use them on, for the file's \"used in\". Defaults to the one open in Resolve." },
                },
                "required": ["files"],
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false },
        }),
    ]
}

pub fn is_library_tool(name: &str) -> bool {
    name == LIBRARY_SEARCH || name == LIBRARY_IMPORT
}

/// The bin a kind's files go into, under "Library/".
pub fn kind_bin(kind: LibraryKind) -> &'static str {
    match kind {
        LibraryKind::Music => "Music",
        LibraryKind::Sfx => "Sound effects",
        LibraryKind::Graphics => "Graphics",
        LibraryKind::Luts => "LUTs",
        LibraryKind::Footage => "Footage",
        LibraryKind::Other => "Other",
    }
}

fn num(value: Option<&Value>) -> Option<f64> {
    value
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .filter(|item| !js::trim(item).is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn text(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(js::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

pub fn query_from_input(input: &Map<String, Value>) -> LibraryQuery {
    LibraryQuery {
        kind: input.get("kind").and_then(library_kind),
        query: input
            .get("query")
            .and_then(Value::as_str)
            .filter(|query| !js::trim(query).is_empty())
            .map(str::to_string),
        tags: Some(strings(input.get("tags"))).filter(|tags| !tags.is_empty()),
        min_seconds: num(input.get("min_seconds")),
        max_seconds: num(input.get("max_seconds")),
        starred: None,
    }
}

/// A result as the agent reads it: only what's there, to keep the answer short.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FileDescription {
    pub path: String,
    pub kind: LibraryKind,
    pub library: linker_edit_protocol::LibraryScopeKind,
    pub folder: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder_note: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "js::opt_num"
    )]
    pub seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channels: Option<u32>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "js::opt_num"
    )]
    pub loudness_lufs: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub star: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used_in: Option<Vec<String>>,
}

/// folder_label and folder_note are the label and note of the file's folder.
pub fn describe_file(file: &LibraryFile, folder_label: &str, folder_note: &str) -> FileDescription {
    let some = |text: &str| Some(text.to_string()).filter(|text| !text.is_empty());
    FileDescription {
        path: file.path.clone(),
        kind: file.kind,
        library: file.scope,
        folder: folder_label.to_string(),
        folder_note: some(folder_note),
        seconds: file.duration.map(|seconds| js::round_to(seconds, 2)),
        channels: file.channels,
        loudness_lufs: file.loudness.map(|lufs| js::round_to(lufs, 1)),
        star: file.star.then_some(true),
        tags: Some(file.tags.clone()).filter(|tags| !tags.is_empty()),
        note: some(&file.note),
        used_in: Some(
            file.used_in
                .iter()
                .map(|used| match &used.timeline {
                    Some(timeline) if !timeline.is_empty() => {
                        format!("{} · {timeline}", used.project)
                    }
                    _ => used.project.clone(),
                })
                .collect::<Vec<_>>(),
        )
        .filter(|used| !used.is_empty()),
    }
}

/// What library_import needs from the turn it runs in.
#[async_trait]
pub trait ImportContext: Send + Sync {
    /// The library file at a path in the turn's project's chain, or None.
    fn find(&self, file: &str) -> Option<LibraryFile>;
    /// Runs one of Linker-Edit's Resolve tools for the turn, returning its text; an error says what went wrong.
    async fn resolve(&self, tool: &str, input: Value) -> anyhow::Result<String>;
    /// Waits for the user's OK in supervised mode and backs the project up; a refusal's text, or None to go on.
    async fn allow_change(&self, input: Value) -> anyhow::Result<Option<String>>;
    /// Notes the file was used: the caller adds the time (LibraryUse.at).
    fn record_use(&self, file: &str, project_id: &str, project: &str, timeline: Option<&str>);
    /// The turn's project: its id and name.
    fn project(&self) -> (String, String);
    /// The timeline open in Resolve, for "used in" when the agent names none.
    fn current_timeline(&self) -> Option<String>;
}

#[derive(Debug, Clone, Default, Deserialize)]
struct PoolClip {
    #[serde(default)]
    id: Value,
    #[serde(default)]
    name: Value,
    #[serde(default)]
    bin: Option<String>,
    #[serde(default)]
    file_path: Option<Value>,
}

impl PoolClip {
    fn file_path(&self) -> Option<&str> {
        self.file_path
            .as_ref()
            .and_then(Value::as_str)
            .filter(|path| !path.is_empty())
    }
}

/// A tool's JSON answer, which Linker-Edit may have shortened with a note after it.
fn parse_result<T: for<'de> Deserialize<'de>>(text: &str) -> anyhow::Result<T> {
    let json = text.split("\nLinker-Edit shortened").next().unwrap_or(text);
    Ok(serde_json::from_str(json)?)
}

fn base_name(file: &str) -> String {
    Path::new(file)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The media pool clip of a file already in the project, found by its file path.
async fn find_in_pool(context: &dyn ImportContext, file: &str) -> anyhow::Result<Option<PoolClip>> {
    #[derive(Deserialize)]
    struct Listed {
        #[serde(default)]
        clips: Option<Vec<PoolClip>>,
    }
    let text = context
        .resolve(
            "list_media",
            json!({ "search": base_name(file), "limit": 500 }),
        )
        .await?;
    let listed: Listed = parse_result(&text)?;
    Ok(listed.clips.unwrap_or_default().into_iter().find(|clip| {
        clip.file_path()
            .is_some_and(|path| path_key(path) == path_key(file))
    }))
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ImportedFile {
    pub path: String,
    pub media_id: Value,
    pub name: Value,
    pub bin: Option<String>,
    pub reused: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ImportResult {
    pub files: Vec<ImportedFile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub luts: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub luts_note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_imported: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_in_library: Option<Vec<String>>,
}

pub async fn library_import(
    input: &Map<String, Value>,
    context: &dyn ImportContext,
) -> anyhow::Result<ImportResult> {
    let asked = strings(input.get("files"));
    if asked.is_empty() {
        bail!("Pass the files to import in files: absolute paths from library_search.");
    }
    let bin = text(input.get("bin"));
    let timeline = text(input.get("timeline")).or_else(|| context.current_timeline());

    let mut not_in_library = Vec::new();
    let mut luts = Vec::new();
    let mut media: Vec<LibraryFile> = Vec::new();
    for file in asked {
        let found = if js::is_absolute(&file) {
            context.find(&file)
        } else {
            None
        };
        match found {
            None => not_in_library.push(file),
            Some(found) if found.kind == LibraryKind::Luts => luts.push(found.path),
            Some(found) => {
                if !media
                    .iter()
                    .any(|other| path_key(&other.path) == path_key(&found.path))
                {
                    media.push(found);
                }
            }
        }
    }
    if media.is_empty() && luts.is_empty() {
        bail!(
            "These aren't files in this project's library: {}. Find them with library_search, or use import_media for other files.",
            not_in_library.join(", ")
        );
    }

    let mut done: Vec<ImportedFile> = Vec::new();
    let mut missing: Vec<LibraryFile> = Vec::new();
    for file in media {
        match find_in_pool(context, &file.path).await? {
            Some(clip) => done.push(ImportedFile {
                path: file.path.clone(),
                media_id: clip.id,
                name: clip.name,
                bin: clip.bin,
                reused: true,
            }),
            None => missing.push(file),
        }
    }

    if !missing.is_empty() {
        let mut by_bin: Vec<(String, Vec<&LibraryFile>)> = Vec::new();
        for file in &missing {
            let target = bin
                .clone()
                .unwrap_or_else(|| format!("Library/{}", kind_bin(file.kind)));
            match by_bin.iter_mut().find(|(name, _)| *name == target) {
                Some((_, files)) => files.push(file),
                None => by_bin.push((target, vec![file])),
            }
        }
        let refusal = context
            .allow_change(json!({
                "files": missing.iter().map(|file| file.path.clone()).collect::<Vec<_>>(),
                "bins": by_bin.iter().map(|(name, _)| name.clone()).collect::<Vec<_>>(),
            }))
            .await?;
        if let Some(refusal) = refusal {
            bail!(refusal);
        }
        #[derive(Deserialize)]
        struct Imported {
            #[serde(default)]
            imported: Option<Vec<PoolClip>>,
        }
        for (target, files) in &by_bin {
            let text = context
                .resolve(
                    "import_media",
                    json!({
                        "paths": files.iter().map(|file| file.path.clone()).collect::<Vec<_>>(),
                        "bin": target,
                    }),
                )
                .await?;
            let imported = parse_result::<Imported>(&text)?
                .imported
                .unwrap_or_default();
            for file in files {
                // Resolve names a clip after its file, and some versions leave its path empty right after an import.
                let clip = imported
                    .iter()
                    .find(|item| {
                        item.file_path()
                            .is_some_and(|path| path_key(path) == path_key(&file.path))
                    })
                    .or_else(|| {
                        imported.iter().find(|item| {
                            item.file_path().is_none()
                                && item.name.as_str() == Some(base_name(&file.path).as_str())
                        })
                    });
                if let Some(clip) = clip {
                    done.push(ImportedFile {
                        path: file.path.clone(),
                        media_id: clip.id.clone(),
                        name: clip.name.clone(),
                        bin: Some(target.clone()),
                        reused: false,
                    });
                }
            }
        }
    }

    let (project_id, project) = context.project();
    for entry in &done {
        context.record_use(&entry.path, &project_id, &project, timeline.as_deref());
    }
    for lut in &luts {
        context.record_use(lut, &project_id, &project, timeline.as_deref());
    }

    let failed: Vec<String> = missing
        .iter()
        .filter(|file| {
            !done
                .iter()
                .any(|entry| path_key(&entry.path) == path_key(&file.path))
        })
        .map(|file| file.path.clone())
        .collect();
    let some = |list: Vec<String>| Some(list).filter(|list| !list.is_empty());
    let has_luts = !luts.is_empty();
    let has_failed = !failed.is_empty();
    Ok(ImportResult {
        files: done,
        luts: some(luts),
        luts_note: has_luts.then(|| {
            "LUTs don't go in the media pool. Apply one with set_color's lut, using its path."
                .to_string()
        }),
        not_imported: some(failed),
        note: has_failed
            .then(|| "Resolve didn't import these. Are they media Resolve can read?".to_string()),
        not_in_library: some(not_in_library),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use linker_edit_protocol::{LibraryScopeKind, LibraryUse};

    use super::*;

    fn library_file(path: &str, kind: LibraryKind) -> LibraryFile {
        LibraryFile {
            folder_id: "music".into(),
            scope: LibraryScopeKind::Universal,
            kind,
            rel: base_name(path),
            name: base_name(path),
            path: path.into(),
            size: 1,
            modified: "2026-11-15T10:00:00.000Z".into(),
            duration: Some(30.0),
            channels: Some(2),
            loudness: Some(-14.0),
            tags: vec![],
            star: false,
            note: String::new(),
            used_in: vec![],
        }
    }

    /// An absolute path on this computer.
    fn p(path: &str) -> String {
        if cfg!(windows) {
            format!("C:{}", path.replace('/', "\\"))
        } else {
            path.to_string()
        }
    }

    /// Stands in for the turn: Song.wav is in the media pool already, the rest imports.
    struct Turn {
        files: Vec<LibraryFile>,
        calls: Mutex<Vec<(String, Value)>>,
        uses: Mutex<Vec<(String, Option<String>)>>,
        refuse: bool,
    }

    #[async_trait]
    impl ImportContext for Turn {
        fn find(&self, file: &str) -> Option<LibraryFile> {
            self.files.iter().find(|f| f.path == file).cloned()
        }
        async fn resolve(&self, tool: &str, input: Value) -> anyhow::Result<String> {
            self.calls
                .lock()
                .unwrap()
                .push((tool.into(), input.clone()));
            Ok(match tool {
                "list_media" if input["search"] == "Song.wav" => json!({ "clips": [{ "id": "m-song", "name": "Song.wav", "bin": "Master/Music", "file_path": p("/music/Song.wav") }] }).to_string() + "\nLinker-Edit shortened this.",
                "list_media" => json!({ "clips": [] }).to_string(),
                _ => json!({ "bin": input["bin"], "imported": input["paths"].as_array().unwrap().iter().map(|path| json!({ "id": "imported-0", "name": base_name(path.as_str().unwrap()), "file_path": "" })).collect::<Vec<_>>() }).to_string(),
            })
        }
        async fn allow_change(&self, _: Value) -> anyhow::Result<Option<String>> {
            Ok(self.refuse.then(|| "The user said no.".to_string()))
        }
        fn record_use(&self, file: &str, _: &str, project: &str, timeline: Option<&str>) {
            self.uses
                .lock()
                .unwrap()
                .push((file.into(), timeline.map(|t| format!("{project} · {t}"))));
        }
        fn project(&self) -> (String, String) {
            ("workspace".into(), "Doc".into())
        }
        fn current_timeline(&self) -> Option<String> {
            Some("Edit".into())
        }
    }

    fn turn(refuse: bool) -> Turn {
        Turn {
            files: vec![
                library_file(&p("/music/Song.wav"), LibraryKind::Music),
                library_file(&p("/music/Fresh.wav"), LibraryKind::Music),
                library_file(&p("/sfx/Whoosh.wav"), LibraryKind::Sfx),
                library_file(&p("/luts/Teal.cube"), LibraryKind::Luts),
            ],
            calls: Mutex::default(),
            uses: Mutex::default(),
            refuse,
        }
    }

    fn input(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[tokio::test]
    async fn imports_into_library_bins_reuses_clips_in_the_media_pool_and_notes_where_files_were_used()
     {
        let context = turn(false);
        let result = library_import(
            &input(json!({ "files": [p("/music/Song.wav"), p("/music/Fresh.wav"), p("/sfx/Whoosh.wav"), p("/luts/Teal.cube"), p("/elsewhere/Other.wav")] })),
            &context,
        )
        .await
        .unwrap();
        assert_eq!(
            serde_json::to_value(&result).unwrap(),
            json!({
                "files": [
                    { "path": p("/music/Song.wav"), "media_id": "m-song", "name": "Song.wav", "bin": "Master/Music", "reused": true },
                    { "path": p("/music/Fresh.wav"), "media_id": "imported-0", "name": "Fresh.wav", "bin": "Library/Music", "reused": false },
                    { "path": p("/sfx/Whoosh.wav"), "media_id": "imported-0", "name": "Whoosh.wav", "bin": "Library/Sound effects", "reused": false },
                ],
                "luts": [p("/luts/Teal.cube")],
                "luts_note": "LUTs don't go in the media pool. Apply one with set_color's lut, using its path.",
                "not_in_library": [p("/elsewhere/Other.wav")],
            })
        );
        let imports: Vec<_> = context
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(tool, _)| tool == "import_media")
            .map(|(_, input)| input["bin"].clone())
            .collect();
        assert_eq!(
            imports,
            [json!("Library/Music"), json!("Library/Sound effects")]
        );
        assert_eq!(context.uses.lock().unwrap().len(), 4);
        assert_eq!(
            context.uses.lock().unwrap()[0].1.as_deref(),
            Some("Doc · Edit")
        );

        let refused = library_import(
            &input(json!({ "files": [p("/music/Fresh.wav")] })),
            &turn(true),
        )
        .await;
        assert_eq!(refused.unwrap_err().to_string(), "The user said no.");
        let none = library_import(&input(json!({ "files": ["relative.wav"] })), &turn(false)).await;
        assert!(
            none.unwrap_err()
                .to_string()
                .starts_with("These aren't files in this project's library: relative.wav.")
        );
    }

    #[test]
    fn describes_a_file_with_only_whats_there() {
        let mut file = library_file("/music/Song.wav", LibraryKind::Music);
        file.duration = Some(30.004);
        file.loudness = Some(-14.25);
        file.used_in = vec![
            LibraryUse {
                project_id: "p".into(),
                project: "Doc".into(),
                timeline: Some("Edit".into()),
                at: "x".into(),
            },
            LibraryUse {
                project_id: "p".into(),
                project: "Doc".into(),
                timeline: None,
                at: "x".into(),
            },
        ];
        assert_eq!(
            serde_json::to_string(&describe_file(&file, "Songs", "Licensed for YouTube only"))
                .unwrap(),
            r#"{"path":"/music/Song.wav","kind":"music","library":"universal","folder":"Songs","folder_note":"Licensed for YouTube only","seconds":30,"channels":2,"loudness_lufs":-14.2,"used_in":["Doc · Edit","Doc"]}"#
        );
    }

    #[test]
    fn reads_a_query_from_the_agents_input() {
        let query = query_from_input(&input(
            json!({ "kind": "sfx", "query": " ", "tags": ["a", " ", 3], "min_seconds": 1, "max_seconds": "long" }),
        ));
        assert_eq!(
            query,
            LibraryQuery {
                kind: Some(LibraryKind::Sfx),
                tags: Some(vec!["a".into()]),
                min_seconds: Some(1.0),
                ..LibraryQuery::default()
            }
        );
    }
}
