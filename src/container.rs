pub mod docker;
pub mod github;
pub mod image;
pub mod webhook;

pub use docker::DockerClient;
pub use github::GitHubClient;
pub use image::ImageReference;

use crate::config::Config;
use crate::features::SharedFlags;
use crate::fs::walk::ServiceEntry;
use color_eyre::Result;
use eyre::WrapErr;
use serde::Serialize;
use std::collections::VecDeque;
use std::path::PathBuf;
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

pub struct DeploymentOrchestrator {
    docker_client: DockerClient,
    github_client: GitHubClient,
    root_dir: PathBuf,
    renovate_username: String,
    renovate_email: String,
    allow_latest: bool,
    flags: SharedFlags,
    history: VecDeque<Deployment>,
}

impl DeploymentOrchestrator {
    pub fn new(config: &Config, flags: SharedFlags) -> Result<Self> {
        Ok(Self {
            docker_client: DockerClient::new()?,
            github_client: GitHubClient::new(config.github_token.clone()),
            root_dir: PathBuf::from(&config.root_dir),
            renovate_username: config.renovate_username.clone(),
            renovate_email: config.renovate_email.clone(),
            allow_latest: config.allow_latest_images,
            flags,
            history: VecDeque::new(),
        })
    }

    async fn sync_compose_file(
        &self,
        owner: &str,
        repo: &str,
        file_path: &str,
        sha: &str,
    ) -> Result<(Vec<ServiceEntry>, Vec<ServiceEntry>)> {
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
            std::fs::create_dir_all(parent)
                .wrap_err_with(|| format!("Creating dirs for {local_path:?}"))?;
        }
        std::fs::write(&local_path, content.as_bytes())
            .wrap_err_with(|| format!("Writing {local_path:?}"))?;
        tracing::info!("Updated local compose file: {local_path:?}");

        let new_services =
            crate::fs::walk::parse_yaml_file(&local_path).unwrap_or_default();

        Ok((old_services, new_services))
    }

    fn diff_services(
        &self,
        old: &[ServiceEntry],
        new: Vec<ServiceEntry>,
    ) -> Vec<ServiceEntry> {
        let mut to_deploy = Vec::new();

        for svc in new {
            let prev = old.iter().find(|s| s.name == svc.name);
            let changed = prev.is_none_or(|o| o.raw_config != svc.raw_config);
            if !changed {
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

            if image_ref.tag.is_latest() && !self.allow_latest {
                tracing::warn!(
                    "Skipping {}: image uses 'latest' tag \
                     (set ALLOW_LATEST_IMAGES=true to override)",
                    svc.name
                );
                continue;
            }

            if !image_ref.tag.is_semver() {
                tracing::warn!(
                    "Service {} uses non-semver tag '{}' — \
                     pinning to a version tag is recommended",
                    svc.name,
                    image_ref.tag
                );
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

    async fn execute_and_record(&mut self, service: &ServiceEntry) -> Result<()> {
        let service_name = service.name.clone();
        let service_image = service.image.clone();
        let timestamp = now_secs();

        match self.update_service(service).await {
            Ok(()) => {
                self.record(Deployment {
                    service: service_name,
                    image: service_image,
                    status: DeploymentStatus::Success,
                    timestamp,
                    error: None,
                });
                Ok(())
            }
            Err(e) => {
                let msg = format!("{e:?}");
                self.record(Deployment {
                    service: service_name,
                    image: service_image,
                    status: DeploymentStatus::Failed,
                    timestamp,
                    error: Some(msg.clone()),
                });
                Err(color_eyre::eyre::eyre!(msg))
            }
        }
    }

    async fn update_service(&self, service: &ServiceEntry) -> Result<()> {
        tracing::info!("Updating service: {}", service.name);
        let image_ref = ImageReference::parse(&service.image)?;

        if self.flags.load().dry_run {
            tracing::info!(
                "[dry-run] Would pull {} and restart {}",
                image_ref,
                service.name
            );
            return Ok(());
        }

        self.docker_client.pull_image(&image_ref).await?;
        self.docker_client
            .restart_compose_service(&service.path, &service.name)
            .await?;
        Ok(())
    }

    fn record(&mut self, deployment: Deployment) {
        self.history.push_back(deployment);
        if self.history.len() > 200 {
            self.history.pop_front();
        }
    }

    pub fn get_managed_services(&self) -> Result<Vec<ServiceEntry>> {
        crate::fs::walk::scan_filesystem(&self.root_dir)
    }

    pub fn list_deployments(&self) -> Vec<Deployment> {
        self.history.iter().rev().cloned().collect::<Vec<_>>()
    }

    pub async fn handle_webhook(
        &mut self,
        payload: &webhook::WebhookPayload,
    ) -> Result<()> {
        if !payload.is_default_branch() {
            tracing::info!("Ignoring push to non-default branch");
            return Ok(());
        }
        if !payload.is_renovate_commit(&self.renovate_username, &self.renovate_email)
        {
            tracing::info!("Ignoring non-Renovate commit");
            return Ok(());
        }
        let modified_compose_files = payload.modified_compose_files();
        if modified_compose_files.is_empty() {
            tracing::info!("No compose file changes, skipping");
            return Ok(());
        }
        let owner = payload.repository.owner();
        let repo = payload.repository.repo_name();

        let mut to_restart: Vec<ServiceEntry> = Vec::new();

        for file_path in &modified_compose_files {
            match self
                .sync_compose_file(owner, repo, file_path, &payload.after)
                .await
            {
                Ok((old, new)) => to_restart.extend(self.diff_services(&old, new)),
                Err(e) => tracing::warn!("Failed to sync {file_path}: {e:?}"),
            }
        }

        for service in &to_restart {
            if let Err(e) = self.execute_and_record(service).await {
                tracing::error!("Failed to update {}: {e:?}", service.name);
            }
        }

        Ok(())
    }

    pub async fn deploy_service_by_name(&mut self, name: &str) -> Result<()> {
        let services = self.get_managed_services()?;
        let service = services
            .iter()
            .find(|s| s.name == name)
            .ok_or_else(|| color_eyre::eyre::eyre!("Service '{name}' not found"))?
            .clone();

        let image_ref = ImageReference::parse(&service.image)?;
        if image_ref.tag.is_latest() && !self.allow_latest {
            return Err(color_eyre::eyre::eyre!(
                "Service '{}' uses 'latest' tag; set ALLOW_LATEST_IMAGES=true to override",
                name
            ));
        }
        if !image_ref.tag.is_semver() {
            tracing::warn!(
                "Service {name} uses non-semver tag '{}' — \
                 pinning to a version tag is recommended",
                image_ref.tag
            );
        }

        self.execute_and_record(&service).await
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
