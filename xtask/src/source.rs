use crate::git;
use crate::scope::{self, Head};
use proc_macro2::{TokenStream, TokenTree};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use syn::{Attribute, Expr, ExprLit, Item, Lit, Meta};

pub type Files = BTreeMap<String, String>;

pub fn rust_files(repo: &Path, head: &Head) -> Result<Vec<String>, String> {
    let mut paths = match head {
        Head::Worktree => {
            let listing = git::run(
                repo,
                &[
                    "ls-files",
                    "-z",
                    "--cached",
                    "--others",
                    "--exclude-standard",
                ],
            )
            .map_err(|e| e.to_string())?;
            listing
                .split(|byte| *byte == 0)
                .filter(|path| !path.is_empty())
                .map(|path| String::from_utf8_lossy(path).into_owned())
                .collect()
        }
        Head::Commit(_) => scope::tracked(repo, head)?,
    };
    paths.retain(|path| path.ends_with(".rs"));
    paths.sort();
    paths.dedup();
    Ok(paths)
}

pub fn read_files(repo: &Path, head: &Head, paths: &[String]) -> Result<Files, String> {
    let mut files = Files::new();
    match head {
        Head::Worktree => {
            for path in paths {
                match std::fs::read(repo.join(path)) {
                    Ok(bytes) => {
                        files.insert(path.clone(), String::from_utf8_lossy(&bytes).into_owned());
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(format!("could not read {path}: {e}")),
                }
            }
        }
        Head::Commit(sha) => {
            let specs: Vec<String> = paths.iter().map(|path| format!("{sha}:{path}")).collect();
            for (path, blob) in paths.iter().zip(scope::blobs(repo, &specs)?) {
                let bytes = blob.ok_or_else(|| format!("{path} is not in {sha}"))?;
                files.insert(path.clone(), String::from_utf8_lossy(&bytes).into_owned());
            }
        }
    }
    Ok(files)
}

pub fn parse(path: &str, text: &str) -> Result<syn::File, String> {
    syn::parse_file(text).map_err(|e| {
        let at = e.span().start();
        format!("{path}:{}: {e}", at.line)
    })
}

pub fn is_test(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path()
            .segments
            .last()
            .is_some_and(|s| s.ident == "test")
    })
}

pub fn has_cfg(attrs: &[Attribute]) -> bool {
    attrs
        .iter()
        .any(|attr| attr.path().is_ident("cfg") || attr.path().is_ident("cfg_attr"))
}

pub fn walk_tokens(stream: &TokenStream, visit: &mut dyn FnMut(&[TokenTree], usize)) {
    let tokens: Vec<TokenTree> = stream.clone().into_iter().collect();
    for index in 0..tokens.len() {
        visit(&tokens, index);
        if let TokenTree::Group(group) = &tokens[index] {
            walk_tokens(&group.stream(), visit);
        }
    }
}

pub fn string_literal(literal: &proc_macro2::Literal) -> Option<String> {
    match Lit::new(literal.clone()) {
        Lit::Str(text) => Some(text.value()),
        _ => None,
    }
}

pub fn doc_line(attr: &Attribute) -> Option<String> {
    let Meta::NameValue(pair) = &attr.meta else {
        return None;
    };
    if !pair.path.is_ident("doc") {
        return None;
    }
    match &pair.value {
        Expr::Lit(ExprLit {
            lit: Lit::Str(text),
            ..
        }) => Some(text.value()),
        _ => None,
    }
}

pub struct ModuleView<'a> {
    pub file: &'a str,
    pub module: &'a [String],
    pub attrs: &'a [Attribute],
    pub items: &'a [Item],
}

struct Walk<'a> {
    files: &'a Files,
    seen: BTreeSet<String>,
}

pub fn walk_modules(
    root: &str,
    files: &Files,
    visit: &mut dyn FnMut(&ModuleView),
) -> Result<(), String> {
    let mut walk = Walk {
        files,
        seen: BTreeSet::new(),
    };
    walk.file(root, &[], true, visit)
}

fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

fn file_name(path: &str) -> &str {
    path.rsplit_once('/').map_or(path, |(_, name)| name)
}

fn join(base: &str, rest: &str) -> String {
    let mut parts: Vec<&str> = base.split('/').filter(|p| !p.is_empty()).collect();
    for part in rest.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

fn path_attribute(attrs: &[Attribute]) -> Option<String> {
    attrs.iter().find_map(|attr| {
        let Meta::NameValue(pair) = &attr.meta else {
            return None;
        };
        if !pair.path.is_ident("path") {
            return None;
        }
        match &pair.value {
            Expr::Lit(ExprLit {
                lit: Lit::Str(text),
                ..
            }) => Some(text.value()),
            _ => None,
        }
    })
}

impl Walk<'_> {
    fn file(
        &mut self,
        path: &str,
        module: &[String],
        mod_rs: bool,
        visit: &mut dyn FnMut(&ModuleView),
    ) -> Result<(), String> {
        if !self.seen.insert(path.to_owned()) {
            return Ok(());
        }
        let text = self
            .files
            .get(path)
            .ok_or_else(|| format!("{path} is not among the tracked Rust files"))?;
        let parsed = parse(path, text)?;
        let owner = if mod_rs {
            parent(path).to_owned()
        } else {
            let stem = file_name(path).trim_end_matches(".rs");
            join(parent(path), stem)
        };
        self.items(
            path,
            module,
            &parsed.attrs,
            &parsed.items,
            &owner,
            &[],
            visit,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn items(
        &mut self,
        path: &str,
        module: &[String],
        attrs: &[Attribute],
        items: &[Item],
        owner: &str,
        inline: &[String],
        visit: &mut dyn FnMut(&ModuleView),
    ) -> Result<(), String> {
        visit(&ModuleView {
            file: path,
            module,
            attrs,
            items,
        });
        let children = join(owner, &inline.join("/"));
        for item in items {
            let Item::Mod(declared) = item else {
                continue;
            };
            let name = declared.ident.to_string();
            let mut below = module.to_vec();
            below.push(name.clone());
            if let Some((_, content)) = &declared.content {
                let mut nested = inline.to_vec();
                nested.push(name);
                self.items(path, &below, &[], content, owner, &nested, visit)?;
                continue;
            }
            let (candidates, via_path) = match path_attribute(&declared.attrs) {
                Some(given) if inline.is_empty() => (vec![join(parent(path), &given)], true),
                Some(given) => (vec![join(&children, &given)], true),
                None => (
                    vec![
                        join(&children, &format!("{name}.rs")),
                        join(&children, &format!("{name}/mod.rs")),
                    ],
                    false,
                ),
            };
            match candidates.iter().find(|c| self.files.contains_key(*c)) {
                Some(found) => {
                    let mod_rs = via_path || file_name(found) == "mod.rs";
                    self.file(found, &below, mod_rs, visit)?;
                }
                None if has_cfg(&declared.attrs) => {}
                None => {
                    return Err(format!(
                        "{path}: `mod {name};` has no file; looked for {}",
                        candidates.join(" and ")
                    ));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scope::tests::Repo;

    fn files(rows: &[(&str, &str)]) -> Files {
        rows.iter()
            .map(|(path, text)| ((*path).to_owned(), (*text).to_owned()))
            .collect()
    }

    fn names_in_modules(root: &str, files: &Files) -> Vec<String> {
        let mut found = Vec::new();
        walk_modules(root, files, &mut |view| {
            for item in view.items {
                if let Item::Fn(f) = item {
                    found.push(format!(
                        "{}::{} @ {}",
                        view.module.join("::"),
                        f.sig.ident,
                        view.file
                    ));
                }
            }
        })
        .unwrap();
        found.sort();
        found
    }

    #[test]
    fn modules_resolve_to_their_files() {
        let tree = files(&[
            (
                "p/src/lib.rs",
                "fn top() {}\nmod a;\n#[path = \"elsewhere/b.rs\"]\nmod b;\nmod c { mod d; }\nmod e;\n#[cfg(windows)]\nmod gated;\n",
            ),
            (
                "p/src/a.rs",
                "fn in_a() {}\nmod inner;\nmod m {\n    #[path = \"y.rs\"]\n    mod z;\n}\n",
            ),
            ("p/src/a/inner.rs", "fn in_inner() {}\n"),
            ("p/src/a/m/y.rs", "fn in_y() {}\n"),
            ("p/src/elsewhere/b.rs", "fn in_b() {}\n"),
            ("p/src/c/d.rs", "fn in_d() {}\n"),
            ("p/src/e/mod.rs", "fn in_e() {}\nmod x;\n"),
            ("p/src/e/x.rs", "fn in_x() {}\n"),
        ]);
        assert_eq!(
            names_in_modules("p/src/lib.rs", &tree),
            [
                "::top @ p/src/lib.rs",
                "a::in_a @ p/src/a.rs",
                "a::inner::in_inner @ p/src/a/inner.rs",
                "a::m::z::in_y @ p/src/a/m/y.rs",
                "b::in_b @ p/src/elsewhere/b.rs",
                "c::d::in_d @ p/src/c/d.rs",
                "e::in_e @ p/src/e/mod.rs",
                "e::x::in_x @ p/src/e/x.rs",
            ]
        );
        let missing = files(&[("p/src/lib.rs", "mod nowhere;\n")]);
        let error = walk_modules("p/src/lib.rs", &missing, &mut |_| {}).unwrap_err();
        assert!(
            error.contains("p/src/lib.rs") && error.contains("p/src/nowhere.rs"),
            "{error}"
        );
    }

    #[test]
    fn a_syntax_error_names_its_file_and_line() {
        let error = parse("crates/x/src/lib.rs", "fn fine() {}\nfn broken( {\n")
            .err()
            .expect("a syntax error");
        assert!(error.starts_with("crates/x/src/lib.rs:2:"), "{error}");
    }

    #[test]
    fn a_commit_is_read_in_one_batch_like_the_worktree() {
        let repo = Repo::new("source-read");
        repo.init();
        repo.write("a/src/lib.rs", "fn a() {}\n");
        repo.write("a/notes.md", "not rust\n");
        repo.write("b.rs", "fn b() {}\n");
        repo.commit("first");
        let sha = repo.sha();
        repo.write("a/src/lib.rs", "fn changed_after() {}\n");
        repo.write("c.rs", "fn untracked() {}\n");
        repo.write(".gitignore", "ignored.rs\n");
        repo.write("ignored.rs", "fn ignored() {}\n");

        let at_commit = Head::Commit(sha);
        let paths = rust_files(&repo.0, &at_commit).unwrap();
        assert_eq!(paths, ["a/src/lib.rs", "b.rs"]);
        let committed = read_files(&repo.0, &at_commit, &paths).unwrap();
        assert_eq!(committed["a/src/lib.rs"], "fn a() {}\n");
        let in_tree = rust_files(&repo.0, &Head::Worktree).unwrap();
        assert_eq!(in_tree, ["a/src/lib.rs", "b.rs", "c.rs"]);
        let current = read_files(&repo.0, &Head::Worktree, &in_tree).unwrap();
        assert_eq!(current["a/src/lib.rs"], "fn changed_after() {}\n");
        let missing = read_files(&repo.0, &at_commit, &["gone.rs".to_owned()]).unwrap_err();
        assert!(missing.contains("gone.rs"), "{missing}");
    }
}
