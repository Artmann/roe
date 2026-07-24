use std::path::{Path, PathBuf};

use rustc_hash::FxHashMap;

use crate::config::build_ignore_globset;
use crate::model::{
    AnalysisResult, Finding, FindingKind, HealthFindingKind, HealthResult, Summary, Workspace,
};

/// A rule namespace the inline markers can target. Each command has its own
/// set of rule names, but the marker syntax and the file-wide behaviour are
/// shared, so the parser below is generic over this rather than duplicated.
pub trait SuppressibleKind: Copy + PartialEq {
    /// The kebab-case rule name as it appears in JSON output and in an
    /// `// roe-ignore-line` comment.
    fn from_rule_name(token: &str) -> Option<Self>;

    /// True for findings pinned at a synthetic line 1, which no marker could
    /// realistically sit next to — a marker anywhere in the file suppresses
    /// them instead.
    fn is_file_wide(self) -> bool;

    /// The accepted rule names, for the "unknown suppression rule" warning.
    fn rule_names() -> &'static str;
}

impl SuppressibleKind for FindingKind {
    fn from_rule_name(token: &str) -> Option<Self> {
        match token {
            "unused-type" => Some(FindingKind::UnusedType),
            "unused-member" => Some(FindingKind::UnusedMember),
            "unused-file" => Some(FindingKind::UnusedFile),
            _ => None,
        }
    }

    fn is_file_wide(self) -> bool {
        self == FindingKind::UnusedFile
    }

    fn rule_names() -> &'static str {
        "unused-type, unused-member, or unused-file"
    }
}

impl SuppressibleKind for HealthFindingKind {
    fn from_rule_name(token: &str) -> Option<Self> {
        match token {
            "high-complexity" => Some(HealthFindingKind::HighComplexity),
            "high-cognitive-complexity" => Some(HealthFindingKind::HighCognitiveComplexity),
            "long-method" => Some(HealthFindingKind::LongMethod),
            "too-many-parameters" => Some(HealthFindingKind::TooManyParameters),
            "large-file" => Some(HealthFindingKind::LargeFile),
            "large-type" => Some(HealthFindingKind::LargeType),
            _ => None,
        }
    }

    fn is_file_wide(self) -> bool {
        self == HealthFindingKind::LargeFile
    }

    fn rule_names() -> &'static str {
        "high-complexity, high-cognitive-complexity, long-method, \
         too-many-parameters, large-file, or large-type"
    }
}

/// eslint-style inline suppression, always applied (no config needed).
/// Re-reads each unique finding's source file once and looks for
/// `// roe-ignore-line [rule[,rule...]]` (suppresses a finding on the same
/// line) or `// roe-ignore-next-line [rule[,rule...]]` (suppresses a finding
/// on the line below). Omitting the rule list suppresses any finding kind on
/// that line. Because `UnusedFile` findings are pinned at a synthetic
/// line 1/column 1, a marker targeting `unused-file` (or a bare marker)
/// suppresses that file's dead-file finding no matter which line it sits on.
pub fn apply_inline_suppressions(result: &mut AnalysisResult, workspace: &mut Workspace) {
    let mut cache: SuppressionCache<FindingKind> = SuppressionCache::default();
    result
        .findings
        .retain(|finding| !cache.suppresses(&finding.file, finding.line, finding.kind, workspace));
    recount(&result.findings, &mut result.summary);
}

/// The same markers, applied to `roe health`. `large-file` is the file-wide
/// rule here, for the same reason `unused-file` is above.
///
/// Circular dependencies are deliberately not suppressible this way: a cycle
/// spans several types in several files, so there is no one line a marker
/// could sit on. Use a config `ignore` glob instead — the same call the
/// `dupes` command makes for the same reason.
pub fn apply_inline_suppressions_health(result: &mut HealthResult, workspace: &mut Workspace) {
    let mut cache: SuppressionCache<HealthFindingKind> = SuppressionCache::default();
    result
        .findings
        .retain(|finding| !cache.suppresses(&finding.file, finding.line, finding.kind, workspace));
}

/// Parsed suppressions per source file, so a file with several findings is
/// only read and scanned once.
struct SuppressionCache<K: SuppressibleKind> {
    files: FxHashMap<PathBuf, FileSuppressions<K>>,
}

impl<K: SuppressibleKind> Default for SuppressionCache<K> {
    fn default() -> Self {
        SuppressionCache {
            files: FxHashMap::default(),
        }
    }
}

impl<K: SuppressibleKind> SuppressionCache<K> {
    /// An unreadable file suppresses nothing — reporting the finding is the
    /// safe direction, and discovery has already warned about the file.
    fn suppresses(&mut self, file: &Path, line: u32, kind: K, workspace: &mut Workspace) -> bool {
        let suppressions = self.files.entry(file.to_path_buf()).or_insert_with(|| {
            match std::fs::read_to_string(file) {
                Ok(source) => parse_suppressions(&source, file, &mut workspace.warnings),
                Err(_) => FileSuppressions::default(),
            }
        });

        suppressions.suppresses(line, kind)
    }
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
    result
        .findings
        .retain(|finding| !set.is_match(&finding.file));
    recount(&result.findings, &mut result.summary);
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
enum RuleFilter<K> {
    Any,
    Only(Vec<K>),
}

impl<K: SuppressibleKind> RuleFilter<K> {
    fn matches(&self, kind: K) -> bool {
        match self {
            RuleFilter::Any => true,
            RuleFilter::Only(kinds) => kinds.contains(&kind),
        }
    }

    /// Whether this filter could suppress a file-wide finding, and so needs
    /// to be remembered beyond the line it was written on.
    fn targets_file_wide(&self) -> bool {
        match self {
            RuleFilter::Any => true,
            RuleFilter::Only(kinds) => kinds.iter().any(|kind| kind.is_file_wide()),
        }
    }
}

#[derive(Debug)]
struct FileSuppressions<K> {
    by_line: FxHashMap<u32, Vec<RuleFilter<K>>>,
    /// Filters from a marker anywhere in the file that target a file-wide
    /// kind (explicitly or via a bare marker) — see the file-level doc
    /// comment.
    file_wide: Vec<RuleFilter<K>>,
}

impl<K> Default for FileSuppressions<K> {
    fn default() -> Self {
        FileSuppressions {
            by_line: FxHashMap::default(),
            file_wide: Vec::new(),
        }
    }
}

impl<K: SuppressibleKind> FileSuppressions<K> {
    fn suppresses(&self, line: u32, kind: K) -> bool {
        if kind.is_file_wide() && self.file_wide.iter().any(|filter| filter.matches(kind)) {
            return true;
        }

        self.by_line
            .get(&line)
            .is_some_and(|filters| filters.iter().any(|filter| filter.matches(kind)))
    }
}

fn parse_suppressions<K: SuppressibleKind>(
    source: &str,
    file: &Path,
    warnings: &mut Vec<String>,
) -> FileSuppressions<K> {
    let mut result = FileSuppressions::default();
    for (index, line) in source.lines().enumerate() {
        let line_no = index as u32 + 1;
        let Some(comment_at) = line.find("//") else {
            continue;
        };
        let comment = line[comment_at + 2..].trim_start();
        let (marker_rest, target_line) =
            if let Some(rest) = comment.strip_prefix("roe-ignore-next-line") {
                (rest, line_no + 1)
            } else if let Some(rest) = comment.strip_prefix("roe-ignore-line") {
                (rest, line_no)
            } else {
                continue;
            };

        let filter = parse_rule_filter(marker_rest, file, line_no, warnings);
        if filter.targets_file_wide() {
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
fn parse_rule_filter<K: SuppressibleKind>(
    remainder: &str,
    file: &Path,
    line_no: u32,
    warnings: &mut Vec<String>,
) -> RuleFilter<K> {
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
        match K::from_rule_name(token) {
            Some(kind) => kinds.push(kind),
            None => warnings.push(format!(
                "unknown suppression rule '{token}' at {}:{line_no} (expected {})",
                file.display(),
                K::rule_names()
            )),
        }
    }

    RuleFilter::Only(kinds)
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
}
