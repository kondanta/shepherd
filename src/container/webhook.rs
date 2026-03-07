use serde::{Deserialize, Serialize};
use std::env;

/// GitHub push webhook payload (simplified)
/// Full schema: https://docs.github.com/en/webhooks/webhook-events-and-payloads#push
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WebhookPayload {
    #[serde(rename = "ref")]
    pub git_ref: String, // e.g., "refs/heads/main"

    pub before: String, // SHA before push
    pub after: String,  // SHA after push (head commit)

    pub repository: Repository,
    pub pusher: Pusher,

    #[serde(default)]
    pub commits: Vec<Commit>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Repository {
    pub id: u64,
    pub name: String,
    pub full_name: String, // e.g., "username/repo"

    #[serde(rename = "clone_url")]
    pub clone_url: String,

    #[serde(rename = "ssh_url")]
    pub ssh_url: String,

    #[serde(rename = "default_branch")]
    pub default_branch: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Pusher {
    pub name: String,
    pub email: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Commit {
    pub id: String, // SHA
    pub message: String,
    pub timestamp: String,
    pub author: Author,

    #[serde(default)]
    pub added: Vec<String>,

    #[serde(default)]
    pub removed: Vec<String>,

    #[serde(default)]
    pub modified: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Author {
    pub name: String,
    pub email: String,
    pub username: Option<String>,
}

/// Event type from X-GitHub-Event header
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebhookEvent {
    Push,
    PullRequest,
    Unknown(String),
}

impl WebhookEvent {
    pub fn from_header(value: &str) -> Self {
        match value {
            "push" => Self::Push,
            "pull_request" => Self::PullRequest,
            other => Self::Unknown(other.to_string()),
        }
    }
}

impl WebhookPayload {
    /// Check if this push is to the default branch (usually main/master)
    pub fn is_default_branch(&self) -> bool {
        self.git_ref == format!("refs/heads/{}", self.repository.default_branch)
    }

    /// Check if this looks like a Renovate commit
    pub fn is_renovate_commit(&self) -> bool {
        // People like me might have their own renovate bots with different names. So it should be configurable.
        let renovate_name = env::var("RENOVATE_USERNAME").unwrap(); // Default to "renovate" if not set
        let renovate_email = env::var("RENOVATE_EMAIL").unwrap(); // Default to "renovate" if not set

        self.commits.iter().any(|c| {
            c.author
                .name
                .to_lowercase()
                .contains(renovate_name.as_str())
                || c.author
                    .email
                    .to_lowercase()
                    .contains(renovate_email.as_str())
        })
    }

    /// Get the branch name (without "refs/heads/" prefix)
    pub fn branch_name(&self) -> Option<&str> {
        self.git_ref.strip_prefix("refs/heads/")
    }

    /// Check if any docker-compose files were modified
    pub fn has_compose_changes(&self) -> bool {
        self.commits.iter().any(|c| {
            c.modified
                .iter()
                .chain(c.added.iter())
                .any(|f| f.ends_with(".yaml") || f.ends_with(".yml"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_webhook_payload() {
        // App normally sets these env vars, but we need to set them here for the test to work
        unsafe {
            env::set_var("RENOVATE_USERNAME", "renovate");
            env::set_var("RENOVATE_EMAIL", "renovate");
        }
        let json = r#"{
            "ref": "refs/heads/main",
            "before": "abc123",
            "after": "def456",
            "repository": {
                "id": 123,
                "name": "my-app",
                "full_name": "user/my-app",
                "clone_url": "https://github.com/user/my-app.git",
                "ssh_url": "git@github.com:user/my-app.git",
                "default_branch": "main"
            },
            "pusher": {
                "name": "renovate[bot]",
                "email": "renovate@github.com"
            },
            "commits": [
                {
                    "id": "def456",
                    "message": "Update nginx Docker tag to v1.25",
                    "timestamp": "2024-01-01T00:00:00Z",
                    "author": {
                        "name": "renovate[bot]",
                        "email": "renovate@github.com",
                        "username": "renovate"
                    },
                    "added": [],
                    "removed": [],
                    "modified": ["docker-compose.yaml"]
                }
            ]
        }"#;

        let payload: WebhookPayload = serde_json::from_str(json).unwrap();

        assert_eq!(payload.repository.name, "my-app");
        assert_eq!(payload.branch_name(), Some("main"));
        assert!(payload.is_default_branch());
        assert!(payload.is_renovate_commit());
        assert!(payload.has_compose_changes());
    }

    #[test]
    fn test_webhook_event_parsing() {
        assert_eq!(WebhookEvent::from_header("push"), WebhookEvent::Push);
        assert_eq!(
            WebhookEvent::from_header("pull_request"),
            WebhookEvent::PullRequest
        );
        assert_eq!(
            WebhookEvent::from_header("issues"),
            WebhookEvent::Unknown("issues".to_string())
        );
    }
}
