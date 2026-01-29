use dotenvy::dotenv;
use std::env;

#[derive(Debug)]
pub struct Config {
    pub root_dir: String,
    pub log_level: String,
    #[cfg(feature = "otlp")]
    pub otlp_endpoint: String,
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

        #[cfg(feature = "otlp")]
        {
            let otlp_endpoint = env::var("OTLP_ENDPOINT").unwrap_or_else(|_| {
                eprintln!("Error: OTLP_ENDPOINT environment variable must be set if 'otlp' feature is enabled");
                std::process::exit(1);
            });

            Config {
                root_dir,
                otlp_endpoint,
                log_level,
            }
        }

        #[cfg(not(feature = "otlp"))]
        {
            Config {
                root_dir: root_dir.into(),
                log_level,
            }
        }
    }
}
