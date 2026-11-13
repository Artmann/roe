pub mod check;
pub mod dupes;
pub mod health;
pub(crate) mod human;
pub(crate) mod json;
pub mod perf;

use std::process::ExitCode;

use crate::cli::OutputFormat;
use crate::model::{AnalysisResult, Workspace};

pub(crate) fn pluralize(count: usize, noun: &str) -> String {
    format!("{count} {noun}{}", if count == 1 { "" } else { "s" })
}

/// Naming the projects answers the question a reader is really asking — "was
/// my project even detected as a test project?" — but a solution with twenty
/// of them must not push the findings off the screen, so the list stops at
/// three.
pub(crate) fn name_projects(names: &[String]) -> String {
    if names.len() <= 3 {
        return names.join(", ");
    }

    format!("{}, +{} more", names[..3].join(", "), names.len() - 3)
}

pub fn emit(result: &AnalysisResult, workspace: &Workspace, format: OutputFormat) -> ExitCode {
    match format {
        OutputFormat::Human => human::print(result, workspace),
        OutputFormat::Json => json::print(result, workspace),
    }
    if result.has_findings() {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

/// Turn colour on for the duration of a test, and off again when the returned
/// guard drops. The harness runs with `NO_COLOR`, under which every band
/// renders identically and a test of which colour was chosen would assert
/// nothing.
///
/// Whether `colored` emits escapes at all is process-wide state, so every
/// colour test in the crate has to take turns on *this* lock. A second lock
/// elsewhere would guard the same state without coordinating with this one,
/// which is exactly the race it looks like it prevents.
#[cfg(test)]
pub(crate) fn forcing_color() -> impl Drop {
    use std::sync::{Mutex, MutexGuard};

    static COLOR_OVERRIDE: Mutex<()> = Mutex::new(());

    // The lock is held for its lifetime alone and never read, hence the
    // underscore — a plain field name would read as dead code.
    struct Guard {
        _lock: MutexGuard<'static, ()>,
    }

    impl Drop for Guard {
        fn drop(&mut self) {
            colored::control::unset_override();
        }
    }

    // A poisoned lock means another colour test panicked; the state it guards
    // is reset below either way, so carry on rather than cascade.
    let lock = COLOR_OVERRIDE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    colored::control::set_override(true);

    Guard { _lock: lock }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_handful_of_excluded_projects_are_all_named() {
        let names = ["Api.Tests".to_string(), "Lib.Tests".to_string()];

        assert_eq!(name_projects(&names), "Api.Tests, Lib.Tests");
    }

    #[test]
    fn a_long_list_of_excluded_projects_is_truncated() {
        // A solution with twenty test projects must not push the findings off
        // the screen.
        let names: Vec<String> = (1..=6).map(|index| format!("P{index}.Tests")).collect();

        assert_eq!(
            name_projects(&names),
            "P1.Tests, P2.Tests, P3.Tests, +3 more"
        );
    }
}
