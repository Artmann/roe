//! Hotspot scoring: the intersection of complexity and change frequency.
//!
//! Complexity alone is a weak priority signal — a gnarly parser nobody has
//! touched in two years is not where the next bug lands. Churn alone is
//! weaker still, since a config file changes constantly and costs nothing to
//! read. The product of the two is what identifies code that is both hard to
//! understand and under constant revision.
//!
//! Both inputs are normalized against the highest value in the same run, so
//! scores are relative: the riskiest file always scores close to 100, and a
//! score is only meaningful next to the other scores beside it.

use std::path::PathBuf;

use crate::model::Hotspot;

/// One analyzed file, before scoring.
#[derive(Debug)]
pub struct Candidate {
    pub file: PathBuf,
    pub project: Option<String>,
    /// Summed cyclomatic complexity of every member in the file.
    pub cyclomatic: u32,
    pub lines: u32,
    /// Recency-weighted commit count from [`crate::churn`].
    pub weighted_commits: f64,
}

/// Score every candidate and return the `limit` riskiest, highest first.
///
/// Files with no churn or no complexity score zero and are dropped — a
/// hotspot list padded with zeroes buries the signal it exists to surface.
pub fn rank(candidates: &[Candidate], limit: usize) -> Vec<Hotspot> {
    let densities: Vec<f64> = candidates.iter().map(Candidate::density).collect();

    let max_churn = fold_max(
        candidates
            .iter()
            .map(|candidate| candidate.weighted_commits),
    );
    let max_density = fold_max(densities.iter().copied());

    if max_churn <= 0.0 || max_density <= 0.0 {
        return Vec::new();
    }

    let mut hotspots: Vec<Hotspot> = candidates
        .iter()
        .zip(&densities)
        .filter_map(|(candidate, &density)| {
            let normalized_churn = candidate.weighted_commits / max_churn;
            let normalized_complexity = density / max_density;
            let score = (normalized_churn * normalized_complexity * 100.0).clamp(0.0, 100.0);

            if score <= 0.0 {
                return None;
            }

            Some(Hotspot {
                file: candidate.file.clone(),
                project: candidate.project.clone(),
                score,
                weighted_commits: candidate.weighted_commits,
                cyclomatic: candidate.cyclomatic,
                lines: candidate.lines,
                complexity_density: density,
            })
        })
        .collect();

    // Ties break on path so the ranking is stable across runs and platforms.
    hotspots.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.file.cmp(&b.file))
    });
    hotspots.truncate(limit);

    hotspots
}

impl Candidate {
    fn density(&self) -> f64 {
        if self.lines == 0 {
            return 0.0;
        }

        f64::from(self.cyclomatic) / f64::from(self.lines)
    }
}

fn fold_max(values: impl Iterator<Item = f64>) -> f64 {
    values.fold(0.0_f64, f64::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(name: &str, cyclomatic: u32, lines: u32, weighted_commits: f64) -> Candidate {
        Candidate {
            file: PathBuf::from(name),
            project: None,
            cyclomatic,
            lines,
            weighted_commits,
        }
    }

    #[test]
    fn the_riskiest_file_scores_one_hundred() {
        let candidates = vec![
            candidate("hot.cs", 50, 100, 10.0),
            candidate("mild.cs", 5, 100, 1.0),
        ];

        let ranked = rank(&candidates, 10);

        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].file, PathBuf::from("hot.cs"));
        assert!((ranked[0].score - 100.0).abs() < 1e-9);
    }

    #[test]
    fn complexity_without_churn_is_not_a_hotspot() {
        // A gnarly file nobody touches is not where the next bug lands.
        let candidates = vec![
            candidate("settled.cs", 200, 100, 0.0),
            candidate("active.cs", 10, 100, 5.0),
        ];

        let ranked = rank(&candidates, 10);

        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].file, PathBuf::from("active.cs"));
    }

    #[test]
    fn churn_without_complexity_is_not_a_hotspot() {
        let candidates = vec![
            candidate("trivial.cs", 0, 100, 50.0),
            candidate("active.cs", 10, 100, 5.0),
        ];

        let ranked = rank(&candidates, 10);

        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].file, PathBuf::from("active.cs"));
    }

    #[test]
    fn density_beats_raw_complexity() {
        // Both files carry the same total complexity, but one packs it into a
        // tenth of the lines and so is far denser to read.
        let candidates = vec![
            candidate("dense.cs", 40, 100, 5.0),
            candidate("spread.cs", 40, 1000, 5.0),
        ];

        let ranked = rank(&candidates, 10);

        assert_eq!(ranked[0].file, PathBuf::from("dense.cs"));
        assert!(ranked[0].score > ranked[1].score);
    }

    #[test]
    fn scores_stay_within_zero_and_one_hundred() {
        let candidates = vec![
            candidate("a.cs", 90, 100, 12.5),
            candidate("b.cs", 3, 400, 0.25),
            candidate("c.cs", 30, 120, 7.0),
        ];

        for hotspot in rank(&candidates, 10) {
            assert!(
                (0.0..=100.0).contains(&hotspot.score),
                "{} scored {}",
                hotspot.file.display(),
                hotspot.score
            );
        }
    }

    #[test]
    fn the_limit_truncates_the_ranking() {
        let candidates = vec![
            candidate("a.cs", 40, 100, 9.0),
            candidate("b.cs", 30, 100, 8.0),
            candidate("c.cs", 20, 100, 7.0),
        ];

        let ranked = rank(&candidates, 2);

        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].file, PathBuf::from("a.cs"));
        assert_eq!(ranked[1].file, PathBuf::from("b.cs"));
    }

    #[test]
    fn a_repository_with_no_history_ranks_nothing() {
        let candidates = vec![
            candidate("a.cs", 40, 100, 0.0),
            candidate("b.cs", 30, 100, 0.0),
        ];

        assert!(rank(&candidates, 10).is_empty());
    }

    #[test]
    fn an_empty_file_does_not_divide_by_zero() {
        let candidates = vec![
            candidate("empty.cs", 0, 0, 3.0),
            candidate("real.cs", 10, 50, 3.0),
        ];

        let ranked = rank(&candidates, 10);

        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].file, PathBuf::from("real.cs"));
    }
}
