use cc_w_types::{
    Axis3, CircularArc2, CircularArc3, CircularProfileSweep, ConvexPolygon, CurveSegment2,
    CurveSegment3, DefaultRenderClass, DisplayColor, ExternalId, FaceVisibility,
    GeometryDefinition, GeometryDefinitionId, GeometryError, GeometryInstance, GeometryInstanceId,
    GeometryPrimitive, ImportMetadata, IndexedPolygon, LengthUnit, LineSegment2, LineSegment3,
    MeshDocument, Polycurve2, Polycurve3, Profile2, ProfileLoop2, SemanticElementId, SourceSpace,
    SweepPath, SweptSolid, TessellatedGeometry,
};
use glam::{DMat4, DVec2, DVec3};
use std::f64::consts::TAU;
use thiserror::Error;

pub const DEFAULT_DEMO_RESOURCE: &str = "demo/pentagon";

const DEMO_RESOURCES: &[&str] = &[
    "demo/pentagon",
    "demo/mapped-pentagon-pair",
    "demo/triangle",
    "demo/polygon-with-hole",
    "demo/concave-polygon",
    "demo/tilted-quad",
    "demo/extruded-profile",
    "demo/arc-extruded-profile",
    "demo/revolved-solid",
    "demo/circular-profile-sweep",
    "demo/curved-circular-profile-sweep",
    "demo/step-y-up-triangle",
    "demo/step-y-up-mm-triangle",
];

pub fn available_demo_resources() -> &'static [&'static str] {
    DEMO_RESOURCES
}

#[derive(Clone, Debug)]
pub struct GeometryResource {
    pub definition: GeometryDefinition,
    pub instances: Vec<GeometryResourceInstance>,
}

#[derive(Clone, Debug)]
pub struct GeometrySceneResource {
    pub definitions: Vec<GeometryDefinition>,
    pub instances: Vec<GeometryResourceInstance>,
}

#[derive(Clone, Debug)]
pub struct GeometryResourceInstance {
    pub instance: GeometryInstance,
    pub element_id: SemanticElementId,
    pub external_id: ExternalId,
    pub label: String,
    pub declared_entity: String,
    pub default_render_class: DefaultRenderClass,
    pub display_color: Option<DisplayColor>,
    pub face_visibility: FaceVisibility,
}

/// Adapter-facing generic geometry resource that still lives in an external source basis/unit
/// system. This is the import seam that IFC RV, STEP, and other frontends should target.
#[derive(Clone, Debug)]
pub struct ImportedGeometryResource {
    pub definition: GeometryDefinition,
    pub instances: Vec<ImportedGeometryResourceInstance>,
    pub source_space: SourceSpace,
}

#[derive(Clone, Debug)]
pub struct ImportedGeometrySceneResource {
    pub definitions: Vec<GeometryDefinition>,
    pub instances: Vec<ImportedGeometryResourceInstance>,
    pub source_space: SourceSpace,
}

#[derive(Clone, Debug)]
pub struct ImportedGeometryResourceInstance {
    pub instance: GeometryInstance,
    pub element_id: SemanticElementId,
    pub external_id: ExternalId,
    pub label: String,
    pub declared_entity: String,
    pub default_render_class: DefaultRenderClass,
    pub display_color: Option<DisplayColor>,
    pub face_visibility: FaceVisibility,
}

/// Backend-side repository boundary for geometry and scene projection data.
///
/// Native/dev flows may satisfy this in-process. Web production should keep the concrete Velr or
/// service transport behind this seam instead of letting frontend code talk to the data source
/// directly.
pub trait SceneRepository {
    fn load_demo_geometry_resource(
        &self,
        resource: &str,
    ) -> Result<GeometryResource, ResourceError>;

    fn load_demo_geometry_scene(
        &self,
        resource: &str,
    ) -> Result<GeometrySceneResource, ResourceError> {
        self.load_demo_geometry_resource(resource)
            .map(GeometrySceneResource::from)
    }
}

impl From<GeometryResource> for GeometrySceneResource {
    fn from(resource: GeometryResource) -> Self {
        Self {
            definitions: vec![resource.definition],
            instances: resource.instances,
        }
    }
}

impl From<ImportedGeometryResource> for ImportedGeometrySceneResource {
    fn from(resource: ImportedGeometryResource) -> Self {
        Self {
            definitions: vec![resource.definition],
            instances: resource.instances,
            source_space: resource.source_space,
        }
    }
}

/// Adapter-facing payload for geometry that still lives in an external source basis/unit system.
#[derive(Clone, Debug)]
pub struct ImportedPolygonDocument {
    pub external_id: ExternalId,
    pub label: String,
    pub polygon: ConvexPolygon,
    pub transform: DMat4,
    pub source_space: SourceSpace,
}

impl ImportedPolygonDocument {
    pub fn new(
        external_id: impl Into<String>,
        label: impl Into<String>,
        polygon: ConvexPolygon,
        source_space: SourceSpace,
    ) -> Self {
        Self {
            external_id: ExternalId::new(external_id),
            label: label.into(),
            polygon,
            transform: DMat4::IDENTITY,
            source_space,
        }
    }

    pub fn from_vertices(
        external_id: impl Into<String>,
        label: impl Into<String>,
        vertices: Vec<DVec3>,
        source_space: SourceSpace,
    ) -> Result<Self, GeometryError> {
        Ok(Self::new(
            external_id,
            label,
            ConvexPolygon::new(vertices)?,
            source_space,
        ))
    }

    pub fn with_transform(mut self, transform: DMat4) -> Self {
        self.transform = transform;
        self
    }

    fn into_source_document(self) -> MeshDocument {
        MeshDocument {
            external_id: self.external_id,
            label: self.label,
            polygon: self.polygon,
            transform: self.transform,
            import: ImportMetadata::from_source(self.source_space),
        }
    }
}

#[derive(Debug, Default)]
pub struct InMemoryGraphRepository;

impl InMemoryGraphRepository {
    fn demo_pentagon_document() -> MeshDocument {
        let radius = 2.5_f64;
        let vertices = (0..5)
            .map(|index| {
                let angle = (index as f64 / 5.0) * TAU;
                DVec3::new(radius * angle.cos(), radius * angle.sin(), 0.0)
            })
            .collect();

        polygon_document("demo/pentagon", "Demo Pentagon", vertices)
    }

    fn demo_triangle_document() -> MeshDocument {
        polygon_document(
            "demo/triangle",
            "Demo Triangle",
            vec![
                DVec3::new(-1.8, -1.2, 0.0),
                DVec3::new(1.9, -1.0, 0.0),
                DVec3::new(0.0, 2.2, 0.0),
            ],
        )
    }

    fn demo_tilted_quad_document() -> MeshDocument {
        let z = |x: f64, y: f64| (0.25 * x) + (0.15 * y);

        polygon_document(
            "demo/tilted-quad",
            "Demo Tilted Quad",
            vec![
                DVec3::new(-2.0, -1.0, z(-2.0, -1.0)),
                DVec3::new(2.0, -1.0, z(2.0, -1.0)),
                DVec3::new(2.0, 1.0, z(2.0, 1.0)),
                DVec3::new(-2.0, 1.0, z(-2.0, 1.0)),
            ],
        )
    }

    fn demo_step_y_up_triangle_document() -> MeshDocument {
        // Simulates a right-handed Y-up source adapter normalizing into `w` world space.
        import_polygon_document(
            ImportedPolygonDocument::from_vertices(
                "demo/step-y-up-triangle",
                "Demo STEP Y-Up Triangle",
                vec![
                    DVec3::new(-1.8, 0.0, 1.2),
                    DVec3::new(1.9, 0.0, 1.0),
                    DVec3::new(0.0, 0.0, -2.2),
                ],
                SourceSpace::right_handed_y_up_meters(),
            )
            .expect("valid demo import"),
        )
    }

    fn demo_step_y_up_millimeter_triangle_document() -> MeshDocument {
        // Simulates a STEP-like millimeter source getting normalized into meter-based `w` world space.
        import_polygon_document(
            ImportedPolygonDocument::from_vertices(
                "demo/step-y-up-mm-triangle",
                "Demo STEP Y-Up Millimeter Triangle",
                vec![
                    DVec3::new(-1_800.0, 0.0, 1_200.0),
                    DVec3::new(1_900.0, 0.0, 1_000.0),
                    DVec3::new(0.0, 0.0, -2_200.0),
                ],
                SourceSpace::right_handed_y_up_millimeters(),
            )
            .expect("valid demo import"),
        )
    }

    fn demo_mapped_pentagon_pair_resource() -> GeometryResource {
        let definition_id = GeometryDefinitionId(1);
        let polygon = Self::demo_pentagon_document().polygon;
        let definition = geometry_definition_from_polygon(definition_id, polygon);

        GeometryResource {
            definition,
            instances: vec![
                GeometryResourceInstance {
                    instance: GeometryInstance {
                        id: GeometryInstanceId(1),
                        definition_id,
                        transform: DMat4::from_translation(DVec3::new(-3.5, 0.0, 0.0)),
                    },
                    element_id: SemanticElementId::new("demo/mapped-pentagon-pair/left"),
                    external_id: ExternalId::new("demo/mapped-pentagon-pair/left"),
                    label: "Demo Mapped Pentagon Pair".to_string(),
                    declared_entity: "DemoGeometry".to_string(),
                    default_render_class: DefaultRenderClass::Physical,
                    display_color: Some(DisplayColor::new(0.95, 0.56, 0.24)),
                    face_visibility: FaceVisibility::OneSided,
                },
                GeometryResourceInstance {
                    instance: GeometryInstance {
                        id: GeometryInstanceId(2),
                        definition_id,
                        transform: DMat4::from_translation(DVec3::new(3.5, 0.75, 0.5)),
                    },
                    element_id: SemanticElementId::new("demo/mapped-pentagon-pair/right"),
                    external_id: ExternalId::new("demo/mapped-pentagon-pair/right"),
                    label: "Demo Mapped Pentagon Pair #2".to_string(),
                    declared_entity: "DemoGeometry".to_string(),
                    default_render_class: DefaultRenderClass::Physical,
                    display_color: Some(DisplayColor::new(0.24, 0.78, 0.55)),
                    face_visibility: FaceVisibility::OneSided,
                },
            ],
        }
    }

    fn demo_polygon_with_hole_resource() -> GeometryResource {
        tessellated_resource(
            GeometryDefinitionId(20),
            "demo/polygon-with-hole",
            "Demo Polygon With Hole",
            vec![
                DVec3::new(-2.0, -2.0, 0.0),
                DVec3::new(2.0, -2.0, 0.0),
                DVec3::new(2.0, 2.0, 0.0),
                DVec3::new(-2.0, 2.0, 0.0),
                DVec3::new(-0.9, -0.9, 0.0),
                DVec3::new(-0.9, 0.9, 0.0),
                DVec3::new(0.9, 0.9, 0.0),
                DVec3::new(0.9, -0.9, 0.0),
            ],
            vec![IndexedPolygon::new(vec![0, 1, 2, 3], vec![vec![4, 5, 6, 7]], 8).expect("face")],
        )
    }

    fn demo_concave_polygon_resource() -> GeometryResource {
        tessellated_resource(
            GeometryDefinitionId(21),
            "demo/concave-polygon",
            "Demo Concave Polygon",
            vec![
                DVec3::new(-2.0, -2.0, 0.0),
                DVec3::new(2.0, -2.0, 0.0),
                DVec3::new(2.0, 2.0, 0.0),
                DVec3::new(0.75, 2.0, 0.0),
                DVec3::new(0.75, 0.3, 0.0),
                DVec3::new(-0.75, 0.3, 0.0),
                DVec3::new(-0.75, 2.0, 0.0),
                DVec3::new(-2.0, 2.0, 0.0),
            ],
            vec![IndexedPolygon::new(vec![0, 1, 2, 3, 4, 5, 6, 7], vec![], 8).expect("face")],
        )
    }

    fn demo_extruded_profile_resource() -> GeometryResource {
        swept_solid_resource(
            GeometryDefinitionId(22),
            "demo/extruded-profile",
            "Demo Extruded Profile",
            rectangular_profile(DVec2::new(-1.8, -1.8), DVec2::new(1.8, 1.8)),
            SweepPath::Linear {
                vector: DVec3::new(0.0, 0.0, 2.0),
            },
        )
    }

    fn demo_arc_extruded_profile_resource() -> GeometryResource {
        swept_solid_resource(
            GeometryDefinitionId(23),
            "demo/arc-extruded-profile",
            "Demo Arc Extruded Profile",
            capsule_profile(1.05, 0.55),
            SweepPath::Linear {
                vector: DVec3::new(0.0, 0.0, 1.8),
            },
        )
    }

    fn demo_revolved_solid_resource() -> GeometryResource {
        swept_solid_resource(
            GeometryDefinitionId(24),
            "demo/revolved-solid",
            "Demo Revolved Solid",
            rectangular_profile(DVec2::new(0.55, -0.8), DVec2::new(1.0, 0.8)),
            SweepPath::Revolved {
                axis: Axis3::new(DVec3::ZERO, DVec3::Z).expect("axis"),
                angle_radians: TAU,
            },
        )
    }

    fn demo_circular_profile_sweep_resource() -> GeometryResource {
        circular_profile_sweep_resource(
            GeometryDefinitionId(25),
            "demo/circular-profile-sweep",
            "Demo Circular Profile Sweep",
            CircularProfileSweep::new(
                Polycurve3::new(vec![CurveSegment3::Line(LineSegment3 {
                    start: DVec3::ZERO,
                    end: DVec3::new(0.0, 0.0, 2.4),
                })])
                .expect("spine"),
                0.28,
                Some(0.12),
            )
            .expect("sweep"),
        )
    }

    fn demo_curved_circular_profile_sweep_resource() -> GeometryResource {
        circular_profile_sweep_resource(
            GeometryDefinitionId(26),
            "demo/curved-circular-profile-sweep",
            "Demo Curved Circular Profile Sweep",
            CircularProfileSweep::new(curved_arc_spine(), 0.18, Some(0.08)).expect("sweep"),
        )
    }
}

impl SceneRepository for InMemoryGraphRepository {
    fn load_demo_geometry_resource(
        &self,
        resource: &str,
    ) -> Result<GeometryResource, ResourceError> {
        match normalize_resource_name(resource) {
            Some("demo/pentagon") => Ok(geometry_resource_from_mesh_document(
                Self::demo_pentagon_document(),
            )),
            Some("demo/mapped-pentagon-pair") => Ok(Self::demo_mapped_pentagon_pair_resource()),
            Some("demo/triangle") => Ok(geometry_resource_from_mesh_document(
                Self::demo_triangle_document(),
            )),
            Some("demo/polygon-with-hole") => Ok(Self::demo_polygon_with_hole_resource()),
            Some("demo/concave-polygon") => Ok(Self::demo_concave_polygon_resource()),
            Some("demo/tilted-quad") => Ok(geometry_resource_from_mesh_document(
                Self::demo_tilted_quad_document(),
            )),
            Some("demo/extruded-profile") => Ok(Self::demo_extruded_profile_resource()),
            Some("demo/arc-extruded-profile") => Ok(Self::demo_arc_extruded_profile_resource()),
            Some("demo/revolved-solid") => Ok(Self::demo_revolved_solid_resource()),
            Some("demo/circular-profile-sweep") => Ok(Self::demo_circular_profile_sweep_resource()),
            Some("demo/curved-circular-profile-sweep") => {
                Ok(Self::demo_curved_circular_profile_sweep_resource())
            }
            Some("demo/step-y-up-triangle") => Ok(geometry_resource_from_mesh_document(
                Self::demo_step_y_up_triangle_document(),
            )),
            Some("demo/step-y-up-mm-triangle") => Ok(geometry_resource_from_mesh_document(
                Self::demo_step_y_up_millimeter_triangle_document(),
            )),
            _ => Err(ResourceError::UnknownResource {
                requested: resource.to_string(),
                available: DEMO_RESOURCES.join(", "),
            }),
        }
    }
}

pub fn normalize_document_to_world(document: MeshDocument) -> MeshDocument {
    let source_space = document.import.source_space;
    let polygon = ConvexPolygon::new(
        document
            .polygon
            .vertices
            .into_iter()
            .map(|vertex| source_space.point_to_world(vertex))
            .collect(),
    )
    .expect("coordinate normalization should preserve polygon validity");

    MeshDocument {
        transform: source_space.transform_to_world(document.transform),
        polygon,
        import: ImportMetadata::normalized_from(source_space),
        ..document
    }
}

pub fn import_polygon_document(document: ImportedPolygonDocument) -> MeshDocument {
    normalize_document_to_world(document.into_source_document())
}

pub fn import_geometry_resource(resource: ImportedGeometryResource) -> GeometryResource {
    let scene = import_geometry_scene_resource(resource.into());
    let mut definitions = scene.definitions;
    GeometryResource {
        definition: definitions
            .pop()
            .expect("single imported resource should yield one normalized definition"),
        instances: scene.instances,
    }
}

pub fn import_geometry_scene_resource(
    resource: ImportedGeometrySceneResource,
) -> GeometrySceneResource {
    let source_space = resource.source_space;

    GeometrySceneResource {
        definitions: resource
            .definitions
            .into_iter()
            .map(|definition| GeometryDefinition {
                id: definition.id,
                primitive: normalize_primitive_to_world(definition.primitive, source_space),
            })
            .collect(),
        instances: resource
            .instances
            .into_iter()
            .map(|instance| GeometryResourceInstance {
                instance: GeometryInstance {
                    id: instance.instance.id,
                    definition_id: instance.instance.definition_id,
                    transform: source_space.transform_to_world(instance.instance.transform),
                },
                element_id: instance.element_id,
                external_id: instance.external_id,
                label: instance.label,
                declared_entity: instance.declared_entity,
                default_render_class: instance.default_render_class,
                display_color: instance.display_color,
                face_visibility: instance.face_visibility,
            })
            .collect(),
    }
}

fn normalize_primitive_to_world(
    primitive: GeometryPrimitive,
    source_space: SourceSpace,
) -> GeometryPrimitive {
    match primitive {
        GeometryPrimitive::Tessellated(geometry) => {
            GeometryPrimitive::Tessellated(normalize_tessellated_geometry(geometry, source_space))
        }
        GeometryPrimitive::SweptSolid(solid) => {
            GeometryPrimitive::SweptSolid(normalize_swept_solid(solid, source_space))
        }
        GeometryPrimitive::CircularProfileSweep(sweep) => GeometryPrimitive::CircularProfileSweep(
            normalize_circular_profile_sweep(sweep, source_space),
        ),
    }
}

fn normalize_tessellated_geometry(
    geometry: TessellatedGeometry,
    source_space: SourceSpace,
) -> TessellatedGeometry {
    TessellatedGeometry::new(
        geometry
            .positions
            .into_iter()
            .map(|position| source_space.point_to_world(position))
            .collect(),
        geometry.faces,
    )
    .expect("source-space normalization should preserve tessellated geometry validity")
}

fn normalize_swept_solid(solid: SweptSolid, source_space: SourceSpace) -> SweptSolid {
    SweptSolid::new(
        normalize_profile2(solid.profile, source_space.length_unit),
        normalize_sweep_path(solid.path, source_space),
    )
    .expect("source-space normalization should preserve swept solid validity")
}

fn normalize_circular_profile_sweep(
    sweep: CircularProfileSweep,
    source_space: SourceSpace,
) -> CircularProfileSweep {
    CircularProfileSweep::new(
        normalize_polycurve3(sweep.spine, source_space),
        source_space.length_unit.scale_to_world(sweep.radius),
        sweep
            .inner_radius
            .map(|radius| source_space.length_unit.scale_to_world(radius)),
    )
    .expect("source-space normalization should preserve circular profile sweep validity")
}

fn normalize_profile2(profile: Profile2, length_unit: LengthUnit) -> Profile2 {
    Profile2::new(
        normalize_profile_loop2(profile.outer, length_unit),
        profile
            .holes
            .into_iter()
            .map(|loop_| normalize_profile_loop2(loop_, length_unit))
            .collect(),
    )
}

fn normalize_profile_loop2(loop_: ProfileLoop2, length_unit: LengthUnit) -> ProfileLoop2 {
    ProfileLoop2::new(normalize_polycurve2(loop_.curve, length_unit))
        .expect("source-space normalization should preserve closed profile loops")
}

fn normalize_polycurve2(curve: Polycurve2, length_unit: LengthUnit) -> Polycurve2 {
    Polycurve2::new(
        curve
            .segments
            .into_iter()
            .map(|segment| normalize_curve_segment2(segment, length_unit))
            .collect(),
    )
    .expect("source-space normalization should preserve connected 2D polycurves")
}

fn normalize_curve_segment2(segment: CurveSegment2, length_unit: LengthUnit) -> CurveSegment2 {
    match segment {
        CurveSegment2::Line(segment) => CurveSegment2::Line(LineSegment2 {
            start: normalize_point2(segment.start, length_unit),
            end: normalize_point2(segment.end, length_unit),
        }),
        CurveSegment2::CircularArc(segment) => CurveSegment2::CircularArc(
            CircularArc2::new(
                normalize_point2(segment.start, length_unit),
                normalize_point2(segment.mid, length_unit),
                normalize_point2(segment.end, length_unit),
            )
            .expect("source-space normalization should preserve 2D arcs"),
        ),
    }
}

fn normalize_point2(point: DVec2, length_unit: LengthUnit) -> DVec2 {
    point * length_unit.meters_per_unit()
}

fn normalize_polycurve3(curve: Polycurve3, source_space: SourceSpace) -> Polycurve3 {
    Polycurve3::new(
        curve
            .segments
            .into_iter()
            .map(|segment| normalize_curve_segment3(segment, source_space))
            .collect(),
    )
    .expect("source-space normalization should preserve connected 3D polycurves")
}

fn normalize_curve_segment3(segment: CurveSegment3, source_space: SourceSpace) -> CurveSegment3 {
    match segment {
        CurveSegment3::Line(segment) => CurveSegment3::Line(LineSegment3 {
            start: source_space.point_to_world(segment.start),
            end: source_space.point_to_world(segment.end),
        }),
        CurveSegment3::CircularArc(segment) => CurveSegment3::CircularArc(
            CircularArc3::new(
                source_space.point_to_world(segment.start),
                source_space.point_to_world(segment.mid),
                source_space.point_to_world(segment.end),
            )
            .expect("source-space normalization should preserve 3D arcs"),
        ),
    }
}

fn normalize_sweep_path(path: SweepPath, source_space: SourceSpace) -> SweepPath {
    match path {
        SweepPath::Linear { vector } => SweepPath::Linear {
            vector: source_space.vector_to_world(vector),
        },
        SweepPath::Revolved {
            axis,
            angle_radians,
        } => SweepPath::Revolved {
            axis: normalize_axis3(axis, source_space),
            angle_radians,
        },
        SweepPath::AlongCurve { curve } => SweepPath::AlongCurve {
            curve: normalize_polycurve3(curve, source_space),
        },
    }
}

fn normalize_axis3(axis: Axis3, source_space: SourceSpace) -> Axis3 {
    Axis3::new(
        source_space.point_to_world(axis.origin),
        source_space.vector_to_world(axis.direction),
    )
    .expect("source-space normalization should preserve non-zero sweep axes")
}

fn geometry_resource_from_mesh_document(document: MeshDocument) -> GeometryResource {
    let MeshDocument {
        external_id,
        label,
        polygon,
        transform,
        ..
    } = document;
    let definition_id = GeometryDefinitionId(1);

    GeometryResource {
        definition: geometry_definition_from_polygon(definition_id, polygon),
        instances: vec![GeometryResourceInstance {
            instance: GeometryInstance {
                id: GeometryInstanceId(1),
                definition_id,
                transform,
            },
            element_id: SemanticElementId::new(external_id.as_str()),
            external_id,
            label,
            declared_entity: "DemoGeometry".to_string(),
            default_render_class: DefaultRenderClass::Physical,
            display_color: None,
            face_visibility: FaceVisibility::OneSided,
        }],
    }
}

fn geometry_definition_from_polygon(
    definition_id: GeometryDefinitionId,
    polygon: ConvexPolygon,
) -> GeometryDefinition {
    let face = IndexedPolygon::new(
        (0..polygon.vertices.len() as u32).collect(),
        vec![],
        polygon.vertices.len(),
    )
    .expect("valid polygon document should become a valid indexed face");
    let tessellated = TessellatedGeometry::new(polygon.vertices, vec![face])
        .expect("valid polygon document should become valid tessellated geometry");

    GeometryDefinition {
        id: definition_id,
        primitive: GeometryPrimitive::Tessellated(tessellated),
    }
}

fn polygon_document(external_id: &str, label: &str, vertices: Vec<DVec3>) -> MeshDocument {
    MeshDocument {
        external_id: external_id.into(),
        label: label.to_string(),
        polygon: ConvexPolygon::new(vertices).expect("valid polygon"),
        transform: DMat4::IDENTITY,
        import: ImportMetadata::world_native(),
    }
}

fn tessellated_resource(
    definition_id: GeometryDefinitionId,
    external_id: &str,
    label: &str,
    positions: Vec<DVec3>,
    faces: Vec<IndexedPolygon>,
) -> GeometryResource {
    geometry_resource_from_primitive(
        definition_id,
        external_id,
        label,
        GeometryPrimitive::Tessellated(
            TessellatedGeometry::new(positions, faces).expect("tessellated geometry"),
        ),
    )
}

fn swept_solid_resource(
    definition_id: GeometryDefinitionId,
    external_id: &str,
    label: &str,
    profile: Profile2,
    path: SweepPath,
) -> GeometryResource {
    geometry_resource_from_primitive(
        definition_id,
        external_id,
        label,
        GeometryPrimitive::SweptSolid(SweptSolid::new(profile, path).expect("swept solid")),
    )
}

fn circular_profile_sweep_resource(
    definition_id: GeometryDefinitionId,
    external_id: &str,
    label: &str,
    sweep: CircularProfileSweep,
) -> GeometryResource {
    geometry_resource_from_primitive(
        definition_id,
        external_id,
        label,
        GeometryPrimitive::CircularProfileSweep(sweep),
    )
}

fn geometry_resource_from_primitive(
    definition_id: GeometryDefinitionId,
    external_id: &str,
    label: &str,
    primitive: GeometryPrimitive,
) -> GeometryResource {
    import_geometry_resource(ImportedGeometryResource {
        definition: GeometryDefinition {
            id: definition_id,
            primitive,
        },
        instances: vec![ImportedGeometryResourceInstance {
            instance: GeometryInstance {
                id: GeometryInstanceId(1),
                definition_id,
                transform: DMat4::IDENTITY,
            },
            element_id: SemanticElementId::new(external_id),
            external_id: ExternalId::new(external_id),
            label: label.to_string(),
            declared_entity: "IfcProduct".to_string(),
            default_render_class: DefaultRenderClass::Physical,
            display_color: None,
            face_visibility: FaceVisibility::OneSided,
        }],
        source_space: SourceSpace::w_world_metric(),
    })
}

fn rectangular_profile(min: DVec2, max: DVec2) -> Profile2 {
    Profile2::new(rectangular_loop(min, max), vec![])
}

fn capsule_profile(half_length: f64, radius: f64) -> Profile2 {
    let left_bottom = DVec2::new(-half_length, -radius);
    let right_bottom = DVec2::new(half_length, -radius);
    let right_top = DVec2::new(half_length, radius);
    let left_top = DVec2::new(-half_length, radius);
    let curve = Polycurve2::new(vec![
        CurveSegment2::Line(LineSegment2 {
            start: left_bottom,
            end: right_bottom,
        }),
        CurveSegment2::CircularArc(
            CircularArc2::new(
                right_bottom,
                DVec2::new(half_length + radius, 0.0),
                right_top,
            )
            .expect("arc"),
        ),
        CurveSegment2::Line(LineSegment2 {
            start: right_top,
            end: left_top,
        }),
        CurveSegment2::CircularArc(
            CircularArc2::new(
                left_top,
                DVec2::new(-(half_length + radius), 0.0),
                left_bottom,
            )
            .expect("arc"),
        ),
    ])
    .expect("capsule");

    Profile2::new(ProfileLoop2::new(curve).expect("loop"), vec![])
}

fn rectangular_loop(min: DVec2, max: DVec2) -> ProfileLoop2 {
    let a = min;
    let b = DVec2::new(max.x, min.y);
    let c = max;
    let d = DVec2::new(min.x, max.y);
    let curve = Polycurve2::new(vec![
        CurveSegment2::Line(LineSegment2 { start: a, end: b }),
        CurveSegment2::Line(LineSegment2 { start: b, end: c }),
        CurveSegment2::Line(LineSegment2 { start: c, end: d }),
        CurveSegment2::Line(LineSegment2 { start: d, end: a }),
    ])
    .expect("rectangle");

    ProfileLoop2::new(curve).expect("loop")
}

fn curved_arc_spine() -> Polycurve3 {
    Polycurve3::new(vec![CurveSegment3::CircularArc(
        CircularArc3::new(
            DVec3::ZERO,
            DVec3::new(1.2, 0.0, 1.2),
            DVec3::new(0.0, 0.0, 2.4),
        )
        .expect("arc"),
    )])
    .expect("spine")
}

fn normalize_resource_name(resource: &str) -> Option<&'static str> {
    match resource {
        "pentagon" | "demo/pentagon" => Some("demo/pentagon"),
        "mapped-pentagon" | "mapped-pentagon-pair" | "demo/mapped-pentagon-pair" => {
            Some("demo/mapped-pentagon-pair")
        }
        "triangle" | "demo/triangle" => Some("demo/triangle"),
        "polygon-with-hole" | "demo/polygon-with-hole" => Some("demo/polygon-with-hole"),
        "concave-polygon" | "demo/concave-polygon" => Some("demo/concave-polygon"),
        "quad" | "tilted-quad" | "demo/tilted-quad" => Some("demo/tilted-quad"),
        "extruded-profile" | "demo/extruded-profile" => Some("demo/extruded-profile"),
        "arc-extruded-profile" | "demo/arc-extruded-profile" => Some("demo/arc-extruded-profile"),
        "revolved-solid" | "demo/revolved-solid" => Some("demo/revolved-solid"),
        "circular-profile-sweep" | "demo/circular-profile-sweep" => {
            Some("demo/circular-profile-sweep")
        }
        "curved-circular-profile-sweep" | "demo/curved-circular-profile-sweep" => {
            Some("demo/curved-circular-profile-sweep")
        }
        "step-triangle" | "demo/step-y-up-triangle" => Some("demo/step-y-up-triangle"),
        "step-mm-triangle" | "demo/step-y-up-mm-triangle" => Some("demo/step-y-up-mm-triangle"),
        _ => None,
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ResourceError {
    #[error("unknown resource `{requested}`; available resources: {available}")]
    UnknownResource {
        requested: String,
        available: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tessellated_positions(resource: &GeometryResource) -> &[DVec3] {
        match &resource.definition.primitive {
            GeometryPrimitive::Tessellated(geometry) => &geometry.positions,
            primitive => panic!("expected tessellated geometry, got {primitive:?}"),
        }
    }

    fn sample_profile_in_source_units() -> Profile2 {
        let outer = ProfileLoop2::new(
            Polycurve2::new(vec![
                CurveSegment2::Line(LineSegment2 {
                    start: DVec2::ZERO,
                    end: DVec2::new(1_000.0, 0.0),
                }),
                CurveSegment2::Line(LineSegment2 {
                    start: DVec2::new(1_000.0, 0.0),
                    end: DVec2::new(1_000.0, 1_000.0),
                }),
                CurveSegment2::Line(LineSegment2 {
                    start: DVec2::new(1_000.0, 1_000.0),
                    end: DVec2::ZERO,
                }),
            ])
            .expect("curve"),
        )
        .expect("loop");

        Profile2::new(outer, vec![])
    }

    #[test]
    fn demo_repository_returns_pentagon() {
        let resource = InMemoryGraphRepository
            .load_demo_geometry_resource(DEFAULT_DEMO_RESOURCE)
            .expect("resource");

        assert_eq!(resource.instances[0].label, "Demo Pentagon");
        assert_eq!(tessellated_positions(&resource).len(), 5);
    }

    #[test]
    fn demo_repository_returns_geometry_resource() {
        let resource = InMemoryGraphRepository
            .load_demo_geometry_resource(DEFAULT_DEMO_RESOURCE)
            .expect("resource");

        assert_eq!(resource.definition.id, GeometryDefinitionId(1));
        assert_eq!(resource.instances.len(), 1);
        assert_eq!(resource.instances[0].external_id.as_str(), "demo/pentagon");
        assert_eq!(resource.instances[0].label, "Demo Pentagon");
        assert_eq!(
            resource.instances[0].instance.definition_id,
            resource.definition.id
        );
        assert!(matches!(
            resource.definition.primitive,
            GeometryPrimitive::Tessellated(_)
        ));
    }

    #[test]
    fn mapped_demo_resource_reuses_one_definition_for_two_instances() {
        let resource = InMemoryGraphRepository
            .load_demo_geometry_resource("mapped-pentagon")
            .expect("resource");

        assert_eq!(resource.definition.id, GeometryDefinitionId(1));
        assert_eq!(resource.instances.len(), 2);
        assert_eq!(
            resource.instances[0].instance.definition_id,
            resource.instances[1].instance.definition_id
        );
        assert_ne!(
            resource.instances[0].instance.transform,
            resource.instances[1].instance.transform
        );
        assert_eq!(
            resource.instances[0].external_id.as_str(),
            "demo/mapped-pentagon-pair/left"
        );
    }

    #[test]
    fn demo_repository_supports_alias_names() {
        let resource = InMemoryGraphRepository
            .load_demo_geometry_resource("quad")
            .expect("resource");

        assert_eq!(resource.instances[0].label, "Demo Tilted Quad");
        assert_eq!(tessellated_positions(&resource).len(), 4);
    }

    #[test]
    fn demo_repository_rejects_unknown_resources() {
        let result = InMemoryGraphRepository.load_demo_geometry_resource("unknown");

        assert!(matches!(result, Err(ResourceError::UnknownResource { .. })));
    }

    #[test]
    fn import_boundary_normalizes_right_handed_y_up_document() {
        let normalized = import_polygon_document(
            ImportedPolygonDocument::from_vertices(
                "test/source",
                "Source",
                vec![
                    DVec3::new(-1.0, 0.0, 1.0),
                    DVec3::new(1.0, 0.0, 1.0),
                    DVec3::new(0.0, 0.0, -1.0),
                ],
                SourceSpace::right_handed_y_up_meters(),
            )
            .expect("polygon")
            .with_transform(DMat4::from_translation(DVec3::new(3.0, 2.0, -4.0))),
        );

        assert!(
            normalized
                .polygon
                .vertices
                .iter()
                .all(|vertex| vertex.z.abs() <= f64::EPSILON)
        );
        assert_eq!(
            normalized.transform.transform_point3(DVec3::ZERO),
            DVec3::new(3.0, 4.0, 2.0)
        );
        assert!(normalized.import.normalized_to_world);
    }

    #[test]
    fn step_demo_resource_is_normalized_into_world_frame() {
        let triangle = InMemoryGraphRepository
            .load_demo_geometry_resource("triangle")
            .expect("triangle resource");
        let step_triangle = InMemoryGraphRepository
            .load_demo_geometry_resource("step-triangle")
            .expect("resource");

        assert_eq!(step_triangle.instances[0].label, "Demo STEP Y-Up Triangle");
        assert_eq!(
            tessellated_positions(&step_triangle),
            tessellated_positions(&triangle)
        );
        assert!(
            step_triangle.instances[0]
                .instance
                .transform
                .abs_diff_eq(DMat4::IDENTITY, 1.0e-12)
        );
    }

    #[test]
    fn import_boundary_normalizes_units_into_meters() {
        let normalized = import_polygon_document(
            ImportedPolygonDocument::from_vertices(
                "test/mm-source",
                "Millimeters",
                vec![
                    DVec3::new(-1_000.0, 0.0, 1_000.0),
                    DVec3::new(1_000.0, 0.0, 1_000.0),
                    DVec3::new(0.0, 0.0, -1_000.0),
                ],
                SourceSpace::right_handed_y_up_millimeters(),
            )
            .expect("polygon")
            .with_transform(DMat4::from_translation(DVec3::new(
                1_500.0, 2_000.0, -4_000.0,
            ))),
        );

        assert!(normalized.polygon.vertices[0].abs_diff_eq(DVec3::new(-1.0, -1.0, 0.0), 1.0e-12));
        assert!(
            normalized
                .transform
                .transform_point3(DVec3::ZERO)
                .abs_diff_eq(DVec3::new(1.5, 4.0, 2.0), 1.0e-12)
        );
        assert_eq!(
            normalized.import.source_space,
            SourceSpace::right_handed_y_up_millimeters()
        );
        assert!(normalized.import.normalized_to_world);
    }

    #[test]
    fn step_millimeter_demo_resource_is_normalized_into_metric_world_frame() {
        let triangle = InMemoryGraphRepository
            .load_demo_geometry_resource("triangle")
            .expect("triangle resource");
        let step_triangle = InMemoryGraphRepository
            .load_demo_geometry_resource("step-mm-triangle")
            .expect("resource");

        assert_eq!(
            step_triangle.instances[0].label,
            "Demo STEP Y-Up Millimeter Triangle"
        );
        assert!(
            tessellated_positions(&step_triangle)
                .iter()
                .zip(tessellated_positions(&triangle))
                .all(|(left, right)| left.abs_diff_eq(*right, 1.0e-12))
        );
        assert!(
            step_triangle.instances[0]
                .instance
                .transform
                .abs_diff_eq(DMat4::IDENTITY, 1.0e-12)
        );
    }

    #[test]
    fn imported_polygon_document_validates_geometry_before_normalization() {
        let result = ImportedPolygonDocument::from_vertices(
            "test/invalid",
            "Invalid",
            vec![DVec3::ZERO, DVec3::X],
            SourceSpace::right_handed_y_up_meters(),
        );

        assert!(matches!(result, Err(GeometryError::TooFewVertices)));
    }

    #[test]
    fn import_geometry_resource_normalizes_tessellated_geometry_and_instances() {
        let definition_id = GeometryDefinitionId(11);
        let imported = ImportedGeometryResource {
            definition: GeometryDefinition {
                id: definition_id,
                primitive: GeometryPrimitive::Tessellated(
                    TessellatedGeometry::new(
                        vec![
                            DVec3::new(-1_000.0, 0.0, 1_000.0),
                            DVec3::new(1_000.0, 0.0, 1_000.0),
                            DVec3::new(0.0, 0.0, -1_000.0),
                        ],
                        vec![IndexedPolygon::new(vec![0, 1, 2], vec![], 3).expect("face")],
                    )
                    .expect("tessellation"),
                ),
            },
            instances: vec![
                ImportedGeometryResourceInstance {
                    instance: GeometryInstance {
                        id: GeometryInstanceId(1),
                        definition_id,
                        transform: DMat4::from_translation(DVec3::new(1_500.0, 2_000.0, -4_000.0)),
                    },
                    element_id: SemanticElementId::new("ifc/product/1"),
                    external_id: ExternalId::new("ifc/product/1"),
                    label: "Instance 1".to_string(),
                    declared_entity: "IfcProduct".to_string(),
                    default_render_class: DefaultRenderClass::Physical,
                    display_color: None,
                    face_visibility: FaceVisibility::OneSided,
                },
                ImportedGeometryResourceInstance {
                    instance: GeometryInstance {
                        id: GeometryInstanceId(2),
                        definition_id,
                        transform: DMat4::IDENTITY,
                    },
                    element_id: SemanticElementId::new("ifc/product/2"),
                    external_id: ExternalId::new("ifc/product/2"),
                    label: "Instance 2".to_string(),
                    declared_entity: "IfcProduct".to_string(),
                    default_render_class: DefaultRenderClass::Physical,
                    display_color: None,
                    face_visibility: FaceVisibility::OneSided,
                },
            ],
            source_space: SourceSpace::right_handed_y_up_millimeters(),
        };

        let resource = import_geometry_resource(imported);

        assert_eq!(
            tessellated_positions(&resource)[0],
            DVec3::new(-1.0, -1.0, 0.0)
        );
        assert_eq!(
            tessellated_positions(&resource)[1],
            DVec3::new(1.0, -1.0, 0.0)
        );
        assert_eq!(
            tessellated_positions(&resource)[2],
            DVec3::new(0.0, 1.0, 0.0)
        );
        assert!(
            resource.instances[0]
                .instance
                .transform
                .transform_point3(DVec3::ZERO)
                .abs_diff_eq(DVec3::new(1.5, 4.0, 2.0), 1.0e-12)
        );
        assert!(
            resource.instances[1]
                .instance
                .transform
                .abs_diff_eq(DMat4::IDENTITY, 1.0e-12)
        );
    }

    #[test]
    fn import_geometry_resource_normalizes_linear_swept_solids() {
        let definition_id = GeometryDefinitionId(21);
        let imported = ImportedGeometryResource {
            definition: GeometryDefinition {
                id: definition_id,
                primitive: GeometryPrimitive::SweptSolid(
                    SweptSolid::new(
                        sample_profile_in_source_units(),
                        SweepPath::Linear {
                            vector: DVec3::new(0.0, 2_000.0, 0.0),
                        },
                    )
                    .expect("solid"),
                ),
            },
            instances: vec![ImportedGeometryResourceInstance {
                instance: GeometryInstance {
                    id: GeometryInstanceId(1),
                    definition_id,
                    transform: DMat4::IDENTITY,
                },
                element_id: SemanticElementId::new("ifc/extrusion/1"),
                external_id: ExternalId::new("ifc/extrusion/1"),
                label: "Extrusion".to_string(),
                declared_entity: "IfcProduct".to_string(),
                default_render_class: DefaultRenderClass::Physical,
                display_color: None,
                face_visibility: FaceVisibility::OneSided,
            }],
            source_space: SourceSpace::right_handed_y_up_millimeters(),
        };

        let resource = import_geometry_resource(imported);
        let GeometryPrimitive::SweptSolid(solid) = &resource.definition.primitive else {
            panic!("expected swept solid");
        };

        assert_eq!(
            solid.profile.outer.curve.segments[0].start(),
            DVec2::new(0.0, 0.0)
        );
        assert_eq!(
            solid.profile.outer.curve.segments[0].end(),
            DVec2::new(1.0, 0.0)
        );
        match solid.path {
            SweepPath::Linear { vector } => assert_eq!(vector, DVec3::new(0.0, 0.0, 2.0)),
            _ => panic!("expected linear sweep path"),
        }
    }

    #[test]
    fn import_geometry_resource_normalizes_revolved_swept_solids() {
        let definition_id = GeometryDefinitionId(22);
        let imported = ImportedGeometryResource {
            definition: GeometryDefinition {
                id: definition_id,
                primitive: GeometryPrimitive::SweptSolid(
                    SweptSolid::new(
                        sample_profile_in_source_units(),
                        SweepPath::Revolved {
                            axis: Axis3::new(
                                DVec3::new(0.0, 2_000.0, 0.0),
                                DVec3::new(0.0, 1_000.0, 0.0),
                            )
                            .expect("axis"),
                            angle_radians: std::f64::consts::FRAC_PI_2,
                        },
                    )
                    .expect("solid"),
                ),
            },
            instances: vec![ImportedGeometryResourceInstance {
                instance: GeometryInstance {
                    id: GeometryInstanceId(1),
                    definition_id,
                    transform: DMat4::IDENTITY,
                },
                element_id: SemanticElementId::new("ifc/revolve/1"),
                external_id: ExternalId::new("ifc/revolve/1"),
                label: "Revolve".to_string(),
                declared_entity: "IfcProduct".to_string(),
                default_render_class: DefaultRenderClass::Physical,
                display_color: None,
                face_visibility: FaceVisibility::OneSided,
            }],
            source_space: SourceSpace::right_handed_y_up_millimeters(),
        };

        let resource = import_geometry_resource(imported);
        let GeometryPrimitive::SweptSolid(solid) = &resource.definition.primitive else {
            panic!("expected swept solid");
        };

        match &solid.path {
            SweepPath::Revolved {
                axis,
                angle_radians,
            } => {
                assert!(axis.origin.abs_diff_eq(DVec3::new(0.0, 0.0, 2.0), 1.0e-12));
                assert!(
                    axis.direction
                        .abs_diff_eq(DVec3::new(0.0, 0.0, 1.0), 1.0e-12)
                );
                assert!((*angle_radians - std::f64::consts::FRAC_PI_2).abs() <= 1.0e-12);
            }
            _ => panic!("expected revolved sweep path"),
        }
    }

    #[test]
    fn import_geometry_resource_normalizes_circular_profile_sweeps() {
        let definition_id = GeometryDefinitionId(23);
        let spine = Polycurve3::new(vec![CurveSegment3::Line(LineSegment3 {
            start: DVec3::ZERO,
            end: DVec3::new(0.0, 2_000.0, 0.0),
        })])
        .expect("spine");
        let imported = ImportedGeometryResource {
            definition: GeometryDefinition {
                id: definition_id,
                primitive: GeometryPrimitive::CircularProfileSweep(
                    CircularProfileSweep::new(spine, 250.0, Some(100.0)).expect("sweep"),
                ),
            },
            instances: vec![ImportedGeometryResourceInstance {
                instance: GeometryInstance {
                    id: GeometryInstanceId(1),
                    definition_id,
                    transform: DMat4::IDENTITY,
                },
                element_id: SemanticElementId::new("ifc/swept-disk/1"),
                external_id: ExternalId::new("ifc/swept-disk/1"),
                label: "Swept Disk".to_string(),
                declared_entity: "IfcProduct".to_string(),
                default_render_class: DefaultRenderClass::Physical,
                display_color: None,
                face_visibility: FaceVisibility::OneSided,
            }],
            source_space: SourceSpace::right_handed_y_up_millimeters(),
        };

        let resource = import_geometry_resource(imported);
        let GeometryPrimitive::CircularProfileSweep(sweep) = &resource.definition.primitive else {
            panic!("expected circular profile sweep");
        };

        assert_eq!(sweep.radius, 0.25);
        assert_eq!(sweep.inner_radius, Some(0.1));
        assert_eq!(sweep.spine.start(), DVec3::ZERO);
        assert_eq!(sweep.spine.end(), DVec3::new(0.0, 0.0, 2.0));
    }
}
