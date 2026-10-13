use std::{
    fs,
    path::PathBuf,
    process::Command,
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
            "yard-scripts-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        let project = Self { path };
        let mut inputs = String::new();
        for (name, body) in modules {
            fs::write(project.path.join(format!("{name}.md")), body).unwrap();
            inputs.push_str(&format!("    {name}: {name}.md\n"));
        }
        fs::write(
            project.path.join("yard.yaml"),
            format!("inputs:\n  modules:\n{inputs}outputs:\n{outputs}"),
        )
        .unwrap();
        project
    }

    fn single(module: &str) -> Self {
        Self::new(
            &[("module", module)],
            "  output.test.Containerfile:\n    - module:\n",
        )
    }

    fn command(&self) -> assert_cmd::Command {
        let mut command = assert_cmd::Command::cargo_bin("yard").unwrap();
        command.current_dir(&self.path).arg("build");
        command
    }

    fn output(&self, name: &str) -> String {
        fs::read_to_string(self.path.join(name)).unwrap()
    }

    fn build(&self) -> String {
        self.command().assert().success();
        self.output("output.test.Containerfile")
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.path).unwrap();
    }
}

/// Execute exactly the shell command and heredoc emitted by the generator, without Docker.
fn execute_scripts(containerfile: &str) -> String {
    let mut lines = containerfile.lines();
    let mut stdout = String::new();
    while let Some(line) = lines.next() {
        let Some(command) = line.strip_prefix("RUN <<'") else {
            continue;
        };
        let delimiter = command.split('\'').next().unwrap();
        let mut command = format!("{}\n", line.strip_prefix("RUN ").unwrap());
        let mut closed = false;
        for line in lines.by_ref() {
            command.push_str(line);
            command.push('\n');
            if line == delimiter {
                closed = true;
                break;
            }
        }
        assert!(closed, "missing closing delimiter");
        let result = Command::new("sh").args(["-c", &command]).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        stdout.push_str(&String::from_utf8(result.stdout).unwrap());
    }
    stdout
}

#[test]
fn literal_scripts_preserve_shell_syntax_and_nested_heredocs() {
    let project = Project::single(
        r#"```bash name=setup
set -euo pipefail
items=(one 'two words')
value='inside'
printf '%s|%s|%s\n' "$value" "${items[1]}" 'a\b'
cat <<'INNER'
  literal $HOME `text` {{ untouched }} {% also_literal %}
# syntax=this-is-script-content
INNER
```
```containerfile name=main
FROM debian
RUN {{ script(name="setup") }}
```
"#,
    );
    let output = project.build();
    assert!(output.starts_with("# syntax=docker/dockerfile:1\n\n"));
    assert!(output.contains("RUN <<'YARD_SCRIPT_0' bash\n"));
    assert!(output.contains("# syntax=this-is-script-content"));
    assert_eq!(
        execute_scripts(&output),
        "inside|two words|a\\b\n  literal $HOME `text` {{ untouched }} {% also_literal %}\n# syntax=this-is-script-content\n"
    );
}

#[test]
fn scripts_strip_fence_indentation_and_normalize_crlf() {
    let module = "  ```sh name=setup\n  printf '%s\\n' 'hello'\n  ```\n  ```containerfile name=main\n  FROM busybox\n      RUN {{ script(name=\"setup\") }}   \n  ```\n".replace('\n', "\r\n");
    let project = Project::single(&module);
    let output = project.build();
    assert!(!output.contains('\r'));
    assert!(output.contains("RUN <<'YARD_SCRIPT_0' sh\nprintf '%s\\n' 'hello'\nYARD_SCRIPT_0\n"));
    assert_eq!(execute_scripts(&output), "hello\n");
}

#[test]
fn dedent_is_explicit_and_preserves_relative_and_trailing_whitespace() {
    let project = Project::single(
        r#"```sh name=setup
    cat <<'DATA'
      preserved spaces  
    DATA
```
```containerfile name=main
FROM busybox
RUN {{ script(name="setup", dedent=true) }}
RUN {{ script(name="setup") }}
```
"#,
    );
    let output = project.build();
    assert!(output.contains("sh\ncat <<'DATA'\n  preserved spaces  \nDATA\nYARD_SCRIPT_0"));
    assert!(
        output.contains("sh\n    cat <<'DATA'\n      preserved spaces  \n    DATA\nYARD_SCRIPT_0")
    );
    let first = output.split("RUN <<'").nth(1).unwrap();
    let first = format!("RUN <<'{}", first.split("RUN <<'").next().unwrap());
    assert_eq!(execute_scripts(&first), "  preserved spaces  \n");
}

#[test]
fn templated_scripts_quote_arguments_and_do_not_render_values_twice() {
    let value =
        "space 'quote' \"$HOME\" $(printf injected) \\ <>& {{ untouched }}\nYARD_SCRIPT_0\nlast";
    let module = r#"```yaml name=config
args:
  required: [message]
```
```bash name=setup
set -euo pipefail
printf '%s\n' {{ message | shell_quote }}
```
```containerfile name=main
FROM debian
RUN {{ script(name="setup", template=true) }}
```
"#;
    let project = Project::new(
        &[("module", module)],
        &format!(
            "  output.test.Containerfile:\n    - module:\n        message: {}\n",
            serde_json::to_string(value).unwrap()
        ),
    );
    let output = project.build();
    assert!(output.contains("RUN <<'YARD_SCRIPT_1' bash"));
    assert_eq!(execute_scripts(&output), format!("{value}\n"));
}

#[test]
fn delimiter_collisions_in_literal_scripts_are_avoided() {
    let project = Project::single(
        r#"```sh name=setup
cat <<'INNER'
YARD_SCRIPT_0
YARD_SCRIPT_1
INNER
```
```containerfile name=main
RUN {{ script(name="setup") }}
```
"#,
    );
    let output = project.build();
    assert!(output.contains("RUN <<'YARD_SCRIPT_2' sh"));
    assert_eq!(execute_scripts(&output), "YARD_SCRIPT_0\nYARD_SCRIPT_1\n");
}

#[test]
fn scripts_support_run_flags_and_conditional_references() {
    let module = r#"```yaml name=config
args:
  optional: [enabled]
```
```sh name=setup
echo installed
```
```containerfile name=main
FROM busybox
{% if enabled %}
RUN --mount=type=cache,target=/cache {{ script(name="setup") }}
{% endif %}
```
"#;
    let project = Project::new(
        &[("module", module)],
        "  enabled.test.Containerfile:\n    - module: {enabled: true}\n  disabled.test.Containerfile:\n    - module: {enabled: false}\n",
    );
    project.command().assert().success();
    assert!(
        project
            .output("enabled.test.Containerfile")
            .contains("RUN --mount=type=cache,target=/cache <<'YARD_SCRIPT_0' sh")
    );
    let disabled = project.output("disabled.test.Containerfile");
    assert!(!disabled.contains("installed"));
    assert!(!disabled.contains("# syntax="));
}

#[test]
fn scripts_are_local_to_each_module_and_available_in_all_named_fragments() {
    let split = r#"```yaml name=config
args:
  required: [message]
```
```sh name=setup
printf '%s\n' {{ message | shell_quote }}
```
```containerfile name=build
FROM busybox AS builder
RUN {{ script(name="setup", template=true) }}
```
```containerfile name=main
RUN {{ script(name="setup", template=true) }}
```
```containerfile name=finalize
RUN {{ script(name="setup", template=true) }}
```
"#;
    let regular = "```sh name=setup\necho other\n```\n```containerfile name=main\nFROM busybox\nRUN {{ script(name=\"setup\") }}\n```\n";
    let project = Project::new(
        &[("split", split), ("regular", regular)],
        "  output.test.Containerfile:\n    - regular:\n    - split: {message: first}\n  second.test.Containerfile:\n    - regular:\n    - split: {message: second}\n",
    );
    project.command().assert().success();
    let first = project.output("output.test.Containerfile");
    let second = project.output("second.test.Containerfile");
    assert_eq!(execute_scripts(&first), "first\nother\nfirst\nfirst\n");
    assert_eq!(execute_scripts(&second), "second\nother\nsecond\nsecond\n");
    assert!(first.starts_with("# syntax=docker/dockerfile:1\n\n"));
    assert_eq!(first.matches("ARG CACHE_BUST_SPLIT=1").count(), 3);
}

#[test]
fn markdown_documentation_is_ignored_and_longer_fences_preserve_embedded_fences() {
    let project = Project::single(
        r#"````markdown
```sh name=setup
echo documentation
```
````
```bash
echo unnamed-documentation
```
~~~~sh name=setup
cat <<'DOC'
```yaml name=config
this is literal script data
```
DOC
~~~~
```containerfile name=main
RUN {{ script(name="setup") }}
```
"#,
    );
    let output = project.build();
    assert!(!output.contains("documentation"));
    assert_eq!(
        execute_scripts(&output),
        "```yaml name=config\nthis is literal script data\n```\n"
    );
}

#[test]
fn invalid_script_declarations_are_rejected() {
    for (module, message) in [
        (
            "```sh name=setup\necho one\n```\n```bash name=setup\necho two\n```\n```containerfile name=main\nFROM busybox\n```",
            "Script 'setup' is declared more than once",
        ),
        (
            "```sh name=setup\necho unclosed",
            "Script 'setup' has an unclosed fenced block",
        ),
        ("```sh name=\n```", "A script name must contain only"),
        (
            "```sh name=bad/name\n```",
            "A script name must contain only",
        ),
        (
            "```sh name=setup extra=true\n```",
            "A named script fence must contain only",
        ),
        (
            "```sh name=setup\necho no-containerfile\n```",
            "at least one named Containerfile block",
        ),
    ] {
        let project = Project::single(module);
        project
            .command()
            .assert()
            .failure()
            .stderr(predicate::str::contains(message));
    }
}

#[test]
fn invalid_script_calls_and_positions_are_rejected() {
    for (call, message) in [
        (
            "RUN {{ script(name=\"missing\") }}",
            "Script 'missing' was not found",
        ),
        ("RUN {{ script() }}", "script() requires a string `name`"),
        (
            "RUN {{ script(name=\"setup\", template=\"true\") }}",
            "script() requires a string `name`",
        ),
        (
            "RUN {{ script(name=\"setup\", unknown=true) }}",
            "script() requires a string `name`",
        ),
        ("{{ script(name=\"setup\") }}", "Place script() after `RUN`"),
        (
            "RUN echo {{ script(name=\"setup\") }}",
            "Place script() after `RUN`",
        ),
        (
            "RUN {{ script(name=\"setup\") }} && echo after",
            "Place script() after `RUN`",
        ),
        (
            "RUN {{ script(name=\"setup\") }} {{ script(name=\"setup\") }}",
            "Place script() after `RUN`",
        ),
        (
            "COPY {{ script(name=\"setup\") }} /script.sh",
            "Place script() after `RUN`",
        ),
    ] {
        let project = Project::single(&format!(
            "```sh name=setup\necho ok\n```\n```containerfile name=main\n{call}\n```\n"
        ));
        project
            .command()
            .assert()
            .failure()
            .stderr(predicate::str::contains(message));
        assert!(!project.path.join("output.test.Containerfile").exists());
    }
}

#[test]
fn script_template_errors_identify_the_named_script() {
    for body in [
        "echo {{ missing }}",
        "{% if broken",
        "{{ script(name=\"setup\") }}",
    ] {
        let project = Project::single(&format!(
            "```sh name=setup\n{body}\n```\n```containerfile name=main\nRUN {{{{ script(name=\"setup\", template=true) }}}}\n```\n"
        ));
        project
            .command()
            .assert()
            .failure()
            .stderr(predicate::str::contains("Could not render script 'setup'"));
    }
}

#[test]
fn parser_directives_are_hoisted_deduplicated_and_preserved_with_cache_busting() {
    let first = "```containerfile name=main\n# syntax=docker/dockerfile:1.7\n# escape=\\\nFROM busybox\n```\n";
    let second = "```sh name=setup\necho works\n```\n```containerfile name=main\n# syntax=docker/dockerfile:1.7\n# check=error=true\nRUN {{ script(name=\"setup\") }}\n```\n";
    let project = Project::new(
        &[("first", first), ("second", second)],
        "  output.test.Containerfile:\n    - first:\n    - second:\n",
    );
    project.command().assert().success();
    let output = project.output("output.test.Containerfile");
    assert!(
        output.starts_with("# syntax=docker/dockerfile:1.7\n# escape=\\\n# check=error=true\n\n")
    );
    assert_eq!(output.matches("# syntax=").count(), 1);
    assert_eq!(execute_scripts(&output), "works\n");
}

#[test]
fn conflicting_directives_fail_only_when_combined_in_one_output() {
    let first = "```containerfile name=main\n# syntax=docker/dockerfile:1.7\nFROM busybox\n```\n";
    let second = "```containerfile name=main\n# syntax=docker/dockerfile:1.8\nFROM busybox\n```\n";
    let project = Project::new(
        &[("first", first), ("second", second)],
        "  output.test.Containerfile:\n    - first:\n    - second:\n",
    );
    project
        .command()
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Conflicting Dockerfile 'syntax' directives",
        ));
    let project = Project::new(
        &[("first", first), ("second", second)],
        "  first.test.Containerfile:\n    - first:\n  second.test.Containerfile:\n    - second:\n",
    );
    project.command().assert().success();
    assert!(
        project
            .output("first.test.Containerfile")
            .starts_with("# syntax=docker/dockerfile:1.7\n")
    );
    assert!(
        project
            .output("second.test.Containerfile")
            .starts_with("# syntax=docker/dockerfile:1.8\n")
    );
}
