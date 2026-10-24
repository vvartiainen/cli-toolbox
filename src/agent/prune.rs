//! Cleanup of a project's review snapshots once their work is on the
//! integration branch, with the review checkouts made from them and the
//! fetch refs of removed worktrees. Sync, review and done only ever add them.
//!
//! `git cherry` decides what is accepted: it compares changes, not commit
//! IDs, so cherry-picked and merged work both count. Anything else is kept
//! and reported unless forced.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};

use super::Step;
use super::review::{self, Snapshot};
use crate::sandbox::{self, project::Project, sync};
use crate::{Error, exec, git};

/// What [`prune`] deletes besides accepted snapshots.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct PruneOptions {
    /// Only this worktree's snapshots and fetch refs.
    pub(crate) worktree: Option<String>,
    /// The integration branch, instead of [`git::integration_branch`].
    pub(crate) into: Option<String>,
    /// Also delete superseded snapshots: older ones of a removed worktree
    /// whose newest snapshot is accepted.
    pub(crate) superseded: bool,
    /// Delete every snapshot, also unaccepted ones, and every fetch ref of
    /// removed worktrees. Implies `superseded`.
    pub(crate) all: bool,
    /// Remove review checkouts with local edits too.
    pub(crate) force: bool,
    /// Only report what would be deleted and what kept.
    pub(crate) dry_run: bool,
}

/// What [`prune`] did, or would do.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Report {
    /// The integration branch.
    pub(crate) into: String,
    pub(crate) steps: Vec<Step>,
    /// Deleted, or to delete.
    pub(crate) deleted: usize,
    /// Kept superseded snapshots; `superseded` deletes them.
    pub(crate) superseded: usize,
    /// Kept unaccepted snapshots and fetch refs; `all` deletes them.
    pub(crate) unaccepted: usize,
    /// Kept review checkouts with local edits; `force` removes them.
    pub(crate) dirty: usize,
}

impl Report {
    fn done(&mut self, text: String) {
        self.deleted += 1;
        self.steps.push(Step::Done(text));
    }

    fn kept(&mut self, text: String) {
        self.steps.push(Step::Skipped(text));
    }

    fn failed(&mut self, text: String) {
        self.steps.push(Step::Failed(text));
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.steps.is_empty() {
            return writeln!(f, "Nothing to prune; integration branch {}.", self.into);
        }
        writeln!(f, "Integration branch: {}", self.into)?;
        for step in &self.steps {
            writeln!(f, "{step}")?;
        }
        Ok(())
    }
}

/// Why a snapshot is deleted or kept.
enum Fate {
    Accepted,
    /// Older than the accepted newest snapshot of its removed worktree.
    Superseded {
        newest: String,
    },
    Unaccepted {
        commits: Vec<String>,
    },
}

/// Delete the project's accepted review snapshots, remove their review
/// checkouts, and delete the stale fetch refs, of removed worktrees and of
/// guest branches that are gone, whose work is accepted or kept in a
/// snapshot. Unaccepted work stays unless `options`
/// say otherwise; the report says what stays and why.
pub(crate) fn prune(project: &Project, options: &PruneOptions) -> Result<Report, Error> {
    let vm = sandbox::vm_for(&super::vm::running_vms(), &project.dir);
    prune_with(project, options, |worktree| {
        let vm = vm.as_ref().filter(|_| project.mounts(worktree))?;
        Some(sync::Bridge::for_vm(vm, worktree).and_then(|bridge| bridge.branches(project.repo()?)))
    })
}

/// [`prune`], with the guest's branches of a current worktree from
/// `guest_branches`: `None` when its VM is not running or does not mount it.
fn prune_with(
    project: &Project,
    options: &PruneOptions,
    guest_branches: impl Fn(&str) -> Option<Result<HashSet<String>, Error>>,
) -> Result<Report, Error> {
    let repo = project.repo()?;
    let into = match &options.into {
        Some(into) => into.clone(),
        None => git::integration_branch(repo)?,
    };
    if git::resolve_commit(repo, &format!("refs/heads/{into}")).is_none() {
        return Err(Error::msg(format!(
            "integration branch {into} does not exist"
        )));
    }
    let mut report = Report {
        into: into.clone(),
        ..Report::default()
    };
    let verb = |past: &'static str, future: &'static str| {
        if options.dry_run { future } else { past }
    };

    let all = review::snapshots(project, None)?;
    let current: HashSet<String> = project
        .worktrees()?
        .into_iter()
        .map(|worktree| worktree.name)
        .collect();
    let selected: Vec<&Snapshot> = all
        .iter()
        .filter(|snapshot| {
            options
                .worktree
                .as_deref()
                .is_none_or(|name| snapshot.worktree == name)
        })
        .collect();

    // Classify every snapshot first: superseded ones depend on the newest.
    let mut unapplied = HashMap::new();
    for snapshot in &selected {
        unapplied.insert(
            snapshot.branch.clone(),
            git::unapplied_commits(repo, &into, &snapshot.branch)?,
        );
    }
    let mut newest: HashMap<&str, &Snapshot> = HashMap::new();
    for snapshot in &selected {
        newest.entry(&snapshot.worktree).or_insert(snapshot);
    }
    let fates: Vec<Fate> = selected
        .iter()
        .map(|snapshot| {
            let commits = &unapplied[&snapshot.branch];
            let latest = newest[snapshot.worktree.as_str()];
            if commits.is_empty() {
                Fate::Accepted
            } else if !current.contains(&snapshot.worktree)
                && latest.branch != snapshot.branch
                && unapplied[&latest.branch].is_empty()
            {
                Fate::Superseded {
                    newest: latest.branch.clone(),
                }
            } else {
                Fate::Unaccepted {
                    commits: commits.clone(),
                }
            }
        })
        .collect();
    let delete_superseded = options.superseded || options.all;
    // The newest snapshot is what marks the older ones superseded, so it
    // stays while they do.
    let marking: HashSet<&str> = fates
        .iter()
        .filter_map(|fate| match fate {
            Fate::Superseded { newest } if !delete_superseded => Some(newest.as_str()),
            _ => None,
        })
        .collect();
    let mut deleted = HashSet::new();
    let mut kept_tips = Vec::new();
    for (snapshot, fate) in selected.iter().zip(&fates) {
        let branch = &snapshot.branch;
        if marking.contains(branch.as_str()) {
            kept_tips.push(snapshot.tip.clone());
            report.kept(format!(
                "snapshot: kept {branch}: on {into}, but marks older snapshots superseded"
            ));
            continue;
        }
        let delete = match fate {
            Fate::Accepted => true,
            Fate::Superseded { .. } => delete_superseded,
            Fate::Unaccepted { .. } => options.all,
        };
        let why = match &fate {
            Fate::Accepted => format!("on {into}"),
            Fate::Superseded { newest } => format!("superseded by accepted {newest}"),
            Fate::Unaccepted { commits } => not_on(&into, commits),
        };
        if !delete {
            kept_tips.push(snapshot.tip.clone());
            match fate {
                Fate::Superseded { .. } => {
                    report.superseded += 1;
                    report.kept(format!(
                        "snapshot: kept {branch}: {why}; --superseded deletes it"
                    ));
                }
                _ => {
                    report.unaccepted += 1;
                    report.kept(format!("snapshot: kept {branch}: {why}"));
                }
            }
            continue;
        }
        let forced = if matches!(fate, Fate::Accepted) {
            ""
        } else {
            "forced, "
        };
        let text = format!(
            "snapshot: {} {branch} ({forced}{why})",
            verb("deleted", "would delete")
        );
        if options.dry_run {
            deleted.insert(branch.clone());
            report.done(text);
            continue;
        }
        match exec::captured_stdout_in("git", &["branch", "-D", "--", branch], Some(repo), &[]) {
            Ok(_) => {
                deleted.insert(branch.clone());
                report.done(text);
            }
            Err(err) => {
                kept_tips.push(snapshot.tip.clone());
                report.failed(format!("snapshot: delete {branch}: {err}"));
            }
        }
    }

    prune_checkouts(project, repo, options, &all, &deleted, &mut report)?;

    // Fetch refs of removed worktrees, and of guest branches that are gone,
    // which sync never prunes. A running VM lists its branches; a stopped one
    // cannot, so its current worktrees' refs stay.
    let dir = review::snapshot_dir(project);
    let mut removed: HashSet<&str> = selected
        .iter()
        .map(|snapshot| snapshot.worktree.as_str())
        .chain(options.worktree.as_deref())
        .collect();
    removed.retain(|name| !current.contains(*name));
    let removed: HashSet<String> = removed
        .iter()
        .map(|name| sync::source_id(&dir, name))
        .collect();
    let mut live: HashMap<String, HashSet<String>> = HashMap::new();
    let mut names: Vec<&String> = current
        .iter()
        .filter(|name| options.worktree.as_ref().is_none_or(|only| only == *name))
        .collect();
    names.sort();
    for name in names {
        match guest_branches(name) {
            Some(Ok(branches)) => {
                live.insert(sync::source_id(&dir, name), branches);
            }
            Some(Err(err)) => report.kept(format!(
                "fetch refs of {name}: kept; could not list the guest's branches: {err}"
            )),
            None => {}
        }
    }
    let refs = exec::captured_stdout_in(
        "git",
        &[
            "for-each-ref",
            "--format=%(refname) %(objectname)",
            "refs/remotes/sandbox-review/",
        ],
        Some(repo),
        &[],
    )?;
    for line in refs.lines() {
        let Some((reference, tip)) = line.split_once(' ') else {
            continue;
        };
        let Some((source, branch)) = reference
            .strip_prefix("refs/remotes/sandbox-review/")
            .and_then(|rest| rest.split_once('/'))
        else {
            continue;
        };
        let stale = removed.contains(source)
            || live
                .get(source)
                .is_some_and(|branches| !branches.contains(branch));
        if !stale {
            continue;
        }
        let commits = git::unapplied_commits(repo, &into, tip)?;
        let kept_in = kept_tips
            .iter()
            .any(|kept| git::is_ancestor(repo, tip, kept));
        let why = if commits.is_empty() {
            format!("on {into}")
        } else if kept_in {
            "in a kept snapshot".into()
        } else if options.all {
            format!("forced, {}", not_on(&into, &commits))
        } else {
            report.unaccepted += 1;
            report.kept(format!(
                "fetch ref: kept stale {reference}: {}, and in no kept snapshot",
                not_on(&into, &commits)
            ));
            continue;
        };
        let text = format!(
            "fetch ref: {} stale {reference} ({why})",
            verb("deleted", "would delete")
        );
        if options.dry_run {
            report.done(text);
            continue;
        }
        match exec::captured_stdout_in(
            "git",
            &["update-ref", "-d", reference, tip],
            Some(repo),
            &[],
        ) {
            Ok(_) => report.done(text),
            Err(err) => report.failed(format!("fetch ref: delete {reference}: {err}")),
        }
    }
    Ok(report)
}

/// `N commits not on <into>, first "<subject>"`.
fn not_on(into: &str, commits: &[String]) -> String {
    let plural = if commits.len() == 1 { "" } else { "s" };
    let first = commits
        .first()
        .map(|subject| format!(", first \"{subject}\""))
        .unwrap_or_default();
    format!("{} commit{plural} not on {into}{first}", commits.len())
}

/// Remove the review checkouts made from deleted snapshots, and those whose
/// snapshot is gone, unless they have local edits.
fn prune_checkouts(
    project: &Project,
    repo: &Path,
    options: &PruneOptions,
    snapshots: &[Snapshot],
    deleted: &HashSet<String>,
    report: &mut Report,
) -> Result<(), Error> {
    let root = review::checkouts_dir(project);
    let Ok(root) = root.canonicalize() else {
        return Ok(());
    };
    let by_key: HashMap<String, &Snapshot> = snapshots
        .iter()
        .map(|snapshot| {
            (
                review::checkout_key(&snapshot.branch, &snapshot.tip),
                snapshot,
            )
        })
        .collect();
    let listed =
        exec::captured_stdout_in("git", &["worktree", "list", "--porcelain"], Some(repo), &[])?;
    for (path, head) in checkouts(&listed) {
        if path.parent() != Some(root.as_path()) {
            continue;
        }
        let folder = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let snapshot = by_key.get(folder.get(..24).unwrap_or_default());
        let tip = match snapshot {
            Some(snapshot) if deleted.contains(&snapshot.branch) => Some(&snapshot.tip),
            // Its snapshot stays.
            Some(_) => continue,
            // Its snapshot is gone; another worktree's when one is chosen.
            None if options.worktree.is_some() => continue,
            None => None,
        };
        let text = path.display().to_string();
        if !path.exists() {
            report.kept(format!(
                "review checkout: {text} is missing; `git worktree prune` forgets it"
            ));
            continue;
        }
        let status = exec::captured_stdout_in("git", &["status", "--porcelain"], Some(&path), &[])?;
        let moved = tip.is_some_and(|tip| head.as_deref() != Some(tip.as_str()));
        if (!status.trim().is_empty() || moved) && !options.force {
            report.dirty += 1;
            report.kept(format!(
                "review checkout: kept {text}: local edits; --force removes it"
            ));
            continue;
        }
        let verb = if options.dry_run {
            "would remove"
        } else {
            "removed"
        };
        if options.dry_run {
            report.done(format!("review checkout: {verb} {text}"));
            continue;
        }
        let path_text = path.to_string_lossy();
        let mut args = vec!["worktree", "remove"];
        if options.force {
            args.push("--force");
        }
        args.extend(["--", &path_text]);
        match exec::captured_stdout_in("git", &args, Some(repo), &[]) {
            Ok(_) => report.done(format!("review checkout: {verb} {text}")),
            Err(err) => report.failed(format!("review checkout: remove {text}: {err}")),
        }
    }
    Ok(())
}

/// The path and HEAD of each worktree in `git worktree list --porcelain`.
fn checkouts(listed: &str) -> Vec<(PathBuf, Option<String>)> {
    listed
        .split("\n\n")
        .filter_map(|entry| {
            let mut path = None;
            let mut head = None;
            for line in entry.lines() {
                if let Some(value) = line.strip_prefix("worktree ") {
                    path = Some(PathBuf::from(value));
                } else if let Some(value) = line.strip_prefix("HEAD ") {
                    head = Some(value.to_string());
                }
            }
            Some((path?, head))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::tests::{init_git_repo, must_git_output, must_run_git, skip_without_git};
    use std::fs;
    use std::process::Command;

    struct Fixture {
        _temp: tempfile::TempDir,
        repo: PathBuf,
        project: Project,
    }

    fn fixture() -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let repo = root.join("repo");
        init_git_repo(&repo);
        let dir = root.join("project");
        fs::create_dir_all(&dir).unwrap();
        let project = Project {
            name: "demo".into(),
            dir,
            repo: Some(repo.clone()),
            state_dir: root.join("state"),
            worktrees_root: root.join("worktrees"),
        };
        Fixture {
            _temp: temp,
            repo,
            project,
        }
    }

    /// Commit `files` on a new branch from `main` at committer `time`, and
    /// publish its tip as a snapshot of `worktree`, as sync names them.
    fn snapshot(fixture: &Fixture, worktree: &str, files: &[(&str, &str)], time: u32) -> String {
        let repo = &fixture.repo;
        let work = format!("work-{worktree}-{time}");
        must_run_git(repo, &["checkout", "-q", "-b", &work, "main"]);
        for (index, (file, text)) in files.iter().enumerate() {
            fs::write(repo.join(file), text).unwrap();
            must_run_git(repo, &["add", file]);
            let date = format!("2026-10-{:02}T12:00:{index:02}Z", time);
            git_at(
                repo,
                &date,
                &["commit", "-q", "-m", &format!("change {file}")],
            );
        }
        must_run_git(repo, &["checkout", "-q", "main"]);
        let tip = must_git_output(repo, &["rev-parse", &work]);
        let source = sync::source_id(&review::snapshot_dir(&fixture.project), worktree);
        let branch = format!(
            "agent-review/{worktree}/202610{time:02}-120000-{source}-{}",
            &tip[..12]
        );
        must_run_git(repo, &["branch", &branch, &work]);
        must_run_git(repo, &["branch", "-D", &work]);
        branch
    }

    fn git_at(repo: &Path, date: &str, args: &[&str]) {
        let output = Command::new("git")
            .current_dir(repo)
            .env("GIT_COMMITTER_DATE", date)
            .env("GIT_AUTHOR_DATE", date)
            .args([
                "-c",
                "user.name=cli-toolbox",
                "-c",
                "user.email=cli-toolbox@example.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// No running VM.
    fn no_guest(_: &str) -> Option<Result<HashSet<String>, Error>> {
        None
    }

    fn branches(repo: &Path) -> Vec<String> {
        must_git_output(
            repo,
            &[
                "for-each-ref",
                "--format=%(refname:short)",
                "refs/heads/agent-review/",
            ],
        )
        .lines()
        .map(String::from)
        .collect()
    }

    fn review_checkout(fixture: &Fixture, branch: &str) -> PathBuf {
        review::prepare(
            &fixture.repo,
            &review::checkouts_dir(&fixture.project),
            branch,
        )
        .unwrap()
        .path
    }

    #[test]
    fn cherry_picked_and_merged_snapshots_go_with_their_checkouts() {
        if skip_without_git() {
            return;
        }
        let fixture = fixture();
        let picked = snapshot(&fixture, "picked", &[("a.txt", "a\n")], 1);
        let merged = snapshot(&fixture, "merged", &[("b.txt", "b\n")], 2);
        must_run_git(&fixture.repo, &["cherry-pick", &picked]);
        must_run_git(&fixture.repo, &["merge", "-q", "--no-edit", &merged]);
        let checkouts = [
            review_checkout(&fixture, &picked),
            review_checkout(&fixture, &merged),
        ];

        let report = prune_with(&fixture.project, &PruneOptions::default(), no_guest).unwrap();
        assert!(branches(&fixture.repo).is_empty(), "{report}");
        for checkout in &checkouts {
            assert!(!checkout.exists(), "{report}");
        }
        assert_eq!(report.deleted, 4, "{report}");
        assert!(!report.steps.iter().any(Step::failed), "{report}");
        let text = report.to_string();
        assert!(
            text.contains(&format!("deleted {picked} (on main)")),
            "{text}"
        );
    }

    #[test]
    fn partly_accepted_snapshots_and_dirty_checkouts_stay_unless_forced() {
        if skip_without_git() {
            return;
        }
        let fixture = fixture();
        let partial = snapshot(
            &fixture,
            "partial",
            &[("a.txt", "a\n"), ("b.txt", "b\n")],
            1,
        );
        must_run_git(&fixture.repo, &["cherry-pick", &format!("{partial}~1")]);
        let accepted = snapshot(&fixture, "accepted", &[("c.txt", "c\n")], 2);
        must_run_git(&fixture.repo, &["cherry-pick", &accepted]);
        let checkout = review_checkout(&fixture, &accepted);
        fs::write(checkout.join("notes.txt"), "review notes\n").unwrap();

        let report = prune_with(&fixture.project, &PruneOptions::default(), no_guest).unwrap();
        let text = report.to_string();
        assert!(
            text.contains(&format!(
                "kept {partial}: 1 commit not on main, first \"change b.txt\""
            )),
            "{text}"
        );
        assert!(text.contains("local edits; --force removes it"), "{text}");
        assert_eq!((report.unaccepted, report.dirty), (1, 1), "{text}");
        assert_eq!(branches(&fixture.repo), std::slice::from_ref(&partial));
        assert!(checkout.exists());

        let forced = PruneOptions {
            all: true,
            force: true,
            ..PruneOptions::default()
        };
        let report = prune_with(&fixture.project, &forced, no_guest).unwrap();
        assert!(branches(&fixture.repo).is_empty(), "{report}");
        assert!(!checkout.exists(), "{report}");
    }

    #[test]
    fn superseded_snapshots_are_listed_until_asked_for() {
        if skip_without_git() {
            return;
        }
        let fixture = fixture();
        // The agent amended its work: the older tip has a commit it dropped.
        let older = snapshot(&fixture, "gone", &[("a.txt", "first try\n")], 1);
        let newest = snapshot(&fixture, "gone", &[("a.txt", "final\n")], 2);
        must_run_git(&fixture.repo, &["cherry-pick", &newest]);

        let report = prune_with(&fixture.project, &PruneOptions::default(), no_guest).unwrap();
        let text = report.to_string();
        assert!(
            text.contains(&format!(
                "kept {older}: superseded by accepted {newest}; --superseded deletes it"
            )),
            "{text}"
        );
        assert_eq!(report.superseded, 1);
        assert!(text.contains("marks older snapshots superseded"), "{text}");
        assert_eq!(branches(&fixture.repo), [older.clone(), newest.clone()]);

        let options = PruneOptions {
            superseded: true,
            ..PruneOptions::default()
        };
        prune_with(&fixture.project, &options, no_guest).unwrap();
        assert!(branches(&fixture.repo).is_empty());
    }

    #[test]
    fn fetch_refs_of_deleted_guest_branches_go_once_their_work_is_safe() {
        if skip_without_git() {
            return;
        }
        let fixture = fixture();
        let repo = &fixture.repo;
        let worktrees_root = &fixture.project.worktrees_root;
        git::create_worktree(repo, worktrees_root, "live", &mut Vec::new()).unwrap();
        let accepted = snapshot(&fixture, "other", &[("a.txt", "a\n")], 1);
        must_run_git(repo, &["cherry-pick", &accepted]);
        // The guest's main is still saved in a kept snapshot of the worktree.
        let kept = snapshot(&fixture, "live", &[("b.txt", "b\n")], 2);
        let unsaved = snapshot(&fixture, "scratch", &[("c.txt", "c\n")], 3);
        let source = sync::source_id(&review::snapshot_dir(&fixture.project), "live");
        let fetched = |branch: &str, tip: &str| {
            let reference = format!("refs/remotes/sandbox-review/{source}/{branch}");
            must_run_git(repo, &["update-ref", &reference, tip]);
            reference
        };
        let still_there = fetched("main", &unsaved);
        let gone_accepted = fetched("old-fix", &accepted);
        let gone_kept = fetched("draft", &kept);
        let gone_only = fetched("experiment", &unsaved);
        must_run_git(repo, &["branch", "-D", &unsaved]);
        let guest = |worktree: &str| {
            assert_eq!(worktree, "live");
            Some(Ok(HashSet::from(["main".to_string()])))
        };

        let report = prune_with(&fixture.project, &PruneOptions::default(), guest).unwrap();
        let text = report.to_string();
        let exists = |reference: &str| git::resolve_commit(repo, reference).is_some();
        assert!(exists(&still_there), "{text}");
        assert!(!exists(&gone_accepted), "{text}");
        assert!(!exists(&gone_kept), "{text}");
        assert!(exists(&gone_only), "{text}");
        assert!(text.contains(&format!("kept stale {gone_only}")), "{text}");

        // A stopped VM cannot say, and one that fails to answer says so.
        let report = prune_with(&fixture.project, &PruneOptions::default(), no_guest).unwrap();
        assert!(!report.to_string().contains("experiment"), "{report}");
        let failing = |_: &str| Some(Err(Error::msg("SSH unavailable")));
        let report = prune_with(&fixture.project, &PruneOptions::default(), failing).unwrap();
        assert!(
            report
                .to_string()
                .contains("could not list the guest's branches: SSH unavailable")
        );
        assert!(exists(&gone_only));
    }

    #[test]
    fn a_dry_run_changes_nothing() {
        if skip_without_git() {
            return;
        }
        let fixture = fixture();
        let accepted = snapshot(&fixture, "w", &[("a.txt", "a\n")], 1);
        must_run_git(&fixture.repo, &["cherry-pick", &accepted]);
        let checkout = review_checkout(&fixture, &accepted);
        let options = PruneOptions {
            dry_run: true,
            all: true,
            ..PruneOptions::default()
        };
        let report = prune_with(&fixture.project, &options, no_guest).unwrap();
        let text = report.to_string();
        assert!(text.contains(&format!("would delete {accepted}")), "{text}");
        assert!(text.contains("would remove"), "{text}");
        assert_eq!(branches(&fixture.repo), [accepted]);
        assert!(checkout.exists());
    }

    #[test]
    fn fetch_refs_of_removed_worktrees_go_once_their_work_is_safe() {
        if skip_without_git() {
            return;
        }
        let fixture = fixture();
        let repo = &fixture.repo;
        let accepted = snapshot(&fixture, "gone", &[("a.txt", "a\n")], 1);
        let kept = snapshot(&fixture, "other", &[("b.txt", "b\n")], 2);
        must_run_git(repo, &["cherry-pick", &accepted]);
        let dir = review::snapshot_dir(&fixture.project);
        let fetched = |worktree: &str, branch: &str, tip: &str| {
            let reference = format!(
                "refs/remotes/sandbox-review/{}/{branch}",
                sync::source_id(&dir, worktree)
            );
            must_run_git(repo, &["update-ref", &reference, tip]);
            reference
        };
        let safe = fetched("gone", "main", &accepted);
        let in_kept = fetched("other", "main", &kept);
        // Only here: a branch the agent never had a snapshot of.
        let unsaved = snapshot(&fixture, "scratch", &[("c.txt", "c\n")], 3);
        let only = fetched("other", "scratch", &unsaved);
        must_run_git(repo, &["branch", "-D", &unsaved]);

        let report = prune_with(&fixture.project, &PruneOptions::default(), no_guest).unwrap();
        let text = report.to_string();
        let exists = |reference: &str| git::resolve_commit(repo, reference).is_some();
        assert!(!exists(&safe), "{text}");
        assert!(!exists(&in_kept), "{text}");
        assert!(exists(&only), "{text}");
        assert!(text.contains("and in no kept snapshot"), "{text}");
        assert_eq!(branches(repo), [kept]);
    }
}
