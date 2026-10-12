use crate::git;
use crate::scope::{self, Head};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Debug, Default, PartialEq)]
pub(crate) struct Diff {
    pub patch: String,
    pub added: BTreeMap<String, BTreeSet<u32>>,
    seen: BTreeSet<String>,
}

#[derive(Default)]
struct Section<'a> {
    header: Vec<&'a str>,
    path: Option<String>,
    hunks: Vec<Hunk<'a>>,
}

struct Hunk<'a> {
    lines: Vec<&'a str>,
    adds: Vec<u32>,
    next: u32,
}

fn header_path(rest: &str) -> Option<String> {
    let path = rest.split('\t').next().unwrap_or(rest);
    path.strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .map(str::to_owned)
}

fn same_path_header(rest: &str) -> Option<String> {
    let both = rest.strip_prefix("a/")?;
    let length = both.len().checked_sub(3)? / 2;
    let (left, right) = (both.get(..length)?, both.get(length..)?);
    (right == format!(" b/{left}")).then(|| left.to_owned())
}

fn new_side_start(hunk_header: &str) -> Option<u32> {
    let new = hunk_header
        .split_whitespace()
        .find(|token| token.starts_with('+'))?;
    new[1..].split(',').next()?.parse().ok()
}

pub(crate) fn strip(raw: &str) -> Diff {
    let mut sections: Vec<Section> = Vec::new();
    let mut seen = BTreeSet::new();
    for line in raw.split_inclusive('\n') {
        let text = line.trim_end_matches('\n');
        if let Some(rest) = text.strip_prefix("diff --git ") {
            seen.extend(same_path_header(rest));
            sections.push(Section {
                header: vec![line],
                ..Section::default()
            });
            continue;
        }
        let Some(section) = sections.last_mut() else {
            continue;
        };
        if let Some(rest) = text.strip_prefix("@@ ") {
            let start = new_side_start(rest).unwrap_or(0);
            section.hunks.push(Hunk {
                lines: vec![line],
                adds: Vec::new(),
                next: start,
            });
            continue;
        }
        if let Some(hunk) = section.hunks.last_mut() {
            hunk.lines.push(line);
            match text.as_bytes().first() {
                Some(b'+') => {
                    hunk.adds.push(hunk.next);
                    hunk.next += 1;
                }
                Some(b' ') => hunk.next += 1,
                _ => {}
            }
            continue;
        }
        section.header.push(line);
        if let Some(path) = text.strip_prefix("--- ") {
            seen.extend(header_path(path));
        } else if let Some(path) = text.strip_prefix("+++ ") {
            section.path = header_path(path);
            seen.extend(section.path.clone());
        } else if let Some(path) = text
            .strip_prefix("rename from ")
            .or_else(|| text.strip_prefix("rename to "))
        {
            seen.insert(path.to_owned());
        }
    }
    let mut diff = Diff {
        seen,
        ..Diff::default()
    };
    for section in sections {
        let kept: Vec<&Hunk> = section
            .hunks
            .iter()
            .filter(|hunk| !hunk.adds.is_empty())
            .collect();
        let Some(path) = section.path.as_ref().filter(|_| !kept.is_empty()) else {
            continue;
        };
        diff.patch.extend(section.header.iter().copied());
        for hunk in &kept {
            diff.patch.extend(hunk.lines.iter().copied());
            diff.added
                .entry(path.clone())
                .or_default()
                .extend(hunk.adds.iter().copied());
        }
    }
    diff
}

pub(crate) fn build(repo: &Path, base: &str, head: &str) -> Result<Diff, String> {
    let raw = git::text(
        repo,
        &[
            "-c",
            "core.quotePath=false",
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "-U0",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            "-M",
            base,
            head,
            "--",
            "*.rs",
        ],
    )
    .map_err(|e| e.to_string())?;
    let diff = strip(&raw);
    let changed = scope::changed_paths(repo, base, &Head::Commit(head.to_owned()))?;
    if let Some(missing) = changed
        .iter()
        .find(|path| path.ends_with(".rs") && !diff.seen.contains(path.as_str()))
    {
        return Err(format!(
            "git lists {missing} as changed against {base}, but the diff built for the mutation gate does not contain it; \
             the diff cannot be trusted, so fix the repository state and rerun"
        ));
    }
    Ok(diff)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DELETE_ONLY: &str = "diff --git a/src/a.rs b/src/a.rs\nindex 111..222 100644\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -3,2 +2,0 @@ fn f() {\n-    let x = 1;\n-    let y = 2;\n";

    const MIXED: &str = "diff --git a/src/a.rs b/src/a.rs\nindex 111..222 100644\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -5,2 +5,3 @@ fn f() {\n-    old();\n-    older();\n+    new();\n+    newer();\n+    newest();\n";

    const RENAME_ONLY: &str = "diff --git a/src/old.rs b/src/new.rs\nsimilarity index 100%\nrename from src/old.rs\nrename to src/new.rs\n";

    const REMOVED_FILE: &str = "diff --git a/src/gone.rs b/src/gone.rs\ndeleted file mode 100644\nindex 111..000\n--- a/src/gone.rs\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-fn gone() {}\n-fn gone_too() {}\n";

    const NEW_FILE: &str = "diff --git a/src/new.rs b/src/new.rs\nnew file mode 100644\nindex 000..111\n--- /dev/null\n+++ b/src/new.rs\n@@ -0,0 +1,3 @@\n+fn a() {}\n+fn b() {}\n+fn c() {}\n";

    const TWO_HUNKS: &str = "diff --git a/src/a.rs b/src/a.rs\nindex 111..222 100644\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -3 +2,0 @@ fn f() {\n-    gone();\n@@ -9 +8 @@ fn g() {\n-    a();\n+    b();\n\\ No newline at end of file\n";

    fn lines(rows: &[(&str, &[u32])]) -> BTreeMap<String, BTreeSet<u32>> {
        rows.iter()
            .map(|(file, numbers)| ((*file).to_owned(), numbers.iter().copied().collect()))
            .collect()
    }

    #[test]
    fn deletion_only_hunks_leave_the_diff() {
        struct Row {
            name: &'static str,
            raw: String,
            patch: String,
            added: BTreeMap<String, BTreeSet<u32>>,
            seen: &'static [&'static str],
        }
        let rows = [
            Row {
                name: "a hunk with only removed lines is dropped",
                raw: DELETE_ONLY.to_owned(),
                patch: String::new(),
                added: lines(&[]),
                seen: &["src/a.rs"],
            },
            Row {
                name: "a hunk with removed and added lines is kept and numbered on the new side",
                raw: MIXED.to_owned(),
                patch: MIXED.to_owned(),
                added: lines(&[("src/a.rs", &[5, 6, 7])]),
                seen: &["src/a.rs"],
            },
            Row {
                name: "a pure rename lists no new-side line",
                raw: RENAME_ONLY.to_owned(),
                patch: String::new(),
                added: lines(&[]),
                seen: &["src/old.rs", "src/new.rs"],
            },
            Row {
                name: "a removed file has no new side",
                raw: REMOVED_FILE.to_owned(),
                patch: String::new(),
                added: lines(&[]),
                seen: &["src/gone.rs"],
            },
            Row {
                name: "a new file numbers every line",
                raw: NEW_FILE.to_owned(),
                patch: NEW_FILE.to_owned(),
                added: lines(&[("src/new.rs", &[1, 2, 3])]),
                seen: &["src/new.rs"],
            },
            Row {
                name: "only the hunk that adds a line survives beside a pure deletion",
                raw: TWO_HUNKS.to_owned(),
                patch: "diff --git a/src/a.rs b/src/a.rs\nindex 111..222 100644\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -9 +8 @@ fn g() {\n-    a();\n+    b();\n\\ No newline at end of file\n".to_owned(),
                added: lines(&[("src/a.rs", &[8])]),
                seen: &["src/a.rs"],
            },
            Row {
                name: "two files keep only the one that adds",
                raw: format!("{DELETE_ONLY}{NEW_FILE}"),
                patch: NEW_FILE.to_owned(),
                added: lines(&[("src/new.rs", &[1, 2, 3])]),
                seen: &["src/a.rs", "src/new.rs"],
            },
        ];
        for Row {
            name,
            raw,
            patch,
            added,
            seen,
        } in rows
        {
            let found = strip(&raw);
            assert_eq!(found.patch, patch, "{name}: patch");
            assert_eq!(found.added, added, "{name}: added lines");
            for path in seen {
                assert!(found.seen.contains(*path), "{name}: {path} not seen");
            }
        }
    }
}
