// Workspaces and series: groups above projects, each with its own rules for the agent. A workspace holds series
// and projects; a series holds projects. One level deep: a series can't hold a series.
//
// Unlike the TypeScript, Collections doesn't keep the app it works for: each call that needs the app's projects
// takes a CollectionsHost, so the hub can lend its own state without a cycle. A brief change an agent proposes is
// data (BriefProposal) the hub keeps until the user answers, then hands to apply_proposal.

use std::path::Path;
use std::sync::{Arc, LazyLock};

use linker_edit_protocol::{
    ApprovedEdit, CollectionIcon, CollectionKind, CollectionMessage, CollectionSummary, IconColor,
    Lesson,
};
use regex::Regex;
use serde_json::{Map, Value, json};

use super::store::{CollectionStore, NewCollection, RULES_FILE, SavedCollection, example_name};
use crate::js;
use crate::projects::store::{ProjectCollection, SavedProject};
use crate::rules::{
    AUTONOMOUS_RULES, LEARNING_RULES, PROJECT_NOTES_FILE, PROJECT_RULES, RuleSource, agent_rules,
    read_project_notes, setup_rules,
};

/// What the workspaces and series need from the app around them.
pub trait CollectionsHost {
    fn projects(&self) -> Vec<SavedProject>;
    /// Saves the project in its new workspace or series, or in none; commit_message is for its edit history.
    fn set_collection(
        &mut self,
        project_id: &str,
        collection: Option<ProjectCollection>,
        commit_message: &str,
    );
    /// Takes a deleted collection's setup threads out of the app.
    fn remove_project(&mut self, project_id: &str);
    /// Clients and the agents' rules need to see a change.
    fn changed(&mut self);
}

/// What workspaces and series are called, in the app and in the agent's rules (src/lib/labels.ts's
/// COLLECTION_LABEL). Renaming them is a change here and there.
pub struct CollectionLabel {
    pub one: &'static str,
    pub many: &'static str,
    pub lower: &'static str,
    pub lower_many: &'static str,
}

pub const WORKSPACE_LABEL: CollectionLabel = CollectionLabel {
    one: "Workspace",
    many: "Workspaces",
    lower: "workspace",
    lower_many: "workspaces",
};

pub const SERIES_LABEL: CollectionLabel = CollectionLabel {
    one: "Series",
    many: "Series",
    lower: "series",
    lower_many: "series",
};

pub fn collection_label(kind: CollectionKind) -> &'static CollectionLabel {
    match kind {
        CollectionKind::Workspace => &WORKSPACE_LABEL,
        CollectionKind::Series => &SERIES_LABEL,
    }
}

/// A change to the brief an agent proposes. The user sees `shows` and accepts or declines; apply_proposal makes
/// the change and returns what the agent is told.
#[derive(Debug, Clone, PartialEq)]
pub struct BriefProposal {
    pub shows: Map<String, Value>,
    change: BriefChange,
}

#[derive(Debug, Clone, PartialEq)]
enum BriefChange {
    Rules {
        target: Found,
        rules: String,
    },
    Lesson {
        collection_id: String,
        label: String,
        rule: String,
        feedback: String,
        project: String,
    },
}

/// The collection a brief tool writes to, or the series it makes.
#[derive(Debug, Clone, PartialEq)]
enum Found {
    Collection(String),
    Create { name: String, parent_id: String },
}

const MAX_LESSONS: usize = 60;
const MAX_LESSON: usize = 400;
const MAX_FEEDBACK: usize = 600;
const MAX_EXAMPLE: usize = 12_000;
const MAX_EDITS: usize = 30;

pub const ICON_COLORS: [IconColor; 9] = [
    IconColor::Gray,
    IconColor::Red,
    IconColor::Orange,
    IconColor::Amber,
    IconColor::Green,
    IconColor::Teal,
    IconColor::Blue,
    IconColor::Violet,
    IconColor::Pink,
];

const KINDS: [CollectionKind; 2] = [CollectionKind::Workspace, CollectionKind::Series];
const MAX_NAME: usize = 80;
// RULES.md is for a style guide, not a book; this keeps a paste accident out of every prompt.
const MAX_RULES: usize = 20_000;

static LUCIDE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z0-9-]{1,40}$").unwrap());
static WHITESPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());

/// A string input trimmed, or "" for anything else.
fn text(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .map(|text| js::trim(text).to_string())
        .unwrap_or_default()
}

pub fn is_collection_icon(icon: &CollectionIcon) -> bool {
    match icon {
        CollectionIcon::Lucide { name, .. } => LUCIDE.is_match(name),
        CollectionIcon::Emoji { emoji, .. } => (1..=16).contains(&js::length(emoji)),
        CollectionIcon::Monogram { text, .. } => js::length(text) <= 3,
    }
}

/// A new collection gets letters from its name, on a color picked by the name.
fn default_icon(name: &str) -> CollectionIcon {
    let mut hash: u64 = 0;
    for c in name.chars() {
        let mut units = [0u16; 2];
        let first = c.encode_utf16(&mut units)[0];
        hash = (hash * 31 + u64::from(first)) & 0xFFFF_FFFF;
    }
    CollectionIcon::Monogram {
        text: String::new(),
        color: ICON_COLORS[1 + (hash % (ICON_COLORS.len() as u64 - 1)) as usize],
    }
}

fn clean_name(name: &str) -> Option<String> {
    let collapsed = WHITESPACE.replace_all(name, " ");
    let cleaned = js::slice(js::trim(&collapsed), MAX_NAME);
    (!cleaned.is_empty()).then(|| cleaned.to_string())
}

fn label(collection: &SavedCollection) -> String {
    format!(
        "{} \"{}\"",
        collection_label(collection.kind).one,
        collection.name
    )
}

/// The workspace a project is in, directly or through its series, and the series.
pub struct Chain<'a> {
    pub workspace: Option<&'a SavedCollection>,
    pub series: Option<&'a SavedCollection>,
}

/// The workspaces and series, in the order they were made, with their store.
pub struct Collections {
    store: Arc<dyn CollectionStore>,
    collections: Vec<SavedCollection>,
}

pub fn create_collections(store: Arc<dyn CollectionStore>) -> Collections {
    Collections::new(store)
}

impl Collections {
    pub fn new(store: Arc<dyn CollectionStore>) -> Self {
        let mut collections: Vec<SavedCollection> = Vec::new();
        for collection in store.load() {
            // A Map by id: a second entry with the same id replaces the first in its place.
            match collections
                .iter_mut()
                .find(|other| other.id == collection.id)
            {
                Some(other) => *other = collection,
                None => collections.push(collection),
            }
        }
        // A series whose workspace went missing stands on its own rather than disappearing.
        let workspaces: Vec<String> = collections
            .iter()
            .filter(|collection| collection.kind == CollectionKind::Workspace)
            .map(|collection| collection.id.clone())
            .collect();
        for collection in &mut collections {
            if let Some(parent) = &collection.parent_id
                && (collection.kind != CollectionKind::Series || !workspaces.contains(parent))
            {
                collection.parent_id = None;
            }
        }
        Collections { store, collections }
    }

    pub fn get(&self, id: &str) -> Option<&SavedCollection> {
        self.collections
            .iter()
            .find(|collection| collection.id == id)
    }

    fn set(&mut self, next: SavedCollection) {
        match self
            .collections
            .iter_mut()
            .find(|collection| collection.id == next.id)
        {
            Some(current) => *current = next,
            None => self.collections.push(next),
        }
    }

    /// The workspace a project is in, directly or through its series, and the series.
    pub fn chain(&self, project: &SavedProject) -> Chain<'_> {
        let own = project
            .collection
            .as_ref()
            .and_then(|collection| self.get(&collection.id));
        match own {
            None => Chain {
                workspace: None,
                series: None,
            },
            Some(own) if own.kind == CollectionKind::Workspace => Chain {
                workspace: Some(own),
                series: None,
            },
            Some(own) => Chain {
                workspace: own.parent_id.as_deref().and_then(|id| self.get(id)),
                series: Some(own),
            },
        }
    }

    /// A collection's part of the brief as agents read it: its rules, its lessons, and where its example notes are.
    fn brief_text(&self, collection: &SavedCollection) -> String {
        let examples = self.store.examples(&collection.id);
        let mut parts = vec![js::trim(&collection.rules).to_string()];
        if !collection.lessons.is_empty() {
            let lessons: Vec<String> = collection
                .lessons
                .iter()
                .map(|lesson| format!("- {}", lesson.rule))
                .collect();
            parts.push(format!(
                "Lessons from the user's feedback, each accepted by the user. They win over the rules above where they disagree:\n{}",
                lessons.join("\n")
            ));
        }
        if let Some(first) = examples.first() {
            let dir = Path::new(&first.file)
                .parent()
                .map(|dir| dir.to_string_lossy().into_owned())
                .unwrap_or_default();
            let names: Vec<&str> = examples
                .iter()
                .map(|example| example.name.as_str())
                .collect();
            parts.push(format!(
                "Notes on {} example video{} are in {dir} ({}). Read one when you need a reference.",
                examples.len(),
                if examples.len() == 1 { "" } else { "s" },
                names.join(", ")
            ));
        }
        parts.retain(|part| !part.is_empty());
        parts.join("\n\n")
    }

    /// Why parent_id can't hold a collection of this kind; None when it can.
    fn parent_refusal(&self, kind: CollectionKind, parent_id: Option<&str>) -> Option<String> {
        let parent_id = parent_id?;
        if kind == CollectionKind::Workspace {
            return Some(format!(
                "A {} can't go inside another one.",
                WORKSPACE_LABEL.lower
            ));
        }
        let Some(parent) = self.get(parent_id) else {
            return Some(format!("That {} is gone.", WORKSPACE_LABEL.lower));
        };
        if parent.kind != CollectionKind::Workspace {
            return Some(format!(
                "A {} can only go in a {}.",
                SERIES_LABEL.lower, WORKSPACE_LABEL.lower
            ));
        }
        None
    }

    /// Projects keep the name of their collection in linker-edit.json; a rename updates it there.
    fn rename_in_projects(&self, host: &mut dyn CollectionsHost, collection: &SavedCollection) {
        for project in host.projects() {
            if let Some(current) = &project.collection
                && current.id == collection.id
                && current.name != collection.name
            {
                host.set_collection(
                    &project.id,
                    Some(ProjectCollection {
                        id: collection.id.clone(),
                        name: collection.name.clone(),
                    }),
                    &format!("{} renamed", label(collection)),
                );
            }
        }
    }

    fn create(
        &mut self,
        kind: CollectionKind,
        name: &str,
        parent_id: Option<&Option<String>>,
        icon: Option<&CollectionIcon>,
    ) -> anyhow::Result<Option<String>> {
        let Some(name) = clean_name(name) else {
            return Ok(Some("Give it a name.".into()));
        };
        let parent_id = match kind {
            CollectionKind::Series => parent_id.cloned().flatten(),
            CollectionKind::Workspace => None,
        };
        if let Some(refusal) = self.parent_refusal(kind, parent_id.as_deref()) {
            return Ok(Some(refusal));
        }
        let icon = icon
            .filter(|icon| is_collection_icon(icon))
            .cloned()
            .unwrap_or_else(|| default_icon(&name));
        let collection = self.store.create(NewCollection {
            kind,
            name,
            parent_id,
            icon,
            rules: String::new(),
        })?;
        self.set(collection);
        Ok(None)
    }

    #[allow(clippy::too_many_arguments)]
    fn update(
        &mut self,
        host: &mut dyn CollectionsHost,
        id: &str,
        name: Option<&String>,
        icon: Option<&CollectionIcon>,
        rules: Option<&String>,
        parent_id: Option<&Option<String>>,
        autonomous: Option<bool>,
    ) -> Option<String> {
        let current = self.get(id)?.clone();
        let mut next = current.clone();
        if let Some(name) = name {
            let Some(name) = clean_name(name) else {
                return Some("Give it a name.".into());
            };
            next.name = name;
        }
        if let Some(icon) = icon {
            if !is_collection_icon(icon) {
                return None;
            }
            next.icon = icon.clone();
        }
        if let Some(rules) = rules {
            if js::length(rules) > MAX_RULES {
                return Some(format!(
                    "Rules can be up to {} characters.",
                    js::group_thousands(MAX_RULES)
                ));
            }
            next.rules = rules.clone();
        }
        if let Some(parent_id) = parent_id {
            if let Some(refusal) = self.parent_refusal(next.kind, parent_id.as_deref()) {
                return Some(refusal);
            }
            next.parent_id = parent_id.clone();
        }
        if let Some(autonomous) = autonomous {
            next.autonomous = autonomous;
        }
        self.set(next.clone());
        self.store.save(&next);
        if next.name != current.name {
            self.rename_in_projects(host, &next);
        }
        None
    }

    fn remove(&mut self, host: &mut dyn CollectionsHost, id: &str) -> Option<String> {
        let collection = self.get(id)?.clone();
        let series = self
            .collections
            .iter()
            .filter(|other| other.parent_id.as_deref() == Some(id))
            .count();
        let in_it: Vec<SavedProject> = host
            .projects()
            .into_iter()
            .filter(|project| {
                project
                    .collection
                    .as_ref()
                    .is_some_and(|collection| collection.id == id)
            })
            .collect();
        let projects = in_it.iter().filter(|project| !project.setup).count();
        if series > 0 || projects > 0 {
            let mut what = Vec::new();
            if series > 0 {
                let noun = if series == 1 {
                    SERIES_LABEL.lower
                } else {
                    SERIES_LABEL.lower_many
                };
                what.push(format!("{series} {noun}"));
            }
            if projects > 0 {
                what.push(format!(
                    "{projects} project{}",
                    if projects == 1 { "" } else { "s" }
                ));
            }
            return Some(format!(
                "{} still has {} in it. Move {} out first.",
                label(&collection),
                what.join(" and "),
                if series + projects == 1 { "it" } else { "them" }
            ));
        }
        self.collections.retain(|other| other.id != id);
        self.store.remove(id);
        // Its setup threads go with it; their folder stays on disk like any project's.
        for project in in_it {
            host.remove_project(&project.id);
        }
        None
    }

    /// The collection a brief tool writes to: the project's workspace or series, or a series of its workspace by name.
    fn target(&self, project: &SavedProject, input: &Map<String, Value>) -> Result<Found, String> {
        let Chain { workspace, series } = self.chain(project);
        let wants = input.get("for").and_then(Value::as_str);
        let wants_workspace = match wants {
            Some("workspace") => true,
            Some("series") => false,
            _ => series.is_none(),
        };
        if workspace.is_none() && series.is_none() {
            return Err(format!(
                "This project isn't in a {} or {}, so it has no brief. The user puts it in one in the project's settings.",
                WORKSPACE_LABEL.lower, SERIES_LABEL.lower
            ));
        }
        if wants_workspace {
            return workspace
                .map(|workspace| Found::Collection(workspace.id.clone()))
                .ok_or_else(|| {
                    format!(
                        "This project's {} isn't in a {}. Pass for: \"series\".",
                        SERIES_LABEL.lower, WORKSPACE_LABEL.lower
                    )
                });
        }
        let name = input
            .get("series")
            .and_then(Value::as_str)
            .and_then(clean_name);
        let Some(name) = name else {
            return series
                .map(|series| Found::Collection(series.id.clone()))
                .ok_or_else(|| format!("Name the {} in series.", SERIES_LABEL.lower));
        };
        if let Some(series) = series
            && series.name.to_lowercase() == name.to_lowercase()
        {
            return Ok(Found::Collection(series.id.clone()));
        }
        let Some(workspace) = workspace else {
            return Err(format!(
                "This project's {} isn't in a {}, so it can only change its own.",
                SERIES_LABEL.lower, WORKSPACE_LABEL.lower
            ));
        };
        let found = self.collections.iter().find(|candidate| {
            candidate.kind == CollectionKind::Series
                && candidate.parent_id.as_deref() == Some(workspace.id.as_str())
                && candidate.name.to_lowercase() == name.to_lowercase()
        });
        Ok(match found {
            Some(found) => Found::Collection(found.id.clone()),
            None => Found::Create {
                name,
                parent_id: workspace.id.clone(),
            },
        })
    }

    fn save_rules(&mut self, found: &Found, rules: String) -> anyhow::Result<SavedCollection> {
        match found {
            Found::Create { name, parent_id } => {
                let created = self.store.create(NewCollection {
                    kind: CollectionKind::Series,
                    name: name.clone(),
                    parent_id: Some(parent_id.clone()),
                    icon: default_icon(name),
                    rules,
                })?;
                self.set(created.clone());
                Ok(created)
            }
            Found::Collection(id) => {
                let current = self.get(id).ok_or_else(|| {
                    anyhow::anyhow!(
                        "That {} or {} is gone.",
                        WORKSPACE_LABEL.lower,
                        SERIES_LABEL.lower
                    )
                })?;
                let next = SavedCollection {
                    rules,
                    ..current.clone()
                };
                self.set(next.clone());
                self.store.save(&next);
                Ok(next)
            }
        }
    }

    fn change(
        &mut self,
        id: &str,
        update: impl FnOnce(SavedCollection) -> SavedCollection,
    ) -> Option<SavedCollection> {
        let current = self.get(id)?.clone();
        let next = update(current);
        self.set(next.clone());
        self.store.save(&next);
        Some(next)
    }

    fn move_project(
        &self,
        host: &mut dyn CollectionsHost,
        project_id: &str,
        collection_id: Option<&str>,
    ) -> Option<String> {
        let project = host
            .projects()
            .into_iter()
            .find(|candidate| candidate.id == project_id)?;
        if project.setup {
            return Some("Setup threads stay with what they set up.".into());
        }
        let Some(collection_id) = collection_id else {
            if let Some(current) = &project.collection {
                host.set_collection(
                    project_id,
                    None,
                    &format!("Take the project out of \"{}\"", current.name),
                );
            }
            return None;
        };
        let Some(collection) = self.get(collection_id) else {
            return Some(format!(
                "That {} or {} is gone.",
                WORKSPACE_LABEL.lower, SERIES_LABEL.lower
            ));
        };
        if project
            .collection
            .as_ref()
            .map(|current| current.id.as_str())
            != Some(collection.id.as_str())
        {
            host.set_collection(
                project_id,
                Some(ProjectCollection {
                    id: collection.id.clone(),
                    name: collection.name.clone(),
                }),
                &format!("Put the project in {}", label(collection)),
            );
        }
        None
    }

    /// Workspaces first, then series, each in the order they were made.
    pub fn summaries(&self, host: &dyn CollectionsHost) -> Vec<CollectionSummary> {
        let projects = host.projects();
        let mut sorted: Vec<&SavedCollection> = self.collections.iter().collect();
        sorted.sort_by_key(|collection| KINDS.iter().position(|kind| *kind == collection.kind));
        let in_it = |project: &SavedProject, id: &str| {
            project
                .collection
                .as_ref()
                .is_some_and(|collection| collection.id == id)
        };
        sorted
            .into_iter()
            .map(|collection| CollectionSummary {
                id: collection.id.clone(),
                kind: collection.kind,
                name: collection.name.clone(),
                parent_id: collection.parent_id.clone(),
                icon: collection.icon.clone(),
                rules: collection.rules.clone(),
                rules_file: self.store.rules_file(&collection.id).unwrap_or_default(),
                project_ids: projects
                    .iter()
                    .filter(|project| in_it(project, &collection.id) && !project.setup)
                    .map(|project| project.id.clone())
                    .collect(),
                autonomous: collection.autonomous,
                lessons: collection.lessons.clone(),
                examples: self.store.examples(&collection.id),
                setup_project_id: projects
                    .iter()
                    .find(|project| project.setup && in_it(project, &collection.id))
                    .map(|project| project.id.clone()),
                edits: collection.edits.clone(),
            })
            .collect()
    }

    /// The agent's rules for this project: the editing rules, then its workspace's, its series' and its own, each
    /// with the lessons learned there. A setup thread gets the setup assistant's rules instead.
    pub fn rules_for(&self, project: &SavedProject) -> String {
        let Chain { workspace, series } = self.chain(project);
        if project.setup
            && let Some(subject) = series.or(workspace)
        {
            let parts: Vec<Option<&SavedCollection>> = if subject.kind == CollectionKind::Workspace
            {
                std::iter::once(Some(subject))
                    .chain(
                        self.collections
                            .iter()
                            .filter(|other| other.parent_id.as_deref() == Some(subject.id.as_str()))
                            .map(Some),
                    )
                    .collect()
            } else {
                vec![workspace, Some(subject)]
            };
            let state: Vec<String> = parts
                .into_iter()
                .flatten()
                .map(|part| {
                    let brief = self.brief_text(part);
                    format!(
                        "## {}\n\n{}",
                        label(part),
                        if brief.is_empty() {
                            "Nothing yet."
                        } else {
                            &brief
                        }
                    )
                })
                .collect();
            return setup_rules(&label(subject), &state.join("\n\n"));
        }
        let mut sources = Vec::new();
        // Like a repository inside folders with instructions of their own: each heading names the file it is from.
        for part in [workspace, series].into_iter().flatten() {
            let file = self
                .store
                .rules_file(&part.id)
                .unwrap_or_else(|| RULES_FILE.to_string());
            sources.push(RuleSource {
                heading: format!("{} ({file})", label(part)),
                text: self.brief_text(part),
            });
        }
        sources.push(RuleSource {
            heading: format!("This project ({PROJECT_NOTES_FILE} in the working folder)"),
            text: read_project_notes(&project.dir),
        });
        let mut sections = vec![PROJECT_RULES];
        if let Some(nearest) = series.or(workspace) {
            if nearest.autonomous {
                sections.push(AUTONOMOUS_RULES);
            }
            sections.push(LEARNING_RULES);
        }
        agent_rules(&sources, &sections)
    }

    /// Whether this project's agent makes the first cut without asking.
    pub fn autonomous(&self, project: &SavedProject) -> bool {
        let Chain { workspace, series } = self.chain(project);
        series
            .or(workspace)
            .is_some_and(|nearest| nearest.autonomous)
    }

    /// save_brief: new rules for the project's workspace or a series, shown to the user as a change. Err is what the
    /// agent is told.
    pub fn propose_brief(
        &self,
        project: &SavedProject,
        input: &Map<String, Value>,
    ) -> Result<BriefProposal, String> {
        let found = self.target(project, input)?;
        let rules = text(input.get("rules"));
        if rules.is_empty() {
            return Err("Pass the full new rules in rules.".into());
        }
        if js::length(&rules) > MAX_RULES {
            return Err(format!(
                "Rules can be up to {} characters; these are {}. Make them shorter.",
                js::group_thousands(MAX_RULES),
                js::group_thousands(js::length(&rules))
            ));
        }
        let (name, was) = match &found {
            Found::Create { name, .. } => (
                format!("New {} \"{name}\"", SERIES_LABEL.lower),
                String::new(),
            ),
            Found::Collection(id) => {
                let collection = self.get(id).expect("target found it");
                (label(collection), collection.rules.clone())
            }
        };
        let shows = json!({
            "for": name,
            "summary": js::slice(&text(input.get("summary")), 600),
            "was": was,
            "rules": rules,
        });
        Ok(BriefProposal {
            shows: shows.as_object().cloned().unwrap_or_default(),
            change: BriefChange::Rules {
                target: found,
                rules,
            },
        })
    }

    /// add_lesson: one rule learned from feedback, shown to the user next to their own words.
    pub fn propose_lesson(
        &self,
        project: &SavedProject,
        input: &Map<String, Value>,
        project_name: &str,
    ) -> Result<BriefProposal, String> {
        let collection = match self.target(project, input)? {
            Found::Create { name, .. } => {
                return Err(format!(
                    "There is no {} \"{name}\" in this {}.",
                    SERIES_LABEL.lower, WORKSPACE_LABEL.lower
                ));
            }
            Found::Collection(id) => self.get(&id).expect("target found it"),
        };
        let rule = text(input.get("rule"));
        let feedback = text(input.get("feedback"));
        if rule.is_empty() || feedback.is_empty() {
            return Err(
                "Pass the rule, and the user's own words it comes from in feedback.".into(),
            );
        }
        if js::length(&rule) > MAX_LESSON {
            return Err(format!(
                "A lesson is one short rule, up to {MAX_LESSON} characters. For more, change the rules with save_brief."
            ));
        }
        if collection.lessons.len() >= MAX_LESSONS {
            return Err(format!(
                "{} has {MAX_LESSONS} lessons already. Fold them into its rules with save_brief; the user removes lessons in Settings.",
                label(collection)
            ));
        }
        let feedback = js::slice(&feedback, MAX_FEEDBACK).to_string();
        let shows = json!({ "for": label(collection), "feedback": feedback, "rule": rule });
        Ok(BriefProposal {
            shows: shows.as_object().cloned().unwrap_or_default(),
            change: BriefChange::Lesson {
                collection_id: collection.id.clone(),
                label: label(collection),
                rule,
                feedback,
                project: project_name.to_string(),
            },
        })
    }

    /// Makes a proposal the user accepted; returns what the agent is told.
    pub fn apply_proposal(
        &mut self,
        host: &mut dyn CollectionsHost,
        proposal: BriefProposal,
    ) -> anyhow::Result<String> {
        match proposal.change {
            BriefChange::Rules { target, rules } => {
                let saved = self.save_rules(&target, format!("{rules}\n"))?;
                host.changed();
                Ok(format!("Saved the rules of {}.", label(&saved)))
            }
            BriefChange::Lesson {
                collection_id,
                label,
                rule,
                feedback,
                project,
            } => {
                let lesson = Lesson {
                    id: uuid::Uuid::new_v4().to_string(),
                    rule,
                    feedback,
                    added_at: js::now_iso(),
                    project: Some(project),
                };
                self.change(&collection_id, |mut current| {
                    current.lessons.push(lesson);
                    current
                });
                host.changed();
                Ok(format!("Added the lesson to {label}."))
            }
        }
    }

    /// save_example: the agent's notes on one example video. They are its own notes, so nobody is asked. Err is what
    /// the agent is told.
    pub fn save_example(
        &self,
        host: &mut dyn CollectionsHost,
        project: &SavedProject,
        input: &Map<String, Value>,
    ) -> anyhow::Result<String> {
        let collection = match self.target(project, input).map_err(anyhow::Error::msg)? {
            Found::Create { name, .. } => anyhow::bail!(
                "There is no {} \"{name}\" yet. Save its rules with save_brief first, or leave series out.",
                SERIES_LABEL.lower
            ),
            Found::Collection(id) => self.get(&id).expect("target found it"),
        };
        let name = text(input.get("name"));
        let notes = text(input.get("notes"));
        if name.is_empty() || notes.is_empty() {
            anyhow::bail!("Pass the video's name and your notes.");
        }
        if js::length(&notes) > MAX_EXAMPLE {
            anyhow::bail!(
                "Notes on one example can be up to {} characters.",
                js::group_thousands(MAX_EXAMPLE)
            );
        }
        let Some(file) =
            self.store
                .save_example(&collection.id, &name, &format!("# {name}\n\n{notes}\n"))?
        else {
            anyhow::bail!("{} is gone.", label(collection));
        };
        host.changed();
        Ok(format!(
            "Saved the notes on \"{}\" for {}: {file}",
            example_name(&name),
            label(collection)
        ))
    }

    /// An AI timeline of this project became its main one, after this many rounds of feedback.
    pub fn record_edit(
        &mut self,
        host: &mut dyn CollectionsHost,
        project: &SavedProject,
        project_name: &str,
        rounds: u32,
    ) {
        let Some(nearest) = ({
            let Chain { workspace, series } = self.chain(project);
            series.or(workspace).map(|nearest| nearest.id.clone())
        }) else {
            return;
        };
        let edit = ApprovedEdit {
            at: js::now_iso(),
            project: project_name.to_string(),
            rounds,
        };
        self.change(&nearest, |mut current| {
            current.edits.insert(0, edit);
            current.edits.truncate(MAX_EDITS);
            current
        });
        host.changed();
    }

    /// Handles the workspace and series messages but setup_collection, which the app handles. Returns why it was
    /// refused, if it was. Err when the store couldn't write a new collection.
    pub fn handle(
        &mut self,
        host: &mut dyn CollectionsHost,
        message: &CollectionMessage,
    ) -> anyhow::Result<Option<String>> {
        let refusal = match message {
            CollectionMessage::SetupCollection { .. } => return Ok(None),
            CollectionMessage::CreateCollection {
                kind,
                name,
                parent_id,
                icon,
            } => self.create(*kind, name, parent_id.as_ref(), icon.as_ref())?,
            CollectionMessage::UpdateCollection {
                id,
                name,
                icon,
                rules,
                parent_id,
                autonomous,
            } => self.update(
                host,
                id,
                name.as_ref(),
                icon.as_ref(),
                rules.as_ref(),
                parent_id.as_ref(),
                *autonomous,
            ),
            CollectionMessage::DeleteCollection { id } => self.remove(host, id),
            CollectionMessage::MoveProject {
                project_id,
                collection_id,
            } => self.move_project(host, project_id, collection_id.as_deref()),
            CollectionMessage::RemoveLesson { id, lesson_id } => {
                self.change(id, |mut current| {
                    current.lessons.retain(|lesson| &lesson.id != lesson_id);
                    current
                });
                None
            }
            CollectionMessage::RemoveExample { id, name } => {
                self.store.remove_example(id, name);
                None
            }
        };
        if refusal.is_none() {
            host.changed();
        }
        Ok(refusal)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use linker_edit_protocol::LocalLink;

    use super::super::testing::{MemoryCollectionStore, memory_collection_store};
    use super::*;
    use crate::rules::EDITING_RULES;

    /// The app around the collections: projects, and what was committed and changed.
    #[derive(Default)]
    struct Host {
        order: Vec<String>,
        saved: HashMap<String, SavedProject>,
        commits: Vec<String>,
        changes: usize,
    }

    impl CollectionsHost for Host {
        fn projects(&self) -> Vec<SavedProject> {
            self.order
                .iter()
                .filter_map(|id| self.saved.get(id).cloned())
                .collect()
        }

        fn set_collection(
            &mut self,
            project_id: &str,
            collection: Option<ProjectCollection>,
            commit_message: &str,
        ) {
            if let Some(project) = self.saved.get_mut(project_id) {
                project.collection = collection;
            }
            self.commits.push(commit_message.into());
        }

        fn remove_project(&mut self, project_id: &str) {
            self.saved.remove(project_id);
        }

        fn changed(&mut self) {
            self.changes += 1;
        }
    }

    fn project(id: &str) -> SavedProject {
        project_in(id, PathBuf::from(format!("/projects/{id}")))
    }

    fn project_in(id: &str, dir: PathBuf) -> SavedProject {
        SavedProject {
            id: id.into(),
            dir,
            name: Some(id.into()),
            link: None,
            local: LocalLink::default(),
            icon: None,
            pinned: false,
            archived: false,
            collection: None,
            setup: false,
        }
    }

    struct Setup {
        collections: Collections,
        store: Arc<MemoryCollectionStore>,
        host: Host,
    }

    fn setup(projects: Vec<SavedProject>) -> Setup {
        let store = Arc::new(memory_collection_store(Vec::new()));
        let mut host = Host::default();
        for project in projects {
            host.order.push(project.id.clone());
            host.saved.insert(project.id.clone(), project);
        }
        Setup {
            collections: Collections::new(store.clone()),
            store,
            host,
        }
    }

    impl Setup {
        fn send(&mut self, message: Value) -> Option<String> {
            let message: CollectionMessage = serde_json::from_value(message).unwrap();
            self.collections.handle(&mut self.host, &message).unwrap()
        }

        fn by_name(&self, name: &str) -> CollectionSummary {
            self.collections
                .summaries(&self.host)
                .into_iter()
                .find(|collection| collection.name == name)
                .unwrap()
        }

        fn names(&self) -> Vec<String> {
            self.collections
                .summaries(&self.host)
                .into_iter()
                .map(|collection| collection.name)
                .collect()
        }
    }

    fn refused(refusal: Option<String>, pattern: &str) {
        let refusal = refusal.unwrap_or_default();
        assert!(
            refusal.contains(pattern),
            "{refusal:?} should say {pattern:?}"
        );
    }

    #[test]
    fn nests_one_level_deep() {
        let mut s = setup(vec![]);
        assert_eq!(s.send(json!({ "type": "create_collection", "kind": "workspace", "name": "  Ferret   Media " })), None);
        let workspace = s.by_name("Ferret Media");
        assert_eq!(workspace.kind, CollectionKind::Workspace);
        assert_eq!(workspace.parent_id, None);
        assert_eq!(workspace.rules, "");
        assert!(workspace.project_ids.is_empty());
        assert!(matches!(workspace.icon, CollectionIcon::Monogram { .. }));

        assert_eq!(
            s.send(json!({ "type": "create_collection", "kind": "series", "name": "Podcast", "parentId": workspace.id })),
            None
        );
        let series = s.by_name("Podcast");
        assert_eq!(series.parent_id.as_deref(), Some(workspace.id.as_str()));

        refused(
            s.send(json!({ "type": "create_collection", "kind": "series", "name": "Clips", "parentId": series.id })),
            "only go in a workspace",
        );
        assert_eq!(
            s.send(json!({ "type": "create_collection", "kind": "workspace", "name": "Sub", "parentId": workspace.id })),
            None
        );
        // A workspace's parent is ignored on create, and refused on update.
        let sub = s.by_name("Sub");
        assert_eq!(sub.parent_id, None);
        refused(
            s.send(json!({ "type": "update_collection", "id": sub.id, "parentId": workspace.id })),
            "can't go inside",
        );
        assert_eq!(
            s.send(json!({ "type": "update_collection", "id": series.id, "parentId": sub.id })),
            None
        );
        assert_eq!(
            s.by_name("Podcast").parent_id.as_deref(),
            Some(sub.id.as_str())
        );
        refused(
            s.send(json!({ "type": "update_collection", "id": series.id, "parentId": series.id })),
            "only go in a workspace",
        );

        // A series on its own is fine too.
        assert_eq!(
            s.send(json!({ "type": "create_collection", "kind": "series", "name": "Shorts" })),
            None
        );
        assert_eq!(s.by_name("Shorts").parent_id, None);
        refused(
            s.send(json!({ "type": "create_collection", "kind": "series", "name": "   " })),
            "name",
        );
        // Workspaces first.
        assert_eq!(s.names(), ["Ferret Media", "Sub", "Podcast", "Shorts"]);
    }

    #[test]
    fn refuses_to_delete_a_collection_that_still_has_projects_or_series_in_it() {
        let mut s = setup(vec![project("ep12"), project("ep11")]);
        s.send(json!({ "type": "create_collection", "kind": "workspace", "name": "Ferret Media" }));
        let workspace = s.by_name("Ferret Media");
        s.send(json!({ "type": "create_collection", "kind": "series", "name": "Podcast", "parentId": workspace.id }));
        let series = s.by_name("Podcast");
        s.send(json!({ "type": "move_project", "projectId": "ep12", "collectionId": series.id }));
        s.send(json!({ "type": "move_project", "projectId": "ep11", "collectionId": series.id }));

        assert_eq!(
            s.send(json!({ "type": "delete_collection", "id": workspace.id }))
                .as_deref(),
            Some("Workspace \"Ferret Media\" still has 1 series in it. Move it out first.")
        );
        assert_eq!(
            s.send(json!({ "type": "delete_collection", "id": series.id }))
                .as_deref(),
            Some("Series \"Podcast\" still has 2 projects in it. Move them out first.")
        );
        assert_eq!(s.store.collections.lock().unwrap().len(), 2);

        s.send(json!({ "type": "move_project", "projectId": "ep12", "collectionId": null }));
        s.send(
            json!({ "type": "move_project", "projectId": "ep11", "collectionId": workspace.id }),
        );
        assert_eq!(
            s.send(json!({ "type": "delete_collection", "id": series.id })),
            None
        );
        refused(
            s.send(json!({ "type": "delete_collection", "id": workspace.id })),
            "still has 1 project in it",
        );
        assert_eq!(s.names(), ["Ferret Media"]);
        assert_eq!(s.store.collections.lock().unwrap().len(), 1);
    }

    #[test]
    fn puts_projects_in_a_collection_and_keeps_its_name_in_them_when_renamed() {
        let mut s = setup(vec![project("ep12")]);
        s.send(json!({ "type": "create_collection", "kind": "series", "name": "Podcast" }));
        let series = s.by_name("Podcast");
        s.send(json!({ "type": "move_project", "projectId": "ep12", "collectionId": series.id }));
        let podcast = |name: &str| {
            Some(ProjectCollection {
                id: series.id.clone(),
                name: name.into(),
            })
        };
        assert_eq!(s.host.saved["ep12"].collection, podcast("Podcast"));
        assert_eq!(s.by_name("Podcast").project_ids, ["ep12"]);

        s.send(json!({ "type": "update_collection", "id": series.id, "name": "The Podcast", "rules": "Keep it short." }));
        assert_eq!(s.host.saved["ep12"].collection, podcast("The Podcast"));
        assert_eq!(s.by_name("The Podcast").rules, "Keep it short.");
        refused(
            s.send(json!({ "type": "move_project", "projectId": "ep12", "collectionId": "gone" })),
            "is gone",
        );
        assert_eq!(
            s.host.commits,
            [
                "Put the project in Series \"Podcast\"",
                "Series \"The Podcast\" renamed"
            ]
        );
        assert_eq!(s.host.changes, 3);
    }

    #[test]
    fn refuses_an_icon_that_isnt_one() {
        let mut s = setup(vec![]);
        s.send(json!({ "type": "create_collection", "kind": "workspace", "name": "W", "icon": { "kind": "emoji", "emoji": "🎙", "color": "pink" } }));
        let id = s.by_name("W").id;
        s.send(json!({ "type": "update_collection", "id": id, "icon": { "kind": "lucide", "name": "<script>", "color": "blue" } }));
        assert_eq!(
            s.by_name("W").icon,
            CollectionIcon::Emoji {
                emoji: "🎙".into(),
                color: IconColor::Pink
            }
        );
        s.send(json!({ "type": "update_collection", "id": id, "icon": { "kind": "lucide", "name": "mic", "color": "teal" } }));
        assert_eq!(
            s.by_name("W").icon,
            CollectionIcon::Lucide {
                name: "mic".into(),
                color: IconColor::Teal
            }
        );
        // An icon whose color isn't one isn't a message at all here: the protocol refuses it before the collections see it.
        let bad: Result<CollectionMessage, _> = serde_json::from_value(
            json!({ "type": "update_collection", "id": id, "icon": { "kind": "monogram", "text": "AB", "color": "nope" } }),
        );
        assert!(bad.is_err());
    }

    #[test]
    fn stacks_the_agents_rules_most_specific_last() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().to_path_buf();
        std::fs::write(
            dir.join("PROJECT.md"),
            "Ep 12 is the anniversary episode.\n",
        )
        .unwrap();
        // A project from before PROJECT.md keeps the notes it had.
        std::fs::write(
            dir.join("RULES.md"),
            "Guest asked to cut the salary talk.\n",
        )
        .unwrap();
        let mut s = setup(vec![project_in("ep12", dir), project("ep11")]);
        s.send(json!({ "type": "create_collection", "kind": "workspace", "name": "Ferret Media" }));
        let workspace = s.by_name("Ferret Media").id;
        s.send(
            json!({ "type": "update_collection", "id": workspace, "rules": "End on the logo." }),
        );
        s.send(json!({ "type": "create_collection", "kind": "series", "name": "Podcast", "parentId": workspace }));
        let series = s.by_name("Podcast").id;
        s.send(json!({ "type": "update_collection", "id": series, "rules": "Lower thirds in Inter Bold." }));
        s.send(json!({ "type": "move_project", "projectId": "ep12", "collectionId": series }));
        s.send(json!({ "type": "move_project", "projectId": "ep11", "collectionId": series }));

        let rules = s.collections.rules_for(&s.host.saved["ep12"]);
        assert!(rules.starts_with(EDITING_RULES));
        let order: Vec<usize> = [
            "# The user's rules".to_string(),
            format!("## Workspace \"Ferret Media\" (/collections/{workspace}/RULES.md)\n\nEnd on the logo."),
            format!("## Series \"Podcast\" (/collections/{series}/RULES.md)\n\nLower thirds in Inter Bold."),
            "## This project (PROJECT.md in the working folder)\n\nEp 12 is the anniversary episode.\n\nGuest asked to cut the salary talk.".to_string(),
        ]
        .iter()
        .map(|part| rules.find(part.as_str()).unwrap_or_else(|| panic!("{part} is missing")))
        .collect();
        assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "{order:?}");

        // Without notes of its own, the project gets the workspace's and series' rules only.
        let other = s.collections.rules_for(&s.host.saved["ep11"]);
        assert!(other.contains("End on the logo."));
        assert!(other.contains("Lower thirds in Inter Bold."));
        assert!(!other.contains("## This project ("));

        // A project in no collection, with no file yet, gets the editing rules and how to keep its file.
        assert_eq!(
            s.collections.rules_for(&project("loose")),
            format!("{EDITING_RULES}\n\n{PROJECT_RULES}")
        );
    }

    #[test]
    fn proposes_brief_changes_and_applies_them_only_when_asked() {
        let mut s = setup(vec![project("ep12"), project("loose")]);
        s.send(json!({ "type": "create_collection", "kind": "workspace", "name": "Ferret Media" }));
        let workspace = s.by_name("Ferret Media").id;
        s.send(json!({ "type": "move_project", "projectId": "ep12", "collectionId": workspace }));
        let ep12 = s.host.saved["ep12"].clone();
        let input = |value: Value| value.as_object().cloned().unwrap();

        assert!(
            s.collections
                .propose_brief(&s.host.saved["loose"], &input(json!({ "rules": "x" })))
                .unwrap_err()
                .contains("has no brief")
        );
        assert_eq!(
            s.collections
                .propose_brief(&ep12, &input(json!({ "rules": " " })))
                .unwrap_err(),
            "Pass the full new rules in rules."
        );
        let proposal = s
            .collections
            .propose_brief(&ep12, &input(json!({ "for": "series", "series": "Reels", "rules": "Vertical.", "summary": "A format" })))
            .unwrap();
        assert_eq!(
            Value::Object(proposal.shows.clone()),
            json!({ "for": "New series \"Reels\"", "summary": "A format", "was": "", "rules": "Vertical." })
        );
        // Nothing changes until the user accepts.
        assert_eq!(s.names(), ["Ferret Media"]);
        assert_eq!(
            s.collections.apply_proposal(&mut s.host, proposal).unwrap(),
            "Saved the rules of Series \"Reels\"."
        );
        let reels = s.by_name("Reels");
        assert_eq!(
            (reels.parent_id.as_deref(), reels.rules.as_str()),
            (Some(workspace.as_str()), "Vertical.\n")
        );

        let lesson = s
            .collections
            .propose_lesson(
                &ep12,
                &input(json!({ "rule": "Intro at most 3 seconds.", "feedback": "too slow" })),
                "Ep 12",
            )
            .unwrap();
        assert_eq!(
            s.collections.apply_proposal(&mut s.host, lesson).unwrap(),
            "Added the lesson to Workspace \"Ferret Media\"."
        );
        let lessons = s.by_name("Ferret Media").lessons;
        assert_eq!(
            (
                lessons[0].rule.as_str(),
                lessons[0].feedback.as_str(),
                lessons[0].project.as_deref()
            ),
            ("Intro at most 3 seconds.", "too slow", Some("Ep 12"))
        );
        assert!(
            s.collections
                .rules_for(&ep12)
                .contains("- Intro at most 3 seconds.")
        );

        let saved = s
            .collections
            .save_example(
                &mut s.host,
                &ep12,
                &input(json!({ "name": "Reel 12.mp4", "notes": "Fast." })),
            )
            .unwrap();
        assert_eq!(
            saved,
            format!(
                "Saved the notes on \"reel-12\" for Workspace \"Ferret Media\": /collections/{workspace}/examples/reel-12.md"
            )
        );
        assert_eq!(
            s.store
                .example_notes
                .lock()
                .unwrap()
                .get(&format!("{workspace}/reel-12"))
                .map(String::as_str),
            Some("# Reel 12.mp4\n\nFast.\n")
        );

        s.collections.record_edit(&mut s.host, &ep12, "Ep 12", 2);
        assert!(!s.collections.autonomous(&ep12));
        let edits = s.by_name("Ferret Media").edits;
        assert_eq!((edits[0].project.as_str(), edits[0].rounds), ("Ep 12", 2));
    }
}
