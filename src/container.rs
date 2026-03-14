pub mod docker;
pub mod github;
pub mod image;
pub mod webhook;

use docker::DockerClient;
use github::GitHubClient;
use image::ImageReference;

use crate::config::Config;
use crate::features::SharedFlags;
use crate::fs::walk::ServiceEntry;
use color_eyre::Result;
use eyre::{WrapErr, eyre};
use serde::Serialize;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DeploymentStatus {
    Success,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct Deployment {
    pub service: String,
    pub image: String,
    pub status: DeploymentStatus,
    pub timestamp: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Orchestrates webhook-driven and manual Docker Compose deployments.
///
/// All public methods take `&self`; mutable state (deployment history) is
/// protected by its own fine-grained `Mutex` so read-only callers never
/// contend with an in-progress deployment.
///
/// Concurrent webhook deliveries are serialized by an internal semaphore —
/// the HTTP response is immediate, but the actual deploy work is queued.
pub struct DeploymentOrchestrator {
    docker_client: DockerClient,
    github_client: GitHubClient,
    root_dir: PathBuf,
    renovate_username: String,
    renovate_email: String,
    allow_latest: bool,
    flags: SharedFlags,
    /// Deployment history, capped at 200 entries. Uses a std Mutex because
    /// the critical section is microseconds — just a VecDeque push.
    history: Mutex<VecDeque<Deployment>>,
    /// Ensures at most one deployment runs at a time while allowing
    /// immediate HTTP responses for incoming webhooks.
    deploy_semaphore: tokio::sync::Semaphore,
}

impl DeploymentOrchestrator {
    pub async fn new(config: &Config, flags: SharedFlags) -> Result<Self> {
        Ok(Self {
            docker_client: DockerClient::new().await?,
            github_client: GitHubClient::new(config.github_token.clone()),
            root_dir: PathBuf::from(&config.root_dir),
            renovate_username: config.renovate_username.clone(),
            renovate_email: config.renovate_email.clone(),
            allow_latest: config.allow_latest_images,
            flags,
            history: Mutex::new(VecDeque::new()),
            deploy_semaphore: tokio::sync::Semaphore::new(1),
        })
    }

    // ── public API ────────────────────────────────────────────────────────────

    pub fn get_managed_services(&self) -> Result<Vec<ServiceEntry>> {
        crate::fs::walk::scan_filesystem(&self.root_dir)
    }

    pub fn list_deployments(&self) -> Vec<Deployment> {
        self.history
            .lock()
            .expect("history mutex poisoned")
            .iter()
            .rev()
            .cloned()
            .collect()
    }

    /// Look up a service by name from the current filesystem state.
    pub fn find_service(&self, name: &str) -> Result<Option<ServiceEntry>> {
        Ok(self
            .get_managed_services()?
            .into_iter()
            .find(|s| s.name == name))
    }

    /// Validate tag policy then pull + restart. Used by the manual deploy path.
    ///
    /// Acquires the same deployment semaphore as `handle_webhook` to prevent
    /// a manual trigger from racing with an in-progress webhook deployment.
    pub async fn deploy_service(&self, service: &ServiceEntry) -> Result<()> {
        let image_ref = ImageReference::parse(&service.image)?;
        check_tag_policy(&service.name, &image_ref, self.allow_latest)?;
        let _permit = self
            .deploy_semaphore
            .acquire()
            .await
            .map_err(|_| eyre!("deploy semaphore closed"))?;
        self.execute_and_record(service).await
    }

    /// Process a GitHub push webhook.
    ///
    /// Callers should invoke this from a spawned task so the HTTP response is
    /// sent before the (potentially slow) deploy begins. Concurrent calls are
    /// serialized by the internal semaphore.
    #[tracing::instrument(
        skip(self, payload),
        fields(sha = %payload.after, repo = %payload.repository.full_name)
    )]
    pub async fn handle_webhook(
        &self,
        payload: &webhook::WebhookPayload,
    ) -> Result<()> {
        if !payload.is_default_branch() {
            tracing::info!("Ignoring push to non-default branch");
            return Ok(());
        }
        if !payload.is_renovate_commit(&self.renovate_username, &self.renovate_email) {
            tracing::info!("Ignoring non-Renovate commit");
            return Ok(());
        }

        let modified = payload.modified_compose_files();
        if modified.is_empty() {
            tracing::info!("No compose file changes, skipping");
            return Ok(());
        }

        // Serialize concurrent deployments. The HTTP response is already sent;
        // this only queues the background work.
        let _permit = self
            .deploy_semaphore
            .acquire()
            .await
            .map_err(|_| eyre!("deploy semaphore closed"))?;

        let owner = payload.repository.owner();
        let repo = payload.repository.repo_name();
        let mut to_restart: Vec<ServiceEntry> = Vec::new();

        for file_path in &modified {
            match self
                .sync_compose_file(owner, repo, file_path, &payload.after)
                .await
            {
                Ok((old, new)) => {
                    to_restart.extend(diff_services(&old, new, self.allow_latest))
                }
                Err(e) => tracing::warn!("Failed to sync {file_path}: {e:?}"),
            }
        }

        // Restarts run sequentially. Independent services across different
        // compose files could theoretically run in parallel, but that would
        // require `self: Arc<Self>` to satisfy `tokio::spawn`'s `'static`
        // bound. Sequential is safe and correct for the current scale.
        for service in to_restart {
            if let Err(e) = self.execute_and_record(&service).await {
                tracing::error!("Failed to update {}: {e:?}", service.name);
            }
        }

        // Duplicate webhook delivery (GitHub retries on timeout) is safe:
        // a re-delivery finds no config diff because the first delivery
        // already wrote and deployed — so no redundant restart occurs.

        Ok(())
    }

    // ── internals ─────────────────────────────────────────────────────────────

    /// Fetch the file at `sha` from GitHub, write it locally, and return
    /// (old_services, new_services) for diffing.
    ///
    /// Uses an atomic write (temp file → rename) so a parse failure on the
    /// new content never corrupts the existing local file.
    async fn sync_compose_file(
        &self,
        owner: &str,
        repo: &str,
        file_path: &str,
        sha: &str,
    ) -> Result<(Vec<ServiceEntry>, Vec<ServiceEntry>)> {
        // Reject paths with parent-dir components before joining with root_dir.
        // PathBuf::join does not resolve ".." so "../../etc/passwd" would escape
        // the root directory. This is defense-in-depth — HMAC verification
        // already gates who can send webhooks.
        if std::path::Path::new(file_path)
            .components()
            .any(|c| c == std::path::Component::ParentDir)
        {
            return Err(eyre!("Rejected suspicious file path: {file_path:?}"));
        }

        let local_path = self.root_dir.join(file_path);

        let old_services = if local_path.exists() {
            crate::fs::walk::parse_yaml_file(&local_path).unwrap_or_default()
        } else {
            vec![]
        };

        let content = self
            .github_client
            .fetch_file_content(owner, repo, file_path, sha)
            .await?;

        if let Some(parent) = local_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .wrap_err_with(|| format!("Creating dirs for {local_path:?}"))?;
        }

        // Parse in memory first — if GitHub sends broken YAML we never touch
        // the local file. Then atomic rename (write to .tmp, rename) ensures
        // the local file is always complete: either the old or the new content.
        let new_services =
            crate::fs::walk::parse_yaml_str(&content, &local_path)
                .wrap_err("New compose file failed to parse; local file left unchanged")?;

        let tmp_path = local_path.with_extension("tmp");
        tokio::fs::write(&tmp_path, content.as_bytes())
            .await
            .wrap_err_with(|| format!("Writing temp file {tmp_path:?}"))?;

        tokio::fs::rename(&tmp_path, &local_path)
            .await
            .wrap_err_with(|| {
                format!("Renaming temp file {tmp_path:?} → {local_path:?}")
            })?;

        tracing::info!("Updated local compose file: {local_path:?}");
        Ok((old_services, new_services))
    }

    async fn execute_and_record(&self, service: &ServiceEntry) -> Result<()> {
        let timestamp = now_secs();
        match self.update_service(service).await {
            Ok(()) => {
                self.record(Deployment {
                    service: service.name.clone(),
                    image: service.image.clone(),
                    status: DeploymentStatus::Success,
                    timestamp,
                    error: None,
                });
                Ok(())
            }
            Err(e) => {
                self.record(Deployment {
                    service: service.name.clone(),
                    image: service.image.clone(),
                    status: DeploymentStatus::Failed,
                    timestamp,
                    error: Some(format!("{e:?}")),
                });
                Err(e)
            }
        }
    }

    async fn update_service(&self, service: &ServiceEntry) -> Result<()> {
        tracing::info!("Updating service: {}", service.name);

        if self.flags.load().dry_run {
            tracing::info!(
                "[dry-run] Would pull {} and restart {}",
                service.image,
                service.name
            );
            return Ok(());
        }

        self.docker_client.pull_image(&service.image).await?;
        self.docker_client
            .restart_compose_service(&service.path, &service.name)
            .await?;
        Ok(())
    }

    fn record(&self, deployment: Deployment) {
        let mut history = self.history.lock().expect("history mutex poisoned");
        history.push_back(deployment);
        if history.len() > 200 {
            history.pop_front();
        }
    }
}

// ── pure functions (standalone for testability) ───────────────────────────────

/// Compare old vs new service configs. Returns services that changed and pass
/// the tag policy. Services that fail policy are warned and dropped.
pub(crate) fn diff_services(
    old: &[ServiceEntry],
    new: Vec<ServiceEntry>,
    allow_latest: bool,
) -> Vec<ServiceEntry> {
    let mut to_deploy = Vec::new();

    for svc in new {
        let prev = old.iter().find(|s| s.name == svc.name);
        if prev.is_some_and(|o| o.raw_config == svc.raw_config) {
            continue;
        }

        let image_ref = match ImageReference::parse(&svc.image) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(
                    "Skipping {}: unparseable image '{}': {e:?}",
                    svc.name,
                    svc.image
                );
                continue;
            }
        };

        if let Err(e) = check_tag_policy(&svc.name, &image_ref, allow_latest) {
            tracing::warn!("{e}");
            continue;
        }

        tracing::info!(
            "Service {} changed (image: {:?} → {}), queuing restart",
            svc.name,
            prev.map(|o| o.image.as_str()),
            svc.image,
        );
        to_deploy.push(svc);
    }

    to_deploy
}

/// Enforce tag policy. Returns `Err` if the tag is blocked; emits a warning
/// for non-semver tags but still allows deployment.
pub(crate) fn check_tag_policy(
    name: &str,
    image_ref: &ImageReference,
    allow_latest: bool,
) -> Result<()> {
    if image_ref.tag.is_latest() && !allow_latest {
        return Err(eyre!(
            "service '{name}' uses 'latest' tag; \
             set ALLOW_LATEST_IMAGES=true to override"
        ));
    }
    if !image_ref.tag.is_semver() {
        tracing::warn!(
            "Service {name}: tag '{}' is not semver — \
             pinning to a version tag is recommended",
            image_ref.tag
        );
    }
    Ok(())
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::walk::ServiceEntry;
    use std::path::PathBuf;

    fn make_service(name: &str, image: &str, raw_yaml: &str) -> ServiceEntry {
        ServiceEntry {
            path: PathBuf::from("docker-compose.yaml"),
            name: name.to_string(),
            image: image.to_string(),
            raw_config: serde_yaml::from_str(raw_yaml).unwrap(),
        }
    }

    // ── check_tag_policy ─────────────────────────────────────────────────────

    #[test]
    fn tag_policy_blocks_latest_when_not_allowed() {
        let img = ImageReference::parse("nginx:latest").unwrap();
        assert!(check_tag_policy("nginx", &img, false).is_err());
    }

    #[test]
    fn tag_policy_allows_latest_when_configured() {
        let img = ImageReference::parse("nginx:latest").unwrap();
        assert!(check_tag_policy("nginx", &img, true).is_ok());
    }

    #[test]
    fn tag_policy_allows_semver() {
        let img = ImageReference::parse("nginx:1.25.3").unwrap();
        assert!(check_tag_policy("nginx", &img, false).is_ok());
    }

    #[test]
    fn tag_policy_allows_non_semver_with_warning() {
        // Non-semver is allowed but should emit a tracing warning (tested
        // only for Ok result here; warning content is an observability concern).
        let img = ImageReference::parse("nginx:stable").unwrap();
        assert!(check_tag_policy("nginx", &img, false).is_ok());
    }

    // ── diff_services ─────────────────────────────────────────────────────────

    #[test]
    fn diff_unchanged_service_is_skipped() {
        let raw = "image: nginx:1.25\nports:\n  - '80:80'";
        let old = vec![make_service("web", "nginx:1.25", raw)];
        let new = vec![make_service("web", "nginx:1.25", raw)];
        assert!(diff_services(&old, new, false).is_empty());
    }

    #[test]
    fn diff_detects_image_change() {
        let old = vec![make_service("web", "nginx:1.24", "image: nginx:1.24")];
        let new = vec![make_service("web", "nginx:1.25", "image: nginx:1.25")];
        let result = diff_services(&old, new, false);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].image, "nginx:1.25");
    }

    #[test]
    fn diff_detects_config_change_beyond_image() {
        let old =
            vec![make_service("web", "nginx:1.25", "image: nginx:1.25\nmemory: 256m")];
        let new =
            vec![make_service("web", "nginx:1.25", "image: nginx:1.25\nmemory: 512m")];
        assert_eq!(diff_services(&old, new, false).len(), 1);
    }

    #[test]
    fn diff_includes_new_services() {
        let old = vec![];
        let new = vec![make_service("web", "nginx:1.25", "image: nginx:1.25")];
        assert_eq!(diff_services(&old, new, false).len(), 1);
    }

    #[test]
    fn diff_drops_latest_tag_when_not_allowed() {
        let old = vec![];
        let new = vec![make_service("web", "nginx:latest", "image: nginx:latest")];
        assert!(diff_services(&old, new, false).is_empty());
    }

    #[test]
    fn diff_keeps_latest_tag_when_allowed() {
        let old = vec![];
        let new = vec![make_service("web", "nginx:latest", "image: nginx:latest")];
        assert_eq!(diff_services(&old, new, true).len(), 1);
    }
}
