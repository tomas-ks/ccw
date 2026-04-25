use cc_w_backend::{GeometryBackend, GeometryBackendError, ResourceError};
use cc_w_db::{
    GeometryResource, ImportedGeometryResource, ImportedGeometryResourceInstance,
    ResourceError as DbResourceError, SceneRepository, import_geometry_resource,
};
use cc_w_kernel::TrivialKernel;
use cc_w_platform_headless::HeadlessCliError;
use cc_w_prepare::MeshPreparePipeline;
use cc_w_render::{
    Camera, NullRenderBackend, RenderedImage, ViewportSize, fit_camera_to_render_scene,
    render_prepared_mesh_offscreen, render_prepared_scene_offscreen,
};
use cc_w_runtime::{Engine, GeometryPackageSource, GeometryPackageSourceError};
use cc_w_types::{
    Axis3, Bounds3, CircularArc2, CircularArc3, CircularProfileSweep, CurveSegment2, CurveSegment3,
    DefaultRenderClass, DisplayColor, ExternalId, GeometryDefinition, GeometryDefinitionId,
    GeometryInstance, GeometryInstanceId, GeometryPrimitive, IndexedPolygon, LineSegment2,
    LineSegment3, Polycurve2, Polycurve3, PreparedMaterial, PreparedMesh, PreparedRenderDefinition,
    PreparedRenderInstance, PreparedRenderScene, PreparedVertex, Profile2, ProfileLoop2,
    SemanticElementId, SourceSpace, SweepPath, SweptSolid, TessellatedGeometry, WORLD_UP,
};
use glam::{DMat4, DVec2, DVec3};

const VIEWPORT: ViewportSize = ViewportSize::new(240, 240);
const PIXEL_TOLERANCE: u8 = 8;

#[test]
fn imported_triangulated_face_set_renders_centered_triangle() {
    let image = render_imported_geometry_resource(
        "test/triangle",
        tessellated_resource(
            GeometryDefinitionId(1),
            vec![
                world_xy_to_source_mm(-1.8, -1.2),
                world_xy_to_source_mm(1.9, -1.0),
                world_xy_to_source_mm(0.0, 2.2),
            ],
            vec![IndexedPolygon::new(vec![0, 1, 2], vec![], 3).expect("triangle")],
        ),
    )
    .expect("triangle render");

    let stats = foreground_stats(&image);
    let center_x = midpoint(stats.min_x, stats.max_x);
    let center_y = midpoint(stats.min_y, stats.max_y);

    assert!(image.has_variation());
    assert!(stats.pixel_count > 2_000);
    assert!(stats.width() > 90);
    assert!(stats.height() > 90);
    assert!(foreground_at(&image, center_x, center_y));
    assert_eq!(foreground_segments_in_row(&image, center_y).len(), 1);
    assert_eq!(foreground_segments_in_column(&image, center_x).len(), 1);
}

#[test]
fn imported_polygonal_face_set_with_hole_renders_open_center() {
    let image = render_imported_geometry_resource(
        "test/polygon-with-hole",
        tessellated_resource(
            GeometryDefinitionId(2),
            vec![
                world_xy_to_source_mm(-2.0, -2.0),
                world_xy_to_source_mm(2.0, -2.0),
                world_xy_to_source_mm(2.0, 2.0),
                world_xy_to_source_mm(-2.0, 2.0),
                world_xy_to_source_mm(-0.9, -0.9),
                world_xy_to_source_mm(-0.9, 0.9),
                world_xy_to_source_mm(0.9, 0.9),
                world_xy_to_source_mm(0.9, -0.9),
            ],
            vec![IndexedPolygon::new(vec![0, 1, 2, 3], vec![vec![4, 5, 6, 7]], 8).expect("face")],
        ),
    )
    .expect("polygon-with-hole render");

    let stats = foreground_stats(&image);
    let center_x = midpoint(stats.min_x, stats.max_x);
    let center_y = midpoint(stats.min_y, stats.max_y);

    assert!(image.has_variation());
    assert!(stats.pixel_count > 3_000, "stats={stats:?}");
    assert!(stats.width() > 100, "stats={stats:?}");
    assert!(stats.height() > 100, "stats={stats:?}");
    assert!(!foreground_at(&image, center_x, center_y));
    assert_eq!(foreground_segments_in_row(&image, center_y).len(), 2);
    assert_eq!(foreground_segments_in_column(&image, center_x).len(), 2);
}

#[test]
fn imported_concave_polygon_renders_visible_notch() {
    let image = render_imported_geometry_resource(
        "test/concave-polygon",
        tessellated_resource(
            GeometryDefinitionId(3),
            vec![
                world_xy_to_source_mm(-2.0, -2.0),
                world_xy_to_source_mm(2.0, -2.0),
                world_xy_to_source_mm(2.0, 2.0),
                world_xy_to_source_mm(0.75, 2.0),
                world_xy_to_source_mm(0.75, 0.3),
                world_xy_to_source_mm(-0.75, 0.3),
                world_xy_to_source_mm(-0.75, 2.0),
                world_xy_to_source_mm(-2.0, 2.0),
            ],
            vec![IndexedPolygon::new(vec![0, 1, 2, 3, 4, 5, 6, 7], vec![], 8).expect("face")],
        ),
    )
    .expect("concave polygon render");

    let stats = foreground_stats(&image);
    let center_x = midpoint(stats.min_x, stats.max_x);
    let center_y = midpoint(stats.min_y, stats.max_y);
    let top_row = stats.min_y + stats.height() / 5;
    let bottom_row = stats.max_y.saturating_sub(stats.height() / 5);
    let top_segments = foreground_segments_in_row(&image, top_row).len();
    let bottom_segments = foreground_segments_in_row(&image, bottom_row).len();

    assert!(image.has_variation());
    assert!(stats.pixel_count > 3_000);
    assert!(stats.width() > 100);
    assert!(stats.height() > 100);
    assert!(foreground_at(&image, center_x, center_y));
    assert_eq!(foreground_segments_in_row(&image, center_y).len(), 1);
    assert!(
        (top_segments == 2 && bottom_segments == 1) || (top_segments == 1 && bottom_segments == 2),
        "expected one edge-biased row to expose the concave notch, got top={top_segments}, bottom={bottom_segments}"
    );
}

#[test]
fn imported_linear_swept_solid_renders_prism_volume() {
    let image = render_imported_geometry_resource_fit_camera(
        "test/extruded-profile",
        swept_solid_resource(
            GeometryDefinitionId(4),
            rectangular_profile(DVec2::new(-1.8, -1.8), DVec2::new(1.8, 1.8)),
            SweepPath::Linear {
                vector: DVec3::new(0.0, 0.0, 2.0),
            },
        ),
    )
    .expect("extruded profile render");

    let stats = foreground_stats(&image);

    assert!(image.has_variation());
    assert!(stats.pixel_count > 6_000, "stats={stats:?}");
    assert!(stats.width() > 110, "stats={stats:?}");
    assert!(stats.height() > 110, "stats={stats:?}");
}

#[test]
fn imported_arc_profile_linear_swept_solid_renders_rounded_volume() {
    let image = render_imported_geometry_resource_fit_camera(
        "test/arc-extruded-profile",
        swept_solid_resource(
            GeometryDefinitionId(7),
            capsule_profile(1.05, 0.55),
            SweepPath::Linear {
                vector: DVec3::new(0.0, 0.0, 1.8),
            },
        ),
    )
    .expect("arc extruded profile render");

    let stats = foreground_stats(&image);

    assert!(image.has_variation());
    assert!(stats.pixel_count > 5_500, "stats={stats:?}");
    assert!(stats.width() > 80, "stats={stats:?}");
    assert!(stats.height() > 100, "stats={stats:?}");
}

#[test]
fn imported_revolved_swept_solid_renders_lathed_volume() {
    let image = render_imported_geometry_resource_fit_camera(
        "test/revolved-solid",
        swept_solid_resource(
            GeometryDefinitionId(5),
            rectangular_profile(DVec2::new(0.55, -0.8), DVec2::new(1.0, 0.8)),
            SweepPath::Revolved {
                axis: Axis3::new(DVec3::ZERO, DVec3::Z).expect("axis"),
                angle_radians: std::f64::consts::TAU,
            },
        ),
    )
    .expect("revolved solid render");

    let stats = foreground_stats(&image);

    assert!(image.has_variation());
    assert!(stats.pixel_count > 4_000, "stats={stats:?}");
    assert!(stats.width() > 100, "stats={stats:?}");
    assert!(stats.height() > 110, "stats={stats:?}");
}

#[test]
fn imported_circular_profile_sweep_renders_pipe_volume() {
    let spine = Polycurve3::new(vec![CurveSegment3::Line(LineSegment3 {
        start: DVec3::ZERO,
        end: DVec3::new(0.0, 0.0, 2.4),
    })])
    .expect("spine");
    let image = render_imported_geometry_resource_fit_camera(
        "test/circular-profile-sweep",
        circular_profile_sweep_resource(
            GeometryDefinitionId(6),
            CircularProfileSweep::new(spine, 0.28, Some(0.12)).expect("sweep"),
        ),
    )
    .expect("circular profile sweep render");

    let stats = foreground_stats(&image);

    assert!(image.has_variation());
    assert!(stats.pixel_count > 3_500, "stats={stats:?}");
    assert!(stats.width() > 35, "stats={stats:?}");
    assert!(stats.height() > 120, "stats={stats:?}");
}

#[test]
fn imported_curved_circular_profile_sweep_renders_bent_pipe() {
    let spine = curved_arc_spine();
    let image = render_imported_geometry_resource_fit_camera(
        "test/curved-circular-profile-sweep",
        circular_profile_sweep_resource(
            GeometryDefinitionId(8),
            CircularProfileSweep::new(spine, 0.18, Some(0.08)).expect("sweep"),
        ),
    )
    .expect("curved circular profile sweep render");

    let stats = foreground_stats(&image);

    assert!(image.has_variation());
    assert!(stats.pixel_count > 3_200, "stats={stats:?}");
    assert!(stats.width() > 70, "stats={stats:?}");
    assert!(stats.height() > 110, "stats={stats:?}");
}

#[test]
fn demo_mapped_geometry_renders_repeated_instances() {
    let image = render_demo_geometry_resource("demo/mapped-pentagon-pair", |render_scene| {
        top_down_camera_for_bounds(render_scene.bounds)
    })
    .expect("mapped geometry render");

    let stats = foreground_stats(&image);
    let center_y = midpoint(stats.min_y, stats.max_y);

    assert!(image.has_variation());
    assert!(stats.pixel_count > 5_000, "stats={stats:?}");
    assert!(stats.width() > 150, "stats={stats:?}");
    assert!(stats.height() > 70, "stats={stats:?}");
    assert_eq!(foreground_segments_in_row(&image, center_y).len(), 2);
}

#[test]
fn demo_mapped_geometry_preserves_per_instance_material_colors() {
    let image = render_demo_geometry_resource("demo/mapped-pentagon-pair", |render_scene| {
        top_down_camera_for_bounds(render_scene.bounds)
    })
    .expect("mapped geometry color render");

    let stats = foreground_stats(&image);
    let center_y = midpoint(stats.min_y, stats.max_y);
    let segments = foreground_segments_in_row(&image, center_y);

    assert_eq!(segments.len(), 2, "expected two visible pentagon segments");

    let left_x = midpoint(segments[0].0, segments[0].1);
    let right_x = midpoint(segments[1].0, segments[1].1);
    let left_pixel = pixel_at(&image, left_x, center_y);
    let right_pixel = pixel_at(&image, right_x, center_y);

    assert!(
        left_pixel[0] > left_pixel[1].saturating_add(40)
            && left_pixel[1] > left_pixel[2].saturating_add(30),
        "expected left instance to stay warm/orange, got pixel={left_pixel:?}"
    );
    assert!(
        right_pixel[1] > right_pixel[0].saturating_add(40)
            && right_pixel[1] > right_pixel[2].saturating_add(20),
        "expected right instance to stay green-dominant, got pixel={right_pixel:?}"
    );
}

#[test]
fn offscreen_renderer_uses_depth_for_overlapping_triangles() {
    let image = pollster::block_on(render_prepared_mesh_offscreen(
        &prepared_mesh(vec![
            PreparedVertex {
                position: [-1.6, -1.4, 0.2],
                normal: [0.0, 0.0, 1.0],
            },
            PreparedVertex {
                position: [1.6, -1.4, 0.2],
                normal: [0.0, 0.0, 1.0],
            },
            PreparedVertex {
                position: [0.0, 1.7, 0.2],
                normal: [0.0, 0.0, 1.0],
            },
            PreparedVertex {
                position: [-1.6, -1.4, -0.2],
                normal: [0.0, 0.0, -1.0],
            },
            PreparedVertex {
                position: [1.6, -1.4, -0.2],
                normal: [0.0, 0.0, -1.0],
            },
            PreparedVertex {
                position: [0.0, 1.7, -0.2],
                normal: [0.0, 0.0, -1.0],
            },
        ]),
        VIEWPORT,
        top_down_camera_for_extent(4.0),
    ))
    .expect("depth-tested render");

    let center = pixel_at(&image, image.width as usize / 2, image.height as usize / 2);

    assert!(foreground_at(
        &image,
        image.width as usize / 2,
        image.height as usize / 2
    ));
    assert!(
        pixel_luma(center) > 420,
        "expected the nearer bright triangle to win the depth test, got pixel={center:?}"
    );
}

#[test]
fn offscreen_renderer_culls_clockwise_faces() {
    let image = pollster::block_on(render_prepared_mesh_offscreen(
        &prepared_mesh(vec![
            PreparedVertex {
                position: [-1.6, -1.4, 0.0],
                normal: [0.0, 0.0, 1.0],
            },
            PreparedVertex {
                position: [1.6, -1.4, 0.0],
                normal: [0.0, 0.0, 1.0],
            },
            PreparedVertex {
                position: [0.0, 1.7, 0.0],
                normal: [0.0, 0.0, 1.0],
            },
            PreparedVertex {
                position: [0.0, 1.7, 0.2],
                normal: [0.0, 0.0, -1.0],
            },
            PreparedVertex {
                position: [1.6, -1.4, 0.2],
                normal: [0.0, 0.0, -1.0],
            },
            PreparedVertex {
                position: [-1.6, -1.4, 0.2],
                normal: [0.0, 0.0, -1.0],
            },
        ]),
        VIEWPORT,
        top_down_camera_for_extent(4.0),
    ))
    .expect("culled render");

    let center = pixel_at(&image, image.width as usize / 2, image.height as usize / 2);

    assert!(foreground_at(
        &image,
        image.width as usize / 2,
        image.height as usize / 2
    ));
    assert!(
        pixel_luma(center) > 420,
        "expected the clockwise triangle to be culled, got pixel={center:?}"
    );
}

#[test]
fn offscreen_renderer_uses_material_color() {
    let image = pollster::block_on(render_prepared_scene_offscreen(
        &PreparedRenderScene {
            bounds: Bounds3::from_points(&[DVec3::new(-1.6, -1.4, 0.0), DVec3::new(1.6, 1.7, 0.0)])
                .expect("bounds"),
            definitions: vec![PreparedRenderDefinition {
                id: GeometryDefinitionId(1),
                mesh: prepared_mesh(vec![
                    PreparedVertex {
                        position: [-1.6, -1.4, 0.0],
                        normal: [0.0, 0.0, 1.0],
                    },
                    PreparedVertex {
                        position: [1.6, -1.4, 0.0],
                        normal: [0.0, 0.0, 1.0],
                    },
                    PreparedVertex {
                        position: [0.0, 1.7, 0.0],
                        normal: [0.0, 0.0, 1.0],
                    },
                ]),
            }],
            instances: vec![PreparedRenderInstance {
                id: GeometryInstanceId(1),
                element_id: SemanticElementId::new("test/triangle"),
                definition_id: GeometryDefinitionId(1),
                model_from_object: DMat4::IDENTITY,
                world_bounds: Bounds3::from_points(&[
                    DVec3::new(-1.6, -1.4, 0.0),
                    DVec3::new(1.6, 1.7, 0.0),
                ])
                .expect("bounds"),
                material: PreparedMaterial::new(DisplayColor::new(0.92, 0.24, 0.18)),
                default_render_class: DefaultRenderClass::Physical,
            }],
        },
        VIEWPORT,
        top_down_camera_for_extent(4.0),
    ))
    .expect("colored render");

    let center = pixel_at(&image, image.width as usize / 2, image.height as usize / 2);

    assert!(foreground_at(
        &image,
        image.width as usize / 2,
        image.height as usize / 2
    ));
    assert!(
        center[0] > center[1].saturating_add(40) && center[0] > center[2].saturating_add(40),
        "expected a red-dominant shaded pixel, got pixel={center:?}"
    );
}

fn render_imported_geometry_resource(
    resource_name: &str,
    imported_resource: ImportedGeometryResource,
) -> Result<RenderedImage, HeadlessCliError> {
    render_imported_geometry_resource_with_camera(
        resource_name,
        imported_resource,
        |render_scene| top_down_camera_for_bounds(render_scene.bounds),
    )
}

fn render_imported_geometry_resource_fit_camera(
    resource_name: &str,
    imported_resource: ImportedGeometryResource,
) -> Result<RenderedImage, HeadlessCliError> {
    render_imported_geometry_resource_with_camera(
        resource_name,
        imported_resource,
        |render_scene| fit_camera_to_render_scene(render_scene),
    )
}

fn render_demo_geometry_resource<F>(
    resource_name: &str,
    camera_for_scene: F,
) -> Result<RenderedImage, HeadlessCliError>
where
    F: FnOnce(&PreparedRenderScene) -> Camera,
{
    let geometry_backend = GeometryBackend::default();
    let engine = Engine::new(
        LocalGeometryBackendBridge { geometry_backend },
        NullRenderBackend::default(),
    );
    let asset = engine.build_demo_asset_for(resource_name)?;
    let image = pollster::block_on(render_prepared_scene_offscreen(
        &asset.render_scene,
        VIEWPORT,
        camera_for_scene(&asset.render_scene),
    ))?;

    Ok(image)
}

fn render_imported_geometry_resource_with_camera<F>(
    resource_name: &str,
    imported_resource: ImportedGeometryResource,
    camera_for_scene: F,
) -> Result<RenderedImage, HeadlessCliError>
where
    F: FnOnce(&PreparedRenderScene) -> Camera,
{
    let repository = ImportedResourceRepository {
        resource_name: resource_name.to_string(),
        imported_resource,
    };
    let geometry_backend = GeometryBackend::new(repository, TrivialKernel, MeshPreparePipeline);
    let engine = Engine::new(
        LocalGeometryBackendBridge { geometry_backend },
        NullRenderBackend::default(),
    );
    let asset = engine.build_demo_asset_for(resource_name)?;
    let image = pollster::block_on(render_prepared_scene_offscreen(
        &asset.render_scene,
        VIEWPORT,
        camera_for_scene(&asset.render_scene),
    ))?;

    Ok(image)
}

fn tessellated_resource(
    definition_id: GeometryDefinitionId,
    positions: Vec<DVec3>,
    faces: Vec<IndexedPolygon>,
) -> ImportedGeometryResource {
    ImportedGeometryResource {
        definition: GeometryDefinition {
            id: definition_id,
            primitive: GeometryPrimitive::Tessellated(
                TessellatedGeometry::new(positions, faces).expect("tessellated geometry"),
            ),
        },
        instances: vec![ImportedGeometryResourceInstance {
            instance: GeometryInstance {
                id: GeometryInstanceId(1),
                definition_id,
                transform: DMat4::IDENTITY,
            },
            element_id: SemanticElementId::new("ifc/test/triangle"),
            external_id: ExternalId::new("ifc/test/instance"),
            label: "Test Instance".to_string(),
            declared_entity: "IfcProduct".to_string(),
            default_render_class: DefaultRenderClass::Physical,
            display_color: None,
        }],
        source_space: cc_w_types::SourceSpace::right_handed_y_up_millimeters(),
    }
}

fn swept_solid_resource(
    definition_id: GeometryDefinitionId,
    profile: Profile2,
    path: SweepPath,
) -> ImportedGeometryResource {
    ImportedGeometryResource {
        definition: GeometryDefinition {
            id: definition_id,
            primitive: GeometryPrimitive::SweptSolid(
                SweptSolid::new(profile, path).expect("swept solid"),
            ),
        },
        instances: vec![ImportedGeometryResourceInstance {
            instance: GeometryInstance {
                id: GeometryInstanceId(1),
                definition_id,
                transform: DMat4::IDENTITY,
            },
            element_id: SemanticElementId::new("test/swept-solid"),
            external_id: ExternalId::new("test/swept-solid"),
            label: "Swept Solid".to_string(),
            declared_entity: "IfcProduct".to_string(),
            default_render_class: DefaultRenderClass::Physical,
            display_color: None,
        }],
        source_space: SourceSpace::w_world_metric(),
    }
}

fn circular_profile_sweep_resource(
    definition_id: GeometryDefinitionId,
    sweep: CircularProfileSweep,
) -> ImportedGeometryResource {
    ImportedGeometryResource {
        definition: GeometryDefinition {
            id: definition_id,
            primitive: GeometryPrimitive::CircularProfileSweep(sweep),
        },
        instances: vec![ImportedGeometryResourceInstance {
            instance: GeometryInstance {
                id: GeometryInstanceId(1),
                definition_id,
                transform: DMat4::IDENTITY,
            },
            element_id: SemanticElementId::new("test/circular-profile-sweep"),
            external_id: ExternalId::new("test/circular-profile-sweep"),
            label: "Circular Profile Sweep".to_string(),
            declared_entity: "IfcProduct".to_string(),
            default_render_class: DefaultRenderClass::Physical,
            display_color: None,
        }],
        source_space: SourceSpace::w_world_metric(),
    }
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

    Profile2::new(ProfileLoop2::new(curve).expect("closed loop"), vec![])
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

    ProfileLoop2::new(curve).expect("closed loop")
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

fn world_xy_to_source_mm(x: f64, y: f64) -> DVec3 {
    DVec3::new(x * 1_000.0, 0.0, -y * 1_000.0)
}

fn top_down_camera_for_extent(extent: f32) -> Camera {
    let distance = f64::from((extent * 2.25).max(7.0));

    Camera {
        eye: DVec3::new(0.0, 0.0, distance),
        target: DVec3::ZERO,
        up: WORLD_UP,
        vertical_fov_degrees: 28.0,
        near_plane: 0.1,
        far_plane: distance * 4.0,
    }
}

fn top_down_camera_for_bounds(bounds: Bounds3) -> Camera {
    let extent = bounds.size().max_element() as f32;
    let distance = f64::from((extent * 2.25).max(7.0));
    let target = bounds.center();

    Camera {
        eye: DVec3::new(target.x, target.y, target.z + distance),
        target,
        up: WORLD_UP,
        vertical_fov_degrees: 28.0,
        near_plane: 0.1,
        far_plane: distance * 4.0,
    }
}

fn prepared_mesh(vertices: Vec<PreparedVertex>) -> PreparedMesh {
    let positions = vertices
        .iter()
        .map(|vertex| {
            DVec3::new(
                vertex.position[0] as f64,
                vertex.position[1] as f64,
                vertex.position[2] as f64,
            )
        })
        .collect::<Vec<_>>();
    let bounds = Bounds3::from_points(&positions).expect("bounds");
    let indices = (0..vertices.len() as u32).collect();

    PreparedMesh {
        local_origin: DVec3::ZERO,
        bounds,
        vertices,
        indices,
    }
}

#[derive(Clone, Debug)]
struct ImportedResourceRepository {
    resource_name: String,
    imported_resource: ImportedGeometryResource,
}

impl SceneRepository for ImportedResourceRepository {
    fn load_demo_geometry_resource(
        &self,
        resource: &str,
    ) -> Result<GeometryResource, DbResourceError> {
        if resource == self.resource_name {
            Ok(import_geometry_resource(self.imported_resource.clone()))
        } else {
            Err(ResourceError::UnknownResource {
                requested: resource.to_string(),
                available: self.resource_name.clone(),
            })
        }
    }
}

struct LocalGeometryBackendBridge<R> {
    geometry_backend: GeometryBackend<R, TrivialKernel, MeshPreparePipeline>,
}

impl<R> GeometryPackageSource for LocalGeometryBackendBridge<R>
where
    R: SceneRepository,
{
    fn load_prepared_package(
        &self,
        resource: &str,
    ) -> Result<cc_w_types::PreparedGeometryPackage, GeometryPackageSourceError> {
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

#[derive(Clone, Copy, Debug)]
struct ForegroundStats {
    min_x: usize,
    max_x: usize,
    min_y: usize,
    max_y: usize,
    pixel_count: usize,
}

impl ForegroundStats {
    fn width(self) -> usize {
        self.max_x - self.min_x + 1
    }

    fn height(self) -> usize {
        self.max_y - self.min_y + 1
    }
}

fn foreground_stats(image: &RenderedImage) -> ForegroundStats {
    let background = background_pixel(image);
    let mut min_x = image.width as usize;
    let mut max_x = 0;
    let mut min_y = image.height as usize;
    let mut max_y = 0;
    let mut pixel_count = 0;

    for y in 0..image.height as usize {
        for x in 0..image.width as usize {
            if is_foreground(pixel_at(image, x, y), background) {
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
                pixel_count += 1;
            }
        }
    }

    assert!(pixel_count > 0, "expected rendered foreground pixels");

    ForegroundStats {
        min_x,
        max_x,
        min_y,
        max_y,
        pixel_count,
    }
}

fn foreground_segments_in_row(image: &RenderedImage, y: usize) -> Vec<(usize, usize)> {
    foreground_segments(
        (0..image.width as usize).map(|x| foreground_at(image, x, y)),
        image.width as usize,
    )
}

fn foreground_segments_in_column(image: &RenderedImage, x: usize) -> Vec<(usize, usize)> {
    foreground_segments(
        (0..image.height as usize).map(|y| foreground_at(image, x, y)),
        image.height as usize,
    )
}

fn foreground_segments<I>(values: I, length: usize) -> Vec<(usize, usize)>
where
    I: IntoIterator<Item = bool>,
{
    let mut segments = Vec::new();
    let mut start = None;

    for (index, filled) in values.into_iter().enumerate() {
        match (start, filled) {
            (None, true) => start = Some(index),
            (Some(segment_start), false) => {
                segments.push((segment_start, index - 1));
                start = None;
            }
            _ => {}
        }
    }

    if let Some(segment_start) = start {
        segments.push((segment_start, length - 1));
    }

    segments
}

fn foreground_at(image: &RenderedImage, x: usize, y: usize) -> bool {
    is_foreground(pixel_at(image, x, y), background_pixel(image))
}

fn background_pixel(image: &RenderedImage) -> [u8; 4] {
    pixel_at(image, 0, 0)
}

fn pixel_at(image: &RenderedImage, x: usize, y: usize) -> [u8; 4] {
    let index = ((y * image.width as usize) + x) * 4;
    [
        image.rgba8[index],
        image.rgba8[index + 1],
        image.rgba8[index + 2],
        image.rgba8[index + 3],
    ]
}

fn pixel_luma(pixel: [u8; 4]) -> u16 {
    u16::from(pixel[0]) + u16::from(pixel[1]) + u16::from(pixel[2])
}

fn is_foreground(pixel: [u8; 4], background: [u8; 4]) -> bool {
    pixel
        .iter()
        .zip(background)
        .map(|(left, right)| left.abs_diff(right))
        .max()
        .unwrap_or(0)
        > PIXEL_TOLERANCE
}

fn midpoint(min: usize, max: usize) -> usize {
    min + ((max - min) / 2)
}
