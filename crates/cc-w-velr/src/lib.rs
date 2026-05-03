use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cc_w_backend::{GeometryBackend, GeometryBackendError};
use cc_w_db::{ImportedGeometryResourceInstance, ImportedGeometrySceneResource};
use cc_w_types::{
    Bounds3, CoordinateFrame, CurveSegment2, DefaultRenderClass, DisplayColor, ExternalId,
    FaceVisibility, GeometryDefinition, GeometryDefinitionId, GeometryInstance, GeometryInstanceId,
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

const BODY_PACKAGE_CACHE_VERSION: u32 = 40;
const BODY_PACKAGE_CACHE_FILE: &str = "prepared-package.json";
const POLYGONAL_FACE_SET_GEOMETRY_QUERY_CHUNK_SIZE: usize = 128;
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
        if let Some(schema) = schema_from_import_log_if_exists(&self.import_log)? {
            return Ok(schema);
        }
        if let Some(schema) = schema_from_source_ifc_if_exists(&self.source_ifc)? {
            return Ok(schema);
        }
        if let Some(schema) = schema_from_import_bundle_if_exists(&self.import_bundle)? {
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IfcBodyPhaseTiming {
    pub name: &'static str,
    pub elapsed_ms: u128,
    pub rows: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IfcBodyBrepDiagnostic {
    pub limit_items: Option<usize>,
    pub geometry_items: usize,
    pub geometry_faces: usize,
    pub geometry_point_rows: usize,
    pub metadata_rows: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IfcBodyPackageDiagnostic {
    pub package: PreparedGeometryPackage,
    pub timings: Vec<IfcBodyPhaseTiming>,
    pub brep: IfcBodyBrepDiagnostic,
    pub cache_written: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IfcBodyPackageCacheDiagnostic {
    pub package: Option<PreparedGeometryPackage>,
    pub cache_status: IfcBodyPackageCacheStatus,
    pub cache_bytes: Option<usize>,
    pub timings: Vec<IfcBodyPhaseTiming>,
}

impl IfcBodyPackageDiagnostic {
    pub fn geometry_summary(&self) -> IfcBodyGeometrySummary {
        summarize_body_package(&self.package)
    }

    pub fn instance_summaries(&self) -> Vec<IfcBodyInstanceSummary> {
        summarize_body_instances(&self.package)
    }
}

impl IfcBodyPackageCacheDiagnostic {
    pub fn geometry_summary(&self) -> Option<IfcBodyGeometrySummary> {
        self.package.as_ref().map(summarize_body_package)
    }
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
    face_visibility: String,
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
            face_visibility: cached_face_visibility_name(instance.face_visibility).to_string(),
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
            face_visibility: parse_cached_face_visibility(&self.face_visibility),
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
    pub face_visibility: FaceVisibility,
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
    occurrence_id: Option<u64>,
    global_id: Option<String>,
    name: Option<String>,
    object_type: Option<String>,
    predefined_type: Option<String>,
    type_object_type: Option<String>,
    type_predefined_type: Option<String>,
    classification_identification: Option<String>,
    display_color: Option<DisplayColor>,
    face_visibility: FaceVisibility,
    declared_entity: String,
    item_transform: DMat4,
    primitive: Arc<GeometryPrimitive>,
}

#[derive(Clone, Debug)]
struct FacetedBrepFaceAccumulator {
    ordinal: Option<u64>,
    points: Vec<FacetedBrepPoint>,
}

#[derive(Clone, Copy, Debug)]
struct FacetedBrepPoint {
    ordinal: Option<u64>,
    sequence: usize,
    coordinates: DVec3,
}

#[derive(Clone, Debug, Default)]
struct FacetedBrepGeometryDiagnostic {
    geometry_items: usize,
    geometry_faces: usize,
    geometry_point_rows: usize,
}

#[derive(Clone, Debug)]
struct FacetedBrepGeometryQuery {
    geometry_by_item: HashMap<u64, Arc<GeometryPrimitive>>,
    diagnostic: FacetedBrepGeometryDiagnostic,
    timings: Vec<IfcBodyPhaseTiming>,
}

#[derive(Clone, Debug)]
struct FacetedBrepRecordQuery {
    records: Vec<IfcBodyRecord>,
    diagnostic: IfcBodyBrepDiagnostic,
    timings: Vec<IfcBodyPhaseTiming>,
}

#[derive(Clone, Debug, PartialEq)]
struct ExtrudedBodyGeometry {
    primitive: Arc<GeometryPrimitive>,
    item_transform: DMat4,
}

#[derive(Clone, Debug)]
struct IfcLocalPlacementRecord {
    placement_id: u64,
    parent: Option<IfcPlacementParent>,
    relative_location: Option<DVec3>,
    axis: Option<DVec3>,
    ref_direction: Option<DVec3>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IfcPlacementParent {
    Local(u64),
    Linear(u64),
}

#[derive(Clone, Debug)]
struct IfcLinearPlacementRecord {
    parent_local_placement_id: Option<u64>,
    curve_id: Option<u64>,
    distance_along: f64,
    offset_longitudinal: f64,
    offset_lateral: f64,
    offset_vertical: f64,
}

#[derive(Clone, Debug)]
struct IfcGradientCurveRecord {
    horizontal_segments: Vec<IfcGradientCurveSegment>,
    vertical_segments: Vec<IfcGradientCurveSegment>,
}

#[derive(Clone, Debug)]
struct IfcSectionedSolidProfile {
    ordinal: u64,
    points: Vec<DVec2>,
}

#[derive(Clone, Debug)]
struct IfcSectionedSolidPosition {
    ordinal: u64,
    curve_entity: String,
    curve_id: u64,
    distance_along: f64,
    offset_lateral: f64,
    offset_vertical: f64,
}

#[derive(Clone, Debug)]
struct IfcSectionedSolidRing {
    positions: Vec<DVec3>,
    tangent: DVec3,
}

#[derive(Clone, Debug)]
struct IfcPolylineDirectrixRecord {
    points: Vec<DVec3>,
}

#[derive(Clone, Copy, Debug)]
struct IfcPolylineDirectrixPoint {
    ordinal: Option<u64>,
    sequence: usize,
    coordinates: DVec3,
}

#[derive(Clone, Debug)]
struct IfcGradientCurveSegment {
    start_station: f64,
    length: f64,
    start_point: DVec2,
    direction: DVec2,
    end_point: Option<DVec2>,
    end_direction: Option<DVec2>,
    kind: IfcGradientCurveSegmentKind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum IfcGradientCurveSegmentKind {
    Line,
    Circular { radius: f64, turn_sign: f64 },
    Clothoid,
}

#[derive(Clone, Debug)]
pub struct IfcImportOptions {
    pub velr_ifc_root: PathBuf,
    pub artifacts_root: PathBuf,
    pub release: bool,
    pub replace_existing: bool,
    pub debug_artifacts: bool,
}

impl Default for IfcImportOptions {
    fn default() -> Self {
        Self {
            velr_ifc_root: default_velr_ifc_checkout(),
            artifacts_root: default_ifc_artifacts_root(),
            release: true,
            replace_existing: false,
            debug_artifacts: true,
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

    pub fn diagnose_body_package_cache_from_artifacts_root(
        artifacts_root: impl AsRef<Path>,
        model_slug: impl Into<String>,
    ) -> Result<IfcBodyPackageCacheDiagnostic, VelrIfcError> {
        let layout = IfcArtifactLayout::new(artifacts_root, model_slug);
        layout.validate_cypher_inputs()?;
        diagnose_cached_body_package_from_layout(&layout)
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

    pub fn build_body_package_diagnostic(
        &self,
        brep_limit_items: Option<usize>,
    ) -> Result<IfcBodyPackageDiagnostic, VelrIfcError> {
        self.build_body_package_diagnostic_with_cache_write(brep_limit_items, false)
    }

    pub fn build_body_package_diagnostic_with_cache_write(
        &self,
        brep_limit_items: Option<usize>,
        write_cache: bool,
    ) -> Result<IfcBodyPackageDiagnostic, VelrIfcError> {
        if write_cache && brep_limit_items.is_some() {
            return Err(VelrIfcError::IfcGeometryData(
                "diagnostic cache write requires a complete body package; remove --limit-brep-items"
                    .to_string(),
            ));
        }

        let mut timings = Vec::new();

        eprintln!("w velr body diagnostic phase triangulated_body_records start");
        let start = Instant::now();
        let mut records = self.query_body_triangulated_records()?;
        eprintln!(
            "w velr body diagnostic phase triangulated_body_records done rows={}",
            records.len()
        );
        timings.push(phase_timing(
            "triangulated_body_records",
            start.elapsed(),
            Some(records.len()),
        ));

        eprintln!("w velr body diagnostic phase terrain_surface_records start");
        let start = Instant::now();
        let terrain_surface_records = self.query_terrain_surface_triangulated_records()?;
        eprintln!(
            "w velr body diagnostic phase terrain_surface_records done rows={}",
            terrain_surface_records.len()
        );
        timings.push(phase_timing(
            "terrain_surface_records",
            start.elapsed(),
            Some(terrain_surface_records.len()),
        ));
        records.extend(terrain_surface_records);

        eprintln!("w velr body diagnostic phase polygonal_face_set_body_records start");
        let start = Instant::now();
        let polygonal_face_set_records = self.query_body_polygonal_face_set_records()?;
        eprintln!(
            "w velr body diagnostic phase polygonal_face_set_body_records done rows={}",
            polygonal_face_set_records.len()
        );
        timings.push(phase_timing(
            "polygonal_face_set_body_records",
            start.elapsed(),
            Some(polygonal_face_set_records.len()),
        ));
        records.extend(polygonal_face_set_records);

        eprintln!("w velr body diagnostic phase extruded_body_records start");
        let start = Instant::now();
        let extruded_records = self.query_body_extruded_records()?;
        eprintln!(
            "w velr body diagnostic phase extruded_body_records done rows={}",
            extruded_records.len()
        );
        timings.push(phase_timing(
            "extruded_body_records",
            start.elapsed(),
            Some(extruded_records.len()),
        ));
        records.extend(extruded_records);

        eprintln!("w velr body diagnostic phase mapped_polygonal_face_set_body_records start");
        let start = Instant::now();
        let mapped_polygonal_face_set_records =
            self.query_body_mapped_polygonal_face_set_records()?;
        eprintln!(
            "w velr body diagnostic phase mapped_polygonal_face_set_body_records done rows={}",
            mapped_polygonal_face_set_records.len()
        );
        timings.push(phase_timing(
            "mapped_polygonal_face_set_body_records",
            start.elapsed(),
            Some(mapped_polygonal_face_set_records.len()),
        ));
        records.extend(mapped_polygonal_face_set_records);

        eprintln!("w velr body diagnostic phase sectioned_solid_horizontal_body_records start");
        let start = Instant::now();
        let sectioned_solid_horizontal_records =
            self.query_body_sectioned_solid_horizontal_records()?;
        eprintln!(
            "w velr body diagnostic phase sectioned_solid_horizontal_body_records done rows={}",
            sectioned_solid_horizontal_records.len()
        );
        timings.push(phase_timing(
            "sectioned_solid_horizontal_body_records",
            start.elapsed(),
            Some(sectioned_solid_horizontal_records.len()),
        ));
        records.extend(sectioned_solid_horizontal_records);

        eprintln!("w velr body diagnostic phase mapped_extruded_body_records start");
        let start = Instant::now();
        let mapped_extruded_records = self.query_body_mapped_extruded_records()?;
        eprintln!(
            "w velr body diagnostic phase mapped_extruded_body_records done rows={}",
            mapped_extruded_records.len()
        );
        timings.push(phase_timing(
            "mapped_extruded_body_records",
            start.elapsed(),
            Some(mapped_extruded_records.len()),
        ));
        records.extend(mapped_extruded_records);

        eprintln!("w velr body diagnostic phase brep_body_records start");
        let brep_query = self.query_body_faceted_brep_records_limited(brep_limit_items)?;
        eprintln!(
            "w velr body diagnostic phase brep_body_records done rows={}",
            brep_query.records.len()
        );
        timings.extend(brep_query.timings);
        records.extend(brep_query.records);

        eprintln!("w velr body diagnostic phase placement_transforms start");
        let start = Instant::now();
        let placement_ids = body_record_placement_ids(&records);
        let placement_transforms = self.resolve_object_placement_transforms_for(placement_ids)?;
        eprintln!(
            "w velr body diagnostic phase placement_transforms done rows={}",
            placement_transforms.len()
        );
        timings.push(phase_timing(
            "placement_transforms",
            start.elapsed(),
            Some(placement_transforms.len()),
        ));

        let source_space = self.query_body_source_space()?;

        eprintln!(
            "w velr body diagnostic phase imported_scene_assembly start rows={}",
            records.len()
        );
        let start = Instant::now();
        let scene = imported_scene_resource_from_body_records(
            records,
            &placement_transforms,
            source_space,
        )?;
        let scene_rows = scene.instances.len();
        eprintln!("w velr body diagnostic phase imported_scene_assembly done rows={scene_rows}");
        timings.push(phase_timing(
            "imported_scene_assembly",
            start.elapsed(),
            Some(scene_rows),
        ));

        eprintln!("w velr body diagnostic phase backend_prepare_package start");
        let start = Instant::now();
        let package = GeometryBackend::default()
            .build_imported_scene_package(scene)
            .map_err(VelrIfcError::from)?;
        eprintln!(
            "w velr body diagnostic phase backend_prepare_package done rows={}",
            package.instance_count()
        );
        timings.push(phase_timing(
            "backend_prepare_package",
            start.elapsed(),
            Some(package.instance_count()),
        ));

        let cache_written = if write_cache {
            let start = Instant::now();
            write_cached_body_package(&self.layout, &package)?;
            timings.push(phase_timing("cache_write", start.elapsed(), None));
            true
        } else {
            timings.push(phase_timing("cache_write_skipped", Duration::ZERO, None));
            false
        };

        Ok(IfcBodyPackageDiagnostic {
            package,
            timings,
            brep: brep_query.diagnostic,
            cache_written,
        })
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
                .filter(|placement| placement.parent.is_some())
                .count(),
        })
    }

    pub fn extract_body_scene_resource(
        &self,
    ) -> Result<ImportedGeometrySceneResource, VelrIfcError> {
        let mut records = self.query_body_triangulated_records()?;
        records.extend(self.query_terrain_surface_triangulated_records()?);
        records.extend(self.query_body_polygonal_face_set_records()?);
        records.extend(self.query_body_extruded_records()?);
        records.extend(self.query_body_mapped_polygonal_face_set_records()?);
        records.extend(self.query_body_sectioned_solid_horizontal_records()?);
        records.extend(self.query_body_mapped_extruded_records()?);
        records.extend(self.query_body_faceted_brep_records()?);
        let placement_ids = body_record_placement_ids(&records);
        let placement_transforms = self.resolve_object_placement_transforms_for(placement_ids)?;
        let source_space = self.query_body_source_space()?;
        imported_scene_resource_from_body_records(records, &placement_transforms, source_space)
    }

    fn query_body_triangulated_records(&self) -> Result<Vec<IfcBodyRecord>, VelrIfcError> {
        self.query_triangulated_records(
            r#"
MATCH (p:IfcProduct)-[:REPRESENTATION]->(:IfcProductDefinitionShape)-[:REPRESENTATIONS]->(rep:IfcShapeRepresentation)-[:ITEMS]->(item:IfcTriangulatedFaceSet)-[:COORDINATES]->(pl:IfcCartesianPointList3D)
WHERE rep.RepresentationIdentifier = 'Body'
OPTIONAL MATCH (p)-[:OBJECT_PLACEMENT]->(placement)
OPTIONAL MATCH (p)<-[:RELATED_ELEMENTS]-(:IfcRelContainedInSpatialStructure)-[:RELATING_STRUCTURE]->(:IfcProduct)-[:OBJECT_PLACEMENT]->(container_placement)
OPTIONAL MATCH (p)<-[:RELATED_OBJECTS]-(:IfcRelDefinesByType)-[:RELATING_TYPE]->(type_node)
OPTIONAL MATCH (p)<-[:RELATED_OBJECTS]-(:IfcRelAssociatesClassification)-[:RELATING_CLASSIFICATION]->(classification_ref)
OPTIONAL MATCH (item)<-[:ITEM]-(styled:IfcStyledItem)
OPTIONAL MATCH (styled)-[:STYLES]->(style_assignment:IfcPresentationStyleAssignment)
OPTIONAL MATCH (style_assignment)-[:STYLES]->(assigned_surface_style:IfcSurfaceStyle)
OPTIONAL MATCH (styled)-[:STYLES]->(surface_style:IfcSurfaceStyle)
OPTIONAL MATCH (surface_style)-[:STYLES]->(surface_colour_style)-[:SURFACE_COLOUR]->(rgb:IfcColourRgb)
OPTIONAL MATCH (assigned_surface_style)-[:STYLES]->(assigned_surface_colour_style)-[:SURFACE_COLOUR]->(assigned_rgb:IfcColourRgb)
WITH p, placement, item, pl,
     head(collect(DISTINCT container_placement)) AS container_placement_id,
     head(collect(DISTINCT { object_type: type_node.ObjectType, predefined_type: type_node.PredefinedType })) AS type_semantics,
     head(collect(DISTINCT classification_ref.Identification)) AS classification_identification,
     head(collect(DISTINCT { red: rgb.Red, green: rgb.Green, blue: rgb.Blue })) AS surface_rgb,
     head(collect(DISTINCT { red: assigned_rgb.Red, green: assigned_rgb.Green, blue: assigned_rgb.Blue })) AS assigned_surface_rgb,
     head(collect(DISTINCT surface_style.Side)) AS surface_side,
     head(collect(DISTINCT assigned_surface_style.Side)) AS assigned_surface_side
RETURN id(p) AS product_id, id(placement) AS placement_id, container_placement_id, id(item) AS item_id, p.GlobalId AS global_id, p.Name AS name, p.ObjectType AS object_type, p.PredefinedType AS predefined_type, type_semantics.object_type AS type_object_type, type_semantics.predefined_type AS type_predefined_type, classification_identification, p.declared_entity AS declared_entity, pl.CoordList AS coord_list, item.CoordIndex AS coord_index, surface_rgb.red AS style_red, surface_rgb.green AS style_green, surface_rgb.blue AS style_blue, assigned_surface_rgb.red AS assigned_style_red, assigned_surface_rgb.green AS assigned_style_green, assigned_surface_rgb.blue AS assigned_style_blue, surface_side, assigned_surface_side
ORDER BY item_id
"#,
        )
    }

    fn query_terrain_surface_triangulated_records(
        &self,
    ) -> Result<Vec<IfcBodyRecord>, VelrIfcError> {
        self.query_triangulated_records(
            r#"
MATCH (p:IfcProduct)-[:REPRESENTATION]->(:IfcProductDefinitionShape)-[:REPRESENTATIONS]->(rep:IfcShapeRepresentation)-[:ITEMS]->(item:IfcTriangulatedFaceSet)-[:COORDINATES]->(pl:IfcCartesianPointList3D)
WHERE rep.RepresentationIdentifier = 'Surface'
  AND (
    p.declared_entity = 'IfcSite'
    OR p.declared_entity = 'IfcTopographyElement'
    OR p.declared_entity = 'IfcGeographicElement'
    OR p.declared_entity = 'IfcGeotechnicalStratum'
    OR p.declared_entity = 'IfcSurfaceFeature'
    OR p.declared_entity = 'IfcWater'
    OR p.declared_entity = 'IfcEarthworksCut'
    OR p.declared_entity = 'IfcEarthworksFill'
    OR (p.declared_entity = 'IfcDistributionChamberElement' AND p.PredefinedType = 'TRENCH')
  )
OPTIONAL MATCH (p)-[:OBJECT_PLACEMENT]->(placement)
OPTIONAL MATCH (p)<-[:RELATED_ELEMENTS]-(:IfcRelContainedInSpatialStructure)-[:RELATING_STRUCTURE]->(:IfcProduct)-[:OBJECT_PLACEMENT]->(container_placement)
OPTIONAL MATCH (p)<-[:RELATED_OBJECTS]-(:IfcRelDefinesByType)-[:RELATING_TYPE]->(type_node)
OPTIONAL MATCH (p)<-[:RELATED_OBJECTS]-(:IfcRelAssociatesClassification)-[:RELATING_CLASSIFICATION]->(classification_ref)
OPTIONAL MATCH (item)<-[:ITEM]-(styled:IfcStyledItem)
OPTIONAL MATCH (styled)-[:STYLES]->(style_assignment:IfcPresentationStyleAssignment)
OPTIONAL MATCH (style_assignment)-[:STYLES]->(assigned_surface_style:IfcSurfaceStyle)
OPTIONAL MATCH (styled)-[:STYLES]->(surface_style:IfcSurfaceStyle)
OPTIONAL MATCH (surface_style)-[:STYLES]->(surface_colour_style)-[:SURFACE_COLOUR]->(rgb:IfcColourRgb)
OPTIONAL MATCH (assigned_surface_style)-[:STYLES]->(assigned_surface_colour_style)-[:SURFACE_COLOUR]->(assigned_rgb:IfcColourRgb)
WITH p, placement, item, pl,
     head(collect(DISTINCT container_placement)) AS container_placement_id,
     head(collect(DISTINCT { object_type: type_node.ObjectType, predefined_type: type_node.PredefinedType })) AS type_semantics,
     head(collect(DISTINCT classification_ref.Identification)) AS classification_identification,
     head(collect(DISTINCT { red: rgb.Red, green: rgb.Green, blue: rgb.Blue })) AS surface_rgb,
     head(collect(DISTINCT { red: assigned_rgb.Red, green: assigned_rgb.Green, blue: assigned_rgb.Blue })) AS assigned_surface_rgb,
     head(collect(DISTINCT surface_style.Side)) AS surface_side,
     head(collect(DISTINCT assigned_surface_style.Side)) AS assigned_surface_side
RETURN id(p) AS product_id, id(placement) AS placement_id, container_placement_id, id(item) AS item_id, p.GlobalId AS global_id, p.Name AS name, p.ObjectType AS object_type, p.PredefinedType AS predefined_type, type_semantics.object_type AS type_object_type, type_semantics.predefined_type AS type_predefined_type, classification_identification, p.declared_entity AS declared_entity, pl.CoordList AS coord_list, item.CoordIndex AS coord_index, surface_rgb.red AS style_red, surface_rgb.green AS style_green, surface_rgb.blue AS style_blue, assigned_surface_rgb.red AS assigned_style_red, assigned_surface_rgb.green AS assigned_style_green, assigned_surface_rgb.blue AS assigned_style_blue, surface_side, assigned_surface_side
ORDER BY item_id
"#,
        )
    }

    fn query_triangulated_records(&self, cypher: &str) -> Result<Vec<IfcBodyRecord>, VelrIfcError> {
        let rows = self.execute_cypher_rows(cypher)?;

        let records: Vec<IfcBodyRecord> = rows
            .rows
            .into_iter()
            .map(|row| {
                let product_id = parse_u64_cell(row.first(), "product_id")?;
                let placement_id = parse_optional_u64_cell(row.get(1), "placement_id")?;
                let container_placement_id =
                    parse_optional_node_identity_cell(row.get(2), "container_placement_id")?;
                let placement_id = placement_id.or(container_placement_id);
                let item_id = parse_u64_cell(row.get(3), "item_id")?;
                let global_id = parse_optional_string_cell(row.get(4));
                let name = parse_optional_string_cell(row.get(5));
                let object_type = parse_optional_string_cell(row.get(6));
                let predefined_type = parse_optional_string_cell(row.get(7));
                let type_object_type = parse_optional_string_cell(row.get(8));
                let type_predefined_type = parse_optional_string_cell(row.get(9));
                let classification_identification = parse_optional_string_cell(row.get(10));
                let display_color = parse_optional_db_style_color_cells(
                    row.get(14),
                    row.get(15),
                    row.get(16),
                    row.get(17),
                    row.get(18),
                    row.get(19),
                )?;
                let face_visibility =
                    parse_optional_face_visibility_cells(row.get(20), row.get(21))?;
                let declared_entity =
                    parse_required_string_cell(row.get(11), "declared_entity")?.to_string();
                let Some(geometry) = tessellated_geometry_from_row(
                    parse_required_string_cell(row.get(12), "coord_list")?,
                    parse_required_string_cell(row.get(13), "coord_index")?,
                )?
                else {
                    eprintln!(
                        "w velr skipped empty IfcTriangulatedFaceSet item {item_id}: no faces with at least three vertices"
                    );
                    return Ok(None);
                };
                let primitive = Arc::new(GeometryPrimitive::Tessellated(geometry));

                Ok(Some(IfcBodyRecord {
                    product_id,
                    placement_id,
                    item_id,
                    occurrence_id: None,
                    global_id,
                    name,
                    object_type,
                    predefined_type,
                    type_object_type,
                    type_predefined_type,
                    classification_identification,
                    display_color,
                    face_visibility,
                    declared_entity,
                    item_transform: DMat4::IDENTITY,
                    primitive,
                }))
            })
            .collect::<Result<Vec<_>, VelrIfcError>>()?
            .into_iter()
            .flatten()
            .collect();

        Ok(records)
    }

    fn query_body_source_space(&self) -> Result<SourceSpace, VelrIfcError> {
        let length_unit = self
            .query_length_unit_from_database()?
            .unwrap_or(LengthUnit::Meter);
        Ok(SourceSpace::new(CoordinateFrame::w_world(), length_unit))
    }

    fn query_length_unit_from_database(&self) -> Result<Option<LengthUnit>, VelrIfcError> {
        let conversion_rows = self.execute_cypher_rows(
            r#"
MATCH (unit:IfcConversionBasedUnit)
WHERE unit.UnitType = 'LENGTHUNIT'
RETURN unit.Name AS name
LIMIT 1
"#,
        )?;
        if let Some(row) = conversion_rows.rows.first() {
            if let Some(unit) =
                parse_length_unit_from_conversion_name(parse_optional_string_cell(row.first()))
            {
                return Ok(Some(unit));
            }
        }

        let si_rows = self.execute_cypher_rows(
            r#"
MATCH (unit:IfcSIUnit)
WHERE unit.UnitType = 'LENGTHUNIT'
RETURN unit.Name AS name, unit.Prefix AS prefix
LIMIT 1
"#,
        )?;
        let Some(row) = si_rows.rows.first() else {
            return Ok(None);
        };

        Ok(parse_length_unit_from_si_unit_cells(
            parse_optional_string_cell(row.first()),
            parse_optional_string_cell(row.get(1)),
        ))
    }

    fn query_polygonal_face_set_geometry_by_item_ids(
        &self,
        item_ids: Option<&HashSet<u64>>,
    ) -> Result<HashMap<u64, Arc<GeometryPrimitive>>, VelrIfcError> {
        if let Some(item_ids) = item_ids {
            let mut item_ids = item_ids.iter().copied().collect::<Vec<_>>();
            item_ids.sort_unstable();
            if item_ids.is_empty() {
                return Ok(HashMap::new());
            }

            let mut geometry_by_item = HashMap::new();
            let total_chunks = item_ids
                .len()
                .div_ceil(POLYGONAL_FACE_SET_GEOMETRY_QUERY_CHUNK_SIZE);
            for (index, chunk) in item_ids
                .chunks(POLYGONAL_FACE_SET_GEOMETRY_QUERY_CHUNK_SIZE)
                .enumerate()
            {
                if total_chunks > 1 {
                    eprintln!(
                        "w velr polygonal geometry chunk {}/{} items={}",
                        index + 1,
                        total_chunks,
                        chunk.len()
                    );
                }
                geometry_by_item
                    .extend(self.query_polygonal_face_set_geometry_by_item_chunk(Some(chunk))?);
            }
            return Ok(geometry_by_item);
        }

        self.query_polygonal_face_set_geometry_by_item_chunk(None)
    }

    fn query_polygonal_face_set_geometry_by_item_chunk(
        &self,
        item_ids: Option<&[u64]>,
    ) -> Result<HashMap<u64, Arc<GeometryPrimitive>>, VelrIfcError> {
        let item_filter = item_ids
            .map(|item_ids| {
                let item_ids = item_ids
                    .iter()
                    .map(|item_id| item_id.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("WHERE id(item) IN [{item_ids}]")
            })
            .unwrap_or_default();
        let coord_rows = self.execute_cypher_rows(&format!(
            r#"
MATCH (item:IfcPolygonalFaceSet)
{item_filter}
MATCH (item)-[:COORDINATES]->(pl:IfcCartesianPointList3D)
RETURN id(item) AS item_id, pl.CoordList AS coord_list
ORDER BY item_id
"#
        ))?;
        let face_rows = self.execute_cypher_rows(&format!(
            r#"
MATCH (item:IfcPolygonalFaceSet)
{item_filter}
MATCH (item)-[face_edge:FACES]->(face:IfcIndexedPolygonalFace)
RETURN id(item) AS item_id, face_edge.ordinal AS face_ordinal, face.CoordIndex AS coord_index
ORDER BY item_id, face_edge.ordinal
"#
        ))?;

        let mut coord_list_by_item = HashMap::<u64, String>::new();
        for row in coord_rows.rows {
            let item_id = parse_u64_cell(row.first(), "item_id")?;
            let coord_list = parse_required_string_cell(row.get(1), "coord_list")?.to_string();
            coord_list_by_item.insert(item_id, coord_list);
        }

        let mut coord_index_rows_by_item = HashMap::<u64, Vec<(u64, String)>>::new();
        for row in face_rows.rows {
            let item_id = parse_u64_cell(row.first(), "item_id")?;
            let face_ordinal =
                parse_optional_u64_cell(row.get(1), "face_ordinal")?.unwrap_or_default();
            let coord_index = parse_required_string_cell(row.get(2), "coord_index")?.to_string();
            coord_index_rows_by_item
                .entry(item_id)
                .or_default()
                .push((face_ordinal, coord_index));
        }

        let mut geometry_by_item = HashMap::new();
        for (item_id, coord_list) in coord_list_by_item {
            let Some(mut coord_index_rows) = coord_index_rows_by_item.remove(&item_id) else {
                continue;
            };
            coord_index_rows.sort_by_key(|(face_ordinal, _)| *face_ordinal);
            let Some(geometry) =
                tessellated_geometry_from_coord_index_rows(&coord_list, &coord_index_rows)?
            else {
                eprintln!(
                    "w velr skipped empty IfcPolygonalFaceSet item {item_id}: no faces with at least three vertices"
                );
                continue;
            };
            let primitive = Arc::new(GeometryPrimitive::Tessellated(geometry));
            geometry_by_item.insert(item_id, primitive);
        }

        Ok(geometry_by_item)
    }

    fn query_body_polygonal_face_set_records(&self) -> Result<Vec<IfcBodyRecord>, VelrIfcError> {
        let rows = self.execute_cypher_rows(
            r#"
MATCH (rep:IfcShapeRepresentation)
WHERE rep.RepresentationIdentifier = 'Body'
WITH rep
MATCH (rep)-[:ITEMS]->(item:IfcPolygonalFaceSet)
WITH rep, item
MATCH (rep)<-[:REPRESENTATIONS]-(shape:IfcProductDefinitionShape)
WITH item, shape
MATCH (shape)<-[:REPRESENTATION]-(p:IfcProduct)
OPTIONAL MATCH (p)-[:OBJECT_PLACEMENT]->(placement)
OPTIONAL MATCH (p)<-[:RELATED_ELEMENTS]-(:IfcRelContainedInSpatialStructure)-[:RELATING_STRUCTURE]->(:IfcProduct)-[:OBJECT_PLACEMENT]->(container_placement)
OPTIONAL MATCH (p)<-[:RELATED_OBJECTS]-(:IfcRelDefinesByType)-[:RELATING_TYPE]->(type_node)
OPTIONAL MATCH (p)<-[:RELATED_OBJECTS]-(:IfcRelAssociatesClassification)-[:RELATING_CLASSIFICATION]->(classification_ref)
OPTIONAL MATCH (item)<-[:ITEM]-(styled:IfcStyledItem)
OPTIONAL MATCH (styled)-[:STYLES]->(style_assignment:IfcPresentationStyleAssignment)
OPTIONAL MATCH (style_assignment)-[:STYLES]->(assigned_surface_style:IfcSurfaceStyle)
OPTIONAL MATCH (styled)-[:STYLES]->(surface_style:IfcSurfaceStyle)
OPTIONAL MATCH (surface_style)-[:STYLES]->(surface_colour_style)-[:SURFACE_COLOUR]->(rgb:IfcColourRgb)
OPTIONAL MATCH (assigned_surface_style)-[:STYLES]->(assigned_surface_colour_style)-[:SURFACE_COLOUR]->(assigned_rgb:IfcColourRgb)
WITH p, placement, item,
     head(collect(DISTINCT container_placement)) AS container_placement_id,
     head(collect(DISTINCT { object_type: type_node.ObjectType, predefined_type: type_node.PredefinedType })) AS type_semantics,
     head(collect(DISTINCT classification_ref.Identification)) AS classification_identification,
     head(collect(DISTINCT { red: rgb.Red, green: rgb.Green, blue: rgb.Blue })) AS surface_rgb,
     head(collect(DISTINCT { red: assigned_rgb.Red, green: assigned_rgb.Green, blue: assigned_rgb.Blue })) AS assigned_surface_rgb,
     head(collect(DISTINCT surface_style.Side)) AS surface_side,
     head(collect(DISTINCT assigned_surface_style.Side)) AS assigned_surface_side
RETURN id(p) AS product_id, id(placement) AS placement_id, container_placement_id, id(item) AS item_id, p.GlobalId AS global_id, p.Name AS name, p.ObjectType AS object_type, p.PredefinedType AS predefined_type, type_semantics.object_type AS type_object_type, type_semantics.predefined_type AS type_predefined_type, classification_identification, p.declared_entity AS declared_entity, surface_rgb.red AS style_red, surface_rgb.green AS style_green, surface_rgb.blue AS style_blue, assigned_surface_rgb.red AS assigned_style_red, assigned_surface_rgb.green AS assigned_style_green, assigned_surface_rgb.blue AS assigned_style_blue, surface_side, assigned_surface_side
ORDER BY item_id
"#,
        )?;

        let raw_rows = rows.rows;
        let item_ids = raw_rows
            .iter()
            .map(|row| parse_u64_cell(row.get(3), "item_id"))
            .collect::<Result<HashSet<_>, VelrIfcError>>()?;
        let geometry_by_item =
            self.query_polygonal_face_set_geometry_by_item_ids(Some(&item_ids))?;

        let records: Vec<IfcBodyRecord> = raw_rows
            .into_iter()
            .map(|row| {
                let product_id = parse_u64_cell(row.first(), "product_id")?;
                let placement_id = parse_optional_u64_cell(row.get(1), "placement_id")?;
                let container_placement_id =
                    parse_optional_node_identity_cell(row.get(2), "container_placement_id")?;
                let placement_id = placement_id.or(container_placement_id);
                let item_id = parse_u64_cell(row.get(3), "item_id")?;
                let global_id = parse_optional_string_cell(row.get(4));
                let name = parse_optional_string_cell(row.get(5));
                let object_type = parse_optional_string_cell(row.get(6));
                let predefined_type = parse_optional_string_cell(row.get(7));
                let type_object_type = parse_optional_string_cell(row.get(8));
                let type_predefined_type = parse_optional_string_cell(row.get(9));
                let classification_identification = parse_optional_string_cell(row.get(10));
                let Some(primitive) = geometry_by_item.get(&item_id).cloned() else {
                    return Ok(None);
                };
                let display_color = parse_optional_db_style_color_cells(
                    row.get(12),
                    row.get(13),
                    row.get(14),
                    row.get(15),
                    row.get(16),
                    row.get(17),
                )?;
                let face_visibility =
                    parse_optional_face_visibility_cells(row.get(18), row.get(19))?;
                let declared_entity =
                    parse_required_string_cell(row.get(11), "declared_entity")?.to_string();

                Ok(Some(IfcBodyRecord {
                    product_id,
                    placement_id,
                    item_id,
                    occurrence_id: None,
                    global_id,
                    name,
                    object_type,
                    predefined_type,
                    type_object_type,
                    type_predefined_type,
                    classification_identification,
                    display_color,
                    face_visibility,
                    declared_entity,
                    item_transform: DMat4::IDENTITY,
                    primitive,
                }))
            })
            .collect::<Result<Vec<_>, VelrIfcError>>()?
            .into_iter()
            .flatten()
            .collect();

        Ok(records)
    }

    fn query_body_faceted_brep_records(&self) -> Result<Vec<IfcBodyRecord>, VelrIfcError> {
        Ok(self.query_body_faceted_brep_records_limited(None)?.records)
    }

    fn query_body_faceted_brep_records_limited(
        &self,
        limit_items: Option<usize>,
    ) -> Result<FacetedBrepRecordQuery, VelrIfcError> {
        let mut timings = Vec::new();

        let geometry_query = self.query_faceted_brep_geometry_by_item_limited(limit_items)?;
        let geometry_by_item = geometry_query.geometry_by_item;
        let geometry_diagnostic = geometry_query.diagnostic;
        timings.extend(geometry_query.timings);

        let metadata_query = faceted_brep_metadata_query(limit_items);
        let start = Instant::now();
        let rows = self.execute_cypher_rows(&metadata_query)?;
        let metadata_elapsed = start.elapsed();
        let metadata_rows = rows.rows.len();
        let direct_product_ids = rows
            .rows
            .iter()
            .map(|row| parse_u64_cell(row.first(), "product_id"))
            .collect::<Result<HashSet<_>, VelrIfcError>>()?;

        let start = Instant::now();
        let placement_parent_by_id = if metadata_rows > 0 {
            self.query_placement_parent_map()?
        } else {
            HashMap::new()
        };
        timings.push(phase_timing(
            "brep_placement_parent_query",
            start.elapsed(),
            Some(placement_parent_by_id.len()),
        ));

        let start = Instant::now();
        let aggregate_parent_placement_by_product = if metadata_rows > 0 {
            self.query_aggregate_child_parent_placement_map_for_products(&direct_product_ids)?
        } else {
            HashMap::new()
        };
        timings.push(phase_timing(
            "brep_aggregate_parent_placement_query",
            start.elapsed(),
            Some(aggregate_parent_placement_by_product.len()),
        ));

        let start = Instant::now();
        let mut records = rows
            .rows
            .into_iter()
            .map(|row| {
                let product_id = parse_u64_cell(row.first(), "product_id")?;
                let placement_id = effective_faceted_brep_placement_id(
                    product_id,
                    parse_optional_u64_cell(row.get(1), "placement_id")?,
                    &placement_parent_by_id,
                    &aggregate_parent_placement_by_product,
                );
                let item_id = parse_u64_cell(row.get(2), "item_id")?;
                let primitive = geometry_by_item.get(&item_id).cloned().ok_or_else(|| {
                    VelrIfcError::IfcGeometryData(format!(
                        "IfcFacetedBrep item `{item_id}` had product metadata but no face geometry"
                    ))
                })?;

                Ok(IfcBodyRecord {
                    product_id,
                    placement_id,
                    item_id,
                    occurrence_id: None,
                    global_id: parse_optional_string_cell(row.get(3)),
                    name: parse_optional_string_cell(row.get(4)),
                    object_type: parse_optional_string_cell(row.get(5)),
                    predefined_type: parse_optional_string_cell(row.get(6)),
                    type_object_type: parse_optional_string_cell(row.get(7)),
                    type_predefined_type: parse_optional_string_cell(row.get(8)),
                    classification_identification: parse_optional_string_cell(row.get(9)),
                    declared_entity: parse_required_string_cell(row.get(10), "declared_entity")?
                        .to_string(),
                    display_color: parse_optional_db_style_color_cells(
                        row.get(11),
                        row.get(12),
                        row.get(13),
                        row.get(14),
                        row.get(15),
                        row.get(16),
                    )?,
                    face_visibility: FaceVisibility::OneSided,
                    item_transform: DMat4::IDENTITY,
                    primitive,
                })
            })
            .collect::<Result<Vec<_>, VelrIfcError>>()?;
        let direct_parse_elapsed = start.elapsed();

        let mapped_metadata_query = faceted_brep_mapped_metadata_query(limit_items);
        let start = Instant::now();
        let mapped_rows = self.execute_cypher_rows(&mapped_metadata_query)?;
        let mapped_metadata_elapsed = start.elapsed();
        let mapped_metadata_rows = mapped_rows.rows.len();
        let mapped_item_transforms = if mapped_metadata_rows > 0 {
            self.query_mapped_item_transform_map()?
        } else {
            HashMap::new()
        };

        let start = Instant::now();
        let mut mapped_records = mapped_rows
            .rows
            .into_iter()
            .map(|row| {
                let mapped_item_id = parse_u64_cell(row.first(), "mapped_item_id")?;
                let product_id = parse_u64_cell(row.get(1), "product_id")?;
                let placement_id = parse_optional_u64_cell(row.get(2), "placement_id")?;
                let item_id = parse_u64_cell(row.get(3), "item_id")?;
                let primitive = geometry_by_item.get(&item_id).cloned().ok_or_else(|| {
                    VelrIfcError::IfcGeometryData(format!(
                        "mapped IfcFacetedBrep item `{item_id}` had product metadata but no face geometry"
                    ))
                })?;
                let mapped_transform =
                    mapped_item_transforms
                        .get(&mapped_item_id)
                        .copied()
                        .ok_or_else(|| {
                            VelrIfcError::IfcGeometryData(format!(
                                "IfcMappedItem `{mapped_item_id}` was missing from mapped item transforms"
                            ))
                        })?;

                Ok(IfcBodyRecord {
                    product_id,
                    placement_id,
                    item_id,
                    occurrence_id: Some(mapped_item_id),
                    global_id: parse_optional_string_cell(row.get(4)),
                    name: parse_optional_string_cell(row.get(5)),
                    object_type: parse_optional_string_cell(row.get(6)),
                    predefined_type: parse_optional_string_cell(row.get(7)),
                    type_object_type: parse_optional_string_cell(row.get(8)),
                    type_predefined_type: parse_optional_string_cell(row.get(9)),
                    classification_identification: parse_optional_string_cell(row.get(10)),
                    declared_entity: parse_required_string_cell(row.get(11), "declared_entity")?
                        .to_string(),
                    display_color: parse_optional_db_style_color_cells(
                        row.get(12),
                        row.get(13),
                        row.get(14),
                        row.get(15),
                        row.get(16),
                        row.get(17),
                    )?,
                    face_visibility: FaceVisibility::OneSided,
                    item_transform: mapped_transform,
                    primitive,
                })
            })
            .collect::<Result<Vec<_>, VelrIfcError>>()?;
        records.append(&mut mapped_records);

        timings.push(phase_timing(
            "brep_metadata_query",
            metadata_elapsed,
            Some(metadata_rows),
        ));
        timings.push(phase_timing(
            "brep_metadata_parse_records",
            direct_parse_elapsed,
            Some(metadata_rows),
        ));
        timings.push(phase_timing(
            "mapped_brep_metadata_query",
            mapped_metadata_elapsed,
            Some(mapped_metadata_rows),
        ));
        timings.push(phase_timing(
            "mapped_brep_metadata_parse_records",
            start.elapsed(),
            Some(mapped_metadata_rows),
        ));

        Ok(FacetedBrepRecordQuery {
            records,
            diagnostic: IfcBodyBrepDiagnostic {
                limit_items,
                geometry_items: geometry_diagnostic.geometry_items,
                geometry_faces: geometry_diagnostic.geometry_faces,
                geometry_point_rows: geometry_diagnostic.geometry_point_rows,
                metadata_rows: metadata_rows + mapped_metadata_rows,
            },
            timings,
        })
    }

    fn query_faceted_brep_geometry_by_item_limited(
        &self,
        limit_items: Option<usize>,
    ) -> Result<FacetedBrepGeometryQuery, VelrIfcError> {
        let cypher = faceted_brep_geometry_query(limit_items);

        let start = Instant::now();
        let (faces_by_item, point_rows) =
            exec_cypher_in_scoped_tx(&self.raw_db, &cypher, |table| {
                let mut faces_by_item =
                    HashMap::<u64, HashMap<u64, FacetedBrepFaceAccumulator>>::new();
                let mut processing_error = None;
                let mut point_rows = 0usize;

                table.for_each_row(|row| {
                    if processing_error.is_some() {
                        return Ok(());
                    }

                    let cells = row.iter().map(render_cell).collect::<Vec<_>>();
                    let row_result = (|| {
                        let item_id = parse_u64_cell(cells.first(), "item_id")?;
                        let face_id = parse_u64_cell(cells.get(1), "face_id")?;
                        let face_ordinal = parse_optional_u64_cell(cells.get(2), "face_ordinal")?;
                        let point_ordinal = parse_optional_u64_cell(cells.get(3), "point_ordinal")?;
                        let coordinates = parse_dvec3_cell(cells.get(4), "coordinates")?;

                        let face = faces_by_item
                            .entry(item_id)
                            .or_default()
                            .entry(face_id)
                            .or_insert_with(|| FacetedBrepFaceAccumulator {
                                ordinal: face_ordinal,
                                points: Vec::new(),
                            });

                        if face.ordinal.is_none() {
                            face.ordinal = face_ordinal;
                        } else if face_ordinal.is_some() && face.ordinal != face_ordinal {
                            return Err(VelrIfcError::IfcGeometryData(format!(
                                "IfcFacetedBrep face `{face_id}` resolved to inconsistent ordinals"
                            )));
                        }

                        face.points.push(FacetedBrepPoint {
                            ordinal: point_ordinal,
                            sequence: face.points.len(),
                            coordinates,
                        });
                        point_rows += 1;
                        Ok(())
                    })();

                    if let Err(error) = row_result {
                        processing_error = Some(error);
                    }
                    Ok(())
                })?;

                if let Some(error) = processing_error {
                    return Err(error);
                }

                Ok((faces_by_item, point_rows))
            })?;
        let query_parse_group_elapsed = start.elapsed();

        let geometry_items = faces_by_item.len();
        let geometry_faces = faces_by_item.values().map(HashMap::len).sum();
        let start = Instant::now();
        let geometry_by_item = faces_by_item
            .into_iter()
            .map(|(item_id, faces)| {
                Ok((
                    item_id,
                    Arc::new(GeometryPrimitive::Tessellated(
                        faceted_brep_geometry_from_faces(faces)?,
                    )),
                ))
            })
            .collect::<Result<HashMap<_, _>, VelrIfcError>>()?;
        let geometry_build_elapsed = start.elapsed();

        Ok(FacetedBrepGeometryQuery {
            geometry_by_item,
            diagnostic: FacetedBrepGeometryDiagnostic {
                geometry_items,
                geometry_faces,
                geometry_point_rows: point_rows,
            },
            timings: vec![
                phase_timing(
                    "brep_geometry_query_parse_group",
                    query_parse_group_elapsed,
                    Some(point_rows),
                ),
                phase_timing(
                    "brep_geometry_build",
                    geometry_build_elapsed,
                    Some(geometry_items),
                ),
            ],
        })
    }

    fn query_body_extruded_records(&self) -> Result<Vec<IfcBodyRecord>, VelrIfcError> {
        let location_by_axis2 =
            self.query_axis2_placement_vector_map("LOCATION", "IfcCartesianPoint", "Coordinates")?;
        let axis_by_axis2 =
            self.query_axis2_placement_vector_map("AXIS", "IfcDirection", "DirectionRatios")?;
        let ref_direction_by_axis2 = self.query_axis2_placement_vector_map(
            "REF_DIRECTION",
            "IfcDirection",
            "DirectionRatios",
        )?;

        let rows = self.execute_cypher_rows(
            r#"
MATCH (solid:IfcExtrudedAreaSolid)
MATCH (solid)<-[:ITEMS]-(rep)
WHERE rep.RepresentationIdentifier = 'Body'
WITH solid, rep
MATCH (rep)<-[:REPRESENTATIONS]-(shape)
WITH solid, shape
MATCH (shape)<-[:REPRESENTATION]-(p)
WITH solid, p
MATCH (solid)-[:SWEPT_AREA]->(profile)
WITH solid, p, profile
MATCH (profile)-[:OUTER_CURVE]->(poly)
WITH solid, p, poly
MATCH (poly)-[edge:POINTS]->(pt)
WITH solid, p, collect(DISTINCT { ordinal: edge.ordinal, coordinates: pt.Coordinates }) AS point_rows
MATCH (solid)-[:EXTRUDED_DIRECTION]->(dir)
OPTIONAL MATCH (p)-[:OBJECT_PLACEMENT]->(placement)
OPTIONAL MATCH (p)<-[:RELATED_OBJECTS]-(:IfcRelDefinesByType)-[:RELATING_TYPE]->(type_node)
OPTIONAL MATCH (p)<-[:RELATED_OBJECTS]-(:IfcRelAssociatesClassification)-[:RELATING_CLASSIFICATION]->(classification_ref)
OPTIONAL MATCH (solid)-[:POSITION]->(solid_position)
OPTIONAL MATCH (solid)<-[:ITEM]-(styled:IfcStyledItem)
OPTIONAL MATCH (styled)-[:STYLES]->(style_assignment:IfcPresentationStyleAssignment)
OPTIONAL MATCH (style_assignment)-[:STYLES]->(assigned_surface_style:IfcSurfaceStyle)
OPTIONAL MATCH (styled)-[:STYLES]->(surface_style:IfcSurfaceStyle)
OPTIONAL MATCH (surface_style)-[:STYLES]->(surface_colour_style)-[:SURFACE_COLOUR]->(rgb:IfcColourRgb)
OPTIONAL MATCH (assigned_surface_style)-[:STYLES]->(assigned_surface_colour_style)-[:SURFACE_COLOUR]->(assigned_rgb:IfcColourRgb)
WITH p, placement, solid, dir, solid_position,
     point_rows,
     head(collect(DISTINCT { object_type: type_node.ObjectType, predefined_type: type_node.PredefinedType })) AS type_semantics,
     head(collect(DISTINCT classification_ref.Identification)) AS classification_identification,
     head(collect(DISTINCT { red: rgb.Red, green: rgb.Green, blue: rgb.Blue })) AS surface_rgb,
     head(collect(DISTINCT { red: assigned_rgb.Red, green: assigned_rgb.Green, blue: assigned_rgb.Blue })) AS assigned_surface_rgb
RETURN id(p) AS product_id, id(placement) AS placement_id, id(solid) AS item_id, p.GlobalId AS global_id, p.Name AS name, p.ObjectType AS object_type, p.PredefinedType AS predefined_type, type_semantics.object_type AS type_object_type, type_semantics.predefined_type AS type_predefined_type, classification_identification, p.declared_entity AS declared_entity, solid.Depth AS depth, dir.DirectionRatios AS extruded_direction, point_rows, id(solid_position) AS solid_position_id, surface_rgb.red AS style_red, surface_rgb.green AS style_green, surface_rgb.blue AS style_blue, assigned_surface_rgb.red AS assigned_style_red, assigned_surface_rgb.green AS assigned_style_green, assigned_surface_rgb.blue AS assigned_style_blue
ORDER BY item_id
"#,
        )?;

        let records: Vec<IfcBodyRecord> = rows
            .rows
            .into_iter()
            .map(|row| {
                let product_id = parse_u64_cell(row.first(), "product_id")?;
                let placement_id = parse_optional_u64_cell(row.get(1), "placement_id")?;
                let item_id = parse_u64_cell(row.get(2), "item_id")?;
                let global_id = parse_optional_string_cell(row.get(3));
                let name = parse_optional_string_cell(row.get(4));
                let object_type = parse_optional_string_cell(row.get(5));
                let predefined_type = parse_optional_string_cell(row.get(6));
                let type_object_type = parse_optional_string_cell(row.get(7));
                let type_predefined_type = parse_optional_string_cell(row.get(8));
                let classification_identification = parse_optional_string_cell(row.get(9));
                let solid_position_id = parse_optional_u64_cell(row.get(14), "solid_position_id")?;
                let item_transform = solid_position_id.map_or(DMat4::IDENTITY, |position_id| {
                    axis2_placement_transform(
                        location_by_axis2
                            .get(&position_id)
                            .copied()
                            .unwrap_or(DVec3::ZERO),
                        axis_by_axis2.get(&position_id).copied(),
                        ref_direction_by_axis2.get(&position_id).copied(),
                    )
                });
                let display_color = parse_optional_db_style_color_cells(
                    row.get(15),
                    row.get(16),
                    row.get(17),
                    row.get(18),
                    row.get(19),
                    row.get(20),
                )?;
                let declared_entity =
                    parse_required_string_cell(row.get(10), "declared_entity")?.to_string();
                let depth = parse_f64_cell(row.get(11), "depth")?;
                let extruded_direction = parse_direction3_json(parse_required_string_cell(
                    row.get(12),
                    "extruded_direction",
                )?)?;
                let primitive = Arc::new(GeometryPrimitive::SweptSolid(swept_solid_from_row(
                    parse_required_string_cell(row.get(13), "point_rows")?,
                    extruded_direction * depth,
                )?));

                Ok(IfcBodyRecord {
                    product_id,
                    placement_id,
                    item_id,
                    occurrence_id: None,
                    global_id,
                    name,
                    object_type,
                    predefined_type,
                    type_object_type,
                    type_predefined_type,
                    classification_identification,
                    display_color,
                    face_visibility: FaceVisibility::OneSided,
                    declared_entity,
                    item_transform,
                    primitive,
                })
            })
            .collect::<Result<_, VelrIfcError>>()?;

        Ok(records)
    }

    fn query_mapped_extruded_geometry_by_item(
        &self,
    ) -> Result<HashMap<u64, ExtrudedBodyGeometry>, VelrIfcError> {
        let location_by_axis2 =
            self.query_axis2_placement_vector_map("LOCATION", "IfcCartesianPoint", "Coordinates")?;
        let axis_by_axis2 =
            self.query_axis2_placement_vector_map("AXIS", "IfcDirection", "DirectionRatios")?;
        let ref_direction_by_axis2 = self.query_axis2_placement_vector_map(
            "REF_DIRECTION",
            "IfcDirection",
            "DirectionRatios",
        )?;

        let rows = self.execute_cypher_rows(
            r#"
MATCH (mapped:IfcMappedItem)
WITH mapped
MATCH (mapped)-[:MAPPING_SOURCE]->(map:IfcRepresentationMap)
WITH map
MATCH (map)-[:MAPPED_REPRESENTATION]->(source_rep:IfcShapeRepresentation)
WITH DISTINCT source_rep
MATCH (source_rep)-[:ITEMS]->(solid:IfcExtrudedAreaSolid)
WITH DISTINCT solid
MATCH (solid)-[:SWEPT_AREA]->(profile)
WITH solid, profile
MATCH (profile)-[:OUTER_CURVE]->(poly)
WITH solid, poly
MATCH (poly)-[edge:POINTS]->(pt)
WITH solid, collect(DISTINCT { ordinal: edge.ordinal, coordinates: pt.Coordinates }) AS point_rows
MATCH (solid)-[:EXTRUDED_DIRECTION]->(dir)
OPTIONAL MATCH (solid)-[:POSITION]->(solid_position)
RETURN id(solid) AS item_id, solid.Depth AS depth, dir.DirectionRatios AS extruded_direction, point_rows, id(solid_position) AS solid_position_id
ORDER BY item_id
"#,
        )?;

        rows.rows
            .into_iter()
            .map(|row| {
                let item_id = parse_u64_cell(row.first(), "item_id")?;
                let depth = parse_f64_cell(row.get(1), "depth")?;
                let extruded_direction = parse_direction3_json(parse_required_string_cell(
                    row.get(2),
                    "extruded_direction",
                )?)?;
                let solid_position_id = parse_optional_u64_cell(row.get(4), "solid_position_id")?;
                let item_transform = solid_position_id.map_or(DMat4::IDENTITY, |position_id| {
                    axis2_placement_transform(
                        location_by_axis2
                            .get(&position_id)
                            .copied()
                            .unwrap_or(DVec3::ZERO),
                        axis_by_axis2.get(&position_id).copied(),
                        ref_direction_by_axis2.get(&position_id).copied(),
                    )
                });
                Ok((
                    item_id,
                    ExtrudedBodyGeometry {
                        primitive: Arc::new(GeometryPrimitive::SweptSolid(swept_solid_from_row(
                            parse_required_string_cell(row.get(3), "point_rows")?,
                            extruded_direction * depth,
                        )?)),
                        item_transform,
                    },
                ))
            })
            .collect()
    }

    fn query_body_mapped_extruded_records(&self) -> Result<Vec<IfcBodyRecord>, VelrIfcError> {
        if !self.has_extruded_area_solid()? {
            return Ok(Vec::new());
        }

        let mapped_item_transforms = self.query_mapped_item_transform_map()?;
        let geometry_by_item = self.query_mapped_extruded_geometry_by_item()?;

        let rows = self.execute_cypher_rows(
            r#"
MATCH (mapped:IfcMappedItem)
WITH mapped
MATCH (mapped)<-[:ITEMS]-(rep:IfcShapeRepresentation)
WHERE rep.RepresentationIdentifier = 'Body'
WITH mapped, rep
MATCH (rep)<-[:REPRESENTATIONS]-(shape:IfcProductDefinitionShape)
WITH mapped, shape
MATCH (shape)<-[:REPRESENTATION]-(p:IfcProduct)
WITH mapped, p
MATCH (mapped)-[:MAPPING_SOURCE]->(map:IfcRepresentationMap)
WITH mapped, p, map
MATCH (map)-[:MAPPED_REPRESENTATION]->(source_rep:IfcShapeRepresentation)
WITH mapped, p, source_rep
MATCH (source_rep)-[:ITEMS]->(solid:IfcExtrudedAreaSolid)
OPTIONAL MATCH (p)-[:OBJECT_PLACEMENT]->(placement)
OPTIONAL MATCH (p)<-[:RELATED_OBJECTS]-(:IfcRelDefinesByType)-[:RELATING_TYPE]->(type_node)
OPTIONAL MATCH (p)<-[:RELATED_OBJECTS]-(:IfcRelAssociatesClassification)-[:RELATING_CLASSIFICATION]->(classification_ref)
OPTIONAL MATCH (solid)<-[:ITEM]-(styled:IfcStyledItem)
OPTIONAL MATCH (styled)-[:STYLES]->(style_assignment:IfcPresentationStyleAssignment)
OPTIONAL MATCH (style_assignment)-[:STYLES]->(assigned_surface_style:IfcSurfaceStyle)
OPTIONAL MATCH (styled)-[:STYLES]->(surface_style:IfcSurfaceStyle)
OPTIONAL MATCH (surface_style)-[:STYLES]->(surface_colour_style)-[:SURFACE_COLOUR]->(rgb:IfcColourRgb)
OPTIONAL MATCH (assigned_surface_style)-[:STYLES]->(assigned_surface_colour_style)-[:SURFACE_COLOUR]->(assigned_rgb:IfcColourRgb)
WITH mapped, p, placement, solid,
     head(collect(DISTINCT { object_type: type_node.ObjectType, predefined_type: type_node.PredefinedType })) AS type_semantics,
     head(collect(DISTINCT classification_ref.Identification)) AS classification_identification,
     head(collect(DISTINCT { red: rgb.Red, green: rgb.Green, blue: rgb.Blue })) AS surface_rgb,
     head(collect(DISTINCT { red: assigned_rgb.Red, green: assigned_rgb.Green, blue: assigned_rgb.Blue })) AS assigned_surface_rgb
RETURN id(mapped) AS mapped_item_id, id(p) AS product_id, id(placement) AS placement_id, id(solid) AS item_id, p.GlobalId AS global_id, p.Name AS name, p.ObjectType AS object_type, p.PredefinedType AS predefined_type, type_semantics.object_type AS type_object_type, type_semantics.predefined_type AS type_predefined_type, classification_identification, p.declared_entity AS declared_entity, surface_rgb.red AS style_red, surface_rgb.green AS style_green, surface_rgb.blue AS style_blue, assigned_surface_rgb.red AS assigned_style_red, assigned_surface_rgb.green AS assigned_style_green, assigned_surface_rgb.blue AS assigned_style_blue
ORDER BY mapped_item_id, item_id
"#,
        )?;

        let records = rows
            .rows
            .into_iter()
            .map(|row| {
                let mapped_item_id = parse_u64_cell(row.first(), "mapped_item_id")?;
                let product_id = parse_u64_cell(row.get(1), "product_id")?;
                let placement_id = parse_optional_u64_cell(row.get(2), "placement_id")?;
                let item_id = parse_u64_cell(row.get(3), "item_id")?;
                let global_id = parse_optional_string_cell(row.get(4));
                let name = parse_optional_string_cell(row.get(5));
                let object_type = parse_optional_string_cell(row.get(6));
                let predefined_type = parse_optional_string_cell(row.get(7));
                let type_object_type = parse_optional_string_cell(row.get(8));
                let type_predefined_type = parse_optional_string_cell(row.get(9));
                let classification_identification = parse_optional_string_cell(row.get(10));
                let Some(geometry) = geometry_by_item.get(&item_id).cloned() else {
                    return Ok(None);
                };
                let mapped_transform =
                    mapped_item_transforms
                        .get(&mapped_item_id)
                        .copied()
                        .ok_or_else(|| {
                            VelrIfcError::IfcGeometryData(format!(
                                "IfcMappedItem `{mapped_item_id}` was missing from mapped item transforms"
                            ))
                        })?;
                let display_color = parse_optional_db_style_color_cells(
                    row.get(12),
                    row.get(13),
                    row.get(14),
                    row.get(15),
                    row.get(16),
                    row.get(17),
                )?;
                let declared_entity =
                    parse_required_string_cell(row.get(11), "declared_entity")?.to_string();

                Ok(Some(IfcBodyRecord {
                    product_id,
                    placement_id,
                    item_id,
                    occurrence_id: Some(mapped_item_id),
                    global_id,
                    name,
                    object_type,
                    predefined_type,
                    type_object_type,
                    type_predefined_type,
                    classification_identification,
                    display_color,
                    face_visibility: FaceVisibility::OneSided,
                    declared_entity,
                    item_transform: mapped_transform * geometry.item_transform,
                    primitive: geometry.primitive,
                }))
            })
            .collect::<Result<Vec<_>, VelrIfcError>>()?
            .into_iter()
            .flatten()
            .collect();

        Ok(records)
    }

    fn has_extruded_area_solid(&self) -> Result<bool, VelrIfcError> {
        let rows = self.execute_cypher_rows(
            r#"
MATCH (solid:IfcExtrudedAreaSolid)
RETURN id(solid) AS item_id
LIMIT 1
"#,
        )?;
        Ok(!rows.rows.is_empty())
    }

    fn query_body_mapped_polygonal_face_set_records(
        &self,
    ) -> Result<Vec<IfcBodyRecord>, VelrIfcError> {
        eprintln!("w velr mapped polygonal phase transforms start");
        let mapped_item_transforms = self.query_mapped_item_transform_map()?;
        eprintln!(
            "w velr mapped polygonal phase transforms done rows={}",
            mapped_item_transforms.len()
        );

        eprintln!("w velr mapped polygonal phase metadata start");
        let rows = self.execute_cypher_rows(
            r#"
MATCH (mapped:IfcMappedItem)
WITH mapped
MATCH (mapped)<-[:ITEMS]-(rep:IfcShapeRepresentation)
WHERE rep.RepresentationIdentifier = 'Body'
WITH mapped, rep
MATCH (rep)<-[:REPRESENTATIONS]-(shape:IfcProductDefinitionShape)
WITH mapped, shape
MATCH (shape)<-[:REPRESENTATION]-(p:IfcProduct)
WITH mapped, p
MATCH (mapped)-[:MAPPING_SOURCE]->(map:IfcRepresentationMap)
WITH mapped, p, map
MATCH (map)-[:MAPPED_REPRESENTATION]->(source_rep:IfcShapeRepresentation)
WITH mapped, p, source_rep
MATCH (source_rep)-[:ITEMS]->(item:IfcPolygonalFaceSet)
OPTIONAL MATCH (p)-[:OBJECT_PLACEMENT]->(placement)
OPTIONAL MATCH (p)<-[:RELATED_ELEMENTS]-(:IfcRelContainedInSpatialStructure)-[:RELATING_STRUCTURE]->(:IfcProduct)-[:OBJECT_PLACEMENT]->(container_placement)
OPTIONAL MATCH (p)<-[:RELATED_OBJECTS]-(:IfcRelDefinesByType)-[:RELATING_TYPE]->(type_node)
OPTIONAL MATCH (p)<-[:RELATED_OBJECTS]-(:IfcRelAssociatesClassification)-[:RELATING_CLASSIFICATION]->(classification_ref)
OPTIONAL MATCH (item)<-[:ITEM]-(styled:IfcStyledItem)
OPTIONAL MATCH (styled)-[:STYLES]->(style_assignment:IfcPresentationStyleAssignment)
OPTIONAL MATCH (style_assignment)-[:STYLES]->(assigned_surface_style:IfcSurfaceStyle)
OPTIONAL MATCH (styled)-[:STYLES]->(surface_style:IfcSurfaceStyle)
OPTIONAL MATCH (surface_style)-[:STYLES]->(surface_colour_style)-[:SURFACE_COLOUR]->(rgb:IfcColourRgb)
OPTIONAL MATCH (assigned_surface_style)-[:STYLES]->(assigned_surface_colour_style)-[:SURFACE_COLOUR]->(assigned_rgb:IfcColourRgb)
WITH mapped, p, placement, item,
     head(collect(DISTINCT container_placement)) AS container_placement_id,
     head(collect(DISTINCT { object_type: type_node.ObjectType, predefined_type: type_node.PredefinedType })) AS type_semantics,
     head(collect(DISTINCT classification_ref.Identification)) AS classification_identification,
     head(collect(DISTINCT { red: rgb.Red, green: rgb.Green, blue: rgb.Blue })) AS surface_rgb,
     head(collect(DISTINCT { red: assigned_rgb.Red, green: assigned_rgb.Green, blue: assigned_rgb.Blue })) AS assigned_surface_rgb,
     head(collect(DISTINCT surface_style.Side)) AS surface_side,
     head(collect(DISTINCT assigned_surface_style.Side)) AS assigned_surface_side
RETURN id(mapped) AS mapped_item_id, id(p) AS product_id, id(placement) AS placement_id, container_placement_id, id(item) AS item_id, p.GlobalId AS global_id, p.Name AS name, p.ObjectType AS object_type, p.PredefinedType AS predefined_type, type_semantics.object_type AS type_object_type, type_semantics.predefined_type AS type_predefined_type, classification_identification, p.declared_entity AS declared_entity, surface_rgb.red AS style_red, surface_rgb.green AS style_green, surface_rgb.blue AS style_blue, assigned_surface_rgb.red AS assigned_style_red, assigned_surface_rgb.green AS assigned_style_green, assigned_surface_rgb.blue AS assigned_style_blue, surface_side, assigned_surface_side
ORDER BY mapped_item_id, item_id
"#,
        )?;
        eprintln!(
            "w velr mapped polygonal phase metadata done rows={}",
            rows.rows.len()
        );

        let raw_rows = rows.rows;
        let item_ids = raw_rows
            .iter()
            .map(|row| parse_u64_cell(row.get(4), "item_id"))
            .collect::<Result<HashSet<_>, VelrIfcError>>()?;
        eprintln!(
            "w velr mapped polygonal phase geometry start items={}",
            item_ids.len()
        );
        let geometry_by_item =
            self.query_polygonal_face_set_geometry_by_item_ids(Some(&item_ids))?;
        eprintln!(
            "w velr mapped polygonal phase geometry done items={}",
            geometry_by_item.len()
        );

        let records: Vec<IfcBodyRecord> = raw_rows
            .into_iter()
            .map(|row| {
                let mapped_item_id = parse_u64_cell(row.first(), "mapped_item_id")?;
                let product_id = parse_u64_cell(row.get(1), "product_id")?;
                let placement_id = parse_optional_u64_cell(row.get(2), "placement_id")?;
                let container_placement_id =
                    parse_optional_node_identity_cell(row.get(3), "container_placement_id")?;
                let placement_id = placement_id.or(container_placement_id);
                let item_id = parse_u64_cell(row.get(4), "item_id")?;
                let global_id = parse_optional_string_cell(row.get(5));
                let name = parse_optional_string_cell(row.get(6));
                let object_type = parse_optional_string_cell(row.get(7));
                let predefined_type = parse_optional_string_cell(row.get(8));
                let type_object_type = parse_optional_string_cell(row.get(9));
                let type_predefined_type = parse_optional_string_cell(row.get(10));
                let classification_identification = parse_optional_string_cell(row.get(11));
                let Some(primitive) = geometry_by_item.get(&item_id).cloned() else {
                    return Ok(None);
                };
                let mapped_transform =
                    mapped_item_transforms
                        .get(&mapped_item_id)
                        .copied()
                        .ok_or_else(|| {
                            VelrIfcError::IfcGeometryData(format!(
                                "IfcMappedItem `{mapped_item_id}` was missing from mapped item transforms"
                            ))
                        })?;
                let display_color = parse_optional_db_style_color_cells(
                    row.get(13),
                    row.get(14),
                    row.get(15),
                    row.get(16),
                    row.get(17),
                    row.get(18),
                )?;
                let face_visibility =
                    parse_optional_face_visibility_cells(row.get(19), row.get(20))?;
                let declared_entity =
                    parse_required_string_cell(row.get(12), "declared_entity")?.to_string();

                Ok(Some(IfcBodyRecord {
                    product_id,
                    placement_id,
                    item_id,
                    occurrence_id: Some(mapped_item_id),
                    global_id,
                    name,
                    object_type,
                    predefined_type,
                    type_object_type,
                    type_predefined_type,
                    classification_identification,
                    display_color,
                    face_visibility,
                    declared_entity,
                    item_transform: mapped_transform,
                    primitive,
                }))
            })
            .collect::<Result<Vec<_>, VelrIfcError>>()?
            .into_iter()
            .flatten()
            .collect();

        eprintln!(
            "w velr mapped polygonal phase records done rows={}",
            records.len()
        );
        Ok(records)
    }

    fn query_body_sectioned_solid_horizontal_records(
        &self,
    ) -> Result<Vec<IfcBodyRecord>, VelrIfcError> {
        let gradient_curves = self.query_gradient_curve_records()?;
        let polyline_directrices = self.query_polyline_directrix_records()?;
        let profiles_by_item = self.query_sectioned_solid_horizontal_profiles()?;
        let positions_by_item = self.query_sectioned_solid_horizontal_positions()?;

        let rows = self.execute_cypher_rows(
            r#"
MATCH (item:IfcSectionedSolidHorizontal)
MATCH (item)<-[:ITEMS]-(rep:IfcShapeRepresentation)
WHERE rep.RepresentationIdentifier = 'Body'
WITH item, rep
MATCH (rep)<-[:REPRESENTATIONS]-(shape:IfcProductDefinitionShape)
WITH item, shape
MATCH (shape)<-[:REPRESENTATION]-(p:IfcProduct)
MATCH (item)-[:DIRECTRIX]->(directrix)
OPTIONAL MATCH (p)-[:OBJECT_PLACEMENT]->(placement)
OPTIONAL MATCH (p)<-[:RELATED_ELEMENTS]-(:IfcRelContainedInSpatialStructure)-[:RELATING_STRUCTURE]->(:IfcProduct)-[:OBJECT_PLACEMENT]->(container_placement)
OPTIONAL MATCH (p)<-[:RELATED_OBJECTS]-(:IfcRelDefinesByType)-[:RELATING_TYPE]->(type_node)
OPTIONAL MATCH (p)<-[:RELATED_OBJECTS]-(:IfcRelAssociatesClassification)-[:RELATING_CLASSIFICATION]->(classification_ref)
OPTIONAL MATCH (item)<-[:ITEM]-(styled:IfcStyledItem)
OPTIONAL MATCH (styled)-[:STYLES]->(style_assignment:IfcPresentationStyleAssignment)
OPTIONAL MATCH (style_assignment)-[:STYLES]->(assigned_surface_style:IfcSurfaceStyle)
OPTIONAL MATCH (styled)-[:STYLES]->(surface_style:IfcSurfaceStyle)
OPTIONAL MATCH (surface_style)-[:STYLES]->(surface_colour_style)-[:SURFACE_COLOUR]->(rgb:IfcColourRgb)
OPTIONAL MATCH (assigned_surface_style)-[:STYLES]->(assigned_surface_colour_style)-[:SURFACE_COLOUR]->(assigned_rgb:IfcColourRgb)
WITH p, placement, item, directrix,
     head(collect(DISTINCT container_placement)) AS container_placement_id,
     head(collect(DISTINCT { object_type: type_node.ObjectType, predefined_type: type_node.PredefinedType })) AS type_semantics,
     head(collect(DISTINCT classification_ref.Identification)) AS classification_identification,
     head(collect(DISTINCT { red: rgb.Red, green: rgb.Green, blue: rgb.Blue })) AS surface_rgb,
     head(collect(DISTINCT { red: assigned_rgb.Red, green: assigned_rgb.Green, blue: assigned_rgb.Blue })) AS assigned_surface_rgb
RETURN id(p) AS product_id, id(placement) AS placement_id, container_placement_id, id(item) AS item_id, id(directrix) AS directrix_id, directrix.declared_entity AS directrix_entity, p.GlobalId AS global_id, p.Name AS name, p.ObjectType AS object_type, p.PredefinedType AS predefined_type, type_semantics.object_type AS type_object_type, type_semantics.predefined_type AS type_predefined_type, classification_identification, p.declared_entity AS declared_entity, surface_rgb.red AS style_red, surface_rgb.green AS style_green, surface_rgb.blue AS style_blue, assigned_surface_rgb.red AS assigned_style_red, assigned_surface_rgb.green AS assigned_style_green, assigned_surface_rgb.blue AS assigned_style_blue
ORDER BY item_id
"#,
        )?;

        rows.rows
            .into_iter()
            .map(|row| {
                let product_id = parse_u64_cell(row.first(), "product_id")?;
                let placement_id = parse_optional_u64_cell(row.get(1), "placement_id")?;
                let container_placement_id =
                    parse_optional_node_identity_cell(row.get(2), "container_placement_id")?;
                let placement_id = placement_id.or(container_placement_id);
                let item_id = parse_u64_cell(row.get(3), "item_id")?;
                let directrix_id = parse_u64_cell(row.get(4), "directrix_id")?;
                let directrix_entity =
                    parse_required_string_cell(row.get(5), "directrix_entity")?;
                match directrix_entity {
                    "IfcGradientCurve" | "IfcPolyline" => {}
                    other => {
                        return Err(VelrIfcError::IfcGeometryData(format!(
                            "IfcSectionedSolidHorizontal item `{item_id}` has unsupported explicit DIRECTRIX `{other}`"
                        )));
                    }
                }
                let profiles = profiles_by_item.get(&item_id).ok_or_else(|| {
                    VelrIfcError::IfcGeometryData(format!(
                        "IfcSectionedSolidHorizontal item `{item_id}` has product metadata but no explicit CROSS_SECTIONS profiles"
                    ))
                })?;
                let positions = positions_by_item.get(&item_id).ok_or_else(|| {
                    VelrIfcError::IfcGeometryData(format!(
                        "IfcSectionedSolidHorizontal item `{item_id}` has product metadata but no explicit CROSS_SECTION_POSITIONS"
                    ))
                })?;
                if positions
                    .iter()
                    .any(|position| position.curve_id != directrix_id || position.curve_entity != directrix_entity)
                {
                    return Err(VelrIfcError::IfcGeometryData(format!(
                        "IfcSectionedSolidHorizontal item `{item_id}` has section positions on a curve different from its explicit DIRECTRIX `{directrix_id}`"
                    )));
                }

                let primitive = Arc::new(GeometryPrimitive::Tessellated(
                    sectioned_solid_horizontal_geometry(
                        item_id,
                        profiles,
                        positions,
                        &gradient_curves,
                        &polyline_directrices,
                    )?,
                ));
                let display_color = parse_optional_db_style_color_cells(
                    row.get(14),
                    row.get(15),
                    row.get(16),
                    row.get(17),
                    row.get(18),
                    row.get(19),
                )?;
                let declared_entity =
                    parse_required_string_cell(row.get(13), "declared_entity")?.to_string();

                Ok(IfcBodyRecord {
                    product_id,
                    placement_id,
                    item_id,
                    occurrence_id: None,
                    global_id: parse_optional_string_cell(row.get(6)),
                    name: parse_optional_string_cell(row.get(7)),
                    object_type: parse_optional_string_cell(row.get(8)),
                    predefined_type: parse_optional_string_cell(row.get(9)),
                    type_object_type: parse_optional_string_cell(row.get(10)),
                    type_predefined_type: parse_optional_string_cell(row.get(11)),
                    classification_identification: parse_optional_string_cell(row.get(12)),
                    display_color,
                    face_visibility: FaceVisibility::OneSided,
                    declared_entity,
                    item_transform: DMat4::IDENTITY,
                    primitive,
                })
            })
            .collect()
    }

    fn query_sectioned_solid_horizontal_profiles(
        &self,
    ) -> Result<HashMap<u64, Vec<IfcSectionedSolidProfile>>, VelrIfcError> {
        let rows = self.execute_cypher_rows(
            r#"
MATCH (item:IfcSectionedSolidHorizontal)-[section_edge:CROSS_SECTIONS]->(profile:IfcDerivedProfileDef)
MATCH (profile)-[:PARENT_PROFILE]->(:IfcArbitraryClosedProfileDef)-[:OUTER_CURVE]->(:IfcIndexedPolyCurve)-[:POINTS]->(point_list:IfcCartesianPointList2D)
MATCH (profile)-[:OPERATOR]->(operator:IfcCartesianTransformationOperator2D)
OPTIONAL MATCH (operator)-[:LOCAL_ORIGIN]->(origin:IfcCartesianPoint)
OPTIONAL MATCH (operator)-[:AXIS1]->(axis1:IfcDirection)
OPTIONAL MATCH (operator)-[:AXIS2]->(axis2:IfcDirection)
RETURN id(item) AS item_id, section_edge.ordinal AS section_ordinal, id(profile) AS profile_id, point_list.CoordList AS coord_list, operator.Scale AS scale, operator.Scale2 AS scale2, origin.Coordinates AS origin, axis1.DirectionRatios AS axis1, axis2.DirectionRatios AS axis2
ORDER BY item_id, section_ordinal
"#,
        )?;

        let mut by_item = HashMap::<u64, Vec<IfcSectionedSolidProfile>>::new();
        for row in rows.rows {
            let item_id = parse_u64_cell(row.first(), "item_id")?;
            let ordinal = parse_u64_cell(row.get(1), "section_ordinal")?;
            let profile_id = parse_u64_cell(row.get(2), "profile_id")?;
            let raw_points = parse_dvec2_rows_json(
                parse_required_string_cell(row.get(3), "coord_list")?,
                "coord_list",
            )?;
            let scale = parse_optional_f64_cell(row.get(4), "scale")?.unwrap_or(1.0);
            let scale2 = parse_optional_f64_cell(row.get(5), "scale2")?.unwrap_or(scale);
            let origin = parse_optional_dvec2_cell(row.get(6), "origin")?.unwrap_or(DVec2::ZERO);
            let axis1 = parse_optional_dvec2_cell(row.get(7), "axis1")?.unwrap_or(DVec2::X);
            let axis2 = parse_optional_dvec2_cell(row.get(8), "axis2")?.unwrap_or(DVec2::Y);
            let x_axis = normalized_2d_or(axis1, DVec2::X);
            let y_axis = normalized_2d_or(axis2, DVec2::Y);
            let mut points = raw_points
                .into_iter()
                .map(|point| origin + x_axis * (point.x * scale) + y_axis * (point.y * scale2))
                .collect::<Vec<_>>();
            points = normalize_profile_points(points);
            if points.len() < 3 {
                return Err(VelrIfcError::IfcGeometryData(format!(
                    "IfcDerivedProfileDef `{profile_id}` for IfcSectionedSolidHorizontal item `{item_id}` has fewer than three explicit profile points"
                )));
            }
            by_item
                .entry(item_id)
                .or_default()
                .push(IfcSectionedSolidProfile { ordinal, points });
        }

        for (item_id, profiles) in &mut by_item {
            profiles.sort_by_key(|profile| profile.ordinal);
            assert_unique_section_ordinals(
                *item_id,
                "CROSS_SECTIONS",
                profiles.iter().map(|p| p.ordinal),
            )?;
        }

        Ok(by_item)
    }

    fn query_sectioned_solid_horizontal_positions(
        &self,
    ) -> Result<HashMap<u64, Vec<IfcSectionedSolidPosition>>, VelrIfcError> {
        let rows = self.execute_cypher_rows(
            r#"
MATCH (item:IfcSectionedSolidHorizontal)-[position_edge:CROSS_SECTION_POSITIONS]->(:IfcAxis2PlacementLinear)-[:LOCATION]->(expression:IfcPointByDistanceExpression)-[:DISTANCE_ALONG]->(distance)
MATCH (expression)-[:BASIS_CURVE]->(curve)
	RETURN id(item) AS item_id, position_edge.ordinal AS position_ordinal, curve.declared_entity AS curve_entity, id(curve) AS curve_id, distance.payload_value AS distance_along, expression.OffsetLongitudinal AS offset_longitudinal, expression.OffsetLateral AS offset_lateral, expression.OffsetVertical AS offset_vertical
ORDER BY item_id, position_ordinal
"#,
        )?;

        let mut by_item = HashMap::<u64, Vec<IfcSectionedSolidPosition>>::new();
        for row in rows.rows {
            let item_id = parse_u64_cell(row.first(), "item_id")?;
            let position_ordinal = parse_u64_cell(row.get(1), "position_ordinal")?;
            validate_sectioned_solid_horizontal_longitudinal_offset(
                item_id,
                position_ordinal,
                parse_optional_f64_cell(row.get(5), "offset_longitudinal")?,
            )?;
            by_item
                .entry(item_id)
                .or_default()
                .push(IfcSectionedSolidPosition {
                    ordinal: position_ordinal,
                    curve_entity: parse_required_string_cell(row.get(2), "curve_entity")?
                        .to_string(),
                    curve_id: parse_u64_cell(row.get(3), "curve_id")?,
                    distance_along: parse_f64_cell(row.get(4), "distance_along")?,
                    offset_lateral: parse_optional_f64_cell(row.get(6), "offset_lateral")?
                        .unwrap_or(0.0),
                    offset_vertical: parse_optional_f64_cell(row.get(7), "offset_vertical")?
                        .unwrap_or(0.0),
                });
        }

        for (item_id, positions) in &mut by_item {
            positions.sort_by_key(|position| position.ordinal);
            assert_unique_section_ordinals(
                *item_id,
                "CROSS_SECTION_POSITIONS",
                positions.iter().map(|p| p.ordinal),
            )?;
        }

        Ok(by_item)
    }

    fn query_placement_parent_map(&self) -> Result<HashMap<u64, u64>, VelrIfcError> {
        let rows = self.execute_cypher_rows(
            r#"
MATCH (lp:IfcLocalPlacement)-[:PLACEMENT_REL_TO]->(parent)
RETURN id(lp) AS placement_id, id(parent) AS parent_placement_id
ORDER BY placement_id
"#,
        )?;

        rows.rows
            .into_iter()
            .map(|row| {
                Ok((
                    parse_u64_cell(row.first(), "placement_id")?,
                    parse_u64_cell(row.get(1), "parent_placement_id")?,
                ))
            })
            .collect()
    }

    fn query_aggregate_child_parent_placement_map_for_products(
        &self,
        product_ids: &HashSet<u64>,
    ) -> Result<HashMap<u64, u64>, VelrIfcError> {
        let mut placement_by_product = HashMap::new();
        let mut product_ids = product_ids.iter().copied().collect::<Vec<_>>();
        product_ids.sort_unstable();

        for product_id_chunk in product_ids.chunks(512) {
            let product_id_list = product_id_chunk
                .iter()
                .map(u64::to_string)
                .collect::<Vec<_>>()
                .join(",");
            let rows = self.execute_cypher_rows(&format!(
                r#"
MATCH (product:IfcProduct)
WHERE id(product) IN [{product_id_list}]
MATCH (product)<-[:RELATED_OBJECTS]-(:IfcRelAggregates)-[:RELATING_OBJECT]->(parent:IfcProduct)-[:OBJECT_PLACEMENT]->(parent_placement)
RETURN id(product) AS product_id, id(parent_placement) AS parent_placement_id
ORDER BY product_id, parent_placement_id
"#,
            ))?;

            for row in rows.rows {
                placement_by_product.insert(
                    parse_u64_cell(row.first(), "product_id")?,
                    parse_u64_cell(row.get(1), "parent_placement_id")?,
                );
            }
        }

        Ok(placement_by_product)
    }

    fn query_local_placement_records(&self) -> Result<Vec<IfcLocalPlacementRecord>, VelrIfcError> {
        let placement_rows = self.execute_cypher_rows(
            r#"
MATCH (lp:IfcLocalPlacement)
RETURN id(lp) AS placement_id
ORDER BY placement_id
"#,
        )?;
        let mut records_by_id = placement_rows
            .rows
            .into_iter()
            .map(|row| {
                let placement_id = parse_u64_cell(row.first(), "placement_id")?;
                Ok((
                    placement_id,
                    IfcLocalPlacementRecord {
                        placement_id,
                        parent: None,
                        relative_location: None,
                        axis: None,
                        ref_direction: None,
                    },
                ))
            })
            .collect::<Result<HashMap<_, _>, VelrIfcError>>()?;

        let parent_rows = self.execute_cypher_rows(
            r#"
MATCH (lp:IfcLocalPlacement)-[:PLACEMENT_REL_TO]->(parent)
RETURN id(lp) AS placement_id, parent.declared_entity AS parent_entity, id(parent) AS parent_placement_id
"#,
        )?;
        for row in parent_rows.rows {
            let placement_id = parse_u64_cell(row.first(), "placement_id")?;
            let parent_entity = parse_required_string_cell(row.get(1), "parent_entity")?;
            let parent_placement_id = parse_u64_cell(row.get(2), "parent_placement_id")?;
            let Some(record) = records_by_id.get_mut(&placement_id) else {
                return Err(VelrIfcError::IfcGeometryData(format!(
                    "IfcLocalPlacement `{placement_id}` has parent placement data but was missing from the placement scan"
                )));
            };
            record.parent = match parent_entity {
                "IfcLocalPlacement" => Some(IfcPlacementParent::Local(parent_placement_id)),
                "IfcLinearPlacement" => Some(IfcPlacementParent::Linear(parent_placement_id)),
                other => {
                    return Err(VelrIfcError::IfcGeometryData(format!(
                        "IfcLocalPlacement `{placement_id}` references unsupported placement parent `{other}`"
                    )));
                }
            };
        }

        let relative_rows = self.execute_cypher_rows(
            r#"
MATCH (lp:IfcLocalPlacement)-[:RELATIVE_PLACEMENT]->(relative:IfcAxis2Placement3D)
RETURN id(lp) AS placement_id, id(relative) AS relative_placement_id
"#,
        )?;
        let mut relative_by_placement = HashMap::new();
        for row in relative_rows.rows {
            let placement_id = parse_u64_cell(row.first(), "placement_id")?;
            let relative_placement_id = parse_u64_cell(row.get(1), "relative_placement_id")?;
            relative_by_placement.insert(placement_id, relative_placement_id);
        }

        let location_by_relative =
            self.query_axis2_placement_vector_map("LOCATION", "IfcCartesianPoint", "Coordinates")?;
        let axis_by_relative =
            self.query_axis2_placement_vector_map("AXIS", "IfcDirection", "DirectionRatios")?;
        let ref_direction_by_relative = self.query_axis2_placement_vector_map(
            "REF_DIRECTION",
            "IfcDirection",
            "DirectionRatios",
        )?;

        for (placement_id, relative_placement_id) in relative_by_placement {
            let Some(record) = records_by_id.get_mut(&placement_id) else {
                return Err(VelrIfcError::IfcGeometryData(format!(
                    "IfcLocalPlacement `{placement_id}` has relative placement data but was missing from the placement scan"
                )));
            };
            record.relative_location = location_by_relative.get(&relative_placement_id).copied();
            record.axis = axis_by_relative.get(&relative_placement_id).copied();
            record.ref_direction = ref_direction_by_relative
                .get(&relative_placement_id)
                .copied();
        }

        let mut records = records_by_id.into_values().collect::<Vec<_>>();
        records.sort_by_key(|record| record.placement_id);
        Ok(records)
    }

    fn query_linear_placement_records(
        &self,
    ) -> Result<HashMap<u64, IfcLinearPlacementRecord>, VelrIfcError> {
        let placement_rows = self.execute_cypher_rows(
            r#"
MATCH (lp:IfcLinearPlacement)
RETURN id(lp) AS placement_id
ORDER BY placement_id
"#,
        )?;
        let mut records_by_id = placement_rows
            .rows
            .into_iter()
            .map(|row| {
                let placement_id = parse_u64_cell(row.first(), "placement_id")?;
                Ok((
                    placement_id,
                    IfcLinearPlacementRecord {
                        parent_local_placement_id: None,
                        curve_id: None,
                        distance_along: 0.0,
                        offset_longitudinal: 0.0,
                        offset_lateral: 0.0,
                        offset_vertical: 0.0,
                    },
                ))
            })
            .collect::<Result<HashMap<_, _>, VelrIfcError>>()?;

        let parent_rows = self.execute_cypher_rows(
            r#"
MATCH (lp:IfcLinearPlacement)-[:PLACEMENT_REL_TO]->(parent:IfcLocalPlacement)
RETURN id(lp) AS placement_id, id(parent) AS parent_placement_id
"#,
        )?;
        for row in parent_rows.rows {
            let placement_id = parse_u64_cell(row.first(), "placement_id")?;
            let parent_placement_id = parse_u64_cell(row.get(1), "parent_placement_id")?;
            let Some(record) = records_by_id.get_mut(&placement_id) else {
                return Err(VelrIfcError::IfcGeometryData(format!(
                    "IfcLinearPlacement `{placement_id}` has parent placement data but was missing from the placement scan"
                )));
            };
            record.parent_local_placement_id = Some(parent_placement_id);
        }

        let location_rows = self.execute_cypher_rows(
            r#"
MATCH (lp:IfcLinearPlacement)-[:RELATIVE_PLACEMENT]->(:IfcAxis2PlacementLinear)-[:LOCATION]->(point:IfcPointByDistanceExpression)-[:BASIS_CURVE]->(curve:IfcGradientCurve)
MATCH (point)-[:DISTANCE_ALONG]->(distance)
RETURN id(lp) AS placement_id, id(curve) AS curve_id, distance.payload_value AS distance_along, point.OffsetLongitudinal AS offset_longitudinal, point.OffsetLateral AS offset_lateral, point.OffsetVertical AS offset_vertical
"#,
        )?;
        for row in location_rows.rows {
            let placement_id = parse_u64_cell(row.first(), "placement_id")?;
            let Some(record) = records_by_id.get_mut(&placement_id) else {
                return Err(VelrIfcError::IfcGeometryData(format!(
                    "IfcLinearPlacement `{placement_id}` has relative placement data but was missing from the placement scan"
                )));
            };
            record.curve_id = Some(parse_u64_cell(row.get(1), "curve_id")?);
            record.distance_along = parse_f64_cell(row.get(2), "distance_along")?;
            record.offset_longitudinal =
                parse_optional_f64_cell(row.get(3), "offset_longitudinal")?.unwrap_or(0.0);
            record.offset_lateral =
                parse_optional_f64_cell(row.get(4), "offset_lateral")?.unwrap_or(0.0);
            record.offset_vertical =
                parse_optional_f64_cell(row.get(5), "offset_vertical")?.unwrap_or(0.0);
        }

        Ok(records_by_id)
    }

    fn query_gradient_curve_records(
        &self,
    ) -> Result<HashMap<u64, IfcGradientCurveRecord>, VelrIfcError> {
        let horizontal_rows = self.execute_cypher_rows(
            r#"
	MATCH (curve:IfcGradientCurve)-[:BASE_CURVE]->(:IfcCompositeCurve)-[segment_edge:SEGMENTS]->(seg:IfcCurveSegment)-[:PLACEMENT]->(place:IfcAxis2Placement2D)-[:LOCATION]->(point:IfcCartesianPoint)
	MATCH (place)-[:REF_DIRECTION]->(dir:IfcDirection)
	OPTIONAL MATCH (seg)-[:SEGMENT_LENGTH]->(length)
	OPTIONAL MATCH (seg)-[:PARENT_CURVE]->(parent_curve)
	RETURN id(curve) AS curve_id, id(seg) AS segment_id, segment_edge.ordinal AS segment_ordinal, point.Coordinates AS start_point, dir.DirectionRatios AS direction, length.payload_value AS segment_length, parent_curve.declared_entity AS parent_curve_entity, parent_curve.Radius AS radius, parent_curve.ClothoidConstant AS clothoid_constant
	ORDER BY curve_id, segment_ordinal, segment_id
	"#,
	        )?;
        let vertical_rows = self.execute_cypher_rows(
	            r#"
	MATCH (curve:IfcGradientCurve)-[segment_edge:SEGMENTS]->(seg:IfcCurveSegment)-[:PLACEMENT]->(place:IfcAxis2Placement2D)-[:LOCATION]->(point:IfcCartesianPoint)
	MATCH (place)-[:REF_DIRECTION]->(dir:IfcDirection)
	OPTIONAL MATCH (seg)-[:SEGMENT_LENGTH]->(length)
	OPTIONAL MATCH (seg)-[:PARENT_CURVE]->(parent_curve)
	RETURN id(curve) AS curve_id, id(seg) AS segment_id, segment_edge.ordinal AS segment_ordinal, point.Coordinates AS start_point, dir.DirectionRatios AS direction, length.payload_value AS segment_length, parent_curve.declared_entity AS parent_curve_entity, parent_curve.Radius AS radius, parent_curve.ClothoidConstant AS clothoid_constant
	ORDER BY curve_id, segment_ordinal, segment_id
	"#,
	        )?;

        #[derive(Clone, Debug)]
        struct SegmentRow {
            segment_id: u64,
            ordinal: u64,
            start_point: DVec2,
            direction: DVec2,
            signed_length: Option<f64>,
            parent_curve_entity: Option<String>,
            radius: Option<f64>,
        }

        let parse_rows_by_curve =
            |rows: CypherQueryResult| -> Result<HashMap<u64, Vec<SegmentRow>>, VelrIfcError> {
                let mut rows_by_curve: HashMap<u64, Vec<SegmentRow>> = HashMap::new();
                for row in rows.rows {
                    let curve_id = parse_u64_cell(row.first(), "curve_id")?;
                    rows_by_curve.entry(curve_id).or_default().push(SegmentRow {
                        segment_id: parse_u64_cell(row.get(1), "segment_id")?,
                        ordinal: parse_u64_cell(row.get(2), "segment_ordinal")?,
                        start_point: parse_dvec2_cell(row.get(3), "start_point")?,
                        direction: normalized_2d_or(
                            parse_dvec2_cell(row.get(4), "direction")?,
                            DVec2::X,
                        ),
                        signed_length: parse_optional_f64_cell(row.get(5), "segment_length")?,
                        parent_curve_entity: parse_optional_string_cell(row.get(6)),
                        radius: parse_optional_f64_cell(row.get(7), "radius")?,
                    });
                }
                Ok(rows_by_curve)
            };

        let mut horizontal_rows_by_curve = parse_rows_by_curve(horizontal_rows)?;
        let mut vertical_rows_by_curve = parse_rows_by_curve(vertical_rows)?;

        let explicit_or_derived_gradient_segment_length = |row: &SegmentRow,
                                                           next: Option<&SegmentRow>,
                                                           use_explicit_station: bool|
         -> Result<f64, VelrIfcError> {
            if let Some(length) = row.signed_length {
                return Ok(length);
            }
            if let Some(next) = next {
                if use_explicit_station {
                    return Ok(next.start_point.x - row.start_point.x);
                }
                let delta = next.start_point - row.start_point;
                let signed = delta.dot(normalized_2d_or(row.direction, DVec2::X));
                return Ok(if signed.abs() > 1.0e-9 {
                    signed
                } else {
                    delta.length()
                });
            }
            if row.parent_curve_entity.as_deref() == Some("IfcLine") {
                return Ok(f64::INFINITY);
            }
            Err(VelrIfcError::IfcGeometryData(format!(
                "IfcCurveSegment `{}` is missing explicit SegmentLength and no following segment provides an explicit span",
                row.segment_id
            )))
        };

        let build_segments = |mut rows: Vec<SegmentRow>,
                              use_explicit_station: bool|
         -> Result<Vec<IfcGradientCurveSegment>, VelrIfcError> {
            rows.sort_by_key(|row| (row.ordinal, row.segment_id));
            if let Some(window) = rows
                .windows(2)
                .find(|window| window[0].ordinal == window[1].ordinal)
            {
                return Err(VelrIfcError::IfcGeometryData(format!(
                    "IfcGradientCurve segment list has duplicate SEGMENTS ordinal `{}` for segment ids `{}` and `{}`",
                    window[0].ordinal, window[0].segment_id, window[1].segment_id
                )));
            }
            let mut cumulative_station = 0.0;
            let mut segments = Vec::with_capacity(rows.len());
            for index in 0..rows.len() {
                let row = &rows[index];
                let next = rows.get(index + 1);
                let signed_length =
                    explicit_or_derived_gradient_segment_length(row, next, use_explicit_station)?;
                let length = signed_length.abs();
                let end_point = next.map(|row| row.start_point);
                let end_direction = next.map(|row| row.direction);
                let kind = match row.parent_curve_entity.as_deref() {
                    Some("IfcCircle") => row
                        .radius
                        .filter(|radius| radius.abs() > 1.0e-9)
                        .map(|radius| IfcGradientCurveSegmentKind::Circular {
                            radius: radius.abs(),
                            turn_sign: choose_circular_segment_turn_sign(
                                row.start_point,
                                row.direction,
                                radius.abs(),
                                length,
                                end_point,
                                signed_length,
                            ),
                        })
                        .unwrap_or(IfcGradientCurveSegmentKind::Line),
                    Some("IfcClothoid") => IfcGradientCurveSegmentKind::Clothoid,
                    _ => IfcGradientCurveSegmentKind::Line,
                };
                let start_station = if use_explicit_station {
                    row.start_point.x
                } else {
                    cumulative_station
                };
                segments.push(IfcGradientCurveSegment {
                    start_station,
                    length,
                    start_point: row.start_point,
                    direction: row.direction,
                    end_point,
                    end_direction,
                    kind,
                });
                cumulative_station += length;
            }
            Ok(segments)
        };

        let mut records = HashMap::with_capacity(horizontal_rows_by_curve.len());
        for (curve_id, horizontal_rows) in horizontal_rows_by_curve.drain() {
            let horizontal_segments = build_segments(horizontal_rows, false)?;
            let vertical_segments = vertical_rows_by_curve
                .remove(&curve_id)
                .map(|rows| build_segments(rows, true))
                .transpose()?
                .unwrap_or_default();
            records.insert(
                curve_id,
                IfcGradientCurveRecord {
                    horizontal_segments,
                    vertical_segments,
                },
            );
        }

        Ok(records)
    }

    fn query_polyline_directrix_records(
        &self,
    ) -> Result<HashMap<u64, IfcPolylineDirectrixRecord>, VelrIfcError> {
        let rows = self.execute_cypher_rows(
            r#"
MATCH (:IfcSectionedSolidHorizontal)-[:DIRECTRIX]->(curve:IfcPolyline)-[point_edge:POINTS]->(point:IfcCartesianPoint)
RETURN id(curve) AS curve_id, point_edge.ordinal AS point_ordinal, point.Coordinates AS coordinates
ORDER BY curve_id, point_ordinal
"#,
        )?;

        let mut points_by_curve = HashMap::<u64, Vec<IfcPolylineDirectrixPoint>>::new();
        for (sequence, row) in rows.rows.into_iter().enumerate() {
            let curve_id = parse_u64_cell(row.first(), "curve_id")?;
            points_by_curve
                .entry(curve_id)
                .or_default()
                .push(IfcPolylineDirectrixPoint {
                    ordinal: parse_optional_u64_cell(row.get(1), "point_ordinal")?,
                    sequence,
                    coordinates: parse_dvec3_cell(row.get(2), "coordinates")?,
                });
        }

        let mut records = HashMap::with_capacity(points_by_curve.len());
        for (curve_id, mut points) in points_by_curve {
            points.sort_by_key(|point| {
                (
                    point.ordinal.unwrap_or(point.sequence as u64),
                    point.sequence,
                )
            });
            let mut coordinates = Vec::with_capacity(points.len());
            for point in points {
                if coordinates.last().is_some_and(|previous: &DVec3| {
                    previous.distance_squared(point.coordinates) <= 1.0e-18
                }) {
                    continue;
                }
                coordinates.push(point.coordinates);
            }
            if coordinates.len() < 2 {
                return Err(VelrIfcError::IfcGeometryData(format!(
                    "IfcPolyline directrix `{curve_id}` for IfcSectionedSolidHorizontal has fewer than two explicit points"
                )));
            }
            records.insert(
                curve_id,
                IfcPolylineDirectrixRecord {
                    points: coordinates,
                },
            );
        }

        Ok(records)
    }

    fn query_axis2_placement_vector_map(
        &self,
        edge_type: &'static str,
        target_label: &'static str,
        target_property: &'static str,
    ) -> Result<HashMap<u64, DVec3>, VelrIfcError> {
        let rows = self.execute_cypher_rows(&format!(
            r#"
MATCH (relative:IfcAxis2Placement3D)-[:{edge_type}]->(target:{target_label})
RETURN id(relative) AS relative_placement_id, target.{target_property} AS vector
"#
        ))?;

        rows.rows
            .into_iter()
            .map(|row| {
                Ok((
                    parse_u64_cell(row.first(), "relative_placement_id")?,
                    parse_dvec3_cell(row.get(1), "vector")?,
                ))
            })
            .collect()
    }

    fn query_axis2_placement_transform_map(&self) -> Result<HashMap<u64, DMat4>, VelrIfcError> {
        let rows = self.execute_cypher_rows(
            r#"
MATCH (relative:IfcAxis2Placement3D)
RETURN id(relative) AS relative_placement_id
"#,
        )?;
        let locations =
            self.query_axis2_placement_vector_map("LOCATION", "IfcCartesianPoint", "Coordinates")?;
        let axes =
            self.query_axis2_placement_vector_map("AXIS", "IfcDirection", "DirectionRatios")?;
        let ref_directions = self.query_axis2_placement_vector_map(
            "REF_DIRECTION",
            "IfcDirection",
            "DirectionRatios",
        )?;

        rows.rows
            .into_iter()
            .map(|row| {
                let relative_id = parse_u64_cell(row.first(), "relative_placement_id")?;
                Ok((
                    relative_id,
                    axis2_placement_transform(
                        locations.get(&relative_id).copied().unwrap_or(DVec3::ZERO),
                        axes.get(&relative_id).copied(),
                        ref_directions.get(&relative_id).copied(),
                    ),
                ))
            })
            .collect()
    }

    fn query_cartesian_operator_vector_map(
        &self,
        edge_type: &'static str,
        target_label: &'static str,
        target_property: &'static str,
    ) -> Result<HashMap<u64, DVec3>, VelrIfcError> {
        let rows = self.execute_cypher_rows(&format!(
            r#"
MATCH (operator:IfcCartesianTransformationOperator3D)-[:{edge_type}]->(target:{target_label})
RETURN id(operator) AS operator_id, target.{target_property} AS vector
"#
        ))?;

        rows.rows
            .into_iter()
            .map(|row| {
                Ok((
                    parse_u64_cell(row.first(), "operator_id")?,
                    parse_dvec3_cell(row.get(1), "vector")?,
                ))
            })
            .collect()
    }

    fn query_cartesian_operator_transform_map(&self) -> Result<HashMap<u64, DMat4>, VelrIfcError> {
        let rows = self.execute_cypher_rows(
            r#"
MATCH (operator:IfcCartesianTransformationOperator3D)
RETURN id(operator) AS operator_id, operator.Scale AS scale, operator.Scale2 AS scale2, operator.Scale3 AS scale3
"#,
        )?;
        let origins = self.query_cartesian_operator_vector_map(
            "LOCAL_ORIGIN",
            "IfcCartesianPoint",
            "Coordinates",
        )?;
        let axis1 =
            self.query_cartesian_operator_vector_map("AXIS1", "IfcDirection", "DirectionRatios")?;
        let axis2 =
            self.query_cartesian_operator_vector_map("AXIS2", "IfcDirection", "DirectionRatios")?;
        let axis3 =
            self.query_cartesian_operator_vector_map("AXIS3", "IfcDirection", "DirectionRatios")?;

        rows.rows
            .into_iter()
            .map(|row| {
                let operator_id = parse_u64_cell(row.first(), "operator_id")?;
                let scale = parse_optional_f64_cell(row.get(1), "scale")?.unwrap_or(1.0);
                let scale2 = parse_optional_f64_cell(row.get(2), "scale2")?.unwrap_or(scale);
                let scale3 = parse_optional_f64_cell(row.get(3), "scale3")?.unwrap_or(scale);
                Ok((
                    operator_id,
                    cartesian_operator_transform(
                        origins.get(&operator_id).copied().unwrap_or(DVec3::ZERO),
                        axis1.get(&operator_id).copied(),
                        axis2.get(&operator_id).copied(),
                        axis3.get(&operator_id).copied(),
                        DVec3::new(scale, scale2, scale3),
                    ),
                ))
            })
            .collect()
    }

    fn query_mapped_item_transform_map(&self) -> Result<HashMap<u64, DMat4>, VelrIfcError> {
        let rows = self.execute_cypher_rows(
            r#"
MATCH (mapped:IfcMappedItem)-[:MAPPING_SOURCE]->(map:IfcRepresentationMap)
WITH mapped, map
MATCH (map)-[:MAPPING_ORIGIN]->(origin:IfcAxis2Placement3D)
WITH mapped, origin
MATCH (mapped)-[:MAPPING_TARGET]->(target:IfcCartesianTransformationOperator3D)
RETURN id(mapped) AS mapped_item_id, id(origin) AS origin_id, id(target) AS target_id
"#,
        )?;
        let origin_transforms = self.query_axis2_placement_transform_map()?;
        let target_transforms = self.query_cartesian_operator_transform_map()?;

        rows.rows
            .into_iter()
            .map(|row| {
                let mapped_item_id = parse_u64_cell(row.first(), "mapped_item_id")?;
                let origin_id = parse_u64_cell(row.get(1), "origin_id")?;
                let target_id = parse_u64_cell(row.get(2), "target_id")?;
                let origin = origin_transforms.get(&origin_id).copied().ok_or_else(|| {
                    VelrIfcError::IfcGeometryData(format!(
                        "IfcRepresentationMap origin `{origin_id}` was missing from Axis2Placement3D transforms"
                    ))
                })?;
                let target = target_transforms.get(&target_id).copied().ok_or_else(|| {
                    VelrIfcError::IfcGeometryData(format!(
                        "IfcMappedItem target `{target_id}` was missing from CartesianTransformationOperator3D transforms"
                    ))
                })?;
                Ok((mapped_item_id, target * origin.inverse()))
            })
            .collect()
    }

    fn resolve_object_placement_transforms_for(
        &self,
        placement_ids: impl IntoIterator<Item = u64>,
    ) -> Result<HashMap<u64, DMat4>, VelrIfcError> {
        let placement_ids = placement_ids.into_iter().collect::<HashSet<_>>();
        if placement_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let placements = self.query_local_placement_records()?;
        let local_by_id = placements
            .into_iter()
            .map(|placement| (placement.placement_id, placement))
            .collect::<HashMap<_, _>>();
        let linear_by_id = self.query_linear_placement_records()?;
        let curves = self.query_gradient_curve_records()?;
        let mut resolved_local = HashMap::with_capacity(local_by_id.len());
        let mut resolved_linear = HashMap::with_capacity(linear_by_id.len());
        let mut visiting_local = HashSet::new();
        let mut visiting_linear = HashSet::new();

        for placement_id in placement_ids {
            if local_by_id.contains_key(&placement_id) {
                resolve_local_placement_transform(
                    placement_id,
                    &local_by_id,
                    &linear_by_id,
                    &curves,
                    &mut resolved_local,
                    &mut resolved_linear,
                    &mut visiting_local,
                    &mut visiting_linear,
                )?;
            } else if linear_by_id.contains_key(&placement_id) {
                resolve_linear_placement_transform(
                    placement_id,
                    &local_by_id,
                    &linear_by_id,
                    &curves,
                    &mut resolved_local,
                    &mut resolved_linear,
                    &mut visiting_local,
                    &mut visiting_linear,
                )?;
            } else {
                return Err(VelrIfcError::IfcGeometryData(format!(
                    "referenced placement `{placement_id}` was not returned by the placement query"
                )));
            }
        }

        resolved_local.extend(resolved_linear);
        Ok(resolved_local)
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
    command.args([
        "-p",
        "ifc-schema-tool",
        "--bin",
        "ifc-schema-tool",
        "--",
        "import-step-into-velr",
    ]);
    command.arg(step_input);
    command.arg(&layout.database);
    if !options.debug_artifacts {
        command.arg("--no-debug-artifacts");
    }

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
    if options.debug_artifacts {
        copy_optional_file(
            import_output_dir.join("import-bundle.json"),
            &layout.import_bundle,
        )?;
        copy_optional_file(
            import_output_dir.join("import.cypher"),
            &layout.import_cypher,
        )?;
        copy_optional_file(import_output_dir.join("issues.json"), &layout.import_issues)?;
    }
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

fn is_ifc_building_element_proxy_helper(record: &IfcBodyRecord) -> bool {
    matches!(
        record.name.as_deref().map(|name| name.trim().to_ascii_lowercase()),
        Some(name) if name == "origin" || name == "geo-reference"
    )
}

fn is_ifc_surface_decal_body(record: &IfcBodyRecord) -> bool {
    record.declared_entity == "IfcSurfaceFeature"
        && record_has_ifc_semantic(record, &["LINEMARKING", "MARKING", "SURFACEMARKING"])
}

fn is_ifc_terrain_body(record: &IfcBodyRecord) -> bool {
    let entity = record.declared_entity.as_str();
    if matches!(
        entity,
        "IfcGeotechnicalStratum" | "IfcSite" | "IfcSurfaceFeature" | "IfcTopographyElement"
    ) {
        return true;
    }

    entity == "IfcGeographicElement"
        && record_has_ifc_semantic(record, &["TERRAIN", "GROUND", "LANDFORM", "SOIL"])
}

fn is_ifc_terrain_feature_body(record: &IfcBodyRecord) -> bool {
    record.declared_entity.starts_with("IfcEarthworks")
        || record_has_ifc_semantic(
            record,
            &["BANK", "BED", "CUT", "DITCH", "FILL", "SLOPE", "TRENCH"],
        )
}

fn is_ifc_water_body(record: &IfcBodyRecord) -> bool {
    if record.declared_entity == "IfcWater" {
        return true;
    }

    record_has_ifc_semantic(
        record,
        &[
            "WATER",
            "WATERBODY",
            "WATER_BODY",
            "RIVER",
            "STREAM",
            "POND",
            "CANAL",
        ],
    )
}

fn is_ifc_vegetation_body(record: &IfcBodyRecord) -> bool {
    record.declared_entity == "IfcGeographicElement"
        && record_has_ifc_semantic(record, &["VEGETATION", "PLANTING", "PLANT", "TREE"])
        && record.classification_identification.is_some()
}

fn is_ifc_vegetation_cover_body(record: &IfcBodyRecord) -> bool {
    record.declared_entity == "IfcGeographicElement"
        && record_has_ifc_semantic(record, &["VEGETATION", "PLANTING", "PLANT", "TREE"])
}

fn record_has_ifc_semantic(record: &IfcBodyRecord, values: &[&str]) -> bool {
    [
        record.type_predefined_type.as_deref(),
        record.predefined_type.as_deref(),
        record.type_object_type.as_deref(),
        record.object_type.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|value| ifc_semantic_matches(value, values))
}

fn ifc_semantic_matches(value: &str, expected_values: &[&str]) -> bool {
    let normalized = normalize_ifc_semantic_value(value);
    expected_values
        .iter()
        .any(|expected| normalized == normalize_ifc_semantic_value(expected))
}

fn normalize_ifc_semantic_value(value: &str) -> String {
    value
        .trim()
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .map(str::to_ascii_uppercase)
        .collect::<Vec<_>>()
        .join("_")
}

fn faceted_brep_solid_limit_clause(limit_items: Option<usize>) -> String {
    match limit_items {
        Some(limit) => format!("WITH solid LIMIT {limit}\n"),
        None => String::new(),
    }
}

fn faceted_brep_geometry_query(limit_items: Option<usize>) -> String {
    format!(
        r#"
MATCH (solid:IfcFacetedBrep)
{}MATCH (solid)-[:OUTER]->(shell)
WITH solid, shell
MATCH (shell)-[face_edge:CFS_FACES]->(face)
WITH solid, face, face_edge
MATCH (face)-[:BOUNDS]->(bound)
WITH solid, face, face_edge, bound
MATCH (bound)-[:BOUND]->(loop)
WITH solid, face, face_edge, loop
MATCH (loop)-[point_edge:POLYGON]->(pt)
RETURN id(solid) AS item_id, id(face) AS face_id, face_edge.ordinal AS face_ordinal, point_edge.ordinal AS point_ordinal, pt.Coordinates AS coordinates
"#,
        faceted_brep_solid_limit_clause(limit_items),
    )
}

fn faceted_brep_metadata_query(limit_items: Option<usize>) -> String {
    format!(
        r#"
MATCH (solid:IfcFacetedBrep)
{}MATCH (solid)<-[:ITEMS]-(rep:IfcShapeRepresentation)
WHERE rep.RepresentationIdentifier = 'Body'
WITH solid, rep
MATCH (rep)<-[:REPRESENTATIONS]-(shape:IfcProductDefinitionShape)
WITH solid, shape
MATCH (shape)<-[:REPRESENTATION]-(p:IfcProduct)
OPTIONAL MATCH (p)-[:OBJECT_PLACEMENT]->(placement)
OPTIONAL MATCH (p)<-[:RELATED_OBJECTS]-(:IfcRelDefinesByType)-[:RELATING_TYPE]->(type_node)
OPTIONAL MATCH (p)<-[:RELATED_OBJECTS]-(:IfcRelAssociatesClassification)-[:RELATING_CLASSIFICATION]->(classification_ref)
OPTIONAL MATCH (solid)<-[:ITEM]-(styled:IfcStyledItem)
OPTIONAL MATCH (styled)-[:STYLES]->(style_assignment:IfcPresentationStyleAssignment)
OPTIONAL MATCH (style_assignment)-[:STYLES]->(assigned_surface_style:IfcSurfaceStyle)
OPTIONAL MATCH (styled)-[:STYLES]->(surface_style:IfcSurfaceStyle)
OPTIONAL MATCH (surface_style)-[:STYLES]->(surface_colour_style)-[:SURFACE_COLOUR]->(rgb:IfcColourRgb)
OPTIONAL MATCH (assigned_surface_style)-[:STYLES]->(assigned_surface_colour_style)-[:SURFACE_COLOUR]->(assigned_rgb:IfcColourRgb)
WITH p, placement, solid,
     head(collect(DISTINCT {{ object_type: type_node.ObjectType, predefined_type: type_node.PredefinedType }})) AS type_semantics,
     head(collect(DISTINCT classification_ref.Identification)) AS classification_identification,
     head(collect(DISTINCT {{ red: rgb.Red, green: rgb.Green, blue: rgb.Blue }})) AS surface_rgb,
     head(collect(DISTINCT {{ red: assigned_rgb.Red, green: assigned_rgb.Green, blue: assigned_rgb.Blue }})) AS assigned_surface_rgb
RETURN id(p) AS product_id, id(placement) AS placement_id, id(solid) AS item_id, p.GlobalId AS global_id, p.Name AS name, p.ObjectType AS object_type, p.PredefinedType AS predefined_type, type_semantics.object_type AS type_object_type, type_semantics.predefined_type AS type_predefined_type, classification_identification, p.declared_entity AS declared_entity, surface_rgb.red AS style_red, surface_rgb.green AS style_green, surface_rgb.blue AS style_blue, assigned_surface_rgb.red AS assigned_style_red, assigned_surface_rgb.green AS assigned_style_green, assigned_surface_rgb.blue AS assigned_style_blue
ORDER BY item_id
"#,
        faceted_brep_solid_limit_clause(limit_items),
    )
}

fn faceted_brep_mapped_metadata_query(limit_items: Option<usize>) -> String {
    format!(
        r#"
MATCH (solid:IfcFacetedBrep)
{}MATCH (solid)<-[:ITEMS]-(source_rep:IfcShapeRepresentation)
WITH solid, source_rep
MATCH (source_rep)<-[:MAPPED_REPRESENTATION]-(map:IfcRepresentationMap)
WITH solid, map
MATCH (map)<-[:MAPPING_SOURCE]-(mapped:IfcMappedItem)
WITH solid, mapped
MATCH (mapped)<-[:ITEMS]-(rep:IfcShapeRepresentation)
WHERE rep.RepresentationIdentifier = 'Body'
WITH solid, mapped, rep
MATCH (rep)<-[:REPRESENTATIONS]-(shape:IfcProductDefinitionShape)
WITH solid, mapped, shape
MATCH (shape)<-[:REPRESENTATION]-(p:IfcProduct)
OPTIONAL MATCH (p)-[:OBJECT_PLACEMENT]->(placement)
OPTIONAL MATCH (p)<-[:RELATED_OBJECTS]-(:IfcRelDefinesByType)-[:RELATING_TYPE]->(type_node)
OPTIONAL MATCH (p)<-[:RELATED_OBJECTS]-(:IfcRelAssociatesClassification)-[:RELATING_CLASSIFICATION]->(classification_ref)
OPTIONAL MATCH (solid)<-[:ITEM]-(styled:IfcStyledItem)
OPTIONAL MATCH (styled)-[:STYLES]->(style_assignment:IfcPresentationStyleAssignment)
OPTIONAL MATCH (style_assignment)-[:STYLES]->(assigned_surface_style:IfcSurfaceStyle)
OPTIONAL MATCH (styled)-[:STYLES]->(surface_style:IfcSurfaceStyle)
OPTIONAL MATCH (surface_style)-[:STYLES]->(surface_colour_style)-[:SURFACE_COLOUR]->(rgb:IfcColourRgb)
OPTIONAL MATCH (assigned_surface_style)-[:STYLES]->(assigned_surface_colour_style)-[:SURFACE_COLOUR]->(assigned_rgb:IfcColourRgb)
WITH mapped, p, placement, solid,
     head(collect(DISTINCT {{ object_type: type_node.ObjectType, predefined_type: type_node.PredefinedType }})) AS type_semantics,
     head(collect(DISTINCT classification_ref.Identification)) AS classification_identification,
     head(collect(DISTINCT {{ red: rgb.Red, green: rgb.Green, blue: rgb.Blue }})) AS surface_rgb,
     head(collect(DISTINCT {{ red: assigned_rgb.Red, green: assigned_rgb.Green, blue: assigned_rgb.Blue }})) AS assigned_surface_rgb
RETURN id(mapped) AS mapped_item_id, id(p) AS product_id, id(placement) AS placement_id, id(solid) AS item_id, p.GlobalId AS global_id, p.Name AS name, p.ObjectType AS object_type, p.PredefinedType AS predefined_type, type_semantics.object_type AS type_object_type, type_semantics.predefined_type AS type_predefined_type, classification_identification, p.declared_entity AS declared_entity, surface_rgb.red AS style_red, surface_rgb.green AS style_green, surface_rgb.blue AS style_blue, assigned_surface_rgb.red AS assigned_style_red, assigned_surface_rgb.green AS assigned_style_green, assigned_surface_rgb.blue AS assigned_style_blue
ORDER BY mapped_item_id, item_id
"#,
        faceted_brep_solid_limit_clause(limit_items),
    )
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
            face_visibility: instance.face_visibility,
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

fn effective_faceted_brep_placement_id(
    product_id: u64,
    placement_id: Option<u64>,
    placement_parent_by_id: &HashMap<u64, u64>,
    aggregate_parent_placement_by_product: &HashMap<u64, u64>,
) -> Option<u64> {
    let Some(child_placement_id) = placement_id else {
        return None;
    };
    let Some(parent_placement_id) = aggregate_parent_placement_by_product.get(&product_id) else {
        return placement_id;
    };
    if placement_parent_by_id.get(&child_placement_id) == Some(parent_placement_id) {
        return Some(*parent_placement_id);
    }

    placement_id
}

fn imported_scene_resource_from_body_records(
    mut records: Vec<IfcBodyRecord>,
    placement_transforms: &HashMap<u64, DMat4>,
    source_space: SourceSpace,
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
            .then_with(|| {
                left.occurrence_id
                    .unwrap_or_default()
                    .cmp(&right.occurrence_id.unwrap_or_default())
            })
            .then_with(|| left.global_id.cmp(&right.global_id))
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.declared_entity.cmp(&right.declared_entity))
    });

    let mut definitions = Vec::new();
    let mut primitive_by_item = HashMap::<u64, Arc<GeometryPrimitive>>::new();

    for record in &records {
        if let Some(existing) = primitive_by_item.get(&record.item_id) {
            if !Arc::ptr_eq(existing, &record.primitive)
                && existing.as_ref() != record.primitive.as_ref()
            {
                return Err(VelrIfcError::IfcGeometryData(format!(
                    "body item {} resolved to inconsistent primitive payloads",
                    record.item_id
                )));
            }
            continue;
        }

        primitive_by_item.insert(record.item_id, Arc::clone(&record.primitive));
        definitions.push(GeometryDefinition {
            id: GeometryDefinitionId(record.item_id),
            primitive: record.primitive.as_ref().clone(),
        });
    }

    let instances = records
        .iter()
        .enumerate()
        .map(
            |(instance_index, record)| -> Result<ImportedGeometryResourceInstance, VelrIfcError> {
                let placement_transform = match record.placement_id {
                    Some(placement_id) => {
                        placement_transforms.get(&placement_id).copied().ok_or_else(|| {
                            VelrIfcError::IfcGeometryData(format!(
                                "body record for product `{}` references unresolved object placement `{placement_id}`",
                                record.product_id
                            ))
                        })?
                    }
                    None => DMat4::IDENTITY,
                };

                Ok(ImportedGeometryResourceInstance {
                    instance: GeometryInstance {
                        id: GeometryInstanceId((instance_index as u64) + 1),
                        definition_id: GeometryDefinitionId(record.item_id),
                        transform: placement_transform * record.item_transform,
                    },
                    element_id: ifc_element_id_for_record(record),
                    external_id: ExternalId::new(external_id_for_body_record(record)),
                    label: record
                        .name
                        .clone()
                        .unwrap_or_else(|| record.declared_entity.clone()),
                    declared_entity: record.declared_entity.clone(),
                    default_render_class: default_render_class_for_ifc_body_record(record),
                    display_color: record.display_color,
                    face_visibility: record.face_visibility,
                })
            },
        )
        .collect::<Result<Vec<_>, VelrIfcError>>()?;

    Ok(ImportedGeometrySceneResource {
        definitions,
        instances,
        source_space,
    })
}

fn body_record_placement_ids(records: &[IfcBodyRecord]) -> Vec<u64> {
    let mut seen = HashSet::new();
    records
        .iter()
        .filter_map(|record| record.placement_id)
        .filter(|placement_id| seen.insert(*placement_id))
        .collect()
}

fn external_id_for_body_record(record: &IfcBodyRecord) -> String {
    let base = record
        .global_id
        .clone()
        .unwrap_or_else(|| record.product_id.to_string());
    if let Some(occurrence_id) = record.occurrence_id {
        format!("{base}/mapped/{occurrence_id}/item/{}", record.item_id)
    } else {
        format!("{base}/item/{}", record.item_id)
    }
}

fn ifc_element_id_for_record(record: &IfcBodyRecord) -> SemanticElementId {
    if let Some(global_id) = &record.global_id {
        SemanticElementId::new(global_id.clone())
    } else {
        SemanticElementId::new(record.product_id.to_string())
    }
}

fn default_render_class_for_ifc_body_record(record: &IfcBodyRecord) -> DefaultRenderClass {
    if is_ifc_surface_decal_body(record) {
        return DefaultRenderClass::SurfaceDecal;
    }
    if is_ifc_water_body(record) {
        return DefaultRenderClass::Water;
    }
    if is_ifc_vegetation_body(record) {
        return DefaultRenderClass::Vegetation;
    }
    if is_ifc_vegetation_cover_body(record) {
        return DefaultRenderClass::VegetationCover;
    }
    if is_ifc_terrain_feature_body(record) {
        return DefaultRenderClass::TerrainFeature;
    }
    if is_ifc_terrain_body(record) {
        return DefaultRenderClass::Terrain;
    }

    match record.declared_entity.as_str() {
        "IfcCourse" => DefaultRenderClass::Course,
        "IfcSpace" => DefaultRenderClass::Space,
        "IfcSpatialZone" => DefaultRenderClass::Zone,
        "IfcBuildingElementProxy" if is_ifc_building_element_proxy_helper(record) => {
            DefaultRenderClass::Helper
        }
        _ => DefaultRenderClass::Physical,
    }
}

fn cached_render_class_name(class: DefaultRenderClass) -> &'static str {
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

fn cached_face_visibility_name(visibility: FaceVisibility) -> &'static str {
    match visibility {
        FaceVisibility::OneSided => "one-sided",
        FaceVisibility::DoubleSided => "double-sided",
    }
}

fn parse_cached_face_visibility(value: &str) -> FaceVisibility {
    match value {
        "one-sided" => FaceVisibility::OneSided,
        "double-sided" => FaceVisibility::DoubleSided,
        other => panic!("unsupported cached face visibility `{other}`"),
    }
}

fn parse_cached_render_class(value: &str) -> DefaultRenderClass {
    match value {
        "physical" => DefaultRenderClass::Physical,
        "course" => DefaultRenderClass::Course,
        "space" => DefaultRenderClass::Space,
        "zone" => DefaultRenderClass::Zone,
        "helper" => DefaultRenderClass::Helper,
        "terrain" => DefaultRenderClass::Terrain,
        "terrain-feature" => DefaultRenderClass::TerrainFeature,
        "vegetation" => DefaultRenderClass::Vegetation,
        "vegetation-cover" => DefaultRenderClass::VegetationCover,
        "water" => DefaultRenderClass::Water,
        "surface-decal" => DefaultRenderClass::SurfaceDecal,
        "other" => DefaultRenderClass::Other,
        _ => DefaultRenderClass::Other,
    }
}

fn phase_timing(name: &'static str, elapsed: Duration, rows: Option<usize>) -> IfcBodyPhaseTiming {
    IfcBodyPhaseTiming {
        name,
        elapsed_ms: elapsed.as_millis(),
        rows,
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
    if value.eq_ignore_ascii_case("null") {
        return Ok(None);
    }
    value.parse().map(Some).map_err(|_| {
        VelrIfcError::IfcGeometryData(format!("failed to parse `{label}` as u64: {value}"))
    })
}

fn parse_optional_node_identity_cell(
    cell: Option<&String>,
    label: &'static str,
) -> Result<Option<u64>, VelrIfcError> {
    let Some(value) = parse_optional_string_cell(cell) else {
        return Ok(None);
    };
    if value.eq_ignore_ascii_case("null") {
        return Ok(None);
    }
    if let Ok(id) = value.parse::<u64>() {
        return Ok(Some(id));
    }

    let json = serde_json::from_str::<JsonValue>(&value).map_err(|_| {
        VelrIfcError::IfcGeometryData(format!(
            "failed to parse `{label}` as node identity: {value}"
        ))
    })?;
    let Some(identity) = json.get("identity") else {
        return Err(VelrIfcError::IfcGeometryData(format!(
            "missing `identity` in `{label}` node value: {value}"
        )));
    };
    if let Some(id) = identity.as_u64() {
        return Ok(Some(id));
    }
    if let Some(id) = identity.as_str().and_then(|identity| identity.parse().ok()) {
        return Ok(Some(id));
    }

    Err(VelrIfcError::IfcGeometryData(format!(
        "failed to parse `{label}.identity` as u64: {value}"
    )))
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

fn parse_optional_f64_cell(
    cell: Option<&String>,
    label: &'static str,
) -> Result<Option<f64>, VelrIfcError> {
    let Some(value) = parse_optional_string_cell(cell) else {
        return Ok(None);
    };
    value.parse().map(Some).map_err(|_| {
        VelrIfcError::IfcGeometryData(format!("failed to parse `{label}` as f64: {value}"))
    })
}

fn validate_sectioned_solid_horizontal_longitudinal_offset(
    item_id: u64,
    position_ordinal: u64,
    value: Option<f64>,
) -> Result<(), VelrIfcError> {
    if value.is_some() {
        return Err(VelrIfcError::IfcGeometryData(format!(
            "IfcSectionedSolidHorizontal `{item_id}` CROSS_SECTION_POSITIONS ordinal `{position_ordinal}` uses `OffsetLongitudinal`, which is forbidden by IFC4X3"
        )));
    }
    Ok(())
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

fn parse_optional_db_style_color_cells(
    red: Option<&String>,
    green: Option<&String>,
    blue: Option<&String>,
    assigned_red: Option<&String>,
    assigned_green: Option<&String>,
    assigned_blue: Option<&String>,
) -> Result<Option<DisplayColor>, VelrIfcError> {
    let direct_color = parse_optional_display_color_cells(red, green, blue)?;
    if direct_color.is_some() {
        return Ok(direct_color);
    }
    parse_optional_display_color_cells(assigned_red, assigned_green, assigned_blue)
}

fn parse_optional_face_visibility_cells(
    side: Option<&String>,
    assigned_side: Option<&String>,
) -> Result<FaceVisibility, VelrIfcError> {
    let side =
        parse_optional_ifc_enum_cell(side).or_else(|| parse_optional_ifc_enum_cell(assigned_side));
    let Some(side) = side else {
        return Ok(FaceVisibility::OneSided);
    };

    match side.as_str() {
        "BOTH" => Ok(FaceVisibility::DoubleSided),
        "POSITIVE" => Ok(FaceVisibility::OneSided),
        other => Err(VelrIfcError::IfcGeometryData(format!(
            "unsupported IfcSurfaceStyle.Side `{other}` for body face visibility"
        ))),
    }
}

fn parse_optional_ifc_enum_cell(cell: Option<&String>) -> Option<String> {
    let value = parse_optional_string_cell(cell)?;
    if value.eq_ignore_ascii_case("null") {
        return None;
    }
    let normalized = value
        .trim_matches('.')
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .to_ascii_uppercase();
    (!normalized.is_empty()).then_some(normalized)
}

fn parse_dvec3_cell(cell: Option<&String>, label: &'static str) -> Result<DVec3, VelrIfcError> {
    let value = parse_required_string_cell(cell, label)?;
    let json = parse_json_value(value, label)?;
    parse_dvec3_json(&json, label)
}

fn parse_dvec2_cell(cell: Option<&String>, label: &'static str) -> Result<DVec2, VelrIfcError> {
    let value = parse_required_string_cell(cell, label)?;
    let json = parse_json_value(value, label)?;
    parse_dvec2_json(&json, label)
}

fn parse_optional_dvec2_cell(
    cell: Option<&String>,
    label: &'static str,
) -> Result<Option<DVec2>, VelrIfcError> {
    let Some(value) = parse_optional_string_cell(cell) else {
        return Ok(None);
    };
    if value.eq_ignore_ascii_case("null") {
        return Ok(None);
    }
    let json = parse_json_value(&value, label)?;
    parse_dvec2_json(&json, label).map(Some)
}

fn tessellated_geometry_from_row(
    coord_list: &str,
    coord_index: &str,
) -> Result<Option<TessellatedGeometry>, VelrIfcError> {
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

    if faces.is_empty() {
        return Ok(None);
    }

    TessellatedGeometry::new(positions, faces)
        .map(Some)
        .map_err(VelrIfcError::from)
}

fn tessellated_geometry_from_coord_index_rows(
    coord_list: &str,
    coord_index_rows: &[(u64, String)],
) -> Result<Option<TessellatedGeometry>, VelrIfcError> {
    let positions = parse_dvec3_rows_json(coord_list, "coord_list")?;
    let mut faces = Vec::with_capacity(coord_index_rows.len());

    for (face_index, (_, coord_index)) in coord_index_rows.iter().enumerate() {
        let face_value = parse_json_value(coord_index, "coord_index")?;
        let face_label = format!("coord_index[{face_index}]");
        let exterior = normalize_index_ring(parse_index_ring_json(&face_value, &face_label)?);
        if exterior.len() < 3 {
            continue;
        }
        faces.push(IndexedPolygon::new(exterior, vec![], positions.len())?);
    }

    if faces.is_empty() {
        return Ok(None);
    }

    TessellatedGeometry::new(positions, faces)
        .map(Some)
        .map_err(VelrIfcError::from)
}

fn sectioned_solid_horizontal_geometry(
    item_id: u64,
    profiles: &[IfcSectionedSolidProfile],
    positions: &[IfcSectionedSolidPosition],
    gradient_curves: &HashMap<u64, IfcGradientCurveRecord>,
    polyline_directrices: &HashMap<u64, IfcPolylineDirectrixRecord>,
) -> Result<TessellatedGeometry, VelrIfcError> {
    if profiles.len() != positions.len() {
        return Err(VelrIfcError::IfcGeometryData(format!(
            "IfcSectionedSolidHorizontal item `{item_id}` has {} CROSS_SECTIONS but {} CROSS_SECTION_POSITIONS",
            profiles.len(),
            positions.len()
        )));
    }
    if profiles.len() < 2 {
        return Err(VelrIfcError::IfcGeometryData(format!(
            "IfcSectionedSolidHorizontal item `{item_id}` needs at least two explicit sections"
        )));
    }

    let vertex_count = profiles[0].points.len();
    if vertex_count < 3 {
        return Err(VelrIfcError::IfcGeometryData(format!(
            "IfcSectionedSolidHorizontal item `{item_id}` first section has fewer than three profile points"
        )));
    }
    for profile in profiles {
        if profile.points.len() != vertex_count {
            return Err(VelrIfcError::IfcGeometryData(format!(
                "IfcSectionedSolidHorizontal item `{item_id}` uses changing profile vertex counts; section {} has {} points, expected {vertex_count}",
                profile.ordinal,
                profile.points.len()
            )));
        }
    }
    let profile_area = signed_profile_area(&profiles[0].points);
    if profile_area.abs() <= 1.0e-12 {
        return Err(VelrIfcError::IfcGeometryData(format!(
            "IfcSectionedSolidHorizontal item `{item_id}` has a degenerate first profile ring"
        )));
    }
    for profile in profiles.iter().skip(1) {
        let area = signed_profile_area(&profile.points);
        if area.abs() <= 1.0e-12 {
            return Err(VelrIfcError::IfcGeometryData(format!(
                "IfcSectionedSolidHorizontal item `{item_id}` has a degenerate profile ring at section {}",
                profile.ordinal
            )));
        }
        if area.signum() != profile_area.signum() {
            return Err(VelrIfcError::IfcGeometryData(format!(
                "IfcSectionedSolidHorizontal item `{item_id}` uses inconsistent profile winding at section {}",
                profile.ordinal
            )));
        }
    }
    let profile_winding_is_clockwise = profile_area < 0.0;

    let mut rings = Vec::with_capacity(profiles.len());
    rings.push(sectioned_solid_ring(
        item_id,
        &profiles[0].points,
        &positions[0],
        gradient_curves,
        polyline_directrices,
    )?);
    for section_index in 0..(profiles.len() - 1) {
        let start_ring = sectioned_solid_ring(
            item_id,
            &profiles[section_index].points,
            &positions[section_index],
            gradient_curves,
            polyline_directrices,
        )?;
        let end_ring = sectioned_solid_ring(
            item_id,
            &profiles[section_index + 1].points,
            &positions[section_index + 1],
            gradient_curves,
            polyline_directrices,
        )?;
        append_sectioned_solid_interval(
            item_id,
            &profiles[section_index].points,
            &positions[section_index],
            &start_ring,
            &profiles[section_index + 1].points,
            &positions[section_index + 1],
            &end_ring,
            gradient_curves,
            polyline_directrices,
            &mut rings,
            0,
        )?;
    }
    let positions_3d = rings
        .iter()
        .flat_map(|ring| ring.positions.iter().copied())
        .collect::<Vec<_>>();
    let section_tangents = rings.iter().map(|ring| ring.tangent).collect::<Vec<_>>();

    let mut faces = Vec::new();
    let mut first_cap = (0..vertex_count)
        .map(|index| u32::try_from(index))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| {
            VelrIfcError::IfcGeometryData(format!(
                "IfcSectionedSolidHorizontal item `{item_id}` exceeds u32 mesh index range"
            ))
        })?;
    let last_offset = (rings.len() - 1) * vertex_count;
    let mut last_cap = (0..vertex_count)
        .map(|index| u32::try_from(last_offset + index))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| {
            VelrIfcError::IfcGeometryData(format!(
                "IfcSectionedSolidHorizontal item `{item_id}` exceeds u32 mesh index range"
            ))
        })?;
    orient_cap_ring(
        &mut first_cap,
        &positions_3d,
        -*section_tangents
            .first()
            .expect("section count was validated"),
    );
    orient_cap_ring(
        &mut last_cap,
        &positions_3d,
        *section_tangents
            .last()
            .expect("section count was validated"),
    );
    faces.push(IndexedPolygon::new(first_cap, vec![], positions_3d.len())?);
    faces.push(IndexedPolygon::new(last_cap, vec![], positions_3d.len())?);

    for section_index in 0..(rings.len() - 1) {
        let current = section_index * vertex_count;
        let next = current + vertex_count;
        for point_index in 0..vertex_count {
            let point_next = (point_index + 1) % vertex_count;
            let a = u32::try_from(current + point_index).map_err(|_| {
                VelrIfcError::IfcGeometryData(format!(
                    "IfcSectionedSolidHorizontal item `{item_id}` exceeds u32 mesh index range"
                ))
            })?;
            let b = u32::try_from(current + point_next).map_err(|_| {
                VelrIfcError::IfcGeometryData(format!(
                    "IfcSectionedSolidHorizontal item `{item_id}` exceeds u32 mesh index range"
                ))
            })?;
            let c = u32::try_from(next + point_next).map_err(|_| {
                VelrIfcError::IfcGeometryData(format!(
                    "IfcSectionedSolidHorizontal item `{item_id}` exceeds u32 mesh index range"
                ))
            })?;
            let d = u32::try_from(next + point_index).map_err(|_| {
                VelrIfcError::IfcGeometryData(format!(
                    "IfcSectionedSolidHorizontal item `{item_id}` exceeds u32 mesh index range"
                ))
            })?;
            let side_triangles = if profile_winding_is_clockwise {
                ([a, c, b], [a, d, c])
            } else {
                ([a, b, c], [a, c, d])
            };
            faces.push(IndexedPolygon::new(
                side_triangles.0.to_vec(),
                vec![],
                positions_3d.len(),
            )?);
            faces.push(IndexedPolygon::new(
                side_triangles.1.to_vec(),
                vec![],
                positions_3d.len(),
            )?);
        }
    }

    TessellatedGeometry::new(positions_3d, faces).map_err(VelrIfcError::from)
}

const SECTIONED_SOLID_DIRECTRIX_CHORD_TOLERANCE: f64 = 0.005;
const SECTIONED_SOLID_MAX_RESAMPLE_DEPTH: usize = 12;

fn append_sectioned_solid_interval(
    item_id: u64,
    start_profile: &[DVec2],
    start_position: &IfcSectionedSolidPosition,
    start_ring: &IfcSectionedSolidRing,
    end_profile: &[DVec2],
    end_position: &IfcSectionedSolidPosition,
    end_ring: &IfcSectionedSolidRing,
    gradient_curves: &HashMap<u64, IfcGradientCurveRecord>,
    polyline_directrices: &HashMap<u64, IfcPolylineDirectrixRecord>,
    rings: &mut Vec<IfcSectionedSolidRing>,
    depth: usize,
) -> Result<(), VelrIfcError> {
    if start_profile.len() != end_profile.len() {
        return Err(VelrIfcError::IfcGeometryData(format!(
            "IfcSectionedSolidHorizontal item `{item_id}` cannot resample between profiles with different vertex counts"
        )));
    }
    if start_position.curve_entity != end_position.curve_entity
        || start_position.curve_id != end_position.curve_id
    {
        return Err(VelrIfcError::IfcGeometryData(format!(
            "IfcSectionedSolidHorizontal item `{item_id}` changes directrix between adjacent cross-section positions"
        )));
    }

    let mid_profile = interpolate_profile_points(start_profile, end_profile, 0.5);
    let mid_position = interpolate_sectioned_solid_position(start_position, end_position, 0.5);
    let mid_ring = sectioned_solid_ring(
        item_id,
        &mid_profile,
        &mid_position,
        gradient_curves,
        polyline_directrices,
    )?;
    let max_chord_error = mid_ring
        .positions
        .iter()
        .zip(start_ring.positions.iter().zip(&end_ring.positions))
        .map(|(mid, (start, end))| mid.distance((*start + *end) * 0.5))
        .fold(0.0, f64::max);

    if max_chord_error > SECTIONED_SOLID_DIRECTRIX_CHORD_TOLERANCE
        && depth < SECTIONED_SOLID_MAX_RESAMPLE_DEPTH
    {
        append_sectioned_solid_interval(
            item_id,
            start_profile,
            start_position,
            start_ring,
            &mid_profile,
            &mid_position,
            &mid_ring,
            gradient_curves,
            polyline_directrices,
            rings,
            depth + 1,
        )?;
        append_sectioned_solid_interval(
            item_id,
            &mid_profile,
            &mid_position,
            &mid_ring,
            end_profile,
            end_position,
            end_ring,
            gradient_curves,
            polyline_directrices,
            rings,
            depth + 1,
        )?;
    } else {
        rings.push(end_ring.clone());
    }

    Ok(())
}

fn interpolate_profile_points(start: &[DVec2], end: &[DVec2], t: f64) -> Vec<DVec2> {
    start.iter().zip(end).map(|(a, b)| a.lerp(*b, t)).collect()
}

fn interpolate_sectioned_solid_position(
    start: &IfcSectionedSolidPosition,
    end: &IfcSectionedSolidPosition,
    t: f64,
) -> IfcSectionedSolidPosition {
    IfcSectionedSolidPosition {
        ordinal: start.ordinal,
        curve_entity: start.curve_entity.clone(),
        curve_id: start.curve_id,
        distance_along: lerp_f64(start.distance_along, end.distance_along, t),
        offset_lateral: lerp_f64(start.offset_lateral, end.offset_lateral, t),
        offset_vertical: lerp_f64(start.offset_vertical, end.offset_vertical, t),
    }
}

fn lerp_f64(start: f64, end: f64, t: f64) -> f64 {
    start + (end - start) * t
}

fn sectioned_solid_ring(
    item_id: u64,
    profile_points: &[DVec2],
    position: &IfcSectionedSolidPosition,
    gradient_curves: &HashMap<u64, IfcGradientCurveRecord>,
    polyline_directrices: &HashMap<u64, IfcPolylineDirectrixRecord>,
) -> Result<IfcSectionedSolidRing, VelrIfcError> {
    let (base_point, tangent) = evaluate_sectioned_solid_directrix(
        item_id,
        position,
        gradient_curves,
        polyline_directrices,
    )?;
    let tangent = normalized_or(tangent, DVec3::X);
    let lateral_axis = DVec3::Z.cross(tangent).normalize_or_zero();
    let lateral_axis = if lateral_axis.length_squared() <= 1.0e-12 {
        DVec3::Y
    } else {
        lateral_axis
    };
    let positions = profile_points
        .iter()
        .map(|point| {
            base_point
                + lateral_axis * (position.offset_lateral + point.x)
                + DVec3::Z * (position.offset_vertical + point.y)
        })
        .collect::<Vec<_>>();

    Ok(IfcSectionedSolidRing { positions, tangent })
}

fn signed_profile_area(points: &[DVec2]) -> f64 {
    if points.len() < 3 {
        return 0.0;
    }

    let mut area = 0.0;
    for index in 0..points.len() {
        let next = (index + 1) % points.len();
        area += points[index].x * points[next].y - points[next].x * points[index].y;
    }
    0.5 * area
}

fn orient_cap_ring(ring: &mut [u32], positions: &[DVec3], expected_normal: DVec3) {
    let Some(normal) = indexed_ring_reference_normal(positions, ring) else {
        return;
    };
    if normal.dot(expected_normal) < 0.0 {
        ring.reverse();
    }
}

fn indexed_ring_reference_normal(positions: &[DVec3], ring: &[u32]) -> Option<DVec3> {
    if ring.len() < 3 {
        return None;
    }

    let origin = positions[*ring.first()? as usize];
    let mut reference_cross = DVec3::ZERO;
    let mut reference_cross_length_squared = 0.0;

    for index in 1..ring.len() - 1 {
        let a = positions[ring[index] as usize] - origin;
        let b = positions[ring[index + 1] as usize] - origin;
        let cross = a.cross(b);
        let cross_length_squared = cross.length_squared();

        if cross_length_squared > reference_cross_length_squared {
            reference_cross = cross;
            reference_cross_length_squared = cross_length_squared;
        }
    }

    if reference_cross_length_squared > 0.0 {
        Some(reference_cross.normalize())
    } else {
        None
    }
}

fn evaluate_sectioned_solid_directrix(
    item_id: u64,
    position: &IfcSectionedSolidPosition,
    gradient_curves: &HashMap<u64, IfcGradientCurveRecord>,
    polyline_directrices: &HashMap<u64, IfcPolylineDirectrixRecord>,
) -> Result<(DVec3, DVec3), VelrIfcError> {
    match position.curve_entity.as_str() {
        "IfcGradientCurve" => {
            let curve = gradient_curves.get(&position.curve_id).ok_or_else(|| {
                VelrIfcError::IfcGeometryData(format!(
                    "IfcSectionedSolidHorizontal item `{item_id}` references IfcGradientCurve `{}`, but no explicit curve record was found",
                    position.curve_id
                ))
            })?;
            curve.evaluate(position.distance_along).ok_or_else(|| {
                VelrIfcError::IfcGeometryData(format!(
                    "IfcSectionedSolidHorizontal item `{item_id}` cannot evaluate explicit IfcGradientCurve directrix `{}` at distance {}",
                    position.curve_id, position.distance_along
                ))
            })
        }
        "IfcPolyline" => {
            let curve = polyline_directrices.get(&position.curve_id).ok_or_else(|| {
                VelrIfcError::IfcGeometryData(format!(
                    "IfcSectionedSolidHorizontal item `{item_id}` references IfcPolyline `{}`, but no explicit polyline record was found",
                    position.curve_id
                ))
            })?;
            curve.evaluate(position.distance_along).ok_or_else(|| {
                VelrIfcError::IfcGeometryData(format!(
                    "IfcSectionedSolidHorizontal item `{item_id}` cannot evaluate explicit IfcPolyline directrix `{}` at distance {}",
                    position.curve_id, position.distance_along
                ))
            })
        }
        other => Err(VelrIfcError::IfcGeometryData(format!(
            "IfcSectionedSolidHorizontal item `{item_id}` has unsupported directrix `{other}`"
        ))),
    }
}

fn normalize_profile_points(mut points: Vec<DVec2>) -> Vec<DVec2> {
    let mut normalized = Vec::with_capacity(points.len());
    for point in points.drain(..) {
        if normalized
            .last()
            .is_some_and(|previous: &DVec2| previous.distance_squared(point) <= 1.0e-18)
        {
            continue;
        }
        normalized.push(point);
    }
    if normalized.len() >= 2
        && normalized
            .first()
            .expect("length was checked")
            .distance_squared(*normalized.last().expect("length was checked"))
            <= 1.0e-18
    {
        normalized.pop();
    }
    normalized
}

fn assert_unique_section_ordinals(
    item_id: u64,
    edge_name: &'static str,
    ordinals: impl IntoIterator<Item = u64>,
) -> Result<(), VelrIfcError> {
    let mut seen = HashSet::new();
    for ordinal in ordinals {
        if !seen.insert(ordinal) {
            return Err(VelrIfcError::IfcGeometryData(format!(
                "IfcSectionedSolidHorizontal item `{item_id}` has duplicate {edge_name} ordinal `{ordinal}`"
            )));
        }
    }
    Ok(())
}

fn faceted_brep_geometry_from_faces(
    faces_by_id: HashMap<u64, FacetedBrepFaceAccumulator>,
) -> Result<TessellatedGeometry, VelrIfcError> {
    let mut face_rows = faces_by_id.into_iter().collect::<Vec<_>>();
    face_rows.sort_by_key(|(face_id, face)| (face.ordinal.unwrap_or(*face_id), *face_id));

    let mut positions = Vec::new();
    let mut faces = Vec::new();

    for (face_id, mut face) in face_rows {
        face.points.sort_by_key(|point| {
            (
                point.ordinal.unwrap_or(point.sequence as u64),
                point.sequence,
            )
        });

        let mut ring = Vec::<DVec3>::with_capacity(face.points.len());
        for point in face.points {
            if ring
                .last()
                .is_some_and(|previous| previous.distance_squared(point.coordinates) <= 1.0e-18)
            {
                continue;
            }
            ring.push(point.coordinates);
        }

        if ring.len() >= 2
            && ring
                .first()
                .expect("ring length was checked")
                .distance_squared(*ring.last().expect("ring length was checked"))
                <= 1.0e-18
        {
            ring.pop();
        }

        if ring.len() < 3 {
            continue;
        }

        let mut exterior = Vec::with_capacity(ring.len());
        for position in ring {
            let index = u32::try_from(positions.len()).map_err(|_| {
                VelrIfcError::IfcGeometryData(format!(
                    "IfcFacetedBrep face `{face_id}` exceeds u32 mesh index range"
                ))
            })?;
            positions.push(position);
            exterior.push(index);
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

fn parse_dvec2_rows_json(text: &str, label: &str) -> Result<Vec<DVec2>, VelrIfcError> {
    let value = parse_json_value(text, label)?;
    let rows = json_array(&value, label)?;
    rows.iter()
        .enumerate()
        .map(|(index, row)| parse_dvec2_json(row, &format!("{label}[{index}]")))
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
    linear_by_id: &HashMap<u64, IfcLinearPlacementRecord>,
    curves: &HashMap<u64, IfcGradientCurveRecord>,
    resolved: &mut HashMap<u64, DMat4>,
    resolved_linear: &mut HashMap<u64, DMat4>,
    visiting: &mut HashSet<u64>,
    visiting_linear: &mut HashSet<u64>,
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
    let world_from_local = if let Some(parent) = placement.parent {
        let parent_world = match parent {
            IfcPlacementParent::Local(parent_placement_id) => resolve_local_placement_transform(
                parent_placement_id,
                by_id,
                linear_by_id,
                curves,
                resolved,
                resolved_linear,
                visiting,
                visiting_linear,
            )?,
            IfcPlacementParent::Linear(parent_placement_id) => resolve_linear_placement_transform(
                parent_placement_id,
                by_id,
                linear_by_id,
                curves,
                resolved,
                resolved_linear,
                visiting,
                visiting_linear,
            )?,
        };
        parent_world * local_from_parent
    } else {
        local_from_parent
    };

    visiting.remove(&placement_id);
    resolved.insert(placement_id, world_from_local);
    Ok(world_from_local)
}

fn resolve_linear_placement_transform(
    placement_id: u64,
    local_by_id: &HashMap<u64, IfcLocalPlacementRecord>,
    by_id: &HashMap<u64, IfcLinearPlacementRecord>,
    curves: &HashMap<u64, IfcGradientCurveRecord>,
    resolved_local: &mut HashMap<u64, DMat4>,
    resolved: &mut HashMap<u64, DMat4>,
    visiting_local: &mut HashSet<u64>,
    visiting: &mut HashSet<u64>,
) -> Result<DMat4, VelrIfcError> {
    if let Some(transform) = resolved.get(&placement_id) {
        return Ok(*transform);
    }

    let placement = by_id.get(&placement_id).ok_or_else(|| {
        VelrIfcError::IfcGeometryData(format!(
            "referenced linear placement `{placement_id}` was not returned by the placement query"
        ))
    })?;

    if !visiting.insert(placement_id) {
        return Err(VelrIfcError::IfcGeometryData(format!(
            "cycle detected while resolving IfcLinearPlacement chain at `{placement_id}`"
        )));
    }

    let local_from_parent = linear_placement_relative_transform(placement, curves)?;
    let world_from_linear = if let Some(parent_placement_id) = placement.parent_local_placement_id {
        let parent_world = resolve_local_placement_transform(
            parent_placement_id,
            local_by_id,
            by_id,
            curves,
            resolved_local,
            resolved,
            visiting_local,
            visiting,
        )?;
        parent_world * local_from_parent
    } else {
        local_from_parent
    };

    visiting.remove(&placement_id);
    resolved.insert(placement_id, world_from_linear);
    Ok(world_from_linear)
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

fn linear_placement_relative_transform(
    placement: &IfcLinearPlacementRecord,
    curves: &HashMap<u64, IfcGradientCurveRecord>,
) -> Result<DMat4, VelrIfcError> {
    let curve_id = placement.curve_id.ok_or_else(|| {
        VelrIfcError::IfcGeometryData(
            "IfcLinearPlacement is missing its explicit PlacementMeasuredAlong curve".to_string(),
        )
    })?;
    let curve = curves.get(&curve_id).ok_or_else(|| {
        VelrIfcError::IfcGeometryData(format!(
            "IfcLinearPlacement references IfcGradientCurve `{curve_id}`, but no explicit BASE_CURVE segments were found"
        ))
    })?;
    let (base_point, tangent) = curve.evaluate(placement.distance_along).ok_or_else(|| {
        VelrIfcError::IfcGeometryData(format!(
            "IfcGradientCurve `{curve_id}` has no evaluable explicit BASE_CURVE and station/elevation segments"
        ))
    })?;
    let tangent = normalized_or(tangent, DVec3::X);
    let lateral_axis = DVec3::Z.cross(tangent).normalize_or_zero();
    let lateral_axis = if lateral_axis.length_squared() <= 1.0e-12 {
        DVec3::Y
    } else {
        lateral_axis
    };
    let location = base_point
        + tangent * placement.offset_longitudinal
        + lateral_axis * placement.offset_lateral
        + DVec3::Z * placement.offset_vertical;

    Ok(axis2_placement_transform(
        location,
        Some(DVec3::Z),
        Some(tangent),
    ))
}

impl IfcGradientCurveRecord {
    fn evaluate(&self, distance_along: f64) -> Option<(DVec3, DVec3)> {
        let (point, horizontal_tangent) =
            evaluate_curve_segments(&self.horizontal_segments, distance_along)?;
        let (vertical_point, _vertical_tangent) =
            evaluate_curve_segments(&self.vertical_segments, distance_along)?;
        // Linear placement uses the declared horizontal BASE_CURVE. Its
        // composite curve points are already in model x/y order. The
        // IfcGradientCurve's own segments carry the explicit station/elevation
        // profile, so z comes from that profile rather than an invented flat
        // default.
        Some((
            DVec3::new(point.x, point.y, vertical_point.y),
            horizontal_tangent.extend(0.0),
        ))
    }
}

impl IfcPolylineDirectrixRecord {
    fn evaluate(&self, distance_along: f64) -> Option<(DVec3, DVec3)> {
        let first = *self.points.first()?;
        let last = *self.points.last().unwrap_or(&first);
        if self.points.len() < 2 {
            return None;
        }

        let mut remaining = distance_along.max(0.0);
        for segment in self.points.windows(2) {
            let start = segment[0];
            let end = segment[1];
            let delta = end - start;
            let length = delta.length();
            if length <= 1.0e-12 {
                continue;
            }
            if remaining <= length {
                let tangent = delta / length;
                return Some((start + tangent * remaining, tangent));
            }
            remaining -= length;
        }

        let tangent = self
            .points
            .windows(2)
            .rev()
            .find_map(|segment| {
                let delta = segment[1] - segment[0];
                let length = delta.length();
                (length > 1.0e-12).then_some(delta / length)
            })
            .unwrap_or(DVec3::X);
        Some((last, tangent))
    }
}

fn evaluate_curve_segments(
    segments: &[IfcGradientCurveSegment],
    distance_along: f64,
) -> Option<(DVec2, DVec2)> {
    let first = segments.first()?;
    let last = segments.last().unwrap_or(first);
    let segment = segments
        .iter()
        .find(|segment| segment.contains_distance(distance_along))
        .unwrap_or(last);
    let along = segment.along_distance(distance_along);
    Some(segment.evaluate_2d(along))
}

impl IfcGradientCurveSegment {
    fn contains_distance(&self, distance_along: f64) -> bool {
        if self.length.is_infinite() {
            return distance_along >= self.start_station;
        }
        distance_along <= self.start_station + self.length || self.length <= 1.0e-12
    }

    fn along_distance(&self, distance_along: f64) -> f64 {
        if self.length <= 1.0e-12 {
            0.0
        } else if self.length.is_infinite() {
            (distance_along - self.start_station).max(0.0)
        } else {
            (distance_along - self.start_station).clamp(0.0, self.length)
        }
    }

    fn evaluate_2d(&self, along: f64) -> (DVec2, DVec2) {
        let start_tangent = normalized_2d_or(self.direction, DVec2::X);
        match self.kind {
            IfcGradientCurveSegmentKind::Line => {
                (self.start_point + start_tangent * along, start_tangent)
            }
            IfcGradientCurveSegmentKind::Circular { radius, turn_sign } => {
                if radius <= 1.0e-9 {
                    return (self.start_point + start_tangent * along, start_tangent);
                }
                let turn_sign = if turn_sign < 0.0 { -1.0 } else { 1.0 };
                let left_normal = DVec2::new(-start_tangent.y, start_tangent.x);
                let center = self.start_point + left_normal * radius * turn_sign;
                let radial = self.start_point - center;
                let angle = turn_sign * along / radius;
                let point = center + rotate_2d(radial, angle);
                let tangent = normalized_2d_or(rotate_2d(start_tangent, angle), start_tangent);
                (point, tangent)
            }
            IfcGradientCurveSegmentKind::Clothoid => {
                if let (Some(end_point), Some(end_direction)) = (self.end_point, self.end_direction)
                {
                    let t = if self.length <= 1.0e-12 {
                        0.0
                    } else {
                        (along / self.length).clamp(0.0, 1.0)
                    };
                    cubic_hermite_2d(
                        self.start_point,
                        start_tangent * self.length,
                        end_point,
                        normalized_2d_or(end_direction, start_tangent) * self.length,
                        t,
                    )
                } else {
                    (self.start_point + start_tangent * along, start_tangent)
                }
            }
        }
    }
}

fn choose_circular_segment_turn_sign(
    start_point: DVec2,
    direction: DVec2,
    radius: f64,
    length: f64,
    next_start_point: Option<DVec2>,
    signed_length: f64,
) -> f64 {
    let fallback = if signed_length < 0.0 { -1.0 } else { 1.0 };
    let Some(next_start_point) = next_start_point else {
        return fallback;
    };
    let direction = normalized_2d_or(direction, DVec2::X);
    [-1.0, 1.0]
        .into_iter()
        .min_by(|left, right| {
            let left_point = circular_segment_point(start_point, direction, radius, *left, length);
            let right_point =
                circular_segment_point(start_point, direction, radius, *right, length);
            left_point
                .distance_squared(next_start_point)
                .partial_cmp(&right_point.distance_squared(next_start_point))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or(fallback)
}

fn circular_segment_point(
    start_point: DVec2,
    direction: DVec2,
    radius: f64,
    turn_sign: f64,
    along: f64,
) -> DVec2 {
    if radius <= 1.0e-9 {
        return start_point + normalized_2d_or(direction, DVec2::X) * along;
    }
    let direction = normalized_2d_or(direction, DVec2::X);
    let turn_sign = if turn_sign < 0.0 { -1.0 } else { 1.0 };
    let left_normal = DVec2::new(-direction.y, direction.x);
    let center = start_point + left_normal * radius * turn_sign;
    let radial = start_point - center;
    center + rotate_2d(radial, turn_sign * along / radius)
}

fn rotate_2d(vector: DVec2, angle: f64) -> DVec2 {
    let (sin, cos) = angle.sin_cos();
    DVec2::new(
        vector.x * cos - vector.y * sin,
        vector.x * sin + vector.y * cos,
    )
}

fn cubic_hermite_2d(p0: DVec2, m0: DVec2, p1: DVec2, m1: DVec2, t: f64) -> (DVec2, DVec2) {
    let t2 = t * t;
    let t3 = t2 * t;
    let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
    let h10 = t3 - 2.0 * t2 + t;
    let h01 = -2.0 * t3 + 3.0 * t2;
    let h11 = t3 - t2;
    let point = p0 * h00 + m0 * h10 + p1 * h01 + m1 * h11;

    let dh00 = 6.0 * t2 - 6.0 * t;
    let dh10 = 3.0 * t2 - 4.0 * t + 1.0;
    let dh01 = -6.0 * t2 + 6.0 * t;
    let dh11 = 3.0 * t2 - 2.0 * t;
    let tangent = normalized_2d_or(p0 * dh00 + m0 * dh10 + p1 * dh01 + m1 * dh11, DVec2::X);

    (point, tangent)
}

fn cartesian_operator_transform(
    location: DVec3,
    axis1: Option<DVec3>,
    axis2: Option<DVec3>,
    axis3: Option<DVec3>,
    scale: DVec3,
) -> DMat4 {
    let x_axis = normalized_or(axis1.unwrap_or(DVec3::X), DVec3::X);
    let z_axis = axis3
        .map(|axis| normalized_or(axis, DVec3::Z))
        .unwrap_or_else(|| {
            let y_hint = normalized_or(axis2.unwrap_or(DVec3::Y), DVec3::Y);
            let candidate = x_axis.cross(y_hint);
            normalized_or(candidate, DVec3::Z)
        });
    let mut y_axis = z_axis.cross(x_axis).normalize_or_zero();
    if y_axis.length_squared() <= 1.0e-12 {
        y_axis = normalized_or(axis2.unwrap_or(DVec3::Y), DVec3::Y);
    }
    let z_axis = x_axis.cross(y_axis).normalize_or_zero();
    let z_axis = if z_axis.length_squared() <= 1.0e-12 {
        DVec3::Z
    } else {
        z_axis
    };

    DMat4::from_cols(
        (x_axis * scale.x).extend(0.0),
        (y_axis * scale.y).extend(0.0),
        (z_axis * scale.z).extend(0.0),
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

fn normalized_2d_or(vector: DVec2, fallback: DVec2) -> DVec2 {
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
    let Some(text) = read_text_prefix_if_exists(path, 64 * 1024)? else {
        return Ok(None);
    };

    let Some(schema_key) = text.find("\"schema\"") else {
        return Ok(None);
    };
    let Some(after_colon) = text[schema_key + "\"schema\"".len()..].split_once(':') else {
        return Ok(None);
    };
    let Some(after_quote) = after_colon.1.trim_start().strip_prefix('"') else {
        return Ok(None);
    };
    let Some((schema, _)) = after_quote.split_once('"') else {
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

fn normalize_ifc_token(value: &str) -> String {
    value
        .trim()
        .trim_matches('.')
        .trim_matches('\'')
        .trim_matches('"')
        .to_ascii_uppercase()
}

fn parse_length_unit_from_conversion_name(name: Option<String>) -> Option<LengthUnit> {
    let name = normalize_ifc_token(name.as_deref()?);
    if name.contains("FOOT") || name == "FT" {
        return Some(LengthUnit::Foot);
    }
    if name.contains("INCH") {
        return Some(LengthUnit::Inch);
    }
    None
}

fn parse_length_unit_from_si_unit_cells(
    name: Option<String>,
    prefix: Option<String>,
) -> Option<LengthUnit> {
    let name = normalize_ifc_token(name.as_deref()?);
    if name != "METRE" && name != "METER" {
        return None;
    }

    match prefix
        .as_deref()
        .map(normalize_ifc_token)
        .unwrap_or_default()
        .as_str()
    {
        "" => Some(LengthUnit::Meter),
        "MILLI" => Some(LengthUnit::Millimeter),
        "CENTI" => Some(LengthUnit::Centimeter),
        "KILO" => Some(LengthUnit::Kilometer),
        _ => None,
    }
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

fn diagnose_cached_body_package_from_layout(
    layout: &IfcArtifactLayout,
) -> Result<IfcBodyPackageCacheDiagnostic, VelrIfcError> {
    let mut timings = Vec::new();
    let cache_path = cache_file_path(layout);

    let start = Instant::now();
    let Some(text) = read_text_if_exists(&cache_path)? else {
        timings.push(phase_timing("cache_read_text", start.elapsed(), None));
        return Ok(IfcBodyPackageCacheDiagnostic {
            package: None,
            cache_status: IfcBodyPackageCacheStatus::Miss,
            cache_bytes: None,
            timings,
        });
    };
    let cache_bytes = text.len();
    timings.push(phase_timing(
        "cache_read_text",
        start.elapsed(),
        Some(cache_bytes),
    ));

    let start = Instant::now();
    let Ok(cached) = serde_json::from_str::<CachedPreparedGeometryPackage>(&text) else {
        timings.push(phase_timing("cache_json_parse", start.elapsed(), None));
        return Ok(IfcBodyPackageCacheDiagnostic {
            package: None,
            cache_status: IfcBodyPackageCacheStatus::Miss,
            cache_bytes: Some(cache_bytes),
            timings,
        });
    };
    let parsed_items = cached.definitions.len() + cached.elements.len() + cached.instances.len();
    timings.push(phase_timing(
        "cache_json_parse",
        start.elapsed(),
        Some(parsed_items),
    ));

    let start = Instant::now();
    let valid_cache = cached.cache_version == BODY_PACKAGE_CACHE_VERSION
        && cached.schema == layout.authoritative_schema()?
        && file_fingerprint(&layout.database)? == cached.database;
    timings.push(phase_timing("cache_validate", start.elapsed(), None));
    if !valid_cache {
        return Ok(IfcBodyPackageCacheDiagnostic {
            package: None,
            cache_status: IfcBodyPackageCacheStatus::Miss,
            cache_bytes: Some(cache_bytes),
            timings,
        });
    }

    let start = Instant::now();
    let package = cached.into_prepared_package();
    timings.push(phase_timing(
        "cache_into_prepared_package",
        start.elapsed(),
        Some(package.instance_count()),
    ));

    Ok(IfcBodyPackageCacheDiagnostic {
        package: Some(package),
        cache_status: IfcBodyPackageCacheStatus::Hit,
        cache_bytes: Some(cache_bytes),
        timings,
    })
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
    fn ifc_surface_style_side_both_maps_to_double_sided_face_visibility() {
        let direct_side = ".BOTH.".to_string();

        assert_eq!(
            parse_optional_face_visibility_cells(Some(&direct_side), None).unwrap(),
            FaceVisibility::DoubleSided
        );
    }

    #[test]
    fn ifc_surface_style_missing_or_positive_side_stays_one_sided() {
        let positive_side = "POSITIVE".to_string();

        assert_eq!(
            parse_optional_face_visibility_cells(None, None).unwrap(),
            FaceVisibility::OneSided
        );
        assert_eq!(
            parse_optional_face_visibility_cells(Some(&positive_side), None).unwrap(),
            FaceVisibility::OneSided
        );
    }

    #[test]
    fn unknown_ifc_surface_style_side_fails_loudly() {
        let unknown_side = "SIDEWAYS".to_string();

        assert!(parse_optional_face_visibility_cells(Some(&unknown_side), None).is_err());
    }

    #[test]
    fn negative_ifc_surface_style_side_fails_until_back_side_rendering_is_supported() {
        let negative_side = "NEGATIVE".to_string();

        assert!(parse_optional_face_visibility_cells(Some(&negative_side), None).is_err());
    }

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
    fn helper_proxy_classification_only_marks_origin_and_geo_reference_markers() {
        let helper = |name: &str| IfcBodyRecord {
            product_id: 1,
            placement_id: None,
            item_id: 1,
            occurrence_id: None,
            global_id: None,
            name: Some(name.to_string()),
            object_type: None,
            predefined_type: None,
            type_object_type: None,
            type_predefined_type: None,
            classification_identification: None,
            display_color: None,
            face_visibility: FaceVisibility::OneSided,
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
            )
            .into(),
        };

        assert_eq!(
            default_render_class_for_ifc_body_record(&helper("origin")),
            DefaultRenderClass::Helper
        );
        assert_eq!(
            default_render_class_for_ifc_body_record(&helper("geo-reference")),
            DefaultRenderClass::Helper
        );
        assert_eq!(
            default_render_class_for_ifc_body_record(&helper("underground - road")),
            DefaultRenderClass::Physical
        );
    }

    #[test]
    fn direct_faceted_brep_aggregate_child_uses_aggregate_parent_placement() {
        let placement_parent_by_id = HashMap::from([(20_u64, 10_u64)]);
        let aggregate_parent_placement_by_product = HashMap::from([(7_u64, 10_u64)]);

        assert_eq!(
            effective_faceted_brep_placement_id(
                7,
                Some(20),
                &placement_parent_by_id,
                &aggregate_parent_placement_by_product,
            ),
            Some(10)
        );
    }

    #[test]
    fn direct_faceted_brep_keeps_child_placement_when_aggregate_parent_does_not_match() {
        let placement_parent_by_id = HashMap::from([(20_u64, 11_u64)]);
        let aggregate_parent_placement_by_product = HashMap::from([(7_u64, 10_u64)]);

        assert_eq!(
            effective_faceted_brep_placement_id(
                7,
                Some(20),
                &placement_parent_by_id,
                &aggregate_parent_placement_by_product,
            ),
            Some(20)
        );
    }

    #[test]
    fn direct_faceted_brep_keeps_child_placement_without_aggregate_parent() {
        let placement_parent_by_id = HashMap::from([(20_u64, 10_u64)]);
        let aggregate_parent_placement_by_product = HashMap::new();

        assert_eq!(
            effective_faceted_brep_placement_id(
                7,
                Some(20),
                &placement_parent_by_id,
                &aggregate_parent_placement_by_product,
            ),
            Some(20)
        );
    }

    #[test]
    fn spatial_semantic_bodies_keep_default_view_classification() {
        let semantic_body = |declared_entity: &str| IfcBodyRecord {
            product_id: 1,
            placement_id: None,
            item_id: 1,
            occurrence_id: None,
            global_id: None,
            name: Some("semantic volume".to_string()),
            object_type: None,
            predefined_type: None,
            type_object_type: None,
            type_predefined_type: None,
            classification_identification: None,
            display_color: None,
            face_visibility: FaceVisibility::OneSided,
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
            )
            .into(),
        };

        assert_eq!(
            default_render_class_for_ifc_body_record(&semantic_body("IfcSpace")),
            DefaultRenderClass::Space
        );
        assert_eq!(
            default_render_class_for_ifc_body_record(&semantic_body("IfcSpatialZone")),
            DefaultRenderClass::Zone
        );
        assert_eq!(
            default_render_class_for_ifc_body_record(&semantic_body("IfcWall")),
            DefaultRenderClass::Physical
        );
        assert_eq!(
            default_render_class_for_ifc_body_record(&semantic_body("IfcCourse")),
            DefaultRenderClass::Course
        );
        assert_eq!(
            default_render_class_for_ifc_body_record(&semantic_body("IfcEarthworksFill")),
            DefaultRenderClass::TerrainFeature
        );
        assert_eq!(
            default_render_class_for_ifc_body_record(&semantic_body("IfcGeotechnicalStratum")),
            DefaultRenderClass::Terrain
        );
        assert_eq!(
            default_render_class_for_ifc_body_record(&semantic_body("IfcWater")),
            DefaultRenderClass::Water
        );
    }

    #[test]
    fn terrain_and_water_classification_uses_ifc_semantics_not_display_names() {
        let body_record =
            |declared_entity: &str,
             name: &str,
             object_type: Option<&str>,
             type_predefined_type: Option<&str>,
             classification_identification: Option<&str>| IfcBodyRecord {
                product_id: 1,
                placement_id: None,
                item_id: 1,
                occurrence_id: None,
                global_id: None,
                name: Some(name.to_string()),
                object_type: object_type.map(str::to_string),
                predefined_type: None,
                type_object_type: None,
                type_predefined_type: type_predefined_type.map(str::to_string),
                classification_identification: classification_identification.map(str::to_string),
                display_color: None,
                face_visibility: FaceVisibility::OneSided,
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
                )
                .into(),
            };

        assert_eq!(
            default_render_class_for_ifc_body_record(&body_record(
                "IfcGeographicElement",
                "river stream",
                Some("water"),
                None,
                None
            )),
            DefaultRenderClass::Water
        );
        assert_eq!(
            default_render_class_for_ifc_body_record(&body_record(
                "IfcGeographicElement",
                "river bed",
                Some("terrain"),
                Some("TERRAIN"),
                None
            )),
            DefaultRenderClass::Terrain
        );
        assert_eq!(
            default_render_class_for_ifc_body_record(&body_record(
                "IfcGeographicElement",
                "road river bridge - grass",
                Some("vegetation"),
                Some("VEGETATION"),
                None
            )),
            DefaultRenderClass::VegetationCover
        );
        assert_eq!(
            default_render_class_for_ifc_body_record(&body_record(
                "IfcGeographicElement",
                "tree",
                Some("vegetation"),
                Some("VEGETATION"),
                Some("L-TRA")
            )),
            DefaultRenderClass::Vegetation
        );
        assert_eq!(
            default_render_class_for_ifc_body_record(&body_record(
                "IfcBuildingElementProxy",
                "underground - road",
                Some("terrain"),
                Some("TERRAIN"),
                None
            )),
            DefaultRenderClass::Physical
        );
        assert_eq!(
            default_render_class_for_ifc_body_record(&body_record(
                "IfcSurfaceFeature",
                "road - line marking",
                Some("linemarking"),
                None,
                None
            )),
            DefaultRenderClass::SurfaceDecal
        );
        assert_eq!(
            default_render_class_for_ifc_body_record(&body_record(
                "IfcBuildingElementProxy",
                "underground - river",
                Some("ditch"),
                None,
                None
            )),
            DefaultRenderClass::TerrainFeature
        );
        assert_eq!(
            default_render_class_for_ifc_body_record(&body_record(
                "IfcGeographicElement",
                "tree",
                None,
                None,
                None
            )),
            DefaultRenderClass::Physical
        );
    }

    #[test]
    fn body_scene_resource_exports_hidden_default_classes() {
        let triangle_primitive = || {
            GeometryPrimitive::Tessellated(
                TessellatedGeometry::new(
                    vec![
                        DVec3::ZERO,
                        DVec3::new(1.0, 0.0, 0.0),
                        DVec3::new(0.0, 1.0, 0.0),
                    ],
                    vec![IndexedPolygon::new(vec![0, 1, 2], vec![], 3).expect("triangle")],
                )
                .expect("geometry"),
            )
            .into()
        };
        let body_record =
            |product_id: u64, item_id: u64, declared_entity: &str, name: &str| -> IfcBodyRecord {
                IfcBodyRecord {
                    product_id,
                    placement_id: None,
                    item_id,
                    occurrence_id: None,
                    global_id: Some(format!("global-{product_id}")),
                    name: Some(name.to_string()),
                    object_type: None,
                    predefined_type: None,
                    type_object_type: None,
                    type_predefined_type: None,
                    classification_identification: None,
                    display_color: None,
                    face_visibility: FaceVisibility::OneSided,
                    declared_entity: declared_entity.to_string(),
                    item_transform: DMat4::IDENTITY,
                    primitive: triangle_primitive(),
                }
            };

        let scene = imported_scene_resource_from_body_records(
            vec![
                body_record(1, 11, "IfcWall", "wall"),
                body_record(2, 12, "IfcSpace", "room volume"),
                body_record(3, 13, "IfcBuildingElementProxy", "origin"),
            ],
            &HashMap::new(),
            SourceSpace::w_world_metric(),
        )
        .expect("scene resource");

        assert_eq!(scene.definitions.len(), 3);
        assert_eq!(scene.instances.len(), 3);
        assert_eq!(
            scene
                .instances
                .iter()
                .map(|instance| instance.default_render_class)
                .collect::<Vec<_>>(),
            vec![
                DefaultRenderClass::Physical,
                DefaultRenderClass::Space,
                DefaultRenderClass::Helper
            ]
        );
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
        .expect("tessellated geometry")
        .expect("non-empty tessellated geometry");

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
        .expect("tessellated geometry")
        .expect("non-empty tessellated geometry");

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
        .expect("tessellated geometry")
        .expect("non-empty tessellated geometry");

        assert_eq!(geometry.face_count(), 1);
        assert_eq!(geometry.faces[0].exterior, vec![0, 1, 2]);
    }

    #[test]
    fn tessellated_geometry_parser_keeps_small_faces_for_kernel_policy() {
        let geometry = tessellated_geometry_from_row(
            "[[-603.1602021555037,294.5945945946744,49.999999998914426],[-610.2356451318109,281.3344594595385,49.99999999894908],[-606.8471046702699,286.5920608108901,49.999999998937525]]",
            "[[1,2,3]]",
        )
        .expect("tessellated geometry")
        .expect("non-empty tessellated geometry");

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
                    parent: None,
                    relative_location: Some(DVec3::new(10.0, 0.0, 0.0)),
                    axis: None,
                    ref_direction: None,
                },
            ),
            (
                2_u64,
                IfcLocalPlacementRecord {
                    placement_id: 2,
                    parent: Some(IfcPlacementParent::Local(1)),
                    relative_location: Some(DVec3::new(0.0, 5.0, 0.0)),
                    axis: None,
                    ref_direction: None,
                },
            ),
        ]);
        let linear_by_id = HashMap::new();
        let curves = HashMap::new();
        let mut resolved = HashMap::new();
        let mut resolved_linear = HashMap::new();
        let mut visiting = HashSet::new();
        let mut visiting_linear = HashSet::new();

        let transform = resolve_local_placement_transform(
            2,
            &by_id,
            &linear_by_id,
            &curves,
            &mut resolved,
            &mut resolved_linear,
            &mut visiting,
            &mut visiting_linear,
        )
        .expect("placement transform");

        assert_eq!(
            transform.transform_point3(DVec3::ZERO),
            DVec3::new(10.0, 5.0, 0.0)
        );
    }

    #[test]
    fn placement_chain_can_use_linear_parent_stationing() {
        let by_id = HashMap::from([(
            2_u64,
            IfcLocalPlacementRecord {
                placement_id: 2,
                parent: Some(IfcPlacementParent::Linear(10)),
                relative_location: Some(DVec3::new(1.0, 0.0, 0.0)),
                axis: None,
                ref_direction: None,
            },
        )]);
        let linear_by_id = HashMap::from([(
            10_u64,
            IfcLinearPlacementRecord {
                parent_local_placement_id: None,
                curve_id: Some(20),
                distance_along: 25.0,
                offset_longitudinal: 4.0,
                offset_lateral: 2.0,
                offset_vertical: 3.0,
            },
        )]);
        let curves = HashMap::from([(
            20_u64,
            IfcGradientCurveRecord {
                horizontal_segments: vec![IfcGradientCurveSegment {
                    start_station: 0.0,
                    length: 100.0,
                    start_point: DVec2::new(10.0, 20.0),
                    direction: DVec2::X,
                    end_point: None,
                    end_direction: None,
                    kind: IfcGradientCurveSegmentKind::Line,
                }],
                vertical_segments: vec![IfcGradientCurveSegment {
                    start_station: 0.0,
                    length: 100.0,
                    start_point: DVec2::new(0.0, 0.0),
                    direction: DVec2::X,
                    end_point: None,
                    end_direction: None,
                    kind: IfcGradientCurveSegmentKind::Line,
                }],
            },
        )]);
        let mut resolved = HashMap::new();
        let mut resolved_linear = HashMap::new();
        let mut visiting = HashSet::new();
        let mut visiting_linear = HashSet::new();

        let transform = resolve_local_placement_transform(
            2,
            &by_id,
            &linear_by_id,
            &curves,
            &mut resolved,
            &mut resolved_linear,
            &mut visiting,
            &mut visiting_linear,
        )
        .expect("placement transform");

        assert_eq!(
            transform.transform_point3(DVec3::ZERO),
            DVec3::new(40.0, 22.0, 3.0)
        );
    }

    #[test]
    fn gradient_curve_unbounded_line_segment_can_place_known_station() {
        let curve = IfcGradientCurveRecord {
            horizontal_segments: vec![IfcGradientCurveSegment {
                start_station: 0.0,
                length: f64::INFINITY,
                start_point: DVec2::new(100.0, 200.0),
                direction: DVec2::X,
                end_point: None,
                end_direction: None,
                kind: IfcGradientCurveSegmentKind::Line,
            }],
            vertical_segments: vec![IfcGradientCurveSegment {
                start_station: 0.0,
                length: f64::INFINITY,
                start_point: DVec2::new(0.0, 12.0),
                direction: DVec2::X,
                end_point: None,
                end_direction: None,
                kind: IfcGradientCurveSegmentKind::Line,
            }],
        };

        let (point, tangent) = curve.evaluate(35.0).expect("station evaluation");

        assert_eq!(point, DVec3::new(135.0, 200.0, 12.0));
        assert_eq!(tangent, DVec3::X);
    }

    #[test]
    fn empty_triangulated_face_set_is_unsupported_geometry() {
        let geometry =
            tessellated_geometry_from_row("[[0.0,0.0,0.0],[1.0,0.0,0.0],[0.0,1.0,0.0]]", "[]")
                .expect("empty tessellation should parse");

        assert!(geometry.is_none());
    }

    #[test]
    fn empty_polygonal_face_set_is_unsupported_geometry() {
        let geometry = tessellated_geometry_from_coord_index_rows(
            "[[0.0,0.0,0.0],[1.0,0.0,0.0],[0.0,1.0,0.0]]",
            &[(0, "[1,1,2]".to_owned())],
        )
        .expect("empty polygonal tessellation should parse");

        assert!(geometry.is_none());
    }

    #[test]
    fn polygonal_face_set_preserves_authored_winding() {
        let geometry = tessellated_geometry_from_coord_index_rows(
            "[[0.0,0.0,0.0],[0.0,1.0,0.0],[1.0,1.0,0.0],[1.0,0.0,0.0]]",
            &[(0, "[1,2,3,4]".to_owned())],
        )
        .expect("polygonal tessellation should parse")
        .expect("polygonal tessellation should produce geometry");

        assert_eq!(geometry.faces[0].exterior, vec![0, 1, 2, 3]);
    }

    #[test]
    fn sectioned_solid_horizontal_sweeps_matching_profiles_on_gradient_curve() {
        let curves = HashMap::from([(
            7,
            IfcGradientCurveRecord {
                horizontal_segments: vec![IfcGradientCurveSegment {
                    start_station: 0.0,
                    length: 10.0,
                    start_point: DVec2::ZERO,
                    direction: DVec2::Y,
                    end_point: None,
                    end_direction: None,
                    kind: IfcGradientCurveSegmentKind::Line,
                }],
                vertical_segments: vec![IfcGradientCurveSegment {
                    start_station: 0.0,
                    length: 10.0,
                    start_point: DVec2::new(0.0, 5.0),
                    direction: DVec2::X,
                    end_point: None,
                    end_direction: None,
                    kind: IfcGradientCurveSegmentKind::Line,
                }],
            },
        )]);
        let profiles = vec![
            IfcSectionedSolidProfile {
                ordinal: 0,
                points: vec![
                    DVec2::new(-1.0, 0.0),
                    DVec2::new(1.0, 0.0),
                    DVec2::new(1.0, 0.5),
                    DVec2::new(-1.0, 0.5),
                ],
            },
            IfcSectionedSolidProfile {
                ordinal: 1,
                points: vec![
                    DVec2::new(-1.0, 0.0),
                    DVec2::new(1.0, 0.0),
                    DVec2::new(1.0, 0.5),
                    DVec2::new(-1.0, 0.5),
                ],
            },
        ];
        let positions = vec![
            IfcSectionedSolidPosition {
                ordinal: 0,
                curve_entity: "IfcGradientCurve".to_string(),
                curve_id: 7,
                distance_along: 0.0,
                offset_lateral: 0.0,
                offset_vertical: 0.0,
            },
            IfcSectionedSolidPosition {
                ordinal: 1,
                curve_entity: "IfcGradientCurve".to_string(),
                curve_id: 7,
                distance_along: 10.0,
                offset_lateral: 0.0,
                offset_vertical: 0.0,
            },
        ];

        let geometry = sectioned_solid_horizontal_geometry(
            42,
            &profiles,
            &positions,
            &curves,
            &HashMap::new(),
        )
        .unwrap();

        assert_eq!(geometry.positions.len(), 8);
        assert_eq!(geometry.faces.len(), 10);
        assert!((geometry.positions[0] - DVec3::new(1.0, 0.0, 5.0)).length() < 1.0e-9);
        assert!((geometry.positions[1] - DVec3::new(-1.0, 0.0, 5.0)).length() < 1.0e-9);
        assert!((geometry.positions[4] - DVec3::new(1.0, 10.0, 5.0)).length() < 1.0e-9);

        let first_cap_normal =
            indexed_ring_reference_normal(&geometry.positions, &geometry.faces[0].exterior)
                .expect("first cap normal");
        let last_cap_normal =
            indexed_ring_reference_normal(&geometry.positions, &geometry.faces[1].exterior)
                .expect("last cap normal");
        assert!(
            first_cap_normal.dot(DVec3::NEG_Y) > 0.99,
            "first cap must face opposite the start tangent, got {first_cap_normal:?}"
        );
        assert!(
            last_cap_normal.dot(DVec3::Y) > 0.99,
            "last cap must face with the end tangent, got {last_cap_normal:?}"
        );
    }

    #[test]
    fn sectioned_solid_horizontal_resamples_curved_directrix_between_sparse_sections() {
        let arc_length = std::f64::consts::FRAC_PI_2 * 10.0;
        let curves = HashMap::from([(
            7,
            IfcGradientCurveRecord {
                horizontal_segments: vec![IfcGradientCurveSegment {
                    start_station: 0.0,
                    length: arc_length,
                    start_point: DVec2::ZERO,
                    direction: DVec2::X,
                    end_point: None,
                    end_direction: None,
                    kind: IfcGradientCurveSegmentKind::Circular {
                        radius: 10.0,
                        turn_sign: 1.0,
                    },
                }],
                vertical_segments: vec![IfcGradientCurveSegment {
                    start_station: 0.0,
                    length: arc_length,
                    start_point: DVec2::ZERO,
                    direction: DVec2::X,
                    end_point: None,
                    end_direction: None,
                    kind: IfcGradientCurveSegmentKind::Line,
                }],
            },
        )]);
        let profile_points = vec![
            DVec2::new(-0.5, -0.5),
            DVec2::new(0.5, -0.5),
            DVec2::new(0.5, 0.5),
            DVec2::new(-0.5, 0.5),
        ];
        let profiles = vec![
            IfcSectionedSolidProfile {
                ordinal: 0,
                points: profile_points.clone(),
            },
            IfcSectionedSolidProfile {
                ordinal: 1,
                points: profile_points,
            },
        ];
        let positions = vec![
            IfcSectionedSolidPosition {
                ordinal: 0,
                curve_entity: "IfcGradientCurve".to_string(),
                curve_id: 7,
                distance_along: 0.0,
                offset_lateral: 0.0,
                offset_vertical: 0.0,
            },
            IfcSectionedSolidPosition {
                ordinal: 1,
                curve_entity: "IfcGradientCurve".to_string(),
                curve_id: 7,
                distance_along: arc_length,
                offset_lateral: 0.0,
                offset_vertical: 0.0,
            },
        ];

        let geometry = sectioned_solid_horizontal_geometry(
            45,
            &profiles,
            &positions,
            &curves,
            &HashMap::new(),
        )
        .unwrap();

        assert!(
            geometry.positions.len() > 8,
            "curved directrix must be resampled instead of chorded between sparse sections"
        );
        let expected_mid_center =
            DVec3::new(10.0 / 2.0_f64.sqrt(), 10.0 - 10.0 / 2.0_f64.sqrt(), 0.0);
        assert!(
            geometry.positions.chunks(4).any(|ring| {
                let center = ring.iter().copied().sum::<DVec3>() / ring.len() as f64;
                center.distance(expected_mid_center) < 1.0e-6
            }),
            "resampled rings should include the explicit midpoint of the quarter-arc directrix"
        );
    }

    #[test]
    fn sectioned_solid_horizontal_rejects_forbidden_longitudinal_offset() {
        let error = validate_sectioned_solid_horizontal_longitudinal_offset(45, 2, Some(0.0))
            .expect_err("IfcSectionedSolidHorizontal must not accept longitudinal offsets");

        assert!(
            error.to_string().contains("OffsetLongitudinal"),
            "error should name the forbidden offset, got {error}"
        );
    }

    #[test]
    fn sectioned_solid_horizontal_sweeps_matching_profiles_on_polyline_directrix() {
        let polylines = HashMap::from([(
            11,
            IfcPolylineDirectrixRecord {
                points: vec![DVec3::new(2.0, 3.0, 4.0), DVec3::new(2.0, 13.0, 5.0)],
            },
        )]);
        let profiles = vec![
            IfcSectionedSolidProfile {
                ordinal: 0,
                points: vec![
                    DVec2::new(-0.5, 0.0),
                    DVec2::new(0.5, 0.0),
                    DVec2::new(0.5, 0.25),
                    DVec2::new(-0.5, 0.25),
                ],
            },
            IfcSectionedSolidProfile {
                ordinal: 1,
                points: vec![
                    DVec2::new(-0.5, 0.0),
                    DVec2::new(0.5, 0.0),
                    DVec2::new(0.5, 0.25),
                    DVec2::new(-0.5, 0.25),
                ],
            },
        ];
        let segment_length = DVec3::new(0.0, 10.0, 1.0).length();
        let positions = vec![
            IfcSectionedSolidPosition {
                ordinal: 0,
                curve_entity: "IfcPolyline".to_string(),
                curve_id: 11,
                distance_along: 0.0,
                offset_lateral: 0.0,
                offset_vertical: 0.0,
            },
            IfcSectionedSolidPosition {
                ordinal: 1,
                curve_entity: "IfcPolyline".to_string(),
                curve_id: 11,
                distance_along: segment_length,
                offset_lateral: 0.0,
                offset_vertical: 0.0,
            },
        ];

        let geometry = sectioned_solid_horizontal_geometry(
            43,
            &profiles,
            &positions,
            &HashMap::new(),
            &polylines,
        )
        .unwrap();

        assert_eq!(geometry.positions.len(), 8);
        assert_eq!(geometry.faces.len(), 10);
        assert!((geometry.positions[0] - DVec3::new(2.5, 3.0, 4.0)).length() < 1.0e-9);
        assert!((geometry.positions[1] - DVec3::new(1.5, 3.0, 4.0)).length() < 1.0e-9);
        assert!((geometry.positions[4] - DVec3::new(2.5, 13.0, 5.0)).length() < 1.0e-9);

        let tangent = DVec3::new(0.0, 10.0, 1.0).normalize();
        let first_cap_normal =
            indexed_ring_reference_normal(&geometry.positions, &geometry.faces[0].exterior)
                .expect("first cap normal");
        let last_cap_normal =
            indexed_ring_reference_normal(&geometry.positions, &geometry.faces[1].exterior)
                .expect("last cap normal");
        assert!(
            first_cap_normal.dot(-tangent) > 0.9,
            "first cap must face opposite the start tangent, got {first_cap_normal:?}"
        );
        assert!(
            last_cap_normal.dot(tangent) > 0.9,
            "last cap must face with the end tangent, got {last_cap_normal:?}"
        );
    }

    #[test]
    fn sectioned_solid_horizontal_orients_side_faces_for_clockwise_profiles() {
        let polylines = HashMap::from([(
            11,
            IfcPolylineDirectrixRecord {
                points: vec![DVec3::ZERO, DVec3::new(0.0, 10.0, 0.0)],
            },
        )]);
        let clockwise_profile = vec![
            DVec2::new(1.0, 0.0),
            DVec2::new(1.0, -2.0),
            DVec2::new(-1.0, -2.0),
            DVec2::new(-1.0, 0.0),
        ];
        assert!(signed_profile_area(&clockwise_profile) < 0.0);
        let profiles = vec![
            IfcSectionedSolidProfile {
                ordinal: 0,
                points: clockwise_profile.clone(),
            },
            IfcSectionedSolidProfile {
                ordinal: 1,
                points: clockwise_profile,
            },
        ];
        let positions = vec![
            IfcSectionedSolidPosition {
                ordinal: 0,
                curve_entity: "IfcPolyline".to_string(),
                curve_id: 11,
                distance_along: 0.0,
                offset_lateral: 0.0,
                offset_vertical: 0.0,
            },
            IfcSectionedSolidPosition {
                ordinal: 1,
                curve_entity: "IfcPolyline".to_string(),
                curve_id: 11,
                distance_along: 10.0,
                offset_lateral: 0.0,
                offset_vertical: 0.0,
            },
        ];

        let geometry = sectioned_solid_horizontal_geometry(
            44,
            &profiles,
            &positions,
            &HashMap::new(),
            &polylines,
        )
        .unwrap();

        let first_cap_normal =
            indexed_ring_reference_normal(&geometry.positions, &geometry.faces[0].exterior)
                .expect("first cap normal");
        let last_cap_normal =
            indexed_ring_reference_normal(&geometry.positions, &geometry.faces[1].exterior)
                .expect("last cap normal");
        assert!(
            first_cap_normal.dot(DVec3::NEG_Y) > 0.99,
            "first cap must face opposite the start tangent, got {first_cap_normal:?}"
        );
        assert!(
            last_cap_normal.dot(DVec3::Y) > 0.99,
            "last cap must face with the end tangent, got {last_cap_normal:?}"
        );

        let bottom_face_normal =
            indexed_ring_reference_normal(&geometry.positions, &geometry.faces[4].exterior)
                .expect("bottom face normal");
        assert!(
            bottom_face_normal.dot(DVec3::NEG_Z) > 0.99,
            "clockwise profile bottom face must point down, got {bottom_face_normal:?}"
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
    fn database_length_unit_parsers_detect_feet_meters_and_millimeters() {
        assert_eq!(
            parse_length_unit_from_conversion_name(Some("FOOT".to_string())),
            Some(LengthUnit::Foot)
        );
        assert_eq!(
            parse_length_unit_from_si_unit_cells(Some("METRE".to_string()), None),
            Some(LengthUnit::Meter)
        );
        assert_eq!(
            parse_length_unit_from_si_unit_cells(
                Some("METRE".to_string()),
                Some("MILLI".to_string())
            ),
            Some(LengthUnit::Millimeter)
        );
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
        let primitive: Arc<GeometryPrimitive> = GeometryPrimitive::Tessellated(
            TessellatedGeometry::new(
                vec![
                    DVec3::ZERO,
                    DVec3::new(1.0, 0.0, 0.0),
                    DVec3::new(0.0, 1.0, 0.0),
                ],
                vec![IndexedPolygon::new(vec![0, 1, 2], vec![], 3).expect("triangle")],
            )
            .expect("geometry"),
        )
        .into();
        let records = vec![
            IfcBodyRecord {
                product_id: 10,
                placement_id: Some(1),
                item_id: 77,
                occurrence_id: None,
                global_id: Some("product-a".to_string()),
                name: Some("Shared A".to_string()),
                object_type: None,
                predefined_type: None,
                type_object_type: None,
                type_predefined_type: None,
                classification_identification: None,
                display_color: Some(DisplayColor::new(0.95, 0.56, 0.24)),
                face_visibility: FaceVisibility::OneSided,
                declared_entity: "IfcBuildingElementProxy".to_string(),
                item_transform: DMat4::from_translation(DVec3::new(1.5, 0.0, 0.0)),
                primitive: primitive.clone(),
            },
            IfcBodyRecord {
                product_id: 20,
                placement_id: Some(2),
                item_id: 77,
                occurrence_id: Some(200),
                global_id: Some("product-b".to_string()),
                name: Some("Shared B".to_string()),
                object_type: None,
                predefined_type: None,
                type_object_type: None,
                type_predefined_type: None,
                classification_identification: None,
                display_color: Some(DisplayColor::new(0.24, 0.78, 0.55)),
                face_visibility: FaceVisibility::OneSided,
                declared_entity: "IfcBuildingElementProxy".to_string(),
                item_transform: DMat4::from_translation(DVec3::new(0.0, 2.0, 0.0)),
                primitive,
            },
        ];
        let placement_transforms = HashMap::from([
            (1_u64, DMat4::IDENTITY),
            (2_u64, DMat4::from_translation(DVec3::new(5.0, 0.0, 0.0))),
        ]);

        let scene = imported_scene_resource_from_body_records(
            records,
            &placement_transforms,
            SourceSpace::w_world_metric(),
        )
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
    fn faceted_brep_faces_become_tessellated_polygons() {
        let mut faces = HashMap::new();
        let mut face = FacetedBrepFaceAccumulator {
            ordinal: Some(2),
            points: Vec::new(),
        };
        for (ordinal, coordinates) in [
            (0, DVec3::new(0.0, 0.0, 0.0)),
            (1, DVec3::new(1.0, 0.0, 0.0)),
            (2, DVec3::new(1.0, 1.0, 0.0)),
            (3, DVec3::new(0.0, 1.0, 0.0)),
            (4, DVec3::new(0.0, 0.0, 0.0)),
        ] {
            face.points.push(FacetedBrepPoint {
                ordinal: Some(ordinal),
                sequence: face.points.len(),
                coordinates,
            });
        }
        faces.insert(42, face);

        let geometry = faceted_brep_geometry_from_faces(faces).expect("faceted brep tessellation");

        assert_eq!(geometry.positions.len(), 4);
        assert_eq!(geometry.faces.len(), 1);
        assert_eq!(geometry.faces[0].exterior, vec![0, 1, 2, 3]);
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
        let records = handle
            .query_body_triangulated_records()
            .expect("triangulated records");

        for record in records {
            let GeometryPrimitive::Tessellated(geometry) = &*record.primitive else {
                continue;
            };

            let scene = ImportedGeometrySceneResource {
                definitions: vec![GeometryDefinition {
                    id: GeometryDefinitionId(record.item_id),
                    primitive: (*record.primitive).clone(),
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
                    face_visibility: FaceVisibility::OneSided,
                }],
                source_space: SourceSpace::w_world_metric(),
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
                        primitive: GeometryPrimitive::Tessellated(face_geometry).into(),
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
                        face_visibility: FaceVisibility::OneSided,
                    }],
                    source_space: SourceSpace::w_world_metric(),
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
                face_visibility: FaceVisibility::OneSided,
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
