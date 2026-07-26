//! Recording today's health findings so CI can gate on tomorrow's.
//!
//! Turning `roe health` on over a codebase that didn't have it from day one
//! starts at a backlog that fails every build. A baseline writes that backlog
//! down once; from then on only findings that aren't in it are reported.
//!
//! Entries match on `(kind, name)` and deliberately not on the line number: a
//! baseline that went stale whenever a file grew by three lines would be
//! worthless as a gate. A matched finding whose metric rose above the recorded
//! one is reported anyway, since that is new debt in an old place.

use std::path::Path;

use anyhow::{Context as _, bail};
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use crate::model::{HealthFindingKind, HealthResult};

/// The only schema roe writes, and the only one it reads.
const VERSION: u32 = 1;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Baseline {
    pub version: u32,
    #[serde(default)]
    pub findings: Vec<BaselineFinding>,
    #[serde(default)]
    pub cycles: Vec<BaselineCycle>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BaselineFinding {
    pub kind: HealthFindingKind,
    pub name: String,
    /// Written for readability and diffing only. Matching ignores it, so
    /// moving a file doesn't silently un-baseline everything in it.
    pub file: String,
    pub metric: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BaselineCycle {
    /// Sorted, so a cycle whose path is printed from a different starting
    /// point still matches.
    pub members: Vec<String>,
}

/// What applying a baseline did to a result.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Applied {
    /// Findings and cycles dropped because the baseline already had them.
    pub hidden: usize,
    /// Distinct baseline entries that matched nothing in this run — debt that
    /// was fixed, renamed, or deleted since the file was written.
    pub stale: usize,
}

/// Read and validate a baseline file.
pub fn load(path: &Path) -> anyhow::Result<Baseline> {
    let display = crate::paths::display(path);

    if !path.is_file() {
        bail!(
            "baseline file not found: {display} — create one with `roe health --write-baseline {display}`"
        );
    }

    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read baseline file: {display}"))?;

    parse(path, &content)
}

/// Parse and version-check baseline text. Split from [`load`] so the parsing
/// rules are testable without touching the filesystem.
///
/// The version is read on its own first, so a file from a future roe is told
/// what to do about it rather than failing on whichever field
/// `deny_unknown_fields` happens to reach.
pub fn parse(path: &Path, content: &str) -> anyhow::Result<Baseline> {
    let display = crate::paths::display(path);

    let probe: Probe = serde_json::from_str(content)
        .with_context(|| format!("failed to parse baseline file: {display}"))?;

    if probe.version != VERSION {
        bail!(
            "unsupported baseline version {} (this roe understands version {VERSION}) — regenerate it with `roe health --write-baseline {display}`",
            probe.version
        );
    }

    serde_json::from_str(content)
        .with_context(|| format!("failed to parse baseline file: {display}"))
}

/// Just enough of the document to decide whether the rest is worth reading.
#[derive(Deserialize)]
struct Probe {
    version: u32,
}

/// Record every finding and cycle in `result`, returning how many of each was
/// written.
pub fn write(path: &Path, result: &HealthResult, root: &Path) -> anyhow::Result<(usize, usize)> {
    let display = crate::paths::display(path);
    let baseline = build(result, root);

    // Trailing newline: this file is meant to be committed, and every other
    // tool in a repository writes one.
    let json = serde_json::to_string_pretty(&baseline)
        .with_context(|| format!("failed to serialize baseline file: {display}"))?;
    std::fs::write(path, format!("{json}\n"))
        .with_context(|| format!("failed to write baseline file: {display}"))?;

    Ok((baseline.findings.len(), baseline.cycles.len()))
}

/// Drop everything `baseline` already knows about.
///
/// Duplicate keys collapse onto the highest metric recorded for them, so a
/// stray hand-edited entry can't quietly un-baseline something, and staleness
/// is counted per distinct entry rather than per line of the file.
pub fn apply(baseline: &Baseline, result: &mut HealthResult) -> Applied {
    // Keyed by kind and then by name — two owned maps rather than one keyed
    // on a `(kind, &str)` tuple, so looking a finding up borrows nothing from
    // the baseline and needs no allocation per lookup.
    let mut findings: FxHashMap<HealthFindingKind, FxHashMap<String, Entry>> = FxHashMap::default();
    for recorded in &baseline.findings {
        findings
            .entry(recorded.kind)
            .or_default()
            .entry(recorded.name.clone())
            .and_modify(|entry| entry.ceiling = entry.ceiling.max(recorded.metric))
            .or_insert(Entry {
                ceiling: recorded.metric,
                matched: false,
            });
    }

    let mut cycles: FxHashMap<String, Entry> = FxHashMap::default();
    for recorded in &baseline.cycles {
        let mut members: Vec<&str> = recorded.members.iter().map(String::as_str).collect();
        members.sort_unstable();

        cycles.entry(cycle_key(&members)).or_insert(Entry {
            ceiling: 0,
            matched: false,
        });
    }

    let mut hidden = 0;

    result.findings.retain(|finding| {
        let Some(entry) = findings
            .get_mut(&finding.kind)
            .and_then(|names| names.get_mut(finding.name.as_str()))
        else {
            return true;
        };

        entry.matched = true;

        // Worse than what was recorded is new debt in an old place, and the
        // entry that covers the old amount has no business hiding it.
        if finding.metric > entry.ceiling {
            return true;
        }

        hidden += 1;

        false
    });

    result.cycles.retain(|cycle| {
        let Some(entry) = cycles.get_mut(&cycle_key(&members_of(cycle))) else {
            return true;
        };

        entry.matched = true;
        hidden += 1;

        false
    });

    let stale = findings
        .values()
        .flat_map(FxHashMap::values)
        .chain(cycles.values())
        .filter(|entry| !entry.matched)
        .count();

    Applied { hidden, stale }
}

/// One distinct baseline key while a result is being filtered against it.
struct Entry {
    /// The highest metric recorded for this key. Unused for cycles, which
    /// have no metric.
    ceiling: u32,
    matched: bool,
}

/// The baseline a clean-slate `--write-baseline` run would produce.
///
/// Sorted by `(kind, name)` and deduplicated on the same key the way [`apply`]
/// matches, so the file diffs cleanly and says exactly what will be matched
/// against.
fn build(result: &HealthResult, root: &Path) -> Baseline {
    let mut findings: FxHashMap<(HealthFindingKind, &str), &crate::model::HealthFinding> =
        FxHashMap::default();
    for finding in &result.findings {
        findings
            .entry((finding.kind, finding.name.as_str()))
            .and_modify(|kept| {
                if finding.metric > kept.metric {
                    *kept = finding;
                }
            })
            .or_insert(finding);
    }

    let mut findings: Vec<BaselineFinding> = findings
        .into_values()
        .map(|finding| BaselineFinding {
            kind: finding.kind,
            name: finding.name.clone(),
            file: crate::paths::display(finding.file.strip_prefix(root).unwrap_or(&finding.file)),
            metric: finding.metric,
        })
        .collect();
    findings.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.name.cmp(&b.name)));

    let mut cycles: Vec<Vec<String>> = result
        .cycles
        .iter()
        .map(|cycle| {
            members_of(cycle)
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<String>>()
        })
        .collect();
    cycles.sort();
    cycles.dedup();

    Baseline {
        version: VERSION,
        findings,
        cycles: cycles
            .into_iter()
            .map(|members| BaselineCycle { members })
            .collect(),
    }
}

/// A cycle's identity: every type caught in the tangle, sorted. The printed
/// path can start anywhere in the loop and the split between `path` and
/// `others` is an artifact of how the shortest cycle was found, so neither is
/// stable enough to key on.
fn members_of(cycle: &crate::model::CircularDependency) -> Vec<&str> {
    let mut members: Vec<&str> = cycle
        .path
        .iter()
        .chain(&cycle.others)
        .map(|member| member.name.as_str())
        .collect();
    members.sort_unstable();

    members
}

/// A sorted member list as one hashable key. A newline can't occur in a C#
/// type name, so joining on one can't collide two different tangles.
fn cycle_key(members: &[&str]) -> String {
    members.join("\n")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::model::{CircularDependency, CycleMember, HealthFinding, HealthSummary};

    fn path() -> PathBuf {
        PathBuf::from("roe-baseline.json")
    }

    fn finding(
        kind: HealthFindingKind,
        name: &str,
        file: &str,
        line: u32,
        metric: u32,
    ) -> HealthFinding {
        HealthFinding {
            kind,
            name: name.to_string(),
            project: None,
            file: PathBuf::from(file),
            line,
            column: 5,
            metric,
            threshold: 10,
            breakdown: None,
            parameters: None,
        }
    }

    fn cycle(names: &[&str]) -> CircularDependency {
        CircularDependency {
            path: names
                .iter()
                .map(|name| CycleMember {
                    name: (*name).to_string(),
                    project: None,
                    file: PathBuf::from("App.cs"),
                    line: 1,
                    column: 1,
                })
                .collect(),
            others: Vec::new(),
        }
    }

    fn result(findings: Vec<HealthFinding>, cycles: Vec<CircularDependency>) -> HealthResult {
        HealthResult {
            findings,
            cycles,
            hotspots: Vec::new(),
            summary: HealthSummary::default(),
        }
    }

    fn recorded(kind: HealthFindingKind, name: &str, metric: u32) -> BaselineFinding {
        BaselineFinding {
            kind,
            name: name.to_string(),
            file: "src/App/Old.cs".to_string(),
            metric,
        }
    }

    fn baseline(findings: Vec<BaselineFinding>, cycles: Vec<BaselineCycle>) -> Baseline {
        Baseline {
            version: VERSION,
            findings,
            cycles,
        }
    }

    #[test]
    fn a_written_baseline_reads_back_as_what_was_written() {
        let analysis = result(
            vec![
                finding(
                    HealthFindingKind::HighComplexity,
                    "App.Widget.Branchy",
                    "/root/src/Widget.cs",
                    12,
                    46,
                ),
                finding(
                    HealthFindingKind::LargeType,
                    "App.Widget",
                    "/root/src/Widget.cs",
                    3,
                    34,
                ),
            ],
            vec![cycle(&["App.Order", "App.Invoice"])],
        );

        let written = build(&analysis, Path::new("/root"));
        let json = serde_json::to_string_pretty(&written).expect("a baseline serializes");
        let read = parse(&path(), &json).expect("what roe writes, roe reads");

        assert_eq!(read.version, VERSION);
        assert_eq!(read.findings.len(), 2);
        assert_eq!(read.cycles.len(), 1);
        assert_eq!(
            read.cycles[0].members,
            vec!["App.Invoice".to_string(), "App.Order".to_string()]
        );
    }

    #[test]
    fn recorded_paths_are_relative_to_the_root() {
        let analysis = result(
            vec![finding(
                HealthFindingKind::HighComplexity,
                "App.Widget.Branchy",
                "/root/src/Widget.cs",
                12,
                46,
            )],
            Vec::new(),
        );

        assert_eq!(
            build(&analysis, Path::new("/root")).findings[0].file,
            "src/Widget.cs"
        );
    }

    #[test]
    fn entries_are_written_sorted_so_the_file_diffs_cleanly() {
        let analysis = result(
            vec![
                finding(HealthFindingKind::LargeType, "App.Zeta", "z.cs", 1, 30),
                finding(
                    HealthFindingKind::HighComplexity,
                    "App.B.Two",
                    "b.cs",
                    1,
                    11,
                ),
                finding(
                    HealthFindingKind::HighComplexity,
                    "App.A.One",
                    "a.cs",
                    1,
                    12,
                ),
                finding(HealthFindingKind::LargeType, "App.Alpha", "a.cs", 1, 21),
            ],
            vec![
                cycle(&["App.Zeta", "App.Alpha"]),
                cycle(&["App.B", "App.A"]),
            ],
        );

        let written = build(&analysis, Path::new(""));
        let order: Vec<(HealthFindingKind, &str)> = written
            .findings
            .iter()
            .map(|entry| (entry.kind, entry.name.as_str()))
            .collect();

        assert_eq!(
            order,
            vec![
                (HealthFindingKind::HighComplexity, "App.A.One"),
                (HealthFindingKind::HighComplexity, "App.B.Two"),
                (HealthFindingKind::LargeType, "App.Alpha"),
                (HealthFindingKind::LargeType, "App.Zeta"),
            ]
        );
        assert_eq!(
            written.cycles[0].members,
            vec!["App.A".to_string(), "App.B".to_string()]
        );
    }

    #[test]
    fn a_recorded_finding_is_hidden_wherever_it_moved_to() {
        // Same kind and name, three lines further down a renamed file: still
        // the debt that was recorded, so still hidden.
        let mut analysis = result(
            vec![finding(
                HealthFindingKind::HighComplexity,
                "App.Widget.Branchy",
                "src/App/Renamed.cs",
                15,
                46,
            )],
            Vec::new(),
        );

        let applied = apply(
            &baseline(
                vec![recorded(
                    HealthFindingKind::HighComplexity,
                    "App.Widget.Branchy",
                    46,
                )],
                Vec::new(),
            ),
            &mut analysis,
        );

        assert_eq!(
            applied,
            Applied {
                hidden: 1,
                stale: 0
            }
        );
        assert!(analysis.findings.is_empty());
    }

    #[test]
    fn the_same_name_under_a_different_kind_is_a_different_entry() {
        let mut analysis = result(
            vec![finding(
                HealthFindingKind::LongMethod,
                "App.Widget.Branchy",
                "a.cs",
                1,
                60,
            )],
            Vec::new(),
        );

        let applied = apply(
            &baseline(
                vec![recorded(
                    HealthFindingKind::HighComplexity,
                    "App.Widget.Branchy",
                    46,
                )],
                Vec::new(),
            ),
            &mut analysis,
        );

        assert_eq!(
            applied,
            Applied {
                hidden: 0,
                stale: 1
            }
        );
        assert_eq!(analysis.findings.len(), 1);
    }

    #[test]
    fn a_finding_that_got_worse_is_reported_even_though_it_is_recorded() {
        // New debt in an old place. The entry still matched, so it is not
        // stale — it just doesn't cover what the method has become.
        let mut analysis = result(
            vec![finding(
                HealthFindingKind::HighComplexity,
                "App.Widget.Branchy",
                "a.cs",
                1,
                30,
            )],
            Vec::new(),
        );

        let applied = apply(
            &baseline(
                vec![recorded(
                    HealthFindingKind::HighComplexity,
                    "App.Widget.Branchy",
                    12,
                )],
                Vec::new(),
            ),
            &mut analysis,
        );

        assert_eq!(
            applied,
            Applied {
                hidden: 0,
                stale: 0
            }
        );
        assert_eq!(analysis.findings.len(), 1);
    }

    #[test]
    fn a_finding_that_improved_stays_hidden() {
        let mut analysis = result(
            vec![finding(
                HealthFindingKind::HighComplexity,
                "App.Widget.Branchy",
                "a.cs",
                1,
                13,
            )],
            Vec::new(),
        );

        let applied = apply(
            &baseline(
                vec![recorded(
                    HealthFindingKind::HighComplexity,
                    "App.Widget.Branchy",
                    30,
                )],
                Vec::new(),
            ),
            &mut analysis,
        );

        assert_eq!(
            applied,
            Applied {
                hidden: 1,
                stale: 0
            }
        );
        assert!(analysis.findings.is_empty());
    }

    #[test]
    fn a_cycle_matches_whatever_order_its_members_come_in() {
        let mut analysis = result(
            Vec::new(),
            vec![cycle(&["App.Invoice", "App.Order", "App.Line"])],
        );

        let applied = apply(
            &baseline(
                Vec::new(),
                vec![BaselineCycle {
                    members: vec![
                        "App.Line".to_string(),
                        "App.Order".to_string(),
                        "App.Invoice".to_string(),
                    ],
                }],
            ),
            &mut analysis,
        );

        assert_eq!(
            applied,
            Applied {
                hidden: 1,
                stale: 0
            }
        );
        assert!(analysis.cycles.is_empty());
    }

    #[test]
    fn a_cycle_that_grew_a_member_is_a_different_cycle() {
        let mut analysis = result(
            Vec::new(),
            vec![cycle(&["App.Order", "App.Invoice", "App.Tax"])],
        );

        let applied = apply(
            &baseline(
                Vec::new(),
                vec![BaselineCycle {
                    members: vec!["App.Invoice".to_string(), "App.Order".to_string()],
                }],
            ),
            &mut analysis,
        );

        assert_eq!(
            applied,
            Applied {
                hidden: 0,
                stale: 1
            }
        );
        assert_eq!(analysis.cycles.len(), 1);
    }

    #[test]
    fn an_entry_that_matches_nothing_is_stale_but_not_a_failure() {
        let mut analysis = result(Vec::new(), Vec::new());

        let applied = apply(
            &baseline(
                vec![
                    recorded(HealthFindingKind::HighComplexity, "App.Gone", 46),
                    recorded(HealthFindingKind::LargeType, "App.AlsoGone", 30),
                ],
                vec![BaselineCycle {
                    members: vec!["App.A".to_string(), "App.B".to_string()],
                }],
            ),
            &mut analysis,
        );

        assert_eq!(
            applied,
            Applied {
                hidden: 0,
                stale: 3
            }
        );
    }

    #[test]
    fn writing_a_repeated_key_records_the_worst_of_it() {
        // Two findings can share a key — a partial type measured through more
        // than one of its halves, say. Recording the lower one would baseline
        // away the worse of the two on the very next run.
        let analysis = result(
            vec![
                finding(
                    HealthFindingKind::LargeType,
                    "App.Widget",
                    "half-one.cs",
                    1,
                    12,
                ),
                finding(
                    HealthFindingKind::LargeType,
                    "App.Widget",
                    "half-two.cs",
                    1,
                    30,
                ),
            ],
            Vec::new(),
        );

        let written = build(&analysis, Path::new("/root"));

        assert_eq!(written.findings.len(), 1, "one key, one entry");
        assert_eq!(written.findings[0].metric, 30);
        assert_eq!(
            written.findings[0].file, "half-two.cs",
            "the file travels with the metric it belongs to"
        );
    }

    #[test]
    fn a_repeated_key_at_the_same_metric_changes_nothing() {
        // Nothing is worse than what is already recorded, so the entry stands
        // as written rather than being rewritten with a second file name.
        let analysis = result(
            vec![
                finding(
                    HealthFindingKind::LargeType,
                    "App.Widget",
                    "first.cs",
                    1,
                    30,
                ),
                finding(
                    HealthFindingKind::LargeType,
                    "App.Widget",
                    "second.cs",
                    1,
                    30,
                ),
            ],
            Vec::new(),
        );

        let written = build(&analysis, Path::new("/root"));

        assert_eq!(written.findings.len(), 1);
        assert_eq!(written.findings[0].file, "first.cs");
    }

    #[test]
    fn duplicate_entries_collapse_to_the_worst_recorded_metric() {
        // A hand-edited file can repeat a key. The higher metric wins, so a
        // stray low duplicate can't un-baseline something.
        let mut analysis = result(
            vec![finding(
                HealthFindingKind::HighComplexity,
                "App.Widget.Branchy",
                "a.cs",
                1,
                30,
            )],
            Vec::new(),
        );

        let applied = apply(
            &baseline(
                vec![
                    recorded(HealthFindingKind::HighComplexity, "App.Widget.Branchy", 12),
                    recorded(HealthFindingKind::HighComplexity, "App.Widget.Branchy", 30),
                ],
                Vec::new(),
            ),
            &mut analysis,
        );

        assert_eq!(
            applied,
            Applied {
                hidden: 1,
                stale: 0
            }
        );
    }

    #[test]
    fn an_unknown_field_is_rejected_rather_than_ignored() {
        let error = parse(
            &path(),
            r#"{ "version": 1, "findings": [], "cycles": [], "ignoer": [] }"#,
        )
        .expect_err("a typo must not pass silently");

        assert!(
            format!("{error:#}").starts_with("failed to parse baseline file: roe-baseline.json:"),
            "got {error:#}"
        );
    }

    #[test]
    fn an_unsupported_version_says_which_one_this_roe_understands() {
        let error = parse(&path(), r#"{ "version": 2, "findings": [] }"#)
            .expect_err("a future schema is not guessed at");

        assert_eq!(
            format!("{error:#}"),
            "unsupported baseline version 2 (this roe understands version 1) — regenerate it with `roe health --write-baseline roe-baseline.json`"
        );
    }

    #[test]
    fn a_file_without_a_version_is_a_parse_error() {
        let error =
            parse(&path(), r#"{ "findings": [] }"#).expect_err("the version is not optional");

        assert!(
            format!("{error:#}").starts_with("failed to parse baseline file: roe-baseline.json:"),
            "got {error:#}"
        );
    }

    #[test]
    fn a_baseline_that_only_lists_findings_needs_no_cycles_key() {
        let read = parse(&path(), r#"{ "version": 1, "findings": [] }"#)
            .expect("both lists default to empty");

        assert!(read.cycles.is_empty());
    }

    #[test]
    fn loading_a_path_that_is_not_there_says_how_to_create_it() {
        let error = load(Path::new("does/not/exist/roe-baseline.json"))
            .expect_err("a missing baseline is never a silent full-backlog run");

        assert_eq!(
            format!("{error:#}"),
            "baseline file not found: does/not/exist/roe-baseline.json — create one with `roe health --write-baseline does/not/exist/roe-baseline.json`"
        );
    }
}
