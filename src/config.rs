use dotenvy::dotenv;
use std::env;

#[derive(Debug)]
pub struct Config {
    pub root_dir: String,
    pub log_level: String,
    pub webhook_secret: String,
    #[cfg(feature = "otlp")]
    pub otlp_endpoint: String,

    pub renovate_username: String,
    pub renovate_email: String,
    pub github_token: Option<String>,
    /// When false (default), services using the `latest` tag are skipped.
    pub allow_latest_images: bool,
    /// Static bearer token protecting the /flags/* endpoints. If unset,
    /// those endpoints are accessible without authentication.
    pub api_token: Option<String>,
}

impl Config {
    pub fn load() -> Self {
        // Load .env if it exists; ignore errors
        let _ = dotenv();

        let root_dir = env::var("ROOT_DIR").unwrap_or_else(|_| {
            eprintln!("Error: ROOT_DIR environment variable must be set");
            std::process::exit(1);
        });

        let log_level = env::var("LOG_LEVEL").unwrap_or_else(|_| "info".to_string());

        let renovate_username = env::var("RENOVATE_USERNAME").unwrap_or_else(|_| {
            eprintln!("RENOVATE_USERNAME not set, using default value");
            "renovate".to_string()
        });

        let renovate_email = env::var("RENOVATE_EMAIL").unwrap_or_else(|_| {
            eprintln!("RENOVATE_EMAIL not set, using default value");
            "renovate".to_string()
        });

        let webhook_secret = env::var("WEBHOOK_SECRET").unwrap_or_else(|_| {
            eprintln!("Error: WEBHOOK_SECRET environment variable must be set");
            std::process::exit(1);
        });
        if webhook_secret.is_empty() {
            eprintln!("Error: WEBHOOK_SECRET must not be empty");
            std::process::exit(1);
        }

        let github_token = env::var("GITHUB_TOKEN").ok();
        let api_token = env::var("API_TOKEN").ok();

        let allow_latest_images = env::var("ALLOW_LATEST_IMAGES")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(false);

        #[cfg(feature = "otlp")]
        let otlp_endpoint = env::var("OTLP_ENDPOINT").unwrap_or_else(|_| {
            eprintln!("Error: OTLP_ENDPOINT environment variable must be set if 'otlp' feature is enabled");
            std::process::exit(1);
        });

        Config {
            root_dir,
            log_level,
            renovate_username,
            renovate_email,
            webhook_secret,
            github_token,
            allow_latest_images,
            api_token,
            #[cfg(feature = "otlp")]
            otlp_endpoint,
        }
    }
}
