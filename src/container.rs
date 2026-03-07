#[allow(dead_code)]
pub mod docker;
pub mod github;
pub mod image;
pub mod webhook;

pub use docker::DockerClient;
pub use image::ImageReference;

use crate::fs::walk::ServiceEntry;
use color_eyre::Result;
use std::path::PathBuf;

pub struct DeploymentManager {
    docker_client: DockerClient,
    // todo: needs github client for fetching compose files from remote repositories
    root_dir: PathBuf,
}

impl DeploymentManager {
    pub fn new(root_dir: PathBuf) -> Result<Self> {
        Ok(Self {
            docker_client: DockerClient::new()?,
            root_dir,
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

    pub fn get_managed_services(&self) -> Result<Vec<ServiceEntry>> {
        crate::fs::walk::scan_filesystem(&self.root_dir)
    }
}
