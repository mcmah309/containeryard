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
    fn new(requirements: &str, outputs: &str) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "yard-conditional-requires-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        let project = Self { path };
        fs::create_dir(project.path.join("modules")).unwrap();
        fs::write(project.path.join("base.md"), "RUN echo base\n").unwrap();
        fs::write(
            project.path.join("modules/consumer.md"),
            format!(
                "```yaml\nargs:\n  optional: [arg1, arg2, cuda, distro, workers]\nrequires:\n{requirements}\n```\n\n```containerfile\nRUN echo consumer\n```\n"
            ),
        )
        .unwrap();
        fs::write(
            project.path.join("yard.yaml"),
            format!(
                "inputs:\n  modules:\n    foundation: base.md\n    consumer: modules/consumer.md\noutputs:\n{outputs}"
            ),
        )
        .unwrap();
        project
    }

    fn conditional(expression: &str, outputs: &str) -> Self {
        Self::new(
            &format!(
                "  - path: ../base.md\n    when: {}",
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
fn conditional_requirements_check_presence_logic_and_typed_values() {
    // With no foundation in the output, exactly the active conditions must fail.
    for (expression, arguments, active) in [
        ("arg1 is defined and arg2 is defined", "{}", false),
        ("arg1 is defined and arg2 is defined", "{arg1: true}", false),
        (
            "arg1 is defined and arg2 is defined",
            "{arg1: false, arg2: ''}",
            true,
        ),
        ("arg1 is defined", "{arg1: false}", true),
        ("arg1 is defined", "{arg1: 0}", true),
        ("arg1 is defined", "{arg1: ''}", true),
        ("arg1 is defined or arg2 is defined", "{}", false),
        ("arg1 is defined or arg2 is defined", "{arg2: 0}", true),
        ("arg1 is not defined", "{}", true),
        ("arg1 is not defined", "{arg1: false}", false),
        ("not (arg1 is defined or arg2 is defined)", "{}", true),
        (
            "not (arg1 is defined or arg2 is defined)",
            "{arg2: false}",
            false,
        ),
        ("cuda is defined and cuda == true", "{}", false),
        ("cuda is defined and cuda == true", "{cuda: true}", true),
        ("cuda is defined and cuda == true", "{cuda: false}", false),
        ("cuda is defined and cuda == true", "{cuda: 'true'}", false),
        (
            "distro is defined and distro == 'alpine'",
            "{distro: alpine}",
            true,
        ),
        (
            "distro is defined and distro == 'alpine'",
            "{distro: debian}",
            false,
        ),
        ("workers is defined and workers >= 4", "{workers: 4}", true),
        ("workers is defined and workers >= 4", "{workers: 2}", false),
        // Identifiers inside string literals are not argument references.
        ("distro == 'undeclared_argument'", "{distro: alpine}", false),
    ] {
        let project = Project::conditional(
            expression,
            &format!("  output.test.Containerfile:\n    - consumer: {arguments}\n"),
        );
        let assert = project.command().assert();
        if active {
            assert.failure().stderr(predicate::str::contains(
                "Module 'consumer' requires module '../base.md'",
            ));
            assert!(!project.path.join("output.test.Containerfile").exists());
        } else {
            assert.success();
            let output =
                fs::read_to_string(project.path.join("output.test.Containerfile")).unwrap();
            assert!(output.contains("RUN echo consumer"));
            assert!(!output.contains("RUN echo base"));
        }
    }
}

#[test]
fn conditional_requirements_preserve_order_and_relative_path_matching() {
    let project = Project::conditional(
        "arg1 is defined",
        "  output.test.Containerfile:\n    - foundation:\n    - consumer: {arg1: true}\n",
    );
    project.command().assert().success();
    let output = fs::read_to_string(project.path.join("output.test.Containerfile")).unwrap();
    assert!(output.find("RUN echo base").unwrap() < output.find("RUN echo consumer").unwrap());

    let project = Project::conditional(
        "arg1 is defined",
        "  output.test.Containerfile:\n    - consumer: {arg1: true}\n    - foundation:\n",
    );
    project.command().assert().failure().stderr(
        predicate::str::contains("requires module '../base.md'").and(predicate::str::contains(
            "included before it in output 'output.test.Containerfile'",
        )),
    );
}

#[test]
fn conditional_requirements_are_evaluated_for_each_output() {
    let project = Project::conditional(
        "cuda is defined and cuda == true",
        "  disabled.test.Containerfile:\n    - consumer: {cuda: false}\n  enabled.test.Containerfile:\n    - foundation:\n    - consumer: {cuda: true}\n",
    );
    project.command().assert().success();
    let disabled = fs::read_to_string(project.path.join("disabled.test.Containerfile")).unwrap();
    let enabled = fs::read_to_string(project.path.join("enabled.test.Containerfile")).unwrap();
    assert!(!disabled.contains("RUN echo base"));
    assert!(enabled.contains("RUN echo base"));

    let project = Project::conditional(
        "cuda is defined and cuda == true",
        "  disabled.test.Containerfile:\n    - consumer: {cuda: false}\n  enabled.test.Containerfile:\n    - consumer: {cuda: true}\n",
    );
    project.command().assert().failure().stderr(
        predicate::str::contains("requires module '../base.md'").and(predicate::str::contains(
            "output 'enabled.test.Containerfile'",
        )),
    );
    assert!(!project.path.join("disabled.test.Containerfile").exists());
}

#[test]
fn conditional_requirements_use_resolved_environment_and_command_values() {
    for argument in ["$YARD_TEST_DISTRO", "$(printf alpine)"] {
        let project = Project::conditional(
            "distro is defined and distro == 'alpine'",
            &format!("  output.test.Containerfile:\n    - consumer: {{distro: '{argument}'}}\n"),
        );
        project
            .command()
            .env("YARD_TEST_DISTRO", "alpine")
            .assert()
            .failure()
            .stderr(predicate::str::contains("requires module '../base.md'"));
    }
}

#[test]
fn unconditional_and_conditional_requirements_can_be_mixed() {
    let project = Project::new(
        "  - path: ../base.md\n  - path: ../base.md\n    when: 'arg1 is defined'",
        "  output.test.Containerfile:\n    - foundation:\n    - consumer: {arg1: true}\n",
    );
    project.command().assert().success();

    let project = Project::new(
        "  - path: ../base.md\n    when: 'arg1 is defined'\n  - path: ../base.md",
        "  output.test.Containerfile:\n    - consumer:\n",
    );
    project
        .command()
        .assert()
        .failure()
        .stderr(predicate::str::contains("requires module '../base.md'"));
}

#[test]
fn conditional_requirements_reject_invalid_expressions_and_undeclared_arguments() {
    for (expression, message) in [
        (
            "arg1 is defined and",
            "A `requires.when` condition is invalid",
        ),
        (
            "",
            "A module configuration does not match the expected schema",
        ),
        (
            "arg1 is defined or typo is defined",
            "references undeclared argument 'typo'",
        ),
        (
            "arg1 == 'typo' or typo == true",
            "references undeclared argument 'typo'",
        ),
        (
            "false %}ignored{% if true",
            "must be a Tera expression without template tags",
        ),
    ] {
        let project = Project::conditional(
            expression,
            "  output.test.Containerfile:\n    - consumer: {arg1: true}\n",
        );
        project
            .command()
            .assert()
            .failure()
            .stderr(predicate::str::contains(message));
    }
}

#[test]
fn requirements_reject_string_entries_and_malformed_objects() {
    for requirement in [
        "  - ../base.md",
        "  - when: 'true'",
        "  - path: ../base.md\n    when: true",
        "  - path: ''\n    when: 'true'",
        "  - path: ../base.md\n    when: 'true'\n    extra: ignored",
    ] {
        let project = Project::new(
            requirement,
            "  output.test.Containerfile:\n    - consumer:\n",
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
fn conditional_requirements_report_evaluation_errors() {
    let project = Project::conditional(
        "workers / 0 > 1",
        "  output.test.Containerfile:\n    - consumer: {workers: 4}\n",
    );
    project
        .command()
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "A `requires.when` condition could not be evaluated",
        ));
}

#[test]
fn conditional_requirements_honor_ignore_flags_but_still_validate_configuration() {
    for flags in [
        vec!["--ignore-requires", "consumer"],
        vec!["--ignore-all-requires"],
    ] {
        let project = Project::conditional(
            "arg1 is defined",
            "  output.test.Containerfile:\n    - consumer: {arg1: true}\n",
        );
        project.command().args(&flags).assert().success();

        let project = Project::conditional(
            "typo is defined",
            "  output.test.Containerfile:\n    - consumer:\n",
        );
        project
            .command()
            .args(&flags)
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "references undeclared argument 'typo'",
            ));

        let project = Project::conditional(
            "arg1 is defined",
            "  output.test.Containerfile:\n    - consumer: {arg1: true}\n",
        );
        project
            .command()
            .args(["--ignore-requires", "foundation"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("requires module '../base.md'"));
    }
}

#[test]
fn conditional_requirements_validate_paths_even_when_inactive() {
    let project = Project::new(
        "  - path: /absolute/base.md\n    when: 'false'",
        "  output.test.Containerfile:\n    - consumer:\n",
    );
    project
        .command()
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Required module paths must be relative to the module file",
        ));
}
