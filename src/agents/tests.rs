use super::*;
use std::os::unix::fs::PermissionsExt;
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
    state: PathBuf,
    catalog: Value,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo");
        fs::create_dir(&root).unwrap();
        run_command("git", &["init", "-q"], &root).unwrap();
        let library = temp.path().join("library");
        let state = temp.path().join("state");
        Self {
            temp,
            root,
            library,
            state,
            catalog: json!({"version":1,"mcp":{"nixos":{"type":"stdio","command":"/store/mcp-nixos","args":["--stdio"],"env":{"LEVEL":"1"}}},"sources":[]}),
        }
    }
    fn project(&self) -> Project {
        Project::new(&self.root, self.catalog.clone(), &self.library, &self.state).unwrap()
    }
    /// Choose both the server and the skill.
    fn choose_all(&self) -> Project {
        self.add_skill();
        let mut project = self.project();
        project.set_choice("mcp", "nixos", true).unwrap();
        project.set_choice("skills", "review", true).unwrap();
        project
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
fn assert_error(result: Result<impl Sized, Error>, text: &str) {
    let error = result.err().expect("expected failure").to_string();
    assert!(error.contains(text), "expected {text:?}: {error}");
}

#[test]
fn choices_live_outside_the_checkout_and_each_worktree_has_its_own() {
    let f = Fixture::new();
    run_command(
        "git",
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-qm",
            "initial",
            "--allow-empty",
        ],
        &f.root,
    )
    .unwrap();
    f.project().set_choice("mcp", "nixos", true).unwrap();
    let entries: Vec<_> = fs::read_dir(&f.root)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(entries, [".git"]);
    assert!(f.project().enabled("mcp", "nixos"));

    let linked = f.temp.path().join("linked");
    run_command(
        "git",
        &["worktree", "add", "-qb", "linked", linked.to_str().unwrap()],
        &f.root,
    )
    .unwrap();
    let subdir = linked.join("subdir");
    fs::create_dir(&subdir).unwrap();
    assert_eq!(
        project_root(&subdir).unwrap(),
        linked.canonicalize().unwrap()
    );
    let worktree = Project::new(&linked, f.catalog.clone(), &f.library, &f.state).unwrap();
    assert!(!worktree.enabled("mcp", "nixos"));

    f.project().set_choice("mcp", "nixos", false).unwrap();
    assert!(!f.project().enabled("mcp", "nixos"));
}

#[test]
fn unknown_names_are_rejected_and_choices_the_catalog_dropped_are_ignored() {
    let mut f = Fixture::new();
    assert_error(
        f.project().set_choice("mcp", "typo", true),
        "not in the catalog",
    );
    assert_error(
        f.project().set_choice("targets", "pi", true),
        "unknown group",
    );
    f.project().set_choice("mcp", "nixos", true).unwrap();
    f.catalog["mcp"] = json!({});
    let project = f.project();
    assert!(project.overview().is_empty());
    assert_eq!(project.launch(Agent::Claude).unwrap(), Launch::default());
}

#[test]
fn launch_gives_each_agent_its_native_options() {
    let f = Fixture::new();
    let project = f.choose_all();
    let skill = f.library.join(".rulesync/skills/.curated/review");

    let claude = project.launch(Agent::Claude).unwrap();
    assert_eq!(claude.args[0], "--plugin-dir");
    let plugin = PathBuf::from(&claude.args[1]);
    assert!(plugin.starts_with(&f.state));
    assert_eq!(
        load_json(&plugin.join(".claude-plugin/plugin.json"), Value::Null).unwrap()["name"],
        "ct-agents"
    );
    assert_eq!(
        load_json(&plugin.join(".mcp.json"), Value::Null).unwrap(),
        json!({"mcpServers":{"nixos":{"command":"/store/mcp-nixos","args":["--stdio"],"env":{"LEVEL":"1"}}}})
    );
    assert_eq!(snapshot(&plugin.join("skills/review")), snapshot(&skill));
    let mode = fs::metadata(plugin.join("skills/review/run.sh"))
        .unwrap()
        .permissions()
        .mode();
    assert_ne!(mode & 0o111, 0);
    assert!(claude.notes.is_empty());
    // The same choices reuse the plugin.
    assert_eq!(project.launch(Agent::Claude).unwrap(), claude);

    let codex = project.launch(Agent::Codex).unwrap();
    assert_eq!(codex.args[0], "-c");
    let value = codex.args[1].strip_prefix("mcp_servers.nixos=").unwrap();
    let doc: toml_edit::DocumentMut = format!("server = {value}").parse().unwrap();
    assert_eq!(doc["server"]["command"].as_str(), Some("/store/mcp-nixos"));
    assert_eq!(doc["server"]["env"]["LEVEL"].as_str(), Some("1"));
    assert_eq!(
        codex.notes,
        ["codex cannot load skills at launch; skipped review"]
    );

    let opencode = project.launch(Agent::Opencode).unwrap();
    assert!(opencode.args.is_empty());
    assert_eq!(
        opencode.notes,
        [
            "opencode cannot load MCP servers at launch; skipped nixos",
            "opencode cannot load skills at launch; skipped review"
        ]
    );

    let copilot = project.launch(Agent::Copilot).unwrap();
    assert_eq!(copilot.args[0], "--plugin-dir");
    // Copilot's plugin has only the skills; servers come separately.
    assert!(!PathBuf::from(&copilot.args[1]).join(".mcp.json").exists());
    assert_eq!(copilot.args[2], "--additional-mcp-config");
    assert_eq!(
        serde_json::from_str::<Value>(&copilot.args[3]).unwrap()["mcpServers"]["nixos"],
        json!({"type":"local","command":"/store/mcp-nixos","args":["--stdio"],"env":{"LEVEL":"1"},"tools":["*"]})
    );

    let pi = project.launch(Agent::Pi).unwrap();
    assert_eq!(pi.args, ["--skill", skill.to_str().unwrap()]);
    assert_eq!(
        pi.notes,
        ["pi cannot load MCP servers at launch; skipped nixos"]
    );
}

#[test]
fn nothing_chosen_adds_nothing_and_pi_commands_take_no_session_options() {
    let f = Fixture::new();
    for agent in Agent::ALL {
        assert_eq!(f.project().launch(agent).unwrap(), Launch::default());
        assert_eq!(Agent::parse(agent.name()), Some(agent));
    }
    assert!(!f.state.join("projects").exists() || snapshot(&f.state).is_empty());
    assert!(Agent::Pi.starts_session(&["fix the bug".into()]));
    assert!(!Agent::Pi.starts_session(&["mcp".into(), "list".into()]));
    assert!(Agent::Claude.starts_session(&["mcp".into(), "list".into()]));
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
    let mut project = f.project();
    project.set_choice("skills", "notes", true).unwrap();
    assert_eq!(
        project.launch(Agent::Pi).unwrap().args,
        ["--skill", skill.to_str().unwrap()]
    );

    f.add_skill();
    f.catalog["localSkills"] = json!({"review": skill});
    assert_error(
        Project::new(&f.root, f.catalog.clone(), &f.library, &f.state),
        "clashes",
    );
    f.catalog["localSkills"] = json!({"notes": "relative"});
    assert_error(
        Project::new(&f.root, f.catalog.clone(), &f.library, &f.state),
        "absolute path",
    );
}

#[test]
fn stale_library_and_invalid_choices_are_errors() {
    let mut f = Fixture::new();
    write_json(
        &private_dir(&f.state, &f.root.canonicalize().unwrap()).join(CHOICES),
        &json!({"version":2}),
    )
    .unwrap();
    assert_error(
        Project::new(&f.root, f.catalog.clone(), &f.library, &f.state),
        "choices version",
    );
    f.catalog["sources"] = json!([{"source":"example/skills"}]);
    assert_error(
        Project::new(&f.root, f.catalog.clone(), &f.library, &f.state),
        "library install",
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
        Project::new(&f.root, f.catalog.clone(), &f.library, &f.state),
        "library was edited",
    );
    f.add_skill();
    write_json(&f.library.join("rulesync.lock"), &json!({"revision":"one"})).unwrap();
    write_receipt(&f.library, &f.catalog).unwrap();
    write_json(&f.library.join("rulesync.lock"), &json!({"revision":"two"})).unwrap();
    assert_error(
        Project::new(&f.root, f.catalog.clone(), &f.library, &f.state),
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
