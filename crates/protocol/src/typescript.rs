// apps/desktop/shared/protocol.ts, written from the Rust types so the Svelte UI and this crate can't drift apart.
// tests/typescript.rs fails when the checked-in file is stale; UPDATE_PROTOCOL_TS=1 rewrites it.

use std::any::TypeId;
use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;
use ts_rs::{Config, TS};

use crate::*;

// ts-rs writes every doc comment over three lines; protocol.ts had one-line ones on one line.
static ONE_LINE_DOC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"/\*\*\n \*([^\n]*)\n \*/").unwrap());

const HEADER: &str = "\
// Messages exchanged between the local server and the browser UI over /ws.
// The iPad app (apps/ipad) reads them too: after a change, run
// RECORD_IPAD_FIXTURES=1 cargo test -p linker-edit-server ipad_fixtures and push, so its CI decodes the new messages.
//
// Generated from crates/protocol: don't edit it here. Change the Rust types, then run
// `UPDATE_PROTOCOL_TS=1 cargo test -p linker-edit-protocol` to write this file again.

";

/// The TypeScript for everything on /ws, in protocol.ts's order, before formatting.
pub fn typescript() -> String {
    let mut file = File {
        cfg: Config::new(),
        out: HEADER.to_string(),
        declared: HashSet::new(),
        needed: Vec::new(),
    };
    file.decl::<Provider>();
    file.decl::<ApprovalMode>();
    file.decl::<ToolStatus>();
    file.decl::<ChatItem>();
    file.decl::<ResolveStatus>();
    file.decl::<ResolveDatabase>();
    file.decl::<ResolveLink>();
    file.decl::<LocalLink>();
    file.decl::<ThreadSummary>();
    file.decl::<ProjectSummary>();
    file.decl::<LibraryProject>();
    file.decl::<ResolveLibrary>();
    file.decl::<ProjectSource>();
    file.decl::<FolderEntry>();
    file.decl::<FolderListing>();
    file.decl::<ApprovalRequest>();
    file.decl::<ClipChange>();
    file.decl::<StripTile>();
    file.decl::<TimelineStrip>();
    file.decl::<HistoryEntry>();
    file.decl::<MediaKind>();
    file.decl::<PlayableClip>();
    file.decl::<PlayableTimeline>();
    file.decl::<PreviewTimeline>();
    file.decl::<PreviewState>();
    file.decl::<ContextUsage>();
    file.constant(
        "/** Below this, rereading a chat after its cache expired is cheap, so compacting isn't suggested. As in T3 Code. */",
        "COMPACT_SUGGESTION_MIN_TOKENS",
        &thousands(COMPACT_SUGGESTION_MIN_TOKENS),
    );
    file.decl::<TranscriptionStatus>();
    file.decl::<RemoteAccess>();
    file.decl::<LinkerAccount>();
    file.decl::<Computer>();
    file.decl::<LinkedComputer>();
    file.decl::<ConnectDeviceKind>();
    file.decl::<ConnectDevice>();
    file.decl::<LinkerConnect>();
    file.decl::<Computers>();
    file.decl::<DesktopUpdate>();
    file.decl::<DesktopUpdateState>();
    file.decl::<AppUpdate>();
    file.decl::<ComputerSetup>();
    file.decl::<AppMeta>();
    file.decl::<AppState>();
    file.decl::<ClientMessage>();
    file.decl::<DesktopToServer>();
    file.decl::<ServerToDesktop>();
    file.decl::<ServerMessage>();

    file.comment(
        "// --- Library ---\n\
         // Folders of files the user uses again and again (music, sound effects, logos, LUTs…). Linker-Edit only points\n\
         // at them: it never copies, moves or deletes a file in them.",
    );
    file.constant("", "LIBRARY_KINDS", &strings(&LIBRARY_KINDS));
    file.decl::<LibraryKind>();
    file.decl::<LibraryFolder>();
    file.decl::<LibraryScope>();
    file.decl::<LibraryUse>();
    file.decl::<LibraryFile>();
    file.decl::<LibraryFolderStatus>();
    file.decl::<LibraryQuery>();
    file.decl::<LibraryView>();
    file.decl::<LibraryClientMessage>();
    file.decl::<LibraryServerMessage>();

    file.comment("// --- Workspaces & series ---");
    file.decl::<CollectionKind>();
    file.decl::<IconColor>();
    file.decl::<CollectionIcon>();
    file.decl::<CollectionSummary>();
    file.decl::<Lesson>();
    file.decl::<ApprovedEdit>();
    file.decl::<CollectionMessage>();

    file.comment(
        "// --- Sidebar (T3 Code) ---\n\
         // Pinning, settling, snoozing and unread threads, and project icons, as T3 Code's sidebar has them (MIT, see NOTICE).",
    );
    file.constant(
        "/** T3 Code's project icon colors. */",
        "PROJECT_ICON_COLORS",
        &strings(&PROJECT_ICON_COLORS),
    );
    file.decl::<ProjectIconColor>();
    file.decl::<ProjectIcon>();
    file.decl::<ThreadSidebarState>();
    file.decl::<ProjectSidebarState>();
    file.decl::<ProjectSidebarChanges>();
    file.decl::<SidebarClientMessage>();

    // Every type a declaration names has to be declared too, or the file wouldn't compile.
    let missing: Vec<&str> = file
        .needed
        .iter()
        .filter(|(id, _)| !file.declared.contains(id))
        .map(|(_, name)| name.as_str())
        .collect();
    assert!(
        missing.is_empty(),
        "protocol.ts uses types it doesn't declare: {missing:?}"
    );
    ONE_LINE_DOC.replace_all(&file.out, "/**$1 */").into_owned()
}

struct File {
    cfg: Config,
    out: String,
    declared: HashSet<TypeId>,
    needed: Vec<(TypeId, String)>,
}

impl File {
    fn decl<T: TS + 'static>(&mut self) {
        self.out.push_str(&T::docs().unwrap_or_default());
        self.out.push_str("export ");
        self.out.push_str(&one_member_per_line(&T::decl(&self.cfg)));
        self.out.push_str("\n\n");
        self.declared.insert(TypeId::of::<T>());
        for dependency in T::dependencies(&self.cfg) {
            self.needed.push((dependency.type_id, dependency.ts_name));
        }
    }

    fn constant(&mut self, docs: &str, name: &str, value: &str) {
        if !docs.is_empty() {
            self.out.push_str(docs);
            self.out.push('\n');
        }
        self.out
            .push_str(&format!("export const {name} = {value};\n\n"));
    }

    fn comment(&mut self, text: &str) {
        self.out.push_str(text);
        self.out.push_str("\n\n");
    }
}

/// A union of objects with each member on its own line, as protocol.ts had them; the formatter keeps line breaks
/// it's given. Unions of strings stay on one line.
fn one_member_per_line(decl: &str) -> String {
    let Some((name, union)) = decl.split_once(" = ") else {
        return decl.to_string();
    };
    if !union.starts_with('{') {
        return decl.to_string();
    }
    let mut out = format!("{name} =\n  | ");
    let mut depth = 0;
    let mut members = 1;
    let mut rest = union;
    while let Some(c) = rest.chars().next() {
        if rest.starts_with("/*") {
            let end = rest.find("*/").map_or(rest.len(), |end| end + 2);
            out.push_str(&rest[..end]);
            rest = &rest[end..];
            continue;
        }
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            _ => {}
        }
        if depth == 0 && rest.starts_with(" | ") {
            out.push_str("\n  | ");
            members += 1;
            rest = &rest[3..];
            continue;
        }
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    if members == 1 { decl.to_string() } else { out }
}

/// A list of string enum values as a TypeScript `as const` array.
fn strings<T: serde::Serialize>(values: &[T]) -> String {
    format!("{} as const", serde_json::to_string(values).unwrap())
}

/// 200000 as 200_000.
fn thousands(value: u32) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push('_');
        }
        out.push(digit);
    }
    out
}
