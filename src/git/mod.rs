use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Error, exec};

const AGENT_WORKTREE_SUFFIX: &str = "-agent-wt";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentWorktree {
    pub name: String,
    pub path: PathBuf,
    pub branch: String,
}

struct CwdLayout {
    parent: PathBuf,
    base: String,
}

#[derive(Clone)]
struct GitWorktree {
    path: PathBuf,
    branch: String,
}

pub fn create_agent_worktree(
    cwd: &Path,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Error> {
    require_git()?;
    let layout = resolve_cwd(cwd)?;
    let used = used_agent_worktree_names(cwd, &layout)?;
    let name = next_agent_worktree_name(&layout.base, &used);
    let path = layout.parent.join(&name);
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
    writeln!(stdout, "{}", path.display())?;
    Ok(())
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

pub fn cleanup_agent_worktree(
    cwd: &Path,
    name: &str,
    all: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Error> {
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

    let selected = if all {
        worktrees
    } else {
        vec![select_agent_worktree(&worktrees, name)?]
    };

    for worktree in selected {
        remove_agent_worktree(cwd, &worktree, stderr)?;
        writeln!(stdout, "{}", worktree.path.display())?;
    }
    Ok(())
}

pub fn agent_worktrees(cwd: &Path) -> Result<Vec<AgentWorktree>, Error> {
    require_git()?;
    let layout = resolve_cwd(cwd)?;
    let listed = list_git_worktrees(cwd)?;
    let mut worktrees: Vec<AgentWorktree> = listed
        .into_iter()
        .filter_map(|worktree| {
            let name = worktree.path.file_name()?.to_string_lossy().into_owned();
            if !is_agent_worktree_name(&layout.base, &name) {
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

fn resolve_cwd(cwd: &Path) -> Result<CwdLayout, Error> {
    let mut abs = cwd
        .canonicalize()
        .or_else(|_| std::path::absolute(cwd))
        .map_err(|err| Error::msg(format!("resolve working directory: {err}")))?;
    if let Ok(resolved) = fs::canonicalize(&abs) {
        abs = resolved;
    }
    let base = abs
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let parent = abs.parent().unwrap_or(Path::new("/")).to_path_buf();
    Ok(CwdLayout { parent, base })
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

fn used_agent_worktree_names(cwd: &Path, layout: &CwdLayout) -> Result<HashSet<String>, Error> {
    let mut used = HashSet::new();
    let entries = fs::read_dir(&layout.parent)
        .map_err(|err| Error::msg(format!("read {}: {err}", layout.parent.display())))?;
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_agent_worktree_name(&layout.base, &name) {
            used.insert(name);
        }
    }

    for worktree in list_git_worktrees(cwd)? {
        if let Some(name) = worktree.path.file_name() {
            let name = name.to_string_lossy().into_owned();
            if is_agent_worktree_name(&layout.base, &name) {
                used.insert(name);
            }
        }
        if !worktree.branch.is_empty() && is_agent_worktree_name(&layout.base, &worktree.branch) {
            used.insert(worktree.branch);
        }
    }

    let branches = git_output(
        cwd,
        &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
    )?;
    for branch in branches.lines() {
        let branch = branch.trim();
        if is_agent_worktree_name(&layout.base, branch) {
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

fn remove_agent_worktree(
    cwd: &Path,
    worktree: &AgentWorktree,
    stderr: &mut dyn Write,
) -> Result<(), Error> {
    run_git(
        cwd,
        stderr,
        &["worktree", "remove", &worktree.path.to_string_lossy()],
    )
    .map_err(|err| {
        Error::msg(format!(
            "remove git worktree \"{}\": {err}",
            worktree.path.display()
        ))
    })?;
    if worktree.branch.is_empty() {
        return Ok(());
    }
    run_git(cwd, stderr, &["branch", "-D", "--", &worktree.branch])
        .map_err(|err| Error::msg(format!("delete git branch \"{}\": {err}", worktree.branch)))
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
mod tests {
    use super::{
        agent_worktrees, cleanup_agent_worktree, create_agent_worktree, is_agent_worktree_name,
        list_agent_worktrees, next_agent_worktree_name,
    };
    use crate::exec;
    use std::collections::HashSet;
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::process::Command;

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
    fn create_agent_worktree_creates_sibling_worktree_and_branch() {
        if skip_without_git() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("demo");
        init_git_repo(&repo);

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        create_agent_worktree(&repo, &mut stdout, &mut stderr).unwrap_or_else(|err| {
            panic!(
                "create: {err}\nstderr: {}",
                String::from_utf8_lossy(&stderr)
            );
        });

        let want_path = root.path().join("demo-agent-wt");
        assert_same_path(String::from_utf8(stdout).unwrap().trim(), &want_path);
        assert!(want_path.exists());
        assert_eq!(
            must_git_output(&want_path, &["rev-parse", "--abbrev-ref", "HEAD"]),
            "demo-agent-wt"
        );
        assert_worktree_listed(&repo, &want_path, "demo-agent-wt");
    }

    #[test]
    fn create_agent_worktree_allocates_next_name() {
        if skip_without_git() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("demo");
        init_git_repo(&repo);

        let mut stderr = Vec::new();
        create_agent_worktree(&repo, &mut io::sink(), &mut stderr).unwrap();
        let mut stdout = Vec::new();
        create_agent_worktree(&repo, &mut stdout, &mut stderr).unwrap();

        let want_path = root.path().join("demo-agent-wt-2");
        assert_same_path(String::from_utf8(stdout).unwrap().trim(), &want_path);
        assert_eq!(
            must_git_output(&want_path, &["rev-parse", "--abbrev-ref", "HEAD"]),
            "demo-agent-wt-2"
        );
    }

    #[test]
    fn list_agent_worktrees_prints_sorted_paths() {
        if skip_without_git() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("demo");
        init_git_repo(&repo);

        let mut stderr = Vec::new();
        create_agent_worktree(&repo, &mut io::sink(), &mut stderr).unwrap();
        create_agent_worktree(&repo, &mut io::sink(), &mut stderr).unwrap();

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
            root.path().join("demo-agent-wt"),
            root.path().join("demo-agent-wt-2"),
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

        let mut stderr = Vec::new();
        create_agent_worktree(&repo, &mut io::sink(), &mut stderr).unwrap();
        create_agent_worktree(&repo, &mut io::sink(), &mut stderr).unwrap();

        let mut stdout = Vec::new();
        cleanup_agent_worktree(&repo, "demo-agent-wt", false, &mut stdout, &mut stderr).unwrap();

        let want_path = root.path().join("demo-agent-wt");
        assert_same_path(String::from_utf8(stdout).unwrap().trim(), &want_path);
        assert!(!want_path.exists());

        let remaining = agent_worktrees(&repo).unwrap();
        assert_eq!(remaining.len(), 1);
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
    fn cleanup_removes_only_worktree_when_unnamed() {
        if skip_without_git() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("demo");
        init_git_repo(&repo);

        let mut stderr = Vec::new();
        create_agent_worktree(&repo, &mut io::sink(), &mut stderr).unwrap();
        cleanup_agent_worktree(&repo, "", false, &mut io::sink(), &mut stderr).unwrap();
        assert!(agent_worktrees(&repo).unwrap().is_empty());
    }

    #[test]
    fn cleanup_all_agent_worktrees() {
        if skip_without_git() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("demo");
        init_git_repo(&repo);

        let mut stderr = Vec::new();
        create_agent_worktree(&repo, &mut io::sink(), &mut stderr).unwrap();
        create_agent_worktree(&repo, &mut io::sink(), &mut stderr).unwrap();
        cleanup_agent_worktree(&repo, "", true, &mut io::sink(), &mut stderr).unwrap();
        assert!(agent_worktrees(&repo).unwrap().is_empty());
    }

    fn skip_without_git() -> bool {
        if exec::look_path("git").is_none() {
            eprintln!("skipping: git is not available on PATH");
            true
        } else {
            false
        }
    }

    fn init_git_repo(dir: &Path) {
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
