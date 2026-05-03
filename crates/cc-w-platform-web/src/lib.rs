use cc_w_backend::{GeometryBackend, GeometryBackendError, ResourceError};
use cc_w_render::NullRenderBackend;
#[cfg(target_arch = "wasm32")]
use cc_w_runtime::RuntimeSceneState;
use cc_w_runtime::{DemoAsset, Engine, GeometryPackageSource, GeometryPackageSourceError};
use cc_w_types::GeometryStartViewRequest;
use cc_w_types::{
    Bounds3, DefaultRenderClass, DisplayColor, ExternalId, FaceVisibility, GeometryCatalog,
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
const WEB_GEOMETRY_INSTANCE_BATCH_CHUNK_SIZE: usize = 5_000;
#[cfg(target_arch = "wasm32")]
const WEB_GEOMETRY_DEFINITION_BATCH_CHUNK_SIZE: usize = 16;
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
    #[serde(default = "default_web_face_visibility")]
    pub face_visibility: String,
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
    #[serde(default = "default_web_face_visibility")]
    pub face_visibility: String,
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
            face_visibility: instance.face_visibility.clone(),
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
            face_visibility: face_visibility_name(instance.face_visibility).to_string(),
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
            face_visibility: face_visibility_name(instance.face_visibility).to_string(),
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
            face_visibility: parse_face_visibility(&self.face_visibility)
                .unwrap_or_else(|error| panic!("{error}")),
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
            face_visibility: parse_face_visibility(&self.face_visibility)
                .unwrap_or_else(|error| panic!("{error}")),
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
        DefaultRenderClass::Course => "course",
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

fn face_visibility_name(visibility: FaceVisibility) -> &'static str {
    match visibility {
        FaceVisibility::OneSided => "one-sided",
        FaceVisibility::DoubleSided => "double-sided",
    }
}

fn default_web_face_visibility() -> String {
    face_visibility_name(FaceVisibility::OneSided).to_string()
}

fn parse_face_visibility(name: &str) -> Result<FaceVisibility, String> {
    match name {
        "one-sided" => Ok(FaceVisibility::OneSided),
        "double-sided" => Ok(FaceVisibility::DoubleSided),
        other => Err(format!("unknown face visibility `{other}`")),
    }
}

fn parse_default_render_class(name: &str) -> Result<DefaultRenderClass, String> {
    match name {
        "physical" => Ok(DefaultRenderClass::Physical),
        "course" => Ok(DefaultRenderClass::Course),
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
        | DefaultRenderClass::Course
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
    Camera, ClipPlaneSide, DepthTarget, MeshRenderer, PICK_DEPTH_BITS_FORMAT, PICK_INDEX_FORMAT,
    RenderDefaults, RenderProfileDescriptor, RenderProfileId, SectionOverlay, ViewportSize,
    fit_camera_to_bounds_with_scene_context, fit_camera_to_render_scene, interpolate_camera,
};
#[cfg(target_arch = "wasm32")]
use cc_w_types::{
    PickHit, PickRegion, PreparedRenderRole, PreparedRenderScene, SceneAnnotationDepthMode,
    SceneAnnotationLayer, SceneAnnotationLayerId, SceneAnnotationLifecycle,
    SceneAnnotationPrimitive, SceneMarker, SceneMarkerKind, ScenePolyline,
    SceneTextHorizontalAlign, SceneTextLabel, SceneTextVerticalAlign, SectionClipMode,
    SectionDisplayMode, SectionPose, SectionState, WORLD_FORWARD, WORLD_RIGHT, WORLD_UP,
};
#[cfg(target_arch = "wasm32")]
use glam::{DMat4, DQuat, DVec2, DVec3, DVec4, Vec2};
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
    HtmlCanvasElement, HtmlElement, HtmlSelectElement, KeyboardEvent, MouseEvent, RequestInit,
    Response, WheelEvent, Window,
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
    scene_bounds: WebBoundsSnapshot,
    total_elements: usize,
    total_instances: usize,
    total_definitions: usize,
    default_element_ids: Vec<String>,
    base_visible_element_ids: Vec<String>,
    visible_element_ids: Vec<String>,
    selected_element_ids: Vec<String>,
    inspected_element_ids: Vec<String>,
    selected_instance_ids: Vec<u64>,
    picked_instance_ids: Vec<u64>,
    hidden_element_ids: Vec<String>,
    shown_element_ids: Vec<String>,
    suppressed_element_ids: Vec<String>,
    section: WebSectionStateSnapshot,
    annotations: WebAnnotationStateSnapshot,
    resident_instances: usize,
    resident_definitions: usize,
    missing_instance_ids: Vec<u64>,
    missing_definition_ids: Vec<u64>,
    triangles: usize,
    draws: usize,
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WebSectionSetRequest {
    resource: Option<String>,
    alignment_id: Option<String>,
    station: Option<f64>,
    pose: WebSectionPoseRequest,
    width: Option<f64>,
    height: Option<f64>,
    thickness: Option<f64>,
    mode: Option<String>,
    clip: Option<String>,
    provenance: Option<Vec<String>>,
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WebAnnotationLayerSetRequest {
    id: String,
    source: Option<String>,
    visible: Option<bool>,
    lifecycle: Option<String>,
    primitives: Vec<WebAnnotationPrimitiveRequest>,
    provenance: Option<Vec<String>>,
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WebAnnotationPrimitiveRequest {
    #[serde(rename = "type", alias = "kind")]
    primitive_type: String,
    id: String,
    points: Option<Vec<[f64; 3]>>,
    position: Option<[f64; 3]>,
    anchor: Option<[f64; 3]>,
    text: Option<String>,
    direction: Option<[f64; 3]>,
    normal: Option<[f64; 3]>,
    color: Option<serde_json::Value>,
    alpha: Option<f32>,
    width_px: Option<f32>,
    size_px: Option<f32>,
    #[serde(alias = "shape")]
    marker_kind: Option<String>,
    depth_mode: Option<String>,
    screen_offset_px: Option<[f64; 2]>,
    horizontal_align: Option<String>,
    vertical_align: Option<String>,
    style: Option<WebAnnotationTextStyleRequest>,
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WebAnnotationTextStyleRequest {
    color: Option<serde_json::Value>,
    background_color: Option<serde_json::Value>,
    outline_color: Option<serde_json::Value>,
    size_px: Option<f32>,
    #[serde(alias = "boldPx", alias = "weightPx")]
    embolden_px: Option<f32>,
    padding_px: Option<f32>,
}

#[cfg(target_arch = "wasm32")]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WebBoundsSnapshot {
    min: [f64; 3],
    max: [f64; 3],
    center: [f64; 3],
    size: [f64; 3],
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WebSectionPoseRequest {
    origin: [f64; 3],
    tangent: [f64; 3],
    normal: [f64; 3],
    up: [f64; 3],
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebSectionStateSnapshot {
    active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    resource: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    alignment_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    station: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pose: Option<WebSectionPoseSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    width: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    height: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thickness: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mode: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    clip: Option<&'static str>,
    provenance: Vec<String>,
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebSectionPoseSnapshot {
    origin: [f64; 3],
    tangent: [f64; 3],
    normal: [f64; 3],
    up: [f64; 3],
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebAnnotationStateSnapshot {
    count: usize,
    layers: Vec<WebAnnotationLayerSnapshot>,
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebAnnotationLayerSnapshot {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<String>,
    visible: bool,
    lifecycle: &'static str,
    primitives: Vec<WebAnnotationPrimitiveSnapshot>,
    provenance: Vec<String>,
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum WebAnnotationPrimitiveSnapshot {
    Polyline {
        id: String,
        points: Vec<[f64; 3]>,
        color: [f32; 3],
        alpha: f32,
        width_px: f32,
        depth_mode: &'static str,
    },
    Marker {
        id: String,
        position: [f64; 3],
        #[serde(skip_serializing_if = "Option::is_none")]
        direction: Option<[f64; 3]>,
        #[serde(skip_serializing_if = "Option::is_none")]
        normal: Option<[f64; 3]>,
        color: [f32; 3],
        alpha: f32,
        size_px: f32,
        marker_kind: &'static str,
        depth_mode: &'static str,
    },
    Text {
        id: String,
        text: String,
        anchor: [f64; 3],
        screen_offset_px: [f64; 2],
        horizontal_align: &'static str,
        vertical_align: &'static str,
        depth_mode: &'static str,
        style: WebAnnotationTextStyleSnapshot,
    },
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebAnnotationTextStyleSnapshot {
    color: [f32; 4],
    #[serde(skip_serializing_if = "Option::is_none")]
    background_color: Option<[f32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    outline_color: Option<[f32; 4]>,
    size_px: f32,
    embolden_px: f32,
    padding_px: f32,
}

#[cfg(target_arch = "wasm32")]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WebRenderProfileDescriptor {
    id: String,
    name: String,
    label: String,
    experimental: bool,
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
    for chunk in instance_ids.chunks(WEB_GEOMETRY_INSTANCE_BATCH_CHUNK_SIZE) {
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
    for chunk in definition_ids.chunks(WEB_GEOMETRY_DEFINITION_BATCH_CHUNK_SIZE) {
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
    let (window, load_generation) = {
        let mut state = state.borrow_mut();
        let load_generation = state.begin_resource_load(&resource);
        (state.window.clone(), load_generation)
    };
    let runtime_scene = match fetch_runtime_scene(&window, &resource).await {
        Ok(runtime_scene) => runtime_scene,
        Err(error) => {
            let mut state = state.borrow_mut();
            if state.resource_load_generation == load_generation {
                state.finish_resource_load_failed(&resource, &error);
            }
            return Err(error);
        }
    };
    let events = {
        let mut state = state.borrow_mut();
        if state.resource_load_generation != load_generation {
            log_viewer_info(&format!(
                "w web viewer ignored stale resource load `{resource}`"
            ));
            return Ok(());
        }
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
pub fn viewer_section_state_json() -> Result<String, JsValue> {
    with_web_viewer_state(|state| state.section_state_json())
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_section_set_json(spec_json: String) -> Result<String, JsValue> {
    let request = parse_web_section_set_request(&spec_json)
        .map_err(|error| JsValue::from_str(&format!("failed to parse section request: {error}")))?;
    let (json, events) = with_web_viewer_state_mut(|state| {
        let section = web_section_request_to_state(request, &state.current_resource)?;
        state.set_section(section)?;
        Ok((
            state.section_state_json()?,
            vec![state.viewer_state_change_event("section")?],
        ))
    })?;
    dispatch_web_events(events).map_err(|error| JsValue::from_str(&error))?;
    Ok(json)
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_section_clear_json() -> Result<String, JsValue> {
    let (json, events) = with_web_viewer_state_mut(|state| {
        let changed = state.clear_section()?;
        let events = changed
            .then(|| state.viewer_state_change_event("section"))
            .transpose()?
            .into_iter()
            .collect();
        Ok((state.section_state_json()?, events))
    })?;
    dispatch_web_events(events).map_err(|error| JsValue::from_str(&error))?;
    Ok(json)
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_annotations_state_json() -> Result<String, JsValue> {
    with_web_viewer_state(|state| state.annotation_state_json())
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_annotations_set_json(spec_json: String) -> Result<String, JsValue> {
    let request = parse_web_annotation_layer_set_request(&spec_json).map_err(|error| {
        JsValue::from_str(&format!(
            "failed to parse annotation layer request: {error}"
        ))
    })?;
    let (json, events) = with_web_viewer_state_mut(|state| {
        let layer = web_annotation_layer_request_to_state(request)?;
        state.set_annotation_layer(layer)?;
        Ok((
            state.annotation_state_json()?,
            vec![state.viewer_state_change_event("annotations")?],
        ))
    })?;
    dispatch_web_events(events).map_err(|error| JsValue::from_str(&error))?;
    Ok(json)
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_annotations_merge_json(spec_json: String) -> Result<String, JsValue> {
    let request = parse_web_annotation_layer_set_request(&spec_json).map_err(|error| {
        JsValue::from_str(&format!(
            "failed to parse annotation layer merge request: {error}"
        ))
    })?;
    let (json, events) = with_web_viewer_state_mut(|state| {
        let layer = web_annotation_layer_request_to_state(request)?;
        state.merge_annotation_layer(layer)?;
        Ok((
            state.annotation_state_json()?,
            vec![state.viewer_state_change_event("annotations")?],
        ))
    })?;
    dispatch_web_events(events).map_err(|error| JsValue::from_str(&error))?;
    Ok(json)
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_annotations_clear_json(layer_id: Option<String>) -> Result<String, JsValue> {
    let (json, events) = with_web_viewer_state_mut(|state| {
        let changed = state.clear_annotation_layers(layer_id.as_deref())?;
        let events = changed
            .then(|| state.viewer_state_change_event("annotations"))
            .transpose()?
            .into_iter()
            .collect();
        Ok((state.annotation_state_json()?, events))
    })?;
    dispatch_web_events(events).map_err(|error| JsValue::from_str(&error))?;
    Ok(json)
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
pub fn viewer_inspected_element_ids() -> Result<Array, JsValue> {
    with_web_viewer_state(|state| {
        let ids = state.runtime_scene.inspected_element_ids();
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
        let inspection_changed = state.runtime_scene.clear_inspection() as u32;
        let events = state.commit_show_change(&ids, changed, inspection_changed)?;
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
pub fn viewer_inspect_elements(ids: Array) -> Result<u32, JsValue> {
    let (changed, events) = with_web_viewer_state_mut(|state| {
        let ids = semantic_ids_from_array(&ids)?;
        let changed = state.runtime_scene.set_inspection_focus(ids.iter()) as u32;
        let events = state.commit_inspection_change(changed, "replace")?;
        Ok((changed, events))
    })?;
    dispatch_web_events(events).map_err(|error| JsValue::from_str(&error))?;
    Ok(changed)
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_add_inspection_elements(ids: Array) -> Result<u32, JsValue> {
    let (changed, events) = with_web_viewer_state_mut(|state| {
        let ids = semantic_ids_from_array(&ids)?;
        let changed = state.runtime_scene.add_inspection_focus(ids.iter()) as u32;
        let events = state.commit_inspection_change(changed, "add")?;
        Ok((changed, events))
    })?;
    dispatch_web_events(events).map_err(|error| JsValue::from_str(&error))?;
    Ok(changed)
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_remove_inspection_elements(ids: Array) -> Result<u32, JsValue> {
    let (changed, events) = with_web_viewer_state_mut(|state| {
        let ids = semantic_ids_from_array(&ids)?;
        let changed = state.runtime_scene.remove_inspection_focus(ids.iter()) as u32;
        let events = state.commit_inspection_change(changed, "remove")?;
        Ok((changed, events))
    })?;
    dispatch_web_events(events).map_err(|error| JsValue::from_str(&error))?;
    Ok(changed)
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_clear_inspection() -> Result<u32, JsValue> {
    let (changed, events) = with_web_viewer_state_mut(|state| {
        let changed = state.runtime_scene.clear_inspection() as u32;
        state.cancel_auto_inspection_probes();
        let mut events = vec![state.upload_runtime_scene(false)?];
        if changed > 0 {
            state.camera_transition = None;
            state.inspection_visual_transition = None;
            state
                .renderer
                .set_inspection_context_alpha_multiplier(&state.queue, 1.0);
            events.push(state.viewer_state_change_event("inspection")?);
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
#[derive(Clone, Copy)]
enum WebPickRequest {
    Point { x: f32, y: f32 },
    Rect { x0: f32, y0: f32, x1: f32, y1: f32 },
}

#[cfg(target_arch = "wasm32")]
struct WebOrbitPivotPickRequest {
    drag_generation: u64,
    request: WebPickRequest,
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
    let pick_index_bytes = strip_padded_rows_web(
        &mapped,
        prepared.unpadded_bytes_per_row as usize,
        prepared.padded_bytes_per_row as usize,
        prepared.region.height as usize,
    );
    drop(mapped);
    prepared.readback.unmap();

    JsFuture::from(prepared.depth_bits_map_promise)
        .await
        .map_err(|error| format!("GPU pick depth-bits readback failed: {:?}", error))?;

    let depth_bits_mapped = prepared.depth_bits_readback.slice(..).get_mapped_range();
    let depth_bits_bytes = strip_padded_rows_web(
        &depth_bits_mapped,
        prepared.depth_bits_unpadded_bytes_per_row as usize,
        prepared.depth_bits_padded_bytes_per_row as usize,
        prepared.region.height as usize,
    );
    drop(depth_bits_mapped);
    prepared.depth_bits_readback.unmap();
    let depth_bits = u32_values_from_bytes(&depth_bits_bytes, "pick depth-bits readback")?;

    let result = decode_pick_pixels_with_depth_bits(
        prepared.region,
        &pick_index_bytes,
        &prepared.pick_targets,
        &depth_bits,
        prepared.clip_from_world,
        ViewportSize::new(prepared.viewport_width, prepared.viewport_height),
    );
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
fn interactive_pick_scene(scene: &PreparedRenderScene) -> PreparedRenderScene {
    let instances = scene
        .instances
        .iter()
        .filter(|instance| instance.render_role != PreparedRenderRole::InspectionContext)
        .cloned()
        .collect::<Vec<_>>();
    let bounds = instances
        .iter()
        .fold(None::<Bounds3>, |bounds, instance| {
            Some(match bounds {
                Some(bounds) => Bounds3 {
                    min: bounds.min.min(instance.world_bounds.min),
                    max: bounds.max.max(instance.world_bounds.max),
                },
                None => instance.world_bounds,
            })
        })
        .unwrap_or(scene.bounds);

    PreparedRenderScene {
        bounds,
        definitions: scene.definitions.clone(),
        instances,
    }
}

#[cfg(target_arch = "wasm32")]
async fn update_orbit_pivot_from_gpu_pick(
    app_state: Rc<RefCell<WebViewerState>>,
    request: WebOrbitPivotPickRequest,
) -> Result<(), String> {
    let prepared = {
        let mut state = app_state.borrow_mut();
        state.prepare_pick_readback(request.request)?
    };

    JsFuture::from(prepared.map_promise)
        .await
        .map_err(|error| format!("GPU orbit pivot pick readback failed: {:?}", error))?;

    let mapped = prepared.readback.slice(..).get_mapped_range();
    let pick_index_bytes = strip_padded_rows_web(
        &mapped,
        prepared.unpadded_bytes_per_row as usize,
        prepared.padded_bytes_per_row as usize,
        prepared.region.height as usize,
    );
    drop(mapped);
    prepared.readback.unmap();

    JsFuture::from(prepared.depth_bits_map_promise)
        .await
        .map_err(|error| format!("GPU orbit pivot depth-bits readback failed: {:?}", error))?;
    let depth_bits_mapped = prepared.depth_bits_readback.slice(..).get_mapped_range();
    let depth_bits_bytes = strip_padded_rows_web(
        &depth_bits_mapped,
        prepared.depth_bits_unpadded_bytes_per_row as usize,
        prepared.depth_bits_padded_bytes_per_row as usize,
        prepared.region.height as usize,
    );
    drop(depth_bits_mapped);
    prepared.depth_bits_readback.unmap();
    let depth_bits = u32_values_from_bytes(&depth_bits_bytes, "orbit pivot depth-bits readback")?;

    let hit = decode_pick_pixels_with_depth_bits(
        prepared.region,
        &pick_index_bytes,
        &prepared.pick_targets,
        &depth_bits,
        prepared.clip_from_world,
        ViewportSize::new(prepared.viewport_width, prepared.viewport_height),
    )
    .first_hit()
    .cloned();
    let Some(hit) = hit else {
        return Ok(());
    };

    app_state
        .borrow_mut()
        .apply_orbit_pivot_pick(request.drag_generation, hit.world_anchor);
    Ok(())
}

#[cfg(target_arch = "wasm32")]
async fn run_show_visibility_probe_in_state(
    app_state: Rc<RefCell<WebViewerState>>,
    probe: ShowVisibilityProbe,
) -> Result<(), String> {
    let prepared = {
        let mut state = app_state.borrow_mut();
        state.prepare_show_visibility_readback(probe)?
    };

    JsFuture::from(prepared.target_only.map_promise.clone())
        .await
        .map_err(|error| {
            format!(
                "GPU target-only show visibility readback failed: {:?}",
                error
            )
        })?;
    JsFuture::from(prepared.full_scene.map_promise.clone())
        .await
        .map_err(|error| {
            format!(
                "GPU full-scene show visibility readback failed: {:?}",
                error
            )
        })?;

    let target_only_pick_indices = pick_index_bytes_from_color_readback(&prepared.target_only);
    let full_scene_pick_indices = pick_index_bytes_from_color_readback(&prepared.full_scene);

    let result = count_show_visibility_probe_ratio(
        &target_only_pick_indices,
        &prepared.target_only.pick_targets,
        &full_scene_pick_indices,
        &prepared.full_scene.pick_targets,
        &prepared.probe.element_ids,
    );
    let events = app_state
        .borrow_mut()
        .apply_show_visibility_probe_result(&prepared.probe, result)?;
    dispatch_web_events(events)
}

#[cfg(target_arch = "wasm32")]
fn pick_index_bytes_from_color_readback(readback: &WebPickColorReadback) -> Vec<u8> {
    let mapped = readback.readback.slice(..).get_mapped_range();
    let pick_index_bytes = strip_padded_rows_web(
        &mapped,
        readback.unpadded_bytes_per_row as usize,
        readback.padded_bytes_per_row as usize,
        readback.region.height as usize,
    );
    drop(mapped);
    readback.readback.unmap();
    pick_index_bytes
}

#[cfg(target_arch = "wasm32")]
fn count_show_visibility_probe_ratio(
    target_only_pick_indices: &[u8],
    target_only_pick_targets: &[PickHit],
    full_scene_pick_indices: &[u8],
    full_scene_pick_targets: &[PickHit],
    target_ids: &[SemanticElementId],
) -> ShowVisibilityProbeResult {
    let unoccluded_target_pixels = count_target_pick_pixels(
        target_only_pick_indices,
        target_only_pick_targets,
        target_ids,
    );
    let visible_target_pixels =
        count_target_pick_pixels(full_scene_pick_indices, full_scene_pick_targets, target_ids);

    ShowVisibilityProbeResult {
        visible_target_pixels,
        unoccluded_target_pixels,
    }
}

#[cfg(target_arch = "wasm32")]
fn count_target_pick_pixels(
    pick_index_bytes: &[u8],
    pick_targets: &[PickHit],
    target_ids: &[SemanticElementId],
) -> usize {
    let target_ids = target_ids.iter().collect::<HashSet<_>>();
    let mut target_pixels = 0usize;

    for pixel in pick_index_bytes.chunks_exact(4) {
        let pick_index = decode_pick_index_u32(pixel);
        if pick_index == 0 {
            continue;
        }
        let Some(target) = pick_targets.get((pick_index - 1) as usize) else {
            continue;
        };
        if target_ids.contains(&target.element_id) {
            target_pixels += 1;
        }
    }

    target_pixels
}

#[cfg(target_arch = "wasm32")]
fn show_visibility_probe_requires_inspection(result: ShowVisibilityProbeResult) -> bool {
    if result.unoccluded_target_pixels < 24 {
        return false;
    }
    let visible_ratio =
        result.visible_target_pixels as f64 / result.unoccluded_target_pixels as f64;
    visible_ratio < 0.5
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
fn u32_values_from_bytes(bytes: &[u8], label: &str) -> Result<Vec<u32>, String> {
    if bytes.len() % 4 != 0 {
        return Err(format!("{label} had a non-u32 byte length"));
    }
    Ok(bytes.chunks_exact(4).map(decode_pick_index_u32).collect())
}

#[cfg(target_arch = "wasm32")]
fn decode_pick_pixels_with_depth_bits(
    region: PickRegion,
    pick_index_bytes: &[u8],
    pick_targets: &[PickHit],
    depth_bits: &[u32],
    clip_from_world: DMat4,
    viewport: ViewportSize,
) -> cc_w_types::PickResult {
    let mut seen = HashSet::new();
    let mut hits = Vec::new();
    let world_from_clip = clip_from_world.inverse();

    for (pixel_index, pixel) in pick_index_bytes.chunks_exact(4).enumerate() {
        let pick_index = decode_pick_index_u32(pixel);
        if pick_index == 0 || !seen.insert(pick_index) {
            continue;
        }
        let Some(target) = pick_targets.get((pick_index - 1) as usize) else {
            continue;
        };
        let mut hit = target.clone();
        if let Some(depth) = depth_bits.get(pixel_index).copied().map(f32::from_bits) {
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
fn decode_pick_index_u32(pixel: &[u8]) -> u32 {
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
fn bounds_corners(bounds: Bounds3) -> [DVec3; 8] {
    [
        DVec3::new(bounds.min.x, bounds.min.y, bounds.min.z),
        DVec3::new(bounds.min.x, bounds.min.y, bounds.max.z),
        DVec3::new(bounds.min.x, bounds.max.y, bounds.min.z),
        DVec3::new(bounds.min.x, bounds.max.y, bounds.max.z),
        DVec3::new(bounds.max.x, bounds.min.y, bounds.min.z),
        DVec3::new(bounds.max.x, bounds.min.y, bounds.max.z),
        DVec3::new(bounds.max.x, bounds.max.y, bounds.min.z),
        DVec3::new(bounds.max.x, bounds.max.y, bounds.max.z),
    ]
}

#[cfg(target_arch = "wasm32")]
fn expand_pick_region_to_minimum(
    region: PickRegion,
    viewport: ViewportSize,
    min_width: u32,
    min_height: u32,
) -> PickRegion {
    let viewport = viewport.clamped();
    let target_width = region.width.max(min_width).min(viewport.width);
    let target_height = region.height.max(min_height).min(viewport.height);
    let center_x = region.x.saturating_add(region.width / 2);
    let center_y = region.y.saturating_add(region.height / 2);
    let x = center_x
        .saturating_sub(target_width / 2)
        .min(viewport.width.saturating_sub(target_width));
    let y = center_y
        .saturating_sub(target_height / 2)
        .min(viewport.height.saturating_sub(target_height));
    PickRegion::rect(x, y, target_width, target_height)
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
    _context_menu: Closure<dyn FnMut(Event)>,
    _key_down: Closure<dyn FnMut(KeyboardEvent)>,
    _key_up: Closure<dyn FnMut(KeyboardEvent)>,
    _window_blur: Closure<dyn FnMut(Event)>,
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
        renderer.set_profile(RenderProfileId::Bim);
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
            camera_transition: None,
            inspection_visual_transition: None,
            pending_show_visibility_probe: None,
            show_visibility_probe_generation: 0,
            inspection_generation: 0,
            resource_load_generation: 0,
            drag: DragState::default(),
            space_pan_modifier: false,
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
            event.prevent_default();
            let (drag_event, orbit_pivot_request) = mouse_down_state.borrow_mut().begin_drag(
                event.client_x() as f32,
                event.client_y() as f32,
                event.button(),
            );
            if let Err(error) = drag_event.dispatch() {
                log_viewer_error(&error);
            }
            if let Some(request) = orbit_pivot_request {
                let mouse_down_state = mouse_down_state.clone();
                spawn_local(async move {
                    if let Err(error) =
                        update_orbit_pivot_from_gpu_pick(mouse_down_state, request).await
                    {
                        log_viewer_error(&error);
                    }
                });
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

        let context_menu = Closure::wrap(Box::new(move |event: Event| {
            event.prevent_default();
        }) as Box<dyn FnMut(Event)>);
        canvas
            .add_event_listener_with_callback("contextmenu", context_menu.as_ref().unchecked_ref())
            .map_err(js_error)?;

        let key_down_state = state.clone();
        let key_down = Closure::wrap(Box::new(move |event: KeyboardEvent| {
            if !is_space_pan_keyboard_event(&event) || keyboard_event_targets_text_input(&event) {
                return;
            }
            event.prevent_default();
            key_down_state.borrow_mut().set_space_pan_modifier(true);
        }) as Box<dyn FnMut(KeyboardEvent)>);
        window
            .add_event_listener_with_callback("keydown", key_down.as_ref().unchecked_ref())
            .map_err(js_error)?;

        let key_up_state = state.clone();
        let key_up = Closure::wrap(Box::new(move |event: KeyboardEvent| {
            if !is_space_pan_keyboard_event(&event) {
                return;
            }
            if keyboard_event_targets_text_input(&event) {
                key_up_state.borrow_mut().set_space_pan_modifier(false);
                return;
            }
            event.prevent_default();
            key_up_state.borrow_mut().set_space_pan_modifier(false);
        }) as Box<dyn FnMut(KeyboardEvent)>);
        window
            .add_event_listener_with_callback("keyup", key_up.as_ref().unchecked_ref())
            .map_err(js_error)?;

        let window_blur_state = state.clone();
        let window_blur = Closure::wrap(Box::new(move |_event: Event| {
            window_blur_state.borrow_mut().set_space_pan_modifier(false);
        }) as Box<dyn FnMut(Event)>);
        window
            .add_event_listener_with_callback("blur", window_blur.as_ref().unchecked_ref())
            .map_err(js_error)?;

        let wheel_state = state.clone();
        let wheel = Closure::wrap(Box::new(move |event: WheelEvent| {
            event.prevent_default();
            match wheel_state.borrow_mut().wheel(
                event.delta_y() as f32,
                event.delta_mode(),
                event.client_x() as f32,
                event.client_y() as f32,
            ) {
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
        *animation_frame.borrow_mut() = Some(Closure::wrap(Box::new(move |_time_ms: f64| {
            let advance = {
                let mut state = animation_state.borrow_mut();
                let advance = match state.advance_animations(js_sys::Date::now()) {
                    Ok(advance) => advance,
                    Err(error) => {
                        log_viewer_error(&error);
                        AnimationAdvance {
                            anchor_event: None,
                            show_visibility_probe: None,
                        }
                    }
                };
                if let Err(error) = state.render() {
                    log_viewer_error(&error);
                    return;
                }
                advance
            };
            if let Some(event) = advance.anchor_event {
                if let Err(error) = event.dispatch() {
                    log_viewer_error(&error);
                }
            }
            if let Some(probe) = advance.show_visibility_probe {
                let probe_state = animation_state.clone();
                spawn_local(async move {
                    if let Err(error) = run_show_visibility_probe_in_state(probe_state, probe).await
                    {
                        log_viewer_error(&error);
                    }
                });
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
            _context_menu: context_menu,
            _key_down: key_down,
            _key_up: key_up,
            _window_blur: window_blur,
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
    camera_transition: Option<CameraTransition>,
    inspection_visual_transition: Option<InspectionVisualTransition>,
    pending_show_visibility_probe: Option<ShowVisibilityProbe>,
    show_visibility_probe_generation: u64,
    inspection_generation: u64,
    resource_load_generation: u64,
    drag: DragState,
    space_pan_modifier: bool,
    last_pick_hits: Vec<PickHit>,
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy, Debug)]
struct CameraTransition {
    from: Camera,
    to: Camera,
    started_at_ms: f64,
    duration_ms: f64,
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy, Debug)]
struct InspectionVisualTransition {
    from_multiplier: f32,
    to_multiplier: f32,
    started_at_ms: f64,
    duration_ms: f64,
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Debug)]
struct ShowVisibilityProbe {
    generation: u64,
    inspection_generation: u64,
    element_ids: Vec<SemanticElementId>,
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
    depth_bits_readback: wgpu::Buffer,
    depth_bits_map_promise: Promise,
    depth_bits_unpadded_bytes_per_row: u32,
    depth_bits_padded_bytes_per_row: u32,
}

#[cfg(target_arch = "wasm32")]
struct WebPickColorReadback {
    region: PickRegion,
    pick_targets: Vec<PickHit>,
    readback: wgpu::Buffer,
    map_promise: Promise,
    unpadded_bytes_per_row: u32,
    padded_bytes_per_row: u32,
}

#[cfg(target_arch = "wasm32")]
struct WebShowVisibilityReadback {
    probe: ShowVisibilityProbe,
    target_only: WebPickColorReadback,
    full_scene: WebPickColorReadback,
}

#[cfg(target_arch = "wasm32")]
struct ShowVisibilityProbeResult {
    visible_target_pixels: usize,
    unoccluded_target_pixels: usize,
}

#[cfg(target_arch = "wasm32")]
struct AnimationAdvance {
    anchor_event: Option<DeferredWebEvent>,
    show_visibility_probe: Option<ShowVisibilityProbe>,
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
    fn begin_resource_load(&mut self, resource: &str) -> u64 {
        self.resource_load_generation = self.resource_load_generation.wrapping_add(1);
        self.resource_picker.set_disabled(true);
        self.resource_picker.set_value(resource);
        self.set_status(&format!("Loading {}...", friendly_resource_label(resource)));
        self.resource_load_generation
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
        self.camera_transition = None;
        self.inspection_visual_transition = None;
        self.pending_show_visibility_probe = None;
        self.show_visibility_probe_generation =
            self.show_visibility_probe_generation.wrapping_add(1);
        self.inspection_generation = self.inspection_generation.wrapping_add(1);
        self.renderer
            .set_inspection_context_alpha_multiplier(&self.queue, 1.0);
        self.renderer.clear_section_overlays();
        self.renderer.clear_clip_plane(&self.queue);
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

    fn set_section(&mut self, section: SectionState) -> Result<(), String> {
        let overlay = section_overlay_from_state(&section)?;
        self.renderer
            .set_section_overlays(&self.device, &[overlay])
            .map_err(|error| format!("failed to upload section overlay: {error}"))?;
        match clip_side_from_section_clip_mode(section.clip) {
            ClipPlaneSide::None => self.renderer.clear_clip_plane(&self.queue),
            side => self
                .renderer
                .set_clip_plane(&self.queue, section.pose.origin, section.pose.normal, side)
                .map_err(|error| format!("failed to set section clip plane: {error}"))?,
        }
        self.runtime_scene.set_section(section);
        self.refresh_status();
        self.render()
    }

    fn clear_section(&mut self) -> Result<bool, String> {
        let changed = self.runtime_scene.clear_section();
        self.renderer.clear_section_overlays();
        self.renderer.clear_clip_plane(&self.queue);
        self.refresh_status();
        self.render()?;
        Ok(changed)
    }

    fn section_state_json(&self) -> Result<String, String> {
        serde_json::to_string(&web_section_state_snapshot(
            self.runtime_scene.section_state(),
        ))
        .map_err(|error| format!("failed to encode section state JSON: {error}"))
    }

    fn set_annotation_layer(&mut self, layer: SceneAnnotationLayer) -> Result<(), String> {
        self.runtime_scene.set_annotation_layer(layer);
        let _ = self.upload_runtime_scene(false)?;
        self.render()
    }

    fn merge_annotation_layer(&mut self, layer: SceneAnnotationLayer) -> Result<(), String> {
        self.runtime_scene.merge_annotation_layer(layer);
        let _ = self.upload_runtime_scene(false)?;
        self.render()
    }

    fn clear_annotation_layers(&mut self, layer_id: Option<&str>) -> Result<bool, String> {
        let changed = match layer_id {
            Some(layer_id) if !layer_id.trim().is_empty() => self
                .runtime_scene
                .clear_annotation_layer(&SceneAnnotationLayerId::new(layer_id.trim())),
            _ => self.runtime_scene.clear_annotation_layers(),
        };
        if changed {
            let _ = self.upload_runtime_scene(false)?;
            self.render()?;
        }
        Ok(changed)
    }

    fn annotation_state_json(&self) -> Result<String, String> {
        serde_json::to_string(&web_annotation_state_snapshot(
            self.runtime_scene.annotation_layers(),
        ))
        .map_err(|error| format!("failed to encode annotation state JSON: {error}"))
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
        let draw_count = render_scene.draw_count();
        let triangle_count = render_scene.triangle_count();
        let bounds = render_scene.bounds;
        let bounds_size = bounds.size();
        let bounds_are_finite = bounds.min.is_finite() && bounds.max.is_finite();
        let next_camera = reset_camera.then(|| fit_camera_to_render_scene(&render_scene));
        if reset_camera || draw_count == 0 || !bounds_are_finite {
            let camera = next_camera.unwrap_or_else(|| self.renderer.camera());
            log_viewer_info(&format!(
                "w web viewer uploading resource={} reset_camera={} meshes={} draws={} tris={} bounds_min=({:.3},{:.3},{:.3}) bounds_max=({:.3},{:.3},{:.3}) bounds_size=({:.3},{:.3},{:.3}) bounds_finite={} camera_eye=({:.3},{:.3},{:.3}) camera_target=({:.3},{:.3},{:.3})",
                self.current_resource,
                reset_camera,
                render_scene.definitions.len(),
                draw_count,
                triangle_count,
                bounds.min.x,
                bounds.min.y,
                bounds.min.z,
                bounds.max.x,
                bounds.max.y,
                bounds.max.z,
                bounds_size.x,
                bounds_size.y,
                bounds_size.z,
                bounds_are_finite,
                camera.eye.x,
                camera.eye.y,
                camera.eye.z,
                camera.target.x,
                camera.target.y,
                camera.target.z,
            ));
            if draw_count == 0 || !bounds_are_finite {
                log_viewer_error(&format!(
                    "w web viewer resource={} has suspicious render scene: draws={} bounds_finite={}",
                    self.current_resource, draw_count, bounds_are_finite
                ));
            }
        }
        self.renderer
            .upload_prepared_scene(&self.device, &self.queue, &render_scene);
        let annotation_layers = self
            .runtime_scene
            .annotation_layers()
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        self.renderer
            .set_annotation_layers(&self.device, &self.queue, &annotation_layers)
            .map_err(|error| format!("failed to upload annotation layers: {error}"))?;
        if reset_camera {
            self.camera_transition = None;
            let camera = next_camera.expect("reset camera path computes the next camera");
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

    fn commit_show_change(
        &mut self,
        ids: &[SemanticElementId],
        visibility_changed: u32,
        inspection_changed: u32,
    ) -> Result<Vec<DeferredWebEvent>, String> {
        if inspection_changed > 0 {
            self.inspection_generation = self.inspection_generation.wrapping_add(1);
        }
        let mut events = vec![self.upload_runtime_scene(false)?];
        if !ids.is_empty() {
            self.start_elements_frame_transition(ids, 360.0)?;
            self.queue_show_visibility_probe(ids);
        }
        if visibility_changed > 0 || inspection_changed > 0 {
            events.push(self.viewer_state_change_event("show")?);
        }
        Ok(events)
    }

    fn commit_inspection_change(
        &mut self,
        changed: u32,
        mode: &str,
    ) -> Result<Vec<DeferredWebEvent>, String> {
        self.cancel_auto_inspection_probes();
        let mut events = vec![self.upload_runtime_scene(false)?];
        if changed > 0 {
            self.start_inspection_visual_transition(mode);
            self.start_inspection_frame_transition()?;
            events.push(self.viewer_state_change_event("inspection")?);
        }
        Ok(events)
    }

    fn frame_visible_scene(&mut self) -> Result<DeferredWebEvent, String> {
        let render_scene = self.runtime_scene.compose_render_scene();
        let camera = fit_camera_to_render_scene(&render_scene);
        self.start_camera_transition(camera, 320.0);
        self.refresh_status();
        self.pick_anchor_event()
    }

    fn start_elements_frame_transition(
        &mut self,
        ids: &[SemanticElementId],
        duration_ms: f64,
    ) -> Result<(), String> {
        let Some(focus_bounds) = self.runtime_scene.bounds_for_elements(ids.iter()) else {
            return Ok(());
        };
        let scene_bounds = self.runtime_scene.visible_bounds().unwrap_or(focus_bounds);
        let camera = fit_camera_to_bounds_with_scene_context(
            self.renderer.camera(),
            focus_bounds,
            scene_bounds,
        );
        self.start_camera_transition(camera, duration_ms);
        Ok(())
    }

    fn start_inspection_frame_transition(&mut self) -> Result<(), String> {
        let inspected_ids = self.runtime_scene.inspected_element_ids();
        if inspected_ids.is_empty() {
            self.camera_transition = None;
            return Ok(());
        }

        let Some(focus_bounds) = self.runtime_scene.bounds_for_elements(inspected_ids.iter())
        else {
            return Ok(());
        };
        let scene_bounds = self.runtime_scene.visible_bounds().unwrap_or(focus_bounds);
        let camera = fit_camera_to_bounds_with_scene_context(
            self.renderer.camera(),
            focus_bounds,
            scene_bounds,
        );
        self.start_camera_transition(camera, 360.0);
        Ok(())
    }

    fn queue_show_visibility_probe(&mut self, ids: &[SemanticElementId]) {
        self.show_visibility_probe_generation =
            self.show_visibility_probe_generation.wrapping_add(1);
        self.pending_show_visibility_probe = Some(ShowVisibilityProbe {
            generation: self.show_visibility_probe_generation,
            inspection_generation: self.inspection_generation,
            element_ids: ids.to_vec(),
        });
    }

    fn ready_show_visibility_probe(&mut self) -> Option<ShowVisibilityProbe> {
        let probe = self.pending_show_visibility_probe.as_ref()?;
        if probe.inspection_generation != self.inspection_generation {
            self.pending_show_visibility_probe = None;
            return None;
        }
        let missing = self
            .runtime_scene
            .missing_stream_plan_for_elements(probe.element_ids.iter());
        if !missing.instance_ids.is_empty() || !missing.definition_ids.is_empty() {
            return None;
        }
        self.pending_show_visibility_probe.take()
    }

    fn cancel_auto_inspection_probes(&mut self) {
        self.inspection_generation = self.inspection_generation.wrapping_add(1);
        self.pending_show_visibility_probe = None;
    }

    fn start_camera_transition(&mut self, camera: Camera, duration_ms: f64) {
        let start = self.renderer.camera();
        if camera_distance(start, camera) <= 1.0e-6 {
            self.camera_transition = None;
            self.orbit = OrbitCameraController::from_camera(camera);
            self.renderer.set_camera(&self.queue, camera);
            return;
        }

        self.camera_transition = Some(CameraTransition {
            from: start,
            to: camera,
            started_at_ms: self.now_ms(),
            duration_ms: duration_ms.max(1.0),
        });
    }

    fn start_inspection_visual_transition(&mut self, mode: &str) {
        let from_multiplier = match mode {
            "add" => 1.45,
            "remove" => 1.2,
            _ => 2.25,
        };
        let to_multiplier = 1.0;
        self.inspection_visual_transition = Some(InspectionVisualTransition {
            from_multiplier,
            to_multiplier,
            started_at_ms: self.now_ms(),
            duration_ms: 300.0,
        });
        self.renderer
            .set_inspection_context_alpha_multiplier(&self.queue, from_multiplier);
    }

    fn advance_animations(&mut self, time_ms: f64) -> Result<AnimationAdvance, String> {
        let mut camera_changed = false;

        if let Some(transition) = self.camera_transition {
            let t = transition_progress(time_ms, transition.started_at_ms, transition.duration_ms);
            let eased = ease_subtle(t);
            let camera = interpolate_camera(transition.from, transition.to, eased);
            self.orbit = OrbitCameraController::from_camera(camera);
            self.renderer.set_camera(&self.queue, self.orbit.camera());
            camera_changed = true;
            if t >= 1.0 {
                self.camera_transition = None;
            }
        }

        if let Some(transition) = self.inspection_visual_transition {
            let t = transition_progress(time_ms, transition.started_at_ms, transition.duration_ms);
            let eased = ease_subtle(t) as f32;
            let multiplier = transition.from_multiplier
                + (transition.to_multiplier - transition.from_multiplier) * eased;
            self.renderer
                .set_inspection_context_alpha_multiplier(&self.queue, multiplier);
            if t >= 1.0 {
                self.inspection_visual_transition = None;
            }
        }

        let show_visibility_probe = if self.camera_transition.is_none() {
            self.ready_show_visibility_probe()
        } else {
            None
        };

        let anchor_event = if camera_changed {
            Some(self.pick_anchor_event()?)
        } else {
            None
        };

        Ok(AnimationAdvance {
            anchor_event,
            show_visibility_probe,
        })
    }

    fn now_ms(&self) -> f64 {
        js_sys::Date::now()
    }

    fn begin_drag(
        &mut self,
        x: f32,
        y: f32,
        button: i16,
    ) -> (DeferredWebEvent, Option<WebOrbitPivotPickRequest>) {
        self.camera_transition = None;
        self.pending_show_visibility_probe = None;
        let operation = self.drag_operation_for_mouse_down(button);
        let orbit_pivot = if operation == DragOperation::Orbit {
            self.scene_bounds_orbit_anchor_at_client(x, y)
        } else {
            None
        };
        let pan_anchor = if operation == DragOperation::Pan {
            self.xy_plane_point_at_client(x, y, self.orbit.target_z())
        } else {
            None
        };
        self.drag.generation = self.drag.generation.wrapping_add(1);
        let drag_generation = self.drag.generation;
        let orbit_pivot_pick = if operation == DragOperation::Orbit {
            let (x, y) = self.client_to_canvas_css(x, y);
            Some(WebOrbitPivotPickRequest {
                drag_generation,
                request: WebPickRequest::Point { x, y },
            })
        } else {
            None
        };
        self.drag.active = true;
        self.drag.suppress_next_click = false;
        self.drag.operation = operation;
        self.drag.orbit_pivot = orbit_pivot;
        self.drag.pan_anchor = pan_anchor;
        self.drag.start_x = x;
        self.drag.start_y = y;
        self.drag.last_x = x;
        self.drag.last_y = y;
        (
            self.web_event("w-viewer-drag-start", r#"{}"#.to_string()),
            orbit_pivot_pick,
        )
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
        match self.drag.operation {
            DragOperation::Orbit if mode.can_orbit() => {
                if let Some(pivot) = self.drag.orbit_pivot {
                    self.orbit.orbit_around_point_by_pixels(dx, dy, pivot);
                } else {
                    self.orbit.orbit_by_pixels(dx, dy);
                }
                self.renderer.set_camera(&self.queue, self.orbit.camera());
                Ok(Some(self.pick_anchor_event()?))
            }
            DragOperation::Pan if mode.can_pan() => {
                if let Some(anchor) = self.drag.pan_anchor {
                    let plane_z = anchor.z;
                    if let Some(current) = self.xy_plane_point_at_client(x, y, plane_z) {
                        self.orbit.translate_by(anchor - current);
                    } else {
                        self.orbit.pan_by_pixels(
                            dx,
                            dy,
                            ViewportSize::new(self.config.width, self.config.height),
                        );
                    }
                } else {
                    self.orbit.pan_by_pixels(
                        dx,
                        dy,
                        ViewportSize::new(self.config.width, self.config.height),
                    );
                }
                self.renderer.set_camera(&self.queue, self.orbit.camera());
                Ok(Some(self.pick_anchor_event()?))
            }
            _ => Ok(None),
        }
    }

    fn end_drag(&mut self) -> (Option<WebPickRequest>, DeferredWebEvent) {
        let request = if self.drag.active {
            let mode = self.interaction_mode();
            let is_box_select = self.drag.is_box_select();
            self.drag.suppress_next_click =
                is_box_select || self.drag.operation == DragOperation::Pan;
            if mode.can_pick() && self.drag.operation == DragOperation::Pick && is_box_select {
                Some(self.drag_pick_request())
            } else {
                None
            }
        } else {
            None
        };
        self.drag.active = false;
        self.drag.operation = DragOperation::None;
        self.drag.orbit_pivot = None;
        self.drag.pan_anchor = None;
        (
            request,
            self.web_event("w-viewer-drag-end", r#"{}"#.to_string()),
        )
    }

    fn cancel_drag(&mut self) -> Option<DeferredWebEvent> {
        let was_active = self.drag.active;
        self.drag.active = false;
        self.drag.operation = DragOperation::None;
        self.drag.orbit_pivot = None;
        self.drag.pan_anchor = None;
        self.drag.suppress_next_click = false;
        if was_active {
            Some(self.web_event("w-viewer-drag-end", r#"{}"#.to_string()))
        } else {
            None
        }
    }

    fn wheel(
        &mut self,
        delta_y: f32,
        delta_mode: u32,
        client_x: f32,
        client_y: f32,
    ) -> Result<DeferredWebEvent, String> {
        self.camera_transition = None;
        self.pending_show_visibility_probe = None;
        let viewport = ViewportSize::new(self.config.width, self.config.height);
        let (_, delta_y) = wheel_delta_to_pixels(0.0, delta_y, delta_mode, viewport);
        if let Some(anchor) = self.zoom_anchor_at_client(client_x, client_y) {
            self.orbit.zoom_towards_anchor_by_wheel(delta_y, anchor);
        } else {
            self.orbit.zoom_by_wheel(delta_y);
        }
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

    fn set_space_pan_modifier(&mut self, enabled: bool) {
        self.space_pan_modifier = enabled;
    }

    fn drag_operation_for_mouse_down(&self, button: i16) -> DragOperation {
        let mode = self.interaction_mode();
        let primary_button = button == 0;
        let pan_gesture = button == 1 || button == 2 || self.space_pan_modifier;

        if mode.can_pan() && (pan_gesture || (primary_button && !mode.can_orbit())) {
            return DragOperation::Pan;
        }
        if mode.can_orbit() && primary_button {
            return DragOperation::Orbit;
        }
        if mode.can_pick() && primary_button {
            return DragOperation::Pick;
        }
        DragOperation::None
    }

    fn scene_bounds_orbit_anchor_at_client(&self, client_x: f32, client_y: f32) -> Option<DVec3> {
        let (x, y) = self.client_to_canvas_css(client_x, client_y);
        let region = self.css_pick_request_to_region(WebPickRequest::Point { x, y });
        let render_scene = interactive_pick_scene(&self.runtime_scene.compose_render_scene());
        let viewport = ViewportSize::new(self.config.width, self.config.height);
        let ray =
            pick_ray_for_viewport_pixel(self.renderer.camera(), viewport, region.x, region.y)?;
        far_scene_bounds_intersection(ray.origin, ray.direction, render_scene.bounds)
    }

    fn apply_orbit_pivot_pick(&mut self, drag_generation: u64, pivot: DVec3) {
        if !pivot.is_finite()
            || !self.drag.active
            || self.drag.generation != drag_generation
            || self.drag.operation != DragOperation::Orbit
        {
            return;
        }
        self.drag.orbit_pivot = Some(pivot);
    }

    fn xy_plane_point_at_client(
        &self,
        client_x: f32,
        client_y: f32,
        plane_z: f64,
    ) -> Option<DVec3> {
        let (x, y) = self.client_to_canvas_css(client_x, client_y);
        let region = self.css_pick_request_to_region(WebPickRequest::Point { x, y });
        let ray = pick_ray_for_viewport_pixel(
            self.renderer.camera(),
            ViewportSize::new(self.config.width, self.config.height),
            region.x,
            region.y,
        )?;
        intersect_ray_with_xy_plane(ray.origin, ray.direction, plane_z)
    }

    fn zoom_anchor_at_client(&self, client_x: f32, client_y: f32) -> Option<DVec3> {
        let (x, y) = self.client_to_canvas_css(client_x, client_y);
        let region = self.css_pick_request_to_region(WebPickRequest::Point { x, y });
        let viewport = ViewportSize::new(self.config.width, self.config.height);
        let ray =
            pick_ray_for_viewport_pixel(self.renderer.camera(), viewport, region.x, region.y)?;
        let render_scene = interactive_pick_scene(&self.runtime_scene.compose_render_scene());
        far_scene_bounds_intersection(ray.origin, ray.direction, render_scene.bounds).or_else(
            || intersect_ray_with_xy_plane(ray.origin, ray.direction, self.orbit.target_z()),
        )
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
        let pick_index_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("w web pick index texture"),
            size: wgpu::Extent3d {
                width: self.config.width,
                height: self.config.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: PICK_INDEX_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let pick_index_view = pick_index_texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("w web pick index texture view"),
            ..Default::default()
        });
        let pick_depth_bits_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("w web pick depth-bits texture"),
            size: wgpu::Extent3d {
                width: self.config.width,
                height: self.config.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: PICK_DEPTH_BITS_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let pick_depth_bits_view =
            pick_depth_bits_texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("w web pick depth-bits texture view"),
                ..Default::default()
            });
        let clip_from_world = self
            .renderer
            .camera()
            .clip_from_world(ViewportSize::new(self.config.width, self.config.height));
        let depth_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("w web pick visibility depth texture"),
            size: wgpu::Extent3d {
                width: self.config.width,
                height: self.config.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.renderer.defaults().depth_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let depth_view = depth_texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("w web pick visibility depth texture view"),
            ..Default::default()
        });
        let region = self.clamp_pick_region(region);
        let unpadded_bytes_per_row = region
            .width
            .checked_mul(4)
            .ok_or("pick region row is too wide")?;
        let padded_bytes_per_row =
            align_to_web(unpadded_bytes_per_row, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let depth_bits_unpadded_bytes_per_row = unpadded_bytes_per_row;
        let depth_bits_padded_bytes_per_row = padded_bytes_per_row;
        let readback_size = u64::from(padded_bytes_per_row)
            .checked_mul(u64::from(region.height))
            .ok_or("pick region is too large")?;
        let depth_bits_readback_size = u64::from(depth_bits_padded_bytes_per_row)
            .checked_mul(u64::from(region.height))
            .ok_or("pick depth-bits region is too large")?;
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("w web pick index readback buffer"),
            size: readback_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let depth_bits_readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("w web pick depth-bits readback buffer"),
            size: depth_bits_readback_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("w web pick encoder"),
            });
        self.renderer.render_pick_region(
            &mut encoder,
            &pick_index_view,
            &pick_depth_bits_view,
            &depth_view,
            region,
        );
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &pick_index_texture,
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
                texture: &pick_depth_bits_texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: region.x,
                    y: region.y,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &depth_bits_readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(depth_bits_padded_bytes_per_row),
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
        let depth_bits_map_promise = map_buffer_for_read_web(&depth_bits_readback);
        let _ = self.device.poll(wgpu::PollType::Poll);

        Ok(WebPickReadback {
            region,
            viewport_width: self.config.width,
            viewport_height: self.config.height,
            clip_from_world,
            pick_targets: self.renderer.pick_targets().to_vec(),
            readback,
            map_promise,
            unpadded_bytes_per_row,
            padded_bytes_per_row,
            depth_bits_readback,
            depth_bits_map_promise,
            depth_bits_unpadded_bytes_per_row,
            depth_bits_padded_bytes_per_row,
        })
    }

    fn prepare_show_visibility_readback(
        &mut self,
        probe: ShowVisibilityProbe,
    ) -> Result<WebShowVisibilityReadback, String> {
        let region = self.show_visibility_probe_region(&probe.element_ids);
        let target_only =
            self.prepare_show_visibility_color_readback(region, Some(&probe.element_ids))?;
        let full_scene = self.prepare_show_visibility_color_readback(region, None)?;

        Ok(WebShowVisibilityReadback {
            probe,
            target_only,
            full_scene,
        })
    }

    fn prepare_show_visibility_color_readback(
        &self,
        region: PickRegion,
        target_ids: Option<&[SemanticElementId]>,
    ) -> Result<WebPickColorReadback, String> {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("w web show visibility probe pick index texture"),
            size: wgpu::Extent3d {
                width: self.config.width,
                height: self.config.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: PICK_INDEX_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("w web show visibility probe pick index texture view"),
            ..Default::default()
        });
        let depth_bits_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("w web show visibility probe depth-bits texture"),
            size: wgpu::Extent3d {
                width: self.config.width,
                height: self.config.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: PICK_DEPTH_BITS_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let depth_bits_view = depth_bits_texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("w web show visibility probe depth-bits texture view"),
            ..Default::default()
        });
        let depth_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("w web show visibility probe visibility depth texture"),
            size: wgpu::Extent3d {
                width: self.config.width,
                height: self.config.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.renderer.defaults().depth_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let depth_view = depth_texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("w web show visibility probe visibility depth texture view"),
            ..Default::default()
        });
        let region = self.clamp_pick_region(region);
        let unpadded_bytes_per_row = region
            .width
            .checked_mul(4)
            .ok_or("show visibility probe region row is too wide")?;
        let padded_bytes_per_row =
            align_to_web(unpadded_bytes_per_row, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let readback_size = u64::from(padded_bytes_per_row)
            .checked_mul(u64::from(region.height))
            .ok_or("show visibility probe region is too large")?;
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("w web show visibility probe readback buffer"),
            size: readback_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("w web show visibility probe encoder"),
            });
        match target_ids {
            Some(ids) => {
                self.renderer.render_pick_region_for_elements(
                    &self.device,
                    &mut encoder,
                    &view,
                    &depth_bits_view,
                    &depth_view,
                    ids,
                    region,
                );
            }
            None => {
                self.renderer.render_pick_region(
                    &mut encoder,
                    &view,
                    &depth_bits_view,
                    &depth_view,
                    region,
                );
            }
        }
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
        self.queue.submit([encoder.finish()]);

        let map_promise = map_buffer_for_read_web(&readback);
        let _ = self.device.poll(wgpu::PollType::Poll);

        Ok(WebPickColorReadback {
            region,
            pick_targets: self.renderer.pick_targets().to_vec(),
            readback,
            map_promise,
            unpadded_bytes_per_row,
            padded_bytes_per_row,
        })
    }

    fn show_visibility_probe_region(&self, ids: &[SemanticElementId]) -> PickRegion {
        self.runtime_scene
            .bounds_for_elements(ids.iter())
            .and_then(|bounds| self.project_bounds_to_pick_region(bounds, 32))
            .unwrap_or_else(|| PickRegion::rect(0, 0, self.config.width, self.config.height))
    }

    fn project_bounds_to_pick_region(&self, bounds: Bounds3, margin: u32) -> Option<PickRegion> {
        let viewport = ViewportSize::new(self.config.width, self.config.height).clamped();
        let clip_from_world = self.renderer.camera().clip_from_world(viewport);
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        let mut projected = false;

        for corner in bounds_corners(bounds) {
            let clip = clip_from_world * DVec4::new(corner.x, corner.y, corner.z, 1.0);
            if clip.w <= f64::EPSILON {
                continue;
            }
            let ndc = clip.truncate() / clip.w;
            if ndc.z < 0.0 || ndc.z > 1.0 {
                continue;
            }
            projected = true;
            let x = ((ndc.x + 1.0) * 0.5) * f64::from(viewport.width);
            let y = (1.0 - ((ndc.y + 1.0) * 0.5)) * f64::from(viewport.height);
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }

        if !projected {
            return None;
        }

        let margin = f64::from(margin);
        let x0 = (min_x - margin)
            .floor()
            .clamp(0.0, f64::from(viewport.width.saturating_sub(1))) as u32;
        let y0 = (min_y - margin)
            .floor()
            .clamp(0.0, f64::from(viewport.height.saturating_sub(1))) as u32;
        let x1 = (max_x + margin)
            .ceil()
            .clamp(0.0, f64::from(viewport.width.saturating_sub(1))) as u32;
        let y1 = (max_y + margin)
            .ceil()
            .clamp(0.0, f64::from(viewport.height.saturating_sub(1))) as u32;

        let region = PickRegion::rect(
            x0,
            y0,
            x1.saturating_sub(x0).saturating_add(1),
            y1.saturating_sub(y0).saturating_add(1),
        );
        Some(expand_pick_region_to_minimum(region, viewport, 96, 96))
    }

    fn apply_show_visibility_probe_result(
        &mut self,
        probe: &ShowVisibilityProbe,
        result: ShowVisibilityProbeResult,
    ) -> Result<Vec<DeferredWebEvent>, String> {
        if probe.generation != self.show_visibility_probe_generation {
            return Ok(Vec::new());
        }
        if probe.inspection_generation != self.inspection_generation {
            return Ok(Vec::new());
        }

        if show_visibility_probe_requires_inspection(result) {
            let changed = self
                .runtime_scene
                .set_inspection_focus(probe.element_ids.iter()) as u32;
            if changed == 0 {
                return Ok(Vec::new());
            }
            self.start_inspection_visual_transition("replace");
            Ok(vec![
                self.upload_runtime_scene(false)?,
                self.viewer_state_change_event("inspection")?,
            ])
        } else {
            let changed = self.runtime_scene.clear_inspection() as u32;
            if changed == 0 {
                return Ok(Vec::new());
            }
            self.inspection_visual_transition = None;
            self.renderer
                .set_inspection_context_alpha_multiplier(&self.queue, 1.0);
            Ok(vec![
                self.upload_runtime_scene(false)?,
                self.viewer_state_change_event("inspection")?,
            ])
        }
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
            scene_bounds: web_bounds_snapshot(render_scene.bounds),
            total_elements: catalog.elements.len(),
            total_instances: catalog.instances.len(),
            total_definitions: catalog.definitions.len(),
            default_element_ids: semantic_ids_to_strings(default_element_ids),
            base_visible_element_ids: semantic_ids_to_strings(
                self.runtime_scene.base_visible_element_ids(),
            ),
            visible_element_ids: semantic_ids_to_strings(self.runtime_scene.visible_element_ids()),
            selected_element_ids: semantic_ids_to_strings(selected_element_ids.clone()),
            inspected_element_ids: semantic_ids_to_strings(
                self.runtime_scene.inspected_element_ids(),
            ),
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
            section: web_section_state_snapshot(self.runtime_scene.section_state()),
            annotations: web_annotation_state_snapshot(self.runtime_scene.annotation_layers()),
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
        if !mode.can_pick()
            || self.drag.operation != DragOperation::Pick
            || !self.drag.active
            || !self.drag.is_box_select()
        {
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
    generation: u64,
    operation: DragOperation,
    orbit_pivot: Option<DVec3>,
    pan_anchor: Option<DVec3>,
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum DragOperation {
    #[default]
    None,
    Orbit,
    Pan,
    Pick,
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy, Debug)]
struct WebPickRay {
    origin: DVec3,
    direction: DVec3,
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WebInteractionMode {
    orbit: bool,
    pick: bool,
    pan: bool,
}

#[cfg(target_arch = "wasm32")]
impl WebInteractionMode {
    fn from_picker_value(value: &str) -> Self {
        let mut mode = Self {
            orbit: false,
            pick: false,
            pan: false,
        };
        for token in value.split('-') {
            match token {
                "orbit" => mode.orbit = true,
                "pick" => mode.pick = true,
                "pan" => mode.pan = true,
                _ => {}
            }
        }
        if !mode.orbit && !mode.pick && !mode.pan && value != "none" {
            mode.orbit = true;
        }
        if mode.orbit {
            mode.pan = true;
        }
        mode
    }

    fn can_orbit(self) -> bool {
        self.orbit
    }

    fn can_pick(self) -> bool {
        self.pick
    }

    fn can_pan(self) -> bool {
        self.pan
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
const MAX_ORBIT_PITCH_RADIANS: f64 = 1.48;

#[cfg(target_arch = "wasm32")]
impl OrbitCameraController {
    fn from_camera(camera: Camera) -> Self {
        let offset = camera.eye - camera.target;
        let radius = offset.length().max(0.25);
        let pitch_radians = (offset.z / radius)
            .clamp(-1.0, 1.0)
            .asin()
            .clamp(-MAX_ORBIT_PITCH_RADIANS, MAX_ORBIT_PITCH_RADIANS);
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

        self.yaw_radians -= f64::from(dx) * ORBIT_SENSITIVITY;
        self.pitch_radians = (self.pitch_radians + (f64::from(dy) * ORBIT_SENSITIVITY))
            .clamp(-MAX_ORBIT_PITCH_RADIANS, MAX_ORBIT_PITCH_RADIANS);
    }

    fn orbit_around_point_by_pixels(&mut self, dx: f32, dy: f32, pivot: DVec3) {
        const ORBIT_SENSITIVITY: f64 = 0.01;

        let camera = self.camera();
        let forward = normalized_or(camera.target - camera.eye, -WORLD_FORWARD);
        let right = normalized_or(forward.cross(WORLD_UP), WORLD_RIGHT);
        let yaw_delta = f64::from(dx) * ORBIT_SENSITIVITY;
        let pitch_delta = self.clamped_pitch_delta(f64::from(dy) * ORBIT_SENSITIVITY);
        let yaw = DQuat::from_axis_angle(WORLD_UP, -yaw_delta);
        let pitch = DQuat::from_axis_angle(right, -pitch_delta);
        let rotation = yaw * pitch;
        let next_camera = Camera {
            eye: pivot + rotation * (camera.eye - pivot),
            target: pivot + rotation * (camera.target - pivot),
            ..camera
        };
        *self = Self::from_camera(next_camera);
    }

    fn clamped_pitch_delta(&self, requested_delta: f64) -> f64 {
        let next_pitch = (self.pitch_radians + requested_delta)
            .clamp(-MAX_ORBIT_PITCH_RADIANS, MAX_ORBIT_PITCH_RADIANS);
        next_pitch - self.pitch_radians
    }

    fn pan_by_pixels(&mut self, dx: f32, dy: f32, viewport: ViewportSize) {
        const PAN_SPEED: f64 = 1.25;

        let camera = self.camera();
        let forward = normalized_or(camera.target - camera.eye, -WORLD_FORWARD);
        let right = normalized_xy_or(forward.cross(WORLD_UP), WORLD_RIGHT);
        let up = normalized_xy_or(right.cross(forward), forward);
        let viewport = viewport.clamped();
        let world_per_pixel =
            2.0 * self.radius * (self.vertical_fov_degrees.to_radians() * 0.5).tan()
                / f64::from(viewport.height);
        let offset = (-right * f64::from(dx) + up * f64::from(dy)) * world_per_pixel * PAN_SPEED;
        self.target += offset;
    }

    fn translate_by(&mut self, offset: DVec3) {
        self.target += DVec3::new(offset.x, offset.y, 0.0);
    }

    fn target_z(&self) -> f64 {
        self.target.z
    }

    fn zoom_by_wheel(&mut self, delta_y: f32) {
        let scale = (f64::from(delta_y) * 0.0015).exp();
        self.radius = (self.radius * scale).clamp(0.2, 500.0);
    }

    fn zoom_towards_anchor_by_wheel(&mut self, delta_y: f32, anchor: DVec3) {
        let scale = (f64::from(delta_y) * 0.0015).exp();
        let next_radius = (self.radius * scale).clamp(0.2, 500.0);
        let actual_scale = next_radius / self.radius.max(f64::EPSILON);
        let camera = self.camera();
        let next_camera = Camera {
            eye: anchor + ((camera.eye - anchor) * actual_scale),
            target: anchor + ((camera.target - anchor) * actual_scale),
            ..camera
        };
        *self = Self::from_camera(next_camera);
        self.radius = next_radius;
    }
}

#[cfg(target_arch = "wasm32")]
fn normalized_or(vector: DVec3, fallback: DVec3) -> DVec3 {
    if vector.length_squared() > 1.0e-12 {
        vector.normalize()
    } else {
        fallback
    }
}

#[cfg(target_arch = "wasm32")]
fn normalized_xy_or(vector: DVec3, fallback: DVec3) -> DVec3 {
    let vector = DVec3::new(vector.x, vector.y, 0.0);
    if vector.length_squared() > 1.0e-12 {
        vector.normalize()
    } else {
        let fallback = DVec3::new(fallback.x, fallback.y, 0.0);
        if fallback.length_squared() > 1.0e-12 {
            fallback.normalize()
        } else {
            WORLD_FORWARD
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn transition_progress(time_ms: f64, started_at_ms: f64, duration_ms: f64) -> f64 {
    ((time_ms - started_at_ms) / duration_ms.max(1.0)).clamp(0.0, 1.0)
}

#[cfg(target_arch = "wasm32")]
fn ease_subtle(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(target_arch = "wasm32")]
fn camera_distance(left: Camera, right: Camera) -> f64 {
    left.eye.distance(right.eye)
        + left.target.distance(right.target)
        + (left.vertical_fov_degrees - right.vertical_fov_degrees).abs()
}

#[cfg(target_arch = "wasm32")]
fn pick_ray_for_viewport_pixel(
    camera: Camera,
    viewport: ViewportSize,
    pixel_x: u32,
    pixel_y: u32,
) -> Option<WebPickRay> {
    pick_ray_for_viewport_position(
        camera,
        viewport,
        f64::from(pixel_x) + 0.5,
        f64::from(pixel_y) + 0.5,
    )
}

#[cfg(target_arch = "wasm32")]
fn pick_ray_for_viewport_position(
    camera: Camera,
    viewport: ViewportSize,
    pixel_x: f64,
    pixel_y: f64,
) -> Option<WebPickRay> {
    let viewport = viewport.clamped();
    let ndc_x = (pixel_x / f64::from(viewport.width)) * 2.0 - 1.0;
    let ndc_y = 1.0 - ((pixel_y / f64::from(viewport.height)) * 2.0);
    let world_from_clip = camera.clip_from_world(viewport).inverse();
    let near = unproject_clip_point(world_from_clip, DVec4::new(ndc_x, ndc_y, 1.0, 1.0))?;
    let far = unproject_clip_point(world_from_clip, DVec4::new(ndc_x, ndc_y, 0.0, 1.0))?;
    let direction = (far - near).try_normalize()?;
    Some(WebPickRay {
        origin: near,
        direction,
    })
}

#[cfg(target_arch = "wasm32")]
fn unproject_clip_point(world_from_clip: DMat4, clip: DVec4) -> Option<DVec3> {
    let world = world_from_clip * clip;
    if world.w.abs() <= f64::EPSILON {
        return None;
    }
    Some(world.truncate() / world.w)
}

#[cfg(target_arch = "wasm32")]
fn intersect_ray_with_xy_plane(origin: DVec3, direction: DVec3, plane_z: f64) -> Option<DVec3> {
    if direction.z.abs() <= 1.0e-12 {
        return None;
    }
    let distance = (plane_z - origin.z) / direction.z;
    if !distance.is_finite() || distance < 0.0 {
        return None;
    }
    Some(origin + direction * distance)
}

#[cfg(target_arch = "wasm32")]
fn far_scene_bounds_intersection(
    origin: DVec3,
    direction: DVec3,
    bounds: Bounds3,
) -> Option<DVec3> {
    let bounds = orbit_anchor_bounds(bounds)?;
    let mut near_t = f64::NEG_INFINITY;
    let mut far_t = f64::INFINITY;

    for (origin_axis, direction_axis, min_axis, max_axis) in [
        (origin.x, direction.x, bounds.min.x, bounds.max.x),
        (origin.y, direction.y, bounds.min.y, bounds.max.y),
        (origin.z, direction.z, bounds.min.z, bounds.max.z),
    ] {
        if direction_axis.abs() <= 1.0e-12 {
            if origin_axis < min_axis || origin_axis > max_axis {
                return None;
            }
            continue;
        }

        let mut t0 = (min_axis - origin_axis) / direction_axis;
        let mut t1 = (max_axis - origin_axis) / direction_axis;
        if t0 > t1 {
            std::mem::swap(&mut t0, &mut t1);
        }
        near_t = near_t.max(t0);
        far_t = far_t.min(t1);
        if near_t > far_t {
            return None;
        }
    }

    if far_t < 0.0 || !far_t.is_finite() {
        return None;
    }
    Some(origin + direction * far_t)
}

#[cfg(target_arch = "wasm32")]
fn orbit_anchor_bounds(bounds: Bounds3) -> Option<Bounds3> {
    if !bounds.min.is_finite() || !bounds.max.is_finite() {
        return None;
    }
    let size = bounds.size();
    if size.x < 0.0 || size.y < 0.0 || size.z < 0.0 {
        return None;
    }
    let max_extent = size.x.max(size.y).max(size.z);
    let epsilon = (max_extent * 1.0e-6).max(1.0e-4);
    let min = DVec3::new(
        if size.x <= epsilon {
            bounds.min.x - epsilon
        } else {
            bounds.min.x
        },
        if size.y <= epsilon {
            bounds.min.y - epsilon
        } else {
            bounds.min.y
        },
        if size.z <= epsilon {
            bounds.min.z - epsilon
        } else {
            bounds.min.z
        },
    );
    let max = DVec3::new(
        if size.x <= epsilon {
            bounds.max.x + epsilon
        } else {
            bounds.max.x
        },
        if size.y <= epsilon {
            bounds.max.y + epsilon
        } else {
            bounds.max.y
        },
        if size.z <= epsilon {
            bounds.max.z + epsilon
        } else {
            bounds.max.z
        },
    );
    Some(Bounds3 { min, max })
}

#[cfg(target_arch = "wasm32")]
fn wheel_delta_to_pixels(
    delta_x: f32,
    delta_y: f32,
    delta_mode: u32,
    viewport: ViewportSize,
) -> (f32, f32) {
    let scale = match delta_mode {
        // DOM_DELTA_LINE. Browser defaults vary, but 16 px is a good UI-line proxy.
        1 => 16.0,
        // DOM_DELTA_PAGE.
        2 => viewport.clamped().height as f32,
        _ => 1.0,
    };
    (delta_x * scale, delta_y * scale)
}

#[cfg(target_arch = "wasm32")]
fn is_space_pan_keyboard_event(event: &KeyboardEvent) -> bool {
    event.code() == "Space" || event.key() == " " || event.key() == "Spacebar"
}

#[cfg(target_arch = "wasm32")]
fn keyboard_event_targets_text_input(event: &KeyboardEvent) -> bool {
    let Some(target) = event.target() else {
        return false;
    };
    let Some(element) = target.dyn_ref::<Element>() else {
        return false;
    };
    matches!(
        element.tag_name().to_ascii_lowercase().as_str(),
        "input" | "textarea" | "select"
    )
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
        .filter(|profile| !profile.experimental)
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
fn parse_web_section_set_request(spec_json: &str) -> Result<WebSectionSetRequest, String> {
    serde_json::from_str(spec_json)
        .map_err(|error| format!("invalid section request JSON: {error}"))
}

#[cfg(target_arch = "wasm32")]
fn web_section_request_to_state(
    request: WebSectionSetRequest,
    current_resource: &str,
) -> Result<SectionState, String> {
    let pose = request.pose.to_section_pose()?;
    let resource = request
        .resource
        .filter(|resource| !resource.trim().is_empty())
        .unwrap_or_else(|| current_resource.to_string());
    let mut section = SectionState::new(resource, pose);
    section.alignment_id = request
        .alignment_id
        .filter(|alignment_id| !alignment_id.trim().is_empty());
    section.station = request
        .station
        .map(require_finite_section_scalar)
        .transpose()?;
    section.width = request
        .width
        .map(|value| require_positive_section_scalar("width", value))
        .transpose()?
        .unwrap_or(section.width);
    section.height = request
        .height
        .map(|value| require_positive_section_scalar("height", value))
        .transpose()?
        .unwrap_or(section.height);
    section.thickness = request
        .thickness
        .map(|value| require_positive_section_scalar("thickness", value))
        .transpose()?
        .unwrap_or(section.thickness);
    section.mode = parse_section_display_mode(request.mode.as_deref())?;
    section.clip = parse_section_clip_mode(request.clip.as_deref())?;
    section.provenance = request.provenance.unwrap_or_default();

    if section.mode != SectionDisplayMode::ThreeDOverlay {
        return Err(format!(
            "section display mode `{}` is not implemented yet; use `3d-overlay`",
            section_display_mode_name(section.mode)
        ));
    }

    Ok(section)
}

#[cfg(target_arch = "wasm32")]
fn parse_web_annotation_layer_set_request(
    spec_json: &str,
) -> Result<WebAnnotationLayerSetRequest, String> {
    serde_json::from_str(spec_json)
        .map_err(|error| format!("invalid annotation layer request JSON: {error}"))
}

#[cfg(target_arch = "wasm32")]
fn web_annotation_layer_request_to_state(
    request: WebAnnotationLayerSetRequest,
) -> Result<SceneAnnotationLayer, String> {
    let id = require_non_empty_annotation_id("layer id", request.id)?;
    let mut layer = SceneAnnotationLayer::new(id);
    layer.source = request.source.filter(|source| !source.trim().is_empty());
    layer.visible = request.visible.unwrap_or(true);
    layer.lifecycle = parse_annotation_lifecycle(request.lifecycle.as_deref())?;
    layer.provenance = request.provenance.unwrap_or_default();
    layer.primitives = request
        .primitives
        .into_iter()
        .map(web_annotation_primitive_request_to_state)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(layer)
}

#[cfg(target_arch = "wasm32")]
fn web_annotation_primitive_request_to_state(
    request: WebAnnotationPrimitiveRequest,
) -> Result<SceneAnnotationPrimitive, String> {
    let primitive_type = normalize_annotation_token(&request.primitive_type);
    match primitive_type.as_str() {
        "polyline" | "line" | "path" => {
            let id = require_non_empty_annotation_id("polyline id", request.id)?;
            let points = request
                .points
                .ok_or_else(|| "annotation polyline requires `points`".to_string())?
                .into_iter()
                .enumerate()
                .map(|(index, point)| annotation_dvec3_from_array("polyline point", index, point))
                .collect::<Result<Vec<_>, _>>()?;
            if points.len() < 2 {
                return Err("annotation polyline requires at least two points".to_string());
            }
            let mut polyline = ScenePolyline::new(id, points);
            if let Some(color) = request.color {
                polyline.color = parse_annotation_color("polyline color", &color)?;
            }
            if let Some(alpha) = request.alpha {
                polyline.alpha = require_annotation_unit_scalar("polyline alpha", alpha)?;
            }
            if let Some(width_px) = request.width_px {
                polyline.width_px =
                    require_positive_annotation_scalar("polyline widthPx", width_px)?;
            }
            polyline.depth_mode = parse_annotation_depth_mode(request.depth_mode.as_deref())?;
            Ok(SceneAnnotationPrimitive::Polyline(polyline))
        }
        "marker" | "point" => {
            let id = require_non_empty_annotation_id("marker id", request.id)?;
            let position = annotation_required_dvec3("marker position", request.position)?;
            let mut marker = SceneMarker::new(id, position);
            marker.direction = request
                .direction
                .map(|direction| annotation_dvec3_from_array("marker direction", 0, direction))
                .transpose()?;
            marker.normal = request
                .normal
                .map(|normal| annotation_dvec3_from_array("marker normal", 0, normal))
                .transpose()?;
            if let Some(color) = request.color {
                marker.color = parse_annotation_color("marker color", &color)?;
            }
            if let Some(alpha) = request.alpha {
                marker.alpha = require_annotation_unit_scalar("marker alpha", alpha)?;
            }
            if let Some(size_px) = request.size_px {
                marker.size_px = require_positive_annotation_scalar("marker sizePx", size_px)?;
            }
            marker.kind = parse_marker_kind(request.marker_kind.as_deref())?;
            marker.depth_mode = parse_annotation_depth_mode(request.depth_mode.as_deref())?;
            Ok(SceneAnnotationPrimitive::Marker(marker))
        }
        "text" | "label" => {
            let id = require_non_empty_annotation_id("text id", request.id)?;
            let text = request
                .text
                .ok_or_else(|| "annotation text requires `text`".to_string())?;
            let anchor = annotation_required_dvec3("text anchor", request.anchor)?;
            let mut label = SceneTextLabel::new(id, text, anchor);
            if let Some(offset) = request.screen_offset_px {
                if !offset[0].is_finite() || !offset[1].is_finite() {
                    return Err(
                        "annotation text screenOffsetPx must contain finite values".to_string()
                    );
                }
                label.screen_offset_px = DVec2::new(offset[0], offset[1]);
            }
            label.horizontal_align =
                parse_text_horizontal_align(request.horizontal_align.as_deref())?;
            label.vertical_align = parse_text_vertical_align(request.vertical_align.as_deref())?;
            label.depth_mode = parse_annotation_depth_mode(request.depth_mode.as_deref())?.into();
            if let Some(color) = request.color {
                let (color, alpha) = parse_annotation_color_with_alpha("text color", &color)?;
                label.style.color = color;
                label.style.color_alpha = alpha;
            }
            if let Some(size_px) = request.size_px {
                label.style.size_px = require_positive_annotation_scalar("text sizePx", size_px)?;
            }
            if let Some(style) = request.style {
                if let Some(color) = style.color {
                    let (color, alpha) =
                        parse_annotation_color_with_alpha("text style color", &color)?;
                    label.style.color = color;
                    label.style.color_alpha = alpha;
                }
                if let Some(color) = style.background_color {
                    let (color, alpha) =
                        parse_annotation_color_with_alpha("text style backgroundColor", &color)?;
                    label.style.background_color = Some(color);
                    label.style.background_alpha = alpha;
                }
                if let Some(color) = style.outline_color {
                    let (color, alpha) =
                        parse_annotation_color_with_alpha("text style outlineColor", &color)?;
                    label.style.outline_color = Some(color);
                    label.style.outline_alpha = alpha;
                }
                if let Some(size_px) = style.size_px {
                    label.style.size_px =
                        require_positive_annotation_scalar("text style sizePx", size_px)?;
                }
                if let Some(embolden_px) = style.embolden_px {
                    label.style.embolden_px = require_non_negative_annotation_scalar(
                        "text style emboldenPx",
                        embolden_px,
                    )?;
                }
                if let Some(padding_px) = style.padding_px {
                    label.style.padding_px =
                        require_non_negative_annotation_scalar("text style paddingPx", padding_px)?;
                }
            }
            Ok(SceneAnnotationPrimitive::Text(label))
        }
        other => Err(format!("unknown annotation primitive type `{other}`")),
    }
}

#[cfg(target_arch = "wasm32")]
impl WebSectionPoseRequest {
    fn to_section_pose(self) -> Result<SectionPose, String> {
        let origin = dvec3_from_array("origin", self.origin)?;
        let tangent = normalized_dvec3_from_array("tangent", self.tangent)?;
        let normal = normalized_dvec3_from_array("normal", self.normal)?;
        let up = normalized_dvec3_from_array("up", self.up)?;
        Ok(SectionPose::new(origin, tangent, normal, up))
    }
}

#[cfg(target_arch = "wasm32")]
fn dvec3_from_array(label: &str, value: [f64; 3]) -> Result<DVec3, String> {
    if !value.into_iter().all(f64::is_finite) {
        return Err(format!(
            "section pose `{label}` contains a non-finite coordinate"
        ));
    }
    Ok(DVec3::new(value[0], value[1], value[2]))
}

#[cfg(target_arch = "wasm32")]
fn normalized_dvec3_from_array(label: &str, value: [f64; 3]) -> Result<DVec3, String> {
    let vector = dvec3_from_array(label, value)?;
    let length = vector.length();
    if length <= f64::EPSILON {
        return Err(format!("section pose `{label}` must be a non-zero vector"));
    }
    Ok(vector / length)
}

#[cfg(target_arch = "wasm32")]
fn require_finite_section_scalar(value: f64) -> Result<f64, String> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err("section station must be finite".to_string())
    }
}

#[cfg(target_arch = "wasm32")]
fn require_positive_section_scalar(label: &str, value: f64) -> Result<f64, String> {
    if value.is_finite() && value > 0.0 {
        Ok(value)
    } else {
        Err(format!(
            "section `{label}` must be a finite positive number"
        ))
    }
}

#[cfg(target_arch = "wasm32")]
fn normalize_annotation_token(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .replace(['_', '-', ' '], "")
}

#[cfg(target_arch = "wasm32")]
fn require_non_empty_annotation_id(label: &str, value: String) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        Err(format!("annotation {label} must not be empty"))
    } else {
        Ok(value.to_string())
    }
}

#[cfg(target_arch = "wasm32")]
fn annotation_required_dvec3(label: &str, value: Option<[f64; 3]>) -> Result<DVec3, String> {
    annotation_dvec3_from_array(
        label,
        0,
        value.ok_or_else(|| format!("annotation {label} is required"))?,
    )
}

#[cfg(target_arch = "wasm32")]
fn annotation_dvec3_from_array(
    label: &str,
    index: usize,
    value: [f64; 3],
) -> Result<DVec3, String> {
    if !value.into_iter().all(f64::is_finite) {
        return Err(format!(
            "annotation {label} #{index} contains a non-finite coordinate"
        ));
    }
    Ok(DVec3::new(value[0], value[1], value[2]))
}

#[cfg(target_arch = "wasm32")]
fn require_positive_annotation_scalar(label: &str, value: f32) -> Result<f32, String> {
    if value.is_finite() && value > 0.0 {
        Ok(value)
    } else {
        Err(format!(
            "annotation {label} must be a finite positive number"
        ))
    }
}

#[cfg(target_arch = "wasm32")]
fn require_non_negative_annotation_scalar(label: &str, value: f32) -> Result<f32, String> {
    if value.is_finite() && value >= 0.0 {
        Ok(value)
    } else {
        Err(format!(
            "annotation {label} must be a finite non-negative number"
        ))
    }
}

#[cfg(target_arch = "wasm32")]
fn require_annotation_unit_scalar(label: &str, value: f32) -> Result<f32, String> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(value)
    } else {
        Err(format!(
            "annotation {label} must be a finite number from 0 to 1"
        ))
    }
}

#[cfg(target_arch = "wasm32")]
fn parse_annotation_color(label: &str, value: &serde_json::Value) -> Result<DisplayColor, String> {
    parse_annotation_color_with_alpha(label, value).map(|(color, _alpha)| color)
}

#[cfg(target_arch = "wasm32")]
fn parse_annotation_color_with_alpha(
    label: &str,
    value: &serde_json::Value,
) -> Result<(DisplayColor, f32), String> {
    if let Some(array) = value.as_array() {
        if array.len() < 3 {
            return Err(format!("annotation {label} requires at least three values"));
        }
        let rgb = [
            annotation_color_component(label, array[0].as_f64())?,
            annotation_color_component(label, array[1].as_f64())?,
            annotation_color_component(label, array[2].as_f64())?,
        ];
        let alpha = array
            .get(3)
            .map(|value| annotation_alpha_component(label, value.as_f64()))
            .transpose()?
            .unwrap_or(1.0);
        return Ok((DisplayColor { rgb }, alpha));
    } else if let Some(object) = value.as_object() {
        let component = |long: &str, short: &str| {
            object
                .get(long)
                .or_else(|| object.get(short))
                .and_then(serde_json::Value::as_f64)
        };
        let rgb = [
            annotation_color_component(label, component("red", "r"))?,
            annotation_color_component(label, component("green", "g"))?,
            annotation_color_component(label, component("blue", "b"))?,
        ];
        let alpha = component("alpha", "a")
            .map(|value| annotation_alpha_component(label, Some(value)))
            .transpose()?
            .unwrap_or(1.0);
        return Ok((DisplayColor { rgb }, alpha));
    } else {
        return Err(format!("annotation {label} must be an RGB array or object"));
    };
}

#[cfg(target_arch = "wasm32")]
fn annotation_color_component(label: &str, value: Option<f64>) -> Result<f32, String> {
    let value = value.ok_or_else(|| format!("annotation {label} is missing an RGB component"))?;
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(value as f32)
    } else {
        Err(format!(
            "annotation {label} RGB components must be finite numbers from 0 to 1"
        ))
    }
}

#[cfg(target_arch = "wasm32")]
fn annotation_alpha_component(label: &str, value: Option<f64>) -> Result<f32, String> {
    let value = value.ok_or_else(|| format!("annotation {label} alpha is not a number"))?;
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(value as f32)
    } else {
        Err(format!(
            "annotation {label} alpha components must be finite numbers from 0 to 1"
        ))
    }
}

#[cfg(target_arch = "wasm32")]
fn parse_annotation_lifecycle(value: Option<&str>) -> Result<SceneAnnotationLifecycle, String> {
    match value
        .map(normalize_annotation_token)
        .as_deref()
        .unwrap_or("")
    {
        "" | "temporary" | "temp" => Ok(SceneAnnotationLifecycle::Temporary),
        "pinned" | "pin" | "persistent" => Ok(SceneAnnotationLifecycle::Pinned),
        "diagnostic" | "debug" => Ok(SceneAnnotationLifecycle::Diagnostic),
        other => Err(format!("unknown annotation lifecycle `{other}`")),
    }
}

#[cfg(target_arch = "wasm32")]
fn parse_annotation_depth_mode(value: Option<&str>) -> Result<SceneAnnotationDepthMode, String> {
    match value
        .map(normalize_annotation_token)
        .as_deref()
        .unwrap_or("")
    {
        "" | "overlay" | "screen" => Ok(SceneAnnotationDepthMode::Overlay),
        "depthtested" | "depth" | "scene" => Ok(SceneAnnotationDepthMode::DepthTested),
        "xray" | "through" => Ok(SceneAnnotationDepthMode::XRay),
        other => Err(format!("unknown annotation depth mode `{other}`")),
    }
}

#[cfg(target_arch = "wasm32")]
fn parse_marker_kind(value: Option<&str>) -> Result<SceneMarkerKind, String> {
    match value
        .map(normalize_annotation_token)
        .as_deref()
        .unwrap_or("")
    {
        "" | "dot" | "point" => Ok(SceneMarkerKind::Dot),
        "cross" | "x" => Ok(SceneMarkerKind::Cross),
        "tick" | "check" => Ok(SceneMarkerKind::Tick),
        "arrow" => Ok(SceneMarkerKind::Arrow),
        other => Err(format!("unknown annotation marker kind `{other}`")),
    }
}

#[cfg(target_arch = "wasm32")]
fn parse_text_horizontal_align(value: Option<&str>) -> Result<SceneTextHorizontalAlign, String> {
    match value
        .map(normalize_annotation_token)
        .as_deref()
        .unwrap_or("")
    {
        "" | "center" | "middle" => Ok(SceneTextHorizontalAlign::Center),
        "left" | "start" => Ok(SceneTextHorizontalAlign::Left),
        "right" | "end" => Ok(SceneTextHorizontalAlign::Right),
        other => Err(format!(
            "unknown annotation text horizontal align `{other}`"
        )),
    }
}

#[cfg(target_arch = "wasm32")]
fn parse_text_vertical_align(value: Option<&str>) -> Result<SceneTextVerticalAlign, String> {
    match value
        .map(normalize_annotation_token)
        .as_deref()
        .unwrap_or("")
    {
        "" | "middle" | "center" => Ok(SceneTextVerticalAlign::Middle),
        "top" => Ok(SceneTextVerticalAlign::Top),
        "bottom" => Ok(SceneTextVerticalAlign::Bottom),
        "baseline" => Ok(SceneTextVerticalAlign::Baseline),
        other => Err(format!("unknown annotation text vertical align `{other}`")),
    }
}

#[cfg(target_arch = "wasm32")]
fn parse_section_display_mode(value: Option<&str>) -> Result<SectionDisplayMode, String> {
    match value
        .unwrap_or("3d-overlay")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "" | "3d-overlay" | "3doverlay" | "three-d-overlay" | "threedoverlay" => {
            Ok(SectionDisplayMode::ThreeDOverlay)
        }
        "2d-section" | "2dsection" | "two-d-section" | "twodsection" => {
            Ok(SectionDisplayMode::TwoDSection)
        }
        "both" => Ok(SectionDisplayMode::Both),
        other => Err(format!("unknown section display mode `{other}`")),
    }
}

#[cfg(target_arch = "wasm32")]
fn parse_section_clip_mode(value: Option<&str>) -> Result<SectionClipMode, String> {
    match value.unwrap_or("none").trim().to_ascii_lowercase().as_str() {
        "" | "none" | "plane" | "section-plane" | "sectionplane" | "overlay" | "3d-overlay"
        | "3doverlay" => Ok(SectionClipMode::None),
        "slice" | "section" | "cross-section" | "crosssection" | "cross_section" | "cut"
        | "cut-plane" | "cutplane" => Ok(SectionClipMode::ClipPositiveNormal),
        "positive" | "clip-positive" | "clip-positive-normal" | "clippositivenormal" => {
            Ok(SectionClipMode::ClipPositiveNormal)
        }
        "negative" | "clip-negative" | "clip-negative-normal" | "clipnegativenormal" => {
            Ok(SectionClipMode::ClipNegativeNormal)
        }
        other => Err(format!("unknown section clip mode `{other}`")),
    }
}

#[cfg(target_arch = "wasm32")]
fn section_overlay_from_state(section: &SectionState) -> Result<SectionOverlay, String> {
    if section.mode != SectionDisplayMode::ThreeDOverlay {
        return Err(format!(
            "section overlay cannot render mode `{}`",
            section_display_mode_name(section.mode)
        ));
    }
    let tangent = normalized_section_direction("tangent", section.pose.tangent)?;
    let up = normalized_section_direction("up", section.pose.up)?;
    let half_width = require_positive_section_scalar("width", section.width)? * 0.5;
    let half_height = require_positive_section_scalar("height", section.height)? * 0.5;
    let right = tangent * half_width;
    let vertical = up * half_height;
    let origin = section.pose.origin;
    Ok(SectionOverlay::new([
        origin - right - vertical,
        origin + right - vertical,
        origin + right + vertical,
        origin - right + vertical,
    ]))
}

#[cfg(target_arch = "wasm32")]
fn web_bounds_snapshot(bounds: Bounds3) -> WebBoundsSnapshot {
    WebBoundsSnapshot {
        min: bounds.min.to_array(),
        max: bounds.max.to_array(),
        center: bounds.center().to_array(),
        size: bounds.size().to_array(),
    }
}

#[cfg(target_arch = "wasm32")]
fn normalized_section_direction(label: &str, vector: DVec3) -> Result<DVec3, String> {
    if !vector.is_finite() {
        return Err(format!("section `{label}` vector is not finite"));
    }
    let length = vector.length();
    if length <= f64::EPSILON {
        return Err(format!("section `{label}` vector must be non-zero"));
    }
    Ok(vector / length)
}

#[cfg(target_arch = "wasm32")]
fn web_section_state_snapshot(section: Option<&SectionState>) -> WebSectionStateSnapshot {
    let Some(section) = section else {
        return WebSectionStateSnapshot {
            active: false,
            resource: None,
            alignment_id: None,
            station: None,
            pose: None,
            width: None,
            height: None,
            thickness: None,
            mode: None,
            clip: None,
            provenance: Vec::new(),
        };
    };

    WebSectionStateSnapshot {
        active: true,
        resource: Some(section.resource.clone()),
        alignment_id: section.alignment_id.clone(),
        station: section.station,
        pose: Some(WebSectionPoseSnapshot {
            origin: section.pose.origin.to_array(),
            tangent: section.pose.tangent.to_array(),
            normal: section.pose.normal.to_array(),
            up: section.pose.up.to_array(),
        }),
        width: Some(section.width),
        height: Some(section.height),
        thickness: Some(section.thickness),
        mode: Some(section_display_mode_name(section.mode)),
        clip: Some(section_clip_mode_name(section.clip)),
        provenance: section.provenance.clone(),
    }
}

#[cfg(target_arch = "wasm32")]
fn web_annotation_state_snapshot(layers: Vec<&SceneAnnotationLayer>) -> WebAnnotationStateSnapshot {
    WebAnnotationStateSnapshot {
        count: layers.len(),
        layers: layers
            .into_iter()
            .map(web_annotation_layer_snapshot)
            .collect(),
    }
}

#[cfg(target_arch = "wasm32")]
fn web_annotation_layer_snapshot(layer: &SceneAnnotationLayer) -> WebAnnotationLayerSnapshot {
    WebAnnotationLayerSnapshot {
        id: layer.id.as_str().to_string(),
        source: layer.source.clone(),
        visible: layer.visible,
        lifecycle: annotation_lifecycle_name(layer.lifecycle),
        primitives: layer
            .primitives
            .iter()
            .map(web_annotation_primitive_snapshot)
            .collect(),
        provenance: layer.provenance.clone(),
    }
}

#[cfg(target_arch = "wasm32")]
fn web_annotation_primitive_snapshot(
    primitive: &SceneAnnotationPrimitive,
) -> WebAnnotationPrimitiveSnapshot {
    match primitive {
        SceneAnnotationPrimitive::Polyline(polyline) => WebAnnotationPrimitiveSnapshot::Polyline {
            id: polyline.id.as_str().to_string(),
            points: polyline
                .points
                .iter()
                .map(|point| point.to_array())
                .collect(),
            color: polyline.color.as_rgb(),
            alpha: polyline.alpha,
            width_px: polyline.width_px,
            depth_mode: annotation_depth_mode_name(polyline.depth_mode),
        },
        SceneAnnotationPrimitive::Marker(marker) => WebAnnotationPrimitiveSnapshot::Marker {
            id: marker.id.as_str().to_string(),
            position: marker.position.to_array(),
            direction: marker.direction.map(|direction| direction.to_array()),
            normal: marker.normal.map(|normal| normal.to_array()),
            color: marker.color.as_rgb(),
            alpha: marker.alpha,
            size_px: marker.size_px,
            marker_kind: marker_kind_name(marker.kind),
            depth_mode: annotation_depth_mode_name(marker.depth_mode),
        },
        SceneAnnotationPrimitive::Text(label) => WebAnnotationPrimitiveSnapshot::Text {
            id: label.id.as_str().to_string(),
            text: label.text.clone(),
            anchor: label.anchor.to_array(),
            screen_offset_px: label.screen_offset_px.to_array(),
            horizontal_align: text_horizontal_align_name(label.horizontal_align),
            vertical_align: text_vertical_align_name(label.vertical_align),
            depth_mode: text_depth_mode_name(label.depth_mode),
            style: WebAnnotationTextStyleSnapshot {
                color: display_color_rgba(label.style.color, label.style.color_alpha),
                background_color: label
                    .style
                    .background_color
                    .map(|color| display_color_rgba(color, label.style.background_alpha)),
                outline_color: label
                    .style
                    .outline_color
                    .map(|color| display_color_rgba(color, label.style.outline_alpha)),
                size_px: label.style.size_px,
                embolden_px: label.style.embolden_px,
                padding_px: label.style.padding_px,
            },
        },
    }
}

#[cfg(target_arch = "wasm32")]
fn display_color_rgba(color: DisplayColor, alpha: f32) -> [f32; 4] {
    let [red, green, blue] = color.as_rgb();
    [red, green, blue, alpha.clamp(0.0, 1.0)]
}

#[cfg(target_arch = "wasm32")]
fn section_display_mode_name(mode: SectionDisplayMode) -> &'static str {
    match mode {
        SectionDisplayMode::ThreeDOverlay => "3d-overlay",
        SectionDisplayMode::TwoDSection => "2d-section",
        SectionDisplayMode::Both => "both",
    }
}

#[cfg(target_arch = "wasm32")]
fn section_clip_mode_name(mode: SectionClipMode) -> &'static str {
    match mode {
        SectionClipMode::None => "none",
        SectionClipMode::ClipPositiveNormal => "clip-positive-normal",
        SectionClipMode::ClipNegativeNormal => "clip-negative-normal",
    }
}

#[cfg(target_arch = "wasm32")]
fn annotation_lifecycle_name(lifecycle: SceneAnnotationLifecycle) -> &'static str {
    match lifecycle {
        SceneAnnotationLifecycle::Temporary => "temporary",
        SceneAnnotationLifecycle::Pinned => "pinned",
        SceneAnnotationLifecycle::Diagnostic => "diagnostic",
    }
}

#[cfg(target_arch = "wasm32")]
fn annotation_depth_mode_name(mode: SceneAnnotationDepthMode) -> &'static str {
    match mode {
        SceneAnnotationDepthMode::Overlay => "overlay",
        SceneAnnotationDepthMode::DepthTested => "depth-tested",
        SceneAnnotationDepthMode::XRay => "xray",
    }
}

#[cfg(target_arch = "wasm32")]
fn marker_kind_name(kind: SceneMarkerKind) -> &'static str {
    match kind {
        SceneMarkerKind::Dot => "dot",
        SceneMarkerKind::Cross => "cross",
        SceneMarkerKind::Tick => "tick",
        SceneMarkerKind::Arrow => "arrow",
    }
}

#[cfg(target_arch = "wasm32")]
fn text_horizontal_align_name(align: SceneTextHorizontalAlign) -> &'static str {
    match align {
        SceneTextHorizontalAlign::Center => "center",
        SceneTextHorizontalAlign::Left => "left",
        SceneTextHorizontalAlign::Right => "right",
    }
}

#[cfg(target_arch = "wasm32")]
fn text_vertical_align_name(align: SceneTextVerticalAlign) -> &'static str {
    match align {
        SceneTextVerticalAlign::Middle => "middle",
        SceneTextVerticalAlign::Top => "top",
        SceneTextVerticalAlign::Bottom => "bottom",
        SceneTextVerticalAlign::Baseline => "baseline",
    }
}

#[cfg(target_arch = "wasm32")]
fn text_depth_mode_name(mode: cc_w_types::SceneTextDepthMode) -> &'static str {
    match mode {
        cc_w_types::SceneTextDepthMode::Overlay => "overlay",
        cc_w_types::SceneTextDepthMode::DepthTested => "depth-tested",
        cc_w_types::SceneTextDepthMode::XRay => "xray",
    }
}

#[cfg(target_arch = "wasm32")]
fn clip_side_from_section_clip_mode(mode: SectionClipMode) -> ClipPlaneSide {
    match mode {
        SectionClipMode::None => ClipPlaneSide::None,
        SectionClipMode::ClipPositiveNormal => ClipPlaneSide::PositiveNormal,
        SectionClipMode::ClipNegativeNormal => ClipPlaneSide::NegativeNormal,
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
            experimental: profile.experimental,
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
fn log_viewer_info(message: &str) {
    web_sys::console::log_1(&JsValue::from_str(message));
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
    let inspected_elements = runtime_scene.inspected_element_ids().len();
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
    let inspection_status = if inspected_elements == 0 {
        String::new()
    } else {
        format!(" · {inspected_elements} inspected")
    };
    let section_status = runtime_scene
        .section_state()
        .map(|section| {
            section
                .station
                .map(|station| format!(" · section station {station}"))
                .unwrap_or_else(|| " · section".to_string())
        })
        .unwrap_or_default();
    format!(
        "{} · {view_mode} · {} meshes · {} tris · {} draws · {visible_elements}/{total_elements} visible · {selected_elements} selected{inspection_status}{section_status} · {stream_status}",
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
