use super::*;
use std::os::unix::fs::{PermissionsExt, symlink};
use tempfile::TempDir;

struct FakeBackend;
impl Backend for FakeBackend {
    fn generate(&self, target: &str, source: &Path, output: &Path) -> Result<(), Error> {
        let spec = target_spec(target)?;
        let servers = load_json(&source.join("mcp.jsonc"), json!({}))?["mcpServers"].clone();
        let mut native = Native::read(&output.join(spec.mcp.last().unwrap()))?;
        native.set_servers(spec.key, servers)?;
        write_file(
            &output.join(spec.mcp.last().unwrap()),
            &native.bytes()?,
            0o600,
        )?;
        copy_tree(&source.join("skills"), &output.join(spec.skills))
    }
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
            catalog: json!({"version":1,"targets":["claudecode"],"mcp":{"nixos":{"type":"stdio","command":"mcp-nixos"}},"sources":[],"defaults":{"targets":{"claudecode":"enable"}}}),
        }
    }
    fn project(&self) -> Project<'_> {
        Project::new(
            &self.root,
            self.catalog.clone(),
            &self.library,
            &self.state,
            &FakeBackend,
        )
        .unwrap()
    }
    /// A personal file of the checkout, outside it.
    fn private(&self, name: &str) -> PathBuf {
        private_dir(&self.state, &self.root.canonicalize().unwrap()).join(name)
    }
    fn choices(&self, shared: bool, groups: Value) {
        let mut groups = groups;
        groups["version"] = json!(1);
        let path = if shared {
            self.root.join(SHARED)
        } else {
            self.private(PERSONAL)
        };
        write_json(&path, &groups).unwrap();
    }
    fn sync(&self) -> Result<bool, Error> {
        self.project().sync(false, &[], &mut Vec::new())
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
fn shared_personal_and_default_inheritance() {
    let mut f = Fixture::new();
    f.catalog["defaults"]["mcp"] = json!({"nixos":"enable"});
    f.choices(true, json!({"mcp":{"nixos":"disable"}}));
    f.choices(false, json!({"mcp":{"nixos":"enable"}}));
    assert!(f.project().enabled("mcp", "nixos"));
    f.choices(false, json!({"mcp":{"nixos":"inherit"}}));
    assert!(!f.project().enabled("mcp", "nixos"));
    f.choices(true, json!({}));
    assert!(f.project().enabled("mcp", "nixos"));
}

#[test]
fn enable_repeat_disable_preserves_other_servers_and_settings() {
    let f = Fixture::new();
    let path = f.root.join(".mcp.json");
    let original = json!({"runtime":{"theme":"dark"},"mcpServers":{"other":{"command":"other"}}});
    write_json(&path, &original).unwrap();
    f.choices(false, json!({"mcp":{"nixos":"enable"}}));
    assert_error(f.sync(), "adopt-mcp claudecode");
    assert_eq!(load_json(&path, Value::Null).unwrap(), original);
    f.project()
        .sync(false, &["claudecode".into()], &mut Vec::new())
        .unwrap();
    let first = fs::read(&path).unwrap();
    assert!(!f.sync().unwrap());
    assert_eq!(fs::read(&path).unwrap(), first);
    f.choices(false, json!({"mcp":{"nixos":"disable"}}));
    f.sync().unwrap();
    assert_eq!(load_json(&path, Value::Null).unwrap(), original);
}

#[test]
fn dry_run_is_read_only_including_library_exclude_and_state() {
    let mut f = Fixture::new();
    f.add_skill();
    f.catalog["defaults"]["mcp"] = json!({"nixos":"enable"});
    f.catalog["defaults"]["skills"] = json!({"review":"enable"});
    let before = snapshot(f.temp.path());
    let mut out = Vec::new();
    f.project().sync(true, &[], &mut out).unwrap();
    assert_eq!(snapshot(f.temp.path()), before);
    assert!(!f.root.join(".cli-toolbox").exists());
    assert!(!f.root.join(".claude").exists());
    assert!(String::from_utf8(out).unwrap().contains(".mcp.json"));
}

#[test]
fn unowned_and_edited_skill_conflicts_preflight_everything() {
    let f = Fixture::new();
    f.add_skill();
    f.choices(
        false,
        json!({"mcp":{"nixos":"enable"},"skills":{"review":"enable"}}),
    );
    let path = f.root.join(".claude/skills/review/SKILL.md");
    write_file(&path, b"personal", 0o644).unwrap();
    let before = snapshot(&f.root);
    assert_error(f.sync(), "unowned skill");
    assert_eq!(snapshot(&f.root), before);
    fs::remove_file(&path).unwrap();
    f.sync().unwrap();
    fs::write(&path, "local edit").unwrap();
    f.choices(false, json!({"skills":{"review":"disable"}}));
    let before = snapshot(&f.root);
    assert_error(f.sync(), "locally edited");
    assert_eq!(snapshot(&f.root), before);
}

#[test]
fn disable_only_removes_owned_files_and_preserves_executable_bits() {
    let f = Fixture::new();
    f.add_skill();
    f.choices(false, json!({"skills":{"review":"enable"}}));
    f.sync().unwrap();
    let script = f.root.join(".claude/skills/review/run.sh");
    assert_ne!(
        fs::metadata(&script).unwrap().permissions().mode() & 0o111,
        0
    );
    let independent = script.with_file_name("personal.txt");
    fs::write(&independent, "keep").unwrap();
    f.choices(false, json!({"skills":{"review":"disable"}}));
    f.sync().unwrap();
    assert!(!script.exists());
    assert_eq!(fs::read_to_string(independent).unwrap(), "keep");
}

#[test]
fn disabling_target_cleans_previous_selections() {
    let f = Fixture::new();
    f.add_skill();
    f.choices(
        false,
        json!({"mcp":{"nixos":"enable"},"skills":{"review":"enable"}}),
    );
    f.sync().unwrap();
    f.choices(false, json!({"targets":{"claudecode":"disable"}}));
    f.sync().unwrap();
    // Nothing is left for the target: no empty file or directories.
    assert!(!f.root.join(".mcp.json").exists());
    assert!(!f.root.join(".claude").exists());
}

#[test]
fn disabling_the_last_server_keeps_other_settings_without_an_empty_section() {
    let mut f = Fixture::new();
    f.catalog["targets"] = json!(["claudecode", "codexcli"]);
    f.catalog["defaults"]["targets"] = json!({"claudecode":"enable","codexcli":"enable"});
    let json_path = f.root.join(".mcp.json");
    write_json(&json_path, &json!({"runtime":{"theme":"dark"}})).unwrap();
    let toml_path = f.root.join(".codex/config.toml");
    write_file(&toml_path, b"# mine\nmodel = 'test-model'\n", 0o644).unwrap();
    f.choices(false, json!({"mcp":{"nixos":"enable"}}));
    f.sync().unwrap();
    assert!(
        fs::read_to_string(&toml_path)
            .unwrap()
            .contains("mcp-nixos")
    );
    f.choices(false, json!({"mcp":{"nixos":"disable"}}));
    f.sync().unwrap();
    assert_eq!(
        load_json(&json_path, Value::Null).unwrap(),
        json!({"runtime":{"theme":"dark"}})
    );
    assert_eq!(
        fs::read_to_string(&toml_path).unwrap(),
        "# mine\nmodel = 'test-model'\n"
    );
    assert!(!f.sync().unwrap());
}

#[test]
fn edited_managed_server_is_not_overwritten() {
    let f = Fixture::new();
    f.choices(false, json!({"mcp":{"nixos":"enable"}}));
    f.sync().unwrap();
    write_json(
        &f.root.join(".mcp.json"),
        &json!({"mcpServers":{"nixos":{"command":"custom"}}}),
    )
    .unwrap();
    assert_error(f.sync(), "locally edited MCP");
}

#[test]
fn personal_choices_and_ownership_live_outside_the_checkout() {
    let f = Fixture::new();
    f.project()
        .set_choice("mcp", "nixos", "enable", false)
        .unwrap();
    f.sync().unwrap();
    assert!(f.private(PERSONAL).exists());
    assert!(f.private(STATE).exists());
    assert!(!f.root.join(".cli-toolbox").exists());
    let exclude = fs::read_to_string(f.root.join(".git/info/exclude")).unwrap();
    assert!(!exclude.contains(".cli-toolbox"));
}

#[test]
fn files_in_the_checkout_from_older_versions_move_out_on_the_next_write() {
    let f = Fixture::new();
    f.choices(true, json!({}));
    let legacy = |i: usize| f.root.join(LEGACY[i].0);
    write_json(&legacy(0), &json!({"version":1,"mcp":{"nixos":"enable"}})).unwrap();
    let exclude = f.root.join(".git/info/exclude");
    fs::write(
        &exclude,
        format!("# mine\n/{}\n/{}\n", LEGACY[0].0, LEGACY[1].0),
    )
    .unwrap();
    assert!(f.project().enabled("mcp", "nixos"));
    // A preview moves nothing.
    f.project().sync(true, &[], &mut Vec::new()).unwrap();
    assert!(legacy(0).exists());
    f.sync().unwrap();
    // Ownership written before the move is read from the old place too.
    let state = fs::read(f.private(STATE)).unwrap();
    fs::rename(f.private(STATE), legacy(1)).unwrap();
    assert!(f.project().is_synced().unwrap());
    f.project()
        .set_choice("mcp", "nixos", "enable", false)
        .unwrap();
    assert!(!legacy(0).exists() && !legacy(1).exists());
    assert_eq!(fs::read(f.private(STATE)).unwrap(), state);
    assert!(f.private(PERSONAL).exists());
    assert_eq!(fs::read_to_string(&exclude).unwrap(), "# mine\n");
    assert!(f.root.join(SHARED).exists());
}

#[test]
fn local_skills_need_no_install_and_disable_removes_owned_files() {
    let mut f = Fixture::new();
    let skill = f.temp.path().join("local/notes");
    write_file(
        &skill.join("SKILL.md"),
        b"---\nname: notes\ndescription: Notes\n---\nNotes.\n",
        0o644,
    )
    .unwrap();
    f.catalog["localSkills"] = json!({"notes": skill});
    f.choices(false, json!({"skills":{"notes":"enable"}}));
    f.sync().unwrap();
    let generated = f.root.join(".claude/skills/notes/SKILL.md");
    assert!(generated.exists());
    write_file(
        &skill.join("SKILL.md"),
        b"---\nname: notes\ndescription: Notes\n---\nChanged.\n",
        0o644,
    )
    .unwrap();
    assert!(!f.project().is_synced().unwrap());
    f.choices(false, json!({"skills":{"notes":"disable"}}));
    f.sync().unwrap();
    assert!(!generated.exists());

    f.add_skill();
    f.catalog["localSkills"] = json!({"review": skill});
    assert_error(
        Project::new(
            &f.root,
            f.catalog.clone(),
            &f.library,
            &f.state,
            &FakeBackend,
        ),
        "clashes",
    );
    f.catalog["localSkills"] = json!({"notes": "relative"});
    assert_error(
        Project::new(
            &f.root,
            f.catalog.clone(),
            &f.library,
            &f.state,
            &FakeBackend,
        ),
        "absolute path",
    );
}

#[test]
fn exclude_hides_untracked_generated_files_and_is_personal() {
    let f = Fixture::new();
    f.add_skill();
    write_json(&f.root.join(".mcp.json"), &json!({})).unwrap();
    run_command("git", &["add", ".mcp.json"], &f.root).unwrap();
    f.choices(
        false,
        json!({"exclude":true,"mcp":{"nixos":"enable"},"skills":{"review":"enable"}}),
    );
    f.project()
        .sync(false, &["claudecode".into()], &mut Vec::new())
        .unwrap();
    let skill = ".claude/skills/review/SKILL.md";
    run_command("git", &["check-ignore", skill], &f.root).unwrap();
    // Excluding a tracked file has no effect, so it is never listed.
    let exclude = fs::read_to_string(f.root.join(".git/info/exclude")).unwrap();
    assert!(!exclude.contains("/.mcp.json"));

    f.project().set_exclude(false).unwrap();
    f.sync().unwrap();
    assert!(run_command("git", &["check-ignore", skill], &f.root).is_err());

    f.choices(true, json!({"exclude":true}));
    assert_error(
        Project::new(
            &f.root,
            f.catalog.clone(),
            &f.library,
            &f.state,
            &FakeBackend,
        ),
        "personal choice",
    );
}

#[test]
fn subdirectories_and_linked_worktrees_have_independent_state() {
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
    let mut project = Project::new(
        &linked,
        f.catalog.clone(),
        &f.library,
        &f.state,
        &FakeBackend,
    )
    .unwrap();
    project.set_choice("mcp", "nixos", "enable", false).unwrap();
    project.sync(false, &[], &mut Vec::new()).unwrap();
    assert!(linked.join(".mcp.json").exists());
    assert!(!f.root.join(".mcp.json").exists());
    let linked = linked.canonicalize().unwrap();
    assert!(private_dir(&f.state, &linked).join(STATE).exists());
    assert!(!f.private(STATE).exists());
}

#[test]
fn rejects_symlinks_path_escape_unknown_choices_and_stale_library() {
    let mut f = Fixture::new();
    f.choices(false, json!({"mcp":{"nixos":"enable"}}));
    let victim = f.temp.path().join("victim");
    fs::write(&victim, "keep").unwrap();
    symlink(&victim, f.root.join(".mcp.json")).unwrap();
    assert_error(f.sync(), "symlink");
    assert_eq!(fs::read_to_string(victim).unwrap(), "keep");
    assert_error(safe_path(&f.root, "../escape"), "unsafe");
    f.choices(false, json!({"mcp":{"typo":"enable"}}));
    assert_error(
        Project::new(
            &f.root,
            f.catalog.clone(),
            &f.library,
            &f.state,
            &FakeBackend,
        ),
        "unknown mcp",
    );
    f.choices(false, json!({}));
    f.catalog["sources"] = json!([{"source":"example/skills"}]);
    assert_error(
        Project::new(
            &f.root,
            f.catalog.clone(),
            &f.library,
            &f.state,
            &FakeBackend,
        ),
        "library install",
    );
}

#[test]
fn status_detects_selection_and_file_drift_without_backend() {
    let f = Fixture::new();
    f.choices(false, json!({"mcp":{"nixos":"enable"}}));
    f.sync().unwrap();
    assert!(f.project().is_synced().unwrap());
    fs::write(f.root.join(".mcp.json"), "{}").unwrap();
    assert!(!f.project().is_synced().unwrap());
}

#[test]
fn corrupt_manifest_and_invalid_config_never_allow_writes() {
    let f = Fixture::new();
    write_json(&f.private(STATE), &json!({"version":999})).unwrap();
    assert_error(
        Project::new(
            &f.root,
            f.catalog.clone(),
            &f.library,
            &f.state,
            &FakeBackend,
        ),
        "state",
    );
    fs::remove_file(f.private(STATE)).unwrap();
    f.choices(false, json!({"mcp":{"nixos":"enable"}}));
    fs::write(f.root.join(".mcp.json"), "{\"mcpServers\":{},\"broken\":").unwrap();
    let before = snapshot(&f.root);
    assert!(f.sync().is_err());
    assert_eq!(snapshot(&f.root), before);
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
        fn generate(&self, _: &str, _: &Path, _: &Path) -> Result<(), Error> {
            unreachable!()
        }
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
fn changes_during_preview_abort_apply() {
    let f = Fixture::new();
    f.choices(false, json!({"mcp":{"nixos":"enable"}}));
    struct Editor(PathBuf);
    impl Write for Editor {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            fs::write(&self.0, b"{\"runtime\":\"edited\"}")?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    assert_error(
        f.project()
            .sync(false, &[], &mut Editor(f.root.join(".mcp.json"))),
        "changed while preparing",
    );
    assert_eq!(
        load_json(&f.root.join(".mcp.json"), Value::Null).unwrap(),
        json!({"runtime":"edited"})
    );
    assert!(!f.private(STATE).exists());
}

#[test]
fn io_failure_rolls_back_prior_writes() {
    let temp = tempfile::tempdir().unwrap();
    let one = temp.path().join("one");
    let two = temp.path().join("two");
    fs::write(&one, "original").unwrap();
    let changes = BTreeMap::from([
        (one.clone(), (Some(b"changed".to_vec()), 0o644)),
        (two.clone(), (Some(b"new".to_vec()), 0o600)),
    ]);
    let result = apply_changes_with(&changes, |path, content| {
        if path == two {
            return Err(Error::msg("simulated I/O failure"));
        }
        write_file(path, content.0.as_ref().unwrap(), content.1)
    });
    assert_error(result, "simulated I/O failure");
    assert_eq!(fs::read_to_string(one).unwrap(), "original");
    assert!(!two.exists());
}

#[test]
fn toml_merge_preserves_comments_authentication_and_runtime_settings() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    let original = "# chosen model\nmodel = 'test-model' # keep\n\n[profiles.work]\nsandbox_mode = 'workspace-write'\n\n[mcp_servers.other]\ncommand = 'other'\n";
    fs::write(&path, original).unwrap();
    let mut native = Native::read(&path).unwrap();
    let mut servers = native.servers("mcp_servers").unwrap();
    servers["nixos"] = json!({"command":"mcp-nixos","enabled":true});
    native.set_servers("mcp_servers", servers).unwrap();
    let output = String::from_utf8(native.bytes().unwrap()).unwrap();
    assert!(
        output.contains("# chosen model\nmodel = 'test-model' # keep"),
        "{output}"
    );
    assert!(
        output.contains("[profiles.work]\nsandbox_mode = 'workspace-write'"),
        "{output}"
    );
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
        Project::new(
            &f.root,
            f.catalog.clone(),
            &f.library,
            &f.state,
            &FakeBackend,
        ),
        "library was edited",
    );
    f.add_skill();
    write_json(&f.library.join("rulesync.lock"), &json!({"revision":"one"})).unwrap();
    write_receipt(&f.library, &f.catalog).unwrap();
    write_json(&f.library.join("rulesync.lock"), &json!({"revision":"two"})).unwrap();
    assert_error(
        Project::new(
            &f.root,
            f.catalog.clone(),
            &f.library,
            &f.state,
            &FakeBackend,
        ),
        "lockfile changed",
    );
}

#[test]
fn unsafe_manifest_paths_and_directory_collisions_fail_before_writes() {
    let f = Fixture::new();
    f.choices(false, json!({"mcp":{"nixos":"enable"}}));
    let mut state = serde_json::to_value(State::default()).unwrap();
    let rec = serde_json::to_value(record(b"keep", 0o644)).unwrap();
    state["skills"] = json!({".claude/skills/../../victim":rec});
    state["files"] = state["skills"].clone();
    write_json(&f.private(STATE), &state).unwrap();
    assert_error(
        Project::new(
            &f.root,
            f.catalog.clone(),
            &f.library,
            &f.state,
            &FakeBackend,
        ),
        "unsafe",
    );
    fs::remove_file(f.private(STATE)).unwrap();
    fs::create_dir(f.root.join(".mcp.json")).unwrap();
    assert_error(f.sync(), "regular file");
}

#[test]
fn opencode_overrides_and_multiple_configs_require_consolidation() {
    let mut f = Fixture::new();
    f.catalog["targets"] = json!(["opencode"]);
    f.catalog["defaults"]["targets"] = json!({"opencode":"enable"});
    write_json(
        &f.root.join(".opencode/opencode.jsonc"),
        &json!({"mcp":{"other":{"command":["other"]}}}),
    )
    .unwrap();
    assert_error(f.sync(), "overrides project MCP");
    fs::remove_file(f.root.join(".opencode/opencode.jsonc")).unwrap();
    write_json(&f.root.join("opencode.json"), &json!({})).unwrap();
    write_json(&f.root.join("opencode.jsonc"), &json!({})).unwrap();
    assert_error(f.sync(), "multiple opencode");
}

#[test]
#[ignore = "requires AGENTS_RULESYNC_TEST_BIN pointing to Rulesync 38.0.0"]
fn rulesync_all_six_targets_enable_repeat_dry_run_disable_offline() {
    let executable =
        std::env::var("AGENTS_RULESYNC_TEST_BIN").expect("set AGENTS_RULESYNC_TEST_BIN");
    let mut f = Fixture::new();
    f.catalog["targets"] = json!(TARGETS);
    f.catalog["defaults"]["targets"] = Value::Object(
        TARGETS
            .into_iter()
            .map(|t| (t.into(), json!("enable")))
            .collect(),
    );
    f.add_skill();
    // Only generation is permitted; block normal HTTP proxy routes as well.
    // No process-wide environment mutation, so this test can run in parallel.
    let wrapper = f.temp.path().join("offline-rulesync");
    let quoted = format!("'{}'", executable.replace('\'', "'\\''"));
    write_file(&wrapper, format!("#!/bin/sh\n[ \"$1\" = generate ] || exit 97\nexport HTTP_PROXY=http://127.0.0.1:9 HTTPS_PROXY=http://127.0.0.1:9 ALL_PROXY=http://127.0.0.1:9 NO_PROXY= npm_config_offline=true\nexec {quoted} \"$@\"\n").as_bytes(), 0o755).unwrap();
    let backend = Rulesync {
        executable: wrapper.to_str().unwrap().into(),
    };
    for target in TARGETS {
        let spec = target_spec(target).unwrap();
        let path = f.root.join(spec.mcp[0]);
        let mut native = Native::read(&path).unwrap();
        let other = if target == "opencode" {
            json!({"command":["other"],"type":"local","enabled":true})
        } else {
            json!({"command":"other"})
        };
        native
            .set_servers(spec.key, json!({"other":other}))
            .unwrap();
        let mut bytes = native.bytes().unwrap();
        if target == "codexcli" {
            bytes.splice(
                0..0,
                b"# keep model\nmodel = 'test-model'\n".iter().copied(),
            );
        }
        write_file(&path, &bytes, 0o600).unwrap();
        write_file(
            &f.root.join(spec.skills).join("personal/SKILL.md"),
            b"personal",
            0o644,
        )
        .unwrap();
    }
    f.choices(
        true,
        json!({"mcp":{"nixos":"enable"},"skills":{"review":"enable"}}),
    );
    let project =
        || Project::new(&f.root, f.catalog.clone(), &f.library, &f.state, &backend).unwrap();
    let adopt: Vec<_> = TARGETS.into_iter().map(str::to_owned).collect();
    let before = snapshot(&f.root);
    let library_before = snapshot(&f.library);
    project().sync(true, &adopt, &mut Vec::new()).unwrap();
    assert_eq!(snapshot(&f.root), before);
    assert_eq!(snapshot(&f.library), library_before);
    project().sync(false, &adopt, &mut Vec::new()).unwrap();
    for target in TARGETS {
        let spec = target_spec(target).unwrap();
        let native = Native::read(&f.root.join(spec.mcp[0]))
            .unwrap()
            .servers(spec.key)
            .unwrap();
        assert!(native.get("nixos").is_some(), "{target}: {native}");
        assert!(native.get("other").is_some(), "{target}");
        assert!(
            f.root.join(spec.skills).join("review/SKILL.md").exists(),
            "{target}"
        );
        assert_ne!(
            fs::metadata(f.root.join(spec.skills).join("review/run.sh"))
                .unwrap()
                .permissions()
                .mode()
                & 0o111,
            0,
            "{target}"
        );
    }
    assert!(!project().sync(false, &[], &mut Vec::new()).unwrap());
    f.choices(
        true,
        json!({"mcp":{"nixos":"disable"},"skills":{"review":"disable"}}),
    );
    project().sync(false, &[], &mut Vec::new()).unwrap();
    for target in TARGETS {
        let spec = target_spec(target).unwrap();
        let servers = Native::read(&f.root.join(spec.mcp[0]))
            .unwrap()
            .servers(spec.key)
            .unwrap();
        assert!(servers.get("nixos").is_none(), "{target}");
        assert!(servers.get("other").is_some(), "{target}");
        assert!(
            !f.root.join(spec.skills).join("review/SKILL.md").exists(),
            "{target}"
        );
        assert_eq!(
            fs::read_to_string(f.root.join(spec.skills).join("personal/SKILL.md")).unwrap(),
            "personal"
        );
    }
    let codex = fs::read_to_string(f.root.join(".codex/config.toml")).unwrap();
    assert!(codex.contains("# keep model\nmodel = 'test-model'"));
    assert_eq!(snapshot(&f.library), library_before);
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

#[test]
fn layers_cycle_from_inherit_to_the_opposite_and_back() {
    let entry = |shared, personal| Entry {
        group: "mcp".into(),
        name: "nixos".into(),
        default: false,
        shared,
        personal,
    };
    assert_eq!(entry(None, None).next_choice(true), "enable");
    assert_eq!(entry(Some(true), None).next_choice(true), "disable");
    assert_eq!(entry(Some(false), None).next_choice(true), "inherit");
    // Yours starts from the repository's choice.
    assert_eq!(entry(Some(true), None).next_choice(false), "disable");
    assert_eq!(entry(Some(true), Some(false)).next_choice(false), "enable");
    assert_eq!(entry(Some(true), Some(true)).next_choice(false), "inherit");
    let overridden = entry(Some(true), Some(false));
    assert!(!overridden.enabled());
    assert_eq!(overridden.origin(), "personal");
}
