use std::collections::HashMap;

use globset::GlobBuilder;
use reqwest::header::{ACCEPT, AUTHORIZATION, USER_AGENT};
use serde::Deserialize;
use tracing::{debug, info, instrument, warn};

const GITHUB_API_BASE: &str = "https://api.github.com";

#[derive(Clone)]
pub struct GitHubClient {
    client: reqwest::Client,
    token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RepoMetadata {
    pub default_branch: String,
}

#[derive(Debug, Deserialize)]
struct ContentsResponse {
    content: Option<String>,
    sha: String,
    #[serde(rename = "type")]
    entry_type: String,
}

#[derive(Debug, Deserialize)]
struct TreeResponse {
    tree: Vec<TreeEntry>,
    truncated: bool,
}

#[derive(Debug, Deserialize)]
struct TreeEntry {
    path: String,
    #[serde(rename = "type")]
    entry_type: String,
    sha: String,
}

#[derive(Debug, Deserialize)]
struct BlobResponse {
    content: String,
    encoding: String,
}

#[derive(Debug, Deserialize)]
struct BranchResponse {
    commit: BranchCommit,
}

#[derive(Debug, Deserialize)]
struct BranchCommit {
    sha: String,
}

#[derive(Debug, Clone)]
pub struct FetchedFile {
    pub path: String,
    pub content: String,
    pub blob_sha: String,
}

#[derive(Debug, thiserror::Error)]
pub enum GitHubError {
    #[error("HTTP request failed: {0}")]
    Request(#[from] reqwest::Error),

    #[error("GitHub API returned {status}: {body}")]
    Api { status: u16, body: String },

    #[error("file not found: {0}")]
    NotFound(String),

    #[error("base64 decode error: {0}")]
    Base64(#[from] base64::DecodeError),

    #[error("UTF-8 decode error: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),

    #[error("tree listing was truncated; repository is too large for recursive tree fetch")]
    TreeTruncated,
}

impl GitHubClient {
    pub fn new(token: Option<String>) -> Self {
        let client = reqwest::Client::builder()
            .user_agent("mcbean/0.1")
            .build()
            .expect("failed to build HTTP client");
        Self { client, token }
    }

    pub fn from_env() -> Self {
        let token = std::env::var("GITHUB_TOKEN").ok();
        if token.is_none() {
            warn!(
                "GITHUB_TOKEN not set; GitHub API requests will be unauthenticated (60 req/hr limit)"
            );
        }
        Self::new(token)
    }

    fn apply_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let req = req
            .header(USER_AGENT, "mcbean/0.1")
            .header(ACCEPT, "application/vnd.github.v3+json");
        if let Some(ref token) = self.token {
            req.header(AUTHORIZATION, format!("Bearer {token}"))
        } else {
            req
        }
    }

    #[instrument(skip(self), fields(owner, repo))]
    pub async fn get_repo_metadata(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<RepoMetadata, GitHubError> {
        let url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}");
        debug!(%url, "fetching repo metadata");

        let resp = self.apply_auth(self.client.get(&url)).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(GitHubError::Api {
                status: status.as_u16(),
                body,
            });
        }
        Ok(resp.json().await?)
    }

    #[instrument(skip(self), fields(owner, repo, path, git_ref))]
    pub async fn get_file_contents(
        &self,
        owner: &str,
        repo: &str,
        path: &str,
        git_ref: &str,
    ) -> Result<FetchedFile, GitHubError> {
        let url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/contents/{path}");
        debug!(%url, %git_ref, "fetching file contents");

        let resp = self
            .apply_auth(self.client.get(&url).query(&[("ref", git_ref)]))
            .send()
            .await?;

        let status = resp.status();
        if status.as_u16() == 404 {
            return Err(GitHubError::NotFound(path.to_string()));
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(GitHubError::Api {
                status: status.as_u16(),
                body,
            });
        }

        let contents: ContentsResponse = resp.json().await?;
        if contents.entry_type != "file" {
            return Err(GitHubError::NotFound(format!(
                "{path} is a {}, not a file",
                contents.entry_type
            )));
        }

        let encoded = contents
            .content
            .ok_or_else(|| GitHubError::NotFound(format!("{path}: no content in response")))?;
        let decoded = decode_github_base64(&encoded)?;

        Ok(FetchedFile {
            path: path.to_string(),
            content: decoded,
            blob_sha: contents.sha,
        })
    }

    #[instrument(skip(self), fields(owner, repo, git_ref))]
    pub async fn get_branch_head_sha(
        &self,
        owner: &str,
        repo: &str,
        git_ref: &str,
    ) -> Result<String, GitHubError> {
        let url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/branches/{git_ref}");
        debug!(%url, "fetching branch info");

        let resp = self.apply_auth(self.client.get(&url)).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(GitHubError::Api {
                status: status.as_u16(),
                body,
            });
        }

        let branch: BranchResponse = resp.json().await?;
        Ok(branch.commit.sha)
    }

    /// Fetch the full recursive tree for a given tree SHA, returning a map of
    /// path -> (blob_sha) for all blob entries.
    #[instrument(skip(self), fields(owner, repo))]
    pub async fn get_tree_recursive(
        &self,
        owner: &str,
        repo: &str,
        tree_sha: &str,
    ) -> Result<HashMap<String, String>, GitHubError> {
        let url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/git/trees/{tree_sha}");
        debug!(%url, "fetching recursive tree");

        let resp = self
            .apply_auth(self.client.get(&url).query(&[("recursive", "1")]))
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(GitHubError::Api {
                status: status.as_u16(),
                body,
            });
        }

        let tree: TreeResponse = resp.json().await?;
        if tree.truncated {
            return Err(GitHubError::TreeTruncated);
        }

        Ok(tree
            .tree
            .into_iter()
            .filter(|e| e.entry_type == "blob")
            .map(|e| (e.path, e.sha))
            .collect())
    }

    /// Fetch a blob by its SHA and return decoded content.
    #[instrument(skip(self), fields(owner, repo, blob_sha))]
    pub async fn get_blob(
        &self,
        owner: &str,
        repo: &str,
        blob_sha: &str,
    ) -> Result<String, GitHubError> {
        let url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/git/blobs/{blob_sha}");
        debug!(%url, "fetching blob");

        let resp = self.apply_auth(self.client.get(&url)).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(GitHubError::Api {
                status: status.as_u16(),
                body,
            });
        }

        let blob: BlobResponse = resp.json().await?;
        if blob.encoding != "base64" {
            return Err(GitHubError::Api {
                status: 0,
                body: format!("unexpected blob encoding: {}", blob.encoding),
            });
        }
        decode_github_base64(&blob.content)
    }

    /// Resolve a list of include patterns (which may contain globs) against a
    /// repository, returning the matched file paths and their blob SHAs.
    ///
    /// Literal paths are fetched directly. Patterns containing glob characters
    /// (`*`, `?`, `[`) trigger a recursive tree fetch so they can be matched
    /// against the full file listing.
    #[instrument(skip(self, patterns), fields(owner, repo, git_ref, pattern_count = patterns.len()))]
    pub async fn resolve_include_patterns(
        &self,
        owner: &str,
        repo: &str,
        git_ref: &str,
        patterns: &[String],
    ) -> Result<Vec<FetchedFile>, GitHubError> {
        fn is_glob(pattern: &str) -> bool {
            pattern.contains('*') || pattern.contains('?') || pattern.contains('[')
        }

        let has_globs = patterns.iter().any(|p| is_glob(p));

        // Only fetch the tree if we actually need glob matching.
        let tree: Option<HashMap<String, String>> = if has_globs {
            let commit_sha = self.get_branch_head_sha(owner, repo, git_ref).await?;
            // The branch endpoint gives the commit SHA; we need the tree SHA
            // from the commit. Re-use get_tree_recursive with the commit SHA —
            // the git/trees endpoint accepts commit SHAs too.
            Some(self.get_tree_recursive(owner, repo, &commit_sha).await?)
        } else {
            None
        };

        let mut results = Vec::new();

        for pattern in patterns {
            if is_glob(pattern) {
                let tree = tree.as_ref().expect("tree must be fetched for globs");
                let glob = GlobBuilder::new(pattern)
                    .literal_separator(true)
                    .build()
                    .map_err(|e| GitHubError::Api {
                        status: 0,
                        body: format!("invalid glob pattern {pattern:?}: {e}"),
                    })?
                    .compile_matcher();

                let matched: Vec<_> = tree
                    .iter()
                    .filter(|(path, _)| glob.is_match(path.as_str()))
                    .collect();

                info!(
                    pattern = %pattern,
                    matched_count = matched.len(),
                    "glob resolved"
                );

                let mut matched: Vec<_> = matched;
                matched.sort_by(|a, b| a.0.cmp(b.0));
                for (path, blob_sha) in matched {
                    let content = self.get_blob(owner, repo, blob_sha).await?;
                    results.push(FetchedFile {
                        path: path.clone(),
                        content,
                        blob_sha: blob_sha.clone(),
                    });
                }
            } else {
                let fetched = self
                    .get_file_contents(owner, repo, pattern, git_ref)
                    .await?;
                results.push(fetched);
            }
        }

        Ok(results)
    }
}

/// Decode base64 content from GitHub API responses, which include newlines
/// in the encoded string.
fn decode_github_base64(encoded: &str) -> Result<String, GitHubError> {
    use base64::Engine;
    let cleaned: String = encoded.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = base64::engine::general_purpose::STANDARD.decode(cleaned)?;
    Ok(String::from_utf8(bytes)?)
}
