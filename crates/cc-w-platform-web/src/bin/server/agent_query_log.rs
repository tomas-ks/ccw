use serde::Serialize;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug)]
pub struct AgentQueryLogger {
    path: PathBuf,
    write_lock: Mutex<()>,
}

impl AgentQueryLogger {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            write_lock: Mutex::new(()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append(&self, entry: &AgentQueryLogEntry) -> Result<(), String> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| "agent query log lock poisoned".to_owned())?;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "could not create agent query log directory `{}`: {error}",
                    parent.display()
                )
            })?;
        }
        let serialized = serde_json::to_vec(entry)
            .map_err(|error| format!("could not serialize agent query log entry: {error}"))?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|error| {
                format!(
                    "could not open agent query log `{}`: {error}",
                    self.path.display()
                )
            })?;
        file.write_all(&serialized).map_err(|error| {
            format!(
                "could not write agent query log `{}`: {error}",
                self.path.display()
            )
        })?;
        file.write_all(b"\n").map_err(|error| {
            format!(
                "could not finalize agent query log `{}`: {error}",
                self.path.display()
            )
        })?;
        file.flush().map_err(|error| {
            format!(
                "could not flush agent query log `{}`: {error}",
                self.path.display()
            )
        })?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentQueryLogEntry {
    pub timestamp_unix_ms: u64,
    pub backend: String,
    pub resource: String,
    pub schema_id: String,
    pub question: String,
    pub why: Option<String>,
    pub cypher: String,
    pub query_index: usize,
    pub success: bool,
    pub rows: Option<usize>,
    pub db_node_ids: Option<usize>,
    pub semantic_element_ids: Option<usize>,
    pub error: Option<String>,
}

impl AgentQueryLogEntry {
    pub fn success(
        backend: &str,
        resource: &str,
        schema_id: &str,
        question: &str,
        why: Option<&str>,
        cypher: &str,
        query_index: usize,
        rows: usize,
        db_node_ids: usize,
        semantic_element_ids: usize,
    ) -> Self {
        Self {
            timestamp_unix_ms: unix_timestamp_ms(),
            backend: backend.to_owned(),
            resource: resource.to_owned(),
            schema_id: schema_id.to_owned(),
            question: question.to_owned(),
            why: normalize_optional_text(why),
            cypher: cypher.to_owned(),
            query_index,
            success: true,
            rows: Some(rows),
            db_node_ids: Some(db_node_ids),
            semantic_element_ids: Some(semantic_element_ids),
            error: None,
        }
    }

    pub fn failure(
        backend: &str,
        resource: &str,
        schema_id: &str,
        question: &str,
        why: Option<&str>,
        cypher: &str,
        query_index: usize,
        rows: Option<usize>,
        error: &str,
    ) -> Self {
        Self {
            timestamp_unix_ms: unix_timestamp_ms(),
            backend: backend.to_owned(),
            resource: resource.to_owned(),
            schema_id: schema_id.to_owned(),
            question: question.to_owned(),
            why: normalize_optional_text(why),
            cypher: cypher.to_owned(),
            query_index,
            success: false,
            rows,
            db_node_ids: None,
            semantic_element_ids: None,
            error: Some(error.to_owned()),
        }
    }
}

fn normalize_optional_text(text: Option<&str>) -> Option<String> {
    text.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    })
}

fn unix_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
