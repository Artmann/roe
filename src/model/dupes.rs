use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Occurrence {
    pub file: PathBuf,
    pub start_line: u32,
    pub start_column: u32,
    pub end_line: u32,
    pub end_column: u32,
}

#[derive(Debug)]
pub struct DupeGroup {
    pub token_count: u32,
    /// Minimum line span across all occurrences — a group only survives the
    /// `--min-lines` filter if every one of its occurrences is long enough.
    pub line_count: u32,
    pub occurrences: Vec<Occurrence>,
}

#[derive(Debug, Default)]
pub struct DupesSummary {
    pub projects: usize,
    pub files_scanned: usize,
    pub groups: usize,
    /// Total lines covered by all occurrences across all groups (each
    /// instance of a duplicate counted, not just the redundant copies).
    pub duplicated_lines: usize,
    pub elapsed_ms: u128,
}

#[derive(Debug)]
pub struct DupesResult {
    pub groups: Vec<DupeGroup>,
    pub summary: DupesSummary,
}
