//! One-action host review of an agent-owned worktree.

use std::fs;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::sandbox::{self, RunnerState, project::Project, sync::Bridge};
use crate::{Error, exec, git, select};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Prepared {
    pub(crate) branch: String,
    pub(crate) path: PathBuf,
}

/// Refresh a live snapshot (or choose a saved one), then enter its own host
/// checkout. The launcher already releases its terminal before invoking us.
pub(crate) fn open(
    project: &Project,
    worktree: Option<&str>,
    stderr: &mut dyn Write,
) -> Result<Option<Prepared>, Error> {
    let target = prepare_with(
        project.repo()?,
        &project.state_dir.join("reviews"),
        || {
            exec::look_path("lazygit").ok_or_else(|| {
                Error::msg("review requires lazygit on the host; install it and try again")
            })?;
            if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
                return Err(Error::msg("review requires an interactive terminal"));
            }
            Ok(())
        },
        || {
            let vm = sandbox::vm_for(&super::vm::running_vms(), &project.dir);
            snapshot_for_review(project, worktree, vm.as_ref(), stderr, |labels| {
                select::pick("review> ", labels, "40%")
            })
        },
    )?;
    if let Some(target) = &target {
        writeln!(
            stderr,
            "Reviewing {} in {}",
            target.branch,
            target.path.display()
        )?;
        writeln!(
            stderr,
            "Opening lazygit's commits view; these changes are committed, not unstaged files."
        )?;
        launch(target, |program, args, cwd| {
            exec::run_on_stderr_in(program, args, Some(cwd), &[])
        })?;
    }
    Ok(target)
}

fn snapshot_for_review(
    project: &Project,
    name: Option<&str>,
    vm: Option<&RunnerState>,
    stderr: &mut dyn Write,
    mut pick: impl FnMut(&[String]) -> Result<Option<usize>, Error>,
) -> Result<Option<String>, Error> {
    if let Some(vm) = vm {
        let worktrees: Vec<_> = project
            .worktrees()?
            .into_iter()
            .filter(|worktree| name.is_none_or(|name| worktree.name == name))
            .collect();
        if !worktrees.is_empty() {
            let labels: Vec<_> = worktrees
                .iter()
                .map(|worktree| worktree.name.clone())
                .collect();
            let Some(index) = choose(&labels, &mut pick)? else {
                return Ok(None);
            };
            let worktree = &worktrees[index];
            if !project.mounts(&worktree.name) {
                return Err(Error::msg(format!(
                    "{} is not mounted by the VM; restart it before reviewing new work",
                    worktree.name
                )));
            }
            let review =
                Bridge::for_vm(vm, &worktree.name)?.fetch(&worktree.path, &worktree.branch)?;
            let branch = review
                .snapshots
                .first()
                .cloned()
                .ok_or_else(|| Error::msg("sync did not publish a review snapshot"))?;
            writeln!(stderr, "Fetched review snapshot: {branch}")?;
            return Ok(Some(branch));
        }
    }

    let labels = saved_snapshots(project, name)?;
    if labels.is_empty() {
        return Err(Error::msg(
            "no saved review snapshots for this project/worktree; start its VM and review committed agent work first",
        ));
    }
    writeln!(
        stderr,
        "Opening a saved snapshot; no new guest work is being fetched."
    )?;
    Ok(choose(&labels, &mut pick)?.map(|index| labels[index].clone()))
}

fn saved_snapshots(project: &Project, name: Option<&str>) -> Result<Vec<String>, Error> {
    let branches = git_output(
        project.repo()?,
        &[
            "for-each-ref",
            "--sort=-committerdate",
            "--format=%(refname:strip=2)",
            "refs/heads/agent-review/",
        ],
    )?;
    let dir = project
        .dir
        .canonicalize()
        .unwrap_or_else(|_| project.dir.clone());
    Ok(branches
        .lines()
        .filter(|branch| {
            let Some((workspace, leaf)) = branch
                .strip_prefix("agent-review/")
                .and_then(|rest| rest.split_once('/'))
            else {
                return false;
            };
            name.is_none_or(|name| workspace == name)
                && leaf.contains(&format!("-{}-", sandbox::sync::source_id(&dir, workspace)))
        })
        .map(str::to_string)
        .collect())
}

fn choose(
    labels: &[String],
    pick: &mut impl FnMut(&[String]) -> Result<Option<usize>, Error>,
) -> Result<Option<usize>, Error> {
    if labels.len() == 1 {
        return Ok(Some(0));
    }
    let index = pick(labels)?;
    if index.is_some_and(|index| index >= labels.len()) {
        return Err(Error::msg("invalid review selection"));
    }
    Ok(index)
}

fn prepare_with(
    repo: &Path,
    root: &Path,
    require: impl FnOnce() -> Result<(), Error>,
    snapshot: impl FnOnce() -> Result<Option<String>, Error>,
) -> Result<Option<Prepared>, Error> {
    require()?;
    snapshot()?
        .map(|branch| prepare(repo, root, &branch))
        .transpose()
}

fn prepare(repo: &Path, root: &Path, branch: &str) -> Result<Prepared, Error> {
    if !branch.starts_with("agent-review/") {
        return Err(Error::msg(
            "review requires an agent-review snapshot branch",
        ));
    }
    let reference = format!("refs/heads/{branch}");
    let tip = git::resolve_commit(repo, &reference)
        .ok_or_else(|| Error::msg(format!("review snapshot {branch} does not exist")))?;
    let key = format!(
        "{:x}",
        Sha256::digest(format!("{branch}\0{tip}").as_bytes())
    );
    fs::create_dir_all(root)?;
    let root = root.canonicalize()?;
    let common = common_dir(repo)?;
    let registered = git_output(repo, &["worktree", "list", "--porcelain", "-z"])?;
    let paths: Vec<_> = registered
        .split('\0')
        .filter_map(|entry| entry.strip_prefix("worktree "))
        .map(PathBuf::from)
        .collect();
    for attempt in 0..100 {
        let folder = if attempt == 0 {
            key[..24].to_string()
        } else {
            format!("{}-{attempt}", &key[..24])
        };
        let path = root.join(folder);
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if metadata.is_dir()
                    && paths.contains(&path)
                    && common_dir(&path).is_ok_and(|directory| directory == common)
                    && git_output(&path, &["rev-parse", "--show-toplevel"]).is_ok_and(|directory| {
                        Path::new(directory.trim()).canonicalize().ok().as_ref() == Some(&path)
                    })
                    && git::resolve_commit(&path, "HEAD").as_deref() == Some(&tip)
                    && git_output(&path, &["symbolic-ref", "-q", "HEAD"]).is_err()
                {
                    // Reuse without reset/checkout/clean: even review edits stay.
                    return Ok(Prepared {
                        branch: branch.into(),
                        path,
                    });
                }
                continue;
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }
        if paths.contains(&path) {
            // A missing directory can still have a registered Git worktree.
            continue;
        }
        let path_text = path
            .to_str()
            .ok_or_else(|| Error::msg("review worktree path must be UTF-8"))?;
        git_output(
            repo,
            &["worktree", "add", "--detach", "--", path_text, &tip],
        )?;
        return Ok(Prepared {
            branch: branch.into(),
            path,
        });
    }
    Err(Error::msg(
        "could not allocate a separate review worktree; existing copies were left untouched",
    ))
}

fn launch(
    target: &Prepared,
    run: impl FnOnce(&str, &[&str], &Path) -> Result<(), Error>,
) -> Result<(), Error> {
    run("lazygit", &["log"], &target.path).map_err(|err| {
        Error::msg(format!(
            "lazygit review failed; snapshot {} and review files at {} remain available: {err}",
            target.branch,
            target.path.display()
        ))
    })
}

fn common_dir(repo: &Path) -> Result<PathBuf, Error> {
    let directory = git_output(
        repo,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    Ok(Path::new(directory.trim()).canonicalize()?)
}

fn git_output(repo: &Path, args: &[&str]) -> Result<String, Error> {
    exec::captured_stdout_in("git", args, Some(repo), &[])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::tests::{commit_all, init_git_repo, skip_without_git};

    fn git(repo: &Path, args: &[&str]) -> String {
        exec::captured_stdout_in("git", args, Some(repo), &[])
            .unwrap()
            .trim()
            .into()
    }

    #[test]
    fn review_opens_committed_work_in_an_independent_reusable_checkout() {
        if skip_without_git() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        init_git_repo(&repo);
        std::fs::write(repo.join("feature.txt"), "committed\n").unwrap();
        commit_all(&repo);
        let branch = "agent-review/demo-agent-wt/snapshot";
        git(&repo, &["branch", branch]);
        let head = git(&repo, &["rev-parse", "HEAD"]);
        let index = std::fs::read(repo.join(".git/index")).unwrap();
        std::fs::write(repo.join("feature.txt"), "agent still editing\n").unwrap();
        let root = temp.path().join("reviews");
        let target = prepare_with(&repo, &root, || Ok(()), || Ok(Some(branch.into())))
            .unwrap()
            .unwrap();
        assert_ne!(target.path, repo);
        assert_eq!(git(&target.path, &["rev-parse", "HEAD"]), head);
        assert_eq!(
            std::fs::read_to_string(target.path.join("feature.txt")).unwrap(),
            "committed\n"
        );
        launch(&target, |program, args, cwd| {
            assert_eq!(program, "lazygit");
            assert_eq!(args, ["log"]);
            assert_eq!(cwd, target.path);
            Ok(())
        })
        .unwrap();
        std::fs::write(target.path.join("review-notes.txt"), "keep notes\n").unwrap();
        std::fs::write(target.path.join("feature.txt"), "review edits\n").unwrap();
        assert_eq!(prepare(&repo, &root, branch).unwrap(), target);
        assert_eq!(
            std::fs::read_to_string(target.path.join("review-notes.txt")).unwrap(),
            "keep notes\n"
        );
        assert_eq!(
            std::fs::read_to_string(target.path.join("feature.txt")).unwrap(),
            "review edits\n"
        );
        assert_eq!(git(&repo, &["rev-parse", "HEAD"]), head);
        assert_eq!(std::fs::read(repo.join(".git/index")).unwrap(), index);
        assert_eq!(
            std::fs::read_to_string(repo.join("feature.txt")).unwrap(),
            "agent still editing\n"
        );
    }

    fn fixture_project(root: &Path, repo: &Path) -> Project {
        let dir = root.join("project");
        fs::create_dir_all(&dir).unwrap();
        Project {
            name: "demo".into(),
            dir,
            repo: Some(repo.into()),
            state_dir: root.join("state"),
            worktrees_root: root.join("worktrees"),
        }
    }

    #[test]
    fn saved_reviews_are_scoped_to_the_project_and_worktree() {
        if skip_without_git() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        init_git_repo(&repo);
        let project = fixture_project(temp.path(), &repo);
        let source =
            sandbox::sync::source_id(&project.dir.canonicalize().unwrap(), "repo-agent-wt");
        let expected = format!("agent-review/repo-agent-wt/date-{source}-tip");
        let expected_two = format!("agent-review/repo-agent-wt/other-{source}-tip");
        for branch in [
            &expected,
            &expected_two,
            "agent-review/repo-agent-wt/date-foreign-tip",
        ] {
            git(&repo, &["branch", branch]);
        }
        assert_eq!(
            saved_snapshots(&project, Some("repo-agent-wt"))
                .unwrap()
                .len(),
            2
        );
        assert!(
            saved_snapshots(&project, Some("not-this-worktree"))
                .unwrap()
                .is_empty()
        );
        let mut report = Vec::new();
        let branch = snapshot_for_review(
            &project,
            Some("repo-agent-wt"),
            None,
            &mut report,
            |labels| {
                assert_eq!(labels.len(), 2);
                assert!(labels.contains(&expected));
                Ok(labels.iter().position(|label| label == &expected))
            },
        )
        .unwrap();
        assert_eq!(branch, Some(expected));
        assert!(
            String::from_utf8(report)
                .unwrap()
                .contains("saved snapshot")
        );
        assert!(
            snapshot_for_review(&project, None, None, &mut Vec::new(), |_| Ok(None))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn live_review_does_not_fetch_an_unmounted_worktree() {
        if skip_without_git() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        init_git_repo(&repo);
        let agent = temp.path().join("worktrees/repo/repo-agent-wt");
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-b",
                "repo-agent-wt",
                agent.to_str().unwrap(),
            ],
        );
        let project = fixture_project(temp.path(), &repo);
        fs::create_dir_all(&project.state_dir).unwrap();
        fs::write(project.state_dir.join("mounted-worktrees.nix"), "{\n}\n").unwrap();
        let error = snapshot_for_review(
            &project,
            Some("repo-agent-wt"),
            Some(&RunnerState::default()),
            &mut Vec::new(),
            |_| panic!("one worktree needs no picker"),
        )
        .unwrap_err();
        assert!(error.to_string().contains("not mounted"), "{error}");
    }

    #[test]
    fn repurposed_foreign_checkout_is_not_reused_even_at_the_same_tip() {
        if skip_without_git() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        init_git_repo(&repo);
        let branch = "agent-review/demo/snapshot";
        git(&repo, &["branch", branch]);
        let root = temp.path().join("reviews");
        let original = prepare(&repo, &root, branch).unwrap();
        fs::rename(&original.path, temp.path().join("old-copy")).unwrap();
        git(
            temp.path(),
            &[
                "clone",
                repo.to_str().unwrap(),
                original.path.to_str().unwrap(),
            ],
        );
        git(&original.path, &["checkout", "--detach"]);
        fs::write(original.path.join("foreign.txt"), "preserve\n").unwrap();
        let replacement = prepare(&repo, &root, branch).unwrap();
        assert_ne!(replacement.path, original.path);
        assert_eq!(
            fs::read_to_string(original.path.join("foreign.txt")).unwrap(),
            "preserve\n"
        );
        assert_eq!(
            common_dir(&replacement.path).unwrap(),
            common_dir(&repo).unwrap()
        );
    }

    #[test]
    fn occupied_and_missing_registered_paths_are_not_overwritten_or_pruned() {
        if skip_without_git() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        init_git_repo(&repo);
        let branch = "agent-review/demo/snapshot";
        git(&repo, &["branch", branch]);
        let root = temp.path().join("reviews");
        let first = prepare(&repo, &root, branch).unwrap();
        let moved = temp.path().join("moved-review");
        git(
            &repo,
            &[
                "worktree",
                "move",
                first.path.to_str().unwrap(),
                moved.to_str().unwrap(),
            ],
        );
        fs::create_dir(&first.path).unwrap();
        fs::write(first.path.join("user.txt"), "do not overwrite\n").unwrap();
        let second = prepare(&repo, &root, branch).unwrap();
        assert_ne!(second.path, first.path);
        assert_eq!(
            fs::read_to_string(first.path.join("user.txt")).unwrap(),
            "do not overwrite\n"
        );
        // A missing checkout still has live registration metadata: allocate
        // another copy, never prune or force-replace that entry.
        fs::remove_dir_all(&second.path).unwrap();
        let third = prepare(&repo, &root, branch).unwrap();
        assert_ne!(third.path, second.path);
        let list = git(&repo, &["worktree", "list", "--porcelain"]);
        assert!(list.contains(second.path.to_str().unwrap()));
        assert!(moved.exists());
    }

    #[test]
    #[cfg(unix)]
    fn symlinked_review_paths_are_not_followed() {
        use std::os::unix::fs::symlink;
        if skip_without_git() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        init_git_repo(&repo);
        let branch = "agent-review/demo/snapshot";
        git(&repo, &["branch", branch]);
        let root = temp.path().join("reviews");
        let first = prepare(&repo, &root, branch).unwrap();
        let moved = temp.path().join("moved-review");
        git(
            &repo,
            &[
                "worktree",
                "move",
                first.path.to_str().unwrap(),
                moved.to_str().unwrap(),
            ],
        );
        symlink(&moved, &first.path).unwrap();
        let second = prepare(&repo, &root, branch).unwrap();
        assert_ne!(second.path, first.path);
        assert!(fs::symlink_metadata(&first.path).unwrap().is_symlink());
        assert!(moved.exists());
    }

    #[test]
    fn missing_viewer_and_cancelled_selection_do_not_prepare_worktrees() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("reviews");
        let result = prepare_with(
            temp.path(),
            &root,
            || Err(Error::msg("lazygit missing")),
            || panic!("must not fetch before checking lazygit"),
        );
        assert!(result.is_err());
        assert!(!root.exists());
        assert!(
            prepare_with(temp.path(), &root, || Ok(()), || Ok(None))
                .unwrap()
                .is_none()
        );
        assert!(!root.exists());
    }

    #[test]
    fn repurposed_review_paths_and_viewer_failures_preserve_user_work() {
        if skip_without_git() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        init_git_repo(&repo);
        let branch = "agent-review/demo-agent-wt/snapshot";
        git(&repo, &["branch", branch]);
        let root = temp.path().join("reviews");
        let first = prepare(&repo, &root, branch).unwrap();
        std::fs::write(first.path.join("notes.txt"), "notes\n").unwrap();
        git(&first.path, &["config", "user.email", "test@example.com"]);
        git(&first.path, &["config", "user.name", "Test"]);
        commit_all(&first.path);
        let moved = git(&first.path, &["rev-parse", "HEAD"]);
        let second = prepare(&repo, &root, branch).unwrap();
        assert_ne!(first.path, second.path);
        assert_eq!(git(&first.path, &["rev-parse", "HEAD"]), moved);
        assert_eq!(
            std::fs::read_to_string(first.path.join("notes.txt")).unwrap(),
            "notes\n"
        );
        let error = launch(&second, |_, _, _| Err(Error::msg("viewer failed"))).unwrap_err();
        assert!(error.to_string().contains("viewer failed"));
        assert!(second.path.exists());
        assert_eq!(prepare(&repo, &root, branch).unwrap(), second);
    }
}
