# Contributing to roe

This covers building, testing, and releasing roe. For install and usage
instructions, see [README.md](README.md).

## Building & testing

```
cargo test                 # unit + integration + snapshot tests
cargo clippy --all-targets
cargo run -- tests/fixtures/console_app          # all three analyses
cargo run -- dead-code tests/fixtures/console_app
cargo run -- dupes tests/fixtures/dupes_exact_clone
cargo run -- health tests/fixtures/health_metrics
```

Fixtures under `tests/fixtures/` are miniature solutions pinning the
false-positive kill list; they are parsed, never compiled or executed.
roe never runs the code it analyzes.

## Static analysis & code quality

Lint policy lives in the repo, not in the CI command line: `[lints]` in
`Cargo.toml` forbids `unsafe` and denies `unwrap` outside tests, and
`clippy.toml` grants the test exemption. So `cargo clippy` locally enforces
exactly what CI enforces.

Every check below runs as its own GitHub check on each pull request
([ci.yml](.github/workflows/ci.yml)). To reproduce them locally:

```
cargo install --locked cargo-deny cargo-llvm-cov cargo-machete cargo-mutants typos-cli

cargo deny check                            # advisories, licenses, bans, sources
cargo llvm-cov --summary-only               # coverage, including the CLI tests
cargo machete                               # unused dependencies
cargo mutants --in-diff <(git diff main...) # do the tests actually assert?
typos                                       # spelling, incl. user-facing errors
```

Configuration lives in `deny.toml` and `_typos.toml`. Clippy findings are also
uploaded as SARIF, so they show up as inline annotations on the pull request
rather than only in the job log.

Nothing is gated on a coverage number yet — the `Coverage` job publishes an
`lcov` artifact and a job summary so we can watch the trend first. The baseline
when these checks landed was 92% of lines and 91% of regions. `Mutants` is
likewise advisory (`continue-on-error`) until its baseline is clean; every other
check blocks.

## Commit conventions

Use [conventional commits](https://www.conventionalcommits.org/), scoped to
the command you touched, e.g. `fix(dead-code): Avoid matching variables with
the same name.`. Versioning follows semver — see `CLAUDE.md` for the full
code style guide.

## How detection works internally

### `roe dead-code`

1. **Discover** — parses `.sln` and SDK-style/legacy `.csproj` files
   (OutputType, ProjectReference, PackageReference, Compile Include/Remove,
   global usings), walks sources gitignore-aware, skips `bin/`, and harvests
   `obj/` for generated sources as reference-only inputs.
2. **Extract** — parses every `.cs` file in parallel with tree-sitter,
   collecting declarations (types, methods, properties, fields, events, …)
   and references (identifiers, member accesses, generic type arguments,
   attributes, `typeof`/`nameof`, `using static`/aliases).
3. **Resolve** — merges partial types and overloads into one symbol table;
   type references resolve with namespace/using scoping, member references by
   conservative name matching.
4. **Mark and sweep** — BFS from entry points over the reference graph. A
   member only lights up when its name is referenced from reachable code AND
   its containing type is reachable.
5. **Report** — unreachable symbols become findings; a file whose every
   declaration is dead is reported once as a dead file.

### `roe dupes`

1. **Tokenize** — parses every `.cs` file in parallel with tree-sitter and
   collects every leaf token (comments and preprocessor directives excluded).
   In `exact` mode (the default) each token keeps its own text; in `semantic`
   mode identifiers and numeric literals collapse to one placeholder per kind,
   so a renamed-but-structurally-identical copy still matches, while string
   literals, keywords, and punctuation always keep their exact text.
2. **Suffix array + LCP array** — the whole codebase becomes one dense token
   stream (a unique sentinel after each file keeps matches from crossing file
   boundaries), and a suffix array plus Kasai's LCP array find every maximal
   repeated run in it.
3. **Extract groups** — LCP intervals are turned into candidate duplicate
   groups, non-maximal submatches (a truncated prefix of a longer repeat
   reported elsewhere) are dropped, and the rest are filtered by
   `--min-tokens`, `--min-lines` (using the shortest span across a group's
   occurrences), and `--min-occurrences`.
4. **Report** — surviving groups are sorted by size (tokens, then occurrence
   count) so the most impactful duplication surfaces first.

### `roe health`

1. **Discover, extract, resolve** — the same first three stages as
   `dead-code`, reusing `resolve::build_symbols` and `graph::build_graph`.
   Then it diverges: there is no mark-and-sweep. Health flags issues in code
   regardless of whether it's reachable, since a method being used is what
   makes its complexity worth fixing.
2. **Measure** — per declaration, during the tree-sitter walk
   (`extract::walk`). `compute_member_complexity` returns cyclomatic
   complexity (McCabe's `1 + decision points`: one per `if`, loop, `catch`,
   `switch` section, ternary, and `&&`/`||`), cognitive complexity, and
   body line span. `??` is deliberately not a decision point — it is a
   defaulting idiom rather than a path a test covers, and counting it scored
   hand-rolled `with` methods at 14 while cognitive complexity scored them 0.
   `count_cognitive` implements Campbell's 2018 SonarSource
   formulation, where a flow breaker costs `1 + nesting` so deeply nested
   code scores worse than the same branches laid flat, while shapes the eye
   reads at a glance are forgiven — an `else if` chain stays flat and a run of
   `&&` counts once. Type size comes from `MemberBreakdown::record`, which
   buckets constructors, operators, and indexers into `methods` and refuses
   enum cases and `const` fields outright — neither carries behaviour, so
   neither can be part of the cohesion problem the check looks for. Signature
   size comes from `count_parameters`, which returns a `ParameterBreakdown`
   rather than a bare count: only *required* input parameters are compared
   against the limit, since a defaulted parameter, a `params` array, or an
   `out` parameter costs the caller nothing. The declared total is kept too —
   `member_names` needs it for the `/arity` overload suffix, which has to
   match the source signature.
3. **Threshold** — `collect_findings` emits a finding where a metric is
   strictly greater than its limit, so a value equal to the threshold is
   still clean. `HealthFinding::severity` is `metric / threshold`, which is
   what lets a cyclomatic complexity of 46 against a limit of 10 be ranked
   against a 242-line body against a limit of 40.
4. **Couple** — `coupling::fan_out` rolls the symbol-level reference graph up
   to type-level edges, dropping self-references. `find_cycles` runs Tarjan's
   SCC over that, keeping components of two or more. Because a strongly
   connected component is not itself a cycle, `shortest_cycle` then BFSes
   within each component to a path where every consecutive pair is a real
   edge, and the members not on that path ride along in `Cycle::others`
   rather than being spliced into the arrow chain. Nodes and neighbors are
   sorted at every stage, so the output is deterministic run to run.
5. **Baseline** (only with `--baseline`, or `health.baseline` in a config) —
   `baseline::apply` drops every finding whose `(kind, name)` pair is already
   recorded, and every cycle whose sorted member set is. It runs after inline
   suppressions and before `summarize`, so hidden findings inflate no count
   except `summary.baselined`. The line number is deliberately not part of
   the key — a baseline that went stale whenever a file grew would be
   worthless — and a matched finding whose metric rose above the recorded one
   survives anyway, since that is new debt in an old place. Entries matching
   nothing are stale and produce a warning, never a failure.
6. **Rank hotspots** (only with `--hotspots`) — `churn::analyze` walks git
   history with gix, diffing first parents only so a squashed pull request
   counts once, and discounts each commit on an exponential decay with a
   90-day half-life. `hotspot::rank` multiplies that by complexity density
   (cyclomatic over lines, so long-but-simple files don't crowd out
   short-but-gnarly ones) and normalizes both terms within the run.

Cycles are the one finding with no inline suppression — they span files, so
there's no single line to attach a comment to. A cycle touching a generated
file, or a test project under `--exclude-tests`, is dropped whole for the same
reason: a path with a hole in it would name edges that don't exist.

## Releasing

Releases are managed by [release-please](https://github.com/googleapis/release-please),
configured in `release-please-config.json` and `.release-please-manifest.json`.
Never hand-edit `version` in `Cargo.toml` — release-please owns it.

Everything runs in one workflow, [release.yml](.github/workflows/release.yml),
triggered on every push to `main`:

1. The `release-please` job reads conventional commits since the last release
   and keeps a "Release PR" up to date with the next `Cargo.toml` version
   bump and generated `CHANGELOG.md` entry (`fix:` → patch, `feat:` → minor,
   `!`/`BREAKING CHANGE` → major).
2. Merge that PR when you want to ship. release-please tags the merge commit
   (`vX.Y.Z`) and creates the GitHub Release with generated notes.
3. Because that tagging happens inside the same workflow run, the remaining
   jobs (`test`, `build`, `publish-assets`, `npm-publish`, `nuget-publish`)
   run right after, gated on `release_created`: they build binaries for every
   platform, attach them plus a `SHA256SUMS` file to the release
   release-please just created, and publish `roe` to NuGet and `roe-cli` to
   npm. On a normal push with no release pending, those jobs are skipped and
   only `release-please` runs.
