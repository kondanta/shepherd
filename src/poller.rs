use crate::container::{DeploymentOrchestrator, github::GitHubClient};
use crate::features::SharedFlags;
use color_eyre::Result;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

pub struct Poller {
    orchestrator: Arc<DeploymentOrchestrator>,
    github: GitHubClient,
    owner: String,
    repo: String,
    branch: String,
    interval: Duration,
    renovate_username: String,
    renovate_email: String,
    flags: SharedFlags,
    last_sha: Mutex<Option<String>>,
}

impl Poller {
    pub fn new(
        orchestrator: Arc<DeploymentOrchestrator>,
        flags: SharedFlags,
        config: &crate::config::Config,
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
            github: GitHubClient::new(config.github_token.clone()),
            owner,
            repo,
            branch,
            interval,
            renovate_username: config.renovate_username.clone(),
            renovate_email: config.renovate_email.clone(),
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

        // Always advance last_sha so a deploy failure doesn't cause
        // the same commits to be re-processed on every subsequent poll.
        *self.last_sha.lock().await = Some(head);

        if commits.is_empty() {
            tracing::debug!("No new commits");
            return Ok(());
        }

        if last.is_none() {
            tracing::info!(
                "First poll — checking latest commit for pending changes"
            );
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

            let compose_files: Vec<String> = files
                .into_iter()
                .filter(|f| f.ends_with(".yaml") || f.ends_with(".yml"))
                .collect();

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

    fn is_renovate(&self, commit: &crate::container::github::PollCommit) -> bool {
        crate::container::webhook::is_renovate_author(
            commit.author_login.as_deref(),
            &commit.author_name,
            &commit.author_email,
            &self.renovate_username,
            &self.renovate_email,
        )
    }
}
