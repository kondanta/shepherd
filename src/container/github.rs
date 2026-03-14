use color_eyre::Result;
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

        tracing::info!("Fetching file from GitHub: {}", url);

        let mut request = self.client.get(&url);

        if let Some(token) = &self.token {
            request = request.header("Authorization", format!("token {}", token));
        }

        let response = request.send().await?;

        if !response.status().is_success() {
            return Err(color_eyre::eyre::eyre!(
                "Failed to fetch file from GitHub: status {}",
                response.status()
            ));
        }

        let content = response.text().await?;
        Ok(content)
    }
}
