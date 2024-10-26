#[cfg(test)]
pub mod test {

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
}
