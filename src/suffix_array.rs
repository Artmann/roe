/// Builds the suffix array of `tokens` via prefix doubling (`O(n log² n)`):
/// `sa[i]` is the corpus index where the `i`'th lexicographically-smallest
/// suffix starts. Each round doubles the prefix length being compared,
/// re-ranking suffixes by `(rank[i], rank[i + k])` until every suffix has a
/// unique rank (or `k` exceeds the corpus length).
pub fn build_suffix_array(tokens: &[u32]) -> Vec<u32> {
    let length = tokens.len();

    if length == 0 {
        return Vec::new();
    }

    let mut suffix_array: Vec<u32> = (0..length as u32).collect();
    let mut rank: Vec<u32> = tokens.to_vec();
    let mut next_rank = vec![0u32; length];
    let mut k = 1usize;

    loop {
        let key = |index: usize| -> (u32, i64) {
            let second = if index + k < length {
                rank[index + k] as i64
            } else {
                -1
            };
            (rank[index], second)
        };

        suffix_array.sort_unstable_by_key(|&index| key(index as usize));

        next_rank[suffix_array[0] as usize] = 0;
        for window in 1..length {
            let previous = suffix_array[window - 1] as usize;
            let current = suffix_array[window] as usize;
            let bump = u32::from(key(previous) < key(current));

            next_rank[current] = next_rank[previous] + bump;
        }
        std::mem::swap(&mut rank, &mut next_rank);

        if rank[suffix_array[length - 1] as usize] as usize == length - 1 {
            break;
        }

        k *= 2;
        if k >= length {
            break;
        }
    }

    suffix_array
}

/// Kasai's algorithm: `lcp[i]` is the length of the longest common prefix
/// between the suffixes at `sa[i - 1]` and `sa[i]`; `lcp[0]` is always `0`
/// (no predecessor). Runs in `O(n)` given the suffix array and its inverse.
pub fn build_lcp_array(tokens: &[u32], suffix_array: &[u32]) -> Vec<u32> {
    let length = tokens.len();

    if length == 0 {
        return Vec::new();
    }

    let mut rank_of = vec![0u32; length];
    for (rank, &position) in suffix_array.iter().enumerate() {
        rank_of[position as usize] = rank as u32;
    }

    let mut lcp = vec![0u32; length];
    let mut run = 0u32;

    for position in 0..length {
        let rank = rank_of[position] as usize;
        if rank == 0 {
            run = 0;
            continue;
        }

        let previous_position = suffix_array[rank - 1] as usize;
        run = run.saturating_sub(1);
        while position + (run as usize) < length
            && previous_position + (run as usize) < length
            && tokens[position + run as usize] == tokens[previous_position + run as usize]
        {
            run += 1;
        }
        lcp[rank] = run;
    }

    lcp
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Brute-force reference LCP length between two suffixes, used to check
    /// the fast implementation against small hand-verified inputs.
    fn naive_lcp(tokens: &[u32], a: usize, b: usize) -> u32 {
        let mut length = 0;
        while a + length < tokens.len()
            && b + length < tokens.len()
            && tokens[a + length] == tokens[b + length]
        {
            length += 1;
        }
        length as u32
    }

    fn assert_valid_suffix_array(tokens: &[u32], suffix_array: &[u32]) {
        assert_eq!(suffix_array.len(), tokens.len());
        for window in suffix_array.windows(2) {
            let (a, b) = (window[0] as usize, window[1] as usize);
            assert!(
                tokens[a..] < tokens[b..],
                "suffix at {a} must sort before suffix at {b}"
            );
        }
    }

    #[test]
    fn empty_input_produces_empty_arrays() {
        assert!(build_suffix_array(&[]).is_empty());
        assert!(build_lcp_array(&[], &[]).is_empty());
    }

    #[test]
    fn single_token_has_one_suffix() {
        let tokens = [7u32];
        let sa = build_suffix_array(&tokens);
        assert_eq!(sa, vec![0]);
        assert_eq!(build_lcp_array(&tokens, &sa), vec![0]);
    }

    #[test]
    fn suffix_array_orders_banana_correctly() {
        // "banana" encoded as token ids preserving character order
        // (a=1, b=2, n=3), a classic hand-verifiable suffix array example.
        let tokens = [2u32, 1, 3, 1, 3, 1];
        let sa = build_suffix_array(&tokens);
        assert_valid_suffix_array(&tokens, &sa);
        // Suffixes sorted: "a"(5) < "ana"(3) < "anana"(1) < "banana"(0) <
        // "na"(4) < "nana"(2)
        assert_eq!(sa, vec![5, 3, 1, 0, 4, 2]);
    }

    #[test]
    fn lcp_array_matches_brute_force_reference() {
        let tokens = [1u32, 2, 3, 2, 3, 2];
        let sa = build_suffix_array(&tokens);
        let lcp = build_lcp_array(&tokens, &sa);

        assert_eq!(lcp[0], 0);
        for i in 1..sa.len() {
            let expected = naive_lcp(&tokens, sa[i - 1] as usize, sa[i] as usize);
            assert_eq!(lcp[i], expected, "mismatch at rank {i}");
        }
    }

    #[test]
    fn repeated_token_run_has_matching_lcp() {
        // [1,2,3, 1,2,3, 1,2,3] — every rotation starting with 1,2,3 repeats.
        let tokens = [1u32, 2, 3, 1, 2, 3, 1, 2, 3];
        let sa = build_suffix_array(&tokens);
        assert_valid_suffix_array(&tokens, &sa);
        let lcp = build_lcp_array(&tokens, &sa);

        for i in 1..sa.len() {
            let expected = naive_lcp(&tokens, sa[i - 1] as usize, sa[i] as usize);
            assert_eq!(lcp[i], expected, "mismatch at rank {i}");
        }
        // The three suffixes starting at 0, 3, 6 all share the full "1,2,3"
        // prefix (and beyond, until one runs out).
        assert!(lcp.contains(&3));
    }

    #[test]
    fn distinct_sentinels_never_match_each_other() {
        // Two "files" of [1,2] each terminated by a unique sentinel.
        let tokens = [1u32, 2, 100, 1, 2, 99];
        let sa = build_suffix_array(&tokens);
        assert_valid_suffix_array(&tokens, &sa);
        let lcp = build_lcp_array(&tokens, &sa);

        // The suffixes starting at 0 and 3 ("1,2,100,..." vs "1,2,99") share
        // exactly the 2-token "1,2" prefix — the distinct sentinels must
        // stop the match from extending further.
        let rank_of_0 = sa.iter().position(|&p| p == 0).unwrap();
        let rank_of_3 = sa.iter().position(|&p| p == 3).unwrap();
        let (lower, upper) = if rank_of_0 < rank_of_3 {
            (rank_of_0, rank_of_3)
        } else {
            (rank_of_3, rank_of_0)
        };
        let between_min = lcp[lower + 1..=upper].iter().min().copied().unwrap();
        assert_eq!(between_min, 2);
    }
}
