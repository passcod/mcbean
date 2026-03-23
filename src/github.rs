use std::collections::HashMap;

use globset::GlobBuilder;
use reqwest::header::{ACCEPT, AUTHORIZATION, USER_AGENT};
use serde::{Deserialize, Serialize};
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
/// A file path + content pair for committing via the Git Data API.
#[derive(Debug, Clone)]
pub struct FileToCommit {
    pub path: String,
    pub content: String,
}

// -- Git Data API request/response types ------------------------------------

#[derive(Debug, Deserialize)]
struct GitCommitResponse {
    tree: GitCommitTree,
}

#[derive(Debug, Deserialize)]
struct GitCommitTree {
    sha: String,
}

#[derive(Debug, Serialize)]
struct CreateRefRequest<'a> {
    #[serde(rename = "ref")]
    git_ref: &'a str,
    sha: &'a str,
}

#[derive(Debug, Serialize)]
struct CreateTreeRequest<'a> {
    base_tree: &'a str,
    tree: Vec<TreeBlobEntry<'a>>,
}

#[derive(Debug, Serialize)]
struct TreeBlobEntry<'a> {
    path: &'a str,
    mode: &'a str,
    #[serde(rename = "type")]
    entry_type: &'a str,
    content: &'a str,
}

#[derive(Debug, Deserialize)]
struct CreateTreeResponse {
    sha: String,
}

#[derive(Debug, Serialize)]
struct CreateCommitRequest<'a> {
    message: &'a str,
    tree: &'a str,
    parents: Vec<&'a str>,
}

#[derive(Debug, Deserialize)]
struct CreateCommitResponseData {
    sha: String,
}

#[derive(Debug, Serialize)]
struct UpdateRefRequest<'a> {
    sha: &'a str,
    force: bool,
}

// -- Pull Request API types -------------------------------------------------

#[derive(Debug, Serialize)]
struct CreatePullRequestBody<'a> {
    title: &'a str,
    head: &'a str,
    base: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
pub struct PullRequestResponse {
    pub number: i64,
    pub state: String,
    pub merged: Option<bool>,
    pub draft: Option<bool>,
}

#[derive(Debug, thiserror::Error)]
pub enum GitHubError {
    #[error("HTTP request failed: {0}")]
    Request(#[from] reqwest::Error),

    #[error("GitHub API returned {status}: {body}")]
    Api { status: u16, body: String },

    #[error("GitHub GraphQL error: {0}")]
    Graphql(String),

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

    // -- Branch management --------------------------------------------------

    /// Create a new branch pointing at `sha`.
    #[instrument(skip(self), fields(owner, repo, branch_name))]
    pub async fn create_branch(
        &self,
        owner: &str,
        repo: &str,
        branch_name: &str,
        sha: &str,
    ) -> Result<(), GitHubError> {
        let url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/git/refs");
        let body = CreateRefRequest {
            git_ref: &format!("refs/heads/{branch_name}"),
            sha,
        };
        debug!(%url, %branch_name, %sha, "creating branch");

        let resp = self
            .apply_auth(self.client.post(&url))
            .json(&body)
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
        Ok(())
    }

    /// Commit a set of files on top of `parent_sha` and update `branch_name`
    /// to point at the new commit.
    #[instrument(skip(self, files), fields(owner, repo, branch_name, file_count = files.len()))]
    pub async fn commit_files(
        &self,
        owner: &str,
        repo: &str,
        branch_name: &str,
        parent_sha: &str,
        message: &str,
        files: &[FileToCommit],
    ) -> Result<String, GitHubError> {
        // 1. Get the tree SHA of the parent commit.
        let commit_url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/git/commits/{parent_sha}");
        let resp = self.apply_auth(self.client.get(&commit_url)).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(GitHubError::Api {
                status: status.as_u16(),
                body,
            });
        }
        let parent_commit: GitCommitResponse = resp.json().await?;
        let base_tree_sha = parent_commit.tree.sha;

        // 2. Create a new tree with the file changes.
        let tree_url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/git/trees");
        let tree_entries: Vec<TreeBlobEntry<'_>> = files
            .iter()
            .map(|f| TreeBlobEntry {
                path: &f.path,
                mode: "100644",
                entry_type: "blob",
                content: &f.content,
            })
            .collect();
        let create_tree = CreateTreeRequest {
            base_tree: &base_tree_sha,
            tree: tree_entries,
        };
        let resp = self
            .apply_auth(self.client.post(&tree_url))
            .json(&create_tree)
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
        let new_tree: CreateTreeResponse = resp.json().await?;

        // 3. Create the commit.
        let commit_create_url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/git/commits");
        let create_commit = CreateCommitRequest {
            message,
            tree: &new_tree.sha,
            parents: vec![parent_sha],
        };
        let resp = self
            .apply_auth(self.client.post(&commit_create_url))
            .json(&create_commit)
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
        let new_commit: CreateCommitResponseData = resp.json().await?;

        // 4. Update the branch ref.
        let ref_url =
            format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/git/refs/heads/{branch_name}");
        let update_ref = UpdateRefRequest {
            sha: &new_commit.sha,
            force: false,
        };
        let resp = self
            .apply_auth(self.client.patch(&ref_url))
            .json(&update_ref)
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

        info!(new_sha = %new_commit.sha, "committed files to branch");
        Ok(new_commit.sha)
    }

    // -- Pull request management --------------------------------------------

    /// Create a pull request and return its number.
    #[instrument(skip(self), fields(owner, repo, head, base))]
    pub async fn create_pull_request(
        &self,
        owner: &str,
        repo: &str,
        title: &str,
        head: &str,
        base: &str,
        body: Option<&str>,
    ) -> Result<i64, GitHubError> {
        let url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/pulls");
        let req = CreatePullRequestBody {
            title,
            head,
            base,
            body,
        };
        debug!(%url, "creating pull request");

        let resp = self
            .apply_auth(self.client.post(&url))
            .json(&req)
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

        let pr: PullRequestResponse = resp.json().await?;
        info!(pr_number = pr.number, "pull request created");
        Ok(pr.number)
    }

    /// Convert an existing PR to draft mode using the GraphQL API, and post
    /// a comment explaining the proposal was reopened for editing.
    #[instrument(skip(self), fields(owner, repo, pr_number))]
    pub async fn convert_pr_to_draft_with_comment(
        &self,
        owner: &str,
        repo: &str,
        pr_number: i64,
        comment: &str,
    ) -> Result<(), GitHubError> {
        // Post the comment via REST.
        let comment_url =
            format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/issues/{pr_number}/comments");
        let comment_body = serde_json::json!({ "body": comment });
        let resp = self
            .apply_auth(self.client.post(&comment_url))
            .json(&comment_body)
            .send()
            .await?;
        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            warn!(pr_number, %body, "failed to post PR comment");
        }

        // Convert to draft via GraphQL (REST doesn't support this).
        let graphql_url = format!("{GITHUB_API_BASE}/graphql");
        let query = "mutation($id: ID!) { convertPullRequestToDraft(input: { pullRequestId: $id }) { pullRequest { id } } }";

        // We need the node_id of the PR.
        let pr_url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/pulls/{pr_number}");
        let resp = self.apply_auth(self.client.get(&pr_url)).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(GitHubError::Api {
                status: status.as_u16(),
                body,
            });
        }

        #[derive(Deserialize)]
        struct PrNodeId {
            node_id: String,
        }
        let pr_data: PrNodeId = resp.json().await?;

        let gql_body = serde_json::json!({
            "query": query,
            "variables": { "id": pr_data.node_id }
        });
        let resp = self
            .apply_auth(self.client.post(&graphql_url))
            .json(&gql_body)
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
        let body: serde_json::Value = resp.json().await?;
        check_graphql_errors(&body)?;

        info!(pr_number, "converted PR to draft with comment");
        Ok(())
    }

    /// Mark a draft pull request as ready for review using the GraphQL API.
    // r[impl lifecycle.submitted.resubmit]
    #[instrument(skip(self), fields(owner, repo, pr_number))]
    pub async fn mark_pr_ready_for_review(
        &self,
        owner: &str,
        repo: &str,
        pr_number: i64,
    ) -> Result<(), GitHubError> {
        let pr_url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/pulls/{pr_number}");
        let resp = self.apply_auth(self.client.get(&pr_url)).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(GitHubError::Api {
                status: status.as_u16(),
                body,
            });
        }

        #[derive(Deserialize)]
        struct PrNodeId {
            node_id: String,
        }
        let pr_data: PrNodeId = resp.json().await?;

        let graphql_url = format!("{GITHUB_API_BASE}/graphql");
        let query = "mutation($id: ID!) { markPullRequestReadyForReview(input: { pullRequestId: $id }) { pullRequest { id } } }";
        let gql_body = serde_json::json!({
            "query": query,
            "variables": { "id": pr_data.node_id }
        });
        let resp = self
            .apply_auth(self.client.post(&graphql_url))
            .json(&gql_body)
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
        let body: serde_json::Value = resp.json().await?;
        check_graphql_errors(&body)?;

        info!(pr_number, "marked PR as ready for review");
        Ok(())
    }

    /// Send a Slack notification via incoming webhook.
    #[instrument(skip(self, webhook_url))]
    pub async fn send_slack_notification(
        &self,
        webhook_url: &str,
        text: &str,
    ) -> Result<(), GitHubError> {
        let body = serde_json::json!({ "text": text });
        let resp = self.client.post(webhook_url).json(&body).send().await?;
        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            warn!(%body, "slack notification failed");
        }
        Ok(())
    }

    /// Get the state of a single PR by number.
    #[instrument(skip(self), fields(owner, repo, pr_number))]
    pub async fn get_pull_request(
        &self,
        owner: &str,
        repo: &str,
        pr_number: i64,
    ) -> Result<PullRequestResponse, GitHubError> {
        let url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/pulls/{pr_number}");
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

    /// List open PRs whose head branch matches the given name.
    /// Returns PRs targeting the proposal branch (implementation PRs).
    #[instrument(skip(self), fields(owner, repo, head_branch))]
    pub async fn list_prs_with_base(
        &self,
        owner: &str,
        repo: &str,
        base_branch: &str,
    ) -> Result<Vec<PullRequestResponse>, GitHubError> {
        let url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/pulls");
        let resp = self
            .apply_auth(
                self.client
                    .get(&url)
                    .query(&[("state", "open"), ("base", base_branch)]),
            )
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
        Ok(resp.json().await?)
    }
}

/// Check a GraphQL response body for a top-level `"errors"` field and return
/// a `GitHubError::Graphql` if any are present. GraphQL always returns HTTP 200
/// even for errors, so an HTTP status check alone is insufficient.
fn check_graphql_errors(body: &serde_json::Value) -> Result<(), GitHubError> {
    if let Some(errors) = body.get("errors") {
        let msg = errors
            .as_array()
            .and_then(|arr| {
                let msgs: Vec<&str> = arr
                    .iter()
                    .filter_map(|e| e.get("message").and_then(|m| m.as_str()))
                    .collect();
                if msgs.is_empty() {
                    None
                } else {
                    Some(msgs.join("; "))
                }
            })
            .unwrap_or_else(|| errors.to_string());
        return Err(GitHubError::Graphql(msg));
    }
    Ok(())
}

/// Decode base64 content from GitHub API responses, which include newlines
/// in the encoded string.
fn decode_github_base64(encoded: &str) -> Result<String, GitHubError> {
    use base64::Engine;
    let cleaned: String = encoded.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = base64::engine::general_purpose::STANDARD.decode(cleaned)?;
    Ok(String::from_utf8(bytes)?)
}
