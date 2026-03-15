use dotenvy::dotenv;
use std::env;

#[derive(Debug)]
pub enum Mode {
    Webhook { secret: String },
    Poll { repo: String, interval_secs: u64, branch: String },
}

#[derive(Debug)]
pub struct Config {
    pub root_dir: String,
    pub log_level: String,
    #[cfg(feature = "otlp")]
    pub otlp_endpoint: String,

    pub renovate_username: String,
    pub renovate_email: String,
    pub github_token: Option<String>,
    /// When false (default), services using the `latest` tag are skipped.
    pub allow_latest_images: bool,
    /// Static bearer token protecting the /flags/* endpoints.
    /// If unset, those endpoints return 401.
    pub api_token: Option<String>,

    /// Whether to run in webhook or polling mode.
    pub mode: Mode,

    /// Only process files whose repo path starts with this prefix.
    /// The prefix is stripped when constructing the local path, so
    /// `baremetals/node1/myapp/compose.yaml` with prefix `baremetals/node1`
    /// writes to `ROOT_DIR/myapp/compose.yaml`.
    pub repo_path_prefix: Option<String>,
}

impl Config {
    pub fn load() -> Result<Self, String> {
        // Load .env if it exists; ignore errors
        let _ = dotenv();

        let root_dir = env::var("ROOT_DIR")
            .map_err(|_| "ROOT_DIR environment variable must be set".to_string())?;

        let log_level = env::var("LOG_LEVEL").unwrap_or_else(|_| "info".to_string());
        const VALID: &[&str] = &["trace", "debug", "info", "warn", "error"];
        if !VALID.contains(&log_level.as_str()) {
            return Err(format!(
                "LOG_LEVEL '{}' is invalid; must be one of: {}",
                log_level,
                VALID.join(", ")
            ));
        }

        let renovate_username = env::var("RENOVATE_USERNAME")
            .unwrap_or_else(|_| "renovate[bot]".to_string());

        let renovate_email = env::var("RENOVATE_EMAIL").unwrap_or_else(|_| {
            "renovate[bot]@users.noreply.github.com".to_string()
        });

        let webhook_secret =
            env::var("WEBHOOK_SECRET").ok().filter(|s| !s.is_empty());
        let poll_repo = env::var("POLL_REPO").ok().filter(|s| !s.is_empty());

        let mode = match (webhook_secret, poll_repo) {
            (Some(_), Some(_)) => {
                return Err("WEBHOOK_SECRET and POLL_REPO are mutually exclusive; \
                     set only one to choose a mode"
                    .to_string());
            }
            (None, None) => {
                return Err("Must set either WEBHOOK_SECRET (webhook mode) \
                     or POLL_REPO (polling mode)"
                    .to_string());
            }
            (Some(secret), None) => Mode::Webhook { secret },
            (None, Some(repo)) => {
                if !repo.contains('/') {
                    return Err("POLL_REPO must be in owner/repo format \
                         (e.g. acme/my-infra)"
                        .to_string());
                }
                let interval_secs = env::var("POLL_INTERVAL_SECS")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(300);
                let branch =
                    env::var("POLL_BRANCH").unwrap_or_else(|_| "main".to_string());
                Mode::Poll { repo, interval_secs, branch }
            }
        };

        let github_token = env::var("GITHUB_TOKEN").ok().filter(|s| !s.is_empty());
        let api_token = env::var("API_TOKEN").ok().filter(|s| !s.is_empty());
        let repo_path_prefix =
            env::var("REPO_PATH_PREFIX").ok().filter(|s| !s.is_empty());

        let allow_latest_images = env::var("ALLOW_LATEST_IMAGES")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(false);

        #[cfg(feature = "otlp")]
        let otlp_endpoint = env::var("OTLP_ENDPOINT")
            .ok()
            .filter(|s| !s.is_empty())
            .ok_or("OTLP_ENDPOINT must be set when the 'otlp' feature is enabled")?;

        Ok(Config {
            root_dir,
            log_level,
            renovate_username,
            renovate_email,
            mode,
            github_token,
            allow_latest_images,
            api_token,
            repo_path_prefix,
            #[cfg(feature = "otlp")]
            otlp_endpoint,
        })
    }
}
