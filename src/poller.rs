use crate::config::Config;
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

pub struct Poller<D: DockerExecutor = DockerClient> {
    orchestrator: Arc<DeploymentOrchestrator<D>>,
    github: Arc<GitHubClient>,
    config: Arc<Config>,
    owner: String,
    repo: String,
    branch: String,
    interval: Duration,
    flags: SharedFlags,
    last_sha: Mutex<Option<String>>,
}

impl<D: DockerExecutor> Poller<D> {
    pub fn new(
        orchestrator: Arc<DeploymentOrchestrator<D>>,
        github_client: Arc<GitHubClient>,
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

