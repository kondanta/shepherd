pub mod docker;
pub mod github;
pub mod image;
pub mod webhook;

use docker::{DockerClient, DockerExecutor};
use github::GitHubClient;
use image::ImageReference;

use crate::config::Config;
use crate::features::SharedFlags;
use crate::fs::walk::ServiceEntry;
use color_eyre::Result;
use eyre::{WrapErr, eyre};
use serde::Serialize;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Maximum time to wait for the deploy semaphore before returning an error.
/// Prevents indefinite hangs when a deploy stalls (e.g. Docker pull timeout).
const SEMAPHORE_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Copy)]
pub(crate) enum DeployMode {
    /// `docker compose up -d --force-recreate --no-deps` — always recreates
    /// the container, guaranteeing the new image is picked up even if the
    /// compose file didn't change. Used for webhook and manual deploys.
    ForceRecreate,
    /// `docker compose up -d --no-deps` — lets docker compose detect image
    /// drift itself and only recreates when something actually changed. Used
    /// for initial sync so already-correct services are not restarted.
    IdempotentUp,
}

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
pub struct DeploymentOrchestrator<D: DockerExecutor = DockerClient> {
    docker_client: D,
    github_client: Arc<GitHubClient>,
    config: Arc<Config>,
    flags: SharedFlags,
    /// Deployment history, capped at 200 entries. Uses a std Mutex because
    /// the critical section is microseconds — just a VecDeque push.
    history: Mutex<VecDeque<Deployment>>,
    /// Ensures at most one deployment runs at a time while allowing
    /// immediate HTTP responses for incoming webhooks.
    deploy_semaphore: tokio::sync::Semaphore,
}

impl DeploymentOrchestrator<DockerClient> {
    pub async fn new(config: Arc<Config>, flags: SharedFlags) -> Result<Self> {
        let github_client = Arc::new(GitHubClient::new(config.github_token.clone()));
        Self::with_executor(config, github_client, flags, DockerClient::new().await?).await
    }
}

impl<D: DockerExecutor> DeploymentOrchestrator<D> {
    pub async fn with_executor(
        config: Arc<Config>,
        github_client: Arc<GitHubClient>,
        flags: SharedFlags,
        docker: D,
    ) -> Result<Self> {
        Ok(Self {
            docker_client: docker,
            github_client,
            config,
            flags,
            history: Mutex::new(VecDeque::new()),
            deploy_semaphore: tokio::sync::Semaphore::new(1),
        })
    }

    pub fn github_client(&self) -> Arc<GitHubClient> {
        Arc::clone(&self.github_client)
    }

    // ── public API ────────────────────────────────────────────────────────────

    pub async fn get_managed_services(&self) -> Result<Vec<ServiceEntry>> {
        let root = PathBuf::from(&self.config.root_dir);
        let services = tokio::task::spawn_blocking(move || {
            crate::fs::walk::scan_filesystem(&root)
        })
        .await
        .map_err(|e| eyre!("scan_filesystem task panicked: {e}"))??;
        crate::metrics::set_managed_services(services.len());
        Ok(services)
    }

    pub fn list_deployments(&self) -> Vec<Deployment> {
        let history = self.history.lock().expect("history mutex poisoned");
        let entries: Vec<Deployment> = history.iter().rev().cloned().collect();
        drop(history);
        entries
    }

    /// Look up a service by name from the current filesystem state.
    pub async fn find_service(&self, name: &str) -> Result<Option<ServiceEntry>> {
        Ok(self.get_managed_services().await?.into_iter().find(|s| s.name == name))
    }

    /// Pull and restart a service, optionally overriding the image.
    ///
    /// If `image` is provided the compose file is updated on disk before
    /// deploying, so the new tag persists across restarts. Acquires the same
    /// deployment semaphore as `handle_webhook` to prevent races.
    pub async fn deploy_service(
        &self,
        service: ServiceEntry,
        image: Option<String>,
    ) -> Result<()> {
        if !self.is_service_allowed(&service.name) {
            return Err(eyre!(
                "Service '{}' is not in SERVICE_FILTER for this instance",
                service.name
            ));
        }
        let service = match image {
            Some(img) => {
                let image_ref = ImageReference::parse(&img)?;
                check_tag_policy(&service.name, &image_ref, self.config.allow_latest_images)?;
                let (path, name, img_for_write) =
                    (service.path.clone(), service.name.clone(), img.clone());
                tokio::task::spawn_blocking(move || {
                    crate::fs::walk::write_service_image(
                        &path,
                        &name,
                        &img_for_write,
                    )
                })
                .await
                .map_err(|e| eyre!("write_service_image task panicked: {e}"))?
                .wrap_err("Failed to update image in compose file")?;
                tracing::info!(
                    service = %service.name,
                    image = %img,
                    "Updated compose file with new image for manual deploy"
                );
                ServiceEntry { image: img, ..service }
            }
            None => {
                let image_ref = ImageReference::parse(&service.image)?;
                check_tag_policy(&service.name, &image_ref, self.config.allow_latest_images)?;
                service
            }
        };
        let _permit =
            tokio::time::timeout(SEMAPHORE_TIMEOUT, self.deploy_semaphore.acquire())
                .await
                .map_err(|_| eyre!("deploy semaphore timeout after 5 minutes"))?
                .map_err(|_| eyre!("deploy semaphore closed"))?;
        self.execute_and_record(&service, DeployMode::ForceRecreate).await
    }

    /// Pull and apply all services currently on disk using idempotent
    /// `docker compose up -d --no-deps`. Services not in SERVICE_FILTER or
    /// that fail tag policy are skipped. Errors per service are logged and
    /// recorded in history but do not abort remaining services.
    ///
    /// The deploy semaphore is acquired and released per service so that
    /// incoming webhook or manual deploys are not blocked for the full duration
    /// of the sync.
    #[tracing::instrument(skip(self))]
    pub async fn initial_sync(&self) -> Result<()> {
        tracing::info!("Starting initial sync");
        let services = self.get_managed_services().await?;
        let mut queued = 0u32;
        for service in services {
            if !self.is_service_allowed(&service.name) {
                tracing::debug!(service = %service.name, "Skipping: not in SERVICE_FILTER");
                continue;
            }
            let image_ref = match ImageReference::parse(&service.image) {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(service = %service.name, "Skipping: unparseable image '{}': {e:?}", service.image);
                    continue;
                }
            };
            if let Err(e) =
                check_tag_policy(&service.name, &image_ref, self.config.allow_latest_images)
            {
                tracing::warn!("{e}");
                continue;
            }
            let _permit = tokio::time::timeout(
                SEMAPHORE_TIMEOUT,
                self.deploy_semaphore.acquire(),
            )
            .await
            .map_err(|_| eyre!("deploy semaphore timeout after 5 minutes"))?
            .map_err(|_| eyre!("deploy semaphore closed"))?;
            queued += 1;
            if let Err(e) =
                self.execute_and_record(&service, DeployMode::IdempotentUp).await
            {
                tracing::error!(service = %service.name, "Sync failed: {e:?}");
            }
        }
        tracing::info!(services = queued, "Initial sync complete");
        Ok(())
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
        let modified: Vec<String> = payload
            .modified_compose_files(&self.config.renovate_username, &self.config.renovate_email)
            .into_iter()
            .collect();
        if modified.is_empty() {
            tracing::info!("No compose file changes, skipping");
            return Ok(());
        }

        self.process_push(
            payload.repository.owner(),
            payload.repository.repo_name(),
            &payload.after,
            &modified,
        )
        .await
    }

    /// Core deploy path: sync changed compose files from GitHub, diff services,
    /// and restart those that changed.
    ///
    /// Acquires the deploy semaphore, so concurrent calls (webhook + poller,
    /// or two webhook deliveries) are serialized automatically.
    ///
    /// Re-entrant by design: if a file is already up to date, `diff_services`
    /// finds no delta and no restart occurs.
    #[tracing::instrument(
        skip(self, modified_files),
        fields(sha = %sha, repo = %format!("{owner}/{repo}"), count = modified_files.len())
    )]
    pub(crate) async fn process_push(
        &self,
        owner: &str,
        repo: &str,
        sha: &str,
        modified_files: &[String],
    ) -> Result<()> {
        let _permit =
            tokio::time::timeout(SEMAPHORE_TIMEOUT, self.deploy_semaphore.acquire())
                .await
                .map_err(|_| eyre!("deploy semaphore timeout after 5 minutes"))?
                .map_err(|_| eyre!("deploy semaphore closed"))?;

        let mut to_restart: Vec<ServiceEntry> = Vec::new();

        for github_path in modified_files {
            let local_rel = match self.strip_repo_prefix(github_path) {
                Some(p) => p.to_owned(),
                None => {
                    tracing::debug!(
                        github_path,
                        "Skipping file outside repo_path_prefix"
                    );
                    continue;
                }
            };
            match self
                .sync_compose_file(owner, repo, github_path, &local_rel, sha)
                .await
            {
                Ok((old, new)) => to_restart.extend(
                    diff_services(&old, new, self.config.allow_latest_images).into_iter().filter(
                        |s| {
                            if self.is_service_allowed(&s.name) {
                                true
                            } else {
                                tracing::debug!(
                                    service = %s.name,
                                    "Skipping service not in SERVICE_FILTER"
                                );
                                false
                            }
                        },
                    ),
                ),
                Err(e) => tracing::warn!("Failed to sync {github_path}: {e:?}"),
            }
        }

        // Deduplicate by service name — a service may appear in multiple
        // modified compose files within the same push.
        let mut seen = std::collections::HashSet::new();
        to_restart.retain(|s| seen.insert(s.name.clone()));

        // Sort so shepherd itself always deploys last. This ensures sibling
        // services are running before shepherd replaces its own container.
        if let Some(self_name) = &self.config.shepherd_service_name {
            to_restart.sort_by_key(|s| s.name.eq_ignore_ascii_case(self_name));
        }

        for service in to_restart {
            if self
                .config
                .shepherd_service_name
                .as_deref()
                .is_some_and(|n| n.eq_ignore_ascii_case(&service.name))
            {
                tracing::info!(
                    service = %service.name,
                    "Deploying self — process will exit after docker compose up returns"
                );
            }
            if let Err(e) =
                self.execute_and_record(&service, DeployMode::ForceRecreate).await
            {
                tracing::error!("Failed to update {}: {e:?}", service.name);
            }
        }

        Ok(())
    }

    // ── internals ─────────────────────────────────────────────────────────────

    fn strip_repo_prefix<'a>(&self, github_path: &'a str) -> Option<&'a str> {
        strip_repo_prefix_inner(self.config.repo_path_prefix.as_deref(), github_path)
    }

    /// Returns true if the service is allowed to be deployed on this instance.
    /// When no filter is configured, all services are allowed.
    fn is_service_allowed(&self, name: &str) -> bool {
        match &self.config.service_filter {
            None => true,
            Some(filter) => filter.iter().any(|f| f.eq_ignore_ascii_case(name)),
        }
    }

    /// Fetch the file at `sha` from GitHub, write it locally, and return
    /// (old_services, new_services) for diffing.
    ///
    /// `github_path` is the path in the repository (used for the API call).
    /// `local_rel` is the path relative to `root_dir` where the file is written
    /// (already stripped of any `REPO_PATH_PREFIX` by the caller).
    ///
    /// Uses an atomic write (temp file → rename) so a parse failure on the
    /// new content never corrupts the existing local file.
    #[tracing::instrument(skip(self), fields(owner, repo, github_path, sha))]
    async fn sync_compose_file(
        &self,
        owner: &str,
        repo: &str,
        github_path: &str,
        local_rel: &str,
        sha: &str,
    ) -> Result<(Vec<ServiceEntry>, Vec<ServiceEntry>)> {
        // Reject paths with parent-dir components before joining with root_dir.
        // PathBuf::join does not resolve ".." so "../../etc/passwd" would escape
        // the root directory. This is defense-in-depth — HMAC verification
        // already gates who can send webhooks.
        let rel_path = std::path::Path::new(local_rel);
        if rel_path.is_absolute()
            || rel_path.components().any(|c| c == std::path::Component::ParentDir)
        {
            return Err(eyre!("Rejected suspicious file path: {local_rel:?}"));
        }

        let local_path = PathBuf::from(&self.config.root_dir).join(local_rel);

        let old_services = if local_path.exists() {
            crate::fs::walk::parse_yaml_file(&local_path).unwrap_or_default()
        } else {
            vec![]
        };

        let content = self
            .github_client
            .fetch_file_content(owner, repo, github_path, sha)
            .await?;

        // Parse in memory first — if GitHub sends broken YAML we never touch
        // the local file. Then atomic rename (write to .tmp, rename) ensures
        // the local file is always complete: either the old or the new content.
        let new_services = crate::fs::walk::parse_yaml_str(&content, &local_path)
            .wrap_err(
                "New compose file failed to parse; local file left unchanged",
            )?;

        // If a service filter is set and none of the services in this file
        // match it, skip the write entirely — no point creating files or
        // directories for services this instance will never deploy.
        if self.config.service_filter.is_some()
            && !new_services.iter().any(|s| self.is_service_allowed(&s.name))
        {
            tracing::debug!("Skipping file: no services match SERVICE_FILTER");
            return Ok((vec![], vec![]));
        }

        write_atomically(&local_path, content.as_bytes()).await?;
        tracing::info!("Updated local compose file: {local_path:?}");
        Ok((old_services, new_services))
    }

    #[tracing::instrument(skip(self, mode), fields(service = %service.name, image = %service.image))]
    async fn execute_and_record(
        &self,
        service: &ServiceEntry,
        mode: DeployMode,
    ) -> Result<()> {
        let timestamp = now_secs();
        let start = Instant::now();
        match self.update_service(service, mode).await {
            Ok(()) => {
                let elapsed = duration_secs(start.elapsed());
                crate::metrics::deployment_recorded(&service.name, true, elapsed);
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
                let elapsed = duration_secs(start.elapsed());
                crate::metrics::deployment_recorded(&service.name, false, elapsed);
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

    #[tracing::instrument(skip(self, mode), fields(service = %service.name))]
    async fn update_service(
        &self,
        service: &ServiceEntry,
        mode: DeployMode,
    ) -> Result<()> {
        tracing::info!("Updating service: {}", service.name);

        if self.flags.load().dry_run {
            tracing::info!(
                "[dry-run] Would pull {} and {} {}",
                service.image,
                match mode {
                    DeployMode::ForceRecreate => "force-recreate",
                    DeployMode::IdempotentUp => "up (idempotent)",
                },
                service.name
            );
            return Ok(());
        }

        self.docker_client.pull_image(&service.image).await?;
        match mode {
            DeployMode::ForceRecreate => {
                self.docker_client
                    .restart_compose_service(&service.path, &service.name)
                    .await?;
            }
            DeployMode::IdempotentUp => {
                self.docker_client
                    .compose_up_service(&service.path, &service.name)
                    .await?;
            }
        }
        tracing::info!("Service updated successfully: {}", service.name);
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

/// Returns the local-relative path for a GitHub repo path, applying the
/// configured `repo_path_prefix` filter and strip.
///
/// - No prefix set → returns the path unchanged.
/// - Prefix set and path starts with it → returns the stripped remainder.
/// - Prefix set but path does not match → returns `None` (caller skips).
fn strip_repo_prefix_inner<'a>(
    prefix: Option<&str>,
    github_path: &'a str,
) -> Option<&'a str> {
    match prefix {
        None => Some(github_path),
        Some(p) => {
            let stripped = github_path.strip_prefix(p)?;
            Some(stripped.trim_start_matches('/'))
        }
    }
}

/// Writes `content` to `path` atomically via a `.tmp` sibling file.
/// Creates parent directories if they don't exist.
async fn write_atomically(path: &Path, content: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .wrap_err_with(|| format!("Creating dirs for {path:?}"))?;
    }
    let tmp = path.with_extension("tmp");
    tokio::fs::write(&tmp, content)
        .await
        .wrap_err_with(|| format!("Writing temp file {tmp:?}"))?;
    tokio::fs::rename(&tmp, path)
        .await
        .wrap_err_with(|| format!("Renaming {tmp:?} → {path:?}"))
}

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
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

fn duration_secs(d: Duration) -> f64 {
    d.as_secs_f64()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{container::docker::DockerExecutor, fs::walk::ServiceEntry};
    use std::path::PathBuf;

    #[derive(Clone, Default)]
    struct FailingDockerExecutor;

    impl DockerExecutor for FailingDockerExecutor {
        async fn pull_image(&self, _image: &str) -> Result<()> {
            Err(eyre::eyre!("docker pull failed"))
        }

        async fn restart_compose_service(
            &self,
            _compose_file: &Path,
            _service_name: &str,
        ) -> Result<()> {
            Err(eyre::eyre!("restart failed"))
        }

        async fn compose_up_service(
            &self,
            _compose_file: &Path,
            _service_name: &str,
        ) -> Result<()> {
            Err(eyre::eyre!("compose up failed"))
        }
    }

    async fn fake_orchestrator()
    -> DeploymentOrchestrator<crate::container::docker::CapturingDockerExecutor>
    {
        use crate::{
            config::Config,
            container::{docker::CapturingDockerExecutor, github::GitHubClient},
            features,
        };
        let config = std::sync::Arc::new(Config::for_test(None));
        let github_client = std::sync::Arc::new(GitHubClient::new(None));
        DeploymentOrchestrator::with_executor(
            config,
            github_client,
            features::new_flags(),
            CapturingDockerExecutor::new(),
        )
        .await
        .expect("DeploymentOrchestrator::with_executor failed")
    }

    fn push_payload(
        git_ref: &str,
        default_branch: &str,
        commits: Vec<webhook::Commit>,
    ) -> webhook::WebhookPayload {
        webhook::WebhookPayload {
            git_ref: git_ref.to_string(),
            after: "abc123def456".to_string(),
            repository: webhook::Repository {
                full_name: "owner/repo".to_string(),
                default_branch: default_branch.to_string(),
            },
            commits,
        }
    }

    fn renovate_commit(files: Vec<&str>) -> webhook::Commit {
        webhook::Commit {
            author: webhook::Author {
                name: "renovate[bot]".to_string(),
                email: "renovate[bot]@users.noreply.github.com".to_string(),
                username: Some("renovate[bot]".to_string()),
            },
            added: vec![],
            modified: files.into_iter().map(String::from).collect(),
        }
    }

    // ── handle_webhook early exits ────────────────────────────────────────────

    #[tokio::test]
    async fn handle_webhook_ignores_non_default_branch() {
        let orch = fake_orchestrator().await;
        let payload = push_payload(
            "refs/heads/feature/my-branch",
            "main",
            vec![renovate_commit(vec!["compose.yaml"])],
        );
        assert!(orch.handle_webhook(&payload).await.is_ok());
    }

    #[tokio::test]
    async fn handle_webhook_ignores_non_renovate_commit() {
        let orch = fake_orchestrator().await;
        let payload = push_payload(
            "refs/heads/main",
            "main",
            vec![webhook::Commit {
                author: webhook::Author {
                    name: "Alice".to_string(),
                    email: "alice@example.com".to_string(),
                    username: Some("alice".to_string()),
                },
                added: vec![],
                modified: vec!["compose.yaml".to_string()],
            }],
        );
        assert!(orch.handle_webhook(&payload).await.is_ok());
    }

    #[tokio::test]
    async fn handle_webhook_ignores_no_compose_files() {
        let orch = fake_orchestrator().await;
        let payload = push_payload(
            "refs/heads/main",
            "main",
            vec![renovate_commit(vec!["README.md", ".renovaterc.json"])],
        );
        assert!(orch.handle_webhook(&payload).await.is_ok());
    }

    fn make_service(name: &str, image: &str, raw_yaml: &str) -> ServiceEntry {
        ServiceEntry {
            path: PathBuf::from("docker-compose.yaml"),
            name: name.to_string(),
            image: image.to_string(),
            raw_config: serde_yaml::from_str(raw_yaml).unwrap(),
        }
    }

    // ── strip_repo_prefix ─────────────────────────────────────────────────────

    #[test]
    fn strip_prefix_none_passes_through() {
        assert_eq!(
            strip_repo_prefix_inner(None, "a/b/compose.yaml"),
            Some("a/b/compose.yaml")
        );
    }

    #[test]
    fn strip_prefix_matching_strips_and_trims_slash() {
        assert_eq!(
            strip_repo_prefix_inner(
                Some("baremetals/node1"),
                "baremetals/node1/myapp/compose.yaml"
            ),
            Some("myapp/compose.yaml")
        );
    }

    #[test]
    fn strip_prefix_non_matching_returns_none() {
        assert_eq!(
            strip_repo_prefix_inner(
                Some("baremetals/node1"),
                "baremetals/node2/app/compose.yaml"
            ),
            None
        );
    }

    #[test]
    fn strip_prefix_exact_match_returns_empty_str() {
        assert_eq!(
            strip_repo_prefix_inner(Some("baremetals/node1"), "baremetals/node1"),
            Some("")
        );
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
        let old = vec![make_service(
            "web",
            "nginx:1.25",
            "image: nginx:1.25\nmemory: 256m",
        )];
        let new = vec![make_service(
            "web",
            "nginx:1.25",
            "image: nginx:1.25\nmemory: 512m",
        )];
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
    //  -- deploy_service --
    #[tokio::test]
    async fn deploy_service_records_failure_in_history() {
        use crate::{config::Config, container::github::GitHubClient, features};
        let config = std::sync::Arc::new(Config::for_test(None));
        let github_client = std::sync::Arc::new(GitHubClient::new(None));
        let orch = DeploymentOrchestrator::with_executor(
            config,
            github_client,
            features::new_flags(),
            FailingDockerExecutor,
        )
        .await
        .unwrap();
        let svc = make_service("web", "nginx:1.25", "image: nginx:1.25");

        let _ = orch.deploy_service(svc, None).await; // expected to fail

        let history = orch.list_deployments();
        assert_eq!(history.len(), 1);
        assert!(matches!(history[0].status, DeploymentStatus::Failed));
    }

    #[tokio::test]
    async fn deploy_service_blocked_by_service_filter() {
        use crate::{
            config::Config,
            container::{docker::CapturingDockerExecutor, github::GitHubClient},
            features,
        };
        let config = std::sync::Arc::new(Config {
            service_filter: Some(vec!["allowed".to_string()]),
            ..Config::for_test(None)
        });
        let github_client = std::sync::Arc::new(GitHubClient::new(None));
        let executor = CapturingDockerExecutor::new();
        let orch = DeploymentOrchestrator::with_executor(
            config,
            github_client,
            features::new_flags(),
            executor.clone(),
        )
        .await
        .unwrap();
        let svc = make_service("blocked", "nginx:1.25", "image: nginx:1.25");

        let result = orch.deploy_service(svc, None).await;

        assert!(result.is_err());
        assert!(executor.calls().is_empty(), "no docker calls should be made");
    }

    #[tokio::test]
    async fn deploy_service_records_success_in_history() {
        use crate::{
            config::Config,
            container::{
                docker::{CapturingDockerExecutor, DockerCall},
                github::GitHubClient,
            },
            features,
        };
        let executor = CapturingDockerExecutor::new();
        let calls = executor.calls.clone();
        let config = std::sync::Arc::new(Config::for_test(None));
        let github_client = std::sync::Arc::new(GitHubClient::new(None));
        let orch = DeploymentOrchestrator::with_executor(
            config,
            github_client,
            features::new_flags(),
            executor,
        )
        .await
        .unwrap();
        let svc = make_service("web", "nginx:1.25", "image: nginx:1.25");

        orch.deploy_service(svc, None).await.unwrap();

        let history = orch.list_deployments();
        assert_eq!(history.len(), 1);
        assert!(matches!(history[0].status, DeploymentStatus::Success));

        let recorded = calls.lock().unwrap();
        assert!(
            matches!(&recorded[0], DockerCall::PullImage(img) if img == "nginx:1.25")
        );
        assert!(matches!(&recorded[1], DockerCall::RestartService { .. }));
    }
}
