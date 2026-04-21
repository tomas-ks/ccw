use cc_w_types::{GeometryError, PreparedMesh, PreparedVertex, TriangleMesh};
use glam::DVec3;
use thiserror::Error;

pub trait PreparePipeline {
    fn prepare_mesh(&self, mesh: &TriangleMesh) -> Result<PreparedMesh, PrepareError>;
}

#[derive(Debug, Default)]
pub struct MeshPreparePipeline;

impl PreparePipeline for MeshPreparePipeline {
    fn prepare_mesh(&self, mesh: &TriangleMesh) -> Result<PreparedMesh, PrepareError> {
        if mesh.indices.is_empty() {
            return Err(PrepareError::from(GeometryError::EmptyMesh));
        }

        let local_origin = mesh.bounds.center();
        let mut vertices = Vec::with_capacity(mesh.indices.len() * 3);
        let mut indices = Vec::with_capacity(mesh.indices.len() * 3);

        for triangle in &mesh.indices {
            let normal = triangle_normal(&mesh.positions, *triangle);

            for &position_index in triangle {
                let position = mesh.positions[position_index as usize];
                let local = position - local_origin;
                let prepared_index = vertices.len() as u32;

                vertices.push(PreparedVertex {
                    position: [local.x as f32, local.y as f32, local.z as f32],
                    normal: [normal.x as f32, normal.y as f32, normal.z as f32],
                });
                indices.push(prepared_index);
            }
        }

        Ok(PreparedMesh {
            local_origin,
            bounds: mesh.bounds,
            vertices,
            indices,
        })
    }
}

fn triangle_normal(positions: &[DVec3], triangle: [u32; 3]) -> DVec3 {
    let [a, b, c] = triangle;
    let a = positions[a as usize];
    let b = positions[b as usize];
    let c = positions[c as usize];
    let cross = (b - a).cross(c - a);

    if cross.length_squared() <= f64::EPSILON {
        DVec3::Z
    } else {
        cross.normalize()
    }
}

#[derive(Debug, Error)]
pub enum PrepareError {
    #[error(transparent)]
    Geometry(#[from] GeometryError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use cc_w_types::ConvexPolygon;
    use glam::DVec3;

    #[test]
    fn prepare_pipeline_recenters_mesh() {
        let polygon = ConvexPolygon::new(vec![
            DVec3::new(-2.0, -1.0, 3.0),
            DVec3::new(2.0, -1.0, 3.0),
            DVec3::new(2.0, 1.0, 3.0),
            DVec3::new(-2.0, 1.0, 3.0),
        ])
        .expect("polygon");
        let mesh = TriangleMesh::new(polygon.vertices, vec![[0, 1, 2], [0, 2, 3]]).expect("mesh");

        let prepared = MeshPreparePipeline.prepare_mesh(&mesh).expect("prepared");

        assert_eq!(prepared.local_origin, DVec3::new(0.0, 0.0, 3.0));
        assert_eq!(prepared.vertex_count(), 6);
        assert_eq!(prepared.triangle_count(), 2);
        assert_eq!(prepared.indices, vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn prepare_pipeline_assigns_flat_normals_per_triangle() {
        let mesh = TriangleMesh::new(
            vec![
                DVec3::new(0.0, 0.0, 0.0),
                DVec3::new(1.0, 0.0, 0.0),
                DVec3::new(0.0, 1.0, 0.0),
                DVec3::new(0.0, 0.0, 1.0),
            ],
            vec![[0, 1, 2], [0, 1, 3]],
        )
        .expect("mesh");

        let prepared = MeshPreparePipeline.prepare_mesh(&mesh).expect("prepared");

        assert_eq!(prepared.vertices[0].normal, [0.0, 0.0, 1.0]);
        assert_eq!(prepared.vertices[1].normal, [0.0, 0.0, 1.0]);
        assert_eq!(prepared.vertices[2].normal, [0.0, 0.0, 1.0]);
        assert_eq!(prepared.vertices[3].normal, [0.0, -1.0, 0.0]);
        assert_eq!(prepared.vertices[4].normal, [0.0, -1.0, 0.0]);
        assert_eq!(prepared.vertices[5].normal, [0.0, -1.0, 0.0]);
    }
}
