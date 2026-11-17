// apps/desktop/shared/protocol.ts is written from the Rust types; this fails when the checked-in file is stale.
//
//   UPDATE_PROTOCOL_TS=1 cargo test -p linker-edit-protocol --test typescript

use std::path::{Path, PathBuf};

use dprint_plugin_typescript::configuration::{ConfigurationBuilder, QuoteProps, QuoteStyle};
use dprint_plugin_typescript::{FormatTextOptions, format_text};

fn protocol_ts() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/desktop/shared/protocol.ts")
}

/// The generated TypeScript, laid out like the rest of apps/desktop.
fn formatted() -> String {
    let text = linker_edit_protocol::typescript::typescript();
    let config = ConfigurationBuilder::new()
        .line_width(120)
        .indent_width(2)
        .quote_style(QuoteStyle::AlwaysDouble)
        .quote_props(QuoteProps::AsNeeded)
        .build();
    let path = protocol_ts();
    let options = FormatTextOptions {
        path: &path,
        extension: None,
        text: text.clone(),
        config: &config,
        external_formatter: None,
    };
    format_text(options)
        .expect("the generated TypeScript parses")
        .unwrap_or(text)
}

#[test]
fn protocol_ts_is_up_to_date() {
    let path = protocol_ts();
    let generated = formatted();
    if std::env::var_os("UPDATE_PROTOCOL_TS").is_some() {
        std::fs::write(&path, &generated).unwrap();
        return;
    }
    let checked_in = std::fs::read_to_string(&path)
        .unwrap()
        .replace("\r\n", "\n");
    assert!(
        checked_in == generated,
        "apps/desktop/shared/protocol.ts is stale: run `UPDATE_PROTOCOL_TS=1 cargo test -p linker-edit-protocol --test typescript`"
    );
}
