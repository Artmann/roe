<div align="center">

# roe

Codebase intelligence for C#. Finds dead code — unused types, members, and
files — duplicated code, and complexity/coupling hotspots, so you can delete
what's unused, de-duplicate what's copy-pasted, and clean up what's hard to
work with. Static analysis only; roe never runs the code it analyzes.

[![npm](https://img.shields.io/npm/v/roe-cli)](https://www.npmjs.com/package/roe-cli)
[![NuGet](https://img.shields.io/nuget/v/roe)](https://www.nuget.org/packages/roe/)
[![CI](https://github.com/Artmann/roe/actions/workflows/ci.yml/badge.svg)](https://github.com/Artmann/roe/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](./LICENSE)

Run `roe` with no command and it does all three at once.

[Install](#install) · [Commands](#commands) · [Releases](https://github.com/Artmann/roe/releases) · [Contributing](CONTRIBUTING.md)

</div>

---

```
$ roe path/to/solution

── dead-code ───────────────────────────────────────────────

src/App/Services/LegacyExporter.cs (App)
      1:1  dead file      every declaration in this file is unused

src/App/Billing/InvoiceService.cs (App)
    88:17  unused member  App.Billing.InvoiceService.RecalculateAll (private method)

found 1 dead file · 0 unused types · 1 unused member — 3 project(s), 214 file(s), 2610 symbol(s) scanned in 74 ms

── dupes ───────────────────────────────────────────────────

2 occurrences (103 tokens, 23 lines)
  src/App/Billing/PaymentService.cs:15:5-37:2
  src/App/Shipping/ShippingService.cs:15:5-37:2

found 1 duplicate group · 46 duplicated lines — 3 project(s), 214 file(s) scanned in 31 ms

── health ──────────────────────────────────────────────────

✓ no health issues found · 3 project(s), 214 file(s) scanned in 68 ms
```

## Install

### .NET SDK (8.0 or later) — recommended

```
dotnet tool install --global roe
```

Or run it one-shot without installing (.NET 10 SDK or later):

```
dnx roe .
```

### npm

```
npm install --global roe-cli
```

The npm package is called `roe-cli` (the name `roe` was already taken on
npm), but the command it installs is still `roe`. To run it one-shot without
installing:

```
npx roe-cli .
```

### Prebuilt binaries

Prebuilt binaries for Linux (x64/arm64), macOS (x64/arm64), and Windows
(x64) are attached to [GitHub Releases](https://github.com/Artmann/roe/releases),
alongside a `SHA256SUMS` file for verification. macOS quarantines binaries
downloaded with a browser; clear it before running with:

```
xattr -d com.apple.quarantine ./roe
```

## Commands

roe has three analyses: `dead-code`, `dupes`, and `health`. All accept a path
to a directory, a `.sln` file, or a `.csproj` file (default: the current
directory), and all exit `0` when clean, `1` when they have findings to
report, and `2` on error.

### `roe` / `roe check`

Runs all three analyses over the same path and prints each report under its own
section header. This is what you get when you run `roe` with no command, and
it's the one-line answer for CI:

```
roe                             # all three, current directory
roe [PATH]                      # directory, .sln, or .csproj
roe check [PATH]                # identical, spelled out
roe --format json               # one document with deadCode, dupes, health
roe --config PATH               # use this roe.json/roe.yaml instead of auto-discovery
```

It exits `1` if *any* of the three has findings. Per-analysis flags aren't
accepted here — put them in a [config file](#suppressing-findings), or run the
individual command when you need to tune one analysis.

### `roe dead-code`

Finds unused types, members, and files. roe builds a map of what references
what across your solution, starting from known entry points (see below), and
reports anything that's unreachable.

```
roe dead-code [PATH]            # directory, .sln, or .csproj (default: cwd)
roe dead-code --format json     # machine-readable, stable v1 schema
roe dead-code --aggressive      # also flag enum members + public auto-properties
roe dead-code --root App.Foo    # treat a symbol as an entry point
roe dead-code --library MyLib   # always treat this project's public API as used
roe dead-code --config PATH     # use this roe.json/roe.yaml instead of auto-discovery
```

**What counts as an entry point** (i.e. never flagged, and the starting
points for reachability):

- `static Main` and top-level-statement files
- ASP.NET controllers (`*Controller` / `ControllerBase`) and their actions,
  `Startup` conventions
- Test methods and fixtures (`[Fact]`, `[Theory]`, `[Test]`, `[TestMethod]`, …)
- Any declaration carrying a non-inert attribute (`[HttpGet]`,
  `[JsonProperty]`, `[UsedImplicitly]`, custom markers — everything except
  `[Obsolete]`-class metadata)
- Reflection-scan contracts: implementations of a type used as a lone generic
  argument in a call (`GetExports<IImageProvider>()`, `AddHostedService<W>()`)
- Public API surface of NuGet-packable projects, of every non-test project
  when the workspace has no executable ("library mode"), and of any project
  named with `--library`/`libraryProjects` (a project consumed outside the
  workspace, e.g. a Unity project referencing the built DLL, regardless of
  what executables live alongside it)
- Declarations in generated files (`*.g.cs`, `*.Designer.cs`,
  `<auto-generated>` headers, `obj/`) — reference-only, never flagged

**What's never flagged individually**, to keep false positives rare: roe is
tuned so that when in doubt, code is marked used — a missed finding is
cheap, a wrong one is a bug. That means constructors, operators, indexers,
and finalizers (invoked structurally); `override` members and explicit
interface implementations (invoked via dispatch); members satisfying an
in-source interface; visible members of types implementing external
`I`-prefixed interfaces; and extension methods. Public auto-properties and
enum members are treated as serializer fodder and skipped by default — opt
in with `--aggressive`.

### `roe dupes`

Finds duplicated — copy-pasted, and optionally copy-pasted-and-renamed —
blocks of code across your solution.

```
$ roe dupes path/to/solution

2 occurrences (105 tokens, 24 lines)
  src/App/Billing/PaymentService.cs:40:5-63:6
  src/App/Shipping/ShippingService.cs:38:5-61:6

found 1 duplicate group · 48 duplicated lines — 1 project(s), 214 file(s) scanned in 61 ms
```

```
roe dupes [PATH]               # directory, .sln, or .csproj (default: cwd)
roe dupes --format json        # machine-readable, stable v1 schema
roe dupes --mode semantic      # also catch copy-paste-and-rename clones
roe dupes --no-code            # hide the duplicated source under each group
roe dupes --min-tokens 50      # shortest match to report (default: 50)
roe dupes --min-lines 5        # shortest line-span to report (default: 5)
roe dupes --min-occurrences 2  # smallest group size to report (default: 2)
roe dupes --config PATH        # use this roe.json/roe.yaml instead of auto-discovery
```

By default, `roe dupes` only matches code that's identical token-for-token.
Pass `--mode semantic` to also catch blocks that were copied and then had
their variable names or literal values changed, but are otherwise
structurally the same.

### `roe health`

Flags complexity, size, and coupling problems: methods that branch or nest
too heavily, files and types that have grown too large, and circular
dependencies between types. With `--hotspots` it also reads git history to
rank the files where complexity and change frequency overlap. Unlike
`dead-code` and `dupes`, findings here are about code that's used but hard to
work with, not code to delete.

```
$ roe health path/to/solution

src/App/Billing/InvoiceService.cs (App)
    42:12  App.Billing.InvoiceService.Reconcile
           cyclomatic 46/10 · cognitive 86/15 · 242/40 lines
    18:14  App.Billing.InvoiceService
           34/20 members (19 properties, 15 methods)

src/App/Orders/OrderValidator.cs (App)
   112:17  App.Orders.OrderValidator.Validate
           cyclomatic 14/10

found 4 issues across 3 locations in 2 files — 3 project(s), 214 file(s), 2610 symbol(s) scanned in 68 ms
  2 complex methods · 1 hard-to-follow method · 1 long method · 0 over-parameterized methods · 0 large files · 1 large type · 0 circular dependencies
```

Each entry is one declaration, listing every check it tripped — a method
that's too long *and* too branchy is one thing to fix, so it's one line
rather than three. Every measurement reads `actual/limit`. The headline
counts both, since they differ: 4 issues, but only 3 places to go.

```
roe health [PATH]                  # directory, .sln, or .csproj (default: cwd)
roe health --format json           # machine-readable, stable v1 schema
roe health --max-complexity 10     # cyclomatic complexity per method (default: 10)
roe health --max-cognitive 15      # cognitive complexity per method (default: 15)
roe health --max-method-lines 40   # lines per method body (default: 40)
roe health --max-parameters 5      # parameters per method (default: 5)
roe health --max-file-lines 750    # lines per file (default: 750)
roe health --max-type-members 20   # declared members per type (default: 20)
roe health --exclude-tests         # skip test projects (default: off)
roe health --sort path             # order by file path instead of by severity
roe health --limit 20              # print at most 20 entries (default: all)
roe health --hotspots              # also rank complex, frequently changed files
roe health --hotspots --top 20     # how many to list (default: 10)
roe health --config PATH           # use this roe.json/roe.yaml instead of auto-discovery
```

Entries are ordered worst-first by default — how many times over its limit
each measurement sits, which is comparable across checks in a way the raw
numbers are not. `--sort path` gives you stable, position-ordered output
instead. `--limit` caps the human report only; `--format json` always
contains everything.

`--exclude-tests` skips test projects entirely. Long arrange/act/assert
methods and multi-case fixtures are normal there, and a test class is
supposed to have one method per case. A circular dependency that touches a
test project is dropped whole, since a path with holes in it would name edges
that aren't there. It's off by default so nothing goes unreported without you
asking.

Declarations in generated files are never flagged, and the same `ignore`
globs from a config file apply here too.

#### What it checks, and what to do about it

| Check | What it measures | Typical fix |
| --- | --- | --- |
| **high complexity** | Cyclomatic complexity — one point per `if`, loop, `catch`, `case`, ternary, and `&&`/`\|\|`/`??`, plus one baseline. Counts the independent paths through a method, which is roughly the number of tests needed to cover it. | Extract the branches into named methods; replace flag parameters and `switch` ladders with polymorphism or a lookup table; use guard clauses to flatten nesting. |
| **hard to follow** | Cognitive complexity — the same control-flow structures, but each one costs more the deeper it is nested, and shapes the eye reads at a glance are forgiven: an `else if` chain stays flat, a whole `switch` costs 1 no matter how many cases it has, and a run of `&&` counts once. Estimates how hard a method is to *understand*, where cyclomatic estimates how hard it is to *test*. | Flatten first — early returns and guard clauses are worth more here than extraction, since removing one level of nesting discounts everything inside it. Then extract the deepest block into a named method, which resets its nesting to zero. |
| **long method** | Lines spanned by a method body. | Extract Method along the seams — usually the comment-delimited "sections" of the body are the extractions waiting to happen. |
| **too many parameters** | Parameters in a method's declaration. | Introduce a Parameter Object: bundle the arguments that always travel together into a `record`, so `Send(string to, string cc, string subject, string body, bool html, int retries)` becomes `Send(EmailMessage message)`. If several belong to the same existing object, Preserve Whole Object and pass that instead. |
| **large file** | Total lines in a `.cs` file. | Usually a symptom rather than a cause — split the file per type, or move nested helper types into their own files. |
| **large type** | Declared members on a type — methods, properties, fields, and events. An enum's cases don't count, since having many of them is the point of an enum, and neither do `const` fields, which are names for literals rather than state or behaviour. `static readonly` fields do count. | Extract Class: find the cluster of fields and the methods that touch only those fields, and move them out together. A type doing two jobs usually shows up as two such clusters. |
| **circular dependency** | Types that reference each other, directly or through a longer chain. | Break the loop by depending on an abstraction: extract an interface that one side owns and the other implements (Dependency Inversion), or move the shared concept both sides need into a third type neither one owns. |

A `large type` entry carries the breakdown of what its members actually are —
`34/20 members (19 properties, 15 methods)`. Thirty auto-properties is a data
holder and thirty methods is a god class; they trip the same threshold and
call for entirely different responses, so roe reports the split rather than
guessing which one you have.

A `circular dependency` prints a real cycle: consecutive names are joined by
an actual reference, and the last one references the first. Types caught in
the same tangle but not on that particular loop are listed separately, since
putting them in the arrow chain would claim references that may not exist:

```
circular dependencies
  App.Orders.Order → App.Orders.Cart → App.Orders.Order
    src/App/Orders/Order.cs 12:14
    src/App/Orders/Cart.cs 9:14
    + 2 more types in this cycle: App.Orders.Line, App.Orders.Tax
```

Where a file declares two overloads that both trip a check, the entries carry
a Roslyn-style arity suffix (`TryBeginActivation/0`, `TryBeginActivation/1`)
so they can be told apart. The suffix appears only where there's an actual
collision.

Note that complexity, length, and member-count thresholds are conventions,
not laws — the defaults sit near the values common linters use,
`--max-complexity 10` traces back to McCabe's original 1976 paper, and the
cognitive complexity rules follow Campbell's 2018 SonarSource formulation.
Tune them to your codebase rather than treating the defaults as a target.
Circular dependencies are the exception: they're reported regardless of any
threshold, because a cycle is a structural fact rather than a judgement
call.

#### Hotspots

`--hotspots` answers a different question from the checks above. Those ask
"what is wrong here"; hotspots ask "of everything that's wrong, what should I
fix first". It ranks files by the product of how dense their complexity is
and how often they actually change, on the theory that complexity you never
touch costs you nothing.

```
$ roe health path/to/solution --hotspots

hotspots (complexity × churn over 1284 commit(s))
    100  src/App/Billing/InvoiceService.cs  complexity 96 over 412 line(s), 18.4 weighted commit(s)
     41  src/App/Orders/OrderValidator.cs   complexity 44 over 260 line(s), 9.1 weighted commit(s)
      7  src/App/Shipping/RateTable.cs      complexity 31 over 890 line(s), 6.2 weighted commit(s)
```

The score is:

```
normalized_churn      = weighted_commits / max_weighted_commits   (0..1)
normalized_complexity = complexity_density / max_density          (0..1)
score                 = normalized_churn × normalized_complexity × 100
```

`weighted_commits` is a recency-weighted commit count: each commit touching
the file is discounted on an exponential decay with a **90-day half-life**, so
a change from today counts 1.0, one from three months ago 0.5, and one from a
year ago about 0.06. A file rewritten fifty times three years ago and left
alone since is settled, not volatile. `complexity_density` is cyclomatic
complexity divided by lines, which stops long-but-simple files from crowding
out short-but-gnarly ones.

Both terms are normalized **within the run**, so the riskiest file always
scores close to 100 and a score only means something next to the other scores
beside it. A 100 is not a grade — it just means "start here". Files with no
churn or no complexity score zero and are left out entirely.

Three things worth knowing:

- **Hotspots never affect the exit code.** Every codebase has a riskiest
  file, so failing a build over the existence of a ranking would make the
  check useless as a CI gate. Only the threshold checks above exit `1`.
- **It needs real git history.** Running outside a git working tree is an
  error rather than a silent zero. In CI, `actions/checkout` defaults to a
  shallow clone, which truncates every score — set `fetch-depth: 0`. roe warns
  when it detects one.
- **Merges are traversed but not counted**, so a squashed pull request is one
  change rather than one per branch commit.

The idea is Adam Tornhill's, from *Your Code as a Crime Scene*; the scoring
formula here matches [Fallow's](https://docs.fallow.tools/cli/health#hotspot-score-formula).

## Suppressing findings

Two mechanisms let you mark a finding as a false positive.

**Inline comments**, eslint-style, checked after the first `//` on a line (so
`///` doc-comments match too):

```csharp
// roe-ignore-next-line unused-type
internal class LegacyHelper { }

public void DoWork()
{
    var unused = 1; // roe-ignore-line unused-member
}
```

`roe-ignore-next-line` suppresses a finding on the line below the comment;
`roe-ignore-line` suppresses one on the same line. Both take an optional
comma-separated rule list — the same names used in JSON output — and omitting
it suppresses any finding kind on that line:

| Command | Rule names |
| --- | --- |
| `roe dead-code` | `unused-type`, `unused-member`, `unused-file` |
| `roe health` | `high-complexity`, `high-cognitive-complexity`, `long-method`, `too-many-parameters`, `large-file`, `large-type` |

Because a dead file's finding and a large file's finding are both pinned at
line 1, an `unused-file` or `large-file` marker (or a bare marker) suppresses
them from anywhere in the file.

Circular dependencies can't be suppressed inline — a cycle spans several
types in several files, so there's no one line for the marker to sit on. Use
an `ignore` glob instead, the same as for `roe dupes`.

**A config file** — `roe.json`, `roe.yaml`, or `roe.yml` — resolved by
walking up from the analysis root to the nearest directory containing one
(like `.eslintrc`/`tsconfig.json`), or pointed to explicitly with `--config`:

```json
{
  "aggressive": true,
  "roots": ["MyApp.Program.Main"],
  "libraryProjects": ["MyLib"],
  "ignore": ["Migrations/**", "Generated/", "**/*.designer.cs"],
  "health": {
    "maxComplexity": 15,
    "maxCognitive": 20,
    "maxMethodLines": 60,
    "maxParameters": 6,
    "maxFileLines": 750,
    "maxTypeMembers": 25,
    "excludeTests": true
  }
}
```

`ignore` is a list of glob patterns, resolved relative to the config file's
own directory, that drop every finding in a matching file (a trailing `/`
matches the whole directory, so `"Generated/"` needs no `**`). `aggressive`,
`roots`, and `libraryProjects` set defaults for the matching CLI flags: an
explicit `--aggressive`, `--root`, or `--library` always wins, otherwise the
config's value applies, otherwise the built-in default (`false` / no extra
roots / no extra library projects). The same `ignore` list also applies to
`roe dupes`, since a duplicate spans multiple files and doesn't map cleanly
onto a single-line inline suppression comment.

The `health` block does the same for `roe health`'s thresholds, so a CI
invocation doesn't have to repeat six flags — an explicit flag wins, then the
config value, then the built-in default. Every field is optional, and an
unrecognised one is an error rather than a silent no-op, so a typo can't leave
you thinking a limit is in force when it isn't.

## Known limitations

- Name-based resolution conflates same-named symbols — dead code hiding
  behind a popular name is missed (by design; the safe direction).
- String-based reflection (`Activator.CreateInstance("…")`,
  `Type.GetType("…")`, config-wired type names) is invisible and can cause
  false positives; the attribute-root and scan-target heuristics absorb the
  common cases.
- MSBuild `Condition` attributes are not evaluated (all branches included),
  `Directory.Build.props` is not read, `InternalsVisibleTo` is not modeled.
- Razor/`.cshtml` markup is not parsed — build first so `obj/` generated
  sources stand in for it.
- Compiler-synthesized calls (e.g. range/index polyfills) are invisible.
- `roe dupes` does not de-duplicate overlapping/nested occurrences of the same
  pattern repeated within a single long method — each is reported as-is.

## Contributing

Want to build roe from source, run its test suite, or cut a release? See
[CONTRIBUTING.md](CONTRIBUTING.md).
