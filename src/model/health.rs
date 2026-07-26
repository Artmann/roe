use std::path::PathBuf;

use serde::Serialize;

use crate::model::MemberKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum HealthFindingKind {
    HighComplexity,
    HighCognitiveComplexity,
    LongMethod,
    TooManyParameters,
    LargeFile,
    LargeType,
}

/// What a [`HealthFindingKind::LargeType`]'s members actually are. A type with
/// thirty auto-properties is a data holder; one with thirty methods is a god
/// class. The count alone can't tell them apart, so the breakdown travels with
/// the finding and the reader judges.
#[derive(Debug, Default, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberBreakdown {
    pub methods: u32,
    pub properties: u32,
    pub fields: u32,
    pub events: u32,
}

impl MemberBreakdown {
    /// Bucket one member. Returns false for kinds that deliberately don't
    /// count towards a type's size: an enum's cases are the enum's whole
    /// point, and counting them would report a 25-case enum as a god class.
    pub fn record(&mut self, kind: MemberKind) -> bool {
        match kind {
            MemberKind::Constructor
            | MemberKind::ConversionOperator
            | MemberKind::Destructor
            | MemberKind::Indexer
            | MemberKind::Method
            | MemberKind::Operator
            | MemberKind::StaticConstructor => self.methods += 1,
            MemberKind::Property => self.properties += 1,
            MemberKind::Field => self.fields += 1,
            MemberKind::Event => self.events += 1,
            MemberKind::EnumMember => return false,
        }

        true
    }

    /// The member count the `large type` threshold is compared against — the
    /// buckets, and nothing that [`record`](Self::record) turned away.
    pub fn total(&self) -> u32 {
        self.methods + self.properties + self.fields + self.events
    }

    /// Largest category first, so the dominant kind leads the description.
    /// Empty categories are dropped rather than printed as zeroes.
    pub fn describe(&self) -> String {
        let mut parts: Vec<(u32, &str)> = vec![
            (self.methods, "method"),
            (self.properties, "property"),
            (self.fields, "field"),
            (self.events, "event"),
        ];
        parts.retain(|&(count, _)| count > 0);
        parts.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)));

        parts
            .iter()
            .map(|&(count, noun)| {
                let plural = match noun {
                    "property" => "properties".to_string(),
                    other => format!("{other}s"),
                };

                format!(
                    "{count} {}",
                    if count == 1 { noun.to_string() } else { plural }
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[derive(Debug)]
pub struct HealthFinding {
    pub kind: HealthFindingKind,
    /// Fully-qualified member/type name for symbol-level findings, relative
    /// path display for file-level findings.
    pub name: String,
    pub project: Option<String>,
    pub file: PathBuf,
    pub line: u32,
    pub column: u32,
    /// The measured value that tripped the threshold — cyclomatic
    /// complexity, line count, parameter count, or member count, depending
    /// on `kind`.
    pub metric: u32,
    pub threshold: u32,
    /// Populated for [`HealthFindingKind::LargeType`] only.
    pub breakdown: Option<MemberBreakdown>,
}

impl HealthFinding {
    /// How far past its threshold this finding sits, as a multiple. The
    /// ordering key for severity-sorted output — comparable across kinds in a
    /// way the raw metric is not, since 46 branches and 242 lines trip limits
    /// of wildly different magnitudes.
    pub fn severity(&self) -> f64 {
        if self.threshold == 0 {
            return f64::from(self.metric);
        }

        f64::from(self.metric) / f64::from(self.threshold)
    }
}

/// One type participating in a circular dependency.
#[derive(Debug)]
pub struct CycleMember {
    pub name: String,
    pub project: Option<String>,
    pub file: PathBuf,
    pub line: u32,
    pub column: u32,
}

/// A set of types whose type-level dependencies form a cycle.
#[derive(Debug)]
pub struct CircularDependency {
    /// A real cycle through the component: consecutive entries are joined by
    /// an actual reference edge, and the last entry references the first.
    pub path: Vec<CycleMember>,
    /// Other types in the same strongly-connected component that the shortest
    /// cycle does not pass through. They are still part of the tangle, but
    /// printing them in the arrow chain would imply edges that may not exist.
    pub others: Vec<CycleMember>,
}

/// A file that is both densely complex and frequently changed — the
/// intersection where defects concentrate and changes are expensive.
#[derive(Debug)]
pub struct Hotspot {
    pub file: PathBuf,
    pub project: Option<String>,
    /// 0–100, normalized within the workspace so the riskiest file scores
    /// close to 100. Only comparable against other files in the same run.
    pub score: f64,
    /// Recency-weighted commit count, on a 90-day half-life.
    pub weighted_commits: f64,
    /// Summed cyclomatic complexity of every member declared in the file.
    pub cyclomatic: u32,
    pub lines: u32,
    /// `cyclomatic / lines` — how tightly packed the complexity is.
    pub complexity_density: f64,
}

#[derive(Debug, Default)]
pub struct HealthSummary {
    pub projects: usize,
    pub files_scanned: usize,
    pub symbols: usize,
    pub high_complexity: usize,
    pub high_cognitive_complexity: usize,
    pub long_methods: usize,
    pub too_many_parameters: usize,
    pub large_files: usize,
    pub large_types: usize,
    pub circular_dependencies: usize,
    /// Commits walked for hotspot scoring. `None` when `--hotspots` was not
    /// requested, which is what distinguishes "not asked for" from "no
    /// history".
    pub commits_walked: Option<usize>,
    pub elapsed_ms: u128,
}

#[derive(Debug)]
pub struct HealthResult {
    pub findings: Vec<HealthFinding>,
    pub cycles: Vec<CircularDependency>,
    /// Ranked highest-risk first. Empty unless `--hotspots` was requested.
    /// Informational only — hotspots never affect the exit code, since every
    /// codebase has a riskiest file and that is not a failure.
    pub hotspots: Vec<Hotspot>,
    pub summary: HealthSummary,
}

impl HealthResult {
    /// Whether this analysis has anything to report — the single definition of
    /// "not clean", used both for this command's exit code and for the combined
    /// `check` run's. Hotspots are excluded deliberately: every codebase has a
    /// riskiest file, and that is not a failure.
    pub fn has_findings(&self) -> bool {
        !self.findings.is_empty() || !self.cycles.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enum_members_do_not_count_towards_type_size() {
        let mut breakdown = MemberBreakdown::default();
        assert!(!breakdown.record(MemberKind::EnumMember));
        assert_eq!(breakdown.total(), 0);
    }

    #[test]
    fn method_like_kinds_all_land_in_the_method_bucket() {
        let mut breakdown = MemberBreakdown::default();
        for kind in [
            MemberKind::Constructor,
            MemberKind::Indexer,
            MemberKind::Method,
            MemberKind::Operator,
        ] {
            assert!(breakdown.record(kind));
        }

        assert_eq!(breakdown.methods, 4);
        assert_eq!(breakdown.total(), 4);
    }

    #[test]
    fn each_kind_lands_in_its_own_bucket_and_total_sums_all_four() {
        // Distinct counts per bucket, so a total that dropped or duplicated
        // one of them can't coincidentally still come out right.
        let mut breakdown = MemberBreakdown::default();
        for (kind, times) in [
            (MemberKind::Method, 4),
            (MemberKind::Property, 3),
            (MemberKind::Field, 2),
            (MemberKind::Event, 1),
        ] {
            for _ in 0..times {
                assert!(breakdown.record(kind));
            }
        }

        assert_eq!(breakdown.methods, 4);
        assert_eq!(breakdown.properties, 3);
        assert_eq!(breakdown.fields, 2);
        assert_eq!(breakdown.events, 1);
        assert_eq!(breakdown.total(), 10);
    }

    #[test]
    fn describe_leads_with_the_dominant_kind_and_skips_empty_ones() {
        let breakdown = MemberBreakdown {
            methods: 8,
            properties: 26,
            fields: 0,
            events: 1,
        };

        assert_eq!(breakdown.describe(), "26 properties, 8 methods, 1 event");
    }

    #[test]
    fn severity_is_the_multiple_of_the_threshold() {
        let finding = HealthFinding {
            kind: HealthFindingKind::HighComplexity,
            name: "App.Foo.Bar".to_string(),
            project: None,
            file: PathBuf::from("Foo.cs"),
            line: 1,
            column: 1,
            metric: 46,
            threshold: 10,
            breakdown: None,
        };

        assert!((finding.severity() - 4.6).abs() < f64::EPSILON);
    }
}
