use serde::{Deserialize, Serialize};
use serde_json::Value;
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
    fn run_project_readonly_cypher(
        &mut self,
        query: &str,
        why: Option<&str>,
        resource_filter: &[String],
    ) -> Result<AgentReadonlyCypherResult, String> {
        let _ = (query, why, resource_filter);
        Err("project-level read-only Cypher is not implemented for this runtime".to_owned())
    }
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum InspectionUpdateMode {
    Replace,
    Add,
    Remove,
}

impl Default for InspectionUpdateMode {
    fn default() -> Self {
        Self::Replace
    }
}

impl InspectionUpdateMode {
    fn order_key(self) -> u8 {
        match self {
            Self::Replace => 0,
            Self::Add => 1,
            Self::Remove => 2,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AgentUiAction {
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
    #[serde(rename = "elements.set_visible")]
    ElementsSetVisible {
        semantic_ids: Vec<String>,
        visible: bool,
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
    #[serde(rename = "viewer.reset_default_view")]
    ViewerResetDefaultView,
    #[serde(rename = "viewer.section.set")]
    ViewerSectionSet { section: Value },
    #[serde(rename = "viewer.section.clear")]
    ViewerSectionClear,
    #[serde(rename = "viewer.annotations.show_path")]
    ViewerAnnotationsShowPath {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resource: Option<String>,
        path: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        line: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        markers: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mode: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_samples: Option<Value>,
    },
    #[serde(rename = "viewer.annotations.clear")]
    ViewerAnnotationsClear {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resource: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentActionCandidate {
    pub kind: String,
    pub semantic_ids: Vec<String>,
    pub db_node_ids: Vec<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inspection_mode: Option<InspectionUpdateMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotation: Option<Value>,
}

fn is_default_inspection_mode(mode: &InspectionUpdateMode) -> bool {
    *mode == InspectionUpdateMode::Replace
}

impl AgentActionCandidate {
    pub fn graph_set_seeds(db_node_ids: Vec<i64>) -> Self {
        Self {
            kind: "graph.set_seeds".to_owned(),
            semantic_ids: Vec::new(),
            db_node_ids,
            resource: None,
            inspection_mode: None,
            visible: None,
            section: None,
            annotation: None,
        }
    }

    pub fn elements_hide(semantic_ids: Vec<String>) -> Self {
        Self {
            kind: "elements.hide".to_owned(),
            semantic_ids,
            db_node_ids: Vec::new(),
            resource: None,
            inspection_mode: None,
            visible: None,
            section: None,
            annotation: None,
        }
    }

    pub fn elements_show(semantic_ids: Vec<String>) -> Self {
        Self {
            kind: "elements.show".to_owned(),
            semantic_ids,
            db_node_ids: Vec::new(),
            resource: None,
            inspection_mode: None,
            visible: None,
            section: None,
            annotation: None,
        }
    }

    pub fn elements_set_visible(semantic_ids: Vec<String>, visible: bool) -> Self {
        Self {
            kind: "elements.set_visible".to_owned(),
            semantic_ids,
            db_node_ids: Vec::new(),
            resource: None,
            inspection_mode: None,
            visible: Some(visible),
            section: None,
            annotation: None,
        }
    }

    pub fn elements_select(semantic_ids: Vec<String>) -> Self {
        Self {
            kind: "elements.select".to_owned(),
            semantic_ids,
            db_node_ids: Vec::new(),
            resource: None,
            inspection_mode: None,
            visible: None,
            section: None,
            annotation: None,
        }
    }

    pub fn elements_inspect(semantic_ids: Vec<String>) -> Self {
        Self::elements_inspect_with_mode(semantic_ids, InspectionUpdateMode::Replace)
    }

    pub fn elements_inspect_with_mode(
        semantic_ids: Vec<String>,
        mode: InspectionUpdateMode,
    ) -> Self {
        Self {
            kind: "elements.inspect".to_owned(),
            semantic_ids,
            db_node_ids: Vec::new(),
            resource: None,
            inspection_mode: Some(mode),
            visible: None,
            section: None,
            annotation: None,
        }
    }

    pub fn properties_show_node(db_node_id: i64) -> Self {
        Self {
            kind: "properties.show_node".to_owned(),
            semantic_ids: Vec::new(),
            db_node_ids: vec![db_node_id],
            resource: None,
            inspection_mode: None,
            visible: None,
            section: None,
            annotation: None,
        }
    }

    pub fn viewer_frame_visible() -> Self {
        Self {
            kind: "viewer.frame_visible".to_owned(),
            semantic_ids: Vec::new(),
            db_node_ids: Vec::new(),
            resource: None,
            inspection_mode: None,
            visible: None,
            section: None,
            annotation: None,
        }
    }

    pub fn viewer_clear_inspection() -> Self {
        Self {
            kind: "viewer.clear_inspection".to_owned(),
            semantic_ids: Vec::new(),
            db_node_ids: Vec::new(),
            resource: None,
            inspection_mode: None,
            visible: None,
            section: None,
            annotation: None,
        }
    }

    pub fn viewer_reset_default_view() -> Self {
        Self {
            kind: "viewer.reset_default_view".to_owned(),
            semantic_ids: Vec::new(),
            db_node_ids: Vec::new(),
            resource: None,
            inspection_mode: None,
            visible: None,
            section: None,
            annotation: None,
        }
    }

    pub fn viewer_section_set(section: Value) -> Self {
        Self {
            kind: "viewer.section.set".to_owned(),
            semantic_ids: Vec::new(),
            db_node_ids: Vec::new(),
            resource: None,
            inspection_mode: None,
            visible: None,
            section: Some(section),
            annotation: None,
        }
    }

    pub fn viewer_section_clear() -> Self {
        Self {
            kind: "viewer.section.clear".to_owned(),
            semantic_ids: Vec::new(),
            db_node_ids: Vec::new(),
            resource: None,
            inspection_mode: None,
            visible: None,
            section: None,
            annotation: None,
        }
    }

    pub fn viewer_annotations_show_path(annotation: Value) -> Self {
        Self {
            kind: "viewer.annotations.show_path".to_owned(),
            semantic_ids: Vec::new(),
            db_node_ids: Vec::new(),
            resource: None,
            inspection_mode: None,
            visible: None,
            section: None,
            annotation: Some(annotation),
        }
    }

    pub fn viewer_annotations_clear() -> Self {
        Self {
            kind: "viewer.annotations.clear".to_owned(),
            semantic_ids: Vec::new(),
            db_node_ids: Vec::new(),
            resource: None,
            inspection_mode: None,
            visible: None,
            section: None,
            annotation: None,
        }
    }

    pub fn with_resource(mut self, resource: impl Into<String>) -> Self {
        let resource = resource.into();
        if !resource.trim().is_empty() {
            self.resource = Some(resource);
        }
        self
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
            Ok(AgentUiAction::GraphSetSeeds {
                db_node_ids,
                resource: candidate.resource,
            })
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
                resource: candidate.resource,
            })
        }
        "elements.hide" => {
            if !candidate.db_node_ids.is_empty() {
                return Err("elements.hide does not accept dbNodeIds".to_owned());
            }
            let semantic_ids = normalize_agent_semantic_ids(candidate.semantic_ids)?;
            Ok(AgentUiAction::ElementsHide {
                semantic_ids,
                resource: candidate.resource,
            })
        }
        "elements.show" => {
            if !candidate.db_node_ids.is_empty() {
                return Err("elements.show does not accept dbNodeIds".to_owned());
            }
            let semantic_ids = normalize_agent_semantic_ids(candidate.semantic_ids)?;
            Ok(AgentUiAction::ElementsShow {
                semantic_ids,
                resource: candidate.resource,
            })
        }
        "elements.set_visible" => {
            if !candidate.db_node_ids.is_empty() {
                return Err("elements.set_visible does not accept dbNodeIds".to_owned());
            }
            let semantic_ids = normalize_agent_semantic_ids(candidate.semantic_ids)?;
            let visible = candidate
                .visible
                .ok_or_else(|| "elements.set_visible requires visible".to_owned())?;
            Ok(AgentUiAction::ElementsSetVisible {
                semantic_ids,
                visible,
                resource: candidate.resource,
            })
        }
        "elements.select" => {
            if !candidate.db_node_ids.is_empty() {
                return Err("elements.select does not accept dbNodeIds".to_owned());
            }
            let semantic_ids = normalize_agent_semantic_ids(candidate.semantic_ids)?;
            Ok(AgentUiAction::ElementsSelect {
                semantic_ids,
                resource: candidate.resource,
            })
        }
        "elements.inspect" => {
            if !candidate.db_node_ids.is_empty() {
                return Err("elements.inspect does not accept dbNodeIds".to_owned());
            }
            let semantic_ids = normalize_agent_semantic_ids(candidate.semantic_ids)?;
            Ok(AgentUiAction::ElementsInspect {
                semantic_ids,
                resource: candidate.resource,
                mode: candidate.inspection_mode.unwrap_or_default(),
            })
        }
        "viewer.frame_visible" => {
            if !candidate.semantic_ids.is_empty() || !candidate.db_node_ids.is_empty() {
                return Err("viewer.frame_visible does not accept ids".to_owned());
            }
            Ok(AgentUiAction::ViewerFrameVisible)
        }
        "viewer.clear_inspection" => {
            if !candidate.semantic_ids.is_empty() || !candidate.db_node_ids.is_empty() {
                return Err("viewer.clear_inspection does not accept ids".to_owned());
            }
            Ok(AgentUiAction::ViewerClearInspection)
        }
        "viewer.reset_default_view" => {
            if !candidate.semantic_ids.is_empty() || !candidate.db_node_ids.is_empty() {
                return Err("viewer.reset_default_view does not accept ids".to_owned());
            }
            Ok(AgentUiAction::ViewerResetDefaultView)
        }
        "viewer.section.set" => {
            if !candidate.semantic_ids.is_empty() || !candidate.db_node_ids.is_empty() {
                return Err("viewer.section.set does not accept ids".to_owned());
            }
            let section = candidate
                .section
                .ok_or_else(|| "viewer.section.set requires a section object".to_owned())?;
            if !section.is_object() {
                return Err("viewer.section.set requires a section object".to_owned());
            }
            Ok(AgentUiAction::ViewerSectionSet { section })
        }
        "viewer.section.clear" => {
            if !candidate.semantic_ids.is_empty() || !candidate.db_node_ids.is_empty() {
                return Err("viewer.section.clear does not accept ids".to_owned());
            }
            Ok(AgentUiAction::ViewerSectionClear)
        }
        "viewer.annotations.show_path" => {
            if !candidate.semantic_ids.is_empty() || !candidate.db_node_ids.is_empty() {
                return Err("viewer.annotations.show_path does not accept ids".to_owned());
            }
            let annotation = candidate.annotation.ok_or_else(|| {
                "viewer.annotations.show_path requires an annotation object".to_owned()
            })?;
            let annotation = normalize_path_annotation(annotation, candidate.resource.as_deref())?;
            Ok(AgentUiAction::ViewerAnnotationsShowPath {
                resource: annotation.resource,
                path: annotation.path,
                line: annotation.line,
                markers: annotation.markers,
                mode: annotation.mode,
                max_samples: annotation.max_samples,
            })
        }
        "viewer.annotations.clear" => {
            if !candidate.semantic_ids.is_empty() || !candidate.db_node_ids.is_empty() {
                return Err("viewer.annotations.clear does not accept ids".to_owned());
            }
            Ok(AgentUiAction::ViewerAnnotationsClear {
                resource: candidate.resource,
            })
        }
        other => Err(format!("unsupported agent UI action kind `{other}`")),
    }
}

struct PathAnnotationAction {
    resource: Option<String>,
    path: Value,
    line: Option<Value>,
    markers: Option<Value>,
    mode: Option<Value>,
    max_samples: Option<Value>,
}

fn normalize_path_annotation(
    value: Value,
    fallback_resource: Option<&str>,
) -> Result<PathAnnotationAction, String> {
    let Value::Object(object) = value else {
        return Err("viewer.annotations.show_path requires an annotation object".to_owned());
    };
    if object.contains_key("points")
        || object.contains_key("polyline")
        || object.contains_key("polylines")
        || object.contains_key("vertices")
        || object.contains_key("coordinates")
    {
        return Err("viewer.annotations.show_path does not accept raw geometry arrays".to_owned());
    }
    let path = normalize_annotation_value(&object, &["path"])
        .filter(|path| path.is_object())
        .ok_or_else(|| "viewer.annotations.show_path requires path".to_owned())?;
    let resource = get_object_string(&object, &["resource", "source_resource", "sourceResource"])
        .or_else(|| {
            fallback_resource
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        });
    let line = normalize_path_annotation_line(&object);
    let markers = normalize_path_annotation_markers(&object);
    let mode = normalize_annotation_value(&object, &["mode", "operation", "update", "behavior"]);
    let line = if should_drop_default_path_line_for_marker_add(
        mode.as_ref(),
        markers.as_ref(),
        line.as_ref(),
    ) {
        None
    } else {
        line
    };
    Ok(PathAnnotationAction {
        resource,
        path,
        line,
        markers,
        mode,
        max_samples: normalize_annotation_interval(&object, &["max_samples", "maxSamples"])?,
    })
}

fn should_drop_default_path_line_for_marker_add(
    mode: Option<&Value>,
    markers: Option<&Value>,
    line: Option<&Value>,
) -> bool {
    annotation_update_mode_is_add(mode)
        && annotation_markers_are_nonempty(markers)
        && line.is_some_and(annotation_line_is_default_path)
}

fn normalize_path_annotation_line(object: &serde_json::Map<String, Value>) -> Option<Value> {
    normalize_annotation_value(object, &["line"])
        .map(canonical_path_annotation_line)
        .or_else(|| {
            normalize_annotation_value(
                object,
                &[
                    "line_range",
                    "lineRange",
                    "line_ranges",
                    "lineRanges",
                    "ranges",
                ],
            )
            .map(canonical_path_annotation_line)
        })
}

fn normalize_path_annotation_markers(object: &serde_json::Map<String, Value>) -> Option<Value> {
    normalize_annotation_value(object, &["markers", "marker_groups", "markerGroups"])
        .map(canonical_path_annotation_markers)
}

fn canonical_path_annotation_line(value: Value) -> Value {
    match value {
        Value::Array(ranges) => serde_json::json!({
            "ranges": canonical_path_annotation_range_list(ranges)
        }),
        Value::Object(object) if object.get("ranges").is_none() => {
            if let Some(range) = object.get("range").cloned() {
                return serde_json::json!({
                    "ranges": [canonical_path_annotation_measure_range(range)]
                });
            }
            let mut line = serde_json::Map::new();
            line.insert(
                "ranges".to_owned(),
                Value::Array(vec![canonical_path_annotation_measure_range(
                    Value::Object(object),
                )]),
            );
            Value::Object(line)
        }
        Value::Object(mut object) => {
            if let Some(ranges) = object.remove("ranges") {
                let ranges = match ranges {
                    Value::Array(ranges) => {
                        Value::Array(canonical_path_annotation_range_list(ranges))
                    }
                    other => other,
                };
                object.insert("ranges".to_owned(), ranges);
            }
            Value::Object(object)
        }
        other => other,
    }
}

fn canonical_path_annotation_range_list(ranges: Vec<Value>) -> Vec<Value> {
    ranges
        .into_iter()
        .map(canonical_path_annotation_measure_range)
        .collect()
}

fn canonical_path_annotation_markers(value: Value) -> Value {
    match value {
        Value::Array(markers) => Value::Array(
            markers
                .into_iter()
                .map(canonical_path_annotation_marker)
                .collect(),
        ),
        other => other,
    }
}

fn canonical_path_annotation_marker(value: Value) -> Value {
    let Value::Object(mut marker) = value else {
        return value;
    };
    if let Some(range) = marker
        .remove("range")
        .or_else(|| marker.remove("measure_range"))
        .or_else(|| marker.remove("measureRange"))
    {
        marker.insert(
            "range".to_owned(),
            canonical_path_annotation_measure_range(range),
        );
    }
    Value::Object(marker)
}

fn canonical_path_annotation_measure_range(value: Value) -> Value {
    const FROM_KEYS: &[&str] = &["from", "start", "from_measure", "fromMeasure"];
    const TO_KEYS: &[&str] = &["to", "end", "to_measure", "toMeasure"];
    const FROM_OFFSET_KEYS: &[&str] = &["from_offset", "fromOffset", "start_offset", "startOffset"];
    const TO_OFFSET_KEYS: &[&str] = &["to_offset", "toOffset", "end_offset", "endOffset"];
    const TO_END_KEYS: &[&str] = &["to_end", "toEnd", "path_end", "pathEnd"];

    let Value::Object(mut range) = value else {
        return value;
    };

    let from = first_non_null_annotation_value(&range, FROM_KEYS);
    let to = first_non_null_annotation_value(&range, TO_KEYS);
    let from_offset = first_non_null_annotation_value(&range, FROM_OFFSET_KEYS);
    let to_offset = first_non_null_annotation_value(&range, TO_OFFSET_KEYS);
    let to_end = annotation_bool_flag_is_true(&range, TO_END_KEYS);

    remove_annotation_keys(&mut range, FROM_KEYS);
    remove_annotation_keys(&mut range, TO_KEYS);
    remove_annotation_keys(&mut range, FROM_OFFSET_KEYS);
    remove_annotation_keys(&mut range, TO_OFFSET_KEYS);
    remove_annotation_keys(&mut range, TO_END_KEYS);

    if let Some(value) = from {
        range.insert("from".to_owned(), value);
    } else if let Some(value) = from_offset {
        range.insert("from_offset".to_owned(), value);
    }

    if to_end {
        range.insert("to_end".to_owned(), Value::Bool(true));
    } else if let Some(value) = to {
        range.insert("to".to_owned(), value);
    } else if let Some(value) = to_offset {
        range.insert("to_offset".to_owned(), value);
    }

    Value::Object(range)
}

fn first_non_null_annotation_value(
    object: &serde_json::Map<String, Value>,
    keys: &[&str],
) -> Option<Value> {
    keys.iter()
        .find_map(|key| object.get(*key).filter(|value| !value.is_null()).cloned())
}

fn annotation_bool_flag_is_true(object: &serde_json::Map<String, Value>, keys: &[&str]) -> bool {
    keys.iter()
        .any(|key| matches!(object.get(*key), Some(Value::Bool(true))))
}

fn remove_annotation_keys(object: &mut serde_json::Map<String, Value>, keys: &[&str]) {
    for key in keys {
        object.remove(*key);
    }
}

fn annotation_update_mode_is_add(mode: Option<&Value>) -> bool {
    let Some(Value::String(mode)) = mode else {
        return false;
    };
    matches!(
        mode.trim().to_ascii_lowercase().as_str(),
        "add" | "append" | "include" | "plus"
    )
}

fn annotation_markers_are_nonempty(markers: Option<&Value>) -> bool {
    matches!(markers, Some(Value::Array(markers)) if !markers.is_empty())
}

fn annotation_line_is_default_path(line: &Value) -> bool {
    let Value::Object(line) = line else {
        return false;
    };
    if line.is_empty() {
        return true;
    }
    if line.len() == 1 {
        return line
            .get("ranges")
            .is_some_and(annotation_line_ranges_are_default_path);
    }
    false
}

fn annotation_line_ranges_are_default_path(ranges: &Value) -> bool {
    match ranges {
        Value::Null => true,
        Value::Array(ranges) => {
            ranges.is_empty() || ranges.iter().all(annotation_measure_range_is_default_path)
        }
        _ => false,
    }
}

fn annotation_measure_range_is_default_path(range: &Value) -> bool {
    match range {
        Value::Null => true,
        Value::Object(range) => ![
            "from",
            "start",
            "from_measure",
            "fromMeasure",
            "to",
            "end",
            "to_measure",
            "toMeasure",
            "from_offset",
            "fromOffset",
            "start_offset",
            "startOffset",
            "to_offset",
            "toOffset",
            "end_offset",
            "endOffset",
            "to_end",
            "toEnd",
            "path_end",
            "pathEnd",
        ]
        .iter()
        .any(|key| range.get(*key).is_some_and(|value| !value.is_null())),
        _ => false,
    }
}

fn get_object_string(object: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        object.get(*key).and_then(|value| match value {
            Value::String(value) => {
                let trimmed = value.trim();
                (!trimmed.is_empty()).then(|| trimmed.to_owned())
            }
            Value::Number(value) => Some(value.to_string()),
            _ => None,
        })
    })
}

fn normalize_annotation_interval(
    object: &serde_json::Map<String, Value>,
    keys: &[&str],
) -> Result<Option<Value>, String> {
    let Some((key, value)) = keys
        .iter()
        .find_map(|key| object.get(*key).map(|value| (*key, value)))
    else {
        return Ok(None);
    };
    match value {
        Value::Null => Ok(None),
        Value::Number(number) => {
            let Some(value) = number.as_f64() else {
                return Err(format!("viewer.annotations.show_path {key} must be finite"));
            };
            if !value.is_finite() || value <= 0.0 {
                return Err(format!(
                    "viewer.annotations.show_path {key} must be positive"
                ));
            }
            Ok(Some(Value::Number(number.clone())))
        }
        Value::String(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                return Err(format!(
                    "viewer.annotations.show_path {key} must be non-empty"
                ));
            }
            Ok(Some(Value::String(trimmed.to_owned())))
        }
        _ => Err(format!(
            "viewer.annotations.show_path {key} must be a number or string"
        )),
    }
}

fn normalize_annotation_value(
    object: &serde_json::Map<String, Value>,
    keys: &[&str],
) -> Option<Value> {
    keys.iter()
        .find_map(|key| object.get(*key).filter(|value| !value.is_null()).cloned())
}

fn normalize_agent_ui_actions(actions: Vec<AgentUiAction>) -> Vec<AgentUiAction> {
    let mut merged_graph_seed_groups = Vec::<(Option<String>, Vec<i64>)>::new();
    let mut graph_seed_seen = HashSet::new();
    let mut merged_hide_groups = Vec::<(Option<String>, Vec<String>)>::new();
    let mut hide_seen = HashSet::new();
    let mut merged_show_groups = Vec::<(Option<String>, Vec<String>)>::new();
    let mut show_seen = HashSet::new();
    let mut merged_set_visible_groups = Vec::<(bool, Option<String>, Vec<String>)>::new();
    let mut set_visible_seen = HashSet::new();
    let mut merged_select_groups = Vec::<(Option<String>, Vec<String>)>::new();
    let mut select_seen = HashSet::new();
    let mut merged_inspect_groups =
        Vec::<(InspectionUpdateMode, Option<String>, Vec<String>)>::new();
    let mut inspect_seen = HashSet::new();
    let mut latest_properties_node = None::<(i64, Option<String>)>;
    let mut graph_set_seeds_present = false;
    let mut properties_show_present = false;
    let mut elements_hide_present = false;
    let mut elements_show_present = false;
    let mut elements_set_visible_present = false;
    let mut elements_select_present = false;
    let mut inspect_modes_present = HashSet::new();
    let mut frame_visible_present = false;
    let mut clear_inspection_present = false;
    let mut reset_default_view_present = false;
    let mut section_set_present = false;
    let mut section_clear_present = false;
    let mut latest_section = None::<Value>;
    let mut annotations_show_present = false;
    let mut annotations_clear_present = false;
    let mut latest_path_annotation = None::<(
        Option<String>,
        Value,
        Option<Value>,
        Option<Value>,
        Option<Value>,
        Option<Value>,
    )>;
    let mut latest_annotations_clear_resource = None::<Option<String>>;
    let mut order = Vec::new();

    for action in actions {
        match action {
            AgentUiAction::GraphSetSeeds {
                db_node_ids,
                resource,
            } => {
                if !graph_set_seeds_present {
                    order.push(0u8);
                    graph_set_seeds_present = true;
                }
                let group_index = merged_graph_seed_groups
                    .iter()
                    .position(|(existing_resource, _)| existing_resource == &resource)
                    .unwrap_or_else(|| {
                        merged_graph_seed_groups.push((resource.clone(), Vec::new()));
                        merged_graph_seed_groups.len() - 1
                    });
                for db_node_id in db_node_ids {
                    if graph_seed_seen.insert((resource.clone(), db_node_id)) {
                        merged_graph_seed_groups[group_index].1.push(db_node_id);
                    }
                }
            }
            AgentUiAction::PropertiesShowNode {
                db_node_id,
                resource,
            } => {
                if !properties_show_present {
                    order.push(1u8);
                    properties_show_present = true;
                }
                latest_properties_node = Some((db_node_id, resource));
            }
            AgentUiAction::ElementsHide {
                semantic_ids,
                resource,
            } => {
                if !elements_hide_present {
                    order.push(2u8);
                    elements_hide_present = true;
                }
                for semantic_id in semantic_ids {
                    push_semantic_action_group(
                        &mut merged_hide_groups,
                        &mut hide_seen,
                        resource.clone(),
                        semantic_id,
                    );
                }
            }
            AgentUiAction::ElementsShow {
                semantic_ids,
                resource,
            } => {
                if !elements_show_present {
                    order.push(3u8);
                    elements_show_present = true;
                }
                for semantic_id in semantic_ids {
                    push_semantic_action_group(
                        &mut merged_show_groups,
                        &mut show_seen,
                        resource.clone(),
                        semantic_id,
                    );
                }
            }
            AgentUiAction::ElementsSetVisible {
                semantic_ids,
                visible,
                resource,
            } => {
                if !elements_set_visible_present {
                    order.push(4u8);
                    elements_set_visible_present = true;
                }
                let group_index = merged_set_visible_groups
                    .iter()
                    .position(|(existing_visible, existing_resource, _)| {
                        existing_visible == &visible && existing_resource == &resource
                    })
                    .unwrap_or_else(|| {
                        merged_set_visible_groups.push((visible, resource.clone(), Vec::new()));
                        merged_set_visible_groups.len() - 1
                    });
                for semantic_id in semantic_ids {
                    if set_visible_seen.insert((visible, resource.clone(), semantic_id.clone())) {
                        merged_set_visible_groups[group_index].2.push(semantic_id);
                    }
                }
            }
            AgentUiAction::ElementsSelect {
                semantic_ids,
                resource,
            } => {
                if !elements_select_present {
                    order.push(5u8);
                    elements_select_present = true;
                }
                for semantic_id in semantic_ids {
                    push_semantic_action_group(
                        &mut merged_select_groups,
                        &mut select_seen,
                        resource.clone(),
                        semantic_id,
                    );
                }
            }
            AgentUiAction::ElementsInspect {
                semantic_ids,
                resource,
                mode,
            } => {
                if inspect_modes_present.insert(mode) {
                    order.push(6u8 + mode.order_key());
                }
                for semantic_id in semantic_ids {
                    push_inspection_action_group(
                        &mut merged_inspect_groups,
                        &mut inspect_seen,
                        mode,
                        resource.clone(),
                        semantic_id,
                    );
                }
            }
            AgentUiAction::ViewerFrameVisible => {
                if !frame_visible_present {
                    order.push(9u8);
                    frame_visible_present = true;
                }
            }
            AgentUiAction::ViewerClearInspection => {
                if !clear_inspection_present {
                    order.push(10u8);
                    clear_inspection_present = true;
                }
            }
            AgentUiAction::ViewerResetDefaultView => {
                if !reset_default_view_present {
                    order.push(11u8);
                    reset_default_view_present = true;
                }
            }
            AgentUiAction::ViewerSectionSet { section } => {
                if !section_set_present {
                    order.push(12u8);
                    section_set_present = true;
                }
                latest_section = Some(section);
            }
            AgentUiAction::ViewerSectionClear => {
                if !section_clear_present {
                    order.push(13u8);
                    section_clear_present = true;
                }
            }
            AgentUiAction::ViewerAnnotationsShowPath {
                resource,
                path,
                line,
                markers,
                mode,
                max_samples,
            } => {
                if !annotations_show_present {
                    order.push(14u8);
                    annotations_show_present = true;
                }
                latest_path_annotation = Some((resource, path, line, markers, mode, max_samples));
            }
            AgentUiAction::ViewerAnnotationsClear { resource } => {
                if !annotations_clear_present {
                    order.push(15u8);
                    annotations_clear_present = true;
                }
                latest_annotations_clear_resource = Some(resource);
            }
        }
    }

    let mut normalized = Vec::new();
    for kind in order {
        match kind {
            0 if merged_graph_seed_groups
                .iter()
                .any(|(_, ids)| !ids.is_empty()) =>
            {
                for (resource, db_node_ids) in &merged_graph_seed_groups {
                    if !db_node_ids.is_empty() {
                        normalized.push(AgentUiAction::GraphSetSeeds {
                            db_node_ids: db_node_ids.clone(),
                            resource: resource.clone(),
                        });
                    }
                }
            }
            1 => {
                if let Some((db_node_id, resource)) = latest_properties_node.clone() {
                    normalized.push(AgentUiAction::PropertiesShowNode {
                        db_node_id,
                        resource,
                    });
                }
            }
            2 if merged_hide_groups.iter().any(|(_, ids)| !ids.is_empty()) => {
                for (resource, semantic_ids) in &merged_hide_groups {
                    if !semantic_ids.is_empty() {
                        normalized.push(AgentUiAction::ElementsHide {
                            semantic_ids: semantic_ids.clone(),
                            resource: resource.clone(),
                        });
                    }
                }
            }
            3 if merged_show_groups.iter().any(|(_, ids)| !ids.is_empty()) => {
                for (resource, semantic_ids) in &merged_show_groups {
                    if !semantic_ids.is_empty() {
                        normalized.push(AgentUiAction::ElementsShow {
                            semantic_ids: semantic_ids.clone(),
                            resource: resource.clone(),
                        });
                    }
                }
            }
            4 if merged_set_visible_groups
                .iter()
                .any(|(_, _, ids)| !ids.is_empty()) =>
            {
                for (visible, resource, semantic_ids) in &merged_set_visible_groups {
                    if !semantic_ids.is_empty() {
                        normalized.push(AgentUiAction::ElementsSetVisible {
                            semantic_ids: semantic_ids.clone(),
                            visible: *visible,
                            resource: resource.clone(),
                        });
                    }
                }
            }
            5 if merged_select_groups.iter().any(|(_, ids)| !ids.is_empty()) => {
                for (resource, semantic_ids) in &merged_select_groups {
                    if !semantic_ids.is_empty() {
                        normalized.push(AgentUiAction::ElementsSelect {
                            semantic_ids: semantic_ids.clone(),
                            resource: resource.clone(),
                        });
                    }
                }
            }
            6..=8 => {
                let mode = match kind {
                    6 => InspectionUpdateMode::Replace,
                    7 => InspectionUpdateMode::Add,
                    8 => InspectionUpdateMode::Remove,
                    _ => unreachable!(),
                };
                let semantic_ids = merged_inspect_groups
                    .iter()
                    .filter(|(group_mode, _, ids)| *group_mode == mode && !ids.is_empty())
                    .flat_map(|(_, resource, ids)| {
                        ids.iter().map(move |semantic_id| match resource {
                            Some(resource) if !semantic_id.contains("::") => {
                                format!("{resource}::{semantic_id}")
                            }
                            _ => semantic_id.clone(),
                        })
                    })
                    .collect::<Vec<_>>();
                if !semantic_ids.is_empty() {
                    normalized.push(AgentUiAction::ElementsInspect {
                        semantic_ids,
                        resource: None,
                        mode,
                    });
                }
            }
            9 => normalized.push(AgentUiAction::ViewerFrameVisible),
            10 => normalized.push(AgentUiAction::ViewerClearInspection),
            11 => normalized.push(AgentUiAction::ViewerResetDefaultView),
            12 => {
                if let Some(section) = latest_section.clone() {
                    normalized.push(AgentUiAction::ViewerSectionSet { section });
                }
            }
            13 => normalized.push(AgentUiAction::ViewerSectionClear),
            14 => {
                if let Some((resource, path, line, markers, mode, max_samples)) =
                    latest_path_annotation.clone()
                {
                    normalized.push(AgentUiAction::ViewerAnnotationsShowPath {
                        resource,
                        path,
                        line,
                        markers,
                        mode,
                        max_samples,
                    });
                }
            }
            15 => normalized.push(AgentUiAction::ViewerAnnotationsClear {
                resource: latest_annotations_clear_resource.clone().flatten(),
            }),
            _ => {}
        }
    }

    normalized
}

fn push_semantic_action_group(
    groups: &mut Vec<(Option<String>, Vec<String>)>,
    seen: &mut HashSet<(Option<String>, String)>,
    resource: Option<String>,
    semantic_id: String,
) {
    let group_index = groups
        .iter()
        .position(|(existing_resource, _)| existing_resource == &resource)
        .unwrap_or_else(|| {
            groups.push((resource.clone(), Vec::new()));
            groups.len() - 1
        });
    if seen.insert((resource, semantic_id.clone())) {
        groups[group_index].1.push(semantic_id);
    }
}

fn push_inspection_action_group(
    groups: &mut Vec<(InspectionUpdateMode, Option<String>, Vec<String>)>,
    seen: &mut HashSet<(InspectionUpdateMode, Option<String>, String)>,
    mode: InspectionUpdateMode,
    resource: Option<String>,
    semantic_id: String,
) {
    let group_index = groups
        .iter()
        .position(|(existing_mode, existing_resource, _)| {
            *existing_mode == mode && existing_resource == &resource
        })
        .unwrap_or_else(|| {
            groups.push((mode, resource.clone(), Vec::new()));
            groups.len() - 1
        });
    if seen.insert((mode, resource, semantic_id.clone())) {
        groups[group_index].2.push(semantic_id);
    }
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
        AgentRelationReference, AgentSchemaContext, AgentUiAction, InspectionUpdateMode,
        NullAgentProgressSink, StubAgentExecutor, validate_agent_action_candidates,
        validate_agent_readonly_cypher,
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
            resource: None,
            inspection_mode: None,
            visible: None,
            section: None,
            annotation: None,
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
            AgentActionCandidate::elements_set_visible(
                vec!["hidden-spatial-a".to_owned(), "hidden-spatial-a".to_owned()],
                true,
            )
            .with_resource("ifc/building-architecture"),
            AgentActionCandidate::elements_inspect(vec!["C".to_owned(), "C".to_owned()])
                .with_resource("ifc/building-hvac"),
            AgentActionCandidate::viewer_frame_visible(),
            AgentActionCandidate::viewer_clear_inspection(),
            AgentActionCandidate::viewer_reset_default_view(),
            AgentActionCandidate::viewer_section_set(serde_json::json!({
                "pose": {
                    "origin": [0.0, 0.0, 0.0],
                    "tangent": [1.0, 0.0, 0.0],
                    "normal": [0.0, 1.0, 0.0],
                    "up": [0.0, 0.0, 1.0]
                },
                "width": 10.0,
                "height": 5.0,
                "thickness": 0.2,
                "mode": "3d-overlay"
            })),
            AgentActionCandidate::viewer_section_clear(),
        ])
        .unwrap();

        assert_eq!(
            actions,
            vec![
                AgentUiAction::GraphSetSeeds {
                    db_node_ids: vec![395, 396],
                    resource: None,
                },
                AgentUiAction::PropertiesShowNode {
                    db_node_id: 215,
                    resource: None,
                },
                AgentUiAction::ElementsHide {
                    semantic_ids: vec!["A".to_owned(), "B".to_owned()],
                    resource: None,
                },
                AgentUiAction::ElementsSetVisible {
                    semantic_ids: vec!["hidden-spatial-a".to_owned()],
                    visible: true,
                    resource: Some("ifc/building-architecture".to_owned()),
                },
                AgentUiAction::ElementsInspect {
                    semantic_ids: vec!["ifc/building-hvac::C".to_owned()],
                    resource: None,
                    mode: InspectionUpdateMode::Replace,
                },
                AgentUiAction::ViewerFrameVisible,
                AgentUiAction::ViewerClearInspection,
                AgentUiAction::ViewerResetDefaultView,
                AgentUiAction::ViewerSectionSet {
                    section: serde_json::json!({
                        "pose": {
                            "origin": [0.0, 0.0, 0.0],
                            "tangent": [1.0, 0.0, 0.0],
                            "normal": [0.0, 1.0, 0.0],
                            "up": [0.0, 0.0, 1.0]
                        },
                        "width": 10.0,
                        "height": 5.0,
                        "thickness": 0.2,
                        "mode": "3d-overlay"
                    }),
                },
                AgentUiAction::ViewerSectionClear,
            ]
        );
    }

    #[test]
    fn elements_set_visible_requires_explicit_visibility() {
        let error = validate_agent_action_candidates(vec![AgentActionCandidate {
            kind: "elements.set_visible".to_owned(),
            semantic_ids: vec!["wall-a".to_owned()],
            db_node_ids: Vec::new(),
            resource: None,
            inspection_mode: None,
            visible: None,
            section: None,
            annotation: None,
        }])
        .unwrap_err();

        assert!(error.contains("elements.set_visible requires visible"));
    }

    #[test]
    fn viewer_section_set_accepts_section_pose_payload_without_ids() {
        let section = serde_json::json!({
            "pose": {
                "origin": [12.0, 34.0, 5.0],
                "tangent": [1.0, 0.0, 0.0],
                "normal": [0.0, 1.0, 0.0],
                "up": [0.0, 0.0, 1.0]
            },
            "width": 18.0,
            "height": 7.5,
            "thickness": 0.35
        });

        let actions =
            validate_agent_action_candidates(vec![AgentActionCandidate::viewer_section_set(
                section.clone(),
            )])
            .unwrap();

        assert_eq!(
            actions,
            vec![AgentUiAction::ViewerSectionSet {
                section: section.clone()
            }]
        );
        assert_eq!(
            serde_json::to_value(&actions[0]).unwrap(),
            serde_json::json!({
                "kind": "viewer.section.set",
                "section": section
            })
        );

        let error = validate_agent_action_candidates(vec![AgentActionCandidate {
            kind: "viewer.section.set".to_owned(),
            semantic_ids: vec!["wall-a".to_owned()],
            db_node_ids: vec![12],
            resource: None,
            inspection_mode: None,
            visible: None,
            section: Some(serde_json::json!({})),
            annotation: None,
        }])
        .unwrap_err();

        assert!(error.contains("viewer.section.set does not accept ids"));
    }

    #[test]
    fn viewer_annotations_show_path_accepts_composable_payload() {
        let actions = validate_agent_action_candidates(vec![
            AgentActionCandidate::viewer_annotations_show_path(serde_json::json!({
                "resource": "ifc/infra-road",
                "mode": "add",
                "path": { "kind": "ifc_alignment", "id": "curve:42", "measure": "station" },
                "line": {
                    "ranges": [
                        { "from": 0.0, "to": 120.0 },
                        { "from": 120.0, "to_end": true }
                    ]
                },
                "markers": [
                    { "range": { "from": 0.0, "to": 120.0 }, "every": 20.0, "label": "measure" },
                    { "range": { "from": 120.0, "to_end": true }, "every": 50.0, "label": "measure" }
                ]
            })),
            AgentActionCandidate::viewer_annotations_clear().with_resource("ifc/infra-road"),
        ])
        .unwrap();

        assert_eq!(
            actions,
            vec![
                AgentUiAction::ViewerAnnotationsShowPath {
                    resource: Some("ifc/infra-road".to_owned()),
                    path: serde_json::json!({
                        "kind": "ifc_alignment",
                        "id": "curve:42",
                        "measure": "station"
                    }),
                    line: Some(serde_json::json!({
                        "ranges": [
                            { "from": 0.0, "to": 120.0 },
                            { "from": 120.0, "to_end": true }
                        ]
                    })),
                    markers: Some(serde_json::json!([
                        { "range": { "from": 0.0, "to": 120.0 }, "every": 20.0, "label": "measure" },
                        { "range": { "from": 120.0, "to_end": true }, "every": 50.0, "label": "measure" }
                    ])),
                    mode: Some(serde_json::json!("add")),
                    max_samples: None,
                },
                AgentUiAction::ViewerAnnotationsClear {
                    resource: Some("ifc/infra-road".to_owned()),
                },
            ]
        );
    }

    #[test]
    fn viewer_annotations_show_path_accepts_line_range_shorthand() {
        let actions = validate_agent_action_candidates(vec![
            AgentActionCandidate::viewer_annotations_show_path(serde_json::json!({
                "resource": "ifc/infra-road",
                "path": { "kind": "ifc_alignment", "id": "curve:42", "measure": "station" },
                "line_ranges": [{ "from": 100.0, "to": 200.0 }]
            })),
        ])
        .unwrap();

        assert_eq!(
            actions,
            vec![AgentUiAction::ViewerAnnotationsShowPath {
                resource: Some("ifc/infra-road".to_owned()),
                path: serde_json::json!({
                    "kind": "ifc_alignment",
                    "id": "curve:42",
                    "measure": "station"
                }),
                line: Some(serde_json::json!({
                    "ranges": [{ "from": 100.0, "to": 200.0 }]
                })),
                markers: None,
                mode: None,
                max_samples: None,
            }]
        );
    }

    #[test]
    fn viewer_annotations_show_path_canonicalizes_ai_payload_ranges_before_rendering() {
        let normalize = |annotation: serde_json::Value| -> AgentUiAction {
            let mut actions = validate_agent_action_candidates(vec![
                AgentActionCandidate::viewer_annotations_show_path(annotation),
            ])
            .unwrap();
            assert_eq!(actions.len(), 1);
            actions.remove(0)
        };
        let path = serde_json::json!({
            "kind": "ifc_alignment",
            "id": "curve:215711",
            "measure": "station"
        });

        assert_eq!(
            normalize(serde_json::json!({
                "resource": "ifc/bridge-for-minnd",
                "path": path.clone(),
                "line": {
                    "ranges": [
                        { "from": 100, "from_offset": 100, "to": 200, "to_offset": 200 }
                    ]
                }
            })),
            AgentUiAction::ViewerAnnotationsShowPath {
                resource: Some("ifc/bridge-for-minnd".to_owned()),
                path: path.clone(),
                line: Some(serde_json::json!({
                    "ranges": [{ "from": 100, "to": 200 }]
                })),
                markers: None,
                mode: None,
                max_samples: None,
            }
        );

        assert_eq!(
            normalize(serde_json::json!({
                "resource": "ifc/bridge-for-minnd",
                "path": path.clone(),
                "line_range": { "fromOffset": 0, "toOffset": 120 },
                "markers": [
                    { "range": { "fromOffset": 0, "toOffset": 120 }, "every": 20, "label": "measure" }
                ]
            })),
            AgentUiAction::ViewerAnnotationsShowPath {
                resource: Some("ifc/bridge-for-minnd".to_owned()),
                path: path.clone(),
                line: Some(serde_json::json!({
                    "ranges": [{ "from_offset": 0, "to_offset": 120 }]
                })),
                markers: Some(serde_json::json!([
                    { "range": { "from_offset": 0, "to_offset": 120 }, "every": 20, "label": "measure" }
                ])),
                mode: None,
                max_samples: None,
            }
        );

        assert_eq!(
            normalize(serde_json::json!({
                "resource": "ifc/bridge-for-minnd",
                "mode": "add",
                "path": path.clone(),
                "line": {
                    "ranges": [
                        { "from": 400, "to": 500, "toEnd": true }
                    ]
                },
                "markers": [
                    { "measureRange": { "from": 400, "to": 500, "to_offset": 500, "pathEnd": true }, "every": 50, "label": "measure" }
                ]
            })),
            AgentUiAction::ViewerAnnotationsShowPath {
                resource: Some("ifc/bridge-for-minnd".to_owned()),
                path,
                line: Some(serde_json::json!({
                    "ranges": [{ "from": 400, "to_end": true }]
                })),
                markers: Some(serde_json::json!([
                    { "range": { "from": 400, "to_end": true }, "every": 50, "label": "measure" }
                ])),
                mode: Some(serde_json::json!("add")),
                max_samples: None,
            }
        );
    }

    #[test]
    fn viewer_annotations_show_path_drops_default_line_for_marker_adds() {
        for line in [
            serde_json::json!({}),
            serde_json::json!({ "ranges": [] }),
            serde_json::json!({ "ranges": [{}] }),
        ] {
            let actions = validate_agent_action_candidates(vec![
                AgentActionCandidate::viewer_annotations_show_path(serde_json::json!({
                    "resource": "ifc/infra-road",
                    "mode": "add",
                    "path": { "kind": "ifc_alignment", "id": "curve:42", "measure": "station" },
                    "line": line,
                    "markers": [
                        { "range": { "from": 120.0 }, "every": 50.0, "label": "measure" }
                    ]
                })),
            ])
            .unwrap();

            assert_eq!(
                actions,
                vec![AgentUiAction::ViewerAnnotationsShowPath {
                    resource: Some("ifc/infra-road".to_owned()),
                    path: serde_json::json!({
                        "kind": "ifc_alignment",
                        "id": "curve:42",
                        "measure": "station"
                    }),
                    line: None,
                    markers: Some(serde_json::json!([
                        { "range": { "from": 120.0 }, "every": 50.0, "label": "measure" }
                    ])),
                    mode: Some(serde_json::json!("add")),
                    max_samples: None,
                }]
            );
        }
    }

    #[test]
    fn viewer_annotations_show_path_rejects_geometry_payloads() {
        let error = validate_agent_action_candidates(vec![
            AgentActionCandidate::viewer_annotations_show_path(serde_json::json!({
                "path": { "kind": "ifc_alignment", "id": "curve:42" },
                "polyline": [[0, 0, 0], [1, 0, 0]]
            })),
        ])
        .unwrap_err();

        assert!(error.contains("does not accept raw geometry arrays"));
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
                    semantic_ids: vec!["wall-a".to_owned(), "wall-b".to_owned()],
                    resource: None,
                },
                AgentUiAction::ViewerFrameVisible,
                AgentUiAction::PropertiesShowNode {
                    db_node_id: 215,
                    resource: None,
                },
            ]
        );
    }

    #[test]
    fn agent_action_validation_keeps_graph_seed_resources_separate() {
        let actions = validate_agent_action_candidates(vec![
            AgentActionCandidate::graph_set_seeds(vec![12, 12, 13]).with_resource("ifc/infra-road"),
            AgentActionCandidate::graph_set_seeds(vec![12]).with_resource("ifc/infra-bridge"),
            AgentActionCandidate::properties_show_node(99).with_resource("ifc/infra-bridge"),
        ])
        .unwrap();

        assert_eq!(
            actions,
            vec![
                AgentUiAction::GraphSetSeeds {
                    db_node_ids: vec![12, 13],
                    resource: Some("ifc/infra-road".to_owned()),
                },
                AgentUiAction::GraphSetSeeds {
                    db_node_ids: vec![12],
                    resource: Some("ifc/infra-bridge".to_owned()),
                },
                AgentUiAction::PropertiesShowNode {
                    db_node_id: 99,
                    resource: Some("ifc/infra-bridge".to_owned()),
                },
            ]
        );
    }

    #[test]
    fn agent_action_validation_keeps_element_action_resources_separate() {
        let actions = validate_agent_action_candidates(vec![
            AgentActionCandidate::elements_select(vec!["same-id".to_owned(), "same-id".to_owned()])
                .with_resource("ifc/infra-road"),
            AgentActionCandidate::elements_select(vec!["same-id".to_owned()])
                .with_resource("ifc/infra-bridge"),
        ])
        .unwrap();

        assert_eq!(
            actions,
            vec![
                AgentUiAction::ElementsSelect {
                    semantic_ids: vec!["same-id".to_owned()],
                    resource: Some("ifc/infra-road".to_owned()),
                },
                AgentUiAction::ElementsSelect {
                    semantic_ids: vec!["same-id".to_owned()],
                    resource: Some("ifc/infra-bridge".to_owned()),
                },
            ]
        );
    }

    #[test]
    fn agent_action_validation_merges_inspection_focus_across_resources() {
        let actions = validate_agent_action_candidates(vec![
            AgentActionCandidate::elements_inspect(vec!["hvac-a".to_owned()])
                .with_resource("ifc/building-hvac"),
            AgentActionCandidate::elements_inspect(vec!["arch-a".to_owned(), "arch-a".to_owned()])
                .with_resource("ifc/building-architecture"),
        ])
        .unwrap();

        assert_eq!(
            actions,
            vec![AgentUiAction::ElementsInspect {
                semantic_ids: vec![
                    "ifc/building-hvac::hvac-a".to_owned(),
                    "ifc/building-architecture::arch-a".to_owned(),
                ],
                resource: None,
                mode: InspectionUpdateMode::Replace,
            }]
        );
    }

    #[test]
    fn agent_action_validation_preserves_inspection_update_modes() {
        let actions = validate_agent_action_candidates(vec![
            AgentActionCandidate::elements_inspect_with_mode(
                vec!["kitchen".to_owned()],
                InspectionUpdateMode::Add,
            )
            .with_resource("ifc/building-architecture"),
            AgentActionCandidate::elements_inspect_with_mode(
                vec!["old-hvac".to_owned()],
                InspectionUpdateMode::Remove,
            )
            .with_resource("ifc/building-hvac"),
        ])
        .unwrap();

        assert_eq!(
            actions,
            vec![
                AgentUiAction::ElementsInspect {
                    semantic_ids: vec!["ifc/building-architecture::kitchen".to_owned()],
                    resource: None,
                    mode: InspectionUpdateMode::Add,
                },
                AgentUiAction::ElementsInspect {
                    semantic_ids: vec!["ifc/building-hvac::old-hvac".to_owned()],
                    resource: None,
                    mode: InspectionUpdateMode::Remove,
                },
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
                semantic_ids: vec!["wall-a".to_owned(), "wall-b".to_owned()],
                resource: None,
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
                    db_node_ids: vec![395, 396],
                    resource: None,
                },
                AgentUiAction::ElementsSelect {
                    semantic_ids: vec!["wall-a".to_owned(), "wall-b".to_owned()],
                    resource: None,
                },
            ]
        );
        assert_eq!(response.queries_executed, 1);
    }
}
