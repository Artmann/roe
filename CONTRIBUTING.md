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

1. Bump `version` in `Cargo.toml` and commit (`chore(release): v0.2.0`).
2. Tag and push: `git tag v0.2.0 && git push origin main v0.2.0`.
3. The [release workflow](.github/workflows/release.yml) builds binaries for
   every platform, creates the GitHub Release, and publishes `roe` to NuGet
   and `roe-cli` to npm. The npm and NuGet versions are stamped from the tag
   in CI, so `Cargo.toml` is the only place a version is ever bumped.

npm publishing authenticates with [trusted publishing](https://docs.npmjs.com/trusted-publishers)
(OIDC, no token). npm can't create a package via OIDC, so bootstrapping a
fresh registry requires publishing a `0.0.0` placeholder once with a granular
token (`npm publish` from `packaging/npm/roe-cli`), then configuring the
trusted publisher on the package's npmjs.com settings page: repository
`Artmann/roe`, workflow `release.yml`. NuGet publishing uses the
`NUGET_API_KEY` repository secret.
