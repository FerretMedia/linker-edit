// Which Resolve project a Linker-Edit project belongs to, and whether it is the one open in Resolve.

use linker_edit_protocol::{
    LocalLink, ResolveDatabase, ResolveLink, ResolveLinkKind, ResolveStatus,
};

/// The project open in Resolve, as the status probe reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenProject {
    pub name: String,
    pub id: Option<String>,
    pub database: Option<ResolveDatabase>,
}

pub fn open_project(status: &ResolveStatus) -> Option<OpenProject> {
    let ResolveStatus::Connected {
        project: Some(project),
        project_id,
        database,
        ..
    } = status
    else {
        return None;
    };
    if project.is_empty() {
        return None;
    }
    Some(OpenProject {
        name: project.clone(),
        id: project_id.clone(),
        database: database.clone(),
    })
}

// Resolve's scripting API documents only "Disk" and "PostgreSQL"; what it reports for a Blackmagic
// Cloud library is unverified, so any other type counts as cloud.
pub fn link_kind(database: Option<&ResolveDatabase>) -> ResolveLinkKind {
    let Some(database) = database else {
        return ResolveLinkKind::Local;
    };
    match database.r#type.to_lowercase().as_str() {
        "disk" => ResolveLinkKind::Local,
        "postgresql" => ResolveLinkKind::Network,
        _ => ResolveLinkKind::Cloud,
    }
}

fn same_database(a: &ResolveDatabase, b: &ResolveDatabase) -> bool {
    a.r#type.to_lowercase() == b.r#type.to_lowercase()
        && a.name == b.name
        && a.host.as_deref().unwrap_or_default() == b.host.as_deref().unwrap_or_default()
}

/// Is the open project this link's? By Resolve's project id once both sides know it, so a rename in
/// Resolve still matches; by name and library before that.
pub fn is_linked_project(link: &ResolveLink, local: &LocalLink, open: &OpenProject) -> bool {
    let known = |id: &Option<String>| {
        id.as_deref()
            .filter(|id| !id.is_empty())
            .map(str::to_string)
    };
    if let (Some(linked), Some(open_id)) = (known(&link.project_id), known(&open.id)) {
        return linked == open_id;
    }
    if link.name != open.name || link.kind != link_kind(open.database.as_ref()) {
        return false;
    }
    match (&local.database, &open.database) {
        (Some(local), Some(open)) => same_database(local, open),
        _ => true,
    }
}

/// A link and this machine's side of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Linked {
    pub link: ResolveLink,
    pub local: LocalLink,
}

pub fn link_from_open(open: &OpenProject) -> Linked {
    Linked {
        link: ResolveLink {
            kind: link_kind(open.database.as_ref()),
            name: open.name.clone(),
            project_id: open.id.clone(),
        },
        local: LocalLink {
            database: open.database.clone(),
            folder: None,
        },
    }
}

/// The link brought up to date with what Resolve reports for the same project: its id, and a rename.
pub fn refreshed_link(link: &ResolveLink, local: &LocalLink, open: &OpenProject) -> Linked {
    Linked {
        link: ResolveLink {
            name: open.name.clone(),
            project_id: open.id.clone().or_else(|| link.project_id.clone()),
            ..link.clone()
        },
        local: match &open.database {
            Some(database) => LocalLink {
                database: Some(database.clone()),
                ..local.clone()
            },
            None => local.clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db(r#type: &str, name: &str, host: Option<&str>) -> ResolveDatabase {
        ResolveDatabase {
            r#type: r#type.into(),
            name: name.into(),
            host: host.map(str::to_string),
        }
    }

    fn disk() -> ResolveDatabase {
        db("Disk", "Local Database", None)
    }

    #[test]
    fn link_kind_by_database_type() {
        assert_eq!(link_kind(None), ResolveLinkKind::Local);
        assert_eq!(link_kind(Some(&disk())), ResolveLinkKind::Local);
        assert_eq!(
            link_kind(Some(&db("PostgreSQL", "Team", Some("10.0.0.5")))),
            ResolveLinkKind::Network
        );
        assert_eq!(
            link_kind(Some(&db("Cloud", "Blackmagic Cloud", None))),
            ResolveLinkKind::Cloud
        );
    }

    #[test]
    fn is_linked_project_cases() {
        let link = |kind: ResolveLinkKind, id: Option<&str>| ResolveLink {
            kind,
            name: "Doc".into(),
            project_id: id.map(str::to_string),
        };
        let open = |name: &str, id: Option<&str>| OpenProject {
            name: name.into(),
            id: id.map(str::to_string),
            database: Some(disk()),
        };
        let local = |database: Option<ResolveDatabase>| LocalLink {
            database,
            folder: None,
        };
        use ResolveLinkKind::{Cloud, Local};
        let cases = [
            (
                "the same id, even after a rename in Resolve",
                link(Local, Some("doc-id")),
                local(None),
                open("Doc v2", Some("doc-id")),
                true,
            ),
            (
                "another id with the same name",
                link(Local, Some("other")),
                local(None),
                open("Doc", Some("doc-id")),
                false,
            ),
            (
                "the name, before the id is known",
                link(Local, None),
                local(None),
                open("Doc", Some("doc-id")),
                true,
            ),
            (
                "the name, when Resolve reports no id",
                link(Local, Some("doc-id")),
                local(None),
                open("Doc", None),
                true,
            ),
            (
                "another name",
                link(Local, None),
                local(None),
                open("Promo", Some("doc-id")),
                false,
            ),
            (
                "the name in another kind of library",
                link(Cloud, None),
                local(None),
                open("Doc", Some("doc-id")),
                false,
            ),
            (
                "the name in this machine's database",
                link(Local, None),
                local(Some(disk())),
                open("Doc", Some("doc-id")),
                true,
            ),
            (
                "the name in another local database",
                link(Local, None),
                local(Some(db("Disk", "Archive", None))),
                open("Doc", Some("doc-id")),
                false,
            ),
        ];
        for (name, link, local, open, expected) in cases {
            assert_eq!(
                is_linked_project(&link, &local, &open),
                expected,
                "matches {name}"
            );
        }
    }
}
