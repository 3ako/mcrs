use crate::floors;
use crate::mutation::outcomes::{self, Class, Measured};
use crate::mutation::plan::{self, Plan};
use crate::scope;
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::LazyLock;

#[derive(Debug, PartialEq)]
pub(crate) struct Survivor {
    pub package: String,
    pub file: String,
    pub line: u32,
    pub change: String,
}

#[derive(Debug, Default, PartialEq)]
pub(crate) struct Counts {
    pub caught: usize,
    pub timeout: usize,
    pub missed: usize,
    pub unviable: usize,
}

#[derive(Debug, PartialEq)]
pub(crate) struct Sweep {
    pub package: String,
    pub floor: u64,
    pub counts: Counts,
}

impl Sweep {
    fn score(&self) -> Option<u64> {
        outcomes::score(
            self.counts.caught as u64,
            self.counts.timeout as u64,
            self.counts.missed as u64,
        )
    }

    fn shortfall(&self) -> Option<String> {
        let floor = floors::show(self.floor);
        let package = &self.package;
        let Some(score) = self.score() else {
            return Some(format!(
                "{package}: the sweep counted no mutant, so it cannot meet the floor {floor}; the crate needs mutants and tests that run them"
            ));
        };
        (score < self.floor).then(|| {
            format!(
                "{package}: the sweep scores {} against the floor {floor} (caught {}, timed out {}, missed {}); if the change deleted tests, keep the ones that hold the floor, otherwise add tests that kill the surviving mutants of {package}",
                floors::show(score),
                self.counts.caught,
                self.counts.timeout,
                self.counts.missed
            )
        })
    }
}

#[derive(Debug)]
pub(crate) struct Judged {
    pub counts: Counts,
    pub survivors: Vec<Survivor>,
    pub sweeps: Vec<Sweep>,
}

static NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?P<file>.+?):(?P<line>\d+):\d+: (?P<change>.*)$")
        .expect("the mutant name pattern is a valid regular expression")
});

fn split_name(name: &str) -> Option<(&str, u32, &str)> {
    let found = NAME.captures(name)?;
    let (_, [file, line, change]) = found.extract();
    Some((file, line.parse().ok()?, change))
}

const REPORTED_PROBLEMS: usize = 5;

fn joined(mut problems: Vec<String>) -> String {
    let more = problems.len().saturating_sub(REPORTED_PROBLEMS);
    problems.truncate(REPORTED_PROBLEMS);
    if more > 0 {
        problems.push(format!("and {more} more"));
    }
    problems.join("; ")
}

fn incomplete(plan: &Plan, results: &BTreeMap<usize, Vec<Measured>>) -> Vec<String> {
    let mut problems: Vec<String> = (0..plan.shards.len())
        .filter(|index| !results.contains_key(index))
        .map(|index| format!("shard {index} is missing"))
        .collect();
    for (unit_index, unit) in plan.units.iter().enumerate() {
        let shards: Vec<usize> = (0..plan.shards.len())
            .filter(|&index| plan.shards[index].unit == unit_index)
            .collect();
        if shards.iter().any(|index| !results.contains_key(index)) {
            continue;
        }
        let mut ran: BTreeMap<&str, usize> = BTreeMap::new();
        for mutant in shards.iter().flat_map(|index| &results[index]) {
            *ran.entry(&mutant.name).or_default() += 1;
        }
        let expected: BTreeSet<&str> = unit.names.iter().map(String::as_str).collect();
        let package = &unit.package;
        for name in expected.iter().filter(|name| !ran.contains_key(*name)) {
            problems.push(format!("{package}: {name} was not run"));
        }
        for (name, count) in &ran {
            if !expected.contains(name) {
                problems.push(format!(
                    "{package}: {name} was run, but the plan does not list it"
                ));
            } else if *count > 1 {
                problems.push(format!("{package}: {name} was run twice"));
            }
        }
    }
    problems
}

pub(crate) fn judge(
    plan: &Plan,
    head: &str,
    results: &BTreeMap<usize, Vec<Measured>>,
) -> Result<Judged, String> {
    let mut problems = Vec::new();
    if head != plan.head {
        problems.push(format!(
            "the head {} differs from the plan's {}; plan again",
            scope::short(head),
            scope::short(&plan.head)
        ));
    }
    problems.extend(incomplete(plan, results));
    if !problems.is_empty() {
        return Err(joined(problems));
    }
    let mut judged = Judged {
        counts: Counts::default(),
        survivors: Vec::new(),
        sweeps: Vec::new(),
    };
    let mut sweeps: BTreeMap<usize, Sweep> = plan
        .units
        .iter()
        .enumerate()
        .filter_map(|(at, unit)| {
            Some((
                at,
                Sweep {
                    package: unit.package.clone(),
                    floor: unit.floor?,
                    counts: Counts::default(),
                },
            ))
        })
        .collect();
    for (index, measured) in results {
        let unit = plan.shards[*index].unit;
        let package = &plan.units[unit].package;
        let counts = match sweeps.get_mut(&unit) {
            Some(sweep) => &mut sweep.counts,
            None => &mut judged.counts,
        };
        let diff_unit = plan.units[unit].floor.is_none();
        for mutant in measured {
            match mutant.class {
                Class::Caught => counts.caught += 1,
                Class::Timeout => counts.timeout += 1,
                Class::Unviable => counts.unviable += 1,
                Class::Missed => {
                    counts.missed += 1;
                    if !diff_unit {
                        continue;
                    }
                    let (file, line, change) = split_name(&mutant.name)
                        .ok_or_else(|| format!("cannot read the mutant name {}", mutant.name))?;
                    judged.survivors.push(Survivor {
                        package: package.clone(),
                        file: file.to_owned(),
                        line,
                        change: change.to_owned(),
                    });
                }
            }
        }
    }
    judged.sweeps = sweeps.into_values().collect();
    Ok(judged)
}

pub(crate) fn annotation(survivor: &Survivor) -> String {
    annotate(
        &survivor.file,
        survivor.line,
        "Surviving mutant",
        &survivor.change,
    )
}

fn annotate(file: &str, line: u32, title: &str, message: &str) -> String {
    format!(
        "::error file={},line={line},endLine={line},title={}::{}",
        crate::escape_property(file),
        crate::escape_property(title),
        crate::escape_data(message)
    )
}

fn cell(text: &str) -> String {
    text.replace(['\r', '\n'], " ")
        .replace('|', "\\|")
        .replace('`', "'")
}

pub(crate) fn summary(plan: &Plan, judged: &Judged) -> String {
    let counts = &judged.counts;
    let mut text = format!(
        "### Mutants in changed code\n\n{} listed: {} caught, {} timed out, {} unviable, {} survived.\n",
        plan.total_mutants(),
        counts.caught,
        counts.timeout,
        counts.unviable,
        counts.missed
    );
    if !plan.in_force {
        text.push_str(&format!(
            "\nThe gate is not in force yet at {}, so survivors are listed and do not fail the run.\n",
            scope::short(&plan.base)
        ));
    }
    if !plan.broken.is_empty() {
        text.push_str("\nChanged lines in a function body that list no mutant:\n\n");
        for broken in &plan.broken {
            text.push_str(&format!(
                "- {} line {} ({})\n",
                cell(&broken.file),
                broken.line,
                cell(&broken.package)
            ));
        }
    }
    if !judged.sweeps.is_empty() {
        text.push_str("\n### Mutation floors\n\n| crate | floor | score | caught | timed out | missed |\n|---|---|---|---|---|---|\n");
        for sweep in &judged.sweeps {
            text.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} |\n",
                cell(&sweep.package),
                floors::show(sweep.floor),
                sweep
                    .score()
                    .map_or_else(|| "none".to_owned(), floors::show),
                sweep.counts.caught,
                sweep.counts.timeout,
                sweep.counts.missed
            ));
        }
    }
    if !judged.survivors.is_empty() {
        text.push_str("\n| crate | file | line | change |\n|---|---|---|---|\n");
        for survivor in &judged.survivors {
            text.push_str(&format!(
                "| {} | {} | {} | `{}` |\n",
                cell(&survivor.package),
                cell(&survivor.file),
                survivor.line,
                cell(&survivor.change)
            ));
        }
    }
    text
}

fn locations(lines: impl Iterator<Item = String>) -> String {
    joined(lines.collect())
}

pub(crate) fn report(
    plan: &Plan,
    head: &str,
    results: &BTreeMap<usize, Vec<Measured>>,
) -> Result<String, String> {
    let judged = judge(plan, head, results)?;
    let total = plan.total_mutants();
    crate::append_summary(&summary(plan, &judged));
    let mut problems: Vec<String> = judged.sweeps.iter().filter_map(Sweep::shortfall).collect();
    if !plan.in_force {
        if !problems.is_empty() {
            return Err(joined(problems));
        }
        return Ok(format!(
            "not in force yet at {}; {total} mutants in changed code, {} survive",
            scope::short(&plan.base),
            judged.counts.missed
        ));
    }
    if !judged.survivors.is_empty() {
        for survivor in &judged.survivors {
            println!("{}", annotation(survivor));
        }
        problems.push(format!(
            "{} of {total} mutants survive in changed code: {}",
            judged.survivors.len(),
            locations(
                judged
                    .survivors
                    .iter()
                    .map(|s| format!("{}:{} {}", s.file, s.line, s.change))
            )
        ));
    }
    if !plan.broken.is_empty() {
        for broken in &plan.broken {
            println!(
                "{}",
                annotate(
                    &broken.file,
                    broken.line,
                    "No mutant listed",
                    "this changed line lies in a function body, but the comparison lists no mutant for it"
                )
            );
        }
        problems.push(format!(
            "{} changed lines lie in a function body but list no mutant, so the comparison is broken (a shallow fetch, a wrong base or a diff mismatch): {}",
            plan.broken.len(),
            locations(plan.broken.iter().map(|b| format!("{}:{}", b.file, b.line)))
        ));
    }
    if !problems.is_empty() {
        return Err(problems.join("; "));
    }
    let held = match judged.sweeps.len() {
        0 => String::new(),
        n => format!("; {n} swept crates at or above their floors"),
    };
    if total == 0 {
        return Ok(format!("no mutant in changed code{held}"));
    }
    Ok(format!(
        "{total} mutants in changed code: {} caught, {} timed out, {} unviable{held}",
        judged.counts.caught, judged.counts.timeout, judged.counts.unviable
    ))
}

pub(crate) fn read_results(
    dir: &Path,
    plan: &Plan,
) -> Result<BTreeMap<usize, Vec<Measured>>, String> {
    let mut results = BTreeMap::new();
    let mut problems = Vec::new();
    for index in 0..plan.shards.len() {
        match read_shard(&dir.join(index.to_string()).join("mutants.out")) {
            Ok(Some(measured)) => {
                results.insert(index, measured);
            }
            Ok(None) => {}
            Err(problem) => problems.push(problem),
        }
    }
    if problems.is_empty() {
        Ok(results)
    } else {
        Err(joined(problems))
    }
}

pub(crate) fn check(repo: &Path, head: &str) -> Result<String, String> {
    let plan = plan::read_plan(repo)?;
    let results = read_results(&repo.join("target/mutation/shards"), &plan)?;
    report(&plan, head, &results)
}

fn read_shard(out: &Path) -> Result<Option<Vec<Measured>>, String> {
    if !out.join("outcomes.json").exists() {
        return Ok(None);
    }
    outcomes::load_measured(out).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mutation::plan::{Baseline, ShardPlan, Unit};

    type Results = BTreeMap<usize, Vec<Measured>>;

    fn plan_of(units: &[(&str, &[&str])], shards: &[(usize, u32, u32)]) -> Plan {
        Plan {
            base: "b".repeat(40),
            head: "a".repeat(40),
            in_force: true,
            baseline: Baseline::Skip,
            diff: String::new(),
            units: units
                .iter()
                .map(|(package, names)| Unit {
                    package: (*package).to_owned(),
                    test_packages: vec![(*package).to_owned()],
                    names: names.iter().map(|n| (*n).to_owned()).collect(),
                    floor: None,
                })
                .collect(),
            shards: shards
                .iter()
                .map(|&(unit, index, count)| ShardPlan { unit, index, count })
                .collect(),
            broken: Vec::new(),
            surface: Vec::new(),
        }
    }

    fn measured(name: &str, class: Class) -> Measured {
        Measured {
            name: name.to_owned(),
            class,
            log_path: String::new(),
        }
    }

    fn results(rows: &[(usize, &[(&str, Class)])]) -> Results {
        rows.iter()
            .map(|(index, found)| {
                (
                    *index,
                    found
                        .iter()
                        .map(|(name, class)| measured(name, *class))
                        .collect(),
                )
            })
            .collect()
    }

    const ONE: &str = "src/a.rs:3:5: replace f -> bool with true";
    const TWO: &str = "src/a.rs:9:5: replace g -> bool with false";
    const THREE: &str = "src/a.rs:20:5: replace h -> bool with true";

    #[test]
    fn a_missing_or_partial_shard_fails_the_verdict() {
        let plan = plan_of(&[("a", &[ONE, TWO])], &[(0, 0, 2), (0, 1, 2)]);
        let head = "a".repeat(40);
        let complete = results(&[(0, &[(ONE, Class::Caught)]), (1, &[(TWO, Class::Missed)])]);
        let judged = judge(&plan, &head, &complete).unwrap();
        assert_eq!(
            judged.counts,
            Counts {
                caught: 1,
                missed: 1,
                ..Counts::default()
            }
        );
        assert_eq!(
            judged.survivors,
            [Survivor {
                package: "a".to_owned(),
                file: "src/a.rs".to_owned(),
                line: 9,
                change: "replace g -> bool with false".to_owned(),
            }]
        );

        let other_head = "0".repeat(40);
        let rows: Vec<(&str, &str, Results, Vec<&str>)> = vec![
            (
                "a missing shard",
                &head,
                results(&[(0, &[(ONE, Class::Caught)])]),
                vec!["shard 1 is missing"],
            ),
            (
                "a shard that lacks an expected mutant",
                &head,
                results(&[(0, &[(ONE, Class::Caught)]), (1, &[])]),
                vec![TWO, "was not run"],
            ),
            (
                "a shard with a mutant the plan does not list",
                &head,
                results(&[
                    (0, &[(ONE, Class::Caught)]),
                    (1, &[(TWO, Class::Caught), (THREE, Class::Caught)]),
                ]),
                vec![THREE, "does not list"],
            ),
            (
                "a mutant run by two shards",
                &head,
                results(&[
                    (0, &[(ONE, Class::Caught)]),
                    (1, &[(TWO, Class::Caught), (ONE, Class::Caught)]),
                ]),
                vec![ONE, "twice"],
            ),
            (
                "a head that is not the plan's",
                &other_head,
                results(&[(0, &[(ONE, Class::Caught)]), (1, &[(TWO, Class::Caught)])]),
                vec!["differs from the plan"],
            ),
        ];
        let mut problems = Vec::new();
        for (name, head, found, parts) in rows {
            match judge(&plan, head, &found) {
                Ok(_) => problems.push(format!("{name}: the verdict passed")),
                Err(message) => {
                    for part in parts {
                        if !message.contains(part) {
                            problems.push(format!("{name}: {part} not in {message}"));
                        }
                    }
                }
            }
        }
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    fn sweep_names(count: usize) -> Vec<String> {
        (0..count)
            .map(|n| format!("src/a.rs:{}:5: replace f{n} -> bool with true", n + 1))
            .collect()
    }

    fn swept(
        floor: u64,
        outcomes: &[(Class, usize)],
        in_force: bool,
        shards: u32,
        drop_shard: Option<usize>,
    ) -> Result<String, String> {
        let total: usize = outcomes.iter().map(|(_, n)| n).sum();
        let names = sweep_names(total);
        let mut plan = plan_of(&[("low", &[])], &[]);
        plan.in_force = in_force;
        plan.units[0].names = names.clone();
        plan.units[0].floor = Some(floor);
        plan.shards = (0..shards)
            .map(|index| ShardPlan {
                unit: 0,
                index,
                count: shards,
            })
            .collect();
        let mut classes = outcomes
            .iter()
            .flat_map(|(class, n)| std::iter::repeat_n(*class, *n));
        let mut found: Results = (0..shards as usize).map(|i| (i, Vec::new())).collect();
        for (n, name) in names.iter().enumerate() {
            let class = classes.next().unwrap();
            found
                .get_mut(&(n % shards as usize))
                .unwrap()
                .push(measured(name, class));
        }
        if let Some(index) = drop_shard {
            found.remove(&index);
        }
        report(&plan, &"a".repeat(40), &found)
    }

    #[test]
    fn a_sweep_below_its_floor_fails() {
        use Class::{Caught, Missed, Timeout, Unviable};
        struct Row {
            name: &'static str,
            floor: u64,
            outcomes: Vec<(Class, usize)>,
            in_force: bool,
            shards: u32,
            drop_shard: Option<usize>,
            fails_with: Vec<&'static str>,
        }
        let row = |name, floor, outcomes, fails_with| Row {
            name,
            floor,
            outcomes,
            in_force: true,
            shards: 1,
            drop_shard: None,
            fails_with,
        };
        let rows = vec![
            row(
                "82 hundredths against a floor of 0.83",
                83,
                vec![(Caught, 82), (Missed, 18)],
                vec!["low", "0.82", "0.83"],
            ),
            row(
                "83 hundredths against a floor of 0.83",
                83,
                vec![(Caught, 83), (Missed, 17)],
                vec![],
            ),
            row(
                "a timeout counts as caught",
                83,
                vec![(Caught, 82), (Timeout, 1), (Missed, 17)],
                vec![],
            ),
            row(
                "an unviable mutant is not counted",
                83,
                vec![(Caught, 83), (Missed, 17), (Unviable, 40)],
                vec![],
            ),
            row(
                "a score above the floor with survivors",
                83,
                vec![(Caught, 90), (Missed, 10)],
                vec![],
            ),
            row(
                "a sweep that counted no mutant",
                83,
                vec![(Unviable, 3)],
                vec!["low", "counted no mutant", "0.83"],
            ),
            Row {
                in_force: false,
                ..row(
                    "a shortfall when the gate is not in force",
                    83,
                    vec![(Caught, 82), (Missed, 18)],
                    vec!["low", "0.82", "0.83"],
                )
            },
            Row {
                shards: 2,
                drop_shard: Some(1),
                ..row(
                    "an incomplete sweep",
                    83,
                    vec![(Caught, 100)],
                    vec!["shard 1 is missing"],
                )
            },
        ];
        let mut wrong = Vec::new();
        for Row {
            name,
            floor,
            outcomes,
            in_force,
            shards,
            drop_shard,
            fails_with,
        } in rows
        {
            let found = swept(floor, &outcomes, in_force, shards, drop_shard);
            match (found, fails_with.is_empty()) {
                (Ok(_), true) => {}
                (Err(message), false) => {
                    for part in fails_with {
                        if !message.contains(part) {
                            wrong.push(format!("{name}: {part} not in {message}"));
                        }
                    }
                }
                (found, _) => wrong.push(format!("{name}: unexpected {found:?}")),
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    #[test]
    fn surviving_mutant_annotations_escape_their_text() {
        let first = Survivor {
            package: "c".to_owned(),
            file: "src/a,b.rs".to_owned(),
            line: 12,
            change: "replace 100% : x, y\n::error::forged".to_owned(),
        };
        assert_eq!(
            annotation(&first),
            "::error file=src/a%2Cb.rs,line=12,endLine=12,title=Surviving mutant::replace 100%25 : x, y%0A::error::forged"
        );
        let plan = plan_of(&[("c", &[ONE])], &[(0, 0, 1)]);
        let second = Survivor {
            package: "c".to_owned(),
            file: "src/b.rs".to_owned(),
            line: 4,
            change: "replace a | b with `c`".to_owned(),
        };
        let judged = Judged {
            counts: Counts {
                missed: 2,
                ..Counts::default()
            },
            survivors: vec![first, second],
            sweeps: Vec::new(),
        };
        let table = summary(&plan, &judged);
        let rows: Vec<&str> = table.lines().filter(|l| l.starts_with('|')).collect();
        assert_eq!(rows.len(), 4, "{table}");
        assert!(
            rows[2].contains("src/a,b.rs") && rows[2].contains("12"),
            "{table}"
        );
        assert!(
            rows[3].contains("src/b.rs") && rows[3].contains('4'),
            "{table}"
        );
        assert!(
            !rows[3].contains("a | b"),
            "an unescaped bar splits the row: {table}"
        );
        for row in &rows[2..] {
            assert!(row.starts_with("| c |"), "{row}");
        }
    }

    #[test]
    fn a_report_not_in_force_lists_survivors_without_failing() {
        let mut plan = plan_of(&[("a", &[ONE])], &[(0, 0, 1)]);
        plan.in_force = false;
        let head = "a".repeat(40);
        let found = results(&[(0, &[(ONE, Class::Missed)])]);
        let message = report(&plan, &head, &found).unwrap();
        assert!(
            message.starts_with("not in force yet at bbbbbbbbb; "),
            "{message}"
        );
        assert!(
            message.contains("1 mutants in changed code") && message.contains("1 survive"),
            "{message}"
        );
    }
}
