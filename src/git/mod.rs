mod git;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use crate::build::ModuleFileData;
use git::Git;
use tokio::fs;
use tracing::{info, trace};

/// Reference information for a git provider
#[derive(Debug)]
pub struct ReferenceInfo<'a> {
    provider: &'a str,
    repo_owner: &'a str,
    repo_name: &'a str,
    url: &'a str,
    commit: &'a str,
}

pub trait GitProvider {
    /// Downloads the module module file or gets from cache at the
    /// specified paths, and returns the raw data.
    async fn retrieve_module(
        &self,
        name_to_path: HashMap<String, String>,
    ) -> anyhow::Result<HashMap<String, ModuleFileData>>;

    /// Returns the reference information for this provider
    fn reference_info<'a>(&'a self) -> ReferenceInfo<'a>;

    /// Downloads the file and returns the data as a [String]
    async fn extract_remote_path_data(&self, remote_path: &str) -> anyhow::Result<String>;

    /// Downloads the file or gets from cache and returns the data as a [String]. Caches locally if the
    /// data is downloaded for the first time
    async fn extract_remote_path_data_save_save_to_cache(
        &self,
        remote_path: &str,
    ) -> anyhow::Result<String> {
        // Check if file is at cache, if so copy over
        let remote_path_as_path = PathBuf::from(remote_path);
        let reference_info = self.reference_info();
        let ReferenceInfo {
            provider,
            repo_owner,
            repo_name,
            url,
            commit,
        } = reference_info;

        trace!(
            "`{:?}` not found in cache, downloading from remote",
            reference_info
        );
        let file_data = self.extract_remote_path_data(&remote_path).await?;

        trace!("Saving `{:?}` downloaded from remote", reference_info);
        save_to_cache(
            &file_data,
            &remote_path_as_path,
            &provider,
            &repo_owner,
            &repo_name,
            &commit,
        )?;
        trace!("`{:?}` saved to cache", reference_info);

        Ok(file_data)
    }

    /// Downloads the file or gets from cache, and ensures it is available at `local_download_path`
    async fn retrieve_file_and_put_at(
        &self,
        remote_path: &str,
        local_download_path: &Path,
    ) -> anyhow::Result<()> {
        let file_data = self
            .extract_remote_path_data_save_save_to_cache(remote_path)
            .await?;
        fs::create_dir_all(local_download_path.parent().unwrap()).await?;
        fs::write(local_download_path, file_data).await?;
        Ok(())
    }
}

#[derive(Debug)]
pub enum GitProviderKind {
    /// Fallback (git clone)
    Git(Git),
}

impl GitProvider for GitProviderKind {
    async fn retrieve_module(
        &self,
        name_to_path: HashMap<String, String>,
    ) -> anyhow::Result<HashMap<String, ModuleFileData>> {
        match self {
            GitProviderKind::Git(git) => git.retrieve_module(name_to_path).await,
        }
    }

    fn reference_info<'a>(&'a self) -> ReferenceInfo<'a> {
        match self {
            GitProviderKind::Git(git) => git.reference_info(),
        }
    }

    async fn extract_remote_path_data(&self, remote_path: &str) -> anyhow::Result<String> {
        match self {
            GitProviderKind::Git(git) => git.extract_remote_path_data(remote_path).await,
        }
    }

    async fn extract_remote_path_data_save_save_to_cache(
        &self,
        remote_path: &str,
    ) -> anyhow::Result<String> {
        match self {
            GitProviderKind::Git(git) => {
                git
                    .extract_remote_path_data_save_save_to_cache(remote_path)
                    .await
            }
        }
    }
}

pub fn create_provider(url: String, commit: String) -> anyhow::Result<GitProviderKind> {
    // Note: Github does not support the `git archive`
    if url.contains("github.com") || url.contains("git@github.com") {
        return Ok(GitProviderKind::Git(Git::new(url, commit)?));
    }

    info!("Unknown provider falling back to using default git resolver");
    Ok(GitProviderKind::Git(Git::new(url, commit)?))
}

pub fn save_to_cache(
    data: &str,
    file_path: &Path,
    provider: &str,
    owner: &str,
    repo_name: &str,
    commit: &str,
) -> anyhow::Result<()> {
    let cache_file_path = path_in_cache_dir(file_path, provider, owner, repo_name, commit);
    if !cache_file_path.exists() {
        if let Some(parent) = cache_file_path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent)?;
            }
        }
        std::fs::write(cache_file_path, data)?;
    }
    Ok(())
}

pub fn path_in_cache_dir(
    file_path: &Path,
    provider: &str,
    owner: &str,
    repo_name: &str,
    commit: &str,
) -> PathBuf {
    dirs::cache_dir()
        .expect("Could not determine cache directory of platform")
        .join("extracted_files")
        .join(&provider)
        .join(&owner)
        .join(&repo_name)
        .join(&commit)
        .join(&file_path)
}