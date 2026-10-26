use super::*;
use tempfile::TempDir;

struct FakeBackend;
impl Backend for FakeBackend {
    fn install(&self, directory: &Path, update: bool) -> Result<(), Error> {
        let lock = load_json(
            &directory.join("rulesync.lock"),
            json!({"revision":"first"}),
        )?;
        write_json(
            &directory.join("rulesync.lock"),
            &if update {
                json!({"revision":"new"})
            } else {
                lock
            },
        )?;
        let skill = directory.join(".rulesync/skills/.curated/review");
        write_file(
            &skill.join("SKILL.md"),
            b"---\nname: review\ndescription: Review code\n---\nReview.\n",
            0o644,
        )
    }
}

struct Fixture {
    temp: TempDir,
    root: PathBuf,
    library: PathBuf,
    catalog: Value,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo");
        fs::create_dir(&root).unwrap();
        run_command("git", &["init", "-q"], &root).unwrap();
        let library = temp.path().join("library");
        Self {
            temp,
            root,
            library,
            catalog: json!({"version":1,"mcp":{"nixos":{"type":"stdio","command":"mcp-nixos","args":["--stdio"],"env":{"LEVEL":"1"}}},"sources":[]}),
        }
    }
    fn project(&self) -> Project {
        Project::new(&self.root, self.catalog.clone(), &self.library).unwrap()
    }
    fn choose(&self, entries: &[(&str, &str)]) {
        let mut project = self.project();
        for (group, name) in entries {
            project.set_choice(group, name, true).unwrap();
        }
    }
    fn drop_choice(&self, group: &str, name: &str) {
        self.project().set_choice(group, name, false).unwrap();
    }
    fn sync(&self) -> Result<String, Error> {
        let mut out = Vec::new();
        self.project().sync(false, &mut out)?;
        Ok(String::from_utf8(out).unwrap())
    }
    fn json(&self, relative: &str) -> Value {
        load_json(&self.root.join(relative), Value::Null).unwrap()
    }
    fn add_skill(&self) {
        let skill = self.library.join(".rulesync/skills/.curated/review");
        write_file(
            &skill.join("SKILL.md"),
            b"---\nname: review\ndescription: Review code\n---\nReview.\n",
            0o644,
        )
        .unwrap();
        write_file(&skill.join("run.sh"), b"#!/bin/sh\necho review\n", 0o755).unwrap();
        write_receipt(&self.library, &self.catalog).unwrap();
    }
}
fn snapshot(root: &Path) -> BTreeMap<String, Record> {
    tree_records(root).unwrap()
}
/// The checkout's files apart from Git's own.
fn files(root: &Path) -> Vec<String> {
    snapshot(root)
        .into_keys()
        .filter(|path| !path.starts_with(".git/"))
        .collect()
}
fn assert_error(result: Result<impl Sized, Error>, text: &str) {
    let error = result.err().expect("expected failure").to_string();
    assert!(error.contains(text), "expected {text:?}: {error}");
}

#[test]
fn targets_start_off_so_choices_alone_write_nothing_else() {
    let f = Fixture::new();
    f.choose(&[("mcp", "nixos")]);
    assert!(
        f.project()
            .overview()
            .iter()
            .all(|e| e.group != "targets" || !e.enabled)
    );
    assert_eq!(f.sync().unwrap(), "Up to date.\n");
    assert_eq!(files(&f.root), [CHOICES]);
    assert_eq!(
        f.json(CHOICES),
        json!({"version":1,"mcp":{"nixos":"enable"}})
    );
}

#[test]
fn sync_writes_every_targets_native_files_and_a_lock() {
    let f = Fixture::new();
    f.add_skill();
    let mut choices: Vec<_> = TARGETS.iter().map(|t| ("targets", *t)).collect();
    choices.extend([("mcp", "nixos"), ("skills", "review")]);
    f.choose(&choices);
    f.sync().unwrap();

    let stdio = json!({"command":"mcp-nixos","args":["--stdio"],"env":{"LEVEL":"1"}});
    let mut claude = stdio.clone();
    claude["type"] = json!("stdio");
    assert_eq!(f.json(".mcp.json"), json!({"mcpServers":{"nixos":claude}}));
    assert_eq!(
        f.json(".cursor/mcp.json"),
        json!({"mcpServers":{"nixos":stdio}})
    );
    assert_eq!(
        f.json(".pi/mcp.json"),
        json!({"mcpServers":{"nixos":stdio}})
    );
    let mut copilot = stdio.clone();
    copilot["type"] = json!("local");
    copilot["tools"] = json!(["*"]);
    assert_eq!(
        f.json(".github/mcp.json"),
        json!({"mcpServers":{"nixos":copilot}})
    );
    assert_eq!(
        f.json("opencode.json"),
        json!({"mcp":{"nixos":{"type":"local","command":["mcp-nixos","--stdio"],"enabled":true,"environment":{"LEVEL":"1"}}}})
    );
    let codex: toml_edit::DocumentMut = fs::read_to_string(f.root.join(".codex/config.toml"))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        codex["mcp_servers"]["nixos"]["command"].as_str(),
        Some("mcp-nixos")
    );
    for dir in [".claude/skills", ".agents/skills"] {
        let script = f.root.join(dir).join("review/run.sh");
        assert_ne!(
            fs::metadata(script).unwrap().permissions().mode() & 0o111,
            0
        );
    }
    let lock = f.json(LOCK);
    assert_eq!(lock["mcp"]["claude"]["nixos"], claude);
    assert!(lock["skills"][".claude/skills/review/SKILL.md"].is_object());
    // Committed files are plain files anyone can read.
    let mode = fs::metadata(f.root.join(".mcp.json"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o644);
    assert!(f.project().is_synced().unwrap());
    assert_eq!(f.sync().unwrap(), "Up to date.\n");
}

#[test]
fn skills_go_to_the_fewest_folders_that_reach_every_target() {
    let dirs = |targets: &[&str]| skill_dirs(&targets.iter().copied().collect());
    assert!(dirs(&[]).is_empty());
    assert_eq!(dirs(&["claude"]), [".claude/skills"]);
    assert_eq!(
        dirs(&["claude", "copilot", "cursor", "opencode"]),
        [".claude/skills"]
    );
    assert_eq!(dirs(&["codex"]), [".agents/skills"]);
    assert_eq!(dirs(&["cursor", "opencode", "pi"]), [".agents/skills"]);
    assert_eq!(
        dirs(&["claude", "codex"]),
        [".claude/skills", ".agents/skills"]
    );
}

#[test]
fn turning_things_off_removes_only_what_sync_wrote() {
    let f = Fixture::new();
    f.add_skill();
    let original =
        json!({"runtime":{"theme":"dark"},"mcpServers":{"team":{"command":"team-server"}}});
    write_json(&f.root.join(".mcp.json"), &original).unwrap();
    f.choose(&[
        ("targets", "claude"),
        ("targets", "codex"),
        ("mcp", "nixos"),
        ("skills", "review"),
    ]);
    f.sync().unwrap();
    assert!(f.json(".mcp.json")["mcpServers"]["nixos"].is_object());
    assert_eq!(
        f.json(".mcp.json")["mcpServers"]["team"],
        original["mcpServers"]["team"]
    );

    f.drop_choice("mcp", "nixos");
    f.sync().unwrap();
    assert_eq!(f.json(".mcp.json"), original);
    assert!(!f.root.join(".codex").exists());

    f.drop_choice("targets", "claude");
    f.sync().unwrap();
    assert!(!f.root.join(".claude").exists());
    assert!(f.root.join(".agents/skills/review/SKILL.md").exists());

    f.drop_choice("targets", "codex");
    f.sync().unwrap();
    assert_eq!(files(&f.root), [".cli-toolbox/agents.json", ".mcp.json"]);
    assert_eq!(f.json(".mcp.json"), original);
}

#[test]
fn foreign_and_edited_files_are_never_overwritten() {
    let f = Fixture::new();
    f.add_skill();
    f.choose(&[
        ("targets", "claude"),
        ("mcp", "nixos"),
        ("skills", "review"),
    ]);
    // A different server of the same name belongs to someone else.
    write_json(
        &f.root.join(".mcp.json"),
        &json!({"mcpServers":{"nixos":{"command":"custom"}}}),
    )
    .unwrap();
    let before = snapshot(&f.root);
    assert_error(f.sync(), "already has another MCP server named nixos");
    assert_eq!(snapshot(&f.root), before);
    // An identical one is adopted.
    write_json(
        &f.root.join(".mcp.json"),
        &json!({"mcpServers":{"nixos":{"type":"stdio","command":"mcp-nixos","args":["--stdio"],"env":{"LEVEL":"1"}}}}),
    )
    .unwrap();
    f.sync().unwrap();
    assert!(f.project().is_synced().unwrap());

    write_json(
        &f.root.join(".mcp.json"),
        &json!({"mcpServers":{"nixos":{"command":"edited"}}}),
    )
    .unwrap();
    assert_error(f.sync(), "was edited");
    fs::remove_file(f.root.join(".mcp.json")).unwrap();
    f.sync().unwrap();

    let skill = f.root.join(".claude/skills/review/SKILL.md");
    fs::write(&skill, "local edit").unwrap();
    f.drop_choice("skills", "review");
    assert_error(f.sync(), "was edited");
    fs::remove_dir_all(f.root.join(".claude")).unwrap();
    f.sync().unwrap();

    write_file(&skill, b"someone else's", 0o644).unwrap();
    f.choose(&[("skills", "review")]);
    assert_error(f.sync(), "not written by ct agents");
}

#[test]
fn servers_with_machine_paths_cannot_be_committed() {
    let mut f = Fixture::new();
    f.catalog["mcp"]["store"] = json!({"type":"stdio","command":"/nix/store/x/bin/server"});
    f.catalog["mcp"]["home"] = json!({"type":"stdio","command":"server","args":["~/vault"]});
    f.choose(&[("mcp", "store")]);
    // Without targets nothing is written, so nothing is checked.
    f.sync().unwrap();
    f.choose(&[("targets", "pi")]);
    assert_error(f.sync(), "store is not portable");
    f.drop_choice("mcp", "store");
    f.choose(&[("mcp", "home")]);
    assert_error(f.sync(), "home is not portable: ~/vault");
}

#[test]
fn another_checkout_with_the_committed_files_continues_where_this_one_left() {
    let f = Fixture::new();
    f.add_skill();
    f.choose(&[
        ("targets", "claude"),
        ("mcp", "nixos"),
        ("skills", "review"),
    ]);
    f.sync().unwrap();
    let clone = f.temp.path().join("clone");
    fs::create_dir(&clone).unwrap();
    run_command("git", &["init", "-q"], &clone).unwrap();
    // As Git would check them out, executable bits included.
    for file in files(&f.root) {
        let source = f.root.join(&file);
        let mode = fs::metadata(&source).unwrap().permissions().mode() & 0o777;
        write_file(&clone.join(&file), &fs::read(&source).unwrap(), mode).unwrap();
    }
    let mut other = Project::new(&clone, f.catalog.clone(), &f.library).unwrap();
    assert!(other.is_synced().unwrap());
    other.set_choice("targets", "claude", false).unwrap();
    other.sync(false, &mut Vec::new()).unwrap();
    assert_eq!(files(&clone), [CHOICES]);
}

#[test]
fn preview_changes_nothing_and_unknown_names_are_rejected() {
    let f = Fixture::new();
    f.choose(&[("targets", "claude"), ("mcp", "nixos")]);
    let before = snapshot(&f.root);
    let mut out = Vec::new();
    f.project().sync(true, &mut out).unwrap();
    assert_eq!(snapshot(&f.root), before);
    let out = String::from_utf8(out).unwrap();
    assert!(out.contains("create .mcp.json"), "{out}");
    assert!(out.contains(&format!("create {LOCK}")), "{out}");
    assert_error(
        f.project().set_choice("mcp", "typo", true),
        "unknown mcp entry",
    );
    assert_error(
        f.project().set_choice("targets", "gemini", true),
        "unknown targets entry",
    );
}

#[test]
fn local_skills_need_no_install_and_must_be_safe() {
    let mut f = Fixture::new();
    let skill = f.temp.path().join("local/notes");
    write_file(
        &skill.join("SKILL.md"),
        b"---\nname: notes\ndescription: Notes\n---\nNotes.\n",
        0o644,
    )
    .unwrap();
    f.catalog["localSkills"] = json!({"notes": skill});
    f.choose(&[("targets", "pi"), ("skills", "notes")]);
    f.sync().unwrap();
    assert!(f.root.join(".agents/skills/notes/SKILL.md").exists());

    f.add_skill();
    f.catalog["localSkills"] = json!({"review": skill});
    assert_error(
        Project::new(&f.root, f.catalog.clone(), &f.library),
        "clashes",
    );
    f.catalog["localSkills"] = json!({"notes": "relative"});
    assert_error(
        Project::new(&f.root, f.catalog.clone(), &f.library),
        "absolute path",
    );
}

#[test]
fn jsonc_preserves_strings_and_rejects_duplicate_keys() {
    assert_eq!(
        parse_json("{/*comment*/\"url\":\"https://test/x//y\",\"items\":[1,],}").unwrap(),
        json!({"url":"https://test/x//y","items":[1]})
    );
    assert_error(parse_json("{\"a\":1,\"a\":2}"), "duplicate JSON key");
}

#[test]
fn library_install_retains_lock_update_refreshes_and_failure_is_atomic() {
    let mut f = Fixture::new();
    f.catalog["sources"] = json!([{"source":"example/skills","skills":["review"]}]);
    install_library(&f.library, &f.catalog, &FakeBackend, false).unwrap();
    install_library(&f.library, &f.catalog, &FakeBackend, false).unwrap();
    assert_eq!(
        load_json(&f.library.join("rulesync.lock"), Value::Null).unwrap(),
        json!({"revision":"first"})
    );
    install_library(&f.library, &f.catalog, &FakeBackend, true).unwrap();
    assert_eq!(
        load_json(&f.library.join("rulesync.lock"), Value::Null).unwrap(),
        json!({"revision":"new"})
    );
    struct Offline;
    impl Backend for Offline {
        fn install(&self, _: &Path, _: bool) -> Result<(), Error> {
            Err(Error::msg("offline"))
        }
    }
    let before = snapshot(&f.library);
    assert_error(
        install_library(&f.library, &f.catalog, &Offline, true),
        "offline",
    );
    assert_eq!(snapshot(&f.library), before);
    f.catalog["sources"] = json!([]);
    install_library(&f.library, &f.catalog, &FakeBackend, false).unwrap();
    assert!(library_skills(&f.library, &f.catalog).unwrap().is_empty());
}

#[test]
fn library_edits_and_lockfile_edits_are_reported() {
    let f = Fixture::new();
    f.add_skill();
    fs::write(
        f.library.join(".rulesync/skills/.curated/review/run.sh"),
        "edited",
    )
    .unwrap();
    assert_error(
        Project::new(&f.root, f.catalog.clone(), &f.library),
        "library was edited",
    );
    f.add_skill();
    write_json(&f.library.join("rulesync.lock"), &json!({"revision":"one"})).unwrap();
    write_receipt(&f.library, &f.catalog).unwrap();
    write_json(&f.library.join("rulesync.lock"), &json!({"revision":"two"})).unwrap();
    assert_error(
        Project::new(&f.root, f.catalog.clone(), &f.library),
        "lockfile changed",
    );
}

#[test]
#[ignore = "requires AGENTS_RULESYNC_TEST_BIN pointing to Rulesync 38.0.0"]
fn rulesync_library_install_locks_update_refreshes_and_failure_preserves_library() {
    let backend = Rulesync {
        executable: std::env::var("AGENTS_RULESYNC_TEST_BIN")
            .expect("set AGENTS_RULESYNC_TEST_BIN"),
    };
    let mut f = Fixture::new();
    let remote = f.temp.path().join("skill-source");
    fs::create_dir(&remote).unwrap();
    run_command("git", &["init", "-q", "-b", "main"], &remote).unwrap();
    let skill = remote.join("skills/review");
    write_file(
        &skill.join("SKILL.md"),
        b"---\nname: review\ndescription: Review code\n---\nFirst revision.\n",
        0o644,
    )
    .unwrap();
    write_file(&skill.join("run.sh"), b"#!/bin/sh\necho review\n", 0o755).unwrap();
    let commit = |message: &str| {
        run_command("git", &["add", "skills"], &remote).unwrap();
        run_command(
            "git",
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "-qm",
                message,
            ],
            &remote,
        )
        .unwrap();
        run_command("git", &["rev-parse", "HEAD"], &remote).unwrap()
    };
    let first = commit("first");
    let url = format!("file://{}", remote.canonicalize().unwrap().display());
    f.catalog["sources"] =
        json!([{"source":url,"transport":"git","ref":"main","skills":["review"]}]);
    install_library(&f.library, &f.catalog, &backend, false).unwrap();
    let installed = f.library.join(".rulesync/skills/.curated/review/SKILL.md");
    let lock_path = f.library.join("rulesync.lock");
    let locked_ref = || {
        let lock = load_json(&lock_path, Value::Null).unwrap();
        let sources = lock["sources"].as_object().expect("source lock entries");
        assert_eq!(sources.len(), 1, "{lock}");
        sources.values().next().unwrap()["resolvedRef"]
            .as_str()
            .expect("locked commit")
            .to_owned()
    };
    assert_eq!(locked_ref(), first);
    assert_ne!(
        fs::metadata(installed.with_file_name("run.sh"))
            .unwrap()
            .permissions()
            .mode()
            & 0o111,
        0
    );
    fs::write(
        skill.join("SKILL.md"),
        "---\nname: review\ndescription: Review code\n---\nSecond revision.\n",
    )
    .unwrap();
    let second = commit("second");
    install_library(&f.library, &f.catalog, &backend, false).unwrap();
    assert_eq!(locked_ref(), first);
    assert!(
        fs::read_to_string(&installed)
            .unwrap()
            .contains("First revision.")
    );
    install_library(&f.library, &f.catalog, &backend, true).unwrap();
    assert_eq!(locked_ref(), second);
    assert!(
        fs::read_to_string(&installed)
            .unwrap()
            .contains("Second revision.")
    );
    let before = snapshot(&f.library);
    f.catalog["sources"][0]["ref"] = json!("missing-branch");
    assert!(install_library(&f.library, &f.catalog, &backend, true).is_err());
    assert_eq!(snapshot(&f.library), before);
}

#[test]
fn library_promotion_failure_restores_or_retains_previous_installation() {
    for fail_restore in [false, true] {
        let f = Fixture::new();
        write_file(&f.library.join("old"), b"previous installation", 0o600).unwrap();
        let temporary = tempfile::tempdir_in(f.temp.path()).unwrap();
        let saved = temporary.path().to_path_buf();
        write_file(&saved.join("library/new"), b"replacement", 0o600).unwrap();
        let mut calls = 0;
        let result = promote_library(temporary, &f.library, |from, to| {
            calls += 1;
            if calls == 2 || (calls == 3 && fail_restore) {
                return Err(std::io::Error::other("simulated rename failure"));
            }
            fs::rename(from, to)
        });
        assert_error(result, "simulated rename failure");
        let old = if fail_restore {
            saved.join("previous/old")
        } else {
            f.library.join("old")
        };
        assert_eq!(fs::read_to_string(old).unwrap(), "previous installation");
    }
}
