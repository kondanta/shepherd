#[allow(dead_code)]
pub mod docker;
pub mod github;
pub mod image;
pub mod webhook;

pub use docker::DockerClient;
pub use github::GitHubClient;
pub use image::ImageReference;

use crate::fs::walk::ServiceEntry;
use color_eyre::Result;
use serde::Serialize;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use eyre::WrapErr;

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
    history: Vec<Deployment>,
}

impl DeploymentOrchestrator {
    pub fn new(root_dir: PathBuf, github_token: Option<String>) -> Result<Self> {
        Ok(Self {
            docker_client: DockerClient::new()?,
            github_client: GitHubClient::new(github_token),
            root_dir,
            history: Vec::new(),
        })
    }

    async fn update_service(&self, service: &ServiceEntry) -> Result<()> {
        tracing::info!("Updating service: {}", service.name);
        let image_ref = ImageReference::parse(&service.image)?;
        self.docker_client.pull_image(&image_ref).await?;
        self.docker_client
            .restart_compose_service(&service.path, &service.name)
            .await?;
        Ok(())
    }

    fn record(&mut self, deployment: Deployment) {
        self.history.push(deployment);
        if self.history.len() > 200 {
            self.history.remove(0);
        }
    }

    pub fn get_managed_services(&self) -> Result<Vec<ServiceEntry>> {
        crate::fs::walk::scan_filesystem(&self.root_dir)
    }

    pub fn list_deployments(&self) -> Vec<Deployment> {
        self.history.iter().rev().cloned().collect()
    }

    pub async fn handle_webhook(
        &mut self,
        payload: &webhook::WebhookPayload,
        renovate_username: &str,
        renovate_email: &str,
    ) -> Result<()> {
        if !payload.is_default_branch() {
            tracing::info!("Ignoring push to non-default branch");
            return Ok(());
        }
        if !payload.is_renovate_commit(renovate_username, renovate_email) {
            tracing::info!("Ignoring non-Renovate commit");
            return Ok(());
        }
        if !payload.has_compose_changes() {
            tracing::info!("No compose file changes, skipping");
            return Ok(());
        }

        let changed_images: HashSet<String> = payload
            .commits
            .iter()
            .flat_map(|c| parse_updated_images_from_commit(&c.message))
            .collect();

        let modified_files: HashSet<String> = payload
            .commits
            .iter()
            .flat_map(|c| c.modified.iter().chain(c.added.iter()))
            .cloned()
            .collect();

        let services = self.get_managed_services()?;

        for service in &services {
            if !modified_files.iter().any(|f| service.path.ends_with(f.as_str())) {
                continue;
            }

            let image_base = service
                .image
                .split('/')
                .last()
                .unwrap_or(&service.image)
                .split(':')
                .next()
                .unwrap_or(&service.image)
                .to_string();

            if !changed_images.is_empty() && !changed_images.contains(&image_base) {
                continue;
            }

            let service_name = service.name.clone();
            let service_image = service.image.clone();
            let timestamp = now_secs();
            let result = self.update_service(service).await;

            match result {
                Ok(()) => self.record(Deployment {
                    service: service_name,
                    image: service_image,
                    status: DeploymentStatus::Success,
                    timestamp,
                    error: None,
                }),
                Err(e) => {
                    let msg = format!("{e:?}");
                    tracing::error!("Failed to update {service_name}: {msg}");
                    self.record(Deployment {
                        service: service_name,
                        image: service_image,
                        status: DeploymentStatus::Failed,
                        timestamp,
                        error: Some(msg),
                    });
                }
            }
        }

        Ok(())
    }

    pub async fn deploy_service_by_name(&mut self, name: &str) -> Result<()> {
        let services = self.get_managed_services()?;
        let service = services
            .iter()
            .find(|s| s.name == name)
            .ok_or_else(|| color_eyre::eyre::eyre!("Service '{name}' not found"))?;

        let service_name = service.name.clone();
        let service_image = service.image.clone();
        let timestamp = now_secs();
        let result = self.update_service(service).await;

        match result {
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
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn parse_updated_images_from_commit(message: &str) -> Vec<String> {
    let mut images = Vec::new();
    let lower = message.to_lowercase();

    if lower.contains("docker tag") || lower.contains("docker digest") {
        let words: Vec<&str> = message.split_whitespace().collect();
        for (i, word) in words.iter().enumerate() {
            if word.to_lowercase() == "docker" && i > 0 {
                let image = words[i - 1];
                let image_name = image.split(':').next().unwrap_or(image);
                images.push(image_name.to_string());
            }
        }
    }

    images
}
