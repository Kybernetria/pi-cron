use crate::types::{Action, CronJob, ExecutionResult, RunnerResponse};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
};
use tokio_util::sync::CancellationToken;

const RESPONSE_LIMIT: usize = 131_072;
const DIAGNOSTIC_LIMIT: usize = 16_384;

#[derive(Clone)]
pub struct Runner {
    executable: PathBuf,
    args: Vec<String>,
    cancel: CancellationToken,
}
impl Runner {
    pub fn new(cancel: CancellationToken) -> Self {
        if let Some(path) = std::env::var_os("PI_CRON_RUNNER_BIN") {
            Self {
                executable: path.into(),
                args: vec![],
                cancel,
            }
        } else {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .to_path_buf();
            Self {
                executable: root.join("node_modules/.bin/tsx"),
                args: vec![root.join("runner/main.ts").display().to_string()],
                cancel,
            }
        }
    }
    pub async fn validate(&self, action: &Action) -> Result<(), String> {
        let cwd = runner_cwd(action)?;
        let value = self
            .invoke(
                json!({"version":1,"operation":"validate_protocol","action":action}),
                Duration::from_secs(60),
                &cwd,
            )
            .await?;
        if value.get("valid").and_then(Value::as_bool) != Some(true) {
            return Err("runner returned an invalid validation response".into());
        }
        Ok(())
    }
    pub async fn execute(&self, job: &CronJob) -> Result<ExecutionResult, String> {
        let cwd = runner_cwd(&job.action)?;
        let value = self
            .invoke(
                json!({"version":1,"operation":"execute","job":job}),
                execution_timeout(),
                &cwd,
            )
            .await?;
        serde_json::from_value(value).map_err(|e| format!("invalid runner execution response: {e}"))
    }
    async fn invoke(&self, request: Value, timeout: Duration, cwd: &Path) -> Result<Value, String> {
        let mut command = Command::new(&self.executable);
        command
            .args(&self.args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
        let mut child = command
            .spawn()
            .map_err(|e| format!("could not start runner {}: {e}", self.executable.display()))?;
        let pid = child.id();
        let payload = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
        if payload.len() > 1_048_576 {
            kill_group(pid);
            return Err("runner request too large".into());
        }
        if let Err(error) = child.stdin.take().unwrap().write_all(&payload).await {
            kill_group(pid);
            return Err(error.to_string());
        }
        let out_task = tokio::spawn(drain_bounded(child.stdout.take().unwrap(), RESPONSE_LIMIT));
        let err_task = tokio::spawn(drain_bounded(
            child.stderr.take().unwrap(),
            DIAGNOSTIC_LIMIT,
        ));
        let status = tokio::select! {
            result = child.wait() => result.map_err(|e| e.to_string())?,
            _ = tokio::time::sleep(timeout) => { kill_group(pid); let _ = child.wait().await; return Err(format!("runner timed out after {}ms", timeout.as_millis())); },
            _ = self.cancel.cancelled() => { kill_group(pid); let _ = child.wait().await; return Err("runner cancelled during daemon shutdown".into()); },
        };
        kill_group(pid);
        let (stdout, stdout_large) = out_task.await.map_err(|e| e.to_string())?;
        let (stderr, _) = err_task.await.map_err(|e| e.to_string())?;
        if stdout_large {
            return Err("runner response too large".into());
        }
        let response: RunnerResponse = serde_json::from_slice(&stdout).map_err(|e| {
            format!(
                "runner returned invalid JSON ({e}); stderr={}",
                String::from_utf8_lossy(&stderr)
            )
        })?;
        if response.version != 1 {
            return Err("runner returned unsupported protocol version".into());
        }
        if !response.ok {
            return Err(response
                .error
                .map(|e| e.message)
                .unwrap_or_else(|| "runner failed".into()));
        }
        if !status.success() {
            return Err(format!(
                "runner exited {status}; stderr={}",
                String::from_utf8_lossy(&stderr)
            ));
        }
        response
            .result
            .ok_or_else(|| "runner response omitted result".into())
    }
}

async fn drain_bounded(mut reader: impl AsyncRead + Unpin, limit: usize) -> (Vec<u8>, bool) {
    let mut kept = Vec::new();
    let mut total = 0usize;
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                total = total.saturating_add(n);
                if kept.len() < limit {
                    kept.extend_from_slice(&buf[..n.min(limit - kept.len())]);
                }
            }
        }
    }
    (kept, total > limit)
}
fn kill_group(pid: Option<u32>) {
    if let Some(pid) = pid {
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
    }
}
fn runner_cwd(action: &Action) -> Result<PathBuf, String> {
    let path = match action {
        Action::Chat { cwd, .. } => PathBuf::from(cwd),
        Action::Protocol { .. } => std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or("HOME is required for protocol actions")?,
    };
    if !path.is_absolute() {
        return Err(format!("runner cwd must be absolute: {}", path.display()));
    }
    let canonical = std::fs::canonicalize(&path).map_err(|_| {
        format!(
            "runner cwd is not an existing directory: {}",
            path.display()
        )
    })?;
    if !canonical.is_dir() {
        return Err(format!(
            "runner cwd is not an existing directory: {}",
            path.display()
        ));
    }
    Ok(canonical)
}
fn execution_timeout() -> Duration {
    std::env::var("PI_CRON_RUNNER_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|value| *value > 0)
        .map(Duration::from_millis)
        .unwrap_or(Duration::from_secs(15 * 60))
}
