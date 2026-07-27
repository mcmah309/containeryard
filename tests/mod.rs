use std::fs;

use predicates::prelude::predicate;

#[test]
fn conflicting_required_files() {
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
    let assert = assert_cmd::Command::cargo_bin("yard")
        .unwrap()
        .current_dir("tests/pure_containerfile")
        .arg("build")
        .assert();
    assert.success();
    let output = fs::read_to_string("tests/pure_containerfile/Containerfile").unwrap();
    assert!(output.contains("# Empty"));
}

#[test]
fn module_file_no_config() {
    let assert = assert_cmd::Command::cargo_bin("yard")
        .unwrap()
        .current_dir("tests/module_file_no_config")
        .arg("build")
        .assert();
    assert.success();
    let output = fs::read_to_string("tests/module_file_no_config/Containerfile").unwrap();
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
        "base.Containerfile\napp.Containerfile\nfinal.Containerfile\n",
    ));
}

#[test]
fn independent_modules() {
    let assert = assert_cmd::Command::cargo_bin("yard")
        .unwrap()
        .current_dir("tests/independent_modules")
        .arg("build")
        .assert();
    assert.success();
    let output = fs::read_to_string("tests/independent_modules/out.Containerfile").unwrap();

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
    let echo_idx = output.find("RUN echo hello").expect("echo should be present");
    assert!(
        echo_idx > install_stage_idx,
        "trailing inline module should come after the install stage"
    );
}

#[test]
fn independent_modules_cache_busting() {
    let assert = assert_cmd::Command::cargo_bin("yard")
        .unwrap()
        .current_dir("tests/independent_modules")
        .arg("build")
        .arg("--with-cache-busting")
        .assert();
    assert.success();
    let output = fs::read_to_string("tests/independent_modules/out.Containerfile").unwrap();
    // Cache busting ARGs are injected before both the build and install stages.
    assert!(output.contains("ARG CACHE_BUST_PYTHON_DEPS=1"));
}

#[test]
fn duplicate_module_rejected() {
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
