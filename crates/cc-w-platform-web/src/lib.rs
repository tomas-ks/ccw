use cc_w_backend::{GeometryBackend, GeometryBackendError, ResourceError};
use cc_w_render::NullRenderBackend;
#[cfg(target_arch = "wasm32")]
use cc_w_runtime::RuntimeSceneState;
use cc_w_runtime::{DemoAsset, Engine, GeometryPackageSource, GeometryPackageSourceError};
use cc_w_types::GeometryStartViewRequest;
use cc_w_types::{
    Bounds3, DefaultRenderClass, DisplayColor, ExternalId, GeometryCatalog,
    GeometryDefinitionBatch, GeometryDefinitionBatchRequest, GeometryDefinitionCatalogEntry,
    GeometryDefinitionId, GeometryElementCatalogEntry, GeometryInstanceBatch,
    GeometryInstanceBatchRequest, GeometryInstanceCatalogEntry, GeometryInstanceId,
    GeometryStreamPlan, PreparedGeometryDefinition, PreparedGeometryElement,
    PreparedGeometryInstance, PreparedGeometryPackage, PreparedMesh, PreparedVertex,
    SemanticElementId,
};
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

#[cfg(target_arch = "wasm32")]
const DEFAULT_WEB_RESOURCE: &str = "ifc/building-architecture";
const DEFAULT_DEMO_RESOURCE: &str = "demo/revolved-solid";
#[cfg(target_arch = "wasm32")]
const WEB_GEOMETRY_BATCH_CHUNK_SIZE: usize = 5_000;
const SOURCE_SCOPED_ID_SEPARATOR: &str = "::";
#[cfg(target_arch = "wasm32")]
const PROJECT_GEOMETRY_LOCAL_ID_BITS: u32 = 48;
#[cfg(target_arch = "wasm32")]
const PROJECT_GEOMETRY_LOCAL_ID_MASK: u64 = (1u64 << PROJECT_GEOMETRY_LOCAL_ID_BITS) - 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceScopedIdParseError {
    MissingSeparator,
    EmptyResource,
    EmptyLocalId,
    InvalidInstanceId(std::num::ParseIntError),
    InvalidDefinitionId(std::num::ParseIntError),
}

impl fmt::Display for SourceScopedIdParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingSeparator => write!(
                f,
                "source-scoped id must be formatted as `resource::local_id`"
            ),
            Self::EmptyResource => write!(f, "source-scoped id resource must not be empty"),
            Self::EmptyLocalId => write!(f, "source-scoped id local id must not be empty"),
            Self::InvalidInstanceId(error) => {
                write!(f, "invalid source-scoped instance id: {error}")
            }
            Self::InvalidDefinitionId(error) => {
                write!(f, "invalid source-scoped definition id: {error}")
            }
        }
    }
}

impl std::error::Error for SourceScopedIdParseError {}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceScopedSemanticElementId {
    pub resource: String,
    pub local_id: String,
}

impl SourceScopedSemanticElementId {
    pub fn new(resource: impl Into<String>, local_id: impl Into<String>) -> Self {
        Self {
            resource: resource.into(),
            local_id: local_id.into(),
        }
    }

    pub fn from_semantic_element_id(resource: impl Into<String>, id: &SemanticElementId) -> Self {
        Self::new(resource, id.as_str())
    }

    pub fn resource(&self) -> &str {
        &self.resource
    }

    pub fn local_id(&self) -> &str {
        &self.local_id
    }

    pub fn as_semantic_element_id(&self) -> SemanticElementId {
        SemanticElementId::new(self.local_id.clone())
    }

    pub fn validate(&self) -> Result<(), SourceScopedIdParseError> {
        validate_source_scoped_parts(&self.resource, &self.local_id)
    }
}

impl fmt::Display for SourceScopedSemanticElementId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}{SOURCE_SCOPED_ID_SEPARATOR}{}",
            self.resource, self.local_id
        )
    }
}

impl FromStr for SourceScopedSemanticElementId {
    type Err = SourceScopedIdParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (resource, local_id) = parse_source_scoped_parts(value)?;
        Ok(Self::new(resource, local_id))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceScopedGeometryInstanceId {
    pub resource: String,
    pub local_id: u64,
}

impl SourceScopedGeometryInstanceId {
    pub fn new(resource: impl Into<String>, local_id: impl Into<u64>) -> Self {
        Self {
            resource: resource.into(),
            local_id: local_id.into(),
        }
    }

    pub fn from_geometry_instance_id(resource: impl Into<String>, id: GeometryInstanceId) -> Self {
        Self::new(resource, id.0)
    }

    pub fn resource(&self) -> &str {
        &self.resource
    }

    pub fn local_id(&self) -> u64 {
        self.local_id
    }

    pub fn as_geometry_instance_id(&self) -> GeometryInstanceId {
        GeometryInstanceId(self.local_id)
    }

    pub fn validate(&self) -> Result<(), SourceScopedIdParseError> {
        validate_source_scoped_parts(&self.resource, &self.local_id.to_string())
    }
}

impl fmt::Display for SourceScopedGeometryInstanceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}{SOURCE_SCOPED_ID_SEPARATOR}{}",
            self.resource, self.local_id
        )
    }
}

impl FromStr for SourceScopedGeometryInstanceId {
    type Err = SourceScopedIdParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (resource, local_id) = parse_source_scoped_parts(value)?;
        let local_id: u64 = local_id
            .parse()
            .map_err(SourceScopedIdParseError::InvalidInstanceId)?;
        Ok(Self::new(resource, local_id))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceScopedGeometryDefinitionId {
    pub resource: String,
    pub local_id: u64,
}

impl SourceScopedGeometryDefinitionId {
    pub fn new(resource: impl Into<String>, local_id: impl Into<u64>) -> Self {
        Self {
            resource: resource.into(),
            local_id: local_id.into(),
        }
    }

    pub fn from_geometry_definition_id(
        resource: impl Into<String>,
        id: GeometryDefinitionId,
    ) -> Self {
        Self::new(resource, id.0)
    }

    pub fn resource(&self) -> &str {
        &self.resource
    }

    pub fn local_id(&self) -> u64 {
        self.local_id
    }

    pub fn as_geometry_definition_id(&self) -> GeometryDefinitionId {
        GeometryDefinitionId(self.local_id)
    }

    pub fn validate(&self) -> Result<(), SourceScopedIdParseError> {
        validate_source_scoped_parts(&self.resource, &self.local_id.to_string())
    }
}

impl fmt::Display for SourceScopedGeometryDefinitionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}{SOURCE_SCOPED_ID_SEPARATOR}{}",
            self.resource, self.local_id
        )
    }
}

impl FromStr for SourceScopedGeometryDefinitionId {
    type Err = SourceScopedIdParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (resource, local_id) = parse_source_scoped_parts(value)?;
        let local_id: u64 = local_id
            .parse()
            .map_err(SourceScopedIdParseError::InvalidDefinitionId)?;
        Ok(Self::new(resource, local_id))
    }
}

fn parse_source_scoped_parts(value: &str) -> Result<(&str, &str), SourceScopedIdParseError> {
    let (resource, local_id) = value
        .split_once(SOURCE_SCOPED_ID_SEPARATOR)
        .ok_or(SourceScopedIdParseError::MissingSeparator)?;
    validate_source_scoped_parts(resource, local_id)?;
    Ok((resource, local_id))
}

fn validate_source_scoped_parts(
    resource: &str,
    local_id: &str,
) -> Result<(), SourceScopedIdParseError> {
    if resource.is_empty() {
        return Err(SourceScopedIdParseError::EmptyResource);
    }
    if local_id.is_empty() {
        return Err(SourceScopedIdParseError::EmptyLocalId);
    }
    Ok(())
}

fn validate_project_member_resource(resource: &str) -> Result<String, String> {
    validate_source_scoped_parts(resource, "0").map_err(|error| error.to_string())?;
    if resource.contains(SOURCE_SCOPED_ID_SEPARATOR) {
        return Err(format!(
            "project member resource `{resource}` must not contain `{SOURCE_SCOPED_ID_SEPARATOR}`"
        ));
    }
    Ok(resource.to_string())
}

fn source_scoped_semantic_element_id_string(
    resource: &str,
    local_id: &str,
) -> Result<String, String> {
    validate_source_scoped_parts(resource, local_id).map_err(|error| error.to_string())?;
    Ok(format!("{resource}{SOURCE_SCOPED_ID_SEPARATOR}{local_id}"))
}

fn source_scoped_geometry_instance_id_string(resource: &str, local_id: u64) -> String {
    SourceScopedGeometryInstanceId::new(resource, local_id).to_string()
}

fn source_scoped_geometry_definition_id_string(resource: &str, local_id: u64) -> String {
    SourceScopedGeometryDefinitionId::new(resource, local_id).to_string()
}

#[cfg(target_arch = "wasm32")]
fn source_resource_from_source_scoped_id(value: &str) -> Option<&str> {
    let (resource, local_id) = value.split_once(SOURCE_SCOPED_ID_SEPARATOR)?;
    (!resource.is_empty()
        && resource.starts_with("ifc/")
        && !local_id.is_empty()
        && !local_id.contains(SOURCE_SCOPED_ID_SEPARATOR))
    .then_some(resource)
}

#[cfg(target_arch = "wasm32")]
fn project_local_geometry_id(id: u64) -> u64 {
    id & PROJECT_GEOMETRY_LOCAL_ID_MASK
}

fn push_project_instance_request(
    requests: &mut Vec<WebGeometryInstanceBatchRequest>,
    resource: String,
    local_id: u64,
) {
    let index = requests
        .iter()
        .position(|request| request.resource == resource)
        .unwrap_or_else(|| {
            requests.push(WebGeometryInstanceBatchRequest {
                resource: resource.clone(),
                instance_ids: Vec::new(),
            });
            requests.len() - 1
        });
    requests[index].instance_ids.push(local_id);
}

fn push_project_definition_request(
    requests: &mut Vec<WebGeometryDefinitionBatchRequest>,
    resource: String,
    local_id: u64,
) {
    let index = requests
        .iter()
        .position(|request| request.resource == resource)
        .unwrap_or_else(|| {
            requests.push(WebGeometryDefinitionBatchRequest {
                resource: resource.clone(),
                definition_ids: Vec::new(),
            });
            requests.len() - 1
        });
    requests[index].definition_ids.push(local_id);
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebResourceCatalog {
    pub resources: Vec<String>,
    #[serde(default)]
    pub projects: Vec<WebProjectResourceCatalogEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebProjectResourceCatalogEntry {
    pub resource: String,
    pub label: String,
    pub members: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebPreparedPackageResponse {
    pub resource: String,
    pub package: WebPreparedGeometryPackage,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebGeometryCatalogRequest {
    pub resource: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebGeometryCatalogResponse {
    pub resource: String,
    pub catalog: WebGeometryCatalog,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebGeometryInstanceBatchRequest {
    pub resource: String,
    pub instance_ids: Vec<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebGeometryInstanceBatchResponse {
    pub resource: String,
    pub batch: WebGeometryInstanceBatch,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<WebGeometryBatchMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub returned: Option<WebGeometryBatchMetadata>,
    #[serde(default)]
    pub missing_instance_ids: Vec<u64>,
    #[serde(default)]
    pub skipped_instance_ids: Vec<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebGeometryDefinitionBatchRequest {
    pub resource: String,
    pub definition_ids: Vec<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebGeometryDefinitionBatchResponse {
    pub resource: String,
    pub batch: WebGeometryDefinitionBatch,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<WebGeometryBatchMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub returned: Option<WebGeometryBatchMetadata>,
    #[serde(default)]
    pub missing_definition_ids: Vec<u64>,
    #[serde(default)]
    pub skipped_definition_ids: Vec<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebGeometryBatchMetadata {
    pub id_field: String,
    pub ids: Vec<u64>,
    pub count: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WebGeometryCatalog {
    pub definitions: Vec<WebGeometryDefinitionCatalogEntry>,
    pub elements: Vec<WebPreparedGeometryElement>,
    pub instances: Vec<WebPreparedGeometryInstance>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WebGeometryDefinitionCatalogEntry {
    pub id: u64,
    pub bounds_min: [f64; 3],
    pub bounds_max: [f64; 3],
    pub vertex_count: usize,
    pub triangle_count: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WebGeometryInstanceBatch {
    pub instances: Vec<WebPreparedGeometryInstance>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WebGeometryDefinitionBatch {
    pub definitions: Vec<WebPreparedGeometryDefinition>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WebPreparedGeometryPackage {
    pub definitions: Vec<WebPreparedGeometryDefinition>,
    pub elements: Vec<WebPreparedGeometryElement>,
    pub instances: Vec<WebPreparedGeometryInstance>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WebProjectPreparedGeometryPackage {
    pub resources: Vec<String>,
    pub definitions: Vec<WebProjectPreparedGeometryDefinition>,
    pub elements: Vec<WebPreparedGeometryElement>,
    pub instances: Vec<WebProjectPreparedGeometryInstance>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WebPreparedGeometryDefinition {
    pub id: u64,
    pub mesh: WebPreparedMesh,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WebProjectPreparedGeometryDefinition {
    pub id: String,
    pub mesh: WebPreparedMesh,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WebPreparedGeometryElement {
    pub id: String,
    pub label: String,
    pub declared_entity: String,
    pub default_render_class: String,
    pub bounds_min: [f64; 3],
    pub bounds_max: [f64; 3],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WebPreparedGeometryInstance {
    pub id: u64,
    pub element_id: String,
    pub definition_id: u64,
    pub transform: [f64; 16],
    pub bounds_min: [f64; 3],
    pub bounds_max: [f64; 3],
    pub external_id: String,
    pub label: String,
    pub display_color: Option<[f32; 3]>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WebProjectPreparedGeometryInstance {
    pub id: String,
    pub element_id: String,
    pub definition_id: String,
    pub transform: [f64; 16],
    pub bounds_min: [f64; 3],
    pub bounds_max: [f64; 3],
    pub external_id: String,
    pub label: String,
    pub display_color: Option<[f32; 3]>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WebProjectGeometryCatalog {
    pub resources: Vec<String>,
    pub definitions: Vec<WebProjectGeometryDefinitionCatalogEntry>,
    pub elements: Vec<WebPreparedGeometryElement>,
    pub instances: Vec<WebProjectPreparedGeometryInstance>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WebProjectGeometryDefinitionCatalogEntry {
    pub id: String,
    pub bounds_min: [f64; 3],
    pub bounds_max: [f64; 3],
    pub vertex_count: usize,
    pub triangle_count: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebProjectGeometryStreamPlan {
    pub instance_ids: Vec<String>,
    pub definition_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebProjectResolvedStartView {
    pub visible_element_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WebProjectGeometryInstanceBatch {
    pub instances: Vec<WebProjectPreparedGeometryInstance>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WebProjectGeometryDefinitionBatch {
    pub definitions: Vec<WebProjectPreparedGeometryDefinition>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebProjectGeometryBatchRequests {
    pub instance_requests: Vec<WebGeometryInstanceBatchRequest>,
    pub definition_requests: Vec<WebGeometryDefinitionBatchRequest>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WebPreparedMesh {
    pub local_origin: [f64; 3],
    pub bounds_min: [f64; 3],
    pub bounds_max: [f64; 3],
    pub vertices: Vec<WebPreparedVertex>,
    pub indices: Vec<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WebPreparedVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
}

pub fn demo_summary_string() -> String {
    match build_demo_asset(DEFAULT_DEMO_RESOURCE) {
        Ok(asset) => asset.summary_line(),
        Err(error) => format!("w web demo failed: {error}"),
    }
}

fn build_demo_asset(resource: &str) -> Result<DemoAsset, cc_w_runtime::RuntimeError> {
    let engine = Engine::new(
        LocalGeometryBackendBridge::default(),
        NullRenderBackend::default(),
    );
    engine.build_demo_asset_for(resource)
}

#[cfg(target_arch = "wasm32")]
fn build_runtime_scene(resource: &str) -> Result<RuntimeSceneState, cc_w_runtime::RuntimeError> {
    let engine = Engine::new(
        LocalGeometryBackendBridge::default(),
        NullRenderBackend::default(),
    );
    engine.build_runtime_scene_for(resource)
}

impl WebPreparedGeometryPackage {
    pub fn from_prepared_package(package: &PreparedGeometryPackage) -> Self {
        Self {
            definitions: package
                .definitions
                .iter()
                .map(WebPreparedGeometryDefinition::from_prepared_definition)
                .collect(),
            elements: package
                .elements
                .iter()
                .map(WebPreparedGeometryElement::from_prepared_element)
                .collect(),
            instances: package
                .instances
                .iter()
                .map(WebPreparedGeometryInstance::from_prepared_instance)
                .collect(),
        }
    }

    pub fn try_into_prepared_package(self) -> Result<PreparedGeometryPackage, String> {
        Ok(PreparedGeometryPackage {
            definitions: self
                .definitions
                .into_iter()
                .map(WebPreparedGeometryDefinition::into_prepared_definition)
                .collect(),
            elements: self
                .elements
                .into_iter()
                .map(WebPreparedGeometryElement::try_into_prepared_element)
                .collect::<Result<Vec<_>, _>>()?,
            instances: self
                .instances
                .into_iter()
                .map(WebPreparedGeometryInstance::into_prepared_instance)
                .collect(),
        })
    }
}

impl WebProjectPreparedGeometryPackage {
    pub fn from_prepared_packages<'a, I, R>(members: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = (R, &'a PreparedGeometryPackage)>,
        R: AsRef<str>,
    {
        let web_members = members
            .into_iter()
            .map(|(resource, package)| {
                (
                    resource.as_ref().to_string(),
                    WebPreparedGeometryPackage::from_prepared_package(package),
                )
            })
            .collect::<Vec<_>>();
        Self::from_web_prepared_packages(
            web_members
                .iter()
                .map(|(resource, package)| (resource.as_str(), package)),
        )
    }

    pub fn from_web_prepared_packages<'a, I, R>(members: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = (R, &'a WebPreparedGeometryPackage)>,
        R: AsRef<str>,
    {
        let mut resources = Vec::new();
        let mut definitions = Vec::new();
        let mut elements = Vec::new();
        let mut instances = Vec::new();
        let mut seen_resources = std::collections::HashSet::new();

        for (resource, package) in members {
            let resource = validate_project_member_resource(resource.as_ref())?;
            if !seen_resources.insert(resource.clone()) {
                return Err(format!("duplicate project member resource `{resource}`"));
            }
            resources.push(resource.clone());

            definitions.extend(package.definitions.iter().map(|definition| {
                WebProjectPreparedGeometryDefinition {
                    id: source_scoped_geometry_definition_id_string(&resource, definition.id),
                    mesh: definition.mesh.clone(),
                }
            }));
            for element in &package.elements {
                let mut element = element.clone();
                element.id = source_scoped_semantic_element_id_string(&resource, &element.id)?;
                elements.push(element);
            }
            for instance in &package.instances {
                instances.push(WebProjectPreparedGeometryInstance::from_web_instance(
                    &resource, instance,
                )?);
            }
        }

        Ok(Self {
            resources,
            definitions,
            elements,
            instances,
        })
    }

    pub fn catalog(&self) -> WebProjectGeometryCatalog {
        WebProjectGeometryCatalog {
            resources: self.resources.clone(),
            definitions: self
                .definitions
                .iter()
                .map(WebProjectGeometryDefinitionCatalogEntry::from_project_definition)
                .collect(),
            elements: self.elements.clone(),
            instances: self.instances.clone(),
        }
    }
}

impl WebProjectGeometryCatalog {
    pub fn from_geometry_catalogs<'a, I, R>(members: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = (R, &'a GeometryCatalog)>,
        R: AsRef<str>,
    {
        let web_members = members
            .into_iter()
            .map(|(resource, catalog)| {
                (
                    resource.as_ref().to_string(),
                    WebGeometryCatalog::from_geometry_catalog(catalog),
                )
            })
            .collect::<Vec<_>>();
        Self::from_web_geometry_catalogs(
            web_members
                .iter()
                .map(|(resource, catalog)| (resource.as_str(), catalog)),
        )
    }

    pub fn from_web_geometry_catalogs<'a, I, R>(members: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = (R, &'a WebGeometryCatalog)>,
        R: AsRef<str>,
    {
        let mut resources = Vec::new();
        let mut definitions = Vec::new();
        let mut elements = Vec::new();
        let mut instances = Vec::new();
        let mut seen_resources = std::collections::HashSet::new();

        for (resource, catalog) in members {
            let resource = validate_project_member_resource(resource.as_ref())?;
            if !seen_resources.insert(resource.clone()) {
                return Err(format!("duplicate project member resource `{resource}`"));
            }
            resources.push(resource.clone());

            definitions.extend(catalog.definitions.iter().map(|definition| {
                WebProjectGeometryDefinitionCatalogEntry::from_web_catalog_entry(
                    &resource, definition,
                )
            }));
            for element in &catalog.elements {
                let mut element = element.clone();
                element.id = source_scoped_semantic_element_id_string(&resource, &element.id)?;
                elements.push(element);
            }
            for instance in &catalog.instances {
                instances.push(WebProjectPreparedGeometryInstance::from_web_instance(
                    &resource, instance,
                )?);
            }
        }

        Ok(Self {
            resources,
            definitions,
            elements,
            instances,
        })
    }

    pub fn default_start_view_element_ids(&self) -> Result<Vec<String>, String> {
        let mut visible_element_ids = Vec::new();
        for element in &self.elements {
            if web_default_visibility_for_class_name(&element.default_render_class)? {
                visible_element_ids.push(element.id.clone());
            }
        }
        Ok(visible_element_ids)
    }

    pub fn all_element_ids(&self) -> Vec<String> {
        self.elements
            .iter()
            .map(|element| element.id.clone())
            .collect()
    }

    pub fn resolve_start_view(
        &self,
        request: &GeometryStartViewRequest,
    ) -> Result<WebProjectResolvedStartView, String> {
        let visible_element_ids = match request {
            GeometryStartViewRequest::Default => self.default_start_view_element_ids()?,
            GeometryStartViewRequest::Minimal(max_elements) => self
                .default_start_view_element_ids()?
                .into_iter()
                .take(*max_elements)
                .collect(),
            GeometryStartViewRequest::All => self.all_element_ids(),
            GeometryStartViewRequest::Elements(ids) => {
                let known = self
                    .elements
                    .iter()
                    .map(|element| element.id.as_str())
                    .collect::<std::collections::HashSet<_>>();
                let mut seen = std::collections::HashSet::new();
                ids.iter()
                    .map(SemanticElementId::as_str)
                    .filter(|id| known.contains(id))
                    .filter(|id| seen.insert((*id).to_string()))
                    .map(str::to_string)
                    .collect()
            }
        };

        Ok(WebProjectResolvedStartView {
            visible_element_ids,
        })
    }

    pub fn stream_plan_for_element_ids(
        &self,
        element_ids: &[String],
    ) -> WebProjectGeometryStreamPlan {
        let mut instance_ids = Vec::new();
        let mut definition_ids = Vec::new();
        let mut seen_instances = std::collections::HashSet::new();
        let mut seen_definitions = std::collections::HashSet::new();

        for element_id in element_ids {
            for instance in self
                .instances
                .iter()
                .filter(|instance| instance.element_id == *element_id)
            {
                if seen_instances.insert(instance.id.clone()) {
                    instance_ids.push(instance.id.clone());
                }

                if seen_definitions.insert(instance.definition_id.clone()) {
                    definition_ids.push(instance.definition_id.clone());
                }
            }
        }

        WebProjectGeometryStreamPlan {
            instance_ids,
            definition_ids,
        }
    }

    pub fn default_start_view_stream_plan(&self) -> Result<WebProjectGeometryStreamPlan, String> {
        let visible_element_ids = self.default_start_view_element_ids()?;
        Ok(self.stream_plan_for_element_ids(&visible_element_ids))
    }

    pub fn all_visible_stream_plan(&self) -> WebProjectGeometryStreamPlan {
        self.stream_plan_for_element_ids(&self.all_element_ids())
    }
}

impl WebProjectGeometryStreamPlan {
    pub fn from_local_stream_plan(
        resource: impl AsRef<str>,
        plan: &GeometryStreamPlan,
    ) -> Result<Self, String> {
        let resource = validate_project_member_resource(resource.as_ref())?;
        Ok(Self {
            instance_ids: plan
                .instance_ids
                .iter()
                .map(|id| source_scoped_geometry_instance_id_string(&resource, id.0))
                .collect(),
            definition_ids: plan
                .definition_ids
                .iter()
                .map(|id| source_scoped_geometry_definition_id_string(&resource, id.0))
                .collect(),
        })
    }

    pub fn to_member_batch_requests(&self) -> Result<WebProjectGeometryBatchRequests, String> {
        let mut instance_requests = Vec::<WebGeometryInstanceBatchRequest>::new();
        let mut definition_requests = Vec::<WebGeometryDefinitionBatchRequest>::new();
        let mut seen_instances = std::collections::HashSet::new();
        let mut seen_definitions = std::collections::HashSet::new();

        for id in &self.instance_ids {
            if !seen_instances.insert(id.clone()) {
                continue;
            }
            let scoped = id
                .parse::<SourceScopedGeometryInstanceId>()
                .map_err(|error| error.to_string())?;
            push_project_instance_request(&mut instance_requests, scoped.resource, scoped.local_id);
        }

        for id in &self.definition_ids {
            if !seen_definitions.insert(id.clone()) {
                continue;
            }
            let scoped = id
                .parse::<SourceScopedGeometryDefinitionId>()
                .map_err(|error| error.to_string())?;
            push_project_definition_request(
                &mut definition_requests,
                scoped.resource,
                scoped.local_id,
            );
        }

        Ok(WebProjectGeometryBatchRequests {
            instance_requests,
            definition_requests,
        })
    }
}

impl WebProjectGeometryInstanceBatch {
    pub fn from_web_geometry_instance_batches<'a, I, R>(members: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = (R, &'a WebGeometryInstanceBatch)>,
        R: AsRef<str>,
    {
        let mut instances = Vec::new();
        let mut seen_resources = std::collections::HashSet::new();
        for (resource, batch) in members {
            let resource = validate_project_member_resource(resource.as_ref())?;
            if !seen_resources.insert(resource.clone()) {
                return Err(format!("duplicate project member resource `{resource}`"));
            }
            for instance in &batch.instances {
                instances.push(WebProjectPreparedGeometryInstance::from_web_instance(
                    &resource, instance,
                )?);
            }
        }
        Ok(Self { instances })
    }
}

impl WebProjectGeometryDefinitionBatch {
    pub fn from_web_geometry_definition_batches<'a, I, R>(members: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = (R, &'a WebGeometryDefinitionBatch)>,
        R: AsRef<str>,
    {
        let mut definitions = Vec::new();
        let mut seen_resources = std::collections::HashSet::new();
        for (resource, batch) in members {
            let resource = validate_project_member_resource(resource.as_ref())?;
            if !seen_resources.insert(resource.clone()) {
                return Err(format!("duplicate project member resource `{resource}`"));
            }
            definitions.extend(batch.definitions.iter().map(|definition| {
                WebProjectPreparedGeometryDefinition {
                    id: source_scoped_geometry_definition_id_string(&resource, definition.id),
                    mesh: definition.mesh.clone(),
                }
            }));
        }
        Ok(Self { definitions })
    }
}

impl WebGeometryCatalogResponse {
    pub fn from_geometry_catalog(resource: impl Into<String>, catalog: &GeometryCatalog) -> Self {
        Self {
            resource: resource.into(),
            catalog: WebGeometryCatalog::from_geometry_catalog(catalog),
        }
    }
}

impl WebGeometryInstanceBatchResponse {
    pub fn from_geometry_instance_batch(
        resource: impl Into<String>,
        batch: &GeometryInstanceBatch,
    ) -> Self {
        let returned_ids = batch
            .instances
            .iter()
            .map(|instance| instance.id.0)
            .collect::<Vec<_>>();
        Self {
            resource: resource.into(),
            batch: WebGeometryInstanceBatch::from_geometry_instance_batch(batch),
            request: Some(WebGeometryBatchMetadata::new(
                "instance_ids",
                returned_ids.clone(),
            )),
            returned: Some(WebGeometryBatchMetadata::new("instance_ids", returned_ids)),
            missing_instance_ids: Vec::new(),
            skipped_instance_ids: Vec::new(),
        }
    }
}

impl WebGeometryDefinitionBatchResponse {
    pub fn from_geometry_definition_batch(
        resource: impl Into<String>,
        batch: &GeometryDefinitionBatch,
    ) -> Self {
        let returned_ids = batch
            .definitions
            .iter()
            .map(|definition| definition.id.0)
            .collect::<Vec<_>>();
        Self {
            resource: resource.into(),
            batch: WebGeometryDefinitionBatch::from_geometry_definition_batch(batch),
            request: Some(WebGeometryBatchMetadata::new(
                "definition_ids",
                returned_ids.clone(),
            )),
            returned: Some(WebGeometryBatchMetadata::new(
                "definition_ids",
                returned_ids,
            )),
            missing_definition_ids: Vec::new(),
            skipped_definition_ids: Vec::new(),
        }
    }
}

impl WebGeometryBatchMetadata {
    pub fn new(id_field: impl Into<String>, ids: Vec<u64>) -> Self {
        Self {
            count: ids.len(),
            id_field: id_field.into(),
            ids,
        }
    }
}

impl WebGeometryCatalog {
    pub fn from_geometry_catalog(catalog: &GeometryCatalog) -> Self {
        Self {
            definitions: catalog
                .definitions
                .iter()
                .map(WebGeometryDefinitionCatalogEntry::from_geometry_definition_catalog_entry)
                .collect(),
            elements: catalog
                .elements
                .iter()
                .map(|element| WebPreparedGeometryElement {
                    id: element.id.as_str().to_string(),
                    label: element.label.clone(),
                    declared_entity: element.declared_entity.clone(),
                    default_render_class: default_render_class_name(element.default_render_class)
                        .to_string(),
                    bounds_min: element.bounds.min.to_array(),
                    bounds_max: element.bounds.max.to_array(),
                })
                .collect(),
            instances: catalog
                .instances
                .iter()
                .map(WebPreparedGeometryInstance::from_geometry_instance_catalog_entry)
                .collect(),
        }
    }

    fn default_start_view_element_ids(&self) -> Result<Vec<SemanticElementId>, String> {
        let mut visible_element_ids = Vec::new();
        for element in &self.elements {
            if web_default_visibility_for_class_name(&element.default_render_class)? {
                visible_element_ids.push(SemanticElementId::new(element.id.clone()));
            }
        }
        Ok(visible_element_ids)
    }

    fn stream_plan_for_element_ids(&self, element_ids: &[SemanticElementId]) -> GeometryStreamPlan {
        let mut instance_ids = Vec::new();
        let mut definition_ids = Vec::new();
        let mut seen_instances = std::collections::HashSet::new();
        let mut seen_definitions = std::collections::HashSet::new();

        for element_id in element_ids {
            for instance in self
                .instances
                .iter()
                .filter(|instance| instance.element_id == element_id.as_str())
            {
                let instance_id = GeometryInstanceId(instance.id);
                if seen_instances.insert(instance_id) {
                    instance_ids.push(instance_id);
                }

                let definition_id = GeometryDefinitionId(instance.definition_id);
                if seen_definitions.insert(definition_id) {
                    definition_ids.push(definition_id);
                }
            }
        }

        GeometryStreamPlan {
            instance_ids,
            definition_ids,
        }
    }

    pub fn default_start_view_stream_plan(&self) -> Result<GeometryStreamPlan, String> {
        let visible_element_ids = self.default_start_view_element_ids()?;
        Ok(self.stream_plan_for_element_ids(&visible_element_ids))
    }

    pub fn try_into_prepared_elements(self) -> Result<Vec<PreparedGeometryElement>, String> {
        self.elements
            .into_iter()
            .map(WebPreparedGeometryElement::try_into_prepared_element)
            .collect()
    }

    pub fn try_into_geometry_catalog(self) -> Result<GeometryCatalog, String> {
        Ok(GeometryCatalog {
            definitions: self
                .definitions
                .into_iter()
                .map(WebGeometryDefinitionCatalogEntry::into_geometry_definition_catalog_entry)
                .collect(),
            elements: self
                .elements
                .into_iter()
                .map(WebPreparedGeometryElement::try_into_geometry_element_catalog_entry)
                .collect::<Result<Vec<_>, _>>()?,
            instances: self
                .instances
                .into_iter()
                .map(WebPreparedGeometryInstance::into_geometry_instance_catalog_entry)
                .collect(),
        })
    }
}

impl WebGeometryDefinitionCatalogEntry {
    fn from_geometry_definition_catalog_entry(
        definition: &cc_w_types::GeometryDefinitionCatalogEntry,
    ) -> Self {
        Self {
            id: definition.id.0,
            bounds_min: definition.bounds.min.to_array(),
            bounds_max: definition.bounds.max.to_array(),
            vertex_count: definition.vertex_count,
            triangle_count: definition.triangle_count,
        }
    }

    fn into_geometry_definition_catalog_entry(self) -> GeometryDefinitionCatalogEntry {
        GeometryDefinitionCatalogEntry {
            id: GeometryDefinitionId(self.id),
            bounds: Bounds3 {
                min: glam::DVec3::from_array(self.bounds_min),
                max: glam::DVec3::from_array(self.bounds_max),
            },
            vertex_count: self.vertex_count,
            triangle_count: self.triangle_count,
        }
    }
}

impl WebProjectGeometryDefinitionCatalogEntry {
    fn from_web_catalog_entry(
        resource: &str,
        definition: &WebGeometryDefinitionCatalogEntry,
    ) -> Self {
        Self {
            id: source_scoped_geometry_definition_id_string(resource, definition.id),
            bounds_min: definition.bounds_min,
            bounds_max: definition.bounds_max,
            vertex_count: definition.vertex_count,
            triangle_count: definition.triangle_count,
        }
    }

    fn from_project_definition(definition: &WebProjectPreparedGeometryDefinition) -> Self {
        Self {
            id: definition.id.clone(),
            bounds_min: definition.mesh.bounds_min,
            bounds_max: definition.mesh.bounds_max,
            vertex_count: definition.mesh.vertices.len(),
            triangle_count: definition.mesh.indices.len() / 3,
        }
    }
}

impl WebGeometryInstanceBatchRequest {
    pub fn to_geometry_instance_batch_request(&self) -> GeometryInstanceBatchRequest {
        GeometryInstanceBatchRequest::new(
            self.instance_ids
                .iter()
                .copied()
                .map(GeometryInstanceId)
                .collect(),
        )
    }
}

impl WebGeometryDefinitionBatchRequest {
    pub fn to_geometry_definition_batch_request(&self) -> GeometryDefinitionBatchRequest {
        GeometryDefinitionBatchRequest::new(
            self.definition_ids
                .iter()
                .copied()
                .map(GeometryDefinitionId)
                .collect(),
        )
    }
}

impl WebProjectPreparedGeometryInstance {
    fn from_web_instance(
        resource: &str,
        instance: &WebPreparedGeometryInstance,
    ) -> Result<Self, String> {
        Ok(Self {
            id: source_scoped_geometry_instance_id_string(resource, instance.id),
            element_id: source_scoped_semantic_element_id_string(resource, &instance.element_id)?,
            definition_id: source_scoped_geometry_definition_id_string(
                resource,
                instance.definition_id,
            ),
            transform: instance.transform,
            bounds_min: instance.bounds_min,
            bounds_max: instance.bounds_max,
            external_id: instance.external_id.clone(),
            label: instance.label.clone(),
            display_color: instance.display_color,
        })
    }
}

impl WebGeometryInstanceBatch {
    pub fn from_geometry_instance_batch(batch: &GeometryInstanceBatch) -> Self {
        Self {
            instances: batch
                .instances
                .iter()
                .map(WebPreparedGeometryInstance::from_geometry_instance_catalog_entry)
                .collect(),
        }
    }

    pub fn into_geometry_instance_batch(self) -> GeometryInstanceBatch {
        GeometryInstanceBatch {
            instances: self
                .instances
                .into_iter()
                .map(WebPreparedGeometryInstance::into_geometry_instance_catalog_entry)
                .collect(),
        }
    }
}

impl WebGeometryDefinitionBatch {
    pub fn from_geometry_definition_batch(batch: &GeometryDefinitionBatch) -> Self {
        Self {
            definitions: batch
                .definitions
                .iter()
                .map(WebPreparedGeometryDefinition::from_prepared_definition)
                .collect(),
        }
    }

    pub fn into_geometry_definition_batch(self) -> GeometryDefinitionBatch {
        GeometryDefinitionBatch {
            definitions: self
                .definitions
                .into_iter()
                .map(WebPreparedGeometryDefinition::into_prepared_definition)
                .collect(),
        }
    }
}

impl WebPreparedGeometryDefinition {
    fn from_prepared_definition(definition: &PreparedGeometryDefinition) -> Self {
        Self {
            id: definition.id.0,
            mesh: WebPreparedMesh::from_prepared_mesh(&definition.mesh),
        }
    }

    fn into_prepared_definition(self) -> PreparedGeometryDefinition {
        PreparedGeometryDefinition {
            id: GeometryDefinitionId(self.id),
            mesh: self.mesh.into_prepared_mesh(),
        }
    }
}

impl WebPreparedGeometryElement {
    fn from_prepared_element(element: &PreparedGeometryElement) -> Self {
        Self {
            id: element.id.as_str().to_string(),
            label: element.label.clone(),
            declared_entity: element.declared_entity.clone(),
            default_render_class: default_render_class_name(element.default_render_class)
                .to_string(),
            bounds_min: element.bounds.min.to_array(),
            bounds_max: element.bounds.max.to_array(),
        }
    }

    fn try_into_prepared_element(self) -> Result<PreparedGeometryElement, String> {
        Ok(PreparedGeometryElement {
            id: SemanticElementId::new(self.id),
            label: self.label,
            declared_entity: self.declared_entity,
            default_render_class: parse_default_render_class(&self.default_render_class)?,
            bounds: Bounds3 {
                min: glam::DVec3::from_array(self.bounds_min),
                max: glam::DVec3::from_array(self.bounds_max),
            },
        })
    }

    fn try_into_geometry_element_catalog_entry(
        self,
    ) -> Result<GeometryElementCatalogEntry, String> {
        Ok(GeometryElementCatalogEntry {
            id: SemanticElementId::new(self.id),
            label: self.label,
            declared_entity: self.declared_entity,
            default_render_class: parse_default_render_class(&self.default_render_class)?,
            bounds: Bounds3 {
                min: glam::DVec3::from_array(self.bounds_min),
                max: glam::DVec3::from_array(self.bounds_max),
            },
        })
    }
}

impl WebPreparedGeometryInstance {
    fn from_geometry_instance_catalog_entry(
        instance: &cc_w_types::GeometryInstanceCatalogEntry,
    ) -> Self {
        Self {
            id: instance.id.0,
            element_id: instance.element_id.as_str().to_string(),
            definition_id: instance.definition_id.0,
            transform: instance.transform.to_cols_array(),
            bounds_min: instance.bounds.min.to_array(),
            bounds_max: instance.bounds.max.to_array(),
            external_id: instance.external_id.as_str().to_string(),
            label: instance.label.clone(),
            display_color: instance.display_color.map(DisplayColor::as_rgb),
        }
    }

    fn from_prepared_instance(instance: &PreparedGeometryInstance) -> Self {
        Self {
            id: instance.id.0,
            element_id: instance.element_id.as_str().to_string(),
            definition_id: instance.definition_id.0,
            transform: instance.transform.to_cols_array(),
            bounds_min: instance.bounds.min.to_array(),
            bounds_max: instance.bounds.max.to_array(),
            external_id: instance.external_id.as_str().to_string(),
            label: instance.label.clone(),
            display_color: instance.display_color.map(DisplayColor::as_rgb),
        }
    }

    fn into_prepared_instance(self) -> PreparedGeometryInstance {
        PreparedGeometryInstance {
            id: GeometryInstanceId(self.id),
            element_id: SemanticElementId::new(self.element_id),
            definition_id: GeometryDefinitionId(self.definition_id),
            transform: glam::DMat4::from_cols_array(&self.transform),
            bounds: Bounds3 {
                min: glam::DVec3::from_array(self.bounds_min),
                max: glam::DVec3::from_array(self.bounds_max),
            },
            external_id: ExternalId::new(self.external_id),
            label: self.label,
            display_color: self.display_color.map(|rgb| DisplayColor { rgb }),
        }
    }

    fn into_geometry_instance_catalog_entry(self) -> GeometryInstanceCatalogEntry {
        GeometryInstanceCatalogEntry {
            id: GeometryInstanceId(self.id),
            element_id: SemanticElementId::new(self.element_id),
            definition_id: GeometryDefinitionId(self.definition_id),
            transform: glam::DMat4::from_cols_array(&self.transform),
            bounds: Bounds3 {
                min: glam::DVec3::from_array(self.bounds_min),
                max: glam::DVec3::from_array(self.bounds_max),
            },
            external_id: ExternalId::new(self.external_id),
            label: self.label,
            display_color: self.display_color.map(|rgb| DisplayColor { rgb }),
        }
    }
}

impl WebPreparedMesh {
    fn from_prepared_mesh(mesh: &PreparedMesh) -> Self {
        Self {
            local_origin: mesh.local_origin.to_array(),
            bounds_min: mesh.bounds.min.to_array(),
            bounds_max: mesh.bounds.max.to_array(),
            vertices: mesh
                .vertices
                .iter()
                .copied()
                .map(WebPreparedVertex::from_prepared_vertex)
                .collect(),
            indices: mesh.indices.clone(),
        }
    }

    fn into_prepared_mesh(self) -> PreparedMesh {
        PreparedMesh {
            local_origin: glam::DVec3::from_array(self.local_origin),
            bounds: Bounds3 {
                min: glam::DVec3::from_array(self.bounds_min),
                max: glam::DVec3::from_array(self.bounds_max),
            },
            vertices: self
                .vertices
                .into_iter()
                .map(WebPreparedVertex::into_prepared_vertex)
                .collect(),
            indices: self.indices,
        }
    }
}

impl WebPreparedVertex {
    fn from_prepared_vertex(vertex: PreparedVertex) -> Self {
        Self {
            position: vertex.position,
            normal: vertex.normal,
        }
    }

    fn into_prepared_vertex(self) -> PreparedVertex {
        PreparedVertex {
            position: self.position,
            normal: self.normal,
        }
    }
}

fn default_render_class_name(class: DefaultRenderClass) -> &'static str {
    match class {
        DefaultRenderClass::Physical => "physical",
        DefaultRenderClass::Space => "space",
        DefaultRenderClass::Zone => "zone",
        DefaultRenderClass::Helper => "helper",
        DefaultRenderClass::Terrain => "terrain",
        DefaultRenderClass::TerrainFeature => "terrain-feature",
        DefaultRenderClass::Vegetation => "vegetation",
        DefaultRenderClass::VegetationCover => "vegetation-cover",
        DefaultRenderClass::Water => "water",
        DefaultRenderClass::SurfaceDecal => "surface-decal",
        DefaultRenderClass::Other => "other",
    }
}

fn parse_default_render_class(name: &str) -> Result<DefaultRenderClass, String> {
    match name {
        "physical" => Ok(DefaultRenderClass::Physical),
        "space" => Ok(DefaultRenderClass::Space),
        "zone" => Ok(DefaultRenderClass::Zone),
        "helper" => Ok(DefaultRenderClass::Helper),
        "terrain" => Ok(DefaultRenderClass::Terrain),
        "terrain-feature" => Ok(DefaultRenderClass::TerrainFeature),
        "vegetation" => Ok(DefaultRenderClass::Vegetation),
        "vegetation-cover" => Ok(DefaultRenderClass::VegetationCover),
        "water" => Ok(DefaultRenderClass::Water),
        "surface-decal" => Ok(DefaultRenderClass::SurfaceDecal),
        "other" => Ok(DefaultRenderClass::Other),
        other => Err(format!("unknown default render class `{other}`")),
    }
}

fn web_default_visibility_for_class_name(name: &str) -> Result<bool, String> {
    Ok(match parse_default_render_class(name)? {
        DefaultRenderClass::Physical
        | DefaultRenderClass::Terrain
        | DefaultRenderClass::TerrainFeature
        | DefaultRenderClass::Vegetation
        | DefaultRenderClass::VegetationCover
        | DefaultRenderClass::Water
        | DefaultRenderClass::SurfaceDecal
        | DefaultRenderClass::Other => true,
        DefaultRenderClass::Space | DefaultRenderClass::Zone | DefaultRenderClass::Helper => false,
    })
}

#[derive(Debug, Default)]
struct LocalGeometryBackendBridge {
    geometry_backend: GeometryBackend,
}

impl GeometryPackageSource for LocalGeometryBackendBridge {
    fn load_prepared_package(
        &self,
        resource: &str,
    ) -> Result<PreparedGeometryPackage, GeometryPackageSourceError> {
        self.geometry_backend
            .build_demo_package_for(resource)
            .map_err(map_geometry_backend_error)
    }
}

fn map_geometry_backend_error(error: GeometryBackendError) -> GeometryPackageSourceError {
    match error {
        GeometryBackendError::Resource(ResourceError::UnknownResource {
            requested,
            available,
        }) => GeometryPackageSourceError::UnknownResource {
            requested,
            available,
        },
        other => GeometryPackageSourceError::LoadFailed(other.to_string()),
    }
}

#[cfg(target_arch = "wasm32")]
use cc_w_backend::available_demo_resources;
#[cfg(target_arch = "wasm32")]
use cc_w_render::{
    Camera, DepthTarget, MeshRenderer, RenderDefaults, RenderProfileDescriptor, RenderProfileId,
    ViewportSize, fit_camera_to_render_scene, pick_prepared_scene_cpu,
};
#[cfg(target_arch = "wasm32")]
use cc_w_types::{PickHit, PickRegion, PreparedRenderScene, WORLD_FORWARD, WORLD_RIGHT, WORLD_UP};
#[cfg(target_arch = "wasm32")]
use glam::{DMat4, DVec3, DVec4, Vec2};
#[cfg(target_arch = "wasm32")]
use js_sys::{Array, JSON, Promise, decode_uri_component};
#[cfg(target_arch = "wasm32")]
use std::{cell::RefCell, collections::HashSet, rc::Rc};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::{JsCast, closure::Closure, prelude::*};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_futures::{JsFuture, spawn_local};
#[cfg(target_arch = "wasm32")]
use web_sys::{
    CanvasRenderingContext2d, CustomEvent, CustomEventInit, Document, Element, Event,
    HtmlCanvasElement, HtmlElement, HtmlSelectElement, MouseEvent, RequestInit, Response,
    WheelEvent, Window,
};

#[cfg(target_arch = "wasm32")]
#[derive(Serialize)]
struct WebPackageRequest<'a> {
    resource: &'a str,
}

#[cfg(target_arch = "wasm32")]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WebViewerViewState {
    resource: String,
    view_mode: String,
    render_profile: String,
    reference_grid_visible: bool,
    available_render_profiles: Vec<WebRenderProfileDescriptor>,
    total_elements: usize,
    total_instances: usize,
    total_definitions: usize,
    default_element_ids: Vec<String>,
    base_visible_element_ids: Vec<String>,
    visible_element_ids: Vec<String>,
    selected_element_ids: Vec<String>,
    selected_instance_ids: Vec<u64>,
    picked_instance_ids: Vec<u64>,
    hidden_element_ids: Vec<String>,
    shown_element_ids: Vec<String>,
    suppressed_element_ids: Vec<String>,
    resident_instances: usize,
    resident_definitions: usize,
    missing_instance_ids: Vec<u64>,
    missing_definition_ids: Vec<u64>,
    triangles: usize,
    draws: usize,
}

#[cfg(target_arch = "wasm32")]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WebRenderProfileDescriptor {
    id: String,
    name: String,
    label: String,
}

#[cfg(target_arch = "wasm32")]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WebPickHit {
    instance_id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    local_instance_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scoped_instance_id: Option<String>,
    element_id: String,
    definition_id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    local_definition_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scoped_definition_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_resource: Option<String>,
    world_centroid: [f64; 3],
    world_anchor: [f64; 3],
}

#[cfg(target_arch = "wasm32")]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WebPickResponse {
    region: WebPickRegion,
    hits: Vec<WebPickHit>,
}

#[cfg(target_arch = "wasm32")]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WebPickRegion {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

#[cfg(target_arch = "wasm32")]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WebPickAnchorEvent {
    visible: bool,
    client_x: f64,
    client_y: f64,
    canvas_x: f64,
    canvas_y: f64,
    element_id: String,
    instance_id: u64,
    definition_id: u64,
    world_anchor: [f64; 3],
}

#[cfg(target_arch = "wasm32")]
fn local_web_resources() -> Vec<String> {
    vec![DEFAULT_WEB_RESOURCE.to_string()]
}

#[cfg(target_arch = "wasm32")]
fn local_web_resource_catalog() -> WebResourceCatalog {
    WebResourceCatalog {
        resources: local_web_resources(),
        projects: Vec::new(),
    }
}

#[cfg(target_arch = "wasm32")]
fn is_demo_resource(resource: &str) -> bool {
    available_demo_resources()
        .iter()
        .any(|candidate| *candidate == resource)
}

#[cfg(target_arch = "wasm32")]
fn is_file_protocol(window: &Window) -> bool {
    window.location().protocol().ok().as_deref() == Some("file:")
}

#[cfg(target_arch = "wasm32")]
async fn fetch_available_resource_catalog(window: &Window) -> Result<WebResourceCatalog, String> {
    if is_file_protocol(window) {
        return Ok(local_web_resource_catalog());
    }

    let text = fetch_server_text(window, "/api/resources", "GET", None).await?;
    let mut catalog: WebResourceCatalog = serde_json::from_str(&text)
        .map_err(|error| format!("invalid /api/resources JSON: {error}"))?;
    catalog.resources = catalog
        .resources
        .into_iter()
        .filter(|resource| resource.starts_with("ifc/") || resource.starts_with("project/"))
        .collect::<Vec<_>>();
    if catalog.resources.is_empty() {
        return Err("server returned an empty resource catalog".to_string());
    }
    Ok(catalog)
}

#[cfg(target_arch = "wasm32")]
fn resource_catalog_resources(catalog: &WebResourceCatalog) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut resources = Vec::new();
    for resource in &catalog.resources {
        if (resource.starts_with("ifc/") || resource.starts_with("project/"))
            && seen.insert(resource.clone())
        {
            resources.push(resource.clone());
        }
    }
    for project in &catalog.projects {
        if project.resource.starts_with("project/") && seen.insert(project.resource.clone()) {
            resources.push(project.resource.clone());
        }
    }
    resources
}

#[cfg(target_arch = "wasm32")]
async fn fetch_runtime_scene_from_server(
    window: &Window,
    resource: &str,
) -> Result<RuntimeSceneState, String> {
    match fetch_runtime_scene_from_server_streaming(window, resource).await {
        Ok(scene) => Ok(scene),
        Err(streaming_error) => {
            log_viewer_error(&format!(
                "w web viewer streaming fetch fell back to /api/package for `{resource}`: {streaming_error}"
            ));
            fetch_runtime_scene_from_server_package(window, resource)
                .await
                .map_err(|package_error| {
                    format!(
                        "streaming fetch failed ({streaming_error}); /api/package fallback failed ({package_error})"
                    )
                })
        }
    }
}

#[cfg(target_arch = "wasm32")]
async fn fetch_runtime_scene_from_server_streaming(
    window: &Window,
    resource: &str,
) -> Result<RuntimeSceneState, String> {
    let catalog_request_text = serde_json::to_string(&WebGeometryCatalogRequest {
        resource: resource.to_string(),
    })
    .map_err(|error| format!("failed to encode geometry catalog request JSON: {error}"))?;
    let catalog_text = fetch_server_text(
        window,
        "/api/geometry/catalog",
        "POST",
        Some(&catalog_request_text),
    )
    .await?;
    let catalog_response: WebGeometryCatalogResponse = serde_json::from_str(&catalog_text)
        .map_err(|error| format!("invalid /api/geometry/catalog JSON: {error}"))?;
    if catalog_response.resource != resource {
        return Err(format!(
            "geometry catalog response resource mismatch: requested `{resource}`, got `{}`",
            catalog_response.resource
        ));
    }

    let mut runtime_scene = RuntimeSceneState::from_catalog_with_start_view(
        catalog_response.catalog.try_into_geometry_catalog()?,
        GeometryStartViewRequest::Default,
    )
    .map_err(|error| error.to_string())?;
    stream_visible_runtime_scene_from_server(window, resource, &mut runtime_scene).await?;

    Ok(runtime_scene)
}

#[cfg(target_arch = "wasm32")]
async fn stream_visible_runtime_scene_from_server(
    window: &Window,
    resource: &str,
    runtime_scene: &mut RuntimeSceneState,
) -> Result<(), String> {
    let plan = runtime_scene.missing_stream_plan_for_visible_elements();
    let (instance_batch, definition_batch) =
        fetch_geometry_batches_for_stream_plan_from_server(window, resource, plan).await?;
    runtime_scene.mark_instance_batch_resident(&instance_batch);
    runtime_scene.mark_definition_batch_resident(&definition_batch);
    let remaining = runtime_scene.missing_stream_plan_for_visible_elements();
    if !remaining.instance_ids.is_empty() || !remaining.definition_ids.is_empty() {
        return Err(format!(
            "geometry streaming left {} instances and {} definitions missing",
            remaining.instance_ids.len(),
            remaining.definition_ids.len()
        ));
    }

    Ok(())
}

#[cfg(target_arch = "wasm32")]
async fn fetch_geometry_batches_for_stream_plan_from_server(
    window: &Window,
    resource: &str,
    plan: GeometryStreamPlan,
) -> Result<(GeometryInstanceBatch, GeometryDefinitionBatch), String> {
    let instance_batch =
        fetch_geometry_instance_batch_from_server(window, resource, &plan.instance_ids).await?;
    let definition_batch =
        fetch_geometry_definition_batch_from_server(window, resource, &plan.definition_ids).await?;

    Ok((instance_batch, definition_batch))
}

#[cfg(target_arch = "wasm32")]
async fn fetch_geometry_instance_batch_from_server(
    window: &Window,
    resource: &str,
    instance_ids: &[GeometryInstanceId],
) -> Result<GeometryInstanceBatch, String> {
    let mut instances = Vec::new();
    for chunk in instance_ids.chunks(WEB_GEOMETRY_BATCH_CHUNK_SIZE) {
        instances.extend(
            fetch_geometry_instance_batch_chunk_from_server(window, resource, chunk)
                .await?
                .instances,
        );
    }
    Ok(GeometryInstanceBatch { instances })
}

#[cfg(target_arch = "wasm32")]
async fn fetch_geometry_instance_batch_chunk_from_server(
    window: &Window,
    resource: &str,
    instance_ids: &[GeometryInstanceId],
) -> Result<GeometryInstanceBatch, String> {
    let instance_request = WebGeometryInstanceBatchRequest {
        resource: resource.to_string(),
        instance_ids: instance_ids.iter().map(|id| id.0).collect(),
    };
    let instance_request_text = serde_json::to_string(&instance_request)
        .map_err(|error| format!("failed to encode geometry instance request JSON: {error}"))?;
    let instance_text = fetch_server_text(
        window,
        "/api/geometry/instances",
        "POST",
        Some(&instance_request_text),
    )
    .await?;
    let instance_response: WebGeometryInstanceBatchResponse = serde_json::from_str(&instance_text)
        .map_err(|error| format!("invalid /api/geometry/instances JSON: {error}"))?;
    if instance_response.resource != resource {
        return Err(format!(
            "geometry instance response resource mismatch: requested `{resource}`, got `{}`",
            instance_response.resource
        ));
    }

    if !instance_response.missing_instance_ids.is_empty()
        || !instance_response.skipped_instance_ids.is_empty()
    {
        return Err(format!(
            "geometry instance batch returned {} missing and {} skipped ids",
            instance_response.missing_instance_ids.len(),
            instance_response.skipped_instance_ids.len()
        ));
    }

    Ok(instance_response.batch.into_geometry_instance_batch())
}

#[cfg(target_arch = "wasm32")]
async fn fetch_geometry_definition_batch_from_server(
    window: &Window,
    resource: &str,
    definition_ids: &[GeometryDefinitionId],
) -> Result<GeometryDefinitionBatch, String> {
    let mut definitions = Vec::new();
    for chunk in definition_ids.chunks(WEB_GEOMETRY_BATCH_CHUNK_SIZE) {
        definitions.extend(
            fetch_geometry_definition_batch_chunk_from_server(window, resource, chunk)
                .await?
                .definitions,
        );
    }
    Ok(GeometryDefinitionBatch { definitions })
}

#[cfg(target_arch = "wasm32")]
async fn fetch_geometry_definition_batch_chunk_from_server(
    window: &Window,
    resource: &str,
    definition_ids: &[GeometryDefinitionId],
) -> Result<GeometryDefinitionBatch, String> {
    let definition_request = WebGeometryDefinitionBatchRequest {
        resource: resource.to_string(),
        definition_ids: definition_ids.iter().map(|id| id.0).collect(),
    };
    let definition_request_text = serde_json::to_string(&definition_request)
        .map_err(|error| format!("failed to encode geometry definition request JSON: {error}"))?;
    let definition_text = fetch_server_text(
        window,
        "/api/geometry/definitions",
        "POST",
        Some(&definition_request_text),
    )
    .await?;
    let definition_response: WebGeometryDefinitionBatchResponse =
        serde_json::from_str(&definition_text)
            .map_err(|error| format!("invalid /api/geometry/definitions JSON: {error}"))?;
    if definition_response.resource != resource {
        return Err(format!(
            "geometry definition response resource mismatch: requested `{resource}`, got `{}`",
            definition_response.resource
        ));
    }

    if !definition_response.missing_definition_ids.is_empty()
        || !definition_response.skipped_definition_ids.is_empty()
    {
        return Err(format!(
            "geometry definition batch returned {} missing and {} skipped ids",
            definition_response.missing_definition_ids.len(),
            definition_response.skipped_definition_ids.len()
        ));
    }

    Ok(definition_response.batch.into_geometry_definition_batch())
}

#[cfg(target_arch = "wasm32")]
async fn fetch_runtime_scene_from_server_package(
    window: &Window,
    resource: &str,
) -> Result<RuntimeSceneState, String> {
    let request_text = serde_json::to_string(&WebPackageRequest { resource })
        .map_err(|error| format!("failed to encode package request JSON: {error}"))?;
    let text = fetch_server_text(window, "/api/package", "POST", Some(&request_text)).await?;
    let response: WebPreparedPackageResponse = serde_json::from_str(&text)
        .map_err(|error| format!("invalid /api/package JSON: {error}"))?;
    let package = response.package.try_into_prepared_package()?;
    RuntimeSceneState::from_prepared_package(package).map_err(|error| error.to_string())
}

#[cfg(target_arch = "wasm32")]
async fn fetch_runtime_scene(window: &Window, resource: &str) -> Result<RuntimeSceneState, String> {
    if !is_file_protocol(window) {
        match fetch_runtime_scene_from_server(window, resource).await {
            Ok(scene) => return Ok(scene),
            Err(error) if !is_demo_resource(resource) => return Err(error),
            Err(error) => log_viewer_error(&format!(
                "w web viewer package fetch fell back to local demo path for `{resource}`: {error}"
            )),
        }
    }

    build_runtime_scene(resource).map_err(|error| error.to_string())
}

#[cfg(target_arch = "wasm32")]
async fn fetch_server_text(
    window: &Window,
    url: &str,
    method: &str,
    body: Option<&str>,
) -> Result<String, String> {
    let response_value = if method == "GET" {
        JsFuture::from(window.fetch_with_str(url))
            .await
            .map_err(js_error)?
    } else {
        let init = RequestInit::new();
        init.set_method(method);
        if let Some(body) = body {
            init.set_body(&JsValue::from_str(body));
        }
        JsFuture::from(window.fetch_with_str_and_init(url, &init))
            .await
            .map_err(js_error)?
    };

    let response: Response = response_value
        .dyn_into()
        .map_err(|_| "w web viewer fetch returned a non-response value".to_string())?;
    let text_promise = response.text().map_err(js_error)?;
    let text = JsFuture::from(text_promise)
        .await
        .map_err(js_error)?
        .as_string()
        .unwrap_or_default();
    if !response.ok() {
        return Err(if text.is_empty() {
            format!(
                "server request to `{url}` failed with {}",
                response.status()
            )
        } else {
            text
        });
    }

    Ok(text)
}

#[cfg(target_arch = "wasm32")]
async fn load_resource_into_state(
    state: Rc<RefCell<WebViewerState>>,
    resource: String,
) -> Result<(), String> {
    let window = {
        let mut state = state.borrow_mut();
        state.begin_resource_load(&resource);
        state.window.clone()
    };
    let runtime_scene = match fetch_runtime_scene(&window, &resource).await {
        Ok(runtime_scene) => runtime_scene,
        Err(error) => {
            state
                .borrow_mut()
                .finish_resource_load_failed(&resource, &error);
            return Err(error);
        }
    };
    let events = {
        let mut state = state.borrow_mut();
        state.apply_runtime_scene(resource, runtime_scene)?
    };
    dispatch_web_events(events)?;
    Ok(())
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    static WEB_VIEWER_APP: RefCell<Option<WebViewerApp>> = const { RefCell::new(None) };
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn demo_summary() -> String {
    demo_summary_string()
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_current_resource() -> Result<String, JsValue> {
    with_web_viewer_state(|state| Ok(state.current_resource.clone()))
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_resource_catalog_json() -> Result<String, JsValue> {
    with_web_viewer_state(|state| {
        serde_json::to_string(&state.resource_catalog)
            .map_err(|error| format!("failed to encode viewer resource catalog JSON: {error}"))
    })
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_view_state_json() -> Result<String, JsValue> {
    with_web_viewer_state(|state| state.view_state_json())
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_available_profiles_json() -> Result<String, JsValue> {
    with_web_viewer_state(|state| state.render_profiles_json())
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_current_profile() -> Result<String, JsValue> {
    with_web_viewer_state(|state| Ok(state.renderer.profile().as_str().to_string()))
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_reference_grid_visible() -> Result<bool, JsValue> {
    with_web_viewer_state(|state| Ok(state.renderer.reference_grid_visible()))
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_set_reference_grid_visible(visible: bool) -> Result<String, JsValue> {
    let (json, event) = with_web_viewer_state_mut(|state| {
        if state.renderer.reference_grid_visible() == visible {
            return Ok((state.view_state_json()?, None));
        }

        state.renderer.set_reference_grid_visible(visible);
        state.refresh_status();
        Ok((
            state.view_state_json()?,
            Some(state.viewer_state_change_event("referenceGrid")?),
        ))
    })?;
    if let Some(event) = event {
        dispatch_web_events(vec![event]).map_err(|error| JsValue::from_str(&error))?;
    }
    Ok(json)
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_set_profile(profile: String) -> Result<String, JsValue> {
    let (json, event) = with_web_viewer_state_mut(|state| {
        let event = state.apply_render_profile_name(&profile)?;
        Ok((state.view_state_json()?, event))
    })?;
    if let Some(event) = event {
        dispatch_web_events(vec![event]).map_err(|error| JsValue::from_str(&error))?;
    }
    Ok(json)
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_set_clear_color(red: f64, green: f64, blue: f64) -> Result<String, JsValue> {
    with_web_viewer_state_mut(|state| {
        state.set_clear_color(red, green, blue);
        state.render()?;
        state.view_state_json()
    })
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub async fn viewer_set_view_mode(mode: String) -> Result<String, JsValue> {
    let request = parse_web_view_mode(&mode).map_err(|error| JsValue::from_str(&error))?;
    let events = with_web_viewer_state_mut(|state| {
        state.runtime_scene.apply_start_view(request);
        state.commit_runtime_scene_change(false, "viewMode")
    })?;
    dispatch_web_events(events).map_err(|error| JsValue::from_str(&error))?;
    stream_current_visible_view_to_json().await
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub async fn viewer_stream_visible() -> Result<String, JsValue> {
    stream_current_visible_view_to_json().await
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_list_element_ids() -> Result<Array, JsValue> {
    with_web_viewer_state(|state| {
        Ok(semantic_ids_to_array(
            state
                .runtime_scene
                .package()
                .elements
                .iter()
                .map(|element| &element.id),
        ))
    })
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_default_element_ids() -> Result<Array, JsValue> {
    with_web_viewer_state(|state| {
        let resolved = state
            .runtime_scene
            .resolve_start_view(&GeometryStartViewRequest::Default);
        Ok(semantic_ids_to_array(resolved.visible_element_ids.iter()))
    })
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_visible_element_ids() -> Result<Array, JsValue> {
    with_web_viewer_state(|state| {
        let ids = state.runtime_scene.visible_element_ids();
        Ok(semantic_ids_to_array(ids.iter()))
    })
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_selected_element_ids() -> Result<Array, JsValue> {
    with_web_viewer_state(|state| {
        let ids = state.runtime_scene.selected_element_ids();
        Ok(semantic_ids_to_array(ids.iter()))
    })
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_hide_elements(ids: Array) -> Result<u32, JsValue> {
    let (changed, events) = with_web_viewer_state_mut(|state| {
        let ids = semantic_ids_from_array(&ids)?;
        let changed = state.runtime_scene.hide_elements(ids.iter()) as u32;
        let mut events = vec![state.upload_runtime_scene(false)?];
        if changed > 0 {
            events.push(state.viewer_state_change_event("visibility")?);
        }
        Ok((changed, events))
    })?;
    dispatch_web_events(events).map_err(|error| JsValue::from_str(&error))?;
    Ok(changed)
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_show_elements(ids: Array) -> Result<u32, JsValue> {
    let (changed, events) = with_web_viewer_state_mut(|state| {
        let ids = semantic_ids_from_array(&ids)?;
        let changed = state.runtime_scene.show_elements(ids.iter()) as u32;
        let mut events = vec![state.upload_runtime_scene(false)?];
        if changed > 0 {
            events.push(state.viewer_state_change_event("visibility")?);
        }
        Ok((changed, events))
    })?;
    dispatch_web_events(events).map_err(|error| JsValue::from_str(&error))?;
    Ok(changed)
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_reset_element_visibility(ids: Array) -> Result<u32, JsValue> {
    let (changed, events) = with_web_viewer_state_mut(|state| {
        let ids = semantic_ids_from_array(&ids)?;
        let changed = state.runtime_scene.reset_visibility(ids.iter()) as u32;
        let mut events = vec![state.upload_runtime_scene(false)?];
        if changed > 0 {
            events.push(state.viewer_state_change_event("visibility")?);
        }
        Ok((changed, events))
    })?;
    dispatch_web_events(events).map_err(|error| JsValue::from_str(&error))?;
    Ok(changed)
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_reset_all_visibility() -> Result<u32, JsValue> {
    let (changed, events) = with_web_viewer_state_mut(|state| {
        let ids = state
            .runtime_scene
            .package()
            .elements
            .iter()
            .map(|element| element.id.clone())
            .collect::<Vec<_>>();
        let changed = state.runtime_scene.reset_visibility(ids.iter()) as u32;
        let mut events = vec![state.upload_runtime_scene(false)?];
        if changed > 0 {
            events.push(state.viewer_state_change_event("visibility")?);
        }
        Ok((changed, events))
    })?;
    dispatch_web_events(events).map_err(|error| JsValue::from_str(&error))?;
    Ok(changed)
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_suppress_elements(ids: Array) -> Result<u32, JsValue> {
    let (changed, events) = with_web_viewer_state_mut(|state| {
        let ids = semantic_ids_from_array(&ids)?;
        let changed = state.runtime_scene.suppress_elements(ids.iter()) as u32;
        let mut events = vec![state.upload_runtime_scene(false)?];
        if changed > 0 {
            events.push(state.viewer_state_change_event("visibility")?);
        }
        Ok((changed, events))
    })?;
    dispatch_web_events(events).map_err(|error| JsValue::from_str(&error))?;
    Ok(changed)
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_unsuppress_elements(ids: Array) -> Result<u32, JsValue> {
    let (changed, events) = with_web_viewer_state_mut(|state| {
        let ids = semantic_ids_from_array(&ids)?;
        let changed = state.runtime_scene.unsuppress_elements(ids.iter()) as u32;
        let mut events = vec![state.upload_runtime_scene(false)?];
        if changed > 0 {
            events.push(state.viewer_state_change_event("visibility")?);
        }
        Ok((changed, events))
    })?;
    dispatch_web_events(events).map_err(|error| JsValue::from_str(&error))?;
    Ok(changed)
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_select_elements(ids: Array) -> Result<u32, JsValue> {
    let (changed, events) = with_web_viewer_state_mut(|state| {
        let ids = semantic_ids_from_array(&ids)?;
        let changed = state.runtime_scene.select_elements(ids.iter()) as u32;
        let mut events = vec![state.upload_runtime_scene(false)?];
        if changed > 0 {
            events.push(state.viewer_state_change_event("selection")?);
        }
        Ok((changed, events))
    })?;
    dispatch_web_events(events).map_err(|error| JsValue::from_str(&error))?;
    Ok(changed)
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_clear_selection() -> Result<u32, JsValue> {
    let (changed, events) = with_web_viewer_state_mut(|state| {
        let changed = state.runtime_scene.clear_selection() as u32;
        let mut events = vec![state.upload_runtime_scene(false)?];
        if changed > 0 {
            events.push(state.viewer_state_change_event("selection")?);
        }
        Ok((changed, events))
    })?;
    dispatch_web_events(events).map_err(|error| JsValue::from_str(&error))?;
    Ok(changed)
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub async fn viewer_pick_at_json(x: f32, y: f32) -> Result<String, JsValue> {
    pick_current_view_region_to_json(WebPickRequest::Point { x, y }).await
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub async fn viewer_pick_rect_json(x0: f32, y0: f32, x1: f32, y1: f32) -> Result<String, JsValue> {
    pick_current_view_region_to_json(WebPickRequest::Rect { x0, y0, x1, y1 }).await
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_frame_visible() -> Result<(), JsValue> {
    let event = with_web_viewer_state_mut(|state| state.frame_visible_scene())?;
    dispatch_web_events(vec![event]).map_err(|error| JsValue::from_str(&error))
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_resize_viewport() -> Result<(), JsValue> {
    let events = with_web_viewer_state_mut(|state| {
        let event = state.resize_to_window()?;
        state.render()?;
        Ok(event.into_iter().collect::<Vec<_>>())
    })?;
    dispatch_web_events(events).map_err(|error| JsValue::from_str(&error))
}

#[cfg(target_arch = "wasm32")]
async fn stream_current_visible_view_to_json() -> Result<String, JsValue> {
    let (window, resource, plan) = with_web_viewer_state(|state| {
        Ok((
            state.window.clone(),
            state.current_resource.clone(),
            state
                .runtime_scene
                .missing_stream_plan_for_visible_elements(),
        ))
    })?;

    let (instance_batch, definition_batch) =
        fetch_geometry_batches_for_stream_plan_from_server(&window, &resource, plan)
            .await
            .map_err(|error| JsValue::from_str(&error))?;

    let (json, events) = with_web_viewer_state_mut(|state| {
        if state.current_resource != resource {
            return Err("w web viewer resource changed while streaming geometry".to_string());
        }
        let resident_instances = state
            .runtime_scene
            .mark_instance_batch_resident(&instance_batch);
        let resident_definitions = state
            .runtime_scene
            .mark_definition_batch_resident(&definition_batch);
        let mut events = vec![state.upload_runtime_scene(false)?];
        if resident_instances > 0 || resident_definitions > 0 {
            events.push(state.viewer_state_change_event("stream")?);
        }

        let remaining = state
            .runtime_scene
            .missing_stream_plan_for_visible_elements();
        if !remaining.instance_ids.is_empty() || !remaining.definition_ids.is_empty() {
            return Err(format!(
                "geometry streaming left {} instances and {} definitions missing",
                remaining.instance_ids.len(),
                remaining.definition_ids.len()
            ));
        }

        Ok((state.view_state_json()?, events))
    })?;
    dispatch_web_events(events).map_err(|error| JsValue::from_str(&error))?;
    Ok(json)
}

#[cfg(target_arch = "wasm32")]
enum WebPickRequest {
    Point { x: f32, y: f32 },
    Rect { x0: f32, y0: f32, x1: f32, y1: f32 },
}

#[cfg(target_arch = "wasm32")]
async fn pick_current_view_region_to_json(request: WebPickRequest) -> Result<String, JsValue> {
    let app_state = WEB_VIEWER_APP
        .with(|slot| slot.borrow().as_ref().map(|app| app._state.clone()))
        .ok_or_else(|| JsValue::from_str("w web viewer is not initialized"))?;

    pick_region_in_state(app_state, request)
        .await
        .map_err(|error| JsValue::from_str(&error))
}

#[cfg(target_arch = "wasm32")]
async fn pick_region_in_state(
    app_state: Rc<RefCell<WebViewerState>>,
    request: WebPickRequest,
) -> Result<String, String> {
    let window = app_state.borrow().window.clone();
    let prepared = {
        let mut state = app_state.borrow_mut();
        state.prepare_pick_readback(request)?
    };

    JsFuture::from(prepared.map_promise)
        .await
        .map_err(|error| format!("GPU pick readback failed: {:?}", error))?;

    let mapped = prepared.readback.slice(..).get_mapped_range();
    let rgba8 = strip_padded_rows_web(
        &mapped,
        prepared.unpadded_bytes_per_row as usize,
        prepared.padded_bytes_per_row as usize,
        prepared.region.height as usize,
    );
    drop(mapped);
    prepared.readback.unmap();

    JsFuture::from(prepared.depth_map_promise)
        .await
        .map_err(|error| format!("GPU pick depth readback failed: {:?}", error))?;

    let depth_mapped = prepared.depth_readback.slice(..).get_mapped_range();
    let depth_bytes = strip_padded_rows_web(
        &depth_mapped,
        prepared.depth_unpadded_bytes_per_row as usize,
        prepared.depth_padded_bytes_per_row as usize,
        prepared.region.height as usize,
    );
    drop(depth_mapped);
    prepared.depth_readback.unmap();
    let depth_values = depth32_values_from_bytes(&depth_bytes)?;

    let mut result = decode_pick_pixels_with_depth(
        prepared.region,
        &rgba8,
        &prepared.pick_targets,
        &depth_values,
        prepared.clip_from_world,
        ViewportSize::new(prepared.viewport_width, prepared.viewport_height),
    );
    if result.hits.is_empty() {
        let state = app_state.borrow();
        let render_scene = state.runtime_scene.compose_render_scene();
        result = pick_prepared_scene_cpu(
            &render_scene,
            state.renderer.camera(),
            ViewportSize::new(prepared.viewport_width, prepared.viewport_height),
            prepared.region,
        );
    }
    let json = web_pick_response_json(prepared.region, &result.hits)?;

    let anchor_json = {
        let mut state = app_state.borrow_mut();
        state.apply_pick_hits(result.hits);
        state.pick_anchor_event_json()?
    };

    dispatch_json_event(&window, "w-viewer-pick", &json)?;
    dispatch_json_event(&window, "w-viewer-anchor", &anchor_json)?;

    Ok(json)
}

#[cfg(target_arch = "wasm32")]
fn dispatch_json_event(window: &Window, event_name: &str, json: &str) -> Result<(), String> {
    let detail = JSON::parse(json)
        .map_err(|error| format!("failed to parse `{event_name}` JSON: {error:?}"))?;
    let init = CustomEventInit::new();
    init.set_detail(&detail);
    let event = CustomEvent::new_with_event_init_dict(event_name, &init).map_err(js_error)?;
    window.dispatch_event(&event).map_err(js_error)?;
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn depth32_values_from_bytes(bytes: &[u8]) -> Result<Vec<f32>, String> {
    if bytes.len() % 4 != 0 {
        return Err("pick depth readback had a non-f32 byte length".to_string());
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect())
}

#[cfg(target_arch = "wasm32")]
fn decode_pick_pixels_with_depth(
    region: PickRegion,
    rgba8: &[u8],
    pick_targets: &[PickHit],
    depth_values: &[f32],
    clip_from_world: DMat4,
    viewport: ViewportSize,
) -> cc_w_types::PickResult {
    let mut seen = HashSet::new();
    let mut hits = Vec::new();
    let world_from_clip = clip_from_world.inverse();

    for (pixel_index, pixel) in rgba8.chunks_exact(4).enumerate() {
        let pick_index = decode_web_pick_index(pixel);
        if pick_index == 0 || !seen.insert(pick_index) {
            continue;
        }
        let Some(target) = pick_targets.get((pick_index - 1) as usize) else {
            continue;
        };
        let mut hit = target.clone();
        if let Some(depth) = depth_values.get(pixel_index).copied() {
            if let Some(world_anchor) =
                unproject_pick_pixel(region, pixel_index, depth, world_from_clip, viewport)
            {
                hit.world_anchor = world_anchor;
            }
        }
        hits.push(hit);
    }

    cc_w_types::PickResult { region, hits }
}

#[cfg(target_arch = "wasm32")]
fn decode_web_pick_index(pixel: &[u8]) -> u32 {
    u32::from(pixel[0])
        | (u32::from(pixel[1]) << 8)
        | (u32::from(pixel[2]) << 16)
        | (u32::from(pixel[3]) << 24)
}

#[cfg(target_arch = "wasm32")]
fn unproject_pick_pixel(
    region: PickRegion,
    pixel_index: usize,
    depth: f32,
    world_from_clip: DMat4,
    viewport: ViewportSize,
) -> Option<DVec3> {
    if !depth.is_finite() {
        return None;
    }
    let viewport = viewport.clamped();
    let local_x = (pixel_index as u32) % region.width;
    let local_y = (pixel_index as u32) / region.width;
    let pixel_x = region.x.saturating_add(local_x);
    let pixel_y = region.y.saturating_add(local_y);
    let ndc_x = ((f64::from(pixel_x) + 0.5) / f64::from(viewport.width)) * 2.0 - 1.0;
    let ndc_y = 1.0 - (((f64::from(pixel_y) + 0.5) / f64::from(viewport.height)) * 2.0);
    let clip = DVec4::new(ndc_x, ndc_y, f64::from(depth), 1.0);
    let world = world_from_clip * clip;
    if world.w.abs() <= f64::EPSILON {
        return None;
    }
    Some(world.truncate() / world.w)
}

#[cfg(target_arch = "wasm32")]
fn map_buffer_for_read_web(buffer: &wgpu::Buffer) -> Promise {
    let slice = buffer.slice(..);
    Promise::new(&mut |resolve, reject| {
        slice.map_async(wgpu::MapMode::Read, move |result| match result {
            Ok(()) => {
                let _ = resolve.call0(&JsValue::NULL);
            }
            Err(error) => {
                let _ = reject.call1(&JsValue::NULL, &JsValue::from_str(&error.to_string()));
            }
        });
    })
}

#[cfg(target_arch = "wasm32")]
fn align_to_web(value: u32, alignment: u32) -> u32 {
    let remainder = value % alignment;
    if remainder == 0 {
        value
    } else {
        value + (alignment - remainder)
    }
}

#[cfg(target_arch = "wasm32")]
fn strip_padded_rows_web(
    data: &[u8],
    unpadded_bytes_per_row: usize,
    padded_bytes_per_row: usize,
    height: usize,
) -> Vec<u8> {
    let mut rgba8 = vec![0; unpadded_bytes_per_row * height];

    for row in 0..height {
        let src_start = row * padded_bytes_per_row;
        let dst_start = row * unpadded_bytes_per_row;
        rgba8[dst_start..dst_start + unpadded_bytes_per_row]
            .copy_from_slice(&data[src_start..src_start + unpadded_bytes_per_row]);
    }

    rgba8
}

#[cfg(target_arch = "wasm32")]
fn web_pick_response_json(region: PickRegion, hits: &[PickHit]) -> Result<String, String> {
    let response = WebPickResponse {
        region: WebPickRegion {
            x: region.x,
            y: region.y,
            width: region.width,
            height: region.height,
        },
        hits: hits
            .iter()
            .map(|hit| {
                let source_resource =
                    source_resource_from_source_scoped_id(hit.element_id.as_str())
                        .map(str::to_owned);
                let local_instance_id = source_resource
                    .as_ref()
                    .map(|_| project_local_geometry_id(hit.instance_id.0));
                let local_definition_id = source_resource
                    .as_ref()
                    .map(|_| project_local_geometry_id(hit.definition_id.0));
                WebPickHit {
                    instance_id: hit.instance_id.0,
                    local_instance_id,
                    scoped_instance_id: source_resource.as_deref().zip(local_instance_id).map(
                        |(resource, id)| source_scoped_geometry_instance_id_string(resource, id),
                    ),
                    element_id: hit.element_id.as_str().to_string(),
                    definition_id: hit.definition_id.0,
                    local_definition_id,
                    scoped_definition_id: source_resource.as_deref().zip(local_definition_id).map(
                        |(resource, id)| source_scoped_geometry_definition_id_string(resource, id),
                    ),
                    source_resource,
                    world_centroid: [
                        hit.world_centroid.x,
                        hit.world_centroid.y,
                        hit.world_centroid.z,
                    ],
                    world_anchor: [hit.world_anchor.x, hit.world_anchor.y, hit.world_anchor.z],
                }
            })
            .collect(),
    };
    serde_json::to_string(&response)
        .map_err(|error| format!("failed to encode pick result JSON: {error}"))
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(start)]
pub fn start() {
    spawn_local(async {
        match WebViewerApp::new().await {
            Ok(app) => {
                WEB_VIEWER_APP.with(|slot| {
                    *slot.borrow_mut() = Some(app);
                });
            }
            Err(error) => log_viewer_error(&error),
        }
    });
}

#[cfg(target_arch = "wasm32")]
struct WebViewerApp {
    _state: Rc<RefCell<WebViewerState>>,
    _resource_change: Closure<dyn FnMut(Event)>,
    _profile_change: Closure<dyn FnMut(Event)>,
    _tool_change: Closure<dyn FnMut(Event)>,
    _mouse_down: Closure<dyn FnMut(MouseEvent)>,
    _mouse_move: Closure<dyn FnMut(MouseEvent)>,
    _mouse_up: Closure<dyn FnMut(MouseEvent)>,
    _mouse_leave: Closure<dyn FnMut(MouseEvent)>,
    _click: Closure<dyn FnMut(MouseEvent)>,
    _wheel: Closure<dyn FnMut(WheelEvent)>,
    _resize: Closure<dyn FnMut(Event)>,
    _animation_frame: Rc<RefCell<Option<Closure<dyn FnMut(f64)>>>>,
}

#[cfg(target_arch = "wasm32")]
impl WebViewerApp {
    async fn new() -> Result<Self, String> {
        let window = web_sys::window().ok_or("w web viewer could not access `window`")?;
        let document = window
            .document()
            .ok_or("w web viewer could not access `document`")?;
        let canvas = typed_element::<HtmlCanvasElement>(&document, "viewer-canvas")?;
        let axes_overlay = typed_element::<HtmlCanvasElement>(&document, "axes-overlay")?;
        let resource_picker = typed_element::<HtmlSelectElement>(&document, "resource-picker")?;
        let profile_picker =
            typed_element::<HtmlSelectElement>(&document, "render-profile-picker")?;
        let tool_picker = typed_element::<HtmlSelectElement>(&document, "tool-picker")?;
        let status_line = typed_element::<HtmlElement>(&document, "status-line")?;
        let resource_catalog = fetch_available_resource_catalog(&window).await?;
        let resources = resource_catalog_resources(&resource_catalog);
        populate_resource_picker(&resource_picker, &resources);
        let initial_resource = initial_web_resource(&window, &resources);
        resource_picker.set_value(&initial_resource);

        let axes_overlay_context = axes_overlay_context(&axes_overlay)?;
        let (width, height) = resize_canvases_to_window(&window, &canvas, &axes_overlay)?;
        let runtime_scene = fetch_runtime_scene(&window, &initial_resource).await?;
        let render_scene = runtime_scene.compose_render_scene();
        let camera = fit_camera_to_render_scene(&render_scene);
        let orbit = OrbitCameraController::from_camera(camera);

        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone()))
            .map_err(|error| format!("w web viewer could not create a GPU surface: {error}"))?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
            })
            .await
            .map_err(|error| format!("w web viewer could not request an adapter: {error}"))?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("w web device"),
                ..Default::default()
            })
            .await
            .map_err(|error| format!("w web viewer could not request a device: {error}"))?;
        let mut config = surface
            .get_default_config(&adapter, width, height)
            .ok_or("w web viewer could not determine a default surface configuration")?;
        config.width = width;
        config.height = height;
        surface.configure(&device, &config);

        let defaults = RenderDefaults {
            depth_format: wgpu::TextureFormat::Depth32Float,
            ..RenderDefaults::default()
        };
        let mut renderer = MeshRenderer::with_defaults(
            &device,
            config.format,
            ViewportSize::new(config.width, config.height),
            camera,
            defaults,
        );
        renderer.set_profile(RenderProfileId::ArchitecturalV3);
        renderer.set_reference_grid_visible(true);
        renderer.upload_prepared_scene(&device, &queue, &render_scene);
        populate_render_profile_picker(&profile_picker, renderer.available_profiles());
        profile_picker.set_value(renderer.profile().as_str());
        let depth_target = DepthTarget::with_defaults(
            &device,
            ViewportSize::new(config.width, config.height),
            defaults,
            "w web depth target",
        );

        let state = Rc::new(RefCell::new(WebViewerState {
            window: window.clone(),
            canvas: canvas.clone(),
            axes_overlay,
            axes_overlay_context,
            resource_picker: resource_picker.clone(),
            profile_picker: profile_picker.clone(),
            tool_picker: tool_picker.clone(),
            status_line,
            resource_catalog,
            current_resource: initial_resource.clone(),
            runtime_scene,
            surface,
            device,
            queue,
            config,
            renderer,
            depth_target,
            clear_color: defaults.clear_color,
            orbit,
            drag: DragState::default(),
            last_pick_hits: Vec::new(),
        }));
        state.borrow().refresh_status();

        let resource_state = state.clone();
        let resource_change = Closure::wrap(Box::new(move |_event: Event| {
            let resource = resource_state.borrow().resource_picker.value();
            let resource_state = resource_state.clone();
            spawn_local(async move {
                if let Err(error) = load_resource_into_state(resource_state, resource).await {
                    log_viewer_error(&error);
                }
            });
        }) as Box<dyn FnMut(Event)>);
        resource_picker
            .add_event_listener_with_callback("change", resource_change.as_ref().unchecked_ref())
            .map_err(js_error)?;

        let profile_state = state.clone();
        let profile_change = Closure::wrap(Box::new(move |_event: Event| {
            let result = {
                let mut state = profile_state.borrow_mut();
                let profile = state.profile_picker.value();
                state.apply_render_profile_name(&profile)
            };
            match result {
                Ok(Some(event)) => {
                    if let Err(error) = event.dispatch() {
                        log_viewer_error(&error);
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    let state = profile_state.borrow_mut();
                    state
                        .profile_picker
                        .set_value(state.renderer.profile().as_str());
                    state.set_status(&format!("Failed to set render profile: {error}"));
                    log_viewer_error(&error);
                }
            }
        }) as Box<dyn FnMut(Event)>);
        profile_picker
            .add_event_listener_with_callback("change", profile_change.as_ref().unchecked_ref())
            .map_err(js_error)?;

        let tool_state = state.clone();
        let tool_change = Closure::wrap(Box::new(move |_event: Event| {
            let event = tool_state.borrow_mut().sync_interaction_mode_from_picker();
            if let Some(event) = event {
                if let Err(error) = event.dispatch() {
                    log_viewer_error(&error);
                }
            }
        }) as Box<dyn FnMut(Event)>);
        tool_picker
            .add_event_listener_with_callback("change", tool_change.as_ref().unchecked_ref())
            .map_err(js_error)?;

        let mouse_down_state = state.clone();
        let mouse_down = Closure::wrap(Box::new(move |event: MouseEvent| {
            let event = mouse_down_state
                .borrow_mut()
                .begin_drag(event.client_x() as f32, event.client_y() as f32);
            if let Err(error) = event.dispatch() {
                log_viewer_error(&error);
            }
        }) as Box<dyn FnMut(MouseEvent)>);
        canvas
            .add_event_listener_with_callback("mousedown", mouse_down.as_ref().unchecked_ref())
            .map_err(js_error)?;

        let mouse_move_state = state.clone();
        let mouse_move = Closure::wrap(Box::new(move |event: MouseEvent| {
            let event = match mouse_move_state
                .borrow_mut()
                .drag_to(event.client_x() as f32, event.client_y() as f32)
            {
                Ok(event) => event,
                Err(error) => {
                    log_viewer_error(&error);
                    return;
                }
            };
            if let Some(event) = event {
                if let Err(error) = event.dispatch() {
                    log_viewer_error(&error);
                }
            }
        }) as Box<dyn FnMut(MouseEvent)>);
        window
            .add_event_listener_with_callback("mousemove", mouse_move.as_ref().unchecked_ref())
            .map_err(js_error)?;

        let mouse_up_state = state.clone();
        let mouse_up = Closure::wrap(Box::new(move |_event: MouseEvent| {
            let (request, event) = mouse_up_state.borrow_mut().end_drag();
            if let Err(error) = event.dispatch() {
                log_viewer_error(&error);
            }
            if let Some(request) = request {
                let mouse_up_state = mouse_up_state.clone();
                spawn_local(async move {
                    if let Err(error) = pick_region_in_state(mouse_up_state, request).await {
                        log_viewer_error(&error);
                    }
                });
            }
        }) as Box<dyn FnMut(MouseEvent)>);
        window
            .add_event_listener_with_callback("mouseup", mouse_up.as_ref().unchecked_ref())
            .map_err(js_error)?;

        let click_state = state.clone();
        let click = Closure::wrap(Box::new(move |event: MouseEvent| {
            let request = click_state
                .borrow_mut()
                .click_pick_request(event.client_x() as f32, event.client_y() as f32);
            if let Some(request) = request {
                let click_state = click_state.clone();
                spawn_local(async move {
                    if let Err(error) = pick_region_in_state(click_state, request).await {
                        log_viewer_error(&error);
                    }
                });
            }
        }) as Box<dyn FnMut(MouseEvent)>);
        canvas
            .add_event_listener_with_callback("click", click.as_ref().unchecked_ref())
            .map_err(js_error)?;

        let mouse_leave_state = state.clone();
        let mouse_leave = Closure::wrap(Box::new(move |event: MouseEvent| {
            if event.buttons() == 0 {
                let drag_event = mouse_leave_state.borrow_mut().cancel_drag();
                if let Some(drag_event) = drag_event {
                    if let Err(error) = drag_event.dispatch() {
                        log_viewer_error(&error);
                    }
                }
            }
        }) as Box<dyn FnMut(MouseEvent)>);
        canvas
            .add_event_listener_with_callback("mouseleave", mouse_leave.as_ref().unchecked_ref())
            .map_err(js_error)?;

        let wheel_state = state.clone();
        let wheel = Closure::wrap(Box::new(move |event: WheelEvent| {
            event.prevent_default();
            match wheel_state.borrow_mut().zoom(event.delta_y() as f32) {
                Ok(event) => {
                    if let Err(error) = event.dispatch() {
                        log_viewer_error(&error);
                    }
                }
                Err(error) => log_viewer_error(&error),
            }
        }) as Box<dyn FnMut(WheelEvent)>);
        canvas
            .add_event_listener_with_callback("wheel", wheel.as_ref().unchecked_ref())
            .map_err(js_error)?;

        let resize_state = state.clone();
        let resize = Closure::wrap(Box::new(move |_event: Event| {
            let event = match resize_state.borrow_mut().resize_to_window() {
                Ok(event) => event,
                Err(error) => {
                    log_viewer_error(&error);
                    return;
                }
            };
            if let Some(event) = event {
                if let Err(error) = event.dispatch() {
                    log_viewer_error(&error);
                }
            }
        }) as Box<dyn FnMut(Event)>);
        window
            .add_event_listener_with_callback("resize", resize.as_ref().unchecked_ref())
            .map_err(js_error)?;

        let animation_frame: Rc<RefCell<Option<Closure<dyn FnMut(f64)>>>> =
            Rc::new(RefCell::new(None));
        let animation_window = window.clone();
        let animation_state = state.clone();
        let animation_handle = animation_frame.clone();
        *animation_frame.borrow_mut() = Some(Closure::wrap(Box::new(move |_time: f64| {
            if let Err(error) = animation_state.borrow_mut().render() {
                log_viewer_error(&error);
                return;
            }

            if let Some(callback) = animation_handle.borrow().as_ref() {
                if let Err(error) =
                    animation_window.request_animation_frame(callback.as_ref().unchecked_ref())
                {
                    log_viewer_error(&js_error(error));
                }
            }
        }) as Box<dyn FnMut(f64)>));

        if let Some(callback) = animation_frame.borrow().as_ref() {
            window
                .request_animation_frame(callback.as_ref().unchecked_ref())
                .map_err(js_error)?;
        }

        Ok(Self {
            _state: state,
            _resource_change: resource_change,
            _profile_change: profile_change,
            _tool_change: tool_change,
            _mouse_down: mouse_down,
            _mouse_move: mouse_move,
            _mouse_up: mouse_up,
            _mouse_leave: mouse_leave,
            _click: click,
            _wheel: wheel,
            _resize: resize,
            _animation_frame: animation_frame,
        })
    }
}

#[cfg(target_arch = "wasm32")]
struct WebViewerState {
    window: Window,
    canvas: HtmlCanvasElement,
    axes_overlay: HtmlCanvasElement,
    axes_overlay_context: CanvasRenderingContext2d,
    resource_picker: HtmlSelectElement,
    profile_picker: HtmlSelectElement,
    tool_picker: HtmlSelectElement,
    status_line: HtmlElement,
    resource_catalog: WebResourceCatalog,
    current_resource: String,
    runtime_scene: RuntimeSceneState,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    renderer: MeshRenderer,
    depth_target: DepthTarget,
    clear_color: wgpu::Color,
    orbit: OrbitCameraController,
    drag: DragState,
    last_pick_hits: Vec<PickHit>,
}

#[cfg(target_arch = "wasm32")]
struct WebPickReadback {
    region: PickRegion,
    viewport_width: u32,
    viewport_height: u32,
    clip_from_world: DMat4,
    pick_targets: Vec<PickHit>,
    readback: wgpu::Buffer,
    map_promise: Promise,
    unpadded_bytes_per_row: u32,
    padded_bytes_per_row: u32,
    depth_readback: wgpu::Buffer,
    depth_map_promise: Promise,
    depth_unpadded_bytes_per_row: u32,
    depth_padded_bytes_per_row: u32,
}

#[cfg(target_arch = "wasm32")]
struct DeferredWebEvent {
    window: Window,
    event_name: &'static str,
    json: String,
}

#[cfg(target_arch = "wasm32")]
impl DeferredWebEvent {
    fn dispatch(self) -> Result<(), String> {
        dispatch_json_event(&self.window, self.event_name, &self.json)
    }
}

#[cfg(target_arch = "wasm32")]
fn dispatch_web_events(events: Vec<DeferredWebEvent>) -> Result<(), String> {
    for event in events {
        event.dispatch()?;
    }
    Ok(())
}

#[cfg(target_arch = "wasm32")]
impl WebViewerState {
    fn begin_resource_load(&mut self, resource: &str) {
        self.resource_picker.set_disabled(true);
        self.resource_picker.set_value(resource);
        self.set_status(&format!("Loading {}...", friendly_resource_label(resource)));
    }

    fn finish_resource_load_failed(&mut self, resource: &str, error: &str) {
        self.resource_picker.set_disabled(false);
        self.resource_picker.set_value(&self.current_resource);
        self.set_status(&format!(
            "Failed to load {}: {}",
            friendly_resource_label(resource),
            error
        ));
    }

    fn apply_runtime_scene(
        &mut self,
        resource: String,
        runtime_scene: RuntimeSceneState,
    ) -> Result<Vec<DeferredWebEvent>, String> {
        self.runtime_scene = runtime_scene;
        self.current_resource = resource.clone();
        self.resource_picker.set_disabled(false);
        self.resource_picker.set_value(&resource);
        self.last_pick_hits.clear();
        Ok(vec![
            self.upload_runtime_scene(true)?,
            self.viewer_state_change_event("resource")?,
        ])
    }

    fn apply_render_profile_name(
        &mut self,
        profile_name: &str,
    ) -> Result<Option<DeferredWebEvent>, String> {
        let profile = parse_render_profile_id(profile_name, self.renderer.available_profiles())?;
        self.profile_picker.set_value(profile.as_str());
        if profile == self.renderer.profile() {
            self.refresh_status();
            return Ok(None);
        }

        self.renderer.set_profile(profile);
        self.refresh_status();
        Ok(Some(self.viewer_state_change_event("renderProfile")?))
    }

    fn set_clear_color(&mut self, red: f64, green: f64, blue: f64) {
        self.clear_color = wgpu::Color {
            r: red.clamp(0.0, 1.0),
            g: green.clamp(0.0, 1.0),
            b: blue.clamp(0.0, 1.0),
            a: 1.0,
        };
    }

    fn resize_to_window(&mut self) -> Result<Option<DeferredWebEvent>, String> {
        let (width, height) =
            resize_canvases_to_window(&self.window, &self.canvas, &self.axes_overlay)?;
        if self.config.width == width && self.config.height == height {
            return Ok(None);
        }

        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.depth_target = DepthTarget::with_defaults(
            &self.device,
            ViewportSize::new(width, height),
            self.renderer.defaults(),
            "w web depth target",
        );
        self.renderer
            .resize(&self.device, &self.queue, ViewportSize::new(width, height));
        self.draw_world_axes_overlay()?;

        Ok(Some(self.pick_anchor_event()?))
    }

    fn upload_runtime_scene(&mut self, reset_camera: bool) -> Result<DeferredWebEvent, String> {
        let render_scene = self.runtime_scene.compose_render_scene();
        self.renderer
            .upload_prepared_scene(&self.device, &self.queue, &render_scene);
        if reset_camera {
            let camera = fit_camera_to_render_scene(&render_scene);
            self.orbit = OrbitCameraController::from_camera(camera);
            self.renderer.set_camera(&self.queue, self.orbit.camera());
        }
        self.refresh_status();
        self.pick_anchor_event()
    }

    fn commit_runtime_scene_change(
        &mut self,
        reset_camera: bool,
        reason: &str,
    ) -> Result<Vec<DeferredWebEvent>, String> {
        Ok(vec![
            self.upload_runtime_scene(reset_camera)?,
            self.viewer_state_change_event(reason)?,
        ])
    }

    fn frame_visible_scene(&mut self) -> Result<DeferredWebEvent, String> {
        let render_scene = self.runtime_scene.compose_render_scene();
        let camera = fit_camera_to_render_scene(&render_scene);
        self.orbit = OrbitCameraController::from_camera(camera);
        self.renderer.set_camera(&self.queue, self.orbit.camera());
        self.refresh_status();
        self.pick_anchor_event()
    }

    fn begin_drag(&mut self, x: f32, y: f32) -> DeferredWebEvent {
        self.drag.active = true;
        self.drag.suppress_next_click = false;
        self.drag.start_x = x;
        self.drag.start_y = y;
        self.drag.last_x = x;
        self.drag.last_y = y;
        self.web_event("w-viewer-drag-start", r#"{}"#.to_string())
    }

    fn drag_to(&mut self, x: f32, y: f32) -> Result<Option<DeferredWebEvent>, String> {
        if !self.drag.active {
            return Ok(None);
        }

        let mode = self.interaction_mode();
        let dx = x - self.drag.last_x;
        let dy = y - self.drag.last_y;
        self.drag.last_x = x;
        self.drag.last_y = y;
        if !mode.can_orbit() {
            return Ok(None);
        }
        self.orbit.orbit_by_pixels(dx, dy);
        self.renderer.set_camera(&self.queue, self.orbit.camera());
        Ok(Some(self.pick_anchor_event()?))
    }

    fn end_drag(&mut self) -> (Option<WebPickRequest>, DeferredWebEvent) {
        let request = if self.drag.active {
            let mode = self.interaction_mode();
            let is_box_select = self.drag.is_box_select();
            self.drag.suppress_next_click = is_box_select;
            if mode == WebInteractionMode::Pick && is_box_select {
                Some(self.drag_pick_request())
            } else {
                None
            }
        } else {
            None
        };
        self.drag.active = false;
        (
            request,
            self.web_event("w-viewer-drag-end", r#"{}"#.to_string()),
        )
    }

    fn cancel_drag(&mut self) -> Option<DeferredWebEvent> {
        let was_active = self.drag.active;
        self.drag.active = false;
        self.drag.suppress_next_click = false;
        if was_active {
            Some(self.web_event("w-viewer-drag-end", r#"{}"#.to_string()))
        } else {
            None
        }
    }

    fn zoom(&mut self, delta_y: f32) -> Result<DeferredWebEvent, String> {
        self.orbit.zoom_by_wheel(delta_y);
        self.renderer.set_camera(&self.queue, self.orbit.camera());
        self.pick_anchor_event()
    }

    fn sync_interaction_mode_from_picker(&mut self) -> Option<DeferredWebEvent> {
        let drag_event = self.cancel_drag();
        self.refresh_status();
        drag_event
    }

    fn interaction_mode(&self) -> WebInteractionMode {
        WebInteractionMode::from_picker_value(&self.tool_picker.value())
    }

    fn click_pick_request(&mut self, x: f32, y: f32) -> Option<WebPickRequest> {
        if self.drag.take_suppress_next_click() || !self.interaction_mode().can_pick() {
            return None;
        }
        let (x, y) = self.client_to_canvas_css(x, y);
        Some(WebPickRequest::Point { x, y })
    }

    fn drag_pick_request(&self) -> WebPickRequest {
        let start = self.client_to_canvas_css(self.drag.start_x, self.drag.start_y);
        let end = self.client_to_canvas_css(self.drag.last_x, self.drag.last_y);
        if self.drag.is_box_select() {
            WebPickRequest::Rect {
                x0: start.0,
                y0: start.1,
                x1: end.0,
                y1: end.1,
            }
        } else {
            WebPickRequest::Point { x: end.0, y: end.1 }
        }
    }

    fn client_to_canvas_css(&self, client_x: f32, client_y: f32) -> (f32, f32) {
        let rect = self.canvas_element().get_bounding_client_rect();
        (
            (client_x - rect.left() as f32).clamp(0.0, rect.width().max(1.0) as f32),
            (client_y - rect.top() as f32).clamp(0.0, rect.height().max(1.0) as f32),
        )
    }

    fn css_pick_request_to_region(&self, request: WebPickRequest) -> PickRegion {
        match request {
            WebPickRequest::Point { x, y } => {
                let (px, py) = self.canvas_css_to_surface_pixel(x, y);
                PickRegion::pixel(px, py)
            }
            WebPickRequest::Rect { x0, y0, x1, y1 } => {
                let (px0, py0) = self.canvas_css_to_surface_pixel(x0.min(x1), y0.min(y1));
                let (px1, py1) = self.canvas_css_to_surface_pixel(x0.max(x1), y0.max(y1));
                PickRegion::rect(
                    px0,
                    py0,
                    px1.saturating_sub(px0).saturating_add(1),
                    py1.saturating_sub(py0).saturating_add(1),
                )
            }
        }
    }

    fn canvas_css_to_surface_pixel(&self, x: f32, y: f32) -> (u32, u32) {
        let rect = self.canvas_element().get_bounding_client_rect();
        let width = rect.width().max(1.0) as f32;
        let height = rect.height().max(1.0) as f32;
        let px = ((x.clamp(0.0, width) / width) * self.config.width as f32)
            .floor()
            .clamp(0.0, self.config.width.saturating_sub(1) as f32) as u32;
        let py = ((y.clamp(0.0, height) / height) * self.config.height as f32)
            .floor()
            .clamp(0.0, self.config.height.saturating_sub(1) as f32) as u32;
        (px, py)
    }

    fn canvas_element(&self) -> &Element {
        self.canvas.unchecked_ref::<Element>()
    }

    fn clamp_pick_region(&self, region: PickRegion) -> PickRegion {
        if region.x >= self.config.width || region.y >= self.config.height {
            return PickRegion::pixel(
                self.config.width.saturating_sub(1),
                self.config.height.saturating_sub(1),
            );
        }
        PickRegion::rect(
            region.x,
            region.y,
            region.width.min(self.config.width - region.x),
            region.height.min(self.config.height - region.y),
        )
    }

    fn prepare_pick_readback(
        &mut self,
        request: WebPickRequest,
    ) -> Result<WebPickReadback, String> {
        let region = self.css_pick_request_to_region(request);
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("w web pick texture"),
            size: wgpu::Extent3d {
                width: self.config.width,
                height: self.config.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Uint,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("w web pick texture view"),
            ..Default::default()
        });
        let depth_format = self.renderer.defaults().depth_format;
        if depth_format != wgpu::TextureFormat::Depth32Float {
            return Err(format!(
                "pick anchors require Depth32Float readback; renderer uses {depth_format:?}"
            ));
        }
        let depth_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("w web pick depth texture"),
            size: wgpu::Extent3d {
                width: self.config.width,
                height: self.config.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: depth_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let depth_view = depth_texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("w web pick depth texture view"),
            ..Default::default()
        });
        let region = self.clamp_pick_region(region);
        let unpadded_bytes_per_row = region
            .width
            .checked_mul(4)
            .ok_or("pick region row is too wide")?;
        let padded_bytes_per_row =
            align_to_web(unpadded_bytes_per_row, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let depth_unpadded_bytes_per_row = unpadded_bytes_per_row;
        let depth_padded_bytes_per_row = padded_bytes_per_row;
        let readback_size = u64::from(padded_bytes_per_row)
            .checked_mul(u64::from(region.height))
            .ok_or("pick region is too large")?;
        let depth_readback_size = u64::from(depth_padded_bytes_per_row)
            .checked_mul(u64::from(region.height))
            .ok_or("pick depth region is too large")?;
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("w web pick readback buffer"),
            size: readback_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let depth_readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("w web pick depth readback buffer"),
            size: depth_readback_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("w web pick encoder"),
            });
        self.renderer
            .render_pick_region(&mut encoder, &view, &depth_view, region);
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: region.x,
                    y: region.y,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row),
                    rows_per_image: Some(region.height),
                },
            },
            wgpu::Extent3d {
                width: region.width,
                height: region.height,
                depth_or_array_layers: 1,
            },
        );
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &depth_texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: region.x,
                    y: region.y,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::DepthOnly,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &depth_readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(depth_padded_bytes_per_row),
                    rows_per_image: Some(region.height),
                },
            },
            wgpu::Extent3d {
                width: region.width,
                height: region.height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);

        let map_promise = map_buffer_for_read_web(&readback);
        let depth_map_promise = map_buffer_for_read_web(&depth_readback);
        let _ = self.device.poll(wgpu::PollType::Poll);

        Ok(WebPickReadback {
            region,
            viewport_width: self.config.width,
            viewport_height: self.config.height,
            clip_from_world: self
                .renderer
                .camera()
                .clip_from_world(ViewportSize::new(self.config.width, self.config.height)),
            pick_targets: self.renderer.pick_targets().to_vec(),
            readback,
            map_promise,
            unpadded_bytes_per_row,
            padded_bytes_per_row,
            depth_readback,
            depth_map_promise,
            depth_unpadded_bytes_per_row,
            depth_padded_bytes_per_row,
        })
    }

    fn apply_pick_hits(&mut self, hits: Vec<PickHit>) {
        self.runtime_scene.clear_selection();
        self.last_pick_hits = hits;
        // Picking should update runtime state only. The ID-color pass is offscreen, and the
        // visible canvas should keep rendering the normal material scene. Selection remains an
        // explicit viewer/API action so picking cannot leak selected-material color into the view.
        self.refresh_status();
    }

    fn web_event(&self, event_name: &'static str, json: String) -> DeferredWebEvent {
        DeferredWebEvent {
            window: self.window.clone(),
            event_name,
            json,
        }
    }

    fn pick_anchor_event(&self) -> Result<DeferredWebEvent, String> {
        Ok(self.web_event("w-viewer-anchor", self.pick_anchor_event_json()?))
    }

    fn pick_anchor_event_json(&self) -> Result<String, String> {
        let Some(hit) = self.last_pick_hits.first() else {
            return Ok(r#"{"visible":false}"#.to_string());
        };
        let Some((client_x, client_y, canvas_x, canvas_y)) =
            self.project_world_to_client(hit.world_anchor)
        else {
            return Ok(r#"{"visible":false}"#.to_string());
        };
        serde_json::to_string(&WebPickAnchorEvent {
            visible: true,
            client_x,
            client_y,
            canvas_x,
            canvas_y,
            element_id: hit.element_id.as_str().to_string(),
            instance_id: hit.instance_id.0,
            definition_id: hit.definition_id.0,
            world_anchor: [hit.world_anchor.x, hit.world_anchor.y, hit.world_anchor.z],
        })
        .map_err(|error| format!("failed to encode pick anchor event JSON: {error}"))
    }

    fn project_world_to_client(&self, point: DVec3) -> Option<(f64, f64, f64, f64)> {
        let viewport = ViewportSize::new(self.config.width, self.config.height).clamped();
        let clip = self.renderer.camera().clip_from_world(viewport)
            * DVec4::new(point.x, point.y, point.z, 1.0);
        if clip.w <= f64::EPSILON {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        if ndc.z < 0.0 || ndc.z > 1.0 || ndc.x < -1.0 || ndc.x > 1.0 || ndc.y < -1.0 || ndc.y > 1.0
        {
            return None;
        }
        let canvas_x = ((ndc.x + 1.0) * 0.5) * f64::from(viewport.width);
        let canvas_y = (1.0 - ((ndc.y + 1.0) * 0.5)) * f64::from(viewport.height);
        let rect = self.canvas_element().get_bounding_client_rect();
        let client_x = rect.left() + (canvas_x / f64::from(viewport.width)) * rect.width();
        let client_y = rect.top() + (canvas_y / f64::from(viewport.height)) * rect.height();
        Some((client_x, client_y, canvas_x, canvas_y))
    }

    fn render(&mut self) -> Result<(), String> {
        if self.config.width == 0 || self.config.height == 0 {
            return Ok(());
        }

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                self.depth_target = DepthTarget::with_defaults(
                    &self.device,
                    ViewportSize::new(self.config.width, self.config.height),
                    self.renderer.defaults(),
                    "w web depth target",
                );
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err("w web viewer hit a surface validation error".to_string());
            }
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("w web frame encoder"),
            });
        self.renderer.render_with_clear_color_and_device(
            &self.device,
            &mut encoder,
            &view,
            self.depth_target.view(),
            self.clear_color,
        );
        self.queue.submit([encoder.finish()]);
        frame.present();
        self.draw_world_axes_overlay()?;

        Ok(())
    }

    fn set_status(&self, message: &str) {
        self.status_line.set_text_content(Some(message));
    }

    fn refresh_status(&self) {
        self.set_status(&web_viewer_status_line(
            &self.runtime_scene,
            self.renderer.profile(),
        ));
    }

    fn render_profiles_json(&self) -> Result<String, String> {
        serde_json::to_string(&web_render_profile_descriptors(
            self.renderer.available_profiles(),
        ))
        .map_err(|error| format!("failed to encode renderer profiles JSON: {error}"))
    }

    fn view_state_json(&self) -> Result<String, String> {
        let catalog = self.runtime_scene.catalog();
        let render_scene = self.runtime_scene.compose_render_scene();
        let residency = self.runtime_scene.residency_counts();
        let missing = self
            .runtime_scene
            .missing_stream_plan_for_visible_elements();
        let selected_element_ids = self.runtime_scene.selected_element_ids();
        let selected_instance_ids =
            selected_instance_ids_from_catalog(&catalog, &selected_element_ids);
        let default_element_ids = self
            .runtime_scene
            .resolve_start_view(&GeometryStartViewRequest::Default)
            .visible_element_ids;
        let snapshot = WebViewerViewState {
            resource: self.current_resource.clone(),
            view_mode: web_view_mode_name(self.runtime_scene.start_view_request()).to_string(),
            render_profile: self.renderer.profile().as_str().to_string(),
            reference_grid_visible: self.renderer.reference_grid_visible(),
            available_render_profiles: web_render_profile_descriptors(
                self.renderer.available_profiles(),
            ),
            total_elements: catalog.elements.len(),
            total_instances: catalog.instances.len(),
            total_definitions: catalog.definitions.len(),
            default_element_ids: semantic_ids_to_strings(default_element_ids),
            base_visible_element_ids: semantic_ids_to_strings(
                self.runtime_scene.base_visible_element_ids(),
            ),
            visible_element_ids: semantic_ids_to_strings(self.runtime_scene.visible_element_ids()),
            selected_element_ids: semantic_ids_to_strings(selected_element_ids.clone()),
            selected_instance_ids,
            picked_instance_ids: self
                .last_pick_hits
                .iter()
                .map(|hit| hit.instance_id.0)
                .collect(),
            hidden_element_ids: semantic_ids_to_strings(self.runtime_scene.hidden_element_ids()),
            shown_element_ids: semantic_ids_to_strings(self.runtime_scene.shown_element_ids()),
            suppressed_element_ids: semantic_ids_to_strings(
                self.runtime_scene.suppressed_element_ids(),
            ),
            resident_instances: residency.instances,
            resident_definitions: residency.definitions,
            missing_instance_ids: missing.instance_ids.iter().map(|id| id.0).collect(),
            missing_definition_ids: missing.definition_ids.iter().map(|id| id.0).collect(),
            triangles: render_scene.triangle_count(),
            draws: render_scene.draw_count(),
        };
        serde_json::to_string(&snapshot)
            .map_err(|error| format!("failed to encode viewer view state JSON: {error}"))
    }

    fn viewer_state_change_event(&self, reason: &str) -> Result<DeferredWebEvent, String> {
        let state: serde_json::Value = serde_json::from_str(&self.view_state_json()?)
            .map_err(|error| format!("failed to reparse viewer state JSON: {error}"))?;
        let json = serde_json::to_string(&serde_json::json!({
            "reason": reason,
            "state": state,
        }))
        .map_err(|error| format!("failed to encode viewer state change JSON: {error}"))?;
        Ok(self.web_event("w-viewer-state-change", json))
    }

    fn draw_world_axes_overlay(&self) -> Result<(), String> {
        let device_pixel_ratio = self.window.device_pixel_ratio() as f32;
        let width = self.axes_overlay.width() as f32 / device_pixel_ratio.max(1.0);
        let height = self.axes_overlay.height() as f32 / device_pixel_ratio.max(1.0);
        if width <= 0.0 || height <= 0.0 {
            return Ok(());
        }

        self.axes_overlay_context
            .set_transform(
                f64::from(device_pixel_ratio),
                0.0,
                0.0,
                f64::from(device_pixel_ratio),
                0.0,
                0.0,
            )
            .map_err(js_error)?;
        self.axes_overlay_context
            .clear_rect(0.0, 0.0, f64::from(width), f64::from(height));

        let radius = 34.0_f32;
        let margin = 16.0_f32;
        let left_offset = -5.0_f32;
        let bottom_offset = 7.0_f32;
        let center = Vec2::new(
            margin + radius + left_offset,
            height - margin - radius + bottom_offset,
        );
        let axis_length = 24.0_f32;
        let origin_fill = "rgba(241, 244, 250, 0.82)";

        let mut axes = [
            overlay_axis(self.renderer.camera(), WORLD_RIGHT, "X", "#ee6352"),
            overlay_axis(self.renderer.camera(), WORLD_FORWARD, "Y", "#66d6a6"),
            overlay_axis(self.renderer.camera(), WORLD_UP, "Z", "#6ea8fe"),
        ];
        axes.sort_by(|left, right| right.depth.total_cmp(&left.depth));

        self.axes_overlay_context.set_fill_style_str(origin_fill);
        self.axes_overlay_context.begin_path();
        self.axes_overlay_context
            .arc(
                f64::from(center.x),
                f64::from(center.y),
                3.5,
                0.0,
                std::f64::consts::TAU,
            )
            .map_err(js_error)?;
        self.axes_overlay_context.fill();

        for axis in axes {
            draw_overlay_axis(&self.axes_overlay_context, center, axis_length, &axis)?;
        }

        self.draw_pick_drag_overlay()?;

        Ok(())
    }

    fn draw_pick_drag_overlay(&self) -> Result<(), String> {
        let mode = self.interaction_mode();
        if !mode.can_pick() || mode.can_orbit() || !self.drag.active || !self.drag.is_box_select() {
            return Ok(());
        }

        let start = self.client_to_canvas_css(self.drag.start_x, self.drag.start_y);
        let end = self.client_to_canvas_css(self.drag.last_x, self.drag.last_y);
        let x = start.0.min(end.0);
        let y = start.1.min(end.1);
        let width = (start.0 - end.0).abs();
        let height = (start.1 - end.1).abs();

        self.axes_overlay_context
            .set_fill_style_str("rgba(146, 219, 255, 0.12)");
        self.axes_overlay_context.fill_rect(
            f64::from(x),
            f64::from(y),
            f64::from(width),
            f64::from(height),
        );
        self.axes_overlay_context
            .set_stroke_style_str("rgba(146, 219, 255, 0.95)");
        self.axes_overlay_context.set_line_width(1.5);
        self.axes_overlay_context.stroke_rect(
            f64::from(x),
            f64::from(y),
            f64::from(width),
            f64::from(height),
        );
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy, Debug, Default)]
struct DragState {
    active: bool,
    suppress_next_click: bool,
    start_x: f32,
    start_y: f32,
    last_x: f32,
    last_y: f32,
}

#[cfg(target_arch = "wasm32")]
impl DragState {
    const PICK_DRAG_THRESHOLD_PIXELS: f32 = 4.0;

    fn is_box_select(self) -> bool {
        let dx = self.last_x - self.start_x;
        let dy = self.last_y - self.start_y;
        dx.hypot(dy) >= Self::PICK_DRAG_THRESHOLD_PIXELS
    }

    fn take_suppress_next_click(&mut self) -> bool {
        let suppress = self.suppress_next_click;
        self.suppress_next_click = false;
        suppress
    }
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WebInteractionMode {
    None,
    Orbit,
    Pick,
    OrbitPick,
}

#[cfg(target_arch = "wasm32")]
impl WebInteractionMode {
    fn from_picker_value(value: &str) -> Self {
        match value {
            "none" => Self::None,
            "pick" => Self::Pick,
            "orbit-pick" | "pick-orbit" => Self::OrbitPick,
            _ => Self::Orbit,
        }
    }

    fn can_orbit(self) -> bool {
        matches!(self, Self::Orbit | Self::OrbitPick)
    }

    fn can_pick(self) -> bool {
        matches!(self, Self::Pick | Self::OrbitPick)
    }
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy, Debug)]
struct OrbitCameraController {
    target: DVec3,
    radius: f64,
    yaw_radians: f64,
    pitch_radians: f64,
    vertical_fov_degrees: f64,
    near_plane: f64,
    far_plane: f64,
}

#[cfg(target_arch = "wasm32")]
impl OrbitCameraController {
    fn from_camera(camera: Camera) -> Self {
        let offset = camera.eye - camera.target;
        let radius = offset.length().max(0.25);
        let pitch_radians = (offset.z / radius).clamp(-1.0, 1.0).asin();
        let yaw_radians = offset.x.atan2(-offset.y);

        Self {
            target: camera.target,
            radius,
            yaw_radians,
            pitch_radians,
            vertical_fov_degrees: camera.vertical_fov_degrees,
            near_plane: camera.near_plane,
            far_plane: camera.far_plane,
        }
    }

    fn camera(&self) -> Camera {
        let cos_pitch = self.pitch_radians.cos();
        let offset = DVec3::new(
            self.yaw_radians.sin() * cos_pitch,
            -self.yaw_radians.cos() * cos_pitch,
            self.pitch_radians.sin(),
        ) * self.radius;

        Camera {
            eye: self.target + offset,
            target: self.target,
            up: WORLD_UP,
            vertical_fov_degrees: self.vertical_fov_degrees,
            near_plane: self.near_plane,
            far_plane: self.far_plane.max(self.radius * 8.0),
        }
    }

    fn orbit_by_pixels(&mut self, dx: f32, dy: f32) {
        const ORBIT_SENSITIVITY: f64 = 0.01;
        const MAX_PITCH: f64 = 1.52;

        self.yaw_radians -= f64::from(dx) * ORBIT_SENSITIVITY;
        self.pitch_radians =
            (self.pitch_radians + (f64::from(dy) * ORBIT_SENSITIVITY)).clamp(-MAX_PITCH, MAX_PITCH);
    }

    fn zoom_by_wheel(&mut self, delta_y: f32) {
        let scale = (f64::from(delta_y) * 0.0015).exp();
        self.radius = (self.radius * scale).clamp(0.2, 500.0);
    }
}

#[cfg(target_arch = "wasm32")]
fn typed_element<T>(document: &Document, id: &str) -> Result<T, String>
where
    T: JsCast,
{
    document
        .get_element_by_id(id)
        .ok_or_else(|| format!("w web viewer is missing the `#{id}` element"))?
        .dyn_into::<T>()
        .map_err(|_| format!("w web viewer found `#{id}` but it has the wrong element type"))
}

#[cfg(target_arch = "wasm32")]
fn populate_resource_picker(resource_picker: &HtmlSelectElement, resources: &[String]) {
    let options = resources
        .iter()
        .map(|resource| {
            format!(
                "<option value=\"{resource}\">{}</option>",
                friendly_resource_label(resource)
            )
        })
        .collect::<Vec<_>>()
        .join("");
    let element: &Element = resource_picker.unchecked_ref();
    element.set_inner_html(&options);
}

#[cfg(target_arch = "wasm32")]
fn populate_render_profile_picker(
    profile_picker: &HtmlSelectElement,
    profiles: &[RenderProfileDescriptor],
) {
    let options = profiles
        .iter()
        .map(|profile| {
            format!(
                "<option value=\"{}\">{}</option>",
                profile.name, profile.label
            )
        })
        .collect::<Vec<_>>()
        .join("");
    let element: &Element = profile_picker.unchecked_ref();
    element.set_inner_html(&options);
}

#[cfg(target_arch = "wasm32")]
fn parse_render_profile_id(
    profile_name: &str,
    available_profiles: &[RenderProfileDescriptor],
) -> Result<RenderProfileId, String> {
    let profile = profile_name
        .parse::<RenderProfileId>()
        .map_err(|error| error.to_string())?;
    if available_profiles
        .iter()
        .any(|descriptor| descriptor.id == profile)
    {
        Ok(profile)
    } else {
        Err(format!(
            "render profile `{}` is not available",
            profile.as_str()
        ))
    }
}

#[cfg(target_arch = "wasm32")]
fn web_render_profile_descriptors(
    profiles: &[RenderProfileDescriptor],
) -> Vec<WebRenderProfileDescriptor> {
    profiles
        .iter()
        .map(|profile| WebRenderProfileDescriptor {
            id: profile.id.as_str().to_string(),
            name: profile.name.to_string(),
            label: profile.label.to_string(),
        })
        .collect()
}

#[cfg(target_arch = "wasm32")]
fn friendly_resource_label(resource: &str) -> &str {
    match resource {
        "demo/mapped-pentagon-pair" => "mapped-pentagon-pair (per-instance color)",
        _ if resource.starts_with("demo/") => resource.trim_start_matches("demo/"),
        _ if resource.starts_with("ifc/") => resource.trim_start_matches("ifc/"),
        _ if resource.starts_with("project/") => resource.trim_start_matches("project/"),
        _ => resource,
    }
}

#[cfg(target_arch = "wasm32")]
fn initial_web_resource(window: &Window, resources: &[String]) -> String {
    window
        .location()
        .search()
        .ok()
        .and_then(|search| resource_from_search(&search))
        .filter(|resource| resources.iter().any(|candidate| candidate == resource))
        .unwrap_or_else(|| {
            resources
                .iter()
                .find(|resource| resource.as_str() == DEFAULT_WEB_RESOURCE)
                .cloned()
                .or_else(|| resources.first().cloned())
                .unwrap_or_else(|| DEFAULT_WEB_RESOURCE.to_string())
        })
}

#[cfg(target_arch = "wasm32")]
fn resource_from_search(search: &str) -> Option<String> {
    search.trim_start_matches('?').split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        if key != "resource" || value.is_empty() {
            return None;
        }

        decode_uri_component(value).ok()?.as_string()
    })
}

#[cfg(target_arch = "wasm32")]
fn resize_canvas_to_window(
    window: &Window,
    canvas: &HtmlCanvasElement,
) -> Result<(u32, u32), String> {
    let device_pixel_ratio = window.device_pixel_ratio();
    let client_width = canvas.client_width().max(1) as f64;
    let client_height = canvas.client_height().max(1) as f64;
    let fallback_width = window
        .inner_width()
        .map_err(js_error)?
        .as_f64()
        .unwrap_or(1280.0);
    let fallback_height = window
        .inner_height()
        .map_err(js_error)?
        .as_f64()
        .unwrap_or(720.0);
    let width = if client_width > 1.0 {
        client_width
    } else {
        fallback_width
    };
    let height = if client_height > 1.0 {
        client_height
    } else {
        fallback_height.max(240.0)
    };
    let surface_width = (width * device_pixel_ratio).round().max(1.0) as u32;
    let surface_height = (height * device_pixel_ratio).round().max(1.0) as u32;

    canvas.set_width(surface_width);
    canvas.set_height(surface_height);

    Ok((surface_width, surface_height))
}

#[cfg(target_arch = "wasm32")]
fn resize_canvases_to_window(
    window: &Window,
    canvas: &HtmlCanvasElement,
    axes_overlay: &HtmlCanvasElement,
) -> Result<(u32, u32), String> {
    let (surface_width, surface_height) = resize_canvas_to_window(window, canvas)?;
    axes_overlay.set_width(surface_width);
    axes_overlay.set_height(surface_height);
    Ok((surface_width, surface_height))
}

#[cfg(target_arch = "wasm32")]
fn axes_overlay_context(canvas: &HtmlCanvasElement) -> Result<CanvasRenderingContext2d, String> {
    canvas
        .get_context("2d")
        .map_err(js_error)?
        .ok_or("w web viewer could not create the axes overlay context".to_string())?
        .dyn_into::<CanvasRenderingContext2d>()
        .map_err(|_| {
            "w web viewer created the axes overlay with the wrong context type".to_string()
        })
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy)]
struct OverlayAxis {
    label: &'static str,
    color: &'static str,
    direction: Vec2,
    depth: f32,
}

#[cfg(target_arch = "wasm32")]
fn overlay_axis(
    camera: Camera,
    axis: DVec3,
    label: &'static str,
    color: &'static str,
) -> OverlayAxis {
    let camera_space = camera.view_from_world().transform_vector3(axis);

    OverlayAxis {
        label,
        color,
        direction: Vec2::new(camera_space.x as f32, -(camera_space.y as f32)),
        depth: camera_space.z as f32,
    }
}

#[cfg(target_arch = "wasm32")]
fn draw_overlay_axis(
    context: &CanvasRenderingContext2d,
    center: Vec2,
    axis_length: f32,
    axis: &OverlayAxis,
) -> Result<(), String> {
    let vector = axis.direction * axis_length;
    let tip = center + vector;
    let head_direction = if axis.direction.length_squared() > 1.0e-6 {
        axis.direction.normalize()
    } else {
        Vec2::new(0.0, -1.0)
    };
    let arrow_head_length = 8.0;
    let arrow_head_angle = 0.55;
    let left = rotate_vec2(head_direction, arrow_head_angle) * arrow_head_length;
    let right = rotate_vec2(head_direction, -arrow_head_angle) * arrow_head_length;

    context.set_stroke_style_str(axis.color);
    context.set_fill_style_str(axis.color);
    context.set_line_width(2.25);

    context.begin_path();
    context.move_to(f64::from(center.x), f64::from(center.y));
    context.line_to(f64::from(tip.x), f64::from(tip.y));
    context.stroke();

    context.begin_path();
    context.move_to(f64::from(tip.x), f64::from(tip.y));
    context.line_to(f64::from((tip - left).x), f64::from((tip - left).y));
    context.move_to(f64::from(tip.x), f64::from(tip.y));
    context.line_to(f64::from((tip - right).x), f64::from((tip - right).y));
    context.stroke();

    let label_offset = if axis.direction.length_squared() > 1.0e-6 {
        axis.direction.normalize() * 12.0
    } else {
        Vec2::new(0.0, -12.0)
    };
    let label_position = tip + label_offset;
    context.set_font("12px Inter, system-ui, sans-serif");
    context.set_text_align("center");
    context.set_text_baseline("middle");
    context
        .fill_text(
            axis.label,
            f64::from(label_position.x),
            f64::from(label_position.y),
        )
        .map_err(js_error)?;

    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn rotate_vec2(vector: Vec2, radians: f32) -> Vec2 {
    let (sin, cos) = radians.sin_cos();
    Vec2::new(
        (vector.x * cos) - (vector.y * sin),
        (vector.x * sin) + (vector.y * cos),
    )
}

#[cfg(target_arch = "wasm32")]
fn log_viewer_error(message: &str) {
    web_sys::console::error_1(&JsValue::from_str(message));
}

#[cfg(target_arch = "wasm32")]
fn js_error(error: JsValue) -> String {
    format!("{error:?}")
}

#[cfg(target_arch = "wasm32")]
fn with_web_viewer_state<T>(
    f: impl FnOnce(&WebViewerState) -> Result<T, String>,
) -> Result<T, JsValue> {
    WEB_VIEWER_APP.with(|slot| {
        let borrow = slot.borrow();
        let app = borrow
            .as_ref()
            .ok_or_else(|| JsValue::from_str("w web viewer is not initialized"))?;
        let state = app._state.borrow();
        f(&state).map_err(|error| JsValue::from_str(&error))
    })
}

#[cfg(target_arch = "wasm32")]
fn with_web_viewer_state_mut<T>(
    f: impl FnOnce(&mut WebViewerState) -> Result<T, String>,
) -> Result<T, JsValue> {
    WEB_VIEWER_APP.with(|slot| {
        let borrow = slot.borrow();
        let app = borrow
            .as_ref()
            .ok_or_else(|| JsValue::from_str("w web viewer is not initialized"))?;
        let mut state = app._state.borrow_mut();
        f(&mut state).map_err(|error| JsValue::from_str(&error))
    })
}

#[cfg(target_arch = "wasm32")]
fn semantic_ids_from_array(ids: &Array) -> Result<Vec<SemanticElementId>, String> {
    ids.iter()
        .enumerate()
        .map(|(index, value)| {
            let Some(id) = value.as_string() else {
                return Err(format!(
                    "w viewer expected a string semantic id at index {index}"
                ));
            };
            Ok(SemanticElementId::new(id))
        })
        .collect()
}

#[cfg(target_arch = "wasm32")]
fn semantic_ids_to_array<'a>(ids: impl IntoIterator<Item = &'a SemanticElementId>) -> Array {
    let array = Array::new();
    for id in ids {
        array.push(&JsValue::from_str(id.as_str()));
    }
    array
}

#[cfg(target_arch = "wasm32")]
fn semantic_ids_to_strings(ids: Vec<SemanticElementId>) -> Vec<String> {
    ids.into_iter().map(|id| id.as_str().to_string()).collect()
}

#[cfg(target_arch = "wasm32")]
fn selected_instance_ids_from_catalog(
    catalog: &GeometryCatalog,
    selected_element_ids: &[SemanticElementId],
) -> Vec<u64> {
    catalog
        .instances
        .iter()
        .filter(|instance| selected_element_ids.contains(&instance.element_id))
        .map(|instance| instance.id.0)
        .collect()
}

#[cfg(target_arch = "wasm32")]
fn parse_web_view_mode(mode: &str) -> Result<GeometryStartViewRequest, String> {
    match mode.trim().to_ascii_lowercase().as_str() {
        "default" => Ok(GeometryStartViewRequest::Default),
        "all" => Ok(GeometryStartViewRequest::All),
        other => Err(format!(
            "unknown web view mode `{other}`; expected `default` or `all`"
        )),
    }
}

#[cfg(target_arch = "wasm32")]
fn web_view_mode_name(request: &GeometryStartViewRequest) -> &'static str {
    match request {
        GeometryStartViewRequest::Default => "default",
        GeometryStartViewRequest::Minimal(_) => "minimal",
        GeometryStartViewRequest::All => "all",
        GeometryStartViewRequest::Elements(_) => "elements",
    }
}

#[cfg(target_arch = "wasm32")]
fn web_viewer_status_line(runtime_scene: &RuntimeSceneState, profile: RenderProfileId) -> String {
    let render_scene: PreparedRenderScene = runtime_scene.compose_render_scene();
    let catalog = runtime_scene.catalog();
    let missing = runtime_scene.missing_stream_plan_for_visible_elements();
    let visible_elements = runtime_scene.visible_element_ids().len();
    let total_elements = catalog.elements.len();
    let selected_elements = runtime_scene.selected_element_ids().len();
    let view_mode = match web_view_mode_name(runtime_scene.start_view_request()) {
        "default" => "Default",
        "minimal" => "Minimal",
        "all" => "All",
        "elements" => "Elements",
        _ => "View",
    };
    let stream_status = if missing.instance_ids.is_empty() && missing.definition_ids.is_empty() {
        "stream ok".to_string()
    } else {
        format!(
            "missing {} inst / {} meshes",
            missing.instance_ids.len(),
            missing.definition_ids.len()
        )
    };
    format!(
        "{} · {view_mode} · {} meshes · {} tris · {} draws · {visible_elements}/{total_elements} visible · {selected_elements} selected · {stream_status}",
        profile.label(),
        render_scene.definitions.len(),
        render_scene.triangle_count(),
        render_scene.draw_count(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_summary_mentions_demo_mesh() {
        let summary = demo_summary_string();

        assert!(summary.contains("Demo"));
    }

    #[test]
    fn geometry_catalog_response_uses_metadata_only_definition_entries() {
        let package = GeometryBackend::default()
            .build_demo_package_for(DEFAULT_DEMO_RESOURCE)
            .expect("demo package should build");
        let response = WebGeometryCatalogResponse::from_geometry_catalog(
            DEFAULT_DEMO_RESOURCE,
            &package.catalog(),
        );
        let json = serde_json::to_string(&response).expect("catalog response should serialize");

        assert_eq!(response.resource, DEFAULT_DEMO_RESOURCE);
        assert!(!response.catalog.definitions.is_empty());
        assert!(!response.catalog.instances.is_empty());
        assert!(json.contains("vertex_count"));
        assert!(!json.contains("vertices"));
        assert!(!json.contains("indices"));
    }

    #[test]
    fn geometry_batch_requests_convert_to_shared_ids() {
        let instance_request = WebGeometryInstanceBatchRequest {
            resource: DEFAULT_DEMO_RESOURCE.to_owned(),
            instance_ids: vec![1, 3],
        }
        .to_geometry_instance_batch_request();
        let definition_request = WebGeometryDefinitionBatchRequest {
            resource: DEFAULT_DEMO_RESOURCE.to_owned(),
            definition_ids: vec![2, 4],
        }
        .to_geometry_definition_batch_request();

        assert_eq!(
            instance_request.instance_ids,
            vec![GeometryInstanceId(1), GeometryInstanceId(3)]
        );
        assert_eq!(
            definition_request.definition_ids,
            vec![GeometryDefinitionId(2), GeometryDefinitionId(4)]
        );
    }

    fn colliding_web_package(default_render_class: &str) -> WebPreparedGeometryPackage {
        WebPreparedGeometryPackage {
            definitions: vec![WebPreparedGeometryDefinition {
                id: 7,
                mesh: WebPreparedMesh {
                    local_origin: [0.0; 3],
                    bounds_min: [0.0; 3],
                    bounds_max: [1.0; 3],
                    vertices: vec![
                        WebPreparedVertex {
                            position: [0.0, 0.0, 0.0],
                            normal: [0.0, 0.0, 1.0],
                        },
                        WebPreparedVertex {
                            position: [1.0, 0.0, 0.0],
                            normal: [0.0, 0.0, 1.0],
                        },
                        WebPreparedVertex {
                            position: [0.0, 1.0, 0.0],
                            normal: [0.0, 0.0, 1.0],
                        },
                    ],
                    indices: vec![0, 1, 2],
                },
            }],
            elements: vec![WebPreparedGeometryElement {
                id: "shared-local-element".to_string(),
                label: "Shared Local Element".to_string(),
                declared_entity: "IfcWall".to_string(),
                default_render_class: default_render_class.to_string(),
                bounds_min: [0.0; 3],
                bounds_max: [1.0; 3],
            }],
            instances: vec![WebPreparedGeometryInstance {
                id: 1,
                element_id: "shared-local-element".to_string(),
                definition_id: 7,
                transform: glam::DMat4::IDENTITY.to_cols_array(),
                bounds_min: [0.0; 3],
                bounds_max: [1.0; 3],
                external_id: "external-shared".to_string(),
                label: "Shared Local Instance".to_string(),
                display_color: None,
            }],
        }
    }

    #[test]
    fn source_scoped_semantic_element_id_round_trips_as_string() {
        let local_id = SemanticElementId::new("3MyeqU7Xn6heW9nCvA0vpL");
        let scoped = SourceScopedSemanticElementId::from_semantic_element_id(
            DEFAULT_DEMO_RESOURCE,
            &local_id,
        );

        assert_eq!(scoped.resource(), DEFAULT_DEMO_RESOURCE);
        assert_eq!(scoped.local_id(), local_id.as_str());
        assert_eq!(
            scoped.to_string(),
            "demo/revolved-solid::3MyeqU7Xn6heW9nCvA0vpL"
        );
        assert_eq!(
            scoped
                .to_string()
                .parse::<SourceScopedSemanticElementId>()
                .expect("scoped semantic id should parse"),
            scoped
        );
        assert_eq!(scoped.as_semantic_element_id(), local_id);
    }

    #[test]
    fn source_scoped_geometry_instance_id_round_trips_as_string() {
        let scoped = SourceScopedGeometryInstanceId::from_geometry_instance_id(
            DEFAULT_DEMO_RESOURCE,
            GeometryInstanceId(42),
        );

        assert_eq!(scoped.resource(), DEFAULT_DEMO_RESOURCE);
        assert_eq!(scoped.local_id(), 42);
        assert_eq!(scoped.to_string(), "demo/revolved-solid::42");
        assert_eq!(
            scoped
                .to_string()
                .parse::<SourceScopedGeometryInstanceId>()
                .expect("scoped instance id should parse"),
            scoped
        );
        assert_eq!(scoped.as_geometry_instance_id(), GeometryInstanceId(42));
    }

    #[test]
    fn source_scoped_geometry_definition_id_round_trips_as_string() {
        let scoped = SourceScopedGeometryDefinitionId::from_geometry_definition_id(
            DEFAULT_DEMO_RESOURCE,
            GeometryDefinitionId(77),
        );

        assert_eq!(scoped.resource(), DEFAULT_DEMO_RESOURCE);
        assert_eq!(scoped.local_id(), 77);
        assert_eq!(scoped.to_string(), "demo/revolved-solid::77");
        assert_eq!(
            scoped
                .to_string()
                .parse::<SourceScopedGeometryDefinitionId>()
                .expect("scoped definition id should parse"),
            scoped
        );
        assert_eq!(scoped.as_geometry_definition_id(), GeometryDefinitionId(77));
    }

    #[test]
    fn source_scoped_ids_reject_ambiguous_strings() {
        assert_eq!(
            "wall-a"
                .parse::<SourceScopedSemanticElementId>()
                .expect_err("missing separator should fail"),
            SourceScopedIdParseError::MissingSeparator
        );
        assert_eq!(
            "::wall-a"
                .parse::<SourceScopedSemanticElementId>()
                .expect_err("empty resource should fail"),
            SourceScopedIdParseError::EmptyResource
        );
        assert_eq!(
            "ifc/building-architecture::"
                .parse::<SourceScopedSemanticElementId>()
                .expect_err("empty local id should fail"),
            SourceScopedIdParseError::EmptyLocalId
        );
        assert!(matches!(
            "ifc/building-architecture::wall-a"
                .parse::<SourceScopedGeometryInstanceId>()
                .expect_err("non-numeric instance id should fail"),
            SourceScopedIdParseError::InvalidInstanceId(_)
        ));
        assert!(matches!(
            "ifc/building-architecture::mesh-a"
                .parse::<SourceScopedGeometryDefinitionId>()
                .expect_err("non-numeric definition id should fail"),
            SourceScopedIdParseError::InvalidDefinitionId(_)
        ));
    }

    #[test]
    fn project_prepared_package_scopes_colliding_local_ids_by_resource() {
        let left = colliding_web_package("physical");
        let right = colliding_web_package("physical");

        let project = WebProjectPreparedGeometryPackage::from_web_prepared_packages([
            ("ifc/left", &left),
            ("ifc/right", &right),
        ])
        .expect("colliding local ids should compose when resources differ");

        assert_eq!(
            project
                .definitions
                .iter()
                .map(|definition| definition.id.as_str())
                .collect::<Vec<_>>(),
            vec!["ifc/left::7", "ifc/right::7"]
        );
        assert_eq!(
            project
                .elements
                .iter()
                .map(|element| element.id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "ifc/left::shared-local-element",
                "ifc/right::shared-local-element"
            ]
        );
        assert_eq!(
            project
                .instances
                .iter()
                .map(|instance| {
                    (
                        instance.id.as_str(),
                        instance.element_id.as_str(),
                        instance.definition_id.as_str(),
                    )
                })
                .collect::<Vec<_>>(),
            vec![
                (
                    "ifc/left::1",
                    "ifc/left::shared-local-element",
                    "ifc/left::7"
                ),
                (
                    "ifc/right::1",
                    "ifc/right::shared-local-element",
                    "ifc/right::7"
                ),
            ]
        );

        assert_eq!(left.definitions[0].id, 7);
        assert_eq!(left.elements[0].id, "shared-local-element");
        assert_eq!(left.instances[0].id, 1);
    }

    #[test]
    fn project_catalog_resolves_default_and_all_visible_state_with_scoped_ids() {
        let physical = colliding_web_package("physical");
        let space = colliding_web_package("space");
        let project = WebProjectPreparedGeometryPackage::from_web_prepared_packages([
            ("ifc/physical", &physical),
            ("ifc/space", &space),
        ])
        .expect("project package should compose");
        let catalog = project.catalog();

        let default_view = catalog
            .resolve_start_view(&GeometryStartViewRequest::Default)
            .expect("default start view should resolve");
        assert_eq!(
            default_view.visible_element_ids,
            vec!["ifc/physical::shared-local-element"]
        );
        assert_eq!(
            catalog
                .default_start_view_stream_plan()
                .expect("default stream plan should resolve"),
            WebProjectGeometryStreamPlan {
                instance_ids: vec!["ifc/physical::1".to_string()],
                definition_ids: vec!["ifc/physical::7".to_string()],
            }
        );

        let all_view = catalog
            .resolve_start_view(&GeometryStartViewRequest::All)
            .expect("all start view should resolve");
        assert_eq!(
            all_view.visible_element_ids,
            vec![
                "ifc/physical::shared-local-element",
                "ifc/space::shared-local-element"
            ]
        );
        let all_plan = catalog.stream_plan_for_element_ids(&all_view.visible_element_ids);
        assert_eq!(
            all_plan,
            WebProjectGeometryStreamPlan {
                instance_ids: vec!["ifc/physical::1".to_string(), "ifc/space::1".to_string()],
                definition_ids: vec!["ifc/physical::7".to_string(), "ifc/space::7".to_string()],
            }
        );
        assert_eq!(
            all_plan
                .to_member_batch_requests()
                .expect("scoped stream plan should group back into member-local requests"),
            WebProjectGeometryBatchRequests {
                instance_requests: vec![
                    WebGeometryInstanceBatchRequest {
                        resource: "ifc/physical".to_string(),
                        instance_ids: vec![1],
                    },
                    WebGeometryInstanceBatchRequest {
                        resource: "ifc/space".to_string(),
                        instance_ids: vec![1],
                    },
                ],
                definition_requests: vec![
                    WebGeometryDefinitionBatchRequest {
                        resource: "ifc/physical".to_string(),
                        definition_ids: vec![7],
                    },
                    WebGeometryDefinitionBatchRequest {
                        resource: "ifc/space".to_string(),
                        definition_ids: vec![7],
                    },
                ],
            }
        );
    }

    #[test]
    fn catalog_default_start_view_stream_plan_uses_default_visible_elements() {
        let catalog = WebGeometryCatalog {
            definitions: Vec::new(),
            elements: vec![
                WebPreparedGeometryElement {
                    id: "physical".to_string(),
                    label: "Physical".to_string(),
                    declared_entity: "IfcWall".to_string(),
                    default_render_class: "physical".to_string(),
                    bounds_min: [0.0; 3],
                    bounds_max: [1.0; 3],
                },
                WebPreparedGeometryElement {
                    id: "space".to_string(),
                    label: "Space".to_string(),
                    declared_entity: "IfcSpace".to_string(),
                    default_render_class: "space".to_string(),
                    bounds_min: [0.0; 3],
                    bounds_max: [1.0; 3],
                },
            ],
            instances: vec![
                WebPreparedGeometryInstance {
                    id: 10,
                    element_id: "physical".to_string(),
                    definition_id: 20,
                    transform: glam::DMat4::IDENTITY.to_cols_array(),
                    bounds_min: [0.0; 3],
                    bounds_max: [1.0; 3],
                    external_id: "physical-ext".to_string(),
                    label: "Physical".to_string(),
                    display_color: None,
                },
                WebPreparedGeometryInstance {
                    id: 11,
                    element_id: "space".to_string(),
                    definition_id: 21,
                    transform: glam::DMat4::IDENTITY.to_cols_array(),
                    bounds_min: [0.0; 3],
                    bounds_max: [1.0; 3],
                    external_id: "space-ext".to_string(),
                    label: "Space".to_string(),
                    display_color: None,
                },
            ],
        };

        let plan = catalog
            .default_start_view_stream_plan()
            .expect("catalog plan should resolve");

        assert_eq!(plan.instance_ids, vec![GeometryInstanceId(10)]);
        assert_eq!(plan.definition_ids, vec![GeometryDefinitionId(20)]);
    }

    #[test]
    fn streamed_batches_feed_catalog_runtime_directly() {
        let package = GeometryBackend::default()
            .build_demo_package_for(DEFAULT_DEMO_RESOURCE)
            .expect("demo package should build");
        let catalog = package.catalog();
        let web_catalog = WebGeometryCatalog::from_geometry_catalog(&catalog);
        let mut runtime_scene = cc_w_runtime::RuntimeSceneState::from_catalog_with_start_view(
            web_catalog
                .clone()
                .try_into_geometry_catalog()
                .expect("catalog should convert"),
            GeometryStartViewRequest::Default,
        )
        .expect("runtime scene should build from catalog");
        let plan = runtime_scene.missing_stream_plan_for_visible_elements();
        let instance_batch = catalog.instance_batch(&GeometryInstanceBatchRequest::new(
            plan.instance_ids.clone(),
        ));
        let definition_batch =
            package.definition_batch(&GeometryDefinitionBatchRequest::new(plan.definition_ids));

        runtime_scene.mark_instance_batch_resident(
            &WebGeometryInstanceBatch::from_geometry_instance_batch(&instance_batch)
                .into_geometry_instance_batch(),
        );
        runtime_scene.mark_definition_batch_resident(
            &WebGeometryDefinitionBatch::from_geometry_definition_batch(&definition_batch)
                .into_geometry_definition_batch(),
        );

        assert!(
            runtime_scene
                .missing_stream_plan_for_visible_elements()
                .instance_ids
                .is_empty()
        );
        assert!(
            runtime_scene
                .missing_stream_plan_for_visible_elements()
                .definition_ids
                .is_empty()
        );
        assert_eq!(
            runtime_scene.compose_render_scene().draw_count(),
            instance_batch.instances.len()
        );
    }

    #[test]
    fn streamed_runtime_detects_missing_definitions() {
        let package = GeometryBackend::default()
            .build_demo_package_for(DEFAULT_DEMO_RESOURCE)
            .expect("demo package should build");
        let catalog = package.catalog();
        let web_catalog = WebGeometryCatalog::from_geometry_catalog(&catalog);
        let mut runtime_scene = cc_w_runtime::RuntimeSceneState::from_catalog_with_start_view(
            web_catalog
                .try_into_geometry_catalog()
                .expect("catalog should convert"),
            GeometryStartViewRequest::Default,
        )
        .expect("runtime scene should build from catalog");
        let instance_batch = catalog.instance_batch(&GeometryInstanceBatchRequest::new(vec![
            catalog.instances[0].id,
        ]));

        runtime_scene.mark_instance_batch_resident(
            &WebGeometryInstanceBatch::from_geometry_instance_batch(&instance_batch)
                .into_geometry_instance_batch(),
        );

        let remaining = runtime_scene.missing_stream_plan_for_visible_elements();
        assert!(remaining.instance_ids.is_empty());
        assert!(!remaining.definition_ids.is_empty());
        assert_eq!(runtime_scene.compose_render_scene().draw_count(), 0);
    }
}
