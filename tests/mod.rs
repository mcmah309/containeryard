#[cfg(test)]
pub mod test {
    use predicates::prelude::*;

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
    }

    #[test]
    fn module_file_no_config() {
        let assert = assert_cmd::Command::cargo_bin("yard")
            .unwrap()
            .current_dir("tests/module_file_no_config")
            .arg("build")
            .assert();
        assert.success();
    }

    #[test]
    fn output_order() {
        let assert = assert_cmd::Command::cargo_bin("yard")
            .unwrap()
            .current_dir("tests/output_order")
            .arg("output-order")
            .assert();
        assert
            .success()
            .stdout(predicate::eq("base.Containerfile\napp.Containerfile\nfinal.Containerfile\n"));
    }
}
