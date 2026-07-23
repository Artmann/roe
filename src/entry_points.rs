use lasso::ThreadedRodeo;

use crate::extract::FileFacts;
use crate::model::{MemberKind, Modifiers, SymbolKind, Workspace};
use crate::resolve::{Resolution, SymbolFlags};

/// Attributes that never signal framework consumption — everything else on a
/// declaration roots it (denylist fails safe: an unknown custom attribute is
/// assumed to be a framework marker).
const INERT_ATTRIBUTES: &[&str] = &[
    "Obsolete",
    "DebuggerDisplay",
    "DebuggerStepThrough",
    "DebuggerBrowsable",
    "DebuggerNonUserCode",
    "EditorBrowsable",
    "ExcludeFromCodeCoverage",
    "CompilerGenerated",
];

/// Methods invoked reflectively by hosting conventions.
const MAGIC_METHOD_NAMES: &[&str] = &["Main", "ConfigureServices", "Configure"];

/// Compiler-synthesized polyfills for language features on older TFMs:
/// referenced only by the compiler (emitted into IL), never from source, so
/// reachability analysis can't see them. A dead-file report on them is
/// always a false positive.
const COMPILER_POLYFILL_FQNS: &[&str] = &[
    "System.Runtime.CompilerServices.IsExternalInit",
    "System.Runtime.CompilerServices.RequiredMemberAttribute",
    "System.Runtime.CompilerServices.CompilerFeatureRequiredAttribute",
    "System.Runtime.CompilerServices.ModuleInitializerAttribute",
    "System.Diagnostics.CodeAnalysis.AllowNullAttribute",
    "System.Diagnostics.CodeAnalysis.DisallowNullAttribute",
    "System.Diagnostics.CodeAnalysis.DoesNotReturnAttribute",
    "System.Diagnostics.CodeAnalysis.DoesNotReturnIfAttribute",
    "System.Diagnostics.CodeAnalysis.MaybeNullAttribute",
    "System.Diagnostics.CodeAnalysis.MaybeNullWhenAttribute",
    "System.Diagnostics.CodeAnalysis.MemberNotNullAttribute",
    "System.Diagnostics.CodeAnalysis.MemberNotNullWhenAttribute",
    "System.Diagnostics.CodeAnalysis.NotNullAttribute",
    "System.Diagnostics.CodeAnalysis.NotNullIfNotNullAttribute",
    "System.Diagnostics.CodeAnalysis.NotNullWhenAttribute",
];

/// Detect entry points and set ROOT (and TEST_ROOT) flags. Returns notes to
/// surface in the report (e.g. library mode).
pub fn mark_roots(
    resolution: &mut Resolution,
    workspace: &Workspace,
    facts: &[FileFacts],
    manual_roots: &[String],
    library_projects: &[String],
    rodeo: &ThreadedRodeo,
) -> Vec<String> {
    let mut notes = Vec::new();

    // Library mode: nothing executable anywhere — the public API surface is
    // the consumer contract, so only internal/private dead code is
    // meaningful. Test projects do NOT disable it (a lib + its tests is
    // still a library package). The Main/top-level-statements heuristic
    // applies only when no project files exist (bare directories) — when
    // csproj metadata is present, trust OutputType: a benchmark harness's
    // Main must not expose a whole library's public API to analysis.
    let main_name = rodeo.get_or_intern("Main");
    let no_project_files = workspace.projects.iter().all(|p| p.csproj_path.is_none());
    let has_executable = workspace
        .projects
        .iter()
        .any(|p| p.is_executable() && !p.is_auxiliary())
        || (no_project_files
            && (facts.iter().any(|f| f.has_top_level_statements)
                || resolution.symbols.iter().any(|s| {
                    s.kind == SymbolKind::Member(MemberKind::Method)
                        && s.name == main_name
                        && s.modifiers.contains(Modifiers::STATIC)
                })));
    let library_mode = !has_executable;
    if library_mode {
        notes.push(
            "library mode: no executable or test project found — public API is treated as used; \
             only internal/private symbols are analyzed"
                .to_string(),
        );
    }

    let inert: Vec<lasso::Spur> = INERT_ATTRIBUTES
        .iter()
        .flat_map(|name| {
            [
                rodeo.get_or_intern(name),
                rodeo.get_or_intern(format!("{name}Attribute")),
            ]
        })
        .collect();
    let magic_methods: Vec<lasso::Spur> = MAGIC_METHOD_NAMES
        .iter()
        .map(|name| rodeo.get_or_intern(name))
        .collect();
    let startup = rodeo.get_or_intern("Startup");
    let compiler_polyfill_fqns: Vec<lasso::Spur> = COMPILER_POLYFILL_FQNS
        .iter()
        .map(|fqn| rodeo.get_or_intern(fqn))
        .collect();

    for index in 0..resolution.symbols.len() {
        let symbol = &resolution.symbols[index];
        let file = &workspace.files[symbol.file.index()];
        let project = file.project.map(|p| &workspace.projects[p.index()]);
        let in_test_project = project.is_some_and(|p| p.is_test());

        let mut is_root = false;

        match symbol.kind {
            // File roots own top-level statements and assembly attributes.
            SymbolKind::FileRoot => is_root = true,

            SymbolKind::Member(MemberKind::Method) => {
                // static Main — the classic entry point. Hosting-convention
                // methods (Startup.ConfigureServices/Configure) are invoked
                // reflectively.
                if magic_methods.contains(&symbol.name) {
                    let is_main = rodeo.resolve(&symbol.name) == "Main";
                    if !is_main || symbol.modifiers.contains(Modifiers::STATIC) {
                        is_root = true;
                    }
                }
                // Controller actions: public methods on controller types.
                if let Some(parent) = symbol.parent
                    && is_controller(&resolution.symbols[parent.index()], rodeo)
                    && symbol.modifiers.contains(Modifiers::PUBLIC)
                {
                    is_root = true;
                }
            }

            SymbolKind::Type(_) => {
                if is_controller(symbol, rodeo)
                    || symbol.name == startup
                    || symbol
                        .fqn
                        .is_some_and(|fqn| compiler_polyfill_fqns.contains(&fqn))
                {
                    is_root = true;
                }
            }

            SymbolKind::Member(_) => {}
        }

        // Any non-inert attribute signals framework consumption ([Fact],
        // [HttpGet], [JsonProperty], [UsedImplicitly], custom markers...).
        if !symbol.attributes.is_empty() && symbol.attributes.iter().any(|a| !inert.contains(a)) {
            is_root = true;
        }

        // Public surface is the contract for shipping code: everything in
        // library mode (except test projects — their publics are not an
        // external contract), packable projects always (they publish to
        // NuGet regardless of what else lives in the workspace), and any
        // project named via --library/libraryProjects (consumed outside the
        // workspace, e.g. by a Unity project referencing the built DLL).
        let public_is_contract = (library_mode && !in_test_project)
            || project.is_some_and(|p| p.is_packable || library_projects.contains(&p.name));
        if public_is_contract && symbol.is_public_surface() && parents_are_public(resolution, index)
        {
            is_root = true;
        }

        if is_root {
            let flags = &mut resolution.symbols[index].flags;
            *flags |= SymbolFlags::ROOT;
            if in_test_project {
                *flags |= SymbolFlags::TEST_ROOT;
            }
        }
    }

    // Manual roots: match fully-qualified display names.
    for manual in manual_roots {
        let mut matched = false;
        for index in 0..resolution.symbols.len() {
            let id = resolution.symbols[index].id;
            if resolution.display_name(id, rodeo) == *manual {
                resolution.symbols[index].flags |= SymbolFlags::ROOT;
                matched = true;
            }
        }
        if !matched {
            notes.push(format!("--root {manual}: no matching symbol found"));
        }
    }

    notes
}

fn is_controller(symbol: &crate::resolve::Symbol, rodeo: &ThreadedRodeo) -> bool {
    if !symbol.kind.is_type() {
        return false;
    }
    if rodeo.resolve(&symbol.name).ends_with("Controller") {
        return true;
    }
    symbol.base_names.iter().any(|base| {
        base.last()
            .is_some_and(|last| matches!(rodeo.resolve(last), "Controller" | "ControllerBase"))
    })
}

fn parents_are_public(resolution: &Resolution, index: usize) -> bool {
    let mut current = resolution.symbols[index].parent;
    while let Some(parent) = current {
        let symbol = &resolution.symbols[parent.index()];
        if symbol.kind.is_type() && !symbol.is_public_surface() {
            return false;
        }
        current = symbol.parent;
    }
    true
}
