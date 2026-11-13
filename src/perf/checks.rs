//! The catalog: one detector per [`PerfFindingKind`].
//!
//! Every detector is a pure function of a node and its surroundings, so the
//! descent in [`super`] owns all the state and this file owns all the
//! judgement. Detectors never look upward past what [`Context`] hands them,
//! with one deliberate exception — [`initializes_a_field`], which has to,
//! because a `new HttpClient()` in a field initializer is the *fix* for the
//! pattern rather than the bug.
//!
//! Where a check needs to know what something is, it asks
//! [`super::types::DeclaredTypes`] and takes silence for an answer.

use tree_sitter::Node;

use super::types::{DeclaredType, DeclaredTypes};
use crate::model::PerfFindingKind;

/// Everything a detector is allowed to know about where it is.
pub struct Context<'a> {
    /// The enclosing function is declared `async`. Scoped to the nearest
    /// lambda or local function, not the outermost method — a synchronous
    /// callback inside an async method is its own function.
    pub in_async: bool,
    /// How many loops enclose this node. LINQ operator lambdas count.
    pub loop_depth: u32,
    /// Names the enclosing loops bind, so a query can be asked whether it
    /// varies with the iteration.
    pub loop_variables: &'a [&'a str],
    pub source: &'a [u8],
    pub types: &'a DeclaredTypes,
}

/// A detector's verdict. The descent supplies the position and loop depth.
pub struct Match {
    pub detail: String,
    pub fix: String,
    pub kind: PerfFindingKind,
}

impl Match {
    fn new(kind: PerfFindingKind, detail: impl Into<String>, fix: impl Into<String>) -> Self {
        Match {
            detail: detail.into(),
            fix: fix.into(),
            kind,
        }
    }
}

/// Terminal operators that only a database library defines.
///
/// **Async names only.** This is the entire reason name-based database
/// detection works without a type system: nothing but EF Core, Dapper, or ADO
/// declares a `ToListAsync`, whereas `ToList` and `First` are ordinary LINQ
/// over an in-memory list and would fire on half of every codebase. Adding a
/// sync name to this list breaks the check.
///
/// `SaveChangesAsync` is absent on purpose — it has its own check.
fn is_database_terminal(name: &str) -> bool {
    matches!(
        name,
        // Entity Framework Core.
        "AllAsync"
            | "AnyAsync"
            | "AverageAsync"
            | "CountAsync"
            | "ExecuteDeleteAsync"
            | "ExecuteUpdateAsync"
            | "FindAsync"
            | "FirstAsync"
            | "FirstOrDefaultAsync"
            | "LastAsync"
            | "LastOrDefaultAsync"
            | "LoadAsync"
            | "LongCountAsync"
            | "MaxAsync"
            | "MinAsync"
            | "SingleAsync"
            | "SingleOrDefaultAsync"
            | "SumAsync"
            | "ToArrayAsync"
            | "ToDictionaryAsync"
            | "ToListAsync"
            // Dapper.
            | "ExecuteAsync"
            | "QueryAsync"
            | "QueryFirstAsync"
            | "QueryFirstOrDefaultAsync"
            | "QueryMultipleAsync"
            | "QuerySingleAsync"
            | "QuerySingleOrDefaultAsync"
            // Raw ADO.NET.
            | "ExecuteNonQueryAsync"
            | "ExecuteReaderAsync"
            | "ExecuteScalarAsync"
    )
}

/// LINQ operators whose lambda runs once per element.
///
/// The descent counts a lambda passed to one of these as a loop level — a
/// `.Select(x => …)` is a loop wearing a different hat, and a linear scan
/// inside one is just as quadratic as it would be inside a `foreach`. A lambda
/// passed to anything else (`Task.Run`, an event handler, a DI registration)
/// runs an unknown number of times and is not counted.
pub fn is_per_element_operator(name: &str) -> bool {
    matches!(
        name,
        "All"
            | "Any"
            | "Average"
            | "Count"
            | "DistinctBy"
            | "ExceptBy"
            | "First"
            | "FirstOrDefault"
            | "ForEach"
            | "GroupBy"
            | "GroupJoin"
            | "Join"
            | "Last"
            | "LastOrDefault"
            | "LongCount"
            | "Max"
            | "MaxBy"
            | "Min"
            | "MinBy"
            | "OrderBy"
            | "OrderByDescending"
            | "Select"
            | "SelectMany"
            | "Single"
            | "SingleOrDefault"
            | "SkipWhile"
            | "Sum"
            | "TakeWhile"
            | "ThenBy"
            | "ThenByDescending"
            | "Where"
    )
}

pub fn text<'a>(node: Node, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap_or("")
}

/// The bare name behind `Foo` or `Foo<T>`.
fn simple_name<'a>(node: Node, source: &'a [u8]) -> Option<&'a str> {
    match node.kind() {
        "identifier" => Some(text(node, source)),
        "generic_name" => {
            let base = node.child_by_field_name("name").or_else(|| node.child(0))?;

            Some(text(base, source))
        }
        _ => None,
    }
}

/// The method an invocation calls: `x.Foo()` → `Foo`, `Foo()` → `Foo`.
pub fn invoked_name<'a>(node: Node, source: &'a [u8]) -> Option<&'a str> {
    if node.kind() != "invocation_expression" {
        return None;
    }

    let function = node.child_by_field_name("function")?;

    match function.kind() {
        "member_access_expression" | "member_binding_expression" => {
            simple_name(function.child_by_field_name("name")?, source)
        }
        _ => simple_name(function, source),
    }
}

/// What an invocation was called on: the `x` in `x.Foo()`. `None` for a bare
/// `Foo()`.
fn invocation_receiver<'tree>(node: Node<'tree>) -> Option<Node<'tree>> {
    let function = node.child_by_field_name("function")?;

    if function.kind() != "member_access_expression" {
        return None;
    }

    function.child_by_field_name("expression")
}

fn argument_count(node: Node) -> usize {
    node.child_by_field_name("arguments")
        .map(|list| list.named_child_count())
        .unwrap_or(0)
}

/// The first identifier anywhere under `node` that names one of the enclosing
/// loop variables — the test that separates a query varying per iteration from
/// one that does not.
///
/// The whole invocation is searched, not just its arguments, because the loop
/// variable is as often in the receiver
/// (`_db.X.Where(r => r.Id == order.Id).FirstOrDefaultAsync()`) as in the
/// argument list.
fn references_loop_variable<'a>(
    node: Node,
    source: &'a [u8],
    loop_variables: &[&'a str],
) -> Option<&'a str> {
    if loop_variables.is_empty() {
        return None;
    }

    let mut cursor = node.walk();
    let mut pending = vec![node];

    while let Some(current) = pending.pop() {
        if current.kind() == "identifier" {
            let name = text(current, source);

            if let Some(found) = loop_variables.iter().find(|variable| **variable == name) {
                return Some(found);
            }
        }

        pending.extend(current.children(&mut cursor));
    }

    None
}

/// Whether `node` sits in a field or property initializer rather than in
/// executable member code.
///
/// `private static readonly Regex Pattern = new Regex("…");` is precisely what
/// `regex-per-call` tells people to write, and a single long-lived
/// `HttpClient` field is what `transient-http-client` tells them to write.
/// Reporting the fix as the bug would be worse than reporting nothing.
fn initializes_a_field(node: Node) -> bool {
    let mut current = node;

    while let Some(parent) = current.parent() {
        match parent.kind() {
            "field_declaration" | "event_field_declaration" => {
                return true;
            }

            // A property's `=> …` body or `= …` initializer runs on every
            // access, so it is executable code, not a one-time initializer.
            "block" | "arrow_expression_clause" | "accessor_declaration" => {
                return false;
            }

            _ => {}
        }

        current = parent;
    }

    false
}

fn is_string_literal(node: Node) -> bool {
    matches!(
        node.kind(),
        "string_literal" | "raw_string_literal" | "verbatim_string_literal"
    )
}

/// `x.ToLower()` and friends, returning the receiver as written.
fn case_fold_call<'a>(node: Node, source: &'a [u8]) -> Option<&'a str> {
    let name = invoked_name(node, source)?;

    if !matches!(
        name,
        "ToLower" | "ToLowerInvariant" | "ToUpper" | "ToUpperInvariant"
    ) {
        return None;
    }

    if argument_count(node) > 0 {
        return None;
    }

    Some(text(invocation_receiver(node)?, source))
}

/// Whether an `await` is already accounted for by a database check, so
/// `await-in-loop` does not report the same line twice.
pub fn is_reported_database_call(node: Node, source: &[u8]) -> bool {
    let Some(name) = invoked_name(node, source) else {
        return false;
    };

    is_database_terminal(name) || matches!(name, "SaveChanges" | "SaveChangesAsync")
}

/// Run every detector that applies to this node kind and return the first
/// verdict.
///
/// At most one finding per node: the detectors are ordered worst-first within
/// each kind, so a `SaveChangesAsync` in a loop is reported as the write
/// amplification it is rather than as a generic repeated query.
pub fn inspect(node: Node, context: &Context) -> Option<Match> {
    match node.kind() {
        "assignment_expression" => string_concat_in_loop(node, context),

        "await_expression" => await_in_loop(node, context),

        "binary_expression" => {
            case_insensitive_compare(node, context).or_else(|| redundant_count(node, context))
        }

        "invocation_expression" => sync_over_async_call(node, context)
            .or_else(|| save_changes_in_loop(node, context))
            .or_else(|| database_query_in_loop(node, context))
            .or_else(|| blocking_sleep_in_async(node, context))
            .or_else(|| list_lookup_in_loop(node, context))
            .or_else(|| redundant_chain(node, context)),

        "member_access_expression" => {
            sync_over_async_result(node, context).or_else(|| unstable_clock(node, context))
        }

        "object_creation_expression" => {
            transient_http_client(node, context).or_else(|| regex_per_call(node, context))
        }

        _ => None,
    }
}

/// `.Result` — a property access, so it has no invocation node of its own.
fn sync_over_async_result(node: Node, context: &Context) -> Option<Match> {
    let name = node.child_by_field_name("name")?;

    if text(name, context.source) != "Result" {
        return None;
    }

    // `x.Result()` is a method called `Result`, not the Task property. Only an
    // invocation has a `function` field, so this is the whole test.
    let is_called = node
        .parent()
        .is_some_and(|parent| parent.child_by_field_name("function") == Some(node));

    if is_called {
        return None;
    }

    let receiver = text(node.child_by_field_name("expression")?, context.source);

    Some(Match::new(
        PerfFindingKind::SyncOverAsync,
        format!("`{receiver}.Result` blocks the calling thread until the task finishes"),
        "await the task instead — under load this deadlocks or exhausts the thread pool",
    ))
}

/// `.Wait()` and `.GetAwaiter().GetResult()`.
fn sync_over_async_call(node: Node, context: &Context) -> Option<Match> {
    let name = invoked_name(node, context.source)?;

    match name {
        "Wait" if argument_count(node) == 0 => Some(Match::new(
            PerfFindingKind::SyncOverAsync,
            "`.Wait()` blocks the calling thread until the task finishes",
            "await the task instead — under load this deadlocks or exhausts the thread pool",
        )),

        "GetResult" => {
            // Only the `.GetAwaiter().GetResult()` shape. A bare `GetResult()`
            // is somebody's own method.
            let receiver = invocation_receiver(node)?;
            let awaiter = invoked_name(receiver, context.source)?;

            if awaiter != "GetAwaiter" {
                return None;
            }

            Some(Match::new(
                PerfFindingKind::SyncOverAsync,
                "`.GetAwaiter().GetResult()` blocks the calling thread until the task finishes",
                "await the task instead — under load this deadlocks or exhausts the thread pool",
            ))
        }

        _ => None,
    }
}

fn save_changes_in_loop(node: Node, context: &Context) -> Option<Match> {
    if context.loop_depth == 0 {
        return None;
    }

    let name = invoked_name(node, context.source)?;

    if !matches!(name, "SaveChanges" | "SaveChangesAsync") {
        return None;
    }

    Some(Match::new(
        PerfFindingKind::SaveChangesInLoop,
        format!("`{name}` runs a round trip and a transaction on every pass"),
        "make every change first and call SaveChangesAsync once after the loop — \
         EF batches the writes into one round trip",
    ))
}

/// The two database-in-a-loop findings, split by whether the query varies with
/// the iteration. They are one function because they share every test but the
/// last, and getting that test the wrong way round would give the reader the
/// wrong fix.
fn database_query_in_loop(node: Node, context: &Context) -> Option<Match> {
    if context.loop_depth == 0 {
        return None;
    }

    let name = invoked_name(node, context.source)?;

    if !is_database_terminal(name) {
        return None;
    }

    match references_loop_variable(node, context.source, context.loop_variables) {
        Some(variable) => Some(Match::new(
            PerfFindingKind::NPlusOneQuery,
            format!("`{name}` runs one query per `{variable}`"),
            "fetch the rows for the whole collection in one query before the loop, \
             then match them in memory",
        )),

        None => Some(Match::new(
            PerfFindingKind::LoopInvariantQuery,
            format!("`{name}` runs every pass but does not depend on the iteration"),
            "hoist the query above the loop and reuse the result",
        )),
    }
}

fn blocking_sleep_in_async(node: Node, context: &Context) -> Option<Match> {
    if !context.in_async {
        return None;
    }

    if invoked_name(node, context.source)? != "Sleep" {
        return None;
    }

    let receiver = text(invocation_receiver(node)?, context.source);

    if !matches!(receiver, "Thread" | "System.Threading.Thread") {
        return None;
    }

    Some(Match::new(
        PerfFindingKind::BlockingSleepInAsync,
        "`Thread.Sleep` holds the thread instead of releasing it",
        "await Task.Delay(…) instead — it frees the thread for other work while it waits",
    ))
}

fn list_lookup_in_loop(node: Node, context: &Context) -> Option<Match> {
    if context.loop_depth == 0 {
        return None;
    }

    let name = invoked_name(node, context.source)?;

    // `Any()`, `First()`, and `FirstOrDefault()` with no predicate stop at the
    // first element, so only the predicate forms scan.
    let scans = match name {
        "Contains" | "IndexOf" | "LastIndexOf" => true,
        "Any" | "Find" | "First" | "FirstOrDefault" | "Single" | "SingleOrDefault" => {
            argument_count(node) > 0
        }
        _ => false,
    };

    if !scans {
        return None;
    }

    let receiver = text(invocation_receiver(node)?, context.source);

    if context.types.lookup_receiver(receiver)? != DeclaredType::LinearCollection {
        return None;
    }

    let fix = if matches!(name, "Contains" | "IndexOf" | "LastIndexOf") {
        format!("declare `{receiver}` as a HashSet<T> — membership then costs one hash lookup")
    } else {
        format!(
            "build a Dictionary keyed by the field you match on before the loop, \
             instead of scanning `{receiver}` each pass"
        )
    };

    Some(Match::new(
        PerfFindingKind::ListLookupInLoop,
        format!("`{receiver}.{name}` walks the whole collection, once per pass"),
        fix,
    ))
}

fn await_in_loop(node: Node, context: &Context) -> Option<Match> {
    if context.loop_depth == 0 {
        return None;
    }

    // Database awaits already have a finding of their own, and two lines
    // saying the same thing is worse than one.
    let awaited = node.named_child(0)?;

    if is_reported_database_call(awaited, context.source) {
        return None;
    }

    Some(Match::new(
        PerfFindingKind::AwaitInLoop,
        "each pass waits for the previous one to finish before starting",
        "if the iterations are independent, start them all and await Task.WhenAll(…)",
    ))
}

fn string_concat_in_loop(node: Node, context: &Context) -> Option<Match> {
    if context.loop_depth == 0 {
        return None;
    }

    let operator = node.child_by_field_name("operator")?;

    if text(operator, context.source) != "+=" {
        return None;
    }

    let target = text(node.child_by_field_name("left")?, context.source);

    if context.types.lookup_receiver(target)? != DeclaredType::Text {
        return None;
    }

    Some(Match::new(
        PerfFindingKind::StringConcatInLoop,
        format!("`{target} += …` allocates a fresh copy of the whole string each pass"),
        "accumulate into a StringBuilder and call ToString() once after the loop",
    ))
}

fn case_insensitive_compare(node: Node, context: &Context) -> Option<Match> {
    let operator = node.child_by_field_name("operator")?;

    if !matches!(text(operator, context.source), "==" | "!=") {
        return None;
    }

    let left = node.child_by_field_name("left")?;
    let right = node.child_by_field_name("right")?;

    // One side folding case is the pattern; the other may be the matching
    // fold or a literal to compare against.
    let folded =
        case_fold_call(left, context.source).or_else(|| case_fold_call(right, context.source))?;
    let other_side_is_comparable = case_fold_call(left, context.source).is_some()
        && case_fold_call(right, context.source).is_some()
        || is_string_literal(left)
        || is_string_literal(right);

    if !other_side_is_comparable {
        return None;
    }

    Some(Match::new(
        PerfFindingKind::CaseInsensitiveCompare,
        format!("folding `{folded}` allocates a second string just to compare it"),
        "use string.Equals(a, b, StringComparison.OrdinalIgnoreCase) — it compares in place",
    ))
}

/// `.Count() > 0` and its relatives.
fn redundant_count(node: Node, context: &Context) -> Option<Match> {
    let operator = text(node.child_by_field_name("operator")?, context.source);
    let left = node.child_by_field_name("left")?;
    let right = node.child_by_field_name("right")?;

    if invoked_name(left, context.source)? != "Count" || argument_count(left) > 0 {
        return None;
    }

    if text(right, context.source) != "0" {
        return None;
    }

    let replacement = match operator {
        ">" | "!=" => "Any()",
        "==" => "!… .Any()",
        _ => return None,
    };

    Some(Match::new(
        PerfFindingKind::RedundantLinq,
        format!("`.Count() {operator} 0` walks the whole sequence to answer a yes-or-no question"),
        format!("use {replacement} — it stops at the first element"),
    ))
}

/// Chained LINQ where the second call could have absorbed the first.
fn redundant_chain(node: Node, context: &Context) -> Option<Match> {
    let outer = invoked_name(node, context.source)?;
    let receiver = invocation_receiver(node)?;
    let inner = invoked_name(receiver, context.source)?;

    // `.Where(p).First()` — the terminal operator takes a predicate itself.
    if inner == "Where"
        && argument_count(receiver) == 1
        && argument_count(node) == 0
        && matches!(
            outer,
            "Any"
                | "Count"
                | "First"
                | "FirstOrDefault"
                | "Last"
                | "LastOrDefault"
                | "Single"
                | "SingleOrDefault"
        )
    {
        return Some(Match::new(
            PerfFindingKind::RedundantLinq,
            format!("`.Where(…).{outer}()` builds an intermediate sequence"),
            format!("pass the predicate straight to {outer}(…)"),
        ));
    }

    // `.OrderBy(k).First()` — a full sort to find one element.
    if matches!(inner, "OrderBy" | "OrderByDescending")
        && matches!(outer, "First" | "FirstOrDefault")
        && argument_count(node) == 0
    {
        let replacement = if inner == "OrderBy" { "MinBy" } else { "MaxBy" };

        return Some(Match::new(
            PerfFindingKind::RedundantLinq,
            format!("`.{inner}(…).{outer}()` sorts the whole sequence to take one element"),
            format!("use {replacement}(…) — it makes a single pass"),
        ));
    }

    // `.ToList().Where(…)` — materialising a list that is thrown away.
    if matches!(inner, "ToArray" | "ToList")
        && argument_count(receiver) == 0
        && is_per_element_operator(outer)
    {
        return Some(Match::new(
            PerfFindingKind::RedundantLinq,
            format!("`.{inner}().{outer}(…)` copies the sequence before reading it"),
            format!("drop the .{inner}() — {outer} streams the source directly"),
        ));
    }

    None
}

fn transient_http_client(node: Node, context: &Context) -> Option<Match> {
    let constructed = simple_name(node.child_by_field_name("type")?, context.source)?;

    if constructed != "HttpClient" {
        return None;
    }

    // A long-lived field holding one client is the recommended fix, not the
    // bug.
    if initializes_a_field(node) {
        return None;
    }

    Some(Match::new(
        PerfFindingKind::TransientHttpClient,
        "`new HttpClient()` opens its own connection pool and leaves the socket \
         in TIME_WAIT after disposal",
        "inject IHttpClientFactory and call CreateClient(), or hold one static client",
    ))
}

fn regex_per_call(node: Node, context: &Context) -> Option<Match> {
    let constructed = simple_name(node.child_by_field_name("type")?, context.source)?;

    if constructed != "Regex" {
        return None;
    }

    // Only a literal pattern can be hoisted without thinking about it.
    let arguments = node.child_by_field_name("arguments")?;
    let first = arguments.named_child(0)?;
    let pattern = first.named_child(0).unwrap_or(first);

    if !is_string_literal(pattern) {
        return None;
    }

    // `private static readonly Regex Pattern = new Regex("…");` is the fix.
    if initializes_a_field(node) {
        return None;
    }

    Some(Match::new(
        PerfFindingKind::RegexPerCall,
        "`new Regex(…)` recompiles the pattern on every call",
        "hoist it to a static readonly field, or use a [GeneratedRegex] partial method",
    ))
}

fn unstable_clock(node: Node, context: &Context) -> Option<Match> {
    let name = text(node.child_by_field_name("name")?, context.source);

    if name != "Now" {
        return None;
    }

    let receiver = text(node.child_by_field_name("expression")?, context.source);

    if !matches!(receiver, "DateTime" | "DateTimeOffset") {
        return None;
    }

    Some(Match::new(
        PerfFindingKind::UnstableClock,
        format!("`{receiver}.Now` converts to the local time zone on every read"),
        format!("use {receiver}.UtcNow if the value is stored or compared, not displayed"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::perf::RawFinding;

    fn scan(source: &str) -> Vec<RawFinding> {
        let mut parser = crate::extract::make_parser();
        let tree = parser.parse(source.as_bytes(), None).expect("parses");

        crate::perf::scan(tree.root_node(), source.as_bytes())
    }

    /// The findings of `body`, placed in an `async` method so awaits parse.
    fn kinds(body: &str) -> Vec<PerfFindingKind> {
        found(&format!(
            "class Sample\n{{\n    async Task Run()\n    {{\n{body}\n    }}\n}}\n"
        ))
    }

    /// The same, in a plain synchronous method.
    fn sync_kinds(body: &str) -> Vec<PerfFindingKind> {
        found(&format!(
            "class Sample\n{{\n    void Run()\n    {{\n{body}\n    }}\n}}\n"
        ))
    }

    fn found(source: &str) -> Vec<PerfFindingKind> {
        scan(source).into_iter().map(|found| found.kind).collect()
    }

    fn depth(body: &str, kind: PerfFindingKind) -> u32 {
        let source =
            format!("class Sample\n{{\n    async Task Run()\n    {{\n{body}\n    }}\n}}\n");

        scan(&source)
            .into_iter()
            .find(|found| found.kind == kind)
            .map(|found| found.loop_depth)
            .unwrap_or_else(|| panic!("no {} finding in:\n{body}", kind.slug()))
    }

    // ---- sync-over-async

    #[test]
    fn blocking_on_a_task_is_reported() {
        assert_eq!(
            kinds("var value = Fetch().Result;"),
            vec![PerfFindingKind::SyncOverAsync]
        );
        assert_eq!(
            kinds("Fetch().Wait();"),
            vec![PerfFindingKind::SyncOverAsync]
        );
        assert_eq!(
            kinds("Fetch().GetAwaiter().GetResult();"),
            vec![PerfFindingKind::SyncOverAsync]
        );
    }

    #[test]
    fn a_method_called_result_is_not_the_task_property() {
        assert!(kinds("var value = calculator.Result();").is_empty());
    }

    #[test]
    fn a_bare_get_result_is_somebody_elses_method() {
        assert!(kinds("var value = handle.GetResult();").is_empty());
    }

    #[test]
    fn waiting_with_a_timeout_is_not_reported() {
        // `Wait(timeout)` returns a bool the caller is deciding on; it is a
        // deliberate bounded wait, not an accidental block.
        assert!(kinds("var done = Fetch().Wait(TimeSpan.FromSeconds(1));").is_empty());
    }

    // ---- the database checks

    #[test]
    fn a_query_that_varies_with_the_loop_is_an_n_plus_one() {
        assert_eq!(
            kinds(
                "foreach (var order in orders)\n\
                 {\n\
                     var customer = await _db.Customers.FirstOrDefaultAsync(c => c.Id == order.CustomerId);\n\
                 }"
            ),
            vec![PerfFindingKind::NPlusOneQuery]
        );
    }

    #[test]
    fn a_loop_variable_in_the_receiver_still_counts() {
        // `.Where(… order …).ToListAsync()` puts the dependency before the
        // terminal operator, not in its arguments.
        assert_eq!(
            kinds(
                "foreach (var order in orders)\n\
                 {\n\
                     var lines = await _db.Lines.Where(l => l.OrderId == order.Id).ToListAsync();\n\
                 }"
            ),
            vec![PerfFindingKind::NPlusOneQuery]
        );
    }

    #[test]
    fn a_query_that_does_not_vary_is_loop_invariant() {
        assert_eq!(
            kinds(
                "foreach (var order in orders)\n\
                 {\n\
                     var settings = await _db.Settings.FirstOrDefaultAsync();\n\
                 }"
            ),
            vec![PerfFindingKind::LoopInvariantQuery]
        );
    }

    #[test]
    fn the_synchronous_linq_names_are_never_database_calls() {
        // The whole premise of name-based database detection: `First` and
        // `ToList` are ordinary LINQ over an in-memory list, and matching them
        // would fire on half of every codebase.
        assert!(
            sync_kinds(
                "foreach (var order in orders)\n\
                 {\n\
                     var first = candidates.First(c => c.Id == order.Id);\n\
                     var all = candidates.ToList();\n\
                     var any = candidates.Any(c => c.Id == order.Id);\n\
                 }"
            )
            .is_empty()
        );
    }

    #[test]
    fn a_query_outside_a_loop_is_not_reported() {
        assert!(kinds("var all = await _db.Orders.ToListAsync();").is_empty());
    }

    #[test]
    fn the_collection_a_foreach_iterates_is_not_inside_the_loop() {
        // It is evaluated once, before the first pass. Reporting it as a
        // repeated query would be exactly backwards.
        assert!(
            kinds(
                "foreach (var order in await _db.Orders.ToListAsync())\n\
                 {\n\
                     Handle(order);\n\
                 }"
            )
            .is_empty()
        );
    }

    #[test]
    fn saving_inside_a_loop_is_reported_as_itself() {
        // Not as a repeated query — the fix is different.
        assert_eq!(
            kinds("foreach (var order in orders)\n{\n    await _db.SaveChangesAsync();\n}"),
            vec![PerfFindingKind::SaveChangesInLoop]
        );
    }

    #[test]
    fn saving_outside_a_loop_is_fine() {
        assert!(kinds("await _db.SaveChangesAsync();").is_empty());
    }

    #[test]
    fn a_for_loop_counter_is_a_loop_variable() {
        assert_eq!(
            kinds(
                "for (var index = 0; index < count; index++)\n\
                 {\n\
                     var row = await _db.Rows.FirstOrDefaultAsync(r => r.Index == index);\n\
                 }"
            ),
            vec![PerfFindingKind::NPlusOneQuery]
        );
    }

    #[test]
    fn a_query_expression_range_is_a_loop_variable() {
        assert_eq!(
            sync_kinds(
                "var customers = from order in orders\n\
                 select _db.Customers.FirstOrDefaultAsync(c => c.Id == order.CustomerId);"
            ),
            vec![PerfFindingKind::NPlusOneQuery]
        );
    }

    #[test]
    fn a_linq_lambda_parameter_is_a_loop_variable() {
        assert_eq!(
            sync_kinds(
                "var customers = orders.Select(order => \
                 _db.Customers.FirstOrDefaultAsync(c => c.Id == order.CustomerId));"
            ),
            vec![PerfFindingKind::NPlusOneQuery]
        );
        assert_eq!(
            sync_kinds(
                "var customers = orders.Select((order, index) => \
                 _db.Customers.FirstOrDefaultAsync(c => c.Id == order.CustomerId));"
            ),
            vec![PerfFindingKind::NPlusOneQuery]
        );
    }

    #[test]
    fn a_loop_variable_goes_out_of_scope_with_its_loop() {
        // A later loop that happens to mention the same name is not iterating
        // over it.
        assert_eq!(
            kinds(
                "foreach (var order in orders)\n\
                 {\n\
                     Handle(order);\n\
                 }\n\
                 while (more)\n\
                 {\n\
                     var settings = await _db.Settings.FirstOrDefaultAsync(s => s.Id == order);\n\
                 }"
            ),
            vec![PerfFindingKind::LoopInvariantQuery]
        );
    }

    #[test]
    fn a_lambda_parameter_goes_out_of_scope_with_its_lambda() {
        assert_eq!(
            kinds(
                "var ids = orders.Select(order => order.Id);\n\
                 while (more)\n\
                 {\n\
                     var settings = await _db.Settings.FirstOrDefaultAsync(s => s.Id == order);\n\
                 }"
            ),
            vec![PerfFindingKind::LoopInvariantQuery]
        );
    }

    #[test]
    fn a_generic_database_call_is_still_a_database_call() {
        assert_eq!(
            kinds(
                "foreach (var order in orders)\n\
                 {\n\
                     var customer = await _db.Customers.FirstOrDefaultAsync<Customer>(c => c.Id == order.CustomerId);\n\
                 }"
            ),
            vec![PerfFindingKind::NPlusOneQuery]
        );
    }

    // ---- transient-http-client

    #[test]
    fn constructing_a_client_per_call_is_reported() {
        assert_eq!(
            kinds("var client = new HttpClient();"),
            vec![PerfFindingKind::TransientHttpClient]
        );
    }

    #[test]
    fn a_client_held_in_a_field_is_the_fix_not_the_bug() {
        assert!(
            found(
                "class Sample\n\
                 {\n\
                     private static readonly HttpClient Client = new HttpClient();\n\
                     void Run() { }\n\
                 }"
            )
            .is_empty()
        );
    }

    #[test]
    fn a_client_built_inside_a_field_initializers_lambda_is_per_call() {
        // The field holds the factory, not the client, so every call still
        // constructs a new one.
        assert_eq!(
            found(
                "class Sample\n\
                 {\n\
                     private readonly Func<HttpClient> _factory = () => { return new HttpClient(); };\n\
                 }"
            ),
            vec![PerfFindingKind::TransientHttpClient]
        );
    }

    // ---- string-concat-in-loop

    #[test]
    fn concatenating_a_declared_string_in_a_loop_is_reported() {
        assert_eq!(
            sync_kinds(
                "string report = \"\";\n\
                 foreach (var line in lines)\n\
                 {\n\
                     report += line;\n\
                 }"
            ),
            vec![PerfFindingKind::StringConcatInLoop]
        );
    }

    #[test]
    fn concatenating_a_name_of_unknown_type_is_not_reported() {
        // The invariant that keeps this check usable: silence when roe cannot
        // tell what the name is.
        assert!(
            sync_kinds(
                "var total = Seed();\n\
                 foreach (var line in lines)\n\
                 {\n\
                     total += line;\n\
                 }"
            )
            .is_empty()
        );
    }

    #[test]
    fn adding_to_a_number_in_a_loop_is_not_reported() {
        assert!(
            sync_kinds(
                "int total = 0;\n\
                 foreach (var line in lines)\n\
                 {\n\
                     total += line.Amount;\n\
                 }"
            )
            .is_empty()
        );
    }

    #[test]
    fn concatenating_outside_a_loop_is_fine() {
        assert!(sync_kinds("string report = \"\";\nreport += \"done\";").is_empty());
    }

    // ---- blocking-sleep-in-async

    #[test]
    fn sleeping_in_an_async_method_is_reported() {
        assert_eq!(
            kinds("Thread.Sleep(500);"),
            vec![PerfFindingKind::BlockingSleepInAsync]
        );
    }

    #[test]
    fn sleeping_in_a_synchronous_method_is_not() {
        // There is no thread to release, so there is nothing to fix.
        assert!(sync_kinds("Thread.Sleep(500);").is_empty());
    }

    #[test]
    fn sleeping_in_an_async_local_function_is_reported() {
        assert_eq!(
            sync_kinds("async Task Inner()\n{\n    Thread.Sleep(500);\n}"),
            vec![PerfFindingKind::BlockingSleepInAsync]
        );
    }

    #[test]
    fn sleeping_in_a_synchronous_local_function_is_not() {
        assert!(kinds("void Inner()\n{\n    Thread.Sleep(500);\n}").is_empty());
    }

    #[test]
    fn a_modifier_other_than_async_does_not_make_a_method_async() {
        assert!(
            found("class Sample\n{\n    private static void Run()\n    {\n        Thread.Sleep(500);\n    }\n}\n")
                .is_empty()
        );
    }

    // ---- case-insensitive-compare

    #[test]
    fn comparing_two_folded_strings_is_reported() {
        assert_eq!(
            sync_kinds("if (left.ToLower() == right.ToLower()) { }"),
            vec![PerfFindingKind::CaseInsensitiveCompare]
        );
    }

    #[test]
    fn comparing_a_folded_string_against_a_literal_is_reported() {
        assert_eq!(
            sync_kinds("if (name.ToUpperInvariant() == \"ADMIN\") { }"),
            vec![PerfFindingKind::CaseInsensitiveCompare]
        );
    }

    #[test]
    fn folding_one_side_against_an_unknown_expression_is_not_reported() {
        assert!(sync_kinds("if (name.ToLower() == other) { }").is_empty());
    }

    #[test]
    fn folding_with_a_culture_is_not_reported() {
        // The culture-aware overload is a different question from an ordinal
        // ignore-case compare, and the suggested fix would change behaviour.
        assert!(sync_kinds("if (left.ToLower(culture) == right.ToLower(culture)) { }").is_empty());
    }

    // ---- list-lookup-in-loop

    #[test]
    fn scanning_a_declared_list_in_a_loop_is_reported() {
        assert_eq!(
            sync_kinds(
                "List<string> skus = Load();\n\
                 foreach (var line in lines)\n\
                 {\n\
                     if (skus.Contains(line.Sku)) { }\n\
                 }"
            ),
            vec![PerfFindingKind::ListLookupInLoop]
        );
    }

    #[test]
    fn scanning_a_hash_set_is_already_fast() {
        assert!(
            sync_kinds(
                "HashSet<string> skus = Load();\n\
                 foreach (var line in lines)\n\
                 {\n\
                     if (skus.Contains(line.Sku)) { }\n\
                 }"
            )
            .is_empty()
        );
    }

    #[test]
    fn scanning_a_name_of_unknown_type_is_not_reported() {
        assert!(
            sync_kinds(
                "var skus = Load();\n\
                 foreach (var line in lines)\n\
                 {\n\
                     if (skus.Contains(line.Sku)) { }\n\
                 }"
            )
            .is_empty()
        );
    }

    #[test]
    fn taking_the_first_element_of_a_list_does_not_scan_it() {
        // `First()` without a predicate stops immediately; only the predicate
        // form walks the collection.
        assert!(
            sync_kinds(
                "List<string> skus = Load();\n\
                 foreach (var line in lines)\n\
                 {\n\
                     var head = skus.FirstOrDefault();\n\
                 }"
            )
            .is_empty()
        );
    }

    #[test]
    fn searching_a_list_by_predicate_in_a_loop_is_reported() {
        assert_eq!(
            sync_kinds(
                "List<Sku> skus = Load();\n\
                 foreach (var line in lines)\n\
                 {\n\
                     var match = skus.FirstOrDefault(s => s.Code == line.Sku);\n\
                 }"
            ),
            vec![PerfFindingKind::ListLookupInLoop]
        );
    }

    // ---- await-in-loop

    #[test]
    fn awaiting_in_a_loop_is_reported() {
        assert_eq!(
            kinds("foreach (var order in orders)\n{\n    await Publish(order);\n}"),
            vec![PerfFindingKind::AwaitInLoop]
        );
    }

    #[test]
    fn a_database_await_is_reported_once_not_twice() {
        // It already has a more specific finding, and two lines saying the
        // same thing is worse than one.
        assert_eq!(
            kinds("foreach (var order in orders)\n{\n    await _db.SaveChangesAsync();\n}"),
            vec![PerfFindingKind::SaveChangesInLoop]
        );
    }

    #[test]
    fn awaiting_outside_a_loop_is_fine() {
        assert!(kinds("await Publish(order);").is_empty());
    }

    // ---- regex-per-call

    #[test]
    fn compiling_a_literal_pattern_per_call_is_reported() {
        assert_eq!(
            sync_kinds("var pattern = new Regex(\"^[a-z]+$\");"),
            vec![PerfFindingKind::RegexPerCall]
        );
    }

    #[test]
    fn a_pattern_hoisted_to_a_field_is_the_fix_not_the_bug() {
        assert!(
            found(
                "class Sample\n\
                 {\n\
                     private static readonly Regex Pattern = new Regex(\"^[a-z]+$\");\n\
                     void Run() { }\n\
                 }"
            )
            .is_empty()
        );
    }

    #[test]
    fn a_pattern_built_at_runtime_cannot_simply_be_hoisted() {
        assert!(sync_kinds("var pattern = new Regex(BuildPattern());").is_empty());
    }

    // ---- redundant-linq

    #[test]
    fn counting_to_answer_a_yes_or_no_question_is_reported() {
        assert_eq!(
            sync_kinds("if (orders.Count() > 0) { }"),
            vec![PerfFindingKind::RedundantLinq]
        );
    }

    #[test]
    fn filtering_before_a_terminal_operator_is_reported() {
        assert_eq!(
            sync_kinds("var first = orders.Where(o => o.IsOpen).First();"),
            vec![PerfFindingKind::RedundantLinq]
        );
    }

    #[test]
    fn sorting_to_take_one_element_is_reported() {
        assert_eq!(
            sync_kinds("var cheapest = orders.OrderBy(o => o.Total).First();"),
            vec![PerfFindingKind::RedundantLinq]
        );
    }

    #[test]
    fn materialising_a_sequence_before_reading_it_is_reported() {
        assert_eq!(
            sync_kinds("var open = orders.ToList().Where(o => o.IsOpen);"),
            vec![PerfFindingKind::RedundantLinq]
        );
    }

    #[test]
    fn a_plain_count_is_not_redundant() {
        assert!(sync_kinds("var total = orders.Count();").is_empty());
    }

    #[test]
    fn comparing_a_count_to_zero_for_equality_is_reported() {
        assert_eq!(
            sync_kinds("if (orders.Count() == 0) { }"),
            vec![PerfFindingKind::RedundantLinq]
        );
    }

    #[test]
    fn only_a_parameterless_count_against_zero_is_redundant() {
        assert!(sync_kinds("if (orders.Count(o => o.IsOpen) > 0) { }").is_empty());
        assert!(sync_kinds("if (orders.Sum() > 0) { }").is_empty());
        assert!(sync_kinds("if (orders.Count() < 0) { }").is_empty());
    }

    #[test]
    fn sorting_suggests_the_matching_single_pass_operator() {
        let fix_for = |body: &str| {
            let source = format!("class Sample\n{{\n    void Run()\n    {{\n{body}\n    }}\n}}\n");

            scan(&source)
                .into_iter()
                .map(|found| found.fix)
                .next()
                .unwrap_or_else(|| panic!("no finding in:\n{body}"))
        };

        assert!(fix_for("var low = orders.OrderBy(o => o.Total).First();").contains("MinBy"));
        assert!(
            fix_for("var high = orders.OrderByDescending(o => o.Total).First();").contains("MaxBy")
        );
    }

    #[test]
    fn chains_that_cannot_be_folded_are_not_redundant() {
        assert!(sync_kinds("var ids = orders.Where(o => o.IsOpen).Select(o => o.Id);").is_empty());
        assert!(sync_kinds("var first = orders.Select(o => o.Id).First();").is_empty());
        assert!(
            sync_kinds("var first = orders.Where(o => o.IsOpen).First(o => o.IsNew);").is_empty()
        );
        assert!(sync_kinds("var sorted = orders.OrderBy(o => o.Total).ToList();").is_empty());
        assert!(sync_kinds("var open = orders.AsEnumerable().Where(o => o.IsOpen);").is_empty());
    }

    // ---- unstable-clock

    #[test]
    fn reading_the_local_clock_is_reported() {
        assert_eq!(
            sync_kinds("var stamp = DateTime.Now;"),
            vec![PerfFindingKind::UnstableClock]
        );
    }

    #[test]
    fn reading_the_utc_clock_is_fine() {
        assert!(sync_kinds("var stamp = DateTime.UtcNow;").is_empty());
    }

    // ---- heat

    #[test]
    fn loop_depth_counts_the_loops_that_actually_enclose_the_finding() {
        assert_eq!(
            depth("var stamp = DateTime.Now;", PerfFindingKind::UnstableClock),
            0
        );
        assert_eq!(
            depth(
                "foreach (var order in orders)\n{\n    var stamp = DateTime.Now;\n}",
                PerfFindingKind::UnstableClock
            ),
            1
        );
        assert_eq!(
            depth(
                "foreach (var order in orders)\n\
                 {\n\
                     foreach (var line in order.Lines)\n\
                     {\n\
                         var stamp = DateTime.Now;\n\
                     }\n\
                 }",
                PerfFindingKind::UnstableClock
            ),
            2
        );
    }

    #[test]
    fn a_linq_operator_lambda_counts_as_a_loop_level() {
        // `.Select(x => …)` is a loop wearing a different hat.
        assert_eq!(
            depth(
                "var stamps = orders.Select(o => DateTime.Now);",
                PerfFindingKind::UnstableClock
            ),
            1
        );
    }

    #[test]
    fn a_lambda_that_is_not_a_linq_operator_is_not_a_loop() {
        // `Task.Run(() => …)` runs once, and guessing otherwise would inflate
        // the ranking of anything wrapped in a callback.
        assert_eq!(
            depth(
                "Task.Run(() => DateTime.Now);",
                PerfFindingKind::UnstableClock
            ),
            0
        );
    }

    #[test]
    fn a_while_loop_counts_its_condition() {
        assert_eq!(
            depth(
                "while (DateTime.Now < deadline)\n{\n    Step();\n}",
                PerfFindingKind::UnstableClock
            ),
            1
        );
    }

    #[test]
    fn a_for_loop_does_not_count_its_initializer() {
        // The initializer runs once; the condition and the update run every
        // pass.
        assert_eq!(
            depth(
                "for (var start = DateTime.Now; index < count; index++)\n{\n    Step();\n}",
                PerfFindingKind::UnstableClock
            ),
            0
        );
    }

    #[test]
    fn a_linq_lambda_inside_a_loop_stacks() {
        assert_eq!(
            depth(
                "foreach (var order in orders)\n\
                 {\n\
                     var stamps = order.Lines.Select(l => DateTime.Now);\n\
                 }",
                PerfFindingKind::UnstableClock
            ),
            2
        );
    }

    #[test]
    fn a_query_expression_counts_as_a_loop() {
        assert_eq!(
            depth(
                "var stamps = from order in orders select DateTime.Now;",
                PerfFindingKind::UnstableClock
            ),
            1
        );
    }

    #[test]
    fn a_for_loop_counts_its_body() {
        assert_eq!(
            depth(
                "for (var index = 0; index < count; index++)\n{\n    var stamp = DateTime.Now;\n}",
                PerfFindingKind::UnstableClock
            ),
            1
        );
    }

    // ---- attribution

    #[test]
    fn a_finding_records_the_position_of_its_enclosing_member_name() {
        // The join key against `RawDecl` — it has to be the name node's
        // position, not the declaration's.
        let source =
            "class Sample\n{\n    void Run()\n    {\n        var stamp = DateTime.Now;\n    }\n}\n";
        let findings = scan(source);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].member, Some((3, 10)));
        assert_eq!(findings[0].line, 5);
    }

    #[test]
    fn a_finding_in_a_local_function_belongs_to_the_member_around_it() {
        let source = "class Sample\n{\n    void Run()\n    {\n        void Inner() { var stamp = DateTime.Now; }\n    }\n}\n";
        let findings = scan(source);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].member, Some((3, 10)));
    }

    #[test]
    fn each_member_gets_its_own_loop_depth() {
        let source = "class Sample\n\
                      {\n\
                          void First() { foreach (var x in xs) { var a = DateTime.Now; } }\n\
                          void Second() { var b = DateTime.Now; }\n\
                      }\n";
        let findings = scan(source);

        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].loop_depth, 1);
        assert_eq!(findings[1].loop_depth, 0);
    }

    #[test]
    fn a_finding_in_a_field_initializer_belongs_to_the_field() {
        let source = "class Sample\n{\n    private DateTime stamp = DateTime.Now;\n}\n";
        let findings = scan(source);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].member, Some((3, 22)));
    }

    #[test]
    fn a_lambda_carries_its_own_async_rather_than_the_methods() {
        // A synchronous callback inside an async method is its own function,
        // and `Thread.Sleep` there is not blocking an async continuation.
        assert!(
            kinds("Register(() => { Thread.Sleep(500); });")
                .iter()
                .all(|kind| *kind != PerfFindingKind::BlockingSleepInAsync)
        );
    }
}
