//! Git churn: how often each file has changed, weighted toward recent work.
//!
//! Raw commit counts are a poor risk signal on their own — a file rewritten
//! fifty times three years ago and untouched since is settled, not volatile.
//! Every commit is therefore discounted by its age on an exponential decay
//! with a 90-day half-life: a change from today counts 1.0, one from three
//! months ago 0.5, one from a year ago about 0.06.
//!
//! Only the first-parent diff is walked. Merge commits are traversed but
//! contribute nothing of their own, so a squashed pull request is one change
//! rather than one per branch commit.

use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow};
use rustc_hash::FxHashMap;

/// Days after which a commit's contribution is halved.
const HALF_LIFE_DAYS: f64 = 90.0;

const SECONDS_PER_DAY: f64 = 86_400.0;

/// Recency-weighted change counts, keyed by absolute file path.
#[derive(Debug, Default)]
pub struct Churn {
    weights: FxHashMap<PathBuf, f64>,
    /// Total commits walked, for reporting how much history backed the
    /// numbers.
    pub commits_walked: usize,
    /// Non-fatal problems worth telling the user about — a shallow clone,
    /// most importantly, since it silently truncates every score.
    pub warnings: Vec<String>,
}

impl Churn {
    /// Recency-weighted change count for `file`, or 0.0 for a file git has
    /// never seen (newly added, ignored, or outside the repository).
    pub fn weight(&self, file: &Path) -> f64 {
        self.weights.get(file).copied().unwrap_or(0.0)
    }
}

/// Walk the history of the repository containing `root` and accumulate
/// per-file recency-weighted change counts.
///
/// Errors when `root` is not inside a git repository, since the caller asked
/// for history-based analysis and silently returning zeros would look like a
/// clean bill of health.
pub fn analyze(root: &Path) -> anyhow::Result<Churn> {
    let repository = gix::discover(root).map_err(|error| {
        anyhow!(
            "not a git repository: {}\n\
             Hotspots rank files by how often they change, which needs git \
             history. Run roe from inside a git working tree, or drop \
             --hotspots.\n\
             ({error})",
            crate::paths::display(root)
        )
    })?;

    let mut churn = Churn::default();

    if repository.is_shallow() {
        churn.warnings.push(
            "this is a shallow clone, so hotspot scores only reflect the \
             commits that were fetched. Run `git fetch --unshallow` (or set \
             `fetch-depth: 0` in CI) for accurate churn."
                .to_string(),
        );
    }

    let work_directory = repository
        .workdir()
        .ok_or_else(|| {
            anyhow!(
                "this is a bare git repository, which has no working tree to \
                 analyze. Run roe from a normal clone, or drop --hotspots."
            )
        })?
        .to_path_buf();
    let work_directory =
        crate::paths::canonicalize(&work_directory).unwrap_or_else(|_| work_directory.clone());

    let Ok(head) = repository.head_commit() else {
        churn.warnings.push(
            "this repository has no commits yet, so every hotspot score is \
             zero."
                .to_string(),
        );

        return Ok(churn);
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0);

    let walk = repository
        .rev_walk([head.id])
        .first_parent_only()
        .all()
        .context("failed to walk the git history")?;

    for step in walk {
        let info = step.context("failed to read a commit while walking history")?;
        let commit = info
            .object()
            .context("failed to load a commit while walking history")?;

        churn.commits_walked += 1;

        let seconds = commit
            .time()
            .context("failed to read a commit timestamp")?
            .seconds;
        let weight = decay(age_in_days(now, seconds));

        for path in changed_paths(&repository, &commit)? {
            *churn
                .weights
                .entry(work_directory.join(path))
                .or_insert(0.0) += weight;
        }
    }

    Ok(churn)
}

/// How old a commit is, in fractional days.
///
/// Clamped at zero: imported and rebased history routinely carries timestamps
/// slightly in the future, and a negative age would weight such a commit at
/// more than a fresh one.
fn age_in_days(now_seconds: i64, commit_seconds: i64) -> f64 {
    ((now_seconds - commit_seconds).max(0) as f64) / SECONDS_PER_DAY
}

/// What a commit of a given age contributes to a file's churn: 1.0 today,
/// halving every [`HALF_LIFE_DAYS`].
fn decay(age_days: f64) -> f64 {
    0.5_f64.powf(age_days / HALF_LIFE_DAYS)
}

/// Repository-relative paths touched by `commit` relative to its first parent.
/// A root commit is diffed against the empty tree, so its files count as added.
fn changed_paths(
    repository: &gix::Repository,
    commit: &gix::Commit<'_>,
) -> anyhow::Result<Vec<PathBuf>> {
    let tree = commit.tree().context("failed to read a commit tree")?;
    let parent = match commit.parent_ids().next() {
        Some(id) => id
            .object()
            .context("failed to load a parent commit")?
            .into_commit()
            .tree()
            .context("failed to read a parent commit tree")?,
        None => repository
            .empty_tree()
            .id()
            .object()
            .context("failed to load the empty tree")?
            .into_tree(),
    };

    let mut paths = Vec::new();
    let mut changes = parent.changes().context("failed to set up a tree diff")?;

    // Rename tracking compares blob contents to pair deletions with additions.
    // It costs a similarity metric per candidate on every commit, and it can't
    // change the answer here — a rename touches the file either way.
    changes.options(|options| {
        options.track_rewrites(None);
    });

    changes
        .for_each_to_obtain_tree(&tree, |change| {
            if change.entry_mode().is_blob() {
                let location = change.location().to_string();
                paths.push(PathBuf::from(
                    location.replace('/', std::path::MAIN_SEPARATOR_STR),
                ));
            }

            Ok::<_, std::convert::Infallible>(gix::object::tree::diff::Action::Continue(()))
        })
        .context("failed to diff a commit against its parent")?;

    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A day in seconds, so the age cases below read as calendar time.
    const DAY: i64 = 86_400;

    #[test]
    fn a_commit_from_today_counts_about_once() {
        assert!((decay(0.0) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_commit_one_half_life_old_counts_half() {
        assert!((decay(HALF_LIFE_DAYS) - 0.5).abs() < 1e-9);
        assert!((decay(2.0 * HALF_LIFE_DAYS) - 0.25).abs() < 1e-9);
    }

    #[test]
    fn a_commit_a_year_old_barely_counts() {
        // The docs promise "about 0.06" at a year, which is the whole reason
        // churn is weighted rather than counted.
        let weight = decay(365.0);

        assert!(weight > 0.05 && weight < 0.07, "got {weight}");
    }

    #[test]
    fn age_is_the_elapsed_time_in_days() {
        let now = 1_000 * DAY;

        assert!((age_in_days(now, now) - 0.0).abs() < 1e-9);
        assert!((age_in_days(now, now - DAY) - 1.0).abs() < 1e-9);
        assert!((age_in_days(now, now - 45 * DAY) - 45.0).abs() < 1e-9);
        // Fractions matter: half a day is half a day, not a whole one.
        assert!((age_in_days(now, now - DAY / 2) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn a_commit_timestamped_in_the_future_is_treated_as_brand_new() {
        let now = 1_000 * DAY;

        assert_eq!(age_in_days(now, now + 30 * DAY), 0.0);
    }

    #[test]
    fn an_unknown_file_has_no_churn() {
        let churn = Churn::default();

        assert_eq!(churn.weight(Path::new("nowhere.cs")), 0.0);
    }

    #[test]
    fn a_recorded_file_reports_its_weight() {
        let mut churn = Churn::default();
        churn.weights.insert(PathBuf::from("a.cs"), 4.25);

        assert_eq!(churn.weight(Path::new("a.cs")), 4.25);
    }
}
