//! Agent-owned worktrees, published as create-only host review branches.
//! Sync never checks out, merges, or moves an existing branch.

use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::Path;

use sha2::{Digest, Sha256};

use super::RunnerState;
use crate::{Error, aws::shell_quote, exec, git};

pub(crate) struct Review {
    pub(crate) report: String,
    pub(crate) snapshots: Vec<String>,
}

pub(crate) struct Bridge {
    remote: String,
    source: String,
    ssh_command: Option<String>,
    ssh_args: Vec<String>,
    workspace: String,
}

/// Stable per-project/workspace namespace shared with saved-snapshot discovery.
pub(crate) fn source_id(dir: &Path, worktree: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(dir.as_os_str().as_encoded_bytes());
    hash.update(b"\0");
    hash.update(worktree.as_bytes());
    format!("{:x}", hash.finalize())[..12].to_string()
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
        let source = source_id(&vm.dir, worktree);
        let config = vm.runtime_dir.join("ssh_config");
        let config = config
            .to_str()
            .ok_or_else(|| Error::msg("SSH config path must be UTF-8"))?;
        Ok(Self {
            remote: format!("{}:/workspace/{worktree}", vm.name),
            source,
            workspace: worktree.into(),
            ssh_args: [
                "-F",
                config,
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=10",
                "-o",
                "StrictHostKeyChecking=yes",
                "--",
                &vm.name,
            ]
            .map(String::from)
            .to_vec(),
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
            ssh_args: Vec::new(),
            workspace: "demo-agent-wt".into(),
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
        let primary = head.lines().find_map(|line| {
            line.strip_prefix("ref: refs/heads/")?
                .strip_suffix("\tHEAD")
        });
        let primary = primary.ok_or_else(|| Error::msg(format!(
            "guest HEAD is detached or unborn; put its work on a named branch before syncing (named branches were fetched into {})",
            self.prefix()
        )))?;
        self.review(repo, host_branch, &format!("{}{primary}", self.prefix()))
    }

    pub(crate) fn exchange(&self, repo: &Path, host_branch: &str) -> Result<String, Error> {
        // Fetch first, so a failed export never hides the guest's commits.
        let review = self.fetch(repo, host_branch)?;
        // The host's agent branch can stay stale forever. Export ordinary
        // host branches (including main) so accepted changes still reach the
        // agent's review refs. Never re-export our snapshot branches.
        let heads = self.git(
            repo,
            &["for-each-ref", "--format=%(refname)", "refs/heads/"],
        )?;
        let refspecs: Vec<_> = heads
            .lines()
            .filter(|reference| !reference.starts_with("refs/heads/agent-review/"))
            .map(|reference| {
                format!(
                    "+{reference}:refs/remotes/host-review/{}/{}",
                    self.source,
                    reference.strip_prefix("refs/heads/").unwrap()
                )
            })
            .collect();
        let mut args = vec![
            "push",
            "--atomic",
            "--porcelain",
            "--no-verify",
            "--no-follow-tags",
            "--recurse-submodules=no",
            "--",
            &self.remote,
        ];
        args.extend(refspecs.iter().map(String::as_str));
        self.git(repo, &args).map_err(|err| {
            Error::msg(format!(
                "Review branches preserved: {}\nhost export failed: {err}",
                review.snapshots.join(", ")
            ))
        })?;
        Ok(format!(
            "{}Host branches available in VM under host-review/{}/.\nAgent checkout and existing host branches unchanged.\n",
            review.report, self.source
        ))
    }

    fn review(&self, repo: &Path, host_branch: &str, primary: &str) -> Result<Review, Error> {
        let host_ref = format!("refs/heads/{host_branch}");
        git::resolve_commit(repo, &host_ref)
            .ok_or_else(|| Error::msg(format!("missing host branch {host_branch}")))?;
        let primary_tip = git::resolve_commit(repo, primary)
            .ok_or_else(|| Error::msg("guest branch moved during sync; retry sync"))?;
        let prefix = self.prefix();
        let refs = self.git(repo, &["for-each-ref", "--format=%(refname)", &prefix])?;
        let mut refs: Vec<_> = refs.lines().collect();
        refs.sort_by_key(|reference| *reference != primary);
        let mut report = String::new();
        let mut snapshots = Vec::new();
        let mut tips = std::collections::HashSet::new();
        let mut published = Vec::new();
        for guest_ref in refs {
            let tip = git::resolve_commit(repo, guest_ref)
                .ok_or_else(|| Error::msg(format!("missing guest tip {guest_ref}")))?;
            // The primary snapshot already preserves ancestral branches. Keep
            // independent extra branches too, without publishing duplicate tips.
            if (guest_ref != primary && git::is_ancestor(repo, &tip, &primary_tip))
                || !tips.insert(tip.clone())
            {
                continue;
            }
            let branch = self.snapshot_tip(repo, &tip)?;
            report.push_str(&format!(
                "Review branch: {branch}\n  View: git show {}\n  Compare: git diff {}\n  Browse files separately: git worktree add --detach {} {}\n",
                shell_quote(&branch),
                shell_quote(&format!("{host_ref}...{branch}")),
                shell_quote(&format!("../{}-review-{}", self.workspace, &tip[..12])),
                shell_quote(&branch)
            ));
            snapshots.push(branch);
            published.push(guest_ref.to_string());
        }
        for guest_ref in published {
            let branch = guest_ref
                .strip_prefix(&prefix)
                .ok_or_else(|| Error::msg("unexpected review ref"))?;
            if git::is_ancestor(repo, &guest_ref, &host_ref) {
                report.push_str(&format!("guest {branch}: already contained on host\n"));
                continue;
            }
            let cherry = self.git(repo, &["cherry", "-v", &host_ref, &guest_ref])?;
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
                "guest {branch}: {new} new {patches}, {applied} already applied on host, {} {merge_commits} to review\n",
                merges.trim()
            ));
            for line in cherry.lines() {
                if let Some(commit) = line.strip_prefix("+ ") {
                    report.push_str(&format!("    new: {commit}\n"));
                }
            }
        }
        Ok(Review { report, snapshots })
    }

    /// Create-only ordinary branches: never advance even a previously published
    /// snapshot, since the user may have it checked out for review.
    fn snapshot_tip(&self, repo: &Path, tip: &str) -> Result<String, Error> {
        let prefix = format!("refs/heads/agent-review/{}/", self.workspace);
        let suffix = format!("-{}-{}", self.source, &tip[..12]);
        let existing = self.git(
            repo,
            &[
                "for-each-ref",
                "--format=%(objectname) %(refname) %(symref)",
                &prefix,
            ],
        )?;
        for line in existing.lines() {
            let fields: Vec<_> = line.split_whitespace().collect();
            if fields.len() == 2
                && fields[0] == tip
                && fields[1]
                    .rsplit('/')
                    .next()
                    .is_some_and(|name| name.contains(&suffix))
            {
                return Ok(fields[1].strip_prefix("refs/heads/").unwrap().into());
            }
        }
        // Use the guest commit's date: no new dependency or wall-clock naming
        // race, and rewritten/same-second commits still have distinct tip IDs.
        let stamp = self.git(
            repo,
            &[
                "show",
                "-s",
                "--format=%cd",
                "--date=format:%Y%m%d-%H%M%S",
                tip,
            ],
        )?;
        let stem = format!("{prefix}{}{suffix}", stamp.trim());
        self.git(repo, &["check-ref-format", &stem])?;
        for attempt in 0..100 {
            let reference = if attempt == 0 {
                stem.clone()
            } else {
                format!("{stem}-{attempt}")
            };
            // Do not follow existing symbolic refs, including dangling ones.
            if self.git(repo, &["symbolic-ref", "-q", &reference]).is_ok() {
                continue;
            }
            if let Err(err) = self.git(repo, &["update-ref", "--no-deref", &reference, tip, ""]) {
                // An existing or concurrently-created branch is never replaced.
                if self
                    .git(repo, &["show-ref", "--verify", &reference])
                    .is_ok()
                {
                    if git::resolve_commit(repo, &reference).as_deref() == Some(tip)
                        && self.git(repo, &["symbolic-ref", "-q", &reference]).is_err()
                    {
                        return Ok(reference.strip_prefix("refs/heads/").unwrap().into());
                    }
                    continue;
                }
                return Err(err);
            }
            return Ok(reference.strip_prefix("refs/heads/").unwrap().into());
        }
        Err(Error::msg("could not allocate a new agent review branch"))
    }

    /// The names of the guest's branches.
    pub(crate) fn branches(&self, repo: &Path) -> Result<HashSet<String>, Error> {
        let heads = self.git(repo, &["ls-remote", "--heads", "--", &self.remote])?;
        Ok(heads
            .lines()
            .filter_map(|line| line.split_once('\t'))
            .filter_map(|(_, reference)| reference.strip_prefix("refs/heads/"))
            .map(String::from)
            .collect())
    }

    /// The guest owns its index. A stale host index must not prevent cleanup
    /// once every committed guest tip has been preserved as a host branch.
    pub(crate) fn prepare_done(&self, repo: &Path, host_branch: &str) -> Result<Review, Error> {
        let review = self.fetch(repo, host_branch)?;
        let status = self.guest_status()?;
        if !status.trim().is_empty() {
            return Err(Error::msg(format!(
                "uncommitted guest changes; commit them before done:\n{status}\nReview branches preserved: {}",
                review.snapshots.join(", ")
            )));
        }
        self.verify_guest_tips(repo, &review.snapshots)?;
        Ok(review)
    }

    fn verify_guest_tips(&self, repo: &Path, snapshots: &[String]) -> Result<(), Error> {
        let remote = self.git(
            repo,
            &[
                "ls-remote",
                "--symref",
                "--",
                &self.remote,
                "HEAD",
                "refs/heads/*",
            ],
        )?;
        let attached = remote
            .lines()
            .any(|line| line.starts_with("ref: refs/heads/") && line.ends_with("\tHEAD"));
        let preserved = remote
            .lines()
            .filter(|line| !line.starts_with("ref: "))
            .all(|line| {
                line.split_once('\t').is_some_and(|(tip, _)| {
                    git::resolve_commit(repo, tip).is_some()
                        && snapshots
                            .iter()
                            .any(|branch| git::is_ancestor(repo, tip, branch))
                })
            });
        if !attached || !preserved {
            return Err(Error::msg(format!(
                "guest changed while saving review snapshots; finish/pause the agent and retry done\nReview branches preserved: {}",
                snapshots.join(", ")
            )));
        }
        Ok(())
    }

    fn guest_status(&self) -> Result<String, Error> {
        if self.ssh_args.is_empty() {
            return self.git(
                Path::new(&self.remote),
                &[
                    "--no-optional-locks",
                    "status",
                    "--porcelain",
                    "--untracked-files=all",
                ],
            );
        }
        let command = format!(
            "git --no-optional-locks -C {} status --porcelain --untracked-files=all",
            shell_quote(&format!("/workspace/{}", self.workspace))
        );
        let mut args: Vec<_> = self.ssh_args.iter().map(String::as_str).collect();
        args.push(&command);
        exec::captured_stdout("ssh", &args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::tests::{commit_all, init_git_repo, skip_without_git};

    fn git(repo: &Path, args: &[&str]) -> String {
        // Build sandboxes have no global git identity for tags and cherry-picks.
        let envs = [
            ("GIT_AUTHOR_NAME", OsStr::new("cli-toolbox")),
            ("GIT_AUTHOR_EMAIL", OsStr::new("cli-toolbox@example.com")),
            ("GIT_COMMITTER_NAME", OsStr::new("cli-toolbox")),
            ("GIT_COMMITTER_EMAIL", OsStr::new("cli-toolbox@example.com")),
        ];
        exec::captured_stdout_in("git", args, Some(repo), &envs)
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
        assert_eq!(
            bridge.branches(&host).unwrap(),
            HashSet::from([branch.clone(), "extra/agent".to_string()])
        );
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
        // Snapshots preserve history without requiring cherry-pick ancestry.
        assert_eq!(
            bridge.prepare_done(&host, &branch).unwrap().snapshots.len(),
            1
        );
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

    fn review_branches(repo: &Path) -> Vec<String> {
        git(
            repo,
            &[
                "for-each-ref",
                "--format=%(refname:short)",
                "refs/heads/agent-review/",
            ],
        )
        .lines()
        .map(str::to_string)
        .collect()
    }

    #[test]
    fn changed_tips_get_new_branches_without_moving_checked_out_reviews() {
        if skip_without_git() {
            return;
        }
        let (temp, host, guest, branch) = pair();
        std::fs::write(guest.join("fix.txt"), "first\n").unwrap();
        commit_all(&guest);
        let base = git(&guest, &["rev-parse", "HEAD~1"]);
        let bridge = Bridge::local(guest.clone(), "source");
        let report = bridge.exchange(&host, &branch).unwrap();
        let first = review_branches(&host);
        assert_eq!(first.len(), 1);
        assert!(report.starts_with("Review branch: "), "{report}");
        assert!(report.contains("git show"), "{report}");
        assert!(report.contains("git worktree add --detach"), "{report}");
        let old_tip = git(&host, &["rev-parse", &first[0]]);
        let copy = temp.path().join("review-copy");
        git(
            &host,
            &["worktree", "add", copy.to_str().unwrap(), &first[0]],
        );
        bridge.exchange(&host, &branch).unwrap();
        assert_eq!(review_branches(&host), first);

        std::fs::write(guest.join("fix.txt"), "second\n").unwrap();
        commit_all(&guest);
        bridge.exchange(&host, &branch).unwrap();
        assert_eq!(review_branches(&host).len(), 2);
        assert_eq!(git(&copy, &["rev-parse", "HEAD"]), old_tip);
        assert_eq!(
            std::fs::read_to_string(copy.join("fix.txt")).unwrap(),
            "first\n"
        );
        assert_eq!(git(&host, &["rev-parse", &first[0]]), old_tip);
        // Rewritten agent history must not erase either earlier review point.
        git(&guest, &["reset", "--hard", &base]);
        std::fs::write(guest.join("fix.txt"), "rewritten\n").unwrap();
        commit_all(&guest);
        bridge.exchange(&host, &branch).unwrap();
        assert_eq!(review_branches(&host).len(), 3);
        assert_eq!(git(&copy, &["rev-parse", "HEAD"]), old_tip);
        // A host-only commit is exported but does not create another guest snapshot.
        std::fs::write(host.join("host-only.txt"), "host\n").unwrap();
        commit_all(&host);
        bridge.exchange(&host, &branch).unwrap();
        assert_eq!(review_branches(&host).len(), 3);
    }

    #[test]
    fn host_integration_branches_are_exported_but_snapshots_are_not() {
        if skip_without_git() {
            return;
        }
        let (_temp, host, guest, branch) = pair();
        let guest_tip = git(&guest, &["rev-parse", "HEAD"]);
        git(&host, &["checkout", "-b", "accepted"]);
        std::fs::write(host.join("accepted.txt"), "accepted\n").unwrap();
        commit_all(&host);
        let accepted = git(&host, &["rev-parse", "HEAD"]);
        git(&host, &["checkout", &branch]);
        Bridge::local(guest.clone(), "source")
            .exchange(&host, &branch)
            .unwrap();
        assert_eq!(
            git(
                &guest,
                &["rev-parse", "refs/remotes/host-review/source/accepted"]
            ),
            accepted
        );
        assert_eq!(git(&guest, &["rev-parse", "HEAD"]), guest_tip);
        assert!(
            git(
                &guest,
                &[
                    "for-each-ref",
                    "--format=%(refname)",
                    "refs/remotes/host-review/source/agent-review/"
                ]
            )
            .is_empty()
        );
    }

    #[test]
    fn existing_snapshot_branches_are_never_overwritten() {
        if skip_without_git() {
            return;
        }
        let (_temp, host, guest, branch) = pair();
        let base = git(&host, &["rev-parse", "HEAD"]);
        std::fs::write(guest.join("fix.txt"), "fix\n").unwrap();
        commit_all(&guest);
        let bridge = Bridge::local(guest.clone(), "source");
        bridge.exchange(&host, &branch).unwrap();
        let original = review_branches(&host)[0].clone();
        // Even if a user repurposes a snapshot branch, sync must leave it alone.
        git(
            &host,
            &["update-ref", &format!("refs/heads/{original}"), &base],
        );
        bridge.exchange(&host, &branch).unwrap();
        assert_eq!(git(&host, &["rev-parse", &original]), base);
        assert_eq!(review_branches(&host).len(), 2);
        assert_eq!(bridge.fetch(&host, &branch).unwrap().snapshots.len(), 1);
        assert_eq!(review_branches(&host).len(), 2);
        let mut other = Bridge::local(guest, "other-source");
        other.workspace = bridge.workspace.clone();
        other.exchange(&host, &branch).unwrap();
        assert_eq!(review_branches(&host).len(), 3);
    }

    #[test]
    fn cleanup_rechecks_guest_tips_after_saving_snapshots() {
        if skip_without_git() {
            return;
        }
        let (_temp, host, guest, branch) = pair();
        let bridge = Bridge::local(guest.clone(), "source");
        let saved = bridge.fetch(&host, &branch).unwrap();
        bridge.verify_guest_tips(&host, &saved.snapshots).unwrap();
        std::fs::write(guest.join("later.txt"), "later\n").unwrap();
        commit_all(&guest);
        let error = bridge
            .verify_guest_tips(&host, &saved.snapshots)
            .unwrap_err();
        assert!(error.to_string().contains("guest changed"), "{error}");
        bridge.prepare_done(&host, &branch).unwrap();
        assert_eq!(review_branches(&host).len(), 2);
    }

    #[test]
    fn snapshot_creation_failure_blocks_done_without_changing_checkouts() {
        if skip_without_git() {
            return;
        }
        let (_temp, host, guest, branch) = pair();
        let host_tip = git(&host, &["rev-parse", "HEAD"]);
        git(&host, &["branch", "agent-review/demo-agent-wt"]);
        std::fs::write(guest.join("work.txt"), "work\n").unwrap();
        commit_all(&guest);
        let guest_tip = git(&guest, &["rev-parse", "HEAD"]);
        let result = Bridge::local(guest.clone(), "source").prepare_done(&host, &branch);
        assert!(result.is_err());
        assert_eq!(git(&host, &["rev-parse", "HEAD"]), host_tip);
        assert_eq!(git(&guest, &["rev-parse", "HEAD"]), guest_tip);
        assert!(guest.join("work.txt").exists());
    }

    #[test]
    fn review_branches_survive_removing_the_agent_worktree() {
        if skip_without_git() {
            return;
        }
        let (temp, host, guest, branch) = pair();
        let agent = temp.path().join("agent-worktree");
        git(
            &host,
            &[
                "worktree",
                "add",
                "-b",
                "agent-test",
                agent.to_str().unwrap(),
                &branch,
            ],
        );
        std::fs::write(guest.join("agent.txt"), "committed\n").unwrap();
        commit_all(&guest);
        std::fs::copy(guest.join("agent.txt"), agent.join("agent.txt")).unwrap();
        let review = Bridge::local(guest.clone(), "source")
            .prepare_done(&agent, "agent-test")
            .unwrap();
        git(
            &host,
            &["worktree", "remove", "--force", agent.to_str().unwrap()],
        );
        assert!(!agent.exists());
        assert_eq!(
            git(&host, &["rev-parse", &review.snapshots[0]]),
            git(&guest, &["rev-parse", "HEAD"])
        );
        assert_eq!(
            git(
                &host,
                &["show", &format!("{}:agent.txt", review.snapshots[0])]
            ),
            "committed"
        );
    }

    #[test]
    fn done_uses_the_guest_index_not_the_stale_host_counterpart() {
        if skip_without_git() {
            return;
        }
        let (_temp, host, guest, branch) = pair();
        std::fs::write(guest.join("agent.txt"), "committed\n").unwrap();
        commit_all(&guest);
        // Simulate shared files visible on the host, but absent from its index.
        std::fs::copy(guest.join("agent.txt"), host.join("agent.txt")).unwrap();
        let host_tip = git(&host, &["rev-parse", "HEAD"]);
        let bridge = Bridge::local(guest.clone(), "source");
        let review = bridge.prepare_done(&host, &branch).unwrap();
        assert_eq!(review.snapshots.len(), 1);
        assert_eq!(git(&host, &["rev-parse", "HEAD"]), host_tip);
        assert!(git(&host, &["status", "--porcelain"]).contains("agent.txt"));
        std::fs::write(guest.join("agent.txt"), "uncommitted\n").unwrap();
        assert!(
            bridge
                .prepare_done(&host, &branch)
                .err()
                .unwrap()
                .to_string()
                .contains("uncommitted guest")
        );
        git(&guest, &["add", "agent.txt"]);
        assert!(bridge.prepare_done(&host, &branch).is_err());
        git(&guest, &["reset", "--hard", "HEAD"]);
        std::fs::write(guest.join("untracked.txt"), "work\n").unwrap();
        assert!(bridge.prepare_done(&host, &branch).is_err());
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
        assert_eq!(
            bridge.prepare_done(&host, &branch).unwrap().snapshots.len(),
            1
        );
    }

    #[test]
    fn extra_guest_branches_are_preserved_for_cleanup() {
        if skip_without_git() {
            return;
        }
        let (_temp, host, guest, branch) = pair();
        git(&guest, &["checkout", "-b", "extra"]);
        std::fs::write(guest.join("extra.txt"), "extra\n").unwrap();
        commit_all(&guest);
        git(&guest, &["checkout", &branch]);
        let review = Bridge::local(guest.clone(), "source")
            .fetch(&host, &branch)
            .unwrap();
        assert_eq!(review.snapshots.len(), 2);
        let tip = git(&guest, &["rev-parse", "extra"]);
        assert!(
            review
                .snapshots
                .iter()
                .any(|branch| git(&host, &["rev-parse", branch]) == tip)
        );
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
