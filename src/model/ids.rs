/// Dense index into the path-sorted list of discovered source files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileId(pub u32);

/// Dense index into the list of discovered projects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProjectId(pub u32);

/// Dense index into the merged symbol table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SymbolId(pub u32);

impl FileId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

impl ProjectId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

impl SymbolId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}
