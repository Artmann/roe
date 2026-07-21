use std::path::{Path, PathBuf};

use globset::{Glob, GlobSet, GlobSetBuilder};
use rustc_hash::FxHashMap;

use crate::model::{AnalysisResult, Finding, FindingKind, Summary, Workspace};

/// eslint-style inline suppression, always applied (no config needed).
/// Re-reads each unique finding's source file once and looks for
/// `// roe-ignore-line [rule[,rule...]]` (suppresses a finding on the same
/// line) or `// roe-ignore-next-line [rule[,rule...]]` (suppresses a finding
/// on the line below). Omitting the rule list suppresses any finding kind on
/// that line. Because `UnusedFile` findings are pinned at a synthetic
/// line 1/column 1, a marker targeting `unused-file` (or a bare marker)
/// suppresses that file's dead-file finding no matter which line it sits on.
pub fn apply_inline_suppressions(result: &mut AnalysisResult, workspace: &mut Workspace) {
    let mut cache: FxHashMap<PathBuf, FileSuppressions> = FxHashMap::default();
    result.findings.retain(|finding| {
        let suppressions = cache.entry(finding.file.clone()).or_insert_with(|| {
            match std::fs::read_to_string(&finding.file) {
                Ok(source) => parse_suppressions(&source, &finding.file, &mut workspace.warnings),
                Err(_) => FileSuppressions::default(),
            }
        });
        !suppressions.suppresses(finding.line, finding.kind)
    });
    recount(&result.findings, &mut result.summary);
}

/// Config-driven ignore list: glob patterns (relative to `config_dir`) whose
/// matching files have all their findings suppressed, regardless of kind.
pub fn apply_config_ignores(
    result: &mut AnalysisResult,
    ignore_patterns: &[String],
    config_dir: &Path,
    warnings: &mut Vec<String>,
) {
    let Some(set) = build_ignore_globset(config_dir, ignore_patterns, warnings) else {
        return;
    };
    result.findings.retain(|finding| !set.is_match(&finding.file));
    recount(&result.findings, &mut result.summary);
}

fn build_ignore_globset(
    config_dir: &Path,
    patterns: &[String],
    warnings: &mut Vec<String>,
) -> Option<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    let mut any = false;
    for pattern in patterns {
        if pattern.contains("..") {
            warnings.push(format!("unsupported ignore glob with '..': {pattern}"));
            continue;
        }
        // A trailing slash reads as "this whole directory" without the user
        // having to spell out `**` themselves.
        let expanded = match pattern.strip_suffix('/') {
            Some(dir) => format!("{dir}/**"),
            None => pattern.clone(),
        };
        let absolute = format!("{}/{}", config_dir.display(), expanded);
        match Glob::new(&absolute) {
            Ok(glob) => {
                builder.add(glob);
                any = true;
            }
            Err(error) => warnings.push(format!("invalid ignore glob {pattern}: {error}")),
        }
    }
    if !any {
        return None;
    }
    builder.build().ok()
}

fn recount(findings: &[Finding], summary: &mut Summary) {
    summary.unused_types = findings
        .iter()
        .filter(|f| f.kind == FindingKind::UnusedType)
        .count();
    summary.unused_members = findings
        .iter()
        .filter(|f| f.kind == FindingKind::UnusedMember)
        .count();
    summary.unused_files = findings
        .iter()
        .filter(|f| f.kind == FindingKind::UnusedFile)
        .count();
}

#[derive(Debug, Clone)]
enum RuleFilter {
    Any,
    Only(Vec<FindingKind>),
}

impl RuleFilter {
    fn matches(&self, kind: FindingKind) -> bool {
        match self {
            RuleFilter::Any => true,
            RuleFilter::Only(kinds) => kinds.contains(&kind),
        }
    }
}

#[derive(Debug, Default)]
struct FileSuppressions {
    by_line: FxHashMap<u32, Vec<RuleFilter>>,
    /// Filters from a marker anywhere in the file that target `UnusedFile`
    /// (explicitly or via a bare marker) — see the file-level doc comment.
    file_wide: Vec<RuleFilter>,
}

impl FileSuppressions {
    fn suppresses(&self, line: u32, kind: FindingKind) -> bool {
        if kind == FindingKind::UnusedFile && self.file_wide.iter().any(|f| f.matches(kind)) {
            return true;
        }
        self.by_line
            .get(&line)
            .is_some_and(|filters| filters.iter().any(|f| f.matches(kind)))
    }
}

fn parse_suppressions(source: &str, file: &Path, warnings: &mut Vec<String>) -> FileSuppressions {
    let mut result = FileSuppressions::default();
    for (index, line) in source.lines().enumerate() {
        let line_no = index as u32 + 1;
        let Some(comment_at) = line.find("//") else {
            continue;
        };
        let comment = line[comment_at + 2..].trim_start();
        let (marker_rest, target_line) = if let Some(rest) = comment.strip_prefix("roe-ignore-next-line") {
            (rest, line_no + 1)
        } else if let Some(rest) = comment.strip_prefix("roe-ignore-line") {
            (rest, line_no)
        } else {
            continue;
        };

        let filter = parse_rule_filter(marker_rest, file, line_no, warnings);
        if filter.matches(FindingKind::UnusedFile) {
            result.file_wide.push(filter.clone());
        }
        result.by_line.entry(target_line).or_default().push(filter);
    }
    result
}

/// `remainder` is whatever followed the marker on the comment line. The first
/// whitespace-separated token is the (optional) comma-separated rule list;
/// anything after that is treated as free-form trailing note text and
/// ignored. An empty/absent rule list means "suppress any finding kind."
fn parse_rule_filter(remainder: &str, file: &Path, line_no: u32, warnings: &mut Vec<String>) -> RuleFilter {
    let rule_list = remainder.split_whitespace().next().unwrap_or("");
    if rule_list.is_empty() {
        return RuleFilter::Any;
    }
    let mut kinds = Vec::new();
    for token in rule_list.split(',') {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        match parse_kind(token) {
            Some(kind) => kinds.push(kind),
            None => warnings.push(format!(
                "unknown suppression rule '{token}' at {}:{line_no} (expected unused-type, unused-member, or unused-file)",
                file.display()
            )),
        }
    }
    RuleFilter::Only(kinds)
}

fn parse_kind(token: &str) -> Option<FindingKind> {
    match token {
        "unused-type" => Some(FindingKind::UnusedType),
        "unused-member" => Some(FindingKind::UnusedMember),
        "unused-file" => Some(FindingKind::UnusedFile),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_marker_suppresses_any_kind() {
        let mut warnings = Vec::new();
        let suppressions = parse_suppressions(
            "// roe-ignore-next-line\nclass Dead {}\n",
            Path::new("x.cs"),
            &mut warnings,
        );
        assert!(suppressions.suppresses(2, FindingKind::UnusedType));
        assert!(suppressions.suppresses(2, FindingKind::UnusedMember));
        assert!(warnings.is_empty());
    }

    #[test]
    fn rule_scoped_marker_only_matches_that_kind() {
        let mut warnings = Vec::new();
        let suppressions = parse_suppressions(
            "// roe-ignore-next-line unused-type\nclass Dead {}\n",
            Path::new("x.cs"),
            &mut warnings,
        );
        assert!(suppressions.suppresses(2, FindingKind::UnusedType));
        assert!(!suppressions.suppresses(2, FindingKind::UnusedMember));
    }

    #[test]
    fn same_line_marker_targets_its_own_line() {
        let mut warnings = Vec::new();
        let suppressions = parse_suppressions(
            "var unused = 1; // roe-ignore-line unused-member\n",
            Path::new("x.cs"),
            &mut warnings,
        );
        assert!(suppressions.suppresses(1, FindingKind::UnusedMember));
        assert!(!suppressions.suppresses(2, FindingKind::UnusedMember));
    }

    #[test]
    fn unknown_rule_name_warns_and_matches_nothing() {
        let mut warnings = Vec::new();
        let suppressions = parse_suppressions(
            "// roe-ignore-next-line unused-mmeber\nvoid Foo() {}\n",
            Path::new("x.cs"),
            &mut warnings,
        );
        assert!(!suppressions.suppresses(2, FindingKind::UnusedMember));
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("unused-mmeber"));
    }

    #[test]
    fn unused_file_marker_matches_anywhere_in_file() {
        let mut warnings = Vec::new();
        let suppressions = parse_suppressions(
            "using System;\n\n// roe-ignore-line unused-file\nclass Dead {}\n",
            Path::new("x.cs"),
            &mut warnings,
        );
        assert!(suppressions.suppresses(1, FindingKind::UnusedFile));
    }

    #[test]
    fn ignore_glob_trailing_slash_matches_whole_directory() {
        let mut warnings = Vec::new();
        let set = build_ignore_globset(
            Path::new("/repo"),
            &["Generated/".to_string()],
            &mut warnings,
        )
        .expect("globset builds");
        assert!(set.is_match(Path::new("/repo/Generated/Nested/File.cs")));
        assert!(!set.is_match(Path::new("/repo/Other/File.cs")));
        assert!(warnings.is_empty());
    }
}
