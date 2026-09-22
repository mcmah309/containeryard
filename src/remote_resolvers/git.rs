use std::{collections::HashMap, path::PathBuf};

use eros::Context;
use regex::Regex;
use tokio::{fs, process::Command};
use tracing::trace;

use crate::{
    build::{ModuleData, RemoteModuleInfo, SourceInfoKind, read_module_file},
    user_error::user_error,
};

use super::{GitProvider, ModuleFileData, ReferenceInfo, path_in_cache_dir};

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
    pub fn new(url: String, commit: String) -> eros::Result<Self> {
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
    ) -> eros::Result<HashMap<String, ModuleFileData>> {
        let mut module_to_files: HashMap<String, ModuleFileData> = HashMap::new();
        for (name, module_path) in name_to_path.into_iter() {
            let module_path_cache = path_in_cache_dir(
                &PathBuf::from(&module_path),
                &self.provider,
                &self.repo_owner,
                &self.repo_name,
                &self.commit,
            )?;
            if !module_path_cache.exists() {
                trace!(
                    "Module `{}` not found in cache. Retrieving from remote...",
                    name
                );
                self.retrieve_file_and_put_at(&module_path, &module_path_cache)
                    .await?;
            }
            if !module_path_cache.exists() {
                return Err(eros::error!(
                    "Remote module was retrieved but is missing from cache at '{}'",
                    module_path_cache.display()
                )
                .user_context(
                    "A remote module could not be loaded after it was downloaded. Try clearing the Container Yard cache and run the command again.",
                ));
            }

            let module_data: ModuleData = read_module_file(&module_path_cache).await?;

            let source_info = SourceInfoKind::Remote(RemoteModuleInfo {
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
                    install_stage_data: module_data.install_stage,
                    source_info,
                },
            );
        }
        Ok(module_to_files)
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

    async fn extract_remote_path_data(&self, remote_path: &str) -> eros::Result<String> {
        // Ensure repo is downloaded
        let cache_dir = dirs::cache_dir().ok_or_else(|| {
            user_error(
                "Could not determine the system cache directory. Set a valid cache directory for this platform.",
            )
        })?;
        let provider_git_cache_dir = cache_dir
            .join("containeryard")
            .join("sources")
            .join("git_repos")
            .join(&self.provider)
            .join(&self.repo_owner);
        let repo_dir = provider_git_cache_dir.join(&self.repo_name);
        let mut will_clone = false;
        if repo_dir.is_dir() {
            if !repo_dir.join(".git").is_dir() {
                return Err(eros::error!(
                    "Cached directory for repo `{}` exists at `{}`, but it is not a git directory.",
                    self.url,
                    repo_dir.to_str().unwrap_or("")
                )
                .user_context(
                    "A cached remote is invalid. Clear the Container Yard cache and try again.",
                ));
            }
            trace!("Found a git cloned repo for `{}`", self.url,);
        } else {
            will_clone = true;
            fs::create_dir_all(&repo_dir)
                .await
                .with_context(|| format!("Create Git cache directory '{}'", repo_dir.display()))
                .user_context(
                    "Could not create the Git cache directory. Check cache directory permissions.",
                )?;
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
                    eros::error!(
                        "Failed to execute git command to clone {}:\n{}",
                        self.url,
                        e
                    )
                })
                .user_context(
                    "Could not run Git. Make sure Git is installed and available on PATH.",
                )?;
            if !clone_output.status.success() {
                return Err(eros::error!(
                    "Git failed with {}.\nCould not clone git repo `{}` to `{}`.\nstdout:\n{}\nstderr:\n{}",
                    &clone_output.status,
                    self.url,
                    provider_git_cache_dir.to_str().unwrap_or(""),
                    String::from_utf8_lossy(&clone_output.stdout),
                    String::from_utf8_lossy(&clone_output.stderr)
                )
                .user_context(
                    "Could not clone a remote repository. Check its URL, your network connection, and Git credentials.",
                ));
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
                    eros::error!(
                        "Failed to execute git command to pull the latest for {}:\n{}",
                        self.url,
                        e
                    )
                })
                .user_context(
                    "Could not run Git. Make sure Git is installed and available on PATH.",
                )?;
            if !fetch_output.status.success() {
                return Err(eros::error!(
                    "Git failed with {}.\nCould not pull git repo `{}` to `{}`.\nstdout:\n{}\nstderr:\n{}",
                    &fetch_output.status,
                    self.url,
                    provider_git_cache_dir.to_str().unwrap_or(""),
                    String::from_utf8_lossy(&fetch_output.stdout),
                    String::from_utf8_lossy(&fetch_output.stderr)
                )
                .user_context(
                    "Could not refresh a remote repository. Check your network connection and Git credentials.",
                ));
            }
        }

        // checkout commit
        trace!(
            "Checking out commit `{}` in repo `{}`",
            self.commit, self.url
        );
        let checkout_output = Command::new("git")
            .args(["checkout", &self.commit])
            .current_dir(&repo_dir)
            .output()
            .await
            .map_err(|e| {
                eros::error!(
                    "Failed to execute git command to checkout {}:\n{}",
                    self.url,
                    e
                )
            })
            .user_context("Could not run Git. Make sure Git is installed and available on PATH.")?;
        if !checkout_output.status.success() {
            return Err(eros::error!(
                "Git failed with {}.\nCould not checkout commit `{}` in git repo `{}`.\nstdout:\n{}\nstderr:\n{}",
                &checkout_output.status,
                self.commit,
                self.url,
                String::from_utf8_lossy(&checkout_output.stdout),
                String::from_utf8_lossy(&checkout_output.stderr)
            )
            .user_context(
                "Could not check out a remote commit. Check that the commit exists in the repository.",
            ));
        }

        // get file data
        let remote_file_path = repo_dir.join(remote_path);
        if !remote_file_path.is_file() {
            return Err(user_error(format!(
                "Remote file '{remote_path}' was not found at the configured commit. Check the path and commit in yard.yaml."
            ))
            .context(format!(
                "Remote file missing in repo '{}' at commit '{}'",
                self.url, self.commit
            )));
        }

        let file_data = fs::read_to_string(&remote_file_path)
            .await
            .map_err(|e| eros::error!(e))
            .with_context(|| format!("Could not read `{}`", remote_file_path.display()))?;

        Ok(file_data)
    }
}

struct RepoInfo {
    provider: String,
    owner: String,
    name: String,
}

fn url_to_repo_info(url: &str) -> eros::Result<RepoInfo> {
    let owner;
    let name;
    if url.starts_with("git@") {
        (owner, name) = extract_user_and_repo_from_ssh(url)?
    } else if url.starts_with("http") {
        (owner, name) = extract_user_and_repo_from_http(url)?;
    } else {
        return Err(user_error(
            "A remote has an unsupported URL. Use an HTTP(S) URL or an SSH URL beginning with `git@`.",
        )
        .context(format!("Unsupported remote URL: '{url}'")));
    }
    let provider = if url.contains("github.com") {
        "github".to_string()
    } else {
        "unknown".to_string()
    };
    Ok(RepoInfo {
        provider,
        owner,
        name,
    })
}

fn extract_user_and_repo_from_ssh(ssh_url: &str) -> eros::Result<(String, String)> {
    let re = Regex::new(r"^[\w-]+@[\w.-]+:([\w-]+)/([\w-]+)(?:\.git)?$").unwrap();
    re.captures(ssh_url)
        .and_then(|caps| {
            let user = caps.get(1).map(|m| m.as_str().to_string())?;
            let repo = caps.get(2).map(|m| m.as_str().to_string())?;
            Some((user, repo))
        })
        .ok_or_else(|| {
            user_error(
                "An SSH remote URL is invalid. Expected a value like `git@example.com:owner/repository.git`.",
            )
            .context(format!(
                "Could not extract owner and repository from SSH URL '{ssh_url}'"
            ))
        })
}

fn extract_user_and_repo_from_http(url: &str) -> eros::Result<(String, String)> {
    let re = Regex::new(r"^https?://[\w.-]+/([\w-]+)/([\w-]+)(?:\.git)?$").unwrap();
    re.captures(url)
        .and_then(|caps| {
            let user = caps.get(1).map(|m| m.as_str().to_string())?;
            let repo = caps.get(2).map(|m| m.as_str().to_string())?;
            Some((user, repo))
        })
        .ok_or_else(|| {
            user_error(
                "An HTTP remote URL is invalid. Expected a value like `https://example.com/owner/repository.git`.",
            )
            .context(format!(
                "Could not extract owner and repository from HTTP URL '{url}'"
            ))
        })
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
