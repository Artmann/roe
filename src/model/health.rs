use std::path::PathBuf;

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum HealthFindingKind {
    HighComplexity,
    HighCognitiveComplexity,
    LongMethod,
    TooManyParameters,
    LargeFile,
    LargeType,
}

#[derive(Debug)]
pub struct HealthFinding {
    pub kind: HealthFindingKind,
    /// Fully-qualified member/type name for symbol-level findings, relative
    /// path display for file-level findings.
    pub name: String,
    pub project: Option<String>,
    pub file: PathBuf,
    pub line: u32,
    pub column: u32,
    /// The measured value that tripped the threshold — cyclomatic
    /// complexity, line count, parameter count, or member count, depending
    /// on `kind`.
    pub metric: u32,
    pub threshold: u32,
}

/// One type participating in a circular dependency.
#[derive(Debug)]
pub struct CycleMember {
    pub name: String,
    pub project: Option<String>,
    pub file: PathBuf,
    pub line: u32,
    pub column: u32,
}

/// A set of types whose type-level dependencies form a cycle.
#[derive(Debug)]
pub struct CircularDependency {
    pub members: Vec<CycleMember>,
}

/// A file that is both densely complex and frequently changed — the
/// intersection where defects concentrate and changes are expensive.
#[derive(Debug)]
pub struct Hotspot {
    pub file: PathBuf,
    pub project: Option<String>,
    /// 0–100, normalized within the workspace so the riskiest file scores
    /// close to 100. Only comparable against other files in the same run.
    pub score: f64,
    /// Recency-weighted commit count, on a 90-day half-life.
    pub weighted_commits: f64,
    /// Summed cyclomatic complexity of every member declared in the file.
    pub cyclomatic: u32,
    pub lines: u32,
    /// `cyclomatic / lines` — how tightly packed the complexity is.
    pub complexity_density: f64,
}

#[derive(Debug, Default)]
pub struct HealthSummary {
    pub projects: usize,
    pub files_scanned: usize,
    pub symbols: usize,
    pub high_complexity: usize,
    pub high_cognitive_complexity: usize,
    pub long_methods: usize,
    pub too_many_parameters: usize,
    pub large_files: usize,
    pub large_types: usize,
    pub circular_dependencies: usize,
    /// Commits walked for hotspot scoring. `None` when `--hotspots` was not
    /// requested, which is what distinguishes "not asked for" from "no
    /// history".
    pub commits_walked: Option<usize>,
    pub elapsed_ms: u128,
}

#[derive(Debug)]
pub struct HealthResult {
    pub findings: Vec<HealthFinding>,
    pub cycles: Vec<CircularDependency>,
    /// Ranked highest-risk first. Empty unless `--hotspots` was requested.
    /// Informational only — hotspots never affect the exit code, since every
    /// codebase has a riskiest file and that is not a failure.
    pub hotspots: Vec<Hotspot>,
    pub summary: HealthSummary,
}
