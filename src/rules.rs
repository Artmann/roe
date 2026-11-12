use crate::extract::{FileFacts, Interner};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::model::{MemberKind, Modifiers, SymbolId, SymbolKind, TypeKind, Visibility, Workspace};
use crate::resolve::{Resolution, SymbolFlags};

/// Apply the false-positive kill list: mark members that must never be
/// flagged individually as LIVE_WITH_TYPE (their containing type keeps them
/// alive), and root declarations that are consumed invisibly (generated
/// files, source-generator partial methods).
pub fn apply_kill_list(
    resolution: &mut Resolution,
    workspace: &Workspace,
    facts: &[FileFacts],
    rodeo: &Interner,
    aggressive: bool,
) {
    let interface_member_names = interface_satisfaction_sets(resolution, rodeo);
    let layout_types = layout_sensitive_types(resolution, facts, rodeo);

    for index in 0..resolution.symbols.len() {
        let symbol = &resolution.symbols[index];
        let mut flags = symbol.flags;

        match symbol.kind {
            SymbolKind::Member(kind) => {
                // Fields of a struct whose memory layout matters (GPU
                // buffers, native interop) hold their place even when C#
                // never reads them: removing one shifts everything after it.
                if kind == MemberKind::Field
                    && !symbol
                        .modifiers
                        .intersects(Modifiers::CONST | Modifiers::STATIC)
                    && let Some(parent) = symbol.parent
                {
                    let is_struct = matches!(
                        resolution.symbols[parent.index()].kind,
                        SymbolKind::Type(TypeKind::Struct)
                    );

                    if layout_types.contains(&parent)
                        || (is_struct && is_padding_name(rodeo.resolve(&symbol.name)))
                    {
                        flags |= SymbolFlags::LIVE_WITH_TYPE;
                    }
                }

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

                // The single instance field of an `[InlineArray(n)]` struct is
                // the element storage behind every indexer and span access,
                // which never name it.
                if kind == MemberKind::Field
                    && !symbol.modifiers.contains(Modifiers::CONST)
                    && !symbol.modifiers.contains(Modifiers::STATIC)
                    && let Some(parent) = symbol.parent
                    && is_inline_array(&resolution.symbols[parent.index()].attributes, rodeo)
                {
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

/// Whether a type carries `[InlineArray(n)]`, written with or without the
/// `Attribute` suffix. Attribute names are stored as their last segment, so a
/// qualified `System.Runtime.CompilerServices.InlineArray` matches too.
fn is_inline_array(attributes: &[lasso::Spur], rodeo: &Interner) -> bool {
    attributes.iter().any(|attribute| {
        matches!(
            rodeo.resolve(attribute),
            "InlineArray" | "InlineArrayAttribute"
        )
    })
}

/// Types whose memory layout is observed somewhere, so none of their instance
/// fields can be removed:
///
/// - `[StructLayout(...)]` on the type, or `[FieldOffset]` on any field.
/// - A struct named in `sizeof(T)`, `Unsafe.SizeOf<T>()`,
///   `Marshal.SizeOf<T>()`, `Marshal.OffsetOf<T>()`, or as a `T*` pointer.
///
/// Measured types match by simple name. A same-named struct elsewhere only
/// keeps a few extra fields alive, which is the safe direction.
fn layout_sensitive_types(
    resolution: &Resolution,
    facts: &[FileFacts],
    rodeo: &Interner,
) -> FxHashSet<SymbolId> {
    let measured: FxHashSet<lasso::Spur> = facts
        .iter()
        .flat_map(|file_facts| file_facts.layout_types.iter().copied())
        .collect();

    let has_attribute = |attributes: &[lasso::Spur], name: &str| {
        attributes.iter().any(|attribute| {
            let text = rodeo.resolve(attribute);

            text == name || text.strip_suffix("Attribute") == Some(name)
        })
    };

    let mut result: FxHashSet<SymbolId> = FxHashSet::default();

    for symbol in &resolution.symbols {
        match symbol.kind {
            SymbolKind::Type(kind) => {
                let is_measured_struct = kind == TypeKind::Struct && measured.contains(&symbol.name);

                if is_measured_struct || has_attribute(&symbol.attributes, "StructLayout") {
                    result.insert(symbol.id);
                }
            }
            SymbolKind::Member(MemberKind::Field) => {
                if has_attribute(&symbol.attributes, "FieldOffset")
                    && let Some(parent) = symbol.parent
                {
                    result.insert(parent);
                }
            }
            SymbolKind::Member(_) | SymbolKind::FileRoot => {}
        }
    }

    result
}

/// Whether a struct field is named as padding: `_pad0`, `padding`,
/// `Reserved1`, `_pad_2`. Leading and trailing underscores and trailing digits
/// are ignored, and case doesn't matter.
fn is_padding_name(name: &str) -> bool {
    let stem = name
        .trim_start_matches('_')
        .trim_end_matches(|character: char| character.is_ascii_digit() || character == '_')
        .to_ascii_lowercase();

    matches!(stem.as_str(), "pad" | "padding" | "reserved")
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
    rodeo: &Interner,
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

#[cfg(test)]
mod tests {
    use super::is_padding_name;

    #[test]
    fn padding_names_are_recognized_in_any_common_spelling() {
        for name in ["_pad0", "_pad_1", "pad", "Padding", "_padding2", "Reserved1", "_reserved"] {
            assert!(is_padding_name(name), "{name} reads as padding");
        }
    }

    #[test]
    fn names_that_only_start_like_padding_are_not_padding() {
        for name in ["Paddle", "Padded", "_reserve", "ReservedSeats", "_unused", "Spread"] {
            assert!(!is_padding_name(name), "{name} is not padding");
        }
    }
}
