use std::{
    fs, io,
    path::{Path, PathBuf},
};

use predicates::prelude::{PredicateBooleanExt, predicate};

const TEST_OUTPUT_FILE_NAME: &str = "output.test.Containerfile";

struct TestOutputFile {
    path: PathBuf,
}

impl TestOutputFile {
    fn new(test_dir: &str) -> Self {
        let output = Self {
            path: Path::new(test_dir).join(TEST_OUTPUT_FILE_NAME),
        };
        output
            .remove()
            .unwrap_or_else(|error| panic!("failed to remove stale test output: {error}"));
        output
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn remove(&self) -> io::Result<()> {
        match fs::remove_file(&self.path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        }
    }
}

impl Drop for TestOutputFile {
    fn drop(&mut self) {
        if let Err(error) = self.remove()
            && !std::thread::panicking()
        {
            panic!("failed to clean up '{}': {error}", self.path.display());
        }
    }
}

#[test]
fn conflicting_required_files() {
    let _output = TestOutputFile::new("tests/conflicting_required_files");
    let assert = assert_cmd::Command::cargo_bin("yard")
        .unwrap()
        .current_dir("tests/conflicting_required_files")
        .arg("build")
        .assert();
    assert.failure();

    // check the only file that exists is yard.yaml
    for entry in std::fs::read_dir("tests/conflicting_required_files").unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_file() {
            assert!(path.ends_with("yard.yaml"));
        }
    }
}

#[test]
fn pure_containerfile() {
    let output_file = TestOutputFile::new("tests/pure_containerfile");
    let assert = assert_cmd::Command::cargo_bin("yard")
        .unwrap()
        .current_dir("tests/pure_containerfile")
        .arg("build")
        .assert();
    assert.success();
    let output = fs::read_to_string(output_file.path()).unwrap();
    assert!(output.contains("# Empty"));
}

#[test]
fn module_file_no_config() {
    let output_file = TestOutputFile::new("tests/module_file_no_config");
    let assert = assert_cmd::Command::cargo_bin("yard")
        .unwrap()
        .current_dir("tests/module_file_no_config")
        .arg("build")
        .assert();
    assert.success();
    let output = fs::read_to_string(output_file.path()).unwrap();
    assert!(output.contains("# Empty"));
}

#[test]
fn output_order() {
    let assert = assert_cmd::Command::cargo_bin("yard")
        .unwrap()
        .current_dir("tests/output_order")
        .arg("outputs")
        .assert();
    assert.success().stdout(predicate::eq(
        "base.test.Containerfile\napp.test.Containerfile\nfinal.test.Containerfile\n",
    ));
}

#[test]
fn normal_errors_show_user_context_without_developer_details() {
    assert_cmd::Command::cargo_bin("yard")
        .unwrap()
        .env_remove("CONTAINERYARD_DEBUG")
        .args(["outputs", "tests/does-not-exist"])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains(
                "Could not list the configured outputs. Check that yard.yaml exists and is valid.",
            )
            .and(predicate::str::contains("For developer diagnostics"))
            .and(predicate::str::contains("Developer diagnostics:").not())
            .and(predicate::str::contains("Run `yard outputs`").not()),
        );
}

#[test]
fn debug_errors_include_developer_context() {
    assert_cmd::Command::cargo_bin("yard")
        .unwrap()
        .env("CONTAINERYARD_DEBUG", "1")
        .args(["outputs", "tests/does-not-exist"])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("Developer diagnostics:")
                .and(predicate::str::contains("Run `yard outputs`"))
                .and(predicate::str::contains("For developer diagnostics").not()),
        );
}

#[test]
fn independent_modules() {
    let output_file = TestOutputFile::new("tests/independent_modules");
    let assert = assert_cmd::Command::cargo_bin("yard")
        .unwrap()
        .current_dir("tests/independent_modules")
        .arg("build")
        .assert();
    assert.success();
    let output = fs::read_to_string(output_file.path()).unwrap();

    // The build stage must be hoisted to the start of the generated Containerfile.
    let build_stage_idx = output
        .find("FROM python:3.11-slim AS builder")
        .expect("build stage should be present");
    let inline_from_idx = output
        .find("FROM python:3.11-slim\n")
        .expect("inline FROM should be present");
    assert!(
        build_stage_idx < inline_from_idx,
        "independent build stage should be hoisted before the inline FROM"
    );

    // The build stage content.
    assert!(output.contains("RUN python -m venv /opt/venv"));
    assert!(output.contains("RUN pip install --no-cache-dir numpy pandas scipy"));

    // The install stage is injected where the module is declared, after the inline FROM.
    let install_stage_idx = output
        .find("COPY --from=builder /opt/venv /opt/venv")
        .expect("install stage should be present");
    assert!(
        install_stage_idx > inline_from_idx,
        "install stage should be injected after the inline FROM"
    );

    // The trailing inline module is preserved after the install stage.
    let echo_idx = output
        .find("RUN echo hello")
        .expect("echo should be present");
    assert!(
        echo_idx > install_stage_idx,
        "trailing inline module should come after the install stage"
    );

    let assert = assert_cmd::Command::cargo_bin("yard")
        .unwrap()
        .current_dir("tests/independent_modules")
        .arg("build")
        .arg("--with-cache-busting")
        .assert();
    assert.success();
    let output = fs::read_to_string(output_file.path()).unwrap();
    // Cache busting ARGs are injected before both the build and install stages.
    assert!(output.contains("ARG CACHE_BUST_PYTHON_DEPS=1"));
}

#[test]
fn duplicate_module_rejected() {
    let _output = TestOutputFile::new("tests/duplicate_module");
    let assert = assert_cmd::Command::cargo_bin("yard")
        .unwrap()
        .current_dir("tests/duplicate_module")
        .arg("build")
        .assert();
    assert.failure();
    // No Containerfile should be produced on failure.
    for entry in std::fs::read_dir("tests/duplicate_module").unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_file() {
            assert!(
                path.ends_with("yard.yaml") || path.ends_with("module.md"),
                "unexpected file produced: {}",
                path.display()
            );
        }
    }
}

#[test]
fn module_requires_accepts_dependency_in_an_earlier_position() {
    let output_file = TestOutputFile::new("tests/module_requires_success");
    assert_cmd::Command::cargo_bin("yard")
        .unwrap()
        .current_dir("tests/module_requires_success")
        .arg("build")
        .assert()
        .success();

    let output = fs::read_to_string(output_file.path()).unwrap();
    let base_idx = output.find("RUN echo base").unwrap();
    let consumer_idx = output.find("RUN echo consumer").unwrap();
    assert!(base_idx < consumer_idx);
}

#[test]
fn module_requires_rejects_dependency_in_a_later_position() {
    let _output = TestOutputFile::new("tests/module_requires_wrong_order");
    assert_cmd::Command::cargo_bin("yard")
        .unwrap()
        .current_dir("tests/module_requires_wrong_order")
        .arg("build")
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("requires module '../base.md'").and(predicate::str::contains(
                "included before it in output 'output.test.Containerfile'",
            )),
        );
}

#[test]
fn module_requires_rejects_dependency_missing_from_output() {
    let _output = TestOutputFile::new("tests/module_requires_missing");
    assert_cmd::Command::cargo_bin("yard")
        .unwrap()
        .current_dir("tests/module_requires_missing")
        .arg("build")
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("requires module '../base.md'").and(predicate::str::contains(
                "included before it in output 'output.test.Containerfile'",
            )),
        );
}

#[test]
fn module_requires_can_be_ignored_for_named_modules() {
    let output_file = TestOutputFile::new("tests/module_requires_ignored_named");
    assert_cmd::Command::cargo_bin("yard")
        .unwrap()
        .current_dir("tests/module_requires_ignored_named")
        .args(["build", "--ignore-requires", "consumer,foundation"])
        .assert()
        .success();

    let output = fs::read_to_string(output_file.path()).unwrap();
    let consumer_idx = output.find("RUN echo consumer").unwrap();
    let base_idx = output.find("RUN echo base").unwrap();
    assert!(consumer_idx < base_idx);
}

#[test]
fn module_requires_named_ignore_is_selective() {
    let _output = TestOutputFile::new("tests/module_requires_wrong_order");
    assert_cmd::Command::cargo_bin("yard")
        .unwrap()
        .current_dir("tests/module_requires_wrong_order")
        .args(["build", "--ignore-requires", "foundation"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Module 'consumer' requires module '../base.md'",
        ));
}

#[test]
fn module_requires_can_be_ignored_for_all_modules() {
    let output_file = TestOutputFile::new("tests/module_requires_ignored_all");
    assert_cmd::Command::cargo_bin("yard")
        .unwrap()
        .current_dir("tests/module_requires_ignored_all")
        .args(["build", "--ignore-all-requires"])
        .assert()
        .success();

    let output = fs::read_to_string(output_file.path()).unwrap();
    assert!(output.contains("RUN echo consumer"));
    assert!(!output.contains("RUN echo base"));
}

#[test]
fn boolean_and_number_args() {
    let output_file = TestOutputFile::new("tests/boolean_args");
    assert_cmd::Command::cargo_bin("yard")
        .unwrap()
        .current_dir("tests/boolean_args")
        .arg("build")
        .assert()
        .success();

    let output = fs::read_to_string(output_file.path()).unwrap();
    assert!(output.contains("RUN echo true false unchanged"));
    assert!(output.contains("RUN echo 4 -2 2"));
    assert!(output.contains("RUN echo enabled"));
    assert!(output.contains("RUN echo disabled"));
    assert!(!output.contains("unexpected"));
}
