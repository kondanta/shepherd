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

    /// If set, only these service names are eligible for deployment.
    /// Services not in the list are ignored even if their config changed.
    pub service_filter: Option<Vec<String>>,

    /// When true (default), shepherd pulls and applies all services on startup
    /// using idempotent `docker compose up -d --no-deps`. Set `INITIAL_SYNC=false`
    /// to opt out. Covers the gap between a shepherd restart and the next push.
    pub initial_sync: bool,

    /// The compose service name shepherd runs as. When set, shepherd sorts
    /// itself last in any batch so sibling services deploy before it replaces
    /// itself. Set to match the service name in your compose file.
    pub shepherd_service_name: Option<String>,

    /// Only process files whose repo path starts with this prefix.
    /// The prefix is stripped when constructing the local path, so
    /// `baremetals/node1/myapp/compose.yaml` with prefix `baremetals/node1`
    /// writes to `ROOT_DIR/myapp/compose.yaml`.
    pub repo_path_prefix: Option<String>,
}

#[cfg(test)]
impl Config {
    pub(crate) fn for_test(api_token: Option<&str>) -> Self {
        Config {
            root_dir: std::env::temp_dir().to_string_lossy().into_owned(),
            log_level: "info".to_string(),
            renovate_username: "renovate[bot]".to_string(),
            renovate_email: "renovate[bot]@users.noreply.github.com".to_string(),
            github_token: None,
            allow_latest_images: false,
            api_token: api_token.map(String::from),
            mode: Mode::Webhook { secret: "test".to_string() },
            service_filter: None,
            repo_path_prefix: None,
            initial_sync: true,
            shepherd_service_name: None,
            #[cfg(feature = "otlp")]
            otlp_endpoint: "http://localhost:4317".to_string(),
        }
    }
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
        let service_filter = match env::var("SERVICE_FILTER")
            .ok()
            .filter(|s| !s.is_empty())
        {
            None => None,
            Some(raw) => {
                let services: Vec<String> = raw
                    .split(',')
                    .map(|name| name.trim().to_string())
                    .filter(|name| !name.is_empty())
                    .collect();
                if services.is_empty() {
                    return Err(format!(
                        "SERVICE_FILTER '{raw}' contains no valid service names; \
                         check for stray commas or whitespace"
                    ));
                }
                Some(services)
            }
        };

        let allow_latest_images = env::var("ALLOW_LATEST_IMAGES")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(false);

        let initial_sync = env::var("INITIAL_SYNC")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(true);

        let shepherd_service_name =
            env::var("SHEPHERD_SERVICE_NAME").ok().filter(|s| !s.is_empty());

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
            service_filter,
            repo_path_prefix,
            initial_sync,
            shepherd_service_name,
            #[cfg(feature = "otlp")]
            otlp_endpoint,
        })
    }
}
