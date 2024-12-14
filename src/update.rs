use std::fs::File;
use std::io::BufRead;
use std::path::Path;
use std::process::Command;
use std::{io, str};

use anyhow::{anyhow, bail, Context};

use crate::build::YARD_YAML_FILE_NAME;

/// Updates the `yard.yaml` file's "commit: <sha>" for each entry in the remote. Does not modify any other parts of the file
/// Even saves comments if they exist on the comment line e.g. "commit: <sha> comment"
pub fn update(path: &Path) -> anyhow::Result<()> {
    let yard_file = path.join(YARD_YAML_FILE_NAME);
    let input_file = File::open(&yard_file)?;
    let reader = io::BufReader::new(input_file);

    let mut lines: Vec<String> = Vec::new();
    let commit_capture_regex = regex::Regex::new(r"^(\s*commit:\s*)([0-9a-f]+)(\s*.*)$")?;
    let url_capture_regex = regex::Regex::new(r"\s*url:\s*(.*)")?;

    let mut latest_commit = String::new();
    let mut commit_line: usize = usize::MAX;
    let mut prefix = String::new();
    let mut suffix = String::new();
    for (line_number, line) in reader.lines().enumerate() {
        let line = line?;
        let trimmed = line.trim();
        if !trimmed.starts_with("#") {
            // Check if the line contains a repository URL
            if let Some(captures) = url_capture_regex.captures(&line) {
                if !latest_commit.is_empty() {
                    bail!(
                        "Found two url's before any commits. At line number '{}'",
                        line_number
                    );
                }
                let current_repo_url = captures.get(1).map_or("", |m| m.as_str()).to_string();
                latest_commit = get_latest_commit_sha(&current_repo_url)
                    .with_context(|| format!("Line number '{}'", line_number))?
            }

            // Check if the line matches the commit pattern
            if let Some(captures) = commit_capture_regex.captures(&line) {
                assert!(captures.len() == 4);
                if commit_line != usize::MAX {
                    bail!(
                        "Found two commits before any url's. At line number '{}'",
                        line_number
                    );
                }
                assert!(prefix.is_empty() && suffix.is_empty());
                commit_line = line_number;
                prefix = captures.get(1).unwrap().as_str().to_string();
                suffix = captures.get(3).unwrap().as_str().to_string();
            }
        }

        lines.push(line);

        if !latest_commit.is_empty() && commit_line != usize::MAX {
            let new_line = format!("{}{}{}", &prefix, &latest_commit, &suffix);
            lines[commit_line] = new_line;
            commit_line = usize::MAX;
            latest_commit.clear();
            prefix.clear();
            suffix.clear();
        }
    }

    std::fs::write(&yard_file, lines.join("\n"))?;

    Ok(())
}

fn get_latest_commit_sha(repo_url: &str) -> anyhow::Result<String> {
    let output = Command::new("git")
        .arg("ls-remote")
        .arg("--symref")
        .arg(repo_url)
        .arg("HEAD")
        .output()
        .map_err(|e| anyhow!("Failed to execute git command to retrieve latest commit: {}", e))?;

    if !output.status.success() {
        bail!(
            "Git command to retrieve latest commit failed with {}\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let output_str = str::from_utf8(&output.stdout)?;

    let mut lines = output_str
        .lines()
        .map(|e| e.parse())
        .collect::<Result<Vec<String>, _>>()?;
    if lines.len() != 2 || !lines[1].contains("HEAD") {
        bail!(
            "Unexpected command output for retrieving the latest commit - `{:?}`",
            lines
        );
    }
    let head_line = lines.remove(1);
    let mut parts = head_line.split_whitespace().collect::<Vec<&str>>();
    if parts.len() != 2 || !parts[1].contains("HEAD") {
        bail!(
            "Unexpected command output for retrieving the latest commit - `{:?}`",
            lines
        );
    }
    let sha = parts.remove(0);

    Ok(sha.to_string())
}
