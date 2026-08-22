use color_eyre::Result;
use eyre::eyre;
use serde::Deserialize;
use std::{future::Future, time::Duration};

pub trait GitHubProvider: Clone + Send + Sync + 'static {
    fn fetch_file_content(
        &self,
        owner: &str,
        repo: &str,
        path: &str,
        sha: &str,
    ) -> impl Future<Output = Result<String>> + Send;
    fn commits_since(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
        since_sha: Option<&str>,
    ) -> impl Future<Output = Result<(String, Vec<PollCommit>)>> + Send;

    fn get_commit_files(
        &self,
        owner: &str,
        repo: &str,
        sha: &str,
    ) -> impl Future<Output = Result<Vec<String>>> + Send;

    fn list_repo_files(
        &self,
        owner: &str,
        repo: &str,
        sha: &str,
    ) -> impl Future<Output = Result<Vec<String>>> + Send;
}

/// A commit returned by the polling API.
pub struct PollCommit {
    pub sha: String,
    pub author_name: String,
    pub author_email: String,
    /// GitHub login (e.g. `renovate[bot]`), absent for unregistered committers.
    pub author_login: Option<String>,
}

// ── private serde types for GitHub REST API responses ────────────────────────

#[derive(Deserialize)]
struct ApiCommitItem {
    sha: String,
    commit: ApiCommitMeta,
    /// Top-level author object (GitHub user); may be null.
    author: Option<ApiGitHubUser>,
}

#[derive(Deserialize)]
struct ApiCommitMeta {
    author: ApiCommitAuthor,
}

#[derive(Deserialize)]
struct ApiCommitAuthor {
    name: String,
    email: String,
}

#[derive(Deserialize)]
struct ApiGitHubUser {
    login: String,
}

#[derive(Deserialize)]
struct ApiCommitDetail {
    files: Option<Vec<ApiCommitFile>>,
}

#[derive(Deserialize)]
struct ApiCommitFile {
    filename: String,
}

#[derive(Deserialize)]
struct ApiTree {
    tree: Vec<ApiTreeEntry>,
    truncated: bool,
}

#[derive(Deserialize)]
struct ApiTreeEntry {
    path: String,
    #[serde(rename = "type")]
    entry_type: String,
}

#[derive(Clone)]
pub struct GitHubClient {
    client: reqwest::Client,
    token: Option<String>,
}

impl GitHubClient {
    pub fn new(token: Option<String>) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("Failed to build reqwest client"),
            token,
        }
    }

    #[tracing::instrument(skip(self), fields(owner, repo, path, sha))]
    pub async fn fetch_file_content(
        &self,
        owner: &str,
        repo: &str,
        path: &str,
        sha: &str,
    ) -> Result<String> {
        validate_repo_component(owner, "owner")?;
        validate_repo_component(repo, "repo")?;
        validate_sha(sha)?;

        let url = format!(
            "https://raw.githubusercontent.com/{}/{}/{}/{}",
            owner, repo, sha, path
        );

        let mut last_err = None;
        for attempt in 0..3u32 {
            if attempt > 0 {
                let delay = Duration::from_millis(500 * 2u64.pow(attempt - 1));
                tracing::warn!(
                    "Retrying GitHub fetch (attempt {}/3) after {}ms: {}",
                    attempt + 1,
                    delay.as_millis(),
                    url
                );
                tokio::time::sleep(delay).await;
            }

            match self.try_fetch(&url).await {
                Ok(content) => return Ok(content),
                Err((e, true)) => last_err = Some(e), // transient — retry
                Err((e, false)) => return Err(e),     // permanent — fail fast
            }
        }

        Err(last_err.unwrap())
    }

    // ── polling API ───────────────────────────────────────────────────────────

    /// Returns `(head_sha, new_commits_oldest_first)`.
    ///
    /// When `since_sha` is `None` (first call), returns just the HEAD commit so
    /// the caller can sync to the current repo state on startup.
    /// On subsequent calls pass the previously returned `head_sha` to get only
    /// commits that arrived since the last poll.
    #[tracing::instrument(skip(self), fields(owner, repo, branch))]
    pub async fn commits_since(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
        since_sha: Option<&str>,
    ) -> Result<(String, Vec<PollCommit>)> {
        validate_repo_component(owner, "owner")?;
        validate_repo_component(repo, "repo")?;
        validate_branch(branch)?;

        let url = format!(
            "https://api.github.com/repos/{owner}/{repo}/commits\
             ?sha={branch}&per_page=100"
        );
        let items: Vec<ApiCommitItem> = self.api_get(&url).await?;

        let head_sha = items.first().map(|c| c.sha.clone()).unwrap_or_default();

        let Some(base) = since_sha else {
            // First poll: the caller handles initial sync via list_repo_files,
            // so return no commits here.
            return Ok((head_sha, vec![]));
        };

        let pos = items.iter().position(|c| c.sha == base);
        let new_items: &[ApiCommitItem] = match pos {
            Some(p) => &items[..p],
            None => {
                if !items.is_empty() {
                    tracing::warn!(
                        "Last-known SHA {base} not found in the last {} \
                         commits — more than {} commits may have landed \
                         between polls. Consider reducing POLL_INTERVAL_SECS. \
                         Processing all visible commits as a best-effort recovery.",
                        items.len(),
                        items.len()
                    );
                }
                &items[..]
            }
        };

        let commits = new_items
            .iter()
            .rev()
            .map(|c| PollCommit {
                sha: c.sha.clone(),
                author_name: c.commit.author.name.clone(),
                author_email: c.commit.author.email.clone(),
                author_login: c.author.as_ref().map(|u| u.login.clone()),
            })
            .collect();

        Ok((head_sha, commits))
    }

    /// Returns the filenames changed by a single commit.
    #[tracing::instrument(skip(self), fields(owner, repo, sha))]
    pub async fn get_commit_files(
        &self,
        owner: &str,
        repo: &str,
        sha: &str,
    ) -> Result<Vec<String>> {
        validate_repo_component(owner, "owner")?;
        validate_repo_component(repo, "repo")?;
        validate_sha(sha)?;

        let url =
            format!("https://api.github.com/repos/{owner}/{repo}/commits/{sha}");
        let detail: ApiCommitDetail = self.api_get(&url).await?;
        let files: Vec<String> = detail
            .files
            .unwrap_or_default()
            .into_iter()
            .map(|f| f.filename)
            .collect();
        if files.len() == 300 {
            tracing::warn!(
                sha,
                "Commit file list is at GitHub's 300-file cap; \
                 some changed files may be missed"
            );
        }
        Ok(files)
    }

    /// Returns every file path (blob) in the repository at the given commit SHA.
    ///
    /// Uses the recursive git trees endpoint — one API call for the full tree.
    /// If the tree is truncated (repos with >100k objects), a warning is logged
    /// and the partial list is returned.
    #[tracing::instrument(skip(self), fields(owner, repo, sha))]
    pub async fn list_repo_files(
        &self,
        owner: &str,
        repo: &str,
        sha: &str,
    ) -> Result<Vec<String>> {
        validate_repo_component(owner, "owner")?;
        validate_repo_component(repo, "repo")?;
        validate_sha(sha)?;

        let url = format!(
            "https://api.github.com/repos/{owner}/{repo}/git/trees/{sha}?recursive=1"
        );
        let tree: ApiTree = self.api_get(&url).await?;
        if tree.truncated {
            tracing::warn!(
                "Repository tree was truncated; \
                 some files may be missed on initial sync"
            );
        }
        Ok(tree
            .tree
            .into_iter()
            .filter(|e| e.entry_type == "blob")
            .map(|e| e.path)
            .collect())
    }

    /// Attaches the GitHub auth token to a request, if one is configured.
    fn with_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(token) = &self.token {
            req.header("Authorization", format!("token {token}"))
        } else {
            req
        }
    }

    /// Thin wrapper around the GitHub REST API. Handles auth and JSON parsing.
    async fn api_get<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T> {
        let request = self.with_auth(
            self.client
                .get(url)
                .header("User-Agent", "shepherd")
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28"),
        );

        let response = request.send().await.map_err(color_eyre::Report::from)?;

        if !response.status().is_success() {
            let status = response.status();
            let hint = match status {
                s if s == reqwest::StatusCode::UNAUTHORIZED => {
                    " — check GITHUB_TOKEN (missing, expired, or wrong account)"
                }
                s if s == reqwest::StatusCode::FORBIDDEN
                    || s == reqwest::StatusCode::TOO_MANY_REQUESTS =>
                {
                    " — rate limited; set GITHUB_TOKEN or increase POLL_INTERVAL_SECS"
                }
                s if s == reqwest::StatusCode::NOT_FOUND => {
                    " — repo not found; check POLL_REPO and that GITHUB_TOKEN has read access"
                }
                _ => "",
            };
            return Err(eyre!("GitHub API error: HTTP {status}{hint}"));
        }

        let text = response.text().await.map_err(color_eyre::Report::from)?;
        serde_json::from_str(&text)
            .map_err(|e| eyre!("GitHub API response parse error: {e}"))
    }

    /// Returns `Ok(content)` on success, `Err((error, retriable))` on failure.
    /// Only 5xx responses and 429 Too Many Requests are considered retriable;
    /// 4xx client errors (except 429) are permanent and should not be retried.
    async fn try_fetch(
        &self,
        url: &str,
    ) -> Result<String, (color_eyre::Report, bool)> {
        tracing::debug!("Fetching file from GitHub: {}", url);

        let response = self
            .with_auth(self.client.get(url))
            .send()
            .await
            .map_err(|e| (color_eyre::Report::from(e), true))?; // network error → retry

        let status = response.status();

        if !status.is_success() {
            let retriable = status.is_server_error()
                || status == reqwest::StatusCode::TOO_MANY_REQUESTS;

            let hint = if status == reqwest::StatusCode::FORBIDDEN
                || status == reqwest::StatusCode::TOO_MANY_REQUESTS
            {
                " (rate limit? consider setting GITHUB_TOKEN)"
            } else {
                ""
            };

            return Err((
                eyre!("Failed to fetch file from GitHub: HTTP {}{hint}", status),
                retriable,
            ));
        }

        response.text().await.map_err(|e| (color_eyre::Report::from(e), true))
    }
}

impl GitHubProvider for GitHubClient {
    async fn fetch_file_content(
        &self,
        owner: &str,
        repo: &str,
        path: &str,
        sha: &str,
    ) -> Result<String> {
        self.fetch_file_content(owner, repo, path, sha).await
    }

    async fn commits_since(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
        since_sha: Option<&str>,
    ) -> Result<(String, Vec<PollCommit>)> {
        self.commits_since(owner, repo, branch, since_sha).await
    }

    async fn get_commit_files(
        &self,
        owner: &str,
        repo: &str,
        sha: &str,
    ) -> Result<Vec<String>> {
        self.get_commit_files(owner, repo, sha).await
    }

    async fn list_repo_files(
        &self,
        owner: &str,
        repo: &str,
        sha: &str,
    ) -> Result<Vec<String>> {
        self.list_repo_files(owner, repo, sha).await
    }
}

// ── input validation (CWE-22/23/36/99) ───────────────────────────────────────

/// SHA must be 40 or 64 lowercase hex chars (SHA-1 / SHA-256).
fn validate_sha(sha: &str) -> Result<()> {
    let valid = (sha.len() == 40 || sha.len() == 64)
        && sha.chars().all(|c| c.is_ascii_hexdigit());
    if valid {
        Ok(())
    } else {
        Err(eyre!("Invalid SHA: {sha:?} — must be 40 or 64 hex chars"))
    }
}

/// Owner and repo names: alphanumeric, hyphens, dots, underscores. No slashes.
fn validate_repo_component(value: &str, label: &str) -> Result<()> {
    let valid = !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if valid {
        Ok(())
    } else {
        Err(eyre!(
            "Invalid {label}: {value:?} — only alphanumeric, hyphens, underscores, dots allowed"
        ))
    }
}

/// Branch names may contain slashes (e.g. `feature/foo`) but must not contain
/// characters that would inject extra URL query parameters or path traversal.
fn validate_branch(branch: &str) -> Result<()> {
    let invalid = branch.is_empty()
        || branch.contains("..")
        || branch.chars().any(|c| matches!(c, '?' | '&' | '#' | '\0'));
    if invalid { Err(eyre!("Invalid branch name: {branch:?}")) } else { Ok(()) }
}

#[cfg(test)]
pub use fake::FakeGitHubClient;

#[cfg(test)]
mod fake {
    use super::*;
    use std::collections::{HashMap, VecDeque};
    use std::sync::{Arc, Mutex};

    // By itself, it is too complex-- hence created a new type
    type CommitsQueue = Arc<Mutex<VecDeque<(String, Vec<PollCommit>)>>>;

    #[derive(Clone, Default)]
    pub struct FakeGitHubClient {
        pub commits_queue: CommitsQueue,
        pub commit_files: Arc<Mutex<HashMap<String, Vec<String>>>>,
        pub repo_files: Arc<Mutex<Vec<String>>>,
        pub file_content: Arc<Mutex<HashMap<String, String>>>,
        pub get_commit_files_calls: Arc<Mutex<Vec<String>>>,
    }

    impl GitHubProvider for FakeGitHubClient {
        async fn commits_since(
            &self,
            _owner: &str,
            _repo: &str,
            _branch: &str,
            _since_sha: Option<&str>,
        ) -> Result<(String, Vec<PollCommit>)> {
            Ok(self
                .commits_queue
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| ("a".repeat(40), vec![])))
        }

        async fn get_commit_files(
            &self,
            _owner: &str,
            _repo: &str,
            sha: &str,
        ) -> Result<Vec<String>> {
            self.get_commit_files_calls.lock().unwrap().push(sha.to_string());
            Ok(self
                .commit_files
                .lock()
                .unwrap()
                .get(sha)
                .cloned()
                .unwrap_or_default())
        }

        async fn list_repo_files(
            &self,
            _owner: &str,
            _repo: &str,
            _sha: &str,
        ) -> Result<Vec<String>> {
            Ok(self.repo_files.lock().unwrap().clone())
        }

        async fn fetch_file_content(
            &self,
            _owner: &str,
            _repo: &str,
            path: &str,
            _sha: &str,
        ) -> Result<String> {
            Ok(self
                .file_content
                .lock()
                .unwrap()
                .get(path)
                .cloned()
                .unwrap_or_default())
        }
    }
}

#[cfg(test)]
mod validation_tests {
    use super::*;

    #[test]
    fn sha_valid() {
        assert!(validate_sha(&"a".repeat(40)).is_ok());
        assert!(validate_sha(&"f".repeat(64)).is_ok());
    }

    #[test]
    fn sha_invalid() {
        assert!(validate_sha("not-a-sha").is_err());
        assert!(validate_sha(&"a".repeat(39)).is_err());
        assert!(validate_sha("../../etc/passwd").is_err());
    }

    #[test]
    fn repo_component_valid() {
        assert!(validate_repo_component("my-org", "owner").is_ok());
        assert!(validate_repo_component("my_repo.git", "repo").is_ok());
    }

    #[test]
    fn repo_component_invalid() {
        assert!(validate_repo_component("org/evil", "owner").is_err());
        assert!(validate_repo_component("", "owner").is_err());
        assert!(validate_repo_component("../etc", "repo").is_err());
    }

    #[test]
    fn branch_valid() {
        assert!(validate_branch("main").is_ok());
        assert!(validate_branch("feature/my-feature").is_ok());
    }

    #[test]
    fn branch_invalid() {
        assert!(validate_branch("").is_err());
        assert!(validate_branch("branch..evil").is_err());
        assert!(validate_branch("branch&injected=1").is_err());
    }
}
