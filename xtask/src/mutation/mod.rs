pub(crate) mod attribution;
pub(crate) mod classify;
pub(crate) mod config;
pub(crate) mod matrix;
pub(crate) mod outcomes;
pub(crate) mod run;
pub(crate) mod tool;
pub(crate) mod worktree;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "usage: xtask mutants compare <mutants.out> <mutants.out>
       xtask mutants matrix <crate> [--shard <k>/<n>]
       xtask mutants matrix-merge <crate> <shard dir>...
       xtask mutants protected [<crate>] [--list]";

pub(crate) fn cli(args: &[String]) -> ExitCode {
    match args {
        [command, first, second] if command == "compare" => {
            compare(Path::new(first), Path::new(second))
        }
        [command, package, rest @ ..] if command == "matrix" => matrix(package, rest),
        [command, package, dirs @ ..] if command == "matrix-merge" && !dirs.is_empty() => {
            merge(package, dirs)
        }
        [command, rest @ ..] if command == "protected" => protected(rest),
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn compare(first: &Path, second: &Path) -> ExitCode {
    let sets = outcomes::load(first).and_then(|a| outcomes::load(second).map(|b| (a, b)));
    let (first_set, second_set) = match sets {
        Ok(sets) => sets,
        Err(problem) => {
            eprintln!("{problem}");
            return ExitCode::from(1);
        }
    };
    let differences = outcomes::differences(&first_set, &second_set);
    if differences.is_empty() {
        println!("identical: {} mutants", first_set.len());
        return ExitCode::SUCCESS;
    }
    println!("{} differences:", differences.len());
    for difference in &differences {
        println!("{difference}");
    }
    ExitCode::from(1)
}

fn parse_shard(text: &str) -> Result<(u32, u32), String> {
    let wrong = || format!("--shard takes <k>/<n> with k below n, not {text:?}");
    let (index, count) = text.split_once('/').ok_or_else(wrong)?;
    let (index, count) = (
        index.parse::<u32>().map_err(|_| wrong())?,
        count.parse::<u32>().map_err(|_| wrong())?,
    );
    if index < count {
        Ok((index, count))
    } else {
        Err(wrong())
    }
}

fn matrix(package: &str, rest: &[String]) -> ExitCode {
    let shard = match rest {
        [] => Ok(None),
        [flag, value] if flag == "--shard" => parse_shard(value).map(Some),
        _ => Err(USAGE.to_owned()),
    };
    let result = shard.and_then(|shard| {
        let repo = crate::checks::root_of(Path::new("."))?;
        let base_ref = std::env::var("CHECKS_BASE_REF")
            .ok()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "origin/main".to_owned());
        let dir = matrix::measure(&repo, package, shard, &base_ref)?;
        println!("wrote {}", dir.display());
        if shard.is_some() {
            return Ok(());
        }
        let merged = matrix::merge_dirs(&repo, package, &[dir])?;
        println!("{}", matrix::summary(&merged));
        Ok(())
    });
    finish(result)
}

fn merge(package: &str, dirs: &[String]) -> ExitCode {
    let result = crate::checks::root_of(Path::new(".")).and_then(|repo| {
        let dirs: Vec<PathBuf> = dirs.iter().map(PathBuf::from).collect();
        let merged = matrix::merge_dirs(&repo, package, &dirs)?;
        println!("{}", matrix::summary(&merged));
        Ok(())
    });
    finish(result)
}

fn finish(result: Result<(), String>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(problem) => {
            eprintln!("{problem}");
            ExitCode::from(1)
        }
    }
}

fn protected(rest: &[String]) -> ExitCode {
    let list = rest.iter().any(|arg| arg == "--list");
    let named: Vec<&String> = rest.iter().filter(|arg| *arg != "--list").collect();
    if named.len() > 1 || named.iter().any(|arg| arg.starts_with("--")) {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    }
    let result = crate::checks::root_of(Path::new(".")).and_then(|repo| {
        let workspace = crate::metadata::Workspace::load(&repo)?;
        if let Some(package) = named.first()
            && !workspace.packages.iter().any(|p| &p.name == *package)
        {
            return Err(format!("{package} is not a package of this workspace"));
        }
        let report = classify::workspace_report(&repo, &workspace)?;
        let mut names: Vec<&str> = workspace.packages.iter().map(|p| p.name.as_str()).collect();
        names.sort_unstable();
        for name in names
            .iter()
            .filter(|n| named.first().is_none_or(|p| p == n))
        {
            println!(
                "{}",
                classify::summary_line(&report, name, Some(name), &classify::ROWS)
            );
            if list {
                for line in classify::protected_lines(&report, name) {
                    println!("{line}");
                }
            }
        }
        if named.is_empty() {
            println!(
                "{}",
                classify::summary_line(&report, "total", None, &classify::ROWS)
            );
        }
        for line in classify::table_lines(&report) {
            println!("{line}");
        }
        Ok(())
    });
    finish(result)
}
