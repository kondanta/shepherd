use crate::container::image::ImageReference;
use color_eyre::Result;
use eyre::{WrapErr, eyre};
use std::path::Path;
use std::process::Command;
use tokio::process::Command as TokioCommand;

pub struct DockerClient {
    docker_bin: String,
}

impl DockerClient {
    pub fn new() -> Result<Self> {
        let docker_bin = Self::find_executable("docker")?;
        // Validate compose plugin is available at startup; we use docker_bin
        // at runtime so there's no separate binary path to store.
        Self::find_compose_binary()?;

        Ok(Self { docker_bin })
    }

    fn find_executable(name: &str) -> Result<String> {
        which::which(name)
            .wrap_err_with(|| format!("Failed to find executable: {}", name))
            .map(|p| p.to_string_lossy().to_string())
    }

    fn find_compose_binary() -> Result<String> {
        let output = Command::new("docker").args(["compose", "version"]).output();

        if output.is_ok() && output.unwrap().status.success() {
            return Ok("docker compose".to_string());
        }

        Err(eyre!("Failed to find docker compose plugin"))
    }

    pub async fn pull_image(&self, image: &ImageReference) -> Result<()> {
        let image_str = image.to_string();

        tracing::info!("Pulling image: {}", image_str);

        let output = TokioCommand::new(&self.docker_bin)
            .args(["pull", &image_str])
            .output()
            .await
            .wrap_err("Failed to execute docker pull command")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(eyre!("Docker pull failed: {}", stderr));
        }

        tracing::info!("Successfully pulled image: {}", image_str);
        Ok(())
    }

    pub async fn restart_compose_service(
        &self,
        compose_file: &Path,
        service_name: &str,
    ) -> Result<()> {
        tracing::info!("Restarting compose service: {}", service_name);

        let compose_dir = compose_file.parent().ok_or_else(|| {
            eyre!("Failed to get parent directory of compose file")
        })?;

        let mut cmd = TokioCommand::new(&self.docker_bin);
        cmd.arg("compose");

        let file_name = compose_file
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| eyre!("Compose file path is not valid UTF-8: {compose_file:?}"))?;

        cmd.current_dir(compose_dir)
            .args(["-f", file_name])
            .args(["up", "-d", "--force-recreate", "--no-deps", service_name]);

        let output = cmd
            .output()
            .await
            .wrap_err("Failed to execute docker compose up command")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(eyre!("docker compose up failed: {}", stderr));
        }

        tracing::info!("Successfully restarted compose service: {}", service_name);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore] // Requires docker to pull the image, which may not be available in all test environments
    async fn test_pull_image() {
        let client = DockerClient::new().unwrap();
        let image = ImageReference::parse("alpine:latest").unwrap();
        assert!(client.pull_image(&image).await.is_ok());

        // cleanup
        Command::new("docker")
            .args(["rmi", "alpine:latest"])
            .output()
            .expect("Failed to remove test image");
    }

    #[test]
    fn test_find_docker() {
        assert!(DockerClient::find_executable("docker").is_ok());
    }

    #[test]
    fn test_find_compose() {
        let result = DockerClient::find_compose_binary();
        assert!(
            result.is_ok(),
            "Failed to find docker compose: {:?}",
            result
        );
    }
}
