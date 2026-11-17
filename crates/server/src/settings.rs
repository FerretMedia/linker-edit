// The user's choices that outlive a restart, in ~/LinkerEdit/settings.json. Kept apart from
// linker-edit.config.json, which the user writes by hand and Linker-Edit only reads.

use std::io::ErrorKind;
use std::path::PathBuf;

use linker_edit_protocol::{ApprovalMode, Provider};
use serde::Serialize;
use serde_json::{Map, Value};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_mode: Option<ApprovalMode>,
    /// False on a computer the user said has no Resolve, such as a laptop that only drives an edit rig.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolve_here: Option<bool>,
    /// The agent new threads start with: the one picked last.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_provider: Option<Provider>,
}

pub trait SettingsStore: Send + Sync {
    fn load(&self) -> Settings;
    fn save(&self, changes: Settings);
}

type OnError = Box<dyn Fn(anyhow::Error) + Send + Sync>;

pub struct FileSettings {
    file: PathBuf,
    on_error: OnError,
}

/// Settings kept in file; problems with it are logged.
pub fn file_settings(file: impl Into<PathBuf>) -> FileSettings {
    FileSettings::new(file, |error| tracing::error!("{error:#}"))
}

impl FileSettings {
    pub fn new(
        file: impl Into<PathBuf>,
        on_error: impl Fn(anyhow::Error) + Send + Sync + 'static,
    ) -> Self {
        FileSettings {
            file: file.into(),
            on_error: Box::new(on_error),
        }
    }

    fn write(&self, settings: &Settings) -> anyhow::Result<()> {
        if let Some(dir) = self.file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut temp = self.file.clone().into_os_string();
        temp.push(".tmp");
        std::fs::write(
            &temp,
            format!("{}\n", serde_json::to_string_pretty(settings)?),
        )?;
        std::fs::rename(&temp, &self.file)?;
        Ok(())
    }
}

/// One of the values Settings knows, or None when it's missing or something else.
fn known<T: serde::de::DeserializeOwned>(values: &Map<String, Value>, key: &str) -> Option<T> {
    values
        .get(key)
        .and_then(|value| serde_json::from_value(value.clone()).ok())
}

/// Reads only what it knows; a missing or broken file is no settings, never an error that stops the app.
impl SettingsStore for FileSettings {
    fn load(&self) -> Settings {
        let raw = match std::fs::read_to_string(&self.file) {
            Ok(raw) => raw,
            Err(error) => {
                if error.kind() != ErrorKind::NotFound {
                    (self.on_error)(error.into());
                }
                return Settings::default();
            }
        };
        let raw: Value = match serde_json::from_str(&raw) {
            Ok(raw) => raw,
            Err(error) => {
                (self.on_error)(error.into());
                return Settings::default();
            }
        };
        let empty = Map::new();
        let values = raw.as_object().unwrap_or(&empty);
        Settings {
            approval_mode: known(values, "approvalMode"),
            resolve_here: values.get("resolveHere").and_then(Value::as_bool),
            default_provider: known(values, "defaultProvider"),
        }
    }

    fn save(&self, changes: Settings) {
        let saved = self.load();
        let settings = Settings {
            approval_mode: changes.approval_mode.or(saved.approval_mode),
            resolve_here: changes.resolve_here.or(saved.resolve_here),
            default_provider: changes.default_provider.or(saved.default_provider),
        };
        if let Err(error) = self.write(&settings) {
            (self.on_error)(error);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    fn temp_file() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::Builder::new()
            .prefix("linker-edit-settings-")
            .tempdir()
            .unwrap();
        let file = dir.path().join("settings.json");
        (dir, file)
    }

    #[test]
    fn keeps_the_approval_mode_across_restarts() {
        let (_dir, file) = temp_file();
        assert_eq!(file_settings(&file).load(), Settings::default());
        file_settings(&file).save(Settings {
            approval_mode: Some(ApprovalMode::Ask),
            ..Settings::default()
        });
        assert_eq!(
            file_settings(&file).load(),
            Settings {
                approval_mode: Some(ApprovalMode::Ask),
                ..Settings::default()
            }
        );
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "{\n  \"approvalMode\": \"ask\"\n}\n"
        );
    }

    #[test]
    fn keeps_whether_this_computer_has_resolve_next_to_the_approval_mode() {
        let (_dir, file) = temp_file();
        file_settings(&file).save(Settings {
            approval_mode: Some(ApprovalMode::Ask),
            ..Settings::default()
        });
        file_settings(&file).save(Settings {
            resolve_here: Some(false),
            ..Settings::default()
        });
        assert_eq!(
            file_settings(&file).load(),
            Settings {
                approval_mode: Some(ApprovalMode::Ask),
                resolve_here: Some(false),
                default_provider: None
            }
        );
    }

    #[test]
    fn ignores_a_broken_file_and_values_it_doesnt_know() {
        let (_dir, file) = temp_file();
        let errors = Arc::new(Mutex::new(Vec::new()));
        let seen = errors.clone();
        std::fs::write(&file, "{ not json").unwrap();
        let settings = FileSettings::new(&file, move |error| {
            seen.lock().unwrap().push(error.to_string())
        });
        assert_eq!(settings.load(), Settings::default());
        assert_eq!(errors.lock().unwrap().len(), 1);
        std::fs::write(&file, r#"{ "approvalMode": "yes", "resolveHere": "no" }"#).unwrap();
        assert_eq!(file_settings(&file).load(), Settings::default());
    }

    #[test]
    fn makes_the_folder_it_saves_in() {
        let (dir, _) = temp_file();
        let file = dir.path().join("LinkerEdit").join("settings.json");
        file_settings(&file).save(Settings {
            default_provider: Some(Provider::Codex),
            ..Settings::default()
        });
        assert_eq!(
            file_settings(&file).load().default_provider,
            Some(Provider::Codex)
        );
    }
}
