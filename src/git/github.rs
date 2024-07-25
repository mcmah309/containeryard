use std::{collections::HashMap, path::Path, sync::LazyLock};

use anyhow::{bail, Context};
use futures::StreamExt;
use regex::Regex;
use reqwest::{Client, Response};
use tokio::{fs, io::AsyncWriteExt};
use tracing::debug;

use crate::{
    build::{
        IntermediateRemote, RemoteModuleInfo, SourceInfoKind, CONTAINERFILE_NAME,
        MODULE_YAML_FILE_NAME,
    },
    common::UserMessageError,
};

use super::{GitProvider, ModuleFilesData};

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

pub struct ModuleLocationInRemote {
    owner: String,
    repo: String,
    commit: String,
    path: String,
    /// The local name in yard.yaml
    name: String,
}

impl GitProvider for Github {
    async fn get_module_files(
        &self,
        remote: &IntermediateRemote,
    ) -> anyhow::Result<HashMap<String, ModuleFilesData>> {
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

        let mut module_to_files: HashMap<String, ModuleFilesData> = HashMap::new();
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
            let files = [
                (&module_file, MODULE_YAML_FILE_NAME),
                (&containerfile_file, CONTAINERFILE_NAME),
            ];
            for (local_path, file_name) in files {
                if local_path.exists() {
                    continue;
                }
                debug!("Did not find '{}' in cache.", module_file.display());
                let response = get_github_file(
                    &self.web_request_client,
                    &owner,
                    &repo,
                    &commit,
                    &format!("{}/{}", &path, file_name),
                )
                .await
                .map_err(UserMessageError::new)?;
                let file_data = response
                    .text()
                    .await
                    .map_err(UserMessageError::new)?;
                let parent = local_path
                    .parent()
                    .expect("Could not get parent directory.");
                if !parent.exists() {
                    fs::create_dir_all(parent).await.with_context(|| {
                        format!("Could not create '{}' directory.", parent.display())
                    })?;
                }
                fs::write(local_path, file_data.as_bytes())
                    .await
                    .with_context(|| {
                        format!("Could not write '{}' to cache.", local_path.display())
                    })?;
            }

            let containerfile_data: String = fs::read_to_string(&containerfile_file)
                .await
                .context(format!(
                    "Could not read '{}' to string.",
                    &containerfile_file.display()
                ))?
                .into();
            let module_file_data: String = fs::read_to_string(&module_file)
                .await
                .context(format!(
                    "Could not read '{}' to string.",
                    &module_file.display()
                ))?
                .into();

            let source_info = SourceInfoKind::RemoteModuleInfo(RemoteModuleInfo {
                url: remote.url.clone(),
                owner: owner,
                repo: repo,
                commit: commit,
                path: path,
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

    async fn download_file(
        &self,
        owner: &str,
        repo: &str,
        commit: &str,
        remote_path: &str,
        local_download_path: &Path,
    ) -> anyhow::Result<()> {
        let response =
            get_github_file(&self.web_request_client, owner, repo, commit, remote_path).await?;
        let mut stream = response.bytes_stream();
        let mut file = fs::File::create(local_download_path).await?;
        while let Some(item) = stream.next().await {
            let chunk = item?; // Get the chunk or the error if occurred
            file.write_all(&chunk).await?;
        }
        Ok(())
    }
}

fn create_url(owner: &str, repo: &str, commit: &str, path: &str) -> String {
    format!(
        "https://api.github.com/repos/{}/{}/contents/{}?ref={}",
        owner, repo, path, commit
    )
}

async fn get_github_file(
    client: &Client,
    owner: &str,
    repo: &str,
    commit: &str,
    path: &str,
) -> anyhow::Result<Response> {
    let url = create_url(owner, repo, commit, path);

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

    Ok(response)
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
