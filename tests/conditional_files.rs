use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

use predicates::prelude::{PredicateBooleanExt, predicate};

struct Project {
    path: PathBuf,
}

impl Project {
    fn new(modules: &[(&str, &str)], outputs: &str) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "yard-conditional-files-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        let project = Self { path };
        let mut inputs = String::new();
        for (name, config) in modules {
            project.write(&format!("modules/{name}.md"), &Self::module(config));
            inputs.push_str(&format!("    {name}: modules/{name}.md\n"));
        }
        project.write(
            "yard.yaml",
            &format!("inputs:\n  modules:\n{inputs}outputs:\n{outputs}"),
        );
        project
    }

    fn module(config: &str) -> String {
        format!(
            "```yaml name=config\n{config}\n```\n```containerfile name=main\nRUN echo module\n```\n"
        )
    }

    fn conditional(expression: &str, outputs: &str) -> Self {
        Self::new(
            &[(
                "module",
                &format!(
                    "args:\n  optional: [enabled, version, other]\nfiles:\n  - path: payload.txt\n    when: {}",
                    serde_json::to_string(expression).unwrap()
                ),
            )],
            outputs,
        )
    }

    fn write(&self, path: &str, content: &str) {
        let path = self.path.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn command(&self) -> assert_cmd::Command {
        let mut command = assert_cmd::Command::cargo_bin("yard").unwrap();
        command
            .current_dir(&self.path)
            .env("XDG_CACHE_HOME", self.path.join("cache"))
            .arg("build");
        command
    }

    /// A cached remote backed by a real local Git repository with no remotes: no network required.
    fn remote(config: &str, module_path: &str, outputs: &str) -> Self {
        let project = Self::new(&[], outputs);
        let repo_relative = "cache/containeryard/sources/git_repos/unknown/owner/repo";
        let module = Self::module(config);
        project.write(&format!("{repo_relative}/{module_path}"), &module);
        let source_file = Path::new(module_path)
            .parent()
            .unwrap()
            .join("assets/payload.txt");
        project.write(
            &format!("{repo_relative}/{}", source_file.display()),
            "downloaded\n",
        );
        let repo = project.path.join(repo_relative);
        for args in [
            vec!["init", "--quiet"],
            vec!["add", "."],
            vec![
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "--quiet",
                "-m",
                "fixture",
            ],
        ] {
            let result = Command::new("git")
                .current_dir(&repo)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .args(args)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
        project.write(
            &format!("cache/containeryard/extracted_files/unknown/owner/repo/HEAD/{module_path}"),
            &module,
        );
        project.write("yard.yaml", &format!(
            "inputs:\n  remotes:\n    - url: https://example.invalid/owner/repo\n      commit: HEAD\n      modules:\n        module: {module_path}\noutputs:\n{outputs}"
        ));
        project
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.path).unwrap();
    }
}

#[test]
fn conditional_files_check_presence_and_typed_values() {
    for (expression, arguments, active) in [
        ("enabled is defined", "{}", false),
        ("enabled is defined", "{enabled: false}", true),
        ("enabled is defined", "{enabled: 0}", true),
        ("enabled is defined", "{enabled: ''}", true),
        ("enabled is defined and enabled == true", "{}", false),
        (
            "enabled is defined and enabled == true",
            "{enabled: false}",
            false,
        ),
        (
            "enabled is defined and enabled == true",
            "{enabled: true}",
            true,
        ),
        (
            "enabled is defined and enabled == true",
            "{enabled: 'true'}",
            false,
        ),
        (
            "enabled is defined and other is defined",
            "{enabled: true}",
            false,
        ),
        (
            "enabled is defined and other is defined",
            "{enabled: true, other: false}",
            true,
        ),
        ("enabled is defined or other is defined", "{other: 0}", true),
        ("not (enabled is defined)", "{}", true),
        (
            "version is defined and version == 'gpu'",
            "{version: cpu}",
            false,
        ),
        (
            "version is defined and version == 'gpu'",
            "{version: gpu}",
            true,
        ),
    ] {
        let project = Project::conditional(
            expression,
            &format!("  output.test.Containerfile:\n    - module: {arguments}\n"),
        );
        let assert = project.command().assert();
        if active {
            assert.failure().stderr(predicate::str::contains(
                "File 'payload.txt' for local module 'module' does not exist",
            ));
        } else {
            assert.success();
        }
        assert!(!project.path.join("payload.txt").exists());
    }
}

#[test]
fn active_local_files_use_yard_relative_paths_and_unconditional_entries_work() {
    let project = Project::new(
        &[("module", "files:\n  - path: assets/payload.txt")],
        "  output.test.Containerfile:\n    - module:\n",
    );
    project.write("assets/payload.txt", "existing file\n");
    project.command().assert().success();
    assert_eq!(
        fs::read_to_string(project.path.join("assets/payload.txt")).unwrap(),
        "existing file\n"
    );
    assert!(!project.path.join("modules/assets/payload.txt").exists());
}

#[test]
fn file_selection_is_evaluated_per_output_and_unioned() {
    let project = Project::conditional(
        "enabled is defined and enabled == true",
        "  disabled.test.Containerfile:\n    - module: {enabled: false}\n  enabled.test.Containerfile:\n    - module: {enabled: true}\n",
    );
    project
        .command()
        .assert()
        .failure()
        .stderr(predicate::str::contains("File 'payload.txt'"));
    assert!(!project.path.join("disabled.test.Containerfile").exists());
    project.write("payload.txt", "shared\n");
    project.command().assert().success();
    assert!(project.path.join("disabled.test.Containerfile").exists());
    assert!(project.path.join("enabled.test.Containerfile").exists());
}

#[test]
fn file_conditions_use_resolved_environment_and_command_values() {
    for (argument, value, active) in [
        ("$YARD_TEST_FILE_VARIANT", "cpu", false),
        ("$YARD_TEST_FILE_VARIANT", "gpu", true),
        ("$(printf gpu)", "unused", true),
    ] {
        let project = Project::conditional(
            "version == 'gpu'",
            &format!("  output.test.Containerfile:\n    - module: {{version: '{argument}'}}\n"),
        );
        let assert = project
            .command()
            .env("YARD_TEST_FILE_VARIANT", value)
            .assert();
        if active {
            assert
                .failure()
                .stderr(predicate::str::contains("File 'payload.txt'"));
        } else {
            assert.success();
        }
    }
}

#[test]
fn only_active_declarations_conflict_and_destinations_are_normalized() {
    let config = "args:\n  optional: [enabled]\nfiles:\n  - path: shared.txt\n    when: 'enabled is defined and enabled == true'";
    for (second, conflict) in [("false", false), ("true", true)] {
        let project = Project::new(
            &[("first", config), ("second", config)],
            &format!(
                "  output.test.Containerfile:\n    - first: {{enabled: true}}\n    - second: {{enabled: {second}}}\n"
            ),
        );
        project.write("shared.txt", "shared\n");
        let assert = project.command().assert();
        if conflict {
            assert.failure().stderr(predicate::str::contains(
                "File 'shared.txt' is declared by both",
            ));
        } else {
            assert.success();
        }
    }
    let project = Project::new(
        &[
            ("first", "files:\n  - path: shared.txt"),
            ("second", "files:\n  - path: ./shared.txt"),
        ],
        "  output.test.Containerfile:\n    - first:\n    - second:\n",
    );
    project
        .command()
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "File 'shared.txt' is declared by both",
        ));
}

#[test]
fn different_outputs_cannot_overwrite_each_others_active_files() {
    let config = "files:\n  - path: shared.txt";
    let project = Project::new(
        &[("first", config), ("second", config)],
        "  first.test.Containerfile:\n    - first:\n  second.test.Containerfile:\n    - second:\n",
    );
    project
        .command()
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "File 'shared.txt' is declared by both",
        ));
    assert!(!project.path.join("shared.txt").exists());
}

#[test]
fn unused_modules_do_not_select_files_or_create_conflicts() {
    let config = "files:\n  - path: shared.txt";
    let project = Project::new(
        &[("first", config), ("unused", config)],
        "  output.test.Containerfile:\n    - first:\n",
    );
    project.write("shared.txt", "shared\n");
    project.command().assert().success();
}

#[test]
fn repeated_uses_and_aliases_of_the_same_module_can_share_files() {
    let project = Project::new(
        &[(
            "module",
            "files:\n  - path: payload.txt\n  - path: ./payload.txt",
        )],
        "  first.test.Containerfile:\n    - module:\n  second.test.Containerfile:\n    - alias:\n",
    );
    let yard = fs::read_to_string(project.path.join("yard.yaml"))
        .unwrap()
        .replace(
            "    module: modules/module.md\n",
            "    module: modules/module.md\n    alias: modules/module.md\n",
        );
    project.write("yard.yaml", &yard);
    project.write("payload.txt", "shared\n");
    project.command().assert().success();
}

#[test]
fn false_conditions_still_validate_path_syntax() {
    for path in ["/absolute.txt", "../outside.txt", "~/payload.txt"] {
        let project = Project::new(
            &[(
                "module",
                &format!("files:\n  - path: {path}\n    when: 'false'"),
            )],
            "  output.test.Containerfile:\n    - module:\n",
        );
        project
            .command()
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "Paths must be relative and cannot contain",
            ));
    }
}

#[test]
fn file_entries_require_path_and_reject_malformed_conditions() {
    for entry in [
        "payload.txt",
        "{when: 'true'}",
        "{path: ''}",
        "{path: payload.txt, when: true}",
        "{path: payload.txt, when: ''}",
        "{path: payload.txt, extra: true}",
    ] {
        let project = Project::new(
            &[("module", &format!("files:\n  - {entry}"))],
            "  output.test.Containerfile:\n    - module:\n",
        );
        project
            .command()
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "A module configuration does not match the expected schema",
            ));
    }
    for (expression, message) in [
        (
            "enabled is defined and",
            "A `files.when` condition is invalid",
        ),
        (
            "enabled is defined or typo is defined",
            "references undeclared argument 'typo'",
        ),
        (
            "false %}ignored{% if true",
            "must be a Tera expression without template tags",
        ),
    ] {
        let project = Project::conditional(
            expression,
            "  output.test.Containerfile:\n    - module: {enabled: true}\n",
        );
        project
            .command()
            .assert()
            .failure()
            .stderr(predicate::str::contains(message));
    }
}

#[test]
fn file_evaluation_errors_are_reported_and_requirements_flags_do_not_skip_files() {
    let project = Project::conditional(
        "other / 0 > 1",
        "  output.test.Containerfile:\n    - module: {other: 4}\n",
    );
    project
        .command()
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "A `files.when` condition could not be evaluated",
        ));
    for flags in [
        vec!["--ignore-requires", "module"],
        vec!["--ignore-all-requires"],
    ] {
        let project = Project::conditional(
            "enabled is defined",
            "  output.test.Containerfile:\n    - module: {enabled: true}\n",
        );
        project
            .command()
            .args(flags)
            .assert()
            .failure()
            .stderr(predicate::str::contains("File 'payload.txt'"));
    }
}

#[test]
fn argument_validation_precedes_file_conditions() {
    let config = "args:\n  optional: [other]\n  validate:\n    - assert: 'false'\n      message: 'Check arguments first.'\nfiles:\n  - path: payload.txt\n    when: 'other / 0 > 1'";
    let project = Project::new(
        &[("module", config)],
        "  output.test.Containerfile:\n    - module: {other: 4}\n",
    );
    project.command().assert().failure().stderr(
        predicate::str::contains("Check arguments first.")
            .and(predicate::str::contains("condition could not be evaluated").not()),
    );
}

#[test]
fn active_remote_files_are_downloaded_relative_to_the_module() {
    for module_path in ["modules/tool.md", "tool.md"] {
        let project = Project::remote(
            "args:\n  optional: [enabled]\nfiles:\n  - path: assets/payload.txt\n    when: 'enabled is defined and enabled == true'\n  - path: assets/missing.txt\n    when: 'false'",
            module_path,
            "  output.test.Containerfile:\n    - module: {enabled: true}\n",
        );
        project.command().assert().success();
        assert_eq!(
            fs::read_to_string(project.path.join("assets/payload.txt")).unwrap(),
            "downloaded\n"
        );
        assert!(!project.path.join("assets/missing.txt").exists());
    }
}

#[test]
fn inactive_and_unused_remote_files_are_not_downloaded() {
    for (condition, outputs) in [
        ("false", "  output.test.Containerfile:\n    - module:\n"),
        ("true", "  output.test.Containerfile:\n    - FROM busybox\n"),
    ] {
        let project = Project::remote(
            &format!(
                "files:\n  - path: assets/payload.txt\n    when: '{condition}'\n  - path: assets/missing.txt\n    when: '{condition}'"
            ),
            "modules/tool.md",
            outputs,
        );
        project.command().assert().success();
        assert!(!project.path.join("assets").exists());
        assert!(
            !project
                .path
                .join("cache/containeryard/extracted_files/unknown/owner/repo/HEAD/modules/assets")
                .exists()
        );
    }
}

#[test]
fn remote_file_selection_uses_all_outputs_and_honors_do_not_refetch() {
    let project = Project::remote(
        "args:\n  optional: [enabled]\nfiles:\n  - path: assets/payload.txt\n    when: 'enabled is defined and enabled == true'",
        "modules/tool.md",
        "  disabled.test.Containerfile:\n    - module: {enabled: false}\n  enabled.test.Containerfile:\n    - module: {enabled: true}\n",
    );
    project.write("assets/payload.txt", "local edit\n");
    project
        .command()
        .arg("--do-not-refetch")
        .assert()
        .success()
        .stdout(predicate::str::contains("is not refetched"));
    assert_eq!(
        fs::read_to_string(project.path.join("assets/payload.txt")).unwrap(),
        "local edit\n"
    );
    project.command().assert().success();
    assert_eq!(
        fs::read_to_string(project.path.join("assets/payload.txt")).unwrap(),
        "downloaded\n"
    );
}
