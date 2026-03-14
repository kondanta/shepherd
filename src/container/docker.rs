use color_eyre::Result;
use eyre::{WrapErr, eyre};
use std::path::Path;
use tokio::process::Command as TokioCommand;

pub struct DockerClient {
    docker_bin: String,
}

impl DockerClient {
    pub async fn new() -> Result<Self> {
        let docker_bin = Self::find_executable("docker")?;
        Self::verify_compose_available().await?;
        Ok(Self { docker_bin })
    }

    fn find_executable(name: &str) -> Result<String> {
        which::which(name)
            .wrap_err_with(|| format!("Failed to find executable: {}", name))
            .map(|p| p.to_string_lossy().to_string())
    }

    async fn verify_compose_available() -> Result<()> {
        let ok = TokioCommand::new("docker")
            .args(["compose", "version"])
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false);

        if ok {
            Ok(())
        } else {
            Err(eyre!("Failed to find docker compose plugin"))
        }
    }

    pub async fn pull_image(&self, image: &str) -> Result<()> {
        tracing::info!("Pulling image: {}", image);

        let output = TokioCommand::new(&self.docker_bin)
            .args(["pull", image])
            .output()
            .await
            .wrap_err("Failed to execute docker pull command")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(eyre!("Docker pull failed: {}", stderr));
        }

        tracing::info!("Successfully pulled image: {}", image);
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

        let file_name = compose_file
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| {
                eyre!("Compose file path is not valid UTF-8: {compose_file:?}")
            })?;

        let mut cmd = TokioCommand::new(&self.docker_bin);
        cmd.current_dir(compose_dir)
            .arg("compose")
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
    use std::process::Command;

    #[tokio::test]
    #[ignore] // Requires docker to pull the image, which may not be available in all test environments
    async fn test_pull_image() {
        let client = DockerClient::new().await.unwrap();
        assert!(client.pull_image("alpine:latest").await.is_ok());

        Command::new("docker")
            .args(["rmi", "alpine:latest"])
            .output()
            .expect("Failed to remove test image");
    }

    #[tokio::test]
    #[ignore = "requires docker to be installed"]
    async fn test_find_docker() {
        assert!(DockerClient::find_executable("docker").is_ok());
    }

    #[tokio::test]
    #[ignore = "requires docker to be installed"]
    async fn test_verify_compose_available() {
        let result = DockerClient::verify_compose_available().await;
        assert!(result.is_ok(), "docker compose not available: {:?}", result);
    }
}
