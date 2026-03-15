use color_eyre::Result;
use eyre::eyre;
use serde::Deserialize;
use std::time::Duration;

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
        let url = format!(
            "https://api.github.com/repos/{owner}/{repo}/commits\
             ?sha={branch}&per_page=100"
        );
        let items: Vec<ApiCommitItem> = self.api_get(&url).await?;

        let head_sha = items.first().map(|c| c.sha.clone()).unwrap_or_default();

        let Some(base) = since_sha else {
            let head_commit = items.first().map(|c| PollCommit {
                sha: c.sha.clone(),
                author_name: c.commit.author.name.clone(),
                author_email: c.commit.author.email.clone(),
                author_login: c.author.as_ref().map(|u| u.login.clone()),
            });
            return Ok((head_sha, head_commit.into_iter().collect()));
        };

        let pos = items.iter().position(|c| c.sha == base);
        let new_items: &[ApiCommitItem] = match pos {
            Some(p) => &items[..p],
            None => {
                if !items.is_empty() {
                    tracing::warn!(
                        "Last-known SHA {base} not found in the last {} \
                         commits; some commits may have been missed",
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
        let url =
            format!("https://api.github.com/repos/{owner}/{repo}/commits/{sha}");
        let detail: ApiCommitDetail = self.api_get(&url).await?;
        Ok(detail
            .files
            .unwrap_or_default()
            .into_iter()
            .map(|f| f.filename)
            .collect())
    }

    /// Thin wrapper around the GitHub REST API. Handles auth and JSON parsing.
    async fn api_get<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T> {
        let mut request = self
            .client
            .get(url)
            .header("User-Agent", "shepherd")
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");

        if let Some(token) = &self.token {
            request = request.header("Authorization", format!("token {token}"));
        }

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

        let mut request = self.client.get(url);
        if let Some(token) = &self.token {
            request = request.header("Authorization", format!("token {}", token));
        }

        let response =
            request.send().await.map_err(|e| (color_eyre::Report::from(e), true))?; // network error → retry

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
