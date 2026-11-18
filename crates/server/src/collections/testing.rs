// A CollectionStore in memory, for app tests.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use linker_edit_protocol::CollectionExample;

use super::store::{CollectionStore, NewCollection, RULES_FILE, SavedCollection, example_name};

#[derive(Default)]
pub struct MemoryCollectionStore {
    /// In the order they were made.
    pub collections: Mutex<Vec<SavedCollection>>,
    /// "<collection id>/<example name>" → its notes.
    pub example_notes: Mutex<BTreeMap<String, String>>,
    next: AtomicUsize,
}

pub fn memory_collection_store(initial: Vec<SavedCollection>) -> MemoryCollectionStore {
    MemoryCollectionStore {
        collections: Mutex::new(initial),
        ..MemoryCollectionStore::default()
    }
}

fn example_file(id: &str, name: &str) -> String {
    format!("/collections/{id}/examples/{name}.md")
}

impl MemoryCollectionStore {
    fn has(&self, id: &str) -> bool {
        self.collections
            .lock()
            .unwrap()
            .iter()
            .any(|collection| collection.id == id)
    }
}

impl CollectionStore for MemoryCollectionStore {
    fn load(&self) -> Vec<SavedCollection> {
        self.collections.lock().unwrap().clone()
    }

    fn create(&self, fields: NewCollection) -> anyhow::Result<SavedCollection> {
        let next = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        let collection = fields.with_id(format!("collection-{next}"));
        self.collections.lock().unwrap().push(collection.clone());
        Ok(collection)
    }

    fn save(&self, collection: &SavedCollection) {
        let mut collections = self.collections.lock().unwrap();
        if let Some(saved) = collections
            .iter_mut()
            .find(|saved| saved.id == collection.id)
        {
            *saved = collection.clone();
        }
    }

    fn remove(&self, id: &str) {
        self.collections
            .lock()
            .unwrap()
            .retain(|collection| collection.id != id);
    }

    fn rules_file(&self, id: &str) -> Option<String> {
        self.has(id)
            .then(|| format!("/collections/{id}/{RULES_FILE}"))
    }

    fn examples(&self, id: &str) -> Vec<CollectionExample> {
        let prefix = format!("{id}/");
        self.example_notes
            .lock()
            .unwrap()
            .keys()
            .filter_map(|key| key.strip_prefix(&prefix))
            .map(|name| CollectionExample {
                name: name.to_string(),
                file: example_file(id, name),
            })
            .collect()
    }

    fn save_example(&self, id: &str, name: &str, text: &str) -> anyhow::Result<Option<String>> {
        if !self.has(id) {
            return Ok(None);
        }
        let name = example_name(name);
        self.example_notes
            .lock()
            .unwrap()
            .insert(format!("{id}/{name}"), text.to_string());
        Ok(Some(example_file(id, &name)))
    }

    fn remove_example(&self, id: &str, name: &str) {
        self.example_notes
            .lock()
            .unwrap()
            .remove(&format!("{id}/{}", example_name(name)));
    }
}
