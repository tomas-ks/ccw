pub use cc_w_db::{DEFAULT_DEMO_RESOURCE, ResourceError, available_demo_resources};

use cc_w_db::{
    GeometrySceneResource, ImportedGeometrySceneResource, InMemoryGraphRepository, SceneRepository,
    import_geometry_scene_resource,
};
use cc_w_kernel::{GeometryKernel, KernelError, TrivialKernel};
use cc_w_prepare::{MeshPreparePipeline, PrepareError, PreparePipeline};
use cc_w_types::{
    GeometryDefinitionId, PreparedGeometryDefinition, PreparedGeometryElement,
    PreparedGeometryInstance, PreparedGeometryPackage, SemanticElementId,
};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

#[derive(Debug)]
pub struct GeometryBackend<R = InMemoryGraphRepository, K = TrivialKernel, P = MeshPreparePipeline>
{
    repository: R,
    kernel: K,
    prepare: P,
}

impl Default for GeometryBackend {
    fn default() -> Self {
        Self {
            repository: InMemoryGraphRepository,
            kernel: TrivialKernel,
            prepare: MeshPreparePipeline,
        }
    }
}

impl<R, K, P> GeometryBackend<R, K, P>
where
    R: SceneRepository,
    K: GeometryKernel,
    P: PreparePipeline,
{
    pub fn new(repository: R, kernel: K, prepare: P) -> Self {
        Self {
            repository,
            kernel,
            prepare,
        }
    }

    pub fn build_demo_package_for(
        &self,
        resource: &str,
    ) -> Result<PreparedGeometryPackage, GeometryBackendError> {
        let geometry = self.repository.load_demo_geometry_scene(resource)?;
        self.build_package_from_scene(geometry)
    }

    pub fn build_demo_package(&self) -> Result<PreparedGeometryPackage, GeometryBackendError> {
        self.build_demo_package_for(DEFAULT_DEMO_RESOURCE)
    }

    pub fn build_imported_scene_package(
        &self,
        geometry: ImportedGeometrySceneResource,
    ) -> Result<PreparedGeometryPackage, GeometryBackendError> {
        self.build_package_from_scene(import_geometry_scene_resource(geometry))
    }

    pub fn build_package_from_scene(
        &self,
        geometry: GeometrySceneResource,
    ) -> Result<PreparedGeometryPackage, GeometryBackendError> {
        if geometry.definitions.is_empty() {
            return Err(GeometryBackendError::EmptyGeometryDefinitions);
        }
        if geometry.instances.is_empty() {
            return Err(GeometryBackendError::EmptyGeometryResource);
        }

        let mut definition_bounds = HashMap::<GeometryDefinitionId, _>::new();
        let mut culled_definitions = HashSet::<GeometryDefinitionId>::new();
        let mut prepared_definitions = Vec::with_capacity(geometry.definitions.len());
        for definition in geometry.definitions {
            if definition_bounds.contains_key(&definition.id)
                || culled_definitions.contains(&definition.id)
            {
                return Err(GeometryBackendError::DuplicateDefinitionId(definition.id));
            }

            let mesh = match self.kernel.tessellate_primitive(&definition.primitive) {
                Ok(mesh) => mesh,
                Err(KernelError::CulledPrimitive { .. }) => {
                    culled_definitions.insert(definition.id);
                    continue;
                }
                Err(error) => return Err(GeometryBackendError::from(error)),
            };
            let prepared_mesh = self
                .prepare
                .prepare_mesh(&mesh)
                .map_err(GeometryBackendError::from)?;
            definition_bounds.insert(definition.id, prepared_mesh.bounds);
            prepared_definitions.push(PreparedGeometryDefinition {
                id: definition.id,
                mesh: prepared_mesh,
            });
        }

        if prepared_definitions.is_empty() {
            return Err(GeometryBackendError::EmptyGeometryDefinitions);
        }

        let mut instances = Vec::with_capacity(geometry.instances.len());
        let mut element_indices = HashMap::<SemanticElementId, usize>::new();
        let mut elements = Vec::<PreparedGeometryElement>::new();

        for instance in &geometry.instances {
            let definition_id = instance.instance.definition_id;
            if culled_definitions.contains(&definition_id) {
                continue;
            }
            let instance_bounds = definition_bounds.get(&definition_id).copied().ok_or(
                GeometryBackendError::MissingDefinitionReference(definition_id),
            )?;
            let world_bounds = instance_bounds.transformed(instance.instance.transform);

            instances.push(PreparedGeometryInstance {
                id: instance.instance.id,
                element_id: instance.element_id.clone(),
                definition_id,
                transform: instance.instance.transform,
                bounds: world_bounds,
                external_id: instance.external_id.clone(),
                label: instance.label.clone(),
                display_color: instance.display_color,
            });

            if let Some(&element_index) = element_indices.get(&instance.element_id) {
                let element = &mut elements[element_index];
                if element.label != instance.label
                    || element.declared_entity != instance.declared_entity
                    || element.default_render_class != instance.default_render_class
                {
                    return Err(GeometryBackendError::InconsistentElementMetadata(
                        instance.element_id.clone(),
                    ));
                }
                element.bounds = cc_w_types::Bounds3 {
                    min: element.bounds.min.min(world_bounds.min),
                    max: element.bounds.max.max(world_bounds.max),
                };
            } else {
                element_indices.insert(instance.element_id.clone(), elements.len());
                elements.push(PreparedGeometryElement {
                    id: instance.element_id.clone(),
                    label: instance.label.clone(),
                    declared_entity: instance.declared_entity.clone(),
                    default_render_class: instance.default_render_class,
                    bounds: world_bounds,
                });
            }
        }

        if instances.is_empty() {
            return Err(GeometryBackendError::EmptyGeometryResource);
        }

        Ok(PreparedGeometryPackage {
            definitions: prepared_definitions,
            elements,
            instances,
        })
    }
}

#[derive(Debug, Error)]
pub enum GeometryBackendError {
    #[error(transparent)]
    Resource(#[from] ResourceError),
    #[error(transparent)]
    Kernel(#[from] KernelError),
    #[error(transparent)]
    Prepare(#[from] PrepareError),
    #[error("geometry scene resources must contain at least one definition")]
    EmptyGeometryDefinitions,
    #[error("geometry resources must contain at least one instance")]
    EmptyGeometryResource,
    #[error("geometry resources produced inconsistent metadata for semantic element {0:?}")]
    InconsistentElementMetadata(SemanticElementId),
    #[error("geometry definition {0:?} appears more than once in the scene resource")]
    DuplicateDefinitionId(cc_w_types::GeometryDefinitionId),
    #[error("geometry instance references missing definition {0:?}")]
    MissingDefinitionReference(cc_w_types::GeometryDefinitionId),
}

#[cfg(test)]
mod tests {
    use super::*;
    use cc_w_db::{GeometryResourceInstance, InMemoryGraphRepository};
    use cc_w_types::{
        DefaultRenderClass, ExternalId, GeometryDefinition, GeometryDefinitionId, GeometryInstance,
        GeometryInstanceId, GeometryPrimitive, IndexedPolygon, SemanticElementId,
        TessellatedGeometry,
    };
    use glam::{DMat4, DVec3};

    #[test]
    fn demo_package_flows_from_primitive_to_prepared_package() {
        let backend = GeometryBackend::default();
        let package = backend.build_demo_package().expect("package");

        assert_eq!(package.definition_count(), 1);
        assert_eq!(package.instance_count(), 1);
        assert_eq!(package.definitions[0].mesh.vertex_count(), 9);
        assert_eq!(package.definitions[0].mesh.triangle_count(), 3);
        assert_eq!(package.instances[0].label, "Demo Pentagon");
        assert_eq!(package.instances[0].external_id.as_str(), "demo/pentagon");
    }

    #[test]
    fn mapped_demo_package_preserves_repeated_instances() {
        let backend = GeometryBackend::default();
        let package = backend
            .build_demo_package_for("demo/mapped-pentagon-pair")
            .expect("package");

        assert_eq!(package.definition_count(), 1);
        assert_eq!(package.instance_count(), 2);
        assert_eq!(
            package.instances[0].definition_id,
            package.instances[1].definition_id
        );
        assert_ne!(
            package.instances[0].transform,
            package.instances[1].transform
        );
        assert_ne!(package.instances[0].bounds, package.instances[1].bounds);
        assert_eq!(
            package.instances[0].external_id.as_str(),
            "demo/mapped-pentagon-pair/left"
        );
        assert_eq!(
            package.instances[1].external_id.as_str(),
            "demo/mapped-pentagon-pair/right"
        );
    }

    #[test]
    fn named_demo_resource_is_supported() {
        let backend =
            GeometryBackend::new(InMemoryGraphRepository, TrivialKernel, MeshPreparePipeline);
        let package = backend
            .build_demo_package_for("demo/tilted-quad")
            .expect("package");

        assert_eq!(package.instances[0].label, "Demo Tilted Quad");
        assert_eq!(package.definitions[0].mesh.triangle_count(), 2);
    }

    #[test]
    fn scene_package_supports_multiple_definitions() {
        let backend = GeometryBackend::default();
        let scene = GeometrySceneResource {
            definitions: vec![
                triangle_definition(GeometryDefinitionId(11), 1.0),
                triangle_definition(GeometryDefinitionId(12), 2.0),
            ],
            instances: vec![
                GeometryResourceInstance {
                    instance: GeometryInstance {
                        id: GeometryInstanceId(1),
                        definition_id: GeometryDefinitionId(11),
                        transform: DMat4::IDENTITY,
                    },
                    element_id: SemanticElementId::new("scene/a"),
                    external_id: ExternalId::new("scene/a"),
                    label: "Scene A".to_string(),
                    declared_entity: "SceneGeometry".to_string(),
                    default_render_class: DefaultRenderClass::Physical,
                    display_color: None,
                },
                GeometryResourceInstance {
                    instance: GeometryInstance {
                        id: GeometryInstanceId(2),
                        definition_id: GeometryDefinitionId(12),
                        transform: DMat4::from_translation(DVec3::new(3.0, 0.0, 0.0)),
                    },
                    element_id: SemanticElementId::new("scene/b"),
                    external_id: ExternalId::new("scene/b"),
                    label: "Scene B".to_string(),
                    declared_entity: "SceneGeometry".to_string(),
                    default_render_class: DefaultRenderClass::Physical,
                    display_color: None,
                },
            ],
        };

        let package = backend.build_package_from_scene(scene).expect("package");

        assert_eq!(package.definition_count(), 2);
        assert_eq!(package.element_count(), 2);
        assert_eq!(package.instance_count(), 2);
        assert_eq!(package.instances[0].definition_id, GeometryDefinitionId(11));
        assert_eq!(package.instances[1].definition_id, GeometryDefinitionId(12));
        assert_ne!(package.instances[0].bounds, package.instances[1].bounds);
    }

    #[test]
    fn scene_package_rejects_missing_definition_reference() {
        let backend = GeometryBackend::default();
        let scene = GeometrySceneResource {
            definitions: vec![triangle_definition(GeometryDefinitionId(11), 1.0)],
            instances: vec![GeometryResourceInstance {
                instance: GeometryInstance {
                    id: GeometryInstanceId(1),
                    definition_id: GeometryDefinitionId(99),
                    transform: DMat4::IDENTITY,
                },
                element_id: SemanticElementId::new("scene/missing"),
                external_id: ExternalId::new("scene/missing"),
                label: "Missing".to_string(),
                declared_entity: "SceneGeometry".to_string(),
                default_render_class: DefaultRenderClass::Physical,
                display_color: None,
            }],
        };

        let error = backend
            .build_package_from_scene(scene)
            .expect_err("missing definition");

        assert!(matches!(
            error,
            GeometryBackendError::MissingDefinitionReference(GeometryDefinitionId(99))
        ));
    }

    #[test]
    fn scene_package_skips_fully_culled_definitions() {
        let backend = GeometryBackend::default();
        let scene = GeometrySceneResource {
            definitions: vec![
                GeometryDefinition {
                    id: GeometryDefinitionId(10),
                    primitive: GeometryPrimitive::Tessellated(
                        TessellatedGeometry::new(
                            vec![
                                DVec3::new(0.0, 0.0, 0.0),
                                DVec3::new(1.0, 0.0, 0.0),
                                DVec3::new(2.0, 0.0, 0.0),
                            ],
                            vec![IndexedPolygon::new(vec![0, 1, 2], vec![], 3).expect("face")],
                        )
                        .expect("geometry"),
                    ),
                },
                triangle_definition(GeometryDefinitionId(11), 1.0),
            ],
            instances: vec![
                GeometryResourceInstance {
                    instance: GeometryInstance {
                        id: GeometryInstanceId(1),
                        definition_id: GeometryDefinitionId(10),
                        transform: DMat4::IDENTITY,
                    },
                    element_id: SemanticElementId::new("scene/culled"),
                    external_id: ExternalId::new("scene/culled"),
                    label: "Culled".to_string(),
                    declared_entity: "SceneGeometry".to_string(),
                    default_render_class: DefaultRenderClass::Physical,
                    display_color: None,
                },
                GeometryResourceInstance {
                    instance: GeometryInstance {
                        id: GeometryInstanceId(2),
                        definition_id: GeometryDefinitionId(11),
                        transform: DMat4::IDENTITY,
                    },
                    element_id: SemanticElementId::new("scene/kept"),
                    external_id: ExternalId::new("scene/kept"),
                    label: "Kept".to_string(),
                    declared_entity: "SceneGeometry".to_string(),
                    default_render_class: DefaultRenderClass::Physical,
                    display_color: None,
                },
            ],
        };

        let package = backend.build_package_from_scene(scene).expect("package");

        assert_eq!(package.definition_count(), 1);
        assert_eq!(package.instance_count(), 1);
        assert_eq!(package.instances[0].external_id.as_str(), "scene/kept");
    }

    fn triangle_definition(id: GeometryDefinitionId, scale: f64) -> GeometryDefinition {
        GeometryDefinition {
            id,
            primitive: GeometryPrimitive::Tessellated(
                TessellatedGeometry::new(
                    vec![
                        DVec3::new(-scale, -scale, 0.0),
                        DVec3::new(scale, -scale, 0.0),
                        DVec3::new(0.0, scale, 0.0),
                    ],
                    vec![IndexedPolygon::new(vec![0, 1, 2], vec![], 3).expect("face")],
                )
                .expect("geometry"),
            ),
        }
    }
}
