# Contributing to roe

This covers building, testing, and releasing roe. For install and usage
instructions, see [README.md](README.md).

## Building & testing

```
cargo test                 # unit + integration + snapshot tests
cargo clippy --all-targets
cargo run -- dead-code tests/fixtures/console_app
cargo run -- dupes tests/fixtures/dupes_exact_clone
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
