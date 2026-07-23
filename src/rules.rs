use lasso::ThreadedRodeo;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::model::{MemberKind, Modifiers, SymbolId, SymbolKind, Visibility, Workspace};
use crate::resolve::{Resolution, SymbolFlags};

/// Apply the false-positive kill list: mark members that must never be
/// flagged individually as LIVE_WITH_TYPE (their containing type keeps them
/// alive), and root declarations that are consumed invisibly (generated
/// files, source-generator partial methods).
pub fn apply_kill_list(
    resolution: &mut Resolution,
    workspace: &Workspace,
    rodeo: &ThreadedRodeo,
    aggressive: bool,
) {
    let interface_member_names = interface_satisfaction_sets(resolution, rodeo);

    for index in 0..resolution.symbols.len() {
        let symbol = &resolution.symbols[index];
        let mut flags = symbol.flags;

        match symbol.kind {
            SymbolKind::Member(kind) => {
                // Structurally invoked: call sites never name these.
                if matches!(
                    kind,
                    MemberKind::Constructor
                        | MemberKind::StaticConstructor
                        | MemberKind::Destructor
                        | MemberKind::Operator
                        | MemberKind::ConversionOperator
                        | MemberKind::Indexer
                ) {
                    flags |= SymbolFlags::LIVE_WITH_TYPE;
                }

                // Deconstruct: invoked implicitly by `var (a, b) = expr;`
                // deconstruction syntax — never named at the call site.
                if kind == MemberKind::Method && rodeo.resolve(&symbol.name) == "Deconstruct" {
                    flags |= SymbolFlags::LIVE_WITH_TYPE;
                }

                // Called via base-class or interface dispatch.
                if symbol.modifiers.contains(Modifiers::OVERRIDE)
                    || symbol.is_explicit_interface_impl
                {
                    flags |= SymbolFlags::LIVE_WITH_TYPE;
                }

                // Extension-method call sites (`"x".Shout()`) never name the
                // containing static class, so the type gate must not apply.
                if symbol.is_extension_method {
                    flags |= SymbolFlags::NO_TYPE_GATE;
                }

                // Serializers bind auto-properties reflectively with no name
                // reference anywhere (DTOs). Only --aggressive flags them.
                if !aggressive
                    && symbol.is_auto_property
                    && matches!(
                        symbol.visibility(),
                        Visibility::Public | Visibility::Protected | Visibility::ProtectedInternal
                    )
                {
                    flags |= SymbolFlags::LIVE_WITH_TYPE;
                }

                // Enum members travel through serialization, Enum.Parse, and
                // databases without ever being named.
                if !aggressive && kind == MemberKind::EnumMember {
                    flags |= SymbolFlags::LIVE_WITH_TYPE;
                }

                // Partial method with no implementing half in sight: the
                // implementation comes from a source generator.
                if kind == MemberKind::Method
                    && symbol.modifiers.contains(Modifiers::PARTIAL)
                    && !symbol.has_body
                {
                    flags |= SymbolFlags::ROOT;
                }

                // Interface satisfaction and external-interface heuristic.
                if let Some(parent) = symbol.parent
                    && let Some(set) = interface_member_names.get(&parent)
                {
                    if set.all_members {
                        // Type implements an unresolved I-prefixed interface;
                        // we can't know which members it demands.
                        let instance = !symbol.modifiers.contains(Modifiers::STATIC);
                        let visible = matches!(
                            symbol.visibility(),
                            Visibility::Public | Visibility::Internal
                        );
                        if instance && visible {
                            flags |= SymbolFlags::LIVE_WITH_TYPE;
                        }
                    }
                    if set.names.contains(&symbol.name) {
                        flags |= SymbolFlags::LIVE_WITH_TYPE;
                    }
                }

                // Without build output (obj/), the generated half of partial
                // types is invisible — exempt their members entirely.
                if workspace.missing_obj
                    && let Some(parent) = symbol.parent
                    && resolution.symbols[parent.index()]
                        .modifiers
                        .contains(Modifiers::PARTIAL)
                {
                    flags |= SymbolFlags::LIVE_WITH_TYPE;
                }
            }
            SymbolKind::Type(_) | SymbolKind::FileRoot => {}
        }

        // Declarations in generated files are reference-only: rooted so their
        // references count, never flagged.
        if flags.contains(SymbolFlags::GENERATED) {
            flags |= SymbolFlags::ROOT;
        }

        resolution.symbols[index].flags = flags;
    }
}

struct SatisfactionSet {
    /// Member names declared by in-source interfaces in the base closure.
    names: FxHashSet<lasso::Spur>,
    /// Base closure contains an unresolved `I`-prefixed name — every visible
    /// instance member might satisfy it.
    all_members: bool,
}

/// For every type, compute the member names of all in-source interfaces in
/// its transitive base closure, plus the external-interface flag.
///
/// Base names resolve by simple name against every project type — deliberate
/// over-resolution: conflating same-named types only marks more members as
/// interface-satisfying, which is the safe direction.
fn interface_satisfaction_sets(
    resolution: &Resolution,
    rodeo: &ThreadedRodeo,
) -> FxHashMap<SymbolId, SatisfactionSet> {
    // Member names per type (for interfaces).
    let mut members_of_type: FxHashMap<SymbolId, Vec<lasso::Spur>> = FxHashMap::default();
    for symbol in &resolution.symbols {
        if symbol.kind.is_member()
            && let Some(parent) = symbol.parent
        {
            members_of_type.entry(parent).or_default().push(symbol.name);
        }
    }

    let mut result: FxHashMap<SymbolId, SatisfactionSet> = FxHashMap::default();

    for symbol in &resolution.symbols {
        if !symbol.kind.is_type() || symbol.base_names.is_empty() {
            continue;
        }

        let mut names: FxHashSet<lasso::Spur> = FxHashSet::default();
        let mut all_members = false;
        let mut visited: FxHashSet<SymbolId> = FxHashSet::default();
        let mut stack: Vec<SymbolId> = Vec::new();

        let expand = |base_names: &[crate::extract::NamePath],
                      stack: &mut Vec<SymbolId>,
                      all_members: &mut bool| {
            for base in base_names {
                let Some(&last) = base.last() else { continue };
                match resolution.types_by_simple.get(&last) {
                    Some(ids) => stack.extend(ids.iter().copied()),
                    None => {
                        let text = rodeo.resolve(&last);
                        let mut chars = text.chars();
                        if chars.next() == Some('I') && chars.next().is_some_and(char::is_uppercase)
                        {
                            *all_members = true;
                        }
                    }
                }
            }
        };

        expand(&symbol.base_names, &mut stack, &mut all_members);

        while let Some(base_id) = stack.pop() {
            if !visited.insert(base_id) {
                continue;
            }
            let base = &resolution.symbols[base_id.index()];
            if matches!(
                base.kind,
                SymbolKind::Type(crate::model::TypeKind::Interface)
            ) && let Some(member_names) = members_of_type.get(&base_id)
            {
                names.extend(member_names.iter().copied());
            }
            expand(&base.base_names, &mut stack, &mut all_members);
        }

        if !names.is_empty() || all_members {
            result.insert(symbol.id, SatisfactionSet { names, all_members });
        }
    }

    result
}
