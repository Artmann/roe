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

/// A clone candidate still in token space: every occurrence starts at one of
/// `positions` (ascending) and runs for `token_count` tokens.
struct Candidate {
    token_count: u32,
    positions: Vec<usize>,
}

/// A candidate is subsumed when at least this fraction of every occurrence's
/// token span is already covered by a kept group — it is a shorter or
/// narrower echo of duplication the report already shows.
const SUBSUMED_COVERAGE_THRESHOLD: f64 = 0.75;

/// Finds duplicated token runs in `corpus` via suffix array + LCP interval
/// extraction, filters out non-left-maximal (redundant, truncated) repeats,
/// snaps matches to whole-line boundaries, applies the threshold flags,
/// suppresses groups subsumed by higher-impact ones, and maps the survivors
/// back to source locations. Output is sorted by redundant-token impact
/// (`token_count × (occurrences − 1)` desc) for most-impactful-first display.
pub fn extract_groups(
    corpus: &Corpus,
    min_tokens: u32,
    min_lines: u32,
    min_occurrences: u32,
) -> Vec<DupeGroup> {
    let suffix_array = build_suffix_array(&corpus.ids);
    let lcp = build_lcp_array(&corpus.ids, &suffix_array);

    let mut candidates: Vec<Candidate> = lcp_intervals(&lcp)
        .into_iter()
        .filter(|interval| interval.lcp >= min_tokens)
        .filter_map(|interval| {
            let occurrence_count = (interval.right - interval.left + 1) as u32;
            if occurrence_count < min_occurrences {
                return None;
            }

            let mut positions: Vec<usize> = suffix_array[interval.left..=interval.right]
                .iter()
                .map(|&index| index as usize)
                .collect();
            positions.sort_unstable();

            if !is_left_maximal(&corpus.ids, &positions) {
                return None;
            }

            snap_to_line_boundaries(corpus, positions, interval.lcp)
        })
        .filter(|candidate| {
            candidate.token_count >= min_tokens && minimum_line_span(corpus, candidate) >= min_lines
        })
        .collect();

    candidates.sort_by(|a, b| {
        impact(b)
            .cmp(&impact(a))
            .then_with(|| b.token_count.cmp(&a.token_count))
            .then_with(|| a.positions.cmp(&b.positions))
    });

    suppress_subsumed(candidates, corpus.ids.len())
        .into_iter()
        .map(|candidate| DupeGroup {
            token_count: candidate.token_count,
            line_count: minimum_line_span(corpus, &candidate),
            occurrences: to_occurrences(corpus, &candidate.positions, candidate.token_count),
        })
        .collect()
}

/// Redundant tokens this clone represents — everything past the first
/// occurrence could in principle be deleted. A short block copied seven times
/// outranks a long block copied twice.
fn impact(candidate: &Candidate) -> u64 {
    candidate.token_count as u64 * (candidate.positions.len() as u64).saturating_sub(1)
}

/// The line span of the shortest occurrence, matching how `min_lines` was
/// always interpreted.
fn minimum_line_span(corpus: &Corpus, candidate: &Candidate) -> u32 {
    let length = candidate.token_count as usize;

    candidate
        .positions
        .iter()
        .map(|&start| {
            let first = &corpus.positions[start];
            let last = &corpus.positions[start + length - 1];

            last.end_line - first.start_line + 1
        })
        .min()
        .unwrap_or(0)
}

/// Trims the shared token range so that, in every occurrence, the first token
/// starts its source line and the last token ends its line. Raw maximal
/// repeats often begin right after a differing token (a method name, say) and
/// get reported as a confusing mid-line span whose first printed line is not
/// actually identical across occurrences. Returns `None` when no whole-line
/// core remains.
fn snap_to_line_boundaries(
    corpus: &Corpus,
    positions: Vec<usize>,
    token_count: u32,
) -> Option<Candidate> {
    let token_count = token_count as usize;

    let leading = (0..token_count).find(|&offset| {
        positions
            .iter()
            .all(|&start| starts_line(corpus, start + offset))
    })?;
    let last = (leading..token_count).rev().find(|&offset| {
        positions
            .iter()
            .all(|&start| ends_line(corpus, start + offset))
    })?;

    Some(Candidate {
        token_count: (last - leading + 1) as u32,
        positions: positions.iter().map(|&start| start + leading).collect(),
    })
}

/// True when no earlier token shares this token's first line. File sentinels
/// carry a zeroed position (`start_line` 0), so the first real token after
/// one also counts as a line start.
fn starts_line(corpus: &Corpus, index: usize) -> bool {
    if index == 0 {
        return true;
    }

    let previous = &corpus.positions[index - 1];
    let current = &corpus.positions[index];

    previous.start_line == 0
        || previous.file_index != current.file_index
        || previous.end_line < current.start_line
}

/// True when no later token shares this token's last line.
fn ends_line(corpus: &Corpus, index: usize) -> bool {
    let current = &corpus.positions[index];

    match corpus.positions.get(index + 1) {
        None => true,
        Some(next) => {
            next.start_line == 0
                || next.file_index != current.file_index
                || next.start_line > current.end_line
        }
    }
}

/// Greedy overlap suppression. Candidates arrive in impact order; each kept
/// candidate claims its token spans, and a later candidate survives only if
/// at least one of its occurrences still lands mostly on unclaimed tokens.
/// This collapses the lattice of sub-runs a maximal-repeat search emits
/// around every real clone (the same block otherwise gets reported at many
/// slightly different lengths and occurrence counts).
fn suppress_subsumed(candidates: Vec<Candidate>, corpus_length: usize) -> Vec<Candidate> {
    let mut claimed = vec![false; corpus_length];
    let mut kept = Vec::new();

    for candidate in candidates {
        let length = candidate.token_count as usize;

        let adds_new_territory = candidate.positions.iter().any(|&start| {
            let covered = claimed[start..start + length]
                .iter()
                .filter(|&&slot| slot)
                .count();

            (covered as f64) < (length as f64) * SUBSUMED_COVERAGE_THRESHOLD
        });

        if !adds_new_territory {
            continue;
        }

        for &start in &candidate.positions {
            for slot in &mut claimed[start..start + length] {
                *slot = true;
            }
        }
        kept.push(candidate);
    }

    kept
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

    #[test]
    fn shorter_echo_of_a_kept_group_is_suppressed() {
        // "2,3,4,5" appears three times (impact 8); "1,2,3,4,5" appears twice
        // (impact 5). The wide trio is kept first and claims its spans; the
        // pair then adds only one fresh token per occurrence (80% covered),
        // so it is dropped as a redundant echo.
        let corpus = corpus_from_ids(
            &[1, 2, 3, 4, 5, 90, 1, 2, 3, 4, 5, 91, 2, 3, 4, 5, 92],
            0,
            &["a.cs"],
        );
        let groups = extract_groups(&corpus, 1, 1, 2);

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].token_count, 4);
        assert_eq!(groups[0].occurrences.len(), 3);
    }

    #[test]
    fn genuinely_longer_pair_survives_suppression() {
        // The 8-token pair (impact 8) is kept first; the 3-token trio
        // (impact 6) still survives because its third occurrence lands
        // entirely on unclaimed tokens.
        let corpus = corpus_from_ids(
            &[
                1, 2, 3, 4, 5, 6, 7, 8, 90, 1, 2, 3, 4, 5, 6, 7, 8, 91, 1, 2, 3, 92,
            ],
            0,
            &["a.cs"],
        );
        let groups = extract_groups(&corpus, 1, 1, 2);

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].token_count, 8);
        assert_eq!(groups[0].occurrences.len(), 2);
        assert_eq!(groups[1].token_count, 3);
        assert_eq!(groups[1].occurrences.len(), 3);
    }

    #[test]
    fn groups_are_ranked_by_redundant_token_impact() {
        // A 3-token clone with three occurrences (impact 6) outranks a
        // 5-token clone with two (impact 5), even though the latter is
        // longer.
        let corpus = corpus_from_ids(
            &[
                1, 2, 3, 90, 1, 2, 3, 91, 1, 2, 3, 92, 4, 5, 6, 7, 8, 93, 4, 5, 6, 7, 8, 94,
            ],
            0,
            &["a.cs"],
        );
        let groups = extract_groups(&corpus, 1, 1, 2);

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].token_count, 3);
        assert_eq!(groups[0].occurrences.len(), 3);
        assert_eq!(groups[1].token_count, 5);
        assert_eq!(groups[1].occurrences.len(), 2);
    }

    /// A corpus with hand-placed positions, for line-snapping tests where
    /// tokens must share lines.
    fn corpus_with_positions(ids: &[u32], positions: Vec<TokenPosition>) -> Corpus {
        Corpus {
            ids: ids.to_vec(),
            positions,
            files: vec![PathBuf::from("a.cs")],
        }
    }

    fn position(start_line: u32, start_column: u32) -> TokenPosition {
        TokenPosition {
            file_index: 0,
            start_line,
            start_column,
            end_line: start_line,
            end_column: start_column + 1,
        }
    }

    #[test]
    fn match_starting_mid_line_is_snapped_to_the_next_line_start() {
        // "1,2,3" repeats at positions 1 and 6, but token 1 shares its line
        // with a differing prefix token (9 vs 8) in both occurrences. The
        // match must shrink to the whole-line "2,3" core.
        let ids = [9, 1, 2, 3, 90, 8, 1, 2, 3, 91];
        let positions = vec![
            position(1, 1), // 9
            position(1, 4), // 1 — same line as the differing 9
            position(2, 1), // 2
            position(3, 1), // 3
            position(4, 1), // 90
            position(5, 1), // 8
            position(5, 4), // 1 — same line as the differing 8
            position(6, 1), // 2
            position(7, 1), // 3
            position(8, 1), // 91
        ];
        let corpus = corpus_with_positions(&ids, positions);
        let groups = extract_groups(&corpus, 1, 1, 2);

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].token_count, 2);
        assert_eq!(groups[0].occurrences[0].start_line, 2);
        assert_eq!(groups[0].occurrences[0].start_column, 1);
        assert_eq!(groups[0].occurrences[0].end_line, 3);
        assert_eq!(groups[0].occurrences[1].start_line, 6);
    }

    #[test]
    fn match_that_never_reaches_a_line_boundary_is_dropped() {
        // "1,2" repeats, but in both occurrences the whole match sits
        // mid-line between differing neighbors — no whole-line core exists.
        let ids = [9, 1, 2, 8, 90, 7, 1, 2, 6, 91];
        let positions = vec![
            position(1, 1),  // 9
            position(1, 4),  // 1
            position(1, 7),  // 2
            position(1, 10), // 8
            position(2, 1),  // 90
            position(3, 1),  // 7
            position(3, 4),  // 1
            position(3, 7),  // 2
            position(3, 10), // 6
            position(4, 1),  // 91
        ];
        let corpus = corpus_with_positions(&ids, positions);

        assert!(extract_groups(&corpus, 1, 1, 2).is_empty());
    }
}
