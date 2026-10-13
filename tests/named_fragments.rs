use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use predicates::prelude::predicate;

struct Project {
    path: PathBuf,
}

impl Project {
    fn new(modules: &[(&str, &str)], outputs: &str) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "yard-named-fragments-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        let project = Self { path };
        let mut inputs = String::new();
        for (name, content) in modules {
            fs::write(project.path.join(format!("{name}.md")), content).unwrap();
            inputs.push_str(&format!("    {name}: {name}.md\n"));
        }
        fs::write(
            project.path.join("yard.yaml"),
            format!("inputs:\n  modules:\n{inputs}outputs:\n{outputs}"),
        )
        .unwrap();
        project
    }

    fn single(content: &str) -> Self {
        Self::new(
            &[("module", content)],
            "  output.test.Containerfile:\n    - module:\n",
        )
    }

    fn command(&self) -> assert_cmd::Command {
        let mut command = assert_cmd::Command::cargo_bin("yard").unwrap();
        command.current_dir(&self.path).arg("build");
        command
    }

    fn output(&self) -> String {
        fs::read_to_string(self.path.join("output.test.Containerfile")).unwrap()
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.path).unwrap();
    }
}

#[test]
fn labels_determine_placement_regardless_of_markdown_order() {
    let first = r#"```dockerfile name=finalize
RUN echo finalize-first
```
```containerfile name=main
RUN echo main-first
```
```dockerfile name=build
FROM busybox AS first-builder
```
"#;
    let second = r#"```containerfile name=main
RUN echo main-{{ message }}
```
```containerfile name=finalize
RUN echo finalize-{{ message }}
```
```yaml name=config
args:
  required: [message]
```
```containerfile name=build
FROM busybox AS {{ message }}-builder
```
"#;
    let project = Project::new(
        &[("first", first), ("second", second)],
        "  output.test.Containerfile:\n    - FROM busybox\n    - first:\n    - RUN echo middle\n    - second: {message: second}\n    - RUN echo after\n",
    );
    project.command().assert().success();
    let output = project.output();
    let mut end = 0;
    for expected in [
        "FROM busybox AS first-builder",
        "FROM busybox AS second-builder",
        "FROM busybox\n",
        "RUN echo main-first",
        "RUN echo middle",
        "RUN echo main-second",
        "RUN echo after",
        "RUN echo finalize-first",
        "RUN echo finalize-second",
    ] {
        end += output[end..]
            .find(expected)
            .unwrap_or_else(|| panic!("missing or out of order: {expected}\n{output}"))
            + expected.len();
    }
}

#[test]
fn each_fragment_works_independently() {
    for name in ["build", "main", "finalize"] {
        let project = Project::single(&format!(
            "```containerfile name={name}\nFROM busybox\nRUN echo {name}\n```\n"
        ));
        project.command().assert().success();
        let output = project.output();
        assert!(output.contains(&format!("({name} fragment)")));
        assert_eq!(output.matches("ARG CACHE_BUST_MODULE=1").count(), 1);
        assert!(output.contains(&format!("FROM busybox\nRUN echo {name}")));
    }
}

#[test]
fn unnamed_configuration_and_containerfile_fences_are_documentation() {
    let project = Project::single(
        r#"```yaml
this is deliberately invalid YAML: [
```
```containerfile
NOT_A_REAL_INSTRUCTION ignored
```
```dockerfile name=main
FROM busybox
RUN echo included
```
"#,
    );
    project.command().assert().success();
    let output = project.output();
    assert!(output.contains("RUN echo included"));
    assert!(!output.contains("deliberately"));
    assert!(!output.contains("NOT_A_REAL_INSTRUCTION"));
}

#[test]
fn duplicate_configuration_and_fragment_names_are_rejected() {
    for (content, message) in [
        (
            "```yaml name=config\ndescription: one\n```\n```yaml name=config\ndescription: two\n```\n```containerfile name=main\nFROM busybox\n```",
            "Configuration block 'config' is declared more than once",
        ),
        (
            "```dockerfile name=build\nFROM busybox\n```\n```containerfile name=build\nFROM busybox\n```",
            "Containerfile fragment 'build' is declared more than once",
        ),
        (
            "```dockerfile name=finalize\nRUN echo one\n```\n```containerfile name=finalize\nRUN echo two\n```",
            "Containerfile fragment 'finalize' is declared more than once",
        ),
    ] {
        let project = Project::single(content);
        project
            .command()
            .assert()
            .failure()
            .stderr(predicate::str::contains(message));
    }
}

#[test]
fn invalid_labels_and_obsolete_split_configuration_are_rejected() {
    for (content, message) in [
        (
            "```containerfile name=install\nRUN echo old\n```",
            "Unknown Containerfile fragment name 'install'",
        ),
        (
            "```dockerfile name=Main\nFROM busybox\n```",
            "Unknown Containerfile fragment name 'Main'",
        ),
        ("```yaml name=other\n```", "Unknown YAML block name 'other'"),
        (
            "```containerfile name=main extra=true\n```",
            "A named block fence must contain only",
        ),
        (
            "```yaml name=config\nsplit: true\n```\n```containerfile name=main\nFROM busybox\n```",
            "A module configuration does not match the expected schema",
        ),
        (
            "```containerfile name=finalize\nRUN echo unclosed",
            "Containerfile fragment 'finalize' has an unclosed fenced block",
        ),
    ] {
        let project = Project::single(content);
        project
            .command()
            .assert()
            .failure()
            .stderr(predicate::str::contains(message));
    }
}

#[test]
fn markdown_modules_must_have_a_named_containerfile_fragment() {
    for content in [
        "```yaml name=config\ndescription: no instructions\n```",
        "```yaml\ndescription: documentation\n```\n```containerfile\nFROM busybox\n```",
        "```bash\necho documentation\n```",
    ] {
        let project = Project::single(content);
        project
            .command()
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "at least one named Containerfile block",
            ));
        assert!(!project.path.join("output.test.Containerfile").exists());
    }
}

#[test]
fn plain_inputs_still_form_a_main_fragment() {
    let project = Project::single("FROM busybox\nRUN echo plain\n");
    project.command().assert().success();
    let output = project.output();
    assert!(output.contains("(main fragment)"));
    assert!(output.contains("FROM busybox\nRUN echo plain"));
}

#[test]
fn native_containerfiles_preserve_markdown_looking_heredoc_content() {
    let content = "FROM busybox\nRUN <<'EOF'\ncat <<'DOC'\n```yaml name=config\nfiles: [literal]\n```\nDOC\nEOF\n";
    for name in [
        "Containerfile",
        "input.Containerfile",
        "Dockerfile",
        "input.Dockerfile",
    ] {
        let project = Project::single(content);
        fs::rename(project.path.join("module.md"), project.path.join(name)).unwrap();
        let yard = fs::read_to_string(project.path.join("yard.yaml")).unwrap();
        fs::write(
            project.path.join("yard.yaml"),
            yard.replace("module.md", name),
        )
        .unwrap();
        project.command().assert().success();
        assert!(project.output().contains(content.trim()));
    }
}
