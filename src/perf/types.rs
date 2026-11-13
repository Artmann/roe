//! What a name was *declared* as, read verbatim from the source.
//!
//! roe never compiles the code it reads, so it has no type system to ask. But
//! `private readonly List<Foo> _items;` is right there in the text, and that
//! is enough to tell a `_items.Contains(x)` that scans the whole list from a
//! `_lookup.Contains(x)` that hashes.
//!
//! There is no inference here and no flow between methods. A name this module
//! has not seen declared resolves to nothing, and **a name that resolves to
//! nothing must never produce a finding**. That silence is what keeps the
//! checks built on this map usable rather than a false-positive generator.

use std::collections::HashMap;

use tree_sitter::Node;

/// The only distinctions the catalog needs to draw.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum DeclaredType {
    /// Membership tests walk every element: `List<T>`, `T[]`, and friends.
    LinearCollection,
    /// Membership tests hash or binary-search: `HashSet<T>`, `Dictionary<K,V>`.
    ///
    /// Findings never fire on these, so behaviourally it matches "unknown" —
    /// but the two are kept apart deliberately. `Unknown` means roe could not
    /// tell; this means it could, and the answer was "already fine". Tests
    /// assert against the distinction so nobody can later make unknown names
    /// default to linear without a failure.
    HashedCollection,
    /// `string`. Concatenation on one of these allocates a fresh copy.
    Text,
}

fn classify_named_type(name: &str) -> Option<DeclaredType> {
    match name {
        "ArrayList"
        | "Collection"
        | "IList"
        | "IReadOnlyList"
        | "ImmutableArray"
        | "ImmutableList"
        | "List"
        | "ObservableCollection" => Some(DeclaredType::LinearCollection),

        "ConcurrentDictionary"
        | "Dictionary"
        | "FrozenDictionary"
        | "FrozenSet"
        | "HashSet"
        | "IDictionary"
        | "IReadOnlyDictionary"
        | "ISet"
        | "ImmutableDictionary"
        | "ImmutableHashSet"
        | "SortedDictionary"
        | "SortedList"
        | "SortedSet" => Some(DeclaredType::HashedCollection),

        "String" | "string" => Some(DeclaredType::Text),

        // `IEnumerable<T>` is deliberately absent. LINQ's `Contains` forwards
        // to `ICollection<T>.Contains` when the runtime value implements it,
        // so an `IEnumerable<T>` holding a `HashSet<T>` is not a linear scan
        // and roe cannot tell which it is holding.
        _ => None,
    }
}

fn text<'a>(node: Node, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap_or("")
}

/// Classify a type annotation as written. `None` means "roe does not
/// recognise this", which is the overwhelmingly common answer.
pub fn classify(node: Node, source: &[u8]) -> Option<DeclaredType> {
    match node.kind() {
        // `Foo[]`, `string[,]` — a membership test walks the whole thing.
        "array_type" => Some(DeclaredType::LinearCollection),

        // `List<Foo>?` — the annotation that matters is inside.
        "nullable_type" => {
            let inner = node.named_child(0)?;

            classify(inner, source)
        }

        // `List<Foo>` — the classification hangs off `List`, not `Foo`.
        "generic_name" => {
            let base = node.child_by_field_name("name").or_else(|| node.child(0))?;

            classify_named_type(text(base, source))
        }

        // `System.Collections.Generic.List<Foo>` — only the last segment says
        // what the type is.
        "qualified_name" | "alias_qualified_name" => {
            let last = node.child_by_field_name("name")?;

            classify(last, source)
        }

        "identifier" | "predefined_type" => classify_named_type(text(node, source)),

        _ => None,
    }
}

/// Classify what an initializer constructs, for `var` bound directly to a
/// construction. Anything less direct than this is inference, which this
/// module does not do.
fn classify_initializer(node: Node, source: &[u8]) -> Option<DeclaredType> {
    match node.kind() {
        // `new int[10]`, `new[] { 1, 2 }`.
        "array_creation_expression" | "implicit_array_creation_expression" => {
            Some(DeclaredType::LinearCollection)
        }

        // `new List<int>()`.
        "object_creation_expression" => {
            let constructed = node.child_by_field_name("type")?;

            classify(constructed, source)
        }

        _ => None,
    }
}

/// Names in scope, in two layers.
///
/// The file layer is built in one pass before any check runs, so a field
/// declared *below* the method that uses it still resolves — C# has no
/// declaration order and neither can this.
#[derive(Debug, Default)]
pub struct DeclaredTypes {
    /// Fields and properties of every type in the file. `None` records a name
    /// that was declared as something unrecognised, or one that two
    /// declarations disagreed about.
    file: HashMap<String, Option<DeclaredType>>,
    /// Parameters and locals of the member currently being walked.
    member: HashMap<String, Option<DeclaredType>>,
}

impl DeclaredTypes {
    /// Phase one: every field and property in the file.
    pub fn from_file(root: Node, source: &[u8]) -> Self {
        let mut types = DeclaredTypes::default();
        let mut cursor = root.walk();
        let mut pending = vec![root];

        while let Some(node) = pending.pop() {
            match node.kind() {
                "field_declaration" | "event_field_declaration" => {
                    types.collect_variable_declaration(node, source, Scope::File);
                }

                "property_declaration" | "indexer_declaration" => {
                    let name = node.child_by_field_name("name");
                    let declared = node.child_by_field_name("type");

                    if let (Some(name), Some(declared)) = (name, declared) {
                        types.insert(Scope::File, text(name, source), classify(declared, source));
                    }
                }

                _ => {}
            }

            pending.extend(node.children(&mut cursor));
        }

        types
    }

    /// Phase two: layer one member's parameters and locals on top.
    ///
    /// Every local in the member is collected at once rather than tracked by
    /// block scope. Shadowing a name inside a nested block and then using the
    /// outer one differently in the same method is rare enough, and the cost
    /// of getting it wrong is one missed or one extra finding in a report the
    /// reader is reviewing by hand anyway.
    pub fn enter_member(&mut self, member: Node, source: &[u8]) {
        self.member.clear();

        let mut cursor = member.walk();
        let mut pending = vec![member];

        while let Some(node) = pending.pop() {
            match node.kind() {
                "parameter" => {
                    let name = node.child_by_field_name("name");
                    let declared = node.child_by_field_name("type");

                    if let (Some(name), Some(declared)) = (name, declared) {
                        self.insert(
                            Scope::Member,
                            text(name, source),
                            classify(declared, source),
                        );
                    }
                }

                "variable_declaration" => {
                    self.collect_variable_declaration(node, source, Scope::Member);
                }

                // `foreach (string name in names)` — an explicit annotation on
                // the loop variable is as good as any other declaration. `var`
                // parses as `implicit_type` and classifies as nothing.
                "foreach_statement" => {
                    let name = node.child_by_field_name("left");
                    let declared = node.child_by_field_name("type");

                    if let (Some(name), Some(declared)) = (name, declared) {
                        self.insert(
                            Scope::Member,
                            text(name, source),
                            classify(declared, source),
                        );
                    }
                }

                _ => {}
            }

            pending.extend(node.children(&mut cursor));
        }
    }

    /// What `name` was declared as, or `None` when roe cannot tell.
    ///
    /// A member-scoped name wins even when it resolved to nothing: a local
    /// shadows a field of the same name, and inheriting the field's type
    /// would be worse than admitting ignorance.
    pub fn lookup(&self, name: &str) -> Option<DeclaredType> {
        if let Some(found) = self.member.get(name) {
            return *found;
        }

        self.file.get(name).copied().flatten()
    }

    /// `_items`, `this._items`, and `Items` all name the same thing to a
    /// reader, so strip the receiver prefix C# lets you leave off.
    pub fn lookup_receiver(&self, receiver: &str) -> Option<DeclaredType> {
        let name = receiver
            .strip_prefix("this.")
            .or_else(|| receiver.strip_prefix("base."))
            .unwrap_or(receiver);

        if name.contains('.') {
            // `_context.Items` — a member of something else, which would need
            // the type of the something else to resolve.
            return None;
        }

        self.lookup(name)
    }

    /// A `variable_declaration` (`List<int> a = …, b = …;`) or the
    /// `field_declaration` wrapping one. The type annotation is shared by
    /// every declarator under it.
    fn collect_variable_declaration(&mut self, node: Node, source: &[u8], scope: Scope) {
        let mut cursor = node.walk();

        let declaration = if node.kind() == "variable_declaration" {
            node
        } else {
            let Some(inner) = node
                .children(&mut cursor)
                .find(|child| child.kind() == "variable_declaration")
            else {
                return;
            };

            inner
        };

        let declared = declaration.child_by_field_name("type");
        let is_inferred = declared.is_some_and(|node| node.kind() == "implicit_type");

        let mut cursor = declaration.walk();
        for declarator in declaration
            .children(&mut cursor)
            .filter(|child| child.kind() == "variable_declarator")
        {
            let Some(name) = declarator.child_by_field_name("name") else {
                continue;
            };

            // An annotation is authoritative. `var` falls back to what the
            // initializer constructs, and only when it constructs something
            // directly — `var x = Build();` stays unknown.
            let classified = if is_inferred {
                declarator
                    .named_child(1)
                    .and_then(|initializer| classify_initializer(initializer, source))
            } else {
                declared.and_then(|annotation| classify(annotation, source))
            };

            self.insert(scope, text(name, source), classified);
        }
    }

    /// Two declarations of one name that disagree resolve to nothing.
    ///
    /// A file holding two classes that each declare `_items`, one a `List` and
    /// one a `HashSet`, must not have the second silently answer for the
    /// first — the file layer has no idea which class a given method is in.
    fn insert(&mut self, scope: Scope, name: &str, classified: Option<DeclaredType>) {
        let names = match scope {
            Scope::File => &mut self.file,
            Scope::Member => &mut self.member,
        };

        match names.get_mut(name) {
            Some(existing) if *existing != classified => {
                *existing = None;
            }

            Some(_) => {}

            None => {
                names.insert(name.to_string(), classified);
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Scope {
    File,
    Member,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build the map for `source`, entering the first method declaration in
    /// it — the shape every check sees.
    fn map(source: &str) -> DeclaredTypes {
        let mut parser = crate::extract::make_parser();
        let tree = parser.parse(source.as_bytes(), None).expect("parses");
        let root = tree.root_node();

        let mut types = DeclaredTypes::from_file(root, source.as_bytes());

        if let Some(method) = find(root, "method_declaration") {
            types.enter_member(method, source.as_bytes());
        }

        types
    }

    fn find<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
        if node.kind() == kind {
            return Some(node);
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if let Some(found) = find(child, kind) {
                return Some(found);
            }
        }

        None
    }

    /// Wrap member declarations in a class so the source parses cleanly.
    fn class(members: &str) -> String {
        format!("class Sample\n{{\n{members}\n}}\n")
    }

    #[test]
    fn an_unmapped_name_resolves_to_nothing() {
        // The invariant the whole module exists to uphold.
        let types = map(&class("void M() { mystery.Contains(1); }"));

        assert_eq!(types.lookup("mystery"), None);
    }

    #[test]
    fn a_name_declared_as_something_unrecognised_resolves_to_nothing() {
        let types = map(&class("private CustomBag<int> _bag;\nvoid M() { }"));

        assert_eq!(types.lookup("_bag"), None);
    }

    #[test]
    fn a_field_declared_below_the_method_still_resolves() {
        // C# has no declaration order, so neither can the file pass.
        let types = map(&class(
            "void M() { _items.Contains(1); }\nprivate List<int> _items;",
        ));

        assert_eq!(types.lookup("_items"), Some(DeclaredType::LinearCollection));
    }

    #[test]
    fn a_hashed_collection_is_recognised_rather_than_unknown() {
        // Distinct from `None` on purpose: this is "roe checked, it's fine",
        // not "roe could not tell".
        let types = map(&class("private HashSet<int> _seen;\nvoid M() { }"));

        assert_eq!(types.lookup("_seen"), Some(DeclaredType::HashedCollection));
    }

    #[test]
    fn an_enumerable_stays_unknown() {
        // `Enumerable.Contains` forwards to `ICollection<T>.Contains`, so an
        // `IEnumerable<T>` holding a `HashSet<T>` is not a linear scan.
        let types = map(&class("private IEnumerable<int> _values;\nvoid M() { }"));

        assert_eq!(types.lookup("_values"), None);
    }

    #[test]
    fn arrays_are_linear() {
        let types = map(&class("private string[] _names;\nvoid M() { }"));

        assert_eq!(types.lookup("_names"), Some(DeclaredType::LinearCollection));
    }

    #[test]
    fn strings_are_text() {
        let types = map(&class("void M(string prefix) { }"));

        assert_eq!(types.lookup("prefix"), Some(DeclaredType::Text));
    }

    #[test]
    fn a_nullable_annotation_reads_through_to_the_inner_type() {
        let types = map(&class("private List<int>? _items;\nvoid M() { }"));

        assert_eq!(types.lookup("_items"), Some(DeclaredType::LinearCollection));
    }

    #[test]
    fn a_fully_qualified_name_classifies_on_its_last_segment() {
        let types = map(&class(
            "private System.Collections.Generic.List<int> _items;\nvoid M() { }",
        ));

        assert_eq!(types.lookup("_items"), Some(DeclaredType::LinearCollection));
    }

    #[test]
    fn properties_are_collected_too() {
        let types = map(&class("public List<int> Items { get; set; }\nvoid M() { }"));

        assert_eq!(types.lookup("Items"), Some(DeclaredType::LinearCollection));
    }

    #[test]
    fn parameters_are_collected() {
        let types = map(&class("void M(List<int> items) { }"));

        assert_eq!(types.lookup("items"), Some(DeclaredType::LinearCollection));
    }

    #[test]
    fn explicitly_typed_locals_are_collected() {
        let types = map(&class("void M() { List<int> local = Build(); }"));

        assert_eq!(types.lookup("local"), Some(DeclaredType::LinearCollection));
    }

    #[test]
    fn every_declarator_in_a_group_shares_the_annotation() {
        let types = map(&class(
            "void M() { List<int> first = null, second = null; }",
        ));

        assert_eq!(types.lookup("first"), Some(DeclaredType::LinearCollection));
        assert_eq!(types.lookup("second"), Some(DeclaredType::LinearCollection));
    }

    #[test]
    fn var_bound_directly_to_a_construction_is_collected() {
        let types = map(&class("void M() { var seen = new List<int>(); }"));

        assert_eq!(types.lookup("seen"), Some(DeclaredType::LinearCollection));
    }

    #[test]
    fn var_bound_to_an_array_construction_is_linear() {
        let types = map(&class(
            "void M() { var buffer = new int[10]; var pair = new[] { 1, 2 }; }",
        ));

        assert_eq!(types.lookup("buffer"), Some(DeclaredType::LinearCollection));
        assert_eq!(types.lookup("pair"), Some(DeclaredType::LinearCollection));
    }

    #[test]
    fn var_bound_to_a_call_stays_unknown() {
        // This is where inference would start, and this module does not infer.
        let types = map(&class("void M() { var items = BuildList(); }"));

        assert_eq!(types.lookup("items"), None);
    }

    #[test]
    fn an_explicitly_typed_loop_variable_is_collected() {
        let types = map(&class("void M() { foreach (string name in names) { } }"));

        assert_eq!(types.lookup("name"), Some(DeclaredType::Text));
    }

    #[test]
    fn an_inferred_loop_variable_stays_unknown() {
        let types = map(&class("void M() { foreach (var name in names) { } }"));

        assert_eq!(types.lookup("name"), None);
    }

    #[test]
    fn a_local_shadows_a_field_of_the_same_name() {
        let types = map(&class(
            "private HashSet<int> items;\nvoid M() { List<int> items = Build(); }",
        ));

        assert_eq!(types.lookup("items"), Some(DeclaredType::LinearCollection));
    }

    #[test]
    fn an_unknown_local_shadows_a_known_field() {
        // Falling through to the field would answer confidently about a name
        // that is not the field.
        let types = map(&class(
            "private List<int> items;\nvoid M() { var items = Build(); }",
        ));

        assert_eq!(types.lookup("items"), None);
    }

    #[test]
    fn two_types_in_one_file_disagreeing_resolve_to_nothing() {
        // The file layer cannot tell which class a method belongs to, so a
        // disagreement has to mean "unknown" rather than "whichever came
        // last".
        let source = "class A { private List<int> _items; }\nclass B { private HashSet<int> _items; void M() { } }";
        let types = map(source);

        assert_eq!(types.lookup("_items"), None);
    }

    #[test]
    fn two_types_agreeing_still_resolve() {
        let source = "class A { private List<int> _items; }\nclass B { private List<int> _items; void M() { } }";
        let types = map(source);

        assert_eq!(types.lookup("_items"), Some(DeclaredType::LinearCollection));
    }

    #[test]
    fn entering_a_member_drops_the_previous_members_locals() {
        let source = class("void First() { List<int> scoped = null; }\nvoid Second() { }");
        let mut parser = crate::extract::make_parser();
        let tree = parser.parse(source.as_bytes(), None).expect("parses");
        let root = tree.root_node();

        let mut types = DeclaredTypes::from_file(root, source.as_bytes());
        let first = find(root, "method_declaration").expect("first method");

        types.enter_member(first, source.as_bytes());
        assert_eq!(types.lookup("scoped"), Some(DeclaredType::LinearCollection));

        let second = first.next_named_sibling().expect("second method");

        types.enter_member(second, source.as_bytes());
        assert_eq!(types.lookup("scoped"), None);
    }

    #[test]
    fn a_this_qualified_receiver_resolves_to_the_field() {
        let types = map(&class("private List<int> _items;\nvoid M() { }"));

        assert_eq!(
            types.lookup_receiver("this._items"),
            Some(DeclaredType::LinearCollection)
        );
    }

    #[test]
    fn a_receiver_reached_through_another_object_stays_unknown() {
        // Resolving `_context.Items` would need the type of `_context`, which
        // is exactly the inference this module refuses to do.
        let types = map(&class("private List<int> Items;\nvoid M() { }"));

        assert_eq!(types.lookup_receiver("_context.Items"), None);
    }
}
