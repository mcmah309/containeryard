use std::{collections::HashMap, path::PathBuf, sync::LazyLock};

use anyhow::{bail, Context};
use regex::Regex;
use reqwest::Client;
use tokio::fs;
use tracing::debug;

use crate::{
    build::{
        IntermediateRemote, RemoteModuleInfo, SourceInfoKind, CONTAINERFILE_NAME,
        MODULE_YAML_FILE_NAME,
    },
    common::UserMessageError,
};

use super::{GitProvider, ModuleFiles, ModuleLocationInRemote};

pub struct Github {
    web_request_client: Client,
}

impl Github {
    pub fn new() -> Self {
        Github {
            web_request_client: get_client(),
        }
    }
}

fn get_client() -> Client {
    static CLIENT: LazyLock<Client> = LazyLock::new(|| Client::new());
    CLIENT.clone()
}

impl GitProvider for Github {
    async fn get_module_files(
        &self,
        remote: &IntermediateRemote,
    ) -> anyhow::Result<HashMap<String, ModuleFiles>> {
        let mut module_infos: Vec<ModuleLocationInRemote> = Vec::new();
        let (owner, repo) = extract_github_info(&remote.url)?;
        for (name, path) in remote.name_to_path.iter() {
            module_infos.push(ModuleLocationInRemote {
                owner: owner.clone(),
                repo: repo.clone(),
                commit: remote.commit.clone(),
                path: path.clone(),
                name: name.clone(),
            })
        }

        let mut module_to_files: HashMap<String, ModuleFiles> = HashMap::new();
        for module_info in module_infos {
            let ModuleLocationInRemote {
                owner,
                repo,
                commit,
                path,
                name,
            } = module_info;

            let cache_dir = dirs::cache_dir()
                .expect("Could not determine cache directory of platform")
                .join(&owner)
                .join(&repo)
                .join(&commit)
                .join(&path);
            let module_file = cache_dir.join(MODULE_YAML_FILE_NAME);
            let containerfile_file = cache_dir.join(CONTAINERFILE_NAME);
            let files = [&module_file, &containerfile_file];
            for file in files {
                if file.exists() {
                    continue;
                }
                debug!("Did not find '{}' in cache.", module_file.display());
                let file_data =
                    get_github_file(&self.web_request_client, &owner, &repo, &commit, &path)
                        .await
                        .context("Failed to get file from github.")?;
                let parent = file.parent().expect("Could not get parent directory.");
                if !parent.exists() {
                    fs::create_dir_all(parent).await.with_context(|| {
                        format!("Could not create '{}' directory.", parent.display())
                    })?;
                }
                fs::write(file, file_data.as_bytes())
                    .await
                    .with_context(|| format!("Could not write '{}' to cache.", file.display()))?;
            }

            let containerfile: PathBuf = fs::read_to_string(&containerfile_file)
                .await
                .context(format!(
                    "Could not read '{}' to string.",
                    &containerfile_file.display()
                ))?
                .into();
            let module_file: PathBuf = fs::read_to_string(&module_file)
                .await
                .context(format!(
                    "Could not read '{}' to string.",
                    &module_file.display()
                ))?
                .into();

            let source_info = SourceInfoKind::RemoteModuleInfo(RemoteModuleInfo {
                url: remote.url.clone(),
                commit: commit,
                path: path,
                name: name.clone(),
            });
            module_to_files.insert(
                name,
                ModuleFiles {
                    containerfile,
                    module_file,
                    source_info,
                },
            );
        }
        return Ok(module_to_files);
    }
}

async fn get_github_file(
    client: &Client,
    owner: &str,
    repo: &str,
    commit: &str,
    path: &str,
) -> anyhow::Result<String> {
    let url = format!(
        "https://api.github.com/repos/{}/{}/contents/{}?ref={}",
        owner, repo, path, commit
    );

    let response = client
        .get(&url)
        .header("Accept", "application/vnd.github.v3.raw")
        .header("User-Agent", "rust-reqwest-client")
        .send()
        .await
        .with_context(|| {
            format!(
                "Could not get file from github. Request for '{}' failed.",
                &url
            )
        })?;

    if !response.status().is_success() {
        bail!("Failed to fetch file metadata for '{}'.", url);
    }

    let text = response
        .text()
        .await
        .with_context(|| format!("Could not read response from '{}'.", &url))?;
    Ok(text)
}

fn extract_github_info(url: &str) -> anyhow::Result<(String, String)> {
    (|| {
        let re = Regex::new(r"^https?://github\.com/([^/]+)/([^/]+)/?$").ok()?;
        if let Some(captures) = re.captures(url) {
            let user = captures.get(1)?.as_str().to_string();
            let repo = captures.get(2)?.as_str().to_string();
            Some((user, repo))
        } else {
            None
        }
    })()
    .with_context(|| {
        UserMessageError::new(format!("'{}' is not a properly formatted github url.", url))
    })
}
