use serde::Serialize;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug)]
pub struct AgentErrorLogger {
    path: PathBuf,
    write_lock: Mutex<()>,
}

impl AgentErrorLogger {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            write_lock: Mutex::new(()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append(&self, entry: &AgentErrorLogEntry) -> Result<(), String> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| "agent error log lock poisoned".to_owned())?;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "could not create agent error log directory `{}`: {error}",
                    parent.display()
                )
            })?;
        }
        let serialized = serde_json::to_vec(entry)
            .map_err(|error| format!("could not serialize agent error log entry: {error}"))?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|error| {
                format!(
                    "could not open agent error log `{}`: {error}",
                    self.path.display()
                )
            })?;
        file.write_all(&serialized).map_err(|error| {
            format!(
                "could not write agent error log `{}`: {error}",
                self.path.display()
            )
        })?;
        file.write_all(b"\n").map_err(|error| {
            format!(
                "could not finalize agent error log `{}`: {error}",
                self.path.display()
            )
        })?;
        file.flush().map_err(|error| {
            format!(
                "could not flush agent error log `{}`: {error}",
                self.path.display()
            )
        })?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentErrorLogEntry {
    pub timestamp_unix_ms: u64,
    pub error_id: String,
    pub backend: String,
    pub category: String,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub resource: String,
    pub schema_id: String,
    pub question: String,
    pub user_message: String,
    pub internal_error: String,
}

impl AgentErrorLogEntry {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        error_id: &str,
        backend: &str,
        category: &str,
        session_id: &str,
        turn_id: Option<&str>,
        resource: &str,
        schema_id: &str,
        question: &str,
        user_message: &str,
        internal_error: &str,
    ) -> Self {
        Self {
            timestamp_unix_ms: unix_timestamp_ms(),
            error_id: error_id.to_owned(),
            backend: backend.to_owned(),
            category: category.to_owned(),
            session_id: session_id.to_owned(),
            turn_id: turn_id.map(ToOwned::to_owned),
            resource: resource.to_owned(),
            schema_id: schema_id.to_owned(),
            question: question.to_owned(),
            user_message: user_message.to_owned(),
            internal_error: internal_error.to_owned(),
        }
    }
}

fn unix_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
