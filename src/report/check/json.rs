use serde::Serialize;

use crate::cli::DupeMode;
use crate::commands::{dead_code, dupes, health};
use crate::report;

/// The combined report. Each nested field is the exact same v1 document the
/// individual command emits, so a consumer can lift `deadCode` out of this and
/// hand it to tooling that already reads `roe dead-code --format json`. That is
/// also why the nested reports keep their own `version` and `root` fields.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonReport<'a> {
    version: u32,
    root: String,
    dead_code: report::json::JsonReport<'a>,
    dupes: report::dupes::json::JsonReport,
    health: report::health::json::JsonReport,
}

pub fn print(
    dead_code: &dead_code::Analysis,
    dupes: &dupes::Analysis,
    health: &health::Analysis,
    mode: DupeMode,
) {
    let report = JsonReport {
        version: 1,
        root: crate::paths::display(&dead_code.workspace.root),
        dead_code: report::json::build(&dead_code.result, &dead_code.workspace),
        dupes: report::dupes::json::build(&dupes.result, &dupes.workspace, mode),
        health: report::health::json::build(&health.result, &health.workspace),
    };

    match serde_json::to_string_pretty(&report) {
        Ok(json) => println!("{json}"),
        Err(error) => eprintln!("error: failed to serialize JSON report: {error}"),
    }
}
