use cc_w_types::{
    CircularArc2, CircularArc3, CircularProfileSweep, CurveSegment2, CurveSegment3, GeometryError,
    GeometryPrimitive, IndexedPolygon, PathFrameMode, Polycurve2, Polycurve3, Profile2, SweepPath,
    SweptSolid, TessellatedGeometry, TessellationQuality, TessellationRequest, TriangleMesh,
};
use glam::{DVec2, DVec3};
use thiserror::Error;

pub trait GeometryKernel {
    fn tessellate_primitive_with_request(
        &self,
        primitive: &GeometryPrimitive,
        request: &TessellationRequest,
    ) -> Result<TriangleMesh, KernelError>;

    fn tessellate_primitive(
        &self,
        primitive: &GeometryPrimitive,
    ) -> Result<TriangleMesh, KernelError> {
        self.tessellate_primitive_with_request(primitive, &TessellationRequest::default())
    }
}

#[derive(Debug, Default)]
pub struct TrivialKernel;

impl GeometryKernel for TrivialKernel {
    fn tessellate_primitive_with_request(
        &self,
        primitive: &GeometryPrimitive,
        request: &TessellationRequest,
    ) -> Result<TriangleMesh, KernelError> {
        let request = request.validated()?;

        match primitive {
            GeometryPrimitive::Tessellated(geometry) => {
                triangulate_tessellated_geometry(geometry, &request)
            }
            GeometryPrimitive::SweptSolid(solid) => tessellate_swept_solid(solid, &request),
            GeometryPrimitive::CircularProfileSweep(sweep) => {
                tessellate_circular_profile_sweep(sweep, &request)
            }
        }
    }
}

const KERNEL_EPSILON: f64 = 1.0e-9;
const FULL_TURN_RADIANS: f64 = std::f64::consts::TAU;

#[derive(Default)]
struct MeshBuilder {
    positions: Vec<DVec3>,
    indices: Vec<[u32; 3]>,
}

impl MeshBuilder {
    fn push_triangle(&mut self, a: DVec3, b: DVec3, c: DVec3) {
        if (b - a).cross(c - a).length_squared() <= KERNEL_EPSILON {
            return;
        }

        let base = self.positions.len() as u32;
        self.positions.extend([a, b, c]);
        self.indices.push([base, base + 1, base + 2]);
    }

    fn push_quad(&mut self, a: DVec3, b: DVec3, c: DVec3, d: DVec3) {
        self.push_triangle(a, b, c);
        self.push_triangle(a, c, d);
    }

    fn build(self, kind: &'static str) -> Result<TriangleMesh, KernelError> {
        if self.indices.is_empty() {
            return Err(KernelError::CulledPrimitive { kind });
        }

        TriangleMesh::new(self.positions, self.indices).map_err(KernelError::from)
    }
}

#[derive(Clone, Debug)]
struct SampledProfile {
    outer: Vec<DVec2>,
    holes: Vec<Vec<DVec2>>,
}

impl SampledProfile {
    fn flattened_points(&self) -> Vec<DVec2> {
        let mut points = self.outer.clone();

        for hole in &self.holes {
            points.extend(hole.iter().copied());
        }

        points
    }
}

fn triangulate_tessellated_geometry(
    geometry: &TessellatedGeometry,
    request: &TessellationRequest,
) -> Result<TriangleMesh, KernelError> {
    let mut indices = Vec::new();

    for face in &geometry.faces {
        triangulate_face(geometry, face, request, &mut indices)?;
    }

    if indices.is_empty() {
        return Err(KernelError::CulledPrimitive {
            kind: "tessellated geometry",
        });
    }

    TriangleMesh::new(geometry.positions.clone(), indices).map_err(KernelError::from)
}

fn triangulate_face(
    geometry: &TessellatedGeometry,
    face: &IndexedPolygon,
    request: &TessellationRequest,
    indices: &mut Vec<[u32; 3]>,
) -> Result<(), KernelError> {
    let Some(reference_cross) = face_reference_cross(&geometry.positions, &face.exterior) else {
        return Ok(());
    };
    let face_area = 0.5 * reference_cross.length();

    if request
        .min_tessellated_face_area
        .is_some_and(|minimum_area| face_area <= minimum_area)
    {
        return Ok(());
    }

    if face.holes.is_empty() && face.exterior.len() == 3 {
        // Triangles are already render-ready primitives; they do not need polygon
        // triangulation machinery or a separate planarity check.
        indices.push([face.exterior[0], face.exterior[1], face.exterior[2]]);
        return Ok(());
    }

    let normal = reference_cross.normalize();

    if !face_is_planar(&geometry.positions, face, normal) {
        return Err(KernelError::InvalidTessellatedFace {
            kind: "non-planar tessellated polygon",
        });
    }

    let (projected_positions, hole_indices, remap) =
        flatten_face_to_projected_2d(&geometry.positions, face, normal)?;
    let triangulated = earcutr::earcut(&projected_positions, &hole_indices, 2)
        .map_err(|error| KernelError::TriangulationFailed(error.to_string()))?;

    for triangle in triangulated.chunks_exact(3) {
        let mut triangle_indices = [remap[triangle[0]], remap[triangle[1]], remap[triangle[2]]];

        if triangle_winding_opposes_normal(&geometry.positions, triangle_indices, normal) {
            triangle_indices.swap(1, 2);
        }

        indices.push(triangle_indices);
    }

    Ok(())
}

fn tessellate_swept_solid(
    solid: &SweptSolid,
    request: &TessellationRequest,
) -> Result<TriangleMesh, KernelError> {
    match &solid.path {
        SweepPath::Linear { vector } => {
            tessellate_linear_swept_solid(&solid.profile, *vector, request)
        }
        SweepPath::Revolved {
            axis,
            angle_radians,
        } => tessellate_revolved_swept_solid(&solid.profile, *axis, *angle_radians, request),
        SweepPath::AlongCurve { .. } => Err(KernelError::UnsupportedPrimitive {
            kind: "curve-following swept solids",
        }),
    }
}

fn tessellate_linear_swept_solid(
    profile: &Profile2,
    vector: DVec3,
    request: &TessellationRequest,
) -> Result<TriangleMesh, KernelError> {
    let sampled = sample_profile(profile, request)?;
    let axis_u = DVec3::X;
    let axis_v = DVec3::Y;
    let cap_points = sampled
        .flattened_points()
        .into_iter()
        .map(|point| map_profile_point(point, DVec3::ZERO, axis_u, axis_v))
        .collect::<Vec<_>>();
    let outer_ring = map_profile_ring(&sampled.outer, DVec3::ZERO, axis_u, axis_v);
    let hole_rings = sampled
        .holes
        .iter()
        .map(|ring| map_profile_ring(ring, DVec3::ZERO, axis_u, axis_v))
        .collect::<Vec<_>>();
    let cap_triangles = triangulate_sampled_profile(&sampled)?;
    let mut builder = MeshBuilder::default();

    for triangle in &cap_triangles {
        let [a, b, c] = *triangle;
        let pa = cap_points[a];
        let pb = cap_points[b];
        let pc = cap_points[c];
        builder.push_triangle(pc, pb, pa);
        builder.push_triangle(pa + vector, pb + vector, pc + vector);
    }

    append_linear_sweep_ring_sides(&mut builder, &outer_ring, vector);

    for hole in &hole_rings {
        append_linear_sweep_ring_sides(&mut builder, hole, vector);
    }

    builder.build("linear swept solid")
}

fn tessellate_revolved_swept_solid(
    profile: &Profile2,
    axis: cc_w_types::Axis3,
    angle_radians: f64,
    request: &TessellationRequest,
) -> Result<TriangleMesh, KernelError> {
    let sampled = sample_profile(profile, request)?;
    let axis_direction = axis.direction.normalize();
    let radial_axis = arbitrary_perpendicular(axis_direction);
    let profile_points = sampled
        .flattened_points()
        .iter()
        .copied()
        .map(|point| map_profile_point(point, axis.origin, radial_axis, axis_direction))
        .collect::<Vec<_>>();
    let outer_ring = map_profile_ring(&sampled.outer, axis.origin, radial_axis, axis_direction);
    let hole_rings = sampled
        .holes
        .iter()
        .map(|ring| map_profile_ring(ring, axis.origin, radial_axis, axis_direction))
        .collect::<Vec<_>>();
    let max_radius = profile_points
        .iter()
        .copied()
        .map(|point| distance_to_axis(point, axis))
        .fold(0.0, f64::max);
    let angle_steps = angular_subdivision_count(max_radius, angle_radians.abs(), request).max(1);
    let end_angle = angle_radians;
    let mut builder = MeshBuilder::default();

    if !sweep_is_closed(angle_radians) {
        let cap_triangles = triangulate_sampled_profile(&sampled)?;
        let end_profile_points = profile_points
            .iter()
            .copied()
            .map(|point| rotate_point_around_axis(point, axis, end_angle))
            .collect::<Vec<_>>();

        for triangle in cap_triangles {
            let [a, b, c] = triangle;
            builder.push_triangle(profile_points[c], profile_points[b], profile_points[a]);
            builder.push_triangle(
                end_profile_points[a],
                end_profile_points[b],
                end_profile_points[c],
            );
        }
    }

    append_revolved_ring_sides(&mut builder, &outer_ring, axis, angle_radians, angle_steps);

    for hole in &hole_rings {
        append_revolved_ring_sides(&mut builder, hole, axis, angle_radians, angle_steps);
    }

    builder.build("revolved swept solid")
}

fn tessellate_circular_profile_sweep(
    sweep: &CircularProfileSweep,
    request: &TessellationRequest,
) -> Result<TriangleMesh, KernelError> {
    let spine_points = sample_polycurve3(&sweep.spine, request)?;
    let tangents = path_tangents(&spine_points);
    let frames = path_frames(&spine_points, &tangents, request.path_frame_mode);
    let radial_steps = angular_subdivision_count(sweep.radius, FULL_TURN_RADIANS, request).max(8);
    let angles = (0..radial_steps)
        .map(|index| FULL_TURN_RADIANS * (index as f64) / (radial_steps as f64))
        .collect::<Vec<_>>();
    let outer_rings = spine_points
        .iter()
        .zip(&frames)
        .map(|(center, (normal, binormal))| {
            angles
                .iter()
                .map(|angle| {
                    *center
                        + (*normal * angle.cos() * sweep.radius)
                        + (*binormal * angle.sin() * sweep.radius)
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let inner_rings = sweep.inner_radius.map(|inner_radius| {
        spine_points
            .iter()
            .zip(&frames)
            .map(|(center, (normal, binormal))| {
                angles
                    .iter()
                    .map(|angle| {
                        *center
                            + (*normal * angle.cos() * inner_radius)
                            + (*binormal * angle.sin() * inner_radius)
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    });
    let mut builder = MeshBuilder::default();
    let start_cap_normal = -tangents[0];
    let end_cap_normal = *tangents.last().expect("spine yields end tangent");

    append_ring_strip_sides(&mut builder, &outer_rings, &spine_points, RingFace::Outward);

    if let Some(inner_rings) = &inner_rings {
        append_ring_strip_sides(&mut builder, inner_rings, &spine_points, RingFace::Inward);
    }

    if let Some(inner_rings) = &inner_rings {
        append_annulus_cap(
            &mut builder,
            &outer_rings[0],
            &inner_rings[0],
            start_cap_normal,
        );
        append_annulus_cap(
            &mut builder,
            outer_rings.last().expect("spine yields end ring"),
            inner_rings.last().expect("spine yields end ring"),
            end_cap_normal,
        );
    } else {
        append_disk_cap(
            &mut builder,
            spine_points[0],
            &outer_rings[0],
            start_cap_normal,
        );
        append_disk_cap(
            &mut builder,
            *spine_points.last().expect("spine yields end point"),
            outer_rings.last().expect("spine yields end ring"),
            end_cap_normal,
        );
    }

    builder.build("circular profile sweep")
}

fn append_linear_sweep_ring_sides(builder: &mut MeshBuilder, ring: &[DVec3], vector: DVec3) {
    for index in 0..ring.len() {
        let next = (index + 1) % ring.len();
        let start = ring[index];
        let end = ring[next];
        builder.push_quad(start, end, end + vector, start + vector);
    }
}

fn append_revolved_ring_sides(
    builder: &mut MeshBuilder,
    ring: &[DVec3],
    axis: cc_w_types::Axis3,
    angle_radians: f64,
    angle_steps: usize,
) {
    for step in 0..angle_steps {
        let start_angle = angle_radians * (step as f64) / (angle_steps as f64);
        let end_angle = angle_radians * ((step + 1) as f64) / (angle_steps as f64);

        for index in 0..ring.len() {
            let next = (index + 1) % ring.len();
            let a = rotate_point_around_axis(ring[index], axis, start_angle);
            let b = rotate_point_around_axis(ring[next], axis, start_angle);
            let c = rotate_point_around_axis(ring[next], axis, end_angle);
            let d = rotate_point_around_axis(ring[index], axis, end_angle);
            let expected_normal = revolved_quad_outward_normal(
                ring[index],
                ring[next],
                axis,
                (start_angle + end_angle) * 0.5,
                (a + b + c + d) * 0.25,
            );

            if quad_triangle_normal(a, b, c).dot(expected_normal) < 0.0 {
                builder.push_quad(a, d, c, b);
            } else {
                builder.push_quad(a, b, c, d);
            }
        }
    }
}

fn revolved_quad_outward_normal(
    start: DVec3,
    end: DVec3,
    axis: cc_w_types::Axis3,
    angle_radians: f64,
    quad_center: DVec3,
) -> DVec3 {
    let axis_direction = axis.direction.normalize();
    let rotated_start = rotate_point_around_axis(start, axis, angle_radians);
    let rotated_end = rotate_point_around_axis(end, axis, angle_radians);
    let profile_tangent = (rotated_end - rotated_start).normalize_or_zero();
    let radial = radial_direction_from_axis(quad_center, axis);
    let tangential = axis_direction.cross(radial).normalize_or_zero();
    let normal = tangential.cross(profile_tangent).normalize_or_zero();

    if normal.length_squared() <= KERNEL_EPSILON {
        radial
    } else {
        normal
    }
}

fn radial_direction_from_axis(point: DVec3, axis: cc_w_types::Axis3) -> DVec3 {
    let axis_direction = axis.direction.normalize();
    let relative = point - axis.origin;
    let radial = relative - (axis_direction * relative.dot(axis_direction));

    if radial.length_squared() <= KERNEL_EPSILON {
        arbitrary_perpendicular(axis_direction)
    } else {
        radial.normalize()
    }
}

fn quad_triangle_normal(a: DVec3, b: DVec3, c: DVec3) -> DVec3 {
    (b - a).cross(c - a).normalize_or_zero()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RingFace {
    Outward,
    Inward,
}

fn append_ring_strip_sides(
    builder: &mut MeshBuilder,
    rings: &[Vec<DVec3>],
    spine_points: &[DVec3],
    face: RingFace,
) {
    for (segment_index, pair) in rings.windows(2).enumerate() {
        let start = &pair[0];
        let end = &pair[1];
        let segment_midpoint =
            (spine_points[segment_index] + spine_points[segment_index + 1]) * 0.5;

        for index in 0..start.len() {
            let next = (index + 1) % start.len();
            let a = start[index];
            let b = start[next];
            let c = end[next];
            let d = end[index];
            let mut expected_normal = ((a + b + c + d) * 0.25) - segment_midpoint;

            if face == RingFace::Inward {
                expected_normal = -expected_normal;
            }

            push_oriented_quad(builder, a, b, c, d, expected_normal);
        }
    }
}

fn append_disk_cap(
    builder: &mut MeshBuilder,
    center: DVec3,
    ring: &[DVec3],
    outward_normal: DVec3,
) {
    for index in 0..ring.len() {
        let next = (index + 1) % ring.len();
        push_oriented_triangle(builder, center, ring[next], ring[index], outward_normal);
    }
}

fn append_annulus_cap(
    builder: &mut MeshBuilder,
    outer: &[DVec3],
    inner: &[DVec3],
    outward_normal: DVec3,
) {
    for index in 0..outer.len() {
        let next = (index + 1) % outer.len();
        push_oriented_quad(
            builder,
            outer[index],
            outer[next],
            inner[next],
            inner[index],
            outward_normal,
        );
    }
}

fn push_oriented_triangle(
    builder: &mut MeshBuilder,
    a: DVec3,
    b: DVec3,
    c: DVec3,
    expected_normal: DVec3,
) {
    if quad_triangle_normal(a, b, c).dot(expected_normal) < 0.0 {
        builder.push_triangle(a, c, b);
    } else {
        builder.push_triangle(a, b, c);
    }
}

fn push_oriented_quad(
    builder: &mut MeshBuilder,
    a: DVec3,
    b: DVec3,
    c: DVec3,
    d: DVec3,
    expected_normal: DVec3,
) {
    if quad_triangle_normal(a, b, c).dot(expected_normal) < 0.0 {
        builder.push_quad(a, d, c, b);
    } else {
        builder.push_quad(a, b, c, d);
    }
}

fn map_profile_ring(ring: &[DVec2], origin: DVec3, axis_u: DVec3, axis_v: DVec3) -> Vec<DVec3> {
    ring.iter()
        .copied()
        .map(|point| map_profile_point(point, origin, axis_u, axis_v))
        .collect()
}

fn map_profile_point(point: DVec2, origin: DVec3, axis_u: DVec3, axis_v: DVec3) -> DVec3 {
    origin + (axis_u * point.x) + (axis_v * point.y)
}

fn sample_profile(
    profile: &Profile2,
    request: &TessellationRequest,
) -> Result<SampledProfile, KernelError> {
    Ok(SampledProfile {
        outer: normalize_ring_orientation(
            sample_profile_loop(&profile.outer.curve, request)?,
            true,
        ),
        holes: profile
            .holes
            .iter()
            .map(|loop_| {
                sample_profile_loop(&loop_.curve, request)
                    .map(|ring| normalize_ring_orientation(ring, false))
            })
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn sample_profile_loop(
    curve: &Polycurve2,
    request: &TessellationRequest,
) -> Result<Vec<DVec2>, KernelError> {
    let mut points = Vec::new();

    for segment in &curve.segments {
        append_sampled_segment2(&mut points, segment, request)?;
    }

    dedupe_polyline_points(&mut points);

    if points.len() < 3 {
        return Err(KernelError::InvalidSweepPrimitive {
            kind: "profile loop sampling yielded fewer than three points",
        });
    }

    Ok(points)
}

fn sample_polycurve3(
    curve: &Polycurve3,
    request: &TessellationRequest,
) -> Result<Vec<DVec3>, KernelError> {
    let mut points = Vec::new();

    for segment in &curve.segments {
        append_sampled_segment3(&mut points, segment, request)?;
    }

    dedupe_polyline_points(&mut points);

    if points.len() < 2 {
        return Err(KernelError::InvalidSweepPrimitive {
            kind: "spine sampling yielded fewer than two points",
        });
    }

    Ok(points)
}

fn append_sampled_segment2(
    points: &mut Vec<DVec2>,
    segment: &CurveSegment2,
    request: &TessellationRequest,
) -> Result<(), KernelError> {
    match segment {
        CurveSegment2::Line(line) => {
            push_unique_point2(points, line.start);
            push_unique_point2(points, line.end);
        }
        CurveSegment2::CircularArc(arc) => {
            for point in sample_circular_arc2(arc, request)? {
                push_unique_point2(points, point);
            }
        }
    }

    Ok(())
}

fn append_sampled_segment3(
    points: &mut Vec<DVec3>,
    segment: &CurveSegment3,
    request: &TessellationRequest,
) -> Result<(), KernelError> {
    match segment {
        CurveSegment3::Line(line) => {
            push_unique_point3(points, line.start);
            push_unique_point3(points, line.end);
        }
        CurveSegment3::CircularArc(arc) => {
            for point in sample_circular_arc3(arc, request)? {
                push_unique_point3(points, point);
            }
        }
    }

    Ok(())
}

fn sample_circular_arc2(
    arc: &CircularArc2,
    request: &TessellationRequest,
) -> Result<Vec<DVec2>, KernelError> {
    let (center, radius) = circle_from_three_points_2d(arc.start, arc.mid, arc.end)?;
    let sweep = oriented_arc_sweep(
        (arc.start - center).y.atan2((arc.start - center).x),
        (arc.mid - center).y.atan2((arc.mid - center).x),
        (arc.end - center).y.atan2((arc.end - center).x),
    );
    let step_count = angular_subdivision_count(radius, sweep.abs(), request).max(1);

    Ok((0..=step_count)
        .map(|index| {
            let angle = (arc.start - center).y.atan2((arc.start - center).x)
                + (sweep * (index as f64) / (step_count as f64));
            center + DVec2::new(angle.cos() * radius, angle.sin() * radius)
        })
        .collect())
}

fn sample_circular_arc3(
    arc: &CircularArc3,
    request: &TessellationRequest,
) -> Result<Vec<DVec3>, KernelError> {
    let plane_normal = (arc.mid - arc.start).cross(arc.end - arc.start).normalize();
    let axis_u = (arc.end - arc.start).normalize();
    let axis_v = plane_normal.cross(axis_u).normalize();
    let start = DVec2::ZERO;
    let mid_3d = arc.mid - arc.start;
    let end_3d = arc.end - arc.start;
    let mid = DVec2::new(mid_3d.dot(axis_u), mid_3d.dot(axis_v));
    let end = DVec2::new(end_3d.dot(axis_u), end_3d.dot(axis_v));
    let sampled = sample_circular_arc2(
        &CircularArc2::new(start, mid, end).map_err(KernelError::from)?,
        request,
    )?;

    Ok(sampled
        .into_iter()
        .map(|point| arc.start + (axis_u * point.x) + (axis_v * point.y))
        .collect())
}

fn triangulate_sampled_profile(profile: &SampledProfile) -> Result<Vec<[usize; 3]>, KernelError> {
    let mut flat_points = Vec::new();
    let mut hole_indices = Vec::new();
    let mut remap = Vec::new();

    append_flat_ring(&profile.outer, &mut flat_points, &mut remap);

    for hole in &profile.holes {
        hole_indices.push(remap.len());
        append_flat_ring(hole, &mut flat_points, &mut remap);
    }

    let triangulated = earcutr::earcut(&flat_points, &hole_indices, 2)
        .map_err(|error| KernelError::TriangulationFailed(error.to_string()))?;

    Ok(triangulated
        .chunks_exact(3)
        .map(|triangle| [remap[triangle[0]], remap[triangle[1]], remap[triangle[2]]])
        .collect())
}

fn append_flat_ring(ring: &[DVec2], flat_points: &mut Vec<f64>, remap: &mut Vec<usize>) {
    for point in ring {
        flat_points.push(point.x);
        flat_points.push(point.y);
        remap.push(remap.len());
    }
}

fn normalize_ring_orientation(mut ring: Vec<DVec2>, want_ccw: bool) -> Vec<DVec2> {
    let is_ccw = signed_ring_area(&ring) > 0.0;

    if is_ccw != want_ccw {
        ring.reverse();
    }

    ring
}

fn signed_ring_area(ring: &[DVec2]) -> f64 {
    let mut area = 0.0;

    for index in 0..ring.len() {
        let next = (index + 1) % ring.len();
        area += (ring[index].x * ring[next].y) - (ring[next].x * ring[index].y);
    }

    area * 0.5
}

fn dedupe_polyline_points<T>(points: &mut Vec<T>)
where
    T: Copy + PartialEq,
{
    points.dedup();

    if points.len() > 1 && points.first() == points.last() {
        points.pop();
    }
}

fn push_unique_point2(points: &mut Vec<DVec2>, point: DVec2) {
    if points
        .last()
        .is_none_or(|last| !last.abs_diff_eq(point, KERNEL_EPSILON))
    {
        points.push(point);
    }
}

fn push_unique_point3(points: &mut Vec<DVec3>, point: DVec3) {
    if points
        .last()
        .is_none_or(|last| !last.abs_diff_eq(point, KERNEL_EPSILON))
    {
        points.push(point);
    }
}

fn path_tangents(points: &[DVec3]) -> Vec<DVec3> {
    (0..points.len())
        .map(|index| {
            let tangent = if index == 0 {
                points[1] - points[0]
            } else if index + 1 == points.len() {
                points[index] - points[index - 1]
            } else {
                (points[index + 1] - points[index - 1]) * 0.5
            };

            tangent.normalize_or(DVec3::Z)
        })
        .collect()
}

fn path_frames(points: &[DVec3], tangents: &[DVec3], mode: PathFrameMode) -> Vec<(DVec3, DVec3)> {
    let mut frames = Vec::with_capacity(points.len());
    let mut normal = arbitrary_perpendicular(tangents[0]);
    let mut binormal = tangents[0].cross(normal).normalize_or(DVec3::X);
    frames.push((normal, binormal));

    for index in 1..points.len() {
        let previous_tangent = tangents[index - 1];
        let tangent = tangents[index];

        normal = match mode {
            PathFrameMode::ParallelTransport => transport_normal(normal, previous_tangent, tangent),
            PathFrameMode::Frenet => frenet_normal(points, tangents, index)
                .unwrap_or_else(|| transport_normal(normal, previous_tangent, tangent)),
        };
        normal = reproject_normal(normal, tangent);
        binormal = tangent
            .cross(normal)
            .normalize_or(arbitrary_perpendicular(tangent));
        frames.push((normal, binormal));
    }

    frames
}

fn transport_normal(normal: DVec3, previous_tangent: DVec3, tangent: DVec3) -> DVec3 {
    let rotation_axis = previous_tangent.cross(tangent);
    let axis_length = rotation_axis.length();

    if axis_length <= KERNEL_EPSILON {
        if previous_tangent.dot(tangent) < 0.0 {
            arbitrary_perpendicular(tangent)
        } else {
            normal
        }
    } else {
        rotate_vector_around_axis(
            normal,
            rotation_axis / axis_length,
            previous_tangent.angle_between(tangent),
        )
    }
}

fn frenet_normal(points: &[DVec3], tangents: &[DVec3], index: usize) -> Option<DVec3> {
    let derivative = if index == 0 {
        tangents[1] - tangents[0]
    } else if index + 1 == points.len() {
        tangents[index] - tangents[index - 1]
    } else {
        tangents[index + 1] - tangents[index - 1]
    };

    if derivative.length_squared() <= KERNEL_EPSILON {
        None
    } else {
        Some(derivative.normalize())
    }
}

fn reproject_normal(normal: DVec3, tangent: DVec3) -> DVec3 {
    let projected = normal - (tangent * normal.dot(tangent));

    if projected.length_squared() <= KERNEL_EPSILON {
        arbitrary_perpendicular(tangent)
    } else {
        projected.normalize()
    }
}

fn arbitrary_perpendicular(vector: DVec3) -> DVec3 {
    let helper = if vector.z.abs() < 0.9 {
        DVec3::Z
    } else {
        DVec3::X
    };
    helper.cross(vector).normalize()
}

fn rotate_point_around_axis(point: DVec3, axis: cc_w_types::Axis3, angle_radians: f64) -> DVec3 {
    axis.origin
        + rotate_vector_around_axis(
            point - axis.origin,
            axis.direction.normalize(),
            angle_radians,
        )
}

fn rotate_vector_around_axis(vector: DVec3, axis: DVec3, angle_radians: f64) -> DVec3 {
    let cos_theta = angle_radians.cos();
    let sin_theta = angle_radians.sin();

    (vector * cos_theta)
        + (axis.cross(vector) * sin_theta)
        + (axis * axis.dot(vector) * (1.0 - cos_theta))
}

fn distance_to_axis(point: DVec3, axis: cc_w_types::Axis3) -> f64 {
    let direction = axis.direction.normalize();
    let relative = point - axis.origin;
    (relative - (direction * relative.dot(direction))).length()
}

fn oriented_arc_sweep(start_angle: f64, mid_angle: f64, end_angle: f64) -> f64 {
    let ccw_end = (end_angle - start_angle).rem_euclid(FULL_TURN_RADIANS);
    let ccw_mid = (mid_angle - start_angle).rem_euclid(FULL_TURN_RADIANS);

    if ccw_mid <= ccw_end {
        ccw_end
    } else {
        ccw_end - FULL_TURN_RADIANS
    }
}

fn circle_from_three_points_2d(
    start: DVec2,
    mid: DVec2,
    end: DVec2,
) -> Result<(DVec2, f64), KernelError> {
    let determinant = 2.0
        * ((start.x * (mid.y - end.y)) + (mid.x * (end.y - start.y)) + (end.x * (start.y - mid.y)));

    if determinant.abs() <= KERNEL_EPSILON {
        return Err(KernelError::InvalidSweepPrimitive {
            kind: "arc sampling requires non-collinear control points",
        });
    }

    let start_sq = start.length_squared();
    let mid_sq = mid.length_squared();
    let end_sq = end.length_squared();
    let center = DVec2::new(
        ((start_sq * (mid.y - end.y))
            + (mid_sq * (end.y - start.y))
            + (end_sq * (start.y - mid.y)))
            / determinant,
        ((start_sq * (end.x - mid.x))
            + (mid_sq * (start.x - end.x))
            + (end_sq * (mid.x - start.x)))
            / determinant,
    );

    Ok((center, center.distance(start)))
}

fn angular_subdivision_count(
    radius: f64,
    angle_radians: f64,
    request: &TessellationRequest,
) -> usize {
    if !radius.is_finite() || radius <= KERNEL_EPSILON || angle_radians <= KERNEL_EPSILON {
        return 1;
    }

    let chord_limit = if request.chord_tolerance >= radius {
        FULL_TURN_RADIANS
    } else {
        2.0 * (1.0 - (request.chord_tolerance / radius)).acos()
    };
    let mut max_step = chord_limit.min(request.normal_tolerance_radians);

    if let Some(max_edge_length) = request.max_edge_length {
        max_step = max_step.min(max_edge_length / radius);
    }

    let quality_step = FULL_TURN_RADIANS / (minimum_full_turn_segments(request.quality) as f64);
    max_step = max_step.min(quality_step);

    if !max_step.is_finite() || max_step <= KERNEL_EPSILON {
        return minimum_full_turn_segments(request.quality).max(1);
    }

    (angle_radians / max_step).ceil().max(1.0) as usize
}

fn minimum_full_turn_segments(quality: TessellationQuality) -> usize {
    match quality {
        TessellationQuality::Draft => 12,
        TessellationQuality::Balanced => 24,
        TessellationQuality::Fine => 48,
    }
}

fn sweep_is_closed(angle_radians: f64) -> bool {
    let remainder = angle_radians.abs().rem_euclid(FULL_TURN_RADIANS);
    remainder <= 1.0e-6 || (FULL_TURN_RADIANS - remainder) <= 1.0e-6
}

fn face_reference_cross(positions: &[DVec3], ring: &[u32]) -> Option<DVec3> {
    if ring.len() < 3 {
        return None;
    }

    let origin = positions[ring[0] as usize];
    let mut reference_cross = None;
    let mut reference_cross_length_squared = 0.0;

    for index in 1..ring.len() - 1 {
        let a = positions[ring[index] as usize] - origin;
        let b = positions[ring[index + 1] as usize] - origin;
        let cross = a.cross(b);
        let cross_length_squared = cross.length_squared();

        if cross_length_squared > reference_cross_length_squared {
            reference_cross = Some(cross);
            reference_cross_length_squared = cross_length_squared;
        }
    }

    reference_cross.filter(|_| reference_cross_length_squared > 0.0)
}

fn face_is_planar(positions: &[DVec3], face: &IndexedPolygon, normal: DVec3) -> bool {
    let origin = positions[face.exterior[0] as usize];

    face.exterior
        .iter()
        .chain(face.holes.iter().flat_map(|ring| ring.iter()))
        .all(|&index| {
            let point = positions[index as usize];
            (point - origin).dot(normal).abs() <= 1.0e-6
        })
}

fn flatten_face_to_projected_2d(
    positions: &[DVec3],
    face: &IndexedPolygon,
    normal: DVec3,
) -> Result<(Vec<f64>, Vec<usize>, Vec<u32>), KernelError> {
    let basis = face_projection_basis(normal);
    let mut projected_positions = Vec::new();
    let mut hole_indices = Vec::new();
    let mut remap = Vec::new();

    append_projected_ring(
        positions,
        &face.exterior,
        basis,
        &mut projected_positions,
        &mut remap,
    );

    for hole in &face.holes {
        hole_indices.push(remap.len());
        append_projected_ring(positions, hole, basis, &mut projected_positions, &mut remap);
    }

    if remap.len() < 3 {
        return Err(KernelError::InvalidTessellatedFace {
            kind: "too few projected tessellated vertices",
        });
    }

    Ok((projected_positions, hole_indices, remap))
}

fn append_projected_ring(
    positions: &[DVec3],
    ring: &[u32],
    basis: (DVec3, DVec3),
    projected_positions: &mut Vec<f64>,
    remap: &mut Vec<u32>,
) {
    for &index in ring {
        let point = positions[index as usize];
        projected_positions.push(point.dot(basis.0));
        projected_positions.push(point.dot(basis.1));
        remap.push(index);
    }
}

fn face_projection_basis(normal: DVec3) -> (DVec3, DVec3) {
    let helper = if normal.z.abs() < 0.9 {
        DVec3::Z
    } else {
        DVec3::X
    };
    let axis_u = helper.cross(normal).normalize();
    let axis_v = normal.cross(axis_u).normalize();
    (axis_u, axis_v)
}

fn triangle_winding_opposes_normal(positions: &[DVec3], triangle: [u32; 3], normal: DVec3) -> bool {
    let a = positions[triangle[0] as usize];
    let b = positions[triangle[1] as usize];
    let c = positions[triangle[2] as usize];
    (b - a).cross(c - a).dot(normal) < 0.0
}

#[derive(Debug, Error)]
pub enum KernelError {
    #[error(transparent)]
    Geometry(#[from] GeometryError),
    #[error("tessellated face triangulation failed: {0}")]
    TriangulationFailed(String),
    #[error("primitive was culled during tessellation: {kind}")]
    CulledPrimitive { kind: &'static str },
    #[error("invalid tessellated face: {kind}")]
    InvalidTessellatedFace { kind: &'static str },
    #[error("invalid swept primitive: {kind}")]
    InvalidSweepPrimitive { kind: &'static str },
    #[error("the trivial kernel does not yet support {kind}")]
    UnsupportedPrimitive { kind: &'static str },
}

#[cfg(test)]
mod tests {
    use super::*;
    use cc_w_types::{
        Axis3, CircularProfileSweep, CurveSegment3, GeometryPrimitive, IndexedPolygon,
        LineSegment3, NormalGenerationMode, PathFrameMode, Polycurve3, Profile2, ProfileLoop2,
        SweepPath, SweptSolid, TessellatedGeometry, TessellationQuality, TessellationRequest,
    };
    use glam::{DVec2, DVec3};

    fn sample_profile() -> Profile2 {
        let loop_curve = cc_w_types::Polycurve2::new(vec![
            cc_w_types::CurveSegment2::Line(cc_w_types::LineSegment2 {
                start: DVec2::ZERO,
                end: DVec2::X,
            }),
            cc_w_types::CurveSegment2::Line(cc_w_types::LineSegment2 {
                start: DVec2::X,
                end: DVec2::new(1.0, 1.0),
            }),
            cc_w_types::CurveSegment2::Line(cc_w_types::LineSegment2 {
                start: DVec2::new(1.0, 1.0),
                end: DVec2::ZERO,
            }),
        ])
        .expect("curve");

        Profile2::new(ProfileLoop2::new(loop_curve).expect("loop"), vec![])
    }

    fn rectangular_loop(min: DVec2, max: DVec2) -> ProfileLoop2 {
        let a = min;
        let b = DVec2::new(max.x, min.y);
        let c = max;
        let d = DVec2::new(min.x, max.y);
        let curve = cc_w_types::Polycurve2::new(vec![
            cc_w_types::CurveSegment2::Line(cc_w_types::LineSegment2 { start: a, end: b }),
            cc_w_types::CurveSegment2::Line(cc_w_types::LineSegment2 { start: b, end: c }),
            cc_w_types::CurveSegment2::Line(cc_w_types::LineSegment2 { start: c, end: d }),
            cc_w_types::CurveSegment2::Line(cc_w_types::LineSegment2 { start: d, end: a }),
        ])
        .expect("rectangle");

        ProfileLoop2::new(curve).expect("loop")
    }

    fn capsule_profile(half_length: f64, radius: f64) -> Profile2 {
        let left_bottom = DVec2::new(-half_length, -radius);
        let right_bottom = DVec2::new(half_length, -radius);
        let right_top = DVec2::new(half_length, radius);
        let left_top = DVec2::new(-half_length, radius);
        let curve = cc_w_types::Polycurve2::new(vec![
            cc_w_types::CurveSegment2::Line(cc_w_types::LineSegment2 {
                start: left_bottom,
                end: right_bottom,
            }),
            cc_w_types::CurveSegment2::CircularArc(
                cc_w_types::CircularArc2::new(
                    right_bottom,
                    DVec2::new(half_length + radius, 0.0),
                    right_top,
                )
                .expect("arc"),
            ),
            cc_w_types::CurveSegment2::Line(cc_w_types::LineSegment2 {
                start: right_top,
                end: left_top,
            }),
            cc_w_types::CurveSegment2::CircularArc(
                cc_w_types::CircularArc2::new(
                    left_top,
                    DVec2::new(-(half_length + radius), 0.0),
                    left_bottom,
                )
                .expect("arc"),
            ),
        ])
        .expect("curve");

        Profile2::new(ProfileLoop2::new(curve).expect("loop"), vec![])
    }

    fn point_in_triangle_2d(point: DVec2, a: DVec2, b: DVec2, c: DVec2) -> bool {
        fn edge(p: DVec2, a: DVec2, b: DVec2) -> f64 {
            (p.x - b.x) * (a.y - b.y) - (a.x - b.x) * (p.y - b.y)
        }

        let ab = edge(point, a, b);
        let bc = edge(point, b, c);
        let ca = edge(point, c, a);
        let has_negative = ab < -1.0e-9 || bc < -1.0e-9 || ca < -1.0e-9;
        let has_positive = ab > 1.0e-9 || bc > 1.0e-9 || ca > 1.0e-9;

        !(has_negative && has_positive)
    }

    fn canonicalize_triangle(mut triangle: [u32; 3]) -> [u32; 3] {
        let min_index = triangle
            .iter()
            .enumerate()
            .min_by_key(|(_, value)| *value)
            .map(|(index, _)| index)
            .expect("triangle");
        triangle.rotate_left(min_index);
        triangle
    }

    fn canonicalize_triangles(triangles: &[[u32; 3]]) -> Vec<[u32; 3]> {
        let mut triangles = triangles
            .iter()
            .copied()
            .map(canonicalize_triangle)
            .collect::<Vec<_>>();
        triangles.sort_unstable();
        triangles
    }

    #[test]
    fn tessellated_triangles_become_triangle_mesh() {
        let geometry = TessellatedGeometry::new(
            vec![
                DVec3::new(-1.0, -1.0, 0.0),
                DVec3::new(1.0, -1.0, 0.0),
                DVec3::new(1.0, 1.0, 0.0),
                DVec3::new(-1.0, 1.0, 0.0),
            ],
            vec![
                IndexedPolygon::new(vec![0, 1, 2], vec![], 4).expect("triangle"),
                IndexedPolygon::new(vec![0, 2, 3], vec![], 4).expect("triangle"),
            ],
        )
        .expect("tessellation");

        let mesh = TrivialKernel
            .tessellate_primitive(&GeometryPrimitive::Tessellated(geometry))
            .expect("mesh");

        assert_eq!(mesh.triangle_count(), 2);
        assert_eq!(
            canonicalize_triangles(&mesh.indices),
            canonicalize_triangles(&[[0, 1, 2], [0, 2, 3]])
        );
    }

    #[test]
    fn tessellated_convex_polygon_becomes_triangle_fan() {
        let geometry = TessellatedGeometry::new(
            vec![
                DVec3::new(-1.0, -1.0, 0.0),
                DVec3::new(1.0, -1.0, 0.0),
                DVec3::new(1.0, 1.0, 0.0),
                DVec3::new(-1.0, 1.0, 0.0),
            ],
            vec![IndexedPolygon::new(vec![0, 1, 2, 3], vec![], 4).expect("quad")],
        )
        .expect("tessellation");

        let mesh = TrivialKernel
            .tessellate_primitive(&GeometryPrimitive::Tessellated(geometry))
            .expect("mesh");

        assert_eq!(mesh.triangle_count(), 2);
        assert_eq!(
            canonicalize_triangles(&mesh.indices),
            canonicalize_triangles(&[[0, 1, 2], [0, 2, 3]])
        );
    }

    #[test]
    fn tessellation_request_is_accepted_for_tessellated_geometry() {
        let geometry = TessellatedGeometry::new(
            vec![
                DVec3::new(-1.0, -1.0, 0.0),
                DVec3::new(1.0, -1.0, 0.0),
                DVec3::new(1.0, 1.0, 0.0),
                DVec3::new(-1.0, 1.0, 0.0),
            ],
            vec![IndexedPolygon::new(vec![0, 1, 2, 3], vec![], 4).expect("quad")],
        )
        .expect("tessellation");
        let request = TessellationRequest {
            quality: TessellationQuality::Fine,
            chord_tolerance: 0.005,
            normal_tolerance_radians: 2.5_f64.to_radians(),
            max_edge_length: Some(0.5),
            min_tessellated_face_area: None,
            normal_mode: NormalGenerationMode::Smooth,
            path_frame_mode: PathFrameMode::ParallelTransport,
        };

        let mesh = TrivialKernel
            .tessellate_primitive_with_request(&GeometryPrimitive::Tessellated(geometry), &request)
            .expect("mesh");

        assert_eq!(mesh.triangle_count(), 2);
    }

    #[test]
    fn small_world_space_tessellated_triangle_is_retained_by_default() {
        let geometry = TessellatedGeometry::new(
            vec![
                DVec3::new(
                    -0.6031602021555037,
                    0.2945945945946744,
                    0.049999999998914426,
                ),
                DVec3::new(-0.6102356451318109, 0.2813344594595385, 0.04999999999894908),
                DVec3::new(
                    -0.6068471046702699,
                    0.2865920608108901,
                    0.049999999998937525,
                ),
            ],
            vec![IndexedPolygon::new(vec![0, 1, 2], vec![], 3).expect("face")],
        )
        .expect("tessellation");

        let mesh = TrivialKernel
            .tessellate_primitive(&GeometryPrimitive::Tessellated(geometry))
            .expect("small world-space face should still tessellate");

        assert_eq!(mesh.triangle_count(), 1);
    }

    #[test]
    fn tessellated_face_area_threshold_can_cull_small_world_space_faces() {
        let geometry = TessellatedGeometry::new(
            vec![
                DVec3::new(
                    -0.6031602021555037,
                    0.2945945945946744,
                    0.049999999998914426,
                ),
                DVec3::new(-0.6102356451318109, 0.2813344594595385, 0.04999999999894908),
                DVec3::new(
                    -0.6068471046702699,
                    0.2865920608108901,
                    0.049999999998937525,
                ),
            ],
            vec![IndexedPolygon::new(vec![0, 1, 2], vec![], 3).expect("face")],
        )
        .expect("tessellation");
        let request = TessellationRequest {
            min_tessellated_face_area: Some(1.0e-5),
            ..TessellationRequest::default()
        };

        let result = TrivialKernel
            .tessellate_primitive_with_request(&GeometryPrimitive::Tessellated(geometry), &request);

        assert!(matches!(
            result,
            Err(KernelError::CulledPrimitive {
                kind: "tessellated geometry"
            })
        ));
    }

    #[test]
    fn infra_landscaping_face_164_is_culled_by_default() {
        // IFC fixture provenance:
        // model=infra-landscaping, item_id=1223, face_index=164.
        // This is an almost-collinear sliver triangle with effectively zero world-space area.
        // It should be handled by the face-area cull path before any polygon triangulation logic.
        let geometry = TessellatedGeometry::new(
            vec![
                DVec3::new(16.00185244070643, 14.504847056921783, 0.2900000000000084),
                DVec3::new(15.628061141992509, 14.131055758207875, 0.027950849718744895),
                DVec3::new(15.930531602933596, 14.43352621914894, 0.23999999999993904),
            ],
            vec![IndexedPolygon::new(vec![0, 1, 2], vec![], 3).expect("face")],
        )
        .expect("tessellation");

        let result = TrivialKernel.tessellate_primitive(&GeometryPrimitive::Tessellated(geometry));

        assert!(matches!(
            result,
            Err(KernelError::CulledPrimitive {
                kind: "tessellated geometry"
            })
        ));
    }

    #[test]
    fn tessellated_faces_with_holes_are_triangulated() {
        let geometry = TessellatedGeometry::new(
            vec![
                DVec3::new(0.0, 0.0, 0.0),
                DVec3::new(3.0, 0.0, 0.0),
                DVec3::new(3.0, 3.0, 0.0),
                DVec3::new(0.0, 3.0, 0.0),
                DVec3::new(1.0, 1.0, 0.0),
                DVec3::new(2.0, 1.0, 0.0),
                DVec3::new(2.0, 2.0, 0.0),
                DVec3::new(1.0, 2.0, 0.0),
            ],
            vec![IndexedPolygon::new(vec![0, 1, 2, 3], vec![vec![4, 5, 6, 7]], 8).expect("face")],
        )
        .expect("tessellation");

        let mesh = TrivialKernel
            .tessellate_primitive(&GeometryPrimitive::Tessellated(geometry))
            .expect("holey faces should triangulate");

        assert_eq!(mesh.triangle_count(), 8);
        assert!(
            mesh.indices
                .iter()
                .all(|triangle| triangle[0] != triangle[1])
        );
    }

    #[test]
    fn tessellated_concave_polygon_is_triangulated() {
        let geometry = TessellatedGeometry::new(
            vec![
                DVec3::new(0.0, 0.0, 0.0),
                DVec3::new(3.0, 0.0, 0.0),
                DVec3::new(3.0, 1.0, 0.0),
                DVec3::new(1.5, 1.0, 0.0),
                DVec3::new(1.5, 3.0, 0.0),
                DVec3::new(0.0, 3.0, 0.0),
            ],
            vec![IndexedPolygon::new(vec![0, 1, 2, 3, 4, 5], vec![], 6).expect("face")],
        )
        .expect("tessellation");

        let mesh = TrivialKernel
            .tessellate_primitive(&GeometryPrimitive::Tessellated(geometry))
            .expect("concave face should triangulate");

        assert_eq!(mesh.triangle_count(), 4);
    }

    #[test]
    fn linear_swept_solids_tessellate_into_prism_meshes() {
        let solid = SweptSolid::new(
            sample_profile(),
            SweepPath::Linear {
                vector: DVec3::new(0.0, 0.0, 2.0),
            },
        )
        .expect("solid");

        let mesh = TrivialKernel
            .tessellate_primitive(&GeometryPrimitive::SweptSolid(solid))
            .expect("linear swept solid should tessellate");

        assert_eq!(mesh.triangle_count(), 8);
        assert!(mesh.bounds.min.z.abs() <= 1.0e-12);
        assert!((mesh.bounds.max.z - 2.0).abs() <= 1.0e-12);
    }

    #[test]
    fn linear_swept_solids_keep_profile_in_local_xy_plane() {
        let solid = SweptSolid::new(
            Profile2::new(
                rectangular_loop(DVec2::new(2.0, -1.0), DVec2::new(5.0, 3.0)),
                vec![],
            ),
            SweepPath::Linear {
                vector: DVec3::new(0.0, 0.0, 2.0),
            },
        )
        .expect("solid");

        let mesh = TrivialKernel
            .tessellate_primitive(&GeometryPrimitive::SweptSolid(solid))
            .expect("linear swept solid should tessellate");

        assert!((mesh.bounds.min.x - 2.0).abs() <= 1.0e-12);
        assert!((mesh.bounds.max.x - 5.0).abs() <= 1.0e-12);
        assert!((mesh.bounds.min.y + 1.0).abs() <= 1.0e-12);
        assert!((mesh.bounds.max.y - 3.0).abs() <= 1.0e-12);
        assert!(mesh.bounds.min.z.abs() <= 1.0e-12);
        assert!((mesh.bounds.max.z - 2.0).abs() <= 1.0e-12);
    }

    #[test]
    fn tiny_linear_swept_solids_are_culled_instead_of_failing() {
        let solid = SweptSolid::new(
            Profile2::new(rectangular_loop(DVec2::ZERO, DVec2::splat(1.0e-6)), vec![]),
            SweepPath::Linear {
                vector: DVec3::new(0.0, 0.0, 1.0),
            },
        )
        .expect("solid");

        let result = TrivialKernel.tessellate_primitive(&GeometryPrimitive::SweptSolid(solid));

        assert!(matches!(
            result,
            Err(KernelError::CulledPrimitive {
                kind: "linear swept solid"
            })
        ));
    }

    #[test]
    fn arc_heavy_linear_swept_solids_tessellate_sampled_profiles() {
        let solid = SweptSolid::new(
            capsule_profile(1.0, 0.5),
            SweepPath::Linear {
                vector: DVec3::new(0.0, 0.0, 1.8),
            },
        )
        .expect("solid");
        let request = TessellationRequest {
            quality: TessellationQuality::Fine,
            chord_tolerance: 0.01,
            normal_tolerance_radians: 4.0_f64.to_radians(),
            max_edge_length: Some(0.15),
            min_tessellated_face_area: None,
            normal_mode: NormalGenerationMode::Flat,
            path_frame_mode: PathFrameMode::ParallelTransport,
        };

        let mesh = TrivialKernel
            .tessellate_primitive_with_request(&GeometryPrimitive::SweptSolid(solid), &request)
            .expect("arc-heavy swept solid should tessellate");

        assert!(mesh.triangle_count() > 120);
        assert!(mesh.bounds.min.z.abs() <= 1.0e-12);
        assert!((mesh.bounds.max.z - 1.8).abs() <= 1.0e-12);
    }

    #[test]
    fn linear_swept_solids_with_holes_keep_cap_centers_open() {
        let profile = Profile2::new(
            rectangular_loop(DVec2::new(-1.5, -1.5), DVec2::new(1.5, 1.5)),
            vec![rectangular_loop(
                DVec2::new(-0.6, -0.6),
                DVec2::new(0.6, 0.6),
            )],
        );
        let solid = SweptSolid::new(
            profile,
            SweepPath::Linear {
                vector: DVec3::new(0.0, 0.0, 2.0),
            },
        )
        .expect("solid");

        let mesh = TrivialKernel
            .tessellate_primitive(&GeometryPrimitive::SweptSolid(solid))
            .expect("linear swept solid with hole should tessellate");
        let cap_covers_center = |z: f64| {
            mesh.indices.iter().any(|triangle| {
                let [a, b, c] = *triangle;
                let pa = mesh.positions[a as usize];
                let pb = mesh.positions[b as usize];
                let pc = mesh.positions[c as usize];

                (pa.z - z).abs() <= 1.0e-9
                    && (pb.z - z).abs() <= 1.0e-9
                    && (pc.z - z).abs() <= 1.0e-9
                    && point_in_triangle_2d(
                        DVec2::ZERO,
                        DVec2::new(pa.x, pa.y),
                        DVec2::new(pb.x, pb.y),
                        DVec2::new(pc.x, pc.y),
                    )
            })
        };

        assert!(!cap_covers_center(0.0));
        assert!(!cap_covers_center(2.0));
    }

    #[test]
    fn revolved_swept_solids_tessellate_into_lathed_meshes() {
        let solid = SweptSolid::new(
            sample_profile(),
            SweepPath::Revolved {
                axis: Axis3::new(DVec3::ZERO, DVec3::Z).expect("axis"),
                angle_radians: std::f64::consts::PI * 1.5,
            },
        )
        .expect("solid");

        let mesh = TrivialKernel
            .tessellate_primitive(&GeometryPrimitive::SweptSolid(solid))
            .expect("revolved swept solid should tessellate");

        assert!(mesh.triangle_count() > 100);
        assert!(mesh.bounds.max.x > 0.95);
        assert!(mesh.bounds.min.x < -0.95);
        assert!(mesh.bounds.max.y > 0.95);
        assert!(mesh.bounds.min.y < -0.95);
        assert!(mesh.bounds.max.z >= 1.0 - 1.0e-12);
    }

    #[test]
    fn revolved_swept_solid_outer_wall_faces_point_outward() {
        let solid = SweptSolid::new(
            Profile2::new(
                rectangular_loop(DVec2::new(0.55, -0.8), DVec2::new(1.0, 0.8)),
                vec![],
            ),
            SweepPath::Revolved {
                axis: Axis3::new(DVec3::ZERO, DVec3::Z).expect("axis"),
                angle_radians: std::f64::consts::TAU,
            },
        )
        .expect("solid");

        let mesh = TrivialKernel
            .tessellate_primitive(&GeometryPrimitive::SweptSolid(solid))
            .expect("revolved swept solid should tessellate");
        let mut outer_wall_triangles = 0;

        for triangle in &mesh.indices {
            let [a, b, c] = *triangle;
            let pa = mesh.positions[a as usize];
            let pb = mesh.positions[b as usize];
            let pc = mesh.positions[c as usize];
            let centroid = (pa + pb + pc) / 3.0;
            let normal = (pb - pa).cross(pc - pa).normalize_or_zero();
            let radius = DVec2::new(centroid.x, centroid.y).length();

            if radius >= 0.95 && normal.z.abs() <= 0.25 {
                outer_wall_triangles += 1;
                let radial = DVec3::new(centroid.x, centroid.y, 0.0).normalize_or_zero();

                assert!(
                    normal.dot(radial) > 0.2,
                    "expected outward wall normal, got centroid={centroid:?}, normal={normal:?}"
                );
            }
        }

        assert!(outer_wall_triangles > 0, "expected outer wall triangles");
    }

    #[test]
    fn circular_profile_sweeps_tessellate_into_tube_meshes() {
        let spine = Polycurve3::new(vec![CurveSegment3::Line(LineSegment3 {
            start: DVec3::ZERO,
            end: DVec3::Z,
        })])
        .expect("spine");
        let sweep = CircularProfileSweep::new(spine, 0.2, Some(0.1)).expect("sweep");

        let mesh = TrivialKernel
            .tessellate_primitive(&GeometryPrimitive::CircularProfileSweep(sweep))
            .expect("circular sweep should tessellate");

        assert!(mesh.triangle_count() > 150);
        assert!((mesh.bounds.min.x + 0.2).abs() <= 1.0e-12);
        assert!((mesh.bounds.max.x - 0.2).abs() <= 1.0e-12);
        assert!((mesh.bounds.min.y + 0.2).abs() <= 1.0e-12);
        assert!((mesh.bounds.max.y - 0.2).abs() <= 1.0e-12);
        assert!(mesh.bounds.min.z.abs() <= 1.0e-12);
        assert!((mesh.bounds.max.z - 1.0).abs() <= 1.0e-12);
    }

    #[test]
    fn circular_profile_sweep_caps_and_inner_wall_face_outward() {
        let spine = Polycurve3::new(vec![CurveSegment3::Line(LineSegment3 {
            start: DVec3::ZERO,
            end: DVec3::Z,
        })])
        .expect("spine");
        let sweep = CircularProfileSweep::new(spine, 0.2, Some(0.1)).expect("sweep");

        let mesh = TrivialKernel
            .tessellate_primitive(&GeometryPrimitive::CircularProfileSweep(sweep))
            .expect("circular sweep should tessellate");

        let mut start_cap_triangles = 0;
        let mut end_cap_triangles = 0;
        let mut inner_wall_triangles = 0;

        for triangle in &mesh.indices {
            let pa = mesh.positions[triangle[0] as usize];
            let pb = mesh.positions[triangle[1] as usize];
            let pc = mesh.positions[triangle[2] as usize];
            let centroid = (pa + pb + pc) / 3.0;
            let normal = (pb - pa).cross(pc - pa).normalize_or_zero();

            if pa.z.abs() <= 1.0e-9 && pb.z.abs() <= 1.0e-9 && pc.z.abs() <= 1.0e-9 {
                start_cap_triangles += 1;
                assert!(
                    normal.z < -0.2,
                    "expected start cap to face downward, got centroid={centroid:?}, normal={normal:?}"
                );
                continue;
            }

            if (pa.z - 1.0).abs() <= 1.0e-9
                && (pb.z - 1.0).abs() <= 1.0e-9
                && (pc.z - 1.0).abs() <= 1.0e-9
            {
                end_cap_triangles += 1;
                assert!(
                    normal.z > 0.2,
                    "expected end cap to face upward, got centroid={centroid:?}, normal={normal:?}"
                );
                continue;
            }

            let radial = DVec3::new(centroid.x, centroid.y, 0.0);
            let radius = radial.length();
            if radius <= 0.11 && normal.z.abs() <= 0.25 {
                inner_wall_triangles += 1;
                assert!(
                    normal.dot(radial.normalize_or_zero()) < -0.2,
                    "expected inner wall to face inward, got centroid={centroid:?}, normal={normal:?}"
                );
            }
        }

        assert!(start_cap_triangles > 0, "expected start cap triangles");
        assert!(end_cap_triangles > 0, "expected end cap triangles");
        assert!(inner_wall_triangles > 0, "expected inner wall triangles");
    }

    #[test]
    fn circular_profile_sweeps_follow_curved_arc_spines() {
        let spine = Polycurve3::new(vec![CurveSegment3::CircularArc(
            cc_w_types::CircularArc3::new(
                DVec3::ZERO,
                DVec3::new(1.2, 0.0, 1.2),
                DVec3::new(0.0, 0.0, 2.4),
            )
            .expect("arc"),
        )])
        .expect("spine");
        let sweep = CircularProfileSweep::new(spine, 0.18, Some(0.08)).expect("sweep");
        let request = TessellationRequest {
            quality: TessellationQuality::Fine,
            chord_tolerance: 0.01,
            normal_tolerance_radians: 4.0_f64.to_radians(),
            max_edge_length: Some(0.12),
            min_tessellated_face_area: None,
            normal_mode: NormalGenerationMode::Flat,
            path_frame_mode: PathFrameMode::ParallelTransport,
        };

        let mesh = TrivialKernel
            .tessellate_primitive_with_request(
                &GeometryPrimitive::CircularProfileSweep(sweep),
                &request,
            )
            .expect("curved circular sweep should tessellate");

        assert!(mesh.triangle_count() > 250);
        assert!(mesh.bounds.max.x > 1.15);
        assert!(mesh.bounds.size().x > 1.0);
        assert!(mesh.bounds.max.z > 2.35);
    }
}
