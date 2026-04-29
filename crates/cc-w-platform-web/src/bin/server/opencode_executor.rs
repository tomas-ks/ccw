use super::agent_executor::{
    AgentActionCandidate, AgentBackendTurnRequest, AgentBackendTurnResponse, AgentEntityReference,
    AgentExecutor, AgentGraphMode, AgentProgressSink, AgentQueryPlaybook,
    AgentReadonlyCypherResult, AgentReadonlyCypherRuntime, AgentRelationReference,
    AgentSchemaContext, AgentTranscriptEvent, InspectionUpdateMode, NullAgentProgressSink,
};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    env,
    error::Error,
    ffi::OsString,
    fmt,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

const DEFAULT_TIMEOUT_MS: u64 = 180_000;
const DEFAULT_MAX_STDOUT_BYTES: usize = 256 * 1024;
const DEFAULT_MAX_STDERR_BYTES: usize = 64 * 1024;
const DEFAULT_MAX_STEPS_PER_TURN: usize = 12;
const DEFAULT_TRANSIENT_PROVIDER_RETRIES: usize = 1;
const DEFAULT_RETRY_BACKOFF_MS: u64 = 750;
const CHILD_POLL_INTERVAL_MS: u64 = 10;
const STDERR_EXCERPT_BYTES: usize = 2 * 1024;
const STDOUT_EXCERPT_BYTES: usize = 4 * 1024;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpencodeExecutorConfig {
    pub executable: PathBuf,
    pub args: Vec<String>,
    pub working_directory: Option<PathBuf>,
    pub config_path: Option<PathBuf>,
    pub agent: Option<String>,
    pub model: Option<String>,
    pub variant: Option<String>,
    pub timeout: Duration,
    pub max_stdout_bytes: usize,
    pub max_stderr_bytes: usize,
    pub max_steps_per_turn: usize,
    pub transient_provider_retries: usize,
    pub retry_backoff: Duration,
}

impl Default for OpencodeExecutorConfig {
    fn default() -> Self {
        Self {
            executable: PathBuf::from("opencode"),
            args: Vec::new(),
            working_directory: None,
            config_path: None,
            agent: None,
            model: None,
            variant: None,
            timeout: Duration::from_millis(DEFAULT_TIMEOUT_MS),
            max_stdout_bytes: DEFAULT_MAX_STDOUT_BYTES,
            max_stderr_bytes: DEFAULT_MAX_STDERR_BYTES,
            max_steps_per_turn: DEFAULT_MAX_STEPS_PER_TURN,
            transient_provider_retries: DEFAULT_TRANSIENT_PROVIDER_RETRIES,
            retry_backoff: Duration::from_millis(DEFAULT_RETRY_BACKOFF_MS),
        }
    }
}

impl OpencodeExecutorConfig {
    pub fn from_env() -> Result<Self, OpencodeExecutorConfigError> {
        Self::from_env_with(|key| env::var_os(key))
    }

    pub fn from_env_with<F>(mut get: F) -> Result<Self, OpencodeExecutorConfigError>
    where
        F: FnMut(&str) -> Option<OsString>,
    {
        let mut config = Self::default();

        if let Some(value) = get("CC_W_OPENCODE_EXECUTABLE") {
            config.executable = PathBuf::from(value);
        }
        if let Some(value) = get("CC_W_OPENCODE_ARGS") {
            config.args = split_args(&value.to_string_lossy());
        }
        if let Some(value) = get("CC_W_OPENCODE_WORKDIR") {
            config.working_directory = Some(PathBuf::from(value));
        }
        if let Some(value) = get("CC_W_OPENCODE_CONFIG") {
            config.config_path = Some(PathBuf::from(value));
        }
        if let Some(value) = get("CC_W_OPENCODE_AGENT") {
            let trimmed = value.to_string_lossy().trim().to_owned();
            if !trimmed.is_empty() {
                config.agent = Some(trimmed);
            }
        }
        if let Some(value) = get("CC_W_OPENCODE_MODEL") {
            let trimmed = value.to_string_lossy().trim().to_owned();
            if !trimmed.is_empty() {
                config.model = Some(trimmed);
            }
        }
        if let Some(value) = get("CC_W_OPENCODE_VARIANT") {
            let trimmed = value.to_string_lossy().trim().to_owned();
            if !trimmed.is_empty() && !trimmed.eq_ignore_ascii_case("none") {
                config.variant = Some(trimmed);
            }
        }
        if let Some(value) = get("CC_W_OPENCODE_TIMEOUT_MS") {
            config.timeout =
                Duration::from_millis(parse_env_u64("CC_W_OPENCODE_TIMEOUT_MS", &value)?);
        }
        if let Some(value) = get("CC_W_OPENCODE_MAX_STDOUT_BYTES") {
            config.max_stdout_bytes = parse_env_usize("CC_W_OPENCODE_MAX_STDOUT_BYTES", &value)?;
        }
        if let Some(value) = get("CC_W_OPENCODE_MAX_STDERR_BYTES") {
            config.max_stderr_bytes = parse_env_usize("CC_W_OPENCODE_MAX_STDERR_BYTES", &value)?;
        }
        if let Some(value) = get("CC_W_OPENCODE_MAX_STEPS_PER_TURN") {
            config.max_steps_per_turn =
                parse_env_usize("CC_W_OPENCODE_MAX_STEPS_PER_TURN", &value)?;
        }
        if let Some(value) = get("CC_W_OPENCODE_TRANSIENT_PROVIDER_RETRIES") {
            config.transient_provider_retries =
                parse_env_usize("CC_W_OPENCODE_TRANSIENT_PROVIDER_RETRIES", &value)?;
        }
        if let Some(value) = get("CC_W_OPENCODE_RETRY_BACKOFF_MS") {
            config.retry_backoff =
                Duration::from_millis(parse_env_u64("CC_W_OPENCODE_RETRY_BACKOFF_MS", &value)?);
        }

        Ok(config)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpencodeExecutorConfigError {
    InvalidUnsignedInteger { key: &'static str, value: String },
}

impl fmt::Display for OpencodeExecutorConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUnsignedInteger { key, value } => {
                write!(f, "{key} must be an unsigned integer, got `{value}`")
            }
        }
    }
}

impl Error for OpencodeExecutorConfigError {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OpencodeTurnRequest {
    pub resource: String,
    pub schema_id: String,
    pub schema_slug: Option<String>,
    pub user_input: String,
    #[serde(default)]
    pub session_history: Vec<OpencodeTranscriptEvent>,
    #[serde(default)]
    pub transcript: Vec<OpencodeTranscriptEvent>,
    #[serde(default)]
    pub tool_results: Vec<OpencodeToolResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OpencodeTurnResponse {
    #[serde(default)]
    pub transcript: Vec<OpencodeTranscriptEvent>,
    #[serde(default)]
    pub tool_calls: Vec<OpencodeToolCall>,
    pub final_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OpencodeTranscriptEvent {
    pub kind: OpencodeTranscriptEventKind,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum OpencodeTranscriptEventKind {
    System,
    User,
    Tool,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OpencodeToolResult {
    pub tool_name: String,
    pub call_id: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum OpencodeToolCall {
    #[serde(rename = "run_readonly_cypher")]
    RunReadonlyCypher {
        cypher: String,
        #[serde(default)]
        why: Option<String>,
    },
    #[serde(rename = "run_project_readonly_cypher")]
    RunProjectReadonlyCypher {
        cypher: String,
        #[serde(default)]
        why: Option<String>,
        #[serde(default)]
        resource_filter: Vec<String>,
    },
    #[serde(rename = "get_schema_context")]
    GetSchemaContext,
    #[serde(rename = "get_model_details")]
    GetModelDetails,
    #[serde(rename = "get_entity_reference")]
    GetEntityReference { entity_names: Vec<String> },
    #[serde(rename = "get_query_playbook")]
    GetQueryPlaybook {
        goal: String,
        #[serde(default)]
        entity_names: Vec<String>,
    },
    #[serde(rename = "get_relation_reference")]
    GetRelationReference { relation_names: Vec<String> },
    #[serde(rename = "request_tools")]
    RequestTools { tools: Vec<String> },
    #[serde(rename = "describe_nodes")]
    DescribeNodes { db_node_ids: Vec<i64> },
    #[serde(rename = "get_node_properties")]
    GetNodeProperties { db_node_id: i64 },
    #[serde(rename = "get_neighbors")]
    GetNeighbors {
        db_node_ids: Vec<i64>,
        hops: Option<usize>,
        mode: Option<AgentGraphMode>,
    },
    #[serde(rename = "emit_ui_actions")]
    EmitUiActions { actions: Vec<PlannedUiAction> },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PlannedUiAction {
    #[serde(rename = "graph.set_seeds")]
    GraphSetSeeds {
        db_node_ids: Vec<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resource: Option<String>,
    },
    #[serde(rename = "properties.show_node")]
    PropertiesShowNode {
        db_node_id: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resource: Option<String>,
    },
    #[serde(rename = "elements.hide")]
    ElementsHide {
        semantic_ids: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resource: Option<String>,
    },
    #[serde(rename = "elements.show")]
    ElementsShow {
        semantic_ids: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resource: Option<String>,
    },
    #[serde(rename = "elements.select")]
    ElementsSelect {
        semantic_ids: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resource: Option<String>,
    },
    #[serde(rename = "elements.inspect")]
    ElementsInspect {
        semantic_ids: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resource: Option<String>,
        #[serde(default, skip_serializing_if = "is_default_inspection_mode")]
        mode: InspectionUpdateMode,
    },
    #[serde(rename = "viewer.frame_visible")]
    ViewerFrameVisible,
    #[serde(rename = "viewer.clear_inspection")]
    ViewerClearInspection,
}

fn is_default_inspection_mode(mode: &InspectionUpdateMode) -> bool {
    *mode == InspectionUpdateMode::Replace
}

#[derive(Debug, Clone)]
pub struct OpencodeExecutor {
    config: OpencodeExecutorConfig,
    native_server: Option<Arc<OpencodeNativeServer>>,
    native_session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpencodeDiscoveredModel {
    pub id: String,
    pub variants: Vec<String>,
}

#[derive(Debug)]
pub struct OpencodeNativeServer {
    base_url: String,
    client: Client,
    process: Mutex<Option<Child>>,
    working_directory: Option<PathBuf>,
    event_bus: Arc<NativeEventBus>,
    shutting_down: AtomicBool,
}

impl OpencodeNativeServer {
    pub fn start(
        config: &OpencodeExecutorConfig,
        viewer_api_base: Option<&str>,
    ) -> Result<Arc<Self>, String> {
        let executable =
            resolve_executable(&config.executable).map_err(|error| error.to_string())?;
        let mut command = Command::new(&executable);
        command.arg("serve");
        command.arg("--pure");
        command.arg("--hostname");
        command.arg("127.0.0.1");
        command.arg("--port");
        command.arg("0");
        if let Some(working_directory) = &config.working_directory {
            command.current_dir(working_directory);
        }
        if let Some(config_path) = &config.config_path {
            command.env("OPENCODE_CONFIG", config_path);
        }
        if let Some(viewer_api_base) = viewer_api_base.filter(|value| !value.trim().is_empty()) {
            command.env("CC_W_VIEWER_API_BASE", viewer_api_base.trim());
        }
        command.stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut child = command
            .spawn()
            .map_err(|source| format!("could not start opencode serve process: {source}"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "opencode serve did not expose stdout".to_owned())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "opencode serve did not expose stderr".to_owned())?;
        let (ready_tx, ready_rx) = mpsc::channel::<Result<String, String>>();
        let startup_timeout = config.timeout.max(Duration::from_secs(10));
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            let mut output = Vec::new();
            loop {
                line.clear();
                let Ok(bytes_read) = reader.read_line(&mut line) else {
                    let _ = ready_tx.send(Err(
                        "failed to read opencode serve startup output".to_owned()
                    ));
                    return;
                };
                if bytes_read == 0 {
                    let message = String::from_utf8_lossy(&output).trim().to_owned();
                    let _ = ready_tx.send(Err(if message.is_empty() {
                        "opencode serve exited before announcing a listening URL".to_owned()
                    } else {
                        format!(
                            "opencode serve exited before announcing a listening URL; output: {message}"
                        )
                    }));
                    return;
                }
                output.extend_from_slice(line.as_bytes());
                let trimmed = line.trim();
                if trimmed.starts_with("opencode server listening") {
                    if let Some(url) = trimmed.split_whitespace().find(|segment| {
                        segment.starts_with("http://") || segment.starts_with("https://")
                    }) {
                        let _ = ready_tx.send(Ok(url.to_owned()));
                        return;
                    }
                    let _ = ready_tx.send(Err(format!(
                        "failed to parse opencode serve URL from startup line: {trimmed}"
                    )));
                    return;
                }
            }
        });

        thread::spawn(move || {
            let mut reader = BufReader::new(stderr);
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap_or(0) > 0 {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    println!("w web opencode serve {}", trimmed);
                }
                line.clear();
            }
        });

        let started = Instant::now();
        let base_url = loop {
            if crate::should_stop_requested() {
                let _ = child.kill();
                let _ = child.wait();
                return Err("opencode serve startup interrupted by shutdown request".to_owned());
            }

            if started.elapsed() >= startup_timeout {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "timed out waiting for opencode serve to start after {} ms",
                    startup_timeout.as_millis()
                ));
            }

            let remaining = startup_timeout - started.elapsed();
            match ready_rx.recv_timeout(remaining.min(Duration::from_millis(100))) {
                Ok(Ok(url)) => break url,
                Ok(Err(error)) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(error);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("opencode serve startup channel disconnected".to_owned());
                }
            }
        };

        let client = Client::builder()
            .build()
            .map_err(|error| format!("could not create opencode HTTP client: {error}"))?;
        let server = Arc::new(Self {
            base_url,
            client,
            process: Mutex::new(Some(child)),
            working_directory: config.working_directory.clone(),
            event_bus: Arc::new(NativeEventBus::default()),
            shutting_down: AtomicBool::new(false),
        });
        server.wait_until_healthy(startup_timeout)?;
        if crate::should_stop_requested() {
            server.shutdown();
            return Err("opencode serve startup interrupted by shutdown request".to_owned());
        }
        server.start_event_router();
        Ok(server)
    }

    fn wait_until_healthy(&self, timeout: Duration) -> Result<(), String> {
        let started = Instant::now();
        loop {
            if crate::should_stop_requested() {
                return Err("opencode serve startup interrupted by shutdown request".to_owned());
            }
            match self.client.get(self.url("/global/health")).send() {
                Ok(response) if response.status().is_success() => return Ok(()),
                Ok(_) | Err(_) => {
                    if started.elapsed() >= timeout {
                        return Err(format!(
                            "timed out waiting for opencode health check after {} ms",
                            timeout.as_millis()
                        ));
                    }
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url.trim_end_matches('/'), path)
    }

    fn query_pairs(&self) -> Vec<(String, String)> {
        self.working_directory
            .as_ref()
            .map(|working_directory| {
                vec![(
                    "directory".to_owned(),
                    working_directory.display().to_string(),
                )]
            })
            .unwrap_or_default()
    }

    pub fn create_session(&self, title: &str) -> Result<String, String> {
        let mut body = serde_json::Map::new();
        if !title.trim().is_empty() {
            body.insert("title".to_owned(), Value::String(title.trim().to_owned()));
        }
        let response = self
            .client
            .post(self.url("/session"))
            .query(&self.query_pairs())
            .json(&Value::Object(body))
            .send()
            .map_err(|error| format!("could not create opencode session: {error}"))?
            .error_for_status()
            .map_err(|error| format!("could not create opencode session: {error}"))?;
        let value: Value = response
            .json()
            .map_err(|error| format!("could not parse opencode session response: {error}"))?;
        value
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| "opencode session response did not include an id".to_owned())
    }

    pub fn subscribe_session_events(
        self: &Arc<Self>,
        session_id: &str,
    ) -> NativeSessionEventSubscription {
        let receiver = self.event_bus.subscribe(session_id);
        NativeSessionEventSubscription {
            session_id: session_id.to_owned(),
            event_bus: Arc::clone(&self.event_bus),
            receiver,
        }
    }

    pub fn prompt_async(
        &self,
        session_id: &str,
        agent: Option<&str>,
        model: Option<&str>,
        variant: Option<&str>,
        text: &str,
    ) -> Result<(), String> {
        let mut body = serde_json::Map::new();
        if let Some(agent) = agent.filter(|value| !value.trim().is_empty()) {
            body.insert("agent".to_owned(), Value::String(agent.trim().to_owned()));
        }
        if let Some(model) = model.filter(|value| !value.trim().is_empty()) {
            if let Some((provider_id, model_id)) = split_provider_model_id(model) {
                body.insert(
                    "model".to_owned(),
                    serde_json::json!({
                        "providerID": provider_id,
                        "modelID": model_id,
                    }),
                );
            }
        }
        if let Some(variant) = variant.filter(|value| !value.trim().is_empty()) {
            body.insert(
                "variant".to_owned(),
                Value::String(variant.trim().to_owned()),
            );
        }
        body.insert(
            "parts".to_owned(),
            serde_json::json!([
                {
                    "type": "text",
                    "text": text,
                }
            ]),
        );

        self.client
            .post(self.url(&format!("/session/{session_id}/prompt_async")))
            .query(&self.query_pairs())
            .json(&Value::Object(body))
            .send()
            .map_err(|error| format!("could not submit opencode prompt: {error}"))?
            .error_for_status()
            .map_err(|error| format!("could not submit opencode prompt: {error}"))?;
        Ok(())
    }

    pub fn abort_session(&self, session_id: &str) -> Result<(), String> {
        self.client
            .delete(self.url(&format!("/session/{session_id}/abort")))
            .query(&self.query_pairs())
            .send()
            .map_err(|error| format!("could not abort opencode session: {error}"))?
            .error_for_status()
            .map_err(|error| format!("could not abort opencode session: {error}"))?;
        Ok(())
    }

    pub fn subscribe_events(&self) -> Result<reqwest::blocking::Response, String> {
        self.client
            .get(self.url("/event"))
            .query(&self.query_pairs())
            .send()
            .map_err(|error| format!("could not subscribe to opencode events: {error}"))?
            .error_for_status()
            .map_err(|error| format!("could not subscribe to opencode events: {error}"))
    }

    fn start_event_router(self: &Arc<Self>) {
        let server = Arc::clone(self);
        thread::spawn(move || {
            let response = match server.subscribe_events() {
                Ok(response) => response,
                Err(error) => {
                    if !server.is_shutting_down() {
                        println!("w web opencode event router failed to subscribe: {}", error);
                    }
                    return;
                }
            };
            let (event_rx, reader_join) = spawn_native_event_reader(response);
            loop {
                match event_rx.recv() {
                    Ok(NativeStreamEvent::Value(value)) => server.event_bus.publish(value),
                    Ok(NativeStreamEvent::End) => break,
                    Ok(NativeStreamEvent::Error(error)) => {
                        if !server.is_shutting_down() {
                            println!("w web opencode event router error: {}", error);
                        }
                        break;
                    }
                    Err(_) => break,
                }
            }
            let _ = reader_join.join();
        });
    }

    fn is_shutting_down(&self) -> bool {
        self.shutting_down.load(Ordering::SeqCst) || crate::should_stop_requested()
    }

    pub fn shutdown(&self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        if let Ok(mut process) = self.process.lock() {
            if let Some(mut child) = process.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

impl Drop for OpencodeNativeServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[derive(Debug, Default)]
struct NativeEventBus {
    subscribers: Mutex<HashMap<String, mpsc::Sender<Value>>>,
}

impl NativeEventBus {
    fn subscribe(&self, session_id: &str) -> mpsc::Receiver<Value> {
        let (sender, receiver) = mpsc::channel();
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.insert(session_id.to_owned(), sender);
        }
        receiver
    }

    fn unsubscribe(&self, session_id: &str) {
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.remove(session_id);
        }
    }

    fn publish(&self, event: Value) {
        let Some(session_id) = native_event_session_id(&event).map(ToOwned::to_owned) else {
            return;
        };
        let Some(sender) = self
            .subscribers
            .lock()
            .ok()
            .and_then(|subscribers| subscribers.get(&session_id).cloned())
        else {
            return;
        };
        if sender.send(event).is_err() {
            self.unsubscribe(&session_id);
        }
    }
}

#[derive(Debug)]
pub struct NativeSessionEventSubscription {
    session_id: String,
    event_bus: Arc<NativeEventBus>,
    receiver: mpsc::Receiver<Value>,
}

impl NativeSessionEventSubscription {
    fn receiver(&self) -> &mpsc::Receiver<Value> {
        &self.receiver
    }
}

impl Drop for NativeSessionEventSubscription {
    fn drop(&mut self) {
        self.event_bus.unsubscribe(&self.session_id);
    }
}

impl OpencodeExecutor {
    pub fn new(config: OpencodeExecutorConfig) -> Self {
        Self {
            config,
            native_server: None,
            native_session_id: None,
        }
    }

    pub fn with_native_server(
        config: OpencodeExecutorConfig,
        native_server: Arc<OpencodeNativeServer>,
        native_session_id: Option<String>,
    ) -> Self {
        Self {
            config,
            native_server: Some(native_server),
            native_session_id,
        }
    }

    pub fn config(&self) -> &OpencodeExecutorConfig {
        &self.config
    }

    pub fn run_step(
        &self,
        request: &OpencodeTurnRequest,
    ) -> Result<OpencodeTurnResponse, OpencodeExecutorError> {
        let mut progress = NullAgentProgressSink;
        self.run_step_with_progress(request, &mut progress)
    }

    pub fn run_step_with_progress(
        &self,
        request: &OpencodeTurnRequest,
        progress: &mut dyn AgentProgressSink,
    ) -> Result<OpencodeTurnResponse, OpencodeExecutorError> {
        let mut attempt = 0usize;
        loop {
            match self.run_step_once(request, progress) {
                Ok(response) => return Ok(response),
                Err(error)
                    if attempt < self.config.transient_provider_retries
                        && should_retry_step_error(&error) =>
                {
                    attempt = attempt.saturating_add(1);
                    thread::sleep(self.config.retry_backoff);
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn execute_turn_native(
        &self,
        request: &AgentBackendTurnRequest,
        progress: &mut dyn AgentProgressSink,
        native_server: Arc<OpencodeNativeServer>,
        native_session_id: String,
    ) -> Result<AgentBackendTurnResponse, String> {
        let model = self.config.model.as_deref();
        let variant = self.config.variant.as_deref();
        let agent = self.config.agent.as_deref();

        println!(
            "w web opencode native agent={} model={} variant={}",
            agent.unwrap_or("ifc-explorer"),
            model.unwrap_or("-"),
            variant.unwrap_or("-"),
        );

        let event_subscription = native_server.subscribe_session_events(&native_session_id);
        let prompt = build_native_turn_prompt(request, agent);

        native_server
            .prompt_async(&native_session_id, agent, model, variant, &prompt)
            .map_err(|error| format!("opencode prompt submission failed: {error}"))?;

        let mut collector = NativeTurnCollector::new();
        let mut last_activity = Instant::now();
        let idle_timeout = self.config.timeout;

        loop {
            if crate::should_stop_requested() {
                native_server.shutdown();
                return Err("opencode turn interrupted by shutdown request".to_owned());
            }
            match event_subscription
                .receiver()
                .recv_timeout(Duration::from_millis(CHILD_POLL_INTERVAL_MS))
            {
                Ok(value) => {
                    let events = native_turn_progress_events_from_value(&value, &mut collector);
                    if !events.is_empty() {
                        last_activity = Instant::now();
                    }
                    for event in events {
                        println!(
                            "w web opencode progress {}",
                            summarize_agent_transcript_event(&event)
                        );
                        progress.emit(event.clone());
                        collector.transcript.push(event);
                    }
                    if collector.done {
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if last_activity.elapsed() >= idle_timeout {
                        let _ = native_server.abort_session(&native_session_id);
                        return Err(format!(
                            "opencode turn timed out after {} ms without progress and was aborted",
                            idle_timeout.as_millis()
                        ));
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }

        collector.finish(&native_session_id, progress);

        Ok(AgentBackendTurnResponse {
            transcript: collector.transcript,
            action_candidates: collector.action_candidates,
            queries_executed: collector.queries_executed,
        })
    }

    fn run_step_once(
        &self,
        request: &OpencodeTurnRequest,
        progress: &mut dyn AgentProgressSink,
    ) -> Result<OpencodeTurnResponse, OpencodeExecutorError> {
        let executable = resolve_executable(&self.config.executable)?;
        let stdin_body = serde_json::to_vec(request).map_err(OpencodeExecutorError::Serialize)?;
        let agent = self
            .config
            .agent
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());

        let mut command = Command::new(&executable);
        if self.config.args.is_empty() {
            if let Some(agent) = agent {
                println!(
                    "w web opencode subprocess agent={} model={} variant={}",
                    agent,
                    self.config.model.as_deref().unwrap_or("-"),
                    self.config.variant.as_deref().unwrap_or("-"),
                );
            } else {
                println!(
                    "w web opencode subprocess model={} variant={}",
                    self.config.model.as_deref().unwrap_or("-"),
                    self.config.variant.as_deref().unwrap_or("-"),
                );
            }
            command.args(default_opencode_run_args(
                request,
                self.config.model.as_deref(),
                self.config.variant.as_deref(),
                agent,
            ));
        } else {
            if let Some(agent) = agent {
                println!(
                    "w web opencode subprocess custom_args=1 agent_selection_ignored={} model_selection_ignored={} variant_selection_ignored={}",
                    agent,
                    self.config.model.as_deref().unwrap_or("-"),
                    self.config.variant.as_deref().unwrap_or("-"),
                );
            } else {
                println!(
                    "w web opencode subprocess custom_args=1 model_selection_ignored={} variant_selection_ignored={}",
                    self.config.model.as_deref().unwrap_or("-"),
                    self.config.variant.as_deref().unwrap_or("-"),
                );
            }
            command.args(&self.config.args);
        }
        if let Some(working_directory) = &self.config.working_directory {
            command.current_dir(working_directory);
        }
        if let Some(config_path) = &self.config.config_path {
            command.env("OPENCODE_CONFIG", config_path);
        }
        command.env("CC_W_AGENT_PROTOCOL", "ccw-opencode-v1");
        command.env("OPENCODE_CLIENT", "ccw-agent-bridge");
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = command
            .spawn()
            .map_err(|source| OpencodeExecutorError::SpawnFailed {
                executable: executable.clone(),
                source,
            })?;

        let mut child_stdin =
            child
                .stdin
                .take()
                .ok_or_else(|| OpencodeExecutorError::SpawnFailedWithoutPipe {
                    executable: executable.clone(),
                    pipe_name: "stdin",
                })?;
        child_stdin
            .write_all(&stdin_body)
            .and_then(|_| child_stdin.flush())
            .map_err(|source| OpencodeExecutorError::WriteStdinFailed {
                executable: executable.clone(),
                source,
            })?;
        drop(child_stdin);

        let child_stdout =
            child
                .stdout
                .take()
                .ok_or_else(|| OpencodeExecutorError::SpawnFailedWithoutPipe {
                    executable: executable.clone(),
                    pipe_name: "stdout",
                })?;
        let child_stderr =
            child
                .stderr
                .take()
                .ok_or_else(|| OpencodeExecutorError::SpawnFailedWithoutPipe {
                    executable: executable.clone(),
                    pipe_name: "stderr",
                })?;

        let stdout_max_bytes = self.config.max_stdout_bytes;
        let stderr_max_bytes = self.config.max_stderr_bytes;
        let last_activity = Arc::new(Mutex::new(Instant::now()));
        let progress_events = Arc::new(Mutex::new(Vec::<AgentTranscriptEvent>::new()));
        let stdout_activity = Arc::clone(&last_activity);
        let stderr_activity = Arc::clone(&last_activity);
        let stdout_progress_events = Arc::clone(&progress_events);
        let stderr_progress_events = Arc::clone(&progress_events);
        let stdout_handle = thread::spawn(move || {
            read_limited_with_progress(
                child_stdout,
                stdout_max_bytes,
                "stdout",
                true,
                stdout_activity,
                stdout_progress_events,
            )
        });
        let stderr_handle = thread::spawn(move || {
            read_limited_with_progress(
                child_stderr,
                stderr_max_bytes,
                "stderr",
                true,
                stderr_activity,
                stderr_progress_events,
            )
        });

        let exit_status = loop {
            if crate::should_stop_requested() {
                let _ = child.kill();
                let _ = child.wait();
                let _ = join_capture(stdout_handle, &executable, "stdout");
                let _ = join_capture(stderr_handle, &executable, "stderr");
                return Err(OpencodeExecutorError::Interrupted { executable });
            }
            drain_progress_events(&progress_events, progress, &executable)?;
            if let Some(status) =
                child
                    .try_wait()
                    .map_err(|source| OpencodeExecutorError::WaitFailed {
                        executable: executable.clone(),
                        source,
                    })?
            {
                break status;
            }

            let idle_elapsed = last_activity
                .lock()
                .map_err(|source| OpencodeExecutorError::WaitFailed {
                    executable: executable.clone(),
                    source: std::io::Error::other(format!(
                        "opencode activity tracker lock poisoned: {source}"
                    )),
                })?
                .elapsed();

            if idle_elapsed >= self.config.timeout {
                let _ = child.kill();
                let _ = child.wait();
                let stdout_capture = join_capture(stdout_handle, &executable, "stdout")?;
                let stderr_capture = join_capture(stderr_handle, &executable, "stderr")?;
                return Err(OpencodeExecutorError::TimedOut {
                    executable,
                    timeout: self.config.timeout,
                    stdout_excerpt: excerpt(&stdout_capture.bytes, STDOUT_EXCERPT_BYTES),
                    stderr_excerpt: excerpt(&stderr_capture.bytes, STDERR_EXCERPT_BYTES),
                });
            }

            thread::sleep(Duration::from_millis(CHILD_POLL_INTERVAL_MS));
        };

        let stdout_capture = join_capture(stdout_handle, &executable, "stdout")?;
        let stderr_capture = join_capture(stderr_handle, &executable, "stderr")?;
        drain_progress_events(&progress_events, progress, &executable)?;

        if !exit_status.success() {
            return Err(OpencodeExecutorError::ExitedBadly {
                executable,
                status: exit_status,
                stdout_excerpt: excerpt(&stdout_capture.bytes, STDOUT_EXCERPT_BYTES),
                stderr_excerpt: excerpt(&stderr_capture.bytes, STDERR_EXCERPT_BYTES),
            });
        }

        parse_opencode_response(&stdout_capture.bytes).map_err(|source| match source {
            OpencodeResponseParseError::ProviderError { message } => {
                OpencodeExecutorError::ProviderError {
                    executable,
                    message,
                    stdout_excerpt: excerpt(&stdout_capture.bytes, STDOUT_EXCERPT_BYTES),
                    stderr_excerpt: excerpt(&stderr_capture.bytes, STDERR_EXCERPT_BYTES),
                }
            }
            OpencodeResponseParseError::InvalidJson(source) => OpencodeExecutorError::InvalidJson {
                executable,
                source,
                stdout_excerpt: excerpt(&stdout_capture.bytes, STDOUT_EXCERPT_BYTES),
                stderr_excerpt: excerpt(&stderr_capture.bytes, STDERR_EXCERPT_BYTES),
            },
        })
    }
}

pub fn discover_opencode_models(
    config: &OpencodeExecutorConfig,
    provider: Option<&str>,
    timeout: Duration,
) -> Result<Vec<OpencodeDiscoveredModel>, OpencodeExecutorError> {
    let executable = resolve_executable(&config.executable)?;
    let mut command = Command::new(&executable);
    command.arg("models");
    if let Some(provider) = provider.map(str::trim).filter(|value| !value.is_empty()) {
        command.arg(provider);
    }
    command.arg("--verbose");
    command.arg("--pure");
    if let Some(working_directory) = &config.working_directory {
        command.current_dir(working_directory);
    }
    if let Some(config_path) = &config.config_path {
        command.env("OPENCODE_CONFIG", config_path);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command
        .spawn()
        .map_err(|source| OpencodeExecutorError::SpawnFailed {
            executable: executable.clone(),
            source,
        })?;
    let child_stdout =
        child
            .stdout
            .take()
            .ok_or_else(|| OpencodeExecutorError::SpawnFailedWithoutPipe {
                executable: executable.clone(),
                pipe_name: "stdout",
            })?;
    let child_stderr =
        child
            .stderr
            .take()
            .ok_or_else(|| OpencodeExecutorError::SpawnFailedWithoutPipe {
                executable: executable.clone(),
                pipe_name: "stderr",
            })?;
    let stdout_handle = thread::spawn(move || read_limited(child_stdout, DEFAULT_MAX_STDOUT_BYTES));
    let stderr_handle = thread::spawn(move || read_limited(child_stderr, STDERR_EXCERPT_BYTES));

    let start = Instant::now();
    let exit_status = loop {
        if crate::should_stop_requested() {
            let _ = child.kill();
            let _ = child.wait();
            let _ = join_capture(stdout_handle, &executable, "stdout");
            let _ = join_capture(stderr_handle, &executable, "stderr");
            return Err(OpencodeExecutorError::Interrupted { executable });
        }
        if let Some(status) =
            child
                .try_wait()
                .map_err(|source| OpencodeExecutorError::WaitFailed {
                    executable: executable.clone(),
                    source,
                })?
        {
            break status;
        }

        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            let stdout_capture = join_capture(stdout_handle, &executable, "stdout")?;
            let stderr_capture = join_capture(stderr_handle, &executable, "stderr")?;
            return Err(OpencodeExecutorError::TimedOut {
                executable,
                timeout,
                stdout_excerpt: excerpt(&stdout_capture.bytes, STDOUT_EXCERPT_BYTES),
                stderr_excerpt: excerpt(&stderr_capture.bytes, STDERR_EXCERPT_BYTES),
            });
        }

        thread::sleep(Duration::from_millis(CHILD_POLL_INTERVAL_MS));
    };

    let stdout_capture = join_capture(stdout_handle, &executable, "stdout")?;
    let stderr_capture = join_capture(stderr_handle, &executable, "stderr")?;

    if !exit_status.success() {
        return Err(OpencodeExecutorError::ExitedBadly {
            executable,
            status: exit_status,
            stdout_excerpt: excerpt(&stdout_capture.bytes, STDOUT_EXCERPT_BYTES),
            stderr_excerpt: excerpt(&stderr_capture.bytes, STDERR_EXCERPT_BYTES),
        });
    }

    Ok(parse_opencode_models_output(&stdout_capture.bytes))
}

impl AgentExecutor for OpencodeExecutor {
    fn execute_turn(
        &mut self,
        request: &AgentBackendTurnRequest,
        runtime: &mut dyn AgentReadonlyCypherRuntime,
        progress: &mut dyn AgentProgressSink,
    ) -> Result<AgentBackendTurnResponse, String> {
        if let (Some(native_server), Some(native_session_id)) = (
            self.native_server.as_ref().cloned(),
            self.native_session_id.as_deref(),
        ) {
            return self.execute_turn_native(
                request,
                progress,
                native_server,
                native_session_id.to_owned(),
            );
        }

        let mut transcript = Vec::new();
        let mut action_candidates = Vec::new();
        let mut tool_results = Vec::new();
        let mut tool_results_by_signature = HashMap::<String, OpencodeToolResult>::new();
        let mut queries_executed = 0usize;

        for step_index in 0..self.config.max_steps_per_turn {
            let step_request = OpencodeTurnRequest {
                resource: request.resource.clone(),
                schema_id: request.schema_id.clone(),
                schema_slug: request.schema_slug.clone(),
                user_input: request.input.clone(),
                session_history: request
                    .session_history
                    .iter()
                    .cloned()
                    .map(opencode_transcript_from_agent)
                    .collect(),
                transcript: transcript
                    .iter()
                    .cloned()
                    .map(opencode_transcript_from_agent)
                    .collect(),
                tool_results: tool_results.clone(),
            };

            let step = self
                .run_step_with_progress(&step_request, progress)
                .map_err(|error| format!("opencode turn failed: {error}"))?;

            for event in step
                .transcript
                .into_iter()
                .map(agent_transcript_from_opencode)
            {
                transcript.push(event);
            }

            if let Some(final_text) = step.final_text {
                let trimmed = final_text.trim();
                if !trimmed.is_empty() {
                    let event = AgentTranscriptEvent::assistant(trimmed.to_owned());
                    transcript.push(event);
                }
            }

            let mut executed_tool = false;
            for (call_index, tool_call) in step.tool_calls.into_iter().enumerate() {
                let tool_call_signature = opencode_tool_call_signature(&tool_call);
                if let Some(previous_result) =
                    tool_results_by_signature.get(&tool_call_signature).cloned()
                {
                    let reused_started_event = AgentTranscriptEvent::tool(format!(
                        "(reused) {}",
                        opencode_progress_event_for_tool_call(&tool_call).text
                    ));
                    progress.emit(reused_started_event.clone());
                    transcript.push(reused_started_event);
                    let reused_finished_event = AgentTranscriptEvent::system(
                        "Reused a prior result from earlier in this turn.".to_owned(),
                    );
                    progress.emit(reused_finished_event.clone());
                    transcript.push(reused_finished_event);
                    if previous_result.tool_name != "emit_ui_actions" {
                        tool_results.push(OpencodeToolResult {
                            tool_name: previous_result.tool_name,
                            call_id: format!("step-{}-tool-{}", step_index + 1, call_index + 1),
                            content: previous_result.content,
                        });
                    }
                    continue;
                }

                match tool_call {
                    OpencodeToolCall::RunReadonlyCypher { cypher, why } => {
                        executed_tool = true;
                        let started_event =
                            AgentTranscriptEvent::tool(match why.as_deref().map(str::trim) {
                                Some(why) if !why.is_empty() => {
                                    format!("{why}\nCypher:\n{}", cypher.trim())
                                }
                                _ => format!(
                                    "Running read-only Cypher for step {}.\nCypher:\n{}",
                                    step_index + 1,
                                    cypher.trim()
                                ),
                            });
                        progress.emit(started_event.clone());
                        transcript.push(started_event);
                        match runtime.run_readonly_cypher(&cypher, why.as_deref()) {
                            Ok(result) => {
                                queries_executed = queries_executed.saturating_add(1);
                                let finished_event = AgentTranscriptEvent::system(format!(
                                    "Read-only Cypher returned {} row{}.",
                                    result.rows.len(),
                                    if result.rows.len() == 1 { "" } else { "s" }
                                ));
                                progress.emit(finished_event.clone());
                                transcript.push(finished_event);
                                tool_results.push(OpencodeToolResult {
                                    tool_name: "run_readonly_cypher".to_owned(),
                                    call_id: format!(
                                        "step-{}-tool-{}",
                                        step_index + 1,
                                        call_index + 1
                                    ),
                                    content: serialize_readonly_cypher_tool_result(&result)
                                        .map_err(|error| {
                                            format!("could not encode Cypher tool result: {error}")
                                        })?,
                                });
                                tool_results_by_signature.insert(
                                    tool_call_signature,
                                    tool_results
                                        .last()
                                        .cloned()
                                        .expect("tool result just pushed"),
                                );
                            }
                            Err(error) => {
                                let failed_event = AgentTranscriptEvent::system(
                                    "That query shape did not work here. Trying a simpler angle."
                                        .to_owned(),
                                );
                                progress.emit(failed_event.clone());
                                transcript.push(failed_event);
                                tool_results.push(OpencodeToolResult {
                                    tool_name: "run_readonly_cypher".to_owned(),
                                    call_id: format!("step-{}-tool-{}", step_index + 1, call_index + 1),
                                    content: serialize_tool_error("run_readonly_cypher", &cypher, &error)
                                        .map_err(|encode_error| {
                                            format!("could not encode Cypher tool error result: {encode_error}")
                                        })?,
                                });
                                tool_results_by_signature.insert(
                                    tool_call_signature,
                                    tool_results
                                        .last()
                                        .cloned()
                                        .expect("tool result just pushed"),
                                );
                            }
                        }
                    }
                    OpencodeToolCall::RunProjectReadonlyCypher {
                        cypher,
                        why,
                        resource_filter,
                    } => {
                        executed_tool = true;
                        let started_event =
                            AgentTranscriptEvent::tool(match why.as_deref().map(str::trim) {
                                Some(why) if !why.is_empty() => {
                                    format!("Project: {why}\nCypher:\n{}", cypher.trim())
                                }
                                _ => format!(
                                    "Running project read-only Cypher for step {}.\nCypher:\n{}",
                                    step_index + 1,
                                    cypher.trim()
                                ),
                            });
                        progress.emit(started_event.clone());
                        transcript.push(started_event);
                        match runtime.run_project_readonly_cypher(
                            &cypher,
                            why.as_deref(),
                            &resource_filter,
                        ) {
                            Ok(result) => {
                                queries_executed = queries_executed.saturating_add(1);
                                let resource_count = project_resource_count_from_rows(&result);
                                let finished_event = AgentTranscriptEvent::system(format!(
                                    "Project read-only Cypher returned {} row{} across {} IFC resource{}.",
                                    result.rows.len(),
                                    if result.rows.len() == 1 { "" } else { "s" },
                                    resource_count,
                                    if resource_count == 1 { "" } else { "s" }
                                ));
                                progress.emit(finished_event.clone());
                                transcript.push(finished_event);
                                tool_results.push(OpencodeToolResult {
                                    tool_name: "run_project_readonly_cypher".to_owned(),
                                    call_id: format!(
                                        "step-{}-tool-{}",
                                        step_index + 1,
                                        call_index + 1
                                    ),
                                    content: serialize_readonly_cypher_tool_result(&result)
                                        .map_err(|error| {
                                            format!(
                                                "could not encode project Cypher tool result: {error}"
                                            )
                                        })?,
                                });
                                tool_results_by_signature.insert(
                                    tool_call_signature,
                                    tool_results
                                        .last()
                                        .cloned()
                                        .expect("tool result just pushed"),
                                );
                            }
                            Err(error) => {
                                let failed_event = AgentTranscriptEvent::system(
                                    "That project query shape did not work here. Trying a simpler angle."
                                        .to_owned(),
                                );
                                progress.emit(failed_event.clone());
                                transcript.push(failed_event);
                                tool_results.push(OpencodeToolResult {
                                    tool_name: "run_project_readonly_cypher".to_owned(),
                                    call_id: format!("step-{}-tool-{}", step_index + 1, call_index + 1),
                                    content: serialize_tool_error("run_project_readonly_cypher", &cypher, &error)
                                        .map_err(|encode_error| {
                                            format!("could not encode project Cypher tool error result: {encode_error}")
                                        })?,
                                });
                                tool_results_by_signature.insert(
                                    tool_call_signature,
                                    tool_results
                                        .last()
                                        .cloned()
                                        .expect("tool result just pushed"),
                                );
                            }
                        }
                    }
                    OpencodeToolCall::GetSchemaContext => {
                        executed_tool = true;
                        queries_executed = queries_executed.saturating_add(1);
                        let started_event = AgentTranscriptEvent::tool(format!(
                            "Loading schema context for {}.",
                            request.schema_id
                        ));
                        progress.emit(started_event.clone());
                        transcript.push(started_event);
                        let result: AgentSchemaContext = runtime.get_schema_context()?;
                        let finished_event = AgentTranscriptEvent::system(format!(
                            "Schema context loaded for {}.",
                            result.schema_id
                        ));
                        progress.emit(finished_event.clone());
                        transcript.push(finished_event);
                        tool_results.push(OpencodeToolResult {
                            tool_name: "get_schema_context".to_owned(),
                            call_id: format!("step-{}-tool-{}", step_index + 1, call_index + 1),
                            content: serialize_tool_result(&result).map_err(|error| {
                                format!("could not encode schema context result: {error}")
                            })?,
                        });
                        tool_results_by_signature.insert(
                            tool_call_signature,
                            tool_results
                                .last()
                                .cloned()
                                .expect("tool result just pushed"),
                        );
                    }
                    OpencodeToolCall::GetModelDetails => {
                        executed_tool = true;
                        let started_event = AgentTranscriptEvent::tool(
                            "Loading model overview for the current IFC model.".to_owned(),
                        );
                        progress.emit(started_event.clone());
                        transcript.push(started_event);
                        let result = collect_model_details(
                            runtime,
                            progress,
                            &mut transcript,
                            &mut queries_executed,
                        )?;
                        let finished_event = AgentTranscriptEvent::system(
                            "Model overview loaded for the current IFC model.".to_owned(),
                        );
                        progress.emit(finished_event.clone());
                        transcript.push(finished_event);
                        tool_results.push(OpencodeToolResult {
                            tool_name: "get_model_details".to_owned(),
                            call_id: format!("step-{}-tool-{}", step_index + 1, call_index + 1),
                            content: serialize_tool_result(&result).map_err(|error| {
                                format!("could not encode model details result: {error}")
                            })?,
                        });
                        tool_results_by_signature.insert(
                            tool_call_signature,
                            tool_results
                                .last()
                                .cloned()
                                .expect("tool result just pushed"),
                        );
                    }
                    OpencodeToolCall::GetEntityReference { entity_names } => {
                        executed_tool = true;
                        queries_executed = queries_executed.saturating_add(1);
                        let started_event = AgentTranscriptEvent::tool(format!(
                            "Loading schema reference for {} entit{}.",
                            entity_names.len(),
                            if entity_names.len() == 1 { "y" } else { "ies" }
                        ));
                        progress.emit(started_event.clone());
                        transcript.push(started_event);
                        let result: Vec<AgentEntityReference> =
                            runtime.get_entity_reference(&entity_names)?;
                        let finished_event = AgentTranscriptEvent::system(format!(
                            "Entity reference lookup returned {} match{}.",
                            result.len(),
                            if result.len() == 1 { "" } else { "es" }
                        ));
                        progress.emit(finished_event.clone());
                        transcript.push(finished_event);
                        tool_results.push(OpencodeToolResult {
                            tool_name: "get_entity_reference".to_owned(),
                            call_id: format!("step-{}-tool-{}", step_index + 1, call_index + 1),
                            content: serialize_tool_result(&result).map_err(|error| {
                                format!("could not encode entity reference result: {error}")
                            })?,
                        });
                        tool_results_by_signature.insert(
                            tool_call_signature,
                            tool_results
                                .last()
                                .cloned()
                                .expect("tool result just pushed"),
                        );
                    }
                    OpencodeToolCall::RequestTools { tools } => {
                        executed_tool = true;
                        let requested_tools = tools
                            .into_iter()
                            .map(|tool| tool.trim().to_owned())
                            .filter(|tool| !tool.is_empty())
                            .collect::<Vec<_>>();
                        let started_event = AgentTranscriptEvent::tool(format!(
                            "Requesting {} tool{}: {}.",
                            requested_tools.len(),
                            if requested_tools.len() == 1 { "" } else { "s" },
                            if requested_tools.is_empty() {
                                "none".to_owned()
                            } else {
                                requested_tools.join(", ")
                            }
                        ));
                        progress.emit(started_event.clone());
                        transcript.push(started_event);

                        let mut requested_results = serde_json::Map::new();
                        let mut unsupported_tools = Vec::new();

                        for requested_tool in requested_tools {
                            match normalize_tool_function_kind(&requested_tool).as_str() {
                                "get_schema_context" => {
                                    let schema_started_event = AgentTranscriptEvent::tool(format!(
                                        "Loading schema context for {}.",
                                        request.schema_id
                                    ));
                                    progress.emit(schema_started_event.clone());
                                    transcript.push(schema_started_event);
                                    let result: AgentSchemaContext =
                                        runtime.get_schema_context()?;
                                    queries_executed = queries_executed.saturating_add(1);
                                    let schema_finished_event = AgentTranscriptEvent::system(
                                        format!("Schema context loaded for {}.", result.schema_id),
                                    );
                                    progress.emit(schema_finished_event.clone());
                                    transcript.push(schema_finished_event);
                                    requested_results.insert(
                                        "schemaContext".to_owned(),
                                        serde_json::to_value(&result).map_err(|error| {
                                            format!(
                                                "could not encode requested schema context result: {error}"
                                            )
                                        })?,
                                    );
                                }
                                "get_model_details" => {
                                    let result = collect_model_details(
                                        runtime,
                                        progress,
                                        &mut transcript,
                                        &mut queries_executed,
                                    )?;
                                    requested_results.insert("modelDetails".to_owned(), result);
                                }
                                other => unsupported_tools.push(other.to_owned()),
                            }
                        }

                        if !unsupported_tools.is_empty() {
                            requested_results.insert(
                                "unsupportedTools".to_owned(),
                                serde_json::to_value(&unsupported_tools).map_err(|error| {
                                    format!("could not encode unsupported requested tools: {error}")
                                })?,
                            );
                        }

                        let finished_event = AgentTranscriptEvent::system(format!(
                            "Requested tool bundle returned {} supported tool{}.",
                            requested_results.len() - usize::from(!unsupported_tools.is_empty()),
                            if requested_results.len() == 1 {
                                ""
                            } else {
                                "s"
                            }
                        ));
                        progress.emit(finished_event.clone());
                        transcript.push(finished_event);
                        tool_results.push(OpencodeToolResult {
                            tool_name: "request_tools".to_owned(),
                            call_id: format!("step-{}-tool-{}", step_index + 1, call_index + 1),
                            content: serde_json::to_string(&serde_json::Value::Object(
                                requested_results,
                            ))
                            .map_err(|error| {
                                format!("could not encode requested tools result: {error}")
                            })?,
                        });
                        tool_results_by_signature.insert(
                            tool_call_signature,
                            tool_results
                                .last()
                                .cloned()
                                .expect("tool result just pushed"),
                        );
                    }
                    OpencodeToolCall::GetQueryPlaybook { goal, entity_names } => {
                        executed_tool = true;
                        queries_executed = queries_executed.saturating_add(1);
                        let started_event = AgentTranscriptEvent::tool(format!(
                            "Loading query playbook for `{}`.",
                            goal.trim()
                        ));
                        progress.emit(started_event.clone());
                        transcript.push(started_event);
                        let result: Vec<AgentQueryPlaybook> =
                            runtime.get_query_playbook(&goal, &entity_names)?;
                        let finished_event = AgentTranscriptEvent::system(format!(
                            "Query playbook lookup returned {} match{}.",
                            result.len(),
                            if result.len() == 1 { "" } else { "es" }
                        ));
                        progress.emit(finished_event.clone());
                        transcript.push(finished_event);
                        tool_results.push(OpencodeToolResult {
                            tool_name: "get_query_playbook".to_owned(),
                            call_id: format!("step-{}-tool-{}", step_index + 1, call_index + 1),
                            content: serialize_tool_result(&result).map_err(|error| {
                                format!("could not encode query playbook result: {error}")
                            })?,
                        });
                        tool_results_by_signature.insert(
                            tool_call_signature,
                            tool_results
                                .last()
                                .cloned()
                                .expect("tool result just pushed"),
                        );
                    }
                    OpencodeToolCall::GetRelationReference { relation_names } => {
                        executed_tool = true;
                        queries_executed = queries_executed.saturating_add(1);
                        let started_event = AgentTranscriptEvent::tool(format!(
                            "Loading relation reference for {} item{}.",
                            relation_names.len(),
                            if relation_names.len() == 1 { "" } else { "s" }
                        ));
                        progress.emit(started_event.clone());
                        transcript.push(started_event);
                        let result: Vec<AgentRelationReference> =
                            runtime.get_relation_reference(&relation_names)?;
                        let finished_event = AgentTranscriptEvent::system(format!(
                            "Relation reference lookup returned {} match{}.",
                            result.len(),
                            if result.len() == 1 { "" } else { "es" }
                        ));
                        progress.emit(finished_event.clone());
                        transcript.push(finished_event);
                        tool_results.push(OpencodeToolResult {
                            tool_name: "get_relation_reference".to_owned(),
                            call_id: format!("step-{}-tool-{}", step_index + 1, call_index + 1),
                            content: serialize_tool_result(&result).map_err(|error| {
                                format!("could not encode relation reference result: {error}")
                            })?,
                        });
                        tool_results_by_signature.insert(
                            tool_call_signature,
                            tool_results
                                .last()
                                .cloned()
                                .expect("tool result just pushed"),
                        );
                    }
                    OpencodeToolCall::DescribeNodes { db_node_ids } => {
                        executed_tool = true;
                        queries_executed = queries_executed.saturating_add(1);
                        let started_event = AgentTranscriptEvent::tool(format!(
                            "Describing {} graph node{}.",
                            db_node_ids.len(),
                            if db_node_ids.len() == 1 { "" } else { "s" }
                        ));
                        progress.emit(started_event.clone());
                        transcript.push(started_event);
                        let result = runtime.describe_nodes(&db_node_ids)?;
                        let finished_event = AgentTranscriptEvent::system(format!(
                            "Node describe returned {} node {}.",
                            result.len(),
                            if result.len() == 1 {
                                "summary"
                            } else {
                                "summaries"
                            }
                        ));
                        progress.emit(finished_event.clone());
                        transcript.push(finished_event);
                        tool_results.push(OpencodeToolResult {
                            tool_name: "describe_nodes".to_owned(),
                            call_id: format!("step-{}-tool-{}", step_index + 1, call_index + 1),
                            content: serialize_tool_result(&result).map_err(|error| {
                                format!("could not encode node summary result: {error}")
                            })?,
                        });
                        tool_results_by_signature.insert(
                            tool_call_signature,
                            tool_results
                                .last()
                                .cloned()
                                .expect("tool result just pushed"),
                        );
                    }
                    OpencodeToolCall::GetNodeProperties { db_node_id } => {
                        executed_tool = true;
                        queries_executed = queries_executed.saturating_add(1);
                        let started_event = AgentTranscriptEvent::tool(format!(
                            "Loading properties for graph node {}.",
                            db_node_id
                        ));
                        progress.emit(started_event.clone());
                        transcript.push(started_event);
                        let result = runtime.get_node_properties(db_node_id)?;
                        let finished_event = AgentTranscriptEvent::system(format!(
                            "Node property inspection returned {} propert{} and {} relation{}.",
                            result.properties.len(),
                            if result.properties.len() == 1 {
                                "y"
                            } else {
                                "ies"
                            },
                            result.relations.len(),
                            if result.relations.len() == 1 { "" } else { "s" }
                        ));
                        progress.emit(finished_event.clone());
                        transcript.push(finished_event);
                        tool_results.push(OpencodeToolResult {
                            tool_name: "get_node_properties".to_owned(),
                            call_id: format!("step-{}-tool-{}", step_index + 1, call_index + 1),
                            content: serialize_tool_result(&result).map_err(|error| {
                                format!("could not encode node properties result: {error}")
                            })?,
                        });
                        tool_results_by_signature.insert(
                            tool_call_signature,
                            tool_results
                                .last()
                                .cloned()
                                .expect("tool result just pushed"),
                        );
                    }
                    OpencodeToolCall::GetNeighbors {
                        db_node_ids,
                        hops,
                        mode,
                    } => {
                        executed_tool = true;
                        queries_executed = queries_executed.saturating_add(1);
                        let resolved_hops = hops.unwrap_or(1).max(1);
                        let resolved_mode = mode.unwrap_or(AgentGraphMode::Semantic);
                        let started_event = AgentTranscriptEvent::tool(format!(
                            "Loading neighbor graph from {} seed node{}.",
                            db_node_ids.len(),
                            if db_node_ids.len() == 1 { "" } else { "s" }
                        ));
                        progress.emit(started_event.clone());
                        transcript.push(started_event);
                        let result =
                            runtime.get_neighbors(&db_node_ids, resolved_hops, resolved_mode)?;
                        let finished_event = AgentTranscriptEvent::system(format!(
                            "Neighbor graph returned {} node{} and {} edge{}.",
                            result.nodes.len(),
                            if result.nodes.len() == 1 { "" } else { "s" },
                            result.edges.len(),
                            if result.edges.len() == 1 { "" } else { "s" }
                        ));
                        progress.emit(finished_event.clone());
                        transcript.push(finished_event);
                        tool_results.push(OpencodeToolResult {
                            tool_name: "get_neighbors".to_owned(),
                            call_id: format!("step-{}-tool-{}", step_index + 1, call_index + 1),
                            content: serialize_tool_result(&result).map_err(|error| {
                                format!("could not encode neighbor graph result: {error}")
                            })?,
                        });
                        tool_results_by_signature.insert(
                            tool_call_signature,
                            tool_results
                                .last()
                                .cloned()
                                .expect("tool result just pushed"),
                        );
                    }
                    OpencodeToolCall::EmitUiActions { actions } => {
                        let action_event = AgentTranscriptEvent::assistant(format!(
                            "Preparing {} viewer action{}.",
                            actions.len(),
                            if actions.len() == 1 { "" } else { "s" }
                        ));
                        progress.emit(action_event.clone());
                        transcript.push(action_event);
                        action_candidates.extend(actions.into_iter().map(map_planned_ui_action));
                        tool_results_by_signature.insert(
                            tool_call_signature,
                            OpencodeToolResult {
                                tool_name: "emit_ui_actions".to_owned(),
                                call_id: format!("step-{}-tool-{}", step_index + 1, call_index + 1),
                                content: serde_json::to_string(&serde_json::json!({
                                    "ok": true,
                                }))
                                .map_err(|error| {
                                    format!("could not encode viewer action reuse marker: {error}")
                                })?,
                            },
                        );
                    }
                }
            }

            if !executed_tool {
                return Ok(AgentBackendTurnResponse {
                    transcript,
                    action_candidates,
                    queries_executed,
                });
            }
        }

        Err(format!(
            "opencode exceeded the maximum step budget for one turn ({})",
            self.config.max_steps_per_turn
        ))
    }
}

#[derive(Debug)]
pub enum OpencodeExecutorError {
    MissingExecutable {
        configured: PathBuf,
    },
    Serialize(serde_json::Error),
    SpawnFailed {
        executable: PathBuf,
        source: std::io::Error,
    },
    SpawnFailedWithoutPipe {
        executable: PathBuf,
        pipe_name: &'static str,
    },
    WriteStdinFailed {
        executable: PathBuf,
        source: std::io::Error,
    },
    WaitFailed {
        executable: PathBuf,
        source: std::io::Error,
    },
    JoinFailed {
        executable: PathBuf,
        stream_name: &'static str,
    },
    TimedOut {
        executable: PathBuf,
        timeout: Duration,
        stdout_excerpt: String,
        stderr_excerpt: String,
    },
    Interrupted {
        executable: PathBuf,
    },
    ExitedBadly {
        executable: PathBuf,
        status: ExitStatus,
        stdout_excerpt: String,
        stderr_excerpt: String,
    },
    InvalidJson {
        executable: PathBuf,
        source: serde_json::Error,
        stdout_excerpt: String,
        stderr_excerpt: String,
    },
    ProviderError {
        executable: PathBuf,
        message: String,
        stdout_excerpt: String,
        stderr_excerpt: String,
    },
}

impl fmt::Display for OpencodeExecutorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingExecutable { configured } => write!(
                f,
                "opencode executable `{}` was not found; configure CC_W_OPENCODE_EXECUTABLE or install it on PATH",
                configured.display()
            ),
            Self::Serialize(error) => write!(f, "could not serialize agent turn request: {error}"),
            Self::SpawnFailed { executable, source } => write!(
                f,
                "could not start opencode executable `{}`: {source}",
                executable.display()
            ),
            Self::SpawnFailedWithoutPipe {
                executable,
                pipe_name,
            } => write!(
                f,
                "opencode executable `{}` did not expose expected {pipe_name} pipe",
                executable.display()
            ),
            Self::WriteStdinFailed { executable, source } => write!(
                f,
                "could not send agent turn request to `{}`: {source}",
                executable.display()
            ),
            Self::WaitFailed { executable, source } => write!(
                f,
                "could not wait for opencode executable `{}`: {source}",
                executable.display()
            ),
            Self::JoinFailed {
                executable,
                stream_name,
            } => write!(
                f,
                "reader thread for `{}` {} stream panicked",
                executable.display(),
                stream_name
            ),
            Self::TimedOut {
                executable,
                timeout,
                stderr_excerpt,
                ..
            } => write!(
                f,
                "opencode executable `{}` timed out after {} ms without progress{}",
                executable.display(),
                timeout.as_millis(),
                format_excerpt_suffix(stderr_excerpt)
            ),
            Self::Interrupted { executable } => write!(
                f,
                "opencode executable `{}` was interrupted by shutdown request",
                executable.display()
            ),
            Self::ExitedBadly {
                executable,
                status,
                stderr_excerpt,
                ..
            } => write!(
                f,
                "opencode executable `{}` exited with {}{}",
                executable.display(),
                format_exit_status(*status),
                format_excerpt_suffix(stderr_excerpt)
            ),
            Self::InvalidJson {
                executable,
                source,
                stdout_excerpt,
                ..
            } => write!(
                f,
                "opencode executable `{}` produced invalid JSON: {source}{}",
                executable.display(),
                format_excerpt_suffix(stdout_excerpt)
            ),
            Self::ProviderError {
                executable,
                message,
                stderr_excerpt,
                ..
            } => write!(
                f,
                "opencode provider error from `{}`: {}{}",
                executable.display(),
                message,
                format_excerpt_suffix(stderr_excerpt)
            ),
        }
    }
}

impl Error for OpencodeExecutorError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Serialize(error) => Some(error),
            Self::SpawnFailed { source, .. } => Some(source),
            Self::WriteStdinFailed { source, .. } => Some(source),
            Self::WaitFailed { source, .. } => Some(source),
            Self::InvalidJson { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[derive(Debug)]
struct OutputCapture {
    bytes: Vec<u8>,
}

#[derive(Debug)]
enum NativeStreamEvent {
    Value(Value),
    End,
    Error(String),
}

#[derive(Debug, Default)]
struct NativeTurnCollector {
    transcript: Vec<AgentTranscriptEvent>,
    action_candidates: Vec<AgentActionCandidate>,
    queries_executed: usize,
    done: bool,
    final_text: Option<String>,
    last_tool_snapshot_by_call_id: HashMap<String, String>,
    last_tool_status_by_call_id: HashMap<String, String>,
    last_tool_call_emitted_by_call_id: HashMap<String, bool>,
}

impl NativeTurnCollector {
    fn new() -> Self {
        Self::default()
    }

    fn handle_event(&mut self, event: &Value) -> Vec<AgentTranscriptEvent> {
        let event_type = native_event_type(event);
        let payload = native_event_payload(event);
        match event_type.as_deref() {
            Some("message.part.updated") | Some("message.part.updated.1") => {
                if let Some(part) = native_event_part(payload) {
                    self.handle_part(part)
                } else {
                    Vec::new()
                }
            }
            Some("message.part.delta") | Some("message.part.delta.1") => {
                self.handle_part_delta(payload);
                Vec::new()
            }
            Some("session.status") => {
                if native_session_is_idle(payload) {
                    self.done = true;
                }
                Vec::new()
            }
            Some("session.idle") => {
                self.done = true;
                Vec::new()
            }
            Some("session.error") => {
                self.done = true;
                vec![AgentTranscriptEvent::system(
                    native_session_error_summary(payload)
                        .unwrap_or_else(|| "OpenCode session reported an error.".to_owned()),
                )]
            }
            Some("message.updated") | Some("message.updated.1") => Vec::new(),
            _ => Vec::new(),
        }
    }

    fn finish(&mut self, session_id: &str, progress: &mut dyn AgentProgressSink) {
        if let Some(final_text) = self.final_text.as_deref().map(str::trim) {
            if !final_text.is_empty() {
                let event = AgentTranscriptEvent::assistant(final_text.to_owned());
                progress.emit(event.clone());
                self.transcript.push(event);
            }
        }
        if self.transcript.is_empty() {
            let event =
                AgentTranscriptEvent::system(format!("OpenCode session {} completed.", session_id));
            progress.emit(event.clone());
            self.transcript.push(event);
        }
        self.done = true;
    }

    fn handle_part(&mut self, part: &Value) -> Vec<AgentTranscriptEvent> {
        let Some(part_type) = part.get("type").and_then(Value::as_str) else {
            return Vec::new();
        };
        match part_type {
            "tool" => self.handle_tool_part(part),
            "text" => {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        self.final_text = Some(trimmed.to_owned());
                    }
                }
                Vec::new()
            }
            "reasoning" => {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        self.final_text = Some(trimmed.to_owned());
                    }
                }
                Vec::new()
            }
            "step-start" => {
                let snapshot = native_part_snapshot(part);
                if native_record_part_snapshot(
                    &mut self.last_tool_snapshot_by_call_id,
                    part,
                    snapshot,
                ) {
                    vec![AgentTranscriptEvent::system(
                        "opencode progress: step started".to_owned(),
                    )]
                } else {
                    Vec::new()
                }
            }
            "step-finish" => {
                let snapshot = native_part_snapshot(part);
                if native_record_part_snapshot(
                    &mut self.last_tool_snapshot_by_call_id,
                    part,
                    snapshot,
                ) {
                    vec![AgentTranscriptEvent::system(
                        "opencode progress: step finished".to_owned(),
                    )]
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        }
    }

    fn handle_part_delta(&mut self, payload: &Value) {
        let Some(part_id) = native_part_id(payload) else {
            return;
        };
        let Some(field) = payload.get("field").and_then(Value::as_str) else {
            return;
        };
        if field != "text" && field != "reasoning" {
            return;
        }
        let Some(delta) = payload.get("delta").and_then(Value::as_str) else {
            return;
        };
        if delta.trim().is_empty() {
            return;
        }
        let mut current = self.final_text.take().unwrap_or_default();
        if current.is_empty() {
            current = delta.to_owned();
        } else if !current.ends_with(delta) {
            current.push_str(delta);
        }
        let _ = part_id;
        self.final_text = Some(current);
    }

    fn handle_tool_part(&mut self, part: &Value) -> Vec<AgentTranscriptEvent> {
        let Some(tool_name) = native_tool_name(part) else {
            return Vec::new();
        };
        let Some(call_id) = native_tool_call_id(part) else {
            return Vec::new();
        };
        let state = part.get("state").and_then(Value::as_object);
        let status = state
            .and_then(|state| state.get("status"))
            .and_then(Value::as_str)
            .unwrap_or("running")
            .trim()
            .to_owned();
        let snapshot = native_tool_state_snapshot(part);
        if !native_record_tool_snapshot(&mut self.last_tool_snapshot_by_call_id, &call_id, snapshot)
        {
            return Vec::new();
        }
        self.last_tool_status_by_call_id
            .insert(call_id.clone(), status.clone());
        let state_value = state
            .map(|state| Value::Object(state.clone()))
            .unwrap_or(Value::Null);
        let input = state
            .and_then(|state| state.get("input"))
            .or_else(|| state.and_then(|state| state.get("args")))
            .or_else(|| state.and_then(|state| state.get("arguments")))
            .or_else(|| state.and_then(|state| state.get("request")));
        let mut events = Vec::new();
        let call_events = opencode_tool_progress_call_events(
            &tool_name,
            state.and_then(|state| state.get("title").and_then(Value::as_str)),
            &status,
            input,
        );
        let call_already_emitted = *self
            .last_tool_call_emitted_by_call_id
            .get(&call_id)
            .unwrap_or(&false);

        match status.as_str() {
            "pending" | "running" | "started" | "streaming" | "in_progress" => {
                if !call_events.is_empty() {
                    self.last_tool_call_emitted_by_call_id
                        .insert(call_id.clone(), true);
                    events.extend(call_events);
                }
            }
            "completed" => {
                if !call_already_emitted {
                    if !call_events.is_empty() {
                        self.last_tool_call_emitted_by_call_id
                            .insert(call_id.clone(), true);
                        events.extend(call_events);
                    }
                    events.push(AgentTranscriptEvent::tool(
                        opencode_tool_progress_output_summary(
                            &tool_name,
                            state.and_then(|state| state.get("output")),
                        ),
                    ));
                } else {
                    events.push(AgentTranscriptEvent::tool(
                        opencode_tool_progress_output_summary(
                            &tool_name,
                            state.and_then(|state| state.get("output")),
                        ),
                    ));
                }
            }
            "error" => {
                if !call_already_emitted {
                    if !call_events.is_empty() {
                        self.last_tool_call_emitted_by_call_id
                            .insert(call_id.clone(), true);
                        events.extend(call_events);
                    }
                    events.push(AgentTranscriptEvent::tool(
                        opencode_tool_progress_error_summary(
                            &tool_name,
                            state
                                .and_then(|state| state.get("error"))
                                .unwrap_or(&state_value),
                        ),
                    ));
                } else {
                    events.push(AgentTranscriptEvent::tool(
                        opencode_tool_progress_error_summary(
                            &tool_name,
                            state
                                .and_then(|state| state.get("error"))
                                .unwrap_or(&state_value),
                        ),
                    ));
                }
            }
            _ => {
                events.extend(opencode_tool_progress_events(&serde_json::json!({
                    "part": part,
                })));
            }
        }

        if matches!(status.as_str(), "completed") {
            if let Some(action_candidate) =
                native_action_candidate_from_tool_call(&tool_name, state)
            {
                self.action_candidates.push(action_candidate);
            }
            if is_native_readonly_cypher_tool_name(&tool_name) {
                self.queries_executed = self.queries_executed.saturating_add(1);
            }
        }

        events
    }
}

fn native_record_part_snapshot(
    snapshots: &mut HashMap<String, String>,
    part: &Value,
    snapshot: String,
) -> bool {
    let Some(part_id) = native_part_id(part) else {
        return false;
    };
    native_record_snapshot(snapshots, &part_id, snapshot)
}

fn native_record_tool_snapshot(
    snapshots: &mut HashMap<String, String>,
    call_id: &str,
    snapshot: String,
) -> bool {
    native_record_snapshot(snapshots, call_id, snapshot)
}

fn native_record_snapshot(
    snapshots: &mut HashMap<String, String>,
    key: &str,
    snapshot: String,
) -> bool {
    match snapshots.get(key) {
        Some(previous) if previous == &snapshot => false,
        _ => {
            snapshots.insert(key.to_owned(), snapshot);
            true
        }
    }
}

fn native_part_id(part: &Value) -> Option<String> {
    part.get("id")
        .and_then(Value::as_str)
        .or_else(|| part.get("callID").and_then(Value::as_str))
        .or_else(|| part.get("callId").and_then(Value::as_str))
        .or_else(|| part.get("messageID").and_then(Value::as_str))
        .map(ToOwned::to_owned)
}

fn native_part_snapshot(part: &Value) -> String {
    serde_json::to_string(&serde_json::json!({
        "type": part.get("type").and_then(Value::as_str),
        "reason": part.get("reason").and_then(Value::as_str),
        "status": part.get("status").and_then(Value::as_str),
        "title": part.get("title").and_then(Value::as_str),
        "reasonCode": part.get("reasonCode").and_then(Value::as_str),
    }))
    .unwrap_or_default()
}

fn native_tool_state_snapshot(part: &Value) -> String {
    let state = part.get("state").cloned().unwrap_or(Value::Null);
    serde_json::to_string(&serde_json::json!({
        "tool": native_tool_name(part),
        "callID": native_tool_call_id(part),
        "status": state.get("status").and_then(Value::as_str),
        "input": state.get("input"),
        "output": state.get("output"),
        "error": state.get("error"),
        "title": state.get("title").and_then(Value::as_str),
    }))
    .unwrap_or_default()
}

fn native_turn_progress_events_from_value(
    value: &Value,
    collector: &mut NativeTurnCollector,
) -> Vec<AgentTranscriptEvent> {
    let event_type = native_event_type(value);
    let payload = native_event_payload(value);
    match event_type.as_deref() {
        Some("message.part.updated") | Some("message.part.updated.1") => {
            if let Some(part) = native_event_part(payload) {
                collector.handle_part(part)
            } else {
                Vec::new()
            }
        }
        Some("message.part.delta") | Some("message.part.delta.1") => {
            collector.handle_part_delta(payload);
            Vec::new()
        }
        Some("session.status") => {
            if native_session_is_idle(payload) {
                collector.done = true;
            }
            Vec::new()
        }
        Some("session.idle") => {
            collector.done = true;
            Vec::new()
        }
        Some("session.error") => {
            collector.done = true;
            vec![AgentTranscriptEvent::system(
                native_session_error_summary(payload)
                    .unwrap_or_else(|| "OpenCode session reported an error.".to_owned()),
            )]
        }
        _ => Vec::new(),
    }
}

fn native_event_type(value: &Value) -> Option<String> {
    value
        .get("type")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            value
                .get("name")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
}

fn native_event_payload(value: &Value) -> &Value {
    value
        .get("properties")
        .or_else(|| value.get("data"))
        .unwrap_or(value)
}

fn native_event_part(value: &Value) -> Option<&Value> {
    value
        .get("part")
        .or_else(|| value.get("info"))
        .or_else(|| value.get("message"))
        .or_else(|| {
            value
                .get("properties")
                .and_then(|properties| properties.get("part"))
        })
}

fn native_event_session_id(value: &Value) -> Option<&str> {
    value
        .get("sessionID")
        .and_then(Value::as_str)
        .or_else(|| value.get("sessionId").and_then(Value::as_str))
        .or_else(|| {
            value
                .get("properties")
                .and_then(|value| value.get("sessionID"))
                .and_then(Value::as_str)
        })
        .or_else(|| {
            value
                .get("data")
                .and_then(|value| value.get("sessionID"))
                .and_then(Value::as_str)
        })
}

fn native_session_is_idle(value: &Value) -> bool {
    matches!(
        value
            .get("status")
            .and_then(Value::as_object)
            .and_then(|status| status.get("type"))
            .and_then(Value::as_str),
        Some("idle")
    ) || matches!(value.get("status").and_then(Value::as_str), Some("idle"))
}

fn native_session_error_summary(value: &Value) -> Option<String> {
    let error = value
        .get("error")
        .or_else(|| value.get("properties")?.get("error"))?;
    let message = error
        .get("data")
        .and_then(|data| data.get("message"))
        .and_then(Value::as_str)
        .or_else(|| error.get("message").and_then(Value::as_str))
        .or_else(|| error.get("name").and_then(Value::as_str))
        .unwrap_or("session error");
    Some(message.trim().to_owned())
}

fn native_tool_name(part: &Value) -> Option<String> {
    part.get("tool")
        .and_then(Value::as_str)
        .or_else(|| part.get("name").and_then(Value::as_str))
        .map(|value| canonical_native_tool_name(value).to_owned())
}

fn native_tool_call_id(part: &Value) -> Option<String> {
    part.get("callID")
        .and_then(Value::as_str)
        .or_else(|| part.get("callId").and_then(Value::as_str))
        .or_else(|| part.get("id").and_then(Value::as_str))
        .map(ToOwned::to_owned)
}

fn canonical_native_tool_name(tool_name: &str) -> &str {
    let trimmed = tool_name.trim();
    trimmed.strip_prefix("ifc_").unwrap_or(trimmed)
}

fn split_provider_model_id(model_id: &str) -> Option<(&str, &str)> {
    let (provider_id, model_id) = model_id.split_once('/')?;
    let provider_id = provider_id.trim();
    let model_id = model_id.trim();
    (!provider_id.is_empty() && !model_id.is_empty()).then_some((provider_id, model_id))
}

fn display_native_tool_name(tool_name: &str) -> String {
    match canonical_native_tool_name(tool_name) {
        "schema_context" => "ifc_schema_context".to_owned(),
        "model_details" => "ifc_model_details".to_owned(),
        "entity_reference" => "ifc_entity_reference".to_owned(),
        "relation_reference" => "ifc_relation_reference".to_owned(),
        "query_playbook" => "ifc_query_playbook".to_owned(),
        "readonly_cypher" => "ifc_readonly_cypher".to_owned(),
        "project_readonly_cypher" => "ifc_project_readonly_cypher".to_owned(),
        "run_project_readonly_cypher" => "ifc_project_readonly_cypher".to_owned(),
        "node_relations" => "ifc_node_relations".to_owned(),
        "renderable_descendants" => "ifc_renderable_descendants".to_owned(),
        "element_search" => "ifc_element_search".to_owned(),
        "scope_summary" => "ifc_scope_summary".to_owned(),
        "scope_inspect" => "ifc_scope_inspect".to_owned(),
        "bridge_structure_summary" => "ifc_bridge_structure_summary".to_owned(),
        "quantity_takeoff" => "ifc_quantity_takeoff".to_owned(),
        "section_at_point_or_station" => "ifc_section_at_point_or_station".to_owned(),
        other => other.to_owned(),
    }
}

fn is_native_readonly_cypher_tool_name(tool_name: &str) -> bool {
    matches!(
        canonical_native_tool_name(tool_name),
        "readonly_cypher"
            | "run_readonly_cypher"
            | "project_readonly_cypher"
            | "run_project_readonly_cypher"
    )
}

fn native_action_candidate_from_tool_call(
    tool_name: &str,
    state: Option<&serde_json::Map<String, Value>>,
) -> Option<AgentActionCandidate> {
    let state = state?;
    let input = state.get("input")?.as_object()?;
    match canonical_native_tool_name(tool_name) {
        "graph_set_seeds" => native_db_node_ids_from_input(input).map(|db_node_ids| {
            let mut candidate = AgentActionCandidate::graph_set_seeds(db_node_ids);
            if let Some(resource) = native_resource_from_input(input) {
                candidate = candidate.with_resource(resource);
            }
            candidate
        }),
        "properties_show_node" => native_db_node_id_from_input(input).map(|db_node_id| {
            let mut candidate = AgentActionCandidate::properties_show_node(db_node_id);
            if let Some(resource) = native_resource_from_input(input) {
                candidate = candidate.with_resource(resource);
            }
            candidate
        }),
        "elements_hide" => native_semantic_ids_from_input(input).map(|semantic_ids| {
            let mut candidate = AgentActionCandidate::elements_hide(semantic_ids);
            if let Some(resource) = native_resource_from_input(input) {
                candidate = candidate.with_resource(resource);
            }
            candidate
        }),
        "elements_show" => native_semantic_ids_from_input(input).map(|semantic_ids| {
            let mut candidate = AgentActionCandidate::elements_show(semantic_ids);
            if let Some(resource) = native_resource_from_input(input) {
                candidate = candidate.with_resource(resource);
            }
            candidate
        }),
        "elements_select" => native_semantic_ids_from_input(input).map(|semantic_ids| {
            let mut candidate = AgentActionCandidate::elements_select(semantic_ids);
            if let Some(resource) = native_resource_from_input(input) {
                candidate = candidate.with_resource(resource);
            }
            candidate
        }),
        "elements_inspect" => native_semantic_ids_from_input(input).map(|semantic_ids| {
            let mut candidate = AgentActionCandidate::elements_inspect_with_mode(
                semantic_ids,
                native_inspection_mode_from_input(input),
            );
            if let Some(resource) = native_resource_from_input(input) {
                candidate = candidate.with_resource(resource);
            }
            candidate
        }),
        "scope_inspect" => native_semantic_ids_from_input(input).map(|semantic_ids| {
            let mut candidate = AgentActionCandidate::elements_inspect_with_mode(
                semantic_ids,
                native_inspection_mode_from_input(input),
            );
            if let Some(resource) = native_resource_from_input(input) {
                candidate = candidate.with_resource(resource);
            }
            candidate
        }),
        "viewer_clear_inspection" | "clear_inspection" => {
            Some(AgentActionCandidate::viewer_clear_inspection())
        }
        "viewer_frame_visible" | "frame" => Some(AgentActionCandidate::viewer_frame_visible()),
        _ => None,
    }
}

fn native_resource_from_input(input: &serde_json::Map<String, Value>) -> Option<String> {
    input
        .get("resource")
        .or_else(|| input.get("source_resource"))
        .or_else(|| input.get("sourceResource"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn native_inspection_mode_from_input(
    input: &serde_json::Map<String, Value>,
) -> InspectionUpdateMode {
    input
        .get("mode")
        .or_else(|| input.get("inspection_mode"))
        .or_else(|| input.get("inspectionMode"))
        .and_then(Value::as_str)
        .and_then(parse_inspection_mode)
        .unwrap_or_default()
}

fn native_db_node_ids_from_input(input: &serde_json::Map<String, Value>) -> Option<Vec<i64>> {
    let ids = input
        .get("db_node_ids")
        .or_else(|| input.get("dbNodeIds"))
        .or_else(|| input.get("nodeIds"))
        .or_else(|| input.get("ids"))?
        .as_array()?;
    let ids = ids
        .iter()
        .filter_map(|value| match value {
            Value::Number(number) => number.as_i64(),
            Value::String(text) => text.trim().parse::<i64>().ok(),
            _ => None,
        })
        .collect::<Vec<_>>();
    (!ids.is_empty()).then_some(ids)
}

fn native_db_node_id_from_input(input: &serde_json::Map<String, Value>) -> Option<i64> {
    input
        .get("db_node_id")
        .or_else(|| input.get("dbNodeId"))
        .or_else(|| input.get("nodeId"))
        .or_else(|| input.get("id"))
        .and_then(|value| match value {
            Value::Number(number) => number.as_i64(),
            Value::String(text) => text.trim().parse::<i64>().ok(),
            _ => None,
        })
}

fn native_semantic_ids_from_input(input: &serde_json::Map<String, Value>) -> Option<Vec<String>> {
    let ids = input
        .get("semantic_ids")
        .or_else(|| input.get("semanticIds"))
        .or_else(|| input.get("elementIds"))
        .or_else(|| input.get("ids"))?
        .as_array()?;
    let ids = ids
        .iter()
        .filter_map(|value| value.as_str().map(str::trim).map(ToOwned::to_owned))
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    (!ids.is_empty()).then_some(ids)
}

fn spawn_native_event_reader(
    response: reqwest::blocking::Response,
) -> (mpsc::Receiver<NativeStreamEvent>, thread::JoinHandle<()>) {
    let (sender, receiver) = mpsc::channel();
    let handle = thread::spawn(move || {
        let mut reader = BufReader::new(response);
        let mut line = String::new();
        let mut event_name = None::<String>;
        let mut data_lines = Vec::<String>::new();

        let flush_event = |event_name: &mut Option<String>,
                           data_lines: &mut Vec<String>,
                           sender: &mpsc::Sender<NativeStreamEvent>| {
            if data_lines.is_empty() {
                event_name.take();
                return;
            }
            let payload_text = data_lines.join("\n");
            data_lines.clear();
            event_name.take();
            let payload = if payload_text.trim_start().starts_with('{')
                || payload_text.trim_start().starts_with('[')
            {
                serde_json::from_str::<Value>(&payload_text)
            } else {
                serde_json::from_str::<Value>(payload_text.trim())
            };
            match payload {
                Ok(value) => {
                    let _ = sender.send(NativeStreamEvent::Value(value));
                }
                Err(error) => {
                    let _ = sender.send(NativeStreamEvent::Error(format!(
                        "could not parse opencode event stream payload: {error}"
                    )));
                }
            }
        };

        loop {
            line.clear();
            let bytes_read = match reader.read_line(&mut line) {
                Ok(bytes_read) => bytes_read,
                Err(error) => {
                    let _ = sender.send(NativeStreamEvent::Error(format!(
                        "could not read opencode event stream: {error}"
                    )));
                    return;
                }
            };
            if bytes_read == 0 {
                flush_event(&mut event_name, &mut data_lines, &sender);
                let _ = sender.send(NativeStreamEvent::End);
                return;
            }

            let trimmed = line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                flush_event(&mut event_name, &mut data_lines, &sender);
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix("event:") {
                event_name = Some(rest.trim().to_owned());
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix("data:") {
                data_lines.push(rest.trim_start().to_owned());
                continue;
            }
            if trimmed.starts_with(':') {
                continue;
            }

            if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
                let _ = sender.send(NativeStreamEvent::Value(value));
            } else {
                data_lines.push(trimmed.to_owned());
            }
        }
    });

    (receiver, handle)
}

fn map_planned_ui_action(action: PlannedUiAction) -> AgentActionCandidate {
    match action {
        PlannedUiAction::GraphSetSeeds {
            db_node_ids,
            resource,
        } => {
            let candidate = AgentActionCandidate::graph_set_seeds(db_node_ids);
            if let Some(resource) = resource {
                candidate.with_resource(resource)
            } else {
                candidate
            }
        }
        PlannedUiAction::PropertiesShowNode {
            db_node_id,
            resource,
        } => {
            let candidate = AgentActionCandidate::properties_show_node(db_node_id);
            if let Some(resource) = resource {
                candidate.with_resource(resource)
            } else {
                candidate
            }
        }
        PlannedUiAction::ElementsHide {
            semantic_ids,
            resource,
        } => {
            let candidate = AgentActionCandidate::elements_hide(semantic_ids);
            if let Some(resource) = resource {
                candidate.with_resource(resource)
            } else {
                candidate
            }
        }
        PlannedUiAction::ElementsShow {
            semantic_ids,
            resource,
        } => {
            let candidate = AgentActionCandidate::elements_show(semantic_ids);
            if let Some(resource) = resource {
                candidate.with_resource(resource)
            } else {
                candidate
            }
        }
        PlannedUiAction::ElementsSelect {
            semantic_ids,
            resource,
        } => {
            let candidate = AgentActionCandidate::elements_select(semantic_ids);
            if let Some(resource) = resource {
                candidate.with_resource(resource)
            } else {
                candidate
            }
        }
        PlannedUiAction::ElementsInspect {
            semantic_ids,
            resource,
            mode,
        } => {
            let candidate = AgentActionCandidate::elements_inspect_with_mode(semantic_ids, mode);
            if let Some(resource) = resource {
                candidate.with_resource(resource)
            } else {
                candidate
            }
        }
        PlannedUiAction::ViewerFrameVisible => AgentActionCandidate::viewer_frame_visible(),
        PlannedUiAction::ViewerClearInspection => AgentActionCandidate::viewer_clear_inspection(),
    }
}

fn serialize_tool_result<T: Serialize>(result: &T) -> Result<String, serde_json::Error> {
    serde_json::to_string(result)
}

fn readonly_cypher_result_value(result: &AgentReadonlyCypherResult) -> serde_json::Value {
    let row_objects = result
        .rows
        .iter()
        .map(|row| {
            result
                .columns
                .iter()
                .enumerate()
                .filter_map(|(index, column)| {
                    row.get(index)
                        .map(|value| (column.clone(), Value::String(value.clone())))
                })
                .collect::<BTreeMap<_, _>>()
        })
        .collect::<Vec<_>>();

    serde_json::json!({
        "ok": true,
        "columns": result.columns,
        "rows": result.rows,
        "rowObjects": row_objects,
        "dbNodeIds": result.db_node_ids,
        "semanticElementIds": result.semantic_element_ids,
        "firstDbNodeId": result.db_node_ids.first(),
        "firstSemanticElementId": result.semantic_element_ids.first(),
    })
}

fn project_resource_count_from_rows(result: &AgentReadonlyCypherResult) -> usize {
    let source_index = result
        .columns
        .iter()
        .position(|column| column.eq_ignore_ascii_case("source_resource"));
    let Some(source_index) = source_index else {
        return 1;
    };
    let mut seen = std::collections::BTreeSet::new();
    for row in &result.rows {
        if let Some(resource) = row.get(source_index).map(String::as_str) {
            let resource = resource.trim();
            if !resource.is_empty() {
                seen.insert(resource.to_owned());
            }
        }
    }
    seen.len().max(1)
}

fn serialize_readonly_cypher_tool_result(
    result: &AgentReadonlyCypherResult,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(&readonly_cypher_result_value(result))
}

fn collect_model_details(
    runtime: &mut dyn AgentReadonlyCypherRuntime,
    progress: &mut dyn AgentProgressSink,
    transcript: &mut Vec<AgentTranscriptEvent>,
    queries_executed: &mut usize,
) -> Result<serde_json::Value, String> {
    let project = run_overview_query(
        runtime,
        progress,
        transcript,
        queries_executed,
        "Loading the project root for a model overview.",
        "MATCH (p:IfcProject) RETURN id(p) AS node_id, p.Name AS project_name, p.GlobalId AS global_id, p.Description AS description, p.LongName AS long_name, p.ObjectType AS object_type LIMIT 1",
        "find the project root for a model overview",
    )?;
    let site_count = run_overview_query(
        runtime,
        progress,
        transcript,
        queries_executed,
        "Counting sites for a model overview.",
        "MATCH (s:IfcSite) RETURN count(s) AS site_count",
        "count sites for a model overview",
    )?;
    let building_count = run_overview_query(
        runtime,
        progress,
        transcript,
        queries_executed,
        "Counting buildings for a model overview.",
        "MATCH (b:IfcBuilding) RETURN count(b) AS building_count",
        "count buildings for a model overview",
    )?;
    let bridge_count = run_overview_query(
        runtime,
        progress,
        transcript,
        queries_executed,
        "Counting bridges for a model overview.",
        "MATCH (bridge:IfcBridge) RETURN count(bridge) AS bridge_count",
        "count bridges for a model overview",
    )?;
    let material_names = run_overview_query(
        runtime,
        progress,
        transcript,
        queries_executed,
        "Sampling the material vocabulary for a model overview.",
        "MATCH (:IfcRelAssociatesMaterial)--(material:IfcMaterial) RETURN DISTINCT material.Name AS material_name LIMIT 12",
        "sample the material vocabulary for a model overview",
    )?;

    Ok(serde_json::json!({
        "project": readonly_cypher_result_value(&project),
        "counts": {
            "sites": readonly_cypher_result_value(&site_count),
            "buildings": readonly_cypher_result_value(&building_count),
            "bridges": readonly_cypher_result_value(&bridge_count),
        },
        "materials": readonly_cypher_result_value(&material_names),
    }))
}

fn run_overview_query(
    runtime: &mut dyn AgentReadonlyCypherRuntime,
    progress: &mut dyn AgentProgressSink,
    transcript: &mut Vec<AgentTranscriptEvent>,
    queries_executed: &mut usize,
    started_message: &str,
    query: &str,
    why: &str,
) -> Result<AgentReadonlyCypherResult, String> {
    let started_event = AgentTranscriptEvent::tool(started_message.to_owned());
    progress.emit(started_event.clone());
    transcript.push(started_event);
    let result = runtime.run_readonly_cypher(query, Some(why))?;
    *queries_executed = queries_executed.saturating_add(1);
    let finished_event = AgentTranscriptEvent::system(format!(
        "Read-only Cypher returned {} row{}.",
        result.rows.len(),
        if result.rows.len() == 1 { "" } else { "s" }
    ));
    progress.emit(finished_event.clone());
    transcript.push(finished_event);
    Ok(result)
}

fn default_opencode_run_args(
    request: &OpencodeTurnRequest,
    model: Option<&str>,
    variant: Option<&str>,
    agent: Option<&str>,
) -> Vec<String> {
    let mut args = vec![
        "run".to_owned(),
        "--format".to_owned(),
        "json".to_owned(),
        "--title".to_owned(),
        "ccw agent turn".to_owned(),
    ];
    if let Some(agent) = agent.filter(|value| !value.trim().is_empty()) {
        args.push("--agent".to_owned());
        args.push(agent.trim().to_owned());
    }
    if let Some(model) = model.filter(|value| !value.trim().is_empty()) {
        args.push("--model".to_owned());
        args.push(model.trim().to_owned());
    }
    if let Some(variant) = variant.filter(|value| !value.trim().is_empty()) {
        args.push("--variant".to_owned());
        args.push(variant.trim().to_owned());
    }
    args.push("--pure".to_owned());
    args.push(build_prompt(request, agent.is_some()));
    args
}

fn turn_context_block(resource: &str, schema_id: &str, schema_slug: Option<&str>) -> Vec<String> {
    let resource = resource.trim();
    let schema_id = schema_id.trim();
    let schema_slug = schema_slug.map(str::trim).filter(|value| !value.is_empty());

    let mut lines = if resource.starts_with("project/") {
        vec![format!(
            "Bound IFC project resource for this turn: {}.",
            resource
        )]
    } else {
        vec![format!("Bound IFC resource for this turn: {}.", resource)]
    };
    if let Some(slug) = schema_slug {
        lines.push(format!(
            "Bound IFC schema for this turn: {} ({}).",
            schema_id, slug
        ));
    } else {
        lines.push(format!("Bound IFC schema for this turn: {}.", schema_id));
    }
    if resource.starts_with("project/") {
        lines.push(
            "For broad project questions, use the exact project resource above with `ifc_project_readonly_cypher`; for single-model follow-ups, use a concrete member `ifc/...` resource with `ifc_readonly_cypher`."
                .to_owned(),
        );
    } else {
        lines.push(
            "Use the exact resource string above in any `ifc_*` tool call. Do not swap in a placeholder or `/api`."
                .to_owned(),
        );
    }
    lines
}

fn build_native_turn_prompt(request: &AgentBackendTurnRequest, agent: Option<&str>) -> String {
    let mut prompt = turn_context_block(
        &request.resource,
        &request.schema_id,
        request.schema_slug.as_deref(),
    );
    if let Some(agent) = agent.filter(|value| !value.trim().is_empty()) {
        prompt.push(format!(
            "Selected native OpenCode agent: `{}`.",
            agent.trim()
        ));
    }
    prompt.push(String::new());
    prompt.push("User request:".to_owned());
    prompt.push(request.input.trim().to_owned());
    prompt.join("\n")
}

fn build_prompt(request: &OpencodeTurnRequest, native_agent: bool) -> String {
    let prompt = vec![
        "You are the ccw IFC viewer AI execution layer.".to_owned(),
        if native_agent {
            "You have native OpenCode access to the repo-local `ifc_*` tools through the `ifc-explorer` agent.".to_owned()
        } else {
            "You do not have direct tools. You must return only JSON, with no markdown fences or commentary.".to_owned()
        },
        if native_agent {
            "Use the native `ifc_*` tools directly when investigating, then return only JSON that summarizes what you learned.".to_owned()
        } else {
            "Return only JSON, with no markdown fences or commentary.".to_owned()
        },
        if native_agent {
            "High-value native model tools: use `ifc_element_search` to find candidates; use `ifc_scope_summary` to understand grouped ids; use `ifc_scope_inspect` for show/inspect flows; use `ifc_bridge_structure_summary` for bridge decomposition; use `ifc_quantity_takeoff` for count/material/BOM style summaries with provenance; use `ifc_section_at_point_or_station` only when explicit station/point/ids are available, and never invent section geometry.".to_owned()
        } else {
            "Use only the JSON toolCalls kinds listed below; do not invent direct native tool wrappers.".to_owned()
        },
        if native_agent {
            "For requests like `inspect all bearings`, once you have renderable `GlobalId` values, immediately issue the viewer inspection action. Do not spend another turn summarizing or reasoning over every returned row unless the user asked for that summary.".to_owned()
        } else {
            "For requests like `inspect all bearings`, once you have renderable `GlobalId` values, emit the viewer inspection action. Do not spend another turn summarizing every returned row unless the user asked for that summary.".to_owned()
        },
        "For viewer actions with explicit complete/plural scope, such as `add the piles`, `inspect all bearings`, or `hide every column`, bounded queries are only for discovery. The final id-collection query/action must be complete and must not use LIMIT unless the user explicitly asks for a sample or subset. If the result could be large, count first, then collect all focused renderable ids or explain why the full action is unsafe.".to_owned(),
        "When using `ifc_element_search` to collect ids for a complete viewer action, set `all_matches: true` and provide a focused anchor such as `entity_names: [\"IfcPile\"]`. Do not use broad text-only search as the final complete action source.".to_owned(),
        turn_context_block(
            &request.resource,
            &request.schema_id,
            request.schema_slug.as_deref(),
        )
        .join("\n"),
        "Return one JSON object matching this exact schema:".to_owned(),
        serde_json::to_string_pretty(&serde_json::json!({
            "transcript": [{ "kind": "assistant", "text": "short progress note" }],
            "toolCalls": [
                { "kind": "get_schema_context" },
                { "kind": "get_entity_reference", "entity_names": ["IfcRoof", "IfcSlab"] },
                { "kind": "get_query_playbook", "goal": "hide the roof", "entity_names": ["IfcRoof", "IfcSlab"] },
                { "kind": "get_relation_reference", "relation_names": ["IfcRelAggregates", "RELATED_OBJECTS"] },
                {
                    "kind": "run_readonly_cypher",
                    "cypher": "MATCH (n) RETURN id(n) AS node_id LIMIT 1",
                    "why": "find one graph seed node before deciding on a viewer action"
                },
                {
                    "kind": "run_project_readonly_cypher",
                    "cypher": "MATCH (p:IfcProject) RETURN p.Name AS project_name LIMIT 1",
                    "why": "sample every IFC resource in the active project",
                    "resource_filter": []
                },
                { "kind": "describe_nodes", "db_node_ids": [123] },
                { "kind": "get_node_properties", "db_node_id": 123 },
                { "kind": "get_neighbors", "db_node_ids": [123], "hops": 1, "mode": "semantic" },
                {
                    "kind": "emit_ui_actions",
                    "actions": [
                        { "kind": "graph.set_seeds", "db_node_ids": [123], "resource": "ifc/building-architecture" },
                        { "kind": "properties.show_node", "db_node_id": 123, "resource": "ifc/building-architecture" },
                        { "kind": "elements.select", "semantic_ids": ["2iPwJwpPDCSgMheXwk9cBT"], "resource": "ifc/building-architecture" },
                        { "kind": "elements.inspect", "semantic_ids": ["2iPwJwpPDCSgMheXwk9cBT"], "resource": "ifc/building-architecture", "mode": "replace" },
                        { "kind": "viewer.frame_visible" }
                    ]
                }
            ],
            "finalText": "optional short final note"
        }))
        .expect("schema example should serialize"),
        "Allowed toolCalls kinds are only `get_schema_context`, `get_model_details`, `get_entity_reference`, `get_query_playbook`, `get_relation_reference`, `run_readonly_cypher`, `run_project_readonly_cypher`, `describe_nodes`, `get_node_properties`, `get_neighbors`, and `emit_ui_actions`.".to_owned(),
        "Do not invent `request_tools`; use the concrete tools above directly.".to_owned(),
        "Allowed UI actions are only `graph.set_seeds`, `properties.show_node`, `elements.hide`, `elements.show`, `elements.select`, `elements.inspect`, `viewer.frame_visible`, and `viewer.clear_inspection`.".to_owned(),
        "Use exact action payload field names: `db_node_ids` for graph seeds, `db_node_id` for properties.show_node, and `semantic_ids` for element actions.".to_owned(),
        "`elements.inspect` also accepts `mode`: `replace`, `add`, or `remove`. Use `replace` for a new/only inspection focus, `add` for wording like `add`, `also`, `include`, or `plus`, and `remove` for wording like `remove`, `exclude`, or `subtract`.".to_owned(),
        "When a DB node id comes from a project-wide query result, include its `source_resource` as the UI action `resource`; DB node ids are only meaningful inside one IFC database.".to_owned(),
        "When semantic ids come from project-wide query results, preserve source by either passing `resource` on the element action or using the source-scoped `source_resource::GlobalId` id returned by the tool.".to_owned(),
        "Project-specific repo guidance may be provided through AGENTS.md; follow it when choosing Cypher and UI actions.".to_owned(),
        "Your job is not just to plan; it is to investigate until you have enough information to answer well or perform the requested viewer action.".to_owned(),
        "Rules:".to_owned(),
        "- Never invent DB ids or semantic ids.".to_owned(),
        "- When a Cypher tool result already includes `dbNodeIds`, `semanticElementIds`, `firstDbNodeId`, or `firstSemanticElementId`, use those extracted fields instead of re-deriving ids from raw rows.".to_owned(),
        "- Never default a missing id to `0`. If an id is missing, ask for another small query or use a different inspection tool.".to_owned(),
        "- Every `run_readonly_cypher` tool call must include a short `why` string explaining what you are trying to learn from that query.".to_owned(),
        "- When the bound resource starts with `project/` and the user asks a broad project-level question, prefer `run_project_readonly_cypher`; use `run_readonly_cypher` for single-IFC focused follow-ups only.".to_owned(),
        "- For project-wide overview/product-family summaries, do not combine several independent `OPTIONAL MATCH` aggregate branches in one query. That shape can explode into a Cartesian product. Prefer `MATCH (n) WHERE n.declared_entity IS NOT NULL RETURN n.declared_entity AS entity, count(*) AS count ORDER BY count DESC LIMIT 20`, or split counts into separate small label-first queries.".to_owned(),
        "- For bridge structural breakdowns, first list `IfcBridgePart` ids, then query contained products one bridge part at a time with `WHERE id(part) = ...`. Do not run one unanchored aggregate query from every bridge part through `IfcRelContainedInSpatialStructure`; that shape is known to be slow.".to_owned(),
        "- If a Cypher tool reports that it timed out and the query process was killed, briefly tell the user the previous query was too broad, then continue with a smaller anchored query. Do not retry the same broad shape.".to_owned(),
        "- For viewer actions with explicit complete/plural scope, bounded queries are only for discovery. The final action ids must be complete and must not use LIMIT unless the user asked for a sample/subset. Prefer `ifc_element_search` with `all_matches: true` and a focused entity label when available.".to_owned(),
        "- The current model schema is already provided. Use it. If entity meaning, relation shape, or query strategy is unclear, ask for schema context, entity reference, relation reference, or a query playbook before guessing.".to_owned(),
        "- Use recent session history to resolve vague follow-up requests like `show me the relations`, `show them`, or `what about that one`.".to_owned(),
        "- Before issuing a new discovery query for a repeated factual question, check sessionHistory for an earlier answer. If the fact was already established, answer from that prior finding and only run a small verification query when freshness or certainty really matters.".to_owned(),
        "- If prior history already identified a concrete object with a GlobalId or DB node id, reuse that identifier directly instead of rediscovering the object from scratch.".to_owned(),
        "- Read-only applies to graph/database inspection only. You are allowed to carry out approved viewer actions by returning them in `emit_ui_actions`.".to_owned(),
        "- When the user asks to hide, show, select, inspect, clear inspection, seed the graph, reveal properties, or frame the scene, prefer returning the corresponding validated viewer action instead of refusing on the grounds that this is a planning phase.".to_owned(),
        "- Treat `show`, `reveal`, or `display` for a concrete element as `elements.show`; do not also seed/open the graph unless the user explicitly asks for relations, graph, neighborhood, or connections.".to_owned(),
        "- If the user says they are done with inspection, thanks you after an inspection, or asks to return to normal rendering, emit `viewer.clear_inspection`.".to_owned(),
        "- Inspection is stateful: `elements.inspect` with `mode: \"replace\"` replaces the current inspection focus, `mode: \"add\"` preserves the existing focus and adds the returned ids, and `mode: \"remove\"` removes only those ids from the current focus.".to_owned(),
        "- `elements.hide`, `elements.show`, `elements.select`, and `elements.inspect` require renderable semantic ids, usually returned from `GlobalId` / `global_id` columns. In project mode, carry the source IFC resource with those ids.".to_owned(),
        "- If the query only returns DB node ids, use `graph.set_seeds`; do not emit element actions from DB ids alone.".to_owned(),
        "- Learn the general difference between semantic/container nodes and visible/product nodes. Do not rely on one-off entity exceptions alone.".to_owned(),
        "- Treat facility roots, project/site/building/storey nodes, relation nodes, aggregate/group nodes, and many `*Part` subdivision nodes as likely semantic/container candidates until the live graph proves otherwise.".to_owned(),
        "- In bridge/infrastructure contexts, treat `IfcFooting`, foundation-like products, piers, and abutments contained by `IfcBridgePart` as likely bridge substructure/support elements. Ground that classification in containment/type relations, not names alone.".to_owned(),
        "- For named bridge requests such as railway/rail/road/girder/arched bridge, first identify the matching `IfcBridge` root by returned name/object type, then anchor descendant/renderable-product queries to that one bridge. Do not use an unfiltered all-bridges descendant query for a specific bridge request.".to_owned(),
        "- For manhole requests in infrastructure models, check `IfcElementAssembly` / `IfcElementAssemblyType` first. In the sample infra project, sewer manholes are renderable `IfcElementAssembly` products with `GlobalId`; avoid broad unlabeled `MATCH (n)` text scans with `toLower(...)` for this lookup.".to_owned(),
        "- Treat concrete products with their own `GlobalId`, especially when they show placement/representation links or appear as contained products under a container, as stronger candidates for viewer element actions.".to_owned(),
        "- Before emitting a viewer element action for an unfamiliar entity family, do one quick renderability check: inspect the candidate's local relations, decide whether it behaves like a semantic/container node or a visible/product node, and if it looks semantic/container then descend to the contained or aggregated products.".to_owned(),
        "- If a node mostly exposes aggregation, containment, owner history, or other structural/context relations, that is a clue it may be semantic/container rather than the visible thing the user wants to act on.".to_owned(),
        "- Never use `toString(id(...))` in Cypher. When you need graph ids, return raw numeric ids as `id(n) AS node_id`.".to_owned(),
        "- Use transcript and finalText for prose. Do not use Cypher string conversion tricks to format ids for display.".to_owned(),
        "- Tool results may include an error object with `ok: false`, `tool`, `input`, and `error`. When that happens, inspect the error, simplify the query, and retry instead of stopping immediately.".to_owned(),
        "- Answer directly in transcript/finalText when the user mainly wants explanation; UI actions are optional.".to_owned(),
        "- Prefer `get_entity_reference` for entity-specific questions, `get_relation_reference` when the user is really asking about a relation family or role name, `get_model_details` when you want a quick model summary, `get_query_playbook` when you need help choosing a parser-safe query strategy, and `get_schema_context` when you need broader schema-family framing or schema-specific cautions.".to_owned(),
        "- For ad hoc Cypher, do not freestyle from scratch if a playbook would fit. Ask for `get_query_playbook` once, use the first returned playbook result, then adapt the recommended pattern minimally to the current question. Do not call `get_query_playbook` again for the same user question.".to_owned(),
        "- Do not mechanically call both schema tools at the start of every turn. Use the smallest schema lookup that will unblock the next good live query.".to_owned(),
        "- Treat questions like `what is`, `what type`, `why`, `how is this connected`, `what relations`, and `how should I query this` as schema-sensitive by default.".to_owned(),
        "- Prefer `describe_nodes` and `get_node_properties` for explanation and property inspection before reaching for raw Cypher.".to_owned(),
        "- Use `properties.show_node` when you want the frontend to open the Properties tab on a specific graph node.".to_owned(),
        "- This Cypher runtime is happier with simple shapes than clever ones. Start with direct label matches, plain traversals, and explicit RETURN columns.".to_owned(),
        "- Avoid starting with parser-fragile patterns like `any(...)` over property lists, `coalesce(...)`/`toLower(...)` chains in `WHERE`, `labels(n)` membership filters, complex `UNION`, or unbounded/unconstrained variable-length traversals.".to_owned(),
        "- Bounded variable-length traversals are allowed for exploration when a simple one-hop query is not enough. Prefer small ranges like `*1..3` or `*0..2`.".to_owned(),
        "- When you use variable-length traversal for exploration, prefer a relation-constrained form such as `[:RELATED_OBJECTS|RELATED_ELEMENTS*1..3]` over a bare `[*1..3]` walk whenever the relation family is known.".to_owned(),
        "- Use bounded varlen as a discovery step to find candidate descendants or relation context. Once you find promising candidates, switch back to a simpler query to inspect the concrete products.".to_owned(),
        "- For discovery-by-name questions like `is there a kitchen unit`, first ask for a relevant query playbook or use a likely entity label with a simple scan, then inspect the returned names/object types before attempting a more filtered query.".to_owned(),
        "- Reuse ids and facts already present in sessionHistory, transcript, and prior toolResults before issuing another discovery query.".to_owned(),
        "- Never repeat the exact same discovery tool call in one turn unless the previous result was empty or clearly insufficient. Reuse the prior result and move forward instead of rediscovering the same fact.".to_owned(),
        "- If you already know the target DB node id and the user asks for properties or explanation, prefer `get_node_properties`, `describe_nodes`, and `properties.show_node` over another raw Cypher lookup.".to_owned(),
        "- Emit one coherent `emit_ui_actions` bundle near the end of the turn. Avoid repeating `elements.select`, `elements.inspect`, `properties.show_node`, `viewer.frame_visible`, or `viewer.clear_inspection` multiple times in the same turn unless the target changed.".to_owned(),
        "- If a requested 'type' answer only yields opaque numeric codes or low-level enum values, say that plainly and then inspect names and nearby relationships to explain the element's role in the model.".to_owned(),
        "- For product questions like slabs, walls, roofs, and doors, relationship context is often more useful than raw property values. Look for aggregation, containment, type, property-set, and material relationships.".to_owned(),
        "- Distinguish observation from inference. If you infer, say so briefly and ground it in the returned graph facts.".to_owned(),
        "- If the first result is thin, opaque, or only partially answers the question, keep exploring instead of stopping early.".to_owned(),
        "- It is good to take 2-4 small tool steps when the problem is ambiguous or the first query is not enough.".to_owned(),
        "- You may use up to two tightly-related inspection tool calls in one step when they naturally pair together, for example schema context plus entity reference, or node description plus neighbors.".to_owned(),
        "- For ambiguous action requests like `hide the roof`, refine toward renderable descendants and related products rather than refusing after the first semantic hit.".to_owned(),
        "- When a question mixes explanation and action, it is good to do both: investigate, explain briefly, and emit the action if you found the right ids.".to_owned(),
        "- If tool results already contain enough information, prefer `emit_ui_actions` or a direct answer.".to_owned(),
        "- For relation-summary questions like `what relations are slabs connected to`, prefer a relevant relation reference or query playbook, then use a simple query shape such as `MATCH (slab:IfcSlab)-[r]-(other) RETURN type(r) AS relation, count(*) AS connections ORDER BY connections DESC LIMIT 24` before trying more complex filters.".to_owned(),
        "- For broad overview questions like `what can you tell me about the model`, `what is in this model`, or `what kind of model is this`, start with `get_model_details` first. Only use `get_query_playbook` afterward if you still need a better query shape for a follow-up question.".to_owned(),
        "- For broad material questions like `what is the house built of`, avoid schema thrash. Start with a single query playbook lookup or a small live material query such as `MATCH (:IfcRelAssociatesMaterial)--(material:IfcMaterial) RETURN DISTINCT material.Name AS material_name LIMIT 24`, then move immediately to Cypher. Do not keep asking the playbook for alternative answers once you already have a plausible material query shape.".to_owned(),
        "- Keep transcript entries short and useful.".to_owned(),
        "- If you are done, you may return `toolCalls: []` and use `finalText`.".to_owned(),
        String::new(),
        format!(
            "Current model schema: {}{}.",
            request.schema_id,
            request
                .schema_slug
                .as_deref()
                .map(|slug| format!(" ({slug})"))
                .unwrap_or_default()
        ),
        "Current turn request JSON:".to_owned(),
        serde_json::to_string_pretty(&serde_json::json!({
            "resource": request.resource,
            "schemaId": request.schema_id,
            "schemaSlug": request.schema_slug,
            "userInput": request.user_input,
            "sessionHistory": request.session_history,
            "transcript": request.transcript,
            "toolResults": request.tool_results,
        }))
        .expect("turn request should serialize"),
    ];

    prompt.join("\n")
}

#[derive(Debug)]
enum OpencodeResponseParseError {
    InvalidJson(serde_json::Error),
    ProviderError { message: String },
}

fn parse_opencode_response(
    stdout: &[u8],
) -> Result<OpencodeTurnResponse, OpencodeResponseParseError> {
    let stdout_text = String::from_utf8_lossy(stdout);
    if let Ok(value) = serde_json::from_str::<Value>(&stdout_text) {
        if let Some(message) = opencode_error_event_message(&value) {
            return Err(OpencodeResponseParseError::ProviderError { message });
        }
        return serde_json::from_value(normalize_response_shape(value))
            .map_err(OpencodeResponseParseError::InvalidJson);
    }

    let extracted = extract_assistant_payload(&stdout_text);
    if let Some(error_message) = extracted.error_message {
        return Err(OpencodeResponseParseError::ProviderError {
            message: error_message,
        });
    }

    let mut last_error = None;
    for assistant_text in extracted.assistant_texts.iter().rev() {
        let normalized_json = normalize_assistant_json(assistant_text);
        match serde_json::from_str::<Value>(&normalized_json) {
            Ok(value) => {
                return serde_json::from_value(normalize_response_shape(value))
                    .map_err(OpencodeResponseParseError::InvalidJson);
            }
            Err(error) => last_error = Some(error),
        }
    }

    Err(OpencodeResponseParseError::InvalidJson(
        last_error.unwrap_or_else(|| {
            serde_json::Error::io(std::io::Error::new(
                std::io::ErrorKind::Other,
                "opencode did not return any parseable assistant JSON payload",
            ))
        }),
    ))
}

struct ExtractedAssistantPayload {
    assistant_texts: Vec<String>,
    error_message: Option<String>,
}

fn extract_assistant_payload(stdout: &str) -> ExtractedAssistantPayload {
    let mut parts = Vec::new();
    let mut error_message = None;
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(event) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };

        if event.get("type").and_then(Value::as_str) == Some("text") {
            if let Some(text) = event
                .get("part")
                .and_then(|part| part.get("text"))
                .and_then(Value::as_str)
            {
                parts.push(text.to_owned());
                continue;
            }
        }

        if event.get("type").and_then(Value::as_str) == Some("message") {
            if let Some(items) = event.get("content").and_then(Value::as_array) {
                for item in items {
                    if let Some(text) = item.get("text").and_then(Value::as_str) {
                        parts.push(text.to_owned());
                    }
                }
                continue;
            }
        }

        if event.get("type").and_then(Value::as_str) == Some("result") {
            if let Some(output) = event.get("output").and_then(Value::as_str) {
                parts.push(output.to_owned());
                continue;
            }
        }

        if let Some(message) = opencode_error_event_message(&event) {
            error_message = Some(message);
        }
    }

    ExtractedAssistantPayload {
        assistant_texts: parts,
        error_message,
    }
}

fn opencode_error_event_message(event: &Value) -> Option<String> {
    if event.get("type").and_then(Value::as_str) != Some("error") {
        return None;
    }

    let raw_message = event
        .get("error")
        .and_then(|error| {
            error
                .get("data")
                .and_then(|data| data.get("message"))
                .and_then(Value::as_str)
                .or_else(|| error.get("message").and_then(Value::as_str))
                .or_else(|| error.get("name").and_then(Value::as_str))
        })
        .unwrap_or("unknown opencode error");

    Some(normalize_provider_error_message(raw_message))
}

fn normalize_provider_error_message(raw_message: &str) -> String {
    let trimmed = raw_message.trim();
    let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
        return trimmed.to_owned();
    };

    let Some(error) = value.get("error") else {
        return trimmed.to_owned();
    };
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or(trimmed);
    let code = error
        .get("code")
        .and_then(Value::as_str)
        .or_else(|| error.get("type").and_then(Value::as_str));

    match code {
        Some(code) if !code.trim().is_empty() => {
            format!("provider returned {code}: {message}")
        }
        _ => message.to_owned(),
    }
}

fn should_retry_step_error(error: &OpencodeExecutorError) -> bool {
    match error {
        OpencodeExecutorError::ProviderError { message, .. } => {
            is_transient_provider_error_message(message)
        }
        _ => false,
    }
}

fn is_transient_provider_error_message(message: &str) -> bool {
    let normalized = message.trim().to_ascii_lowercase();
    normalized.contains("server_error")
        || normalized.contains("temporarily unavailable")
        || normalized.contains("overloaded")
        || normalized.contains("please retry")
}

fn normalize_assistant_json(text: &str) -> String {
    let trimmed = text.trim();
    if let Some(stripped) = trimmed.strip_prefix("```") {
        let without_language = stripped
            .split_once('\n')
            .map(|(_, rest)| rest)
            .unwrap_or(stripped);
        return without_language
            .strip_suffix("```")
            .unwrap_or(without_language)
            .trim()
            .to_owned();
    }

    let start = trimmed.find('{');
    let end = trimmed.rfind('}');
    if let (Some(start), Some(end)) = (start, end) {
        if end >= start {
            return trimmed[start..=end].to_owned();
        }
    }
    trimmed.to_owned()
}

fn serialize_tool_error(tool: &str, input: &str, error: &str) -> Result<String, serde_json::Error> {
    serde_json::to_string(&serde_json::json!({
        "ok": false,
        "tool": tool,
        "input": input,
        "error": error,
    }))
}

fn normalize_response_shape(mut response: Value) -> Value {
    let Some(object) = response.as_object_mut() else {
        return response;
    };

    let tool_calls = object
        .remove("toolCalls")
        .or_else(|| object.remove("tool_calls"));
    if let Some(Value::Array(tool_calls)) = tool_calls {
        object.insert(
            "toolCalls".to_owned(),
            Value::Array(tool_calls.into_iter().map(normalize_tool_call).collect()),
        );
    }

    response
}

fn normalize_tool_call(tool_call: Value) -> Value {
    let Some(object) = tool_call.as_object() else {
        return tool_call;
    };
    let mut normalized = object.clone();
    merge_tool_call_arguments(&mut normalized);
    if let Some(kind) = infer_tool_call_kind(&normalized) {
        normalized.insert("kind".to_owned(), Value::String(kind));
    }
    match normalized.get("kind").and_then(Value::as_str) {
        Some("run_readonly_cypher") | Some("run_project_readonly_cypher") => {
            if let Some(value) = get_first_value(&normalized, &["why", "rationale", "reason"]) {
                normalized.insert("why".to_owned(), value);
                normalized.remove("rationale");
                normalized.remove("reason");
            }
            if let Some(Value::Array(resources)) = get_first_value(
                &normalized,
                &[
                    "resource_filter",
                    "resourceFilter",
                    "resources",
                    "resource_ids",
                ],
            ) {
                normalized.insert(
                    "resource_filter".to_owned(),
                    Value::Array(resources.clone()),
                );
                normalized.remove("resourceFilter");
                normalized.remove("resources");
                normalized.remove("resource_ids");
            }
        }
        Some("get_schema_context") => {
            normalized.remove("schema");
        }
        Some("get_model_details") => {}
        Some("get_entity_reference") => {
            if let Some(Value::Array(names)) = get_first_value(
                &normalized,
                &["entity_names", "entityNames", "entities", "entityTypes"],
            ) {
                normalized.insert("entity_names".to_owned(), Value::Array(names.clone()));
                normalized.remove("entityNames");
                normalized.remove("entities");
                normalized.remove("entityTypes");
            }
        }
        Some("request_tools") => {
            if let Some(Value::Array(tools)) = get_first_value(
                &normalized,
                &[
                    "tools",
                    "toolNames",
                    "tool_names",
                    "requestedTools",
                    "requested_tools",
                ],
            ) {
                let canonical_tools = tools
                    .iter()
                    .filter_map(Value::as_str)
                    .map(normalize_tool_function_kind)
                    .filter(|tool| !tool.trim().is_empty())
                    .map(Value::String)
                    .collect::<Vec<_>>();
                normalized.insert("tools".to_owned(), Value::Array(canonical_tools));
                normalized.remove("toolNames");
                normalized.remove("tool_names");
                normalized.remove("requestedTools");
                normalized.remove("requested_tools");
            }
        }
        Some("get_query_playbook") => {
            if let Some(value) = get_first_value(
                &normalized,
                &["goal", "task", "query", "query_goal", "queryGoal", "topic"],
            ) {
                normalized.insert("goal".to_owned(), value.clone());
                normalized.remove("task");
                normalized.remove("query");
                normalized.remove("query_goal");
                normalized.remove("queryGoal");
                if normalized.get("topic") == Some(&value) {
                    normalized.remove("topic");
                }
            }
            if let Some(Value::Array(names)) = get_first_value(
                &normalized,
                &["entity_names", "entityNames", "entities", "entityTypes"],
            ) {
                normalized.insert("entity_names".to_owned(), Value::Array(names.clone()));
                normalized.remove("entityNames");
                normalized.remove("entities");
                normalized.remove("entityTypes");
            }
        }
        Some("get_relation_reference") => {
            if let Some(Value::Array(names)) = get_first_value(
                &normalized,
                &[
                    "relation_names",
                    "relationNames",
                    "names",
                    "relations",
                    "relationTypes",
                ],
            ) {
                normalized.insert("relation_names".to_owned(), Value::Array(names.clone()));
                normalized.remove("relationNames");
                normalized.remove("names");
                normalized.remove("relations");
                normalized.remove("relationTypes");
            }
        }
        Some("describe_nodes") | Some("get_neighbors") => {
            if let Some(Value::Array(ids)) = get_first_value(
                &normalized,
                &["db_node_ids", "dbNodeIds", "nodeIds", "node_ids", "ids"],
            ) {
                normalized.insert("db_node_ids".to_owned(), Value::Array(ids.clone()));
                normalized.remove("dbNodeIds");
                normalized.remove("nodeIds");
                normalized.remove("node_ids");
                if normalized.get("ids") == Some(&Value::Array(ids.clone())) {
                    normalized.remove("ids");
                }
            }
        }
        Some("get_node_properties") => {
            if let Some(value) =
                get_first_value(&normalized, &["db_node_id", "dbNodeId", "nodeId", "id"])
            {
                normalized.insert("db_node_id".to_owned(), value.clone());
                normalized.remove("dbNodeId");
                normalized.remove("nodeId");
                if normalized.get("id") == Some(&value) {
                    normalized.remove("id");
                }
            }
        }
        Some("emit_ui_actions") => {
            if let Some(Value::Array(actions)) = normalized.remove("actions") {
                normalized.insert(
                    "actions".to_owned(),
                    Value::Array(actions.into_iter().map(normalize_ui_action).collect()),
                );
            }
        }
        _ => {}
    }
    normalized.remove("args");
    normalized.remove("arguments");
    normalized.remove("function");
    normalized.remove("function_name");
    normalized.remove("name");
    normalized.remove("tool_name");
    Value::Object(normalized)
}

fn merge_tool_call_arguments(normalized: &mut serde_json::Map<String, Value>) {
    if let Some(args) = normalized.get("args").cloned() {
        merge_tool_call_argument_value(normalized, args);
    }
    if let Some(arguments) = normalized.get("arguments").cloned() {
        merge_tool_call_argument_value(normalized, arguments);
    }
}

fn merge_tool_call_argument_value(normalized: &mut serde_json::Map<String, Value>, value: Value) {
    match value {
        Value::Object(mut map) => {
            if let Some(nested) = map.remove("args") {
                merge_tool_call_argument_value(normalized, nested);
            }
            if let Some(nested) = map.remove("arguments") {
                merge_tool_call_argument_value(normalized, nested);
            }
            for (key, value) in map {
                normalized.entry(key).or_insert(value);
            }
        }
        Value::String(raw) => {
            if let Ok(parsed) = serde_json::from_str::<Value>(&raw) {
                merge_tool_call_argument_value(normalized, parsed);
            }
        }
        _ => {}
    }
}

fn infer_tool_call_kind(normalized: &serde_json::Map<String, Value>) -> Option<String> {
    if let Some(kind) = normalized.get("kind").and_then(Value::as_str) {
        let trimmed = kind.trim();
        if !trimmed.is_empty() && !is_tool_call_wrapper_kind(trimmed) {
            return Some(normalize_tool_function_kind(trimmed));
        }
    }

    if let Some(kind) = normalized
        .get("function_name")
        .and_then(Value::as_str)
        .or_else(|| normalized.get("tool_name").and_then(Value::as_str))
        .or_else(|| normalized.get("name").and_then(Value::as_str))
    {
        let trimmed = kind.trim();
        if !trimmed.is_empty() {
            return Some(normalize_tool_function_kind(trimmed));
        }
    }

    if let Some(function) = normalized.get("function").and_then(Value::as_str) {
        let trimmed = function.trim();
        if trimmed.is_empty() {
            return None;
        }
        if is_tool_call_wrapper_kind(trimmed) {
            return normalized
                .get("function_name")
                .and_then(Value::as_str)
                .map(|value| normalize_tool_function_kind(value.trim()))
                .or_else(|| {
                    normalized
                        .get("tool_name")
                        .and_then(Value::as_str)
                        .map(|value| normalize_tool_function_kind(value.trim()))
                })
                .or_else(|| {
                    normalized
                        .get("name")
                        .and_then(Value::as_str)
                        .map(|value| normalize_tool_function_kind(value.trim()))
                });
        }
        return Some(normalize_tool_function_kind(trimmed));
    }

    None
}

fn is_tool_call_wrapper_kind(value: &str) -> bool {
    matches!(
        value.trim(),
        "call" | "function" | "tool" | "tool_call" | "toolCall" | "function_call"
    )
}

fn normalize_tool_function_kind(function: &str) -> String {
    match function.trim() {
        "run_readonly_cypher" | "runReadonlyCypher" => "run_readonly_cypher".to_owned(),
        "run_project_readonly_cypher"
        | "runProjectReadonlyCypher"
        | "project_readonly_cypher"
        | "projectReadonlyCypher" => "run_project_readonly_cypher".to_owned(),
        "get_schema" | "getSchema" | "get_schema_context" | "getSchemaContext" => {
            "get_schema_context".to_owned()
        }
        "get_model_details" | "getModelDetails" => "get_model_details".to_owned(),
        "get_entity_reference" | "getEntityReference" => "get_entity_reference".to_owned(),
        "get_query_playbook" | "getQueryPlaybook" => "get_query_playbook".to_owned(),
        "get_relation_reference" | "getRelationReference" => "get_relation_reference".to_owned(),
        "request_tools" | "requestTools" => "request_tools".to_owned(),
        "describe_nodes" | "describeNodes" => "describe_nodes".to_owned(),
        "get_node_properties" | "getNodeProperties" => "get_node_properties".to_owned(),
        "get_neighbors" | "getNeighbors" => "get_neighbors".to_owned(),
        "emit_ui_actions" | "emitUiActions" => "emit_ui_actions".to_owned(),
        other => other.to_owned(),
    }
}

fn normalize_ui_action(action: Value) -> Value {
    let Some(object) = action.as_object() else {
        return action;
    };
    let mut normalized = object.clone();

    let db_node_ids = get_first_value(
        &normalized,
        &["db_node_ids", "dbNodeIds", "nodeIds", "node_ids", "ids"],
    );
    let semantic_ids = get_first_value(
        &normalized,
        &[
            "semantic_ids",
            "semanticIds",
            "elementIds",
            "element_ids",
            "ids",
        ],
    );

    if let Some(kind) = normalized.get("kind").and_then(Value::as_str) {
        if let Some(canonical_kind) = normalize_ui_action_kind(kind) {
            normalized.insert("kind".to_owned(), Value::String(canonical_kind));
        }
    }
    if let Some(resource) = get_first_value(
        &normalized,
        &[
            "resource",
            "source_resource",
            "sourceResource",
            "ifcResource",
        ],
    ) {
        normalized.insert("resource".to_owned(), resource.clone());
        normalized.remove("source_resource");
        normalized.remove("sourceResource");
        normalized.remove("ifcResource");
    }

    match normalized.get("kind").and_then(Value::as_str) {
        Some("graph.set_seeds") => {
            if let Some(Value::Array(ids)) = db_node_ids {
                normalized.insert("db_node_ids".to_owned(), Value::Array(ids.clone()));
                normalized.remove("dbNodeIds");
                normalized.remove("nodeIds");
                normalized.remove("node_ids");
                if normalized.get("ids") == Some(&Value::Array(ids.clone())) {
                    normalized.remove("ids");
                }
            }
        }
        Some("properties.show_node") => {
            if let Some(value) =
                get_first_value(&normalized, &["db_node_id", "dbNodeId", "nodeId", "id"])
            {
                normalized.insert("db_node_id".to_owned(), value.clone());
                normalized.remove("dbNodeId");
                normalized.remove("nodeId");
                if normalized.get("id") == Some(&value) {
                    normalized.remove("id");
                }
            }
        }
        Some("elements.hide")
        | Some("elements.show")
        | Some("elements.select")
        | Some("elements.inspect") => {
            if let Some(Value::Array(ids)) = semantic_ids {
                normalized.insert("semantic_ids".to_owned(), Value::Array(ids.clone()));
                normalized.remove("semanticIds");
                normalized.remove("elementIds");
                normalized.remove("element_ids");
                if normalized.get("ids") == Some(&Value::Array(ids.clone())) {
                    normalized.remove("ids");
                }
            }
            if matches!(
                normalized.get("kind").and_then(Value::as_str),
                Some("elements.inspect")
            ) {
                if let Some(mode) =
                    get_first_value(&normalized, &["mode", "inspection_mode", "inspectionMode"])
                        .and_then(|value| value.as_str().and_then(parse_inspection_mode))
                {
                    normalized.insert(
                        "mode".to_owned(),
                        Value::String(
                            match mode {
                                InspectionUpdateMode::Replace => "replace",
                                InspectionUpdateMode::Add => "add",
                                InspectionUpdateMode::Remove => "remove",
                            }
                            .to_owned(),
                        ),
                    );
                    normalized.remove("inspection_mode");
                    normalized.remove("inspectionMode");
                }
            }
        }
        _ => {}
    }

    Value::Object(normalized)
}

fn normalize_ui_action_kind(kind: &str) -> Option<String> {
    match kind.trim() {
        "graph.set_seeds" | "graph.setSeeds" | "graphSetSeeds" => {
            Some("graph.set_seeds".to_owned())
        }
        "properties.show_node" | "properties.showNode" | "propertiesShowNode" => {
            Some("properties.show_node".to_owned())
        }
        "elements.hide" | "elements.hideElements" | "elementsHide" => {
            Some("elements.hide".to_owned())
        }
        "elements.show" | "elements.showElements" | "elementsShow" => {
            Some("elements.show".to_owned())
        }
        "elements.select" | "elements.selectElements" | "elementsSelect" => {
            Some("elements.select".to_owned())
        }
        "elements.inspect" | "elements.inspectElements" | "elementsInspect" => {
            Some("elements.inspect".to_owned())
        }
        "viewer.frame_visible" | "viewer.frameVisible" | "viewerFrameVisible" | "frame" => {
            Some("viewer.frame_visible".to_owned())
        }
        "viewer.clear_inspection"
        | "viewer.clearInspection"
        | "viewerClearInspection"
        | "clear_inspection"
        | "clearInspection" => Some("viewer.clear_inspection".to_owned()),
        _ => None,
    }
}

fn parse_inspection_mode(value: &str) -> Option<InspectionUpdateMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "replace" | "set" | "focus" | "only" => Some(InspectionUpdateMode::Replace),
        "add" | "append" | "include" | "plus" => Some(InspectionUpdateMode::Add),
        "remove" | "subtract" | "exclude" | "drop" => Some(InspectionUpdateMode::Remove),
        _ => None,
    }
}

fn get_first_value(object: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<Value> {
    keys.iter().find_map(|key| object.get(*key).cloned())
}

fn agent_transcript_from_opencode(event: OpencodeTranscriptEvent) -> AgentTranscriptEvent {
    match event.kind {
        OpencodeTranscriptEventKind::System => AgentTranscriptEvent::system(event.text),
        OpencodeTranscriptEventKind::User => AgentTranscriptEvent::user(event.text),
        OpencodeTranscriptEventKind::Tool => AgentTranscriptEvent::tool(event.text),
        OpencodeTranscriptEventKind::Assistant => AgentTranscriptEvent::assistant(event.text),
    }
}

fn opencode_transcript_from_agent(event: AgentTranscriptEvent) -> OpencodeTranscriptEvent {
    OpencodeTranscriptEvent {
        kind: match event.kind {
            super::agent_executor::AgentTranscriptEventKind::System => {
                OpencodeTranscriptEventKind::System
            }
            super::agent_executor::AgentTranscriptEventKind::User => {
                OpencodeTranscriptEventKind::User
            }
            super::agent_executor::AgentTranscriptEventKind::Tool => {
                OpencodeTranscriptEventKind::Tool
            }
            super::agent_executor::AgentTranscriptEventKind::Assistant => {
                OpencodeTranscriptEventKind::Assistant
            }
        },
        text: event.text,
    }
}

fn parse_env_u64(key: &'static str, value: &OsString) -> Result<u64, OpencodeExecutorConfigError> {
    value.to_string_lossy().parse::<u64>().map_err(|_| {
        OpencodeExecutorConfigError::InvalidUnsignedInteger {
            key,
            value: value.to_string_lossy().into_owned(),
        }
    })
}

fn parse_env_usize(
    key: &'static str,
    value: &OsString,
) -> Result<usize, OpencodeExecutorConfigError> {
    value.to_string_lossy().parse::<usize>().map_err(|_| {
        OpencodeExecutorConfigError::InvalidUnsignedInteger {
            key,
            value: value.to_string_lossy().into_owned(),
        }
    })
}

fn split_args(value: &str) -> Vec<String> {
    value
        .split_ascii_whitespace()
        .filter(|segment| !segment.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn parse_opencode_models_output(bytes: &[u8]) -> Vec<OpencodeDiscoveredModel> {
    let mut models = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let text = String::from_utf8_lossy(bytes);
    let lines = text.lines().collect::<Vec<_>>();
    let mut index = 0usize;

    while index < lines.len() {
        let line = lines[index].trim();
        if !looks_like_model_id(line) {
            index = index.saturating_add(1);
            continue;
        }

        let model_id = line.to_owned();
        index = index.saturating_add(1);
        let mut variants = Vec::new();
        while index < lines.len() && lines[index].trim().is_empty() {
            index = index.saturating_add(1);
        }
        if index < lines.len() && lines[index].trim_start().starts_with('{') {
            let mut json_lines = Vec::new();
            let mut depth = 0i64;
            while index < lines.len() {
                let json_line = lines[index];
                depth += json_line.chars().filter(|value| *value == '{').count() as i64;
                depth -= json_line.chars().filter(|value| *value == '}').count() as i64;
                json_lines.push(json_line);
                index = index.saturating_add(1);
                if depth <= 0 {
                    break;
                }
            }
            variants = parse_opencode_model_variants(&json_lines.join("\n"));
        }

        if seen.insert(model_id.clone()) {
            models.push(OpencodeDiscoveredModel {
                id: model_id,
                variants,
            });
        }
    }

    models
}

fn parse_opencode_model_variants(json: &str) -> Vec<String> {
    let mut variants: Vec<String> = serde_json::from_str::<Value>(json)
        .ok()
        .and_then(|value| {
            value
                .get("variants")
                .and_then(Value::as_object)
                .map(|variants| variants.keys().cloned().collect())
        })
        .unwrap_or_default();
    variants.retain(|variant| is_supported_opencode_variant(variant));
    variants.sort_by(|left, right| {
        variant_rank(left)
            .cmp(&variant_rank(right))
            .then_with(|| left.cmp(right))
    });
    variants.dedup();
    variants
}

fn variant_rank(value: &str) -> usize {
    match value {
        "minimal" => 1,
        "low" => 2,
        "medium" => 3,
        "high" => 4,
        "xhigh" => 5,
        "max" => 6,
        _ => usize::MAX,
    }
}

fn is_supported_opencode_variant(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty() && !trimmed.eq_ignore_ascii_case("none")
}

fn looks_like_model_id(value: &str) -> bool {
    let Some((provider, model)) = value.split_once('/') else {
        return false;
    };
    !provider.trim().is_empty()
        && !model.trim().is_empty()
        && !value.trim_start().starts_with('{')
        && !value.contains(char::is_whitespace)
}

fn resolve_executable(configured: &Path) -> Result<PathBuf, OpencodeExecutorError> {
    if configured.is_absolute() || configured.components().count() > 1 {
        if configured.is_file() {
            return Ok(configured.to_path_buf());
        }
        return Err(OpencodeExecutorError::MissingExecutable {
            configured: configured.to_path_buf(),
        });
    }

    let Some(path_env) = env::var_os("PATH") else {
        return Err(OpencodeExecutorError::MissingExecutable {
            configured: configured.to_path_buf(),
        });
    };

    for directory in env::split_paths(&path_env) {
        let candidate = directory.join(configured);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    Err(OpencodeExecutorError::MissingExecutable {
        configured: configured.to_path_buf(),
    })
}

fn read_limited<R>(mut reader: R, max_bytes: usize) -> OutputCapture
where
    R: Read,
{
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 8 * 1024];

    loop {
        let bytes_read = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(_) => break,
        };

        let remaining_capacity = max_bytes.saturating_sub(bytes.len());
        let bytes_to_copy = remaining_capacity.min(bytes_read);
        bytes.extend_from_slice(&buffer[..bytes_to_copy]);
        if bytes.len() >= max_bytes {
            break;
        }
    }

    OutputCapture { bytes }
}

fn read_limited_with_progress<R>(
    reader: R,
    max_bytes: usize,
    stream_name: &'static str,
    emit_progress: bool,
    last_activity: Arc<Mutex<Instant>>,
    progress_events: Arc<Mutex<Vec<AgentTranscriptEvent>>>,
) -> OutputCapture
where
    R: Read,
{
    let mut bytes = Vec::new();
    let mut reader = BufReader::new(reader);
    let mut line = Vec::new();

    loop {
        line.clear();
        let bytes_read = match reader.read_until(b'\n', &mut line) {
            Ok(0) => break,
            Ok(count) => count,
            Err(_) => break,
        };

        if let Ok(mut guard) = last_activity.lock() {
            *guard = Instant::now();
        }

        let remaining_capacity = max_bytes.saturating_sub(bytes.len());
        let bytes_to_copy = remaining_capacity.min(bytes_read);
        bytes.extend_from_slice(&line[..bytes_to_copy]);

        if emit_progress {
            for event in opencode_stream_progress_events(stream_name, &line) {
                println!(
                    "w web opencode progress {}",
                    summarize_agent_transcript_event(&event)
                );
                if let Ok(mut queue) = progress_events.lock() {
                    queue.push(event);
                }
            }
        }

        if bytes.len() >= max_bytes {
            break;
        }
    }

    OutputCapture { bytes }
}

fn drain_progress_events(
    progress_events: &Arc<Mutex<Vec<AgentTranscriptEvent>>>,
    progress: &mut dyn AgentProgressSink,
    executable: &Path,
) -> Result<(), OpencodeExecutorError> {
    let events = {
        let mut guard =
            progress_events
                .lock()
                .map_err(|source| OpencodeExecutorError::WaitFailed {
                    executable: executable.to_path_buf(),
                    source: std::io::Error::other(format!(
                        "opencode progress queue lock poisoned: {source}"
                    )),
                })?;
        if guard.is_empty() {
            return Ok(());
        }
        std::mem::take(&mut *guard)
    };

    for event in events {
        progress.emit(event);
    }

    Ok(())
}

fn opencode_stream_progress_events(stream_name: &str, line: &[u8]) -> Vec<AgentTranscriptEvent> {
    let text = String::from_utf8_lossy(line);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return opencode_stream_progress_events_from_value(stream_name, &value);
    }

    vec![AgentTranscriptEvent::system(format!(
        "{}: {}",
        stream_name,
        shorten_for_progress(trimmed, 120)
    ))]
}

fn opencode_stream_progress_events_from_value(
    stream_name: &str,
    event: &Value,
) -> Vec<AgentTranscriptEvent> {
    let Some(kind) = event.get("type").and_then(Value::as_str) else {
        return vec![AgentTranscriptEvent::system(format!(
            "{}: event",
            stream_name
        ))];
    };
    match kind {
        "step_start" => vec![AgentTranscriptEvent::system(
            "opencode progress: step started".to_owned(),
        )],
        "step_finish" => vec![AgentTranscriptEvent::system(
            "opencode progress: step finished".to_owned(),
        )],
        "tool_use" | "tool_result" => {
            let events = opencode_tool_progress_events(event);
            if events.is_empty() {
                vec![AgentTranscriptEvent::system(format!(
                    "{}: tool event",
                    stream_name
                ))]
            } else {
                events
            }
        }
        "text" => {
            if let Some(part_text) = event
                .get("part")
                .and_then(|part| part.get("text"))
                .and_then(Value::as_str)
            {
                if let Some(response) = parse_opencode_turn_response_text(part_text) {
                    let mut events = Vec::new();
                    events.extend(
                        response
                            .transcript
                            .into_iter()
                            .map(agent_transcript_from_opencode),
                    );
                    events.extend(
                        response
                            .tool_calls
                            .iter()
                            .map(opencode_progress_event_for_tool_call),
                    );
                    if let Some(final_text) = response.final_text {
                        let trimmed = final_text.trim();
                        if !trimmed.is_empty() {
                            events.push(AgentTranscriptEvent::assistant(trimmed.to_owned()));
                        }
                    }
                    if !events.is_empty() {
                        return events;
                    }
                }
            }
            vec![AgentTranscriptEvent::system(format!(
                "{}: model produced a progress chunk",
                stream_name
            ))]
        }
        "message" => {
            let mut events = Vec::new();
            if let Some(content) = event.get("content").and_then(Value::as_array) {
                for item in content {
                    if let Some(text) = item.get("text").and_then(Value::as_str) {
                        let trimmed = text.trim();
                        if !trimmed.is_empty() {
                            events.push(AgentTranscriptEvent::assistant(trimmed.to_owned()));
                        }
                    }
                }
            }
            if events.is_empty() {
                events.push(AgentTranscriptEvent::system(format!(
                    "{}: message stream chunk",
                    stream_name
                )));
            }
            events
        }
        "result" => {
            let mut events = Vec::new();
            if let Some(output) = event.get("output").and_then(Value::as_str) {
                let trimmed = output.trim();
                if !trimmed.is_empty() {
                    events.push(AgentTranscriptEvent::assistant(trimmed.to_owned()));
                }
            }
            if events.is_empty() {
                events.push(AgentTranscriptEvent::system(format!(
                    "{}: result stream chunk",
                    stream_name
                )));
            }
            events
        }
        "error" => vec![AgentTranscriptEvent::system(
            opencode_error_event_message(event).unwrap_or_else(|| "error event".to_owned()),
        )],
        other => vec![AgentTranscriptEvent::system(format!(
            "{}: {other} event",
            stream_name
        ))],
    }
}

fn shorten_for_progress(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    let mut shortened = trimmed.chars().take(max_chars).collect::<String>();
    if trimmed.chars().count() > max_chars {
        shortened.push_str("...");
    }
    shortened
}

fn summarize_agent_transcript_event(event: &AgentTranscriptEvent) -> String {
    let prefix = match event.kind {
        super::agent_executor::AgentTranscriptEventKind::System => "system",
        super::agent_executor::AgentTranscriptEventKind::User => "user",
        super::agent_executor::AgentTranscriptEventKind::Tool => "",
        super::agent_executor::AgentTranscriptEventKind::Assistant => "assistant",
    };
    if prefix.is_empty() {
        shorten_for_progress(&event.text.replace('\n', " | "), 160)
    } else {
        format!("{prefix}: {}", shorten_for_progress(&event.text, 160))
    }
}

fn opencode_tool_progress_events(event: &Value) -> Vec<AgentTranscriptEvent> {
    let Some(part) = event.get("part").and_then(Value::as_object) else {
        return Vec::new();
    };
    let state = part
        .get("state")
        .and_then(Value::as_object)
        .or_else(|| Some(part));

    let tool_name = state
        .and_then(|object| get_first_string(object, &["tool", "toolName", "name"]))
        .or_else(|| get_first_string(part, &["tool", "toolName", "name"]))
        .unwrap_or_else(|| "tool".to_owned());
    let input = state
        .and_then(|object| get_first_value(object, &["input", "args", "arguments", "request"]))
        .or_else(|| get_first_value(part, &["input", "args", "arguments", "request"]));
    let output = state
        .and_then(|object| get_first_value(object, &["output", "result", "data", "response"]))
        .or_else(|| get_first_value(part, &["output", "result", "data", "response"]));
    let error = state
        .and_then(|object| get_first_value(object, &["error", "errors", "failure", "reason"]))
        .or_else(|| get_first_value(part, &["error", "errors", "failure", "reason"]));
    let title = state
        .and_then(|object| get_first_string(object, &["title", "message", "text"]))
        .or_else(|| get_first_string(part, &["title", "message", "text"]));
    let status = state
        .and_then(|object| get_first_string(object, &["status"]))
        .or_else(|| get_first_string(part, &["status"]))
        .unwrap_or_else(|| "running".to_owned());

    let mut events = Vec::new();
    events.extend(opencode_tool_progress_call_events(
        &tool_name,
        title.as_deref(),
        &status,
        input.as_ref(),
    ));

    if let Some(output) = output {
        events.push(AgentTranscriptEvent::tool(
            opencode_tool_progress_output_summary(&tool_name, Some(&output)),
        ));
    } else if let Some(error) = error {
        events.push(AgentTranscriptEvent::tool(
            opencode_tool_progress_error_summary(&tool_name, &error),
        ));
    } else if !matches!(
        status.as_str(),
        "running" | "started" | "streaming" | "in_progress" | "pending"
    ) {
        events.push(AgentTranscriptEvent::tool(format!(
            "Result from `{tool_name}`: {status}"
        )));
    }

    events
}

fn opencode_tool_progress_call_events(
    tool_name: &str,
    title: Option<&str>,
    status: &str,
    input: Option<&Value>,
) -> Vec<AgentTranscriptEvent> {
    if is_cypher_tool_name(tool_name) {
        return opencode_cypher_progress_call_events(tool_name, title, status, input);
    }
    let tool_name = display_native_tool_name(tool_name);

    let input_summary = input.and_then(|input| {
        let summary = summarize_tool_input(&tool_name, input);
        if summary.is_empty() {
            None
        } else {
            Some(summary)
        }
    });
    let title = title
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToOwned::to_owned);
    let reason = input_summary
        .clone()
        .or(title.clone())
        .unwrap_or_else(|| "tool call".to_owned());

    if is_placeholder_tool_reason(&reason) {
        return Vec::new();
    }

    vec![AgentTranscriptEvent::tool(format!(
        "{tool_name} : {reason}"
    ))]
}

fn opencode_cypher_progress_call_events(
    tool_name: &str,
    title: Option<&str>,
    _status: &str,
    input: Option<&Value>,
) -> Vec<AgentTranscriptEvent> {
    let title = title
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToOwned::to_owned);
    let why = input
        .and_then(|input| match input {
            Value::Object(object) => get_first_string(object, &["why", "reason", "goal"]),
            Value::String(text) => Some(text.trim().to_owned()),
            _ => None,
        })
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
        .or_else(|| title.clone())
        .unwrap_or_else(|| "read-only Cypher".to_owned());

    let display_name = if is_project_cypher_tool_name(tool_name) {
        "ifc_project_readonly_cypher"
    } else {
        "ifc_readonly_cypher"
    };
    let mut lines = vec![format!("{display_name} : {why}")];
    if let Some(Value::Object(object)) = input {
        if let Some(cypher) = get_first_string(object, &["cypher", "query"]) {
            let trimmed = cypher.trim();
            if !trimmed.is_empty() {
                lines.push(format!("Cypher:\n{}", trimmed));
            }
        }
    } else if let Some(input) = input {
        let summary = summarize_tool_input("ifc_readonly_cypher", input);
        if !summary.is_empty() {
            lines.push(summary);
        }
    }
    if lines.len() == 1 && why == "read-only Cypher" {
        return Vec::new();
    }
    vec![AgentTranscriptEvent::tool(lines.join("\n"))]
}

fn summarize_tool_input(tool_name: &str, input: &Value) -> String {
    if is_cypher_tool_name(tool_name) {
        return summarize_cypher_tool_input(input);
    }
    match input {
        Value::Object(object) => {
            if let Some(reason) = get_first_string(
                object,
                &["why", "reason", "goal", "task", "prompt", "message", "text"],
            ) {
                let trimmed = reason.trim();
                if !trimmed.is_empty() {
                    return trimmed.to_owned();
                }
            }
            if let Some(schema) = get_first_string(object, &["schema"]) {
                let trimmed = schema.trim();
                if !trimmed.is_empty() {
                    return format!("schema {trimmed}");
                }
            }
            if let Some(entity_names) = get_first_value(object, &["entity_names"]) {
                return format!(
                    "entity_names: {}",
                    format_progress_value(&entity_names, 120)
                );
            }
            if let Some(relation_names) = get_first_value(object, &["relation_names"]) {
                return format!(
                    "relation_names: {}",
                    format_progress_value(&relation_names, 120)
                );
            }
            if let Some(db_node_ids) = get_first_value(object, &["db_node_id", "db_node_ids"]) {
                return format!("db_node_ids: {}", format_progress_value(&db_node_ids, 120));
            }
            if let Some(semantic_ids) = get_first_value(object, &["semantic_ids"]) {
                return format!(
                    "semantic_ids: {}",
                    format_progress_value(&semantic_ids, 120)
                );
            }
            let keys = object.keys().take(6).cloned().collect::<Vec<_>>();
            if keys.is_empty() {
                String::new()
            } else {
                format!("Input keys: {}", keys.join(", "))
            }
        }
        Value::String(text) => shorten_for_progress(text, 240),
        _ => format_progress_value(input, 240),
    }
}

fn is_placeholder_tool_reason(reason: &str) -> bool {
    let lowered = reason.trim().to_ascii_lowercase();
    lowered.is_empty()
        || lowered.starts_with("tool call")
        || lowered.starts_with("input keys:")
        || lowered == "tool event"
}

fn summarize_cypher_tool_input(input: &Value) -> String {
    match input {
        Value::Object(object) => {
            let mut lines = Vec::new();
            if let Some(why) = get_first_string(object, &["why", "reason", "goal"]) {
                let trimmed = why.trim();
                if !trimmed.is_empty() {
                    lines.push(trimmed.to_owned());
                }
            }
            if let Some(cypher) = get_first_string(object, &["cypher", "query"]) {
                let trimmed = cypher.trim();
                if !trimmed.is_empty() {
                    lines.push(format!("Cypher:\n{}", trimmed));
                }
            }
            lines.join("\n")
        }
        Value::String(text) => format!("Cypher:\n{}", shorten_for_progress(text, 1_200)),
        other => format!("Cypher:\n{}", format_progress_value(other, 1_200)),
    }
}

fn opencode_tool_progress_output_summary(tool_name: &str, output: Option<&Value>) -> String {
    if is_cypher_tool_name(tool_name) {
        return summarize_cypher_tool_output(output);
    }
    let tool_name = canonical_native_tool_name(tool_name);
    match output {
        Some(value) => {
            let normalized = normalize_progress_payload(value);
            let summary = summarize_tool_output(tool_name, &normalized);
            if summary.is_empty() {
                format!("Result from `{tool_name}`: completed")
            } else {
                format!("Result from `{tool_name}`: {summary}")
            }
        }
        None => format!("Result from `{tool_name}`: completed"),
    }
}

fn opencode_tool_progress_error_summary(tool_name: &str, error: &Value) -> String {
    if is_cypher_tool_name(tool_name) {
        return format!(
            "Read-only Cypher failed: {}",
            summarize_tool_output(tool_name, error)
        );
    }
    format!(
        "Tool `{tool_name}` failed: {}",
        summarize_tool_output(tool_name, error)
    )
}

fn summarize_cypher_tool_output(output: Option<&Value>) -> String {
    let Some(raw_output) = output else {
        return "Read-only Cypher completed.".to_owned();
    };
    let output = normalize_progress_payload(raw_output);
    match &output {
        Value::Object(object) => {
            if object.get("ok").and_then(Value::as_bool) == Some(false) {
                let message = get_first_string(object, &["error", "message"])
                    .unwrap_or_else(|| "unknown error".to_owned());
                let mut summary = format!(
                    "Read-only Cypher failed: {}",
                    shorten_for_progress(&message, 280)
                );
                if let Some(base) = get_first_string(object, &["base"]) {
                    summary.push_str(&format!(" Base: {}.", shorten_for_progress(&base, 120)));
                } else if let Some(tried) = object.get("tried").and_then(Value::as_array) {
                    let tried = tried
                        .iter()
                        .filter_map(Value::as_str)
                        .take(3)
                        .collect::<Vec<_>>()
                        .join("; ");
                    if !tried.is_empty() {
                        summary
                            .push_str(&format!(" Tried: {}.", shorten_for_progress(&tried, 240)));
                    }
                }
                return summary;
            }
            let rows = object.get("rows").and_then(Value::as_array).map(Vec::len);
            let row_count = rows.unwrap_or(0);
            let row_label = if row_count == 1 { "row" } else { "rows" };
            format!("Read-only Cypher returned {row_count} {row_label}.")
        }
        Value::Array(rows) => {
            let row_count = rows.len();
            let row_label = if row_count == 1 { "row" } else { "rows" };
            format!("Read-only Cypher returned {row_count} {row_label}.")
        }
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                "Read-only Cypher returned no visible output.".to_owned()
            } else {
                format!(
                    "Read-only Cypher returned {}.",
                    shorten_for_progress(trimmed, 280)
                )
            }
        }
        other => format!(
            "Read-only Cypher returned {}.",
            shorten_for_progress(&format_progress_value(other, 280), 280)
        ),
    }
}

fn summarize_tool_output(tool_name: &str, value: &Value) -> String {
    if is_cypher_tool_name(tool_name) {
        return summarize_cypher_tool_output(Some(value));
    }
    match canonical_native_tool_name(tool_name) {
        "schema_context" => summarize_schema_context_output(value),
        "model_details" => summarize_model_details_output(value),
        "entity_reference" => summarize_collection_output("Entity reference lookup", value),
        "relation_reference" => summarize_collection_output("Relation reference lookup", value),
        "query_playbook" => summarize_collection_output("Query playbook lookup", value),
        _ => summarize_generic_tool_output(value),
    }
}

fn summarize_schema_context_output(value: &Value) -> String {
    let Value::Object(object) = value else {
        return summarize_generic_tool_output(value);
    };
    let schema_id = get_first_string(object, &["schemaId", "schema_id"]).unwrap_or_default();
    let cautions = object
        .get("cautions")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let query_habits = object
        .get("queryHabits")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let playbooks = object
        .get("queryPlaybooks")
        .and_then(Value::as_object)
        .map(|map| map.len())
        .unwrap_or(0);

    let mut parts = Vec::new();
    if !schema_id.trim().is_empty() {
        parts.push(format!("schema {schema_id}"));
    } else {
        parts.push("schema context".to_owned());
    }
    let mut details = Vec::new();
    if cautions > 0 {
        details.push(format!("{cautions} caution{}", plural_suffix(cautions)));
    }
    if query_habits > 0 {
        details.push(format!(
            "{query_habits} query habit{}",
            plural_suffix(query_habits)
        ));
    }
    if playbooks > 0 {
        details.push(format!("{playbooks} playbook{}", plural_suffix(playbooks)));
    }
    if !details.is_empty() {
        parts.push(format!("loaded with {}", details.join(", ")));
    } else {
        parts.push("loaded".to_owned());
    }
    parts.join(" ")
}

fn summarize_model_details_output(value: &Value) -> String {
    let Value::Object(object) = value else {
        return summarize_generic_tool_output(value);
    };
    let schema_id = get_first_string(object, &["schemaId", "schema_id"]).unwrap_or_default();
    let mut parts = Vec::new();
    if !schema_id.trim().is_empty() {
        parts.push(format!("model overview for {schema_id}"));
    } else {
        parts.push("model overview".to_owned());
    }
    let extra = object
        .get("summary")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty());
    if let Some(extra) = extra {
        parts.push(format!("loaded: {}", shorten_for_progress(extra, 120)));
    } else {
        parts.push("loaded".to_owned());
    }
    parts.join(" ")
}

fn summarize_collection_output(label: &str, value: &Value) -> String {
    if let Some(count) = count_collection_items(value) {
        return format!("{label} returned {count} item{}.", plural_suffix(count));
    }
    summarize_generic_tool_output(value)
}

fn count_collection_items(value: &Value) -> Option<usize> {
    match value {
        Value::Array(items) => Some(items.len()),
        Value::Object(object) => object
            .get("items")
            .and_then(Value::as_array)
            .map(Vec::len)
            .or_else(|| object.get("rows").and_then(Value::as_array).map(Vec::len))
            .or_else(|| {
                object
                    .get("results")
                    .and_then(Value::as_array)
                    .map(Vec::len)
            }),
        _ => None,
    }
}

fn summarize_generic_tool_output(value: &Value) -> String {
    match value {
        Value::Object(object) => {
            if let Some(text) = get_first_string(object, &["message", "title", "text", "output"]) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    return shorten_for_progress(trimmed, 280);
                }
            }
            if let Some(count) = count_collection_items(value) {
                let item_label = if count == 1 { "item" } else { "items" };
                return format!("returned {count} {item_label}");
            }
            "returned data".to_owned()
        }
        Value::Array(items) => {
            let count = items.len();
            let item_label = if count == 1 { "item" } else { "items" };
            format!("returned {count} {item_label}")
        }
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                "returned text".to_owned()
            } else if trimmed.starts_with('{') || trimmed.starts_with('[') {
                if let Ok(parsed) = serde_json::from_str::<Value>(trimmed) {
                    summarize_generic_tool_output(&parsed)
                } else {
                    shorten_for_progress(trimmed, 280)
                }
            } else {
                shorten_for_progress(trimmed, 280)
            }
        }
        Value::Null => "returned null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
    }
}

fn plural_suffix(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

fn is_cypher_tool_name(tool_name: &str) -> bool {
    matches!(
        canonical_native_tool_name(tool_name),
        "readonly_cypher"
            | "run_readonly_cypher"
            | "project_readonly_cypher"
            | "run_project_readonly_cypher"
    )
}

fn is_project_cypher_tool_name(tool_name: &str) -> bool {
    matches!(
        canonical_native_tool_name(tool_name),
        "project_readonly_cypher" | "run_project_readonly_cypher"
    )
}

fn normalize_progress_payload(value: &Value) -> Value {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.starts_with('{') || trimmed.starts_with('[') {
                serde_json::from_str::<Value>(trimmed).unwrap_or_else(|_| value.clone())
            } else {
                value.clone()
            }
        }
        _ => value.clone(),
    }
}

fn format_progress_value(value: &Value, max_chars: usize) -> String {
    let rendered = match value {
        Value::String(text) => text.trim().to_owned(),
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::Array(_) | Value::Object(_) => serde_json::to_string_pretty(value)
            .unwrap_or_else(|_| value.to_string())
            .trim()
            .to_owned(),
    };
    shorten_for_progress(&rendered, max_chars)
}

fn get_first_string(object: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    get_first_value(object, keys).map(|value| match value {
        Value::String(text) => text,
        other => other.to_string(),
    })
}

fn parse_opencode_turn_response_text(text: &str) -> Option<OpencodeTurnResponse> {
    let normalized_json = normalize_assistant_json(text);
    let value = serde_json::from_str::<Value>(&normalized_json).ok()?;
    serde_json::from_value(normalize_response_shape(value)).ok()
}

fn opencode_progress_event_for_tool_call(tool_call: &OpencodeToolCall) -> AgentTranscriptEvent {
    match tool_call {
        OpencodeToolCall::RunReadonlyCypher { cypher, why } => {
            AgentTranscriptEvent::tool(match why.as_deref().map(str::trim) {
                Some(why) if !why.is_empty() => {
                    format!("ifc_readonly_cypher : {why}\nCypher:\n{}", cypher.trim())
                }
                _ => format!(
                    "ifc_readonly_cypher : read-only Cypher\nCypher:\n{}",
                    cypher.trim()
                ),
            })
        }
        OpencodeToolCall::RunProjectReadonlyCypher { cypher, why, .. } => {
            AgentTranscriptEvent::tool(match why.as_deref().map(str::trim) {
                Some(why) if !why.is_empty() => {
                    format!(
                        "ifc_project_readonly_cypher : {why}\nCypher:\n{}",
                        cypher.trim()
                    )
                }
                _ => format!(
                    "ifc_project_readonly_cypher : project read-only Cypher\nCypher:\n{}",
                    cypher.trim()
                ),
            })
        }
        OpencodeToolCall::GetSchemaContext => AgentTranscriptEvent::tool(
            "ifc_schema_context : loading schema context for the current IFC model.".to_owned(),
        ),
        OpencodeToolCall::GetModelDetails => AgentTranscriptEvent::tool(
            "ifc_model_details : loading model overview for the current IFC model.".to_owned(),
        ),
        OpencodeToolCall::GetEntityReference { entity_names } => {
            AgentTranscriptEvent::tool(format!(
                "ifc_entity_reference : loading schema reference for {} entit{}.",
                entity_names.len(),
                if entity_names.len() == 1 { "y" } else { "ies" }
            ))
        }
        OpencodeToolCall::GetQueryPlaybook { goal, .. } => AgentTranscriptEvent::tool(format!(
            "ifc_query_playbook : loading query playbook for `{}`.",
            goal.trim()
        )),
        OpencodeToolCall::GetRelationReference { relation_names } => {
            AgentTranscriptEvent::tool(format!(
                "ifc_relation_reference : loading relation reference for {} item{}.",
                relation_names.len(),
                if relation_names.len() == 1 { "" } else { "s" }
            ))
        }
        OpencodeToolCall::RequestTools { tools } => AgentTranscriptEvent::tool(format!(
            "request_tools : requesting {} tool{}: {}.",
            tools.len(),
            if tools.len() == 1 { "" } else { "s" },
            if tools.is_empty() {
                "none".to_owned()
            } else {
                tools.join(", ")
            }
        )),
        OpencodeToolCall::DescribeNodes { db_node_ids } => AgentTranscriptEvent::tool(format!(
            "describe_nodes : describing {} graph node{}.",
            db_node_ids.len(),
            if db_node_ids.len() == 1 { "" } else { "s" }
        )),
        OpencodeToolCall::GetNodeProperties { db_node_id } => AgentTranscriptEvent::tool(format!(
            "get_node_properties : loading properties for graph node {}.",
            db_node_id
        )),
        OpencodeToolCall::GetNeighbors { db_node_ids, .. } => AgentTranscriptEvent::tool(format!(
            "get_neighbors : loading neighbor graph from {} seed node{}.",
            db_node_ids.len(),
            if db_node_ids.len() == 1 { "" } else { "s" }
        )),
        OpencodeToolCall::EmitUiActions { actions } => AgentTranscriptEvent::assistant(format!(
            "preparing {} viewer action{}.",
            actions.len(),
            if actions.len() == 1 { "" } else { "s" }
        )),
    }
}

fn opencode_tool_call_signature(tool_call: &OpencodeToolCall) -> String {
    match tool_call {
        OpencodeToolCall::RunReadonlyCypher { cypher, .. } => {
            format!("run_readonly_cypher|{}", cypher.trim())
        }
        OpencodeToolCall::RunProjectReadonlyCypher {
            cypher,
            resource_filter,
            ..
        } => {
            let mut resource_filter = resource_filter.clone();
            resource_filter.sort_unstable();
            resource_filter.dedup();
            format!(
                "run_project_readonly_cypher|{}|{}",
                cypher.trim(),
                resource_filter.join("|")
            )
        }
        OpencodeToolCall::GetSchemaContext => "get_schema_context".to_owned(),
        OpencodeToolCall::GetModelDetails => "get_model_details".to_owned(),
        OpencodeToolCall::GetEntityReference { entity_names } => {
            let mut entity_names = entity_names
                .iter()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>();
            entity_names.sort_unstable();
            entity_names.dedup();
            format!("get_entity_reference|{}", entity_names.join("|"))
        }
        OpencodeToolCall::GetQueryPlaybook { goal, entity_names } => {
            let mut entity_names = entity_names
                .iter()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>();
            entity_names.sort_unstable();
            entity_names.dedup();
            format!(
                "get_query_playbook|{}|{}",
                goal.trim(),
                entity_names.join("|")
            )
        }
        OpencodeToolCall::GetRelationReference { relation_names } => {
            let mut relation_names = relation_names
                .iter()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>();
            relation_names.sort_unstable();
            relation_names.dedup();
            format!("get_relation_reference|{}", relation_names.join("|"))
        }
        OpencodeToolCall::RequestTools { tools } => {
            let mut tools = tools
                .iter()
                .map(|value| normalize_tool_function_kind(value.trim()))
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>();
            tools.sort_unstable();
            tools.dedup();
            format!("request_tools|{}", tools.join("|"))
        }
        OpencodeToolCall::DescribeNodes { db_node_ids } => {
            let mut db_node_ids = db_node_ids.clone();
            db_node_ids.sort_unstable();
            db_node_ids.dedup();
            format!(
                "describe_nodes|{}",
                db_node_ids
                    .into_iter()
                    .map(|value| value.to_string())
                    .collect::<Vec<_>>()
                    .join("|")
            )
        }
        OpencodeToolCall::GetNodeProperties { db_node_id } => {
            format!("get_node_properties|{db_node_id}")
        }
        OpencodeToolCall::GetNeighbors {
            db_node_ids,
            hops,
            mode,
        } => {
            let mut db_node_ids = db_node_ids.clone();
            db_node_ids.sort_unstable();
            db_node_ids.dedup();
            format!(
                "get_neighbors|{}|{}|{}",
                db_node_ids
                    .into_iter()
                    .map(|value| value.to_string())
                    .collect::<Vec<_>>()
                    .join("|"),
                hops.unwrap_or(1).max(1),
                normalize_graph_mode(mode.unwrap_or(AgentGraphMode::Semantic))
            )
        }
        OpencodeToolCall::EmitUiActions { actions } => serde_json::to_string(&serde_json::json!({
            "kind": "emit_ui_actions",
            "actions": actions,
        }))
        .unwrap_or_else(|_| format!("{tool_call:?}")),
    }
}

fn normalize_graph_mode(mode: AgentGraphMode) -> &'static str {
    match mode {
        AgentGraphMode::Semantic => "semantic",
        AgentGraphMode::Raw => "raw",
    }
}

fn join_capture(
    handle: thread::JoinHandle<OutputCapture>,
    executable: &Path,
    stream_name: &'static str,
) -> Result<OutputCapture, OpencodeExecutorError> {
    handle
        .join()
        .map_err(|_| OpencodeExecutorError::JoinFailed {
            executable: executable.to_path_buf(),
            stream_name,
        })
}

fn format_exit_status(status: ExitStatus) -> String {
    match status.code() {
        Some(code) => format!("exit code {code}"),
        None => "termination by signal".to_owned(),
    }
}

fn excerpt(bytes: &[u8], max_bytes: usize) -> String {
    let slice = if bytes.len() > max_bytes {
        &bytes[..max_bytes]
    } else {
        bytes
    };
    String::from_utf8_lossy(slice).trim().to_owned()
}

fn format_excerpt_suffix(excerpt: &str) -> String {
    if excerpt.is_empty() {
        String::new()
    } else {
        format!("; excerpt: {}", excerpt.replace('\n', " "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_executor::{
        AgentEntityReference, AgentExecutor, AgentGraphMode, AgentNeighborGraph,
        AgentNodePropertiesResult, AgentNodeSummary, AgentProgressSink, AgentQueryPlaybook,
        AgentReadonlyCypherResult, AgentReadonlyCypherRuntime, AgentRelationReference,
        AgentSchemaContext, AgentTranscriptEventKind, NullAgentProgressSink,
    };
    use std::collections::{BTreeMap, HashMap};

    struct RecordingRuntime {
        calls: Vec<String>,
        result: AgentReadonlyCypherResult,
    }

    #[derive(Default)]
    struct RecordingProgressSink {
        events: Vec<AgentTranscriptEvent>,
    }

    impl AgentProgressSink for RecordingProgressSink {
        fn emit(&mut self, event: AgentTranscriptEvent) {
            self.events.push(event);
        }
    }

    impl AgentReadonlyCypherRuntime for RecordingRuntime {
        fn run_readonly_cypher(
            &mut self,
            query: &str,
            _why: Option<&str>,
        ) -> Result<AgentReadonlyCypherResult, String> {
            self.calls.push(query.to_owned());
            Ok(self.result.clone())
        }

        fn describe_nodes(
            &mut self,
            _db_node_ids: &[i64],
        ) -> Result<Vec<AgentNodeSummary>, String> {
            Ok(Vec::new())
        }

        fn get_schema_context(&mut self) -> Result<AgentSchemaContext, String> {
            Ok(AgentSchemaContext {
                schema_id: "IFC4X3_ADD2".to_owned(),
                schema_slug: Some("ifc4x3_add2".to_owned()),
                summary: "test schema".to_owned(),
                cautions: Vec::new(),
                query_habits: Vec::new(),
                query_playbooks: BTreeMap::new(),
            })
        }

        fn get_entity_reference(
            &mut self,
            _entity_names: &[String],
        ) -> Result<Vec<AgentEntityReference>, String> {
            Ok(Vec::new())
        }

        fn get_query_playbook(
            &mut self,
            _goal: &str,
            _entity_names: &[String],
        ) -> Result<Vec<AgentQueryPlaybook>, String> {
            Ok(Vec::new())
        }

        fn get_relation_reference(
            &mut self,
            _relation_names: &[String],
        ) -> Result<Vec<AgentRelationReference>, String> {
            Ok(Vec::new())
        }

        fn get_node_properties(
            &mut self,
            _db_node_id: i64,
        ) -> Result<AgentNodePropertiesResult, String> {
            Err("not used in this test".to_owned())
        }

        fn get_neighbors(
            &mut self,
            _db_node_ids: &[i64],
            _hops: usize,
            _mode: AgentGraphMode,
        ) -> Result<AgentNeighborGraph, String> {
            Err("not used in this test".to_owned())
        }
    }

    #[test]
    fn model_discovery_parser_keeps_plain_model_ids() {
        let models = parse_opencode_models_output(
            b"openai/gpt-5.4\nopenai/gpt-5.4-mini\n\nnot-a-model\nopenai/gpt-5.4\n",
        );

        assert_eq!(
            models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            vec!["openai/gpt-5.4", "openai/gpt-5.4-mini"]
        );
    }

    #[test]
    fn model_discovery_parser_keeps_verbose_variants() {
        let models = parse_opencode_models_output(
            br#"openai/gpt-5.4
{
  "id": "gpt-5.4",
  "providerID": "openai",
  "variants": {
    "none": {
      "reasoningEffort": "none"
    },
    "medium": {
      "reasoningEffort": "medium"
    },
    "high": {
      "reasoningEffort": "high"
    }
  }
}
"#,
        );

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "openai/gpt-5.4");
        assert_eq!(models[0].variants, vec!["medium", "high"]);
    }

    #[test]
    fn config_ignores_none_variant_override() {
        let values = HashMap::from([("CC_W_OPENCODE_VARIANT".to_owned(), OsString::from("none"))]);

        let config = OpencodeExecutorConfig::from_env_with(|key| values.get(key).cloned())
            .expect("config should parse");

        assert_eq!(config.variant, None);
    }

    #[test]
    fn default_run_args_include_selected_model_and_variant() {
        let request = OpencodeTurnRequest {
            resource: "ifc/building-architecture".to_owned(),
            schema_id: "IFC4X3_ADD2".to_owned(),
            schema_slug: Some("ifc4x3_add2".to_owned()),
            user_input: "hello".to_owned(),
            session_history: Vec::new(),
            transcript: Vec::new(),
            tool_results: Vec::new(),
        };
        let args = default_opencode_run_args(
            &request,
            Some("openai/gpt-5.4-mini"),
            Some("high"),
            Some("ifc-explorer"),
        );

        assert_eq!(args[0], "run");
        assert!(
            args.windows(2)
                .any(|window| window[0] == "--agent" && window[1] == "ifc-explorer")
        );
        assert!(
            args.windows(2)
                .any(|window| window[0] == "--model" && window[1] == "openai/gpt-5.4-mini")
        );
        assert!(
            args.windows(2)
                .any(|window| window[0] == "--variant" && window[1] == "high")
        );
    }

    #[test]
    fn config_parses_env_overrides() {
        let values = HashMap::from([
            (
                "CC_W_OPENCODE_EXECUTABLE".to_owned(),
                OsString::from("/tmp/fake-opencode"),
            ),
            (
                "CC_W_OPENCODE_ARGS".to_owned(),
                OsString::from("--stdio --model local"),
            ),
            (
                "CC_W_OPENCODE_WORKDIR".to_owned(),
                OsString::from("/tmp/opencode-workdir"),
            ),
            (
                "CC_W_OPENCODE_AGENT".to_owned(),
                OsString::from("ifc-explorer"),
            ),
            ("CC_W_OPENCODE_VARIANT".to_owned(), OsString::from("medium")),
            (
                "CC_W_OPENCODE_TIMEOUT_MS".to_owned(),
                OsString::from("2500"),
            ),
            (
                "CC_W_OPENCODE_MAX_STDOUT_BYTES".to_owned(),
                OsString::from("8192"),
            ),
            (
                "CC_W_OPENCODE_MAX_STDERR_BYTES".to_owned(),
                OsString::from("2048"),
            ),
            (
                "CC_W_OPENCODE_MAX_STEPS_PER_TURN".to_owned(),
                OsString::from("3"),
            ),
            (
                "CC_W_OPENCODE_TRANSIENT_PROVIDER_RETRIES".to_owned(),
                OsString::from("2"),
            ),
            (
                "CC_W_OPENCODE_RETRY_BACKOFF_MS".to_owned(),
                OsString::from("125"),
            ),
        ]);

        let config = OpencodeExecutorConfig::from_env_with(|key| values.get(key).cloned())
            .expect("config should parse");

        assert_eq!(config.executable, PathBuf::from("/tmp/fake-opencode"));
        assert_eq!(
            config.args,
            vec![
                "--stdio".to_owned(),
                "--model".to_owned(),
                "local".to_owned()
            ]
        );
        assert_eq!(
            config.working_directory,
            Some(PathBuf::from("/tmp/opencode-workdir"))
        );
        assert_eq!(config.agent.as_deref(), Some("ifc-explorer"));
        assert_eq!(config.variant.as_deref(), Some("medium"));
        assert_eq!(config.timeout, Duration::from_millis(2500));
        assert_eq!(config.max_stdout_bytes, 8192);
        assert_eq!(config.max_stderr_bytes, 2048);
        assert_eq!(config.max_steps_per_turn, 3);
        assert_eq!(config.transient_provider_retries, 2);
        assert_eq!(config.retry_backoff, Duration::from_millis(125));
    }

    #[test]
    fn missing_executable_reports_cleanly() {
        let executor = OpencodeExecutor::new(OpencodeExecutorConfig {
            executable: PathBuf::from("/tmp/definitely-missing-opencode-executable"),
            ..OpencodeExecutorConfig::default()
        });

        let error = executor
            .run_step(&sample_step_request())
            .expect_err("missing executable should fail");

        assert!(matches!(
            error,
            OpencodeExecutorError::MissingExecutable { .. }
        ));
        assert!(
            error
                .to_string()
                .contains("configure CC_W_OPENCODE_EXECUTABLE")
        );
    }

    #[test]
    fn child_process_response_is_parsed() {
        let script = "cat >/dev/null; printf '%s' '{\"transcript\":[{\"kind\":\"assistant\",\"text\":\"planned\"}],\"toolCalls\":[{\"kind\":\"run_readonly_cypher\",\"cypher\":\"MATCH (n) RETURN id(n) AS node_id LIMIT 1\"}],\"finalText\":\"ok\"}'";
        let executor = OpencodeExecutor::new(OpencodeExecutorConfig {
            executable: PathBuf::from("/bin/sh"),
            args: vec!["-c".to_owned(), script.to_owned()],
            timeout: Duration::from_millis(1000),
            ..OpencodeExecutorConfig::default()
        });

        let response = executor
            .run_step(&sample_step_request())
            .expect("shell wrapper should succeed");

        assert_eq!(
            response.transcript,
            vec![OpencodeTranscriptEvent {
                kind: OpencodeTranscriptEventKind::Assistant,
                text: "planned".to_owned(),
            }]
        );
        assert_eq!(
            response.tool_calls,
            vec![OpencodeToolCall::RunReadonlyCypher {
                cypher: "MATCH (n) RETURN id(n) AS node_id LIMIT 1".to_owned(),
                why: None,
            }]
        );
        assert_eq!(response.final_text.as_deref(), Some("ok"));
    }

    #[test]
    fn child_process_progress_events_are_forwarded_to_progress_sink() {
        let script = "printf '%s\\n' '{\"type\":\"step_start\",\"timestamp\":1}' ; printf '%s\\n' '{\"type\":\"tool_use\",\"part\":{\"tool\":\"ifc_readonly_cypher\",\"state\":{\"status\":\"completed\",\"title\":\"Reading the current IFC model.\",\"input\":{\"why\":\"get a quick high-level spatial summary of the current IFC model\",\"cypher\":\"MATCH (p:IfcProject) RETURN p.Name AS project_name LIMIT 1\"},\"output\":{\"rows\":[[\"ifc silly sample scene - project\"]],\"columns\":[\"project_name\"]}}}}' ; printf '%s\\n' '{\"type\":\"text\",\"part\":{\"text\":\"{\\\"transcript\\\":[{\\\"kind\\\":\\\"assistant\\\",\\\"text\\\":\\\"Used a model-overview playbook to keep the first pass small.\\\"}],\\\"toolCalls\\\":[{\\\"kind\\\":\\\"get_query_playbook\\\",\\\"goal\\\":\\\"broad overview\\\",\\\"entity_names\\\":[\\\"IfcProject\\\"]}],\\\"finalText\\\":\\\"\\\"}\"}}' ; printf '%s\\n' '{\"type\":\"step_finish\",\"part\":{\"type\":\"step-finish\"}}'";
        let executor = OpencodeExecutor::new(OpencodeExecutorConfig {
            executable: PathBuf::from("/bin/sh"),
            args: vec!["-c".to_owned(), script.to_owned()],
            timeout: Duration::from_millis(1000),
            ..OpencodeExecutorConfig::default()
        });
        let mut progress = RecordingProgressSink::default();

        let response = executor
            .run_step_with_progress(&sample_step_request(), &mut progress)
            .expect("shell wrapper should succeed");

        assert_eq!(response.final_text.as_deref(), Some(""));
        assert!(
            progress.events.iter().any(|event| {
                matches!(event.kind, AgentTranscriptEventKind::System)
                    && event.text.contains("opencode progress")
                    && event.text.contains("step started")
            }),
            "expected a live progress event to be forwarded to the progress sink"
        );
        assert!(
            progress.events.iter().any(|event| {
                matches!(event.kind, AgentTranscriptEventKind::Assistant)
                    && event
                        .text
                        .contains("Used a model-overview playbook to keep the first pass small.")
            }),
            "expected nested assistant transcript text to be forwarded to the progress sink"
        );
        assert!(
            progress.events.iter().any(|event| {
                matches!(event.kind, AgentTranscriptEventKind::Tool)
                    && event.text.contains("ifc_readonly_cypher :")
                    && event.text.contains("Cypher:")
                    && event
                        .text
                        .contains("MATCH (p:IfcProject) RETURN p.Name AS project_name LIMIT 1")
            }),
            "expected the tool call to be forwarded to the progress sink"
        );
        assert!(
            progress.events.iter().any(|event| {
                matches!(event.kind, AgentTranscriptEventKind::Tool)
                    && event.text.contains("Read-only Cypher returned 1 row.")
            }),
            "expected the tool result to be forwarded to the progress sink"
        );
    }

    #[test]
    fn child_process_progress_events_redact_stringified_json_tool_payloads() {
        let schema_context = "{\"schemaId\":\"IFC4X3_ADD2\",\"schemaSlug\":\"ifc4x3_add2\",\"summary\":\"mixed building and infrastructure\",\"cautions\":[\"caution-one\",\"caution-two\"],\"queryHabits\":[\"habit-one\"],\"queryPlaybooks\":{\"model overview\":[\"a\",\"b\"]}}";
        let cypher_result = "{\"resource\":\"ifc/building-architecture\",\"columns\":[\"project_name\"],\"rows\":[[\"ifc silly sample scene - project\"]],\"semanticElementIds\":[]}";
        let schema_line = serde_json::json!({
            "type": "tool_use",
            "part": {
                "tool": "ifc_schema_context",
                "state": {
                    "status": "completed",
                    "title": "Loading schema context for IFC4X3_ADD2.",
                    "input": { "schema": "IFC4X3_ADD2" },
                    "output": schema_context,
                },
            },
        })
        .to_string();
        let cypher_line = serde_json::json!({
            "type": "tool_use",
            "part": {
                "tool": "ifc_readonly_cypher",
                "state": {
                    "status": "completed",
                    "title": "Reading the current IFC model.",
                    "input": {
                        "why": "get a quick high-level spatial summary of the current IFC model",
                        "cypher": "MATCH (p:IfcProject) RETURN p.Name AS project_name LIMIT 1",
                    },
                    "output": cypher_result,
                },
            },
        })
        .to_string();
        let assistant_line = serde_json::json!({
            "type": "text",
            "part": {
                "text": "{\"transcript\":[{\"kind\":\"assistant\",\"text\":\"done\"}],\"toolCalls\":[],\"finalText\":\"ok\"}"
            }
        })
        .to_string();
        let step_start_line = serde_json::json!({"type":"step_start","timestamp":1}).to_string();
        let step_finish_line =
            serde_json::json!({"type":"step_finish","part":{"type":"step-finish"}}).to_string();
        let script = format!(
            "printf '%s\\n' '{}' ; printf '%s\\n' '{}' ; printf '%s\\n' '{}' ; printf '%s\\n' '{}' ; printf '%s\\n' '{}'",
            step_start_line, schema_line, cypher_line, assistant_line, step_finish_line
        );
        let executor = OpencodeExecutor::new(OpencodeExecutorConfig {
            executable: PathBuf::from("/bin/sh"),
            args: vec!["-c".to_owned(), script],
            timeout: Duration::from_millis(1000),
            ..OpencodeExecutorConfig::default()
        });
        let mut progress = RecordingProgressSink::default();

        let response = executor
            .run_step_with_progress(&sample_step_request(), &mut progress)
            .expect("shell wrapper should succeed");

        assert_eq!(response.final_text.as_deref(), Some("ok"));
        assert!(
            progress.events.iter().any(|event| {
                matches!(event.kind, AgentTranscriptEventKind::Tool)
                    && event.text.contains("schema IFC4X3_ADD2 loaded")
                    && !event.text.contains("{\"schemaId\"")
                    && !event.text.contains("\"cautions\"")
            }),
            "expected schema context to be summarized instead of emitted as a response document"
        );
        assert!(
            progress.events.iter().any(|event| {
                matches!(event.kind, AgentTranscriptEventKind::Tool)
                    && event.text.contains("Read-only Cypher returned 1 row.")
                    && !event.text.contains("{\"resource\"")
                    && !event.text.contains("\"rows\"")
            }),
            "expected cypher results to be summarized instead of emitted as a response document"
        );
    }

    #[test]
    fn cypher_progress_summary_reports_structured_transport_failures() {
        let output = serde_json::json!({
            "ok": false,
            "path": "/api/cypher",
            "error": "viewer API connection failed",
            "tried": [
                "http://127.0.0.1:8001/: fetch failed",
                "http://localhost:8001/: fetch failed"
            ]
        });

        let summary = summarize_cypher_tool_output(Some(&output));

        assert!(summary.contains("Read-only Cypher failed: viewer API connection failed"));
        assert!(summary.contains("Tried: http://127.0.0.1:8001/: fetch failed"));
    }

    #[test]
    fn run_step_retries_transient_provider_error_once_then_succeeds() {
        let unique = format!(
            "ccw-opencode-retry-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time should be after epoch")
                .as_nanos()
        );
        let state_path = std::env::temp_dir().join(unique);
        let state = state_path.to_string_lossy().replace('"', "\\\"");
        let script = format!(
            "cat >/dev/null; if [ ! -f \"{state}\" ]; then touch \"{state}\"; printf '%s' '{{\"type\":\"error\",\"error\":{{\"name\":\"UnknownError\",\"data\":{{\"message\":\"{{\\\"type\\\":\\\"error\\\",\\\"sequence_number\\\":2,\\\"error\\\":{{\\\"type\\\":\\\"server_error\\\",\\\"code\\\":\\\"server_error\\\",\\\"message\\\":\\\"temporary upstream wobble\\\",\\\"param\\\":null}}}}\"}}}}}}'; else printf '%s' '{{\"finalText\":\"ok after retry\"}}'; fi"
        );
        let executor = OpencodeExecutor::new(OpencodeExecutorConfig {
            executable: PathBuf::from("/bin/sh"),
            args: vec!["-c".to_owned(), script],
            timeout: Duration::from_millis(1000),
            transient_provider_retries: 1,
            retry_backoff: Duration::from_millis(1),
            ..OpencodeExecutorConfig::default()
        });

        let response = executor
            .run_step(&sample_step_request())
            .expect("executor should retry transient provider errors once");

        assert_eq!(response.final_text.as_deref(), Some("ok after retry"));
        let _ = std::fs::remove_file(state_path);
    }

    #[test]
    fn executor_can_loop_query_then_emit_actions() {
        let script = "python3 -c 'import json,sys; req=json.load(sys.stdin); json.dump({\"transcript\":[{\"kind\":\"assistant\",\"text\":\"planning\"}],\"toolCalls\":[{\"kind\":\"run_readonly_cypher\",\"cypher\":\"MATCH (n) RETURN id(n) AS node_id LIMIT 1\"}]} if not req.get(\"toolResults\") else {\"toolCalls\":[{\"kind\":\"emit_ui_actions\",\"actions\":[{\"kind\":\"viewer.frame_visible\"}]}],\"finalText\":\"done\"}, sys.stdout)'";
        let mut executor = OpencodeExecutor::new(OpencodeExecutorConfig {
            executable: PathBuf::from("/bin/sh"),
            args: vec!["-c".to_owned(), script.to_owned()],
            timeout: Duration::from_millis(2000),
            max_steps_per_turn: 3,
            ..OpencodeExecutorConfig::default()
        });
        let mut runtime = RecordingRuntime {
            calls: Vec::new(),
            result: AgentReadonlyCypherResult {
                columns: vec!["node_id".to_owned()],
                rows: vec![vec!["1".to_owned()]],
                db_node_ids: vec![1],
                semantic_element_ids: Vec::new(),
            },
        };

        let response = executor
            .execute_turn(
                &AgentBackendTurnRequest {
                    resource: "ifc/building-architecture".to_owned(),
                    schema_id: "IFC4X3_ADD2".to_owned(),
                    schema_slug: Some("ifc4x3_add2".to_owned()),
                    input: "show project".to_owned(),
                    session_history: Vec::new(),
                },
                &mut runtime,
                &mut NullAgentProgressSink,
            )
            .expect("executor should complete");

        assert_eq!(
            runtime.calls,
            vec!["MATCH (n) RETURN id(n) AS node_id LIMIT 1"]
        );
        assert_eq!(response.queries_executed, 1);
        assert_eq!(
            response.action_candidates,
            vec![AgentActionCandidate::viewer_frame_visible()]
        );
    }

    #[test]
    fn executor_reuses_identical_tool_calls_within_one_turn() {
        let first_step = serde_json::json!({
            "transcript": [
                { "kind": "assistant", "text": "checking" }
            ],
            "toolCalls": [
                {
                    "kind": "run_readonly_cypher",
                    "cypher": "MATCH (n) RETURN id(n) AS node_id LIMIT 1",
                    "why": "find one seed node"
                }
            ]
        })
        .to_string();
        let second_step = serde_json::json!({
            "transcript": [
                { "kind": "assistant", "text": "checking again" }
            ],
            "toolCalls": [
                {
                    "kind": "run_readonly_cypher",
                    "cypher": "MATCH (n) RETURN id(n) AS node_id LIMIT 1",
                    "why": "find one seed node"
                }
            ],
            "finalText": "done"
        })
        .to_string();
        let script = format!(
            "printf '%s\\n' '{}' ; printf '%s\\n' '{}' ; printf '%s\\n' '{}' ; printf '%s\\n' '{}' ; printf '%s\\n' '{}' ; printf '%s\\n' '{}'",
            serde_json::json!({"type":"step_start","timestamp":1}),
            serde_json::json!({"type":"text","part":{"text":first_step}}),
            serde_json::json!({"type":"step_finish","part":{"type":"step-finish"}}),
            serde_json::json!({"type":"step_start","timestamp":2}),
            serde_json::json!({"type":"text","part":{"text":second_step}}),
            serde_json::json!({"type":"step_finish","part":{"type":"step-finish"}})
        );
        let mut executor = OpencodeExecutor::new(OpencodeExecutorConfig {
            executable: PathBuf::from("/bin/sh"),
            args: vec!["-c".to_owned(), script],
            timeout: Duration::from_millis(1000),
            max_steps_per_turn: 3,
            ..OpencodeExecutorConfig::default()
        });
        let mut runtime = RecordingRuntime {
            calls: Vec::new(),
            result: AgentReadonlyCypherResult {
                columns: vec!["node_id".to_owned()],
                rows: vec![vec!["1".to_owned()]],
                db_node_ids: vec![1],
                semantic_element_ids: Vec::new(),
            },
        };
        let mut progress = RecordingProgressSink::default();

        let response = executor
            .execute_turn(
                &AgentBackendTurnRequest {
                    resource: "ifc/building-architecture".to_owned(),
                    schema_id: "IFC4X3_ADD2".to_owned(),
                    schema_slug: Some("ifc4x3_add2".to_owned()),
                    input: "what can you tell me about the model?".to_owned(),
                    session_history: Vec::new(),
                },
                &mut runtime,
                &mut progress,
            )
            .expect("executor should reuse identical tool calls");

        assert_eq!(
            runtime.calls,
            vec!["MATCH (n) RETURN id(n) AS node_id LIMIT 1"]
        );
        assert_eq!(response.queries_executed, 1);
        assert!(
            progress.events.iter().any(|event| {
                matches!(event.kind, AgentTranscriptEventKind::Tool)
                    && event.text.contains("(reused)")
                    && event.text.contains("ifc_readonly_cypher")
            }),
            "expected the repeated Cypher call to be marked as reused"
        );
        assert!(
            progress.events.iter().any(|event| {
                matches!(event.kind, AgentTranscriptEventKind::System)
                    && event
                        .text
                        .contains("Reused a prior result from earlier in this turn.")
            }),
            "expected the repeated Cypher call to reuse the previous result"
        );
    }

    #[test]
    fn normalize_response_shape_accepts_new_tool_and_action_aliases() {
        let normalized = normalize_response_shape(serde_json::json!({
            "toolCalls": [
                { "kind": "get_entity_reference", "entityNames": ["IfcRoof"] },
                { "kind": "get_query_playbook", "queryGoal": "hide the roof", "entityNames": ["IfcRoof", "IfcSlab"] },
                { "kind": "get_relation_reference", "relationNames": ["IfcRelAggregates"] },
                { "kind": "run_readonly_cypher", "cypher": "MATCH (n) RETURN id(n) AS node_id LIMIT 1", "reason": "find one seed node" },
                { "kind": "describe_nodes", "nodeIds": [215, 216] },
                {
                    "kind": "emit_ui_actions",
                    "actions": [
                        { "kind": "properties.show_node", "dbNodeId": 215 }
                    ]
                }
            ]
        }));

        let response: OpencodeTurnResponse =
            serde_json::from_value(normalized).expect("normalized response should parse");

        assert_eq!(
            response.tool_calls,
            vec![
                OpencodeToolCall::GetEntityReference {
                    entity_names: vec!["IfcRoof".to_owned()]
                },
                OpencodeToolCall::GetQueryPlaybook {
                    goal: "hide the roof".to_owned(),
                    entity_names: vec!["IfcRoof".to_owned(), "IfcSlab".to_owned()]
                },
                OpencodeToolCall::GetRelationReference {
                    relation_names: vec!["IfcRelAggregates".to_owned()]
                },
                OpencodeToolCall::RunReadonlyCypher {
                    cypher: "MATCH (n) RETURN id(n) AS node_id LIMIT 1".to_owned(),
                    why: Some("find one seed node".to_owned()),
                },
                OpencodeToolCall::DescribeNodes {
                    db_node_ids: vec![215, 216]
                },
                OpencodeToolCall::EmitUiActions {
                    actions: vec![PlannedUiAction::PropertiesShowNode {
                        db_node_id: 215,
                        resource: None,
                    }]
                },
            ]
        );
    }

    #[test]
    fn normalize_response_shape_accepts_openai_style_function_tool_calls() {
        let normalized = normalize_response_shape(serde_json::json!({
            "tool_calls": [
                {
                    "function": "run_readonly_cypher",
                    "args": {
                        "cypher": "MATCH (n) RETURN id(n) AS node_id LIMIT 1",
                        "why": "find one seed node"
                    }
                }
            ]
        }));

        let response: OpencodeTurnResponse =
            serde_json::from_value(normalized).expect("normalized response should parse");

        assert_eq!(
            response.tool_calls,
            vec![OpencodeToolCall::RunReadonlyCypher {
                cypher: "MATCH (n) RETURN id(n) AS node_id LIMIT 1".to_owned(),
                why: Some("find one seed node".to_owned()),
            }]
        );
    }

    #[test]
    fn normalize_response_shape_accepts_schema_and_playbook_aliases() {
        let normalized = normalize_response_shape(serde_json::json!({
            "tool_calls": [
                { "function": "get_schema", "args": {} },
                { "function": "get_query_playbook", "args": { "task": "overview", "entityNames": ["IfcBuilding"] } }
            ]
        }));

        let response: OpencodeTurnResponse =
            serde_json::from_value(normalized).expect("normalized response should parse");

        assert_eq!(
            response.tool_calls,
            vec![
                OpencodeToolCall::GetSchemaContext,
                OpencodeToolCall::GetQueryPlaybook {
                    goal: "overview".to_owned(),
                    entity_names: vec!["IfcBuilding".to_owned()],
                }
            ]
        );
    }

    #[test]
    fn normalize_response_shape_accepts_call_wrapper_tool_calls() {
        let normalized = normalize_response_shape(serde_json::json!({
            "tool_calls": [
                {
                    "function": "call",
                    "args": {
                        "function_name": "get_query_playbook",
                        "arguments": {
                            "topic": "general_overview",
                            "entityTypes": ["IfcProject"]
                        }
                    }
                }
            ]
        }));

        let response: OpencodeTurnResponse =
            serde_json::from_value(normalized).expect("normalized response should parse");

        assert_eq!(
            response.tool_calls,
            vec![OpencodeToolCall::GetQueryPlaybook {
                goal: "general_overview".to_owned(),
                entity_names: vec!["IfcProject".to_owned()],
            }]
        );
    }

    #[test]
    fn normalize_response_shape_accepts_request_tools_meta_call() {
        let normalized = normalize_response_shape(serde_json::json!({
            "tool_calls": [
                {
                    "function": "request_tools",
                    "args": {
                        "tools": ["get_schema", "get_model_details"]
                    }
                }
            ]
        }));

        let response: OpencodeTurnResponse =
            serde_json::from_value(normalized).expect("normalized response should parse");

        assert_eq!(
            response.tool_calls,
            vec![OpencodeToolCall::RequestTools {
                tools: vec![
                    "get_schema_context".to_owned(),
                    "get_model_details".to_owned(),
                ],
            }]
        );
    }

    #[test]
    fn normalize_response_shape_accepts_action_kind_aliases() {
        let normalized = normalize_response_shape(serde_json::json!({
            "toolCalls": [
                {
                    "kind": "emit_ui_actions",
                    "actions": [
                        { "kind": "graphSetSeeds", "dbNodeIds": [215, 216], "sourceResource": "ifc/infra-road" },
                        { "kind": "propertiesShowNode", "dbNodeId": 215, "source_resource": "ifc/infra-road" },
                        { "kind": "elementsSelect", "semanticIds": ["wall-a"] },
                        { "kind": "frame" }
                    ]
                }
            ]
        }));

        let response: OpencodeTurnResponse =
            serde_json::from_value(normalized).expect("normalized response should parse");

        assert_eq!(
            response.tool_calls,
            vec![OpencodeToolCall::EmitUiActions {
                actions: vec![
                    PlannedUiAction::GraphSetSeeds {
                        db_node_ids: vec![215, 216],
                        resource: Some("ifc/infra-road".to_owned()),
                    },
                    PlannedUiAction::PropertiesShowNode {
                        db_node_id: 215,
                        resource: Some("ifc/infra-road".to_owned()),
                    },
                    PlannedUiAction::ElementsSelect {
                        semantic_ids: vec!["wall-a".to_owned()],
                        resource: None,
                    },
                    PlannedUiAction::ViewerFrameVisible,
                ]
            }]
        );
    }

    #[test]
    fn normalize_response_shape_accepts_inspection_update_modes() {
        let normalized = normalize_response_shape(serde_json::json!({
            "toolCalls": [
                {
                    "kind": "emit_ui_actions",
                    "actions": [
                        {
                            "kind": "elementsInspect",
                            "semanticIds": ["kitchen-a"],
                            "sourceResource": "ifc/building-architecture",
                            "inspectionMode": "include"
                        },
                        {
                            "kind": "elements.inspect",
                            "ids": ["old-hvac"],
                            "resource": "ifc/building-hvac",
                            "mode": "subtract"
                        }
                    ]
                }
            ]
        }));

        let response: OpencodeTurnResponse =
            serde_json::from_value(normalized).expect("normalized response should parse");

        assert_eq!(
            response.tool_calls,
            vec![OpencodeToolCall::EmitUiActions {
                actions: vec![
                    PlannedUiAction::ElementsInspect {
                        semantic_ids: vec!["kitchen-a".to_owned()],
                        resource: Some("ifc/building-architecture".to_owned()),
                        mode: InspectionUpdateMode::Add,
                    },
                    PlannedUiAction::ElementsInspect {
                        semantic_ids: vec!["old-hvac".to_owned()],
                        resource: Some("ifc/building-hvac".to_owned()),
                        mode: InspectionUpdateMode::Remove,
                    },
                ]
            }]
        );
    }

    #[test]
    fn parse_opencode_response_accepts_streamed_event_objects() {
        let stdout = br#"{"type":"step_start","part":{"type":"step-start"}}
{"type":"text","part":{"text":"{\"toolCalls\":[{\"kind\":\"get_entity_reference\",\"entity_names\":[\"IfcSlab\"]}],\"transcript\":[{\"kind\":\"assistant\",\"text\":\"Checking the slab entity reference first.\"}],\"finalText\":\"\"}"}}
{"type":"text","part":{"text":"{\"finalText\":\"\",\"toolCalls\":[{\"kind\":\"get_entity_reference\",\"entity_names\":[\"IfcSlab\"]},{\"kind\":\"run_readonly_cypher\",\"cypher\":\"MATCH (s:IfcSlab)-[r]-(n) RETURN type(r) AS relation LIMIT 10\"}],\"transcript\":[{\"kind\":\"assistant\",\"text\":\"Checking the slab entity reference, then querying the live model.\"}]}"}}
{"type":"step_finish","part":{"type":"step-finish"}}"#;

        let response =
            parse_opencode_response(stdout).expect("streamed event response should parse");

        assert_eq!(
            response.transcript,
            vec![OpencodeTranscriptEvent {
                kind: OpencodeTranscriptEventKind::Assistant,
                text: "Checking the slab entity reference, then querying the live model."
                    .to_owned(),
            }]
        );
        assert_eq!(
            response.tool_calls,
            vec![
                OpencodeToolCall::GetEntityReference {
                    entity_names: vec!["IfcSlab".to_owned()]
                },
                OpencodeToolCall::RunReadonlyCypher {
                    cypher: "MATCH (s:IfcSlab)-[r]-(n) RETURN type(r) AS relation LIMIT 10"
                        .to_owned(),
                    why: None,
                }
            ]
        );
    }

    #[test]
    fn parse_opencode_response_surfaces_streamed_provider_error() {
        let stdout = br#"{"type":"step_start","part":{"type":"step-start"}}
{"type":"error","error":{"name":"UnknownError","data":{"message":"{\"type\":\"error\",\"sequence_number\":2,\"error\":{\"type\":\"server_error\",\"code\":\"server_error\",\"message\":\"An error occurred while processing your request. Please include the request ID abc-123.\",\"param\":null}}"}}}"#;

        let error = parse_opencode_response(stdout)
            .expect_err("provider error event should not be reported as invalid planner JSON");

        match error {
            OpencodeResponseParseError::ProviderError { message } => {
                assert!(message.contains("provider returned server_error"));
                assert!(message.contains("request ID abc-123"));
            }
            OpencodeResponseParseError::InvalidJson(error) => {
                panic!("expected provider error, got invalid json: {error}");
            }
        }
    }

    #[test]
    fn transient_provider_error_detection_matches_server_errors() {
        assert!(is_transient_provider_error_message(
            "provider returned server_error: please retry later"
        ));
        assert!(!is_transient_provider_error_message(
            "provider returned invalid_api_key: bad key"
        ));
    }

    #[test]
    fn readonly_cypher_tool_result_includes_convenience_ids_and_row_objects() {
        let payload = serialize_readonly_cypher_tool_result(&AgentReadonlyCypherResult {
            columns: vec!["node_id".to_owned(), "project_name".to_owned()],
            rows: vec![vec!["215".to_owned(), "Bridge project".to_owned()]],
            db_node_ids: vec![215],
            semantic_element_ids: Vec::new(),
        })
        .expect("tool result should serialize");

        let value: Value = serde_json::from_str(&payload).expect("payload should be json");
        assert_eq!(
            value.get("firstDbNodeId").and_then(Value::as_i64),
            Some(215)
        );
        assert_eq!(
            value
                .get("dbNodeIds")
                .and_then(Value::as_array)
                .map(|items| items.len()),
            Some(1)
        );
        assert_eq!(
            value
                .get("rowObjects")
                .and_then(Value::as_array)
                .and_then(|rows| rows.first())
                .and_then(|row| row.get("node_id"))
                .and_then(Value::as_str),
            Some("215")
        );
    }

    fn sample_step_request() -> OpencodeTurnRequest {
        OpencodeTurnRequest {
            resource: "ifc/building-architecture".to_owned(),
            schema_id: "IFC4X3_ADD2".to_owned(),
            schema_slug: Some("ifc4x3_add2".to_owned()),
            user_input: "hide the roof".to_owned(),
            session_history: vec![OpencodeTranscriptEvent {
                kind: OpencodeTranscriptEventKind::Assistant,
                text: "The slabs appear under RELATED_OBJECTS and RELATED_ELEMENTS.".to_owned(),
            }],
            transcript: vec![OpencodeTranscriptEvent {
                kind: OpencodeTranscriptEventKind::User,
                text: "hide the roof".to_owned(),
            }],
            tool_results: Vec::new(),
        }
    }

    #[test]
    fn prompt_includes_session_history_and_id_guidance() {
        let prompt = build_prompt(&sample_step_request(), false);

        assert!(prompt.contains("Bound IFC resource for this turn: ifc/building-architecture."));
        assert!(prompt.contains("Bound IFC schema for this turn: IFC4X3_ADD2"));
        assert!(prompt.contains("Do not swap in a placeholder or `/api`."));
        assert!(prompt.contains("sessionHistory"));
        assert!(prompt.contains("show me the relations"));
        assert!(prompt.contains("Never use `toString(id(...))` in Cypher."));
        assert!(prompt.contains("must include a short `why` string"));
        assert!(prompt.contains("get_query_playbook"));
        assert!(prompt.contains("get_model_details"));
        assert!(prompt.contains("request_tools"));
        assert!(prompt.contains("get_relation_reference"));
    }

    #[test]
    fn native_agent_prompt_mentions_native_tools() {
        let prompt = build_prompt(&sample_step_request(), true);

        assert!(prompt.contains("ifc-explorer"));
        assert!(prompt.contains("native OpenCode access"));
        assert!(prompt.contains("Use the native `ifc_*` tools directly"));
        assert!(prompt.contains("Bound IFC resource for this turn: ifc/building-architecture."));
        assert!(!prompt.contains("You do not have direct tools."));
    }

    #[test]
    fn native_turn_prompt_includes_bound_resource_and_schema() {
        let request = AgentBackendTurnRequest {
            resource: "ifc/building-architecture".to_owned(),
            schema_id: "IFC4X3_ADD2".to_owned(),
            schema_slug: Some("ifc4x3_add2".to_owned()),
            input: "what can you tell me about the model?".to_owned(),
            session_history: Vec::new(),
        };

        let prompt = build_native_turn_prompt(&request, Some("ifc-explorer-strict"));

        assert!(prompt.contains("Bound IFC resource for this turn: ifc/building-architecture."));
        assert!(prompt.contains("Bound IFC schema for this turn: IFC4X3_ADD2 (ifc4x3_add2)."));
        assert!(prompt.contains("Use the exact resource string above in any `ifc_*` tool call."));
        assert!(prompt.contains("Selected native OpenCode agent: `ifc-explorer-strict`."));
        assert!(prompt.contains("User request:"));
        assert!(prompt.contains("what can you tell me about the model?"));
    }
}
