pub(crate) mod config;
pub(crate) mod outcomes;
pub(crate) mod tool;

use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "usage: xtask mutants compare <mutants.out> <mutants.out>";

pub(crate) fn cli(args: &[String]) -> ExitCode {
    match args {
        [command, first, second] if command == "compare" => {
            compare(Path::new(first), Path::new(second))
        }
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
