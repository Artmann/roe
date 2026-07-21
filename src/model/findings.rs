use std::path::PathBuf;

use serde::Serialize;

use crate::model::{SymbolKind, Visibility};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FindingKind {
    UnusedType,
    UnusedMember,
    UnusedFile,
}

#[derive(Debug)]
pub struct Finding {
    pub kind: FindingKind,
    pub symbol_kind: Option<SymbolKind>,
    /// Fully-qualified name for symbols; relative path display for files.
    pub name: String,
    pub project: Option<String>,
    pub file: PathBuf,
    pub line: u32,
    pub column: u32,
    pub visibility: Option<Visibility>,
}

#[derive(Debug, Default)]
pub struct Summary {
    pub projects: usize,
    pub files_scanned: usize,
    pub symbols: usize,
    pub unused_types: usize,
    pub unused_members: usize,
    pub unused_files: usize,
    pub elapsed_ms: u128,
}

#[derive(Debug)]
pub struct AnalysisResult {
    pub findings: Vec<Finding>,
    pub summary: Summary,
    /// Notes about analysis mode (e.g. library mode) surfaced in output.
    pub notes: Vec<String>,
}
