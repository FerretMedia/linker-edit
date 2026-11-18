// Resolve's state for the status pill and the project check before each turn.

use std::time::Duration;

use linker_edit_protocol::ResolveStatus;
use serde_json::{Map, Value, json};

use crate::resolve::connection::{ResolveConnection, ResolveErrorKind, error_kind};
use crate::resolve::locate::MISSING_RESOLVE_MCP;

const BLOCKED_HINT: &str = "Resolve answers but refuses edits while the Project Manager or a dialog, such as an update prompt, is open. Open a project and close the dialog in Resolve, then refresh.";

// Seen on Studio 21.1.2 after its update prompt: scripting stayed dead after the prompt closed, until
// Resolve restarted.
const STUCK_HINT: &str = "Resolve is running but doesn't answer scripts. An update prompt or another dialog usually causes this, and Resolve can stay stuck after it closes. Close any dialog in Resolve and refresh. If that doesn't help, save your work and restart Resolve.";

fn unreachable(detail: &str) -> ResolveStatus {
    ResolveStatus::Unreachable {
        hint: format!(
            "Resolve is running but doesn't answer Linker-Edit ({detail}). Restart Resolve, then refresh."
        ),
    }
}

/// What status.py returns: {"state": "connected", product, version, project, projectId, database, timeline, page}
/// or {"state": "unreachable"}.
pub fn status_from_probe(probe: Option<&Value>) -> ResolveStatus {
    let Some(Value::Object(probe)) = probe else {
        return unreachable("Resolve's scripting isn't reachable");
    };
    if probe.get("state").and_then(Value::as_str) != Some("connected") {
        return unreachable("Resolve's scripting isn't reachable");
    }
    // No page means Resolve is on the Project Manager or behind a modal dialog.
    if probe.get("page").is_none_or(Value::is_null) {
        return ResolveStatus::Blocked {
            hint: BLOCKED_HINT.to_string(),
        };
    }
    let mut status = Map::new();
    status.insert("projectId".into(), Value::Null);
    status.insert("database".into(), Value::Null);
    for (key, value) in probe {
        if key != "page" {
            status.insert(key.clone(), value.clone());
        }
    }
    // The protocol always has a product and version; status.py always sends them, as strings.
    for key in ["product", "version"] {
        if !status.get(key).is_some_and(Value::is_string) {
            status.insert(key.into(), json!(""));
        }
    }
    serde_json::from_value(Value::Object(status)).unwrap_or_else(|error| {
        unreachable(&format!("Resolve's status didn't make sense: {error}"))
    })
}

pub async fn check_resolve(connection: &dyn ResolveConnection) -> ResolveStatus {
    if connection.command().is_none() {
        return ResolveStatus::SetupNeeded {
            hint: MISSING_RESOLVE_MCP.to_string(),
        };
    }
    let running = match connection
        .call_tool("get_resolve_status", json!({}), Duration::from_secs(15))
        .await
    {
        Ok(text) => {
            serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|value| value.get("running").cloned())
                == Some(Value::Bool(true))
        }
        Err(error) => {
            let message = error.to_string();
            // An older Resolve's server can start but lack what Linker-Edit needs.
            if message.contains("no run_script") {
                return ResolveStatus::SetupNeeded { hint: message };
            }
            return ResolveStatus::Unreachable {
                hint: format!("Linker-Edit can't use Resolve's MCP server: {message}"),
            };
        }
    };
    if !running {
        return ResolveStatus::NotRunning;
    }
    match connection.run_script_value("status", json!({}), 10).await {
        Ok(probe) => status_from_probe(Some(&probe)),
        Err(error)
            if matches!(
                error_kind(&error),
                Some(ResolveErrorKind::Timeout | ResolveErrorKind::Unavailable)
            ) =>
        {
            ResolveStatus::Blocked {
                hint: STUCK_HINT.to_string(),
            }
        }
        Err(error) => unreachable(&error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use linker_edit_protocol::ResolveDatabase;

    use super::*;
    use crate::resolve::connection::ResolveError;
    use crate::resolve::testing::{FakeConnection, fake_connection};

    fn connected() -> ResolveStatus {
        ResolveStatus::Connected {
            product: "DaVinci Resolve Studio".into(),
            version: "21.1.2".into(),
            project: Some("Doc".into()),
            project_id: Some("a1b2".into()),
            database: Some(ResolveDatabase {
                r#type: "Disk".into(),
                name: "Local Database".into(),
                host: None,
            }),
            timeline: None,
        }
    }

    fn probe(page: Value) -> Value {
        let mut probe = serde_json::to_value(connected()).unwrap();
        probe["page"] = page;
        probe
    }

    fn hint(status: &ResolveStatus) -> &str {
        match status {
            ResolveStatus::Unreachable { hint }
            | ResolveStatus::Blocked { hint }
            | ResolveStatus::SetupNeeded { hint } => hint,
            _ => "",
        }
    }

    #[test]
    fn status_from_probe_reads_what_status_py_says() {
        assert_eq!(status_from_probe(Some(&probe(json!("edit")))), connected());
        assert_eq!(
            status_from_probe(Some(
                &json!({ "state": "connected", "project": "Doc", "timeline": null, "page": "edit" })
            )),
            ResolveStatus::Connected {
                product: "".into(),
                version: "".into(),
                project: Some("Doc".into()),
                project_id: None,
                database: None,
                timeline: None
            }
        );
        assert!(matches!(
            status_from_probe(Some(&probe(Value::Null))),
            ResolveStatus::Blocked { .. }
        ));
        assert!(matches!(
            status_from_probe(Some(&json!({ "state": "unreachable" }))),
            ResolveStatus::Unreachable { .. }
        ));
        assert!(matches!(
            status_from_probe(None),
            ResolveStatus::Unreachable { .. }
        ));
    }

    fn running(
        value: bool,
    ) -> impl Fn(&str, &Value) -> anyhow::Result<String> + Send + Sync + 'static {
        move |_, _| Ok(json!({ "running": value, "version": "21.1.2" }).to_string())
    }

    #[tokio::test]
    async fn asks_for_resolve_studio_21_1_2_or_newer_when_its_mcp_server_is_missing() {
        let status = check_resolve(&FakeConnection::new(None)).await;
        assert!(matches!(status, ResolveStatus::SetupNeeded { .. }));
        assert!(hint(&status).contains("21.1.2"));
    }

    #[tokio::test]
    async fn reports_resolve_not_running_without_running_a_script() {
        let connection = fake_connection().tool(running(false));
        assert_eq!(check_resolve(&connection).await, ResolveStatus::NotRunning);
        assert!(connection.scripts().is_empty());
    }

    #[tokio::test]
    async fn reads_the_open_project() {
        let connection = fake_connection()
            .tool(running(true))
            .script(|_, _| Ok(probe(json!("edit"))));
        assert_eq!(check_resolve(&connection).await, connected());
        assert_eq!(connection.script_names(), ["status"]);
    }

    #[tokio::test]
    async fn calls_a_script_that_never_finishes_stuck() {
        let connection = fake_connection().tool(running(true)).script(|_, _| {
            Err(ResolveError::new(ResolveErrorKind::Timeout, "Script timed out after 10s.").into())
        });
        let status = check_resolve(&connection).await;
        assert!(matches!(status, ResolveStatus::Blocked { .. }));
        assert!(hint(&status).contains("restart Resolve"));
    }

    #[tokio::test]
    async fn says_why_when_the_mcp_server_cant_start_or_is_too_old() {
        let broken = fake_connection().tool(|_, _| {
            Err(ResolveError::new(
                ResolveErrorKind::Unavailable,
                "Couldn't start Resolve's MCP server: spawn EACCES",
            )
            .into())
        });
        let status = check_resolve(&broken).await;
        assert!(matches!(status, ResolveStatus::Unreachable { .. }));
        assert!(hint(&status).contains("EACCES"));
        let old = fake_connection().tool(|_, _| {
            Err(ResolveError::new(
                ResolveErrorKind::Unavailable,
                "This Resolve's MCP server has no run_script tool. Update DaVinci Resolve Studio.",
            )
            .into())
        });
        assert!(matches!(
            check_resolve(&old).await,
            ResolveStatus::SetupNeeded { .. }
        ));
    }
}
