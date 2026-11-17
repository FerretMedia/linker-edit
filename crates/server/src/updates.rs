// This Linker-Edit's version, and whether npm has a newer one for `npx linker-edit`.

use std::time::Duration;

use serde::Deserialize;

/// The npm package `npx linker-edit` runs.
pub const PACKAGE_NAME: &str = "linker-edit";

/// A checkout says 0.0.0; releases are YYYY.MDD.N, stamped into the crate version when they're published. People
/// see the release name instead, 2026.11.07: see linker_edit_protocol::release_name.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub use linker_edit_protocol::release_name::is_newer;

/// The latest version on npm, or None when the registry can't be reached.
pub async fn latest_version() -> Option<String> {
    #[derive(Deserialize)]
    struct Latest {
        version: Option<String>,
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .ok()?;
    let response = client
        .get(format!("https://registry.npmjs.org/{PACKAGE_NAME}/latest"))
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.json::<Latest>().await.ok()?.version
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_numerically_not_as_text() {
        assert!(is_newer("2026.1108.0", "2026.205.3"));
        assert!(is_newer("2026.205.10", "2026.205.9"));
        assert!(!is_newer("2026.205.0", "2026.205.0"));
        assert!(!is_newer("0.0.0", "2026.205.0"));
    }
}
