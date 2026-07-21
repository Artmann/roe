use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

/// Files larger than this are skipped to avoid choking on generated/bundled
/// monsters (mirrors fallow's max-file-size guard).
pub const MAX_FILE_SIZE: u64 = 5 * 1024 * 1024;

#[derive(Debug, Default)]
pub struct WalkedFiles {
    pub cs_files: Vec<PathBuf>,
    pub sln_files: Vec<PathBuf>,
    pub csproj_files: Vec<PathBuf>,
    pub skipped_large: Vec<PathBuf>,
}

/// Gitignore-aware walk of the analysis root. Skips bin/ and obj/ entirely —
/// obj/ is harvested separately (reference-only) per project.
pub fn walk(root: &Path) -> WalkedFiles {
    let mut result = WalkedFiles::default();

    let walker = WalkBuilder::new(root)
        .follow_links(false)
        .filter_entry(|entry| {
            let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
            if !is_dir {
                return true;
            }
            !is_build_output_dir(entry.file_name().to_string_lossy().as_ref())
        })
        .build();

    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.path();
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        match ext.to_ascii_lowercase().as_str() {
            "cs" => {
                if entry.metadata().is_ok_and(|m| m.len() > MAX_FILE_SIZE) {
                    result.skipped_large.push(path.to_path_buf());
                } else {
                    result.cs_files.push(path.to_path_buf());
                }
            }
            "sln" => result.sln_files.push(path.to_path_buf()),
            "csproj" => result.csproj_files.push(path.to_path_buf()),
            _ => {}
        }
    }

    result.cs_files.sort();
    result.sln_files.sort();
    result.csproj_files.sort();
    result
}

/// Walk a project's obj/ directory for generated .cs files (source-generator
/// output, GlobalUsings.g.cs). No gitignore filtering — obj/ is always ignored
/// by git, but we want its contents as reference-only sources.
pub fn walk_obj(project_root: &Path) -> Vec<PathBuf> {
    let obj_dir = project_root.join("obj");
    if !obj_dir.is_dir() {
        return Vec::new();
    }

    let mut files = Vec::new();
    let walker = WalkBuilder::new(&obj_dir)
        .standard_filters(false)
        .follow_links(false)
        .build();
    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("cs")
            && entry.metadata().is_ok_and(|m| m.len() <= MAX_FILE_SIZE)
        {
            files.push(path.to_path_buf());
        }
    }
    files.sort();
    files
}

fn is_build_output_dir(name: &str) -> bool {
    name.eq_ignore_ascii_case("bin") || name.eq_ignore_ascii_case("obj")
}
