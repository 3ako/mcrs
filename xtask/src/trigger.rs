#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Check {
    Size,
    GateConfig,
    MutationMarker,
    Floors,
    Parity,
    UnusedDeps,
    MutationConfig,
    Fmt,
    Clippy,
    Test,
    NoBevy,
    Wasm32,
    ExportedBuild,
    Mutation,
}

impl Check {
    pub const fn name(self) -> &'static str {
        match self {
            Check::Size => "size",
            Check::GateConfig => "gate-config",
            Check::MutationMarker => "mutation-marker",
            Check::Floors => "floors",
            Check::Parity => "parity",
            Check::UnusedDeps => "unused-deps",
            Check::MutationConfig => "mutation-config",
            Check::Fmt => "fmt",
            Check::Clippy => "clippy",
            Check::Test => "test",
            Check::NoBevy => "no-bevy",
            Check::Wasm32 => "wasm32",
            Check::ExportedBuild => "exported-build",
            Check::Mutation => "mutation",
        }
    }

    #[cfg(test)]
    pub const fn is_git_only(self) -> bool {
        matches!(
            self,
            Check::Size | Check::GateConfig | Check::MutationMarker | Check::Floors | Check::Parity
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Trigger {
    Edit,
    Commit,
    Push,
    Ci,
    Full,
    Gpu,
    Policy,
    Mutation,
}

impl Trigger {
    pub const ALL: [Trigger; 8] = [
        Trigger::Edit,
        Trigger::Commit,
        Trigger::Push,
        Trigger::Ci,
        Trigger::Full,
        Trigger::Gpu,
        Trigger::Policy,
        Trigger::Mutation,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Trigger::Edit => "edit",
            Trigger::Commit => "commit",
            Trigger::Push => "push",
            Trigger::Ci => "ci",
            Trigger::Full => "full",
            Trigger::Gpu => "gpu",
            Trigger::Policy => "policy",
            Trigger::Mutation => "mutation",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Trigger::Edit => {
                "after a file is written: change size, mutation markers, fixture parity, formatting"
            }
            Trigger::Commit => {
                "before a commit: change size, gate configuration, mutation markers, mutation floors, fixture parity, unused dependencies, mutation configuration, formatting, clippy"
            }
            Trigger::Push => {
                "before a push: every check that ci runs, then the mutants in changed code"
            }
            Trigger::Ci => "on every pull request and every push to main: all checks",
            Trigger::Full => "the whole test suite, exhaustive sweeps included",
            Trigger::Gpu => "the tests that need a Metal GPU",
            Trigger::Policy => {
                "in CI from the default branch: the git-only checks on a pull request head read as git data"
            }
            Trigger::Mutation => {
                "after the mutation shards of a pull request: the surviving mutants in changed code"
            }
        }
    }

    pub const fn is_local(self) -> bool {
        match self {
            Trigger::Edit | Trigger::Commit | Trigger::Push => true,
            Trigger::Ci | Trigger::Full | Trigger::Gpu | Trigger::Policy | Trigger::Mutation => {
                false
            }
        }
    }

    #[cfg(test)]
    pub const fn runs_on_every_pull_request(self) -> bool {
        matches!(self, Trigger::Ci | Trigger::Policy | Trigger::Mutation)
    }

    pub fn parse(name: &str) -> Option<Trigger> {
        Trigger::ALL.into_iter().find(|t| t.name() == name)
    }
}

pub const fn checks(trigger: Trigger) -> &'static [Check] {
    use Check::*;
    match trigger {
        Trigger::Edit => &[Size, MutationMarker, Parity, Fmt],
        Trigger::Commit => &[
            Size,
            GateConfig,
            MutationMarker,
            Floors,
            Parity,
            UnusedDeps,
            MutationConfig,
            Fmt,
            Clippy,
        ],
        Trigger::Push => &[
            Size,
            GateConfig,
            MutationMarker,
            Floors,
            Parity,
            UnusedDeps,
            MutationConfig,
            Fmt,
            Clippy,
            Test,
            NoBevy,
            Wasm32,
            ExportedBuild,
            Mutation,
        ],
        Trigger::Ci => &[
            Size,
            GateConfig,
            MutationMarker,
            Floors,
            Parity,
            UnusedDeps,
            MutationConfig,
            Fmt,
            Clippy,
            Test,
            NoBevy,
            Wasm32,
            ExportedBuild,
        ],
        Trigger::Full | Trigger::Gpu => &[Test],
        Trigger::Policy => &[Size, GateConfig, MutationMarker, Floors, Parity],
        Trigger::Mutation => &[Mutation],
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Invocation {
    pub trigger: Trigger,
    pub head: Option<String>,
}

pub fn from_args(args: &[String]) -> Result<Invocation, String> {
    let plain = |trigger| Invocation {
        trigger,
        head: None,
    };
    match args {
        [] => Err("missing trigger".to_owned()),
        [name] if name.is_empty() => Err("the trigger is empty".to_owned()),
        [name] => match Trigger::parse(name) {
            Some(Trigger::Policy) => Err("policy needs the head commit: policy <head>".to_owned()),
            Some(trigger) => Ok(plain(trigger)),
            None => Err(format!("unknown trigger {name:?}")),
        },
        [name, head] if name == Trigger::Policy.name() => match head.as_str() {
            "" => Err("the head commit is empty".to_owned()),
            option if option.starts_with('-') => {
                Err(format!("the head must be a commit id, not {option:?}"))
            }
            _ => Ok(Invocation {
                trigger: Trigger::Policy,
                head: Some(head.clone()),
            }),
        },
        more => Err(format!(
            "expected exactly one trigger, got {}: {}",
            more.len(),
            more.join(" ")
        )),
    }
}

pub fn usage() -> String {
    let mut text = String::from("usage: run-checks <trigger>\n");
    for (heading, local) in [
        ("triggers that run on this machine", true),
        ("triggers that run in CI or on demand", false),
    ] {
        text.push_str(&format!("\n{heading}:\n"));
        for trigger in Trigger::ALL.into_iter().filter(|t| t.is_local() == local) {
            let usage = match trigger {
                Trigger::Policy => "policy <head>",
                other => other.name(),
            };
            text.push_str(&format!("  {usage:<15}{}\n", trigger.description()));
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outside_pull_requests(table: impl Fn(Trigger) -> &'static [Check]) -> Vec<(Trigger, Check)> {
        let on_pull_requests: Vec<Check> = Trigger::ALL
            .into_iter()
            .filter(|t| t.runs_on_every_pull_request())
            .flat_map(|t| table(t).iter().copied())
            .collect();
        Trigger::ALL
            .into_iter()
            .filter(|t| t.is_local())
            .flat_map(|t| table(t).iter().map(move |&c| (t, c)))
            .filter(|(_, c)| !on_pull_requests.contains(c))
            .collect()
    }

    #[test]
    fn every_local_trigger_runs_a_subset_of_the_pull_request_checks() {
        assert_eq!(outside_pull_requests(checks), []);
        let local: Vec<_> = Trigger::ALL.into_iter().filter(|t| t.is_local()).collect();
        assert!(local.len() >= 3, "{local:?}");
        let every: Vec<_> = Trigger::ALL
            .into_iter()
            .filter(|t| t.runs_on_every_pull_request())
            .collect();
        assert_eq!(every, [Trigger::Ci, Trigger::Policy, Trigger::Mutation]);
    }

    #[test]
    fn the_subset_test_fails_on_a_check_no_pull_request_trigger_lists_whatever_the_order() {
        fn table(trigger: Trigger) -> &'static [Check] {
            match trigger {
                Trigger::Ci => &[Check::Test, Check::Fmt],
                Trigger::Mutation => &[Check::Mutation],
                Trigger::Push => &[Check::Mutation, Check::Fmt, Check::Test],
                Trigger::Commit => &[Check::Fmt, Check::Wasm32],
                _ => &[Check::Fmt],
            }
        }
        assert_eq!(
            outside_pull_requests(table),
            [(Trigger::Commit, Check::Wasm32)]
        );
    }

    #[test]
    fn the_mutation_check_is_the_last_of_the_push_trigger_and_alone_in_its_own() {
        assert_eq!(checks(Trigger::Push).last(), Some(&Check::Mutation));
        assert_eq!(checks(Trigger::Mutation), [Check::Mutation]);
        for trigger in [Trigger::Edit, Trigger::Commit, Trigger::Ci, Trigger::Policy] {
            assert!(!checks(trigger).contains(&Check::Mutation), "{trigger:?}");
        }
    }

    #[test]
    fn every_trigger_lists_a_check_and_round_trips_its_name() {
        for trigger in Trigger::ALL {
            assert!(!checks(trigger).is_empty(), "{trigger:?} lists no check");
            assert_eq!(Trigger::parse(trigger.name()), Some(trigger));
        }
        let mut names: Vec<_> = Trigger::ALL.iter().map(|t| t.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), Trigger::ALL.len());
    }

    #[test]
    fn arguments_other_than_one_known_trigger_are_refused() {
        let cases: [(&[&str], Option<Trigger>); 7] = [
            (&[], None),
            (&[""], None),
            (&["bogus"], None),
            (&["Edit"], None),
            (&["edit", "ci"], None),
            (&["edit"], Some(Trigger::Edit)),
            (&["gpu"], Some(Trigger::Gpu)),
        ];
        for (args, expected) in cases {
            let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
            let found = from_args(&args).ok().map(|invocation| invocation.trigger);
            assert_eq!(found, expected, "{args:?}");
        }
    }

    #[test]
    fn only_policy_takes_a_head_and_it_must_be_a_commit_id() {
        let cases: [(&[&str], Option<&str>); 8] = [
            (&["policy", "abc123"], Some("abc123")),
            (&["policy"], None),
            (&["policy", ""], None),
            (&["policy", "--output=x"], None),
            (&["policy", "a", "b"], None),
            (&["edit", "abc123"], None),
            (&["ci", "abc123"], None),
            (&["push", "abc123"], None),
        ];
        for (args, head) in cases {
            let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
            let found = from_args(&args).ok().and_then(|invocation| invocation.head);
            assert_eq!(found.as_deref(), head, "{args:?}");
        }
    }

    #[test]
    fn every_trigger_lists_the_git_only_checks_first() {
        for trigger in Trigger::ALL {
            let listed = checks(trigger);
            let first_other = listed.iter().position(|check| !check.is_git_only());
            if let Some(first_other) = first_other {
                let late: Vec<_> = listed[first_other..]
                    .iter()
                    .filter(|check| check.is_git_only())
                    .collect();
                assert!(
                    late.is_empty(),
                    "{trigger:?} lists {late:?} after another check"
                );
            }
        }
    }

    #[test]
    fn the_mutation_configuration_is_checked_after_unused_deps_and_before_fmt() {
        let runs_it = [Trigger::Commit, Trigger::Push, Trigger::Ci];
        for trigger in Trigger::ALL {
            let listed = checks(trigger);
            let position = listed.iter().position(|&c| c == Check::MutationConfig);
            if !runs_it.contains(&trigger) {
                assert_eq!(position, None, "{trigger:?}");
                continue;
            }
            let position = position.unwrap_or_else(|| panic!("{trigger:?} lacks the check"));
            assert_eq!(listed[position - 1], Check::UnusedDeps, "{trigger:?}");
            assert_eq!(listed[position + 1], Check::Fmt, "{trigger:?}");
        }
    }

    #[test]
    fn floors_are_judged_right_after_the_mutation_marker_wherever_a_pull_request_is() {
        let runs_it = [Trigger::Commit, Trigger::Push, Trigger::Ci, Trigger::Policy];
        for trigger in Trigger::ALL {
            let listed = checks(trigger);
            let position = listed.iter().position(|&c| c == Check::Floors);
            if !runs_it.contains(&trigger) {
                assert_eq!(position, None, "{trigger:?}");
                continue;
            }
            let position = position.unwrap_or_else(|| panic!("{trigger:?} lacks the check"));
            assert_eq!(listed[position - 1], Check::MutationMarker, "{trigger:?}");
        }
    }

    #[test]
    fn parity_is_the_last_git_only_check_of_every_trigger_that_runs_on_a_pull_request_or_locally() {
        let carries_it = [
            Trigger::Edit,
            Trigger::Commit,
            Trigger::Push,
            Trigger::Ci,
            Trigger::Policy,
        ];
        for trigger in Trigger::ALL {
            let listed = checks(trigger);
            let position = listed.iter().position(|&c| c == Check::Parity);
            if !carries_it.contains(&trigger) {
                assert_eq!(position, None, "{trigger:?}");
                continue;
            }
            let position = position.unwrap_or_else(|| panic!("{trigger:?} lacks the check"));
            assert!(Check::Parity.is_git_only());
            assert!(
                listed[position + 1..]
                    .iter()
                    .all(|check| !check.is_git_only()),
                "{trigger:?} lists a git-only check after parity"
            );
            assert_eq!(listed.iter().filter(|&&c| c == Check::Parity).count(), 1);
        }
    }

    #[test]
    fn every_check_and_trigger_has_its_own_name_and_description() {
        let mut checks: Vec<Check> = Trigger::ALL
            .into_iter()
            .flat_map(|t| checks(t).to_vec())
            .collect();
        checks.sort_by_key(|check| check.name());
        checks.dedup();
        let names: Vec<&str> = checks.iter().map(|check| check.name()).collect();
        let mut unique = names.clone();
        unique.dedup();
        assert_eq!(names, unique);
        assert!(names.iter().all(|name| !name.is_empty()));
        assert!(names.contains(&"parity"));

        let mut descriptions: Vec<&str> = Trigger::ALL.iter().map(|t| t.description()).collect();
        assert!(descriptions.iter().all(|text| text.len() > 20));
        descriptions.sort_unstable();
        descriptions.dedup();
        assert_eq!(descriptions.len(), Trigger::ALL.len());
        for trigger in [Trigger::Edit, Trigger::Commit] {
            assert!(
                trigger.description().contains("fixture parity"),
                "{trigger:?}"
            );
        }
    }

    #[test]
    fn the_base_run_trigger_runs_only_git_checks() {
        let reads_the_head: Vec<_> = checks(Trigger::Policy)
            .iter()
            .filter(|check| !check.is_git_only())
            .collect();
        assert!(
            reads_the_head.is_empty(),
            "the policy trigger runs {reads_the_head:?}, which would run cargo on the head"
        );
    }

    #[test]
    fn policy_runs_a_subset_of_ci() {
        let outside: Vec<_> = checks(Trigger::Policy)
            .iter()
            .filter(|check| !checks(Trigger::Ci).contains(check))
            .collect();
        assert!(outside.is_empty(), "{outside:?}");
    }
}
