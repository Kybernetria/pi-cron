use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronJob {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub schedule: String,
    pub timezone: String,
    pub action: Action,
    pub created_at: String,
    pub updated_at: String,
    pub next_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum Action {
    Chat { prompt: String, cwd: String },
    Protocol { target: String, input: Value },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JobReplacement {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    pub enabled: bool,
    pub schedule: String,
    pub timezone: String,
    pub action: Action,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Occurrence {
    pub id: i64,
    pub job_id: String,
    pub scheduled_at: String,
    pub claimed_at: String,
    pub status: String,
    pub finished_at: Option<String>,
    pub result: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DaemonRequest {
    pub id: String,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Serialize)]
pub struct DaemonResponse {
    pub id: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<DaemonError>,
}

#[derive(Debug, Serialize)]
pub struct DaemonError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct RunnerResponse {
    pub version: u32,
    pub ok: bool,
    pub result: Option<Value>,
    pub error: Option<RunnerError>,
}
#[derive(Debug, Deserialize)]
pub struct RunnerError {
    pub message: String,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionStatus {
    Completed,
    Skipped,
}
#[derive(Debug, Deserialize)]
pub struct ExecutionResult {
    pub status: ExecutionStatus,
    pub result: Option<String>,
    pub error: Option<String>,
}
