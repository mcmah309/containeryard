use std::{collections::HashMap, path::PathBuf, process::Stdio};

use anyhow::{anyhow, bail, Context};
use regex::Regex;
use tokio::{fs, process::Command};
use tracing::trace;

use crate::{
    build::{RemoteModuleInfo, SourceInfoKind, CONTAINERFILE_NAME, MODULE_YAML_FILE_NAME},
    common::is_debug,
};

use super::{path_in_cache_dir, GitProvider, ModuleFilesData, ReferenceInfo};

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
    ) -> anyhow::Result<HashMap<String, ModuleFilesData>> {
        let mut module_to_files: HashMap<String, ModuleFilesData> = HashMap::new();
        for (name, module_path) in name_to_path.into_iter() {
            let module_cache_dir = path_in_cache_dir(
                &PathBuf::from(&module_path),
                &self.provider,
                &self.repo_owner,
                &self.repo_name,
                &self.commit,
            );
            let cache_module_file = module_cache_dir.join(MODULE_YAML_FILE_NAME);
            let cache_containerfile_file = module_cache_dir.join(CONTAINERFILE_NAME);
            let files = [
                (&cache_module_file, MODULE_YAML_FILE_NAME),
                (&cache_containerfile_file, CONTAINERFILE_NAME),
            ];
            for (cache_path, file_name) in files {
                if !cache_path.exists() {
                    trace!(
                        "Path not found in cache. Retrieving file `{}` from remote for module `{}`",
                        file_name,
                        name
                    );
                    let remote_path = format!("{module_path}/{file_name}");
                    self.retrieve_file_and_put_at(&remote_path, cache_path)
                        .await?;
                }
                assert!(cache_path.exists());
            }

            let containerfile_data: String = fs::read_to_string(&cache_containerfile_file)
                .await
                .context(format!(
                    "Could not read '{}' to string.",
                    &cache_containerfile_file.display()
                ))?
                .into();
            let module_file_data: String = fs::read_to_string(&cache_module_file)
                .await
                .context(format!(
                    "Could not read '{}' to string.",
                    &cache_module_file.display()
                ))?
                .into();

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
                ModuleFilesData {
                    containerfile_data,
                    module_file_data,
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
            let clone_command_exit = setup_output(Command::new("git"))
                .args(["clone", &self.url])
                .current_dir(&provider_git_cache_dir)
                .spawn()?
                .wait()
                .await;
            if !clone_command_exit?.success() {
                bail!(format!(
                    "Could not clone git repo `{}` to `{}`",
                    self.url,
                    provider_git_cache_dir.to_str().unwrap_or("")
                ))
            }
        } else {
            trace!(
                "Pulling git repo `{}` to `{}`",
                self.url,
                provider_git_cache_dir.to_str().unwrap_or("")
            );
            let fetch_command_exit = setup_output(Command::new("git"))
                .args(["fetch", "--all", "--prune"])
                .current_dir(&repo_dir)
                .spawn()?
                .wait()
                .await;
            if !fetch_command_exit?.success() {
                bail!(format!(
                    "Could not pull git repo `{}` to `{}`",
                    self.url,
                    repo_dir.to_str().unwrap_or("")
                ))
            }
        }

        // checkout commit
        trace!(
            "Checking out commit `{}` in repo `{}`",
            self.commit,
            self.url
        );
        let checkout_command_exit = setup_output(Command::new("git"))
            .args(["checkout", &self.commit])
            .current_dir(&repo_dir)
            .spawn()?
            .wait()
            .await;
        if !checkout_command_exit?.success() {
            bail!(format!(
                "Could not checkout commit `{}` in repo `{}`",
                self.commit, self.url
            ))
        }

        // get file data
        let remote_file_path = repo_dir.join(&remote_path);
        if !remote_file_path.is_file() {
            bail!(format!(
                "Could not find file at remote path `{}` in repo `{}` at commit `{}`",
                &remote_path, &self.url, &self.commit
            ))
        }

        let file_data = fs::read_to_string(remote_file_path)
            .await
            .map_err(|e| anyhow::Error::from(e))
            .context("wumbo")?;

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

//************************************************************************//

fn setup_output(mut command: Command) -> Command {
    // inherits by default
    // command.stderr(Stdio::inherit());
    if is_debug() {
        // command.stdout(Stdio::inherit());
    } else {
        command.stdout(Stdio::null());
    }
    command
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
