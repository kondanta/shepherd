use serde::Deserialize;
use std::collections::HashSet;

/// GitHub push webhook payload (simplified)
/// Full schema: https://docs.github.com/en/webhooks/webhook-events-and-payloads#push
#[derive(Debug, Clone, Deserialize)]
pub struct WebhookPayload {
    #[serde(rename = "ref")]
    pub git_ref: String, // e.g., "refs/heads/main"

    pub after: String, // SHA after push (head commit)

    pub repository: Repository,

    #[serde(default)]
    pub commits: Vec<Commit>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Repository {
    pub full_name: String, // e.g., "username/repo"

    #[serde(rename = "default_branch")]
    pub default_branch: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Commit {
    pub author: Author,

    #[serde(default)]
    pub added: Vec<String>,

    #[serde(default)]
    pub modified: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Author {
    pub name: String,
    pub email: String,
    pub username: Option<String>,
}

impl Repository {
    /// The owner part of `full_name` (e.g. "acme" from "acme/my-app").
    pub fn owner(&self) -> &str {
        self.full_name.split_once('/').map(|(o, _)| o).unwrap_or(&self.full_name)
    }

    /// The repo part of `full_name` (e.g. "my-app" from "acme/my-app").
    pub fn repo_name(&self) -> &str {
        self.full_name.split_once('/').map(|(_, r)| r).unwrap_or(&self.full_name)
    }
}

/// Event type from X-GitHub-Event header.
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

/// Returns true if the commit author matches the expected Renovate identity.
///
/// Checks (in order): GitHub login, display name (case-insensitive), email
/// substring (case-insensitive). The email substring check handles self-hosted
/// Renovate variants that use a domain-prefixed address.
pub(crate) fn is_renovate_author(
    login: Option<&str>,
    name: &str,
    email: &str,
    expected_username: &str,
    expected_email: &str,
) -> bool {
    login.is_some_and(|l| l.eq_ignore_ascii_case(expected_username))
        || name.eq_ignore_ascii_case(expected_username)
        || email.eq_ignore_ascii_case(expected_email)
}

impl WebhookPayload {
    /// Check if this push is to the default branch (usually main/master).
    pub fn is_default_branch(&self) -> bool {
        self.git_ref == format!("refs/heads/{}", self.repository.default_branch)
    }

    /// Returns compose files (*.yaml) touched by Renovate-authored commits only.
    ///
    /// Filtering by author here (rather than as a separate `is_renovate_commit` guard)
    /// prevents human commits in a mixed push from being auto-deployed alongside
    /// legitimate Renovate changes. Returns an empty set when no Renovate commits
    /// touched compose files, which the caller treats as "nothing to do".
    ///
    /// Duplicate delivery of the same webhook is safe: the second pass will
    /// find no config diff after the first already wrote and deployed.
    pub fn modified_compose_files(
        &self,
        username: &str,
        email: &str,
    ) -> HashSet<String> {
        self.commits
            .iter()
            .filter(|c| {
                is_renovate_author(
                    c.author.username.as_deref(),
                    &c.author.name,
                    &c.author.email,
                    username,
                    email,
                )
            })
            .flat_map(|c| c.modified.iter().chain(c.added.iter()))
            .filter(|f| crate::fs::walk::is_compose_file(f))
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_webhook_payload() {
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

        assert_eq!(payload.repository.full_name, "user/my-app");
        assert!(payload.is_default_branch());
        assert!(
            !payload
                .modified_compose_files("renovate", "renovate@github.com")
                .is_empty()
        );
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
