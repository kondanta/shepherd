use color_eyre::Result;
use eyre::{WrapErr, eyre};
use std::{future::Future, path::Path, process::Stdio, time::Duration};
use tokio::{process::Command as TokioCommand, time::timeout};

// Per-operation ceiling: kills the docker child on expiry via kill_on_drop(true).
const DOCKER_COMMAND_TIMEOUT: Duration = Duration::from_secs(600);

pub trait DockerExecutor: Clone + Send + Sync + 'static {
    fn pull_image(&self, image: &str) -> impl Future<Output = Result<()>> + Send;
    fn restart_compose_service(
        &self,
        compose_file: &Path,
        service_name: &str,
    ) -> impl Future<Output = Result<()>> + Send;
    fn compose_up_service(
        &self,
        compose_file: &Path,
        service_name: &str,
    ) -> impl Future<Output = Result<()>> + Send;
}

#[derive(Clone)]
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

    pub(crate) async fn verify_compose_available() -> Result<()> {
        let ok = TokioCommand::new("docker")
            .args(["compose", "version"])
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false);

        if ok { Ok(()) } else { Err(eyre!("Failed to find docker compose plugin")) }
    }

    #[tracing::instrument(skip(self), fields(image = %image))]
    pub async fn pull_image(&self, image: &str) -> Result<()> {
        let child = TokioCommand::new(&self.docker_bin)
            .args(["pull", image])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .wrap_err("Failed to spawn docker pull")?;

        let output = timeout(DOCKER_COMMAND_TIMEOUT, child.wait_with_output())
            .await
            .map_err(|_| {
                eyre!(
                    "docker pull timed out after {}s",
                    DOCKER_COMMAND_TIMEOUT.as_secs()
                )
            })?
            .wrap_err("Failed to execute docker pull command")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(eyre!("Docker pull failed: {}", stderr));
        }

        Ok(())
    }

    #[tracing::instrument(skip(self), fields(service = %service_name, file = ?compose_file))]
    pub async fn restart_compose_service(
        &self,
        compose_file: &Path,
        service_name: &str,
    ) -> Result<()> {
        self.compose_up(compose_file, service_name, true).await
    }

    #[tracing::instrument(skip(self), fields(service = %service_name, file = ?compose_file))]
    pub async fn compose_up_service(
        &self,
        compose_file: &Path,
        service_name: &str,
    ) -> Result<()> {
        self.compose_up(compose_file, service_name, false).await
    }

    async fn compose_up(
        &self,
        compose_file: &Path,
        service_name: &str,
        force_recreate: bool,
    ) -> Result<()> {
        let compose_dir = compose_file.parent().ok_or_else(|| {
            eyre!("Failed to get parent directory of compose file")
        })?;

        let file_name =
            compose_file.file_name().and_then(|n| n.to_str()).ok_or_else(|| {
                eyre!("Compose file path is not valid UTF-8: {compose_file:?}")
            })?;

        let mut up_args = vec!["up", "-d", "--no-deps"];
        if force_recreate {
            up_args.push("--force-recreate");
        }
        up_args.push(service_name);

        let child = TokioCommand::new(&self.docker_bin)
            .current_dir(compose_dir)
            .arg("compose")
            .args(["-f", file_name])
            .args(&up_args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .wrap_err("Failed to spawn docker compose up")?;

        let output = timeout(DOCKER_COMMAND_TIMEOUT, child.wait_with_output())
            .await
            .map_err(|_| {
                eyre!(
                    "docker compose up timed out after {}s",
                    DOCKER_COMMAND_TIMEOUT.as_secs()
                )
            })?
            .wrap_err("Failed to execute docker compose up command")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(eyre!("docker compose up failed: {}", stderr));
        }

        Ok(())
    }
}

impl DockerExecutor for DockerClient {
    fn pull_image(&self, image: &str) -> impl Future<Output = Result<()>> + Send {
        let image = image.to_owned();
        async move { self.pull_image(&image).await }
    }

    fn restart_compose_service(
        &self,
        compose_file: &Path,
        service_name: &str,
    ) -> impl Future<Output = Result<()>> + Send {
        let path = compose_file.to_owned();
        let name = service_name.to_owned();
        async move { self.restart_compose_service(&path, &name).await }
    }

    fn compose_up_service(
        &self,
        compose_file: &Path,
        service_name: &str,
    ) -> impl Future<Output = Result<()>> + Send {
        let path = compose_file.to_owned();
        let name = service_name.to_owned();
        async move { self.compose_up_service(&path, &name).await }
    }
}

#[cfg(test)]
#[allow(dead_code)] // This enum is testing only. I'll keep the variables as is.
#[derive(Debug, Clone)]
pub enum DockerCall {
    PullImage(String),
    RestartService { path: std::path::PathBuf, service: String },
    ComposeUp { path: std::path::PathBuf, service: String },
}

#[cfg(test)]
use std::sync::{Arc, Mutex};

#[cfg(test)]
#[derive(Clone, Default)]
pub struct CapturingDockerExecutor {
    pub calls: Arc<Mutex<Vec<DockerCall>>>,
}

#[cfg(test)]
impl CapturingDockerExecutor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn calls(&self) -> Vec<DockerCall> {
        self.calls.lock().unwrap().clone()
    }
}

#[cfg(test)]
impl DockerExecutor for CapturingDockerExecutor {
    fn pull_image(&self, image: &str) -> impl Future<Output = Result<()>> + Send {
        let image = image.to_owned();
        async move {
            self.calls.lock().unwrap().push(DockerCall::PullImage(image));
            Ok(())
        }
    }

    fn restart_compose_service(
        &self,
        compose_file: &Path,
        service_name: &str,
    ) -> impl Future<Output = Result<()>> + Send {
        let path = compose_file.to_owned();
        let name = service_name.to_owned();
        async move {
            self.calls
                .lock()
                .unwrap()
                .push(DockerCall::RestartService { path, service: name });
            Ok(())
        }
    }

    fn compose_up_service(
        &self,
        compose_file: &Path,
        service_name: &str,
    ) -> impl Future<Output = Result<()>> + Send {
        let path = compose_file.to_owned();
        let name = service_name.to_owned();
        async move {
            self.calls
                .lock()
                .unwrap()
                .push(DockerCall::ComposeUp { path, service: name });
            Ok(())
        }
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

    // Instead of relying on underlying host to provide container runtime, we use mocked version
    #[tokio::test]
    async fn test_docker_executor_logic_with_fake() {
        let executor = CapturingDockerExecutor::new();
        let compose_path = std::path::Path::new("docker-compose.yaml");

        executor.pull_image("alpine:latest").await.unwrap();
        executor.compose_up_service(compose_path, "web").await.unwrap();
        executor.compose_up_service(compose_path, "db").await.unwrap();

        let calls = executor.calls();
        assert_eq!(calls.len(), 3);
        assert!(matches!(calls[0], DockerCall::PullImage(_)));
    }
}
