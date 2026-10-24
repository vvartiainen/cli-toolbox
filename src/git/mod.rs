use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Error, exec};

const AGENT_WORKTREE_SUFFIX: &str = "-agent-wt";

/// Overrides the directory that holds sandbox project worktrees.
pub const WORKTREES_DIR_ENV: &str = "CLI_TOOLBOX_WORKTREES_DIR";

/// A linked worktree of a repository. `name` is its folder, `branch` the
/// branch checked out in it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Worktree {
    pub name: String,
    pub path: PathBuf,
    pub branch: String,
}

/// One entry of `git worktree list`.
#[derive(Clone)]
struct Listed {
    path: PathBuf,
    branch: String,
}

impl From<Listed> for Worktree {
    fn from(listed: Listed) -> Self {
        Self {
            name: listed
                .path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            path: listed.path,
            branch: listed.branch,
        }
    }
}

/// The directory that holds sandbox project worktrees, grouped by
/// repository: `$CLI_TOOLBOX_WORKTREES_DIR`, or `~/.agent-sandbox/worktrees`.
pub fn worktrees_root() -> Result<PathBuf, Error> {
    resolve_root(
        WORKTREES_DIR_ENV,
        std::env::var_os(WORKTREES_DIR_ENV),
        dirs::home_dir(),
        Path::new(".agent-sandbox/worktrees"),
    )
}

fn resolve_root(
    env: &str,
    dir: Option<OsString>,
    home: Option<PathBuf>,
    default: &Path,
) -> Result<PathBuf, Error> {
    if let Some(dir) = dir.filter(|dir| !dir.is_empty()) {
        return std::path::absolute(&dir)
            .map_err(|err| Error::msg(format!("resolve {env}: {err}")));
    }
    let home =
        home.ok_or_else(|| Error::msg("resolve home directory: home directory not found"))?;
    Ok(home.join(default))
}

/// Create a worktree of the repository at `cwd` in `<root>/<repo>/<dir>` on
/// the branch `name`, where `dir` is `name` made folder-safe. An existing
/// local branch is checked out; otherwise the branch is created.
pub(crate) fn create_worktree(
    cwd: &Path,
    root: &Path,
    name: &str,
    stderr: &mut dyn Write,
) -> Result<Worktree, Error> {
    require_git()?;
    // `--branch` also expands names such as `@{-1}`; only a literal name will do.
    let valid = !name.starts_with('-')
        && git_output(cwd, &["check-ref-format", "--branch", name])
            .is_ok_and(|expanded| expanded.trim() == name);
    if !valid {
        return Err(Error::msg(format!("\"{name}\" is not a valid branch name")));
    }
    let dir = worktree_dir_name(name)?;
    let group = root.join(repo_name(cwd)?);
    let path = group.join(&dir);
    if fs::symlink_metadata(&path).is_ok() {
        return Err(Error::msg(format!("{} already exists", path.display())));
    }
    fs::create_dir_all(&group)
        .map_err(|err| Error::msg(format!("create {}: {err}", group.display())))?;
    let path_text = path.to_string_lossy();
    let reference = format!("refs/heads/{name}");
    let existing = git_output(cwd, &["show-ref", "--verify", "--quiet", &reference]).is_ok();
    let args = if existing {
        vec!["worktree", "add", "--", &path_text, name]
    } else {
        vec!["worktree", "add", "-b", name, "--", &path_text]
    };
    run_git(cwd, stderr, &args).map_err(|err| {
        Error::msg(format!(
            "create git worktree \"{}\" on branch \"{name}\": {err}",
            path.display()
        ))
    })?;
    Ok(Worktree {
        name: dir,
        path,
        branch: name.into(),
    })
}

/// The folder of a worktree named `name`: `/` becomes `-`, and anything
/// outside `[A-Za-z0-9._-]` too. It is also the guest mount, the Herdr label
/// and the review snapshot namespace, so it must be one path component.
pub(crate) fn worktree_dir_name(name: &str) -> Result<String, Error> {
    let dir: String = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || "._-".contains(ch) {
                ch
            } else {
                '-'
            }
        })
        .collect();
    let dir = dir.trim_matches(|ch| ch == '-' || ch == '.');
    if dir.is_empty() {
        return Err(Error::msg(format!(
            "\"{name}\" gives no usable folder name; use letters, digits, dots and dashes"
        )));
    }
    Ok(dir.into())
}

/// The sandbox project worktrees of the repository at `cwd`: its worktrees
/// in `<root>/<repo>`, sorted by name. Where they are decides, not their name.
pub fn project_worktrees(cwd: &Path, root: &Path) -> Result<Vec<Worktree>, Error> {
    require_git()?;
    let group = root.join(repo_name(cwd)?);
    let resolved_group = resolved(&group);
    let mut worktrees: Vec<Worktree> = list_git_worktrees(cwd)?
        .into_iter()
        .skip(1)
        .filter(|worktree| {
            worktree
                .path
                .parent()
                .is_some_and(|parent| parent == group || resolved(parent) == resolved_group)
        })
        .map(Worktree::from)
        .collect();
    worktrees.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(worktrees)
}

fn resolved(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// The worktrees a removal targets: the one named (or the only one), or all
/// of them with `all`.
pub(crate) fn select_worktrees(
    worktrees: Vec<Worktree>,
    name: &str,
    all: bool,
) -> Result<Vec<Worktree>, Error> {
    if all && !name.is_empty() {
        return Err(Error::msg(
            "specify either a worktree name or --all, not both",
        ));
    }
    if worktrees.is_empty() {
        return Err(Error::msg("no worktrees found"));
    }
    if all {
        Ok(worktrees)
    } else {
        Ok(vec![select_worktree(&worktrees, name)?])
    }
}

/// Worktree groups are named after the main checkout, which git lists first,
/// so commands behave the same from the repository and from its worktrees.
fn repo_name(cwd: &Path) -> Result<String, Error> {
    let main = main_checkout(cwd)?;
    main.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(|| Error::msg(format!("no repository name in {}", main.display())))
}

/// The next free `<repo>-agent-wt[-N]` name, suggested for a new project
/// worktree. Names of this repository's worktrees and branches are taken, and
/// so are the entries of its group in `root`, which another repository with
/// the same name may share.
pub(crate) fn next_agent_worktree_name(cwd: &Path, root: &Path) -> Result<String, Error> {
    require_git()?;
    let base = repo_name(cwd)?;
    let used = used_names(cwd, &root.join(&base))?;
    Ok(free_agent_worktree_name(&base, &used))
}

fn free_agent_worktree_name(base: &str, used: &HashSet<String>) -> String {
    (1..)
        .map(|n| {
            if n == 1 {
                format!("{base}{AGENT_WORKTREE_SUFFIX}")
            } else {
                format!("{base}{AGENT_WORKTREE_SUFFIX}-{n}")
            }
        })
        .find(|name| !used.contains(name))
        .expect("an unused name")
}

fn used_names(cwd: &Path, group: &Path) -> Result<HashSet<String>, Error> {
    let mut used = HashSet::new();
    match fs::read_dir(group) {
        Ok(entries) => {
            for entry in entries {
                used.insert(entry?.file_name().to_string_lossy().into_owned());
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(Error::msg(format!("read {}: {err}", group.display()))),
    }
    for worktree in list_git_worktrees(cwd)? {
        used.insert(Worktree::from(worktree).name);
    }
    let branches = git_output(
        cwd,
        &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
    )?;
    used.extend(branches.lines().map(|branch| branch.trim().to_string()));
    Ok(used)
}

fn list_git_worktrees(cwd: &Path) -> Result<Vec<Listed>, Error> {
    let output = git_output(cwd, &["worktree", "list", "--porcelain"])?;
    Ok(parse_worktree_porcelain(&output))
}

fn parse_worktree_porcelain(output: &str) -> Vec<Listed> {
    let mut worktrees = Vec::new();
    let empty = || Listed {
        path: PathBuf::new(),
        branch: String::new(),
    };
    let mut current = empty();
    let mut flush = |current: &mut Listed| {
        if !current.path.as_os_str().is_empty() {
            worktrees.push(std::mem::replace(current, empty()));
        }
    };

    for line in output.lines() {
        if line.is_empty() {
            flush(&mut current);
            continue;
        }
        let (key, value) = line.split_once(' ').unwrap_or((line, ""));
        match key {
            "worktree" => current.path = PathBuf::from(value),
            "branch" => {
                current.branch = value
                    .strip_prefix("refs/heads/")
                    .unwrap_or(value)
                    .to_string();
            }
            _ => {}
        }
    }
    flush(&mut current);
    worktrees
}

fn select_worktree(worktrees: &[Worktree], name: &str) -> Result<Worktree, Error> {
    if name.is_empty() {
        if worktrees.len() == 1 {
            return Ok(worktrees[0].clone());
        }
        return Err(Error::msg(
            "multiple worktrees found; specify a name or --all",
        ));
    }

    worktrees
        .iter()
        .find(|worktree| worktree_matches(worktree, name))
        .cloned()
        .ok_or_else(|| Error::msg(format!("worktree \"{name}\" not found")))
}

/// A worktree is named by its folder, its branch or its path.
fn worktree_matches(worktree: &Worktree, query: &str) -> bool {
    if query == worktree.name || query == worktree.branch || Path::new(query) == worktree.path {
        return true;
    }
    let Ok(abs) = std::path::absolute(query) else {
        return false;
    };
    if abs == worktree.path {
        return true;
    }
    fs::canonicalize(&abs).is_ok_and(|resolved| resolved == worktree.path)
}

/// How [`remove_worktree`] treats the branch and local changes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct RemoveOptions {
    /// Keep the worktree's branch, for example for review after the removal.
    pub(crate) keep_branch: bool,
    /// Remove the worktree even with uncommitted or untracked files.
    pub(crate) force: bool,
}

pub(crate) fn remove_worktree(
    cwd: &Path,
    worktree: &Worktree,
    options: RemoveOptions,
    stderr: &mut dyn Write,
) -> Result<(), Error> {
    // The group is named after the repository, found while git still can.
    let repo = repo_name(cwd).ok();
    let path = worktree.path.to_string_lossy();
    let mut args = vec!["worktree", "remove"];
    if options.force {
        args.push("--force");
    }
    args.extend(["--", &path]);
    run_git(cwd, stderr, &args).map_err(|err| {
        Error::msg(format!(
            "remove git worktree \"{}\": {err}",
            worktree.path.display()
        ))
    })?;
    remove_empty_group_dir(worktree, repo.as_deref());
    if options.keep_branch || worktree.branch.is_empty() {
        return Ok(());
    }
    run_git(cwd, stderr, &["branch", "-D", "--", &worktree.branch]).map_err(|err| {
        Error::msg(format!(
            "worktree \"{}\" was removed, but could not delete git branch \"{}\": {err}",
            worktree.path.display(),
            worktree.branch
        ))
    })
}

/// Drop the `<root>/<repo>` group once its last worktree is gone. Worktrees
/// outside a group directory, like older sibling ones, leave their parent
/// alone.
fn remove_empty_group_dir(worktree: &Worktree, repo: Option<&str>) {
    let Some(parent) = worktree.path.parent() else {
        return;
    };
    if repo.is_some_and(|repo| parent.file_name().is_some_and(|group| group == repo)) {
        // Fails while the directory still has entries, which is fine.
        let _ = fs::remove_dir(parent);
    }
}

/// The main checkout of the repository at `cwd`, which git lists first.
pub(crate) fn main_checkout(cwd: &Path) -> Result<PathBuf, Error> {
    list_git_worktrees(cwd)?
        .into_iter()
        .next()
        .map(|worktree| worktree.path)
        .ok_or_else(|| Error::msg("git worktree list: no main worktree"))
}

/// The commit `rev` names in `dir`, if any.
pub(crate) fn resolve_commit(dir: &Path, rev: &str) -> Option<String> {
    let spec = format!("{rev}^{{commit}}");
    git_output(dir, &["rev-parse", "-q", "--verify", &spec]).ok()
}

/// Whether `ancestor` is `descendant` or one of its ancestors.
pub(crate) fn is_ancestor(dir: &Path, ancestor: &str, descendant: &str) -> bool {
    exec::combined_output(
        "git",
        &["merge-base", "--is-ancestor", ancestor, descendant],
        Some(dir),
    )
    .is_ok()
}

/// `git status --porcelain` lines: uncommitted and untracked files.
pub(crate) fn status_lines(dir: &Path) -> Result<Vec<String>, Error> {
    Ok(git_output(dir, &["status", "--porcelain"])?
        .lines()
        .map(str::to_string)
        .collect())
}

fn require_git() -> Result<(), Error> {
    exec::require("git").map(|_| ())
}

fn run_git(cwd: &Path, stderr: &mut dyn Write, args: &[&str]) -> Result<(), Error> {
    exec::run_merged("git", args, Some(cwd), stderr)
}

fn git_output(cwd: &Path, args: &[&str]) -> Result<String, Error> {
    exec::combined_output("git", args, Some(cwd))
        .map_err(|err| Error::msg(format!("git {}: {err}", args.join(" "))))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io;
    use std::process::Command;

    #[test]
    fn worktree_roots_default_under_home_and_honor_overrides() {
        let home = Some(PathBuf::from("/home/me"));
        let default = Path::new(".agent-sandbox/worktrees");
        let resolve = |dir: Option<&str>| {
            resolve_root("ENV", dir.map(OsString::from), home.clone(), default).unwrap()
        };
        assert_eq!(
            resolve(None),
            Path::new("/home/me/.agent-sandbox/worktrees")
        );
        assert_eq!(
            resolve(Some("")),
            Path::new("/home/me/.agent-sandbox/worktrees")
        );
        assert_eq!(resolve(Some("/srv/wt")), Path::new("/srv/wt"));
        assert!(resolve_root("ENV", None, None, default).is_err());
    }

    #[test]
    fn next_agent_worktree_name_fills_gaps() {
        assert_eq!(
            free_agent_worktree_name("cli-toolbox", &HashSet::new()),
            "cli-toolbox-agent-wt"
        );
        let used = HashSet::from([
            "cli-toolbox-agent-wt".into(),
            "cli-toolbox-agent-wt-2".into(),
        ]);
        assert_eq!(
            free_agent_worktree_name("cli-toolbox", &used),
            "cli-toolbox-agent-wt-3"
        );
        let used = HashSet::from(["cli-toolbox-agent-wt-2".into()]);
        assert_eq!(
            free_agent_worktree_name("cli-toolbox", &used),
            "cli-toolbox-agent-wt"
        );
    }

    #[test]
    fn folder_names_are_one_safe_path_component() {
        assert_eq!(worktree_dir_name("feature/login").unwrap(), "feature-login");
        assert_eq!(worktree_dir_name("fix it: #12").unwrap(), "fix-it---12");
        assert_eq!(worktree_dir_name("v1.2_rc").unwrap(), "v1.2_rc");
        for bad in ["", "/", "..", "--"] {
            assert!(worktree_dir_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn create_makes_a_named_worktree_on_a_new_branch() {
        if skip_without_git() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("demo");
        init_git_repo(&repo);
        let wt_root = root.path().join("worktrees");

        let worktree = create_worktree(&repo, &wt_root, "login", &mut io::sink()).unwrap();
        let want = wt_root.join("demo").join("login");
        assert_eq!(worktree.name, "login");
        assert_eq!(worktree.branch, "login");
        assert_same_path(worktree.path.to_string_lossy(), &want);
        assert_eq!(
            must_git_output(&want, &["rev-parse", "--abbrev-ref", "HEAD"]),
            "login"
        );
        assert_worktree_listed(&repo, &want, "login");
        // The folder is taken now.
        let err = create_worktree(&repo, &wt_root, "login", &mut io::sink()).unwrap_err();
        assert!(err.to_string().contains("already exists"), "{err}");
    }

    #[test]
    fn create_checks_out_an_existing_branch() {
        if skip_without_git() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("demo");
        init_git_repo(&repo);
        must_run_git(&repo, &["branch", "feature/login"]);
        let tip = must_git_output(&repo, &["rev-parse", "feature/login"]);

        let worktree = create_worktree(
            &repo,
            &root.path().join("worktrees"),
            "feature/login",
            &mut io::sink(),
        )
        .unwrap();
        assert_eq!(worktree.name, "feature-login");
        assert_eq!(worktree.branch, "feature/login");
        assert!(worktree.path.ends_with("demo/feature-login"));
        assert_eq!(
            must_git_output(&worktree.path, &["rev-parse", "--abbrev-ref", "HEAD"]),
            "feature/login"
        );
        assert_eq!(must_git_output(&worktree.path, &["rev-parse", "HEAD"]), tip);
    }

    #[test]
    fn create_rejects_invalid_branch_names() {
        if skip_without_git() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("demo");
        init_git_repo(&repo);
        for bad in ["two words", "-x", "a..b", "@{-1}", ""] {
            let err =
                create_worktree(&repo, &root.path().join("wt"), bad, &mut io::sink()).unwrap_err();
            assert!(
                err.to_string().contains("not a valid branch"),
                "{bad}: {err}"
            );
        }
    }

    #[test]
    fn same_named_repos_share_a_group_without_clashing() {
        if skip_without_git() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let one = root.path().join("one").join("demo");
        let two = root.path().join("two").join("demo");
        init_git_repo(&one);
        init_git_repo(&two);
        let wt_root = root.path().join("worktrees");

        let first = next_agent_worktree_name(&one, &wt_root).unwrap();
        create_worktree(&one, &wt_root, &first, &mut io::sink()).unwrap();
        let other = next_agent_worktree_name(&two, &wt_root).unwrap();
        create_worktree(&two, &wt_root, &other, &mut io::sink()).unwrap();

        assert_eq!(other, "demo-agent-wt-2");
        assert_eq!(
            project_worktrees(&one, &wt_root).unwrap()[0].name,
            "demo-agent-wt"
        );
        assert_eq!(
            project_worktrees(&two, &wt_root).unwrap()[0].name,
            "demo-agent-wt-2"
        );
    }

    #[test]
    fn removal_drops_the_empty_group_folder_only() {
        if skip_without_git() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("demo");
        init_git_repo(&repo);
        let wt_root = root.path().join("worktrees");
        let one = create_worktree(&repo, &wt_root, "one", &mut io::sink()).unwrap();
        let two = create_worktree(&repo, &wt_root, "two", &mut io::sink()).unwrap();

        remove_worktree(&repo, &one, RemoveOptions::default(), &mut io::sink()).unwrap();
        assert!(wt_root.join("demo").exists());
        remove_worktree(&repo, &two, RemoveOptions::default(), &mut io::sink()).unwrap();
        assert!(!wt_root.join("demo").exists());

    }

    #[test]
    fn removal_deletes_the_branch_unless_kept() {
        if skip_without_git() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("demo");
        init_git_repo(&repo);
        let wt_root = root.path().join("worktrees");
        let gone = create_worktree(&repo, &wt_root, "gone", &mut io::sink()).unwrap();
        let kept = create_worktree(&repo, &wt_root, "kept", &mut io::sink()).unwrap();
        fs::write(kept.path.join("scratch"), "untracked\n").unwrap();

        remove_worktree(&repo, &gone, RemoveOptions::default(), &mut io::sink()).unwrap();
        let options = RemoveOptions {
            keep_branch: true,
            force: true,
        };
        remove_worktree(&repo, &kept, options, &mut io::sink()).unwrap();

        assert!(!kept.path.exists());
        let branches = must_git_output(
            &repo,
            &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
        );
        let branches: HashSet<_> = branches.lines().collect();
        assert!(!branches.contains("gone"));
        assert!(branches.contains("kept"));
    }

    #[test]
    fn removal_refuses_a_dirty_worktree_without_force() {
        if skip_without_git() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("demo");
        init_git_repo(&repo);
        let worktree =
            create_worktree(&repo, &root.path().join("wt"), "dirty", &mut io::sink()).unwrap();
        fs::write(worktree.path.join("README.md"), "uncommitted change\n").unwrap();

        let err = remove_worktree(&repo, &worktree, RemoveOptions::default(), &mut io::sink())
            .unwrap_err();
        assert!(err.to_string().contains("remove git worktree"), "{err}");
        assert!(worktree.path.exists());
    }

    #[test]
    fn removal_reports_a_branch_failure_after_removing_the_worktree() {
        if skip_without_git() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("demo");
        init_git_repo(&repo);
        let wt_root = root.path().join("worktrees");
        let worktree = create_worktree(&repo, &wt_root, "held", &mut io::sink()).unwrap();
        let other = root.path().join("other");
        must_run_git(
            &repo,
            &[
                "worktree",
                "add",
                "--force",
                other.to_str().unwrap(),
                "held",
            ],
        );

        let mut stderr = Vec::new();
        let err = remove_worktree(&repo, &worktree, RemoveOptions::default(), &mut stderr)
            .unwrap_err()
            .to_string();
        assert!(err.contains("delete git branch \"held\""), "{err}");
        assert!(err.contains("was removed"), "{err}");
        assert!(!stderr.is_empty());
        assert!(!worktree.path.exists());
        must_run_git(
            &repo,
            &["worktree", "remove", "--force", other.to_str().unwrap()],
        );
    }

    #[test]
    fn selection_takes_a_name_branch_the_only_one_or_all() {
        let worktree = |name: &str, branch: &str| Worktree {
            name: name.into(),
            path: PathBuf::from(format!("/wt/{name}")),
            branch: branch.into(),
        };
        let list = vec![worktree("a", "a"), worktree("feature-b", "feature/b")];
        let names = |selected: Vec<Worktree>| -> Vec<String> {
            selected.into_iter().map(|worktree| worktree.name).collect()
        };
        assert_eq!(
            names(select_worktrees(list.clone(), "a", false).unwrap()),
            ["a"]
        );
        assert_eq!(
            names(select_worktrees(list.clone(), "feature/b", false).unwrap()),
            ["feature-b"]
        );
        assert_eq!(
            names(select_worktrees(list.clone(), "/wt/feature-b", false).unwrap()),
            ["feature-b"]
        );
        assert_eq!(select_worktrees(list.clone(), "", true).unwrap().len(), 2);
        assert!(select_worktrees(list.clone(), "", false).is_err());
        assert!(select_worktrees(list.clone(), "a", true).is_err());
        assert!(select_worktrees(list[..1].to_vec(), "", false).is_ok());
        assert!(select_worktrees(Vec::new(), "", false).is_err());
    }

    pub(crate) fn skip_without_git() -> bool {
        if exec::look_path("git").is_none() {
            eprintln!("skipping: git is not available on PATH");
            true
        } else {
            false
        }
    }

    pub(crate) fn commit_all(dir: &Path) {
        must_run_git(dir, &["add", "-A"]);
        must_run_git(dir, &["commit", "-m", "update"]);
    }

    pub(crate) fn init_git_repo(dir: &Path) {
        fs::create_dir_all(dir).unwrap();
        must_run_git(dir, &["init"]);
        fs::write(dir.join("README.md"), "demo\n").unwrap();
        must_run_git(dir, &["add", "README.md"]);
        must_run_git(dir, &["commit", "-m", "initial commit"]);
    }

    fn assert_same_path(got: impl AsRef<str>, want: &Path) {
        assert_eq!(
            resolve_test_path(Path::new(got.as_ref())),
            resolve_test_path(want)
        );
    }

    fn resolve_test_path(path: &Path) -> PathBuf {
        if let Ok(resolved) = fs::canonicalize(path) {
            return resolved;
        }
        if let (Some(parent), Some(base)) = (path.parent(), path.file_name())
            && let Ok(resolved_parent) = fs::canonicalize(parent)
        {
            return resolved_parent.join(base);
        }
        std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
    }

    fn assert_worktree_listed(repo: &Path, path: &Path, branch: &str) {
        let canonical = fs::canonicalize(path).unwrap();
        let list = must_git_output(repo, &["worktree", "list", "--porcelain"]);
        assert!(
            list.contains(&format!("worktree {}", canonical.display())),
            "{list}"
        );
        assert!(
            list.contains(&format!("branch refs/heads/{branch}")),
            "{list}"
        );
    }

    pub(crate) fn must_git_output(dir: &Path, args: &[&str]) -> String {
        let output = test_git_command(dir, args).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    }

    pub(crate) fn must_run_git(dir: &Path, args: &[&str]) {
        let output = test_git_command(dir, args).output().unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn test_git_command(dir: &Path, args: &[&str]) -> Command {
        let mut cmd = Command::new("git");
        cmd.current_dir(dir)
            .args([
                "-c",
                "init.defaultBranch=main",
                "-c",
                "user.name=cli-toolbox",
                "-c",
                "user.email=cli-toolbox@example.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args);
        cmd
    }
}
