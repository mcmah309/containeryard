mod github;

use std::{collections::HashMap, path::Path};

use crate::{
    build::{IntermediateRemote, ModuleFilesData},
    common::UserMessageError,
};
use github::Github;

pub trait GitProvider {
    /// Gets the information from the provider or the cache.
    async fn get_module_files(
        &self,
        remote: &IntermediateRemote,
    ) -> anyhow::Result<HashMap<String, ModuleFilesData>>;

    async fn download_file(
        &self,
        owner: &str,
        repo: &str,
        commit: &str,
        remote_path: &str,
        local_download_path: &Path,
    ) -> anyhow::Result<()>;
}

pub enum GitProviderKind {
    Github(Github),
}

impl GitProvider for GitProviderKind {
    async fn get_module_files(
        &self,
        remote: &IntermediateRemote,
    ) -> anyhow::Result<HashMap<String, ModuleFilesData>> {
        match self {
            GitProviderKind::Github(github) => github.get_module_files(remote).await,
        }
    }

    async fn download_file(
        &self,
        owner: &str,
        repo: &str,
        commit: &str,
        remote_path: &str,
        local_download_path: &Path,
    ) -> anyhow::Result<()> {
        match self {
            GitProviderKind::Github(github) => {
                github
                    .download_file(owner, repo, commit, remote_path, local_download_path)
                    .await
            }
        }
    }
}

pub fn git_provider_from_url(url: &str) -> anyhow::Result<GitProviderKind> {
    // Note: Github does not support the `git archive`
    if url.contains("github.com") {
        return Ok(GitProviderKind::Github(Github::new()));
    }
    // Note: As a general case we can add something like e.g.`git archive --remote=https://github.com/mcmah309/indices.git a55f1eae8789123ee7de5aff603445da4d6e387d  Cargo.toml`
    anyhow::bail!(UserMessageError::new(format!(
        "A git provider for '{}' has not been implemented yet. Please make a PR for your use case if there isn't already one :)",
        url
    )))
}
