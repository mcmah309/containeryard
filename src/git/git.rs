use std::{collections::HashMap, path::PathBuf};

use anyhow::{anyhow, bail, Context};
use regex::Regex;
use tokio::{fs, process::Command};
use tracing::trace;

use crate::build::{read_module_file, ModuleData, RemoteModuleInfo, SourceInfoKind};

use super::{path_in_cache_dir, GitProvider, ModuleFileData, ReferenceInfo};

/// Uses local `git` instance to clone and resolve references.
#[derive(Debug)]
pub struct Git {
    provider: String,
    repo_owner: String,
    repo_name: String,
    url: String,
    commit: String,
}

impl Git {
    pub fn new(url: String, commit: String) -> anyhow::Result<Self> {
        let RepoInfo {
            provider,
            owner,
            name,
        } = url_to_repo_info(&url)?;
        Ok(Git {
            provider,
            repo_owner: owner,
            repo_name: name,
            url,
            commit,
        })
    }
}

impl GitProvider for Git {
    async fn retrieve_module(
        &self,
        name_to_path: HashMap<String, String>,
    ) -> anyhow::Result<HashMap<String, ModuleFileData>> {
        let mut module_to_files: HashMap<String, ModuleFileData> = HashMap::new();
        for (name, module_path) in name_to_path.into_iter() {
            let module_path_cache = path_in_cache_dir(
                &PathBuf::from(&module_path),
                &self.provider,
                &self.repo_owner,
                &self.repo_name,
                &self.commit,
            );
            if !module_path_cache.exists() {
                trace!(
                    "Module `{}` not found in cache. Retrieving from remote...",
                    name
                );
                self.retrieve_file_and_put_at(&module_path, &module_path_cache)
                    .await?;
            }
            assert!(module_path_cache.exists());

            let module_data: ModuleData =
                read_module_file(&module_path_cache).await.context(format!(
                    "Could not read '{}' as a module.",
                    &module_path_cache.display()
                ))?;

            let source_info = SourceInfoKind::RemoteModuleInfo(RemoteModuleInfo {
                url: self.url.clone(),
                repo_owner: self.repo_owner.clone(),
                repo_name: self.repo_name.clone(),
                commit: self.commit.clone(),
                path: module_path.clone(),
                name: name.clone(),
            });
            module_to_files.insert(
                name,
                ModuleFileData {
                    containerfile_data: module_data.containerfile,
                    config_data: module_data.config,
                    source_info,
                },
            );
        }
        return Ok(module_to_files);
    }

    fn reference_info<'a>(&'a self) -> ReferenceInfo<'a> {
        ReferenceInfo {
            provider: self.provider.as_str(),
            repo_owner: self.repo_owner.as_str(),
            repo_name: self.repo_name.as_str(),
            url: self.url.as_str(),
            commit: self.commit.as_str(),
        }
    }

    async fn extract_remote_path_data(&self, remote_path: &str) -> anyhow::Result<String> {
        // Ensure repo is downloaded
        let provider_git_cache_dir = dirs::cache_dir()
            .expect("Could not determine cache directory of platform")
            .join("containeryard")
            .join("sources")
            .join("git_repos")
            .join(&self.provider)
            .join(&self.repo_owner);
        let repo_dir = provider_git_cache_dir.join(&self.repo_name);
        let mut will_clone = false;
        if repo_dir.is_dir() {
            if !repo_dir.join(".git").is_dir() {
                bail!(format!(
                    "Cached directory for repo `{}` exists at `{}`, but it is not a git directory.",
                    self.url,
                    repo_dir.to_str().unwrap_or("")
                ))
            }
            trace!("Found a git cloned repo for `{}`", self.url,);
        } else {
            will_clone = true;
            fs::create_dir_all(&repo_dir).await?;
        }

        if will_clone {
            trace!(
                "Cloning git repo `{}` to `{}`",
                self.url,
                provider_git_cache_dir.to_str().unwrap_or("")
            );
            let clone_output = Command::new("git")
                .args(["clone", &self.url])
                .current_dir(&provider_git_cache_dir)
                .output()
                .await
                .map_err(|e| {
                    anyhow!(
                        "Failed to execute git command to clone {}:\n{}",
                        self.url,
                        e
                    )
                })?;
            if !clone_output.status.success() {
                bail!(
                    "Git failed with {}.\nCould not clone git repo `{}` to `{}`.\nstdout:\n{}\nstderr:\n{}",
                    &clone_output.status,
                    self.url,
                    provider_git_cache_dir.to_str().unwrap_or(""),
                    String::from_utf8_lossy(&clone_output.stdout),
                    String::from_utf8_lossy(&clone_output.stderr)
                );
            }
        } else {
            trace!(
                "Pulling git repo `{}` to `{}`",
                self.url,
                provider_git_cache_dir.to_str().unwrap_or("")
            );
            let fetch_output = Command::new("git")
                .args(["fetch", "--all", "--prune"])
                .current_dir(&repo_dir)
                .output()
                .await
                .map_err(|e| {
                    anyhow!(
                        "Failed to execute git command to pull the latest for {}:\n{}",
                        self.url,
                        e
                    )
                })?;
            if !fetch_output.status.success() {
                bail!(
                    "Git failed with {}.\nCould not pull git repo `{}` to `{}`.\nstdout:\n{}\nstderr:\n{}",
                    &fetch_output.status,
                    self.url,
                    provider_git_cache_dir.to_str().unwrap_or(""),
                    String::from_utf8_lossy(&fetch_output.stdout),
                    String::from_utf8_lossy(&fetch_output.stderr)
                );
            }
        }

        // checkout commit
        trace!(
            "Checking out commit `{}` in repo `{}`",
            self.commit,
            self.url
        );
        let checkout_output = Command::new("git")
            .args(["checkout", &self.commit])
            .current_dir(&repo_dir)
            .output()
            .await
            .map_err(|e| {
                anyhow!(
                    "Failed to execute git command to checkout {}:\n{}",
                    self.url,
                    e
                )
            })?;
        if !checkout_output.status.success() {
            bail!(
                "Git failed with {}.\nCould not checkout commit `{}` in git repo `{}`.\nstdout:\n{}\nstderr:\n{}",
                &checkout_output.status,
                self.commit,
                self.url,
                String::from_utf8_lossy(&checkout_output.stdout),
                String::from_utf8_lossy(&checkout_output.stderr)
            );
        }

        // get file data
        let remote_file_path = repo_dir.join(&remote_path);
        if !remote_file_path.is_file() {
            bail!(format!(
                "Could not find file at remote path `{}` in repo `{}` at commit `{}`",
                &remote_path, &self.url, &self.commit
            ))
        }

        let file_data = fs::read_to_string(&remote_file_path)
            .await
            .map_err(|e| anyhow::Error::from(e))
            .with_context(|| format!("Could not read `{}`", &remote_file_path.display()))?;

        Ok(file_data)
    }
}

struct RepoInfo {
    provider: String,
    owner: String,
    name: String,
}

fn url_to_repo_info(url: &str) -> anyhow::Result<RepoInfo> {
    let owner;
    let name;
    if url.starts_with("git@") {
        (owner, name) = extract_user_and_repo_from_ssh(url)?
    } else if url.starts_with("http") {
        (owner, name) = extract_user_and_repo_from_http(url)?;
    } else {
        bail!(format!(
            "Unknown url type for `{url}`. Expected to start with `git@` or `http`"
        ))
    }
    let provider;
    if url.contains("github.com") {
        provider = "github".to_string();
    } else {
        provider = "unknown".to_string();
    }
    Ok(RepoInfo {
        provider,
        owner,
        name,
    })
}

fn extract_user_and_repo_from_ssh(ssh_url: &str) -> anyhow::Result<(String, String)> {
    let re = Regex::new(r"^[\w-]+@[\w.-]+:([\w-]+)/([\w-]+)(?:\.git)?$").unwrap();
    re.captures(ssh_url)
        .and_then(|caps| {
            let user = caps.get(1).map(|m| m.as_str().to_string())?;
            let repo = caps.get(2).map(|m| m.as_str().to_string())?;
            Some((user, repo))
        })
        .ok_or(anyhow!(format!(
            "Could not extract user and repo from ssh url `{}`",
            ssh_url
        )))
}

fn extract_user_and_repo_from_http(url: &str) -> anyhow::Result<(String, String)> {
    let re = Regex::new(r"^https?://[\w.-]+/([\w-]+)/([\w-]+)(?:\.git)?$").unwrap();
    re.captures(url)
        .and_then(|caps| {
            let user = caps.get(1).map(|m| m.as_str().to_string())?;
            let repo = caps.get(2).map(|m| m.as_str().to_string())?;
            Some((user, repo))
        })
        .ok_or(anyhow!(format!(
            "Could not extract user and repo from url `{}`",
            url
        )))
}

// /// characters not allowed in dirs on windows and linux
// fn replace_disallowed_dir_name_symbols(string: &str) -> String {
//     return string
//         .replace("/", "_fslash_")
//         .replace("\\", "_bslash_")
//         .replace(":", "_colon_")
//         .replace("*", "_star_")
//         .replace("?", "_qmark_")
//         .replace("\"", "_quote_")
//         .replace("<", "_lt_")
//         .replace(">", "_gt_")
//         .replace("|", "_pipe_")
//         .replace("&", "_amp_")
//         .replace(" ", "_space_");
// }
