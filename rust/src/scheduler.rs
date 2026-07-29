use crate::{
    repository::{Repository, iso},
    runner::Runner,
    types::{CronJob, ExecutionStatus, Occurrence},
};
use chrono::{DateTime, Utc};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::{Mutex, Notify, oneshot},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Clone)]
pub struct Scheduler {
    repository: Arc<Mutex<Repository>>,
    runner: Runner,
    execution: Arc<Mutex<()>>,
    notify: Arc<Notify>,
    cancel: CancellationToken,
    task: Arc<Mutex<Option<JoinHandle<()>>>>,
    pending: Arc<AtomicUsize>,
    drained: Arc<Notify>,
}
impl Scheduler {
    pub fn new(
        repository: Arc<Mutex<Repository>>,
        runner: Runner,
        cancel: CancellationToken,
    ) -> Self {
        Self {
            repository,
            runner,
            execution: Arc::new(Mutex::new(())),
            notify: Arc::new(Notify::new()),
            cancel,
            task: Arc::new(Mutex::new(None)),
            pending: Arc::new(AtomicUsize::new(0)),
            drained: Arc::new(Notify::new()),
        }
    }
    pub async fn start(&self) -> Result<(), String> {
        {
            let repo = self.repository.lock().await;
            repo.skip_interrupted(Utc::now())?;
            repo.recalculate_all(Utc::now())?;
        }
        let this = self.clone();
        *self.task.lock().await = Some(tokio::spawn(async move {
            this.run_loop().await;
        }));
        Ok(())
    }
    pub fn notify(&self) {
        self.notify.notify_one();
    }
    pub async fn stop(&self) {
        self.cancel.cancel();
        self.notify();
        if let Some(task) = self.task.lock().await.take() {
            let _ = task.await;
        }
        loop {
            let drained = self.drained.notified();
            if self.pending.load(Ordering::Acquire) == 0 {
                break;
            }
            drained.await;
        }
    }
    pub async fn run_now(&self, id: &str) -> Result<Occurrence, String> {
        let (job, occurrence) = {
            let repo = self.repository.lock().await;
            let job = repo.get_required(id)?;
            let scheduled = format!("manual:{}:{}", iso(Utc::now()), Uuid::new_v4());
            let occurrence = repo
                .claim(id, &scheduled, Utc::now())?
                .ok_or("manual occurrence could not be claimed")?;
            (job, occurrence)
        };
        self.enqueue(job, occurrence)
            .await
            .await
            .map_err(|_| "execution task stopped".to_string())?
    }
    async fn run_loop(&self) {
        loop {
            if self.cancel.is_cancelled() {
                break;
            }
            let nearest = self.repository.lock().await.nearest();
            let delay = match nearest {
                Ok(Some(ref job)) => job
                    .next_at
                    .as_deref()
                    .and_then(parse)
                    .map(|at| {
                        let millis = (at - Utc::now()).num_milliseconds().max(0) as u64;
                        Duration::from_millis(millis.min(60_000))
                    })
                    .unwrap_or(Duration::from_secs(60)),
                Ok(None) => Duration::from_secs(24 * 60 * 60),
                Err(e) => {
                    eprintln!("pi-cron scheduler repository error: {e}");
                    Duration::from_secs(1)
                }
            };
            tokio::select! { _ = tokio::time::sleep(delay) => {}, _ = self.notify.notified() => continue, _ = self.cancel.cancelled() => break }
            if let Err(e) = self.wake().await {
                eprintln!("pi-cron scheduler wake error: {e}");
            }
        }
    }
    async fn wake(&self) -> Result<(), String> {
        let now = Utc::now();
        let claimed = {
            let repo = self.repository.lock().await;
            let Some(job) = repo.nearest()? else {
                return Ok(());
            };
            let Some(scheduled_text) = job.next_at.clone() else {
                return Ok(());
            };
            let Some(scheduled) = parse(&scheduled_text) else {
                return Err("invalid next_at in database".into());
            };
            if scheduled > now {
                return Ok(());
            }
            let occurrence = repo.claim(&job.id, &scheduled_text, now)?;
            let missed = (now - scheduled).num_milliseconds() >= 60_000;
            repo.advance(&job.id, if missed { now } else { scheduled })?;
            occurrence.map(|occurrence| (job, occurrence, missed))
        };
        self.notify();
        if let Some((job, occurrence, missed)) = claimed {
            if missed {
                self.repository.lock().await.finish(
                    occurrence.id,
                    "skipped",
                    None,
                    Some("missed occurrence; catch-up is disabled"),
                    now,
                )?;
            } else {
                drop(self.enqueue(job, occurrence).await);
            }
        }
        Ok(())
    }
    async fn enqueue(
        &self,
        job: CronJob,
        occurrence: Occurrence,
    ) -> oneshot::Receiver<Result<Occurrence, String>> {
        let (send, receive) = oneshot::channel();
        let this = self.clone();
        self.pending.fetch_add(1, Ordering::Release);
        tokio::spawn(async move {
            let _ = send.send(this.execute(job, occurrence).await);
            if this.pending.fetch_sub(1, Ordering::AcqRel) == 1 {
                this.drained.notify_waiters();
            }
        });
        receive
    }
    async fn execute(&self, job: CronJob, occurrence: Occurrence) -> Result<Occurrence, String> {
        let _serial = self.execution.lock().await;
        let output = self.runner.execute(&job).await;
        let repo = self.repository.lock().await;
        match output {
            Ok(value) => repo.finish(
                occurrence.id,
                match value.status {
                    ExecutionStatus::Completed => "completed",
                    ExecutionStatus::Skipped => "skipped",
                },
                value.result.as_deref().map(bound).as_deref(),
                value.error.as_deref().map(bound).as_deref(),
                Utc::now(),
            ),
            Err(e) => repo.finish(
                occurrence.id,
                "completed",
                None,
                Some(&bound(&e)),
                Utc::now(),
            ),
        }
    }
}
fn parse(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|v| v.with_timezone(&Utc))
}
fn bound(value: &str) -> String {
    if value.chars().count() <= 40_000 {
        value.into()
    } else {
        format!(
            "{}\n[truncated]",
            value.chars().take(40_000).collect::<String>()
        )
    }
}
