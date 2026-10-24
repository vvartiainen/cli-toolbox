use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Error, exec};

const AGENT_WORKTREE_SUFFIX: &str = "-agent-wt";

/// Overrides the directory that holds agent worktrees.
pub const WORKTREES_DIR_ENV: &str = "CLI_TOOLBOX_WORKTREES_DIR";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentWorktree {
    pub name: String,
    pub path: PathBuf,
    pub branch: String,
}

#[derive(Clone)]
struct GitWorktree {
    path: PathBuf,
    branch: String,
}

/// The directory that holds agent worktrees, grouped by repository:
/// `$CLI_TOOLBOX_WORKTREES_DIR`, or `~/.agent-sandbox/worktrees`.
pub fn worktrees_root() -> Result<PathBuf, Error> {
    resolve_worktrees_root(std::env::var_os(WORKTREES_DIR_ENV), dirs::home_dir())
}

fn resolve_worktrees_root(dir: Option<OsString>, home: Option<PathBuf>) -> Result<PathBuf, Error> {
    if let Some(dir) = dir.filter(|dir| !dir.is_empty()) {
        return std::path::absolute(&dir)
            .map_err(|err| Error::msg(format!("resolve {WORKTREES_DIR_ENV}: {err}")));
    }
    let home =
        home.ok_or_else(|| Error::msg("resolve home directory: home directory not found"))?;
    Ok(home.join(".agent-sandbox").join("worktrees"))
}

/// Create the next agent worktree of the repository at `cwd` as
/// `<root>/<repo>/<repo>-agent-wt[-N]`, on a branch of the same name.
pub(crate) fn create_agent_worktree_record(
    cwd: &Path,
    root: &Path,
    stderr: &mut dyn Write,
) -> Result<AgentWorktree, Error> {
    require_git()?;
    let base = repo_name(cwd)?;
    let dir = root.join(&base);
    let used = used_agent_worktree_names(cwd, &base, &dir)?;
    let name = next_agent_worktree_name(&base, &used);
    let path = dir.join(&name);
    fs::create_dir_all(&dir)
        .map_err(|err| Error::msg(format!("create {}: {err}", dir.display())))?;
    run_git(
        cwd,
        stderr,
        &["worktree", "add", "-b", &name, &path.to_string_lossy()],
    )
    .map_err(|err| {
        Error::msg(format!(
            "create git worktree \"{}\" on branch \"{name}\": {err}",
            path.display()
        ))
    })?;
    Ok(AgentWorktree {
        name: name.clone(),
        path,
        branch: name,
    })
}

pub fn list_agent_worktrees(
    cwd: &Path,
    stdout: &mut dyn Write,
    _stderr: &mut dyn Write,
) -> Result<(), Error> {
    for worktree in agent_worktrees(cwd)? {
        writeln!(stdout, "{}", worktree.path.display())?;
    }
    Ok(())
}

/// The worktrees a cleanup targets: the one named (or the only one), or all
/// of them with `all`.
pub(crate) fn select_agent_worktrees(
    cwd: &Path,
    name: &str,
    all: bool,
) -> Result<Vec<AgentWorktree>, Error> {
    require_git()?;
    if all && !name.is_empty() {
        return Err(Error::msg(
            "specify either a worktree name or --all, not both",
        ));
    }

    let worktrees = agent_worktrees(cwd)?;
    if worktrees.is_empty() {
        return Err(Error::msg("no agent worktrees found"));
    }

    if all {
        Ok(worktrees)
    } else {
        Ok(vec![select_agent_worktree(&worktrees, name)?])
    }
}

pub fn agent_worktrees(cwd: &Path) -> Result<Vec<AgentWorktree>, Error> {
    require_git()?;
    let base = repo_name(cwd)?;
    let listed = list_git_worktrees(cwd)?;
    let mut worktrees: Vec<AgentWorktree> = listed
        .into_iter()
        .filter_map(|worktree| {
            let name = worktree.path.file_name()?.to_string_lossy().into_owned();
            if !is_agent_worktree_name(&base, &name) {
                return None;
            }
            Some(AgentWorktree {
                name,
                path: worktree.path,
                branch: worktree.branch,
            })
        })
        .collect();
    worktrees.sort_by(|a, b| {
        if worktree_name_less(&a.name, &b.name) {
            std::cmp::Ordering::Less
        } else if worktree_name_less(&b.name, &a.name) {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    });
    Ok(worktrees)
}

/// Agent worktrees are named after the main checkout, which git lists first,
/// so commands behave the same from the repository and from its worktrees.
fn repo_name(cwd: &Path) -> Result<String, Error> {
    let main = main_checkout(cwd)?;
    main.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(|| Error::msg(format!("no repository name in {}", main.display())))
}

fn next_agent_worktree_name(base: &str, used: &HashSet<String>) -> String {
    let mut n = 1;
    loop {
        let name = agent_worktree_name(base, n);
        if !used.contains(&name) {
            return name;
        }
        n += 1;
    }
}

fn agent_worktree_name(base: &str, n: usize) -> String {
    if n <= 1 {
        format!("{base}{AGENT_WORKTREE_SUFFIX}")
    } else {
        format!("{base}{AGENT_WORKTREE_SUFFIX}-{n}")
    }
}

fn is_agent_worktree_name(base: &str, name: &str) -> bool {
    let prefix = format!("{base}{AGENT_WORKTREE_SUFFIX}");
    if name == prefix {
        return true;
    }
    let Some(rest) = name.strip_prefix(&format!("{prefix}-")) else {
        return false;
    };
    if rest.is_empty() {
        return false;
    }
    rest.parse::<usize>()
        .is_ok_and(|n| n >= 1 && !rest.starts_with('0'))
}

fn worktree_name_less(a: &str, b: &str) -> bool {
    match (agent_worktree_index(a), agent_worktree_index(b)) {
        (Some(ai), Some(bi)) => ai < bi,
        _ => a < b,
    }
}

fn agent_worktree_index(name: &str) -> Option<usize> {
    let idx = name.rfind(AGENT_WORKTREE_SUFFIX)?;
    let rest = &name[idx + AGENT_WORKTREE_SUFFIX.len()..];
    if rest.is_empty() {
        return Some(1);
    }
    rest.strip_prefix('-')?
        .parse::<usize>()
        .ok()
        .filter(|n| *n >= 1)
}

/// Names taken by this repository's worktrees and branches, and by entries of
/// `dir`, which another repository with the same name may share.
fn used_agent_worktree_names(cwd: &Path, base: &str, dir: &Path) -> Result<HashSet<String>, Error> {
    let mut used = HashSet::new();
    match fs::read_dir(dir) {
        Ok(entries) => {
            for entry in entries {
                let name = entry?.file_name().to_string_lossy().into_owned();
                if is_agent_worktree_name(base, &name) {
                    used.insert(name);
                }
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(Error::msg(format!("read {}: {err}", dir.display()))),
    }

    for worktree in list_git_worktrees(cwd)? {
        if let Some(name) = worktree.path.file_name() {
            let name = name.to_string_lossy().into_owned();
            if is_agent_worktree_name(base, &name) {
                used.insert(name);
            }
        }
        if !worktree.branch.is_empty() && is_agent_worktree_name(base, &worktree.branch) {
            used.insert(worktree.branch);
        }
    }

    let branches = git_output(
        cwd,
        &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
    )?;
    for branch in branches.lines() {
        let branch = branch.trim();
        if is_agent_worktree_name(base, branch) {
            used.insert(branch.to_string());
        }
    }

    Ok(used)
}

fn list_git_worktrees(cwd: &Path) -> Result<Vec<GitWorktree>, Error> {
    let output = git_output(cwd, &["worktree", "list", "--porcelain"])?;
    Ok(parse_worktree_porcelain(&output))
}

fn parse_worktree_porcelain(output: &str) -> Vec<GitWorktree> {
    let mut worktrees = Vec::new();
    let mut current = GitWorktree {
        path: PathBuf::new(),
        branch: String::new(),
    };

    let mut flush = |current: &mut GitWorktree| {
        if !current.path.as_os_str().is_empty() {
            worktrees.push(current.clone());
            *current = GitWorktree {
                path: PathBuf::new(),
                branch: String::new(),
            };
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

fn select_agent_worktree(worktrees: &[AgentWorktree], name: &str) -> Result<AgentWorktree, Error> {
    if name.is_empty() {
        if worktrees.len() == 1 {
            return Ok(worktrees[0].clone());
        }
        return Err(Error::msg(
            "multiple agent worktrees found; specify a name or --all",
        ));
    }

    worktrees
        .iter()
        .find(|worktree| worktree_matches(worktree, name))
        .cloned()
        .ok_or_else(|| Error::msg(format!("agent worktree \"{name}\" not found")))
}

fn worktree_matches(worktree: &AgentWorktree, query: &str) -> bool {
    if query == worktree.name || Path::new(query) == worktree.path {
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

/// How [`remove_agent_worktree`] treats the branch and local changes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct RemoveOptions {
    /// Keep the worktree's branch, for example for review after the removal.
    pub(crate) keep_branch: bool,
    /// Remove the worktree even with uncommitted or untracked files.
    pub(crate) force: bool,
}

pub(crate) fn remove_agent_worktree(
    cwd: &Path,
    worktree: &AgentWorktree,
    options: RemoveOptions,
    stderr: &mut dyn Write,
) -> Result<(), Error> {
    let path = worktree.path.to_string_lossy();
    let mut args = vec!["worktree", "remove"];
    if options.force {
        args.push("--force");
    }
    args.push(&path);
    run_git(cwd, stderr, &args).map_err(|err| {
        Error::msg(format!(
            "remove git worktree \"{}\": {err}",
            worktree.path.display()
        ))
    })?;
    remove_empty_group_dir(worktree);
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

/// Drop `<root>/<repo>` once its last worktree is gone. Worktrees outside a
/// group directory, like older sibling ones, leave their parent alone.
fn remove_empty_group_dir(worktree: &AgentWorktree) {
    let Some(parent) = worktree.path.parent() else {
        return;
    };
    let in_group = parent.file_name().is_some_and(|group| {
        worktree.name.starts_with(&format!(
            "{}{AGENT_WORKTREE_SUFFIX}",
            group.to_string_lossy()
        ))
    });
    if in_group {
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
    use super::{
        RemoveOptions, agent_worktrees, create_agent_worktree_record, is_agent_worktree_name,
        list_agent_worktrees, next_agent_worktree_name, remove_agent_worktree,
        resolve_worktrees_root, select_agent_worktrees,
    };
    use crate::exec;
    use std::collections::HashSet;
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    #[test]
    fn worktrees_root_defaults_under_home_and_honors_override() {
        let home = Some(PathBuf::from("/home/me"));
        assert_eq!(
            resolve_worktrees_root(None, home.clone()).unwrap(),
            Path::new("/home/me/.agent-sandbox/worktrees")
        );
        assert_eq!(
            resolve_worktrees_root(Some("".into()), home.clone()).unwrap(),
            Path::new("/home/me/.agent-sandbox/worktrees")
        );
        assert_eq!(
            resolve_worktrees_root(Some("/srv/wt".into()), home).unwrap(),
            Path::new("/srv/wt")
        );
    }

    #[test]
    fn next_agent_worktree_name_fills_gaps() {
        assert_eq!(
            next_agent_worktree_name("cli-toolbox", &HashSet::new()),
            "cli-toolbox-agent-wt"
        );

        let used = HashSet::from([
            "cli-toolbox-agent-wt".into(),
            "cli-toolbox-agent-wt-2".into(),
        ]);
        assert_eq!(
            next_agent_worktree_name("cli-toolbox", &used),
            "cli-toolbox-agent-wt-3"
        );

        let used = HashSet::from(["cli-toolbox-agent-wt-2".into()]);
        assert_eq!(
            next_agent_worktree_name("cli-toolbox", &used),
            "cli-toolbox-agent-wt"
        );
    }

    #[test]
    fn agent_worktree_name_detection() {
        let cases = [
            ("cli-toolbox-agent-wt", true),
            ("cli-toolbox-agent-wt-2", true),
            ("cli-toolbox-agent-wt-10", true),
            ("cli-toolbox-agent-wt-1", true),
            ("cli-toolbox", false),
            ("cli-toolbox-agent-wt-02", false),
            ("cli-toolbox-agent-wt-extra", false),
            ("other-agent-wt", false),
        ];
        for (name, want) in cases {
            assert_eq!(is_agent_worktree_name("cli-toolbox", name), want, "{name}");
        }
    }

    #[test]
    fn create_agent_worktree_creates_worktree_and_branch_under_root() {
        if skip_without_git() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("demo");
        init_git_repo(&repo);
        let wt_root = root.path().join("worktrees");

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        create_agent_worktree(&repo, &wt_root, &mut stdout, &mut stderr).unwrap_or_else(|err| {
            panic!(
                "create: {err}\nstderr: {}",
                String::from_utf8_lossy(&stderr)
            );
        });

        let want_path = wt_root.join("demo").join("demo-agent-wt");
        assert_same_path(String::from_utf8(stdout).unwrap().trim(), &want_path);
        assert!(want_path.exists());
        assert_eq!(
            must_git_output(&want_path, &["rev-parse", "--abbrev-ref", "HEAD"]),
            "demo-agent-wt"
        );
        assert_worktree_listed(&repo, &want_path, "demo-agent-wt");
    }

    #[test]
    fn create_agent_worktree_record_returns_created_worktree() {
        if skip_without_git() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("demo");
        init_git_repo(&repo);
        let wt_root = root.path().join("worktrees");

        let mut stderr = Vec::new();
        let worktree = create_agent_worktree_record(&repo, &wt_root, &mut stderr).unwrap();

        assert_eq!(worktree.name, "demo-agent-wt");
        assert_same_path(
            worktree.path.to_string_lossy(),
            &wt_root.join("demo").join("demo-agent-wt"),
        );
        assert_eq!(worktree.branch, "demo-agent-wt");
        assert!(worktree.path.exists());
    }

    #[test]
    fn create_agent_worktree_allocates_next_name() {
        if skip_without_git() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("demo");
        init_git_repo(&repo);
        let wt_root = root.path().join("worktrees");

        let mut stderr = Vec::new();
        create_agent_worktree(&repo, &wt_root, &mut io::sink(), &mut stderr).unwrap();
        let mut stdout = Vec::new();
        create_agent_worktree(&repo, &wt_root, &mut stdout, &mut stderr).unwrap();

        let want_path = wt_root.join("demo").join("demo-agent-wt-2");
        assert_same_path(String::from_utf8(stdout).unwrap().trim(), &want_path);
        assert_eq!(
            must_git_output(&want_path, &["rev-parse", "--abbrev-ref", "HEAD"]),
            "demo-agent-wt-2"
        );
    }

    #[test]
    fn create_agent_worktree_from_a_worktree_names_after_main_checkout() {
        if skip_without_git() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("demo");
        init_git_repo(&repo);
        let wt_root = root.path().join("worktrees");

        let mut stderr = Vec::new();
        let first = create_agent_worktree_record(&repo, &wt_root, &mut stderr).unwrap();
        let second = create_agent_worktree_record(&first.path, &wt_root, &mut stderr).unwrap();

        assert_eq!(second.name, "demo-agent-wt-2");
        let names: Vec<_> = agent_worktrees(&first.path)
            .unwrap()
            .into_iter()
            .map(|wt| wt.name)
            .collect();
        assert_eq!(names, ["demo-agent-wt", "demo-agent-wt-2"]);
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

        let mut stderr = Vec::new();
        create_agent_worktree_record(&one, &wt_root, &mut stderr).unwrap();
        let other = create_agent_worktree_record(&two, &wt_root, &mut stderr).unwrap();

        assert_eq!(other.name, "demo-agent-wt-2");
        assert_eq!(agent_worktrees(&one).unwrap()[0].name, "demo-agent-wt");
        assert_eq!(agent_worktrees(&two).unwrap()[0].name, "demo-agent-wt-2");
    }

    #[test]
    fn list_agent_worktrees_prints_sorted_paths() {
        if skip_without_git() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("demo");
        init_git_repo(&repo);
        let wt_root = root.path().join("worktrees");

        let mut stderr = Vec::new();
        create_agent_worktree(&repo, &wt_root, &mut io::sink(), &mut stderr).unwrap();
        create_agent_worktree(&repo, &wt_root, &mut io::sink(), &mut stderr).unwrap();

        let worktrees = agent_worktrees(&repo).unwrap();
        let names: Vec<_> = worktrees.iter().map(|wt| wt.name.as_str()).collect();
        let branches: Vec<_> = worktrees.iter().map(|wt| wt.branch.as_str()).collect();
        assert_eq!(names, ["demo-agent-wt", "demo-agent-wt-2"]);
        assert_eq!(branches, ["demo-agent-wt", "demo-agent-wt-2"]);

        let mut stdout = Vec::new();
        list_agent_worktrees(&repo, &mut stdout, &mut stderr).unwrap();
        let got: Vec<_> = String::from_utf8(stdout)
            .unwrap()
            .trim()
            .lines()
            .map(str::to_string)
            .collect();
        let want = [
            wt_root.join("demo").join("demo-agent-wt"),
            wt_root.join("demo").join("demo-agent-wt-2"),
        ];
        assert_eq!(got.len(), want.len());
        for (got, want) in got.iter().zip(want.iter()) {
            assert_same_path(got, want);
        }
    }

    #[test]
    fn cleanup_removes_named_worktree() {
        if skip_without_git() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("demo");
        init_git_repo(&repo);
        let wt_root = root.path().join("worktrees");

        let mut stderr = Vec::new();
        create_agent_worktree(&repo, &wt_root, &mut io::sink(), &mut stderr).unwrap();
        create_agent_worktree(&repo, &wt_root, &mut io::sink(), &mut stderr).unwrap();

        let mut stdout = Vec::new();
        cleanup_agent_worktree(&repo, "demo-agent-wt", false, &mut stdout, &mut stderr).unwrap();

        let want_path = wt_root.join("demo").join("demo-agent-wt");
        assert_same_path(String::from_utf8(stdout).unwrap().trim(), &want_path);
        assert!(!want_path.exists());

        let remaining = agent_worktrees(&repo).unwrap();
        assert_eq!(remaining.len(), 1);
        assert!(wt_root.join("demo").exists());
        assert_eq!(remaining[0].name, "demo-agent-wt-2");

        let branches = must_git_output(
            &repo,
            &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
        );
        let branch_set: HashSet<_> = branches.lines().collect();
        assert!(!branch_set.contains("demo-agent-wt"));
        assert!(branch_set.contains("demo-agent-wt-2"));
    }

    #[test]
    fn cleanup_refuses_dirty_worktree_without_force() {
        if skip_without_git() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("demo");
        init_git_repo(&repo);
        let wt_root = root.path().join("worktrees");

        let mut stderr = Vec::new();
        create_agent_worktree(&repo, &wt_root, &mut io::sink(), &mut stderr).unwrap();
        let worktree_path = wt_root.join("demo").join("demo-agent-wt");
        fs::write(worktree_path.join("README.md"), "uncommitted change\n").unwrap();

        let mut cleanup_stderr = Vec::new();
        let result = cleanup_agent_worktree(
            &repo,
            "demo-agent-wt",
            false,
            &mut Vec::new(),
            &mut cleanup_stderr,
        );

        let error = result.unwrap_err().to_string();
        assert!(error.contains("remove git worktree"), "{error}");
        assert!(worktree_path.exists());
        assert_eq!(agent_worktrees(&repo).unwrap().len(), 1);
    }

    #[test]
    fn cleanup_reports_branch_failure_after_worktree_removal() {
        if skip_without_git() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("demo");
        init_git_repo(&repo);
        let wt_root = root.path().join("worktrees");

        let mut stderr = Vec::new();
        create_agent_worktree(&repo, &wt_root, &mut io::sink(), &mut stderr).unwrap();
        let worktree_path = wt_root.join("demo").join("demo-agent-wt");
        let held_path = wt_root.join("demo").join("demo-agent-wt-held");
        must_run_git(
            &repo,
            &[
                "worktree",
                "add",
                "--force",
                held_path.to_str().unwrap(),
                "demo-agent-wt",
            ],
        );

        let mut cleanup_stderr = Vec::new();
        let result = cleanup_agent_worktree(
            &repo,
            "demo-agent-wt",
            false,
            &mut Vec::new(),
            &mut cleanup_stderr,
        );

        let error = result.unwrap_err().to_string();
        assert!(
            error.contains("delete git branch \"demo-agent-wt\""),
            "{error}"
        );
        assert!(error.contains("was removed"), "{error}");
        assert!(!cleanup_stderr.is_empty());
        assert!(!worktree_path.exists());
        assert!(held_path.exists());
        let branches = must_git_output(
            &repo,
            &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
        );
        assert!(branches.lines().any(|branch| branch == "demo-agent-wt"));

        must_run_git(
            &repo,
            &["worktree", "remove", "--force", held_path.to_str().unwrap()],
        );
    }

    #[test]
    fn cleanup_removes_only_worktree_when_unnamed() {
        if skip_without_git() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("demo");
        init_git_repo(&repo);
        let wt_root = root.path().join("worktrees");

        let mut stderr = Vec::new();
        create_agent_worktree(&repo, &wt_root, &mut io::sink(), &mut stderr).unwrap();
        cleanup_agent_worktree(&repo, "", false, &mut io::sink(), &mut stderr).unwrap();
        assert!(agent_worktrees(&repo).unwrap().is_empty());
    }

    #[test]
    fn remove_can_keep_the_branch_for_review() {
        if skip_without_git() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("demo");
        init_git_repo(&repo);
        let wt_root = root.path().join("worktrees");
        create_agent_worktree(&repo, &wt_root, &mut io::sink(), &mut io::sink()).unwrap();
        let worktree = &agent_worktrees(&repo).unwrap()[0];
        fs::write(worktree.path.join("scratch"), "untracked\n").unwrap();

        let options = RemoveOptions {
            keep_branch: true,
            force: true,
        };
        remove_agent_worktree(&repo, worktree, options, &mut io::sink()).unwrap();

        assert!(!worktree.path.exists());
        let branches = must_git_output(
            &repo,
            &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
        );
        assert!(branches.lines().any(|branch| branch == "demo-agent-wt"));
    }

    #[test]
    fn cleanup_all_agent_worktrees() {
        if skip_without_git() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("demo");
        init_git_repo(&repo);
        let wt_root = root.path().join("worktrees");

        let mut stderr = Vec::new();
        create_agent_worktree(&repo, &wt_root, &mut io::sink(), &mut stderr).unwrap();
        create_agent_worktree(&repo, &wt_root, &mut io::sink(), &mut stderr).unwrap();
        cleanup_agent_worktree(&repo, "", true, &mut io::sink(), &mut stderr).unwrap();
        assert!(agent_worktrees(&repo).unwrap().is_empty());
        assert!(!wt_root.join("demo").exists());
    }

    /// Create a worktree and print its path, like `agent worktree create`
    /// without the init and Herdr steps.
    fn create_agent_worktree(
        cwd: &Path,
        root: &Path,
        stdout: &mut dyn io::Write,
        stderr: &mut dyn io::Write,
    ) -> Result<(), crate::Error> {
        let worktree = create_agent_worktree_record(cwd, root, stderr)?;
        writeln!(stdout, "{}", worktree.path.display())?;
        Ok(())
    }

    /// Remove worktrees and print their paths, like `agent worktree cleanup`
    /// without the Herdr step.
    fn cleanup_agent_worktree(
        cwd: &Path,
        name: &str,
        all: bool,
        stdout: &mut dyn io::Write,
        stderr: &mut dyn io::Write,
    ) -> Result<(), crate::Error> {
        for worktree in select_agent_worktrees(cwd, name, all)? {
            remove_agent_worktree(cwd, &worktree, RemoveOptions::default(), stderr)?;
            writeln!(stdout, "{}", worktree.path.display())?;
        }
        Ok(())
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

    fn must_git_output(dir: &Path, args: &[&str]) -> String {
        let output = test_git_command(dir, args).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    }

    fn must_run_git(dir: &Path, args: &[&str]) {
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
