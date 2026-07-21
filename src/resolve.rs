use bitflags::bitflags;
use lasso::{Spur, ThreadedRodeo};
use rustc_hash::FxHashMap;
use smallvec::SmallVec;

use crate::extract::{FileFacts, NamePath};
use crate::model::{FileId, MemberKind, Modifiers, SourceFile, SymbolId, SymbolKind, Visibility};

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct SymbolFlags: u8 {
        /// Declared in a generated file: reference-only, never flagged, root.
        const GENERATED = 1;
        /// Entry point — seeds the reachability BFS.
        const ROOT = 1 << 1;
        /// Kept alive by its containing type (containment-down edge).
        const LIVE_WITH_TYPE = 1 << 2;
        /// Root that came from a test context (categorization).
        const TEST_ROOT = 1 << 3;
        /// Reachable by name match alone, without the containing-type gate
        /// (extension methods: call sites never name the static class).
        const NO_TYPE_GATE = 1 << 4;
    }
}

/// A merged symbol: partial type declarations, method overloads, and
/// partial-method halves all collapse into one Symbol.
#[derive(Debug)]
pub struct Symbol {
    pub id: SymbolId,
    pub name: Spur,
    pub kind: SymbolKind,
    pub parent: Option<SymbolId>,
    pub file: FileId,
    pub line: u32,
    pub column: u32,
    pub modifiers: Modifiers,
    pub attributes: SmallVec<[Spur; 4]>,
    /// base_list name paths (types only), unresolved.
    pub base_names: SmallVec<[NamePath; 2]>,
    pub namespace: NamePath,
    /// Interned dotted fully-qualified name (types only).
    pub fqn: Option<Spur>,
    pub flags: SymbolFlags,
    pub is_explicit_interface_impl: bool,
    pub is_extension_method: bool,
    pub is_auto_property: bool,
    pub has_body: bool,
    pub arity: u8,
}

impl Symbol {
    pub fn is_reachable_gate(&self) -> bool {
        self.kind.is_member()
    }

    pub fn visibility(&self) -> Visibility {
        let is_top_level_type = self.kind.is_type() && self.parent.is_none();
        Visibility::from_modifiers(self.modifiers, is_top_level_type)
    }

    pub fn is_public_surface(&self) -> bool {
        matches!(
            self.visibility(),
            Visibility::Public | Visibility::Protected | Visibility::ProtectedInternal
        )
    }
}

#[derive(Debug)]
pub struct Resolution {
    pub symbols: Vec<Symbol>,
    /// Synthetic per-file root symbol (top-level statements, assembly
    /// attributes, generated-file references), indexed by FileId.
    pub file_roots: Vec<SymbolId>,
    /// All types sharing a fully-qualified name — `IValidator` and
    /// `IValidator<T>` collide here, and a textual reference cannot reliably
    /// distinguish arity, so a match marks every entry.
    pub types_by_fqn: FxHashMap<Spur, SmallVec<[SymbolId; 2]>>,
    pub types_by_simple: FxHashMap<Spur, SmallVec<[SymbolId; 2]>>,
    pub members_by_name: FxHashMap<Spur, SmallVec<[SymbolId; 4]>>,
    /// Per file: local decl index → merged SymbolId.
    pub decl_map: Vec<Vec<SymbolId>>,
}

impl Resolution {
    pub fn display_name(&self, id: SymbolId, rodeo: &ThreadedRodeo) -> String {
        let symbol = &self.symbols[id.index()];
        if let Some(fqn) = symbol.fqn {
            return rodeo.resolve(&fqn).to_string();
        }
        match symbol.parent {
            Some(parent) => format!(
                "{}.{}",
                self.display_name(parent, rodeo),
                rodeo.resolve(&symbol.name)
            ),
            None => rodeo.resolve(&symbol.name).to_string(),
        }
    }
}

/// Build the merged symbol table from per-file extraction facts.
///
/// Merge keys: types by (FQN, generic arity) — partial classes and `#if`
/// duplicates collapse; members by (containing type, name, kind) — overloads
/// and partial-method halves collapse. Merging unions modifiers/attributes,
/// which only ever adds liveness exemptions (the safe direction).
pub fn build_symbols(
    files: &[SourceFile],
    facts: &[FileFacts],
    rodeo: &ThreadedRodeo,
) -> Resolution {
    let mut symbols: Vec<Symbol> = Vec::new();
    let mut file_roots: Vec<SymbolId> = Vec::with_capacity(files.len());
    let mut types_by_fqn: FxHashMap<Spur, SmallVec<[SymbolId; 2]>> = FxHashMap::default();
    let mut types_by_simple: FxHashMap<Spur, SmallVec<[SymbolId; 2]>> = FxHashMap::default();
    let mut members_by_name: FxHashMap<Spur, SmallVec<[SymbolId; 4]>> = FxHashMap::default();
    let mut type_merge: FxHashMap<(Spur, u8), SymbolId> = FxHashMap::default();
    let mut member_merge: FxHashMap<(SymbolId, Spur, SymbolKind), SymbolId> = FxHashMap::default();
    let mut decl_map: Vec<Vec<SymbolId>> = Vec::with_capacity(facts.len());

    let file_root_name = rodeo.get_or_intern("<file>");

    for file_facts in facts {
        let file = file_facts.file;
        let is_generated = file_facts.is_generated;

        let root_id = SymbolId(symbols.len() as u32);
        symbols.push(Symbol {
            id: root_id,
            name: file_root_name,
            kind: SymbolKind::FileRoot,
            parent: None,
            file,
            line: 1,
            column: 1,
            modifiers: Modifiers::empty(),
            attributes: SmallVec::new(),
            base_names: SmallVec::new(),
            namespace: NamePath::new(),
            fqn: None,
            flags: if is_generated {
                SymbolFlags::GENERATED
            } else {
                SymbolFlags::empty()
            },
            is_explicit_interface_impl: false,
            is_extension_method: false,
            is_auto_property: false,
            has_body: false,
            arity: 0,
        });
        file_roots.push(root_id);

        let mut local_map: Vec<SymbolId> = Vec::with_capacity(file_facts.decls.len());

        for decl in &file_facts.decls {
            let generated_flag = if is_generated {
                SymbolFlags::GENERATED
            } else {
                SymbolFlags::empty()
            };

            let merged_id = if decl.kind.is_type() {
                let fqn = intern_type_fqn(decl, &local_map, &symbols, rodeo);
                match type_merge.get(&(fqn, decl.arity)) {
                    Some(&existing) => {
                        let symbol = &mut symbols[existing.index()];
                        symbol.modifiers |= decl.modifiers;
                        symbol.attributes.extend(decl.attributes.iter().copied());
                        symbol.base_names.extend(decl.base_names.iter().cloned());
                        symbol.flags |= generated_flag;
                        existing
                    }
                    None => {
                        let id = SymbolId(symbols.len() as u32);
                        let parent = decl.parent.map(|p| local_map[p as usize]);
                        symbols.push(Symbol {
                            id,
                            name: decl.name,
                            kind: decl.kind,
                            parent,
                            file,
                            line: decl.line,
                            column: decl.column,
                            modifiers: decl.modifiers,
                            attributes: decl.attributes.clone().into_iter().collect(),
                            base_names: decl.base_names.clone(),
                            namespace: decl.namespace.clone(),
                            fqn: Some(fqn),
                            flags: generated_flag,
                            is_explicit_interface_impl: false,
                            is_extension_method: false,
                            is_auto_property: false,
                            has_body: decl.has_body,
                            arity: decl.arity,
                        });
                        type_merge.insert((fqn, decl.arity), id);
                        types_by_fqn.entry(fqn).or_default().push(id);
                        types_by_simple.entry(decl.name).or_default().push(id);
                        id
                    }
                }
            } else {
                // Member: parent should always be a type; a member that
                // somehow lacks one attaches to the file root (never flagged).
                let parent = decl
                    .parent
                    .map(|p| local_map[p as usize])
                    .unwrap_or(root_id);
                // Interface members are public by default, not private.
                let mut modifiers = decl.modifiers;
                let visibility_bits = Modifiers::PUBLIC
                    | Modifiers::INTERNAL
                    | Modifiers::PROTECTED
                    | Modifiers::PRIVATE;
                if matches!(
                    symbols[parent.index()].kind,
                    SymbolKind::Type(crate::model::TypeKind::Interface)
                ) && !modifiers.intersects(visibility_bits)
                {
                    modifiers |= Modifiers::PUBLIC;
                }
                let key = (parent, decl.name, decl.kind);
                match member_merge.get(&key) {
                    Some(&existing) => {
                        let symbol = &mut symbols[existing.index()];
                        symbol.modifiers |= modifiers;
                        symbol.attributes.extend(decl.attributes.iter().copied());
                        symbol.has_body |= decl.has_body;
                        symbol.is_explicit_interface_impl |= decl.is_explicit_interface_impl;
                        symbol.is_extension_method |= decl.is_extension_method;
                        symbol.is_auto_property |= decl.is_auto_property;
                        symbol.flags |= generated_flag;
                        existing
                    }
                    None => {
                        let id = SymbolId(symbols.len() as u32);
                        symbols.push(Symbol {
                            id,
                            name: decl.name,
                            kind: decl.kind,
                            parent: Some(parent),
                            file,
                            line: decl.line,
                            column: decl.column,
                            modifiers,
                            attributes: decl.attributes.clone().into_iter().collect(),
                            base_names: SmallVec::new(),
                            namespace: NamePath::new(),
                            fqn: None,
                            flags: generated_flag,
                            is_explicit_interface_impl: decl.is_explicit_interface_impl,
                            is_extension_method: decl.is_extension_method,
                            is_auto_property: decl.is_auto_property,
                            has_body: decl.has_body,
                            arity: decl.arity,
                        });
                        member_merge.insert(key, id);
                        if decl.kind != SymbolKind::Member(MemberKind::Constructor)
                            && decl.kind != SymbolKind::Member(MemberKind::StaticConstructor)
                        {
                            members_by_name.entry(decl.name).or_default().push(id);
                        }
                        id
                    }
                }
            };
            local_map.push(merged_id);
        }

        decl_map.push(local_map);
    }

    Resolution {
        symbols,
        file_roots,
        types_by_fqn,
        types_by_simple,
        members_by_name,
        decl_map,
    }
}

/// FQN = namespace segments + enclosing type names + own name, dotted.
fn intern_type_fqn(
    decl: &crate::extract::RawDecl,
    local_map: &[SymbolId],
    symbols: &[Symbol],
    rodeo: &ThreadedRodeo,
) -> Spur {
    let mut fqn = String::new();
    if let Some(parent_local) = decl.parent {
        let parent_id = local_map[parent_local as usize];
        if let Some(parent_fqn) = symbols[parent_id.index()].fqn {
            fqn.push_str(rodeo.resolve(&parent_fqn));
        }
    } else {
        for segment in &decl.namespace {
            if !fqn.is_empty() {
                fqn.push('.');
            }
            fqn.push_str(rodeo.resolve(segment));
        }
    }
    if !fqn.is_empty() {
        fqn.push('.');
    }
    fqn.push_str(rodeo.resolve(&decl.name));
    rodeo.get_or_intern(&fqn)
}
