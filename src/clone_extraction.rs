use crate::model::{DupeGroup, Occurrence};
use crate::suffix_array::{build_lcp_array, build_suffix_array};
use crate::tokenize::Corpus;

/// A maximal-repeat candidate: suffixes `sa[left..=right]` all share a common
/// prefix of exactly `lcp` tokens (bounded by strictly smaller neighboring
/// LCP values on both sides, so it can't be widened without shrinking `lcp`).
struct Interval {
    lcp: u32,
    left: usize,
    right: usize,
}

/// Finds duplicated token runs in `corpus` via suffix array + LCP interval
/// extraction, filters out non-left-maximal (redundant, truncated) repeats,
/// applies the threshold flags, and maps surviving token ranges back to
/// source locations. Output is sorted by `(token_count desc, occurrence_count
/// desc, first location asc)` for deterministic, most-impactful-first display.
pub fn extract_groups(
    corpus: &Corpus,
    min_tokens: u32,
    min_lines: u32,
    min_occurrences: u32,
) -> Vec<DupeGroup> {
    let suffix_array = build_suffix_array(&corpus.ids);
    let lcp = build_lcp_array(&corpus.ids, &suffix_array);

    let mut groups: Vec<DupeGroup> = lcp_intervals(&lcp)
        .into_iter()
        .filter(|interval| interval.lcp >= min_tokens)
        .filter_map(|interval| {
            let occurrence_count = (interval.right - interval.left + 1) as u32;
            if occurrence_count < min_occurrences {
                return None;
            }

            let positions: Vec<usize> = suffix_array[interval.left..=interval.right]
                .iter()
                .map(|&index| index as usize)
                .collect();

            if !is_left_maximal(&corpus.ids, &positions) {
                return None;
            }

            let occurrences = to_occurrences(corpus, &positions, interval.lcp);
            let line_count = occurrences
                .iter()
                .map(|occurrence| occurrence.end_line - occurrence.start_line + 1)
                .min()
                .unwrap_or(0);
            if line_count < min_lines {
                return None;
            }

            Some(DupeGroup {
                token_count: interval.lcp,
                line_count,
                occurrences,
            })
        })
        .collect();

    groups.sort_by(|a, b| {
        b.token_count
            .cmp(&a.token_count)
            .then_with(|| b.occurrences.len().cmp(&a.occurrences.len()))
            .then_with(|| first_location(a).cmp(&first_location(b)))
    });

    groups
}

fn first_location(group: &DupeGroup) -> (std::path::PathBuf, u32, u32) {
    let first = &group.occurrences[0];
    (first.file.clone(), first.start_line, first.start_column)
}

fn to_occurrences(corpus: &Corpus, positions: &[usize], token_count: u32) -> Vec<Occurrence> {
    positions
        .iter()
        .map(|&start| {
            let end = start + token_count as usize - 1;
            assert!(
                end < corpus.ids.len(),
                "a duplicate match can never extend past the end of the token corpus"
            );

            let start_pos = &corpus.positions[start];
            let end_pos = &corpus.positions[end];

            Occurrence {
                file: corpus.files[start_pos.file_index as usize].clone(),
                start_line: start_pos.start_line,
                start_column: start_pos.start_column,
                end_line: end_pos.end_line,
                end_column: end_pos.end_column,
            }
        })
        .collect()
}

/// A repeat is left-maximal unless every occurrence shares the exact same
/// preceding token — in that case the same set of occurrences (shifted one
/// token left) forms a longer, strictly more specific repeat reported as its
/// own interval elsewhere, making this one a redundant, truncated duplicate
/// of it.
fn is_left_maximal(tokens: &[u32], positions: &[usize]) -> bool {
    let mut first_left: Option<Option<u32>> = None;

    for &position in positions {
        let left = if position == 0 {
            None
        } else {
            Some(tokens[position - 1])
        };

        match first_left {
            None => first_left = Some(left),
            Some(seen) if seen != left => return true,
            Some(_) => {}
        }
    }

    false
}

/// Linear-time LCP-interval extraction (Abouelhoda–Kurtz–Ohlebusch): a
/// monotone stack of open `(lcp, left)` frames, closing a frame whenever the
/// next LCP value drops below it.
fn lcp_intervals(lcp: &[u32]) -> Vec<Interval> {
    let length = lcp.len();
    let mut stack: Vec<(u32, usize)> = vec![(0, 0)];
    let mut intervals = Vec::new();

    for (i, &value) in lcp.iter().enumerate().skip(1) {
        let mut left = i - 1;

        while stack.last().expect("sentinel frame is never popped here").0 > value {
            let (top_lcp, top_left) = stack.pop().expect("checked non-empty above");
            intervals.push(Interval {
                lcp: top_lcp,
                left: top_left,
                right: i - 1,
            });
            left = top_left;
        }

        if stack.last().expect("sentinel frame is never popped here").0 < value {
            stack.push((value, left));
        }
    }

    while let Some((top_lcp, top_left)) = stack.pop() {
        if top_lcp > 0 {
            intervals.push(Interval {
                lcp: top_lcp,
                left: top_left,
                right: length - 1,
            });
        }
    }

    intervals
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::tokenize::TokenPosition;

    /// Builds a `Corpus` directly from token ids, one synthetic line per
    /// token, for pure-logic testing without going through the tokenizer.
    fn corpus_from_ids(ids: &[u32], file_index: u32, files: &[&str]) -> Corpus {
        let positions = ids
            .iter()
            .enumerate()
            .map(|(i, _)| TokenPosition {
                file_index,
                start_line: i as u32 + 1,
                start_column: 1,
                end_line: i as u32 + 1,
                end_column: 2,
            })
            .collect();

        Corpus {
            ids: ids.to_vec(),
            positions,
            files: files.iter().map(PathBuf::from).collect(),
        }
    }

    #[test]
    fn basic_repeated_pair_is_found() {
        // "1,2,3" repeated at positions 0 and 3.
        let corpus = corpus_from_ids(&[1, 2, 3, 1, 2, 3], 0, &["a.cs"]);
        let groups = extract_groups(&corpus, 1, 1, 2);

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].token_count, 3);
        assert_eq!(groups[0].occurrences.len(), 2);
    }

    #[test]
    fn left_maximality_filter_drops_the_truncated_submatch() {
        // "9,1,2,3" repeated at positions 0 and 4. The inner "1,2,3" run (at
        // positions 1 and 5) is a strict, non-left-maximal submatch: both of
        // its occurrences are preceded by the same token (9), so it must be
        // dropped in favor of the longer 4-token match.
        let corpus = corpus_from_ids(&[9, 1, 2, 3, 9, 1, 2, 3], 0, &["a.cs"]);
        let groups = extract_groups(&corpus, 1, 1, 2);

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].token_count, 4);
        assert_eq!(groups[0].occurrences.len(), 2);
    }

    #[test]
    fn no_match_crosses_a_file_boundary() {
        // Two "files" both containing "1,2,3", separated by distinct
        // sentinel ids that can never recur — the match must be reported as
        // two 3-token occurrences, never one 7-token occurrence spanning
        // both files.
        let corpus = corpus_from_ids(&[1, 2, 3, 100, 1, 2, 3, 99], 0, &["a.cs", "b.cs"]);
        let groups = extract_groups(&corpus, 1, 1, 2);

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].token_count, 3);
        assert_eq!(groups[0].occurrences.len(), 2);
    }

    #[test]
    fn min_tokens_threshold_filters_short_matches() {
        let corpus = corpus_from_ids(&[1, 2, 3, 1, 2, 3], 0, &["a.cs"]);
        assert!(extract_groups(&corpus, 4, 1, 2).is_empty());
    }

    #[test]
    fn min_occurrences_threshold_filters_rare_matches() {
        let corpus = corpus_from_ids(&[1, 2, 3, 1, 2, 3], 0, &["a.cs"]);
        assert!(extract_groups(&corpus, 1, 1, 3).is_empty());
    }

    #[test]
    fn min_lines_threshold_uses_minimum_span_across_occurrences() {
        // Each token is one synthetic line, so a 3-token match spans 3 lines
        // in this fixture — requiring 4 must filter it out.
        let corpus = corpus_from_ids(&[1, 2, 3, 1, 2, 3], 0, &["a.cs"]);
        assert!(extract_groups(&corpus, 1, 4, 2).is_empty());
        assert_eq!(extract_groups(&corpus, 1, 3, 2).len(), 1);
    }

    #[test]
    fn no_duplicates_produces_no_groups() {
        let corpus = corpus_from_ids(&[1, 2, 3, 4, 5, 6], 0, &["a.cs"]);
        assert!(extract_groups(&corpus, 1, 1, 2).is_empty());
    }
}
