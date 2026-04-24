use bytemuck::{Pod, Zeroable};
use glam::{DMat4, DVec2, DVec3, DVec4, Vec3};
use std::collections::HashSet;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ExternalId(String);

impl ExternalId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ExternalId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SemanticElementId(String);

impl SemanticElementId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for SemanticElementId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct GeometryLayerId(String);

impl GeometryLayerId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for GeometryLayerId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct GeometryResourceId(String);

impl GeometryResourceId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for GeometryResourceId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GeometryAssetId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GeometryDefinitionId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GeometryInstanceId(pub u64);

/// Internal engine world space is right-handed with +X right, +Y forward, and +Z up.
pub const WORLD_RIGHT: DVec3 = DVec3::X;
/// Internal engine world space is right-handed with +X right, +Y forward, and +Z up.
pub const WORLD_FORWARD: DVec3 = DVec3::Y;
/// Internal engine world space is right-handed with +X right, +Y forward, and +Z up.
pub const WORLD_UP: DVec3 = DVec3::Z;

pub const WORLD_RIGHT_F32: Vec3 = Vec3::X;
pub const WORLD_FORWARD_F32: Vec3 = Vec3::Y;
pub const WORLD_UP_F32: Vec3 = Vec3::Z;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SignedAxis {
    PositiveX,
    NegativeX,
    PositiveY,
    NegativeY,
    PositiveZ,
    NegativeZ,
}

impl SignedAxis {
    pub fn as_dvec3(self) -> DVec3 {
        match self {
            Self::PositiveX => WORLD_RIGHT,
            Self::NegativeX => -WORLD_RIGHT,
            Self::PositiveY => WORLD_FORWARD,
            Self::NegativeY => -WORLD_FORWARD,
            Self::PositiveZ => WORLD_UP,
            Self::NegativeZ => -WORLD_UP,
        }
    }

    const fn axis_index(self) -> u8 {
        match self {
            Self::PositiveX | Self::NegativeX => 0,
            Self::PositiveY | Self::NegativeY => 1,
            Self::PositiveZ | Self::NegativeZ => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handedness {
    Right,
    Left,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LengthUnit {
    Meter,
    Millimeter,
    Centimeter,
    Kilometer,
    Inch,
    Foot,
}

impl LengthUnit {
    pub const fn meters_per_unit(self) -> f64 {
        match self {
            Self::Meter => 1.0,
            Self::Millimeter => 0.001,
            Self::Centimeter => 0.01,
            Self::Kilometer => 1_000.0,
            Self::Inch => 0.0254,
            Self::Foot => 0.3048,
        }
    }

    pub fn scale_to_world(self, value: f64) -> f64 {
        value * self.meters_per_unit()
    }

    pub fn point_to_world(self, point: DVec3) -> DVec3 {
        point * self.meters_per_unit()
    }

    pub fn vector_to_world(self, vector: DVec3) -> DVec3 {
        vector * self.meters_per_unit()
    }

    pub fn similarity_to_world(self) -> DMat4 {
        DMat4::from_scale(DVec3::splat(self.meters_per_unit()))
    }

    pub fn transform_to_world(self, transform: DMat4) -> DMat4 {
        let scale = self.similarity_to_world();
        scale * transform * scale.inverse()
    }
}

/// Internal engine distances are meters, so one world-space unit equals one meter.
pub const WORLD_LENGTH_UNIT: LengthUnit = LengthUnit::Meter;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CoordinateFrame {
    pub x_axis: SignedAxis,
    pub y_axis: SignedAxis,
    pub z_axis: SignedAxis,
}

impl CoordinateFrame {
    pub const fn w_world() -> Self {
        Self {
            x_axis: SignedAxis::PositiveX,
            y_axis: SignedAxis::PositiveY,
            z_axis: SignedAxis::PositiveZ,
        }
    }

    /// Common CAD/DCC-style right-handed Y-up frame where +Z points backward relative to `w`.
    pub const fn right_handed_y_up() -> Self {
        Self {
            x_axis: SignedAxis::PositiveX,
            y_axis: SignedAxis::PositiveZ,
            z_axis: SignedAxis::NegativeY,
        }
    }

    pub fn new(
        x_axis: SignedAxis,
        y_axis: SignedAxis,
        z_axis: SignedAxis,
    ) -> Result<Self, CoordinateFrameError> {
        let frame = Self {
            x_axis,
            y_axis,
            z_axis,
        };

        if frame.has_duplicate_axes() {
            return Err(CoordinateFrameError::AxisReuse {
                x_axis,
                y_axis,
                z_axis,
            });
        }

        Ok(frame)
    }

    pub fn handedness(self) -> Handedness {
        if self.basis_from_local().determinant() > 0.0 {
            Handedness::Right
        } else {
            Handedness::Left
        }
    }

    pub fn basis_from_local(self) -> DMat4 {
        DMat4::from_cols(
            self.x_axis.as_dvec3().extend(0.0),
            self.y_axis.as_dvec3().extend(0.0),
            self.z_axis.as_dvec3().extend(0.0),
            DVec4::new(0.0, 0.0, 0.0, 1.0),
        )
    }

    pub fn point_to_world(self, point: DVec3) -> DVec3 {
        self.basis_from_local().transform_point3(point)
    }

    pub fn vector_to_world(self, vector: DVec3) -> DVec3 {
        self.basis_from_local().transform_vector3(vector)
    }

    pub fn transform_to_world(self, transform: DMat4) -> DMat4 {
        let basis = self.basis_from_local();
        basis * transform * basis.inverse()
    }

    fn has_duplicate_axes(self) -> bool {
        let indices = [
            self.x_axis.axis_index(),
            self.y_axis.axis_index(),
            self.z_axis.axis_index(),
        ];

        indices[0] == indices[1] || indices[0] == indices[2] || indices[1] == indices[2]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceSpace {
    pub frame: CoordinateFrame,
    pub length_unit: LengthUnit,
}

impl SourceSpace {
    pub const fn new(frame: CoordinateFrame, length_unit: LengthUnit) -> Self {
        Self { frame, length_unit }
    }

    pub const fn w_world_metric() -> Self {
        Self::new(CoordinateFrame::w_world(), WORLD_LENGTH_UNIT)
    }

    pub const fn right_handed_y_up_meters() -> Self {
        Self::new(CoordinateFrame::right_handed_y_up(), LengthUnit::Meter)
    }

    pub const fn right_handed_y_up_millimeters() -> Self {
        Self::new(CoordinateFrame::right_handed_y_up(), LengthUnit::Millimeter)
    }

    pub fn point_to_world(self, point: DVec3) -> DVec3 {
        self.length_unit
            .point_to_world(self.frame.point_to_world(point))
    }

    pub fn vector_to_world(self, vector: DVec3) -> DVec3 {
        self.length_unit
            .vector_to_world(self.frame.vector_to_world(vector))
    }

    pub fn transform_to_world(self, transform: DMat4) -> DMat4 {
        self.length_unit
            .transform_to_world(self.frame.transform_to_world(transform))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImportMetadata {
    pub source_space: SourceSpace,
    pub normalized_to_world: bool,
}

impl ImportMetadata {
    pub const fn world_native() -> Self {
        Self {
            source_space: SourceSpace::w_world_metric(),
            normalized_to_world: true,
        }
    }

    pub const fn from_source(source_space: SourceSpace) -> Self {
        Self {
            source_space,
            normalized_to_world: false,
        }
    }

    pub const fn normalized_from(source_space: SourceSpace) -> Self {
        Self {
            source_space,
            normalized_to_world: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResidencyState {
    Unloaded,
    CpuReady,
    GpuReady,
    Resident,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds3 {
    pub min: DVec3,
    pub max: DVec3,
}

impl Bounds3 {
    pub fn from_points(points: &[DVec3]) -> Result<Self, GeometryError> {
        let (first, rest) = points.split_first().ok_or(GeometryError::EmptyPointSet)?;
        let mut min = *first;
        let mut max = *first;

        for point in rest {
            min = min.min(*point);
            max = max.max(*point);
        }

        Ok(Self { min, max })
    }

    pub const fn zero() -> Self {
        Self {
            min: DVec3::ZERO,
            max: DVec3::ZERO,
        }
    }

    pub fn center(&self) -> DVec3 {
        (self.min + self.max) * 0.5
    }

    pub fn size(&self) -> DVec3 {
        self.max - self.min
    }

    pub fn transformed(&self, transform: DMat4) -> Self {
        let corners = [
            DVec3::new(self.min.x, self.min.y, self.min.z),
            DVec3::new(self.min.x, self.min.y, self.max.z),
            DVec3::new(self.min.x, self.max.y, self.min.z),
            DVec3::new(self.min.x, self.max.y, self.max.z),
            DVec3::new(self.max.x, self.min.y, self.min.z),
            DVec3::new(self.max.x, self.min.y, self.max.z),
            DVec3::new(self.max.x, self.max.y, self.min.z),
            DVec3::new(self.max.x, self.max.y, self.max.z),
        ]
        .map(|corner| transform.transform_point3(corner));

        Self::from_points(&corners).expect("transformed bounds should still have corners")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConvexPolygon {
    pub vertices: Vec<DVec3>,
}

impl ConvexPolygon {
    pub fn new(vertices: Vec<DVec3>) -> Result<Self, GeometryError> {
        if vertices.len() < 3 {
            return Err(GeometryError::TooFewVertices);
        }

        Ok(Self { vertices })
    }

    pub fn triangle_count(&self) -> usize {
        self.vertices.len().saturating_sub(2)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TriangleMesh {
    pub positions: Vec<DVec3>,
    pub indices: Vec<[u32; 3]>,
    pub bounds: Bounds3,
}

impl TriangleMesh {
    pub fn new(positions: Vec<DVec3>, indices: Vec<[u32; 3]>) -> Result<Self, GeometryError> {
        if indices.is_empty() {
            return Err(GeometryError::EmptyMesh);
        }

        for triangle in &indices {
            for &vertex in triangle {
                if vertex as usize >= positions.len() {
                    return Err(GeometryError::InvalidMeshIndex {
                        index: *triangle,
                        vertex,
                        vertex_count: positions.len(),
                    });
                }
            }
        }

        let bounds = Bounds3::from_points(&positions)?;

        Ok(Self {
            positions,
            indices,
            bounds,
        })
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len()
    }
}

const GEOMETRY_EPSILON: f64 = 1.0e-9;

#[derive(Clone, Debug, PartialEq)]
pub struct GeometryDefinition {
    pub id: GeometryDefinitionId,
    pub primitive: GeometryPrimitive,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeometryInstance {
    pub id: GeometryInstanceId,
    pub definition_id: GeometryDefinitionId,
    pub transform: DMat4,
}

#[derive(Clone, Debug, PartialEq)]
pub enum GeometryPrimitive {
    Tessellated(TessellatedGeometry),
    SweptSolid(SweptSolid),
    CircularProfileSweep(CircularProfileSweep),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TessellationQuality {
    Draft,
    Balanced,
    Fine,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NormalGenerationMode {
    Flat,
    Smooth,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathFrameMode {
    ParallelTransport,
    Frenet,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TessellationRequest {
    pub quality: TessellationQuality,
    /// World-space tolerance in meters because `w` uses a metric internal frame.
    pub chord_tolerance: f64,
    /// Angular tolerance in radians for curve/sweep subdivision.
    pub normal_tolerance_radians: f64,
    /// Optional maximum world-space edge length in meters.
    pub max_edge_length: Option<f64>,
    /// Optional world-space cull threshold in square meters for tessellated faces.
    pub min_tessellated_face_area: Option<f64>,
    pub normal_mode: NormalGenerationMode,
    pub path_frame_mode: PathFrameMode,
}

const DEFAULT_MIN_TESSELLATED_FACE_AREA: f64 = 1.0e-12;

impl TessellationRequest {
    pub fn validated(self) -> Result<Self, GeometryError> {
        validate_positive_finite("chord_tolerance", self.chord_tolerance)?;
        validate_positive_finite("normal_tolerance_radians", self.normal_tolerance_radians)?;

        if let Some(max_edge_length) = self.max_edge_length {
            validate_positive_finite("max_edge_length", max_edge_length)?;
        }

        if let Some(min_tessellated_face_area) = self.min_tessellated_face_area {
            validate_non_negative_finite("min_tessellated_face_area", min_tessellated_face_area)?;
        }

        Ok(self)
    }
}

impl Default for TessellationRequest {
    fn default() -> Self {
        Self {
            quality: TessellationQuality::Balanced,
            chord_tolerance: 0.01,
            normal_tolerance_radians: 5.0_f64.to_radians(),
            max_edge_length: None,
            min_tessellated_face_area: Some(DEFAULT_MIN_TESSELLATED_FACE_AREA),
            normal_mode: NormalGenerationMode::Flat,
            path_frame_mode: PathFrameMode::ParallelTransport,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct IndexedPolygon {
    pub exterior: Vec<u32>,
    pub holes: Vec<Vec<u32>>,
}

impl IndexedPolygon {
    pub fn new(
        exterior: Vec<u32>,
        holes: Vec<Vec<u32>>,
        vertex_count: usize,
    ) -> Result<Self, GeometryError> {
        Self::validate_ring(&exterior, vertex_count)?;

        for hole in &holes {
            Self::validate_ring(hole, vertex_count)?;
        }

        Ok(Self { exterior, holes })
    }

    fn validate_ring(ring: &[u32], vertex_count: usize) -> Result<(), GeometryError> {
        if ring.len() < 3 {
            return Err(GeometryError::TooFewRingVertices);
        }

        for &index in ring {
            if index as usize >= vertex_count {
                return Err(GeometryError::InvalidPolygonIndex {
                    index,
                    vertex_count,
                });
            }
        }

        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TessellatedGeometry {
    pub positions: Vec<DVec3>,
    pub faces: Vec<IndexedPolygon>,
    pub bounds: Bounds3,
}

impl TessellatedGeometry {
    pub fn new(positions: Vec<DVec3>, faces: Vec<IndexedPolygon>) -> Result<Self, GeometryError> {
        if faces.is_empty() {
            return Err(GeometryError::EmptyTessellation);
        }

        for face in &faces {
            IndexedPolygon::validate_ring(&face.exterior, positions.len())?;

            for hole in &face.holes {
                IndexedPolygon::validate_ring(hole, positions.len())?;
            }
        }

        let bounds = Bounds3::from_points(&positions)?;

        Ok(Self {
            positions,
            faces,
            bounds,
        })
    }

    pub fn face_count(&self) -> usize {
        self.faces.len()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineSegment2 {
    pub start: DVec2,
    pub end: DVec2,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CircularArc2 {
    pub start: DVec2,
    pub mid: DVec2,
    pub end: DVec2,
}

impl CircularArc2 {
    pub fn new(start: DVec2, mid: DVec2, end: DVec2) -> Result<Self, GeometryError> {
        if (mid - start).perp_dot(end - start).abs() <= GEOMETRY_EPSILON {
            return Err(GeometryError::DegenerateArc);
        }

        Ok(Self { start, mid, end })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CurveSegment2 {
    Line(LineSegment2),
    CircularArc(CircularArc2),
}

impl CurveSegment2 {
    pub fn start(&self) -> DVec2 {
        match self {
            Self::Line(segment) => segment.start,
            Self::CircularArc(segment) => segment.start,
        }
    }

    pub fn end(&self) -> DVec2 {
        match self {
            Self::Line(segment) => segment.end,
            Self::CircularArc(segment) => segment.end,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Polycurve2 {
    pub segments: Vec<CurveSegment2>,
}

impl Polycurve2 {
    pub fn new(segments: Vec<CurveSegment2>) -> Result<Self, GeometryError> {
        let curve = Self { segments };
        curve.validate()?;
        Ok(curve)
    }

    pub fn start(&self) -> DVec2 {
        self.segments
            .first()
            .expect("polycurve validation guarantees a first segment")
            .start()
    }

    pub fn end(&self) -> DVec2 {
        self.segments
            .last()
            .expect("polycurve validation guarantees a last segment")
            .end()
    }

    pub fn is_closed(&self) -> bool {
        !self.segments.is_empty() && points2_match(self.start(), self.end())
    }

    fn validate(&self) -> Result<(), GeometryError> {
        if self.segments.is_empty() {
            return Err(GeometryError::EmptyPolycurve);
        }

        for pair in self.segments.windows(2) {
            if !points2_match(pair[0].end(), pair[1].start()) {
                return Err(GeometryError::DisconnectedPolycurve);
            }
        }

        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProfileLoop2 {
    pub curve: Polycurve2,
}

impl ProfileLoop2 {
    pub fn new(curve: Polycurve2) -> Result<Self, GeometryError> {
        if !curve.is_closed() {
            return Err(GeometryError::OpenProfileLoop);
        }

        Ok(Self { curve })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Profile2 {
    pub outer: ProfileLoop2,
    pub holes: Vec<ProfileLoop2>,
}

impl Profile2 {
    pub fn new(outer: ProfileLoop2, holes: Vec<ProfileLoop2>) -> Self {
        Self { outer, holes }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineSegment3 {
    pub start: DVec3,
    pub end: DVec3,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CircularArc3 {
    pub start: DVec3,
    pub mid: DVec3,
    pub end: DVec3,
}

impl CircularArc3 {
    pub fn new(start: DVec3, mid: DVec3, end: DVec3) -> Result<Self, GeometryError> {
        if (mid - start).cross(end - start).length_squared() <= GEOMETRY_EPSILON {
            return Err(GeometryError::DegenerateArc);
        }

        Ok(Self { start, mid, end })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CurveSegment3 {
    Line(LineSegment3),
    CircularArc(CircularArc3),
}

impl CurveSegment3 {
    pub fn start(&self) -> DVec3 {
        match self {
            Self::Line(segment) => segment.start,
            Self::CircularArc(segment) => segment.start,
        }
    }

    pub fn end(&self) -> DVec3 {
        match self {
            Self::Line(segment) => segment.end,
            Self::CircularArc(segment) => segment.end,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Polycurve3 {
    pub segments: Vec<CurveSegment3>,
}

impl Polycurve3 {
    pub fn new(segments: Vec<CurveSegment3>) -> Result<Self, GeometryError> {
        let curve = Self { segments };
        curve.validate()?;
        Ok(curve)
    }

    pub fn start(&self) -> DVec3 {
        self.segments
            .first()
            .expect("polycurve validation guarantees a first segment")
            .start()
    }

    pub fn end(&self) -> DVec3 {
        self.segments
            .last()
            .expect("polycurve validation guarantees a last segment")
            .end()
    }

    pub fn is_closed(&self) -> bool {
        !self.segments.is_empty() && points3_match(self.start(), self.end())
    }

    fn validate(&self) -> Result<(), GeometryError> {
        if self.segments.is_empty() {
            return Err(GeometryError::EmptyPolycurve);
        }

        for pair in self.segments.windows(2) {
            if !points3_match(pair[0].end(), pair[1].start()) {
                return Err(GeometryError::DisconnectedPolycurve);
            }
        }

        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Axis3 {
    pub origin: DVec3,
    pub direction: DVec3,
}

impl Axis3 {
    pub fn new(origin: DVec3, direction: DVec3) -> Result<Self, GeometryError> {
        if direction.length_squared() <= GEOMETRY_EPSILON {
            return Err(GeometryError::ZeroAxisDirection);
        }

        Ok(Self { origin, direction })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SweepPath {
    Linear { vector: DVec3 },
    Revolved { axis: Axis3, angle_radians: f64 },
    AlongCurve { curve: Polycurve3 },
}

impl SweepPath {
    fn validate(&self) -> Result<(), GeometryError> {
        match self {
            Self::Linear { vector } => {
                if vector.length_squared() <= GEOMETRY_EPSILON {
                    return Err(GeometryError::DegenerateSweepPath);
                }
            }
            Self::Revolved {
                axis,
                angle_radians,
            } => {
                if axis.direction.length_squared() <= GEOMETRY_EPSILON {
                    return Err(GeometryError::ZeroAxisDirection);
                }

                if angle_radians.abs() <= GEOMETRY_EPSILON {
                    return Err(GeometryError::DegenerateSweepPath);
                }
            }
            Self::AlongCurve { .. } => {}
        }

        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SweptSolid {
    pub profile: Profile2,
    pub path: SweepPath,
}

impl SweptSolid {
    pub fn new(profile: Profile2, path: SweepPath) -> Result<Self, GeometryError> {
        path.validate()?;
        Ok(Self { profile, path })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CircularProfileSweep {
    pub spine: Polycurve3,
    pub radius: f64,
    pub inner_radius: Option<f64>,
}

impl CircularProfileSweep {
    pub fn new(
        spine: Polycurve3,
        radius: f64,
        inner_radius: Option<f64>,
    ) -> Result<Self, GeometryError> {
        if !radius.is_finite() || radius <= 0.0 {
            return Err(GeometryError::InvalidSweepRadius { radius });
        }

        if let Some(inner_radius) = inner_radius {
            if !inner_radius.is_finite() || inner_radius <= 0.0 || inner_radius >= radius {
                return Err(GeometryError::InvalidInnerRadius {
                    inner_radius,
                    outer_radius: radius,
                });
            }
        }

        Ok(Self {
            spine,
            radius,
            inner_radius,
        })
    }
}

fn points2_match(a: DVec2, b: DVec2) -> bool {
    a.abs_diff_eq(b, GEOMETRY_EPSILON)
}

fn points3_match(a: DVec3, b: DVec3) -> bool {
    a.abs_diff_eq(b, GEOMETRY_EPSILON)
}

fn validate_positive_finite(name: &'static str, value: f64) -> Result<(), GeometryError> {
    if value.is_finite() && value > 0.0 {
        Ok(())
    } else {
        Err(GeometryError::InvalidTessellationParameter { name, value })
    }
}

fn validate_non_negative_finite(name: &'static str, value: f64) -> Result<(), GeometryError> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        Err(GeometryError::InvalidTessellationParameter { name, value })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplayColor {
    pub rgb: [f32; 3],
}

impl DisplayColor {
    pub const fn new(red: f32, green: f32, blue: f32) -> Self {
        Self {
            rgb: [red, green, blue],
        }
    }

    pub const fn as_rgb(self) -> [f32; 3] {
        self.rgb
    }
}

impl Default for DisplayColor {
    fn default() -> Self {
        Self::new(0.2, 0.65, 0.95)
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct PreparedVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreparedMesh {
    pub local_origin: DVec3,
    pub bounds: Bounds3,
    pub vertices: Vec<PreparedVertex>,
    pub indices: Vec<u32>,
}

impl PreparedMesh {
    pub fn vertex_count(&self) -> usize {
        self.vertices.len()
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PreparedMaterial {
    pub color: DisplayColor,
}

impl PreparedMaterial {
    pub const fn new(color: DisplayColor) -> Self {
        Self { color }
    }
}

impl Default for PreparedMaterial {
    fn default() -> Self {
        Self {
            color: DisplayColor::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreparedRenderDefinition {
    pub id: GeometryDefinitionId,
    pub mesh: PreparedMesh,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreparedRenderInstance {
    pub id: GeometryInstanceId,
    pub element_id: SemanticElementId,
    pub definition_id: GeometryDefinitionId,
    pub model_from_object: DMat4,
    pub world_bounds: Bounds3,
    pub material: PreparedMaterial,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PickRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl PickRegion {
    pub fn pixel(x: u32, y: u32) -> Self {
        Self {
            x,
            y,
            width: 1,
            height: 1,
        }
    }

    pub fn rect(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width: width.max(1),
            height: height.max(1),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickHit {
    pub instance_id: GeometryInstanceId,
    pub element_id: SemanticElementId,
    pub definition_id: GeometryDefinitionId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickResult {
    pub region: PickRegion,
    pub hits: Vec<PickHit>,
}

impl PickResult {
    pub fn empty(region: PickRegion) -> Self {
        Self {
            region,
            hits: Vec::new(),
        }
    }

    pub fn first_hit(&self) -> Option<&PickHit> {
        self.hits.first()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreparedRenderScene {
    pub bounds: Bounds3,
    pub definitions: Vec<PreparedRenderDefinition>,
    pub instances: Vec<PreparedRenderInstance>,
}

impl PreparedRenderScene {
    pub fn draw_count(&self) -> usize {
        self.instances.len()
    }

    pub fn definition_count(&self) -> usize {
        self.definitions.len()
    }

    pub fn vertex_count(&self) -> usize {
        self.instances
            .iter()
            .map(|instance| {
                self.definitions
                    .iter()
                    .find(|definition| definition.id == instance.definition_id)
                    .expect("render scene instance references an existing definition")
                    .mesh
                    .vertex_count()
            })
            .sum()
    }

    pub fn triangle_count(&self) -> usize {
        self.instances
            .iter()
            .map(|instance| {
                self.definitions
                    .iter()
                    .find(|definition| definition.id == instance.definition_id)
                    .expect("render scene instance references an existing definition")
                    .mesh
                    .triangle_count()
            })
            .sum()
    }

    pub fn is_empty(&self) -> bool {
        self.definitions.is_empty() && self.instances.is_empty()
    }
}

/// Backend-produced render package entry for one reusable geometry definition.
#[derive(Clone, Debug, PartialEq)]
pub struct PreparedGeometryDefinition {
    pub id: GeometryDefinitionId,
    pub mesh: PreparedMesh,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DefaultRenderClass {
    Physical,
    Space,
    Zone,
    Helper,
    Other,
}

impl Default for DefaultRenderClass {
    fn default() -> Self {
        Self::Physical
    }
}

/// Backend-produced semantic element metadata for higher-level viewer control.
#[derive(Clone, Debug, PartialEq)]
pub struct PreparedGeometryElement {
    pub id: SemanticElementId,
    pub label: String,
    pub declared_entity: String,
    pub default_render_class: DefaultRenderClass,
    pub bounds: Bounds3,
}

/// Backend-produced scene/package entry for one geometry instance.
#[derive(Clone, Debug, PartialEq)]
pub struct PreparedGeometryInstance {
    pub id: GeometryInstanceId,
    pub element_id: SemanticElementId,
    pub definition_id: GeometryDefinitionId,
    pub transform: DMat4,
    pub bounds: Bounds3,
    pub external_id: ExternalId,
    pub label: String,
    pub display_color: Option<DisplayColor>,
}

/// Transport-neutral boundary payload between geometry-processing backend and rendering frontend.
///
/// In native/dev flows this can stay in-process. In web production it is expected to cross a
/// service boundary so the frontend can stay a thin `wgpu` client that streams prepared assets.
#[derive(Clone, Debug, PartialEq)]
pub struct PreparedGeometryPackage {
    pub definitions: Vec<PreparedGeometryDefinition>,
    pub elements: Vec<PreparedGeometryElement>,
    pub instances: Vec<PreparedGeometryInstance>,
}

impl PreparedGeometryPackage {
    pub fn definition_count(&self) -> usize {
        self.definitions.len()
    }

    pub fn element_count(&self) -> usize {
        self.elements.len()
    }

    pub fn instance_count(&self) -> usize {
        self.instances.len()
    }

    pub fn is_empty(&self) -> bool {
        self.definitions.is_empty() && self.elements.is_empty() && self.instances.is_empty()
    }

    pub fn catalog(&self) -> GeometryCatalog {
        GeometryCatalog::from_prepared_package(self)
    }

    pub fn definition_batch(
        &self,
        request: &GeometryDefinitionBatchRequest,
    ) -> GeometryDefinitionBatch {
        GeometryDefinitionBatch {
            definitions: request
                .definition_ids
                .iter()
                .filter_map(|id| {
                    self.definitions
                        .iter()
                        .find(|definition| definition.id == *id)
                        .cloned()
                })
                .collect(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeometryDefinitionCatalogEntry {
    pub id: GeometryDefinitionId,
    pub bounds: Bounds3,
    pub vertex_count: usize,
    pub triangle_count: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeometryElementCatalogEntry {
    pub id: SemanticElementId,
    pub label: String,
    pub declared_entity: String,
    pub default_render_class: DefaultRenderClass,
    pub bounds: Bounds3,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeometryInstanceCatalogEntry {
    pub id: GeometryInstanceId,
    pub element_id: SemanticElementId,
    pub definition_id: GeometryDefinitionId,
    pub transform: DMat4,
    pub bounds: Bounds3,
    pub external_id: ExternalId,
    pub label: String,
    pub display_color: Option<DisplayColor>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeometryCatalog {
    pub definitions: Vec<GeometryDefinitionCatalogEntry>,
    pub elements: Vec<GeometryElementCatalogEntry>,
    pub instances: Vec<GeometryInstanceCatalogEntry>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeometryCatalogLayer {
    pub layer_id: GeometryLayerId,
    pub resource_id: GeometryResourceId,
    pub catalog: GeometryCatalog,
}

impl GeometryCatalog {
    pub fn from_prepared_package(package: &PreparedGeometryPackage) -> Self {
        Self {
            definitions: package
                .definitions
                .iter()
                .map(|definition| GeometryDefinitionCatalogEntry {
                    id: definition.id,
                    bounds: definition.mesh.bounds,
                    vertex_count: definition.mesh.vertex_count(),
                    triangle_count: definition.mesh.triangle_count(),
                })
                .collect(),
            elements: package
                .elements
                .iter()
                .map(|element| GeometryElementCatalogEntry {
                    id: element.id.clone(),
                    label: element.label.clone(),
                    declared_entity: element.declared_entity.clone(),
                    default_render_class: element.default_render_class,
                    bounds: element.bounds,
                })
                .collect(),
            instances: package
                .instances
                .iter()
                .map(|instance| GeometryInstanceCatalogEntry {
                    id: instance.id,
                    element_id: instance.element_id.clone(),
                    definition_id: instance.definition_id,
                    transform: instance.transform,
                    bounds: instance.bounds,
                    external_id: instance.external_id.clone(),
                    label: instance.label.clone(),
                    display_color: instance.display_color,
                })
                .collect(),
        }
    }

    pub fn instance_batch(&self, request: &GeometryInstanceBatchRequest) -> GeometryInstanceBatch {
        GeometryInstanceBatch {
            instances: request
                .instance_ids
                .iter()
                .filter_map(|id| {
                    self.instances
                        .iter()
                        .find(|instance| instance.id == *id)
                        .cloned()
                })
                .collect(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GeometryInstanceBatchRequest {
    pub instance_ids: Vec<GeometryInstanceId>,
}

impl GeometryInstanceBatchRequest {
    pub fn new(instance_ids: Vec<GeometryInstanceId>) -> Self {
        Self { instance_ids }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GeometryInstanceBatch {
    pub instances: Vec<GeometryInstanceCatalogEntry>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GeometryDefinitionBatchRequest {
    pub definition_ids: Vec<GeometryDefinitionId>,
}

impl GeometryDefinitionBatchRequest {
    pub fn new(definition_ids: Vec<GeometryDefinitionId>) -> Self {
        Self { definition_ids }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GeometryDefinitionBatch {
    pub definitions: Vec<PreparedGeometryDefinition>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GeometryResidencyState {
    Missing,
    Requested,
    Ready,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GeometryStreamingBudget {
    pub max_instances: usize,
    pub max_definitions: usize,
}

impl GeometryStreamingBudget {
    pub const fn unlimited() -> Self {
        Self {
            max_instances: usize::MAX,
            max_definitions: usize::MAX,
        }
    }

    pub const fn new(max_instances: usize, max_definitions: usize) -> Self {
        Self {
            max_instances,
            max_definitions,
        }
    }
}

impl Default for GeometryStreamingBudget {
    fn default() -> Self {
        Self::unlimited()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GeometryStreamPlan {
    pub instance_ids: Vec<GeometryInstanceId>,
    pub definition_ids: Vec<GeometryDefinitionId>,
}

impl GeometryStreamPlan {
    pub fn from_visible_element_ids(
        catalog: &GeometryCatalog,
        visible_element_ids: &[SemanticElementId],
    ) -> Self {
        let mut instance_ids = Vec::new();
        let mut definition_ids = Vec::new();
        let mut seen_instances = HashSet::new();
        let mut seen_definitions = HashSet::new();

        for element_id in visible_element_ids {
            for instance in catalog
                .instances
                .iter()
                .filter(|instance| instance.element_id == *element_id)
            {
                if seen_instances.insert(instance.id) {
                    instance_ids.push(instance.id);
                }

                if seen_definitions.insert(instance.definition_id) {
                    definition_ids.push(instance.definition_id);
                }
            }
        }

        Self {
            instance_ids,
            definition_ids,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeometryStreamPlanReason {
    Selected,
    InView,
    VisibleElement,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeometryPrioritizedStreamEntry {
    pub instance_id: GeometryInstanceId,
    pub element_id: SemanticElementId,
    pub definition_id: GeometryDefinitionId,
    pub reason: GeometryStreamPlanReason,
    pub priority_score: f64,
    pub projected_screen_area: f64,
    pub distance_to_camera: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GeometryPrioritizedStreamPlan {
    pub entries: Vec<GeometryPrioritizedStreamEntry>,
    pub instance_ids: Vec<GeometryInstanceId>,
    pub definition_ids: Vec<GeometryDefinitionId>,
}

impl GeometryPrioritizedStreamPlan {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.instance_ids.is_empty() && self.definition_ids.is_empty()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum GeometryStartViewRequest {
    #[default]
    Default,
    Minimal(usize),
    All,
    Elements(Vec<SemanticElementId>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedGeometryStartView {
    pub visible_element_ids: Vec<SemanticElementId>,
}

#[derive(Clone, Debug)]
pub struct MeshDocument {
    pub external_id: ExternalId,
    pub label: String,
    pub polygon: ConvexPolygon,
    pub transform: DMat4,
    pub import: ImportMetadata,
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum GeometryError {
    #[error("bounds require at least one point")]
    EmptyPointSet,
    #[error("a polygon requires at least three vertices")]
    TooFewVertices,
    #[error("polygon rings require at least three vertices")]
    TooFewRingVertices,
    #[error("a mesh requires at least one triangle")]
    EmptyMesh,
    #[error("tessellated geometry requires at least one face")]
    EmptyTessellation,
    #[error("polycurves require at least one segment")]
    EmptyPolycurve,
    #[error("polycurve segments must connect end-to-start")]
    DisconnectedPolycurve,
    #[error("profile loops must be closed")]
    OpenProfileLoop,
    #[error("circular arcs require three non-collinear points")]
    DegenerateArc,
    #[error("sweep axes require a non-zero direction")]
    ZeroAxisDirection,
    #[error("sweep paths must span a non-zero distance or angle")]
    DegenerateSweepPath,
    #[error("polygon index {index} is out of range for {vertex_count} vertices")]
    InvalidPolygonIndex { index: u32, vertex_count: usize },
    #[error(
        "mesh index {index:?} references vertex {vertex}, but only {vertex_count} vertices exist"
    )]
    InvalidMeshIndex {
        index: [u32; 3],
        vertex: u32,
        vertex_count: usize,
    },
    #[error("circular sweep radius must be positive and finite, got {radius}")]
    InvalidSweepRadius { radius: f64 },
    #[error(
        "circular sweep inner radius must be positive, finite, and less than the outer radius ({outer_radius}), got {inner_radius}"
    )]
    InvalidInnerRadius {
        inner_radius: f64,
        outer_radius: f64,
    },
    #[error("tessellation parameter `{name}` must be positive and finite, got {value}")]
    InvalidTessellationParameter { name: &'static str, value: f64 },
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CoordinateFrameError {
    #[error(
        "coordinate frame axes must refer to three distinct dimensions, got x={x_axis:?}, y={y_axis:?}, z={z_axis:?}"
    )]
    AxisReuse {
        x_axis: SignedAxis,
        y_axis: SignedAxis,
        z_axis: SignedAxis,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_profile() -> Profile2 {
        let outer = ProfileLoop2::new(
            Polycurve2::new(vec![
                CurveSegment2::Line(LineSegment2 {
                    start: DVec2::ZERO,
                    end: DVec2::X,
                }),
                CurveSegment2::Line(LineSegment2 {
                    start: DVec2::X,
                    end: DVec2::new(1.0, 1.0),
                }),
                CurveSegment2::Line(LineSegment2 {
                    start: DVec2::new(1.0, 1.0),
                    end: DVec2::ZERO,
                }),
            ])
            .expect("curve"),
        )
        .expect("loop");

        Profile2::new(outer, vec![])
    }

    fn sample_prepared_mesh() -> PreparedMesh {
        PreparedMesh {
            local_origin: DVec3::ZERO,
            bounds: Bounds3::from_points(&[DVec3::ZERO, DVec3::new(1.0, 1.0, 1.0)])
                .expect("bounds"),
            vertices: vec![],
            indices: vec![],
        }
    }

    fn sample_stream_package() -> PreparedGeometryPackage {
        PreparedGeometryPackage {
            definitions: vec![
                PreparedGeometryDefinition {
                    id: GeometryDefinitionId(10),
                    mesh: sample_prepared_mesh(),
                },
                PreparedGeometryDefinition {
                    id: GeometryDefinitionId(20),
                    mesh: sample_prepared_mesh(),
                },
            ],
            elements: vec![
                PreparedGeometryElement {
                    id: SemanticElementId::new("element-a"),
                    label: "Element A".to_string(),
                    declared_entity: "IfcWall".to_string(),
                    default_render_class: DefaultRenderClass::Physical,
                    bounds: Bounds3::zero(),
                },
                PreparedGeometryElement {
                    id: SemanticElementId::new("element-b"),
                    label: "Element B".to_string(),
                    declared_entity: "IfcSlab".to_string(),
                    default_render_class: DefaultRenderClass::Physical,
                    bounds: Bounds3::zero(),
                },
            ],
            instances: vec![
                PreparedGeometryInstance {
                    id: GeometryInstanceId(1),
                    element_id: SemanticElementId::new("element-a"),
                    definition_id: GeometryDefinitionId(10),
                    transform: DMat4::IDENTITY,
                    bounds: Bounds3::zero(),
                    external_id: ExternalId::new("instance-1"),
                    label: "Instance 1".to_string(),
                    display_color: None,
                },
                PreparedGeometryInstance {
                    id: GeometryInstanceId(2),
                    element_id: SemanticElementId::new("element-b"),
                    definition_id: GeometryDefinitionId(20),
                    transform: DMat4::IDENTITY,
                    bounds: Bounds3::zero(),
                    external_id: ExternalId::new("instance-2"),
                    label: "Instance 2".to_string(),
                    display_color: None,
                },
                PreparedGeometryInstance {
                    id: GeometryInstanceId(3),
                    element_id: SemanticElementId::new("element-b"),
                    definition_id: GeometryDefinitionId(10),
                    transform: DMat4::IDENTITY,
                    bounds: Bounds3::zero(),
                    external_id: ExternalId::new("instance-3"),
                    label: "Instance 3".to_string(),
                    display_color: None,
                },
            ],
        }
    }

    #[test]
    fn geometry_layer_id_preserves_string_value() {
        let id = GeometryLayerId::new("primary-layer");
        let from_str = GeometryLayerId::from("overlay-layer");

        assert_eq!(id.as_str(), "primary-layer");
        assert_eq!(from_str.as_str(), "overlay-layer");
    }

    #[test]
    fn geometry_resource_id_preserves_string_value() {
        let id = GeometryResourceId::new("ifc-resource");
        let from_str = GeometryResourceId::from("native-resource");

        assert_eq!(id.as_str(), "ifc-resource");
        assert_eq!(from_str.as_str(), "native-resource");
    }

    #[test]
    fn geometry_catalog_layer_keeps_resource_and_layer_identity() {
        let layer = GeometryCatalogLayer {
            layer_id: GeometryLayerId::from("structural"),
            resource_id: GeometryResourceId::from("sample.ifc"),
            catalog: sample_stream_package().catalog(),
        };

        assert_eq!(layer.layer_id.as_str(), "structural");
        assert_eq!(layer.resource_id.as_str(), "sample.ifc");
        assert_eq!(layer.catalog.definitions.len(), 2);
        assert_eq!(layer.catalog.elements[0].id.as_str(), "element-a");
        assert_eq!(layer.catalog.instances[2].id, GeometryInstanceId(3));
    }

    #[test]
    fn bounds_are_derived_from_points() {
        let bounds = Bounds3::from_points(&[DVec3::new(-2.0, 1.0, 0.0), DVec3::new(3.0, 5.0, 4.0)])
            .expect("bounds");

        assert_eq!(bounds.center(), DVec3::new(0.5, 3.0, 2.0));
        assert_eq!(bounds.size(), DVec3::new(5.0, 4.0, 4.0));
    }

    #[test]
    fn bounds_can_be_transformed_into_instance_space() {
        let bounds =
            Bounds3::from_points(&[DVec3::new(-1.0, -2.0, 0.5), DVec3::new(2.0, 3.0, 1.5)])
                .expect("bounds");
        let transformed = bounds.transformed(DMat4::from_translation(DVec3::new(5.0, -1.0, 2.0)));

        assert_eq!(transformed.min, DVec3::new(4.0, -3.0, 2.5));
        assert_eq!(transformed.max, DVec3::new(7.0, 2.0, 3.5));
    }

    #[test]
    fn polygon_requires_three_vertices() {
        let result = ConvexPolygon::new(vec![DVec3::ZERO, DVec3::X]);

        assert!(matches!(result, Err(GeometryError::TooFewVertices)));
    }

    #[test]
    fn world_basis_is_right_handed_and_z_up() {
        assert_eq!(WORLD_RIGHT.cross(WORLD_FORWARD), WORLD_UP);
        assert_eq!(WORLD_UP, DVec3::Z);
    }

    #[test]
    fn coordinate_frame_converts_right_handed_y_up_points_into_world() {
        let frame = CoordinateFrame::right_handed_y_up();
        let point = frame.point_to_world(DVec3::new(2.0, 3.0, 4.0));

        assert_eq!(frame.handedness(), Handedness::Right);
        assert_eq!(point, DVec3::new(2.0, -4.0, 3.0));
    }

    #[test]
    fn coordinate_frame_rejects_duplicate_axes() {
        let result = CoordinateFrame::new(
            SignedAxis::PositiveX,
            SignedAxis::NegativeX,
            SignedAxis::PositiveZ,
        );

        assert!(matches!(
            result,
            Err(CoordinateFrameError::AxisReuse { .. })
        ));
    }

    #[test]
    fn coordinate_frame_conjugates_transforms_into_world_basis() {
        let frame = CoordinateFrame::right_handed_y_up();
        let transform = DMat4::from_translation(DVec3::new(3.0, 2.0, -4.0));
        let world_transform = frame.transform_to_world(transform);

        assert_eq!(
            world_transform.transform_point3(DVec3::ZERO),
            DVec3::new(3.0, 4.0, 2.0)
        );
    }

    #[test]
    fn millimeters_are_scaled_into_world_meters() {
        assert_eq!(WORLD_LENGTH_UNIT, LengthUnit::Meter);
        assert_eq!(
            LengthUnit::Millimeter.point_to_world(DVec3::new(1_800.0, -1_200.0, 0.0)),
            DVec3::new(1.8, -1.2, 0.0)
        );
        assert_eq!(LengthUnit::Foot.scale_to_world(10.0), 3.048);
    }

    #[test]
    fn source_space_combines_axis_and_unit_conversion() {
        let source_space = SourceSpace::right_handed_y_up_millimeters();

        assert_eq!(
            source_space.point_to_world(DVec3::new(1_800.0, 0.0, 1_200.0)),
            DVec3::new(1.8, -1.2, 0.0)
        );
        assert_eq!(
            source_space.vector_to_world(DVec3::new(0.0, 1_000.0, 0.0)),
            DVec3::new(0.0, 0.0, 1.0)
        );
    }

    #[test]
    fn unit_similarity_scales_translation_into_meters() {
        let transform = DMat4::from_translation(DVec3::new(1_500.0, 0.0, 0.0));
        let world_transform = LengthUnit::Millimeter.transform_to_world(transform);

        assert!(
            world_transform
                .transform_point3(DVec3::ZERO)
                .abs_diff_eq(DVec3::new(1.5, 0.0, 0.0), 1.0e-12)
        );
    }

    #[test]
    fn import_metadata_can_track_original_source_space() {
        let import = ImportMetadata::normalized_from(SourceSpace::right_handed_y_up_meters());

        assert!(import.normalized_to_world);
        assert_eq!(
            import.source_space.frame,
            CoordinateFrame::right_handed_y_up()
        );
        assert_eq!(import.source_space.length_unit, LengthUnit::Meter);
    }

    #[test]
    fn indexed_polygons_reject_out_of_range_indices() {
        let result = IndexedPolygon::new(vec![0, 1, 4], vec![], 4);

        assert!(matches!(
            result,
            Err(GeometryError::InvalidPolygonIndex {
                index: 4,
                vertex_count: 4,
            })
        ));
    }

    #[test]
    fn tessellated_geometry_tracks_polygon_faces_and_bounds() {
        let positions = vec![
            DVec3::new(0.0, 0.0, 0.0),
            DVec3::new(4.0, 0.0, 0.0),
            DVec3::new(4.0, 4.0, 0.0),
            DVec3::new(0.0, 4.0, 0.0),
            DVec3::new(1.0, 1.0, 0.0),
            DVec3::new(3.0, 1.0, 0.0),
            DVec3::new(3.0, 3.0, 0.0),
            DVec3::new(1.0, 3.0, 0.0),
        ];
        let face = IndexedPolygon::new(vec![0, 1, 2, 3], vec![vec![4, 5, 6, 7]], positions.len())
            .expect("face");
        let tessellation = TessellatedGeometry::new(positions, vec![face]).expect("tessellation");

        assert_eq!(tessellation.face_count(), 1);
        assert_eq!(tessellation.bounds.size(), DVec3::new(4.0, 4.0, 0.0));
    }

    #[test]
    fn tessellation_request_defaults_are_valid() {
        let request = TessellationRequest::default().validated().expect("request");

        assert_eq!(request.quality, TessellationQuality::Balanced);
        assert_eq!(request.min_tessellated_face_area, Some(1.0e-12));
        assert_eq!(request.normal_mode, NormalGenerationMode::Flat);
        assert_eq!(request.path_frame_mode, PathFrameMode::ParallelTransport);
    }

    #[test]
    fn tessellation_request_rejects_non_positive_values() {
        let result = TessellationRequest {
            chord_tolerance: 0.0,
            ..TessellationRequest::default()
        }
        .validated();

        assert!(matches!(
            result,
            Err(GeometryError::InvalidTessellationParameter {
                name: "chord_tolerance",
                value: 0.0,
            })
        ));
    }

    #[test]
    fn profile_loops_can_preserve_arc_segments() {
        let outer = ProfileLoop2::new(
            Polycurve2::new(vec![
                CurveSegment2::Line(LineSegment2 {
                    start: DVec2::ZERO,
                    end: DVec2::new(2.0, 0.0),
                }),
                CurveSegment2::CircularArc(
                    CircularArc2::new(DVec2::new(2.0, 0.0), DVec2::new(1.0, 1.0), DVec2::ZERO)
                        .expect("arc"),
                ),
            ])
            .expect("curve"),
        )
        .expect("loop");
        let profile = Profile2::new(outer, vec![]);

        assert!(matches!(
            profile.outer.curve.segments[1],
            CurveSegment2::CircularArc(_)
        ));
    }

    #[test]
    fn profile_loops_require_closed_curves() {
        let open_curve = Polycurve2::new(vec![
            CurveSegment2::Line(LineSegment2 {
                start: DVec2::ZERO,
                end: DVec2::X,
            }),
            CurveSegment2::Line(LineSegment2 {
                start: DVec2::X,
                end: DVec2::new(2.0, 1.0),
            }),
        ])
        .expect("curve");

        let result = ProfileLoop2::new(open_curve);

        assert!(matches!(result, Err(GeometryError::OpenProfileLoop)));
    }

    #[test]
    fn swept_solids_reject_degenerate_paths() {
        let result = SweptSolid::new(
            sample_profile(),
            SweepPath::Linear {
                vector: DVec3::ZERO,
            },
        );

        assert!(matches!(result, Err(GeometryError::DegenerateSweepPath)));
    }

    #[test]
    fn circular_profile_sweeps_validate_inner_radius() {
        let spine = Polycurve3::new(vec![
            CurveSegment3::Line(LineSegment3 {
                start: DVec3::ZERO,
                end: DVec3::new(0.0, 0.0, 1.0),
            }),
            CurveSegment3::CircularArc(
                CircularArc3::new(
                    DVec3::new(0.0, 0.0, 1.0),
                    DVec3::new(1.0, 0.0, 2.0),
                    DVec3::new(0.0, 0.0, 3.0),
                )
                .expect("arc"),
            ),
        ])
        .expect("spine");
        let result = CircularProfileSweep::new(spine, 0.25, Some(0.25));

        assert!(matches!(
            result,
            Err(GeometryError::InvalidInnerRadius {
                inner_radius: 0.25,
                outer_radius: 0.25,
            })
        ));
    }

    #[test]
    fn prepared_geometry_package_tracks_definition_and_instance_counts() {
        let mesh = PreparedMesh {
            local_origin: DVec3::ZERO,
            bounds: Bounds3::from_points(&[DVec3::new(-1.0, -1.0, 0.0), DVec3::new(1.0, 1.0, 0.0)])
                .expect("bounds"),
            vertices: vec![],
            indices: vec![],
        };
        let package = PreparedGeometryPackage {
            definitions: vec![PreparedGeometryDefinition {
                id: GeometryDefinitionId(1),
                mesh,
            }],
            elements: vec![PreparedGeometryElement {
                id: SemanticElementId::new("demo/element"),
                label: "Demo".to_string(),
                declared_entity: "DemoGeometry".to_string(),
                default_render_class: DefaultRenderClass::Physical,
                bounds: Bounds3::zero(),
            }],
            instances: vec![PreparedGeometryInstance {
                id: GeometryInstanceId(1),
                element_id: SemanticElementId::new("demo/element"),
                definition_id: GeometryDefinitionId(1),
                transform: DMat4::IDENTITY,
                bounds: Bounds3::zero(),
                external_id: ExternalId::new("demo/instance"),
                label: "Demo".to_string(),
                display_color: None,
            }],
        };

        assert_eq!(package.definition_count(), 1);
        assert_eq!(package.element_count(), 1);
        assert_eq!(package.instance_count(), 1);
        assert!(!package.is_empty());

        let catalog = package.catalog();
        assert_eq!(catalog.definitions.len(), 1);
        assert_eq!(catalog.definitions[0].vertex_count, 0);
        assert_eq!(catalog.definitions[0].triangle_count, 0);
        assert_eq!(
            catalog.elements[0].id,
            SemanticElementId::new("demo/element")
        );
        assert_eq!(catalog.instances[0].definition_id, GeometryDefinitionId(1));
    }

    #[test]
    fn instance_batch_requests_preserve_order_and_skip_unknown_ids() {
        let catalog = sample_stream_package().catalog();
        let request = GeometryInstanceBatchRequest::new(vec![
            GeometryInstanceId(3),
            GeometryInstanceId(99),
            GeometryInstanceId(1),
            GeometryInstanceId(2),
        ]);

        let batch = catalog.instance_batch(&request);
        let ids = batch
            .instances
            .iter()
            .map(|instance| instance.id)
            .collect::<Vec<_>>();

        assert_eq!(
            ids,
            vec![
                GeometryInstanceId(3),
                GeometryInstanceId(1),
                GeometryInstanceId(2),
            ]
        );
    }

    #[test]
    fn definition_batch_requests_preserve_order_and_skip_unknown_ids() {
        let package = sample_stream_package();
        let request = GeometryDefinitionBatchRequest::new(vec![
            GeometryDefinitionId(20),
            GeometryDefinitionId(99),
            GeometryDefinitionId(10),
        ]);

        let batch = package.definition_batch(&request);
        let ids = batch
            .definitions
            .iter()
            .map(|definition| definition.id)
            .collect::<Vec<_>>();

        assert_eq!(
            ids,
            vec![GeometryDefinitionId(20), GeometryDefinitionId(10)]
        );
    }

    #[test]
    fn stream_plan_selects_visible_instances_and_unique_definitions() {
        let catalog = sample_stream_package().catalog();
        let plan = GeometryStreamPlan::from_visible_element_ids(
            &catalog,
            &[
                SemanticElementId::new("element-b"),
                SemanticElementId::new("missing-element"),
                SemanticElementId::new("element-a"),
            ],
        );

        assert_eq!(
            plan.instance_ids,
            vec![
                GeometryInstanceId(2),
                GeometryInstanceId(3),
                GeometryInstanceId(1),
            ]
        );
        assert_eq!(
            plan.definition_ids,
            vec![GeometryDefinitionId(20), GeometryDefinitionId(10)]
        );
    }
}
