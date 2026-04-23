use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

const MAX_AGENT_ACTIONS: usize = 16;
const MAX_AGENT_ACTION_IDS: usize = 2_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentBackendTurnRequest {
    pub resource: String,
    pub schema_id: String,
    pub schema_slug: Option<String>,
    pub input: String,
    #[serde(default)]
    pub session_history: Vec<AgentTranscriptEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentBackendTurnResponse {
    pub transcript: Vec<AgentTranscriptEvent>,
    pub action_candidates: Vec<AgentActionCandidate>,
    pub queries_executed: usize,
}

pub trait AgentReadonlyCypherRuntime {
    fn run_readonly_cypher(
        &mut self,
        query: &str,
        why: Option<&str>,
    ) -> Result<AgentReadonlyCypherResult, String>;
    fn get_schema_context(&mut self) -> Result<AgentSchemaContext, String>;
    fn get_entity_reference(
        &mut self,
        entity_names: &[String],
    ) -> Result<Vec<AgentEntityReference>, String>;
    fn get_query_playbook(
        &mut self,
        goal: &str,
        entity_names: &[String],
    ) -> Result<Vec<AgentQueryPlaybook>, String>;
    fn get_relation_reference(
        &mut self,
        relation_names: &[String],
    ) -> Result<Vec<AgentRelationReference>, String>;
    fn describe_nodes(&mut self, db_node_ids: &[i64]) -> Result<Vec<AgentNodeSummary>, String>;
    fn get_node_properties(&mut self, db_node_id: i64)
    -> Result<AgentNodePropertiesResult, String>;
    fn get_neighbors(
        &mut self,
        db_node_ids: &[i64],
        hops: usize,
        mode: AgentGraphMode,
    ) -> Result<AgentNeighborGraph, String>;
}

pub trait AgentProgressSink {
    fn emit(&mut self, event: AgentTranscriptEvent);
}

#[derive(Debug, Default)]
pub struct NullAgentProgressSink;

impl AgentProgressSink for NullAgentProgressSink {
    fn emit(&mut self, _event: AgentTranscriptEvent) {}
}

pub trait AgentExecutor {
    fn execute_turn(
        &mut self,
        request: &AgentBackendTurnRequest,
        runtime: &mut dyn AgentReadonlyCypherRuntime,
        progress: &mut dyn AgentProgressSink,
    ) -> Result<AgentBackendTurnResponse, String>;
}

pub struct FnReadonlyCypherRuntime<F> {
    callback: F,
}

impl<F> FnReadonlyCypherRuntime<F> {
    pub fn new(callback: F) -> Self {
        Self { callback }
    }
}

impl<F> AgentReadonlyCypherRuntime for FnReadonlyCypherRuntime<F>
where
    F: FnMut(&str) -> Result<AgentReadonlyCypherResult, String>,
{
    fn run_readonly_cypher(
        &mut self,
        query: &str,
        _why: Option<&str>,
    ) -> Result<AgentReadonlyCypherResult, String> {
        (self.callback)(query)
    }

    fn get_schema_context(&mut self) -> Result<AgentSchemaContext, String> {
        Err("get_schema_context is not implemented for this runtime".to_owned())
    }

    fn get_entity_reference(
        &mut self,
        _entity_names: &[String],
    ) -> Result<Vec<AgentEntityReference>, String> {
        Err("get_entity_reference is not implemented for this runtime".to_owned())
    }

    fn get_query_playbook(
        &mut self,
        _goal: &str,
        _entity_names: &[String],
    ) -> Result<Vec<AgentQueryPlaybook>, String> {
        Err("get_query_playbook is not implemented for this runtime".to_owned())
    }

    fn get_relation_reference(
        &mut self,
        _relation_names: &[String],
    ) -> Result<Vec<AgentRelationReference>, String> {
        Err("get_relation_reference is not implemented for this runtime".to_owned())
    }

    fn describe_nodes(&mut self, _db_node_ids: &[i64]) -> Result<Vec<AgentNodeSummary>, String> {
        Err("describe_nodes is not implemented for this runtime".to_owned())
    }

    fn get_node_properties(
        &mut self,
        _db_node_id: i64,
    ) -> Result<AgentNodePropertiesResult, String> {
        Err("get_node_properties is not implemented for this runtime".to_owned())
    }

    fn get_neighbors(
        &mut self,
        _db_node_ids: &[i64],
        _hops: usize,
        _mode: AgentGraphMode,
    ) -> Result<AgentNeighborGraph, String> {
        Err("get_neighbors is not implemented for this runtime".to_owned())
    }
}

#[derive(Debug, Default)]
pub struct StubAgentExecutor;

impl AgentExecutor for StubAgentExecutor {
    fn execute_turn(
        &mut self,
        request: &AgentBackendTurnRequest,
        runtime: &mut dyn AgentReadonlyCypherRuntime,
        _progress: &mut dyn AgentProgressSink,
    ) -> Result<AgentBackendTurnResponse, String> {
        run_stub_agent_turn(request, runtime)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentTranscriptEvent {
    pub kind: AgentTranscriptEventKind,
    pub text: String,
}

impl AgentTranscriptEvent {
    pub fn system(text: impl Into<String>) -> Self {
        Self {
            kind: AgentTranscriptEventKind::System,
            text: text.into(),
        }
    }

    pub fn user(text: impl Into<String>) -> Self {
        Self {
            kind: AgentTranscriptEventKind::User,
            text: text.into(),
        }
    }

    pub fn tool(text: impl Into<String>) -> Self {
        Self {
            kind: AgentTranscriptEventKind::Tool,
            text: text.into(),
        }
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            kind: AgentTranscriptEventKind::Assistant,
            text: text.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AgentTranscriptEventKind {
    System,
    User,
    Tool,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AgentUiAction {
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentActionCandidate {
    pub kind: String,
    pub semantic_ids: Vec<String>,
    pub db_node_ids: Vec<i64>,
}

impl AgentActionCandidate {
    pub fn graph_set_seeds(db_node_ids: Vec<i64>) -> Self {
        Self {
            kind: "graph.set_seeds".to_owned(),
            semantic_ids: Vec::new(),
            db_node_ids,
        }
    }

    pub fn elements_hide(semantic_ids: Vec<String>) -> Self {
        Self {
            kind: "elements.hide".to_owned(),
            semantic_ids,
            db_node_ids: Vec::new(),
        }
    }

    pub fn elements_show(semantic_ids: Vec<String>) -> Self {
        Self {
            kind: "elements.show".to_owned(),
            semantic_ids,
            db_node_ids: Vec::new(),
        }
    }

    pub fn elements_select(semantic_ids: Vec<String>) -> Self {
        Self {
            kind: "elements.select".to_owned(),
            semantic_ids,
            db_node_ids: Vec::new(),
        }
    }

    pub fn properties_show_node(db_node_id: i64) -> Self {
        Self {
            kind: "properties.show_node".to_owned(),
            semantic_ids: Vec::new(),
            db_node_ids: vec![db_node_id],
        }
    }

    pub fn viewer_frame_visible() -> Self {
        Self {
            kind: "viewer.frame_visible".to_owned(),
            semantic_ids: Vec::new(),
            db_node_ids: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentReadonlyCypherResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub db_node_ids: Vec<i64>,
    pub semantic_element_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentSchemaContext {
    pub schema_id: String,
    pub schema_slug: Option<String>,
    pub summary: String,
    pub cautions: Vec<String>,
    pub query_habits: Vec<String>,
    pub query_playbooks: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentEntityReference {
    pub entity_name: String,
    pub summary: String,
    pub common_relations: Vec<String>,
    pub query_patterns: Vec<String>,
    pub cautions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentQueryPlaybook {
    pub playbook_name: String,
    pub summary: String,
    pub when_to_use: Vec<String>,
    pub recommended_patterns: Vec<String>,
    pub related_entities: Vec<String>,
    pub cautions: Vec<String>,
    pub avoid_patterns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentRelationReference {
    pub relation_name: String,
    pub summary: String,
    pub common_connections: Vec<String>,
    pub query_patterns: Vec<String>,
    pub cautions: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AgentGraphMode {
    Raw,
    Semantic,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentNodeSummary {
    pub db_node_id: i64,
    pub declared_entity: String,
    pub global_id: Option<String>,
    pub name: Option<String>,
    pub display_label: String,
    pub semantic_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentNodeRelationSummary {
    pub direction: String,
    pub relationship_type: String,
    pub other: AgentNodeSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentNodePropertiesResult {
    pub node: AgentNodeSummary,
    pub properties: BTreeMap<String, String>,
    pub relations: Vec<AgentNodeRelationSummary>,
    pub truncated_relations: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentNeighborNode {
    pub db_node_id: i64,
    pub declared_entity: String,
    pub global_id: Option<String>,
    pub name: Option<String>,
    pub display_label: String,
    pub hop_distance: usize,
    pub is_seed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentNeighborEdge {
    pub edge_id: String,
    pub source_db_node_id: i64,
    pub target_db_node_id: i64,
    pub relationship_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentNeighborGraph {
    pub mode: AgentGraphMode,
    pub hops: usize,
    pub seed_node_ids: Vec<i64>,
    pub nodes: Vec<AgentNeighborNode>,
    pub edges: Vec<AgentNeighborEdge>,
    pub truncated: bool,
}

pub fn run_stub_agent_turn(
    request: &AgentBackendTurnRequest,
    runtime: &mut dyn AgentReadonlyCypherRuntime,
) -> Result<AgentBackendTurnResponse, String> {
    let trimmed = request.input.trim();
    if trimmed.eq_ignore_ascii_case("help") {
        return Ok(AgentBackendTurnResponse {
            transcript: vec![AgentTranscriptEvent::assistant(
                "Stub agent commands: `cypher: ...`, `graph: 1,2`, `hide: id1,id2`, `show: ...`, `select: ...`, `frame`."
                    .to_owned(),
            )],
            action_candidates: Vec::new(),
            queries_executed: 0,
        });
    }

    if trimmed.eq_ignore_ascii_case("frame") {
        return Ok(AgentBackendTurnResponse {
            transcript: vec![AgentTranscriptEvent::assistant(
                "Prepared viewer.frame_visible.".to_owned(),
            )],
            action_candidates: vec![AgentActionCandidate::viewer_frame_visible()],
            queries_executed: 0,
        });
    }

    if let Some(raw_ids) = trimmed.strip_prefix("graph:") {
        let db_node_ids = parse_agent_db_node_ids(raw_ids)?;
        return Ok(AgentBackendTurnResponse {
            transcript: vec![AgentTranscriptEvent::assistant(format!(
                "Prepared graph.set_seeds for {} node{} in {}.",
                db_node_ids.len(),
                plural_suffix(db_node_ids.len()),
                request.resource
            ))],
            action_candidates: vec![AgentActionCandidate::graph_set_seeds(db_node_ids)],
            queries_executed: 0,
        });
    }

    if let Some(raw_ids) = trimmed.strip_prefix("hide:") {
        let semantic_ids = parse_agent_semantic_ids(raw_ids)?;
        return Ok(AgentBackendTurnResponse {
            transcript: vec![AgentTranscriptEvent::assistant(format!(
                "Prepared elements.hide for {} element{}.",
                semantic_ids.len(),
                plural_suffix(semantic_ids.len())
            ))],
            action_candidates: vec![AgentActionCandidate::elements_hide(semantic_ids)],
            queries_executed: 0,
        });
    }

    if let Some(raw_ids) = trimmed.strip_prefix("show:") {
        let semantic_ids = parse_agent_semantic_ids(raw_ids)?;
        return Ok(AgentBackendTurnResponse {
            transcript: vec![AgentTranscriptEvent::assistant(format!(
                "Prepared elements.show for {} element{}.",
                semantic_ids.len(),
                plural_suffix(semantic_ids.len())
            ))],
            action_candidates: vec![AgentActionCandidate::elements_show(semantic_ids)],
            queries_executed: 0,
        });
    }

    if let Some(raw_ids) = trimmed.strip_prefix("select:") {
        let semantic_ids = parse_agent_semantic_ids(raw_ids)?;
        return Ok(AgentBackendTurnResponse {
            transcript: vec![AgentTranscriptEvent::assistant(format!(
                "Prepared elements.select for {} element{}.",
                semantic_ids.len(),
                plural_suffix(semantic_ids.len())
            ))],
            action_candidates: vec![AgentActionCandidate::elements_select(semantic_ids)],
            queries_executed: 0,
        });
    }

    if let Some(raw_query) = trimmed.strip_prefix("cypher:") {
        let query = validate_agent_readonly_cypher(raw_query)?;
        let mut transcript = vec![AgentTranscriptEvent::tool(format!(
            "Running read-only Cypher against {}.",
            request.resource
        ))];
        let result = runtime.run_readonly_cypher(
            &query,
            Some("Execute the exact user-provided read-only Cypher query."),
        )?;
        transcript.push(AgentTranscriptEvent::assistant(format!(
            "Query returned {} row{} across {} column{}.",
            result.rows.len(),
            plural_suffix(result.rows.len()),
            result.columns.len(),
            plural_suffix(result.columns.len())
        )));

        let mut action_candidates = Vec::new();
        if !result.db_node_ids.is_empty() {
            transcript.push(AgentTranscriptEvent::assistant(format!(
                "Prepared graph.set_seeds from {} returned node id{}.",
                result.db_node_ids.len(),
                plural_suffix(result.db_node_ids.len())
            )));
            action_candidates.push(AgentActionCandidate::graph_set_seeds(
                result.db_node_ids.clone(),
            ));
        }
        if !result.semantic_element_ids.is_empty() {
            transcript.push(AgentTranscriptEvent::assistant(format!(
                "Prepared elements.select from {} returned semantic id{}.",
                result.semantic_element_ids.len(),
                plural_suffix(result.semantic_element_ids.len())
            )));
            action_candidates.push(AgentActionCandidate::elements_select(
                result.semantic_element_ids.clone(),
            ));
        }

        return Ok(AgentBackendTurnResponse {
            transcript,
            action_candidates,
            queries_executed: 1,
        });
    }

    Ok(AgentBackendTurnResponse {
        transcript: vec![AgentTranscriptEvent::assistant(
            "Stub agent only understands `cypher: ...`, `graph: ...`, `hide: ...`, `show: ...`, `select: ...`, `frame`, and `help` for now."
                .to_owned(),
        )],
        action_candidates: Vec::new(),
        queries_executed: 0,
    })
}

pub fn validate_agent_readonly_cypher(query: &str) -> Result<String, String> {
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

pub fn validate_agent_action_candidates(
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

pub fn validate_agent_action_candidate(
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

pub fn normalize_agent_semantic_ids(ids: Vec<String>) -> Result<Vec<String>, String> {
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

pub fn normalize_agent_db_node_ids(ids: Vec<i64>) -> Result<Vec<i64>, String> {
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

pub fn parse_agent_semantic_ids(raw_ids: &str) -> Result<Vec<String>, String> {
    normalize_agent_semantic_ids(
        raw_ids
            .split(',')
            .map(|value| value.trim().to_owned())
            .collect::<Vec<_>>(),
    )
}

pub fn parse_agent_db_node_ids(raw_ids: &str) -> Result<Vec<i64>, String> {
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

fn plural_suffix(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

#[cfg(test)]
mod tests {
    use super::{
        AgentActionCandidate, AgentBackendTurnRequest, AgentBackendTurnResponse,
        AgentEntityReference, AgentExecutor, AgentGraphMode, AgentNeighborGraph,
        AgentNodePropertiesResult, AgentNodeSummary, AgentQueryPlaybook, AgentReadonlyCypherResult,
        AgentRelationReference, AgentSchemaContext, AgentUiAction, NullAgentProgressSink,
        StubAgentExecutor, validate_agent_action_candidates, validate_agent_readonly_cypher,
    };
    use std::collections::BTreeMap;

    struct RecordingRuntime {
        last_query: Option<String>,
        result: AgentReadonlyCypherResult,
    }

    impl super::AgentReadonlyCypherRuntime for RecordingRuntime {
        fn run_readonly_cypher(
            &mut self,
            query: &str,
            _why: Option<&str>,
        ) -> Result<AgentReadonlyCypherResult, String> {
            self.last_query = Some(query.to_owned());
            Ok(self.result.clone())
        }

        fn describe_nodes(
            &mut self,
            _db_node_ids: &[i64],
        ) -> Result<Vec<AgentNodeSummary>, String> {
            Err("not needed in this test".to_owned())
        }

        fn get_schema_context(&mut self) -> Result<AgentSchemaContext, String> {
            Err("not needed in this test".to_owned())
        }

        fn get_entity_reference(
            &mut self,
            _entity_names: &[String],
        ) -> Result<Vec<AgentEntityReference>, String> {
            Err("not needed in this test".to_owned())
        }

        fn get_query_playbook(
            &mut self,
            _goal: &str,
            _entity_names: &[String],
        ) -> Result<Vec<AgentQueryPlaybook>, String> {
            Err("not needed in this test".to_owned())
        }

        fn get_relation_reference(
            &mut self,
            _relation_names: &[String],
        ) -> Result<Vec<AgentRelationReference>, String> {
            Err("not needed in this test".to_owned())
        }

        fn get_node_properties(
            &mut self,
            _db_node_id: i64,
        ) -> Result<AgentNodePropertiesResult, String> {
            Ok(AgentNodePropertiesResult {
                node: AgentNodeSummary {
                    db_node_id: 0,
                    declared_entity: "IfcEntity".to_owned(),
                    global_id: None,
                    name: None,
                    display_label: "IfcEntity".to_owned(),
                    semantic_id: None,
                },
                properties: BTreeMap::new(),
                relations: Vec::new(),
                truncated_relations: false,
            })
        }

        fn get_neighbors(
            &mut self,
            _db_node_ids: &[i64],
            _hops: usize,
            _mode: AgentGraphMode,
        ) -> Result<AgentNeighborGraph, String> {
            Err("not needed in this test".to_owned())
        }
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
            AgentActionCandidate::properties_show_node(215),
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
    fn stub_agent_direct_commands_prepare_expected_actions() {
        let mut executor = StubAgentExecutor;
        let mut runtime = RecordingRuntime {
            last_query: None,
            result: AgentReadonlyCypherResult {
                columns: Vec::new(),
                rows: Vec::new(),
                db_node_ids: Vec::new(),
                semantic_element_ids: Vec::new(),
            },
        };

        let response = executor
            .execute_turn(
                &AgentBackendTurnRequest {
                    resource: "ifc/building-architecture".to_owned(),
                    schema_id: "IFC4X3_ADD2".to_owned(),
                    schema_slug: Some("ifc4x3_add2".to_owned()),
                    input: "hide: wall-a, wall-b".to_owned(),
                    session_history: Vec::new(),
                },
                &mut runtime,
                &mut NullAgentProgressSink,
            )
            .unwrap();

        assert_eq!(runtime.last_query, None);
        assert_eq!(
            validate_agent_action_candidates(response.action_candidates).unwrap(),
            vec![AgentUiAction::ElementsHide {
                semantic_ids: vec!["wall-a".to_owned(), "wall-b".to_owned()]
            }]
        );
    }

    #[test]
    fn stub_agent_cypher_turn_can_prepare_graph_and_selection_actions() {
        let mut executor = StubAgentExecutor;
        let mut runtime = RecordingRuntime {
            last_query: None,
            result: AgentReadonlyCypherResult {
                columns: vec!["node_id".to_owned(), "global_id".to_owned()],
                rows: vec![
                    vec!["395".to_owned(), "wall-a".to_owned()],
                    vec!["396".to_owned(), "wall-b".to_owned()],
                ],
                db_node_ids: vec![395, 396],
                semantic_element_ids: vec!["wall-a".to_owned(), "wall-b".to_owned()],
            },
        };

        let response: AgentBackendTurnResponse = executor
            .execute_turn(
                &AgentBackendTurnRequest {
                    resource: "ifc/building-architecture".to_owned(),
                    schema_id: "IFC4X3_ADD2".to_owned(),
                    schema_slug: Some("ifc4x3_add2".to_owned()),
                    input:
                        "cypher: MATCH (w:IfcWall) RETURN id(w) AS node_id, w.GlobalId AS global_id LIMIT 2"
                            .to_owned(),
                    session_history: Vec::new(),
                },
                &mut runtime,
                &mut NullAgentProgressSink,
            )
            .unwrap();

        assert_eq!(
            runtime.last_query.as_deref(),
            Some("MATCH (w:IfcWall) RETURN id(w) AS node_id, w.GlobalId AS global_id LIMIT 2")
        );
        assert_eq!(
            validate_agent_action_candidates(response.action_candidates).unwrap(),
            vec![
                AgentUiAction::GraphSetSeeds {
                    db_node_ids: vec![395, 396]
                },
                AgentUiAction::ElementsSelect {
                    semantic_ids: vec!["wall-a".to_owned(), "wall-b".to_owned()]
                },
            ]
        );
        assert_eq!(response.queries_executed, 1);
    }
}
