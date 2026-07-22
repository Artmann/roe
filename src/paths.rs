//! Path handling that stays consistent across platforms.
//!
//! Windows needs two corrections the rest of the codebase shouldn't have to
//! think about: `std::fs::canonicalize` returns verbatim (`\\?\C:\...`)
//! paths, and native rendering uses backslashes. Reports promise a stable
//! schema across operating systems, so paths are always rendered with `/`.

use std::path::{Path, PathBuf};

/// Canonicalize without producing Windows verbatim (`\\?\`) paths.
///
/// Canonicalized paths are compared and prefix-stripped against each other
/// all over the codebase, so every canonicalization must go through this —
/// mixing verbatim and non-verbatim flavors would break those comparisons.
pub fn canonicalize(path: &Path) -> std::io::Result<PathBuf> {
    dunce::canonicalize(path)
}

/// Render a path for report output with `/` separators on every platform.
///
/// On Unix a backslash is an ordinary filename character, so the text is
/// only rewritten on Windows, where `\` is always a separator.
pub fn display(path: &Path) -> String {
    let text = path.display().to_string();

    if cfg!(windows) {
        text.replace('\\', "/")
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_uses_forward_slashes_on_every_platform() {
        let path = Path::new("src").join("App").join("Program.cs");

        assert_eq!(display(&path), "src/App/Program.cs");
    }

    #[cfg(windows)]
    #[test]
    fn canonicalize_strips_the_verbatim_prefix() {
        let current_directory = std::env::current_dir().expect("cwd exists");
        let canonical = canonicalize(&current_directory).expect("cwd canonicalizes");

        assert!(!canonical.display().to_string().starts_with(r"\\?\"));
    }
}
