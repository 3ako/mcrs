use crate::mutation::outcomes::{self, Measured};
use crate::mutation::tool::{self, Mode, Spec};
use crate::mutation::worktree::Tree;
use std::path::{Path, PathBuf};

pub(crate) struct Run<'a> {
    pub name: &'a str,
    pub package: &'a str,
    pub test_packages: &'a [String],
    pub test_timeout_seconds: u32,
    pub skip_baseline: bool,
    pub shard: Option<(u32, u32)>,
    pub in_diff: Option<&'a Path>,
}

const NORMAL_EXITS: [i32; 3] = [0, 2, 3];

pub(crate) fn build_scope(tree: &Tree, packages: &[String]) -> Result<(), String> {
    let mut command = tool::cargo(&tree.path);
    command
        .args(["nextest", "run", "--no-run", "--cargo-profile", "mutants"])
        .env("CARGO_TARGET_DIR", &tree.target_dir);
    for package in packages {
        command.args(["-p", package]);
    }
    let status = command
        .status()
        .map_err(|e| format!("could not run cargo nextest: {e}; install cargo-nextest"))?;
    if status.success() {
        return Ok(());
    }
    Err(format!(
        "the unmutated build of {} failed ({status}); fix the build before measuring",
        packages.join(", ")
    ))
}

pub(crate) fn execute(tree: &Tree, config: &Path, run: &Run) -> Result<PathBuf, String> {
    build_scope(tree, run.test_packages)?;
    let output = tree.out_dir.join(run.name);
    if output.exists() {
        std::fs::remove_dir_all(&output)
            .map_err(|e| format!("could not clear {}: {e}", output.display()))?;
    }
    std::fs::create_dir_all(&output)
        .map_err(|e| format!("could not create {}: {e}", output.display()))?;
    let spec = Spec {
        tree: &tree.path,
        config,
        package: run.package,
        mode: Mode::Run {
            test_packages: run.test_packages,
            output: &output,
            skip_baseline: run.skip_baseline,
            test_timeout_seconds: run.test_timeout_seconds,
        },
        shard: run.shard,
        in_diff: run.in_diff,
    };
    let mut command = tool::command(&spec);
    command.env("CARGO_TARGET_DIR", &tree.target_dir);
    let status = command
        .status()
        .map_err(|e| format!("could not run cargo mutants: {e}; install it and put it on PATH"))?;
    let out_dir = output.join("mutants.out");
    match status.code() {
        Some(code) if NORMAL_EXITS.contains(&code) => Ok(out_dir),
        Some(code) => Err(format!(
            "cargo mutants for {} exited {code}; the end of {}:\n{}",
            run.package,
            out_dir.join("debug.log").display(),
            last_lines(&out_dir.join("debug.log"), 8)
        )),
        None => Err(format!(
            "cargo mutants for {} was ended by a signal ({status}); run it again",
            run.package
        )),
    }
}

fn last_lines(path: &Path, count: usize) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(count)..].join("\n")
}

pub(crate) fn measured(out_dir: &Path) -> Result<Vec<Measured>, String> {
    if !out_dir.join("outcomes.json").exists() {
        return Ok(Vec::new());
    }
    outcomes::load_measured(out_dir)
}
