#[path = "server/agent_executor.rs"]
mod agent_executor;
#[path = "server/agent_error_log.rs"]
mod agent_error_log;
#[path = "server/agent_query_log.rs"]
mod agent_query_log;
#[path = "server/opencode_executor.rs"]
mod opencode_executor;
#[path = "server/opencode_acp.rs"]
mod opencode_acp;
#[path = "server/schema_reference.rs"]
mod schema_reference;

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    env,
    error::Error,
    fs,
    io::{ErrorKind, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use cc_w_backend::{GeometryBackend, available_demo_resources};
use cc_w_platform_web::{
    WebPreparedGeometryPackage, WebPreparedPackageResponse, WebResourceCatalog,
};
use cc_w_velr::{
    IfcArtifactLayout, IfcSchemaId, VelrIfcModel, available_ifc_body_resources,
    default_ifc_artifacts_root, parse_ifc_body_resource,
};
use serde::{Deserialize, Serialize};

use crate::agent_executor::{
    AgentBackendTurnRequest as BackendAgentTurnRequest,
    AgentEntityReference as BackendAgentEntityReference, AgentExecutor as BackendAgentExecutor,
    AgentGraphMode as BackendAgentGraphMode, AgentNeighborEdge as BackendAgentNeighborEdge,
    AgentNeighborGraph as BackendAgentNeighborGraph, AgentNeighborNode as BackendAgentNeighborNode,
    AgentNodePropertiesResult as BackendAgentNodePropertiesResult,
    AgentNodeRelationSummary as BackendAgentNodeRelationSummary,
    AgentNodeSummary as BackendAgentNodeSummary, AgentProgressSink as BackendAgentProgressSink,
    AgentQueryPlaybook as BackendAgentQueryPlaybook,
    AgentReadonlyCypherResult as BackendAgentReadonlyCypherResult,
    AgentReadonlyCypherRuntime as BackendAgentReadonlyCypherRuntime,
    AgentRelationReference as BackendAgentRelationReference,
    AgentSchemaContext as BackendAgentSchemaContext,
    AgentTranscriptEvent as BackendAgentTranscriptEvent,
    AgentTranscriptEventKind as BackendAgentTranscriptEventKind,
    AgentUiAction as BackendAgentUiAction, NullAgentProgressSink, StubAgentExecutor,
    validate_agent_action_candidates as validate_backend_agent_action_candidates,
    validate_agent_readonly_cypher as validate_backend_agent_readonly_cypher,
};
use crate::agent_error_log::{AgentErrorLogEntry, AgentErrorLogger};
use crate::agent_query_log::{AgentQueryLogEntry, AgentQueryLogger};
use crate::opencode_executor::{
    OpencodeDiscoveredModel, OpencodeExecutor, OpencodeExecutorConfig, OpencodeNativeServer,
    discover_opencode_models,
};
use crate::schema_reference::{
    load_entity_references, load_query_playbooks, load_relation_references, load_schema_context,
};

const DEFAULT_HOST: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 8001;
const DEFAULT_ROOT: &str = "crates/cc-w-platform-web/artifacts/viewer";
const MAX_REQUEST_HEADER_BYTES: usize = 16 * 1024;
const MAX_REQUEST_BODY_BYTES: usize = 64 * 1024;
const PORT_SEARCH_LIMIT: u16 = 32;
const RESOURCES_API_PATH: &str = "/api/resources";
const PACKAGE_API_PATH: &str = "/api/package";
const CYPHER_API_PATH: &str = "/api/cypher";
const GRAPH_SUBGRAPH_API_PATH: &str = "/api/graph/subgraph";
const GRAPH_NODE_PROPERTIES_API_PATH: &str = "/api/graph/node-properties";
const AGENT_CAPABILITIES_API_PATH: &str = "/api/agent/capabilities";
const AGENT_SESSION_API_PATH: &str = "/api/agent/session";
const AGENT_TURN_API_PATH: &str = "/api/agent/turn";
const AGENT_TURN_START_API_PATH: &str = "/api/agent/turn-start";
const AGENT_TURN_POLL_API_PATH: &str = "/api/agent/turn-poll";
const DEFAULT_GRAPH_HOPS: usize = 1;
const MAX_GRAPH_HOPS: usize = 2;
const DEFAULT_GRAPH_MAX_NODES: usize = 120;
const DEFAULT_GRAPH_MAX_EDGES: usize = 240;
const DEFAULT_GRAPH_NODE_PROPERTIES_MAX_RELATIONS: usize = 24;
const MAX_GRAPH_NODE_PROPERTIES_MAX_RELATIONS: usize = 64;
const DEFAULT_AGENT_DESCRIBE_NODE_LIMIT: usize = 24;
const MAX_GRAPH_MAX_NODES: usize = 400;
const MAX_GRAPH_MAX_EDGES: usize = 800;
const MAX_AGENT_ACTIONS: usize = 16;
const MAX_AGENT_ACTION_IDS: usize = 2_000;
const MAX_AGENT_TRANSCRIPT_EVENTS: usize = 32;
const MAX_AGENT_SESSION_CONTEXT_EVENTS: usize = 24;
const DEFAULT_AGENT_MAX_READONLY_QUERIES_PER_TURN: usize = 12;
const DEFAULT_AGENT_MAX_ROWS_PER_QUERY: usize = 200;
const DEFAULT_AGENT_QUERY_LOG_PATH: &str = ".tools/logs/agent-readonly-cypher.jsonl";
const DEFAULT_AGENT_ERROR_LOG_PATH: &str = ".tools/logs/agent-backend-errors.jsonl";
const DEFAULT_OPENCODE_MODEL_ID: &str = "openai/gpt-5.4";
const DEFAULT_OPENCODE_MODEL_IDS: &[&str] = &["openai/gpt-5.4", "openai/gpt-5.4-mini"];
const DEFAULT_OPENCODE_MODEL_DISCOVERY_TIMEOUT_MS: u64 = 5_000;
const ANSI_RESET: &str = "\x1b[0m";
const ANSI_RED_BOLD: &str = "\x1b[1;31m";
const ANSI_YELLOW_BOLD: &str = "\x1b[1;33m";
const ANSI_MAGENTA_BOLD: &str = "\x1b[1;35m";

static SHOULD_STOP: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy)]
enum ConsoleLogKind {
    Error,
    Warn,
    AgentError,
}

impl ConsoleLogKind {
    const fn color(self) -> &'static str {
        match self {
            Self::Error => ANSI_RED_BOLD,
            Self::Warn => ANSI_YELLOW_BOLD,
            Self::AgentError => ANSI_MAGENTA_BOLD,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Error => "[w web error]",
            Self::Warn => "[w web warn]",
            Self::AgentError => "[w web agent error]",
        }
    }
}

fn console_log(kind: ConsoleLogKind, message: impl AsRef<str>) {
    println!(
        "{}{}{} {}",
        kind.color(),
        kind.label(),
        ANSI_RESET,
        message.as_ref()
    );
}

extern "C" fn handle_shutdown_signal(_signal: libc::c_int) {
    SHOULD_STOP.store(true, Ordering::SeqCst);
}

fn install_shutdown_signal_handlers() -> Result<(), std::io::Error> {
    unsafe {
        install_single_shutdown_signal(libc::SIGINT)?;
        install_single_shutdown_signal(libc::SIGTERM)?;
    }
    Ok(())
}

unsafe fn install_single_shutdown_signal(signal: libc::c_int) -> Result<(), std::io::Error> {
    let previous = unsafe { libc::signal(signal, handle_shutdown_signal as libc::sighandler_t) };
    if previous == libc::SIG_ERR {
        return Err(std::io::Error::other(format!(
            "could not install shutdown handler for signal {signal}"
        )));
    }
    Ok(())
}

pub(crate) fn should_stop_requested() -> bool {
    SHOULD_STOP.load(Ordering::SeqCst)
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse(env::args().skip(1))?;
    install_shutdown_signal_handlers()?;
    let root = fs::canonicalize(&args.root)
        .map_err(|error| format!("w web server could not resolve {:?}: {error}", args.root))?;
    let mut agent_runtime = AgentRuntimeConfig::from_env()?;
    let (listener, bound_port) = bind_listener(&args.host, args.port)?;
    listener.set_nonblocking(true)?;
    let url = format!("http://{}:{}/", args.host, bound_port);
    if agent_runtime.native_server.is_none() {
        if let AgentBackend::Opencode(config) = &agent_runtime.backend {
            agent_runtime.native_server = Some(OpencodeNativeServer::start(config, Some(&url))?);
        }
    }
    let server_state = Arc::new(ServerState {
        root,
        ifc_artifacts_root: args.ifc_artifacts_root,
        ifc_model_cache: Mutex::new(HashMap::new()),
        agent_sessions: Arc::new(Mutex::new(AgentSessionStore::default())),
        agent_turns: Arc::new(Mutex::new(AgentTurnStore::default())),
        agent_runtime,
    });

    println!("w web viewer serving {}", server_state.root.display());
    println!(
        "w web query artifacts {}",
        server_state.ifc_artifacts_root.display()
    );
println!(
        "w web AI backend {}",
        server_state.agent_runtime.backend.label()
    );
    if let Some(agent_name) = agent_backend_agent_name(&server_state.agent_runtime.backend) {
        println!("w web AI agent {}", agent_name);
    }
    println!(
        "w web AI models {}",
        capability_option_ids(&server_state.agent_runtime.models).join(", ")
    );
    println!(
        "w web AI levels {}",
        capability_option_ids(&server_state.agent_runtime.levels).join(", ")
    );
    println!(
        "w web AI query log {}",
        server_state.agent_runtime.query_logger.path().display()
    );
    println!(
        "w web AI error log {}",
        server_state.agent_runtime.error_logger.path().display()
    );
    if bound_port != args.port {
        console_log(
            ConsoleLogKind::Warn,
            format!(
                "viewer port {} was busy, using {} instead",
                args.port, bound_port
            ),
        );
    }
    println!("open {}", url);

    loop {
        if should_stop_requested() {
            console_log(
                ConsoleLogKind::Warn,
                "shutdown requested; stopping OpenCode child and viewer",
            );
            if let Some(native_server) = server_state.agent_runtime.native_server.as_ref() {
                native_server.shutdown();
            }
            break;
        }

        match listener.accept() {
            Ok((stream, _)) => {
                if let Err(error) = handle_connection(stream, &server_state) {
                    console_log(
                        ConsoleLogKind::Error,
                        format!("server request failed: {error}"),
                    );
                }
            }
            Err(error)
                if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) =>
            {
                thread::sleep(Duration::from_millis(50));
            }
            Err(error) => console_log(
                ConsoleLogKind::Error,
                format!("server accept failed: {error}"),
            ),
        }
    }

    Ok(())
}

#[derive(Debug, Clone)]
struct Args {
    host: String,
    port: u16,
    root: PathBuf,
    ifc_artifacts_root: PathBuf,
}

struct ServerState {
    root: PathBuf,
    ifc_artifacts_root: PathBuf,
    ifc_model_cache: Mutex<HashMap<String, CachedIfcModel>>,
    agent_sessions: Arc<Mutex<AgentSessionStore>>,
    agent_turns: Arc<Mutex<AgentTurnStore>>,
    agent_runtime: AgentRuntimeConfig,
}

#[derive(Clone)]
struct AgentWorkerState {
    ifc_artifacts_root: PathBuf,
    agent_sessions: Arc<Mutex<AgentSessionStore>>,
    agent_turns: Arc<Mutex<AgentTurnStore>>,
    agent_runtime: AgentRuntimeConfig,
    native_server: Option<Arc<OpencodeNativeServer>>,
}

#[derive(Debug, Default)]
struct AgentSessionStore {
    next_session_number: u64,
    sessions: HashMap<String, AgentSession>,
}

#[derive(Debug, Default)]
struct AgentTurnStore {
    next_turn_number: u64,
    turns: HashMap<String, AgentTurnState>,
}

#[derive(Debug, Clone)]
struct AgentSession {
    session_id: String,
    resource: String,
    schema_id: String,
    schema_slug: Option<String>,
    opencode_session_id: Option<String>,
    turn_count: u64,
    transcript: Vec<AgentTranscriptEvent>,
}

#[derive(Debug, Clone)]
struct AgentTurnState {
    turn_id: String,
    session_id: String,
    resource: String,
    schema_id: String,
    schema_slug: Option<String>,
    next_seq: u64,
    events: Vec<AgentTurnProgressEvent>,
    done: bool,
    result: Option<AgentTurnApiResponse>,
    error: Option<String>,
}

#[derive(Debug, Clone)]
struct AgentRuntimeConfig {
    backend: AgentBackend,
    native_server: Option<Arc<OpencodeNativeServer>>,
    models: Vec<AgentCapabilityOption>,
    levels: Vec<AgentCapabilityOption>,
    levels_by_model: BTreeMap<String, Vec<AgentCapabilityOption>>,
    default_model_id: Option<String>,
    default_level_id: Option<String>,
    max_queries_per_turn: usize,
    max_rows_per_query: usize,
    query_logger: Arc<AgentQueryLogger>,
    error_logger: Arc<AgentErrorLogger>,
}

#[derive(Debug, Clone)]
enum AgentBackend {
    Stub,
    Opencode(OpencodeExecutorConfig),
}

impl AgentBackend {
    const fn id(&self) -> &'static str {
        match self {
            Self::Stub => "stub",
            Self::Opencode(_) => "opencode",
        }
    }

    const fn label(&self) -> &'static str {
        match self {
            Self::Stub => "stub",
            Self::Opencode(_) => "opencode",
        }
    }
}

fn agent_backend_agent_name(backend: &AgentBackend) -> Option<&str> {
    match backend {
        AgentBackend::Stub => None,
        AgentBackend::Opencode(config) => config.agent.as_deref(),
    }
}

fn agent_capabilities_response(runtime: &AgentRuntimeConfig) -> AgentCapabilitiesApiResponse {
    let backend_id = runtime.backend.id().to_owned();
    AgentCapabilitiesApiResponse {
        default_backend_id: backend_id.clone(),
        default_model_id: runtime.default_model_id.clone(),
        default_level_id: runtime.default_level_id.clone(),
        backends: vec![AgentBackendCapability {
            id: backend_id,
            label: runtime.backend.label().to_owned(),
            models: runtime.models.clone(),
            levels: runtime.levels.clone(),
            levels_by_model: runtime.levels_by_model.clone(),
        }],
    }
}

fn default_agent_model_id(backend: &AgentBackend) -> Option<String> {
    match backend {
        AgentBackend::Stub => Some("stub/default".to_owned()),
        AgentBackend::Opencode(config) => config
            .model
            .clone()
            .or_else(|| Some(DEFAULT_OPENCODE_MODEL_ID.to_owned())),
    }
}

fn default_agent_level_id(backend: &AgentBackend) -> Option<String> {
    match backend {
        AgentBackend::Stub => Some("standard".to_owned()),
        AgentBackend::Opencode(config) => config
            .variant
            .clone()
            .filter(|value| is_visible_agent_level_id(value)),
    }
}

fn agent_model_options_from_env(
    backend: &AgentBackend,
    default_model_id: Option<&str>,
) -> Result<
    (
        Vec<AgentCapabilityOption>,
        BTreeMap<String, Vec<AgentCapabilityOption>>,
    ),
    Box<dyn Error>,
> {
    let defaults = match backend {
        AgentBackend::Stub => vec!["stub/default".to_owned()],
        AgentBackend::Opencode(_) => {
            let mut values = DEFAULT_OPENCODE_MODEL_IDS
                .iter()
                .map(|value| (*value).to_owned())
                .collect::<Vec<_>>();
            if let Some(default_model_id) = default_model_id {
                values.push(default_model_id.to_owned());
            }
            values
        }
    };
    let mut levels_by_model = BTreeMap::new();
    let mut values = if let Some(values) =
        env_list("CC_W_AGENT_MODELS").or_else(|| env_list("CC_W_OPENCODE_MODELS"))
    {
        values
    } else if should_discover_agent_models()? {
        match discover_agent_models(backend, default_model_id)? {
            Some(models) => {
                levels_by_model = discovered_levels_by_model(&models);
                models.into_iter().map(|model| model.id).collect()
            }
            None => defaults,
        }
    } else {
        defaults
    };
    if let Some(default_model_id) = default_model_id {
        values.push(default_model_id.to_owned());
    }
    Ok((
        capability_options(values, label_for_agent_model),
        levels_by_model,
    ))
}

fn agent_level_options_from_env(
    backend: &AgentBackend,
    default_level_id: Option<&str>,
) -> Vec<AgentCapabilityOption> {
    let defaults = match backend {
        AgentBackend::Stub => vec!["standard".to_owned()],
        AgentBackend::Opencode(_) => Vec::new(),
    };
    let mut values = env_list("CC_W_AGENT_LEVELS")
        .or_else(|| env_list("CC_W_OPENCODE_VARIANTS"))
        .unwrap_or(defaults);
    values.retain(|value| is_visible_agent_level_id(value));
    if let Some(default_level_id) = default_level_id.filter(|value| is_visible_agent_level_id(value)) {
        values.push(default_level_id.to_owned());
    }
    capability_options(values, label_for_agent_level)
}

fn env_list(key: &str) -> Option<Vec<String>> {
    let value = env::var(key).ok()?;
    let values = value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    (!values.is_empty()).then_some(values)
}

fn should_discover_agent_models() -> Result<bool, Box<dyn Error>> {
    if let Some(value) = env_bool("CC_W_AGENT_DISCOVER_MODELS")? {
        return Ok(value);
    }
    if let Some(value) = env_bool("CC_W_OPENCODE_DISCOVER_MODELS")? {
        return Ok(value);
    }
    Ok(true)
}

fn env_bool(key: &str) -> Result<Option<bool>, Box<dyn Error>> {
    let Ok(value) = env::var(key) else {
        return Ok(None);
    };
    let value = value.trim();
    if value.eq_ignore_ascii_case("1")
        || value.eq_ignore_ascii_case("true")
        || value.eq_ignore_ascii_case("yes")
        || value.eq_ignore_ascii_case("on")
    {
        Ok(Some(true))
    } else if value.eq_ignore_ascii_case("0")
        || value.eq_ignore_ascii_case("false")
        || value.eq_ignore_ascii_case("no")
        || value.eq_ignore_ascii_case("off")
    {
        Ok(Some(false))
    } else {
        Err(format!("{key} must be one of 1/0, true/false, yes/no, or on/off").into())
    }
}

fn discover_agent_models(
    backend: &AgentBackend,
    default_model_id: Option<&str>,
) -> Result<Option<Vec<OpencodeDiscoveredModel>>, Box<dyn Error>> {
    let AgentBackend::Opencode(config) = backend else {
        return Ok(None);
    };
    let providers = opencode_model_discovery_providers(config, default_model_id);
    let timeout = Duration::from_millis(parse_env_u64_with_default(
        "CC_W_OPENCODE_MODEL_DISCOVERY_TIMEOUT_MS",
        DEFAULT_OPENCODE_MODEL_DISCOVERY_TIMEOUT_MS,
    )?);
    let discovered = if providers.is_empty() {
        discover_opencode_models(config, None, timeout).map_err(Into::into)
    } else {
        discover_opencode_models_for_providers(config, &providers, timeout)
    };
    match discovered {
        Ok(models) if !models.is_empty() => {
            println!(
                "w web AI discovered {} model{}{}",
                models.len(),
                if models.len() == 1 { "" } else { "s" },
                if providers.is_empty() {
                    " across all providers".to_owned()
                } else {
                    format!(" for {}", providers.join(", "))
                }
            );
            Ok(Some(models))
        }
        Ok(_) => {
            console_log(
                ConsoleLogKind::Warn,
                "opencode model discovery returned no models; using fallback model list",
            );
            Ok(None)
        }
        Err(error) => {
            console_log(
                ConsoleLogKind::Warn,
                format!("opencode model discovery failed; using fallback model list: {error}"),
            );
            Ok(None)
        }
    }
}

fn discovered_levels_by_model(
    models: &[OpencodeDiscoveredModel],
) -> BTreeMap<String, Vec<AgentCapabilityOption>> {
    models
        .iter()
        .filter_map(|model| {
            let variants = model
                .variants
                .iter()
                .filter(|variant| is_visible_agent_level_id(variant))
                .cloned()
                .collect::<Vec<_>>();
            if variants.is_empty() {
                return None;
            }
            Some((
                model.id.clone(),
                capability_options(variants, label_for_agent_level),
            ))
        })
        .collect()
}

#[derive(Debug, Clone, Deserialize)]
struct OpencodeProviderWhitelistFile {
    providers: Vec<String>,
}

fn discover_opencode_models_for_providers(
    config: &OpencodeExecutorConfig,
    providers: &[String],
    timeout: Duration,
) -> Result<Vec<OpencodeDiscoveredModel>, Box<dyn Error>> {
    let mut merged = Vec::new();
    let mut seen = HashSet::new();

    for provider in providers {
        let provider = provider.trim();
        if provider.is_empty() {
            continue;
        }
        match discover_opencode_models(config, Some(provider), timeout) {
            Ok(models) => {
                for model in models {
                    if seen.insert(model.id.clone()) {
                        merged.push(model);
                    }
                }
            }
            Err(error) => {
                console_log(
                    ConsoleLogKind::Warn,
                    format!("opencode model discovery for `{provider}` failed: {error}"),
                );
            }
        }
    }

    Ok(merged)
}

fn opencode_model_discovery_providers(
    config: &OpencodeExecutorConfig,
    default_model_id: Option<&str>,
) -> Vec<String> {
    let mut providers = load_opencode_provider_whitelist(config).unwrap_or_default();
    if providers.is_empty() {
        if let Some(provider) = default_model_id
            .and_then(agent_model_provider)
            .map(ToOwned::to_owned)
        {
            providers.push(provider);
        }
    }
    providers
}

fn load_opencode_provider_whitelist(config: &OpencodeExecutorConfig) -> Option<Vec<String>> {
    let whitelist_path = config.config_path.as_ref()?.with_file_name("provider-whitelist.json");
    let Ok(contents) = fs::read_to_string(&whitelist_path) else {
        return None;
    };
    let parsed: OpencodeProviderWhitelistFile = match serde_json::from_str(&contents) {
        Ok(parsed) => parsed,
        Err(error) => {
            console_log(
                ConsoleLogKind::Warn,
                format!(
                    "could not parse provider whitelist {}; using default provider list: {error}",
                    whitelist_path.display()
                ),
            );
            return None;
        }
    };
    let mut providers = Vec::new();
    let mut seen = HashSet::new();
    for provider in parsed.providers {
        let trimmed = provider.trim();
        if trimmed.is_empty() {
            continue;
        }
        if seen.insert(trimmed.to_owned()) {
            providers.push(trimmed.to_owned());
        }
    }
    if providers.is_empty() {
        console_log(
            ConsoleLogKind::Warn,
            format!(
                "provider whitelist {} was empty; using default provider list",
                whitelist_path.display()
            ),
        );
        return None;
    }
    println!("w web AI provider whitelist {}", providers.join(", "));
    Some(providers)
}

fn agent_model_provider(model_id: &str) -> Option<&str> {
    let (provider, model) = model_id.split_once('/')?;
    (!provider.trim().is_empty() && !model.trim().is_empty()).then_some(provider.trim())
}

fn capability_options(
    values: Vec<String>,
    labeler: fn(&str) -> String,
) -> Vec<AgentCapabilityOption> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter_map(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() || !seen.insert(trimmed.to_owned()) {
                return None;
            }
            Some(AgentCapabilityOption {
                id: trimmed.to_owned(),
                label: labeler(trimmed),
            })
        })
        .collect()
}

fn capability_option_ids(options: &[AgentCapabilityOption]) -> Vec<&str> {
    options.iter().map(|option| option.id.as_str()).collect()
}

fn label_for_agent_model(model_id: &str) -> String {
    if model_id == "stub/default" {
        return "stub".to_owned();
    }
    model_id.to_owned()
}

fn label_for_agent_level(level_id: &str) -> String {
    match level_id {
        "minimal" => "minimal".to_owned(),
        "low" => "low".to_owned(),
        "medium" => "medium".to_owned(),
        "high" => "high".to_owned(),
        "xhigh" => "xhigh".to_owned(),
        "max" => "max".to_owned(),
        "standard" => "standard".to_owned(),
        value => value.to_owned(),
    }
}

fn is_visible_agent_level_id(level_id: &str) -> bool {
    let trimmed = level_id.trim();
    !trimmed.is_empty() && !trimmed.eq_ignore_ascii_case("none")
}

impl AgentRuntimeConfig {
    fn from_env() -> Result<Self, Box<dyn Error>> {
        let backend = match env::var("CC_W_AGENT_BACKEND") {
            Ok(value) if value.eq_ignore_ascii_case("opencode") => {
                AgentBackend::Opencode(OpencodeExecutorConfig::from_env()?)
            }
            Ok(value) if value.eq_ignore_ascii_case("stub") => AgentBackend::Stub,
            Ok(value) => {
                return Err(format!(
                    "unsupported CC_W_AGENT_BACKEND `{value}`; expected `stub` or `opencode`"
                )
                .into());
            }
            Err(_) => AgentBackend::Stub,
        };
        let default_model_id = default_agent_model_id(&backend);
        let configured_default_level_id = default_agent_level_id(&backend);
        let (models, levels_by_model) =
            agent_model_options_from_env(&backend, default_model_id.as_deref())?;
        let levels = agent_level_options_from_env(&backend, configured_default_level_id.as_deref());
        let default_level_id = configured_default_level_id.or_else(|| {
            default_model_id
                .as_deref()
                .and_then(|model_id| levels_by_model.get(model_id))
                .and_then(|levels| middle_level_id(levels))
                .or_else(|| middle_level_id(&levels))
                .map(ToOwned::to_owned)
        });

        Ok(Self {
            backend,
            native_server: None,
            models,
            levels,
            levels_by_model,
            default_model_id,
            default_level_id,
            max_queries_per_turn: parse_env_usize_with_default(
                "CC_W_AGENT_MAX_READONLY_QUERIES_PER_TURN",
                DEFAULT_AGENT_MAX_READONLY_QUERIES_PER_TURN,
            )?,
            max_rows_per_query: parse_env_usize_with_default(
                "CC_W_AGENT_MAX_ROWS_PER_QUERY",
                DEFAULT_AGENT_MAX_ROWS_PER_QUERY,
            )?,
            query_logger: Arc::new(AgentQueryLogger::new(default_agent_query_log_path())),
            error_logger: Arc::new(AgentErrorLogger::new(default_agent_error_log_path())),
        })
    }
}

fn default_agent_query_log_path() -> PathBuf {
    env::var_os("CC_W_AGENT_QUERY_LOG_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_AGENT_QUERY_LOG_PATH))
}

fn default_agent_error_log_path() -> PathBuf {
    env::var_os("CC_W_AGENT_ERROR_LOG_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_AGENT_ERROR_LOG_PATH))
}

#[derive(Clone)]
struct CachedIfcModel {
    database_stamp: DatabaseStamp,
    model: Arc<VelrIfcModel>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DatabaseStamp {
    bytes: u64,
    modified_unix_seconds: u64,
    modified_subsec_nanos: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IfcModelCacheStatus {
    Hit,
    Miss,
    Reloaded,
}

impl IfcModelCacheStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Hit => "cache_hit",
            Self::Miss => "cache_miss",
            Self::Reloaded => "cache_reloaded",
        }
    }
}

#[derive(Debug, Clone)]
struct HttpRequest {
    method: String,
    target: String,
    body: Vec<u8>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CypherApiRequest {
    resource: String,
    cypher: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PackageApiRequest {
    resource: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentSessionApiRequest {
    resource: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct AgentCapabilitiesApiResponse {
    default_backend_id: String,
    default_model_id: Option<String>,
    default_level_id: Option<String>,
    backends: Vec<AgentBackendCapability>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct AgentBackendCapability {
    id: String,
    label: String,
    models: Vec<AgentCapabilityOption>,
    levels: Vec<AgentCapabilityOption>,
    levels_by_model: BTreeMap<String, Vec<AgentCapabilityOption>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct AgentCapabilityOption {
    id: String,
    label: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentTurnApiRequest {
    session_id: String,
    input: String,
    backend_id: Option<String>,
    model_id: Option<String>,
    level_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentTurnStartApiRequest {
    session_id: String,
    input: String,
    backend_id: Option<String>,
    model_id: Option<String>,
    level_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct AgentTurnStartApiResponse {
    turn_id: String,
    session_id: String,
    resource: String,
    schema_id: String,
    schema_slug: Option<String>,
    backend_id: String,
    model_id: Option<String>,
    level_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentTurnPollApiRequest {
    turn_id: String,
    after_seq: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct GraphSubgraphApiRequest {
    resource: String,
    seed_node_ids: Vec<i64>,
    hops: Option<usize>,
    max_nodes: Option<usize>,
    max_edges: Option<usize>,
    mode: Option<GraphSubgraphMode>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct GraphNodePropertiesApiRequest {
    resource: String,
    db_node_id: i64,
    max_relations: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct CypherApiResponse {
    resource: String,
    columns: Vec<String>,
    rows: Vec<Vec<String>>,
    semantic_element_ids: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct AgentSessionApiResponse {
    session_id: String,
    resource: String,
    schema_id: String,
    schema_slug: Option<String>,
    transcript: Vec<AgentTranscriptEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct AgentTurnApiResponse {
    session_id: String,
    resource: String,
    schema_id: String,
    schema_slug: Option<String>,
    transcript: Vec<AgentTranscriptEvent>,
    actions: Vec<AgentUiAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct AgentTurnProgressEvent {
    seq: u64,
    item: AgentTranscriptEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct AgentTurnPollApiResponse {
    turn_id: String,
    done: bool,
    events: Vec<AgentTurnProgressEvent>,
    result: Option<AgentTurnApiResponse>,
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct AgentTranscriptEvent {
    kind: AgentTranscriptEventKind,
    text: String,
}

impl AgentTranscriptEvent {
    fn system(text: impl Into<String>) -> Self {
        Self {
            kind: AgentTranscriptEventKind::System,
            text: text.into(),
        }
    }

    fn user(text: impl Into<String>) -> Self {
        Self {
            kind: AgentTranscriptEventKind::User,
            text: text.into(),
        }
    }

    fn tool(text: impl Into<String>) -> Self {
        Self {
            kind: AgentTranscriptEventKind::Tool,
            text: text.into(),
        }
    }

    fn assistant(text: impl Into<String>) -> Self {
        Self {
            kind: AgentTranscriptEventKind::Assistant,
            text: text.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum AgentTranscriptEventKind {
    System,
    User,
    Tool,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum AgentUiAction {
    #[serde(rename = "graph.set_seeds")]
    GraphSetSeeds { db_node_ids: Vec<i64> },
    #[serde(rename = "properties.show_node")]
    PropertiesShowNode { db_node_id: i64 },
    #[serde(rename = "elements.hide")]
    ElementsHide { semantic_ids: Vec<String> },
    #[serde(rename = "elements.show")]
    ElementsShow { semantic_ids: Vec<String> },
    #[serde(rename = "elements.select")]
    ElementsSelect { semantic_ids: Vec<String> },
    #[serde(rename = "viewer.frame_visible")]
    ViewerFrameVisible,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AgentActionCandidate {
    kind: String,
    semantic_ids: Vec<String>,
    db_node_ids: Vec<i64>,
}

impl AgentActionCandidate {
    fn graph_set_seeds(db_node_ids: Vec<i64>) -> Self {
        Self {
            kind: "graph.set_seeds".to_owned(),
            semantic_ids: Vec::new(),
            db_node_ids,
        }
    }

    fn elements_hide(semantic_ids: Vec<String>) -> Self {
        Self {
            kind: "elements.hide".to_owned(),
            semantic_ids,
            db_node_ids: Vec::new(),
        }
    }

    fn elements_show(semantic_ids: Vec<String>) -> Self {
        Self {
            kind: "elements.show".to_owned(),
            semantic_ids,
            db_node_ids: Vec::new(),
        }
    }

    fn elements_select(semantic_ids: Vec<String>) -> Self {
        Self {
            kind: "elements.select".to_owned(),
            semantic_ids,
            db_node_ids: Vec::new(),
        }
    }

    fn properties_show_node(db_node_id: i64) -> Self {
        Self {
            kind: "properties.show_node".to_owned(),
            semantic_ids: Vec::new(),
            db_node_ids: vec![db_node_id],
        }
    }

    fn viewer_frame_visible() -> Self {
        Self {
            kind: "viewer.frame_visible".to_owned(),
            semantic_ids: Vec::new(),
            db_node_ids: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum GraphSubgraphMode {
    Raw,
    Semantic,
}

impl Default for GraphSubgraphMode {
    fn default() -> Self {
        Self::Raw
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct GraphSubgraphApiResponse {
    resource: String,
    mode: GraphSubgraphMode,
    hops: usize,
    max_nodes: usize,
    max_edges: usize,
    seed_node_ids: Vec<i64>,
    nodes: Vec<GraphSubgraphNode>,
    edges: Vec<GraphSubgraphEdge>,
    truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct GraphNodePropertiesApiResponse {
    resource: String,
    node: BackendAgentNodeSummary,
    properties: BTreeMap<String, String>,
    relations: Vec<BackendAgentNodeRelationSummary>,
    truncated_relations: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct GraphSubgraphNode {
    db_node_id: i64,
    declared_entity: String,
    global_id: Option<String>,
    name: Option<String>,
    display_label: String,
    hop_distance: usize,
    is_seed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct GraphSubgraphEdge {
    edge_id: String,
    source_db_node_id: i64,
    target_db_node_id: i64,
    relationship_type: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct ApiErrorResponse {
    error: String,
}

#[derive(Debug, Clone)]
struct PackageApiMetrics {
    kind: &'static str,
    cache_status: Option<&'static str>,
    definitions: usize,
    elements: usize,
    instances: usize,
}

#[derive(Debug, Clone)]
struct CypherApiMetrics {
    model_slug: String,
    model_cache_status: &'static str,
    open_ms: u128,
    query_ms: u128,
    extract_ids_ms: u128,
    columns: usize,
    rows: usize,
    semantic_element_ids: usize,
}

#[derive(Debug, Clone)]
struct GraphSubgraphApiMetrics {
    model_slug: String,
    model_cache_status: &'static str,
    open_ms: u128,
    build_ms: u128,
    nodes: usize,
    edges: usize,
    truncated: bool,
}

#[derive(Debug, Clone)]
struct GraphBuildLimits {
    hops: usize,
    max_nodes: usize,
    max_edges: usize,
    mode: GraphSubgraphMode,
}

#[derive(Debug, Clone)]
struct GraphNodeQueryRecord {
    db_node_id: i64,
    declared_entity: String,
    global_id: Option<String>,
    name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GraphEdgeQueryRecord {
    source_db_node_id: i64,
    target_db_node_id: i64,
    relationship_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GraphNodeRelationQueryRecord {
    direction: String,
    relationship_type: String,
    other_db_node_id: i64,
    other_declared_entity: String,
    other_global_id: Option<String>,
    other_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct CypherNodeCell {
    identity: i64,
    #[serde(default)]
    labels: Vec<String>,
    #[serde(default)]
    properties: BTreeMap<String, serde_json::Value>,
    #[serde(rename = "elementId")]
    element_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AgentReadonlyCypherResult {
    columns: Vec<String>,
    rows: Vec<Vec<String>>,
    db_node_ids: Vec<i64>,
    semantic_element_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StubAgentExecution {
    transcript: Vec<AgentTranscriptEvent>,
    action_candidates: Vec<AgentActionCandidate>,
    queries_executed: usize,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            host: DEFAULT_HOST.to_owned(),
            port: DEFAULT_PORT,
            root: PathBuf::from(DEFAULT_ROOT),
            ifc_artifacts_root: default_ifc_artifacts_root(),
        }
    }
}

fn parse_env_usize_with_default(key: &str, default: usize) -> Result<usize, Box<dyn Error>> {
    match env::var(key) {
        Ok(value) => value
            .parse::<usize>()
            .map_err(|error| format!("{key} must be an unsigned integer: {error}").into()),
        Err(_) => Ok(default),
    }
}

fn parse_env_u64_with_default(key: &str, default: u64) -> Result<u64, Box<dyn Error>> {
    match env::var(key) {
        Ok(value) => value
            .parse::<u64>()
            .map_err(|error| format!("{key} must be an unsigned integer: {error}").into()),
        Err(_) => Ok(default),
    }
}

impl Args {
    fn parse<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        let mut parsed = Self::default();
        let mut args = args.into_iter();

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--host" => {
                    parsed.host = args
                        .next()
                        .ok_or_else(|| "--host requires a value".to_owned())?;
                }
                "--port" => {
                    let value = args
                        .next()
                        .ok_or_else(|| "--port requires a value".to_owned())?;
                    parsed.port = value
                        .parse::<u16>()
                        .map_err(|_| format!("invalid port `{value}`"))?;
                }
                "--root" => {
                    let value = args
                        .next()
                        .ok_or_else(|| "--root requires a value".to_owned())?;
                    parsed.root = PathBuf::from(value);
                }
                "--ifc-artifacts-root" => {
                    let value = args
                        .next()
                        .ok_or_else(|| "--ifc-artifacts-root requires a value".to_owned())?;
                    parsed.ifc_artifacts_root = PathBuf::from(value);
                }
                "--help" | "-h" => {
                    print_usage();
                    std::process::exit(0);
                }
                other => {
                    return Err(format!("unknown argument `{other}`"));
                }
            }
        }

        Ok(parsed)
    }
}

fn print_usage() {
    println!("w web viewer server");
    println!();
    println!("Usage:");
    println!(
        "  cargo run -p cc-w-platform-web --bin cc-w-platform-web-server -- [--host 127.0.0.1] [--port 8001] [--root crates/cc-w-platform-web/artifacts/viewer] [--ifc-artifacts-root artifacts/ifc]"
    );
    println!();
    println!(
        "If the requested port is busy, the server will try the next {} ports.",
        PORT_SEARCH_LIMIT - 1
    );
}

fn bind_listener(host: &str, requested_port: u16) -> Result<(TcpListener, u16), String> {
    let mut last_error = None;

    for port in candidate_ports(requested_port) {
        match TcpListener::bind((host, port)) {
            Ok(listener) => return Ok((listener, port)),
            Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
                last_error = Some(error);
            }
            Err(error) => {
                return Err(format!(
                    "w web server could not bind {host}:{port}: {error}"
                ));
            }
        }
    }

    let upper_port = requested_port.saturating_add(PORT_SEARCH_LIMIT.saturating_sub(1));
    match last_error {
        Some(error) => Err(format!(
            "w web server could not bind any port in {}-{} on {}: {}",
            requested_port, upper_port, host, error
        )),
        None => Err(format!(
            "w web server did not have any candidate ports to try from {} on {}",
            requested_port, host
        )),
    }
}

fn candidate_ports(start: u16) -> impl Iterator<Item = u16> {
    (0..PORT_SEARCH_LIMIT).map(move |offset| start.saturating_add(offset))
}

fn handle_connection(mut stream: TcpStream, state: &Arc<ServerState>) -> Result<(), String> {
    stream
        .set_nonblocking(false)
        .map_err(|error| error.to_string())?;
    let request = read_request(&mut stream)?;
    let request_path = request_path_only(&request.target);

    match request.method.as_str() {
        "GET" | "HEAD" => {
            if request_path == RESOURCES_API_PATH {
                serve_resources_api(&mut stream, request.method == "HEAD", state)
            } else if request_path == AGENT_CAPABILITIES_API_PATH {
                serve_agent_capabilities_api(&mut stream, request.method == "HEAD", state)
            } else if request_path == CYPHER_API_PATH
                || request_path == PACKAGE_API_PATH
                || request_path == GRAPH_SUBGRAPH_API_PATH
                || request_path == GRAPH_NODE_PROPERTIES_API_PATH
                || request_path == AGENT_CAPABILITIES_API_PATH
                || request_path == AGENT_SESSION_API_PATH
                || request_path == AGENT_TURN_API_PATH
                || request_path == AGENT_TURN_START_API_PATH
                || request_path == AGENT_TURN_POLL_API_PATH
            {
                write_json_error(
                    &mut stream,
                    "405 Method Not Allowed",
                    "use POST for package, cypher, graph, and agent API routes",
                )
            } else {
                serve_path(
                    &mut stream,
                    request.method == "HEAD",
                    &request.target,
                    &state.root,
                )
            }
        }
        "POST" if request_path == PACKAGE_API_PATH => {
            serve_package_api(&mut stream, &request, state)
        }
        "POST" if request_path == CYPHER_API_PATH => serve_cypher_api(&mut stream, &request, state),
        "POST" if request_path == GRAPH_SUBGRAPH_API_PATH => {
            serve_graph_subgraph_api(&mut stream, &request, state)
        }
        "POST" if request_path == GRAPH_NODE_PROPERTIES_API_PATH => {
            serve_graph_node_properties_api(&mut stream, &request, state)
        }
        "POST" if request_path == AGENT_SESSION_API_PATH => {
            serve_agent_session_api(&mut stream, &request, state)
        }
        "POST" if request_path == AGENT_TURN_API_PATH => {
            serve_agent_turn_api(&mut stream, &request, state)
        }
        "POST" if request_path == AGENT_TURN_START_API_PATH => {
            serve_agent_turn_start_api(&mut stream, &request, state.clone())
        }
        "POST" if request_path == AGENT_TURN_POLL_API_PATH => {
            serve_agent_turn_poll_api(&mut stream, &request, state)
        }
        _ => write_response(
            &mut stream,
            "405 Method Not Allowed",
            "text/plain; charset=utf-8",
            b"method not allowed",
            false,
        ),
    }
}

fn read_request(stream: &mut TcpStream) -> Result<HttpRequest, String> {
    let mut buffer = [0_u8; 1024];
    let mut request = Vec::new();

    loop {
        let read = stream
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);
        if find_header_end(&request).is_some() {
            break;
        }
        if request.len() >= MAX_REQUEST_HEADER_BYTES {
            return Err("request headers exceeded 16 KiB".to_owned());
        }
    }

    let header_end =
        find_header_end(&request).ok_or_else(|| "request headers were incomplete".to_owned())?;
    let header_text =
        std::str::from_utf8(&request[..header_end]).map_err(|error| error.to_string())?;
    let mut lines = header_text.lines();
    let request_line = lines
        .next()
        .ok_or_else(|| "missing HTTP request line".to_owned())?;
    let mut parts = request_line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| "missing HTTP method".to_owned())?
        .to_string();
    let target = parts
        .next()
        .ok_or_else(|| "missing HTTP target".to_owned())?
        .to_string();

    let content_length = lines
        .filter_map(|line| line.split_once(':'))
        .find_map(|(name, value)| {
            if name.trim().eq_ignore_ascii_case("content-length") {
                Some(
                    value
                        .trim()
                        .parse::<usize>()
                        .map_err(|_| "invalid Content-Length header".to_owned()),
                )
            } else {
                None
            }
        })
        .transpose()?
        .unwrap_or(0);

    if content_length > MAX_REQUEST_BODY_BYTES {
        return Err(format!(
            "request body exceeded {} bytes",
            MAX_REQUEST_BODY_BYTES
        ));
    }

    let body_start = header_end + 4;
    while request.len().saturating_sub(body_start) < content_length {
        let read = stream
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            return Err("request body ended before Content-Length bytes were read".to_owned());
        }
        request.extend_from_slice(&buffer[..read]);
        if request.len().saturating_sub(body_start) > MAX_REQUEST_BODY_BYTES {
            return Err(format!(
                "request body exceeded {} bytes",
                MAX_REQUEST_BODY_BYTES
            ));
        }
    }

    let body = request[body_start..body_start + content_length].to_vec();

    Ok(HttpRequest {
        method,
        target,
        body,
    })
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

fn request_path_only(target: &str) -> &str {
    target.split('?').next().unwrap_or("/")
}

fn serve_resources_api(
    stream: &mut TcpStream,
    head_only: bool,
    state: &ServerState,
) -> Result<(), String> {
    let payload = WebResourceCatalog {
        resources: available_server_resources(&state.ifc_artifacts_root),
    };
    if head_only {
        return write_response(
            stream,
            "200 OK",
            "application/json; charset=utf-8",
            b"",
            true,
        );
    }
    write_json_response(stream, "200 OK", &payload)
}

fn serve_package_api(
    stream: &mut TcpStream,
    request: &HttpRequest,
    state: &ServerState,
) -> Result<(), String> {
    let request_started = Instant::now();
    let parse_started = Instant::now();
    let api_request: PackageApiRequest = serde_json::from_slice(&request.body)
        .map_err(|error| format!("invalid /api/package JSON body: {error}"))?;
    let parse_ms = parse_started.elapsed().as_millis();

    let load_started = Instant::now();
    match load_package_response(&api_request.resource, &state.ifc_artifacts_root) {
        Ok((response, metrics)) => {
            let load_ms = load_started.elapsed().as_millis();
            let write_started = Instant::now();
            let write_result = write_json_response(stream, "200 OK", &response);
            let write_ms = write_started.elapsed().as_millis();
            println!(
                "[w web timing] package resource={} kind={} cache_status={} parse_ms={} load_ms={} write_ms={} total_ms={} definitions={} elements={} instances={}",
                api_request.resource,
                metrics.kind,
                metrics.cache_status.unwrap_or("-"),
                parse_ms,
                load_ms,
                write_ms,
                request_started.elapsed().as_millis(),
                metrics.definitions,
                metrics.elements,
                metrics.instances,
            );
            write_result
        }
        Err(error) => {
            let load_ms = load_started.elapsed().as_millis();
            let write_started = Instant::now();
            let write_result = write_json_error(stream, "400 Bad Request", &error);
            let write_ms = write_started.elapsed().as_millis();
            console_log(
                ConsoleLogKind::Error,
                format!(
                    "package resource={} parse_ms={} load_ms={} write_ms={} total_ms={} error={}",
                    api_request.resource,
                    parse_ms,
                    load_ms,
                    write_ms,
                    request_started.elapsed().as_millis(),
                    error,
                ),
            );
            write_result
        }
    }
}

fn serve_cypher_api(
    stream: &mut TcpStream,
    request: &HttpRequest,
    state: &ServerState,
) -> Result<(), String> {
    let request_started = Instant::now();
    let parse_started = Instant::now();
    let api_request: CypherApiRequest = serde_json::from_slice(&request.body)
        .map_err(|error| format!("invalid /api/cypher JSON body: {error}"))?;
    let parse_ms = parse_started.elapsed().as_millis();
    let query_preview = summarize_query_for_log(&api_request.cypher);

    let execute_started = Instant::now();
    match execute_cypher_api(&api_request, state) {
        Ok((response, metrics)) => {
            let execute_ms = execute_started.elapsed().as_millis();
            let write_started = Instant::now();
            let write_result = write_json_response(stream, "200 OK", &response);
            let write_ms = write_started.elapsed().as_millis();
            println!(
                "[w web timing] cypher resource={} model={} model_cache={} parse_ms={} open_ms={} query_ms={} ids_ms={} exec_ms={} write_ms={} total_ms={} cols={} rows={} semantic_ids={} query=\"{}\"",
                api_request.resource,
                metrics.model_slug,
                metrics.model_cache_status,
                parse_ms,
                metrics.open_ms,
                metrics.query_ms,
                metrics.extract_ids_ms,
                execute_ms,
                write_ms,
                request_started.elapsed().as_millis(),
                metrics.columns,
                metrics.rows,
                metrics.semantic_element_ids,
                query_preview,
            );
            write_result
        }
        Err(error) => {
            let execute_ms = execute_started.elapsed().as_millis();
            let write_started = Instant::now();
            let write_result = write_json_error(stream, "400 Bad Request", &error);
            let write_ms = write_started.elapsed().as_millis();
            console_log(
                ConsoleLogKind::Error,
                format!(
                    "cypher resource={} parse_ms={} exec_ms={} write_ms={} total_ms={} query=\"{}\" error={}",
                    api_request.resource,
                    parse_ms,
                    execute_ms,
                    write_ms,
                    request_started.elapsed().as_millis(),
                    query_preview,
                    error,
                ),
            );
            write_result
        }
    }
}

fn serve_graph_subgraph_api(
    stream: &mut TcpStream,
    request: &HttpRequest,
    state: &ServerState,
) -> Result<(), String> {
    let request_started = Instant::now();
    let parse_started = Instant::now();
    let api_request: GraphSubgraphApiRequest = serde_json::from_slice(&request.body)
        .map_err(|error| format!("invalid /api/graph/subgraph JSON body: {error}"))?;
    let parse_ms = parse_started.elapsed().as_millis();
    let seed_count = api_request.seed_node_ids.len();

    let execute_started = Instant::now();
    match execute_graph_subgraph_api(&api_request, state) {
        Ok((response, metrics)) => {
            let execute_ms = execute_started.elapsed().as_millis();
            let write_started = Instant::now();
            let write_result = write_json_response(stream, "200 OK", &response);
            let write_ms = write_started.elapsed().as_millis();
            println!(
                "[w web timing] graph_subgraph resource={} model={} model_cache={} parse_ms={} open_ms={} build_ms={} exec_ms={} write_ms={} total_ms={} seeds={} hops={} max_nodes={} max_edges={} nodes={} edges={} truncated={} mode={:?}",
                api_request.resource,
                metrics.model_slug,
                metrics.model_cache_status,
                parse_ms,
                metrics.open_ms,
                metrics.build_ms,
                execute_ms,
                write_ms,
                request_started.elapsed().as_millis(),
                seed_count,
                response.hops,
                response.max_nodes,
                response.max_edges,
                metrics.nodes,
                metrics.edges,
                metrics.truncated,
                response.mode,
            );
            write_result
        }
        Err(error) => {
            let execute_ms = execute_started.elapsed().as_millis();
            let write_started = Instant::now();
            let write_result = write_json_error(stream, "400 Bad Request", &error);
            let write_ms = write_started.elapsed().as_millis();
            console_log(
                ConsoleLogKind::Error,
                format!(
                    "graph_subgraph resource={} parse_ms={} exec_ms={} write_ms={} total_ms={} seeds={} error={}",
                    api_request.resource,
                    parse_ms,
                    execute_ms,
                    write_ms,
                    request_started.elapsed().as_millis(),
                    seed_count,
                    error,
                ),
            );
            write_result
        }
    }
}

fn serve_graph_node_properties_api(
    stream: &mut TcpStream,
    request: &HttpRequest,
    state: &ServerState,
) -> Result<(), String> {
    let request_started = Instant::now();
    let parse_started = Instant::now();
    let api_request: GraphNodePropertiesApiRequest = serde_json::from_slice(&request.body)
        .map_err(|error| format!("invalid /api/graph/node-properties JSON body: {error}"))?;
    let parse_ms = parse_started.elapsed().as_millis();

    let execute_started = Instant::now();
    match execute_graph_node_properties_api(&api_request, state) {
        Ok(response) => {
            let execute_ms = execute_started.elapsed().as_millis();
            let write_started = Instant::now();
            let write_result = write_json_response(stream, "200 OK", &response);
            let write_ms = write_started.elapsed().as_millis();
            println!(
                "[w web timing] graph_node_properties resource={} parse_ms={} exec_ms={} write_ms={} total_ms={} db_node_id={} properties={} relations={} truncated_relations={}",
                api_request.resource,
                parse_ms,
                execute_ms,
                write_ms,
                request_started.elapsed().as_millis(),
                api_request.db_node_id,
                response.properties.len(),
                response.relations.len(),
                response.truncated_relations,
            );
            write_result
        }
        Err(error) => {
            let execute_ms = execute_started.elapsed().as_millis();
            let write_started = Instant::now();
            let write_result = write_json_error(stream, "400 Bad Request", &error);
            let write_ms = write_started.elapsed().as_millis();
            console_log(
                ConsoleLogKind::Error,
                format!(
                    "graph_node_properties resource={} parse_ms={} exec_ms={} write_ms={} total_ms={} db_node_id={} error={}",
                    api_request.resource,
                    parse_ms,
                    execute_ms,
                    write_ms,
                    request_started.elapsed().as_millis(),
                    api_request.db_node_id,
                    error,
                ),
            );
            write_result
        }
    }
}

fn serve_agent_session_api(
    stream: &mut TcpStream,
    request: &HttpRequest,
    state: &ServerState,
) -> Result<(), String> {
    let request_started = Instant::now();
    let parse_started = Instant::now();
    let api_request: AgentSessionApiRequest = serde_json::from_slice(&request.body)
        .map_err(|error| format!("invalid /api/agent/session JSON body: {error}"))?;
    let parse_ms = parse_started.elapsed().as_millis();

    let execute_started = Instant::now();
    match create_agent_session_api(&api_request, state) {
        Ok(response) => {
            let execute_ms = execute_started.elapsed().as_millis();
            let write_started = Instant::now();
            let write_result = write_json_response(stream, "200 OK", &response);
            let write_ms = write_started.elapsed().as_millis();
            println!(
                "[w web timing] agent_session resource={} parse_ms={} exec_ms={} write_ms={} total_ms={} session_id={}",
                response.resource,
                parse_ms,
                execute_ms,
                write_ms,
                request_started.elapsed().as_millis(),
                response.session_id,
            );
            write_result
        }
        Err(error) => {
            let execute_ms = execute_started.elapsed().as_millis();
            let write_started = Instant::now();
            let write_result = write_json_error(stream, "400 Bad Request", &error);
            let write_ms = write_started.elapsed().as_millis();
            console_log(
                ConsoleLogKind::Error,
                format!(
                    "agent_session resource={} parse_ms={} exec_ms={} write_ms={} total_ms={} error={}",
                    api_request.resource,
                    parse_ms,
                    execute_ms,
                    write_ms,
                    request_started.elapsed().as_millis(),
                    error,
                ),
            );
            write_result
        }
    }
}

fn serve_agent_capabilities_api(
    stream: &mut TcpStream,
    head_only: bool,
    state: &ServerState,
) -> Result<(), String> {
    let response = agent_capabilities_response(&state.agent_runtime);
    if head_only {
        return write_response(
            stream,
            "200 OK",
            "application/json; charset=utf-8",
            b"",
            true,
        );
    }
    write_json_response(stream, "200 OK", &response)
}

fn serve_agent_turn_api(
    stream: &mut TcpStream,
    request: &HttpRequest,
    state: &ServerState,
) -> Result<(), String> {
    let request_started = Instant::now();
    let parse_started = Instant::now();
    let api_request: AgentTurnApiRequest = serde_json::from_slice(&request.body)
        .map_err(|error| format!("invalid /api/agent/turn JSON body: {error}"))?;
    let parse_ms = parse_started.elapsed().as_millis();
    let input_preview = summarize_query_for_log(&api_request.input);

    let execute_started = Instant::now();
    let mut progress = NullAgentProgressSink;
    match execute_agent_turn_api(&api_request, state, &mut progress) {
        Ok(response) => {
            let execute_ms = execute_started.elapsed().as_millis();
            let write_started = Instant::now();
            let write_result = write_json_response(stream, "200 OK", &response);
            let write_ms = write_started.elapsed().as_millis();
            println!(
                "[w web timing] agent_turn session_id={} resource={} parse_ms={} exec_ms={} write_ms={} total_ms={} transcript_events={} actions={} input=\"{}\"",
                response.session_id,
                response.resource,
                parse_ms,
                execute_ms,
                write_ms,
                request_started.elapsed().as_millis(),
                response.transcript.len(),
                response.actions.len(),
                input_preview,
            );
            write_result
        }
        Err(error) => {
            let execute_ms = execute_started.elapsed().as_millis();
            let write_started = Instant::now();
            let write_result = write_json_error(stream, "400 Bad Request", &error);
            let write_ms = write_started.elapsed().as_millis();
            console_log(
                ConsoleLogKind::Error,
                format!(
                    "agent_turn session_id={} parse_ms={} exec_ms={} write_ms={} total_ms={} input=\"{}\" error={}",
                    api_request.session_id,
                    parse_ms,
                    execute_ms,
                    write_ms,
                    request_started.elapsed().as_millis(),
                    input_preview,
                    error,
                ),
            );
            write_result
        }
    }
}

fn serve_agent_turn_start_api(
    stream: &mut TcpStream,
    request: &HttpRequest,
    state: Arc<ServerState>,
) -> Result<(), String> {
    let request_started = Instant::now();
    let parse_started = Instant::now();
    let api_request: AgentTurnStartApiRequest = serde_json::from_slice(&request.body)
        .map_err(|error| format!("invalid /api/agent/turn-start JSON body: {error}"))?;
    let parse_ms = parse_started.elapsed().as_millis();
    let input_preview = summarize_query_for_log(&api_request.input);

    let execute_started = Instant::now();
    match start_agent_turn_api(&api_request, state) {
        Ok(response) => {
            let execute_ms = execute_started.elapsed().as_millis();
            let write_started = Instant::now();
            let write_result = write_json_response(stream, "200 OK", &response);
            let write_ms = write_started.elapsed().as_millis();
            println!(
                "[w web timing] agent_turn_start turn_id={} session_id={} resource={} backend={} model={} level={} parse_ms={} exec_ms={} write_ms={} total_ms={} input=\"{}\"",
                response.turn_id,
                response.session_id,
                response.resource,
                response.backend_id,
                response.model_id.as_deref().unwrap_or("-"),
                response.level_id.as_deref().unwrap_or("-"),
                parse_ms,
                execute_ms,
                write_ms,
                request_started.elapsed().as_millis(),
                input_preview,
            );
            write_result
        }
        Err(error) => {
            let execute_ms = execute_started.elapsed().as_millis();
            let write_started = Instant::now();
            let write_result = write_json_error(stream, "400 Bad Request", &error);
            let write_ms = write_started.elapsed().as_millis();
            console_log(
                ConsoleLogKind::Error,
                format!(
                    "agent_turn_start session_id={} parse_ms={} exec_ms={} write_ms={} total_ms={} input=\"{}\" error={}",
                    api_request.session_id,
                    parse_ms,
                    execute_ms,
                    write_ms,
                    request_started.elapsed().as_millis(),
                    input_preview,
                    error,
                ),
            );
            write_result
        }
    }
}

fn serve_agent_turn_poll_api(
    stream: &mut TcpStream,
    request: &HttpRequest,
    state: &ServerState,
) -> Result<(), String> {
    let request_started = Instant::now();
    let parse_started = Instant::now();
    let api_request: AgentTurnPollApiRequest = serde_json::from_slice(&request.body)
        .map_err(|error| format!("invalid /api/agent/turn-poll JSON body: {error}"))?;
    let parse_ms = parse_started.elapsed().as_millis();

    let execute_started = Instant::now();
    match poll_agent_turn_api(&api_request, state) {
        Ok(response) => write_json_response(stream, "200 OK", &response),
        Err(error) => {
            let execute_ms = execute_started.elapsed().as_millis();
            let write_started = Instant::now();
            let write_result = write_json_error(stream, "400 Bad Request", &error);
            let write_ms = write_started.elapsed().as_millis();
            console_log(
                ConsoleLogKind::Error,
                format!(
                    "agent_turn_poll turn_id={} parse_ms={} exec_ms={} write_ms={} total_ms={} error={}",
                    api_request.turn_id,
                    parse_ms,
                    execute_ms,
                    write_ms,
                    request_started.elapsed().as_millis(),
                    error,
                ),
            );
            write_result
        }
    }
}

fn load_package_response(
    resource: &str,
    ifc_artifacts_root: &Path,
) -> Result<(WebPreparedPackageResponse, PackageApiMetrics), String> {
    let (package, metrics) = if let Some(model_slug) = parse_ifc_body_resource(resource) {
        let load = VelrIfcModel::load_body_package_with_cache_status_from_artifacts_root(
            ifc_artifacts_root,
            model_slug,
        )
        .map_err(|error| format!("failed to load IFC package `{resource}`: {error}"))?;
        let summary = load.geometry_summary();
        (
            load.package,
            PackageApiMetrics {
                kind: "ifc",
                cache_status: Some(load.cache_status.as_str()),
                definitions: summary.definitions,
                elements: summary.elements,
                instances: summary.instances,
            },
        )
    } else {
        let package = GeometryBackend::default()
            .build_demo_package_for(resource)
            .map_err(|error| format!("failed to load demo package `{resource}`: {error}"))?;
        let metrics = PackageApiMetrics {
            kind: "demo",
            cache_status: None,
            definitions: package.definitions.len(),
            elements: package.elements.len(),
            instances: package.instances.len(),
        };
        (package, metrics)
    };

    Ok((
        WebPreparedPackageResponse {
            resource: resource.to_string(),
            package: WebPreparedGeometryPackage::from_prepared_package(&package),
        },
        metrics,
    ))
}

fn available_server_resources(ifc_artifacts_root: &Path) -> Vec<String> {
    let mut resources = available_demo_resources()
        .into_iter()
        .map(|resource| resource.to_string())
        .collect::<Vec<_>>();
    if let Ok(mut ifc_resources) = available_ifc_body_resources(ifc_artifacts_root) {
        resources.append(&mut ifc_resources);
    }
    resources.sort();
    resources
}

fn execute_cypher_api(
    request: &CypherApiRequest,
    state: &ServerState,
) -> Result<(CypherApiResponse, CypherApiMetrics), String> {
    if request.cypher.trim().is_empty() {
        return Err("cypher query must not be empty".to_owned());
    }

    let model_slug = parse_ifc_body_resource(&request.resource).ok_or_else(|| {
        format!(
            "cypher queries require an IFC resource like `ifc/building-architecture`; got `{}`",
            request.resource
        )
    })?;
    let open_started = Instant::now();
    let (model, cache_status) = cached_ifc_model(state, model_slug)?;
    let open_ms = open_started.elapsed().as_millis();
    let query_started = Instant::now();
    let query_result = model
        .execute_cypher_rows(&request.cypher)
        .map_err(|error| format!("cypher execution failed for `{model_slug}`: {error}"))?;
    let query_ms = query_started.elapsed().as_millis();
    let extract_started = Instant::now();
    let semantic_element_ids =
        extract_semantic_element_ids(&query_result.columns, &query_result.rows);
    let extract_ids_ms = extract_started.elapsed().as_millis();
    let metrics = CypherApiMetrics {
        model_slug: model_slug.to_owned(),
        model_cache_status: cache_status.as_str(),
        open_ms,
        query_ms,
        extract_ids_ms,
        columns: query_result.columns.len(),
        rows: query_result.rows.len(),
        semantic_element_ids: semantic_element_ids.len(),
    };

    Ok((
        CypherApiResponse {
            resource: request.resource.clone(),
            columns: query_result.columns,
            rows: query_result.rows,
            semantic_element_ids,
        },
        metrics,
    ))
}

fn execute_graph_subgraph_api(
    request: &GraphSubgraphApiRequest,
    state: &ServerState,
) -> Result<(GraphSubgraphApiResponse, GraphSubgraphApiMetrics), String> {
    let model_slug = parse_ifc_body_resource(&request.resource).ok_or_else(|| {
        format!(
            "graph exploration requires an IFC resource like `ifc/building-architecture`; got `{}`",
            request.resource
        )
    })?;
    let limits = validate_graph_subgraph_request(request)?;
    let open_started = Instant::now();
    let (model, cache_status) = cached_ifc_model(state, model_slug)?;
    let open_ms = open_started.elapsed().as_millis();
    let build_started = Instant::now();
    let response = build_graph_subgraph_response(request, model.as_ref(), limits)?;
    let build_ms = build_started.elapsed().as_millis();
    let metrics = GraphSubgraphApiMetrics {
        model_slug: model_slug.to_owned(),
        model_cache_status: cache_status.as_str(),
        open_ms,
        build_ms,
        nodes: response.nodes.len(),
        edges: response.edges.len(),
        truncated: response.truncated,
    };
    Ok((response, metrics))
}

fn execute_graph_node_properties_api(
    request: &GraphNodePropertiesApiRequest,
    state: &ServerState,
) -> Result<GraphNodePropertiesApiResponse, String> {
    let model_slug = parse_ifc_body_resource(&request.resource).ok_or_else(|| {
        format!(
            "node property inspection requires an IFC resource like `ifc/building-architecture`; got `{}`",
            request.resource
        )
    })?;
    let max_relations = validate_graph_limit(
        request.max_relations,
        DEFAULT_GRAPH_NODE_PROPERTIES_MAX_RELATIONS,
        MAX_GRAPH_NODE_PROPERTIES_MAX_RELATIONS,
        "maxRelations",
    )?;
    let (model, _) = cached_ifc_model(state, model_slug)?;
    let details = fetch_agent_node_properties(model.as_ref(), request.db_node_id, max_relations)?;
    Ok(GraphNodePropertiesApiResponse {
        resource: request.resource.clone(),
        node: details.node,
        properties: details.properties,
        relations: details.relations,
        truncated_relations: details.truncated_relations,
    })
}

fn create_agent_session_api(
    request: &AgentSessionApiRequest,
    state: &ServerState,
) -> Result<AgentSessionApiResponse, String> {
    validate_agent_resource(&request.resource)?;
    let schema = resolve_agent_resource_schema(&request.resource, &state.ifc_artifacts_root)?;
    let schema_id = schema.canonical_name().to_owned();
    let schema_slug = schema.generated_artifact_stem().map(ToOwned::to_owned);
    let opencode_session_id = match &state.agent_runtime.native_server {
        Some(native_server) => Some(
            native_server
                .create_session(&format!("ccw {}", request.resource))
                .map_err(|error| format!("could not create opencode session: {error}"))?,
        ),
        None => None,
    };
    let mut store = state
        .agent_sessions
        .lock()
        .map_err(|_| "agent session store lock poisoned".to_owned())?;
    store.next_session_number = store.next_session_number.saturating_add(1);
    let session_id = format!("agent-session-{}", store.next_session_number);
    let transcript = vec![AgentTranscriptEvent::system(format!(
        "AI session bound to {} ({}).",
        request.resource, schema_id
    ))];
    store.sessions.insert(
        session_id.clone(),
        AgentSession {
            session_id: session_id.clone(),
            resource: request.resource.clone(),
            schema_id: schema_id.clone(),
            schema_slug: schema_slug.clone(),
            opencode_session_id,
            turn_count: 0,
            transcript: transcript.clone(),
        },
    );
    Ok(AgentSessionApiResponse {
        session_id,
        resource: request.resource.clone(),
        schema_id,
        schema_slug,
        transcript,
    })
}

fn execute_agent_turn_api(
    request: &AgentTurnApiRequest,
    state: &ServerState,
    progress: &mut dyn BackendAgentProgressSink,
) -> Result<AgentTurnApiResponse, String> {
    if request.input.trim().is_empty() {
        return Err("agent input must not be empty".to_owned());
    }

    let session = {
        let store = state
            .agent_sessions
            .lock()
            .map_err(|_| "agent session store lock poisoned".to_owned())?;
        store
            .sessions
            .get(&request.session_id)
            .cloned()
            .ok_or_else(|| format!("unknown agent session `{}`", request.session_id))?
    };

    let session_history = backend_agent_session_history(&session);
    let execution = execute_agent_backend_turn(
        &session.resource,
        &session.schema_id,
        session.schema_slug.as_deref(),
        &request.input,
        request.backend_id.as_deref(),
        request.model_id.as_deref(),
        request.level_id.as_deref(),
        session.opencode_session_id.as_deref(),
        state.agent_runtime.native_server.clone(),
        &session_history,
        state,
        progress,
    )
    .map_err(|error| {
        format_and_log_agent_turn_error(
            None,
            &session,
            request.input.trim(),
            &error,
            &state.agent_runtime,
        )
    })?;
    finalize_agent_turn_response(
        &session,
        request.input.trim(),
        execution,
        state.agent_sessions.as_ref(),
        progress,
    )
}

fn execute_agent_turn_api_in_worker(
    request: &AgentTurnApiRequest,
    turn_id: &str,
    state: &AgentWorkerState,
    progress: &mut dyn BackendAgentProgressSink,
) -> Result<AgentTurnApiResponse, String> {
    if request.input.trim().is_empty() {
        return Err("agent input must not be empty".to_owned());
    }

    let session = {
        let store = state
            .agent_sessions
            .lock()
            .map_err(|_| "agent session store lock poisoned".to_owned())?;
        store
            .sessions
            .get(&request.session_id)
            .cloned()
            .ok_or_else(|| format!("unknown agent session `{}`", request.session_id))?
    };

    let session_history = backend_agent_session_history(&session);
    let execution = execute_agent_backend_turn_in_worker(
        &session.resource,
        &session.schema_id,
        session.schema_slug.as_deref(),
        &request.input,
        request.backend_id.as_deref(),
        request.model_id.as_deref(),
        request.level_id.as_deref(),
        session.opencode_session_id.as_deref(),
        state.agent_runtime.native_server.clone(),
        &session_history,
        state,
        progress,
    )
    .map_err(|error| {
        format_and_log_agent_turn_error(
            Some(turn_id),
            &session,
            request.input.trim(),
            &error,
            &state.agent_runtime,
        )
    })?;
    finalize_agent_turn_response(
        &session,
        request.input.trim(),
        execution,
        state.agent_sessions.as_ref(),
        progress,
    )
}

fn start_agent_turn_api(
    request: &AgentTurnStartApiRequest,
    state: Arc<ServerState>,
) -> Result<AgentTurnStartApiResponse, String> {
    if request.input.trim().is_empty() {
        return Err("agent input must not be empty".to_owned());
    }

    let session = {
        let store = state
            .agent_sessions
            .lock()
            .map_err(|_| "agent session store lock poisoned".to_owned())?;
        store
            .sessions
            .get(&request.session_id)
            .cloned()
            .ok_or_else(|| format!("unknown agent session `{}`", request.session_id))?
    };
    let selected_backend_id = request
        .backend_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| state.agent_runtime.backend.id())
        .to_owned();
    let selected_model_id = selected_agent_option_id(
        request.model_id.as_deref(),
        state.agent_runtime.default_model_id.as_deref(),
        &state.agent_runtime.models,
        "model",
    )?
    .map(ToOwned::to_owned);
    let selected_level_id = selected_agent_option_id(
        request.level_id.as_deref(),
        default_level_for_model(&state.agent_runtime, selected_model_id.as_deref()),
        agent_levels_for_model(&state.agent_runtime, selected_model_id.as_deref()),
        "level",
    )?
    .map(ToOwned::to_owned);
    let _ = resolve_agent_backend_for_turn(
        &state.agent_runtime,
        Some(&selected_backend_id),
        selected_model_id.as_deref(),
        selected_level_id.as_deref(),
    )?;

    let turn_id = {
        let mut store = state
            .agent_turns
            .lock()
            .map_err(|_| "agent turn store lock poisoned".to_owned())?;
        store.next_turn_number = store.next_turn_number.saturating_add(1);
        let turn_id = format!("agent-turn-{}", store.next_turn_number);
        store.turns.insert(
            turn_id.clone(),
            AgentTurnState {
                turn_id: turn_id.clone(),
                session_id: session.session_id.clone(),
                resource: session.resource.clone(),
                schema_id: session.schema_id.clone(),
                schema_slug: session.schema_slug.clone(),
                next_seq: 1,
                events: Vec::new(),
                done: false,
                result: None,
                error: None,
            },
        );
        turn_id
    };

    let input = request.input.trim().to_owned();
    let session_id = session.session_id.clone();
    let resource = session.resource.clone();
    let schema_id = session.schema_id.clone();
    let schema_slug = session.schema_slug.clone();
    let session_id_for_thread = session_id.clone();
    let backend_id_for_thread = Some(selected_backend_id.clone());
    let model_id_for_thread = selected_model_id.clone();
    let level_id_for_thread = selected_level_id.clone();
    let worker_state = AgentWorkerState {
        ifc_artifacts_root: state.ifc_artifacts_root.clone(),
        agent_sessions: Arc::clone(&state.agent_sessions),
        agent_turns: Arc::clone(&state.agent_turns),
        agent_runtime: state.agent_runtime.clone(),
        native_server: state.agent_runtime.native_server.clone(),
    };
    let turn_id_for_thread = turn_id.clone();
    thread::spawn(move || {
        let mut progress = AgentTurnProgressSinkImpl {
            agent_turns: Arc::clone(&worker_state.agent_turns),
            turn_id: turn_id_for_thread.clone(),
        };
        progress.emit(BackendAgentTranscriptEvent::system(
            agent_turn_selection_summary(
                model_id_for_thread.as_deref(),
                level_id_for_thread.as_deref(),
            ),
        ));
        progress.emit(BackendAgentTranscriptEvent::system(
            "Thinking about the request.".to_owned(),
        ));
        let request = AgentTurnApiRequest {
            session_id: session_id_for_thread.clone(),
            input: input.clone(),
            backend_id: backend_id_for_thread.clone(),
            model_id: model_id_for_thread.clone(),
            level_id: level_id_for_thread.clone(),
        };
        let result = execute_agent_turn_api_in_worker(
            &request,
            &turn_id_for_thread,
            &worker_state,
            &mut progress,
        );
        match result {
            Ok(response) => finish_agent_turn_state_success(
                worker_state.agent_turns.as_ref(),
                &turn_id_for_thread,
                response,
            ),
            Err(error) => finish_agent_turn_state_error(
                worker_state.agent_turns.as_ref(),
                &turn_id_for_thread,
                error,
            ),
        }
    });

    Ok(AgentTurnStartApiResponse {
        turn_id,
        session_id,
        resource,
        schema_id,
        schema_slug,
        backend_id: selected_backend_id,
        model_id: selected_model_id,
        level_id: selected_level_id,
    })
}

fn poll_agent_turn_api(
    request: &AgentTurnPollApiRequest,
    state: &ServerState,
) -> Result<AgentTurnPollApiResponse, String> {
    let after_seq = request.after_seq.unwrap_or(0);
    let store = state
        .agent_turns
        .lock()
        .map_err(|_| "agent turn store lock poisoned".to_owned())?;
    let turn = store
        .turns
        .get(&request.turn_id)
        .ok_or_else(|| format!("unknown agent turn `{}`", request.turn_id))?;

    let events = turn
        .events
        .iter()
        .filter(|event| event.seq > after_seq)
        .cloned()
        .collect();

    Ok(AgentTurnPollApiResponse {
        turn_id: turn.turn_id.clone(),
        done: turn.done,
        events,
        result: turn.result.clone(),
        error: turn.error.clone(),
    })
}

fn finalize_agent_turn_response(
    session: &AgentSession,
    user_input: &str,
    execution: crate::agent_executor::AgentBackendTurnResponse,
    agent_sessions: &Mutex<AgentSessionStore>,
    progress: &mut dyn BackendAgentProgressSink,
) -> Result<AgentTurnApiResponse, String> {
    let actions =
        validate_backend_agent_action_candidates(execution.action_candidates).map(|actions| {
            actions
                .into_iter()
                .map(agent_ui_action_from_backend)
                .collect::<Vec<_>>()
        })?;
    let queries_executed = execution.queries_executed;

    let mut stored_transcript = Vec::with_capacity(2 + execution.transcript.len());
    stored_transcript.push(AgentTranscriptEvent::user(user_input.to_owned()));
    stored_transcript.extend(
        execution
            .transcript
            .into_iter()
            .map(agent_transcript_event_from_backend),
    );
    if !actions.is_empty() {
        let backend_event = BackendAgentTranscriptEvent::assistant(format!(
            "Prepared {} validated UI action{}.",
            actions.len(),
            if actions.len() == 1 { "" } else { "s" }
        ));
        progress.emit(backend_event.clone());
        let event = agent_transcript_event_from_backend(backend_event);
        stored_transcript.push(event);
    }
    if queries_executed > 0 {
        let backend_event = BackendAgentTranscriptEvent::system(format!(
            "Completed {} read-only Cypher quer{}.",
            queries_executed,
            if queries_executed == 1 { "y" } else { "ies" }
        ));
        progress.emit(backend_event.clone());
        let event = agent_transcript_event_from_backend(backend_event);
        stored_transcript.push(event);
    }
    if stored_transcript.len() > MAX_AGENT_TRANSCRIPT_EVENTS {
        stored_transcript.truncate(MAX_AGENT_TRANSCRIPT_EVENTS);
    }

    let transcript = summarize_agent_turn_response_transcript(&stored_transcript);

    {
        let mut store = agent_sessions
            .lock()
            .map_err(|_| "agent session store lock poisoned".to_owned())?;
        let Some(stored_session) = store.sessions.get_mut(&session.session_id) else {
            return Err(format!("unknown agent session `{}`", session.session_id));
        };
        stored_session.turn_count = stored_session.turn_count.saturating_add(1);
        stored_session
            .transcript
            .extend(stored_transcript.iter().cloned());
        if stored_session.transcript.len() > MAX_AGENT_TRANSCRIPT_EVENTS * 8 {
            let keep_from = stored_session
                .transcript
                .len()
                .saturating_sub(MAX_AGENT_TRANSCRIPT_EVENTS * 8);
            stored_session.transcript.drain(0..keep_from);
        }
    }

    Ok(AgentTurnApiResponse {
        session_id: session.session_id.clone(),
        resource: session.resource.clone(),
        schema_id: session.schema_id.clone(),
        schema_slug: session.schema_slug.clone(),
        transcript,
        actions,
    })
}

fn finish_agent_turn_state_success(
    agent_turns: &Mutex<AgentTurnStore>,
    turn_id: &str,
    response: AgentTurnApiResponse,
) {
    if let Ok(mut store) = agent_turns.lock() {
        if let Some(turn) = store.turns.get_mut(turn_id) {
            turn.done = true;
            turn.result = Some(response);
            turn.error = None;
        }
    }
}

fn finish_agent_turn_state_error(
    agent_turns: &Mutex<AgentTurnStore>,
    turn_id: &str,
    error: String,
) {
    if let Ok(mut store) = agent_turns.lock() {
        if let Some(turn) = store.turns.get_mut(turn_id) {
            turn.done = true;
            turn.error = Some(error);
        }
    }
}

struct AgentTurnProgressSinkImpl {
    agent_turns: Arc<Mutex<AgentTurnStore>>,
    turn_id: String,
}

impl BackendAgentProgressSink for AgentTurnProgressSinkImpl {
    fn emit(&mut self, event: BackendAgentTranscriptEvent) {
        if let Ok(mut store) = self.agent_turns.lock() {
            if let Some(turn) = store.turns.get_mut(&self.turn_id) {
                let seq = turn.next_seq;
                turn.next_seq = turn.next_seq.saturating_add(1);
                turn.events.push(AgentTurnProgressEvent {
                    seq,
                    item: agent_transcript_event_from_backend(event),
                });
            }
        }
    }
}

fn validate_agent_resource(resource: &str) -> Result<&str, String> {
    parse_ifc_body_resource(resource).ok_or_else(|| {
        format!(
            "agent sessions require an IFC resource like `ifc/building-architecture`; got `{resource}`"
        )
    })
}

fn resolve_agent_resource_schema(
    resource: &str,
    ifc_artifacts_root: &Path,
) -> Result<IfcSchemaId, String> {
    let model_slug = validate_agent_resource(resource)?;
    let layout = IfcArtifactLayout::new(ifc_artifacts_root, model_slug);
    layout
        .authoritative_schema()
        .map_err(|error| format!("could not determine IFC schema for `{resource}`: {error}"))
}

fn execute_agent_backend_turn(
    resource: &str,
    schema_id: &str,
    schema_slug: Option<&str>,
    input: &str,
    backend_id: Option<&str>,
    model_id: Option<&str>,
    level_id: Option<&str>,
    native_session_id: Option<&str>,
    native_server: Option<Arc<OpencodeNativeServer>>,
    session_history: &[BackendAgentTranscriptEvent],
    state: &ServerState,
    progress: &mut dyn BackendAgentProgressSink,
) -> Result<crate::agent_executor::AgentBackendTurnResponse, String> {
    let request = BackendAgentTurnRequest {
        resource: resource.to_owned(),
        schema_id: schema_id.to_owned(),
        schema_slug: schema_slug.map(ToOwned::to_owned),
        input: input.to_owned(),
        session_history: session_history.to_vec(),
    };
    let mut runtime = BoundedReadonlyCypherRuntime::new(resource, schema_id, input, state);
    let selected_backend =
        resolve_agent_backend_for_turn(&state.agent_runtime, backend_id, model_id, level_id)?;

    let response = match selected_backend {
        AgentBackend::Stub => {
            let mut executor = StubAgentExecutor;
            executor.execute_turn(&request, &mut runtime, progress)
        }
        AgentBackend::Opencode(config) => {
            let native_server = native_server
                .or_else(|| state.agent_runtime.native_server.clone())
                .ok_or_else(|| "native opencode server is not available".to_owned())?;
            let native_session_id = native_session_id.ok_or_else(|| {
                "native opencode session is not available for this turn".to_owned()
            })?;
            let mut executor = OpencodeExecutor::with_native_server(
                config,
                native_server,
                Some(native_session_id.to_owned()),
            );
            executor.execute_turn(&request, &mut runtime, progress)
        }
    }?;

    Ok(crate::agent_executor::AgentBackendTurnResponse {
        queries_executed: runtime.queries_executed.max(response.queries_executed),
        ..response
    })
}

fn resolve_agent_backend_for_turn(
    runtime: &AgentRuntimeConfig,
    backend_id: Option<&str>,
    model_id: Option<&str>,
    level_id: Option<&str>,
) -> Result<AgentBackend, String> {
    let selected_backend_id = backend_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| runtime.backend.id());
    if selected_backend_id != runtime.backend.id() {
        return Err(format!(
            "agent backend `{selected_backend_id}` is not available; available backend is `{}`",
            runtime.backend.id()
        ));
    }

    let selected_model_id = selected_agent_option_id(
        model_id,
        runtime.default_model_id.as_deref(),
        &runtime.models,
        "model",
    )?;
    let selected_level_id = selected_agent_option_id(
        level_id,
        default_level_for_model(runtime, selected_model_id),
        agent_levels_for_model(runtime, selected_model_id),
        "level",
    )?;

    match &runtime.backend {
        AgentBackend::Stub => Ok(AgentBackend::Stub),
        AgentBackend::Opencode(config) => {
            let mut selected = config.clone();
            selected.model = selected_model_id
                .filter(|value| *value != "default")
                .map(ToOwned::to_owned);
            selected.variant = selected_level_id
                .filter(|value| *value != "default" && *value != "standard")
                .map(ToOwned::to_owned);
            Ok(AgentBackend::Opencode(selected))
        }
    }
}

fn agent_levels_for_model<'a>(
    runtime: &'a AgentRuntimeConfig,
    model_id: Option<&str>,
) -> &'a [AgentCapabilityOption] {
    model_id
        .and_then(|model_id| runtime.levels_by_model.get(model_id))
        .map(Vec::as_slice)
        .filter(|levels| !levels.is_empty())
        .unwrap_or(runtime.levels.as_slice())
}

fn default_level_for_model<'a>(
    runtime: &'a AgentRuntimeConfig,
    model_id: Option<&str>,
) -> Option<&'a str> {
    let levels = agent_levels_for_model(runtime, model_id);
    runtime
        .default_level_id
        .as_deref()
        .filter(|default_id| levels.iter().any(|level| level.id == *default_id))
        .or_else(|| middle_level_id(levels))
}

fn agent_turn_selection_summary(model_id: Option<&str>, level_id: Option<&str>) -> String {
    match (model_id, level_id) {
        (Some(model_id), Some(level_id)) => format!("Using {model_id} / {level_id}."),
        (Some(model_id), None) => format!("Using {model_id}."),
        _ => "Using selected AI settings.".to_owned(),
    }
}

fn middle_level_id(levels: &[AgentCapabilityOption]) -> Option<&str> {
    levels
        .get(levels.len().saturating_sub(1) / 2)
        .map(|level| level.id.as_str())
}

fn selected_agent_option_id<'a>(
    requested: Option<&'a str>,
    default_id: Option<&'a str>,
    available: &'a [AgentCapabilityOption],
    kind: &str,
) -> Result<Option<&'a str>, String> {
    let selected = requested
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or(default_id);
    let Some(selected) = selected else {
        return Ok(None);
    };
    if available.iter().any(|option| option.id == selected) {
        Ok(Some(selected))
    } else {
        let available_ids = available
            .iter()
            .map(|option| option.id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        Err(format!(
            "agent {kind} `{selected}` is not available; available {kind}s: {available_ids}"
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentBackendErrorCategory {
    Timeout,
    ProviderServer,
    ProviderAuthentication,
    ProviderRateLimit,
    ProviderOther,
    BackendOther,
}

impl AgentBackendErrorCategory {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::ProviderServer => "provider_server",
            Self::ProviderAuthentication => "provider_authentication",
            Self::ProviderRateLimit => "provider_rate_limit",
            Self::ProviderOther => "provider_other",
            Self::BackendOther => "backend_other",
        }
    }
}

fn format_and_log_agent_turn_error(
    turn_id: Option<&str>,
    session: &AgentSession,
    question: &str,
    internal_error: &str,
    runtime: &AgentRuntimeConfig,
) -> String {
    let error_id = generate_agent_error_id(turn_id, &session.session_id);
    let category = classify_agent_backend_error(internal_error);
    let user_message = format_user_facing_agent_error(&error_id, category, internal_error);
    let log_entry = AgentErrorLogEntry::new(
        &error_id,
        runtime.backend.label(),
        category.as_str(),
        &session.session_id,
        turn_id,
        &session.resource,
        &session.schema_id,
        question,
        &user_message,
        internal_error,
    );
    if let Err(log_error) = runtime.error_logger.append(&log_entry) {
        console_log(
            ConsoleLogKind::AgentError,
            format!(
                "error_id={} logging_failure={} internal_error={}",
                error_id, log_error, internal_error
            ),
        );
    }
    console_log(
        ConsoleLogKind::AgentError,
        format!(
            "error_id={} backend={} category={} session_id={} turn_id={} resource={} schema_id={} question=\"{}\" internal_error={}",
            error_id,
            runtime.backend.label(),
            category.as_str(),
            session.session_id,
            turn_id.unwrap_or("-"),
            session.resource,
            session.schema_id,
            summarize_query_for_log(question),
            internal_error,
        ),
    );
    user_message
}

fn generate_agent_error_id(turn_id: Option<&str>, session_id: &str) -> String {
    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0);
    let suffix = turn_id.unwrap_or(session_id).replace("agent-", "");
    format!("ae-{timestamp_ms}-{suffix}")
}

fn classify_agent_backend_error(error: &str) -> AgentBackendErrorCategory {
    let lowered = error.trim().to_ascii_lowercase();
    if lowered.contains("timed out after") {
        return AgentBackendErrorCategory::Timeout;
    }
    if lowered.contains("provider returned server_error")
        || lowered.contains("temporary")
        || lowered.contains("temporarily unavailable")
        || lowered.contains("overloaded")
    {
        return AgentBackendErrorCategory::ProviderServer;
    }
    if lowered.contains("rate limit") || lowered.contains("too many requests") {
        return AgentBackendErrorCategory::ProviderRateLimit;
    }
    if lowered.contains("invalid_api_key")
        || lowered.contains("authentication")
        || lowered.contains("unauthorized")
        || lowered.contains("login")
    {
        return AgentBackendErrorCategory::ProviderAuthentication;
    }
    if lowered.contains("provider") || lowered.contains("opencode") {
        return AgentBackendErrorCategory::ProviderOther;
    }
    AgentBackendErrorCategory::BackendOther
}

fn format_user_facing_agent_error(
    error_id: &str,
    category: AgentBackendErrorCategory,
    internal_error: &str,
) -> String {
    match category {
        AgentBackendErrorCategory::Timeout => {
            let seconds = extract_timeout_seconds(internal_error)
                .map(format_seconds_for_user)
                .unwrap_or_else(|| "45 seconds".to_owned());
            format!(
                "The AI request timed out after {seconds} without progress. Please try again shortly. Error ID: {error_id}"
            )
        }
        AgentBackendErrorCategory::ProviderServer => format!(
            "The AI provider had a temporary server issue. Please try again shortly. Error ID: {error_id}"
        ),
        AgentBackendErrorCategory::ProviderRateLimit => format!(
            "The AI provider is busy right now. Please try again shortly. Error ID: {error_id}"
        ),
        AgentBackendErrorCategory::ProviderAuthentication => format!(
            "The AI provider authentication appears to be unavailable right now. Please re-authenticate and try again. Error ID: {error_id}"
        ),
        AgentBackendErrorCategory::ProviderOther | AgentBackendErrorCategory::BackendOther => {
            format!("The AI request failed. Please try again shortly. Error ID: {error_id}")
        }
    }
}

fn extract_timeout_seconds(error: &str) -> Option<f64> {
    let marker = "timed out after ";
    let start = error.find(marker)? + marker.len();
    let rest = &error[start..];
    let digits = rest
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect::<String>();
    let milliseconds = digits.parse::<u64>().ok()?;
    Some(milliseconds as f64 / 1000.0)
}

fn format_seconds_for_user(seconds: f64) -> String {
    if (seconds.fract() - 0.0).abs() < f64::EPSILON {
        format!("{seconds:.0} seconds")
    } else {
        format!("{seconds:.1} seconds")
    }
}

fn execute_agent_backend_turn_in_worker(
    resource: &str,
    schema_id: &str,
    schema_slug: Option<&str>,
    input: &str,
    backend_id: Option<&str>,
    model_id: Option<&str>,
    level_id: Option<&str>,
    native_session_id: Option<&str>,
    native_server: Option<Arc<OpencodeNativeServer>>,
    session_history: &[BackendAgentTranscriptEvent],
    state: &AgentWorkerState,
    progress: &mut dyn BackendAgentProgressSink,
) -> Result<crate::agent_executor::AgentBackendTurnResponse, String> {
    let request = BackendAgentTurnRequest {
        resource: resource.to_owned(),
        schema_id: schema_id.to_owned(),
        schema_slug: schema_slug.map(ToOwned::to_owned),
        input: input.to_owned(),
        session_history: session_history.to_vec(),
    };
    let mut runtime = UncachedReadonlyCypherRuntime::new(resource, schema_id, input, state);
    let selected_backend =
        resolve_agent_backend_for_turn(&state.agent_runtime, backend_id, model_id, level_id)?;

    let response = match selected_backend {
        AgentBackend::Stub => {
            let mut executor = StubAgentExecutor;
            executor.execute_turn(&request, &mut runtime, progress)
        }
        AgentBackend::Opencode(config) => {
            let native_server = native_server
                .or_else(|| state.native_server.clone())
                .ok_or_else(|| "native opencode server is not available".to_owned())?;
            let native_session_id = native_session_id.ok_or_else(|| {
                "native opencode session is not available for this turn".to_owned()
            })?;
            let mut executor = OpencodeExecutor::with_native_server(
                config,
                native_server,
                Some(native_session_id.to_owned()),
            );
            executor.execute_turn(&request, &mut runtime, progress)
        }
    }?;

    Ok(crate::agent_executor::AgentBackendTurnResponse {
        queries_executed: runtime.queries_executed.max(response.queries_executed),
        ..response
    })
}

fn recent_agent_session_history(session: &AgentSession) -> Vec<AgentTranscriptEvent> {
    let salient = session
        .transcript
        .iter()
        .filter(|event| is_salient_agent_session_event(event))
        .cloned()
        .collect::<Vec<_>>();

    let source = if salient.is_empty() {
        &session.transcript
    } else {
        &salient
    };

    let keep_from = source
        .len()
        .saturating_sub(MAX_AGENT_SESSION_CONTEXT_EVENTS);
    source[keep_from..].to_vec()
}

fn is_salient_agent_session_event(event: &AgentTranscriptEvent) -> bool {
    match event.kind {
        AgentTranscriptEventKind::User => true,
        AgentTranscriptEventKind::Assistant => {
            let text = event.text.trim();
            if text.is_empty() {
                return false;
            }
            !is_agent_transcript_meta_text(text)
        }
        AgentTranscriptEventKind::System | AgentTranscriptEventKind::Tool => false,
    }
}

fn is_agent_transcript_meta_text(text: &str) -> bool {
    let lowered = text.trim().to_ascii_lowercase();
    lowered.starts_with("prepared ")
        || lowered.starts_with("preparing ")
        || lowered.starts_with("completed ")
        || lowered.starts_with("applied:")
        || lowered.starts_with("ai session bound to ")
        || lowered.starts_with("ai context switched to ")
        || lowered.starts_with("ai context cleared.")
}

fn summarize_agent_turn_response_transcript(
    transcript: &[AgentTranscriptEvent],
) -> Vec<AgentTranscriptEvent> {
    let conclusion = transcript.iter().rev().find(|event| {
        matches!(event.kind, AgentTranscriptEventKind::Assistant)
            && !is_agent_transcript_meta_text(event.text.trim())
    });

    let summary = transcript.iter().rev().find(|event| {
        matches!(
            event.kind,
            AgentTranscriptEventKind::Assistant | AgentTranscriptEventKind::System
        ) && is_agent_turn_summary_text(event.text.trim())
    });

    match (conclusion, summary) {
        (Some(conclusion), Some(summary)) if conclusion != summary => {
            vec![conclusion.clone(), summary.clone()]
        }
        (Some(conclusion), _) => vec![conclusion.clone()],
        (None, Some(summary)) => vec![summary.clone()],
        (None, None) => transcript
            .iter()
            .filter(|event| {
                matches!(
                    event.kind,
                    AgentTranscriptEventKind::Assistant | AgentTranscriptEventKind::System
                )
            })
            .rev()
            .take(2)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect(),
    }
}

fn is_agent_turn_summary_text(text: &str) -> bool {
    let lowered = text.trim().to_ascii_lowercase();
    lowered.starts_with("prepared ")
        || lowered.starts_with("completed ")
        || lowered.starts_with("applied:")
}

fn backend_agent_session_history(
    session: &AgentSession,
) -> Vec<crate::agent_executor::AgentTranscriptEvent> {
    recent_agent_session_history(session)
        .into_iter()
        .map(agent_transcript_event_to_backend)
        .collect()
}

struct BoundedReadonlyCypherRuntime<'a> {
    resource: &'a str,
    schema_id: &'a str,
    question: &'a str,
    state: &'a ServerState,
    queries_executed: usize,
}

impl<'a> BoundedReadonlyCypherRuntime<'a> {
    fn new(
        resource: &'a str,
        schema_id: &'a str,
        question: &'a str,
        state: &'a ServerState,
    ) -> Self {
        Self {
            resource,
            schema_id,
            question,
            state,
            queries_executed: 0,
        }
    }
}

struct UncachedReadonlyCypherRuntime<'a> {
    resource: &'a str,
    schema_id: &'a str,
    question: &'a str,
    state: &'a AgentWorkerState,
    queries_executed: usize,
}

impl<'a> UncachedReadonlyCypherRuntime<'a> {
    fn new(
        resource: &'a str,
        schema_id: &'a str,
        question: &'a str,
        state: &'a AgentWorkerState,
    ) -> Self {
        Self {
            resource,
            schema_id,
            question,
            state,
            queries_executed: 0,
        }
    }
}

fn append_agent_query_log_entry(logger: &AgentQueryLogger, entry: AgentQueryLogEntry) {
    if let Err(error) = logger.append(&entry) {
        console_log(
            ConsoleLogKind::Warn,
            format!("agent query log error: {error}"),
        );
    }
}

impl BackendAgentReadonlyCypherRuntime for BoundedReadonlyCypherRuntime<'_> {
    fn run_readonly_cypher(
        &mut self,
        query: &str,
        why: Option<&str>,
    ) -> Result<BackendAgentReadonlyCypherResult, String> {
        let query_index = self.queries_executed + 1;
        if let Err(error) = enforce_agent_runtime_budget(
            self.queries_executed,
            self.state.agent_runtime.max_queries_per_turn,
        ) {
            append_agent_query_log_entry(
                self.state.agent_runtime.query_logger.as_ref(),
                AgentQueryLogEntry::failure(
                    self.state.agent_runtime.backend.label(),
                    self.resource,
                    self.schema_id,
                    self.question,
                    why,
                    query,
                    query_index,
                    None,
                    &error,
                ),
            );
            return Err(error);
        }

        let query = match validate_backend_agent_readonly_cypher(query) {
            Ok(query) => query,
            Err(error) => {
                append_agent_query_log_entry(
                    self.state.agent_runtime.query_logger.as_ref(),
                    AgentQueryLogEntry::failure(
                        self.state.agent_runtime.backend.label(),
                        self.resource,
                        self.schema_id,
                        self.question,
                        why,
                        query,
                        query_index,
                        None,
                        &error,
                    ),
                );
                return Err(error);
            }
        };

        let model_slug = match validate_agent_resource(self.resource) {
            Ok(model_slug) => model_slug,
            Err(error) => {
                append_agent_query_log_entry(
                    self.state.agent_runtime.query_logger.as_ref(),
                    AgentQueryLogEntry::failure(
                        self.state.agent_runtime.backend.label(),
                        self.resource,
                        self.schema_id,
                        self.question,
                        why,
                        &query,
                        query_index,
                        None,
                        &error,
                    ),
                );
                return Err(error);
            }
        };
        let (model, _) = match cached_ifc_model(self.state, model_slug) {
            Ok(model) => model,
            Err(error) => {
                append_agent_query_log_entry(
                    self.state.agent_runtime.query_logger.as_ref(),
                    AgentQueryLogEntry::failure(
                        self.state.agent_runtime.backend.label(),
                        self.resource,
                        self.schema_id,
                        self.question,
                        why,
                        &query,
                        query_index,
                        None,
                        &error,
                    ),
                );
                return Err(error);
            }
        };
        let query_result = match model.execute_cypher_rows(&query) {
            Ok(result) => result,
            Err(error) => {
                let error = format!("agent cypher execution failed for `{model_slug}`: {error}");
                append_agent_query_log_entry(
                    self.state.agent_runtime.query_logger.as_ref(),
                    AgentQueryLogEntry::failure(
                        self.state.agent_runtime.backend.label(),
                        self.resource,
                        self.schema_id,
                        self.question,
                        why,
                        &query,
                        query_index,
                        None,
                        &error,
                    ),
                );
                return Err(error);
            }
        };

        if query_result.rows.len() > self.state.agent_runtime.max_rows_per_query {
            let error = format!(
                "agent cypher returned {} rows, over the per-query cap of {}; refine the query",
                query_result.rows.len(),
                self.state.agent_runtime.max_rows_per_query
            );
            append_agent_query_log_entry(
                self.state.agent_runtime.query_logger.as_ref(),
                AgentQueryLogEntry::failure(
                    self.state.agent_runtime.backend.label(),
                    self.resource,
                    self.schema_id,
                    self.question,
                    why,
                    &query,
                    query_index,
                    Some(query_result.rows.len()),
                    &error,
                ),
            );
            return Err(error);
        }

        self.queries_executed = self.queries_executed.saturating_add(1);
        let db_node_ids = extract_db_node_ids(&query_result.columns, &query_result.rows);
        let semantic_element_ids =
            extract_semantic_element_ids(&query_result.columns, &query_result.rows);
        append_agent_query_log_entry(
            self.state.agent_runtime.query_logger.as_ref(),
            AgentQueryLogEntry::success(
                self.state.agent_runtime.backend.label(),
                self.resource,
                self.schema_id,
                self.question,
                why,
                &query,
                query_index,
                query_result.rows.len(),
                db_node_ids.len(),
                semantic_element_ids.len(),
            ),
        );

        Ok(BackendAgentReadonlyCypherResult {
            columns: query_result.columns,
            rows: query_result.rows,
            db_node_ids,
            semantic_element_ids,
        })
    }

    fn get_schema_context(&mut self) -> Result<BackendAgentSchemaContext, String> {
        enforce_agent_runtime_budget(
            self.queries_executed,
            self.state.agent_runtime.max_queries_per_turn,
        )?;
        let schema = resolve_agent_resource_schema(self.resource, &self.state.ifc_artifacts_root)?;
        let context = load_schema_context(&self.state.ifc_artifacts_root, &schema)?;
        self.queries_executed = self.queries_executed.saturating_add(1);
        Ok(context)
    }

    fn get_entity_reference(
        &mut self,
        entity_names: &[String],
    ) -> Result<Vec<BackendAgentEntityReference>, String> {
        enforce_agent_runtime_budget(
            self.queries_executed,
            self.state.agent_runtime.max_queries_per_turn,
        )?;
        let schema = resolve_agent_resource_schema(self.resource, &self.state.ifc_artifacts_root)?;
        let references =
            load_entity_references(&self.state.ifc_artifacts_root, &schema, entity_names)?;
        self.queries_executed = self.queries_executed.saturating_add(1);
        Ok(references)
    }

    fn get_query_playbook(
        &mut self,
        goal: &str,
        entity_names: &[String],
    ) -> Result<Vec<BackendAgentQueryPlaybook>, String> {
        enforce_agent_runtime_budget(
            self.queries_executed,
            self.state.agent_runtime.max_queries_per_turn,
        )?;
        let schema = resolve_agent_resource_schema(self.resource, &self.state.ifc_artifacts_root)?;
        let playbooks =
            load_query_playbooks(&self.state.ifc_artifacts_root, &schema, goal, entity_names)?;
        self.queries_executed = self.queries_executed.saturating_add(1);
        Ok(playbooks)
    }

    fn get_relation_reference(
        &mut self,
        relation_names: &[String],
    ) -> Result<Vec<BackendAgentRelationReference>, String> {
        enforce_agent_runtime_budget(
            self.queries_executed,
            self.state.agent_runtime.max_queries_per_turn,
        )?;
        let schema = resolve_agent_resource_schema(self.resource, &self.state.ifc_artifacts_root)?;
        let references =
            load_relation_references(&self.state.ifc_artifacts_root, &schema, relation_names)?;
        self.queries_executed = self.queries_executed.saturating_add(1);
        Ok(references)
    }

    fn describe_nodes(
        &mut self,
        db_node_ids: &[i64],
    ) -> Result<Vec<BackendAgentNodeSummary>, String> {
        enforce_agent_runtime_budget(
            self.queries_executed,
            self.state.agent_runtime.max_queries_per_turn,
        )?;
        let model_slug = validate_agent_resource(self.resource)?;
        let (model, _) = cached_ifc_model(self.state, model_slug)?;
        let nodes = fetch_agent_node_summaries(model.as_ref(), db_node_ids)?;
        self.queries_executed = self.queries_executed.saturating_add(1);
        Ok(nodes)
    }

    fn get_node_properties(
        &mut self,
        db_node_id: i64,
    ) -> Result<BackendAgentNodePropertiesResult, String> {
        enforce_agent_runtime_budget(
            self.queries_executed,
            self.state.agent_runtime.max_queries_per_turn,
        )?;
        let model_slug = validate_agent_resource(self.resource)?;
        let (model, _) = cached_ifc_model(self.state, model_slug)?;
        let details = fetch_agent_node_properties(
            model.as_ref(),
            db_node_id,
            DEFAULT_GRAPH_NODE_PROPERTIES_MAX_RELATIONS,
        )?;
        self.queries_executed = self.queries_executed.saturating_add(1);
        Ok(details)
    }

    fn get_neighbors(
        &mut self,
        db_node_ids: &[i64],
        hops: usize,
        mode: BackendAgentGraphMode,
    ) -> Result<BackendAgentNeighborGraph, String> {
        enforce_agent_runtime_budget(
            self.queries_executed,
            self.state.agent_runtime.max_queries_per_turn,
        )?;
        let model_slug = validate_agent_resource(self.resource)?;
        let (model, _) = cached_ifc_model(self.state, model_slug)?;
        let graph =
            fetch_agent_neighbor_graph(model.as_ref(), self.resource, db_node_ids, hops, mode)?;
        self.queries_executed = self.queries_executed.saturating_add(1);
        Ok(graph)
    }
}

impl BackendAgentReadonlyCypherRuntime for UncachedReadonlyCypherRuntime<'_> {
    fn run_readonly_cypher(
        &mut self,
        query: &str,
        why: Option<&str>,
    ) -> Result<BackendAgentReadonlyCypherResult, String> {
        let query_index = self.queries_executed + 1;
        if let Err(error) = enforce_agent_runtime_budget(
            self.queries_executed,
            self.state.agent_runtime.max_queries_per_turn,
        ) {
            append_agent_query_log_entry(
                self.state.agent_runtime.query_logger.as_ref(),
                AgentQueryLogEntry::failure(
                    self.state.agent_runtime.backend.label(),
                    self.resource,
                    self.schema_id,
                    self.question,
                    why,
                    query,
                    query_index,
                    None,
                    &error,
                ),
            );
            return Err(error);
        }

        let query = match validate_backend_agent_readonly_cypher(query) {
            Ok(query) => query,
            Err(error) => {
                append_agent_query_log_entry(
                    self.state.agent_runtime.query_logger.as_ref(),
                    AgentQueryLogEntry::failure(
                        self.state.agent_runtime.backend.label(),
                        self.resource,
                        self.schema_id,
                        self.question,
                        why,
                        query,
                        query_index,
                        None,
                        &error,
                    ),
                );
                return Err(error);
            }
        };

        let model_slug = match validate_agent_resource(self.resource) {
            Ok(model_slug) => model_slug,
            Err(error) => {
                append_agent_query_log_entry(
                    self.state.agent_runtime.query_logger.as_ref(),
                    AgentQueryLogEntry::failure(
                        self.state.agent_runtime.backend.label(),
                        self.resource,
                        self.schema_id,
                        self.question,
                        why,
                        &query,
                        query_index,
                        None,
                        &error,
                    ),
                );
                return Err(error);
            }
        };
        let layout = IfcArtifactLayout::new(&self.state.ifc_artifacts_root, model_slug);
        let model = match VelrIfcModel::open(layout) {
            Ok(model) => model,
            Err(error) => {
                let error = format!("failed to open IFC model `{model_slug}`: {error}");
                append_agent_query_log_entry(
                    self.state.agent_runtime.query_logger.as_ref(),
                    AgentQueryLogEntry::failure(
                        self.state.agent_runtime.backend.label(),
                        self.resource,
                        self.schema_id,
                        self.question,
                        why,
                        &query,
                        query_index,
                        None,
                        &error,
                    ),
                );
                return Err(error);
            }
        };
        let query_result = match model.execute_cypher_rows(&query) {
            Ok(result) => result,
            Err(error) => {
                let error = format!("agent cypher execution failed for `{model_slug}`: {error}");
                append_agent_query_log_entry(
                    self.state.agent_runtime.query_logger.as_ref(),
                    AgentQueryLogEntry::failure(
                        self.state.agent_runtime.backend.label(),
                        self.resource,
                        self.schema_id,
                        self.question,
                        why,
                        &query,
                        query_index,
                        None,
                        &error,
                    ),
                );
                return Err(error);
            }
        };

        if query_result.rows.len() > self.state.agent_runtime.max_rows_per_query {
            let error = format!(
                "agent cypher returned {} rows, over the per-query cap of {}; refine the query",
                query_result.rows.len(),
                self.state.agent_runtime.max_rows_per_query
            );
            append_agent_query_log_entry(
                self.state.agent_runtime.query_logger.as_ref(),
                AgentQueryLogEntry::failure(
                    self.state.agent_runtime.backend.label(),
                    self.resource,
                    self.schema_id,
                    self.question,
                    why,
                    &query,
                    query_index,
                    Some(query_result.rows.len()),
                    &error,
                ),
            );
            return Err(error);
        }

        self.queries_executed = self.queries_executed.saturating_add(1);
        let db_node_ids = extract_db_node_ids(&query_result.columns, &query_result.rows);
        let semantic_element_ids =
            extract_semantic_element_ids(&query_result.columns, &query_result.rows);
        append_agent_query_log_entry(
            self.state.agent_runtime.query_logger.as_ref(),
            AgentQueryLogEntry::success(
                self.state.agent_runtime.backend.label(),
                self.resource,
                self.schema_id,
                self.question,
                why,
                &query,
                query_index,
                query_result.rows.len(),
                db_node_ids.len(),
                semantic_element_ids.len(),
            ),
        );

        Ok(BackendAgentReadonlyCypherResult {
            columns: query_result.columns,
            rows: query_result.rows,
            db_node_ids,
            semantic_element_ids,
        })
    }

    fn get_schema_context(&mut self) -> Result<BackendAgentSchemaContext, String> {
        enforce_agent_runtime_budget(
            self.queries_executed,
            self.state.agent_runtime.max_queries_per_turn,
        )?;
        let schema = resolve_agent_resource_schema(self.resource, &self.state.ifc_artifacts_root)?;
        let context = load_schema_context(&self.state.ifc_artifacts_root, &schema)?;
        self.queries_executed = self.queries_executed.saturating_add(1);
        Ok(context)
    }

    fn get_entity_reference(
        &mut self,
        entity_names: &[String],
    ) -> Result<Vec<BackendAgentEntityReference>, String> {
        enforce_agent_runtime_budget(
            self.queries_executed,
            self.state.agent_runtime.max_queries_per_turn,
        )?;
        let schema = resolve_agent_resource_schema(self.resource, &self.state.ifc_artifacts_root)?;
        let references =
            load_entity_references(&self.state.ifc_artifacts_root, &schema, entity_names)?;
        self.queries_executed = self.queries_executed.saturating_add(1);
        Ok(references)
    }

    fn get_query_playbook(
        &mut self,
        goal: &str,
        entity_names: &[String],
    ) -> Result<Vec<BackendAgentQueryPlaybook>, String> {
        enforce_agent_runtime_budget(
            self.queries_executed,
            self.state.agent_runtime.max_queries_per_turn,
        )?;
        let schema = resolve_agent_resource_schema(self.resource, &self.state.ifc_artifacts_root)?;
        let playbooks =
            load_query_playbooks(&self.state.ifc_artifacts_root, &schema, goal, entity_names)?;
        self.queries_executed = self.queries_executed.saturating_add(1);
        Ok(playbooks)
    }

    fn get_relation_reference(
        &mut self,
        relation_names: &[String],
    ) -> Result<Vec<BackendAgentRelationReference>, String> {
        enforce_agent_runtime_budget(
            self.queries_executed,
            self.state.agent_runtime.max_queries_per_turn,
        )?;
        let schema = resolve_agent_resource_schema(self.resource, &self.state.ifc_artifacts_root)?;
        let references =
            load_relation_references(&self.state.ifc_artifacts_root, &schema, relation_names)?;
        self.queries_executed = self.queries_executed.saturating_add(1);
        Ok(references)
    }

    fn describe_nodes(
        &mut self,
        db_node_ids: &[i64],
    ) -> Result<Vec<BackendAgentNodeSummary>, String> {
        enforce_agent_runtime_budget(
            self.queries_executed,
            self.state.agent_runtime.max_queries_per_turn,
        )?;
        let model_slug = validate_agent_resource(self.resource)?;
        let layout = IfcArtifactLayout::new(&self.state.ifc_artifacts_root, model_slug);
        let model = VelrIfcModel::open(layout)
            .map_err(|error| format!("failed to open IFC model `{model_slug}`: {error}"))?;
        let nodes = fetch_agent_node_summaries(&model, db_node_ids)?;
        self.queries_executed = self.queries_executed.saturating_add(1);
        Ok(nodes)
    }

    fn get_node_properties(
        &mut self,
        db_node_id: i64,
    ) -> Result<BackendAgentNodePropertiesResult, String> {
        enforce_agent_runtime_budget(
            self.queries_executed,
            self.state.agent_runtime.max_queries_per_turn,
        )?;
        let model_slug = validate_agent_resource(self.resource)?;
        let layout = IfcArtifactLayout::new(&self.state.ifc_artifacts_root, model_slug);
        let model = VelrIfcModel::open(layout)
            .map_err(|error| format!("failed to open IFC model `{model_slug}`: {error}"))?;
        let details = fetch_agent_node_properties(
            &model,
            db_node_id,
            DEFAULT_GRAPH_NODE_PROPERTIES_MAX_RELATIONS,
        )?;
        self.queries_executed = self.queries_executed.saturating_add(1);
        Ok(details)
    }

    fn get_neighbors(
        &mut self,
        db_node_ids: &[i64],
        hops: usize,
        mode: BackendAgentGraphMode,
    ) -> Result<BackendAgentNeighborGraph, String> {
        enforce_agent_runtime_budget(
            self.queries_executed,
            self.state.agent_runtime.max_queries_per_turn,
        )?;
        let model_slug = validate_agent_resource(self.resource)?;
        let layout = IfcArtifactLayout::new(&self.state.ifc_artifacts_root, model_slug);
        let model = VelrIfcModel::open(layout)
            .map_err(|error| format!("failed to open IFC model `{model_slug}`: {error}"))?;
        let graph = fetch_agent_neighbor_graph(&model, self.resource, db_node_ids, hops, mode)?;
        self.queries_executed = self.queries_executed.saturating_add(1);
        Ok(graph)
    }
}

fn agent_transcript_event_from_backend(event: BackendAgentTranscriptEvent) -> AgentTranscriptEvent {
    AgentTranscriptEvent {
        kind: match event.kind {
            BackendAgentTranscriptEventKind::System => AgentTranscriptEventKind::System,
            BackendAgentTranscriptEventKind::User => AgentTranscriptEventKind::User,
            BackendAgentTranscriptEventKind::Tool => AgentTranscriptEventKind::Tool,
            BackendAgentTranscriptEventKind::Assistant => AgentTranscriptEventKind::Assistant,
        },
        text: event.text,
    }
}

fn agent_transcript_event_to_backend(event: AgentTranscriptEvent) -> BackendAgentTranscriptEvent {
    BackendAgentTranscriptEvent {
        kind: match event.kind {
            AgentTranscriptEventKind::System => BackendAgentTranscriptEventKind::System,
            AgentTranscriptEventKind::User => BackendAgentTranscriptEventKind::User,
            AgentTranscriptEventKind::Tool => BackendAgentTranscriptEventKind::Tool,
            AgentTranscriptEventKind::Assistant => BackendAgentTranscriptEventKind::Assistant,
        },
        text: event.text,
    }
}

fn agent_ui_action_from_backend(action: BackendAgentUiAction) -> AgentUiAction {
    match action {
        BackendAgentUiAction::GraphSetSeeds { db_node_ids } => {
            AgentUiAction::GraphSetSeeds { db_node_ids }
        }
        BackendAgentUiAction::PropertiesShowNode { db_node_id } => {
            AgentUiAction::PropertiesShowNode { db_node_id }
        }
        BackendAgentUiAction::ElementsHide { semantic_ids } => {
            AgentUiAction::ElementsHide { semantic_ids }
        }
        BackendAgentUiAction::ElementsShow { semantic_ids } => {
            AgentUiAction::ElementsShow { semantic_ids }
        }
        BackendAgentUiAction::ElementsSelect { semantic_ids } => {
            AgentUiAction::ElementsSelect { semantic_ids }
        }
        BackendAgentUiAction::ViewerFrameVisible => AgentUiAction::ViewerFrameVisible,
    }
}

fn enforce_agent_runtime_budget(executed: usize, maximum: usize) -> Result<(), String> {
    if executed >= maximum {
        return Err(format!(
            "agent exceeded the read-only inspection budget for one turn ({maximum})"
        ));
    }
    Ok(())
}

fn run_stub_agent_turn<F>(
    resource: &str,
    input: &str,
    mut run_query: F,
) -> Result<StubAgentExecution, String>
where
    F: FnMut(&str) -> Result<AgentReadonlyCypherResult, String>,
{
    let trimmed = input.trim();
    if trimmed.eq_ignore_ascii_case("help") {
        return Ok(StubAgentExecution {
            transcript: vec![AgentTranscriptEvent::assistant(
                "Stub agent commands: `cypher: ...`, `graph: 1,2`, `hide: id1,id2`, `show: ...`, `select: ...`, `frame`."
                    .to_owned(),
            )],
            action_candidates: Vec::new(),
            queries_executed: 0,
        });
    }

    if trimmed.eq_ignore_ascii_case("frame") {
        return Ok(StubAgentExecution {
            transcript: vec![AgentTranscriptEvent::assistant(
                "Prepared viewer.frame_visible.".to_owned(),
            )],
            action_candidates: vec![AgentActionCandidate::viewer_frame_visible()],
            queries_executed: 0,
        });
    }

    if let Some(raw_ids) = trimmed.strip_prefix("graph:") {
        let db_node_ids = parse_agent_db_node_ids(raw_ids)?;
        return Ok(StubAgentExecution {
            transcript: vec![AgentTranscriptEvent::assistant(format!(
                "Prepared graph.set_seeds for {} node{} in {}.",
                db_node_ids.len(),
                if db_node_ids.len() == 1 { "" } else { "s" },
                resource
            ))],
            action_candidates: vec![AgentActionCandidate::graph_set_seeds(db_node_ids)],
            queries_executed: 0,
        });
    }

    if let Some(raw_ids) = trimmed.strip_prefix("hide:") {
        let semantic_ids = parse_agent_semantic_ids(raw_ids)?;
        return Ok(StubAgentExecution {
            transcript: vec![AgentTranscriptEvent::assistant(format!(
                "Prepared elements.hide for {} element{}.",
                semantic_ids.len(),
                if semantic_ids.len() == 1 { "" } else { "s" }
            ))],
            action_candidates: vec![AgentActionCandidate::elements_hide(semantic_ids)],
            queries_executed: 0,
        });
    }

    if let Some(raw_ids) = trimmed.strip_prefix("show:") {
        let semantic_ids = parse_agent_semantic_ids(raw_ids)?;
        return Ok(StubAgentExecution {
            transcript: vec![AgentTranscriptEvent::assistant(format!(
                "Prepared elements.show for {} element{}.",
                semantic_ids.len(),
                if semantic_ids.len() == 1 { "" } else { "s" }
            ))],
            action_candidates: vec![AgentActionCandidate::elements_show(semantic_ids)],
            queries_executed: 0,
        });
    }

    if let Some(raw_ids) = trimmed.strip_prefix("select:") {
        let semantic_ids = parse_agent_semantic_ids(raw_ids)?;
        return Ok(StubAgentExecution {
            transcript: vec![AgentTranscriptEvent::assistant(format!(
                "Prepared elements.select for {} element{}.",
                semantic_ids.len(),
                if semantic_ids.len() == 1 { "" } else { "s" }
            ))],
            action_candidates: vec![AgentActionCandidate::elements_select(semantic_ids)],
            queries_executed: 0,
        });
    }

    if let Some(raw_query) = trimmed.strip_prefix("cypher:") {
        let query = validate_agent_readonly_cypher(raw_query)?;
        let mut transcript = vec![AgentTranscriptEvent::tool(format!(
            "Running read-only Cypher against {}.",
            resource
        ))];
        let result = run_query(&query)?;
        transcript.push(AgentTranscriptEvent::assistant(format!(
            "Query returned {} row{} across {} column{}.",
            result.rows.len(),
            if result.rows.len() == 1 { "" } else { "s" },
            result.columns.len(),
            if result.columns.len() == 1 { "" } else { "s" }
        )));

        let mut action_candidates = Vec::new();
        if !result.db_node_ids.is_empty() {
            transcript.push(AgentTranscriptEvent::assistant(format!(
                "Prepared graph.set_seeds from {} returned node id{}.",
                result.db_node_ids.len(),
                if result.db_node_ids.len() == 1 {
                    ""
                } else {
                    "s"
                }
            )));
            action_candidates.push(AgentActionCandidate::graph_set_seeds(
                result.db_node_ids.clone(),
            ));
        }
        if !result.semantic_element_ids.is_empty() {
            transcript.push(AgentTranscriptEvent::assistant(format!(
                "Prepared elements.select from {} returned semantic id{}.",
                result.semantic_element_ids.len(),
                if result.semantic_element_ids.len() == 1 {
                    ""
                } else {
                    "s"
                }
            )));
            action_candidates.push(AgentActionCandidate::elements_select(
                result.semantic_element_ids.clone(),
            ));
        }

        return Ok(StubAgentExecution {
            transcript,
            action_candidates,
            queries_executed: 1,
        });
    }

    Ok(StubAgentExecution {
        transcript: vec![AgentTranscriptEvent::assistant(
            "Stub agent only understands `cypher: ...`, `graph: ...`, `hide: ...`, `show: ...`, `select: ...`, `frame`, and `help` for now."
                .to_owned(),
        )],
        action_candidates: Vec::new(),
        queries_executed: 0,
    })
}

fn execute_agent_readonly_cypher(
    resource: &str,
    query: &str,
    state: &ServerState,
) -> Result<AgentReadonlyCypherResult, String> {
    let model_slug = validate_agent_resource(resource)?;
    let (model, _) = cached_ifc_model(state, model_slug)?;
    let query_result = model
        .execute_cypher_rows(query)
        .map_err(|error| format!("agent cypher execution failed for `{model_slug}`: {error}"))?;
    let db_node_ids = extract_db_node_ids(&query_result.columns, &query_result.rows);
    let semantic_element_ids =
        extract_semantic_element_ids(&query_result.columns, &query_result.rows);

    Ok(AgentReadonlyCypherResult {
        columns: query_result.columns,
        rows: query_result.rows,
        db_node_ids,
        semantic_element_ids,
    })
}

fn validate_agent_readonly_cypher(query: &str) -> Result<String, String> {
    let statements = query
        .split(';')
        .map(str::trim)
        .filter(|statement| !statement.is_empty())
        .collect::<Vec<_>>();
    if statements.is_empty() {
        return Err("agent cypher query must not be empty".to_owned());
    }
    if statements.len() != 1 {
        return Err("agent cypher must be a single statement".to_owned());
    }

    let normalized = statements[0].to_owned();
    let upper_tokens = normalized
        .split(|ch: char| !ch.is_ascii_alphabetic())
        .filter(|token| !token.is_empty())
        .map(|token| token.to_ascii_uppercase())
        .collect::<HashSet<_>>();
    let blocked_keywords = [
        "CALL", "CREATE", "DELETE", "DROP", "FOREACH", "LOAD", "MERGE", "REMOVE", "SET",
    ];
    if let Some(keyword) = blocked_keywords
        .iter()
        .find(|keyword| upper_tokens.contains(**keyword))
    {
        return Err(format!(
            "agent cypher must be read-only; `{keyword}` is not allowed"
        ));
    }

    Ok(normalized)
}

fn validate_agent_action_candidates(
    candidates: Vec<AgentActionCandidate>,
) -> Result<Vec<AgentUiAction>, String> {
    if candidates.len() > MAX_AGENT_ACTIONS {
        return Err(format!(
            "agent returned too many UI actions: {} (max {})",
            candidates.len(),
            MAX_AGENT_ACTIONS
        ));
    }

    let actions = candidates
        .into_iter()
        .map(validate_agent_action_candidate)
        .collect::<Result<Vec<_>, _>>()?;

    Ok(normalize_agent_ui_actions(actions))
}

fn validate_agent_action_candidate(
    candidate: AgentActionCandidate,
) -> Result<AgentUiAction, String> {
    match candidate.kind.as_str() {
        "graph.set_seeds" => {
            if !candidate.semantic_ids.is_empty() {
                return Err("graph.set_seeds does not accept semanticIds".to_owned());
            }
            let db_node_ids = normalize_agent_db_node_ids(candidate.db_node_ids)?;
            Ok(AgentUiAction::GraphSetSeeds { db_node_ids })
        }
        "properties.show_node" => {
            if !candidate.semantic_ids.is_empty() {
                return Err("properties.show_node does not accept semanticIds".to_owned());
            }
            let db_node_ids = normalize_agent_db_node_ids(candidate.db_node_ids)?;
            if db_node_ids.len() != 1 {
                return Err("properties.show_node requires exactly one dbNodeId".to_owned());
            }
            Ok(AgentUiAction::PropertiesShowNode {
                db_node_id: db_node_ids[0],
            })
        }
        "elements.hide" => {
            if !candidate.db_node_ids.is_empty() {
                return Err("elements.hide does not accept dbNodeIds".to_owned());
            }
            let semantic_ids = normalize_agent_semantic_ids(candidate.semantic_ids)?;
            Ok(AgentUiAction::ElementsHide { semantic_ids })
        }
        "elements.show" => {
            if !candidate.db_node_ids.is_empty() {
                return Err("elements.show does not accept dbNodeIds".to_owned());
            }
            let semantic_ids = normalize_agent_semantic_ids(candidate.semantic_ids)?;
            Ok(AgentUiAction::ElementsShow { semantic_ids })
        }
        "elements.select" => {
            if !candidate.db_node_ids.is_empty() {
                return Err("elements.select does not accept dbNodeIds".to_owned());
            }
            let semantic_ids = normalize_agent_semantic_ids(candidate.semantic_ids)?;
            Ok(AgentUiAction::ElementsSelect { semantic_ids })
        }
        "viewer.frame_visible" => {
            if !candidate.semantic_ids.is_empty() || !candidate.db_node_ids.is_empty() {
                return Err("viewer.frame_visible does not accept ids".to_owned());
            }
            Ok(AgentUiAction::ViewerFrameVisible)
        }
        other => Err(format!("unsupported agent UI action kind `{other}`")),
    }
}

fn normalize_agent_ui_actions(actions: Vec<AgentUiAction>) -> Vec<AgentUiAction> {
    let mut merged_graph_seed_ids = Vec::new();
    let mut graph_seed_seen = HashSet::new();
    let mut merged_hide_ids = Vec::new();
    let mut hide_seen = HashSet::new();
    let mut merged_show_ids = Vec::new();
    let mut show_seen = HashSet::new();
    let mut merged_select_ids = Vec::new();
    let mut select_seen = HashSet::new();
    let mut latest_properties_node_id = None;
    let mut graph_set_seeds_present = false;
    let mut properties_show_present = false;
    let mut elements_hide_present = false;
    let mut elements_show_present = false;
    let mut elements_select_present = false;
    let mut frame_visible_present = false;
    let mut order = Vec::new();

    for action in actions {
        match action {
            AgentUiAction::GraphSetSeeds { db_node_ids } => {
                if !graph_set_seeds_present {
                    order.push(0u8);
                    graph_set_seeds_present = true;
                }
                for db_node_id in db_node_ids {
                    if graph_seed_seen.insert(db_node_id) {
                        merged_graph_seed_ids.push(db_node_id);
                    }
                }
            }
            AgentUiAction::PropertiesShowNode { db_node_id } => {
                if !properties_show_present {
                    order.push(1u8);
                    properties_show_present = true;
                }
                latest_properties_node_id = Some(db_node_id);
            }
            AgentUiAction::ElementsHide { semantic_ids } => {
                if !elements_hide_present {
                    order.push(2u8);
                    elements_hide_present = true;
                }
                for semantic_id in semantic_ids {
                    if hide_seen.insert(semantic_id.clone()) {
                        merged_hide_ids.push(semantic_id);
                    }
                }
            }
            AgentUiAction::ElementsShow { semantic_ids } => {
                if !elements_show_present {
                    order.push(3u8);
                    elements_show_present = true;
                }
                for semantic_id in semantic_ids {
                    if show_seen.insert(semantic_id.clone()) {
                        merged_show_ids.push(semantic_id);
                    }
                }
            }
            AgentUiAction::ElementsSelect { semantic_ids } => {
                if !elements_select_present {
                    order.push(4u8);
                    elements_select_present = true;
                }
                for semantic_id in semantic_ids {
                    if select_seen.insert(semantic_id.clone()) {
                        merged_select_ids.push(semantic_id);
                    }
                }
            }
            AgentUiAction::ViewerFrameVisible => {
                if !frame_visible_present {
                    order.push(5u8);
                    frame_visible_present = true;
                }
            }
        }
    }

    let mut normalized = Vec::new();
    for kind in order {
        match kind {
            0 if !merged_graph_seed_ids.is_empty() => {
                normalized.push(AgentUiAction::GraphSetSeeds {
                    db_node_ids: merged_graph_seed_ids.clone(),
                })
            }
            1 => {
                if let Some(db_node_id) = latest_properties_node_id {
                    normalized.push(AgentUiAction::PropertiesShowNode { db_node_id });
                }
            }
            2 if !merged_hide_ids.is_empty() => normalized.push(AgentUiAction::ElementsHide {
                semantic_ids: merged_hide_ids.clone(),
            }),
            3 if !merged_show_ids.is_empty() => normalized.push(AgentUiAction::ElementsShow {
                semantic_ids: merged_show_ids.clone(),
            }),
            4 if !merged_select_ids.is_empty() => normalized.push(AgentUiAction::ElementsSelect {
                semantic_ids: merged_select_ids.clone(),
            }),
            5 => normalized.push(AgentUiAction::ViewerFrameVisible),
            _ => {}
        }
    }

    normalized
}

fn normalize_agent_semantic_ids(ids: Vec<String>) -> Result<Vec<String>, String> {
    let mut normalized = Vec::new();
    let mut seen = HashSet::new();
    for id in ids {
        let trimmed = id.trim();
        if trimmed.is_empty() {
            continue;
        }
        if seen.insert(trimmed.to_owned()) {
            normalized.push(trimmed.to_owned());
        }
    }
    if normalized.is_empty() {
        return Err("agent action requires at least one semantic id".to_owned());
    }
    if normalized.len() > MAX_AGENT_ACTION_IDS {
        return Err(format!(
            "agent action includes too many semantic ids: {} (max {})",
            normalized.len(),
            MAX_AGENT_ACTION_IDS
        ));
    }
    Ok(normalized)
}

fn normalize_agent_db_node_ids(ids: Vec<i64>) -> Result<Vec<i64>, String> {
    let mut normalized = Vec::new();
    let mut seen = HashSet::new();
    for id in ids {
        if id <= 0 {
            continue;
        }
        if seen.insert(id) {
            normalized.push(id);
        }
    }
    if normalized.is_empty() {
        return Err("agent action requires at least one db node id".to_owned());
    }
    if normalized.len() > MAX_AGENT_ACTION_IDS {
        return Err(format!(
            "agent action includes too many db node ids: {} (max {})",
            normalized.len(),
            MAX_AGENT_ACTION_IDS
        ));
    }
    Ok(normalized)
}

fn parse_agent_semantic_ids(raw_ids: &str) -> Result<Vec<String>, String> {
    normalize_agent_semantic_ids(
        raw_ids
            .split(',')
            .map(|value| value.trim().to_owned())
            .collect::<Vec<_>>(),
    )
}

fn parse_agent_db_node_ids(raw_ids: &str) -> Result<Vec<i64>, String> {
    let parsed = raw_ids
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            value
                .parse::<i64>()
                .map_err(|error| format!("invalid db node id `{value}`: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    normalize_agent_db_node_ids(parsed)
}

fn build_graph_subgraph_response(
    request: &GraphSubgraphApiRequest,
    model: &VelrIfcModel,
    limits: GraphBuildLimits,
) -> Result<GraphSubgraphApiResponse, String> {
    let requested_seed_count = request.seed_node_ids.len();
    let mut seed_node_ids = dedup_sorted_ids(&request.seed_node_ids);
    if seed_node_ids.len() > limits.max_nodes {
        seed_node_ids.truncate(limits.max_nodes);
    }

    let seed_node_records = fetch_graph_nodes_by_ids(model, &seed_node_ids)?;
    let seed_node_records_by_id = seed_node_records
        .into_iter()
        .map(|record| (record.db_node_id, record))
        .collect::<HashMap<_, _>>();

    let mut nodes_by_id = HashMap::<i64, GraphSubgraphNode>::new();
    let mut edges_by_id = HashMap::<String, GraphSubgraphEdge>::new();
    let mut frontier = seed_node_ids
        .iter()
        .filter_map(|db_node_id| {
            seed_node_records_by_id
                .get(db_node_id)
                .cloned()
                .map(|record| (*db_node_id, graph_node_from_record(record, 0, true)))
        })
        .collect::<Vec<_>>();
    let mut truncated =
        seed_node_ids.len() < requested_seed_count || frontier.len() < seed_node_ids.len();

    for (db_node_id, node) in frontier.iter().cloned() {
        nodes_by_id.insert(db_node_id, node);
    }

    for depth in 0..limits.hops {
        if frontier.is_empty()
            || nodes_by_id.len() >= limits.max_nodes
            || edges_by_id.len() >= limits.max_edges
        {
            break;
        }

        let frontier_ids = frontier
            .iter()
            .map(|(db_node_id, _)| *db_node_id)
            .collect::<Vec<_>>();
        let frontier_set = frontier_ids.iter().copied().collect::<HashSet<_>>();
        let remaining_edge_capacity = limits.max_edges.saturating_sub(edges_by_id.len());
        let (candidate_edges, edge_query_truncated) =
            fetch_incident_graph_edges(model, &frontier_ids, remaining_edge_capacity)?;
        truncated |= edge_query_truncated;
        let mut neighbor_ids = HashSet::new();

        for edge in &candidate_edges {
            if frontier_set.contains(&edge.source_db_node_id) {
                neighbor_ids.insert(edge.target_db_node_id);
            }
            if frontier_set.contains(&edge.target_db_node_id) {
                neighbor_ids.insert(edge.source_db_node_id);
            }
        }

        let mut neighbor_ids = neighbor_ids
            .into_iter()
            .filter(|db_node_id| !nodes_by_id.contains_key(db_node_id))
            .collect::<Vec<_>>();
        neighbor_ids.sort_unstable();

        let remaining_node_capacity = limits.max_nodes.saturating_sub(nodes_by_id.len());
        if neighbor_ids.len() > remaining_node_capacity {
            neighbor_ids.truncate(remaining_node_capacity);
            truncated = true;
        }

        let neighbor_records = fetch_graph_nodes_by_ids(model, &neighbor_ids)?;
        let neighbor_records_by_id = neighbor_records
            .into_iter()
            .map(|record| (record.db_node_id, record))
            .collect::<HashMap<_, _>>();
        let mut next_frontier = Vec::new();
        for db_node_id in neighbor_ids {
            let Some(record) = neighbor_records_by_id.get(&db_node_id).cloned() else {
                truncated = true;
                continue;
            };
            let node = graph_node_from_record(record, depth + 1, false);
            if !graph_mode_keeps_node(&node, limits.mode) {
                continue;
            }
            let db_node_id = node.db_node_id;
            if nodes_by_id.insert(db_node_id, node.clone()).is_none() {
                next_frontier.push((db_node_id, node));
            }
        }

        for edge in candidate_edges {
            if edges_by_id.len() >= limits.max_edges {
                truncated = true;
                break;
            }
            if !nodes_by_id.contains_key(&edge.source_db_node_id)
                || !nodes_by_id.contains_key(&edge.target_db_node_id)
            {
                continue;
            }
            let edge_id = graph_edge_id(
                edge.source_db_node_id,
                &edge.relationship_type,
                edge.target_db_node_id,
            );
            edges_by_id
                .entry(edge_id.clone())
                .or_insert(GraphSubgraphEdge {
                    edge_id,
                    source_db_node_id: edge.source_db_node_id,
                    target_db_node_id: edge.target_db_node_id,
                    relationship_type: edge.relationship_type,
                });
        }

        frontier = next_frontier;
    }

    let mut nodes = nodes_by_id.into_values().collect::<Vec<_>>();
    nodes.sort_by(|left, right| {
        left.hop_distance
            .cmp(&right.hop_distance)
            .then_with(|| left.db_node_id.cmp(&right.db_node_id))
    });

    let mut edges = edges_by_id.into_values().collect::<Vec<_>>();
    edges.sort_by(|left, right| left.edge_id.cmp(&right.edge_id));

    Ok(GraphSubgraphApiResponse {
        resource: request.resource.clone(),
        mode: limits.mode,
        hops: limits.hops,
        max_nodes: limits.max_nodes,
        max_edges: limits.max_edges,
        seed_node_ids,
        nodes,
        edges,
        truncated,
    })
}

fn validate_graph_subgraph_request(
    request: &GraphSubgraphApiRequest,
) -> Result<GraphBuildLimits, String> {
    if request.seed_node_ids.is_empty() {
        return Err("graph exploration requires at least one seedNodeId".to_owned());
    }

    let hops = request.hops.unwrap_or(DEFAULT_GRAPH_HOPS);
    if hops > MAX_GRAPH_HOPS {
        return Err(format!(
            "graph exploration supports hops up to {}; got {}",
            MAX_GRAPH_HOPS, hops
        ));
    }

    let max_nodes = validate_graph_limit(
        request.max_nodes,
        DEFAULT_GRAPH_MAX_NODES,
        MAX_GRAPH_MAX_NODES,
        "maxNodes",
    )?;
    let max_edges = validate_graph_limit(
        request.max_edges,
        DEFAULT_GRAPH_MAX_EDGES,
        MAX_GRAPH_MAX_EDGES,
        "maxEdges",
    )?;
    Ok(GraphBuildLimits {
        hops,
        max_nodes,
        max_edges,
        mode: request.mode.unwrap_or_default(),
    })
}

fn validate_graph_limit(
    requested: Option<usize>,
    default: usize,
    maximum: usize,
    label: &str,
) -> Result<usize, String> {
    let value = requested.unwrap_or(default);
    if value == 0 {
        return Err(format!("{label} must be at least 1"));
    }
    if value > maximum {
        return Err(format!("{label} must be at most {maximum}; got {value}"));
    }
    Ok(value)
}

fn cypher_id_list(ids: &[i64]) -> String {
    ids.iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

fn fetch_graph_nodes_by_ids(
    model: &VelrIfcModel,
    db_node_ids: &[i64],
) -> Result<Vec<GraphNodeQueryRecord>, String> {
    if db_node_ids.is_empty() {
        return Ok(Vec::new());
    }
    let ids = cypher_id_list(db_node_ids);
    let query = format!(
        "MATCH (n) WHERE id(n) IN [{ids}] RETURN id(n) AS db_node_id, n.declared_entity AS declared_entity, n.GlobalId AS global_id, n.Name AS name ORDER BY id(n)"
    );
    let result = model
        .execute_cypher_rows(&query)
        .map_err(|error| format!("failed to load graph nodes by id: {error}"))?;
    parse_graph_node_query_result(&result)
}

fn parse_graph_node_query_result(
    result: &cc_w_velr::CypherQueryResult,
) -> Result<Vec<GraphNodeQueryRecord>, String> {
    let db_node_id_index = required_column_index(&result.columns, &["dbnodeid"], "db_node_id")?;
    let declared_entity_index =
        required_column_index(&result.columns, &["declaredentity"], "declared_entity")?;
    let global_id_index = required_column_index(&result.columns, &["globalid"], "global_id")?;
    let name_index = required_column_index(&result.columns, &["name"], "name")?;

    let mut records = Vec::with_capacity(result.rows.len());
    for row in &result.rows {
        let db_node_id = parse_required_i64_cell(row.get(db_node_id_index), "db_node_id")?;
        let declared_entity = parse_optional_string_cell(row.get(declared_entity_index))
            .unwrap_or_else(|| "IfcEntity".to_owned());
        let global_id = parse_optional_string_cell(row.get(global_id_index));
        let name = parse_optional_string_cell(row.get(name_index));
        records.push(GraphNodeQueryRecord {
            db_node_id,
            declared_entity,
            global_id,
            name,
        });
    }

    Ok(records)
}

fn fetch_incident_graph_edges(
    model: &VelrIfcModel,
    db_node_ids: &[i64],
    max_edges: usize,
) -> Result<(Vec<GraphEdgeQueryRecord>, bool), String> {
    if db_node_ids.is_empty() || max_edges == 0 {
        return Ok((Vec::new(), false));
    }
    let ids = cypher_id_list(db_node_ids);
    let query = format!(
        "MATCH (source)-[rel]->(target) WHERE id(source) IN [{ids}] OR id(target) IN [{ids}] RETURN id(source) AS source_db_node_id, id(target) AS target_db_node_id, type(rel) AS relationship_type ORDER BY id(source), id(target), type(rel) LIMIT {}",
        max_edges + 1
    );
    let result = model
        .execute_cypher_rows(&query)
        .map_err(|error| format!("failed to load incident graph edges: {error}"))?;
    let mut edges = parse_graph_edge_query_result(&result)?;
    let truncated = edges.len() > max_edges;
    if truncated {
        edges.truncate(max_edges);
    }
    Ok((edges, truncated))
}

fn parse_graph_edge_query_result(
    result: &cc_w_velr::CypherQueryResult,
) -> Result<Vec<GraphEdgeQueryRecord>, String> {
    let source_index =
        required_column_index(&result.columns, &["sourcedbnodeid"], "source_db_node_id")?;
    let target_index =
        required_column_index(&result.columns, &["targetdbnodeid"], "target_db_node_id")?;
    let relationship_type_index =
        required_column_index(&result.columns, &["relationshiptype"], "relationship_type")?;

    let mut edges = Vec::with_capacity(result.rows.len());
    for row in &result.rows {
        edges.push(GraphEdgeQueryRecord {
            source_db_node_id: parse_required_i64_cell(row.get(source_index), "source_db_node_id")?,
            target_db_node_id: parse_required_i64_cell(row.get(target_index), "target_db_node_id")?,
            relationship_type: parse_required_string_cell(
                row.get(relationship_type_index),
                "relationship_type",
            )?
            .to_owned(),
        });
    }

    Ok(edges)
}

fn graph_node_from_record(
    record: GraphNodeQueryRecord,
    hop_distance: usize,
    is_seed: bool,
) -> GraphSubgraphNode {
    let display_label = record
        .name
        .clone()
        .or_else(|| record.global_id.clone())
        .unwrap_or_else(|| record.declared_entity.clone());
    GraphSubgraphNode {
        db_node_id: record.db_node_id,
        declared_entity: record.declared_entity,
        global_id: record.global_id,
        name: record.name,
        display_label,
        hop_distance,
        is_seed,
    }
}

fn agent_node_summary_from_record(record: GraphNodeQueryRecord) -> BackendAgentNodeSummary {
    let display_label = record
        .name
        .clone()
        .or_else(|| record.global_id.clone())
        .unwrap_or_else(|| record.declared_entity.clone());
    BackendAgentNodeSummary {
        db_node_id: record.db_node_id,
        declared_entity: record.declared_entity,
        global_id: record.global_id.clone(),
        name: record.name,
        display_label,
        semantic_id: record.global_id,
    }
}

fn fetch_agent_node_summaries(
    model: &VelrIfcModel,
    db_node_ids: &[i64],
) -> Result<Vec<BackendAgentNodeSummary>, String> {
    let ids = dedup_sorted_ids(db_node_ids);
    if ids.len() > DEFAULT_AGENT_DESCRIBE_NODE_LIMIT {
        return Err(format!(
            "describe_nodes supports at most {} ids per call; got {}",
            DEFAULT_AGENT_DESCRIBE_NODE_LIMIT,
            ids.len()
        ));
    }
    Ok(fetch_graph_nodes_by_ids(model, &ids)?
        .into_iter()
        .map(agent_node_summary_from_record)
        .collect())
}

fn fetch_agent_node_properties(
    model: &VelrIfcModel,
    db_node_id: i64,
    max_relations: usize,
) -> Result<BackendAgentNodePropertiesResult, String> {
    let node = fetch_graph_nodes_by_ids(model, &[db_node_id])?
        .into_iter()
        .next()
        .ok_or_else(|| format!("could not find graph node with dbNodeId {db_node_id}"))?;
    let node_summary = agent_node_summary_from_record(node);
    let properties = fetch_agent_node_property_map(model, db_node_id)?;
    let (relations, truncated_relations) =
        fetch_agent_node_relations(model, db_node_id, max_relations)?;
    Ok(BackendAgentNodePropertiesResult {
        node: node_summary,
        properties,
        relations,
        truncated_relations,
    })
}

fn fetch_agent_neighbor_graph(
    model: &VelrIfcModel,
    resource: &str,
    db_node_ids: &[i64],
    hops: usize,
    mode: BackendAgentGraphMode,
) -> Result<BackendAgentNeighborGraph, String> {
    let request = GraphSubgraphApiRequest {
        resource: resource.to_owned(),
        seed_node_ids: dedup_sorted_ids(db_node_ids),
        hops: Some(hops),
        max_nodes: Some(DEFAULT_GRAPH_MAX_NODES),
        max_edges: Some(DEFAULT_GRAPH_MAX_EDGES),
        mode: Some(match mode {
            BackendAgentGraphMode::Raw => GraphSubgraphMode::Raw,
            BackendAgentGraphMode::Semantic => GraphSubgraphMode::Semantic,
        }),
    };
    let response =
        build_graph_subgraph_response(&request, model, validate_graph_subgraph_request(&request)?)?;
    Ok(BackendAgentNeighborGraph {
        mode: match response.mode {
            GraphSubgraphMode::Raw => BackendAgentGraphMode::Raw,
            GraphSubgraphMode::Semantic => BackendAgentGraphMode::Semantic,
        },
        hops: response.hops,
        seed_node_ids: response.seed_node_ids,
        nodes: response
            .nodes
            .into_iter()
            .map(|node| BackendAgentNeighborNode {
                db_node_id: node.db_node_id,
                declared_entity: node.declared_entity,
                global_id: node.global_id,
                name: node.name,
                display_label: node.display_label,
                hop_distance: node.hop_distance,
                is_seed: node.is_seed,
            })
            .collect(),
        edges: response
            .edges
            .into_iter()
            .map(|edge| BackendAgentNeighborEdge {
                edge_id: edge.edge_id,
                source_db_node_id: edge.source_db_node_id,
                target_db_node_id: edge.target_db_node_id,
                relationship_type: edge.relationship_type,
            })
            .collect(),
        truncated: response.truncated,
    })
}

fn fetch_agent_node_property_map(
    model: &VelrIfcModel,
    db_node_id: i64,
) -> Result<BTreeMap<String, String>, String> {
    let query = format!("MATCH (n) WHERE id(n) = {db_node_id} RETURN n LIMIT 1");
    let result = model
        .execute_cypher_rows(&query)
        .map_err(|error| format!("failed to load graph node properties: {error}"))?;
    let node_json = result
        .rows
        .first()
        .and_then(|row| row.first())
        .ok_or_else(|| {
            format!("node property query did not return a node for dbNodeId {db_node_id}")
        })?;
    let node = parse_cypher_node_cell(node_json)?;
    let mut properties = BTreeMap::new();
    for (key, value) in node.properties {
        if matches!(key.as_str(), "declared_entity" | "GlobalId" | "Name") {
            continue;
        }
        properties.insert(key, cypher_node_property_value_to_string(&value));
    }
    Ok(properties)
}

fn fetch_agent_node_relations(
    model: &VelrIfcModel,
    db_node_id: i64,
    max_relations: usize,
) -> Result<(Vec<BackendAgentNodeRelationSummary>, bool), String> {
    let query_limit = max_relations + 1;
    let incoming_query = format!(
        "MATCH (n)<-[rel]-(other) WHERE id(n) = {db_node_id} RETURN 'incoming' AS direction, type(rel) AS relationship_type, id(other) AS other_db_node_id, other.declared_entity AS other_declared_entity, other.GlobalId AS other_global_id, other.Name AS other_name LIMIT {query_limit}"
    );
    let outgoing_query = format!(
        "MATCH (n)-[rel]->(other) WHERE id(n) = {db_node_id} RETURN 'outgoing' AS direction, type(rel) AS relationship_type, id(other) AS other_db_node_id, other.declared_entity AS other_declared_entity, other.GlobalId AS other_global_id, other.Name AS other_name LIMIT {query_limit}"
    );

    let incoming = model
        .execute_cypher_rows(&incoming_query)
        .map_err(|error| format!("failed to load incoming graph node relations: {error}"))?;
    let outgoing = model
        .execute_cypher_rows(&outgoing_query)
        .map_err(|error| format!("failed to load outgoing graph node relations: {error}"))?;

    let mut records = parse_graph_node_relation_query_result(&incoming)?;
    records.extend(parse_graph_node_relation_query_result(&outgoing)?);
    let truncated = records.len() > max_relations;
    if truncated {
        records.truncate(max_relations);
    }
    Ok((
        records
            .into_iter()
            .map(|record| BackendAgentNodeRelationSummary {
                direction: record.direction,
                relationship_type: record.relationship_type,
                other: BackendAgentNodeSummary {
                    db_node_id: record.other_db_node_id,
                    declared_entity: record.other_declared_entity.clone(),
                    global_id: record.other_global_id.clone(),
                    name: record.other_name.clone(),
                    display_label: record
                        .other_name
                        .clone()
                        .or_else(|| record.other_global_id.clone())
                        .unwrap_or_else(|| record.other_declared_entity.clone()),
                    semantic_id: record.other_global_id,
                },
            })
            .collect(),
        truncated,
    ))
}

fn parse_graph_node_relation_query_result(
    result: &cc_w_velr::CypherQueryResult,
) -> Result<Vec<GraphNodeRelationQueryRecord>, String> {
    let direction_index = required_column_index(&result.columns, &["direction"], "direction")?;
    let relationship_type_index =
        required_column_index(&result.columns, &["relationshiptype"], "relationship_type")?;
    let other_db_node_id_index =
        required_column_index(&result.columns, &["otherdbnodeid"], "other_db_node_id")?;
    let other_declared_entity_index = required_column_index(
        &result.columns,
        &["otherdeclaredentity"],
        "other_declared_entity",
    )?;
    let other_global_id_index =
        required_column_index(&result.columns, &["otherglobalid"], "other_global_id")?;
    let other_name_index = required_column_index(&result.columns, &["othername"], "other_name")?;

    let mut records = Vec::with_capacity(result.rows.len());
    for row in &result.rows {
        records.push(GraphNodeRelationQueryRecord {
            direction: parse_required_string_cell(row.get(direction_index), "direction")?
                .to_owned(),
            relationship_type: parse_required_string_cell(
                row.get(relationship_type_index),
                "relationship_type",
            )?
            .to_owned(),
            other_db_node_id: parse_required_i64_cell(
                row.get(other_db_node_id_index),
                "other_db_node_id",
            )?,
            other_declared_entity: parse_optional_string_cell(row.get(other_declared_entity_index))
                .unwrap_or_else(|| "IfcEntity".to_owned()),
            other_global_id: parse_optional_string_cell(row.get(other_global_id_index)),
            other_name: parse_optional_string_cell(row.get(other_name_index)),
        });
    }

    Ok(records)
}

fn parse_cypher_node_cell(value: &str) -> Result<CypherNodeCell, String> {
    serde_json::from_str(value)
        .map_err(|error| format!("invalid node JSON from Cypher result: {error}"))
}

fn cypher_node_property_value_to_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".to_owned(),
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn graph_mode_keeps_node(node: &GraphSubgraphNode, mode: GraphSubgraphMode) -> bool {
    match mode {
        GraphSubgraphMode::Raw => true,
        GraphSubgraphMode::Semantic => {
            node.is_seed || node.global_id.is_some() || node.declared_entity.starts_with("IfcRel")
        }
    }
}

fn dedup_sorted_ids(ids: &[i64]) -> Vec<i64> {
    let mut ids = ids.to_vec();
    ids.sort_unstable();
    ids.dedup();
    ids
}

fn graph_edge_id(
    source_db_node_id: i64,
    relationship_type: &str,
    target_db_node_id: i64,
) -> String {
    format!("{source_db_node_id}:{relationship_type}:{target_db_node_id}")
}

fn required_column_index(
    columns: &[String],
    candidates: &[&str],
    label: &str,
) -> Result<usize, String> {
    find_column_index(columns, candidates)
        .ok_or_else(|| format!("cypher result is missing required column `{label}`"))
}

fn parse_required_i64_cell(cell: Option<&String>, label: &str) -> Result<i64, String> {
    let value = parse_required_string_cell(cell, label)?;
    value
        .trim()
        .parse::<i64>()
        .map_err(|error| format!("invalid integer in `{label}`: {error}"))
}

fn parse_required_string_cell<'a>(
    cell: Option<&'a String>,
    label: &str,
) -> Result<&'a str, String> {
    let value = cell
        .ok_or_else(|| format!("cypher result row is missing `{label}`"))?
        .trim();
    if value.is_empty() {
        return Err(format!("cypher result row has an empty `{label}` value"));
    }
    Ok(value)
}

fn parse_optional_string_cell(cell: Option<&String>) -> Option<String> {
    cell.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    })
}

fn cached_ifc_model(
    state: &ServerState,
    model_slug: &str,
) -> Result<(Arc<VelrIfcModel>, IfcModelCacheStatus), String> {
    let layout = IfcArtifactLayout::new(&state.ifc_artifacts_root, model_slug);
    let database_stamp = database_stamp(&layout.database)?;

    let cached = {
        let cache = state
            .ifc_model_cache
            .lock()
            .map_err(|_| "IFC model cache lock poisoned".to_owned())?;
        cache.get(model_slug).cloned()
    };
    let had_cached_entry = cached.is_some();

    if let Some(cached) = cached {
        if cached.database_stamp == database_stamp {
            return Ok((cached.model, IfcModelCacheStatus::Hit));
        }
    }

    let model = Arc::new(
        VelrIfcModel::open(layout)
            .map_err(|error| format!("failed to open IFC model `{model_slug}`: {error}"))?,
    );
    let cache_status = if had_cached_entry {
        IfcModelCacheStatus::Reloaded
    } else {
        IfcModelCacheStatus::Miss
    };

    let mut cache = state
        .ifc_model_cache
        .lock()
        .map_err(|_| "IFC model cache lock poisoned".to_owned())?;
    cache.insert(
        model_slug.to_owned(),
        CachedIfcModel {
            database_stamp,
            model: Arc::clone(&model),
        },
    );

    Ok((model, cache_status))
}

fn database_stamp(path: &Path) -> Result<DatabaseStamp, String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("failed to inspect database `{}`: {error}", path.display()))?;
    let modified = metadata
        .modified()
        .map_err(|error| {
            format!(
                "failed to inspect database mtime `{}`: {error}",
                path.display()
            )
        })?
        .duration_since(UNIX_EPOCH)
        .map_err(|error| {
            format!(
                "failed to normalize database mtime `{}`: {error}",
                path.display()
            )
        })?;

    Ok(DatabaseStamp {
        bytes: metadata.len(),
        modified_unix_seconds: modified.as_secs(),
        modified_subsec_nanos: modified.subsec_nanos(),
    })
}

fn summarize_query_for_log(query: &str) -> String {
    let collapsed = query.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = collapsed.chars();
    let preview = chars.by_ref().take(160).collect::<String>();
    if chars.next().is_some() {
        format!("{preview}...")
    } else {
        preview
    }
}

fn serve_path(
    stream: &mut TcpStream,
    head_only: bool,
    target: &str,
    root: &Path,
) -> Result<(), String> {
    let relative_path = sanitize_request_path(target)?;
    let file_path = root.join(&relative_path);

    if !file_path.starts_with(root) {
        return write_response(
            stream,
            "403 Forbidden",
            "text/plain; charset=utf-8",
            b"forbidden",
            head_only,
        );
    }

    let bytes = match fs::read(&file_path) {
        Ok(bytes) => bytes,
        Err(_) => {
            return write_response(
                stream,
                "404 Not Found",
                "text/plain; charset=utf-8",
                b"not found",
                head_only,
            );
        }
    };

    write_response(
        stream,
        "200 OK",
        content_type_for_path(&file_path),
        &bytes,
        head_only,
    )
}

fn extract_semantic_element_ids(columns: &[String], rows: &[Vec<String>]) -> Vec<String> {
    let explicit_index = find_column_index(columns, &["semanticelementid", "elementid"]);
    let global_id_index = find_column_index(columns, &["globalid"]);
    let product_id_index = find_column_index(columns, &["productid"]);
    let mut ids = Vec::new();
    let mut seen = HashSet::new();

    for row in rows {
        let candidate = explicit_index
            .and_then(|index| row.get(index))
            .filter(|value| !value.trim().is_empty())
            .cloned()
            .or_else(|| {
                global_id_index
                    .and_then(|index| row.get(index))
                    .filter(|value| !value.trim().is_empty())
                    .cloned()
            })
            .or_else(|| {
                product_id_index
                    .and_then(|index| row.get(index))
                    .filter(|value| !value.trim().is_empty())
                    .cloned()
            });

        if let Some(id) = candidate {
            if seen.insert(id.clone()) {
                ids.push(id);
            }
        }
    }

    ids
}

fn extract_db_node_ids(columns: &[String], rows: &[Vec<String>]) -> Vec<i64> {
    let node_id_index = find_column_index(columns, &["dbnodeid", "nodeid"]);
    let mut ids = Vec::new();
    let mut seen = HashSet::new();

    for row in rows {
        let Some(index) = node_id_index else {
            break;
        };
        let Some(value) = row.get(index) else {
            continue;
        };
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(id) = trimmed.parse::<i64>() else {
            continue;
        };
        if id > 0 && seen.insert(id) {
            ids.push(id);
        }
    }

    ids
}

fn find_column_index(columns: &[String], candidates: &[&str]) -> Option<usize> {
    columns.iter().position(|column| {
        let normalized = normalize_column_name(column);
        candidates.iter().any(|candidate| normalized == *candidate)
    })
}

fn normalize_column_name(name: &str) -> String {
    name.chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(|ch| ch.to_lowercase())
        .collect()
}

fn sanitize_request_path(target: &str) -> Result<PathBuf, String> {
    let path_only = request_path_only(target);
    let trimmed = path_only.trim_start_matches('/');
    let candidate = if trimmed.is_empty() {
        PathBuf::from("index.html")
    } else {
        PathBuf::from(trimmed)
    };

    let mut sanitized = PathBuf::new();
    for component in candidate.components() {
        match component {
            Component::Normal(part) => sanitized.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err("parent directory segments are not allowed".to_owned());
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err("absolute paths are not allowed".to_owned());
            }
        }
    }

    Ok(sanitized)
}

fn content_type_for_path(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("png") => "image/png",
        Some("svg") => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

fn write_response(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
    head_only: bool,
) -> Result<(), String> {
    let headers = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(headers.as_bytes())
        .map_err(|error| error.to_string())?;
    if !head_only {
        stream.write_all(body).map_err(|error| error.to_string())?;
    }
    stream.flush().map_err(|error| error.to_string())
}

fn write_json_response<T>(stream: &mut TcpStream, status: &str, payload: &T) -> Result<(), String>
where
    T: Serialize,
{
    let body = serde_json::to_vec_pretty(payload)
        .map_err(|error| format!("json encode failed: {error}"))?;
    write_response(
        stream,
        status,
        "application/json; charset=utf-8",
        &body,
        false,
    )
}

fn write_json_error(stream: &mut TcpStream, status: &str, error: &str) -> Result<(), String> {
    write_json_response(
        stream,
        status,
        &ApiErrorResponse {
            error: error.to_owned(),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::{
        AgentActionCandidate, AgentReadonlyCypherResult, AgentSession, AgentSessionApiRequest,
        AgentTranscriptEvent, AgentTranscriptEventKind, AgentTurnApiRequest, AgentUiAction,
        AgentBackendErrorCategory, GraphSubgraphMode, ServerState, agent_capabilities_response,
        agent_model_provider, candidate_ports, content_type_for_path, create_agent_session_api,
        agent_turn_selection_summary, dedup_sorted_ids, default_level_for_model,
        discovered_levels_by_model, execute_agent_turn_api, extract_db_node_ids,
        extract_semantic_element_ids, format_user_facing_agent_error, graph_edge_id,
        graph_mode_keeps_node,
        opencode_model_discovery_providers, recent_agent_session_history, request_path_only,
        resolve_agent_backend_for_turn, run_stub_agent_turn, sanitize_request_path,
        summarize_agent_turn_response_transcript,
        validate_agent_action_candidates, validate_agent_readonly_cypher, validate_graph_limit,
    };
    use std::{
        collections::{BTreeMap, HashMap},
        fs,
        path::{Path, PathBuf},
        sync::{Arc, Mutex},
        time::{SystemTime, UNIX_EPOCH},
    };

    use crate::{
        AgentBackend, AgentCapabilityOption, AgentErrorLogger, AgentQueryLogger,
        AgentRuntimeConfig, AgentSessionStore, DEFAULT_AGENT_MAX_READONLY_QUERIES_PER_TURN,
        DEFAULT_AGENT_MAX_ROWS_PER_QUERY, GraphSubgraphNode, NullAgentProgressSink,
    };
    use crate::opencode_executor::{OpencodeDiscoveredModel, OpencodeExecutorConfig};

    fn test_server_state() -> ServerState {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after epoch")
            .as_nanos();
        let ifc_artifacts_root = std::env::temp_dir().join(format!(
            "ccw-agent-server-tests-{}-{}",
            std::process::id(),
            nonce
        ));
        let import_dir = ifc_artifacts_root
            .join("building-architecture")
            .join("import");
        fs::create_dir_all(&import_dir).expect("test import dir");
        fs::write(
            import_dir.join("import-log.txt"),
            "# stdout\nschema: IFC4X3_ADD2\n\n# stderr\n",
        )
        .expect("test import log");

        ServerState {
            root: PathBuf::from("."),
            ifc_artifacts_root: ifc_artifacts_root.clone(),
            ifc_model_cache: Mutex::new(HashMap::new()),
            agent_sessions: Arc::new(Mutex::new(AgentSessionStore::default())),
            agent_turns: Arc::new(Mutex::new(crate::AgentTurnStore::default())),
            agent_runtime: AgentRuntimeConfig {
                backend: AgentBackend::Stub,
                native_server: None,
                models: vec![AgentCapabilityOption {
                    id: "stub/default".to_owned(),
                    label: "stub".to_owned(),
                }],
                levels: vec![AgentCapabilityOption {
                    id: "standard".to_owned(),
                    label: "standard".to_owned(),
                }],
                levels_by_model: BTreeMap::new(),
                default_model_id: Some("stub/default".to_owned()),
                default_level_id: Some("standard".to_owned()),
                max_queries_per_turn: DEFAULT_AGENT_MAX_READONLY_QUERIES_PER_TURN,
                max_rows_per_query: DEFAULT_AGENT_MAX_ROWS_PER_QUERY,
                query_logger: Arc::new(AgentQueryLogger::new(
                    ifc_artifacts_root.join("agent-query-log.jsonl"),
                )),
                error_logger: Arc::new(AgentErrorLogger::new(
                    ifc_artifacts_root.join("agent-error-log.jsonl"),
                )),
            },
        }
    }

    #[test]
    fn request_root_maps_to_index() {
        assert_eq!(sanitize_request_path("/").unwrap(), Path::new("index.html"));
    }

    #[test]
    fn request_keeps_nested_assets() {
        assert_eq!(
            sanitize_request_path("/pkg/cc_w_platform_web.js").unwrap(),
            Path::new("pkg/cc_w_platform_web.js")
        );
    }

    #[test]
    fn request_rejects_parent_segments() {
        assert!(sanitize_request_path("/../secret.txt").is_err());
    }

    #[test]
    fn wasm_uses_wasm_content_type() {
        assert_eq!(
            content_type_for_path(Path::new("pkg/cc_w_platform_web_bg.wasm")),
            "application/wasm"
        );
    }

    #[test]
    fn mjs_uses_javascript_content_type() {
        assert_eq!(
            content_type_for_path(Path::new("vendor/xterm.mjs")),
            "text/javascript; charset=utf-8"
        );
    }

    #[test]
    fn agent_capabilities_surface_current_backend_model_and_level() {
        let state = test_server_state();
        let capabilities = agent_capabilities_response(&state.agent_runtime);

        assert_eq!(capabilities.default_backend_id, "stub");
        assert_eq!(capabilities.default_model_id.as_deref(), Some("stub/default"));
        assert_eq!(capabilities.default_level_id.as_deref(), Some("standard"));
        assert_eq!(capabilities.backends.len(), 1);
        assert_eq!(capabilities.backends[0].models[0].id, "stub/default");
        assert_eq!(capabilities.backends[0].levels[0].id, "standard");
    }

    #[test]
    fn agent_turn_selection_summary_mentions_model_and_level() {
        assert_eq!(
            agent_turn_selection_summary(Some("openai/gpt-5.4"), Some("medium")),
            "Using openai/gpt-5.4 / medium."
        );
        assert_eq!(
            agent_turn_selection_summary(Some("ollama/gemma4:e4b"), None),
            "Using ollama/gemma4:e4b."
        );
    }

    #[test]
    fn agent_model_provider_reads_provider_prefix() {
        assert_eq!(agent_model_provider("openai/gpt-5.4"), Some("openai"));
        assert_eq!(agent_model_provider("gpt-5.4"), None);
    }

    #[test]
    fn provider_whitelist_json_controls_discovery_targets() {
        let root = std::env::temp_dir().join(format!(
            "ccw-provider-whitelist-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be monotonic enough for tests")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("temp dir should be created");
        fs::write(root.join("opencode.json"), "{}").expect("config file should be written");
        fs::write(
            root.join("provider-whitelist.json"),
            "{ \"providers\": [\"openai\", \"ollama\"] }",
        )
        .expect("whitelist file should be written");

        let config = OpencodeExecutorConfig {
            config_path: Some(root.join("opencode.json")),
            ..OpencodeExecutorConfig::default()
        };

        assert_eq!(
            opencode_model_discovery_providers(&config, Some("cloudflare/gpt-5.4")),
            vec!["openai".to_owned(), "ollama".to_owned()]
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn discovered_model_variants_become_model_scoped_levels() {
        let levels = discovered_levels_by_model(&[OpencodeDiscoveredModel {
            id: "openai/gpt-5.4".to_owned(),
            variants: vec!["none".to_owned(), "medium".to_owned(), "high".to_owned()],
        }]);
        let ids = levels
            .get("openai/gpt-5.4")
            .expect("model levels should exist")
            .iter()
            .map(|level| level.id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(ids, vec!["medium", "high"]);
    }

    #[test]
    fn default_level_for_model_uses_middle_discovered_level() {
        let mut state = test_server_state();
        state.agent_runtime.default_level_id = None;
        state.agent_runtime.levels_by_model.insert(
            "openai/gpt-5.4".to_owned(),
            vec![
                AgentCapabilityOption {
                    id: "low".to_owned(),
                    label: "low".to_owned(),
                },
                AgentCapabilityOption {
                    id: "medium".to_owned(),
                    label: "medium".to_owned(),
                },
                AgentCapabilityOption {
                    id: "high".to_owned(),
                    label: "high".to_owned(),
                },
                AgentCapabilityOption {
                    id: "xhigh".to_owned(),
                    label: "xhigh".to_owned(),
                },
            ],
        );

        assert_eq!(
            default_level_for_model(&state.agent_runtime, Some("openai/gpt-5.4")),
            Some("medium")
        );
    }

    #[test]
    fn agent_backend_selection_rejects_unavailable_model() {
        let state = test_server_state();
        let error = resolve_agent_backend_for_turn(
            &state.agent_runtime,
            Some("stub"),
            Some("openai/gpt-5.4"),
            Some("standard"),
        )
        .unwrap_err();

        assert!(error.contains("agent model `openai/gpt-5.4` is not available"));
    }

    #[test]
    fn agent_backend_selection_applies_opencode_model_and_level() {
        let mut state = test_server_state();
        let mut config = OpencodeExecutorConfig::default();
        config.model = Some("openai/gpt-5.4".to_owned());
        config.variant = Some("medium".to_owned());
        state.agent_runtime.backend = AgentBackend::Opencode(config);
        state.agent_runtime.models = vec![
            AgentCapabilityOption {
                id: "openai/gpt-5.4".to_owned(),
                label: "openai/gpt-5.4".to_owned(),
            },
            AgentCapabilityOption {
                id: "openai/gpt-5.4-mini".to_owned(),
                label: "openai/gpt-5.4-mini".to_owned(),
            },
        ];
        state.agent_runtime.levels = vec![
            AgentCapabilityOption {
                id: "medium".to_owned(),
                label: "medium".to_owned(),
            },
            AgentCapabilityOption {
                id: "high".to_owned(),
                label: "high".to_owned(),
            },
        ];
        state.agent_runtime.default_model_id = Some("openai/gpt-5.4".to_owned());
        state.agent_runtime.default_level_id = Some("medium".to_owned());

        let selected = resolve_agent_backend_for_turn(
            &state.agent_runtime,
            Some("opencode"),
            Some("openai/gpt-5.4-mini"),
            Some("high"),
        )
        .expect("selected opencode backend should be valid");

        match selected {
            AgentBackend::Opencode(config) => {
                assert_eq!(config.model.as_deref(), Some("openai/gpt-5.4-mini"));
                assert_eq!(config.variant.as_deref(), Some("high"));
            }
            AgentBackend::Stub => panic!("expected opencode backend"),
        }
    }

    #[test]
    fn timeout_error_message_uses_seconds_and_error_id() {
        let message = format_user_facing_agent_error(
            "ae-123-agent-turn-7",
            AgentBackendErrorCategory::Timeout,
            "opencode turn failed: opencode executable `/tmp/opencode` timed out after 45000 ms",
        );

        assert!(message.contains("timed out after 45 seconds"));
        assert!(message.contains("Please try again shortly."));
        assert!(message.contains("Error ID: ae-123-agent-turn-7"));
        assert!(!message.contains("/tmp/opencode"));
    }

    #[test]
    fn provider_server_error_message_hides_executable_path() {
        let message = format_user_facing_agent_error(
            "ae-456-agent-turn-8",
            AgentBackendErrorCategory::ProviderServer,
            "opencode turn failed: opencode provider error from `/tmp/opencode`: provider returned server_error: upstream issue",
        );

        assert!(message.contains("temporary server issue"));
        assert!(message.contains("Error ID: ae-456-agent-turn-8"));
        assert!(!message.contains("/tmp/opencode"));
    }

    #[test]
    fn candidate_ports_scan_forward_from_requested_port() {
        let ports = candidate_ports(8001).take(4).collect::<Vec<_>>();
        assert_eq!(ports, vec![8001, 8002, 8003, 8004]);
    }

    #[test]
    fn api_path_ignores_query_string() {
        assert_eq!(
            request_path_only("/api/cypher?resource=ifc/building"),
            "/api/cypher"
        );
    }

    #[test]
    fn semantic_id_extraction_prefers_explicit_column() {
        let ids = extract_semantic_element_ids(
            &[String::from("semantic_element_id"), String::from("label")],
            &[
                vec![String::from("A"), String::from("Wall")],
                vec![String::from("A"), String::from("Wall duplicate")],
                vec![String::from("B"), String::from("Door")],
            ],
        );

        assert_eq!(ids, vec!["A", "B"]);
    }

    #[test]
    fn semantic_id_extraction_maps_global_and_product_id_columns() {
        let from_global = extract_semantic_element_ids(
            &[String::from("GlobalId")],
            &[vec![String::from("2xQ$n5SLP5MBLyL442paFx")]],
        );
        assert_eq!(from_global, vec!["2xQ$n5SLP5MBLyL442paFx"]);

        let from_product = extract_semantic_element_ids(
            &[String::from("product_id")],
            &[vec![String::from("42")]],
        );
        assert_eq!(from_product, vec!["42"]);
    }

    #[test]
    fn db_node_id_extraction_maps_node_id_columns() {
        let ids = extract_db_node_ids(
            &[String::from("node_id"), String::from("label")],
            &[
                vec![String::from("395"), String::from("Wall")],
                vec![String::from("395"), String::from("Wall duplicate")],
                vec![String::from("396"), String::from("Door")],
            ],
        );

        assert_eq!(ids, vec![395, 396]);
    }

    #[test]
    fn graph_limit_validation_rejects_zero_and_oversized_values() {
        assert_eq!(validate_graph_limit(None, 12, 50, "maxNodes").unwrap(), 12);
        assert!(validate_graph_limit(Some(0), 12, 50, "maxNodes").is_err());
        assert!(validate_graph_limit(Some(51), 12, 50, "maxNodes").is_err());
    }

    #[test]
    fn graph_mode_semantic_filters_out_non_semantic_internal_nodes() {
        let internal = GraphSubgraphNode {
            db_node_id: 7,
            declared_entity: "IfcCartesianPoint".to_string(),
            global_id: None,
            name: None,
            display_label: "IfcCartesianPoint".to_string(),
            hop_distance: 1,
            is_seed: false,
        };
        let rel = GraphSubgraphNode {
            db_node_id: 8,
            declared_entity: "IfcRelAggregates".to_string(),
            global_id: None,
            name: None,
            display_label: "IfcRelAggregates".to_string(),
            hop_distance: 1,
            is_seed: false,
        };
        let product = GraphSubgraphNode {
            db_node_id: 9,
            declared_entity: "IfcWall".to_string(),
            global_id: Some("0abc".to_string()),
            name: Some("Wall".to_string()),
            display_label: "Wall".to_string(),
            hop_distance: 1,
            is_seed: false,
        };

        assert!(!graph_mode_keeps_node(
            &internal,
            GraphSubgraphMode::Semantic
        ));
        assert!(graph_mode_keeps_node(&rel, GraphSubgraphMode::Semantic));
        assert!(graph_mode_keeps_node(&product, GraphSubgraphMode::Semantic));
    }

    #[test]
    fn graph_edge_ids_are_stable_and_directed() {
        assert_eq!(
            graph_edge_id(7, "RELATING_OBJECT", 9),
            "7:RELATING_OBJECT:9"
        );
        assert_ne!(
            graph_edge_id(7, "RELATING_OBJECT", 9),
            graph_edge_id(9, "RELATING_OBJECT", 7)
        );
    }

    #[test]
    fn graph_seed_ids_are_sorted_and_deduplicated() {
        assert_eq!(dedup_sorted_ids(&[9, 2, 9, 4, 2]), vec![2, 4, 9]);
    }

    #[test]
    fn recent_agent_session_history_prefers_salient_user_and_assistant_events() {
        let session = AgentSession {
            session_id: "agent-session-1".to_owned(),
            resource: "ifc/building-architecture".to_owned(),
            schema_id: "IFC4X3_ADD2".to_owned(),
            schema_slug: Some("ifc4x3_add2".to_owned()),
            opencode_session_id: None,
            turn_count: 3,
            transcript: vec![
                AgentTranscriptEvent::system(
                    "AI session bound to ifc/building-architecture (IFC4X3_ADD2).",
                ),
                AgentTranscriptEvent::user("Is there a kitchen unit in the building?"),
                AgentTranscriptEvent::tool("check furniture instances"),
                AgentTranscriptEvent::assistant(
                    "Yes. I found one furniture element that appears to be a kitchen unit: name `kitchen`, object type `kitchen`, GlobalId `2e9pghUJbBqR4jTInsONQT`.",
                ),
                AgentTranscriptEvent::assistant("Prepared 2 validated UI actions."),
                AgentTranscriptEvent::system("Completed 2 read-only Cypher queries."),
                AgentTranscriptEvent::user("do we have a kitchen in the model ?"),
            ],
        };

        let history = recent_agent_session_history(&session);
        let texts = history
            .iter()
            .map(|event| event.text.as_str())
            .collect::<Vec<_>>();

        assert!(
            texts
                .iter()
                .any(|text| text.contains("GlobalId `2e9pghUJbBqR4jTInsONQT`"))
        );
        assert!(
            texts
                .iter()
                .any(|text| text.contains("do we have a kitchen in the model"))
        );
        assert!(!texts.iter().any(|text| text.starts_with("Prepared ")));
        assert!(!texts.iter().any(|text| text.starts_with("Completed ")));
        assert!(history.iter().all(|event| matches!(
            event.kind,
            AgentTranscriptEventKind::User | AgentTranscriptEventKind::Assistant
        )));
    }

    #[test]
    fn agent_turn_response_summary_prefers_conclusion_and_summary_lines() {
        let transcript = vec![
            AgentTranscriptEvent::tool(
                "ifc_readonly_cypher : quickly determine whether the model is building-centric"
                    .to_owned(),
            ),
            AgentTranscriptEvent::assistant(
                "This looks like a small building-oriented IFC4X3 model.".to_owned(),
            ),
            AgentTranscriptEvent::assistant("Prepared 2 validated UI actions.".to_owned()),
            AgentTranscriptEvent::system("Completed 2 read-only Cypher queries.".to_owned()),
        ];

        let summary = summarize_agent_turn_response_transcript(&transcript);
        let texts = summary
            .iter()
            .map(|event| event.text.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            texts,
            vec![
                "This looks like a small building-oriented IFC4X3 model.",
                "Completed 2 read-only Cypher queries.",
            ]
        );
    }

    #[test]
    fn readonly_agent_cypher_accepts_single_read_query_and_trims_trailing_semicolon() {
        assert_eq!(
            validate_agent_readonly_cypher(" MATCH (n) RETURN id(n) AS node_id; ").unwrap(),
            "MATCH (n) RETURN id(n) AS node_id"
        );
    }

    #[test]
    fn readonly_agent_cypher_rejects_mutating_keywords_and_multiple_statements() {
        assert!(validate_agent_readonly_cypher("MATCH (n) SET n.Name = 'x' RETURN n").is_err());
        assert!(validate_agent_readonly_cypher("MATCH (n) RETURN n; MATCH (m) RETURN m").is_err());
        assert!(validate_agent_readonly_cypher("CALL db.labels()").is_err());
    }

    #[test]
    fn agent_action_validation_rejects_unknown_kinds() {
        let error = validate_agent_action_candidates(vec![AgentActionCandidate {
            kind: "viewer.run_js".to_owned(),
            semantic_ids: Vec::new(),
            db_node_ids: Vec::new(),
        }])
        .unwrap_err();

        assert!(error.contains("unsupported agent UI action kind"));
    }

    #[test]
    fn agent_action_validation_normalizes_whitelisted_payloads() {
        let actions = validate_agent_action_candidates(vec![
            AgentActionCandidate::graph_set_seeds(vec![395, 395, 396]),
            AgentActionCandidate {
                kind: "properties.show_node".to_owned(),
                semantic_ids: Vec::new(),
                db_node_ids: vec![215],
            },
            AgentActionCandidate::elements_hide(vec![
                "A".to_owned(),
                " ".to_owned(),
                "A".to_owned(),
                "B".to_owned(),
            ]),
            AgentActionCandidate::viewer_frame_visible(),
        ])
        .unwrap();

        assert_eq!(
            actions,
            vec![
                AgentUiAction::GraphSetSeeds {
                    db_node_ids: vec![395, 396]
                },
                AgentUiAction::PropertiesShowNode { db_node_id: 215 },
                AgentUiAction::ElementsHide {
                    semantic_ids: vec!["A".to_owned(), "B".to_owned()]
                },
                AgentUiAction::ViewerFrameVisible,
            ]
        );
    }

    #[test]
    fn agent_action_validation_merges_duplicate_actions_within_a_turn() {
        let actions = validate_agent_action_candidates(vec![
            AgentActionCandidate::elements_select(vec!["wall-a".to_owned()]),
            AgentActionCandidate::viewer_frame_visible(),
            AgentActionCandidate::elements_select(vec!["wall-a".to_owned(), "wall-b".to_owned()]),
            AgentActionCandidate::properties_show_node(158),
            AgentActionCandidate::properties_show_node(215),
            AgentActionCandidate::viewer_frame_visible(),
        ])
        .unwrap();

        assert_eq!(
            actions,
            vec![
                AgentUiAction::ElementsSelect {
                    semantic_ids: vec!["wall-a".to_owned(), "wall-b".to_owned()]
                },
                AgentUiAction::ViewerFrameVisible,
                AgentUiAction::PropertiesShowNode { db_node_id: 215 },
            ]
        );
    }

    #[test]
    fn agent_session_creation_requires_ifc_resource() {
        let state = test_server_state();
        let error = create_agent_session_api(
            &AgentSessionApiRequest {
                resource: "demo/triangle".to_owned(),
            },
            &state,
        )
        .unwrap_err();

        assert!(error.contains("agent sessions require an IFC resource"));
    }

    #[test]
    fn agent_turn_uses_session_bound_resource_for_stub_actions() {
        let state = test_server_state();
        let session = create_agent_session_api(
            &AgentSessionApiRequest {
                resource: "ifc/building-architecture".to_owned(),
            },
            &state,
        )
        .unwrap();

        assert_eq!(session.schema_id, "IFC4X3_ADD2");
        assert_eq!(session.schema_slug.as_deref(), Some("ifc4x3_add2"));

        let response = execute_agent_turn_api(
            &AgentTurnApiRequest {
                session_id: session.session_id.clone(),
                input: "hide: wall-a, wall-b".to_owned(),
                backend_id: None,
                model_id: None,
                level_id: None,
            },
            &state,
            &mut NullAgentProgressSink,
        )
        .unwrap();

        assert_eq!(response.session_id, session.session_id);
        assert_eq!(response.resource, "ifc/building-architecture");
        assert_eq!(response.schema_id, "IFC4X3_ADD2");
        assert_eq!(
            response.actions,
            vec![AgentUiAction::ElementsHide {
                semantic_ids: vec!["wall-a".to_owned(), "wall-b".to_owned()]
            }]
        );
    }

    #[test]
    fn agent_turn_rejects_unknown_session() {
        let state = test_server_state();
        let error = execute_agent_turn_api(
            &AgentTurnApiRequest {
                session_id: "agent-session-missing".to_owned(),
                input: "frame".to_owned(),
                backend_id: None,
                model_id: None,
                level_id: None,
            },
            &state,
            &mut NullAgentProgressSink,
        )
        .unwrap_err();

        assert!(error.contains("unknown agent session"));
    }

    #[test]
    fn stub_agent_cypher_turn_can_prepare_graph_and_selection_actions() {
        let execution = run_stub_agent_turn(
            "ifc/building-architecture",
            "cypher: MATCH (w:IfcWall) RETURN id(w) AS node_id, w.GlobalId AS global_id LIMIT 2",
            |_| {
                Ok(AgentReadonlyCypherResult {
                    columns: vec!["node_id".to_owned(), "global_id".to_owned()],
                    rows: vec![
                        vec!["395".to_owned(), "wall-a".to_owned()],
                        vec!["396".to_owned(), "wall-b".to_owned()],
                    ],
                    db_node_ids: vec![395, 396],
                    semantic_element_ids: vec!["wall-a".to_owned(), "wall-b".to_owned()],
                })
            },
        )
        .unwrap();

        let actions = validate_agent_action_candidates(execution.action_candidates).unwrap();
        assert_eq!(
            actions,
            vec![
                AgentUiAction::GraphSetSeeds {
                    db_node_ids: vec![395, 396]
                },
                AgentUiAction::ElementsSelect {
                    semantic_ids: vec!["wall-a".to_owned(), "wall-b".to_owned()]
                },
            ]
        );
    }
}
