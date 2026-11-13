use std::path::PathBuf;

use serde::{Serialize, Serializer};

/// Nesting past this depth stops raising the estimate.
///
/// Three levels is already 1000× and enough to sort a finding to the top of
/// any report. Letting a five-deep nest score 100,000× would mean one pocket
/// of the codebase permanently owned every slot in the ranking, which is the
/// failure mode this command exists to avoid.
const MAX_COUNTED_DEPTH: u32 = 3;

/// What a [`PerfFinding`] found.
///
/// The slug is the name users type in a config file's `disabledChecks` and the
/// one that appears in JSON, so it is part of the public interface and must
/// not be renamed casually.
#[derive(Debug, Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum PerfFindingKind {
    AwaitInLoop,
    BlockingSleepInAsync,
    CaseInsensitiveCompare,
    ListLookupInLoop,
    LoopInvariantQuery,
    NPlusOneQuery,
    RedundantLinq,
    RegexPerCall,
    SaveChangesInLoop,
    StringConcatInLoop,
    SyncOverAsync,
    TransientHttpClient,
    UnstableClock,
}

/// How much runtime a check's pattern wastes each time it executes.
///
/// Ordered by severity rather than alphabetically, because the derived `Ord`
/// is the point: a `Severe` finding has to compare greater than a `Moderate`
/// one.
#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Cost {
    Moderate,
    High,
    Severe,
}

impl Cost {
    /// The multiplier the ranking uses. The gaps are deliberately wide: a
    /// severe finding at the top of a method should outrank a moderate one
    /// inside a single loop, so the steps have to beat a factor of ten.
    pub fn weight(self) -> f64 {
        match self {
            Cost::Moderate => 3.0,
            Cost::High => 10.0,
            Cost::Severe => 100.0,
        }
    }
}

/// How sure roe is that a finding is real.
///
/// This drives the exit code and nothing else. It deliberately does not
/// participate in the ranking: a `Medium` finding running 1000× per call is
/// still the most valuable thing in the report, and burying it under a
/// certain-but-cold one would defeat the point of ranking at all.
///
/// Ordered by confidence rather than alphabetically, for the same reason
/// [`Cost`] is.
#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Confidence {
    Medium,
    High,
}

impl Confidence {
    pub fn label(self) -> &'static str {
        match self {
            Confidence::Medium => "medium",
            Confidence::High => "high",
        }
    }
}

/// The fixed properties of a check.
///
/// Returned from a single match rather than three separate ones so adding a
/// kind cannot leave two of them updated and the third quietly forgotten.
pub struct CheckMeta {
    pub confidence: Confidence,
    pub cost: Cost,
    /// What the report calls it — short enough to sit in a column.
    pub label: &'static str,
    /// The stable identifier used in JSON and `disabledChecks`.
    pub slug: &'static str,
}

impl PerfFindingKind {
    /// Every check, for validating `disabledChecks` and for naming the valid
    /// options when that validation fails.
    pub const ALL: [PerfFindingKind; 13] = [
        PerfFindingKind::AwaitInLoop,
        PerfFindingKind::BlockingSleepInAsync,
        PerfFindingKind::CaseInsensitiveCompare,
        PerfFindingKind::ListLookupInLoop,
        PerfFindingKind::LoopInvariantQuery,
        PerfFindingKind::NPlusOneQuery,
        PerfFindingKind::RedundantLinq,
        PerfFindingKind::RegexPerCall,
        PerfFindingKind::SaveChangesInLoop,
        PerfFindingKind::StringConcatInLoop,
        PerfFindingKind::SyncOverAsync,
        PerfFindingKind::TransientHttpClient,
        PerfFindingKind::UnstableClock,
    ];

    pub fn meta(self) -> CheckMeta {
        match self {
            PerfFindingKind::AwaitInLoop => CheckMeta {
                confidence: Confidence::Medium,
                cost: Cost::High,
                label: "await in loop",
                slug: "await-in-loop",
            },
            PerfFindingKind::BlockingSleepInAsync => CheckMeta {
                confidence: Confidence::High,
                cost: Cost::High,
                label: "blocking sleep",
                slug: "blocking-sleep-in-async",
            },
            PerfFindingKind::CaseInsensitiveCompare => CheckMeta {
                confidence: Confidence::High,
                cost: Cost::Moderate,
                label: "case compare",
                slug: "case-insensitive-compare",
            },
            PerfFindingKind::ListLookupInLoop => CheckMeta {
                confidence: Confidence::Medium,
                cost: Cost::Severe,
                label: "list lookup",
                slug: "list-lookup-in-loop",
            },
            PerfFindingKind::LoopInvariantQuery => CheckMeta {
                confidence: Confidence::High,
                cost: Cost::High,
                label: "repeated query",
                slug: "loop-invariant-query",
            },
            PerfFindingKind::NPlusOneQuery => CheckMeta {
                confidence: Confidence::High,
                cost: Cost::Severe,
                label: "n+1 query",
                slug: "n-plus-one-query",
            },
            PerfFindingKind::RedundantLinq => CheckMeta {
                confidence: Confidence::Medium,
                cost: Cost::Moderate,
                label: "redundant linq",
                slug: "redundant-linq",
            },
            PerfFindingKind::RegexPerCall => CheckMeta {
                confidence: Confidence::Medium,
                cost: Cost::High,
                label: "regex per call",
                slug: "regex-per-call",
            },
            PerfFindingKind::SaveChangesInLoop => CheckMeta {
                confidence: Confidence::High,
                cost: Cost::Severe,
                label: "save in loop",
                slug: "save-changes-in-loop",
            },
            PerfFindingKind::StringConcatInLoop => CheckMeta {
                confidence: Confidence::High,
                cost: Cost::High,
                label: "string concat",
                slug: "string-concat-in-loop",
            },
            PerfFindingKind::SyncOverAsync => CheckMeta {
                confidence: Confidence::High,
                cost: Cost::Severe,
                label: "sync-over-async",
                slug: "sync-over-async",
            },
            PerfFindingKind::TransientHttpClient => CheckMeta {
                confidence: Confidence::High,
                cost: Cost::Severe,
                label: "transient client",
                slug: "transient-http-client",
            },
            PerfFindingKind::UnstableClock => CheckMeta {
                confidence: Confidence::Medium,
                cost: Cost::Moderate,
                label: "unstable clock",
                slug: "unstable-clock",
            },
        }
    }

    pub fn confidence(self) -> Confidence {
        self.meta().confidence
    }

    pub fn cost(self) -> Cost {
        self.meta().cost
    }

    pub fn label(self) -> &'static str {
        self.meta().label
    }

    pub fn slug(self) -> &'static str {
        self.meta().slug
    }

    /// Parse a `disabledChecks` entry. `None` is a user typo, not a bug — the
    /// caller warns and names the valid slugs.
    pub fn from_slug(slug: &str) -> Option<Self> {
        PerfFindingKind::ALL
            .into_iter()
            .find(|kind| kind.slug() == slug)
    }

    /// Every slug, comma-separated, for the "unknown check" warning.
    pub fn slug_list() -> String {
        PerfFindingKind::ALL
            .iter()
            .map(|kind| kind.slug())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Serialized through [`PerfFindingKind::slug`] rather than a derived rename,
/// so the JSON name and the `disabledChecks` name cannot drift apart.
impl Serialize for PerfFindingKind {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.slug())
    }
}

#[derive(Debug)]
pub struct PerfFinding {
    pub kind: PerfFindingKind,
    /// Fully-qualified name of the member the finding sits in.
    pub member: String,
    pub project: Option<String>,
    pub file: PathBuf,
    pub line: u32,
    pub column: u32,
    /// How many loops enclose this finding *within its own member*.
    ///
    /// Kept as a count rather than folded into [`PerfFinding::score`] because
    /// the cross-method pass raises it: once heat propagates through the call
    /// graph, a method called from inside two loops has every finding in it
    /// re-weighted, and that pass needs a number it can add to rather than a
    /// score it would have to reverse-engineer.
    pub loop_depth: u32,
    /// What was matched, in the reader's own terms — the receiver and method
    /// as written, and the loop variable where one is implicated.
    pub detail: String,
    /// One line saying what to do about it. Not optional: "quick wins"
    /// promises the fix is obvious, and a finding nobody can act on is noise.
    pub fix: String,
}

impl PerfFinding {
    pub fn confidence(&self) -> Confidence {
        self.kind.confidence()
    }

    pub fn cost(&self) -> Cost {
        self.kind.cost()
    }

    /// Estimated executions per call of the enclosing member, as a multiple.
    ///
    /// A decimal order of magnitude per loop level is the classic static
    /// execution-frequency heuristic — nobody knows the real trip count
    /// without running the code, but "one loop deeper is roughly ten times
    /// more" is right often enough to rank by.
    pub fn heat(&self) -> f64 {
        10f64.powi(self.loop_depth.min(MAX_COUNTED_DEPTH) as i32)
    }

    /// Whether this finding should fail a build.
    pub fn is_gating(&self) -> bool {
        self.confidence() == Confidence::High
    }

    /// How much runtime this finding is estimated to waste: what the pattern
    /// costs, times how often it runs. The sort key for the whole report.
    pub fn score(&self) -> f64 {
        self.cost().weight() * self.heat()
    }
}

#[derive(Debug, Default)]
pub struct PerfSummary {
    /// The scan totals count what was *eligible to be reported*, not what was
    /// parsed, so they narrow under `--exclude-tests` and the config's
    /// `ignore` globs — the same contract `roe health`'s footer follows.
    pub projects: usize,
    pub files_scanned: usize,
    pub excluded_test_projects: Vec<String>,
    pub excluded_files: usize,
    /// Findings per kind, worst-scoring kind first. Kinds that found nothing
    /// are left out rather than printed as zeroes.
    pub counts: Vec<(PerfFindingKind, usize)>,
    /// How many findings would fail a build, stated separately because the
    /// rest are advice and a reader deserves to know which is which.
    pub gating: usize,
    /// Checks the config turned off, so a reader can tell "clean" from "not
    /// looked for".
    pub disabled_checks: Vec<PerfFindingKind>,
    pub elapsed_ms: u128,
}

#[derive(Debug)]
pub struct PerfResult {
    /// Ranked worst-first by [`PerfFinding::score`].
    pub findings: Vec<PerfFinding>,
    pub summary: PerfSummary,
}

impl PerfResult {
    pub fn has_findings(&self) -> bool {
        !self.findings.is_empty()
    }

    /// Whether anything here should fail a build.
    ///
    /// Distinct from [`has_findings`](Self::has_findings) on purpose: a
    /// medium-confidence check is roe guessing without type information, and a
    /// guess that turns out wrong must not be able to break someone's CI.
    pub fn has_gating_findings(&self) -> bool {
        self.findings.iter().any(PerfFinding::is_gating)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finding(kind: PerfFindingKind, loop_depth: u32) -> PerfFinding {
        PerfFinding {
            kind,
            member: "App.Orders.Service.Run".to_string(),
            project: None,
            file: PathBuf::from("Service.cs"),
            line: 10,
            column: 5,
            loop_depth,
            detail: String::new(),
            fix: String::new(),
        }
    }

    #[test]
    fn each_loop_level_is_worth_an_order_of_magnitude() {
        assert!((finding(PerfFindingKind::UnstableClock, 0).heat() - 1.0).abs() < f64::EPSILON);
        assert!((finding(PerfFindingKind::UnstableClock, 1).heat() - 10.0).abs() < f64::EPSILON);
        assert!((finding(PerfFindingKind::UnstableClock, 2).heat() - 100.0).abs() < f64::EPSILON);
    }

    #[test]
    fn heat_stops_climbing_past_the_cap() {
        // Otherwise one deeply nested pocket of the codebase owns every slot
        // in the ranking for ever.
        let capped = finding(PerfFindingKind::UnstableClock, MAX_COUNTED_DEPTH);
        let deeper = finding(PerfFindingKind::UnstableClock, MAX_COUNTED_DEPTH + 9);

        assert!((capped.heat() - deeper.heat()).abs() < f64::EPSILON);
    }

    #[test]
    fn a_cheap_pattern_in_a_loop_outranks_a_costly_one_at_the_top() {
        // The whole premise of the command: where it runs beats what it is.
        let hot = finding(PerfFindingKind::UnstableClock, 2);
        let cold = finding(PerfFindingKind::SyncOverAsync, 0);

        assert!(hot.score() > cold.score());
    }

    #[test]
    fn at_equal_depth_the_costlier_pattern_wins() {
        let severe = finding(PerfFindingKind::SyncOverAsync, 1);
        let moderate = finding(PerfFindingKind::UnstableClock, 1);

        assert!(severe.score() > moderate.score());
    }

    #[test]
    fn an_empty_result_has_no_findings() {
        let result = PerfResult {
            findings: Vec::new(),
            summary: PerfSummary::default(),
        };

        assert!(!result.has_findings());
        assert!(!result.has_gating_findings());
    }

    #[test]
    fn confidence_does_not_move_the_ranking() {
        // A medium-confidence finding running 100× is still the most valuable
        // thing in the report.
        let medium = finding(PerfFindingKind::ListLookupInLoop, 2);
        let high = finding(PerfFindingKind::SyncOverAsync, 0);

        assert!(medium.score() > high.score());
        assert!(!medium.is_gating());
        assert!(high.is_gating());
    }

    #[test]
    fn only_high_confidence_findings_gate() {
        for kind in PerfFindingKind::ALL {
            assert_eq!(
                finding(kind, 0).is_gating(),
                kind.confidence() == Confidence::High,
                "{} gates inconsistently with its confidence",
                kind.slug()
            );
        }
    }

    #[test]
    fn medium_confidence_findings_alone_do_not_gate() {
        let result = PerfResult {
            findings: vec![
                finding(PerfFindingKind::UnstableClock, 0),
                finding(PerfFindingKind::RedundantLinq, 1),
            ],
            summary: PerfSummary::default(),
        };

        assert!(result.has_findings());
        assert!(!result.has_gating_findings());
    }

    #[test]
    fn one_high_confidence_finding_is_enough_to_gate() {
        let result = PerfResult {
            findings: vec![
                finding(PerfFindingKind::UnstableClock, 0),
                finding(PerfFindingKind::SyncOverAsync, 0),
            ],
            summary: PerfSummary::default(),
        };

        assert!(result.has_gating_findings());
    }

    #[test]
    fn every_slug_is_unique_and_round_trips() {
        let mut seen: Vec<&str> = Vec::new();

        for kind in PerfFindingKind::ALL {
            let slug = kind.slug();

            assert!(!seen.contains(&slug), "duplicate slug {slug}");
            assert_eq!(PerfFindingKind::from_slug(slug), Some(kind));

            seen.push(slug);
        }
    }

    #[test]
    fn every_label_is_unique() {
        // Labels are a report column, so two checks sharing one would make the
        // output ambiguous.
        let mut seen: Vec<&str> = Vec::new();

        for kind in PerfFindingKind::ALL {
            let label = kind.label();

            assert!(!seen.contains(&label), "duplicate label {label}");

            seen.push(label);
        }
    }

    #[test]
    fn an_unknown_slug_does_not_parse() {
        assert_eq!(PerfFindingKind::from_slug("list-lookups"), None);
    }

    #[test]
    fn a_kind_serializes_as_its_slug() {
        // JSON consumers and `disabledChecks` have to agree on one spelling.
        let json = serde_json::to_string(&PerfFindingKind::NPlusOneQuery).expect("serializes");

        assert_eq!(json, "\"n-plus-one-query\"");
    }

    #[test]
    fn the_slug_list_names_every_check() {
        let list = PerfFindingKind::slug_list();

        for kind in PerfFindingKind::ALL {
            assert!(list.contains(kind.slug()), "{} is missing", kind.slug());
        }
    }
}
