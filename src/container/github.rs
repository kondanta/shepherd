use color_eyre::Result;
use eyre::eyre;
use std::time::Duration;

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
                Err((e, true)) => last_err = Some(e),   // transient — retry
                Err((e, false)) => return Err(e),        // permanent — fail fast
            }
        }

        Err(last_err.unwrap())
    }

    /// Returns `Ok(content)` on success, `Err((error, retriable))` on failure.
    /// Only 5xx responses and 429 Too Many Requests are considered retriable;
    /// 4xx client errors (except 429) are permanent and should not be retried.
    async fn try_fetch(&self, url: &str) -> Result<String, (color_eyre::Report, bool)> {
        tracing::debug!("Fetching file from GitHub: {}", url);

        let mut request = self.client.get(url);
        if let Some(token) = &self.token {
            request = request.header("Authorization", format!("token {}", token));
        }

        let response = request
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

        response
            .text()
            .await
            .map_err(|e| (color_eyre::Report::from(e), true))
    }
}
