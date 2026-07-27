use lasso::Spur;
use crate::extract::Interner;
use smallvec::{SmallVec, smallvec};
use tree_sitter::Node;

use crate::extract::{FILE_ROOT, FileFacts, NamePath, RawDecl, RawRef, RawRefKind, UsingEntry};
use crate::model::{FileId, MemberKind, Modifiers, ParameterBreakdown, SymbolKind, TypeKind};

/// Extract declarations and references from a parsed compilation unit.
///
/// Reference collection is deliberately over-inclusive: an identifier we
/// cannot classify still becomes an Ambient reference, because marking too
/// much code as used only costs false negatives, never false positives.
pub fn extract(file: FileId, source: &[u8], root: Node, rodeo: &Interner) -> FileFacts {
    let mut walker = Walker {
        source,
        rodeo,
        facts: FileFacts {
            file,
            decls: Vec::new(),
            refs: Vec::new(),
            usings: Vec::new(),
            has_top_level_statements: false,
            has_errors: root.has_error(),
            is_generated: false,
            line_count: root.end_position().row as u32 + 1,
        },
        namespace_stack: Vec::new(),
        decl_stack: Vec::new(),
    };
    walker.walk_children(root, false);
    walker.facts
}

struct Walker<'a> {
    source: &'a [u8],
    rodeo: &'a Interner,
    facts: FileFacts,
    namespace_stack: Vec<Spur>,
    /// Indices into facts.decls for the enclosing declarations, innermost
    /// last. References attach to the innermost one.
    decl_stack: Vec<u32>,
}

impl<'a> Walker<'a> {
    fn text(&self, node: Node) -> &'a str {
        node.utf8_text(self.source).unwrap_or("")
    }

    fn intern(&self, text: &str) -> Spur {
        self.rodeo.get_or_intern(text)
    }

    fn origin(&self) -> u32 {
        self.decl_stack.last().copied().unwrap_or(FILE_ROOT)
    }

    fn enclosing_type(&self) -> Option<u32> {
        self.decl_stack
            .iter()
            .rev()
            .copied()
            .find(|&index| self.facts.decls[index as usize].kind.is_type())
    }

    fn emit(&mut self, kind: RawRefKind, path: NamePath) {
        if path.is_empty() {
            return;
        }
        self.facts.refs.push(RawRef {
            kind,
            path,
            origin: self.origin(),
        });
    }

    // ------------------------------------------------------------------
    // Dispatch
    // ------------------------------------------------------------------

    fn walk(&mut self, node: Node, type_ctx: bool) {
        match node.kind() {
            // ---- type declarations
            "class_declaration" => self.type_decl(node, TypeKind::Class),
            "interface_declaration" => self.type_decl(node, TypeKind::Interface),
            "struct_declaration" => self.type_decl(node, TypeKind::Struct),
            "enum_declaration" => self.type_decl(node, TypeKind::Enum),
            "record_declaration" => self.type_decl(node, TypeKind::Record),
            "delegate_declaration" => self.type_decl(node, TypeKind::Delegate),

            // ---- namespaces
            "namespace_declaration" => self.namespace_decl(node),
            "file_scoped_namespace_declaration" => self.file_scoped_namespace(node),

            // ---- member declarations
            "method_declaration" => self.member_decl(node, MemberKind::Method),
            "constructor_declaration" => {
                let kind = if has_modifier(node, self.source, "static") {
                    MemberKind::StaticConstructor
                } else {
                    MemberKind::Constructor
                };
                self.member_decl(node, kind);
            }
            "destructor_declaration" => self.member_decl(node, MemberKind::Destructor),
            "property_declaration" => self.member_decl(node, MemberKind::Property),
            "indexer_declaration" => self.member_decl(node, MemberKind::Indexer),
            "field_declaration" => self.field_decl(node, MemberKind::Field),
            "event_field_declaration" => self.field_decl(node, MemberKind::Event),
            "event_declaration" => self.member_decl(node, MemberKind::Event),
            "enum_member_declaration" => self.member_decl(node, MemberKind::EnumMember),
            "operator_declaration" => self.member_decl(node, MemberKind::Operator),
            "conversion_operator_declaration" => {
                self.member_decl(node, MemberKind::ConversionOperator);
            }

            "invocation_expression" => {
                self.mark_scan_target(node);
                self.walk_children(node, type_ctx);
            }

            // ---- directives
            "using_directive" => self.using_directive(node),
            "extern_alias_directive" => {}
            "global_statement" => {
                self.facts.has_top_level_statements = true;
                self.walk_children(node, false);
            }

            // ---- names & references
            "identifier" => {
                let text = self.text(node);
                if !matches!(text, "var" | "nameof" | "global" | "dynamic" | "_") {
                    let kind = if type_ctx {
                        RawRefKind::Type
                    } else {
                        RawRefKind::Ambient
                    };
                    let path: NamePath = smallvec![self.intern(text)];
                    self.emit(kind, path);
                }
            }
            "qualified_name" | "alias_qualified_name" => {
                let path = self.name_path(node);
                let kind = if type_ctx {
                    RawRefKind::Type
                } else {
                    RawRefKind::Ambient
                };
                self.emit(kind, path);
            }
            "generic_name" => {
                let path = self.name_path(node);
                let kind = if type_ctx {
                    RawRefKind::Type
                } else {
                    RawRefKind::Ambient
                };
                self.emit(kind, path);
                if let Some(args) = node
                    .children(&mut node.walk())
                    .find(|c| c.kind() == "type_argument_list")
                {
                    self.walk_children(args, true);
                }
            }
            "member_access_expression" | "member_binding_expression" => {
                if node.kind() == "member_access_expression"
                    && let Some(path) = self.try_flatten_dotted_path(node)
                {
                    // A.B.C — try the whole chain as a type/namespace path so
                    // intermediate containers (nested types) resolve like a
                    // qualified name, not just the leaf member.
                    self.emit(RawRefKind::Ambient, path);
                } else {
                    if let Some(name) = node.child_by_field_name("name") {
                        self.emit_member_name(name);
                    }
                    if let Some(expression) = node.child_by_field_name("expression") {
                        self.walk(expression, false);
                    }
                }
            }
            "attribute" => self.attribute_ref(node),

            // ---- leaves that never contain references
            "comment"
            | "predefined_type"
            | "implicit_type"
            | "discard"
            | "modifier"
            | "string_content"
            | "raw_string_content"
            | "escape_sequence"
            | "character_literal"
            | "integer_literal"
            | "real_literal"
            | "boolean_literal"
            | "null_literal"
            | "verbatim_string_literal"
            | "preproc_arg" => {}

            _ => self.walk_children(node, type_ctx),
        }
    }

    /// Iterate a node's children, computing each child's type-context and
    /// skipping identifiers that are declaration names rather than uses.
    fn walk_children(&mut self, node: Node, type_ctx: bool) {
        let parent_kind = node.kind();
        let mut cursor = node.walk();
        if !cursor.goto_first_child() {
            return;
        }
        loop {
            let child = cursor.node();
            let field = cursor.field_name();

            if !skip_child(parent_kind, field, child.kind()) {
                let child_ctx = child_type_ctx(parent_kind, field, child.kind(), type_ctx);
                self.walk(child, child_ctx);
            }

            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }

    // ------------------------------------------------------------------
    // Names
    // ------------------------------------------------------------------

    /// Flatten identifier / qualified_name / alias_qualified_name /
    /// generic_name into dotted segments (generic args are NOT part of the
    /// path; alias qualifiers like `global::` are dropped).
    fn name_path(&self, node: Node) -> NamePath {
        match node.kind() {
            "identifier" => smallvec![self.intern(self.text(node))],
            "generic_name" => {
                let mut cursor = node.walk();
                node.children(&mut cursor)
                    .find(|c| c.kind() == "identifier")
                    .map(|id| smallvec![self.intern(self.text(id))])
                    .unwrap_or_default()
            }
            "qualified_name" => {
                let mut path = node
                    .child_by_field_name("qualifier")
                    .map(|q| self.name_path(q))
                    .unwrap_or_default();
                if let Some(name) = node.child_by_field_name("name") {
                    path.extend(self.name_path(name));
                }
                path
            }
            "alias_qualified_name" => node
                .child_by_field_name("name")
                .map(|n| self.name_path(n))
                .unwrap_or_default(),
            _ => NamePath::new(),
        }
    }

    /// Flatten a chain of `member_access_expression` nodes rooted in a plain
    /// identifier (`A.B.C`) into one dotted path, so it can be tried as a
    /// type/namespace path the same way `qualified_name` is. Returns `None`
    /// as soon as a link isn't a bare identifier (a call, indexer, `this`,
    /// generic name, ...) — those keep falling through to the ordinary
    /// per-segment member-access walk.
    fn try_flatten_dotted_path(&self, node: Node) -> Option<NamePath> {
        match node.kind() {
            "identifier" => {
                let text = self.text(node);
                if matches!(text, "var" | "nameof" | "global" | "dynamic" | "_") {
                    None
                } else {
                    Some(smallvec![self.intern(text)])
                }
            }
            "member_access_expression" => {
                let name = node.child_by_field_name("name")?;
                if name.kind() != "identifier" {
                    return None;
                }
                let expression = node.child_by_field_name("expression")?;
                let mut path = self.try_flatten_dotted_path(expression)?;
                path.push(self.intern(self.text(name)));
                Some(path)
            }
            _ => None,
        }
    }

    fn emit_member_name(&mut self, name: Node) {
        let path = self.name_path(name);
        self.emit(RawRefKind::Member, path);
        // x.Generic<T>() — the type arguments are type references.
        if name.kind() == "generic_name"
            && let Some(args) = name
                .children(&mut name.walk())
                .find(|c| c.kind() == "type_argument_list")
        {
            self.walk_children(args, true);
        }
    }

    /// Reflection-based registration (`GetExports<IImageProvider>()`,
    /// `AddHostedService<Worker>()`) names only the contract as a lone
    /// generic argument; implementations are discovered at runtime. Emit the
    /// argument as a ScanTarget so implementations can be rooted.
    fn mark_scan_target(&mut self, node: Node) {
        let Some(function) = node.child_by_field_name("function") else {
            return;
        };
        let generic = match function.kind() {
            "generic_name" => Some(function),
            "member_access_expression" | "member_binding_expression" => function
                .child_by_field_name("name")
                .filter(|n| n.kind() == "generic_name"),
            _ => None,
        };
        let Some(generic) = generic else { return };
        let Some(args) = generic
            .children(&mut generic.walk())
            .find(|c| c.kind() == "type_argument_list")
        else {
            return;
        };

        let mut cursor = args.walk();
        let type_args: Vec<Node> = args.named_children(&mut cursor).collect();
        if type_args.len() != 1 {
            return;
        }
        let path = self.name_path(type_args[0]);
        self.emit(RawRefKind::ScanTarget, path);
    }

    fn attribute_ref(&mut self, node: Node) {
        if let Some(name) = node.child_by_field_name("name") {
            let path = self.name_path(name);
            self.emit(RawRefKind::Attribute, path);
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "attribute_argument_list" {
                self.walk_children(child, false);
            }
        }
    }

    // ------------------------------------------------------------------
    // Namespaces & usings
    // ------------------------------------------------------------------

    fn namespace_decl(&mut self, node: Node) {
        let segments = node
            .child_by_field_name("name")
            .map(|n| self.name_path(n))
            .unwrap_or_default();
        let pushed = segments.len();
        self.namespace_stack.extend(segments);
        if let Some(body) = node.child_by_field_name("body") {
            self.walk_children(body, false);
        }
        self.namespace_stack
            .truncate(self.namespace_stack.len() - pushed);
    }

    fn file_scoped_namespace(&mut self, node: Node) {
        if let Some(name) = node.child_by_field_name("name") {
            let segments = self.name_path(name);
            self.namespace_stack.extend(segments);
        }
        // Applies to the rest of the compilation unit; never popped. The
        // declarations follow as siblings, so nothing to walk here.
    }

    fn using_directive(&mut self, node: Node) {
        let alias = node.child_by_field_name("name");
        let mut is_global = false;
        let mut is_static = false;
        let mut target: Option<Node> = None;

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "global" => is_global = true,
                "static" => is_static = true,
                "identifier" | "qualified_name" | "alias_qualified_name" | "generic_name"
                    if alias.is_none_or(|a| a.id() != child.id()) =>
                {
                    target = Some(child);
                }
                _ => {}
            }
        }

        let Some(target) = target else { return };
        let path = self.name_path(target);

        // `using static T;` and `using F = T;` keep T alive: the members/type
        // are consumable without further name mentions of T itself.
        if is_static || alias.is_some() {
            self.emit(RawRefKind::Type, path.clone());
        }

        self.facts.usings.push(UsingEntry {
            path: path.into_vec(),
            alias: alias.map(|a| self.intern(self.text(a))),
            is_global,
            is_static,
        });
    }

    // ------------------------------------------------------------------
    // Declarations
    // ------------------------------------------------------------------

    fn type_decl(&mut self, node: Node, kind: TypeKind) {
        let Some(name_node) = node.child_by_field_name("name") else {
            // Broken declaration (parse error) — still walk for references.
            self.walk_children(node, false);
            return;
        };

        let arity = node
            .children(&mut node.walk())
            .find(|c| c.kind() == "type_parameter_list")
            .map(|list| {
                list.children(&mut list.walk())
                    .filter(|c| c.kind() == "type_parameter")
                    .count() as u8
            })
            .unwrap_or(0);

        let mut base_names: SmallVec<[NamePath; 2]> = SmallVec::new();
        if let Some(base_list) = node
            .children(&mut node.walk())
            .find(|c| c.kind() == "base_list")
        {
            let mut cursor = base_list.walk();
            for child in base_list.children(&mut cursor) {
                match child.kind() {
                    "identifier" | "qualified_name" | "alias_qualified_name" | "generic_name" => {
                        let path = self.name_path(child);
                        if !path.is_empty() {
                            base_names.push(path);
                        }
                    }
                    "primary_constructor_base_type" => {
                        if let Some(base_type) = child.child_by_field_name("type") {
                            let path = self.name_path(base_type);
                            if !path.is_empty() {
                                base_names.push(path);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        let namespace = SmallVec::from_slice(&self.namespace_stack);
        let index = self.push_decl(node, name_node, SymbolKind::Type(kind), |decl| {
            decl.namespace = namespace;
            decl.base_names = base_names;
            decl.arity = arity;
        });

        self.decl_stack.push(index);
        self.walk_children(node, false);
        self.decl_stack.pop();
    }

    fn member_decl(&mut self, node: Node, kind: MemberKind) {
        let name_node = node.child_by_field_name("name");
        let name = match (name_node, kind) {
            (Some(n), _) => self.text(n).to_string(),
            (None, MemberKind::Indexer) => "this[]".to_string(),
            (None, MemberKind::Operator) => node
                .child_by_field_name("operator")
                .map(|op| format!("operator {}", self.text(op)))
                .unwrap_or_else(|| "operator".to_string()),
            (None, MemberKind::ConversionOperator) => "conversion operator".to_string(),
            (None, _) => {
                self.walk_children(node, false);
                return;
            }
        };

        let is_explicit_interface_impl = node
            .children(&mut node.walk())
            .any(|c| c.kind() == "explicit_interface_specifier");

        let is_extension_method = kind == MemberKind::Method
            && node
                .child_by_field_name("parameters")
                .and_then(|params| {
                    params
                        .children(&mut params.walk())
                        .find(|c| c.kind() == "parameter")
                })
                .is_some_and(|first| has_modifier(first, self.source, "this"));

        let (is_auto_property, has_accessors) = auto_property_shape(node);
        let has_body = node.child_by_field_name("body").is_some()
            || node.child_by_field_name("value").is_some()
            || has_accessors;
        let (cyclomatic, cognitive, body_lines) = compute_member_complexity(node, kind);
        let parameters = count_parameters(node, self.source);

        let position_node = name_node.unwrap_or(node);
        let interned_name = self.intern(&name);

        let index = self.push_decl_named(
            node,
            position_node,
            interned_name,
            SymbolKind::Member(kind),
            |decl| {
                decl.is_explicit_interface_impl = is_explicit_interface_impl;
                decl.is_extension_method = is_extension_method;
                decl.is_auto_property = is_auto_property;
                decl.has_body = has_body;
                decl.cyclomatic = cyclomatic;
                decl.cognitive = cognitive;
                decl.body_lines = body_lines;
                decl.parameters = parameters;
            },
        );

        self.decl_stack.push(index);
        self.walk_children(node, false);
        self.decl_stack.pop();
    }

    /// field_declaration / event_field_declaration: one declaration per
    /// variable_declarator. The shared type annotation is re-walked with each
    /// declarator as origin so that any single live field keeps the type
    /// reference alive.
    fn field_decl(&mut self, node: Node, kind: MemberKind) {
        let modifiers = collect_modifiers(node, self.source);
        let attributes = self.collect_attribute_names(node);

        let Some(variable_declaration) = node
            .children(&mut node.walk())
            .find(|c| c.kind() == "variable_declaration")
        else {
            self.walk_children(node, false);
            return;
        };
        let type_node = variable_declaration.child_by_field_name("type");

        // Attribute references attach to the field group's first declarator,
        // but the attribute names live on every declarator's RawDecl.
        let mut first_index: Option<u32> = None;

        let mut cursor = variable_declaration.walk();
        for declarator in variable_declaration
            .children(&mut cursor)
            .filter(|c| c.kind() == "variable_declarator")
        {
            let Some(name_node) = declarator.child_by_field_name("name") else {
                continue;
            };
            let interned_name = self.intern(self.text(name_node));
            let index = self.push_decl_named(
                node,
                name_node,
                interned_name,
                SymbolKind::Member(kind),
                |decl| {
                    decl.modifiers = modifiers;
                    decl.attributes = attributes.clone();
                    decl.has_body = true;
                },
            );

            self.decl_stack.push(index);
            if let Some(type_node) = type_node {
                self.walk(type_node, true);
            }
            // Initializer and array-size expressions.
            let mut inner = declarator.walk();
            for child in declarator.children(&mut inner) {
                if child.kind() != "identifier" {
                    self.walk(child, false);
                }
            }
            self.decl_stack.pop();

            if first_index.is_none() {
                first_index = Some(index);
            }
        }

        // Emit attribute references once, attached to the first declarator.
        if let Some(first) = first_index {
            self.decl_stack.push(first);
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "attribute_list" {
                    self.walk_children(child, false);
                }
            }
            self.decl_stack.pop();
        }
    }

    fn push_decl(
        &mut self,
        node: Node,
        name_node: Node,
        kind: SymbolKind,
        customize: impl FnOnce(&mut RawDecl),
    ) -> u32 {
        let interned = self.intern(self.text(name_node));
        self.push_decl_named(node, name_node, interned, kind, customize)
    }

    fn push_decl_named(
        &mut self,
        node: Node,
        position_node: Node,
        name: Spur,
        kind: SymbolKind,
        customize: impl FnOnce(&mut RawDecl),
    ) -> u32 {
        let position = position_node.start_position();
        let mut decl = RawDecl {
            name,
            kind,
            namespace: NamePath::new(),
            parent: self.enclosing_type(),
            modifiers: collect_modifiers(node, self.source),
            attributes: self.collect_attribute_names(node),
            base_names: SmallVec::new(),
            is_explicit_interface_impl: false,
            is_extension_method: false,
            is_auto_property: false,
            has_body: false,
            arity: 0,
            cyclomatic: 0,
            cognitive: 0,
            body_lines: 0,
            parameters: ParameterBreakdown::default(),
            line: position.row as u32 + 1,
            column: position.column as u32 + 1,
        };
        customize(&mut decl);
        let index = self.facts.decls.len() as u32;
        self.facts.decls.push(decl);
        index
    }

    /// Last segment of every attribute name on this declaration node.
    fn collect_attribute_names(&self, node: Node) -> SmallVec<[Spur; 2]> {
        let mut names = SmallVec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() != "attribute_list" {
                continue;
            }
            let mut inner = child.walk();
            for attribute in child.children(&mut inner) {
                if attribute.kind() == "attribute"
                    && let Some(name) = attribute.child_by_field_name("name")
                {
                    let path = self.name_path(name);
                    if let Some(&last) = path.last() {
                        names.push(last);
                    }
                }
            }
        }
        names
    }
}

// ----------------------------------------------------------------------
// Child classification
// ----------------------------------------------------------------------

/// Identifiers in these (parent kind, field) positions declare names rather
/// than use them. Everything not listed is collected — over-collection is
/// safe, under-collection is not.
fn skip_child(parent_kind: &str, field: Option<&str>, child_kind: &str) -> bool {
    match (parent_kind, field) {
        // Declaration name fields (decl helpers handle their own names, but
        // broken/nested paths can still route through walk_children).
        (
            "class_declaration"
            | "interface_declaration"
            | "struct_declaration"
            | "enum_declaration"
            | "record_declaration"
            | "delegate_declaration"
            | "method_declaration"
            | "constructor_declaration"
            | "destructor_declaration"
            | "property_declaration"
            | "event_declaration"
            | "enum_member_declaration"
            | "local_function_statement"
            | "variable_declarator"
            | "parameter"
            | "type_parameter"
            | "catch_declaration"
            | "declaration_pattern"
            | "recursive_pattern"
            | "declaration_expression"
            | "using_directive"
            | "argument"
            | "attribute_argument"
            | "accessor_declaration",
            Some("name"),
        ) => true,
        // foreach (var x in ...) — x is a declaration.
        ("foreach_statement", Some("left")) => child_kind == "identifier",
        // label: — the label identifier declares the label.
        ("labeled_statement", None) => child_kind == "identifier",
        // Deconstruction designations declare variables.
        (_, _) if child_kind == "parenthesized_variable_designation" => true,
        _ => false,
    }
}

/// Compute whether a child is in type-annotation position, where bare
/// identifiers resolve as types (with namespace scoping) instead of as
/// ambient names.
fn child_type_ctx(
    parent_kind: &str,
    field: Option<&str>,
    child_kind: &str,
    inherited: bool,
) -> bool {
    // Expressions can nest inside type annotations only via array sizes.
    if child_kind == "array_rank_specifier" || parent_kind == "array_rank_specifier" {
        return false;
    }
    if matches!(field, Some("type") | Some("returns")) {
        return true;
    }
    match parent_kind {
        "base_list" => child_kind != "argument_list",
        "type_argument_list" | "explicit_interface_specifier" | "type_parameter_constraint" => true,
        "as_expression" | "is_expression" => field == Some("right"),
        _ => inherited,
    }
}

fn collect_modifiers(node: Node, source: &[u8]) -> Modifiers {
    let mut modifiers = Modifiers::empty();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "modifier"
            && let Some(modifier) = Modifiers::from_keyword(child.utf8_text(source).unwrap_or(""))
        {
            modifiers |= modifier;
        }
    }
    modifiers
}

fn has_modifier(node: Node, source: &[u8], keyword: &str) -> bool {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .any(|c| c.kind() == "modifier" && c.utf8_text(source).unwrap_or("") == keyword)
}

/// (is_auto_property, has_accessor_list). Auto = every accessor is bodyless
/// (`{ get; set; }`, `{ get; init; }`, `{ get; }`).
fn auto_property_shape(node: Node) -> (bool, bool) {
    let Some(accessors) = node.child_by_field_name("accessors") else {
        return (false, false);
    };
    let mut cursor = accessors.walk();
    let mut saw_accessor = false;
    let mut all_bodyless = true;
    for accessor in accessors.children(&mut cursor) {
        if accessor.kind() != "accessor_declaration" {
            continue;
        }
        saw_accessor = true;
        if accessor.child_by_field_name("body").is_some() {
            all_bodyless = false;
        }
    }
    (saw_accessor && all_bodyless, saw_accessor)
}

/// The executable body node(s) of a member declaration. Methods,
/// constructors, operators, and destructors have exactly one (the `body`
/// field, block or arrow); properties/indexers/events either have one arrow
/// body (the `value` field) or one body per accessor. Auto-properties,
/// abstract/partial members, and bodyless events have none.
fn method_bodies(node: Node, kind: MemberKind) -> SmallVec<[Node; 4]> {
    let mut bodies = SmallVec::new();

    match kind {
        MemberKind::Method
        | MemberKind::Constructor
        | MemberKind::StaticConstructor
        | MemberKind::Destructor
        | MemberKind::Operator
        | MemberKind::ConversionOperator => {
            if let Some(body) = node.child_by_field_name("body") {
                bodies.push(body);
            }
        }
        MemberKind::Property | MemberKind::Indexer | MemberKind::Event => {
            if let Some(value) = node.child_by_field_name("value") {
                bodies.push(value);
            } else if let Some(accessors) = node.child_by_field_name("accessors") {
                let mut cursor = accessors.walk();
                for accessor in accessors.children(&mut cursor) {
                    if accessor.kind() != "accessor_declaration" {
                        continue;
                    }
                    if let Some(body) = accessor.child_by_field_name("body") {
                        bodies.push(body);
                    }
                }
            }
        }
        MemberKind::Field | MemberKind::EnumMember => {}
    }

    bodies
}

/// McCabe cyclomatic complexity (1 + decision points) and body line span. A
/// member with several accessor bodies (`get`/`set`) sums their decision
/// points and counts each additional accessor beyond the first as its own
/// branch, since reaching one accessor rather than another is itself a fork
/// in control flow; the line span covers from the first body's start to the
/// last body's end.
fn compute_member_complexity(node: Node, kind: MemberKind) -> (u32, u32, u32) {
    let bodies = method_bodies(node, kind);
    let Some(first) = bodies.first() else {
        return (0, 0, 0);
    };

    let mut cyclomatic = 1 + (bodies.len() as u32 - 1);
    let mut cognitive = 0;
    let mut start_row = first.start_position().row;
    let mut end_row = first.end_position().row;
    for body in &bodies {
        cyclomatic += count_decision_points(*body);
        cognitive += count_cognitive(*body, 0);
        start_row = start_row.min(body.start_position().row);
        end_row = end_row.max(body.end_position().row);
    }

    (cyclomatic, cognitive, (end_row - start_row) as u32 + 1)
}

/// Decision points within a body: branches (`if`/`for`/`foreach`/`while`/
/// `do`/`catch`/ternary/switch branches) and short-circuiting boolean
/// operators (`&&`/`||`), each adding one path through the method. The
/// grammar starts a new `switch_section` at every `case`/`default` label
/// (even label-only fallthroughs with no statements of their own), so each
/// label counts as its own branch.
///
/// `??` is deliberately absent. It is a defaulting idiom, not control flow the
/// reader traces — nobody writes a second test for the null side of
/// `name ?? "anonymous"` — and charging for it scored a hand-rolled `with`
/// method with thirteen defaulted fields at 14 while `count_cognitive`, which
/// has always ignored it, scored the same method 0. Real branching hidden
/// behind one still counts: the right-hand side is walked like any other
/// expression, so a ternary or a lambda in the fallback is charged on its own
/// merits. `??=` is an `assignment_expression` rather than a
/// `binary_expression`, so it never reached this match to begin with.
fn count_decision_points(node: Node) -> u32 {
    let mut count = match node.kind() {
        "if_statement"
        | "for_statement"
        | "foreach_statement"
        | "while_statement"
        | "do_statement"
        | "catch_clause"
        | "conditional_expression"
        | "switch_section"
        | "switch_expression_arm" => 1,
        "binary_expression" => match node.child_by_field_name("operator") {
            Some(op) if matches!(op.kind(), "&&" | "||") => 1,
            _ => 0,
        },
        _ => 0,
    };

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        count += count_decision_points(child);
    }

    count
}

/// Cognitive complexity (Campbell, SonarSource 2018) of a body. Where
/// cyclomatic complexity counts paths, this estimates how hard code is to
/// *read*, so the two disagree in useful ways: a flat ten-case `switch`
/// scores 1 here but 10 there, while four levels of nested `if` score far
/// worse here than their path count suggests.
///
/// The rules, in short: a flow-breaking structure costs `1 + nesting`, so
/// the same construct costs more the deeper it sits; `else`/`else if` cost a
/// flat 1 because a chain reads linearly; lambdas and local functions nest
/// their contents without costing anything themselves; and a run of one
/// boolean operator costs 1 however long it is (`a && b && c` is one idea,
/// `a && b || c` is two). There is no baseline of 1 — straight-line code
/// scores 0.
///
/// Two deliberate departures from the spec: recursion is not counted (it
/// would need name resolution the extractor does not have here), and `??` is
/// not treated as a boolean operator. `count_decision_points` agrees on the
/// latter, so the two metrics no longer disagree about defaulting.
fn count_cognitive(node: Node, nesting: u32) -> u32 {
    match node.kind() {
        // Handled as a chain so `else if` does not nest.
        "if_statement" => cognitive_if(node, nesting),

        // Structures that both cost and nest their contents.
        "for_statement"
        | "foreach_statement"
        | "while_statement"
        | "do_statement"
        | "catch_clause"
        | "switch_statement"
        | "switch_expression"
        | "conditional_expression" => 1 + nesting + cognitive_children(node, nesting + 1),

        // Nest their contents without costing anything themselves.
        "lambda_expression" | "anonymous_method_expression" | "local_function_statement" => {
            cognitive_children(node, nesting + 1)
        }

        // An unconditional jump breaks the reader's flow, but is not nested.
        "goto_statement" => 1 + cognitive_children(node, nesting),

        "binary_expression" => cognitive_boolean_sequence(node) + cognitive_children(node, nesting),

        _ => cognitive_children(node, nesting),
    }
}

fn cognitive_children(node: Node, nesting: u32) -> u32 {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .map(|child| count_cognitive(child, nesting))
        .sum()
}

/// `if` costs `1 + nesting` and nests its body; every `else`/`else if` after
/// it costs a flat 1 and stays at the original depth, because a chain of
/// conditions reads as one decision, not a deepening one.
fn cognitive_if(node: Node, nesting: u32) -> u32 {
    let mut count = 1 + nesting;

    if let Some(condition) = node.child_by_field_name("condition") {
        count += count_cognitive(condition, nesting);
    }
    if let Some(consequence) = node.child_by_field_name("consequence") {
        count += count_cognitive(consequence, nesting + 1);
    }
    if let Some(alternative) = node.child_by_field_name("alternative") {
        count += cognitive_else(alternative, nesting);
    }

    count
}

fn cognitive_else(alternative: Node, nesting: u32) -> u32 {
    // Some grammar versions wrap the branch in an `else_clause`; unwrap it so
    // the `else if` check below sees the real statement either way.
    let branch = if alternative.kind() == "else_clause" {
        alternative.named_child(0).unwrap_or(alternative)
    } else {
        alternative
    };

    if branch.kind() != "if_statement" {
        return 1 + count_cognitive(branch, nesting + 1);
    }

    let mut count = 1;

    if let Some(condition) = branch.child_by_field_name("condition") {
        count += count_cognitive(condition, nesting);
    }
    if let Some(consequence) = branch.child_by_field_name("consequence") {
        count += count_cognitive(consequence, nesting + 1);
    }
    if let Some(next) = branch.child_by_field_name("alternative") {
        count += cognitive_else(next, nesting);
    }

    count
}

/// One point per run of the same boolean operator. Left-associative parsing
/// makes `a && b && c` a `&&` nested inside a `&&`, so a node only starts a
/// new run when its operator differs from its parent's.
fn cognitive_boolean_sequence(node: Node) -> u32 {
    let Some(operator) = node.child_by_field_name("operator") else {
        return 0;
    };
    if !matches!(operator.kind(), "&&" | "||") {
        return 0;
    }

    let parent_operator = node
        .parent()
        .filter(|parent| parent.kind() == "binary_expression")
        .and_then(|parent| parent.child_by_field_name("operator"))
        .map(|parent_operator| parent_operator.kind());

    if parent_operator == Some(operator.kind()) {
        0
    } else {
        1
    }
}

/// Split the `parameters` field (`parameter_list` or
/// `bracketed_parameter_list` — both use `parameter` children) into what a
/// call site has to supply and what it may leave out.
///
/// `out` parameters are return values, and defaulted parameters and `params`
/// arrays are omittable, so none of the three cost the caller anything. `ref`,
/// `in`, and an extension method's `this` receiver are all written at the call
/// site, so all three are required.
fn count_parameters(node: Node, source: &[u8]) -> ParameterBreakdown {
    let mut breakdown = ParameterBreakdown::default();

    let Some(parameters) = node.child_by_field_name("parameters") else {
        return breakdown;
    };

    let mut cursor = parameters.walk();
    for child in parameters.children(&mut cursor) {
        match child.kind() {
            // The grammar hides `_parameter_array`, so it inlines into the
            // parameter list: a `params` array shows up as a bare `params`
            // token followed by loose type and name nodes, with no `parameter`
            // wrapper to match on.
            "params" => breakdown.optional += 1,
            "parameter" => {
                if has_modifier(child, source, "out") {
                    breakdown.out += 1;
                } else if has_default_value(child) {
                    breakdown.optional += 1;
                } else {
                    breakdown.required += 1;
                }
            }
            _ => {}
        }
    }

    breakdown
}

/// A default value is a bare `= expression` tail on the parameter with no
/// wrapper node of its own, so the `=` token is what there is to look for.
fn has_default_value(parameter: Node) -> bool {
    let mut cursor = parameter.walk();

    parameter.children(&mut cursor).any(|c| c.kind() == "=")
}

#[cfg(test)]
mod tests {
    use crate::extract::Interner;

    use crate::extract::{FILE_ROOT, FileFacts, RawDecl, RawRefKind, extract_source};
    use crate::model::{MemberKind, Modifiers, SymbolKind, TypeKind};

    fn extract(source: &str) -> (FileFacts, Interner) {
        let rodeo = crate::extract::new_interner();
        let facts = extract_source(source, &rodeo);
        (facts, rodeo)
    }

    fn decl_names(facts: &FileFacts, rodeo: &Interner) -> Vec<String> {
        facts
            .decls
            .iter()
            .map(|d| rodeo.resolve(&d.name).to_string())
            .collect()
    }

    fn ref_names(facts: &FileFacts, rodeo: &Interner) -> Vec<String> {
        facts
            .refs
            .iter()
            .map(|r| {
                r.path
                    .iter()
                    .map(|s| rodeo.resolve(s))
                    .collect::<Vec<_>>()
                    .join(".")
            })
            .collect()
    }

    #[test]
    fn extracts_types_members_and_namespaces() {
        let (facts, rodeo) = extract(
            r#"
namespace App.Services;

public partial class Greeter : IGreeter
{
    private readonly string prefix = "Hello";
    public string Greet(string name) => $"{prefix}, {name}";
    public int Count { get; set; }
}
"#,
        );
        let names = decl_names(&facts, &rodeo);
        assert_eq!(names, vec!["Greeter", "prefix", "Greet", "Count"]);

        let greeter = &facts.decls[0];
        assert_eq!(greeter.kind, SymbolKind::Type(TypeKind::Class));
        assert!(
            greeter
                .modifiers
                .contains(Modifiers::PUBLIC | Modifiers::PARTIAL)
        );
        assert_eq!(
            greeter
                .namespace
                .iter()
                .map(|s| rodeo.resolve(s))
                .collect::<Vec<_>>(),
            vec!["App", "Services"]
        );
        assert_eq!(greeter.base_names.len(), 1);

        let count = &facts.decls[3];
        assert_eq!(count.kind, SymbolKind::Member(MemberKind::Property));
        assert!(count.is_auto_property);

        // Member decls have the class as parent.
        assert!(facts.decls[1..].iter().all(|d| d.parent == Some(0)));
    }

    #[test]
    fn declaration_names_are_not_references() {
        let (facts, rodeo) = extract(
            r#"
namespace App;

public class Widget
{
    public void Spin(int speed)
    {
        int local = speed;
    }
}
"#,
        );
        let refs = ref_names(&facts, &rodeo);
        // "Widget", "Spin", "speed" (declaration side), "local" must not be
        // references; the parameter *use* of speed is one.
        assert!(!refs.contains(&"Widget".to_string()));
        assert!(!refs.contains(&"Spin".to_string()));
        assert!(!refs.contains(&"local".to_string()));
        assert!(refs.contains(&"speed".to_string()));
    }

    #[test]
    fn member_access_and_invocation_references() {
        let (facts, rodeo) = extract(
            r#"
class C
{
    void M()
    {
        var g = new Greeter();
        Console.WriteLine(g.Greet("hi"));
        Helper();
    }
}
"#,
        );
        let refs = ref_names(&facts, &rodeo);
        assert!(refs.contains(&"Greeter".to_string()));
        // Console.WriteLine and g.Greet are dotted identifier chains, so they
        // flatten into one path each (see try_flatten_dotted_path) rather
        // than separate Console/WriteLine and Greet references.
        assert!(refs.contains(&"Console.WriteLine".to_string()));
        assert!(refs.contains(&"g.Greet".to_string()));
        assert!(refs.contains(&"Helper".to_string()));

        let write_line = facts
            .refs
            .iter()
            .find(|r| rodeo.resolve(r.path.last().unwrap()) == "WriteLine")
            .unwrap();
        assert_eq!(write_line.kind, RawRefKind::Ambient);
    }

    #[test]
    fn nested_static_class_member_access_flattens_to_one_path() {
        let (facts, rodeo) = extract(
            r#"
class C
{
    void M()
    {
        var x = Tuning.Scores.Base;
    }
}
"#,
        );
        let path = facts
            .refs
            .iter()
            .find(|r| rodeo.resolve(r.path.last().unwrap()) == "Base")
            .map(|r| {
                r.path
                    .iter()
                    .map(|spur| rodeo.resolve(spur).to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap();
        assert_eq!(path, vec!["Tuning", "Scores", "Base"]);
    }

    #[test]
    fn generic_arguments_are_type_references() {
        let (facts, rodeo) = extract(
            r#"
class C
{
    void M(IServiceCollection services)
    {
        services.AddScoped<IFoo, Foo>();
    }
}
"#,
        );
        let type_refs: Vec<String> = facts
            .refs
            .iter()
            .filter(|r| r.kind == RawRefKind::Type)
            .map(|r| rodeo.resolve(r.path.last().unwrap()).to_string())
            .collect();
        assert!(type_refs.contains(&"IFoo".to_string()));
        assert!(type_refs.contains(&"Foo".to_string()));
        // The method itself is a member reference.
        let member_refs: Vec<String> = facts
            .refs
            .iter()
            .filter(|r| r.kind == RawRefKind::Member)
            .map(|r| rodeo.resolve(r.path.last().unwrap()).to_string())
            .collect();
        assert!(member_refs.contains(&"AddScoped".to_string()));
    }

    #[test]
    fn attributes_typeof_and_nameof() {
        let (facts, rodeo) = extract(
            r#"
class C
{
    [Fact]
    void Test()
    {
        var t = typeof(Migration001);
        var n = nameof(Order.Total);
    }
}
"#,
        );
        let refs = ref_names(&facts, &rodeo);
        assert!(refs.contains(&"Fact".to_string()));
        assert!(refs.contains(&"Migration001".to_string()));
        // Order.Total flattens into one dotted path (see
        // try_flatten_dotted_path) so the intermediate Order type resolves
        // too, not just the leaf member.
        assert!(refs.contains(&"Order.Total".to_string()));

        let fact = facts
            .refs
            .iter()
            .find(|r| rodeo.resolve(r.path.last().unwrap()) == "Fact")
            .unwrap();
        assert_eq!(fact.kind, RawRefKind::Attribute);

        let migration = facts
            .refs
            .iter()
            .find(|r| rodeo.resolve(r.path.last().unwrap()) == "Migration001")
            .unwrap();
        assert_eq!(migration.kind, RawRefKind::Type);

        // The test method records its attribute for entry-point detection.
        let test_method = facts
            .decls
            .iter()
            .find(|d| rodeo.resolve(&d.name) == "Test")
            .unwrap();
        assert_eq!(test_method.attributes.len(), 1);
        assert_eq!(rodeo.resolve(&test_method.attributes[0]), "Fact");
    }

    #[test]
    fn using_directives() {
        let (facts, rodeo) = extract(
            r#"
using System;
using static App.MathUtil;
using F = App.Factories.WidgetFactory;
global using App.Common;

class C { }
"#,
        );
        assert_eq!(facts.usings.len(), 4);
        assert!(!facts.usings[0].is_static && facts.usings[0].alias.is_none());
        assert!(facts.usings[1].is_static);
        assert!(facts.usings[2].alias.is_some());
        assert!(facts.usings[3].is_global);

        // Plain namespace usings are context, not references; static/alias
        // targets ARE references.
        let refs = ref_names(&facts, &rodeo);
        assert!(!refs.contains(&"System".to_string()));
        assert!(refs.contains(&"App.MathUtil".to_string()));
        assert!(refs.contains(&"App.Factories.WidgetFactory".to_string()));
    }

    #[test]
    fn field_declarators_each_get_a_decl_and_type_refs() {
        let (facts, rodeo) = extract(
            r#"
class C
{
    private Widget a = new Widget(), b;
}
"#,
        );
        let names = decl_names(&facts, &rodeo);
        assert_eq!(names, vec!["C", "a", "b"]);

        // The Widget type annotation is credited to both declarators (a is
        // decl index 1, b is decl index 2), so either field alone keeps the
        // type alive. The `new Widget()` initializer adds a third ref.
        let widget_origins: Vec<u32> = facts
            .refs
            .iter()
            .filter(|r| {
                r.kind == RawRefKind::Type && rodeo.resolve(r.path.last().unwrap()) == "Widget"
            })
            .map(|r| r.origin)
            .collect();
        assert!(widget_origins.contains(&1));
        assert!(widget_origins.contains(&2));
    }

    #[test]
    fn top_level_statements_and_file_root_origin() {
        let (facts, rodeo) = extract(
            r#"
using App;

var greeter = new Greeter();
Console.WriteLine(greeter.Greet("hi"));
"#,
        );
        assert!(facts.has_top_level_statements);
        let greeter_ref = facts
            .refs
            .iter()
            .find(|r| rodeo.resolve(r.path.last().unwrap()) == "Greeter")
            .unwrap();
        assert_eq!(greeter_ref.origin, FILE_ROOT);
    }

    #[test]
    fn explicit_interface_impl_and_base_types() {
        let (facts, rodeo) = extract(
            r#"
namespace App;

class Repo : IRepo, BaseRepo
{
    void IRepo.Save() { }
}
"#,
        );
        let repo = &facts.decls[0];
        let bases: Vec<String> = repo
            .base_names
            .iter()
            .map(|p| {
                p.iter()
                    .map(|s| rodeo.resolve(s))
                    .collect::<Vec<_>>()
                    .join(".")
            })
            .collect();
        assert_eq!(bases, vec!["IRepo", "BaseRepo"]);

        let save = &facts.decls[1];
        assert!(save.is_explicit_interface_impl);

        // The explicit interface qualifier keeps IRepo referenced.
        let refs = ref_names(&facts, &rodeo);
        assert!(refs.contains(&"IRepo".to_string()));
    }

    #[test]
    fn enums_operators_events_and_indexers() {
        let (facts, rodeo) = extract(
            r#"
namespace App;

enum Status { Active, Inactive = 2 }

class Box
{
    public event EventHandler Changed;
    public int this[int i] => i;
    public static Box operator +(Box a, Box b) => a;
}
"#,
        );
        let names = decl_names(&facts, &rodeo);
        assert_eq!(
            names,
            vec![
                "Status",
                "Active",
                "Inactive",
                "Box",
                "Changed",
                "this[]",
                "operator +"
            ]
        );
        assert_eq!(
            facts.decls[1].kind,
            SymbolKind::Member(MemberKind::EnumMember)
        );
        assert_eq!(facts.decls[4].kind, SymbolKind::Member(MemberKind::Event));
        assert_eq!(facts.decls[5].kind, SymbolKind::Member(MemberKind::Indexer));
        assert_eq!(
            facts.decls[6].kind,
            SymbolKind::Member(MemberKind::Operator)
        );
    }

    #[test]
    fn nested_namespaces_and_nested_types() {
        let (facts, rodeo) = extract(
            r#"
namespace Outer
{
    namespace Inner
    {
        class A
        {
            class B { }
        }
    }
}
"#,
        );
        let a = &facts.decls[0];
        assert_eq!(
            a.namespace
                .iter()
                .map(|s| rodeo.resolve(s))
                .collect::<Vec<_>>(),
            vec!["Outer", "Inner"]
        );
        let b = &facts.decls[1];
        assert_eq!(b.parent, Some(0));
    }

    #[test]
    fn parse_errors_are_flagged_but_extraction_continues() {
        let (facts, _rodeo) = extract("class Broken { void M( } \nclass Fine { }");
        assert!(facts.has_errors);
        assert!(!facts.decls.is_empty());
    }

    fn decl_named<'a>(facts: &'a FileFacts, rodeo: &Interner, name: &str) -> &'a RawDecl {
        facts
            .decls
            .iter()
            .find(|d| rodeo.resolve(&d.name) == name)
            .unwrap_or_else(|| panic!("no decl named {name}"))
    }

    #[test]
    fn empty_method_has_baseline_complexity_of_one() {
        let (facts, rodeo) = extract(
            r#"
class C
{
    void M() { }
}
"#,
        );
        assert_eq!(decl_named(&facts, &rodeo, "M").cyclomatic, 1);
    }

    #[test]
    fn if_else_adds_one_decision_point() {
        let (facts, rodeo) = extract(
            r#"
class C
{
    void M(bool flag)
    {
        if (flag)
        {
            DoA();
        }
        else
        {
            DoB();
        }
    }
}
"#,
        );
        assert_eq!(decl_named(&facts, &rodeo, "M").cyclomatic, 2);
    }

    #[test]
    fn short_circuit_operators_each_add_one_decision_point() {
        let (facts, rodeo) = extract(
            r#"
class C
{
    bool M(bool a, bool b, string s)
    {
        return a && b || s != null && (s ?? "x").Length > 0;
    }
}
"#,
        );
        // baseline 1 + && + || + && = 4. The `??` is not a decision point.
        assert_eq!(decl_named(&facts, &rodeo, "M").cyclomatic, 4);
    }

    #[test]
    fn null_coalescing_is_not_a_decision_point() {
        // A hand-rolled `with`: thirteen defaulted fields, no branching a
        // reader has to trace. Counting each `??` scored this 14 against a
        // limit of 10 while cognitive complexity correctly scored it 0.
        let (facts, rodeo) = extract(
            r#"
class Unit
{
    public Unit Copy(string? a1 = null, string? a2 = null, string? a3 = null,
                     string? a4 = null, string? a5 = null, string? a6 = null,
                     string? a7 = null, string? a8 = null, string? a9 = null,
                     string? a10 = null, string? a11 = null, string? a12 = null,
                     string? a13 = null)
    {
        return new Unit(a1 ?? A1, a2 ?? A2, a3 ?? A3, a4 ?? A4, a5 ?? A5,
                        a6 ?? A6, a7 ?? A7, a8 ?? A8, a9 ?? A9, a10 ?? A10,
                        a11 ?? A11, a12 ?? A12, a13 ?? A13);
    }
}
"#,
        );
        let copy = decl_named(&facts, &rodeo, "Copy");
        assert_eq!(copy.cyclomatic, 1);
        assert_eq!(copy.cognitive, 0);
    }

    #[test]
    fn a_chain_of_null_coalescing_is_still_one() {
        // Sibling `??`s in an argument list are what the issue reported, but a
        // left-associative chain has to come out at 1 as well.
        let (facts, rodeo) = extract(
            r#"
class C
{
    string M(string? a, string? b, string c) => a ?? b ?? c;
}
"#,
        );
        assert_eq!(decl_named(&facts, &rodeo, "M").cyclomatic, 1);
    }

    #[test]
    fn dropping_null_coalescing_leaves_the_boolean_operators_alone() {
        // `&&` and `||` are McCabe decision points and stay that way; only
        // `??` was ever the false positive.
        let (facts, rodeo) = extract(
            r#"
class C
{
    bool M(bool a, bool b, string? s, string fallback)
    {
        return a && b && (s ?? fallback).Length > 0;
    }
}
"#,
        );
        // baseline 1 + two `&&` = 3.
        assert_eq!(decl_named(&facts, &rodeo, "M").cyclomatic, 3);
    }

    #[test]
    fn a_throwing_null_coalescing_fallback_is_not_charged_for_the_operator() {
        // The deliberate trade-off, pinned rather than left to be discovered:
        // `?? throw` really is a branch, but roe cannot tell it apart from
        // `?? "default"` without evaluating the right-hand side, and the
        // defaulting case is overwhelmingly the common one.
        let (facts, rodeo) = extract(
            r#"
class C
{
    string M(string? s) => s ?? throw new InvalidOperationException();
}
"#,
        );
        assert_eq!(decl_named(&facts, &rodeo, "M").cyclomatic, 1);
    }

    #[test]
    fn branching_inside_a_null_coalescing_fallback_is_still_counted() {
        // Dropping the operator must not drop the expression: the fallback is
        // walked like any other, so a ternary hiding in it still costs.
        let (facts, rodeo) = extract(
            r#"
class C
{
    string M(string? s, bool flag) => s ?? (flag ? "yes" : "no");
}
"#,
        );
        // baseline 1 + the ternary = 2.
        assert_eq!(decl_named(&facts, &rodeo, "M").cyclomatic, 2);
    }

    #[test]
    fn null_coalescing_assignment_never_counted_either() {
        // `??=` parses as an `assignment_expression`, not a
        // `binary_expression`, so it was never a decision point. Pinned so
        // nobody "fixes" the asymmetry back into existence.
        let (facts, rodeo) = extract(
            r#"
class C
{
    string M(string? s, string fallback)
    {
        s ??= fallback;
        return s;
    }
}
"#,
        );
        assert_eq!(decl_named(&facts, &rodeo, "M").cyclomatic, 1);
    }

    #[test]
    fn every_loop_form_is_a_decision_point() {
        let (facts, rodeo) = extract(
            r#"
class C
{
    void M(int[] xs)
    {
        for (var i = 0; i < xs.Length; i++) { DoA(); }
        foreach (var x in xs) { DoB(); }
        while (Ready()) { DoC(); }
        do { DoD(); } while (Ready());
    }
}
"#,
        );
        // The grammar calls it `foreach_statement`, not `for_each_statement`;
        // spelling it the latter way silently skipped every foreach loop.
        assert_eq!(decl_named(&facts, &rodeo, "M").cyclomatic, 5);
    }

    #[test]
    fn straight_line_code_has_no_cognitive_complexity() {
        let (facts, rodeo) = extract(
            r#"
class C
{
    int M()
    {
        var a = 1;
        var b = 2;
        return a + b;
    }
}
"#,
        );
        // Unlike cyclomatic, cognitive complexity has no baseline of 1.
        assert_eq!(decl_named(&facts, &rodeo, "M").cognitive, 0);
        assert_eq!(decl_named(&facts, &rodeo, "M").cyclomatic, 1);
    }

    #[test]
    fn nesting_costs_more_than_flat_structure_at_equal_cyclomatic() {
        let (facts, rodeo) = extract(
            r#"
class C
{
    void Flat(bool a, bool b, bool c)
    {
        if (a) { DoA(); }
        if (b) { DoB(); }
        if (c) { DoC(); }
    }

    void Nested(bool a, bool b, bool c)
    {
        if (a)
        {
            if (b)
            {
                if (c) { DoC(); }
            }
        }
    }
}
"#,
        );
        let flat = decl_named(&facts, &rodeo, "Flat");
        let nested = decl_named(&facts, &rodeo, "Nested");

        // Cyclomatic cannot tell these apart — three branches either way.
        assert_eq!(flat.cyclomatic, 4);
        assert_eq!(nested.cyclomatic, 4);

        // Cognitive can: 1+1+1 flat, versus 1+2+3 as the nesting deepens.
        assert_eq!(flat.cognitive, 3);
        assert_eq!(nested.cognitive, 6);
    }

    #[test]
    fn else_if_chain_stays_flat() {
        let (facts, rodeo) = extract(
            r#"
class C
{
    void M(int x)
    {
        if (x == 1) { DoA(); }
        else if (x == 2) { DoB(); }
        else { DoC(); }
    }
}
"#,
        );
        // if + else-if + else, each a flat 1 — the chain reads linearly.
        assert_eq!(decl_named(&facts, &rodeo, "M").cognitive, 3);
    }

    #[test]
    fn a_condition_costs_on_top_of_the_branch_that_tests_it() {
        let (facts, rodeo) = extract(
            r#"
class C
{
    void M(bool a, bool b)
    {
        if (a && b) { DoA(); }
    }
}
"#,
        );
        // 1 for the `if`, 1 for the `&&` run inside its condition.
        assert_eq!(decl_named(&facts, &rodeo, "M").cognitive, 2);
    }

    #[test]
    fn both_kinds_of_else_nest_their_bodies_but_cost_a_flat_one() {
        let (facts, rodeo) = extract(
            r#"
class C
{
    void M(bool a, bool b, bool c, bool d)
    {
        if (a) { DoA(); }
        else if (b) { if (c) { DoB(); } }
        else { if (d) { DoC(); } }
    }
}
"#,
        );
        // The `if` costs 1. The `else if` costs a flat 1 plus the nested `if`
        // in its body at depth 1, so 3; the trailing `else` the same.
        assert_eq!(decl_named(&facts, &rodeo, "M").cognitive, 7);
    }

    #[test]
    fn a_loop_inside_a_branch_costs_more_than_one_at_the_top() {
        let (facts, rodeo) = extract(
            r#"
class C
{
    void Top(int[] xs)
    {
        foreach (var x in xs) { DoA(); }
        foreach (var x in xs) { DoB(); }
    }

    void Buried(int[] xs)
    {
        foreach (var x in xs)
        {
            foreach (var y in xs) { DoA(); }
        }
    }
}
"#,
        );
        // Two loops at depth 0 cost 1 each...
        assert_eq!(decl_named(&facts, &rodeo, "Top").cognitive, 2);
        // ...where the same two loops nested cost 1 + 2, because the inner one
        // is charged for the depth it sits at.
        assert_eq!(decl_named(&facts, &rodeo, "Buried").cognitive, 3);
    }

    #[test]
    fn goto_costs_one_wherever_it_sits() {
        let (facts, rodeo) = extract(
            r#"
class C
{
    void M(bool a)
    {
        if (a) { goto done; }
        DoA();
        done:
        DoB();
    }
}
"#,
        );
        // 1 for the `if`, 1 for the jump. A `goto` breaks the reader's flow
        // but does not deepen it, so its depth does not matter.
        assert_eq!(decl_named(&facts, &rodeo, "M").cognitive, 2);
    }

    #[test]
    fn a_goto_costs_on_top_of_what_it_jumps_on() {
        // `goto case` takes a constant expression, and a constant expression
        // can still hold a boolean operator the reader has to work through.
        // The jump costs its own point on top of that.
        let (facts, rodeo) = extract(
            r#"
class C
{
    const bool A = true;
    const bool B = false;

    void M(bool x)
    {
        switch (x)
        {
            case true:
                goto case A || B;
            case false:
                break;
        }
    }
}
"#,
        );
        // 1 for the `switch`, 1 for the jump, 1 for the `||` it jumps on.
        assert_eq!(decl_named(&facts, &rodeo, "M").cognitive, 3);
    }

    #[test]
    fn line_count_covers_the_whole_file() {
        let (facts, _) = extract("class C\n{\n}\n");

        assert_eq!(facts.line_count, 4);
    }

    #[test]
    fn switch_costs_one_regardless_of_case_count() {
        let (facts, rodeo) = extract(
            r#"
class C
{
    void M(int x)
    {
        switch (x)
        {
            case 1: DoA(); break;
            case 2: DoB(); break;
            case 3: DoC(); break;
            default: DoD(); break;
        }
    }
}
"#,
        );
        let m = decl_named(&facts, &rodeo, "M");
        // One dispatch to understand, however many arms it has...
        assert_eq!(m.cognitive, 1);
        // ...where cyclomatic charges for every one of them.
        assert_eq!(m.cyclomatic, 5);
    }

    #[test]
    fn boolean_runs_count_once_but_mixed_operators_count_separately() {
        let (facts, rodeo) = extract(
            r#"
class C
{
    bool Same(bool a, bool b, bool c) => a && b && c;

    bool Mixed(bool a, bool b, bool c) => a && b || c;
}
"#,
        );
        assert_eq!(decl_named(&facts, &rodeo, "Same").cognitive, 1);
        assert_eq!(decl_named(&facts, &rodeo, "Mixed").cognitive, 2);
    }

    #[test]
    fn lambdas_nest_their_contents_without_costing_anything() {
        let (facts, rodeo) = extract(
            r#"
class C
{
    void M(List<int> xs)
    {
        xs.ForEach(x =>
        {
            if (x > 0) { DoA(); }
        });
    }
}
"#,
        );
        // The lambda itself is free, but the `if` inside it sits one level
        // deep, so it costs 2 rather than 1.
        assert_eq!(decl_named(&facts, &rodeo, "M").cognitive, 2);
    }

    #[test]
    fn loops_nest_the_branches_inside_them() {
        let (facts, rodeo) = extract(
            r#"
class C
{
    void M(int[] xs)
    {
        foreach (var x in xs)
        {
            if (x > 0) { DoA(); }
        }
    }
}
"#,
        );
        // foreach at depth 0 costs 1, the if at depth 1 costs 2.
        assert_eq!(decl_named(&facts, &rodeo, "M").cognitive, 3);
    }

    #[test]
    fn switch_sections_and_arms_are_decision_points() {
        let (facts, rodeo) = extract(
            r#"
class C
{
    void Statement(int x)
    {
        switch (x)
        {
            case 1:
            case 2:
                DoA();
                break;
            default:
                DoB();
                break;
        }
    }

    int Expression(int x) => x switch
    {
        1 => 10,
        2 => 20,
        _ => 0,
    };
}
"#,
        );
        // Three switch_section nodes (case 1, case 2, default — the grammar
        // starts a new section at every label) => baseline 1 + 3 = 4.
        assert_eq!(decl_named(&facts, &rodeo, "Statement").cyclomatic, 4);
        // Three switch_expression_arm nodes => baseline 1 + 3 = 4.
        assert_eq!(decl_named(&facts, &rodeo, "Expression").cyclomatic, 4);
    }

    #[test]
    fn arrow_bodied_method_is_measured() {
        let (facts, rodeo) = extract(
            r#"
class C
{
    int M(bool flag) => flag ? 1 : 0;
}
"#,
        );
        let m = decl_named(&facts, &rodeo, "M");
        assert_eq!(m.cyclomatic, 2);
        assert_eq!(m.body_lines, 1);
        assert_eq!(m.parameters.required, 1);
    }

    #[test]
    fn property_accessors_are_measured_and_summed() {
        let (facts, rodeo) = extract(
            r#"
class C
{
    private int backing;

    public int Value
    {
        get { return backing; }
        set
        {
            if (value < 0)
            {
                throw new ArgumentOutOfRangeException();
            }
            backing = value;
        }
    }
}
"#,
        );
        let value = decl_named(&facts, &rodeo, "Value");
        // baseline 1 + 1 extra accessor + 1 if = 3.
        assert_eq!(value.cyclomatic, 3);
    }

    #[test]
    fn auto_property_has_no_complexity() {
        let (facts, rodeo) = extract(
            r#"
class C
{
    public int Value { get; set; }
}
"#,
        );
        let value = decl_named(&facts, &rodeo, "Value");
        assert_eq!(value.cyclomatic, 0);
        assert_eq!(value.body_lines, 0);
    }

    #[test]
    fn parameter_count_is_recorded() {
        let (facts, rodeo) = extract(
            r#"
class C
{
    void M(int a, string b, bool c) { }
}
"#,
        );
        let parameters = decl_named(&facts, &rodeo, "M").parameters;
        assert_eq!(parameters.required, 3);
        assert_eq!(parameters.total(), 3);
    }

    #[test]
    fn defaulted_parameters_are_optional_not_required() {
        // The issue's first repro: sixteen parameters, two of which the caller
        // actually has to supply. `Sum(1, 2)` is the whole call site.
        let (facts, rodeo) = extract(
            r#"
class C
{
    int Sum(int required1, int required2, int p3 = 0, int p4 = 0, int p5 = 0,
            int p6 = 0, int p7 = 0, int p8 = 0, int p9 = 0, int p10 = 0,
            int p11 = 0, int p12 = 0, int p13 = 0, int p14 = 0, int p15 = 0,
            int p16 = 0)
    {
        return required1 + required2;
    }
}
"#,
        );
        let parameters = decl_named(&facts, &rodeo, "Sum").parameters;
        assert_eq!(parameters.required, 2);
        assert_eq!(parameters.optional, 14);
        assert_eq!(parameters.out, 0);
        assert_eq!(parameters.total(), 16, "the declared signature is intact");
    }

    #[test]
    fn out_parameters_are_returns_not_arguments() {
        // The issue's second repro. An `out` is a return value wearing a
        // parameter's clothes — the caller supplies nothing.
        let (facts, rodeo) = extract(
            r#"
class C
{
    bool TryGet(int a, int b, int c, int d, out int first, out int second)
    {
        first = 0;
        second = 0;
        return false;
    }
}
"#,
        );
        let parameters = decl_named(&facts, &rodeo, "TryGet").parameters;
        assert_eq!(parameters.required, 4);
        assert_eq!(parameters.optional, 0);
        assert_eq!(parameters.out, 2);
        assert_eq!(parameters.total(), 6);
    }

    #[test]
    fn ref_and_in_and_the_extension_receiver_are_all_required() {
        // `ref` is supplied *and* has to be reasoned about; `in` is a calling
        // convention, not an escape hatch; and an extension method's `this`
        // receiver is still an argument, just written to the left of the dot.
        let (facts, rodeo) = extract(
            r#"
static class C
{
    public static void M(this Widget widget, ref int total, in Span<byte> data)
    {
    }
}
"#,
        );
        let parameters = decl_named(&facts, &rodeo, "M").parameters;
        assert_eq!(parameters.required, 3);
        assert_eq!(parameters.optional, 0);
        assert_eq!(parameters.out, 0);
    }

    #[test]
    fn a_params_array_is_optional() {
        // The grammar inlines `_parameter_array` into the parameter list, so
        // there is no `parameter` node to find and roe used not to count a
        // `params` array at all. Omitting it is always legal, so it belongs
        // with the defaulted ones rather than nowhere.
        let (facts, rodeo) = extract(
            r#"
class C
{
    void M(string format, params object[] arguments) { }
}
"#,
        );
        let parameters = decl_named(&facts, &rodeo, "M").parameters;
        assert_eq!(parameters.required, 1);
        assert_eq!(parameters.optional, 1);
        assert_eq!(parameters.total(), 2);
    }

    #[test]
    fn an_indexers_parameters_are_measured_the_same_way() {
        // Indexers use `bracketed_parameter_list`, a different node kind with
        // the same `parameter` children.
        let (facts, rodeo) = extract(
            r#"
class C
{
    public int this[int row, int column = 0] => 0;
}
"#,
        );
        let parameters = decl_named(&facts, &rodeo, "this[]").parameters;
        assert_eq!(parameters.required, 1);
        assert_eq!(parameters.optional, 1);
    }

    #[test]
    fn an_overload_suffix_uses_the_declared_arity_not_the_required_one() {
        // The `/arity` suffix exists so a reader can match a report row to a
        // signature in the source, so it has to count what the source shows.
        let (facts, rodeo) = extract(
            r#"
class C
{
    void Send(string to) { }

    void Send(string to, string cc = "", out int sent) { sent = 0; }
}
"#,
        );
        let declared: Vec<u32> = facts
            .decls
            .iter()
            .filter(|decl| rodeo.resolve(&decl.name) == "Send")
            .map(|decl| decl.parameters.total())
            .collect();

        assert_eq!(declared, vec![1, 3]);
    }
}
