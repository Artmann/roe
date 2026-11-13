//! The `perf` catalog: a second descent over the tree-sitter tree
//! `src/extract` has already parsed, looking for patterns that waste runtime.
//!
//! This module owns the state — where the loops are, what they bind, whether
//! the enclosing function is `async` — and [`checks`] owns the judgement about
//! what any of it means.
//!
//! Findings are attributed to the member they sit in by source position rather
//! than by re-deriving the walker's declaration numbering: [`RawFinding::member`]
//! records exactly what `RawDecl::line` and `RawDecl::column` record, so the
//! command can join the two lists without either side knowing about the other.

mod checks;
pub mod types;

use tree_sitter::Node;

use crate::model::PerfFindingKind;
use checks::Context;
use types::DeclaredTypes;

/// A finding as the descent sees it: everything but which file it came from
/// and what the enclosing member is called.
#[derive(Debug)]
pub struct RawFinding {
    pub column: u32,
    pub detail: String,
    pub fix: String,
    pub kind: PerfFindingKind,
    pub line: u32,
    /// Loops enclosing the finding inside its own member.
    pub loop_depth: u32,
    /// Start of the enclosing member's name, matching `RawDecl::line` and
    /// `RawDecl::column`. `None` for top-level statements, which belong to no
    /// declaration.
    pub member: Option<(u32, u32)>,
}

/// Scan one already-parsed file.
pub fn scan(root: Node, source: &[u8]) -> Vec<RawFinding> {
    let mut scan = Scan {
        findings: Vec::new(),
        loop_variables: Vec::new(),
        source,
        types: DeclaredTypes::from_file(root, source),
    };

    scan.walk(root, Frame::default());

    scan.findings
}

/// What is true of the node currently being walked. Copied down the tree
/// rather than pushed and popped, so a detector can never see a sibling's
/// state.
#[derive(Clone, Copy, Default)]
struct Frame {
    in_async: bool,
    loop_depth: u32,
    member: Option<(u32, u32)>,
}

struct Scan<'source> {
    findings: Vec<RawFinding>,
    /// Names bound by the enclosing loops, innermost last. A stack rather than
    /// part of [`Frame`] because it grows, and copying it at every node would
    /// dominate the cost of the pass.
    loop_variables: Vec<&'source str>,
    source: &'source [u8],
    types: DeclaredTypes,
}

impl<'source> Scan<'source> {
    fn walk(&mut self, node: Node, frame: Frame) {
        let mut frame = frame;

        // A new member resets everything: its own loop nesting, its own
        // `async`, and its own locals layered over the file's fields.
        if let Some(position) = member_position(node) {
            self.types.enter_member(node, self.source);

            frame = Frame {
                in_async: is_async(node, self.source),
                loop_depth: 0,
                member: Some(position),
            };
        }

        if let Some(found) = self.inspect(node, frame) {
            let position = node.start_position();

            self.findings.push(RawFinding {
                column: position.column as u32 + 1,
                detail: found.detail,
                fix: found.fix,
                kind: found.kind,
                line: position.row as u32 + 1,
                loop_depth: frame.loop_depth,
                member: frame.member,
            });
        }

        match node.kind() {
            // The collection is evaluated once, before the first pass, so only
            // the body is hot. Reporting `foreach (var x in await
            // _db.Orders.ToListAsync())` as a query inside a loop would be
            // exactly backwards.
            "foreach_statement" => {
                self.walk_loop(node, frame, &["body"]);
            }

            // The condition and the update run on every pass; only the
            // initializer runs once.
            "for_statement" => {
                self.walk_loop_excluding(node, frame, &["initializer"]);
            }

            "do_statement" | "while_statement" => {
                self.walk_loop_excluding(node, frame, &[]);
            }

            // `from x in items select …` — the same shape in different syntax.
            "query_expression" => {
                self.walk_loop_excluding(node, frame, &[]);
            }

            "anonymous_method_expression" | "lambda_expression" => {
                self.walk_lambda(node, frame);
            }

            // A local function is its own function: its `async` is its own,
            // but it is not a separate declaration, so findings inside it
            // still belong to the member that contains it.
            "local_function_statement" => {
                frame.in_async = is_async(node, self.source);
                self.walk_children(node, frame);
            }

            _ => self.walk_children(node, frame),
        }
    }

    fn inspect(&self, node: Node, frame: Frame) -> Option<checks::Match> {
        let context = Context {
            in_async: frame.in_async,
            loop_depth: frame.loop_depth,
            loop_variables: &self.loop_variables,
            source: self.source,
            types: &self.types,
        };

        checks::inspect(node, &context)
    }

    fn walk_children(&mut self, node: Node, frame: Frame) {
        let mut cursor = node.walk();

        for child in node.children(&mut cursor).collect::<Vec<_>>() {
            self.walk(child, frame);
        }
    }

    /// Walk a loop, counting only the named fields as hot.
    fn walk_loop(&mut self, node: Node, frame: Frame, hot_fields: &[&str]) {
        let bound = self.push_loop_variables(node);
        let mut cursor = node.walk();
        let children = node.children(&mut cursor).collect::<Vec<_>>();
        let mut hot = Vec::new();

        for field in hot_fields {
            if let Some(child) = node.child_by_field_name(field) {
                hot.push(child);
            }
        }

        for child in children {
            let frame = if hot.contains(&child) {
                Frame {
                    loop_depth: frame.loop_depth + 1,
                    ..frame
                }
            } else {
                frame
            };

            self.walk(child, frame);
        }

        self.pop_loop_variables(bound);
    }

    /// Walk a loop where everything is hot except the named fields.
    fn walk_loop_excluding(&mut self, node: Node, frame: Frame, cold_fields: &[&str]) {
        let bound = self.push_loop_variables(node);
        let mut cursor = node.walk();
        let children = node.children(&mut cursor).collect::<Vec<_>>();
        let mut cold = Vec::new();

        for field in cold_fields {
            if let Some(child) = node.child_by_field_name(field) {
                cold.push(child);
            }
        }

        let inner = Frame {
            loop_depth: frame.loop_depth + 1,
            ..frame
        };

        for child in children {
            let frame = if cold.contains(&child) { frame } else { inner };

            self.walk(child, frame);
        }

        self.pop_loop_variables(bound);
    }

    /// A lambda is its own function, and — when it is handed to a LINQ
    /// operator — its own loop.
    fn walk_lambda(&mut self, node: Node, frame: Frame) {
        let mut frame = Frame {
            in_async: is_async(node, self.source),
            ..frame
        };

        if !is_per_element_lambda(node, self.source) {
            self.walk_children(node, frame);

            return;
        }

        let bound = self.push_lambda_parameters(node);

        frame.loop_depth += 1;
        self.walk_children(node, frame);
        self.pop_loop_variables(bound);
    }

    /// Bind whatever a loop construct names, returning how many were pushed.
    fn push_loop_variables(&mut self, node: Node) -> usize {
        let before = self.loop_variables.len();

        match node.kind() {
            "foreach_statement" => {
                if let Some(left) = node.child_by_field_name("left") {
                    self.push_identifiers(left);
                }
            }

            "for_statement" => {
                let mut cursor = node.walk();

                for child in node.children(&mut cursor) {
                    if child.kind() == "variable_declaration" {
                        self.push_declarator_names(child);
                    }
                }
            }

            "query_expression" => {
                let mut cursor = node.walk();

                for clause in node.children(&mut cursor) {
                    if !matches!(clause.kind(), "from_clause" | "join_clause" | "let_clause") {
                        continue;
                    }

                    if let Some(name) = clause.child_by_field_name("name") {
                        self.push_name(name);
                    }
                }
            }

            _ => {}
        }

        self.loop_variables.len() - before
    }

    fn push_lambda_parameters(&mut self, node: Node) -> usize {
        let before = self.loop_variables.len();

        if let Some(parameters) = node.child_by_field_name("parameters") {
            match parameters.kind() {
                "implicit_parameter" => self.push_name(parameters),

                _ => {
                    let mut cursor = parameters.walk();

                    for parameter in parameters.children(&mut cursor) {
                        if matches!(parameter.kind(), "implicit_parameter" | "parameter") {
                            let name = parameter.child_by_field_name("name").unwrap_or(parameter);

                            self.push_name(name);
                        }
                    }
                }
            }
        }

        self.loop_variables.len() - before
    }

    fn push_declarator_names(&mut self, declaration: Node) {
        let mut cursor = declaration.walk();

        for declarator in declaration.children(&mut cursor) {
            if declarator.kind() != "variable_declarator" {
                continue;
            }

            if let Some(name) = declarator.child_by_field_name("name") {
                self.push_name(name);
            }
        }
    }

    /// `foreach (var (id, name) in pairs)` binds more than one name.
    fn push_identifiers(&mut self, node: Node) {
        if node.kind() == "identifier" {
            self.push_name(node);

            return;
        }

        let mut cursor = node.walk();
        let children = node.children(&mut cursor).collect::<Vec<_>>();

        for child in children {
            self.push_identifiers(child);
        }
    }

    fn push_name(&mut self, node: Node) {
        let name = checks::text(node, self.source);

        if !name.is_empty() {
            self.loop_variables.push(name);
        }
    }

    fn pop_loop_variables(&mut self, count: usize) {
        self.loop_variables
            .truncate(self.loop_variables.len() - count);
    }
}

/// Where a declaration's name starts, or `None` when the node is not a member
/// declaration.
///
/// This mirrors `walk::member_decl`, which records the name node's position on
/// the `RawDecl` (falling back to the declaration itself when there is no name,
/// as for operators). The two must agree exactly or attribution silently fails.
fn member_position(node: Node) -> Option<(u32, u32)> {
    let named = match node.kind() {
        "constructor_declaration"
        | "conversion_operator_declaration"
        | "destructor_declaration"
        | "event_declaration"
        | "indexer_declaration"
        | "method_declaration"
        | "operator_declaration"
        | "property_declaration" => node.child_by_field_name("name").unwrap_or(node),

        // Fields carry the shared annotation on the declaration and the name
        // on each declarator; the walker attributes to the declarator.
        "event_field_declaration" | "field_declaration" => {
            let mut cursor = node.walk();
            let declaration = node
                .children(&mut cursor)
                .find(|child| child.kind() == "variable_declaration")?;

            let mut cursor = declaration.walk();
            let declarator = declaration
                .children(&mut cursor)
                .find(|child| child.kind() == "variable_declarator")?;

            declarator.child_by_field_name("name")?
        }

        _ => return None,
    };

    let position = named.start_position();

    Some((position.row as u32 + 1, position.column as u32 + 1))
}

fn is_async(node: Node, source: &[u8]) -> bool {
    let mut cursor = node.walk();

    node.children(&mut cursor).any(|child| {
        matches!(child.kind(), "async" | "modifier") && checks::text(child, source) == "async"
    })
}

/// Whether a lambda was handed to a LINQ operator, and so runs once per
/// element rather than an unknown number of times.
fn is_per_element_lambda(node: Node, source: &[u8]) -> bool {
    let Some(argument) = node.parent() else {
        return false;
    };

    let Some(list) = argument.parent() else {
        return false;
    };

    if list.kind() != "argument_list" {
        return false;
    }

    let Some(invocation) = list.parent() else {
        return false;
    };

    checks::invoked_name(invocation, source).is_some_and(checks::is_per_element_operator)
}
