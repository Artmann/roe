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

1. Every push to `main` runs the
   [release-please workflow](.github/workflows/release-please.yml), which
   reads conventional commits since the last release and keeps a "Release PR"
   up to date with the next `Cargo.toml` version bump and generated
   `CHANGELOG.md` entry (`fix:` → patch, `feat:` → minor, `!`/`BREAKING
   CHANGE` → major).
2. Merge that PR when you want to ship. release-please tags the resulting
   commit (`vX.Y.Z`) but does not create a GitHub Release itself
   (`skip-github-release: true`) — that's left to the workflow below so
   binaries and package publishes stay attached to the same release.
3. The tag push triggers the [release workflow](.github/workflows/release.yml),
   which builds binaries for every platform, creates the GitHub Release, and
   publishes `roe` to NuGet and `roe-cli` to npm. The npm and NuGet versions
   are stamped from the tag in CI.

The release-please workflow authenticates with a `RELEASE_PLEASE_TOKEN`
repository secret (a PAT or GitHub App token) rather than the default
`GITHUB_TOKEN` — pushes made with `GITHUB_TOKEN` don't trigger other
workflows, so the release tag would never fire `release.yml`.

The first release (`1.0.0`) is pinned via `release-as` in
`release-please-config.json`, since there's no prior tag for release-please
to compute a bump from. Remove that `release-as` override once the `1.0.0`
release PR has merged — after that, versions should always come from
conventional commits.

npm publishing authenticates with [trusted publishing](https://docs.npmjs.com/trusted-publishers)
(OIDC, no token). npm can't create a package via OIDC, so bootstrapping a
fresh registry requires publishing a `0.0.0` placeholder once with a granular
token (`npm publish` from `packaging/npm/roe-cli`), then configuring the
trusted publisher on the package's npmjs.com settings page: repository
`Artmann/roe`, workflow `release.yml`. NuGet publishing uses the
`NUGET_API_KEY` repository secret.
