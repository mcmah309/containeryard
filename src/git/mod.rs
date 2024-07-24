mod github;

use std::collections::HashMap;

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

pub struct ModuleLocationInRemote {
    owner: String,
    repo: String,
    commit: String,
    path: String,
    /// The local name in yard.yaml
    pub name: String,
}
