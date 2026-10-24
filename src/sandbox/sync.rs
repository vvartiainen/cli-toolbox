//! Host-owned Git exchange: review refs only, never a checkout or merge.

use std::ffi::OsStr;
use std::path::Path;

use sha2::{Digest, Sha256};

use super::RunnerState;
use crate::{Error, aws::shell_quote, exec, git};

pub(crate) struct Review {
    pub(crate) report: String,
    /// Exact ancestry, not patch equivalence: cleanup must remain conservative.
    pub(crate) pending: Vec<(String, String)>,
}

pub(crate) struct Bridge {
    remote: String,
    source: String,
    ssh_command: Option<String>,
}

impl Bridge {
    pub(crate) fn for_vm(vm: &RunnerState, worktree: &str) -> Result<Self, Error> {
        if worktree.is_empty()
            || worktree == "."
            || worktree == ".."
            || worktree.contains(['/', '\\', '\n', '\r', '\0'])
            || !vm.name.starts_with(|ch: char| ch.is_ascii_alphanumeric())
            || !vm
                .name
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || "-_.".contains(ch))
        {
            return Err(Error::msg("invalid VM or workspace name for Git exchange"));
        }
        let mut hash = Sha256::new();
        hash.update(vm.dir.as_os_str().as_encoded_bytes());
        hash.update(b"\0");
        hash.update(worktree.as_bytes());
        let source = format!("{:x}", hash.finalize())[..12].to_string();
        let config = vm.runtime_dir.join("ssh_config");
        let config = config
            .to_str()
            .ok_or_else(|| Error::msg("SSH config path must be UTF-8"))?;
        Ok(Self {
            remote: format!("{}:/workspace/{worktree}", vm.name),
            source,
            ssh_command: Some(format!(
                "ssh -F {} -o BatchMode=yes -o ConnectTimeout=10 -o StrictHostKeyChecking=yes",
                shell_quote(config)
            )),
        })
    }

    #[cfg(test)]
    fn local(remote: std::path::PathBuf, source: &str) -> Self {
        Self {
            remote: remote.to_str().unwrap().into(),
            source: source.into(),
            ssh_command: None,
        }
    }

    fn git(&self, repo: &Path, args: &[&str]) -> Result<String, Error> {
        let mut envs = vec![("GIT_TERMINAL_PROMPT", OsStr::new("0"))];
        if let Some(ssh) = &self.ssh_command {
            envs.push(("GIT_SSH_COMMAND", OsStr::new(ssh)));
        }
        exec::captured_stdout_in("git", args, Some(repo), &envs)
    }

    fn prefix(&self) -> String {
        format!("refs/remotes/sandbox-review/{}/", self.source)
    }

    /// Fetch all named guest branches, including branches created by agents.
    /// Do not prune: a deleted guest branch may still contain unreviewed work.
    pub(crate) fn fetch(&self, repo: &Path, host_branch: &str) -> Result<Review, Error> {
        let refspec = format!("+refs/heads/*:{}*", self.prefix());
        self.git(
            repo,
            &[
                "fetch",
                "--atomic",
                "--no-tags",
                "--no-prune",
                "--no-recurse-submodules",
                "--no-write-fetch-head",
                "--no-auto-maintenance",
                "--",
                &self.remote,
                &refspec,
            ],
        )?;
        // A detached tip is not part of refs/heads/*; never silently omit it
        // and then allow done to remove the worktree.
        let head = self.git(repo, &["ls-remote", "--symref", "--", &self.remote, "HEAD"])?;
        if !head
            .lines()
            .any(|line| line.starts_with("ref: refs/heads/") && line.ends_with("\tHEAD"))
        {
            return Err(Error::msg(format!(
                "guest HEAD is detached or unborn; put its work on a named branch before syncing (named branches were fetched into {})",
                self.prefix()
            )));
        }
        self.review(repo, host_branch)
    }

    pub(crate) fn exchange(&self, repo: &Path, host_branch: &str) -> Result<String, Error> {
        // Fetch first, so a failed export never hides the guest's commits.
        let review = self.fetch(repo, host_branch)?;
        let host_ref = format!("refs/heads/{host_branch}");
        let guest_ref = format!("refs/remotes/host-review/{}/{host_branch}", self.source);
        let refspec = format!("+{host_ref}:{guest_ref}");
        self.git(
            repo,
            &[
                "push",
                "--porcelain",
                "--no-verify",
                "--no-follow-tags",
                "--recurse-submodules=no",
                "--",
                &self.remote,
                &refspec,
            ],
        )
        .map_err(|err| {
            Error::msg(format!(
                "guest commits fetched into {}; host export failed: {err}",
                self.prefix()
            ))
        })?;
        Ok(format!(
            "{}host history available in VM as {guest_ref}\n  In VM, inspect: git log --oneline --left-right --cherry-mark {}\nActive branches, indexes and working files unchanged.\n",
            review.report,
            shell_quote(&format!("HEAD...{guest_ref}"))
        ))
    }

    fn review(&self, repo: &Path, host_branch: &str) -> Result<Review, Error> {
        let host_ref = format!("refs/heads/{host_branch}");
        git::resolve_commit(repo, &host_ref)
            .ok_or_else(|| Error::msg(format!("missing host branch {host_branch}")))?;
        let prefix = self.prefix();
        let refs = self.git(repo, &["for-each-ref", "--format=%(refname)", &prefix])?;
        if refs.trim().is_empty() {
            return Err(Error::msg(
                "no named guest branches to fetch; commit on a branch, not detached HEAD",
            ));
        }
        let mut report = String::new();
        let mut pending = Vec::new();
        for guest_ref in refs.lines() {
            let branch = guest_ref
                .strip_prefix(&prefix)
                .ok_or_else(|| Error::msg("unexpected review ref"))?;
            let short_ref = guest_ref.strip_prefix("refs/remotes/").unwrap_or(guest_ref);
            if git::is_ancestor(repo, guest_ref, &host_ref) {
                report.push_str(&format!(
                    "guest {branch}: already contained on host ({short_ref})\n"
                ));
                continue;
            }
            let log = git::log_oneline(repo, &format!("{host_ref}..{guest_ref}"))?;
            let cherry = self.git(repo, &["cherry", "-v", &host_ref, guest_ref])?;
            let new = cherry.lines().filter(|line| line.starts_with("+ ")).count();
            let applied = cherry.lines().filter(|line| line.starts_with("- ")).count();
            let merges = self.git(
                repo,
                &[
                    "rev-list",
                    "--count",
                    "--merges",
                    &format!("{host_ref}..{guest_ref}"),
                ],
            )?;
            let patches = if new == 1 { "patch" } else { "patches" };
            let merge_commits = if merges.trim() == "1" {
                "merge commit"
            } else {
                "merge commits"
            };
            report.push_str(&format!(
                "guest {branch}: {new} new {patches}, {applied} already applied on host, {} {merge_commits} to review\n  Review: {short_ref}\n  Inspect: git log --oneline --left-right --cherry-mark {}\n",
                merges.trim(), shell_quote(&format!("{host_ref}...{guest_ref}"))
            ));
            for line in cherry.lines() {
                let label = if line.starts_with("+ ") {
                    "new"
                } else {
                    "already applied"
                };
                report.push_str(&format!("    {label}: {}\n", &line[2..]));
            }
            pending.push((guest_ref.into(), log));
        }
        Ok(Review { report, pending })
    }
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
    use std::path::Path;

    #[derive(Debug, Eq, PartialEq)]
    struct Snapshot {
        head: String,
        branch: String,
        index: Vec<u8>,
        dirty: Vec<u8>,
        untracked: Vec<u8>,
    }

    fn snapshot(repo: &Path) -> Snapshot {
        let index = git(repo, &["rev-parse", "--git-path", "index"]);
        let index = repo.join(index);
        Snapshot {
            head: git(repo, &["rev-parse", "HEAD"]),
            branch: git(repo, &["symbolic-ref", "HEAD"]),
            index: std::fs::read(index).unwrap(),
            dirty: std::fs::read(repo.join("dirty.txt")).unwrap(),
            untracked: std::fs::read(repo.join("untracked.txt")).unwrap(),
        }
    }

    #[test]
    fn exchanges_diverged_histories_without_touching_either_checkout() {
        if skip_without_git() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let host = temp.path().join("host");
        let guest = temp.path().join("guest with spaces");
        init_git_repo(&host);
        let branch = git(&host, &["symbolic-ref", "--short", "HEAD"]);
        git(
            temp.path(),
            &["clone", host.to_str().unwrap(), guest.to_str().unwrap()],
        );
        git(&guest, &["config", "user.email", "test@example.com"]);
        git(&guest, &["config", "user.name", "Test"]);
        std::fs::write(host.join("host.txt"), "host\n").unwrap();
        commit_all(&host);
        std::fs::write(guest.join("guest.txt"), "guest\n").unwrap();
        commit_all(&guest);
        git(&guest, &["branch", "extra/agent"]);
        // User defaults must not cause pruning or unrelated tag exports.
        git(&host, &["config", "fetch.prune", "true"]);
        git(&host, &["config", "push.followTags", "true"]);
        git(&host, &["tag", "-a", "host-only-tag", "-m", "not exported"]);
        for repo in [&host, &guest] {
            std::fs::write(repo.join("dirty.txt"), "staged\n").unwrap();
            git(repo, &["add", "dirty.txt"]);
            std::fs::write(repo.join("dirty.txt"), "unstaged\n").unwrap();
            std::fs::write(repo.join("untracked.txt"), "untracked\n").unwrap();
        }
        let before_host = snapshot(&host);
        let before_guest = snapshot(&guest);
        let bridge = Bridge::local(guest.clone(), "test-source");
        let first = bridge.exchange(&host, &branch).unwrap();
        assert!(first.contains("1 new patch"), "{first}");
        let guest_ref = format!("refs/remotes/sandbox-review/test-source/{branch}");
        let host_ref = format!("refs/remotes/host-review/test-source/{branch}");
        assert_eq!(git(&host, &["rev-parse", &guest_ref]), before_guest.head);
        assert_eq!(git(&guest, &["rev-parse", &host_ref]), before_host.head);
        assert_eq!(snapshot(&host), before_host);
        assert_eq!(snapshot(&guest), before_guest);
        assert!(
            git(
                &guest,
                &["for-each-ref", "--format=%(refname)", "refs/tags/"]
            )
            .is_empty()
        );
        assert_eq!(bridge.exchange(&host, &branch).unwrap(), first);
        git(&guest, &["branch", "-D", "extra/agent"]);
        bridge.exchange(&host, &branch).unwrap();
        // No pruning: even deleted guest branches remain available for review.
        assert!(
            !git(
                &host,
                &[
                    "rev-parse",
                    "refs/remotes/sandbox-review/test-source/extra/agent"
                ]
            )
            .is_empty()
        );
    }

    #[test]
    fn cherry_picked_patches_are_not_reported_as_new_work() {
        if skip_without_git() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let host = temp.path().join("host");
        let guest = temp.path().join("guest");
        init_git_repo(&host);
        let branch = git(&host, &["symbolic-ref", "--short", "HEAD"]);
        git(
            temp.path(),
            &["clone", host.to_str().unwrap(), guest.to_str().unwrap()],
        );
        git(&guest, &["config", "user.email", "test@example.com"]);
        git(&guest, &["config", "user.name", "Test"]);
        std::fs::write(guest.join("fix.txt"), "fix\n").unwrap();
        commit_all(&guest);
        let bridge = Bridge::local(guest, "source");
        bridge.exchange(&host, &branch).unwrap();
        std::fs::write(host.join("host.txt"), "host\n").unwrap();
        commit_all(&host);
        let review = format!("refs/remotes/sandbox-review/source/{branch}");
        git(&host, &["cherry-pick", &review]);
        let report = bridge.exchange(&host, &branch).unwrap();
        assert!(report.contains("0 new patches"), "{report}");
        assert!(report.contains("1 already applied"), "{report}");
        // done remains conservative: equivalent patches are not ancestry.
        assert!(!bridge.fetch(&host, &branch).unwrap().pending.is_empty());
    }

    fn pair() -> (
        tempfile::TempDir,
        std::path::PathBuf,
        std::path::PathBuf,
        String,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let host = temp.path().join("host");
        let guest = temp.path().join("guest");
        init_git_repo(&host);
        let branch = git(&host, &["symbolic-ref", "--short", "HEAD"]);
        git(
            temp.path(),
            &["clone", host.to_str().unwrap(), guest.to_str().unwrap()],
        );
        git(&guest, &["config", "user.email", "test@example.com"]);
        git(&guest, &["config", "user.name", "Test"]);
        (temp, host, guest, branch)
    }

    #[test]
    fn merge_commits_remain_visible_even_when_their_patches_were_picked() {
        if skip_without_git() {
            return;
        }
        let (_temp, host, guest, branch) = pair();
        git(&guest, &["checkout", "-b", "feature"]);
        std::fs::write(guest.join("feature.txt"), "feature\n").unwrap();
        commit_all(&guest);
        let feature = git(&guest, &["rev-parse", "HEAD"]);
        git(&guest, &["checkout", &branch]);
        git(
            &guest,
            &["merge", "--no-ff", "feature", "-m", "merge feature"],
        );
        let bridge = Bridge::local(guest, "source");
        bridge.fetch(&host, &branch).unwrap();
        git(&host, &["cherry-pick", &feature]);
        let report = bridge.exchange(&host, &branch).unwrap();
        assert!(report.contains("1 merge commit to review"), "{report}");
        assert!(!bridge.fetch(&host, &branch).unwrap().pending.is_empty());
    }

    #[test]
    fn extra_guest_branches_are_included_in_cleanup_guard() {
        if skip_without_git() {
            return;
        }
        let (_temp, host, guest, branch) = pair();
        git(&guest, &["checkout", "-b", "extra"]);
        std::fs::write(guest.join("extra.txt"), "extra\n").unwrap();
        commit_all(&guest);
        git(&guest, &["checkout", &branch]);
        let review = Bridge::local(guest, "source")
            .fetch(&host, &branch)
            .unwrap();
        assert_eq!(review.pending.len(), 1);
        assert!(review.pending[0].0.ends_with("/extra"));
    }

    #[test]
    #[cfg(unix)]
    fn failed_host_export_retains_the_fetched_guest_history() {
        use std::os::unix::fs::PermissionsExt;
        if skip_without_git() {
            return;
        }
        let (_temp, host, guest, branch) = pair();
        std::fs::write(guest.join("fix.txt"), "fix\n").unwrap();
        commit_all(&guest);
        let hook = guest.join(".git/hooks/pre-receive");
        std::fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        let error = Bridge::local(guest.clone(), "source")
            .exchange(&host, &branch)
            .unwrap_err();
        assert!(error.to_string().contains("host export failed"), "{error}");
        assert_eq!(
            git(
                &host,
                &[
                    "rev-parse",
                    &format!("refs/remotes/sandbox-review/source/{branch}")
                ]
            ),
            git(&guest, &["rev-parse", "HEAD"])
        );
        assert!(
            crate::git::resolve_commit(
                &guest,
                &format!("refs/remotes/host-review/source/{branch}")
            )
            .is_none()
        );
    }

    #[test]
    fn detached_guest_work_cannot_silently_pass_the_cleanup_guard() {
        if skip_without_git() {
            return;
        }
        let (_temp, host, guest, branch) = pair();
        git(&guest, &["checkout", "--detach"]);
        std::fs::write(guest.join("detached.txt"), "work\n").unwrap();
        commit_all(&guest);
        let result = Bridge::local(guest, "source").fetch(&host, &branch);
        assert!(
            result
                .err()
                .unwrap()
                .to_string()
                .contains("HEAD is detached")
        );
    }

    #[test]
    fn transport_failure_is_not_treated_as_divergence() {
        if skip_without_git() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        init_git_repo(temp.path());
        let branch = git(temp.path(), &["symbolic-ref", "--short", "HEAD"]);
        assert!(
            Bridge::local(temp.path().join("missing"), "source")
                .exchange(temp.path(), &branch)
                .is_err()
        );
    }

    #[test]
    fn ssh_uses_the_runner_config_without_prompting() {
        let vm = super::super::RunnerState {
            name: "agent-demo".into(),
            dir: "/projects/demo".into(),
            runtime_dir: "/state/VM's runtime".into(),
            ..Default::default()
        };
        let bridge = Bridge::for_vm(&vm, "demo-agent-wt").unwrap();
        assert_ne!(
            bridge.source,
            Bridge::for_vm(&vm, "demo-agent-wt-2").unwrap().source
        );
        assert_eq!(bridge.remote, "agent-demo:/workspace/demo-agent-wt");
        let ssh = bridge.ssh_command.unwrap();
        assert!(ssh.contains("BatchMode=yes"));
        assert!(ssh.contains("StrictHostKeyChecking=yes"));
        assert!(ssh.contains(&crate::aws::shell_quote("/state/VM's runtime/ssh_config")));
        assert!(Bridge::for_vm(&vm, "../escape").is_err());
    }
}
