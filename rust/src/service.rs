use crate::{
    repository::Repository,
    runner::Runner,
    schedule::next_occurrence,
    scheduler::Scheduler,
    types::{Action, JobReplacement},
};
use chrono::Utc;
use regex::Regex;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Clone)]
pub struct Service {
    pub repository: Arc<Mutex<Repository>>,
    pub scheduler: Scheduler,
    runner: Runner,
}
impl Service {
    pub fn new(repository: Arc<Mutex<Repository>>, scheduler: Scheduler, runner: Runner) -> Self {
        Self {
            repository,
            scheduler,
            runner,
        }
    }
    pub async fn dispatch(&self, method: &str, params: Value) -> Result<Value, String> {
        let object = params.as_object().ok_or("params must be an object")?;
        match method {
            "ping" => Ok(json!({"pid":std::process::id()})),
            "list" => Ok(serde_json::to_value(self.repository.lock().await.list()?).unwrap()),
            "get" => {
                let id = field_string(object, "id")?;
                let repo = self.repository.lock().await;
                Ok(json!({"job":repo.get_required(id)?,"occurrences":repo.history(id)?}))
            }
            "add" | "edit" => {
                let mut replacement: JobReplacement = serde_json::from_value(
                    object.get("job").cloned().ok_or("job must be an object")?,
                )
                .map_err(|e| format!("invalid job: {e}"))?;
                if method == "add" && replacement.id.is_none() {
                    replacement.id = Some(Uuid::new_v4().to_string());
                }
                if method == "edit" && replacement.id.is_none() {
                    return Err("job.id must be a string".into());
                }
                self.validate(&mut replacement).await?;
                let repo = self.repository.lock().await;
                let result = if method == "add" {
                    repo.add(&replacement, Utc::now())?
                } else {
                    repo.replace(&replacement, Utc::now())?
                };
                drop(repo);
                self.scheduler.notify();
                Ok(serde_json::to_value(result).unwrap())
            }
            "delete" => {
                let id = field_string(object, "id")?;
                if !self.repository.lock().await.delete(id)? {
                    return Err(format!("job not found: {id}"));
                }
                self.scheduler.notify();
                Ok(json!({"deleted":true,"id":id}))
            }
            "enable" | "disable" => {
                let id = field_string(object, "id")?;
                let job =
                    self.repository
                        .lock()
                        .await
                        .set_enabled(id, method == "enable", Utc::now())?;
                self.scheduler.notify();
                Ok(serde_json::to_value(job).unwrap())
            }
            "run_now" => Ok(serde_json::to_value(
                self.scheduler.run_now(field_string(object, "id")?).await?,
            )
            .unwrap()),
            _ => Err(format!("unknown daemon method: {method}")),
        }
    }
    async fn validate(&self, input: &mut JobReplacement) -> Result<(), String> {
        let id = input
            .id
            .as_deref()
            .ok_or("id must be 1-128 safe characters")?;
        if !Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$")
            .unwrap()
            .is_match(id)
        {
            return Err("id must be 1-128 safe characters".into());
        }
        input.name = input.name.trim().to_string();
        if input.name.is_empty() {
            return Err("name must not be empty".into());
        }
        next_occurrence(&input.schedule, &input.timezone, Utc::now())?;
        match &mut input.action {
            Action::Chat { prompt, cwd } => {
                if prompt.trim().is_empty() {
                    return Err("chat prompt must not be empty".into());
                }
                if !std::path::Path::new(cwd).is_absolute() {
                    return Err("chat cwd must be absolute".into());
                }
                let canonical = tokio::fs::canonicalize(&cwd)
                    .await
                    .map_err(|_| format!("chat cwd is not an existing directory: {cwd}"))?;
                if !tokio::fs::metadata(&canonical)
                    .await
                    .map_err(|e| e.to_string())?
                    .is_dir()
                {
                    return Err(format!("chat cwd is not an existing directory: {cwd}"));
                }
                *cwd = canonical.to_string_lossy().into_owned();
            }
            Action::Protocol { target, .. } => {
                if !Regex::new(r"^([a-z0-9][a-z0-9_-]*)\.([a-z0-9][a-z0-9_-]*)$")
                    .unwrap()
                    .is_match(target)
                {
                    return Err("protocol target must be one exact node.provide".into());
                }
                if target.starts_with("pi_cron.") {
                    return Err("pi_cron cannot schedule its own control API".into());
                }
                self.runner.validate(&input.action).await?;
            }
        }
        Ok(())
    }
}
fn field_string<'a>(
    object: &'a serde_json::Map<String, Value>,
    key: &str,
) -> Result<&'a str, String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{key} must be a string"))
}
