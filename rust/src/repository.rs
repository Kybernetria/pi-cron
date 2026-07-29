use crate::{
    schedule::next_occurrence,
    types::{Action, CronJob, JobReplacement, Occurrence},
};
use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use std::{fs, os::unix::fs::PermissionsExt, path::Path};

pub struct Repository {
    conn: Connection,
    memory: bool,
}

impl Repository {
    pub fn open(path: &Path) -> Result<Self, String> {
        let memory = path == Path::new(":memory:");
        if !memory
            && let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(err)?;
            fs::set_permissions(parent, fs::Permissions::from_mode(0o700)).map_err(err)?;
        }
        let conn = Connection::open(path).map_err(err)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;",
        )
        .map_err(err)?;
        let repo = Self { conn, memory };
        repo.migrate()?;
        repo.secure_files(path);
        Ok(repo)
    }
    fn migrate(&self) -> Result<(), String> {
        self.conn.execute_batch(r#"
          CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL);
          INSERT INTO schema_version(version) SELECT 1 WHERE NOT EXISTS (SELECT 1 FROM schema_version);
          CREATE TABLE IF NOT EXISTS jobs (
            id TEXT PRIMARY KEY, name TEXT NOT NULL, enabled INTEGER NOT NULL CHECK(enabled IN (0,1)),
            schedule TEXT NOT NULL, timezone TEXT NOT NULL, action_json TEXT NOT NULL,
            created_at TEXT NOT NULL, updated_at TEXT NOT NULL, next_at TEXT
          );
          CREATE INDEX IF NOT EXISTS jobs_next_at ON jobs(enabled, next_at);
          CREATE TABLE IF NOT EXISTS occurrences (
            id INTEGER PRIMARY KEY AUTOINCREMENT, job_id TEXT NOT NULL,
            scheduled_at TEXT NOT NULL, claimed_at TEXT NOT NULL,
            status TEXT NOT NULL CHECK(status IN ('claimed','completed','skipped')),
            finished_at TEXT, result TEXT, error TEXT,
            UNIQUE(job_id, scheduled_at)
          );
          CREATE INDEX IF NOT EXISTS occurrences_job_history ON occurrences(job_id, id DESC);
        "#).map_err(err)?;
        let version: i64 = self
            .conn
            .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
                row.get(0)
            })
            .map_err(err)?;
        if version != 1 {
            return Err(format!("unsupported database schema version: {version}"));
        }
        Ok(())
    }
    fn secure_files(&self, path: &Path) {
        if self.memory {
            return;
        }
        for suffix in ["", "-wal", "-shm"] {
            let p = format!("{}{suffix}", path.display());
            let _ = fs::set_permissions(p, fs::Permissions::from_mode(0o600));
        }
    }
    pub fn add(&self, input: &JobReplacement, now: DateTime<Utc>) -> Result<CronJob, String> {
        let id = input.id.as_ref().ok_or("job id is required")?;
        let stamp = iso(now);
        let next = if input.enabled {
            Some(iso(next_occurrence(&input.schedule, &input.timezone, now)?))
        } else {
            None
        };
        self.conn.execute("INSERT INTO jobs(id,name,enabled,schedule,timezone,action_json,created_at,updated_at,next_at) VALUES(?,?,?,?,?,?,?,?,?)",
            params![id, input.name, input.enabled as i32, input.schedule, input.timezone, serde_json::to_string(&input.action).map_err(err)?, stamp, stamp, next])
            .map_err(|e| if e.to_string().contains("UNIQUE constraint") { format!("job id already exists: {id}") } else { e.to_string() })?;
        self.get_required(id)
    }
    pub fn replace(&self, input: &JobReplacement, now: DateTime<Utc>) -> Result<CronJob, String> {
        let id = input.id.as_ref().ok_or("job id is required")?;
        if self.get(id)?.is_none() {
            return Err(format!("job not found: {id}"));
        }
        let next = if input.enabled {
            Some(iso(next_occurrence(&input.schedule, &input.timezone, now)?))
        } else {
            None
        };
        let changed = self.conn.execute("UPDATE jobs SET name=?,enabled=?,schedule=?,timezone=?,action_json=?,updated_at=?,next_at=? WHERE id=?",
            params![input.name, input.enabled as i32, input.schedule, input.timezone, serde_json::to_string(&input.action).map_err(err)?, iso(now), next, id]).map_err(err)?;
        if changed != 1 {
            return Err(format!("job not found: {id}"));
        }
        self.get_required(id)
    }
    pub fn delete(&self, id: &str) -> Result<bool, String> {
        self.transaction(|| {
            let deleted = self
                .conn
                .execute("DELETE FROM jobs WHERE id=?", [id])
                .map_err(err)?
                == 1;
            if deleted {
                self.conn
                    .execute(
                        "DELETE FROM occurrences WHERE job_id=? AND status!='claimed'",
                        [id],
                    )
                    .map_err(err)?;
            }
            Ok(deleted)
        })
    }
    pub fn set_enabled(
        &self,
        id: &str,
        enabled: bool,
        now: DateTime<Utc>,
    ) -> Result<CronJob, String> {
        let job = self.get_required(id)?;
        let next = if enabled {
            Some(iso(next_occurrence(&job.schedule, &job.timezone, now)?))
        } else {
            None
        };
        self.conn
            .execute(
                "UPDATE jobs SET enabled=?,updated_at=?,next_at=? WHERE id=?",
                params![enabled as i32, iso(now), next, id],
            )
            .map_err(err)?;
        self.get_required(id)
    }
    pub fn list(&self) -> Result<Vec<CronJob>, String> {
        let mut stmt = self.conn.prepare("SELECT id,name,enabled,schedule,timezone,action_json,created_at,updated_at,next_at FROM jobs ORDER BY name COLLATE NOCASE,id").map_err(err)?;
        let rows = stmt.query_map([], row_job).map_err(err)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(err)
    }
    pub fn get(&self, id: &str) -> Result<Option<CronJob>, String> {
        self.conn.query_row("SELECT id,name,enabled,schedule,timezone,action_json,created_at,updated_at,next_at FROM jobs WHERE id=?", [id], row_job).optional().map_err(err)
    }
    pub fn get_required(&self, id: &str) -> Result<CronJob, String> {
        self.get(id)?.ok_or_else(|| format!("job not found: {id}"))
    }
    pub fn nearest(&self) -> Result<Option<CronJob>, String> {
        self.conn.query_row("SELECT id,name,enabled,schedule,timezone,action_json,created_at,updated_at,next_at FROM jobs WHERE enabled=1 AND next_at IS NOT NULL ORDER BY next_at LIMIT 1", [], row_job).optional().map_err(err)
    }
    pub fn recalculate_all(&self, now: DateTime<Utc>) -> Result<(), String> {
        for job in self.list()? {
            let next = if job.enabled {
                Some(iso(next_occurrence(&job.schedule, &job.timezone, now)?))
            } else {
                None
            };
            self.conn
                .execute(
                    "UPDATE jobs SET next_at=? WHERE id=?",
                    params![next, job.id],
                )
                .map_err(err)?;
        }
        Ok(())
    }
    pub fn advance(&self, id: &str, after: DateTime<Utc>) -> Result<(), String> {
        if let Some(job) = self.get(id)?
            && job.enabled
        {
            let next = iso(next_occurrence(&job.schedule, &job.timezone, after)?);
            self.conn
                .execute("UPDATE jobs SET next_at=? WHERE id=?", params![next, id])
                .map_err(err)?;
        }
        Ok(())
    }
    pub fn skip_interrupted(&self, now: DateTime<Utc>) -> Result<usize, String> {
        self.conn.execute("UPDATE occurrences SET status='skipped',finished_at=?,error='daemon restarted during execution' WHERE status='claimed'", [iso(now)]).map_err(err)
    }
    pub fn claim(
        &self,
        job_id: &str,
        scheduled_at: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<Occurrence>, String> {
        match self.conn.execute("INSERT INTO occurrences(job_id,scheduled_at,claimed_at,status) VALUES(?,?,?,'claimed')", params![job_id, scheduled_at, iso(now)]) {
            Ok(_) => self.get_occurrence(self.conn.last_insert_rowid()),
            Err(e) if e.to_string().contains("UNIQUE constraint") => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }
    pub fn finish(
        &self,
        id: i64,
        status: &str,
        result: Option<&str>,
        error_text: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<Occurrence, String> {
        let changed = self.conn.execute("UPDATE occurrences SET status=?,finished_at=?,result=?,error=? WHERE id=? AND status='claimed'", params![status, iso(now), result, error_text, id]).map_err(err)?;
        if changed != 1 {
            return Err(format!("occurrence {id} is not claimed"));
        }
        let occurrence = self.get_occurrence(id)?.unwrap();
        self.conn.execute("DELETE FROM occurrences WHERE job_id=? AND id NOT IN (SELECT id FROM occurrences WHERE job_id=? ORDER BY id DESC LIMIT 100)", params![occurrence.job_id, occurrence.job_id]).map_err(err)?;
        if self.get(&occurrence.job_id)?.is_none() {
            self.conn
                .execute(
                    "DELETE FROM occurrences WHERE job_id=?",
                    [&occurrence.job_id],
                )
                .map_err(err)?;
        }
        Ok(occurrence)
    }
    pub fn get_occurrence(&self, id: i64) -> Result<Option<Occurrence>, String> {
        self.conn.query_row("SELECT id,job_id,scheduled_at,claimed_at,status,finished_at,result,error FROM occurrences WHERE id=?", [id], row_occurrence).optional().map_err(err)
    }
    pub fn history(&self, id: &str) -> Result<Vec<Occurrence>, String> {
        let mut stmt = self.conn.prepare("SELECT id,job_id,scheduled_at,claimed_at,status,finished_at,result,error FROM occurrences WHERE job_id=? ORDER BY id DESC").map_err(err)?;
        stmt.query_map([id], row_occurrence)
            .map_err(err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(err)
    }
    fn transaction<T>(&self, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        self.conn.execute_batch("BEGIN IMMEDIATE").map_err(err)?;
        match f() {
            Ok(v) => {
                self.conn.execute_batch("COMMIT").map_err(err)?;
                Ok(v)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }
}

fn row_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<CronJob> {
    let action: String = row.get(5)?;
    Ok(CronJob {
        id: row.get(0)?,
        name: row.get(1)?,
        enabled: row.get::<_, i64>(2)? != 0,
        schedule: row.get(3)?,
        timezone: row.get(4)?,
        action: serde_json::from_str::<Action>(&action).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                action.len(),
                rusqlite::types::Type::Text,
                Box::new(e),
            )
        })?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
        next_at: row.get(8)?,
    })
}
fn row_occurrence(row: &rusqlite::Row<'_>) -> rusqlite::Result<Occurrence> {
    Ok(Occurrence {
        id: row.get(0)?,
        job_id: row.get(1)?,
        scheduled_at: row.get(2)?,
        claimed_at: row.get(3)?,
        status: row.get(4)?,
        finished_at: row.get(5)?,
        result: row.get(6)?,
        error: row.get(7)?,
    })
}
pub fn iso(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
}
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn job() -> JobReplacement {
        JobReplacement {
            id: Some("job-1".into()),
            name: "Existing".into(),
            enabled: true,
            schedule: "*/5 * * * *".into(),
            timezone: "UTC".into(),
            action: Action::Protocol {
                target: "test.ok".into(),
                input: serde_json::json!({"v":1}),
            },
        }
    }

    #[test]
    fn opens_existing_schema_and_prevents_duplicate_claims() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("jobs.sqlite");
        let repo = Repository::open(&path).unwrap();
        repo.add(&job(), "2025-01-01T00:00:00Z".parse().unwrap())
            .unwrap();
        let first = repo
            .claim("job-1", "2025-01-01T00:05:00.000Z", Utc::now())
            .unwrap()
            .unwrap();
        assert!(
            repo.claim("job-1", "2025-01-01T00:05:00.000Z", Utc::now())
                .unwrap()
                .is_none()
        );
        drop(repo);
        let reopened = Repository::open(&path).unwrap();
        reopened.skip_interrupted(Utc::now()).unwrap();
        assert_eq!(
            reopened.get_occurrence(first.id).unwrap().unwrap().status,
            "skipped"
        );
        assert_eq!(reopened.get_required("job-1").unwrap().name, "Existing");
    }

    #[test]
    fn history_is_pruned_to_one_hundred() {
        let repo = Repository::open(Path::new(":memory:")).unwrap();
        repo.add(&job(), Utc::now()).unwrap();
        for index in 0..105 {
            let occurrence = repo
                .claim("job-1", &format!("manual:{index}"), Utc::now())
                .unwrap()
                .unwrap();
            repo.finish(occurrence.id, "completed", Some("ok"), None, Utc::now())
                .unwrap();
        }
        assert_eq!(repo.history("job-1").unwrap().len(), 100);
    }
}
