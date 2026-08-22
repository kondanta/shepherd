use crate::config::Config;
use crate::container::github::GitHubProvider;
use crate::container::{
    DeploymentOrchestrator,
    docker::{DockerClient, DockerExecutor},
    github::GitHubClient,
};
use crate::features::SharedFlags;
use crate::fs::walk::is_compose_file;
use color_eyre::Result;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

pub struct Poller<D: DockerExecutor = DockerClient, G: GitHubProvider = GitHubClient>
{
    orchestrator: Arc<DeploymentOrchestrator<D, G>>,
    github: Arc<G>,
    config: Arc<Config>,
    owner: String,
    repo: String,
    branch: String,
    interval: Duration,
    flags: SharedFlags,
    last_sha: Mutex<Option<String>>,
}

impl<D: DockerExecutor, G: GitHubProvider> Poller<D, G> {
    pub fn new(
        orchestrator: Arc<DeploymentOrchestrator<D, G>>,
        github_client: Arc<G>,
        flags: SharedFlags,
        config: Arc<Config>,
    ) -> Self {
        let (owner, repo, branch, interval) = match &config.mode {
            crate::config::Mode::Poll { repo, interval_secs, branch } => {
                let (o, r) = repo
                    .split_once('/')
                    .expect("poll_repo format validated at config load");
                (
                    o.to_string(),
                    r.to_string(),
                    branch.clone(),
                    Duration::from_secs(*interval_secs),
                )
            }
            crate::config::Mode::Webhook { .. } => {
                unreachable!("Poller::new called in webhook mode")
            }
        };
        Self {
            github: github_client,
            owner,
            repo,
            branch,
            interval,
            config,
            flags,
            orchestrator,
            last_sha: Mutex::new(None),
        }
    }

    pub async fn run(&self) {
        let mut ticker = tokio::time::interval(self.interval);
        // Skip missed ticks rather than firing them all at once after a slow poll.
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            if let Err(e) = self.poll_once().await {
                tracing::error!("Poll cycle failed: {e:?}");
            }
        }
    }

    #[tracing::instrument(
        skip(self),
        fields(repo = %format!("{}/{}", self.owner, self.repo))
    )]
    async fn poll_once(&self) -> Result<()> {
        let last = self.last_sha.lock().await.clone();

        let (head, commits) = self
            .github
            .commits_since(&self.owner, &self.repo, &self.branch, last.as_deref())
            .await?;

        if head.is_empty() {
            tracing::debug!("No commits found in repository");
            return Ok(()); // empty or inaccessible repo
        }

        // Always advance last_sha before doing any work so that a failure
        // doesn't cause the same commits to be re-processed on every poll.
        *self.last_sha.lock().await = Some(head.clone());

        if last.is_none() {
            // First startup: sync the full current state of the repo rather
            // than replaying commit history. The diff machinery handles
            // missing files, changed files, and already-current files.
            tracing::info!(sha = %head, "First poll — syncing repo state from HEAD");
            return self.sync_from_head(&head).await;
        }

        if commits.is_empty() {
            tracing::debug!("No new commits");
            return Ok(());
        }

        if self.flags.load().deployments_paused {
            tracing::info!(
                "Deployments paused; skipping {} new commit(s)",
                commits.len()
            );
            return Ok(());
        }

        for commit in &commits {
            if !self.is_renovate(commit) {
                tracing::debug!(sha = %commit.sha, "Skipping non-Renovate commit");
                continue;
            }

            let files = self
                .github
                .get_commit_files(&self.owner, &self.repo, &commit.sha)
                .await?;

            let compose_files: Vec<String> =
                files.into_iter().filter(|f| is_compose_file(f)).collect();

            if compose_files.is_empty() {
                continue;
            }

            tracing::info!(
                sha = %commit.sha,
                ?compose_files,
                "Processing Renovate commit from poll"
            );

            if let Err(e) = self
                .orchestrator
                .process_push(&self.owner, &self.repo, &commit.sha, &compose_files)
                .await
            {
                tracing::error!(sha = %commit.sha, "Failed to process commit: {e:?}");
            }
        }

        Ok(())
    }

    /// Fetch every YAML file in the repo at `sha` and run them through the
    /// normal sync + diff path. Used on first startup to bring the local
    /// filesystem in line with the current repo state.
    async fn sync_from_head(&self, sha: &str) -> Result<()> {
        let files =
            self.github.list_repo_files(&self.owner, &self.repo, sha).await?;

        let prefix = self.config.repo_path_prefix.as_deref().unwrap_or("");
        let compose_files: Vec<String> = files
            .into_iter()
            .filter(|f| {
                (prefix.is_empty() || f.starts_with(prefix)) && is_compose_file(f)
            })
            .collect();

        if compose_files.is_empty() {
            tracing::info!("No YAML files found in repository");
            return Ok(());
        }

        tracing::info!(count = compose_files.len(), "Syncing YAML files from HEAD");
        self.orchestrator
            .process_push(&self.owner, &self.repo, sha, &compose_files)
            .await
    }

    fn is_renovate(&self, commit: &crate::container::github::PollCommit) -> bool {
        crate::container::webhook::is_renovate_author(
            commit.author_login.as_deref(),
            &commit.author_name,
            &commit.author_email,
            &self.config.renovate_username,
            &self.config.renovate_email,
        )
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{
        config::{self, Config},
        container::{
            docker::CapturingDockerExecutor,
            github::{FakeGitHubClient, PollCommit},
        },
        features,
    };
    use std::sync::{Arc, Mutex};

    fn poll_config() -> Config {
        Config {
            mode: config::Mode::Poll {
                repo: "owner/repo".to_string(),
                interval_secs: 60,
                branch: "main".to_string(),
            },
            ..Config::for_test(None)
        }
    }

    async fn fake_poller() -> (
        Poller<CapturingDockerExecutor, FakeGitHubClient>,
        Arc<FakeGitHubClient>,
        Arc<Mutex<Vec<crate::container::docker::DockerCall>>>,
    ) {
        let config = Arc::new(poll_config());
        let github = Arc::new(FakeGitHubClient::default());
        let executor = CapturingDockerExecutor::default();
        let calls = executor.calls.clone();
        let orchestrator = Arc::new(
            DeploymentOrchestrator::with_executor(
                Arc::clone(&config),
                Arc::clone(&github),
                features::new_flags(),
                executor,
            )
            .await
            .unwrap(),
        );

        let poller = Poller::new(
            Arc::clone(&orchestrator),
            Arc::clone(&github),
            features::new_flags(),
            Arc::clone(&config),
        );

        (poller, github, calls)
    }

    #[tokio::test]
    async fn poll_once_no_commit_is_noop() {
        let (poller, _github, _calls) = fake_poller().await;

        // Commit queue is empty("a"*40, [])
        // Nothing to process
        poller.poll_once().await.unwrap();
    }

    #[tokio::test]
    async fn poll_once_non_renovate_commit_is_ignored() {
        let (poller, github, _calls) = fake_poller().await;

        github.commits_queue.lock().unwrap().push_back((
            "b".repeat(40),
            vec![PollCommit {
                sha: "c".repeat(40),
                author_email: "some@example.com".to_string(),
                author_name: "some person".to_string(),
                author_login: None,
            }],
        ));

        poller.poll_once().await.unwrap();

        // No compose files, nothing to process
        assert!(github.commit_files.lock().unwrap().is_empty())
    }

    #[tokio::test]
    async fn poll_once_renovate_commit_triggers_deploy() {
        let (poller, github, calls) = fake_poller().await;
        let commit_sha = "d".repeat(40);

        // We need to init last_sha via sync_from_head
        github.commits_queue.lock().unwrap().push_back(("a".repeat(40), vec![]));
        poller.poll_once().await.unwrap();

        // Second poll: we have previous sha, this is what triggers the commit processing path
        github.commits_queue.lock().unwrap().push_back((
            "e".repeat(40),
            vec![PollCommit {
                sha: commit_sha.clone(),
                author_name: "renovate[bot]".to_string(),
                author_email: "renovate[bot]@users.noreply.github.com".to_string(),
                author_login: Some("renovate[bot]".to_string()),
            }],
        ));
        github
            .commit_files
            .lock()
            .unwrap()
            .insert(commit_sha.clone(), vec!["docker-compose.yaml".to_string()]);
        github.file_content.lock().unwrap().insert(
            "docker-compose.yaml".to_string(),
            "services:\n    web:\n       image: nginx:1.25.0\n".to_string(),
        );

        poller.poll_once().await.unwrap();

        let recorded_calls = calls.lock().unwrap();
        assert!(!recorded_calls.is_empty(), "expected a Docker deploy call");
    }
}
