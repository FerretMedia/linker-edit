// A ProjectStore in memory, for app tests: what it saves is what the next app loads.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use linker_edit_protocol::LocalLink;

use super::store::{LoadedProject, NewProject, ProjectStore, SavedProject, SavedThread};

#[derive(Default)]
pub struct MemoryProjectStore {
    /// In the order they were added.
    pub projects: Mutex<Vec<SavedProject>>,
    /// Each project's threads by project id, in the order they were first saved.
    pub threads: Mutex<HashMap<String, Vec<SavedThread>>>,
    next: AtomicUsize,
}

pub fn memory_project_store(initial: Vec<LoadedProject>) -> MemoryProjectStore {
    let store = MemoryProjectStore::default();
    for LoadedProject { project, threads } in initial {
        store
            .threads
            .lock()
            .unwrap()
            .insert(project.id.clone(), threads);
        store.projects.lock().unwrap().push(project);
    }
    store
}

impl MemoryProjectStore {
    /// The project as saved last.
    pub fn project(&self, id: &str) -> Option<SavedProject> {
        self.projects
            .lock()
            .unwrap()
            .iter()
            .find(|project| project.id == id)
            .cloned()
    }
}

impl ProjectStore for MemoryProjectStore {
    fn load(&self) -> Vec<LoadedProject> {
        let threads = self.threads.lock().unwrap();
        self.projects
            .lock()
            .unwrap()
            .iter()
            .map(|project| LoadedProject {
                project: project.clone(),
                threads: threads.get(&project.id).cloned().unwrap_or_default(),
            })
            .collect()
    }

    fn create(&self, fields: NewProject) -> SavedProject {
        let next = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        let project = SavedProject {
            id: format!("project-{next}"),
            dir: PathBuf::from(format!("/projects/{next}")),
            name: fields.name,
            link: fields.link,
            local: fields.local,
            icon: None,
            pinned: false,
            archived: false,
            collection: fields.collection,
            setup: fields.setup,
        };
        self.projects.lock().unwrap().push(project.clone());
        self.threads
            .lock()
            .unwrap()
            .insert(project.id.clone(), Vec::new());
        project
    }

    fn adopt(&self, dir: &Path) -> anyhow::Result<LoadedProject> {
        anyhow::bail!(
            "{} has no linker-edit.json, so it isn't a Linker-Edit project folder.",
            dir.display()
        )
    }

    fn save_project(&self, project: &SavedProject) {
        let mut projects = self.projects.lock().unwrap();
        match projects.iter_mut().find(|saved| saved.id == project.id) {
            Some(saved) => *saved = project.clone(),
            None => projects.push(project.clone()),
        }
    }

    fn remove_project(&self, id: &str) {
        self.projects
            .lock()
            .unwrap()
            .retain(|project| project.id != id);
    }

    fn save_thread(&self, project: &SavedProject, thread: &SavedThread) {
        if let Some(threads) = self.threads.lock().unwrap().get_mut(&project.id) {
            match threads.iter_mut().find(|saved| saved.id == thread.id) {
                Some(saved) => *saved = thread.clone(),
                None => threads.push(thread.clone()),
            }
        }
    }

    fn delete_thread(&self, project: &SavedProject, thread_id: &str) {
        if let Some(threads) = self.threads.lock().unwrap().get_mut(&project.id) {
            threads.retain(|thread| thread.id != thread_id);
        }
    }
}

/// A project of its own for tests, as the TypeScript's workspaceProject.
pub fn workspace_project() -> SavedProject {
    SavedProject {
        id: "workspace".into(),
        dir: PathBuf::from("/work"),
        name: None,
        link: None,
        local: LocalLink::default(),
        icon: None,
        pinned: false,
        archived: false,
        collection: None,
        setup: false,
    }
}
