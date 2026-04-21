use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;

use cc_w_backend::{GeometryBackend, GeometryBackendError};
use cc_w_db::{ImportedGeometryResourceInstance, ImportedGeometrySceneResource};
use cc_w_types::{
    Bounds3, CoordinateFrame, CurveSegment2, DefaultRenderClass, DisplayColor, ExternalId,
    GeometryDefinition, GeometryDefinitionId, GeometryInstance, GeometryInstanceId,
    GeometryPrimitive, IndexedPolygon, LengthUnit, LineSegment2, Polycurve2,
    PreparedGeometryDefinition, PreparedGeometryElement, PreparedGeometryInstance,
    PreparedGeometryPackage, PreparedMesh, PreparedVertex, Profile2, ProfileLoop2,
    SemanticElementId, SourceSpace, SweepPath, SweptSolid, TessellatedGeometry,
};
use glam::{DMat4, DVec2, DVec3};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use thiserror::Error;
use velr::{CellRef, TableResult, Velr};
use velr_graphql_core::{GraphQlRequest, GraphQlResponse, GraphQlServer, JsonMap};
use velr_graphql_managed::{ManagedGraphQlDatabase, ManagedGraphQlExecutor};

// Keep the first GraphQL smoke query intentionally small while we validate the local
// import -> Velr -> GraphQL wiring against real IFC fixtures.
pub const IFC_PROJECT_LIST_QUERY: &str = r#"
query {
  ifcProjectList {
    __typename
    id
  }
}
"#;

const BODY_PACKAGE_CACHE_VERSION: u32 = 8;
const BODY_PACKAGE_CACHE_FILE: &str = "prepared-package.json";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IfcSchemaId {
    Ifc2x3Tc1,
    Ifc4,
    Ifc4x3Add2,
    Other(String),
}

impl IfcSchemaId {
    pub fn parse(token: &str) -> Self {
        let normalized = token
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric())
            .map(|ch| ch.to_ascii_uppercase())
            .collect::<String>();

        match normalized.as_str() {
            "IFC2X3" | "IFC2X3TC1" => Self::Ifc2x3Tc1,
            "IFC4" => Self::Ifc4,
            "IFC4X3" | "IFC4X3ADD2" => Self::Ifc4x3Add2,
            _ => Self::Other(token.trim().to_string()),
        }
    }

    pub const fn canonical_name(&self) -> &str {
        match self {
            Self::Ifc2x3Tc1 => "IFC2X3_TC1",
            Self::Ifc4 => "IFC4",
            Self::Ifc4x3Add2 => "IFC4X3_ADD2",
            Self::Other(_) => "OTHER",
        }
    }

    pub const fn generated_artifact_stem(&self) -> Option<&'static str> {
        match self {
            Self::Ifc2x3Tc1 => Some("ifc2x3_tc1"),
            Self::Ifc4 => Some("ifc4"),
            Self::Ifc4x3Add2 => Some("ifc4x3_add2"),
            Self::Other(_) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IfcFixtureSpec {
    pub slug: &'static str,
    pub file_name: &'static str,
    pub source_relative_path: &'static str,
}

impl IfcFixtureSpec {
    pub fn repo_fixture_path(self, workspace_root: &Path) -> PathBuf {
        workspace_root.join("fixtures/ifc").join(self.file_name)
    }

    pub fn velr_ifc_source_path(self, velr_ifc_root: &Path) -> PathBuf {
        velr_ifc_root.join(self.source_relative_path)
    }
}

pub const CURATED_IFC_FIXTURES: &[IfcFixtureSpec] = &[
    IfcFixtureSpec {
        slug: "building-architecture",
        file_name: "building-architecture.ifc",
        source_relative_path: "testdata/buildingSMART/IFC4X3_ADD2/PCERT-Sample-Scene/Building-Architecture.ifc",
    },
    IfcFixtureSpec {
        slug: "building-hvac",
        file_name: "building-hvac.ifc",
        source_relative_path: "testdata/buildingSMART/IFC4X3_ADD2/PCERT-Sample-Scene/Building-Hvac.ifc",
    },
    IfcFixtureSpec {
        slug: "building-landscaping",
        file_name: "building-landscaping.ifc",
        source_relative_path: "testdata/buildingSMART/IFC4X3_ADD2/PCERT-Sample-Scene/Building-Landscaping.ifc",
    },
    IfcFixtureSpec {
        slug: "building-structural",
        file_name: "building-structural.ifc",
        source_relative_path: "testdata/buildingSMART/IFC4X3_ADD2/PCERT-Sample-Scene/Building-Structural.ifc",
    },
    IfcFixtureSpec {
        slug: "infra-bridge",
        file_name: "infra-bridge.ifc",
        source_relative_path: "testdata/buildingSMART/IFC4X3_ADD2/PCERT-Sample-Scene/Infra-Bridge.ifc",
    },
    IfcFixtureSpec {
        slug: "infra-landscaping",
        file_name: "infra-landscaping.ifc",
        source_relative_path: "testdata/buildingSMART/IFC4X3_ADD2/PCERT-Sample-Scene/Infra-Landscaping.ifc",
    },
    IfcFixtureSpec {
        slug: "infra-plumbing",
        file_name: "infra-plumbing.ifc",
        source_relative_path: "testdata/buildingSMART/IFC4X3_ADD2/PCERT-Sample-Scene/Infra-Plumbing.ifc",
    },
    IfcFixtureSpec {
        slug: "infra-rail",
        file_name: "infra-rail.ifc",
        source_relative_path: "testdata/buildingSMART/IFC4X3_ADD2/PCERT-Sample-Scene/Infra-Rail.ifc",
    },
    IfcFixtureSpec {
        slug: "infra-road",
        file_name: "infra-road.ifc",
        source_relative_path: "testdata/buildingSMART/IFC4X3_ADD2/PCERT-Sample-Scene/Infra-Road.ifc",
    },
    IfcFixtureSpec {
        slug: "openifcmodel-20210219-architecture",
        file_name: "openifcmodel-20210219-architecture.ifc",
        source_relative_path: "testdata/buildingSMART/IFC4X3_ADD2/openifcmodel/20210219Architecture.ifc",
    },
    IfcFixtureSpec {
        slug: "fzk-haus",
        file_name: "fzk-haus.ifc",
        source_relative_path: "testdata/openifcmodel/ifc4/20201030AC20-FZK-Haus.ifc",
    },
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IfcArtifactLayout {
    pub model_slug: String,
    pub model_root: PathBuf,
    pub source_ifc: PathBuf,
    pub database: PathBuf,
    pub import_dir: PathBuf,
    pub import_bundle: PathBuf,
    pub import_cypher: PathBuf,
    pub import_issues: PathBuf,
    pub import_timing: PathBuf,
    pub import_log: PathBuf,
    pub geometry_dir: PathBuf,
}

impl IfcArtifactLayout {
    pub fn new(base_root: impl AsRef<Path>, model_slug: impl Into<String>) -> Self {
        let model_slug = model_slug.into();
        let model_root = base_root.as_ref().join(&model_slug);
        let import_dir = model_root.join("import");
        let geometry_dir = model_root.join("geometry");

        Self {
            source_ifc: model_root.join("source.ifc"),
            database: model_root.join("model.velr.db"),
            import_bundle: import_dir.join("import-bundle.json"),
            import_cypher: import_dir.join("import.cypher"),
            import_issues: import_dir.join("issues.json"),
            import_timing: import_dir.join("import-timing.json"),
            import_log: import_dir.join("import-log.txt"),
            geometry_dir,
            import_dir,
            model_root,
            model_slug,
        }
    }

    pub fn ensure_dirs(&self) -> Result<(), VelrIfcError> {
        create_dir_all(&self.model_root)?;
        create_dir_all(&self.import_dir)?;
        create_dir_all(&self.geometry_dir)?;
        Ok(())
    }

    pub fn validate_cypher_inputs(&self) -> Result<(), VelrIfcError> {
        require_exists(&self.database)?;
        Ok(())
    }

    pub fn validate_graphql_inputs(&self) -> Result<(), VelrIfcError> {
        self.validate_cypher_inputs()?;
        self.graphql_runtime_layout()?.validate_graphql_inputs()
    }

    pub fn validate_model_inputs(&self) -> Result<(), VelrIfcError> {
        self.validate_cypher_inputs()?;
        self.graphql_runtime_layout()?.validate_model_inputs()
    }

    pub fn authoritative_schema(&self) -> Result<IfcSchemaId, VelrIfcError> {
        if let Some(schema) = schema_from_import_bundle_if_exists(&self.import_bundle)? {
            return Ok(schema);
        }
        if let Some(schema) = schema_from_import_log_if_exists(&self.import_log)? {
            return Ok(schema);
        }
        if let Some(schema) = schema_from_source_ifc_if_exists(&self.source_ifc)? {
            return Ok(schema);
        }

        Err(VelrIfcError::IfcGeometryData(format!(
            "could not determine IFC schema for model `{}`",
            self.model_slug
        )))
    }

    pub fn graphql_runtime_layout(&self) -> Result<IfcSchemaGraphQlLayout, VelrIfcError> {
        let artifacts_root = self.model_root.parent().ok_or_else(|| {
            VelrIfcError::IfcGeometryData(format!(
                "model `{}` has no artifacts root parent",
                self.model_slug
            ))
        })?;
        IfcSchemaGraphQlLayout::new(artifacts_root, self.authoritative_schema()?)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IfcSchemaGraphQlLayout {
    pub schema: IfcSchemaId,
    pub root: PathBuf,
    pub runtime_graphql: PathBuf,
    pub runtime_mapping: PathBuf,
    pub runtime_manifest: PathBuf,
    pub feature_queries: PathBuf,
}

impl IfcSchemaGraphQlLayout {
    pub fn new(
        artifacts_root: impl AsRef<Path>,
        schema: IfcSchemaId,
    ) -> Result<Self, VelrIfcError> {
        let stem = schema.generated_artifact_stem().ok_or_else(|| {
            VelrIfcError::IfcGeometryData(format!(
                "no shared GraphQL runtime bundle is configured for `{}`",
                schema.canonical_name()
            ))
        })?;
        let root = artifacts_root.as_ref().join("_graphql").join(stem);
        Ok(Self {
            schema,
            runtime_graphql: root.join("ifc-runtime.graphql"),
            runtime_mapping: root.join("ifc-runtime.mapping.json"),
            runtime_manifest: root.join("handoff-manifest.json"),
            feature_queries: root.join("feature-queries.graphql"),
            root,
        })
    }

    pub fn ensure_dirs(&self) -> Result<(), VelrIfcError> {
        create_dir_all(&self.root)?;
        Ok(())
    }

    pub fn validate_graphql_inputs(&self) -> Result<(), VelrIfcError> {
        require_exists(&self.runtime_graphql)?;
        Ok(())
    }

    pub fn validate_model_inputs(&self) -> Result<(), VelrIfcError> {
        self.validate_graphql_inputs()?;
        require_exists(&self.runtime_mapping)?;
        require_exists(&self.runtime_manifest)?;
        Ok(())
    }

    pub fn runtime_bundle_schema(&self) -> Result<Option<IfcSchemaId>, VelrIfcError> {
        if let Some(schema) = schema_from_runtime_mapping_if_exists(&self.runtime_mapping)? {
            return Ok(Some(schema));
        }
        schema_from_runtime_manifest_if_exists(&self.runtime_manifest)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IfcFixtureSyncResult {
    pub slug: String,
    pub source: PathBuf,
    pub destination: PathBuf,
    pub bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IfcImportSummary {
    pub model_slug: String,
    pub model_root: PathBuf,
    pub source_ifc: PathBuf,
    pub database: PathBuf,
    pub schema: IfcSchemaId,
    pub import_timing: PathBuf,
    pub import_log: PathBuf,
    pub reused_existing: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IfcRuntimeRefreshSummary {
    pub schema: IfcSchemaId,
    pub root: PathBuf,
    pub runtime_graphql: PathBuf,
    pub runtime_mapping: PathBuf,
    pub runtime_manifest: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IfcProjectOverview {
    pub id: String,
    pub declared_entity: String,
    pub global_id: Option<String>,
    pub name: Option<String>,
    pub long_name: Option<String>,
    pub phase: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IfcModelOverview {
    pub database: PathBuf,
    pub node_count: i64,
    pub edge_count: i64,
    pub projects: Vec<IfcProjectOverview>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CypherQueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IfcBodyGeometrySummary {
    pub definitions: usize,
    pub elements: usize,
    pub instances: usize,
    pub triangles: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IfcBodyPackageCacheStatus {
    Hit,
    Miss,
}

impl IfcBodyPackageCacheStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Hit => "cache_hit",
            Self::Miss => "cache_miss",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct IfcBodyPackageLoad {
    pub package: PreparedGeometryPackage,
    pub cache_status: IfcBodyPackageCacheStatus,
}

impl IfcBodyPackageLoad {
    pub fn geometry_summary(&self) -> IfcBodyGeometrySummary {
        summarize_body_package(&self.package)
    }

    pub fn instance_summaries(&self) -> Vec<IfcBodyInstanceSummary> {
        summarize_body_instances(&self.package)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct CachedFileFingerprint {
    bytes: u64,
    modified_unix_seconds: u64,
    modified_subsec_nanos: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct CachedPreparedGeometryPackage {
    cache_version: u32,
    schema: IfcSchemaId,
    database: CachedFileFingerprint,
    definitions: Vec<CachedPreparedGeometryDefinition>,
    elements: Vec<CachedPreparedGeometryElement>,
    instances: Vec<CachedPreparedGeometryInstance>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct CachedPreparedGeometryDefinition {
    id: u64,
    mesh: CachedPreparedMesh,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct CachedPreparedGeometryElement {
    id: String,
    label: String,
    declared_entity: String,
    default_render_class: String,
    bounds_min: [f64; 3],
    bounds_max: [f64; 3],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct CachedPreparedGeometryInstance {
    id: u64,
    element_id: String,
    definition_id: u64,
    transform: [f64; 16],
    bounds_min: [f64; 3],
    bounds_max: [f64; 3],
    external_id: String,
    label: String,
    display_color: Option<[f32; 3]>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct CachedPreparedMesh {
    local_origin: [f64; 3],
    bounds_min: [f64; 3],
    bounds_max: [f64; 3],
    vertices: Vec<CachedPreparedVertex>,
    indices: Vec<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
struct CachedPreparedVertex {
    position: [f32; 3],
    normal: [f32; 3],
}

impl CachedPreparedGeometryPackage {
    fn from_prepared_package(
        schema: IfcSchemaId,
        database: CachedFileFingerprint,
        package: &PreparedGeometryPackage,
    ) -> Self {
        Self {
            cache_version: BODY_PACKAGE_CACHE_VERSION,
            schema,
            database,
            definitions: package
                .definitions
                .iter()
                .map(CachedPreparedGeometryDefinition::from_prepared_definition)
                .collect(),
            elements: package
                .elements
                .iter()
                .map(CachedPreparedGeometryElement::from_prepared_element)
                .collect(),
            instances: package
                .instances
                .iter()
                .map(CachedPreparedGeometryInstance::from_prepared_instance)
                .collect(),
        }
    }

    fn into_prepared_package(self) -> PreparedGeometryPackage {
        PreparedGeometryPackage {
            definitions: self
                .definitions
                .into_iter()
                .map(CachedPreparedGeometryDefinition::into_prepared_definition)
                .collect(),
            elements: self
                .elements
                .into_iter()
                .map(CachedPreparedGeometryElement::into_prepared_element)
                .collect(),
            instances: self
                .instances
                .into_iter()
                .map(CachedPreparedGeometryInstance::into_prepared_instance)
                .collect(),
        }
    }
}

impl CachedPreparedGeometryDefinition {
    fn from_prepared_definition(definition: &PreparedGeometryDefinition) -> Self {
        Self {
            id: definition.id.0,
            mesh: CachedPreparedMesh::from_prepared_mesh(&definition.mesh),
        }
    }

    fn into_prepared_definition(self) -> PreparedGeometryDefinition {
        PreparedGeometryDefinition {
            id: GeometryDefinitionId(self.id),
            mesh: self.mesh.into_prepared_mesh(),
        }
    }
}

impl CachedPreparedGeometryElement {
    fn from_prepared_element(element: &PreparedGeometryElement) -> Self {
        Self {
            id: element.id.as_str().to_string(),
            label: element.label.clone(),
            declared_entity: element.declared_entity.clone(),
            default_render_class: cached_render_class_name(element.default_render_class)
                .to_string(),
            bounds_min: dvec3_to_array(element.bounds.min),
            bounds_max: dvec3_to_array(element.bounds.max),
        }
    }

    fn into_prepared_element(self) -> PreparedGeometryElement {
        PreparedGeometryElement {
            id: SemanticElementId::new(self.id),
            label: self.label,
            declared_entity: self.declared_entity,
            default_render_class: parse_cached_render_class(&self.default_render_class),
            bounds: bounds_from_arrays(self.bounds_min, self.bounds_max),
        }
    }
}

impl CachedPreparedGeometryInstance {
    fn from_prepared_instance(instance: &PreparedGeometryInstance) -> Self {
        Self {
            id: instance.id.0,
            element_id: instance.element_id.as_str().to_string(),
            definition_id: instance.definition_id.0,
            transform: instance.transform.to_cols_array(),
            bounds_min: dvec3_to_array(instance.bounds.min),
            bounds_max: dvec3_to_array(instance.bounds.max),
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
            transform: DMat4::from_cols_array(&self.transform),
            bounds: bounds_from_arrays(self.bounds_min, self.bounds_max),
            external_id: ExternalId::new(self.external_id),
            label: self.label,
            display_color: self
                .display_color
                .map(|rgb| DisplayColor::new(rgb[0], rgb[1], rgb[2])),
        }
    }
}

impl CachedPreparedMesh {
    fn from_prepared_mesh(mesh: &PreparedMesh) -> Self {
        Self {
            local_origin: dvec3_to_array(mesh.local_origin),
            bounds_min: dvec3_to_array(mesh.bounds.min),
            bounds_max: dvec3_to_array(mesh.bounds.max),
            vertices: mesh
                .vertices
                .iter()
                .copied()
                .map(CachedPreparedVertex::from_prepared_vertex)
                .collect(),
            indices: mesh.indices.clone(),
        }
    }

    fn into_prepared_mesh(self) -> PreparedMesh {
        PreparedMesh {
            local_origin: array_to_dvec3(self.local_origin),
            bounds: bounds_from_arrays(self.bounds_min, self.bounds_max),
            vertices: self
                .vertices
                .into_iter()
                .map(CachedPreparedVertex::into_prepared_vertex)
                .collect(),
            indices: self.indices,
        }
    }
}

impl CachedPreparedVertex {
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

fn dvec3_to_array(value: DVec3) -> [f64; 3] {
    [value.x, value.y, value.z]
}

fn array_to_dvec3(value: [f64; 3]) -> DVec3 {
    DVec3::new(value[0], value[1], value[2])
}

fn bounds_from_arrays(min: [f64; 3], max: [f64; 3]) -> Bounds3 {
    Bounds3 {
        min: array_to_dvec3(min),
        max: array_to_dvec3(max),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct IfcBodyInstanceSummary {
    pub instance_id: u64,
    pub definition_id: u64,
    pub external_id: String,
    pub label: String,
    pub display_color: Option<DisplayColor>,
    pub bounds_min: DVec3,
    pub bounds_max: DVec3,
    pub bounds_center: DVec3,
    pub bounds_size: DVec3,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IfcPlacementSummary {
    pub local_placements: usize,
    pub placements_with_relative_placement: usize,
    pub placements_missing_relative_placement: usize,
    pub placements_with_parent: usize,
}

#[derive(Clone, Debug, PartialEq)]
struct IfcBodyRecord {
    product_id: u64,
    placement_id: Option<u64>,
    item_id: u64,
    global_id: Option<String>,
    name: Option<String>,
    display_color: Option<DisplayColor>,
    declared_entity: String,
    item_transform: DMat4,
    primitive: GeometryPrimitive,
}

#[derive(Clone, Debug)]
struct IfcLocalPlacementRecord {
    placement_id: u64,
    parent_placement_id: Option<u64>,
    relative_location: Option<DVec3>,
    axis: Option<DVec3>,
    ref_direction: Option<DVec3>,
}

#[derive(Clone, Debug)]
pub struct IfcImportOptions {
    pub velr_ifc_root: PathBuf,
    pub artifacts_root: PathBuf,
    pub release: bool,
    pub replace_existing: bool,
}

impl Default for IfcImportOptions {
    fn default() -> Self {
        Self {
            velr_ifc_root: default_velr_ifc_checkout(),
            artifacts_root: default_ifc_artifacts_root(),
            release: true,
            replace_existing: false,
        }
    }
}

pub struct VelrIfcModel {
    layout: IfcArtifactLayout,
    raw_db: Velr,
}

impl VelrIfcModel {
    pub fn open(layout: IfcArtifactLayout) -> Result<Self, VelrIfcError> {
        layout.validate_cypher_inputs()?;

        let database_path = path_to_utf8(&layout.database)?;
        let raw_db = Velr::open(Some(database_path.as_str()))?;

        Ok(Self { layout, raw_db })
    }

    pub fn schema_id(&self) -> Result<IfcSchemaId, VelrIfcError> {
        self.layout.authoritative_schema()
    }

    pub fn open_from_artifacts_root(
        artifacts_root: impl AsRef<Path>,
        model_slug: impl Into<String>,
    ) -> Result<Self, VelrIfcError> {
        Self::open(IfcArtifactLayout::new(artifacts_root, model_slug))
    }

    pub fn load_body_package_from_artifacts_root(
        artifacts_root: impl AsRef<Path>,
        model_slug: impl Into<String>,
    ) -> Result<PreparedGeometryPackage, VelrIfcError> {
        Ok(
            Self::load_body_package_with_cache_status_from_artifacts_root(
                artifacts_root,
                model_slug,
            )?
            .package,
        )
    }

    pub fn load_body_package_with_cache_status_from_artifacts_root(
        artifacts_root: impl AsRef<Path>,
        model_slug: impl Into<String>,
    ) -> Result<IfcBodyPackageLoad, VelrIfcError> {
        let layout = IfcArtifactLayout::new(artifacts_root, model_slug);
        layout.validate_cypher_inputs()?;

        if let Some(package) = load_cached_body_package_from_layout(&layout)? {
            return Ok(IfcBodyPackageLoad {
                package,
                cache_status: IfcBodyPackageCacheStatus::Hit,
            });
        }

        let model = Self::open(layout)?;
        model.build_body_package_with_cache_status()
    }

    pub fn layout(&self) -> &IfcArtifactLayout {
        &self.layout
    }

    pub fn execute_graphql_blocking(
        &self,
        request: GraphQlRequest,
    ) -> Result<GraphQlResponse, VelrIfcError> {
        Ok(self.graphql_executor()?.execute_blocking(request)?)
    }

    pub fn execute_cypher_rows(&self, cypher: &str) -> Result<CypherQueryResult, VelrIfcError> {
        exec_cypher_in_scoped_tx(&self.raw_db, cypher, |table| {
            let columns = table.column_names().to_vec();
            let mut rows = Vec::new();

            table.for_each_row(|row| {
                rows.push(row.iter().map(render_cell).collect());
                Ok(())
            })?;

            Ok(CypherQueryResult { columns, rows })
        })
    }

    pub fn query_projects_graphql(&self) -> Result<Vec<IfcProjectOverview>, VelrIfcError> {
        let response =
            self.execute_graphql_blocking(GraphQlRequest::new(IFC_PROJECT_LIST_QUERY.trim()))?;
        parse_projects_response(response)
    }

    pub fn query_projects_raw(&self) -> Result<Vec<IfcProjectOverview>, VelrIfcError> {
        let result =
            self.execute_cypher_rows("MATCH (n:IfcProject) RETURN id(n) AS id ORDER BY id")?;

        let mut projects = Vec::with_capacity(result.rows.len());
        for row in result.rows {
            projects.push(IfcProjectOverview {
                id: row.first().cloned().unwrap_or_default(),
                declared_entity: "IfcProject".to_string(),
                global_id: None,
                name: None,
                long_name: None,
                phase: None,
            });
        }

        Ok(projects)
    }

    pub fn model_overview(&self) -> Result<IfcModelOverview, VelrIfcError> {
        Ok(IfcModelOverview {
            database: self.layout.database.clone(),
            node_count: scalar_count(&self.raw_db, "MATCH (n) RETURN count(n)")?,
            edge_count: scalar_count(&self.raw_db, "MATCH ()-[r]->() RETURN count(r)")?,
            projects: self
                .query_projects_graphql()
                .or_else(|_| self.query_projects_raw())?,
        })
    }

    pub fn build_body_package(&self) -> Result<PreparedGeometryPackage, VelrIfcError> {
        Ok(self.build_body_package_with_cache_status()?.package)
    }

    pub fn build_body_package_with_cache_status(&self) -> Result<IfcBodyPackageLoad, VelrIfcError> {
        if let Some(package) = load_cached_body_package_from_layout(&self.layout)? {
            return Ok(IfcBodyPackageLoad {
                package,
                cache_status: IfcBodyPackageCacheStatus::Hit,
            });
        }

        let package = self.build_body_package_uncached()?;
        write_cached_body_package(&self.layout, &package)?;
        Ok(IfcBodyPackageLoad {
            package,
            cache_status: IfcBodyPackageCacheStatus::Miss,
        })
    }

    pub fn body_geometry_summary(&self) -> Result<IfcBodyGeometrySummary, VelrIfcError> {
        Ok(self
            .build_body_package_with_cache_status()?
            .geometry_summary())
    }

    pub fn body_geometry_summary_with_cache_status(
        &self,
    ) -> Result<(IfcBodyGeometrySummary, IfcBodyPackageCacheStatus), VelrIfcError> {
        let load = self.build_body_package_with_cache_status()?;
        Ok((load.geometry_summary(), load.cache_status))
    }

    pub fn body_instance_summaries(&self) -> Result<Vec<IfcBodyInstanceSummary>, VelrIfcError> {
        Ok(self
            .build_body_package_with_cache_status()?
            .instance_summaries())
    }

    fn build_body_package_uncached(&self) -> Result<PreparedGeometryPackage, VelrIfcError> {
        GeometryBackend::default()
            .build_imported_scene_package(self.extract_body_scene_resource()?)
            .map_err(VelrIfcError::from)
    }

    pub fn placement_summary(&self) -> Result<IfcPlacementSummary, VelrIfcError> {
        let placements = self.query_local_placement_records()?;
        Ok(IfcPlacementSummary {
            local_placements: placements.len(),
            placements_with_relative_placement: placements
                .iter()
                .filter(|placement| placement.relative_location.is_some())
                .count(),
            placements_missing_relative_placement: placements
                .iter()
                .filter(|placement| placement.relative_location.is_none())
                .count(),
            placements_with_parent: placements
                .iter()
                .filter(|placement| placement.parent_placement_id.is_some())
                .count(),
        })
    }

    pub fn extract_body_scene_resource(
        &self,
    ) -> Result<ImportedGeometrySceneResource, VelrIfcError> {
        let placement_transforms = self.resolve_local_placement_transforms()?;
        let mut records = self.query_body_triangulated_records()?;
        records.extend(self.query_body_extruded_records()?);
        records.retain(|record| !is_non_render_helper_body(record));
        imported_scene_resource_from_body_records(records, &placement_transforms)
    }

    fn query_body_triangulated_records(&self) -> Result<Vec<IfcBodyRecord>, VelrIfcError> {
        let rows = self.execute_cypher_rows(
            r#"
MATCH (p:IfcProduct)-[:REPRESENTATION]->(:IfcProductDefinitionShape)-[:REPRESENTATIONS]->(rep:IfcShapeRepresentation)-[:ITEMS]->(item:IfcTriangulatedFaceSet)-[:COORDINATES]->(pl:IfcCartesianPointList3D)
OPTIONAL MATCH (p)-[:OBJECT_PLACEMENT]->(placement:IfcLocalPlacement)
OPTIONAL MATCH (item)<-[:ITEM]-(styled:IfcStyledItem)
OPTIONAL MATCH (styled)-[:STYLES]->(surface_style:IfcSurfaceStyle)
OPTIONAL MATCH (surface_style)-[:STYLES]->(rendering:IfcSurfaceStyleRendering)
OPTIONAL MATCH (rendering)-[:SURFACE_COLOUR]->(rgb:IfcColourRgb)
WHERE rep.RepresentationIdentifier = 'Body'
WITH p, placement, item, pl,
     head(collect(DISTINCT { red: rgb.Red, green: rgb.Green, blue: rgb.Blue })) AS surface_rgb
RETURN id(p) AS product_id, id(placement) AS placement_id, id(item) AS item_id, p.GlobalId AS global_id, p.Name AS name, p.declared_entity AS declared_entity, pl.CoordList AS coord_list, item.CoordIndex AS coord_index, surface_rgb.red AS style_red, surface_rgb.green AS style_green, surface_rgb.blue AS style_blue
ORDER BY item_id
"#,
        )?;

        rows.rows
            .into_iter()
            .map(|row| {
                let product_id = parse_u64_cell(row.first(), "product_id")?;
                let placement_id = parse_optional_u64_cell(row.get(1), "placement_id")?;
                let item_id = parse_u64_cell(row.get(2), "item_id")?;
                let global_id = parse_optional_string_cell(row.get(3));
                let name = parse_optional_string_cell(row.get(4));
                let display_color =
                    parse_optional_display_color_cells(row.get(8), row.get(9), row.get(10))?;
                let declared_entity =
                    parse_required_string_cell(row.get(5), "declared_entity")?.to_string();
                let primitive = GeometryPrimitive::Tessellated(tessellated_geometry_from_row(
                    parse_required_string_cell(row.get(6), "coord_list")?,
                    parse_required_string_cell(row.get(7), "coord_index")?,
                )?);

                Ok(IfcBodyRecord {
                    product_id,
                    placement_id,
                    item_id,
                    global_id,
                    name,
                    display_color,
                    declared_entity,
                    item_transform: DMat4::IDENTITY,
                    primitive,
                })
            })
            .collect()
    }

    fn query_body_extruded_records(&self) -> Result<Vec<IfcBodyRecord>, VelrIfcError> {
        let rows = self.execute_cypher_rows(
            r#"
MATCH (p:IfcProduct)-[:REPRESENTATION]->(:IfcProductDefinitionShape)-[:REPRESENTATIONS]->(rep:IfcShapeRepresentation)-[:ITEMS]->(solid:IfcExtrudedAreaSolid)-[:SWEPT_AREA]->(profile:IfcArbitraryClosedProfileDef)-[:OUTER_CURVE]->(poly:IfcPolyline)-[edge:POINTS]->(pt:IfcCartesianPoint)
MATCH (solid)-[:EXTRUDED_DIRECTION]->(dir:IfcDirection)
OPTIONAL MATCH (p)-[:OBJECT_PLACEMENT]->(placement:IfcLocalPlacement)
OPTIONAL MATCH (solid)-[:POSITION]->(solid_position:IfcAxis2Placement3D)
OPTIONAL MATCH (solid_position)-[:LOCATION]->(solid_location:IfcCartesianPoint)
OPTIONAL MATCH (solid_position)-[:AXIS]->(solid_axis:IfcDirection)
OPTIONAL MATCH (solid_position)-[:REF_DIRECTION]->(solid_ref_direction:IfcDirection)
OPTIONAL MATCH (solid)<-[:ITEM]-(styled:IfcStyledItem)
OPTIONAL MATCH (styled)-[:STYLES]->(surface_style:IfcSurfaceStyle)
OPTIONAL MATCH (surface_style)-[:STYLES]->(rendering:IfcSurfaceStyleRendering)
OPTIONAL MATCH (rendering)-[:SURFACE_COLOUR]->(rgb:IfcColourRgb)
WHERE rep.RepresentationIdentifier = 'Body'
WITH p, placement, solid, dir, solid_location, solid_axis, solid_ref_direction,
     collect(DISTINCT { ordinal: edge.ordinal, coordinates: pt.Coordinates }) AS point_rows,
     head(collect(DISTINCT { red: rgb.Red, green: rgb.Green, blue: rgb.Blue })) AS surface_rgb
RETURN id(p) AS product_id, id(placement) AS placement_id, id(solid) AS item_id, p.GlobalId AS global_id, p.Name AS name, p.declared_entity AS declared_entity, solid.Depth AS depth, dir.DirectionRatios AS extruded_direction, point_rows, solid_location.Coordinates AS solid_position_location, solid_axis.DirectionRatios AS solid_position_axis, solid_ref_direction.DirectionRatios AS solid_position_ref_direction, surface_rgb.red AS style_red, surface_rgb.green AS style_green, surface_rgb.blue AS style_blue
ORDER BY item_id
"#,
        )?;

        rows.rows
            .into_iter()
            .map(|row| {
                let product_id = parse_u64_cell(row.first(), "product_id")?;
                let placement_id = parse_optional_u64_cell(row.get(1), "placement_id")?;
                let item_id = parse_u64_cell(row.get(2), "item_id")?;
                let global_id = parse_optional_string_cell(row.get(3));
                let name = parse_optional_string_cell(row.get(4));
                let item_transform =
                    parse_optional_axis2_placement3d_cells(row.get(9), row.get(10), row.get(11))?
                        .unwrap_or(DMat4::IDENTITY);
                let display_color =
                    parse_optional_display_color_cells(row.get(12), row.get(13), row.get(14))?;
                let declared_entity =
                    parse_required_string_cell(row.get(5), "declared_entity")?.to_string();
                let depth = parse_f64_cell(row.get(6), "depth")?;
                let extruded_direction = parse_direction3_json(parse_required_string_cell(
                    row.get(7),
                    "extruded_direction",
                )?)?;
                let primitive = GeometryPrimitive::SweptSolid(swept_solid_from_row(
                    parse_required_string_cell(row.get(8), "point_rows")?,
                    extruded_direction * depth,
                )?);

                Ok(IfcBodyRecord {
                    product_id,
                    placement_id,
                    item_id,
                    global_id,
                    name,
                    display_color,
                    declared_entity,
                    item_transform,
                    primitive,
                })
            })
            .collect()
    }

    fn query_local_placement_records(&self) -> Result<Vec<IfcLocalPlacementRecord>, VelrIfcError> {
        let rows = self.execute_cypher_rows(
            r#"
MATCH (lp:IfcLocalPlacement)
OPTIONAL MATCH (lp)-[:PLACEMENT_REL_TO]->(parent:IfcLocalPlacement)
OPTIONAL MATCH (lp)-[:RELATIVE_PLACEMENT]->(relative:IfcAxis2Placement3D)
OPTIONAL MATCH (relative)-[:LOCATION]->(location:IfcCartesianPoint)
OPTIONAL MATCH (relative)-[:AXIS]->(axis:IfcDirection)
OPTIONAL MATCH (relative)-[:REF_DIRECTION]->(ref_direction:IfcDirection)
RETURN id(lp) AS placement_id, id(parent) AS parent_placement_id, id(relative) AS relative_placement_id, location.Coordinates AS location, axis.DirectionRatios AS axis, ref_direction.DirectionRatios AS ref_direction
ORDER BY placement_id
"#,
        )?;

        rows.rows
            .into_iter()
            .map(|row| {
                let placement_id = parse_u64_cell(row.first(), "placement_id")?;
                let parent_placement_id =
                    parse_optional_u64_cell(row.get(1), "parent_placement_id")?;
                let relative_location = parse_optional_dvec3_cell(row.get(3), "location")?;
                let axis = parse_optional_dvec3_cell(row.get(4), "axis")?;
                let ref_direction = parse_optional_dvec3_cell(row.get(5), "ref_direction")?;

                Ok(IfcLocalPlacementRecord {
                    placement_id,
                    parent_placement_id,
                    relative_location,
                    axis,
                    ref_direction,
                })
            })
            .collect()
    }

    fn resolve_local_placement_transforms(&self) -> Result<HashMap<u64, DMat4>, VelrIfcError> {
        let placements = self.query_local_placement_records()?;
        let by_id = placements
            .into_iter()
            .map(|placement| (placement.placement_id, placement))
            .collect::<HashMap<_, _>>();
        let mut resolved = HashMap::with_capacity(by_id.len());
        let mut visiting = HashSet::new();

        for placement_id in by_id.keys().copied().collect::<Vec<_>>() {
            resolve_local_placement_transform(placement_id, &by_id, &mut resolved, &mut visiting)?;
        }

        Ok(resolved)
    }
}

impl VelrIfcModel {
    fn graphql_executor(&self) -> Result<ManagedGraphQlExecutor, VelrIfcError> {
        let runtime_layout = self.layout.graphql_runtime_layout()?;
        runtime_layout.validate_graphql_inputs()?;

        let database_path = path_to_utf8(&self.layout.database)?;
        let graphql_sdl = strip_embedded_velr_prelude(&read_text(&runtime_layout.runtime_graphql)?);
        let graphql_server = Arc::new(GraphQlServer::new(&graphql_sdl)?);
        let managed_db = ManagedGraphQlDatabase::open(database_path.as_str())?;
        Ok(managed_db.bind(graphql_server))
    }
}

pub fn curated_fixture_specs() -> &'static [IfcFixtureSpec] {
    CURATED_IFC_FIXTURES
}

pub fn curated_fixture_by_slug(slug: &str) -> Option<IfcFixtureSpec> {
    CURATED_IFC_FIXTURES
        .iter()
        .copied()
        .find(|fixture| fixture.slug == slug)
}

pub fn workspace_root() -> PathBuf {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    path.canonicalize().unwrap_or(path)
}

pub fn default_ifc_fixtures_root() -> PathBuf {
    workspace_root().join("fixtures/ifc")
}

pub fn default_ifc_artifacts_root() -> PathBuf {
    workspace_root().join("artifacts/ifc")
}

pub const IFC_BODY_RESOURCE_PREFIX: &str = "ifc/";

pub fn ifc_body_resource_name(model_slug: &str) -> String {
    format!("{IFC_BODY_RESOURCE_PREFIX}{model_slug}")
}

pub fn parse_ifc_body_resource(resource: &str) -> Option<&str> {
    let suffix = resource.strip_prefix(IFC_BODY_RESOURCE_PREFIX)?;
    if suffix.is_empty() {
        return None;
    }

    if let Some(model_slug) = suffix.strip_suffix("/body") {
        if model_slug.is_empty() || model_slug.contains('/') {
            return None;
        }
        return Some(model_slug);
    }

    if suffix.contains('/') {
        return None;
    }

    Some(suffix)
}

pub fn available_ifc_body_resources(
    artifacts_root: impl AsRef<Path>,
) -> Result<Vec<String>, VelrIfcError> {
    let artifacts_root = artifacts_root.as_ref();
    let entries = match fs::read_dir(artifacts_root) {
        Ok(entries) => entries,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(VelrIfcError::Io {
                path: artifacts_root.to_path_buf(),
                source,
            });
        }
    };

    let mut resources = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| VelrIfcError::Io {
            path: artifacts_root.to_path_buf(),
            source,
        })?;
        let file_type = entry.file_type().map_err(|source| VelrIfcError::Io {
            path: entry.path(),
            source,
        })?;
        if !file_type.is_dir() {
            continue;
        }

        let model_slug = entry.file_name();
        let model_slug = model_slug
            .to_str()
            .ok_or_else(|| VelrIfcError::NonUtf8Path(entry.path()))?;
        let layout = IfcArtifactLayout::new(artifacts_root, model_slug);
        if layout.database.exists() {
            resources.push(ifc_body_resource_name(model_slug));
        }
    }

    resources.sort();
    Ok(resources)
}

pub fn default_velr_ifc_checkout() -> PathBuf {
    workspace_root().join("../../../velr/codex/velr-ifc")
}

pub fn sync_curated_fixtures(
    velr_ifc_root: impl AsRef<Path>,
    fixtures_root: impl AsRef<Path>,
) -> Result<Vec<IfcFixtureSyncResult>, VelrIfcError> {
    let velr_ifc_root = velr_ifc_root.as_ref();
    let fixtures_root = fixtures_root.as_ref();
    create_dir_all(fixtures_root)?;

    let mut results = Vec::with_capacity(CURATED_IFC_FIXTURES.len());
    for fixture in CURATED_IFC_FIXTURES {
        let source = fixture.velr_ifc_source_path(velr_ifc_root);
        let destination = fixtures_root.join(fixture.file_name);
        let bytes = copy_file(&source, &destination)?;
        results.push(IfcFixtureSyncResult {
            slug: fixture.slug.to_string(),
            source,
            destination,
            bytes,
        });
    }

    Ok(results)
}

pub fn clear_ifc_model_artifacts(
    artifacts_root: impl AsRef<Path>,
    model_slug: &str,
) -> Result<bool, VelrIfcError> {
    let model_root = IfcArtifactLayout::new(artifacts_root, model_slug).model_root;
    if !model_root.exists() {
        return Ok(false);
    }
    clear_model_root(&model_root)?;
    Ok(true)
}

pub fn clear_all_ifc_model_artifacts(
    artifacts_root: impl AsRef<Path>,
) -> Result<usize, VelrIfcError> {
    clear_ifc_roots_by(artifacts_root, |layout| layout.model_root.clone())
}

pub fn clear_ifc_geometry_cache(
    artifacts_root: impl AsRef<Path>,
    model_slug: &str,
) -> Result<bool, VelrIfcError> {
    let geometry_dir = IfcArtifactLayout::new(artifacts_root, model_slug).geometry_dir;
    if !geometry_dir.exists() {
        return Ok(false);
    }
    clear_model_root(&geometry_dir)?;
    Ok(true)
}

pub fn clear_all_ifc_geometry_caches(
    artifacts_root: impl AsRef<Path>,
) -> Result<usize, VelrIfcError> {
    clear_ifc_roots_by(artifacts_root, |layout| layout.geometry_dir.clone())
}

pub fn clear_ifc_legacy_runtime_sidecars(
    artifacts_root: impl AsRef<Path>,
    model_slug: &str,
) -> Result<bool, VelrIfcError> {
    let runtime_dir = IfcArtifactLayout::new(artifacts_root, model_slug)
        .model_root
        .join("runtime");
    if !runtime_dir.exists() {
        return Ok(false);
    }
    clear_model_root(&runtime_dir)?;
    Ok(true)
}

pub fn clear_all_ifc_legacy_runtime_sidecars(
    artifacts_root: impl AsRef<Path>,
) -> Result<usize, VelrIfcError> {
    clear_ifc_roots_by(artifacts_root, |layout| layout.model_root.join("runtime"))
}

pub fn import_curated_fixture(
    fixture_slug: &str,
    options: &IfcImportOptions,
) -> Result<IfcImportSummary, VelrIfcError> {
    let fixture = curated_fixture_by_slug(fixture_slug)
        .ok_or_else(|| VelrIfcError::UnknownFixture(fixture_slug.to_string()))?;
    let fixture_path = fixture.repo_fixture_path(&workspace_root());
    require_exists(&fixture_path)?;
    import_ifc_file(&fixture_path, fixture.slug, options)
}

pub fn import_ifc_file(
    step_input: impl AsRef<Path>,
    model_slug: impl Into<String>,
    options: &IfcImportOptions,
) -> Result<IfcImportSummary, VelrIfcError> {
    let step_input = step_input.as_ref();
    let model_slug = model_slug.into();
    let layout = IfcArtifactLayout::new(&options.artifacts_root, &model_slug);
    if !options.replace_existing && existing_import_matches_input(step_input, &layout)? {
        return existing_import_summary(layout, true);
    }
    if options.replace_existing || layout.model_root.exists() {
        clear_model_root(&layout.model_root)?;
    }
    layout.ensure_dirs()?;

    copy_file(step_input, &layout.source_ifc)?;

    let mut command = Command::new("cargo");
    command.current_dir(&options.velr_ifc_root);
    command.arg("run");
    if options.release {
        command.arg("--release");
    }
    command.args(["-p", "ifc-schema-tool", "--", "import-step-into-velr"]);
    command.arg(step_input);
    command.arg(&layout.database);

    let rendered_command = render_command(&command);
    let output = command.output().map_err(|source| VelrIfcError::CommandIo {
        command: rendered_command.clone(),
        source,
    })?;
    write_command_log(&layout.import_log, &output)?;

    if !output.status.success() {
        return Err(VelrIfcError::ImportCommandFailed {
            command: rendered_command,
            status: output.status.code().unwrap_or(-1),
            log_path: layout.import_log.clone(),
        });
    }

    let import_output_dir = options
        .velr_ifc_root
        .join("generated/step-import")
        .join(stem_or_default(step_input, "input"));
    copy_optional_file(
        import_output_dir.join("import-bundle.json"),
        &layout.import_bundle,
    )?;
    copy_optional_file(
        import_output_dir.join("import.cypher"),
        &layout.import_cypher,
    )?;
    copy_optional_file(import_output_dir.join("issues.json"), &layout.import_issues)?;
    copy_required_file(
        import_output_dir.join("import-timing.json"),
        &layout.import_timing,
    )?;

    let imported_schema = layout.authoritative_schema()?;

    Ok(IfcImportSummary {
        model_slug,
        model_root: layout.model_root,
        source_ifc: layout.source_ifc,
        database: layout.database,
        schema: imported_schema,
        import_timing: layout.import_timing,
        import_log: layout.import_log,
        reused_existing: false,
    })
}

pub fn refresh_ifc_schema_runtime_sidecars(
    artifacts_root: impl AsRef<Path>,
    schema: IfcSchemaId,
    velr_ifc_root: impl AsRef<Path>,
) -> Result<IfcRuntimeRefreshSummary, VelrIfcError> {
    let runtime_layout = IfcSchemaGraphQlLayout::new(artifacts_root, schema.clone())?;
    runtime_layout.ensure_dirs()?;
    copy_schema_runtime_bundle(velr_ifc_root.as_ref(), &runtime_layout)?;

    Ok(IfcRuntimeRefreshSummary {
        schema,
        root: runtime_layout.root,
        runtime_graphql: runtime_layout.runtime_graphql,
        runtime_mapping: runtime_layout.runtime_mapping,
        runtime_manifest: runtime_layout.runtime_manifest,
    })
}

pub fn refresh_ifc_runtime_sidecars(
    artifacts_root: impl AsRef<Path>,
    model_slug: impl Into<String>,
    velr_ifc_root: impl AsRef<Path>,
) -> Result<IfcRuntimeRefreshSummary, VelrIfcError> {
    let model_slug = model_slug.into();
    let layout = IfcArtifactLayout::new(artifacts_root, &model_slug);
    layout.validate_cypher_inputs()?;
    let schema = layout.authoritative_schema()?;
    refresh_ifc_schema_runtime_sidecars(
        layout.model_root.parent().ok_or_else(|| {
            VelrIfcError::IfcGeometryData(format!(
                "model `{}` has no artifacts root parent",
                model_slug
            ))
        })?,
        schema,
        velr_ifc_root,
    )
}

pub fn slugify_model_name(input: &str) -> String {
    let mut slug = String::with_capacity(input.len());
    let mut last_was_dash = false;

    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_was_dash = false;
        } else if !last_was_dash {
            slug.push('-');
            last_was_dash = true;
        }
    }

    slug.trim_matches('-').to_string()
}

fn ifc_body_source_space() -> SourceSpace {
    SourceSpace::new(CoordinateFrame::w_world(), LengthUnit::Millimeter)
}

fn is_non_render_helper_body(record: &IfcBodyRecord) -> bool {
    match record.declared_entity.as_str() {
        // Keep spaces and zones in Velr for semantic queries, but do not include
        // them in the default physical render package.
        "IfcSpace" | "IfcSpatialZone" => true,
        "IfcBuildingElementProxy" => matches!(
            record.name.as_deref().map(|name| name.trim().to_ascii_lowercase()),
            Some(name) if name == "origin" || name == "geo-reference"
        ),
        _ => false,
    }
}

fn parse_optional_string_cell(cell: Option<&String>) -> Option<String> {
    let value = cell?.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn summarize_body_package(package: &PreparedGeometryPackage) -> IfcBodyGeometrySummary {
    IfcBodyGeometrySummary {
        definitions: package.definition_count(),
        elements: package.element_count(),
        instances: package.instance_count(),
        triangles: package
            .definitions
            .iter()
            .map(|definition| definition.mesh.triangle_count())
            .sum(),
    }
}

fn summarize_body_instances(package: &PreparedGeometryPackage) -> Vec<IfcBodyInstanceSummary> {
    let mut instances = package
        .instances
        .iter()
        .map(|instance| IfcBodyInstanceSummary {
            instance_id: instance.id.0,
            definition_id: instance.definition_id.0,
            external_id: instance.external_id.as_str().to_string(),
            label: instance.label.clone(),
            display_color: instance.display_color,
            bounds_min: instance.bounds.min,
            bounds_max: instance.bounds.max,
            bounds_center: instance.bounds.center(),
            bounds_size: instance.bounds.size(),
        })
        .collect::<Vec<_>>();

    instances.sort_by(|left, right| {
        left.bounds_center
            .x
            .total_cmp(&right.bounds_center.x)
            .then_with(|| left.bounds_center.y.total_cmp(&right.bounds_center.y))
            .then_with(|| left.bounds_center.z.total_cmp(&right.bounds_center.z))
    });

    instances
}

fn imported_scene_resource_from_body_records(
    mut records: Vec<IfcBodyRecord>,
    placement_transforms: &HashMap<u64, DMat4>,
) -> Result<ImportedGeometrySceneResource, VelrIfcError> {
    if records.is_empty() {
        return Err(VelrIfcError::NoBodyGeometry);
    }

    records.sort_by(|left, right| {
        left.item_id
            .cmp(&right.item_id)
            .then_with(|| left.product_id.cmp(&right.product_id))
            .then_with(|| {
                left.placement_id
                    .unwrap_or_default()
                    .cmp(&right.placement_id.unwrap_or_default())
            })
            .then_with(|| left.global_id.cmp(&right.global_id))
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.declared_entity.cmp(&right.declared_entity))
    });

    let mut definitions = Vec::new();
    let mut primitive_by_item = HashMap::<u64, GeometryPrimitive>::new();

    for record in &records {
        if let Some(existing) = primitive_by_item.get(&record.item_id) {
            if existing != &record.primitive {
                return Err(VelrIfcError::IfcGeometryData(format!(
                    "body item {} resolved to inconsistent primitive payloads",
                    record.item_id
                )));
            }
            continue;
        }

        primitive_by_item.insert(record.item_id, record.primitive.clone());
        definitions.push(GeometryDefinition {
            id: GeometryDefinitionId(record.item_id),
            primitive: record.primitive.clone(),
        });
    }

    let instances = records
        .iter()
        .enumerate()
        .map(
            |(instance_index, record)| ImportedGeometryResourceInstance {
                instance: GeometryInstance {
                    id: GeometryInstanceId((instance_index as u64) + 1),
                    definition_id: GeometryDefinitionId(record.item_id),
                    transform: record
                        .placement_id
                        .and_then(|placement_id| placement_transforms.get(&placement_id))
                        .copied()
                        .unwrap_or(DMat4::IDENTITY)
                        * record.item_transform,
                },
                element_id: ifc_element_id_for_record(record),
                external_id: ExternalId::new(if let Some(global_id) = &record.global_id {
                    format!("{global_id}/item/{}", record.item_id)
                } else {
                    format!("{}/item/{}", record.product_id, record.item_id)
                }),
                label: record
                    .name
                    .clone()
                    .unwrap_or_else(|| record.declared_entity.clone()),
                declared_entity: record.declared_entity.clone(),
                default_render_class: default_render_class_for_ifc_entity(&record.declared_entity),
                display_color: record.display_color,
            },
        )
        .collect();

    Ok(ImportedGeometrySceneResource {
        definitions,
        instances,
        source_space: ifc_body_source_space(),
    })
}

fn ifc_element_id_for_record(record: &IfcBodyRecord) -> SemanticElementId {
    if let Some(global_id) = &record.global_id {
        SemanticElementId::new(global_id.clone())
    } else {
        SemanticElementId::new(record.product_id.to_string())
    }
}

fn default_render_class_for_ifc_entity(declared_entity: &str) -> DefaultRenderClass {
    match declared_entity {
        "IfcSpace" => DefaultRenderClass::Space,
        "IfcSpatialZone" => DefaultRenderClass::Zone,
        "IfcBuildingElementProxy" => DefaultRenderClass::Helper,
        _ => DefaultRenderClass::Physical,
    }
}

fn cached_render_class_name(class: DefaultRenderClass) -> &'static str {
    match class {
        DefaultRenderClass::Physical => "physical",
        DefaultRenderClass::Space => "space",
        DefaultRenderClass::Zone => "zone",
        DefaultRenderClass::Helper => "helper",
        DefaultRenderClass::Other => "other",
    }
}

fn parse_cached_render_class(value: &str) -> DefaultRenderClass {
    match value {
        "physical" => DefaultRenderClass::Physical,
        "space" => DefaultRenderClass::Space,
        "zone" => DefaultRenderClass::Zone,
        "helper" => DefaultRenderClass::Helper,
        "other" => DefaultRenderClass::Other,
        _ => DefaultRenderClass::Other,
    }
}

fn parse_required_string_cell<'a>(
    cell: Option<&'a String>,
    label: &'static str,
) -> Result<&'a str, VelrIfcError> {
    let value = cell
        .map(|cell| cell.trim())
        .filter(|cell| !cell.is_empty())
        .ok_or_else(|| VelrIfcError::IfcGeometryData(format!("missing `{label}` column value")))?;
    Ok(value)
}

fn parse_u64_cell(cell: Option<&String>, label: &'static str) -> Result<u64, VelrIfcError> {
    let value = parse_required_string_cell(cell, label)?;
    value.parse().map_err(|_| {
        VelrIfcError::IfcGeometryData(format!("failed to parse `{label}` as u64: {value}"))
    })
}

fn parse_optional_u64_cell(
    cell: Option<&String>,
    label: &'static str,
) -> Result<Option<u64>, VelrIfcError> {
    let Some(value) = parse_optional_string_cell(cell) else {
        return Ok(None);
    };
    value.parse().map(Some).map_err(|_| {
        VelrIfcError::IfcGeometryData(format!("failed to parse `{label}` as u64: {value}"))
    })
}

fn parse_optional_f32_cell(
    cell: Option<&String>,
    label: &'static str,
) -> Result<Option<f32>, VelrIfcError> {
    let Some(value) = parse_optional_string_cell(cell) else {
        return Ok(None);
    };
    value.parse().map(Some).map_err(|_| {
        VelrIfcError::IfcGeometryData(format!("failed to parse `{label}` as f32: {value}"))
    })
}

fn parse_f64_cell(cell: Option<&String>, label: &'static str) -> Result<f64, VelrIfcError> {
    let value = parse_required_string_cell(cell, label)?;
    value.parse().map_err(|_| {
        VelrIfcError::IfcGeometryData(format!("failed to parse `{label}` as f64: {value}"))
    })
}

fn parse_optional_display_color_cells(
    red: Option<&String>,
    green: Option<&String>,
    blue: Option<&String>,
) -> Result<Option<DisplayColor>, VelrIfcError> {
    let red = parse_optional_f32_cell(red, "style_red")?;
    let green = parse_optional_f32_cell(green, "style_green")?;
    let blue = parse_optional_f32_cell(blue, "style_blue")?;

    match (red, green, blue) {
        (Some(red), Some(green), Some(blue)) => Ok(Some(DisplayColor::new(
            red.clamp(0.0, 1.0),
            green.clamp(0.0, 1.0),
            blue.clamp(0.0, 1.0),
        ))),
        (None, None, None) => Ok(None),
        _ => Ok(None),
    }
}

fn parse_optional_dvec3_cell(
    cell: Option<&String>,
    label: &'static str,
) -> Result<Option<DVec3>, VelrIfcError> {
    let Some(value) = parse_optional_string_cell(cell) else {
        return Ok(None);
    };
    let json = parse_json_value(&value, label)?;
    parse_dvec3_json(&json, label).map(Some)
}

fn parse_optional_axis2_placement3d_cells(
    location: Option<&String>,
    axis: Option<&String>,
    ref_direction: Option<&String>,
) -> Result<Option<DMat4>, VelrIfcError> {
    let location = parse_optional_dvec3_cell(location, "solid_position_location")?;
    let axis = parse_optional_dvec3_cell(axis, "solid_position_axis")?;
    let ref_direction = parse_optional_dvec3_cell(ref_direction, "solid_position_ref_direction")?;

    if location.is_none() && axis.is_none() && ref_direction.is_none() {
        return Ok(None);
    }

    Ok(Some(axis2_placement_transform(
        location.unwrap_or(DVec3::ZERO),
        axis,
        ref_direction,
    )))
}

fn tessellated_geometry_from_row(
    coord_list: &str,
    coord_index: &str,
) -> Result<TessellatedGeometry, VelrIfcError> {
    let positions = parse_dvec3_rows_json(coord_list, "coord_list")?;
    let faces_value = parse_json_value(coord_index, "coord_index")?;
    let face_rows = json_array(&faces_value, "coord_index")?;
    let mut faces = Vec::with_capacity(face_rows.len());

    for (face_index, face_row) in face_rows.iter().enumerate() {
        let face_label = format!("coord_index[{face_index}]");
        let exterior = normalize_index_ring(parse_index_ring_json(face_row, &face_label)?);
        if exterior.len() < 3 {
            continue;
        }
        faces.push(IndexedPolygon::new(exterior, vec![], positions.len())?);
    }

    TessellatedGeometry::new(positions, faces).map_err(VelrIfcError::from)
}

fn parse_direction3_json(text: &str) -> Result<DVec3, VelrIfcError> {
    let value = parse_json_value(text, "extruded_direction")?;
    parse_dvec3_json(&value, "extruded_direction")
}

fn swept_solid_from_row(point_rows: &str, vector: DVec3) -> Result<SweptSolid, VelrIfcError> {
    let outer = ProfileLoop2::new(polycurve2_from_ordered_points(parse_ordered_point_rows(
        point_rows,
        "point_rows",
    )?)?)?;
    let profile = Profile2::new(outer, vec![]);
    SweptSolid::new(profile, SweepPath::Linear { vector }).map_err(VelrIfcError::from)
}

fn polycurve2_from_ordered_points(mut points: Vec<DVec2>) -> Result<Polycurve2, VelrIfcError> {
    if points.len() >= 2
        && points
            .first()
            .unwrap()
            .distance_squared(*points.last().unwrap())
            <= 1e-12
    {
        points.pop();
    }

    if points.len() < 3 {
        return Err(VelrIfcError::IfcGeometryData(
            "profile polyline must contain at least three distinct points".to_string(),
        ));
    }

    let mut segments = Vec::with_capacity(points.len());
    for pair in points.windows(2) {
        segments.push(CurveSegment2::Line(LineSegment2 {
            start: pair[0],
            end: pair[1],
        }));
    }
    segments.push(CurveSegment2::Line(LineSegment2 {
        start: *points
            .last()
            .expect("point count was validated above for profile reconstruction"),
        end: points[0],
    }));

    Polycurve2::new(segments).map_err(VelrIfcError::from)
}

fn parse_ordered_point_rows(text: &str, label: &str) -> Result<Vec<DVec2>, VelrIfcError> {
    let value = parse_json_value(text, label)?;
    let rows = json_array(&value, label)?;
    let mut points = Vec::with_capacity(rows.len());

    for (row_index, row) in rows.iter().enumerate() {
        let row_label = format!("{label}[{row_index}]");
        let object = row.as_object().ok_or_else(|| {
            VelrIfcError::IfcGeometryData(format!("expected `{row_label}` to be a JSON object"))
        })?;
        let ordinal = json_u64(
            object.get("ordinal").ok_or_else(|| {
                VelrIfcError::IfcGeometryData(format!("missing `{row_label}.ordinal`"))
            })?,
            &format!("{row_label}.ordinal"),
        )?;
        let coordinates = parse_dvec2_json(
            object.get("coordinates").ok_or_else(|| {
                VelrIfcError::IfcGeometryData(format!("missing `{row_label}.coordinates`"))
            })?,
            &format!("{row_label}.coordinates"),
        )?;
        points.push((ordinal, coordinates));
    }

    points.sort_by_key(|(ordinal, _)| *ordinal);
    Ok(points.into_iter().map(|(_, point)| point).collect())
}

fn parse_dvec3_rows_json(text: &str, label: &str) -> Result<Vec<DVec3>, VelrIfcError> {
    let value = parse_json_value(text, label)?;
    let rows = json_array(&value, label)?;
    rows.iter()
        .enumerate()
        .map(|(index, row)| parse_dvec3_json(row, &format!("{label}[{index}]")))
        .collect()
}

fn parse_dvec3_json(value: &JsonValue, label: &str) -> Result<DVec3, VelrIfcError> {
    let row = json_array(value, label)?;
    if row.len() < 3 {
        return Err(VelrIfcError::IfcGeometryData(format!(
            "`{label}` must contain at least three coordinates"
        )));
    }

    Ok(DVec3::new(
        json_f64(&row[0], &format!("{label}[0]"))?,
        json_f64(&row[1], &format!("{label}[1]"))?,
        json_f64(&row[2], &format!("{label}[2]"))?,
    ))
}

fn parse_dvec2_json(value: &JsonValue, label: &str) -> Result<DVec2, VelrIfcError> {
    let row = json_array(value, label)?;
    if row.len() < 2 {
        return Err(VelrIfcError::IfcGeometryData(format!(
            "`{label}` must contain at least two coordinates"
        )));
    }

    Ok(DVec2::new(
        json_f64(&row[0], &format!("{label}[0]"))?,
        json_f64(&row[1], &format!("{label}[1]"))?,
    ))
}

fn parse_index_ring_json(value: &JsonValue, label: &str) -> Result<Vec<u32>, VelrIfcError> {
    let ring = json_array(value, label)?;
    let mut indices = Vec::with_capacity(ring.len());

    for (index, entry) in ring.iter().enumerate() {
        let raw = json_u64(entry, &format!("{label}[{index}]"))?;
        let zero_based = raw.checked_sub(1).ok_or_else(|| {
            VelrIfcError::IfcGeometryData(format!(
                "`{label}[{index}]` must use positive 1-based IFC indices"
            ))
        })?;
        indices.push(u32::try_from(zero_based).map_err(|_| {
            VelrIfcError::IfcGeometryData(format!(
                "`{label}[{index}]` exceeds u32 mesh index range: {zero_based}"
            ))
        })?);
    }

    Ok(indices)
}

fn normalize_index_ring(indices: Vec<u32>) -> Vec<u32> {
    let mut normalized = Vec::with_capacity(indices.len());

    for index in indices {
        if normalized.last().copied() == Some(index) {
            continue;
        }
        normalized.push(index);
    }

    if normalized.len() >= 2 && normalized.first() == normalized.last() {
        normalized.pop();
    }

    normalized
}

fn parse_json_value(text: &str, label: &str) -> Result<JsonValue, VelrIfcError> {
    serde_json::from_str(text).map_err(|error| {
        VelrIfcError::IfcGeometryData(format!("failed to parse `{label}` JSON payload: {error}"))
    })
}

fn json_array<'a>(value: &'a JsonValue, label: &str) -> Result<&'a [JsonValue], VelrIfcError> {
    value.as_array().map(Vec::as_slice).ok_or_else(|| {
        VelrIfcError::IfcGeometryData(format!("expected `{label}` to be a JSON array"))
    })
}

fn json_f64(value: &JsonValue, label: &str) -> Result<f64, VelrIfcError> {
    value.as_f64().ok_or_else(|| {
        VelrIfcError::IfcGeometryData(format!("expected `{label}` to be a JSON number"))
    })
}

fn json_u64(value: &JsonValue, label: &str) -> Result<u64, VelrIfcError> {
    if let Some(value) = value.as_u64() {
        return Ok(value);
    }
    if let Some(value) = value.as_i64().filter(|value| *value >= 0) {
        return Ok(value as u64);
    }
    if let Some(value) = value
        .as_f64()
        .filter(|value| value.is_finite() && *value >= 0.0 && value.fract() == 0.0)
    {
        return Ok(value as u64);
    }

    Err(VelrIfcError::IfcGeometryData(format!(
        "expected `{label}` to be a non-negative integer"
    )))
}

fn resolve_local_placement_transform(
    placement_id: u64,
    by_id: &HashMap<u64, IfcLocalPlacementRecord>,
    resolved: &mut HashMap<u64, DMat4>,
    visiting: &mut HashSet<u64>,
) -> Result<DMat4, VelrIfcError> {
    if let Some(transform) = resolved.get(&placement_id) {
        return Ok(*transform);
    }

    let placement = by_id.get(&placement_id).ok_or_else(|| {
        VelrIfcError::IfcGeometryData(format!(
            "referenced placement `{placement_id}` was not returned by the placement query"
        ))
    })?;

    if !visiting.insert(placement_id) {
        return Err(VelrIfcError::IfcGeometryData(format!(
            "cycle detected while resolving IfcLocalPlacement chain at `{placement_id}`"
        )));
    }

    let local_from_parent = axis2_placement_transform(
        placement.relative_location.unwrap_or(DVec3::ZERO),
        placement.axis,
        placement.ref_direction,
    );
    let world_from_local = if let Some(parent_placement_id) = placement.parent_placement_id {
        let parent_world =
            resolve_local_placement_transform(parent_placement_id, by_id, resolved, visiting)?;
        parent_world * local_from_parent
    } else {
        local_from_parent
    };

    visiting.remove(&placement_id);
    resolved.insert(placement_id, world_from_local);
    Ok(world_from_local)
}

fn axis2_placement_transform(
    location: DVec3,
    axis: Option<DVec3>,
    ref_direction: Option<DVec3>,
) -> DMat4 {
    let z_axis = normalized_or(axis.unwrap_or(DVec3::Z), DVec3::Z);
    let x_hint = normalized_or(ref_direction.unwrap_or(DVec3::X), DVec3::X);
    let mut x_axis = x_hint - z_axis * z_axis.dot(x_hint);
    if x_axis.length_squared() <= 1.0e-12 {
        x_axis = orthogonal_axis(z_axis);
    } else {
        x_axis = x_axis.normalize();
    }
    let y_axis = z_axis.cross(x_axis).normalize();

    DMat4::from_cols(
        x_axis.extend(0.0),
        y_axis.extend(0.0),
        z_axis.extend(0.0),
        location.extend(1.0),
    )
}

fn normalized_or(vector: DVec3, fallback: DVec3) -> DVec3 {
    if vector.length_squared() <= 1.0e-12 {
        fallback
    } else {
        vector.normalize()
    }
}

fn orthogonal_axis(axis: DVec3) -> DVec3 {
    let helper = if axis.z.abs() < 0.9 {
        DVec3::Z
    } else {
        DVec3::X
    };
    helper.cross(axis).normalize()
}

fn scalar_count(raw_db: &Velr, cypher: &str) -> Result<i64, VelrIfcError> {
    exec_cypher_in_scoped_tx(raw_db, cypher, |table| {
        let mut count = None;
        table.for_each_row(|row| {
            if let Some(CellRef::Integer(value)) = row.first() {
                count = Some(*value);
            }
            Ok(())
        })?;
        count.ok_or_else(|| VelrIfcError::MissingIntegerResult(cypher.to_string()))
    })
}

// Ad hoc Cypher queries run against cached model handles in the web server, so we keep each
// execution inside a short-lived transaction and close it explicitly even when the query fails.
fn exec_cypher_in_scoped_tx<T>(
    raw_db: &Velr,
    cypher: &str,
    extract: impl FnOnce(&mut TableResult) -> Result<T, VelrIfcError>,
) -> Result<T, VelrIfcError> {
    let tx = raw_db.begin_tx()?;
    let result = match tx.exec_one(cypher) {
        Ok(mut table) => {
            let extracted = extract(&mut table);
            drop(table);
            extracted
        }
        Err(query_error) => {
            return match tx.rollback() {
                Ok(()) => Err(VelrIfcError::from(query_error)),
                Err(rollback_error) => Err(VelrIfcError::IfcGeometryData(format!(
                    "cypher query failed and transaction rollback cleanup also failed: query error: {query_error}; rollback error: {rollback_error}"
                ))),
            };
        }
    };

    match tx.rollback() {
        Ok(()) => result,
        Err(rollback_error) => match result {
            Ok(_) => Err(VelrIfcError::from(rollback_error)),
            Err(query_error) => Err(VelrIfcError::IfcGeometryData(format!(
                "cypher query processing failed and transaction rollback cleanup also failed: query error: {query_error}; rollback error: {rollback_error}"
            ))),
        },
    }
}

fn render_cell(cell: &CellRef<'_>) -> String {
    match cell {
        CellRef::Null => String::new(),
        CellRef::Bool(value) => value.to_string(),
        CellRef::Integer(value) => value.to_string(),
        CellRef::Float(value) => value.to_string(),
        CellRef::Text(bytes) | CellRef::Json(bytes) => String::from_utf8_lossy(bytes).into_owned(),
    }
}

fn strip_embedded_velr_prelude(sdl: &str) -> String {
    let mut stripped = Vec::new();
    let mut lines = sdl.lines().peekable();

    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        let is_velr_enum = matches!(
            trimmed,
            "enum VelrDirection {"
                | "enum VelrLabelMatch {"
                | "enum VelrPropertyCodec {"
                | "enum VelrScalarFamily {"
        );

        if is_velr_enum {
            for next_line in lines.by_ref() {
                if next_line.trim() == "}" {
                    break;
                }
            }
            if matches!(lines.peek(), Some(next_line) if next_line.trim().is_empty()) {
                lines.next();
            }
            continue;
        }

        let is_velr_directive = trimmed.starts_with("directive @velrNode(")
            || trimmed.starts_with("directive @velrProperty(")
            || trimmed.starts_with("directive @velrScalar(")
            || trimmed.starts_with("directive @velrRelationship(")
            || trimmed.starts_with("directive @velrCypher(")
            || trimmed.starts_with("directive @velrValueWrapper(")
            || trimmed.starts_with("directive @velrCreate(")
            || trimmed.starts_with("directive @velrUpdate(")
            || trimmed.starts_with("directive @velrDelete(")
            || trimmed.starts_with("directive @velrConnect(")
            || trimmed.starts_with("directive @velrDisconnect(")
            || trimmed.starts_with("directive @velrSetRelationship(")
            || trimmed.starts_with("directive @velrClearRelationship(");

        if !is_velr_directive {
            stripped.push(line);
        }
    }

    let mut normalized = stripped.join("\n");
    if sdl.ends_with('\n') {
        normalized.push('\n');
    }
    normalized
}

fn parse_projects_response(
    response: GraphQlResponse,
) -> Result<Vec<IfcProjectOverview>, VelrIfcError> {
    if !response.errors.is_empty() {
        let rendered = response
            .errors
            .iter()
            .map(|error| format!("{error:?}"))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(VelrIfcError::GraphQlRequestErrors(rendered));
    }

    let data = response.data.ok_or(VelrIfcError::MissingGraphQlData)?;
    let projects = data
        .get("ifcProjectList")
        .ok_or(VelrIfcError::MissingGraphQlField("ifcProjectList"))?
        .as_array()
        .ok_or(VelrIfcError::UnexpectedGraphQlShape("ifcProjectList"))?;

    projects
        .iter()
        .map(|project| {
            let object = project
                .as_object()
                .ok_or(VelrIfcError::UnexpectedGraphQlShape("ifcProjectList[]"))?;
            Ok(IfcProjectOverview {
                id: required_json_string(object, "id")?,
                declared_entity: optional_json_string(object, "declaredEntity")?
                    .or(optional_json_string(object, "__typename")?)
                    .unwrap_or_else(|| "IfcProject".to_string()),
                global_id: None,
                name: None,
                long_name: None,
                phase: None,
            })
        })
        .collect()
}

fn required_json_string(object: &JsonMap, key: &'static str) -> Result<String, VelrIfcError> {
    optional_json_string(object, key)?.ok_or(VelrIfcError::MissingGraphQlField(key))
}

fn optional_json_string(
    object: &JsonMap,
    key: &'static str,
) -> Result<Option<String>, VelrIfcError> {
    let Some(value) = object.get(key) else {
        return Ok(None);
    };

    if value.is_null() {
        return Ok(None);
    }

    value
        .as_str()
        .map(|value| Some(value.to_string()))
        .ok_or(VelrIfcError::UnexpectedGraphQlShape(key))
}

fn stem_or_default(path: &Path, default: &str) -> String {
    path.file_stem()
        .and_then(OsStr::to_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| default.to_string())
}

fn copy_required_file(
    source: impl AsRef<Path>,
    destination: impl AsRef<Path>,
) -> Result<(), VelrIfcError> {
    let source = source.as_ref();
    require_exists(source)?;
    copy_file(source, destination).map(|_| ())
}

fn copy_optional_file(
    source: impl AsRef<Path>,
    destination: impl AsRef<Path>,
) -> Result<(), VelrIfcError> {
    let source = source.as_ref();
    if source.exists() {
        copy_required_file(source, destination)?;
    }
    Ok(())
}

fn clear_model_root(path: &Path) -> Result<(), VelrIfcError> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(VelrIfcError::RemoveDir {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn copy_file(source: impl AsRef<Path>, destination: impl AsRef<Path>) -> Result<u64, VelrIfcError> {
    let source = source.as_ref();
    let destination = destination.as_ref();
    require_exists(source)?;
    if let Some(parent) = destination.parent() {
        create_dir_all(parent)?;
    }
    fs::copy(source, destination).map_err(|source_error| VelrIfcError::Io {
        path: destination.to_path_buf(),
        source: source_error,
    })
}

fn create_dir_all(path: &Path) -> Result<(), VelrIfcError> {
    fs::create_dir_all(path).map_err(|source| VelrIfcError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn read_text(path: &Path) -> Result<String, VelrIfcError> {
    fs::read_to_string(path).map_err(|source| VelrIfcError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn write_text(path: &Path, text: &str) -> Result<(), VelrIfcError> {
    if let Some(parent) = path.parent() {
        create_dir_all(parent)?;
    }
    fs::write(path, text).map_err(|source| VelrIfcError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn read_text_if_exists(path: &Path) -> Result<Option<String>, VelrIfcError> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(VelrIfcError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn read_text_prefix_if_exists(
    path: &Path,
    max_bytes: usize,
) -> Result<Option<String>, VelrIfcError> {
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(VelrIfcError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    let mut buffer = vec![0_u8; max_bytes];
    let read = file.read(&mut buffer).map_err(|source| VelrIfcError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    buffer.truncate(read);

    Ok(Some(String::from_utf8_lossy(&buffer).into_owned()))
}

fn schema_from_import_log_if_exists(path: &Path) -> Result<Option<IfcSchemaId>, VelrIfcError> {
    let Some(text) = read_text_if_exists(path)? else {
        return Ok(None);
    };

    Ok(text.lines().find_map(|line| {
        let value = line.trim().strip_prefix("schema:")?;
        Some(IfcSchemaId::parse(value.trim()))
    }))
}

fn schema_from_import_bundle_if_exists(path: &Path) -> Result<Option<IfcSchemaId>, VelrIfcError> {
    let Some(text) = read_text_if_exists(path)? else {
        return Ok(None);
    };
    let value = parse_json_value(&text, "import_bundle")?;
    let Some(schema) = value
        .as_object()
        .and_then(|object| object.get("schema"))
        .and_then(JsonValue::as_str)
    else {
        return Ok(None);
    };

    Ok(Some(IfcSchemaId::parse(schema)))
}

fn schema_from_source_ifc_if_exists(path: &Path) -> Result<Option<IfcSchemaId>, VelrIfcError> {
    let Some(text) = read_text_prefix_if_exists(path, 8 * 1024)? else {
        return Ok(None);
    };
    let upper = text.to_ascii_uppercase();

    if upper.contains("IFC4X3_ADD2") || upper.contains("IFC4X3ADD2") {
        return Ok(Some(IfcSchemaId::Ifc4x3Add2));
    }
    if upper.contains("IFC2X3") {
        return Ok(Some(IfcSchemaId::Ifc2x3Tc1));
    }
    if upper.contains("IFC4") {
        return Ok(Some(IfcSchemaId::Ifc4));
    }

    Ok(None)
}

fn schema_from_runtime_mapping_if_exists(path: &Path) -> Result<Option<IfcSchemaId>, VelrIfcError> {
    let Some(text) = read_text_if_exists(path)? else {
        return Ok(None);
    };
    let value = parse_json_value(&text, "runtime_mapping")?;
    let Some(schema) = value
        .as_object()
        .and_then(|object| object.get("schema"))
        .and_then(JsonValue::as_str)
    else {
        return Ok(None);
    };

    Ok(Some(IfcSchemaId::parse(schema)))
}

fn schema_from_runtime_manifest_if_exists(
    path: &Path,
) -> Result<Option<IfcSchemaId>, VelrIfcError> {
    let Some(text) = read_text_if_exists(path)? else {
        return Ok(None);
    };
    let Ok(value) = serde_json::from_str::<JsonValue>(&text) else {
        return Ok(None);
    };
    let Some(schema) = value
        .as_object()
        .and_then(|object| object.get("schema"))
        .and_then(JsonValue::as_str)
    else {
        return Ok(None);
    };

    Ok(Some(IfcSchemaId::parse(schema)))
}

fn copy_schema_runtime_bundle(
    velr_ifc_root: &Path,
    runtime_layout: &IfcSchemaGraphQlLayout,
) -> Result<(), VelrIfcError> {
    let stem = runtime_layout
        .schema
        .generated_artifact_stem()
        .ok_or_else(|| {
            VelrIfcError::IfcGeometryData(format!(
                "no shared GraphQL runtime bundle is configured for `{}`",
                runtime_layout.schema.canonical_name()
            ))
        })?;
    let bundle_root = velr_ifc_root.join("generated/velr-graphql-test").join(stem);

    copy_required_file(
        bundle_root.join("ifc-runtime.graphql"),
        &runtime_layout.runtime_graphql,
    )?;
    copy_required_file(
        bundle_root.join("ifc-runtime.mapping.json"),
        &runtime_layout.runtime_mapping,
    )?;
    copy_required_file(
        bundle_root.join("handoff-manifest.json"),
        &runtime_layout.runtime_manifest,
    )?;
    copy_optional_file(
        bundle_root.join("feature-queries.graphql"),
        &runtime_layout.feature_queries,
    )?;

    Ok(())
}

fn cache_file_path(layout: &IfcArtifactLayout) -> PathBuf {
    layout.geometry_dir.join(BODY_PACKAGE_CACHE_FILE)
}

fn existing_import_summary(
    layout: IfcArtifactLayout,
    reused_existing: bool,
) -> Result<IfcImportSummary, VelrIfcError> {
    let schema = layout.authoritative_schema()?;
    Ok(IfcImportSummary {
        model_slug: layout.model_slug,
        model_root: layout.model_root,
        source_ifc: layout.source_ifc,
        database: layout.database,
        schema,
        import_timing: layout.import_timing,
        import_log: layout.import_log,
        reused_existing,
    })
}

fn existing_import_matches_input(
    step_input: &Path,
    layout: &IfcArtifactLayout,
) -> Result<bool, VelrIfcError> {
    for required in [
        &layout.source_ifc,
        &layout.database,
        &layout.import_timing,
        &layout.import_log,
    ] {
        if !required.exists() {
            return Ok(false);
        }
    }

    if !files_match(step_input, &layout.source_ifc)? {
        return Ok(false);
    }

    Ok(true)
}

fn files_match(left: &Path, right: &Path) -> Result<bool, VelrIfcError> {
    let left_metadata = fs::metadata(left).map_err(|source| VelrIfcError::Io {
        path: left.to_path_buf(),
        source,
    })?;
    let right_metadata = fs::metadata(right).map_err(|source| VelrIfcError::Io {
        path: right.to_path_buf(),
        source,
    })?;

    if left_metadata.len() != right_metadata.len() {
        return Ok(false);
    }

    let mut left_file = fs::File::open(left).map_err(|source| VelrIfcError::Io {
        path: left.to_path_buf(),
        source,
    })?;
    let mut right_file = fs::File::open(right).map_err(|source| VelrIfcError::Io {
        path: right.to_path_buf(),
        source,
    })?;
    let mut left_buffer = [0_u8; 8 * 1024];
    let mut right_buffer = [0_u8; 8 * 1024];

    loop {
        let left_read = left_file
            .read(&mut left_buffer)
            .map_err(|source| VelrIfcError::Io {
                path: left.to_path_buf(),
                source,
            })?;
        let right_read = right_file
            .read(&mut right_buffer)
            .map_err(|source| VelrIfcError::Io {
                path: right.to_path_buf(),
                source,
            })?;

        if left_read != right_read {
            return Ok(false);
        }
        if left_read == 0 {
            return Ok(true);
        }
        if left_buffer[..left_read] != right_buffer[..right_read] {
            return Ok(false);
        }
    }
}

fn file_fingerprint(path: &Path) -> Result<CachedFileFingerprint, VelrIfcError> {
    let metadata = fs::metadata(path).map_err(|source| VelrIfcError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let modified = metadata.modified().map_err(|source| VelrIfcError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let duration = modified
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();

    Ok(CachedFileFingerprint {
        bytes: metadata.len(),
        modified_unix_seconds: duration.as_secs(),
        modified_subsec_nanos: duration.subsec_nanos(),
    })
}

fn load_cached_body_package_from_layout(
    layout: &IfcArtifactLayout,
) -> Result<Option<PreparedGeometryPackage>, VelrIfcError> {
    let cache_path = cache_file_path(layout);
    let Some(text) = read_text_if_exists(&cache_path)? else {
        return Ok(None);
    };
    let Ok(cached) = serde_json::from_str::<CachedPreparedGeometryPackage>(&text) else {
        return Ok(None);
    };
    if cached.cache_version != BODY_PACKAGE_CACHE_VERSION {
        return Ok(None);
    }
    if cached.schema != layout.authoritative_schema()? {
        return Ok(None);
    }
    if file_fingerprint(&layout.database)? != cached.database {
        return Ok(None);
    }

    Ok(Some(cached.into_prepared_package()))
}

fn write_cached_body_package(
    layout: &IfcArtifactLayout,
    package: &PreparedGeometryPackage,
) -> Result<(), VelrIfcError> {
    let cache = CachedPreparedGeometryPackage::from_prepared_package(
        layout.authoritative_schema()?,
        file_fingerprint(&layout.database)?,
        package,
    );
    let text = serde_json::to_string_pretty(&cache).map_err(|source| {
        VelrIfcError::IfcGeometryData(format!(
            "failed to serialize prepared package cache: {source}"
        ))
    })?;
    write_text(&cache_file_path(layout), &text)
}

fn clear_ifc_roots_by<F>(
    artifacts_root: impl AsRef<Path>,
    root_for_layout: F,
) -> Result<usize, VelrIfcError>
where
    F: Fn(&IfcArtifactLayout) -> PathBuf,
{
    let artifacts_root = artifacts_root.as_ref();
    let entries = match fs::read_dir(artifacts_root) {
        Ok(entries) => entries,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(source) => {
            return Err(VelrIfcError::Io {
                path: artifacts_root.to_path_buf(),
                source,
            });
        }
    };

    let mut cleared = 0;
    for entry in entries {
        let entry = entry.map_err(|source| VelrIfcError::Io {
            path: artifacts_root.to_path_buf(),
            source,
        })?;
        let file_type = entry.file_type().map_err(|source| VelrIfcError::Io {
            path: entry.path(),
            source,
        })?;
        if !file_type.is_dir() {
            continue;
        }

        let model_slug = entry
            .file_name()
            .to_str()
            .ok_or_else(|| VelrIfcError::NonUtf8Path(entry.path()))?
            .to_string();
        let layout = IfcArtifactLayout::new(artifacts_root, model_slug);
        let root = root_for_layout(&layout);
        if root.exists() {
            clear_model_root(&root)?;
            cleared += 1;
        }
    }

    Ok(cleared)
}

fn require_exists(path: &Path) -> Result<(), VelrIfcError> {
    if path.exists() {
        Ok(())
    } else {
        Err(VelrIfcError::MissingRequiredPath(path.to_path_buf()))
    }
}

fn write_command_log(path: &Path, output: &Output) -> Result<(), VelrIfcError> {
    let mut log = String::new();
    log.push_str("# stdout\n");
    log.push_str(&String::from_utf8_lossy(&output.stdout));
    if !log.ends_with('\n') {
        log.push('\n');
    }
    log.push_str("\n# stderr\n");
    log.push_str(&String::from_utf8_lossy(&output.stderr));
    if !log.ends_with('\n') {
        log.push('\n');
    }
    write_text(path, &log)
}

fn path_to_utf8(path: &Path) -> Result<String, VelrIfcError> {
    path.to_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| VelrIfcError::NonUtf8Path(path.to_path_buf()))
}

fn render_command(command: &Command) -> String {
    let program = command.get_program().to_string_lossy().into_owned();
    let args = command
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ");
    if args.is_empty() {
        program
    } else {
        format!("{program} {args}")
    }
}

#[derive(Debug, Error)]
pub enum VelrIfcError {
    #[error("unknown curated IFC fixture `{0}`")]
    UnknownFixture(String),
    #[error("imported IFC model does not expose any supported Body geometry yet")]
    NoBodyGeometry,
    #[error("required path does not exist: {0}")]
    MissingRequiredPath(PathBuf),
    #[error("path is not valid UTF-8: {0}")]
    NonUtf8Path(PathBuf),
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to remove directory {path}: {source}")]
    RemoveDir {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to launch external command `{command}`: {source}")]
    CommandIo {
        command: String,
        #[source]
        source: io::Error,
    },
    #[error("external IFC import command failed with status {status}: {command}; see {log_path}")]
    ImportCommandFailed {
        command: String,
        status: i32,
        log_path: PathBuf,
    },
    #[error("GraphQL response contained errors: {0}")]
    GraphQlRequestErrors(String),
    #[error("GraphQL response did not contain data")]
    MissingGraphQlData,
    #[error("GraphQL response did not contain `{0}`")]
    MissingGraphQlField(&'static str),
    #[error("GraphQL response shape for `{0}` did not match expectations")]
    UnexpectedGraphQlShape(&'static str),
    #[error("query did not return an integer result: {0}")]
    MissingIntegerResult(String),
    #[error("IFC body geometry data was malformed or outside the current extractor support: {0}")]
    IfcGeometryData(String),
    #[error(transparent)]
    Geometry(#[from] cc_w_types::GeometryError),
    #[error(transparent)]
    Backend(#[from] GeometryBackendError),
    #[error(transparent)]
    Velr(#[from] velr::Error),
    #[error(transparent)]
    GraphQlCore(#[from] velr_graphql_core::Error),
    #[error(transparent)]
    GraphQlManaged(#[from] velr_graphql_managed::ManagedError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};
    use velr_graphql_core::JsonValue;

    #[test]
    fn curated_fixture_catalog_matches_expected_local_corpus() {
        let slugs = curated_fixture_specs()
            .iter()
            .map(|fixture| fixture.slug)
            .collect::<Vec<_>>();

        assert_eq!(
            slugs,
            vec![
                "building-architecture",
                "building-hvac",
                "building-landscaping",
                "building-structural",
                "infra-bridge",
                "infra-landscaping",
                "infra-plumbing",
                "infra-rail",
                "infra-road",
                "openifcmodel-20210219-architecture",
                "fzk-haus",
            ]
        );
    }

    #[test]
    fn ifc_body_resource_parser_accepts_canonical_and_body_forms() {
        assert_eq!(
            parse_ifc_body_resource("ifc/building-architecture"),
            Some("building-architecture")
        );
        assert_eq!(
            parse_ifc_body_resource("ifc/building-architecture/body"),
            Some("building-architecture")
        );
        assert_eq!(parse_ifc_body_resource("ifc/"), None);
        assert_eq!(
            parse_ifc_body_resource("ifc/building-architecture/mesh"),
            None
        );
        assert_eq!(parse_ifc_body_resource("demo/pentagon"), None);
    }

    #[test]
    fn helper_proxy_filter_skips_origin_and_geo_reference_markers() {
        let helper = |name: &str| IfcBodyRecord {
            product_id: 1,
            placement_id: None,
            item_id: 1,
            global_id: None,
            name: Some(name.to_string()),
            display_color: None,
            declared_entity: "IfcBuildingElementProxy".to_string(),
            item_transform: DMat4::IDENTITY,
            primitive: GeometryPrimitive::Tessellated(
                TessellatedGeometry::new(
                    vec![
                        DVec3::ZERO,
                        DVec3::new(1.0, 0.0, 0.0),
                        DVec3::new(0.0, 1.0, 0.0),
                    ],
                    vec![IndexedPolygon::new(vec![0, 1, 2], vec![], 3).expect("triangle")],
                )
                .expect("geometry"),
            ),
        };

        assert!(is_non_render_helper_body(&helper("origin")));
        assert!(is_non_render_helper_body(&helper("geo-reference")));
        assert!(!is_non_render_helper_body(&helper("real proxy")));
    }

    #[test]
    fn spatial_semantic_bodies_are_hidden_from_default_render_package() {
        let semantic_body = |declared_entity: &str| IfcBodyRecord {
            product_id: 1,
            placement_id: None,
            item_id: 1,
            global_id: None,
            name: Some("semantic volume".to_string()),
            display_color: None,
            declared_entity: declared_entity.to_string(),
            item_transform: DMat4::IDENTITY,
            primitive: GeometryPrimitive::Tessellated(
                TessellatedGeometry::new(
                    vec![
                        DVec3::ZERO,
                        DVec3::new(1.0, 0.0, 0.0),
                        DVec3::new(0.0, 1.0, 0.0),
                    ],
                    vec![IndexedPolygon::new(vec![0, 1, 2], vec![], 3).expect("triangle")],
                )
                .expect("geometry"),
            ),
        };

        assert!(is_non_render_helper_body(&semantic_body("IfcSpace")));
        assert!(is_non_render_helper_body(&semantic_body("IfcSpatialZone")));
        assert!(!is_non_render_helper_body(&semantic_body("IfcWall")));
    }

    #[test]
    fn artifact_layout_matches_documented_shape() {
        let layout = IfcArtifactLayout::new("/tmp/ifc-artifacts", "building-architecture");
        let graphql_layout =
            IfcSchemaGraphQlLayout::new("/tmp/ifc-artifacts", IfcSchemaId::Ifc4x3Add2)
                .expect("graphql layout");

        assert_eq!(
            layout.model_root,
            PathBuf::from("/tmp/ifc-artifacts/building-architecture")
        );
        assert_eq!(
            layout.database,
            PathBuf::from("/tmp/ifc-artifacts/building-architecture/model.velr.db")
        );
        assert_eq!(
            graphql_layout.root,
            PathBuf::from("/tmp/ifc-artifacts/_graphql/ifc4x3_add2")
        );
        assert_eq!(
            graphql_layout.runtime_graphql,
            PathBuf::from("/tmp/ifc-artifacts/_graphql/ifc4x3_add2/ifc-runtime.graphql")
        );
        assert_eq!(
            layout.import_timing,
            PathBuf::from("/tmp/ifc-artifacts/building-architecture/import/import-timing.json")
        );
    }

    #[test]
    fn cypher_validation_only_requires_database_artifact() {
        let temp_root = temp_test_root("cypher-validation");
        let layout = IfcArtifactLayout::new(&temp_root, "building-architecture");
        layout.ensure_dirs().expect("layout dirs");
        fs::write(&layout.database, b"velr-db").expect("database");

        layout
            .validate_cypher_inputs()
            .expect("cypher validation should accept database-only setup");
        assert!(layout.validate_graphql_inputs().is_err());
        assert!(layout.validate_model_inputs().is_err());

        fs::remove_dir_all(&temp_root).expect("cleanup temp dir");
    }

    #[test]
    fn project_response_parser_accepts_expected_shape() {
        let mut project = JsonMap::new();
        project.insert(
            "id".to_string(),
            JsonValue::String("node-1".to_string().into()),
        );
        project.insert(
            "declaredEntity".to_string(),
            JsonValue::String("IfcProject".to_string().into()),
        );
        project.insert(
            "GlobalId".to_string(),
            JsonValue::String("3hX2$abc".to_string().into()),
        );
        project.insert(
            "Name".to_string(),
            JsonValue::String("Building Demo".to_string().into()),
        );
        project.insert("LongName".to_string(), JsonValue::Null);
        project.insert(
            "Phase".to_string(),
            JsonValue::String("Design".to_string().into()),
        );

        let mut data = JsonMap::new();
        data.insert(
            "ifcProjectList".to_string(),
            JsonValue::Array(vec![JsonValue::Object(project)]),
        );

        let projects = parse_projects_response(GraphQlResponse {
            errors: Vec::new(),
            data: Some(data),
        })
        .expect("projects");

        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].declared_entity, "IfcProject");
        assert_eq!(projects[0].name, None);
        assert_eq!(projects[0].phase, None);
    }

    #[test]
    fn slugify_model_name_normalizes_paths() {
        assert_eq!(
            slugify_model_name("20201030AC20-FZK Haus.ifc"),
            "20201030ac20-fzk-haus-ifc"
        );
        assert_eq!(slugify_model_name(" Building  Hvac "), "building-hvac");
    }

    #[test]
    fn tessellated_geometry_parser_accepts_ifc_point_lists() {
        let geometry = tessellated_geometry_from_row(
            "[[0.0,0.0,0.0],[1000.0,0.0,0.0],[1000.0,1000.0,0.0],[0.0,1000.0,0.0]]",
            "[[1,2,3,4]]",
        )
        .expect("tessellated geometry");

        assert_eq!(geometry.positions.len(), 4);
        assert_eq!(geometry.face_count(), 1);
        assert_eq!(geometry.faces[0].exterior, vec![0, 1, 2, 3]);
    }

    #[test]
    fn tessellated_geometry_parser_preserves_faces_for_kernel_culling_policy() {
        let geometry = tessellated_geometry_from_row(
            "[[0.0,0.0,0.0],[1000.0,0.0,0.0],[2000.0,0.0,0.0],[0.0,1000.0,0.0]]",
            "[[1,2,3],[1,2,4]]",
        )
        .expect("tessellated geometry");

        assert_eq!(geometry.face_count(), 2);
        assert_eq!(geometry.faces[0].exterior, vec![0, 1, 2]);
        assert_eq!(geometry.faces[1].exterior, vec![0, 1, 3]);
    }

    #[test]
    fn tessellated_geometry_parser_normalizes_duplicate_ring_indices() {
        let geometry = tessellated_geometry_from_row(
            "[[0.0,0.0,0.0],[1000.0,0.0,0.0],[0.0,1000.0,0.0]]",
            "[[1,2,2,3,1]]",
        )
        .expect("tessellated geometry");

        assert_eq!(geometry.face_count(), 1);
        assert_eq!(geometry.faces[0].exterior, vec![0, 1, 2]);
    }

    #[test]
    fn tessellated_geometry_parser_keeps_small_faces_for_kernel_policy() {
        let geometry = tessellated_geometry_from_row(
            "[[-603.1602021555037,294.5945945946744,49.999999998914426],[-610.2356451318109,281.3344594595385,49.99999999894908],[-606.8471046702699,286.5920608108901,49.999999998937525]]",
            "[[1,2,3]]",
        )
        .expect("tessellated geometry");

        assert_eq!(geometry.face_count(), 1);
        assert_eq!(geometry.faces[0].exterior, vec![0, 1, 2]);
    }

    #[test]
    fn swept_solid_parser_accepts_ordered_profile_points() {
        let solid = swept_solid_from_row(
            r#"[{"ordinal":2,"coordinates":[1000.0,0.0]},{"ordinal":1,"coordinates":[0.0,0.0]},{"ordinal":4,"coordinates":[0.0,500.0]},{"ordinal":3,"coordinates":[1000.0,500.0]}]"#,
            DVec3::new(0.0, 0.0, 2200.0),
        )
        .expect("swept solid");

        assert_eq!(solid.profile.outer.curve.segments.len(), 4);
        match solid.path {
            SweepPath::Linear { vector } => {
                assert_eq!(vector, DVec3::new(0.0, 0.0, 2200.0));
            }
            _ => panic!("expected linear sweep path"),
        }
    }

    #[test]
    fn axis2_placement_transform_defaults_to_identity_basis() {
        let transform = axis2_placement_transform(DVec3::new(12.0, -4.0, 2.5), None, None);

        assert_eq!(
            transform.transform_point3(DVec3::ZERO),
            DVec3::new(12.0, -4.0, 2.5)
        );
        assert_eq!(transform.transform_vector3(DVec3::X), DVec3::X);
        assert_eq!(transform.transform_vector3(DVec3::Y), DVec3::Y);
        assert_eq!(transform.transform_vector3(DVec3::Z), DVec3::Z);
    }

    #[test]
    fn placement_chain_composes_parent_and_child_transforms() {
        let by_id = HashMap::from([
            (
                1_u64,
                IfcLocalPlacementRecord {
                    placement_id: 1,
                    parent_placement_id: None,
                    relative_location: Some(DVec3::new(10.0, 0.0, 0.0)),
                    axis: None,
                    ref_direction: None,
                },
            ),
            (
                2_u64,
                IfcLocalPlacementRecord {
                    placement_id: 2,
                    parent_placement_id: Some(1),
                    relative_location: Some(DVec3::new(0.0, 5.0, 0.0)),
                    axis: None,
                    ref_direction: None,
                },
            ),
        ]);
        let mut resolved = HashMap::new();
        let mut visiting = HashSet::new();

        let transform = resolve_local_placement_transform(2, &by_id, &mut resolved, &mut visiting)
            .expect("placement transform");

        assert_eq!(
            transform.transform_point3(DVec3::ZERO),
            DVec3::new(10.0, 5.0, 0.0)
        );
    }

    #[test]
    fn cached_prepared_package_roundtrips() {
        let package = sample_prepared_package();
        let fingerprint = CachedFileFingerprint {
            bytes: 123,
            modified_unix_seconds: 456,
            modified_subsec_nanos: 789,
        };

        let cached = CachedPreparedGeometryPackage::from_prepared_package(
            IfcSchemaId::Ifc2x3Tc1,
            fingerprint.clone(),
            &package,
        );

        assert_eq!(cached.cache_version, BODY_PACKAGE_CACHE_VERSION);
        assert_eq!(cached.schema, IfcSchemaId::Ifc2x3Tc1);
        assert_eq!(cached.database, fingerprint);
        assert_eq!(cached.clone().into_prepared_package(), package);
    }

    #[test]
    fn files_match_detects_identical_and_changed_files() {
        let temp_root = std::env::temp_dir().join(format!(
            "cc-w-velr-files-match-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        fs::create_dir_all(&temp_root).expect("temp dir");
        let left = temp_root.join("left.ifc");
        let right = temp_root.join("right.ifc");

        fs::write(&left, b"same-bytes").expect("left file");
        fs::write(&right, b"same-bytes").expect("right file");
        assert!(files_match(&left, &right).expect("matching files"));

        fs::write(&right, b"different-bytes").expect("updated right file");
        assert!(!files_match(&left, &right).expect("different files"));

        fs::remove_dir_all(&temp_root).expect("cleanup temp dir");
    }

    #[test]
    fn cached_body_package_invalidates_when_database_changes() {
        let temp_root = temp_test_root("cache-invalidation");
        let layout = IfcArtifactLayout::new(&temp_root, "building-architecture");
        layout.ensure_dirs().expect("layout dirs");
        fs::write(&layout.database, b"velr-v1").expect("database");
        write_test_source_ifc(&layout, "IFC2X3");

        let package = sample_prepared_package();
        write_cached_body_package(&layout, &package).expect("write cache");

        let cached = load_cached_body_package_from_layout(&layout).expect("load cache");
        assert_eq!(cached, Some(package.clone()));

        fs::write(&layout.database, b"velr-v2-with-new-bytes").expect("updated database");
        let invalidated = load_cached_body_package_from_layout(&layout).expect("load cache");
        assert_eq!(invalidated, None);

        fs::remove_dir_all(&temp_root).expect("cleanup temp dir");
    }

    #[test]
    fn public_artifacts_root_loader_reports_cache_hit_when_package_exists_without_graphql_runtime()
    {
        let temp_root = temp_test_root("artifacts-root-hit");
        let layout = IfcArtifactLayout::new(&temp_root, "building-architecture");
        layout.ensure_dirs().expect("layout dirs");
        fs::write(&layout.database, b"velr-db").expect("database");
        write_test_source_ifc(&layout, "IFC2X3");

        let package = sample_prepared_package();
        write_cached_body_package(&layout, &package).expect("write cache");

        let load = VelrIfcModel::load_body_package_with_cache_status_from_artifacts_root(
            &temp_root,
            "building-architecture",
        )
        .expect("body package load");

        assert_eq!(load.cache_status, IfcBodyPackageCacheStatus::Hit);
        assert_eq!(load.package, package);

        fs::remove_dir_all(&temp_root).expect("cleanup temp dir");
    }

    #[test]
    fn existing_import_without_runtime_sidecars_is_still_reusable_for_cypher_rendering() {
        let temp_root = temp_test_root("schema-sidecars-optional");
        let layout = IfcArtifactLayout::new(&temp_root, "building-architecture");
        layout.ensure_dirs().expect("layout dirs");
        fs::write(&layout.database, b"velr-db").expect("database");
        write_test_source_ifc(&layout, "IFC2X3");
        write_text(&layout.import_timing, "{\n  \"total_ms\": 1\n}\n").expect("timing");
        write_text(
            &layout.import_log,
            "# stdout\nschema: IFC2X3_TC1\n\n# stderr\n",
        )
        .expect("import log");

        assert!(
            existing_import_matches_input(&layout.source_ifc, &layout).expect("reuse check"),
            "cypher/render reuse should not require GraphQL runtime sidecars"
        );

        fs::remove_dir_all(&temp_root).expect("cleanup temp dir");
    }

    #[test]
    fn shared_graphql_runtime_schema_is_read_from_schema_root() {
        let temp_root = temp_test_root("shared-graphql-schema");
        let runtime_layout = IfcSchemaGraphQlLayout::new(&temp_root, IfcSchemaId::Ifc2x3Tc1)
            .expect("graphql layout");
        runtime_layout.ensure_dirs().expect("runtime dirs");
        write_text(
            &runtime_layout.runtime_mapping,
            "{\n  \"schema\": \"IFC2X3_TC1\"\n}\n",
        )
        .expect("runtime mapping");

        assert_eq!(
            runtime_layout
                .runtime_bundle_schema()
                .expect("runtime bundle schema"),
            Some(IfcSchemaId::Ifc2x3Tc1)
        );

        fs::remove_dir_all(&temp_root).expect("cleanup temp dir");
    }

    #[test]
    fn available_ifc_body_resources_only_requires_database() {
        let temp_root = temp_test_root("resources-db-only");
        let layout = IfcArtifactLayout::new(&temp_root, "building-architecture");
        layout.ensure_dirs().expect("layout dirs");
        fs::write(&layout.database, b"velr-db").expect("database");

        let resources = available_ifc_body_resources(&temp_root).expect("available resources");
        assert_eq!(
            resources,
            vec![ifc_body_resource_name("building-architecture")]
        );

        fs::remove_dir_all(&temp_root).expect("cleanup temp dir");
    }

    #[test]
    fn clear_all_ifc_legacy_runtime_sidecars_removes_model_runtime_dirs_only() {
        let temp_root = temp_test_root("clear-legacy-runtime");
        let layout = IfcArtifactLayout::new(&temp_root, "building-architecture");
        layout.ensure_dirs().expect("layout dirs");
        let legacy_runtime_dir = layout.model_root.join("runtime");
        fs::create_dir_all(&legacy_runtime_dir).expect("legacy runtime dir");
        write_text(
            &legacy_runtime_dir.join("ifc-runtime.graphql"),
            "type Query { noop: String }\n",
        )
        .expect("legacy runtime file");

        let shared_runtime = IfcSchemaGraphQlLayout::new(&temp_root, IfcSchemaId::Ifc4x3Add2)
            .expect("shared graphql layout");
        shared_runtime.ensure_dirs().expect("shared runtime dir");
        write_text(
            &shared_runtime.runtime_graphql,
            "type Query { noop: String }\n",
        )
        .expect("shared runtime file");

        let cleared = clear_all_ifc_legacy_runtime_sidecars(&temp_root)
            .expect("clear legacy runtime sidecars");

        assert_eq!(cleared, 1);
        assert!(
            !legacy_runtime_dir.exists(),
            "legacy runtime dir should be removed"
        );
        assert!(
            shared_runtime.runtime_graphql.exists(),
            "shared schema runtime should stay intact"
        );

        fs::remove_dir_all(&temp_root).expect("cleanup temp dir");
    }

    #[test]
    fn refresh_runtime_sidecars_uses_existing_schema_without_reimport() {
        let temp_root = temp_test_root("refresh-runtime-sidecars");
        let artifacts_root = temp_root.join("artifacts");
        let velr_ifc_root = temp_root.join("velr-ifc");
        let layout = IfcArtifactLayout::new(&artifacts_root, "openifcmodel");
        layout.ensure_dirs().expect("layout dirs");
        fs::write(&layout.database, b"velr-db").expect("database");
        write_test_source_ifc(&layout, "IFC2X3");

        let bundle_root = velr_ifc_root
            .join("generated/velr-graphql-test")
            .join("ifc2x3_tc1");
        write_text(
            &bundle_root.join("ifc-runtime.graphql"),
            "type Query { noop: String }\nschema { query: Query }\n",
        )
        .expect("runtime graphql bundle");
        write_text(
            &bundle_root.join("ifc-runtime.mapping.json"),
            "{\n  \"schema\": \"IFC2X3_TC1\"\n}\n",
        )
        .expect("runtime mapping bundle");
        write_text(
            &bundle_root.join("handoff-manifest.json"),
            "{\n  \"schema\": \"IFC2X3_TC1\"\n}\n",
        )
        .expect("runtime manifest bundle");

        let refreshed =
            refresh_ifc_runtime_sidecars(&artifacts_root, "openifcmodel", &velr_ifc_root)
                .expect("refresh runtime sidecars");

        assert_eq!(refreshed.schema, IfcSchemaId::Ifc2x3Tc1);
        assert_eq!(
            refreshed.root,
            artifacts_root.join("_graphql").join("ifc2x3_tc1")
        );
        assert_eq!(
            schema_from_runtime_mapping_if_exists(&refreshed.runtime_mapping)
                .expect("runtime schema"),
            Some(IfcSchemaId::Ifc2x3Tc1)
        );

        fs::remove_dir_all(&temp_root).expect("cleanup temp dir");
    }

    #[test]
    fn failed_cypher_query_does_not_poison_reused_model_handle() {
        let temp_root = temp_test_root("cypher-tx-cleanup");
        let layout = IfcArtifactLayout::new(&temp_root, "query-model");
        layout.ensure_dirs().expect("layout dirs");
        write_test_source_ifc(&layout, "IFC2X3");

        {
            let database_path = path_to_utf8(&layout.database).expect("database utf-8");
            let db = Velr::open(Some(database_path.as_str())).expect("open test db");
            db.run("CREATE (:Thing {name:'ok'})").expect("seed db");
        }

        let model = VelrIfcModel::open(layout.clone()).expect("open model");

        model
            .execute_cypher_rows("MATCH (")
            .expect_err("invalid cypher should fail");

        let result = model
            .execute_cypher_rows("MATCH (n:Thing) RETURN count(n) AS count")
            .expect("follow-up query should still succeed");

        assert_eq!(result.columns, vec!["count".to_string()]);
        assert_eq!(result.rows, vec![vec!["1".to_string()]]);

        fs::remove_dir_all(&temp_root).expect("cleanup temp dir");
    }

    #[test]
    fn failed_scalar_count_query_does_not_poison_database_handle() {
        let db = Velr::open(None).expect("open in-memory db");
        db.run("CREATE (:Thing)")
            .expect("seed in-memory test database");

        scalar_count(&db, "MATCH (").expect_err("invalid cypher should fail");

        let count = scalar_count(&db, "MATCH (n:Thing) RETURN count(n)")
            .expect("follow-up scalar query should succeed");

        assert_eq!(count, 1);
    }

    #[test]
    fn shared_item_ids_become_one_definition_with_multiple_instances() {
        let primitive = GeometryPrimitive::Tessellated(
            TessellatedGeometry::new(
                vec![
                    DVec3::ZERO,
                    DVec3::new(1.0, 0.0, 0.0),
                    DVec3::new(0.0, 1.0, 0.0),
                ],
                vec![IndexedPolygon::new(vec![0, 1, 2], vec![], 3).expect("triangle")],
            )
            .expect("geometry"),
        );
        let records = vec![
            IfcBodyRecord {
                product_id: 10,
                placement_id: Some(1),
                item_id: 77,
                global_id: Some("product-a".to_string()),
                name: Some("Shared A".to_string()),
                display_color: Some(DisplayColor::new(0.95, 0.56, 0.24)),
                declared_entity: "IfcBuildingElementProxy".to_string(),
                item_transform: DMat4::from_translation(DVec3::new(1.5, 0.0, 0.0)),
                primitive: primitive.clone(),
            },
            IfcBodyRecord {
                product_id: 20,
                placement_id: Some(2),
                item_id: 77,
                global_id: Some("product-b".to_string()),
                name: Some("Shared B".to_string()),
                display_color: Some(DisplayColor::new(0.24, 0.78, 0.55)),
                declared_entity: "IfcBuildingElementProxy".to_string(),
                item_transform: DMat4::from_translation(DVec3::new(0.0, 2.0, 0.0)),
                primitive,
            },
        ];
        let placement_transforms = HashMap::from([
            (1_u64, DMat4::IDENTITY),
            (2_u64, DMat4::from_translation(DVec3::new(5.0, 0.0, 0.0))),
        ]);

        let scene = imported_scene_resource_from_body_records(records, &placement_transforms)
            .expect("scene");

        assert_eq!(scene.definitions.len(), 1);
        assert_eq!(scene.definitions[0].id, GeometryDefinitionId(77));
        assert_eq!(scene.instances.len(), 2);
        assert_eq!(
            scene.instances[0].instance.definition_id,
            GeometryDefinitionId(77)
        );
        assert_eq!(
            scene.instances[1].instance.definition_id,
            GeometryDefinitionId(77)
        );
        assert_ne!(
            scene.instances[0].instance.id,
            scene.instances[1].instance.id
        );
        assert_ne!(
            scene.instances[0].instance.transform,
            scene.instances[1].instance.transform
        );
        assert_eq!(
            scene.instances[0].instance.transform,
            DMat4::from_translation(DVec3::new(1.5, 0.0, 0.0))
        );
        assert_eq!(
            scene.instances[1].instance.transform,
            DMat4::from_translation(DVec3::new(5.0, 2.0, 0.0))
        );
        assert_eq!(
            scene.instances[0].display_color,
            Some(DisplayColor::new(0.95, 0.56, 0.24))
        );
        assert_eq!(
            scene.instances[1].display_color,
            Some(DisplayColor::new(0.24, 0.78, 0.55))
        );
    }

    #[test]
    fn body_instance_summaries_preserve_display_color() {
        let summaries = summarize_body_instances(&sample_prepared_package());

        assert_eq!(summaries.len(), 1);
        assert_eq!(
            summaries[0].display_color,
            Some(DisplayColor::new(0.25, 0.5, 0.75))
        );
    }

    #[test]
    #[ignore = "manual fixture diagnosis against imported IFC artifacts"]
    fn diagnose_manual_fixture_tessellation_failure() {
        let model =
            std::env::var("CC_W_IFC_DIAG_MODEL").unwrap_or_else(|_| "infra-bridge".to_string());
        let handle =
            VelrIfcModel::open(IfcArtifactLayout::new(default_ifc_artifacts_root(), &model))
                .expect("open imported model");
        let mut records = handle
            .query_body_triangulated_records()
            .expect("triangulated records");
        records.retain(|record| !is_non_render_helper_body(record));

        for record in records {
            let GeometryPrimitive::Tessellated(geometry) = &record.primitive else {
                continue;
            };

            let scene = ImportedGeometrySceneResource {
                definitions: vec![GeometryDefinition {
                    id: GeometryDefinitionId(record.item_id),
                    primitive: record.primitive.clone(),
                }],
                instances: vec![ImportedGeometryResourceInstance {
                    instance: GeometryInstance {
                        id: GeometryInstanceId(1),
                        definition_id: GeometryDefinitionId(record.item_id),
                        transform: DMat4::IDENTITY,
                    },
                    element_id: SemanticElementId::new("diag"),
                    external_id: ExternalId::new("diag"),
                    label: "diag".to_string(),
                    declared_entity: "IfcProduct".to_string(),
                    default_render_class: DefaultRenderClass::Physical,
                    display_color: None,
                }],
                source_space: ifc_body_source_space(),
            };

            if GeometryBackend::default()
                .build_imported_scene_package(scene)
                .is_ok()
            {
                continue;
            }

            println!("failing item_id={}", record.item_id);
            for (face_index, face) in geometry.faces.iter().enumerate() {
                let face_geometry =
                    TessellatedGeometry::new(geometry.positions.clone(), vec![face.clone()])
                        .expect("single face geometry");
                let face_scene = ImportedGeometrySceneResource {
                    definitions: vec![GeometryDefinition {
                        id: GeometryDefinitionId(record.item_id),
                        primitive: GeometryPrimitive::Tessellated(face_geometry),
                    }],
                    instances: vec![ImportedGeometryResourceInstance {
                        instance: GeometryInstance {
                            id: GeometryInstanceId(1),
                            definition_id: GeometryDefinitionId(record.item_id),
                            transform: DMat4::IDENTITY,
                        },
                        element_id: SemanticElementId::new("diag"),
                        external_id: ExternalId::new("diag"),
                        label: "diag".to_string(),
                        declared_entity: "IfcProduct".to_string(),
                        default_render_class: DefaultRenderClass::Physical,
                        display_color: None,
                    }],
                    source_space: ifc_body_source_space(),
                };

                let result = GeometryBackend::default().build_imported_scene_package(face_scene);
                if let Err(error) = result {
                    println!("failing face_index={face_index}");
                    println!("face exterior={:?}", face.exterior);
                    println!(
                        "face positions={:?}",
                        face.exterior
                            .iter()
                            .map(|index| geometry.positions[*index as usize])
                            .collect::<Vec<_>>()
                    );
                    panic!("manual diagnosis: {error}");
                }
            }

            panic!(
                "manual diagnosis: item {} failed as a whole but no individual face reproduced it",
                record.item_id
            );
        }

        panic!("manual diagnosis: no failing tessellated item found for model {model}");
    }

    fn sample_prepared_package() -> PreparedGeometryPackage {
        PreparedGeometryPackage {
            definitions: vec![PreparedGeometryDefinition {
                id: GeometryDefinitionId(7),
                mesh: PreparedMesh {
                    local_origin: DVec3::new(1.0, 2.0, 3.0),
                    bounds: Bounds3 {
                        min: DVec3::new(-1.0, -2.0, -3.0),
                        max: DVec3::new(4.0, 5.0, 6.0),
                    },
                    vertices: vec![
                        PreparedVertex {
                            position: [0.0, 0.0, 0.0],
                            normal: [0.0, 0.0, 1.0],
                        },
                        PreparedVertex {
                            position: [1.0, 0.0, 0.0],
                            normal: [0.0, 0.0, 1.0],
                        },
                        PreparedVertex {
                            position: [0.0, 1.0, 0.0],
                            normal: [0.0, 0.0, 1.0],
                        },
                    ],
                    indices: vec![0, 1, 2],
                },
            }],
            elements: vec![PreparedGeometryElement {
                id: SemanticElementId::new("sample"),
                label: "demo".to_string(),
                declared_entity: "IfcWall".to_string(),
                default_render_class: DefaultRenderClass::Physical,
                bounds: Bounds3 {
                    min: DVec3::new(9.0, 18.0, 27.0),
                    max: DVec3::new(14.0, 25.0, 36.0),
                },
            }],
            instances: vec![PreparedGeometryInstance {
                id: GeometryInstanceId(9),
                element_id: SemanticElementId::new("sample"),
                definition_id: GeometryDefinitionId(7),
                transform: DMat4::from_translation(DVec3::new(10.0, 20.0, 30.0)),
                bounds: Bounds3 {
                    min: DVec3::new(9.0, 18.0, 27.0),
                    max: DVec3::new(14.0, 25.0, 36.0),
                },
                external_id: ExternalId::new("9/item/7"),
                label: "demo".to_string(),
                display_color: Some(DisplayColor::new(0.25, 0.5, 0.75)),
            }],
        }
    }

    fn write_test_source_ifc(layout: &IfcArtifactLayout, schema: &str) {
        write_text(
            &layout.source_ifc,
            &format!(
                "ISO-10303-21;\nHEADER;\nFILE_SCHEMA(('{}'));\nENDSEC;\nDATA;\nENDSEC;\nEND-ISO-10303-21;\n",
                schema
            ),
        )
        .expect("source ifc");
    }

    fn temp_test_root(prefix: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();

        std::env::temp_dir().join(format!("cc-w-velr-{prefix}-{}-{stamp}", std::process::id()))
    }
}
