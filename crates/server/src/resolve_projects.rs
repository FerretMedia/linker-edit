// Resolve's project library: listing its projects, and opening the one a Linker-Edit project belongs to.
// The scripts are crates/server/src/resolve/scripts/list_projects.py and open_project.py.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use linker_edit_protocol::{LocalLink, ResolveDatabase, ResolveLibrary, ResolveLink};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::resolve::connection::{ResolveConnection, ResolveErrorKind, error_kind};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenedProjectName {
    pub name: String,
    pub id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenedProject {
    pub project: OpenedProjectName,
    pub database: Option<ResolveDatabase>,
    /// The Project Manager folders it was found in; None when that isn't known.
    pub folder: Option<Vec<String>>,
}

#[async_trait]
pub trait ResolveProjects: Send + Sync {
    async fn library(&self) -> anyhow::Result<ResolveLibrary>;
    /// Saves Resolve's open project, then opens this one.
    async fn open(&self, link: &ResolveLink, local: &LocalLink) -> anyhow::Result<OpenedProject>;
}

/// How long opening may take: a big project can take minutes to load.
const OPEN_TIMEOUT: Duration = Duration::from_secs(5 * 60);

pub struct ScriptResolveProjects {
    connection: Arc<dyn ResolveConnection>,
    poll: Duration,
}

/// Resolve's projects through its scripts; poll is how often a slow open is checked on (3 seconds in the app).
pub fn create_resolve_projects(
    connection: Arc<dyn ResolveConnection>,
    poll: Duration,
) -> ScriptResolveProjects {
    ScriptResolveProjects { connection, poll }
}

#[async_trait]
impl ResolveProjects for ScriptResolveProjects {
    async fn library(&self) -> anyhow::Result<ResolveLibrary> {
        self.connection
            .run_script("list_projects", json!({}), 60)
            .await
    }

    async fn open(&self, link: &ResolveLink, local: &LocalLink) -> anyhow::Result<OpenedProject> {
        let args = json!({
            "kind": link.kind,
            "name": link.name,
            "projectId": link.project_id,
            "database": local.database,
            "folder": local.folder.clone().unwrap_or_default(),
        });
        match self.connection.run_script("open_project", args, 60).await {
            Ok(opened) => Ok(opened),
            // Scripts stop after 60 seconds, but Resolve goes on opening: wait for the project to show up.
            Err(error)
                if matches!(
                    error_kind(&error),
                    Some(ResolveErrorKind::Timeout | ResolveErrorKind::Unavailable)
                ) =>
            {
                wait_for_project(self.connection.as_ref(), link, self.poll).await
            }
            Err(error) => Err(error),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    #[serde(default)]
    project: Option<String>,
    #[serde(default)]
    project_id: Option<String>,
    #[serde(default)]
    database: Option<ResolveDatabase>,
}

async fn wait_for_project(
    connection: &dyn ResolveConnection,
    link: &ResolveLink,
    poll: Duration,
) -> anyhow::Result<OpenedProject> {
    let deadline = Instant::now() + OPEN_TIMEOUT;
    while Instant::now() < deadline {
        tokio::time::sleep(poll).await;
        let Ok(status) = connection
            .run_script::<Status>("status", json!({}), 10)
            .await
        else {
            continue;
        };
        let Some(project) = status.project.filter(|project| !project.is_empty()) else {
            continue;
        };
        let same = match &link.project_id {
            Some(id) if !id.is_empty() => status.project_id.as_deref() == Some(id.as_str()),
            _ => project == link.name,
        };
        if same {
            return Ok(OpenedProject {
                project: OpenedProjectName {
                    name: project,
                    id: status.project_id,
                },
                database: status.database,
                folder: None,
            });
        }
    }
    anyhow::bail!(
        "Resolve didn't open \"{}\" within 5 minutes. Is a dialog open in Resolve?",
        link.name
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Mutex;

    use linker_edit_protocol::{LibraryProject, ResolveLinkKind};
    use serde_json::{Value, json};

    use super::*;
    use crate::resolve::connection::ResolveError;
    use crate::resolve::testing::{fake_connection, has_python, logged_calls, python_connection};

    // Stands in for Resolve's scripting module: a Project Manager over folder trees from a JSON file,
    // logging every call that changes something.
    const FAKE_MODULE: &str = r#"
import json, os
STATE = json.load(open(os.environ["FAKE_RESOLVE"], encoding="utf8"))
def log(*args):
    with open(os.environ["FAKE_RESOLVE"] + ".log", "a", encoding="utf8") as f:
        f.write(json.dumps(args) + "\n")

class Project:
    def __init__(self, name, id):
        self.name, self.id = name, id
    def GetName(self): return self.name
    def GetUniqueId(self): return self.id

class PM:
    def __init__(self):
        self.path, self.db = [], STATE["database"]
        self.current = Project(*STATE["current"]) if STATE.get("current") else None
    def tree(self):
        node = STATE["trees"][self.db["DbName"]]
        for name in self.path:
            node = node["folders"][name]
        return node
    def GetCurrentDatabase(self): return self.db
    def GetDatabaseList(self): return STATE["databaseList"]
    def SetCurrentDatabase(self, info):
        log("SetCurrentDatabase", info)
        if info["DbName"] not in STATE["trees"]:
            return False
        self.db, self.current, self.path = info, None, []
        return True
    def GotoRootFolder(self):
        self.path = []
        return True
    def GotoParentFolder(self):
        self.path = self.path[:-1]
        return True
    def OpenFolder(self, name):
        if name not in self.tree().get("folders", {}):
            return False
        self.path = self.path + [name]
        return True
    def GetProjectListInCurrentFolder(self): return list(self.tree().get("projects", {}))
    def GetFolderListInCurrentFolder(self): return list(self.tree().get("folders", {}))
    def GetCurrentProject(self): return self.current
    def SaveProject(self):
        log("SaveProject", self.current.GetName())
        return True
    def LoadProject(self, name):
        projects = self.tree().get("projects", {})
        if name not in projects:
            return None
        log("LoadProject", self.path, name)
        self.current = Project(name, projects[name])
        return self.current
    def LoadCloudProject(self, settings):
        log("LoadCloudProject", settings)
        cloud = STATE.get("cloud", {})
        name = settings.get("name")
        if name not in cloud:
            return None
        self.current = Project(name, cloud[name])
        return self.current

PM_INSTANCE = PM()

class Resolve:
    CLOUD_SETTING_PROJECT_NAME = "name"
    CLOUD_SETTING_PROJECT_MEDIA_PATH = "media"
    CLOUD_SETTING_SYNC_MODE = "sync"
    CLOUD_SYNC_NONE = 0
    CLOUD_SYNC_PROXY_ONLY = 1
    CLOUD_SYNC_PROXY_AND_ORIG = 2
    def GetProjectManager(self): return PM_INSTANCE

def scriptapp(name): return Resolve()
"#;

    fn local_db() -> Value {
        json!({ "DbType": "Disk", "DbName": "Local Database" })
    }

    fn team_db() -> Value {
        json!({ "DbType": "PostgreSQL", "DbName": "Team", "IpAddress": "10.0.0.5" })
    }

    struct Fake {
        _dir: tempfile::TempDir,
        state: PathBuf,
        projects: ScriptResolveProjects,
    }

    impl Fake {
        fn calls(&self) -> Vec<Value> {
            logged_calls(&self.state)
        }
    }

    fn fake_resolve(state: Value) -> Fake {
        let dir = tempfile::Builder::new()
            .prefix("linker-edit-fake-resolve-")
            .tempdir()
            .unwrap();
        let file = dir.path().join("state.json");
        let mut value = json!({
            "database": local_db(),
            "databaseList": [local_db(), team_db()],
            "current": ["Promo", "promo-id"],
            "trees": {
                "Local Database": {
                    "projects": { "Promo": "promo-id" },
                    "folders": { "Clients": { "folders": { "2026": { "projects": { "Interview": "int-id" } } } } }
                },
                "Team": { "projects": { "Doc": "doc-id" } }
            }
        });
        if let Value::Object(state) = state {
            value.as_object_mut().unwrap().extend(state);
        }
        std::fs::write(&file, value.to_string()).unwrap();
        let connection =
            python_connection(FAKE_MODULE, &[("FAKE_RESOLVE", file.to_str().unwrap())]);
        Fake {
            _dir: dir,
            state: file,
            projects: create_resolve_projects(connection, Duration::from_secs(3)),
        }
    }

    fn db(r#type: &str, name: &str, host: Option<&str>) -> ResolveDatabase {
        ResolveDatabase {
            r#type: r#type.into(),
            name: name.into(),
            host: host.map(str::to_string),
        }
    }

    fn link(kind: ResolveLinkKind, name: &str, project_id: Option<&str>) -> ResolveLink {
        ResolveLink {
            kind,
            name: name.into(),
            project_id: project_id.map(str::to_string),
        }
    }

    fn team() -> LocalLink {
        LocalLink {
            database: Some(db("PostgreSQL", "Team", Some("10.0.0.5"))),
            folder: None,
            library: None,
        }
    }

    fn named(name: &str, id: &str) -> OpenedProjectName {
        OpenedProjectName {
            name: name.into(),
            id: Some(id.into()),
        }
    }

    #[tokio::test]
    async fn lists_the_open_databases_projects_with_their_folders_and_the_other_databases() {
        if !has_python() {
            return;
        }
        let fake = fake_resolve(json!({}));
        assert_eq!(
            fake.projects.library().await.unwrap(),
            ResolveLibrary {
                database: Some(db("Disk", "Local Database", None)),
                databases: vec![
                    db("Disk", "Local Database", None),
                    db("PostgreSQL", "Team", Some("10.0.0.5"))
                ],
                projects: vec![
                    LibraryProject {
                        name: "Promo".into(),
                        folder: vec![]
                    },
                    LibraryProject {
                        name: "Interview".into(),
                        folder: vec!["Clients".into(), "2026".into()]
                    },
                ],
            }
        );
        assert_eq!(fake.calls(), Vec::<Value>::new());
    }

    #[tokio::test]
    async fn saves_the_open_project_then_finds_and_opens_another_one_by_searching_its_folders() {
        if !has_python() {
            return;
        }
        let fake = fake_resolve(json!({}));
        let local = LocalLink {
            database: None,
            folder: Some(vec!["Old".into()]),
            library: None,
        };
        assert_eq!(
            fake.projects
                .open(&link(ResolveLinkKind::Local, "Interview", None), &local)
                .await
                .unwrap(),
            OpenedProject {
                project: named("Interview", "int-id"),
                database: Some(db("Disk", "Local Database", None)),
                folder: Some(vec!["Clients".into(), "2026".into()]),
            }
        );
        assert_eq!(
            fake.calls(),
            vec![
                json!(["SaveProject", "Promo"]),
                json!(["LoadProject", ["Clients", "2026"], "Interview"])
            ]
        );
    }

    #[tokio::test]
    async fn switches_to_the_projects_network_database_first() {
        if !has_python() {
            return;
        }
        let fake = fake_resolve(json!({}));
        let opened = fake
            .projects
            .open(
                &link(ResolveLinkKind::Network, "Doc", Some("doc-id")),
                &team(),
            )
            .await
            .unwrap();
        assert_eq!(opened.project, named("Doc", "doc-id"));
        assert_eq!(
            fake.calls(),
            vec![
                json!(["SaveProject", "Promo"]),
                json!(["SetCurrentDatabase", team_db()]),
                json!(["LoadProject", [], "Doc"])
            ]
        );
    }

    #[tokio::test]
    async fn does_nothing_when_the_project_is_already_open() {
        if !has_python() {
            return;
        }
        let fake = fake_resolve(json!({}));
        let opened = fake
            .projects
            .open(
                &link(ResolveLinkKind::Local, "Promo", Some("promo-id")),
                &LocalLink::default(),
            )
            .await
            .unwrap();
        assert_eq!(opened.project, named("Promo", "promo-id"));
        assert_eq!(fake.calls(), Vec::<Value>::new());
    }

    #[tokio::test]
    async fn never_opens_a_blackmagic_cloud_project_itself_only_finds_it_once_youve_opened_it_in_resolve()
     {
        if !has_python() {
            return;
        }
        let none = LocalLink::default();
        let by_name = link(ResolveLinkKind::Cloud, "Client X", None);
        let by_id = link(ResolveLinkKind::Cloud, "Client X", Some("cx-id"));
        let elsewhere = fake_resolve(json!({ "cloud": { "Client X": "cx-id" } }));
        let error = elsewhere
            .projects
            .open(&by_name, &none)
            .await
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("Open \"Client X\" yourself in Resolve's Project Manager"),
            "{error}"
        );
        let error = elsewhere
            .projects
            .open(&by_id, &none)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("terms don't allow scripts"), "{error}");
        assert_eq!(elsewhere.calls(), Vec::<Value>::new());

        let open = fake_resolve(json!({ "current": ["Client X", "cx-id"] }));
        assert_eq!(
            open.projects.open(&by_name, &none).await.unwrap().project,
            named("Client X", "cx-id")
        );
        assert_eq!(
            open.projects.open(&by_id, &none).await.unwrap().project,
            named("Client X", "cx-id")
        );
        assert_eq!(open.calls(), Vec::<Value>::new());
    }

    #[tokio::test]
    async fn says_when_resolve_cant_find_the_project_back_in_the_project_it_had_open() {
        if !has_python() {
            return;
        }
        let fake = fake_resolve(json!({}));
        let error = fake
            .projects
            .open(&link(ResolveLinkKind::Network, "Gone", None), &team())
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Resolve couldn't find or open the project \"Gone\"."),
            "{error}"
        );
        assert_eq!(
            fake.calls(),
            vec![
                json!(["SaveProject", "Promo"]),
                json!(["SetCurrentDatabase", team_db()]),
                json!(["SetCurrentDatabase", local_db()]),
                json!(["LoadProject", [], "Promo"])
            ]
        );
    }

    #[tokio::test]
    async fn waits_for_resolve_to_show_a_project_that_takes_longer_to_open_than_a_script_may_run() {
        let polls = Arc::new(Mutex::new(0));
        let counter = polls.clone();
        let connection = Arc::new(fake_connection().script(move |name, _| {
            if name == "open_project" {
                return Err(ResolveError::new(
                    ResolveErrorKind::Timeout,
                    "Script timed out after 60s.",
                )
                .into());
            }
            let mut polls = counter.lock().unwrap();
            *polls += 1;
            let (project, id) = if *polls < 3 {
                ("Promo", "promo-id")
            } else {
                ("Client X", "cx-id")
            };
            Ok(json!({ "project": project, "projectId": id, "database": null }))
        }));
        let opened = create_resolve_projects(connection, Duration::from_millis(1))
            .open(
                &link(ResolveLinkKind::Local, "Client X", Some("cx-id")),
                &LocalLink::default(),
            )
            .await
            .unwrap();
        assert_eq!(
            opened,
            OpenedProject {
                project: named("Client X", "cx-id"),
                database: None,
                folder: None
            }
        );
        assert_eq!(*polls.lock().unwrap(), 3);
    }
}
