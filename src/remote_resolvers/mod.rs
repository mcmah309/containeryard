mod git;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use crate::build::ModuleFileData;
use crate::user_error::user_error;
use eros::Context;
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
    ) -> eros::Result<HashMap<String, ModuleFileData>>;

    /// Returns the reference information for this provider
    fn reference_info<'a>(&'a self) -> ReferenceInfo<'a>;

    /// Downloads the file and returns the data as a [String]
    async fn extract_remote_path_data(&self, remote_path: &str) -> eros::Result<String>;

    /// Downloads the file or gets from cache and returns the data as a [String]. Caches locally if the
    /// data is downloaded for the first time
    async fn extract_remote_path_data_save_save_to_cache(
        &self,
        remote_path: &str,
    ) -> eros::Result<String> {
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
        let file_data = self
            .extract_remote_path_data(remote_path)
            .await
            .with_context(|| {
                format!("Retrieve remote path '{remote_path}' from '{url}' at commit '{commit}'")
            })?;

        trace!("Saving `{:?}` downloaded from remote", reference_info);
        save_to_cache(
            &file_data,
            &remote_path_as_path,
            provider,
            repo_owner,
            repo_name,
            commit,
        )?;
        trace!("`{:?}` saved to cache", reference_info);

        Ok(file_data)
    }

    /// Downloads the file or gets from cache, and ensures it is available at `local_download_path`
    async fn retrieve_file_and_put_at(
        &self,
        remote_path: &str,
        local_download_path: &Path,
    ) -> eros::Result<()> {
        let file_data = self
            .extract_remote_path_data_save_save_to_cache(remote_path)
            .await?;
        let parent = local_download_path
            .parent()
            .ok_or_else(|| user_error("A required file has an invalid local destination path."))?;
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("Create directory '{}'", parent.display()))
            .user_context(
                "Could not create a directory for a required file. Check destination permissions.",
            )?;
        fs::write(local_download_path, file_data)
            .await
            .with_context(|| format!("Write required file to '{}'", local_download_path.display()))
            .user_context(
                "Could not save a required file. Check available disk space and destination permissions.",
            )?;
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
    ) -> eros::Result<HashMap<String, ModuleFileData>> {
        match self {
            GitProviderKind::Git(git) => git.retrieve_module(name_to_path).await,
        }
    }

    fn reference_info<'a>(&'a self) -> ReferenceInfo<'a> {
        match self {
            GitProviderKind::Git(git) => git.reference_info(),
        }
    }

    async fn extract_remote_path_data(&self, remote_path: &str) -> eros::Result<String> {
        match self {
            GitProviderKind::Git(git) => git.extract_remote_path_data(remote_path).await,
        }
    }

    async fn extract_remote_path_data_save_save_to_cache(
        &self,
        remote_path: &str,
    ) -> eros::Result<String> {
        match self {
            GitProviderKind::Git(git) => {
                git.extract_remote_path_data_save_save_to_cache(remote_path)
                    .await
            }
        }
    }
}

pub fn create_provider(url: String, commit: String) -> eros::Result<GitProviderKind> {
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
) -> eros::Result<()> {
    let cache_file_path = path_in_cache_dir(file_path, provider, owner, repo_name, commit)?;
    if !cache_file_path.exists() {
        if let Some(parent) = cache_file_path.parent()
            && !parent.exists()
        {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Create cache directory '{}'", parent.display()))?;
        }
        std::fs::write(&cache_file_path, data)
            .with_context(|| format!("Write cache file '{}'", cache_file_path.display()))?;
    }
    Ok(())
}

pub fn path_in_cache_dir(
    file_path: &Path,
    provider: &str,
    owner: &str,
    repo_name: &str,
    commit: &str,
) -> eros::Result<PathBuf> {
    let cache_dir = dirs::cache_dir().ok_or_else(|| {
        user_error(
            "Could not determine the system cache directory. Set a valid cache directory for this platform.",
        )
    })?;
    Ok(cache_dir
        .join("containeryard")
        .join("extracted_files")
        .join(provider)
        .join(owner)
        .join(repo_name)
        .join(commit)
        .join(file_path))
}
