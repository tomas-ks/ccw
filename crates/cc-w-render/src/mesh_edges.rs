use cc_w_types::PreparedMesh;
use glam::Vec3;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshEdgeExtractionConfig {
    pub crease_angle_radians: f32,
}

impl Default for MeshEdgeExtractionConfig {
    fn default() -> Self {
        Self {
            crease_angle_radians: 30.0_f32.to_radians(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExtractedMeshEdges {
    pub boundary_edges: Vec<[u32; 2]>,
    pub crease_edges: Vec<[u32; 2]>,
}

impl ExtractedMeshEdges {
    pub fn extract(mesh: &PreparedMesh, config: MeshEdgeExtractionConfig) -> Self {
        extract_mesh_edges(mesh, config)
    }
}

pub fn extract_mesh_edges(
    mesh: &PreparedMesh,
    config: MeshEdgeExtractionConfig,
) -> ExtractedMeshEdges {
    let canonical_vertices = canonical_vertices_by_position(mesh);
    let mut adjacency = BTreeMap::<EdgeKey, Vec<FaceUse>>::new();

    for triangle in mesh.indices.chunks_exact(3) {
        let Some(indices) = triangle_indices(triangle) else {
            continue;
        };
        let Some(normal) = triangle_normal(mesh, indices) else {
            continue;
        };
        let Some(edges) = triangle_edges(&canonical_vertices, indices) else {
            continue;
        };

        for edge in edges {
            adjacency.entry(edge).or_default().push(FaceUse { normal });
        }
    }

    let crease_angle = normalized_crease_angle(config);
    let mut boundary_edges = Vec::new();
    let mut crease_edges = Vec::new();

    for (edge, faces) in adjacency {
        match faces.as_slice() {
            [_] => boundary_edges.push(edge.as_pair()),
            [first, second] => {
                if face_angle(first.normal, second.normal) > crease_angle {
                    crease_edges.push(edge.as_pair());
                }
            }
            _ => crease_edges.push(edge.as_pair()),
        }
    }

    ExtractedMeshEdges {
        boundary_edges,
        crease_edges,
    }
}

#[derive(Clone, Copy, Debug)]
struct FaceUse {
    normal: Vec3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct EdgeKey {
    a: u32,
    b: u32,
}

impl EdgeKey {
    fn new(a: u32, b: u32) -> Option<Self> {
        if a == b {
            return None;
        }

        Some(if a < b {
            Self { a, b }
        } else {
            Self { a: b, b: a }
        })
    }

    const fn as_pair(self) -> [u32; 2] {
        [self.a, self.b]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct PositionKey([u32; 3]);

impl PositionKey {
    fn from_position(position: [f32; 3]) -> Self {
        Self(position.map(position_component_key))
    }
}

fn position_component_key(value: f32) -> u32 {
    if value == 0.0 {
        0.0_f32.to_bits()
    } else {
        value.to_bits()
    }
}

fn canonical_vertices_by_position(mesh: &PreparedMesh) -> Vec<u32> {
    let mut canonical_by_position = BTreeMap::<PositionKey, u32>::new();
    let mut canonical_vertices = Vec::with_capacity(mesh.vertices.len());

    for (index, vertex) in mesh.vertices.iter().enumerate() {
        let position = PositionKey::from_position(vertex.position);
        let canonical = *canonical_by_position
            .entry(position)
            .or_insert(index as u32);
        canonical_vertices.push(canonical);
    }

    canonical_vertices
}

fn triangle_indices(triangle: &[u32]) -> Option<[u32; 3]> {
    Some([*triangle.first()?, *triangle.get(1)?, *triangle.get(2)?])
}

fn triangle_edges(canonical_vertices: &[u32], indices: [u32; 3]) -> Option<Vec<EdgeKey>> {
    let [a, b, c] = indices.map(|index| canonical_vertices.get(index as usize).copied());
    let mut edges = Vec::with_capacity(3);

    for edge in [
        EdgeKey::new(a?, b?),
        EdgeKey::new(b?, c?),
        EdgeKey::new(c?, a?),
    ]
    .into_iter()
    .flatten()
    {
        edges.push(edge);
    }

    edges.sort_unstable();
    edges.dedup();
    Some(edges)
}

fn triangle_normal(mesh: &PreparedMesh, indices: [u32; 3]) -> Option<Vec3> {
    let [a, b, c] = indices.map(|index| {
        mesh.vertices
            .get(index as usize)
            .map(|vertex| Vec3::from_array(vertex.position))
    });
    let normal = (b? - a?).cross(c? - a?).normalize_or_zero();

    if normal.length_squared() > f32::EPSILON {
        Some(normal)
    } else {
        None
    }
}

fn face_angle(a: Vec3, b: Vec3) -> f32 {
    a.dot(b).clamp(-1.0, 1.0).acos()
}

fn normalized_crease_angle(config: MeshEdgeExtractionConfig) -> f32 {
    if config.crease_angle_radians.is_finite() {
        config.crease_angle_radians.clamp(0.0, std::f32::consts::PI)
    } else {
        MeshEdgeExtractionConfig::default().crease_angle_radians
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cc_w_types::{Bounds3, PreparedVertex};
    use glam::DVec3;

    #[test]
    fn single_triangle_extracts_three_boundary_edges() {
        let mesh = prepared_mesh(
            &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            &[0, 1, 2],
        );

        let edges = ExtractedMeshEdges::extract(&mesh, MeshEdgeExtractionConfig::default());

        assert_eq!(edges.boundary_edges, vec![[0, 1], [0, 2], [1, 2]]);
        assert!(edges.crease_edges.is_empty());
    }

    #[test]
    fn coplanar_quad_split_into_triangles_extracts_outer_boundary_only() {
        let mesh = prepared_mesh(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            &[0, 1, 2, 0, 2, 3],
        );

        let edges = ExtractedMeshEdges::extract(&mesh, MeshEdgeExtractionConfig::default());

        assert_eq!(edges.boundary_edges, vec![[0, 1], [0, 3], [1, 2], [2, 3]]);
        assert!(edges.crease_edges.is_empty());
    }

    #[test]
    fn angled_two_triangle_mesh_extracts_shared_edge_as_crease() {
        let mesh = prepared_mesh(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
            ],
            &[0, 1, 2, 1, 0, 3],
        );

        let edges = ExtractedMeshEdges::extract(&mesh, MeshEdgeExtractionConfig::default());

        assert_eq!(edges.crease_edges, vec![[0, 1]]);
    }

    #[test]
    fn non_manifold_shared_edge_extracts_shared_edge_as_crease() {
        let mesh = prepared_mesh(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
                [0.0, -1.0, 0.0],
            ],
            &[0, 1, 2, 1, 0, 3, 0, 1, 4],
        );

        let edges = ExtractedMeshEdges::extract(&mesh, MeshEdgeExtractionConfig::default());

        assert!(edges.crease_edges.contains(&[0, 1]));
    }

    fn prepared_mesh(positions: &[[f32; 3]], indices: &[u32]) -> PreparedMesh {
        PreparedMesh {
            local_origin: DVec3::ZERO,
            bounds: Bounds3::zero(),
            vertices: positions
                .iter()
                .copied()
                .map(|position| PreparedVertex {
                    position,
                    normal: [0.0, 0.0, 1.0],
                })
                .collect(),
            indices: indices.to_vec(),
        }
    }
}
