use std::path::PathBuf;

use serde::Serialize;

use crate::model::{MemberKind, Modifiers};

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
    /// Bucket one member. Returns false for members that deliberately don't
    /// count towards a type's size:
    ///
    /// - An enum's cases are the enum's whole point, and counting them would
    ///   report a 25-case enum as a god class.
    /// - A `const` field is a name for a literal, inlined at every call site.
    ///   It has no behaviour and no runtime state, so a type holding fifty of
    ///   them is a lookup table rather than a cohesion problem.
    ///
    /// `static readonly` is *not* exempt. roe has no type analysis, so it
    /// cannot tell a `static readonly float` tuning value from a
    /// `static readonly HttpClient` the type genuinely depends on.
    pub fn record(&mut self, kind: MemberKind, modifiers: Modifiers) -> bool {
        match kind {
            MemberKind::Constructor
            | MemberKind::ConversionOperator
            | MemberKind::Destructor
            | MemberKind::Indexer
            | MemberKind::Method
            | MemberKind::Operator
            | MemberKind::StaticConstructor => self.methods += 1,
            MemberKind::Property => self.properties += 1,
            MemberKind::Field => {
                if modifiers.contains(Modifiers::CONST) {
                    return false;
                }

                self.fields += 1;
            }
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

/// What a [`HealthFindingKind::TooManyParameters`] finding's parameter list is
/// actually made of. The limit exists to bound call-site burden — how much a
/// caller has to supply, and how much they have to keep straight while doing
/// it — so only `required` is measured against it. The rest travels with the
/// finding so a long declaration stays visible in the report.
#[derive(Debug, Default, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParameterBreakdown {
    /// Parameters the caller has to supply and think about: no default value
    /// and not `out`. `ref`, `in`, and an extension method's `this` receiver
    /// all land here — each is still an argument written at the call site.
    pub required: u32,
    /// Defaulted parameters and `params` arrays. Omitting either is legal, so
    /// neither costs the caller anything.
    pub optional: u32,
    /// `out` parameters — return values wearing a parameter's clothes. The
    /// caller supplies nothing.
    pub out: u32,
}

impl ParameterBreakdown {
    /// The declared parameter count, which is what the source signature shows
    /// and what the `/arity` overload suffix has to match.
    pub fn total(&self) -> u32 {
        self.required + self.optional + self.out
    }

    /// The declared signature spelled out, or an empty string when every
    /// parameter is required — `6/5 params (6 declared: 6 required)` says the
    /// same thing twice. Empty categories are dropped.
    pub fn describe(&self) -> String {
        if self.optional == 0 && self.out == 0 {
            return String::new();
        }

        // `required` is the metric being explained, so it is always named even
        // at zero; the other two only appear when they are the reason the
        // metric and the declared total disagree.
        let mut parts = vec![format!("{} required", self.required)];

        if self.optional > 0 {
            parts.push(format!("{} optional", self.optional));
        }
        if self.out > 0 {
            parts.push(format!("{} out", self.out));
        }

        format!("{} declared: {}", self.total(), parts.join(", "))
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
    /// Populated for [`HealthFindingKind::TooManyParameters`] only. `metric`
    /// is this breakdown's `required` count; the rest of the declaration lives
    /// here.
    pub parameters: Option<ParameterBreakdown>,
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
    /// The three scan totals count what was *eligible to be reported*, not
    /// what was parsed — so they narrow under `--exclude-tests` and the
    /// config's `ignore` globs. A footer that reported the same numbers either
    /// way could not tell a reader whether their setting took effect.
    pub projects: usize,
    pub files_scanned: usize,
    pub symbols: usize,
    /// Test projects skipped under `--exclude-tests`, named so a reader can
    /// also confirm their project was recognized as a test project at all.
    /// Empty when the flag is off.
    pub excluded_test_projects: Vec<String>,
    /// Non-generated files dropped by an `ignore` glob, over and above the
    /// ones already dropped with their test project.
    pub excluded_files: usize,
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
        assert!(!breakdown.record(MemberKind::EnumMember, Modifiers::empty()));
        assert_eq!(breakdown.total(), 0);
    }

    #[test]
    fn const_fields_do_not_count_towards_type_size() {
        // A `const` is a name for a literal, inlined at every call site. It
        // carries no behaviour and no runtime state, so a registry of tuning
        // values is a lookup table rather than a god class.
        let mut breakdown = MemberBreakdown::default();
        assert!(!breakdown.record(
            MemberKind::Field,
            Modifiers::PUBLIC | Modifiers::CONST | Modifiers::STATIC
        ));

        assert_eq!(breakdown.fields, 0);
        assert_eq!(breakdown.total(), 0);
        assert_eq!(breakdown.describe(), "");
    }

    #[test]
    fn plain_and_static_readonly_fields_still_count() {
        // roe has no type analysis, so it cannot tell a `static readonly`
        // tuning value from a `static readonly HttpClient` the type genuinely
        // depends on. Only `const` is unambiguous enough to exempt.
        let mut breakdown = MemberBreakdown::default();
        assert!(breakdown.record(MemberKind::Field, Modifiers::PRIVATE));
        assert!(breakdown.record(
            MemberKind::Field,
            Modifiers::PRIVATE | Modifiers::STATIC | Modifiers::READONLY
        ));

        assert_eq!(breakdown.fields, 2);
        assert_eq!(breakdown.total(), 2);
    }

    #[test]
    fn a_const_modifier_only_exempts_fields() {
        // Nothing else in C# can be `const`, but the guard must not leak into
        // another bucket if a future extractor ever sets the flag elsewhere.
        let mut breakdown = MemberBreakdown::default();
        assert!(breakdown.record(MemberKind::Method, Modifiers::CONST));

        assert_eq!(breakdown.methods, 1);
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
            assert!(breakdown.record(kind, Modifiers::empty()));
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
                assert!(breakdown.record(kind, Modifiers::empty()));
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
    fn a_parameter_breakdown_totals_the_declared_signature() {
        let parameters = ParameterBreakdown {
            required: 8,
            optional: 2,
            out: 1,
        };

        assert_eq!(parameters.total(), 11);
        assert_eq!(
            parameters.describe(),
            "11 declared: 8 required, 2 optional, 1 out"
        );
    }

    #[test]
    fn a_parameter_breakdown_drops_the_categories_it_has_none_of() {
        let parameters = ParameterBreakdown {
            required: 4,
            optional: 0,
            out: 2,
        };

        assert_eq!(parameters.describe(), "6 declared: 4 required, 2 out");
    }

    #[test]
    fn an_all_required_signature_has_nothing_to_explain() {
        // `6/5 params (6 declared: 6 required)` says the same thing twice.
        let parameters = ParameterBreakdown {
            required: 6,
            optional: 0,
            out: 0,
        };

        assert_eq!(parameters.total(), 6);
        assert_eq!(parameters.describe(), "");
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
            parameters: None,
        };

        assert!((finding.severity() - 4.6).abs() < f64::EPSILON);
    }
}
