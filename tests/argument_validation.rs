use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use predicates::prelude::{PredicateBooleanExt, predicate};

struct Project {
    path: PathBuf,
}

impl Project {
    fn new(config: &str, outputs: &str) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "yard-argument-validation-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        let project = Self { path };
        fs::write(project.path.join("module.md"), format!(
            "```yaml name=config\n{config}\n```\n```containerfile name=main\nFROM busybox\nRUN echo validated\n```\n"
        )).unwrap();
        fs::write(
            project.path.join("yard.yaml"),
            format!("inputs:\n  modules:\n    module: module.md\noutputs:\n{outputs}"),
        )
        .unwrap();
        project
    }

    fn assertion(expression: &str, outputs: &str) -> Self {
        Self::new(
            &format!(
                "args:\n  optional: [username, password, workers, tls, insecure]\n  validate:\n    - assert: {}\n      message: 'Supply compatible arguments.'",
                serde_json::to_string(expression).unwrap()
            ),
            outputs,
        )
    }

    fn command(&self) -> assert_cmd::Command {
        let mut command = assert_cmd::Command::cargo_bin("yard").unwrap();
        command.current_dir(&self.path).arg("build");
        command
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.path).unwrap();
    }
}

#[test]
fn optional_arguments_can_be_required_together_by_presence() {
    for (arguments, valid) in [
        ("{}", true),
        ("{username: user}", false),
        ("{password: secret}", false),
        ("{username: user, password: secret}", true),
        ("{username: false, password: ''}", true),
        ("{username: 0, password: false}", true),
    ] {
        let project = Project::assertion(
            "(username is defined) == (password is defined)",
            &format!("  output.test.Containerfile:\n    - module: {arguments}\n"),
        );
        let assert = project.command().assert();
        if valid {
            assert.success();
            assert!(
                fs::read_to_string(project.path.join("output.test.Containerfile"))
                    .unwrap()
                    .contains("RUN echo validated")
            );
        } else {
            assert
                .failure()
                .stderr(predicate::str::contains("Supply compatible arguments."));
            assert!(!project.path.join("output.test.Containerfile").exists());
        }
    }
}

#[test]
fn assertions_support_directional_dependencies_exclusion_and_value_constraints() {
    for (expression, arguments, valid) in [
        ("username is not defined or password is defined", "{}", true),
        (
            "username is not defined or password is defined",
            "{password: secret}",
            true,
        ),
        (
            "username is not defined or password is defined",
            "{username: user}",
            false,
        ),
        (
            "not (tls is defined and insecure is defined)",
            "{tls: true}",
            true,
        ),
        (
            "not (tls is defined and insecure is defined)",
            "{tls: true, insecure: false}",
            false,
        ),
        ("workers is not defined or workers > 0", "{}", true),
        (
            "workers is not defined or workers > 0",
            "{workers: 4}",
            true,
        ),
        (
            "workers is not defined or workers > 0",
            "{workers: 0}",
            false,
        ),
        (
            "workers is not defined or workers > 0",
            "{workers: -1}",
            false,
        ),
    ] {
        let project = Project::assertion(
            expression,
            &format!("  output.test.Containerfile:\n    - module: {arguments}\n"),
        );
        let assert = project.command().assert();
        if valid {
            assert.success();
        } else {
            assert
                .failure()
                .stderr(predicate::str::contains("Supply compatible arguments."));
        }
    }
}

#[test]
fn every_assertion_must_pass_and_the_failed_message_is_reported() {
    let project = Project::new(
        "args:\n  optional: [username, password, workers]\n  validate:\n    - assert: '(username is defined) == (password is defined)'\n      message: 'Provide both credentials.'\n    - assert: 'workers > 0'\n      message: 'workers must be positive.'",
        "  output.test.Containerfile:\n    - module: {username: user, password: secret, workers: 0}\n",
    );
    project.command().assert().failure().stderr(
        predicate::str::contains("workers must be positive.")
            .and(predicate::str::contains("Provide both credentials.").not()),
    );
}

#[test]
fn argument_presence_and_acceptance_are_checked_before_assertions() {
    for (arguments, message) in [
        ("{}", "Required variable 'username' was not provided"),
        (
            "{username: user, typo: value}",
            "Template variable 'typo' is not accepted",
        ),
    ] {
        let project = Project::new(
            "args:\n  required: [username]\n  validate:\n    - assert: 'false'\n      message: 'Assertion should run later.'",
            &format!("  output.test.Containerfile:\n    - module: {arguments}\n"),
        );
        project.command().assert().failure().stderr(
            predicate::str::contains(message)
                .and(predicate::str::contains("Assertion should run later.").not()),
        );
    }
}

#[test]
fn assertions_run_before_requirements_and_cannot_be_ignored() {
    for flags in [
        vec![],
        vec!["--ignore-requires", "module"],
        vec!["--ignore-all-requires"],
    ] {
        let project = Project::new(
            "args:\n  validate:\n    - assert: 'false'\n      message: 'Arguments must be valid.'\nrequires:\n  - path: base.md",
            "  output.test.Containerfile:\n    - module:\n",
        );
        project.command().args(flags).assert().failure().stderr(
            predicate::str::contains("Arguments must be valid.")
                .and(predicate::str::contains("requires module").not()),
        );
    }
}

#[test]
fn assertions_are_evaluated_separately_for_each_output() {
    let project = Project::assertion(
        "(username is defined) == (password is defined)",
        "  first.test.Containerfile:\n    - module: {username: user, password: secret}\n  second.test.Containerfile:\n    - module: {username: user}\n",
    );
    project.command().assert().failure().stderr(
        predicate::str::contains("Supply compatible arguments.").and(predicate::str::contains(
            "output 'second.test.Containerfile'",
        )),
    );
    assert!(!project.path.join("first.test.Containerfile").exists());
    assert!(!project.path.join("second.test.Containerfile").exists());
}

#[test]
fn assertions_use_resolved_environment_and_shell_command_values() {
    for (argument, value, valid) in [
        ("$YARD_TEST_VALIDATION_VALUE", "ready", true),
        ("$YARD_TEST_VALIDATION_VALUE", "wrong", false),
        ("$(printf ready)", "unused", true),
    ] {
        let project = Project::assertion(
            "username == 'ready'",
            &format!("  output.test.Containerfile:\n    - module: {{username: '{argument}'}}\n"),
        );
        let assert = project
            .command()
            .env("YARD_TEST_VALIDATION_VALUE", value)
            .assert();
        if valid {
            assert.success();
        } else {
            assert
                .failure()
                .stderr(predicate::str::contains("Supply compatible arguments."));
        }
    }
}

#[test]
fn invalid_expressions_and_undeclared_references_are_configuration_errors() {
    for (expression, message) in [
        (
            "username is defined and",
            "A `args.validate.assert` condition is invalid",
        ),
        (
            "username is defined or typo is defined",
            "references undeclared argument 'typo'",
        ),
        (
            "false %}ignored{% if true",
            "must be a Tera expression without template tags",
        ),
    ] {
        let project = Project::assertion(
            expression,
            "  output.test.Containerfile:\n    - module: {username: user}\n",
        );
        project
            .command()
            .arg("--ignore-all-requires")
            .assert()
            .failure()
            .stderr(predicate::str::contains(message));
    }
}

#[test]
fn malformed_assertions_are_rejected_by_the_schema() {
    for entry in [
        "'true'",
        "{assert: 'true'}",
        "{message: 'Missing expression.'}",
        "{assert: true, message: 'Expression must be a string.'}",
        "{assert: '', message: 'Empty expression.'}",
        "{assert: 'true', message: ''}",
        "{assert: 'true', message: '   '}",
        "{assert: 'true', message: 'Unexpected key.', extra: true}",
    ] {
        let project = Project::new(
            &format!("args:\n  validate:\n    - {entry}"),
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
}

#[test]
fn evaluation_errors_are_distinct_from_false_assertions() {
    let project = Project::assertion(
        "workers / 0 > 1",
        "  output.test.Containerfile:\n    - module: {workers: 4}\n",
    );
    project.command().assert().failure().stderr(
        predicate::str::contains("A `args.validate.assert` condition could not be evaluated")
            .and(predicate::str::contains("Supply compatible arguments.").not()),
    );
}

#[test]
fn validation_only_arguments_and_empty_validation_lists_are_supported() {
    for config in [
        "args:\n  validate:\n    - assert: 'true'\n      message: 'Constant assertion.'",
        "args:\n  validate: []",
    ] {
        let project = Project::new(config, "  output.test.Containerfile:\n    - module:\n");
        project.command().assert().success();
    }
}
