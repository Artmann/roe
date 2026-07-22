use std::path::PathBuf;

use rayon::prelude::*;
use rustc_hash::FxHashMap;
use tree_sitter::Node;

use crate::cli::DupeMode;
use crate::extract::make_parser;
use crate::model::SourceFile;

/// One leaf token straight off the tree-sitter CST: punctuation/keywords are
/// anonymous nodes whose `kind()` string is the text itself; `identifier`,
/// numeric/boolean/null literals, and the content/escape children of string
/// and char literals are named leaves. Comments and preprocessor directives
/// are `is_extra()` and never reach this struct.
struct RawToken {
    kind: &'static str,
    text: Box<str>,
    start_line: u32,
    start_column: u32,
    end_line: u32,
    end_column: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenPosition {
    pub file_index: u32,
    pub start_line: u32,
    pub start_column: u32,
    pub end_line: u32,
    pub end_column: u32,
}

/// The suffix-array input: a dense `u32` token stream across every file,
/// each file's real tokens followed by a sentinel id unique to that file so
/// no repeated run can span two files. `positions[i]` describes `ids[i]`;
/// sentinel slots carry a zeroed placeholder position that is never read
/// (a `min_tokens`-sized match can never include a sentinel, since it's
/// globally unique and so can never recur).
pub struct Corpus {
    pub ids: Vec<u32>,
    pub positions: Vec<TokenPosition>,
    pub files: Vec<PathBuf>,
}

/// Tokenizes every file in parallel (one tree-sitter parser per rayon
/// worker, mirroring `extract::extract_all`), then serially interns tokens
/// into a dense `u32` alphabet in file order for determinism.
pub fn tokenize_all(files: &[SourceFile], mode: DupeMode) -> Corpus {
    let per_file: Vec<Vec<RawToken>> = files
        .par_iter()
        .map_init(make_parser, tokenize_file)
        .collect();

    let mut interner: FxHashMap<(&'static str, Option<Box<str>>), u32> = FxHashMap::default();
    let mut ids = Vec::new();
    let mut positions = Vec::new();

    for (file_index, tokens) in per_file.into_iter().enumerate() {
        let file_index = file_index as u32;

        for token in tokens {
            let key = symbol_key(token.kind, &token.text, mode);
            let id = match interner.get(&key) {
                Some(&id) => id,
                None => {
                    let id = interner.len() as u32;
                    assert!(
                        id < u32::MAX - files.len() as u32,
                        "token alphabet exhausted the u32 id space reserved for file sentinels"
                    );
                    interner.insert(key, id);
                    id
                }
            };
            ids.push(id);
            positions.push(TokenPosition {
                file_index,
                start_line: token.start_line,
                start_column: token.start_column,
                end_line: token.end_line,
                end_column: token.end_column,
            });
        }

        ids.push(u32::MAX - file_index);
        positions.push(TokenPosition {
            file_index,
            start_line: 0,
            start_column: 0,
            end_line: 0,
            end_column: 0,
        });
    }

    let file_paths = files.iter().map(|file| file.path.clone()).collect();

    Corpus {
        ids,
        positions,
        files: file_paths,
    }
}

/// In `Exact` mode every token keeps its own text. In `Semantic` mode,
/// identifiers and numeric literals collapse to one shared placeholder id
/// per kind (renamed-but-structurally-identical code still matches);
/// keywords, punctuation, and string/char/bool/null literals always keep
/// their exact text in both modes.
fn symbol_key(kind: &'static str, text: &str, mode: DupeMode) -> (&'static str, Option<Box<str>>) {
    let collapse = mode == DupeMode::Semantic
        && matches!(kind, "identifier" | "integer_literal" | "real_literal");

    if collapse {
        (kind, None)
    } else {
        (kind, Some(text.into()))
    }
}

fn tokenize_file(parser: &mut tree_sitter::Parser, file: &SourceFile) -> Vec<RawToken> {
    let Ok(source) = std::fs::read(&file.path) else {
        return Vec::new();
    };
    let Some(tree) = parser.parse(&source, None) else {
        return Vec::new();
    };

    let mut tokens = Vec::new();
    collect_leaves(tree.root_node(), &source, &mut tokens);
    tokens
}

fn collect_leaves(node: Node, source: &[u8], tokens: &mut Vec<RawToken>) {
    // Skip the whole subtree for extra/missing nodes (comments, preprocessor
    // directives, error-recovery phantoms) rather than only checking at leaf
    // level — that way a future grammar version giving one of these node
    // kinds children can't leak "real" tokens out of a disabled region.
    if node.is_extra() || node.is_missing() {
        return;
    }

    if node.child_count() == 0 {
        tokens.push(to_token(node, source));
        return;
    }

    for child in node.children(&mut node.walk()) {
        collect_leaves(child, source, tokens);
    }
}

fn to_token(node: Node, source: &[u8]) -> RawToken {
    let start = node.start_position();
    let end = node.end_position();

    RawToken {
        kind: node.kind(),
        text: node.utf8_text(source).unwrap_or("").into(),
        start_line: start.row as u32 + 1,
        start_column: start.column as u32 + 1,
        end_line: end.row as u32 + 1,
        end_column: end.column as u32 + 1,
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;
    use crate::model::FileId;

    fn write_source(dir: &std::path::Path, name: &str, contents: &str) -> SourceFile {
        let path = dir.join(name);
        let mut handle = std::fs::File::create(&path).expect("create fixture file");
        handle
            .write_all(contents.as_bytes())
            .expect("write fixture file");

        SourceFile {
            id: FileId(0),
            path,
            project: None,
            is_generated: false,
        }
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("roe-tokenize-test-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn comments_and_preprocessor_directives_are_excluded() {
        let dir = temp_dir("comments");
        let file = write_source(
            &dir,
            "A.cs",
            "// a comment\n#region Foo\nclass A {}\n#endregion\n",
        );

        let corpus = tokenize_all(&[file], DupeMode::Exact);
        std::fs::remove_dir_all(&dir).ok();

        // class, A, {, }, <sentinel>
        assert_eq!(corpus.ids.len(), 5);
    }

    #[test]
    fn semantic_mode_collapses_identifiers_but_not_keywords() {
        let dir = temp_dir("semantic");
        let a = write_source(&dir, "A.cs", "class Foo { void M(int x) { var y = x; } }");
        let b = write_source(&dir, "B.cs", "class Bar { void N(int z) { var w = z; } }");

        // Tokenized one file at a time so each corpus's lone sentinel is
        // u32::MAX in both cases, making the id sequences directly comparable.
        let corpus_a = tokenize_all(&[a], DupeMode::Semantic);
        let corpus_b = tokenize_all(&[b], DupeMode::Semantic);
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(corpus_a.ids, corpus_b.ids);
    }

    #[test]
    fn exact_mode_keeps_distinct_identifiers_distinct() {
        let dir = temp_dir("exact");
        let a = write_source(&dir, "A.cs", "class Foo {}");
        let b = write_source(&dir, "B.cs", "class Bar {}");

        // Both files interned into the same corpus, so a shared identifier
        // alphabet would reuse ids for identical text — "Foo" and "Bar"
        // must still land on different ids.
        let corpus = tokenize_all(&[a, b], DupeMode::Exact);
        std::fs::remove_dir_all(&dir).ok();

        // class, Foo, {, }, <sentinel A>, class, Bar, {, }, <sentinel B>
        assert_eq!(corpus.ids.len(), 10);
        assert_ne!(corpus.ids[1], corpus.ids[6]);
    }
}
