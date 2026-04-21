use cc_w_render::{NullRenderBackend, RenderBackend, UploadedMesh};
use cc_w_scene::{MeshHandle, Scene};
use cc_w_types::{
    Bounds3, DefaultRenderClass, GeometryDefinitionId, PreparedGeometryElement,
    PreparedGeometryPackage, PreparedMaterial, PreparedRenderDefinition, PreparedRenderInstance,
    PreparedRenderScene, ResidencyState, SemanticElementId,
};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

pub trait GeometryPackageSource {
    fn load_prepared_package(
        &self,
        resource: &str,
    ) -> Result<PreparedGeometryPackage, GeometryPackageSourceError>;
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum GeometryPackageSourceError {
    #[error("unknown prepared package `{requested}`; available resources: {available}")]
    UnknownResource {
        requested: String,
        available: String,
    },
    #[error("failed to load prepared package: {0}")]
    LoadFailed(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ElementVisibilityOverride {
    #[default]
    Inherit,
    Hidden,
    Visible,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RuntimeElementState {
    pub visibility: ElementVisibilityOverride,
    pub selected: bool,
}

#[derive(Clone, Debug)]
struct IndexedRuntimeElement {
    package_index: usize,
    instance_indices: Vec<usize>,
    state: RuntimeElementState,
}

#[derive(Clone, Debug)]
pub struct RuntimeSceneState {
    package: PreparedGeometryPackage,
    elements_by_id: HashMap<SemanticElementId, IndexedRuntimeElement>,
}

impl RuntimeSceneState {
    pub fn from_prepared_package(package: PreparedGeometryPackage) -> Result<Self, RuntimeError> {
        validate_prepared_package(&package)?;

        let mut elements_by_id = HashMap::with_capacity(package.elements.len());
        for (package_index, element) in package.elements.iter().enumerate() {
            elements_by_id.insert(
                element.id.clone(),
                IndexedRuntimeElement {
                    package_index,
                    instance_indices: Vec::new(),
                    state: RuntimeElementState::default(),
                },
            );
        }

        for (instance_index, instance) in package.instances.iter().enumerate() {
            if let Some(indexed) = elements_by_id.get_mut(&instance.element_id) {
                indexed.instance_indices.push(instance_index);
            }
        }

        Ok(Self {
            package,
            elements_by_id,
        })
    }

    pub fn package(&self) -> &PreparedGeometryPackage {
        &self.package
    }

    pub fn primary_label(&self) -> String {
        self.package
            .elements
            .iter()
            .find(|element| self.is_element_visible_by_id(&element.id).unwrap_or(false))
            .or_else(|| self.package.elements.first())
            .map(|element| element.label.clone())
            .unwrap_or_else(|| "w scene".to_string())
    }

    pub fn element(&self, id: &SemanticElementId) -> Option<&PreparedGeometryElement> {
        let indexed = self.elements_by_id.get(id)?;
        self.package.elements.get(indexed.package_index)
    }

    pub fn element_state(&self, id: &SemanticElementId) -> Option<RuntimeElementState> {
        self.elements_by_id.get(id).map(|indexed| indexed.state)
    }

    pub fn selected_element_ids(&self) -> Vec<SemanticElementId> {
        self.package
            .elements
            .iter()
            .filter(|element| {
                self.elements_by_id
                    .get(&element.id)
                    .is_some_and(|indexed| indexed.state.selected)
            })
            .map(|element| element.id.clone())
            .collect()
    }

    pub fn visible_element_ids(&self) -> Vec<SemanticElementId> {
        self.package
            .elements
            .iter()
            .filter(|element| self.is_element_visible_by_id(&element.id).unwrap_or(false))
            .map(|element| element.id.clone())
            .collect()
    }

    pub fn hide_elements<'a, I>(&mut self, ids: I) -> usize
    where
        I: IntoIterator<Item = &'a SemanticElementId>,
    {
        self.set_visibility_override(ids, ElementVisibilityOverride::Hidden)
    }

    pub fn show_elements<'a, I>(&mut self, ids: I) -> usize
    where
        I: IntoIterator<Item = &'a SemanticElementId>,
    {
        self.set_visibility_override(ids, ElementVisibilityOverride::Visible)
    }

    pub fn reset_visibility<'a, I>(&mut self, ids: I) -> usize
    where
        I: IntoIterator<Item = &'a SemanticElementId>,
    {
        self.set_visibility_override(ids, ElementVisibilityOverride::Inherit)
    }

    pub fn select_elements<'a, I>(&mut self, ids: I) -> usize
    where
        I: IntoIterator<Item = &'a SemanticElementId>,
    {
        let mut changed = 0;
        for id in ids {
            let Some(indexed) = self.elements_by_id.get_mut(id) else {
                continue;
            };
            if !indexed.state.selected {
                indexed.state.selected = true;
                changed += 1;
            }
        }
        changed
    }

    pub fn deselect_elements<'a, I>(&mut self, ids: I) -> usize
    where
        I: IntoIterator<Item = &'a SemanticElementId>,
    {
        let mut changed = 0;
        for id in ids {
            let Some(indexed) = self.elements_by_id.get_mut(id) else {
                continue;
            };
            if indexed.state.selected {
                indexed.state.selected = false;
                changed += 1;
            }
        }
        changed
    }

    pub fn clear_selection(&mut self) -> usize {
        let mut changed = 0;
        for indexed in self.elements_by_id.values_mut() {
            if indexed.state.selected {
                indexed.state.selected = false;
                changed += 1;
            }
        }
        changed
    }

    pub fn visible_bounds(&self) -> Option<Bounds3> {
        self.bounds_for_elements(self.visible_element_ids().iter())
    }

    pub fn bounds_for_elements<'a, I>(&self, ids: I) -> Option<Bounds3>
    where
        I: IntoIterator<Item = &'a SemanticElementId>,
    {
        let mut bounds = None;
        for id in ids {
            let Some(element) = self.element(id) else {
                continue;
            };
            bounds = Some(union_bounds(bounds, element.bounds));
        }
        bounds
    }

    pub fn compose_render_scene(&self) -> PreparedRenderScene {
        let visible_instances = self
            .package
            .instances
            .iter()
            .filter(|instance| {
                self.is_element_visible_by_id(&instance.element_id)
                    .unwrap_or(false)
            })
            .collect::<Vec<_>>();

        let visible_definition_ids = visible_instances
            .iter()
            .map(|instance| instance.definition_id)
            .collect::<HashSet<_>>();
        let definitions = self
            .package
            .definitions
            .iter()
            .filter(|definition| visible_definition_ids.contains(&definition.id))
            .map(|definition| PreparedRenderDefinition {
                id: definition.id,
                mesh: definition.mesh.clone(),
            })
            .collect::<Vec<_>>();
        let instances = visible_instances
            .into_iter()
            .map(|instance| PreparedRenderInstance {
                id: instance.id,
                definition_id: instance.definition_id,
                model_from_object: instance.transform,
                world_bounds: instance.bounds,
                material: PreparedMaterial::new(instance.display_color.unwrap_or_default()),
            })
            .collect::<Vec<_>>();

        PreparedRenderScene {
            bounds: visible_scene_bounds(&instances),
            definitions,
            instances,
        }
    }

    pub fn compose_scene_graph(&self) -> Scene {
        let mut scene = Scene::new(self.primary_label());

        for instance in &self.package.instances {
            if !self
                .is_element_visible_by_id(&instance.element_id)
                .unwrap_or(false)
            {
                continue;
            }

            scene.insert_geometry_instance(
                scene.root(),
                instance.external_id.clone(),
                instance.label.clone(),
                instance.transform,
                instance.bounds,
                instance.definition_id,
                MeshHandle(instance.definition_id.0),
                ResidencyState::CpuReady,
            );
        }

        scene
    }

    fn is_element_visible_by_id(&self, id: &SemanticElementId) -> Option<bool> {
        let indexed = self.elements_by_id.get(id)?;
        let element = &self.package.elements[indexed.package_index];
        Some(match indexed.state.visibility {
            ElementVisibilityOverride::Inherit => {
                default_visibility_for_class(element.default_render_class)
            }
            ElementVisibilityOverride::Hidden => false,
            ElementVisibilityOverride::Visible => true,
        })
    }

    fn set_visibility_override<'a, I>(
        &mut self,
        ids: I,
        visibility: ElementVisibilityOverride,
    ) -> usize
    where
        I: IntoIterator<Item = &'a SemanticElementId>,
    {
        let mut changed = 0;
        for id in ids {
            let Some(indexed) = self.elements_by_id.get_mut(id) else {
                continue;
            };
            if indexed.state.visibility != visibility {
                indexed.state.visibility = visibility;
                changed += 1;
            }
        }
        changed
    }
}

fn default_visibility_for_class(class: DefaultRenderClass) -> bool {
    match class {
        DefaultRenderClass::Physical | DefaultRenderClass::Other => true,
        DefaultRenderClass::Space | DefaultRenderClass::Zone | DefaultRenderClass::Helper => false,
    }
}

fn visible_scene_bounds(instances: &[PreparedRenderInstance]) -> Bounds3 {
    instances
        .iter()
        .fold(None, |bounds, instance| {
            Some(union_bounds(bounds, instance.world_bounds))
        })
        .unwrap_or_else(Bounds3::zero)
}

fn union_bounds(current: Option<Bounds3>, next: Bounds3) -> Bounds3 {
    match current {
        Some(bounds) => Bounds3 {
            min: bounds.min.min(next.min),
            max: bounds.max.max(next.max),
        },
        None => next,
    }
}

#[derive(Debug)]
pub struct Engine<S, B = NullRenderBackend> {
    package_source: S,
    renderer: B,
}

impl<S, B> Engine<S, B>
where
    S: GeometryPackageSource,
    B: RenderBackend,
{
    pub fn new(package_source: S, renderer: B) -> Self {
        Self {
            package_source,
            renderer,
        }
    }

    pub fn build_runtime_scene_for(
        &self,
        resource: &str,
    ) -> Result<RuntimeSceneState, RuntimeError> {
        let package = self
            .package_source
            .load_prepared_package(resource)
            .map_err(RuntimeError::from)?;
        RuntimeSceneState::from_prepared_package(package)
    }

    pub fn build_demo_asset_for(&self, resource: &str) -> Result<DemoAsset, RuntimeError> {
        let runtime_scene = self.build_runtime_scene_for(resource)?;
        let render_scene = runtime_scene.compose_render_scene();
        let label = runtime_scene.primary_label();
        let scene = runtime_scene.compose_scene_graph();

        Ok(DemoAsset {
            label,
            scene_nodes: scene.node_count(),
            definitions: render_scene.definition_count(),
            source_vertices: render_scene.vertex_count(),
            prepared_vertices: render_scene.vertex_count(),
            triangles: render_scene.triangle_count(),
            bounds: render_scene.bounds,
            render_scene,
        })
    }

    pub fn build_demo_frame_for(&mut self, resource: &str) -> Result<DemoFrame, RuntimeError> {
        let asset = self.build_demo_asset_for(resource)?;
        let uploads = asset
            .render_scene
            .definitions
            .iter()
            .map(|definition| self.renderer.upload(&definition.mesh))
            .collect::<Vec<_>>();

        Ok(DemoFrame {
            label: asset.label,
            scene_nodes: asset.scene_nodes,
            definitions: asset.definitions,
            source_vertices: asset.source_vertices,
            prepared_vertices: asset.prepared_vertices,
            triangles: asset.triangles,
            bounds: asset.bounds,
            uploads,
            draw_instances: asset.render_scene.draw_count(),
        })
    }
}

#[derive(Clone, Debug)]
pub struct DemoAsset {
    pub label: String,
    pub scene_nodes: usize,
    pub definitions: usize,
    pub source_vertices: usize,
    pub prepared_vertices: usize,
    pub triangles: usize,
    pub bounds: Bounds3,
    pub render_scene: PreparedRenderScene,
}

impl DemoAsset {
    pub fn summary_line(&self) -> String {
        format!(
            "{}: {} source vertices -> {} prepared vertices, {} triangles, {} scene nodes, {} definitions",
            self.label,
            self.source_vertices,
            self.prepared_vertices,
            self.triangles,
            self.scene_nodes,
            self.definitions
        )
    }
}

#[derive(Clone, Debug)]
pub struct DemoFrame {
    pub label: String,
    pub scene_nodes: usize,
    pub definitions: usize,
    pub source_vertices: usize,
    pub prepared_vertices: usize,
    pub triangles: usize,
    pub bounds: Bounds3,
    pub uploads: Vec<UploadedMesh>,
    pub draw_instances: usize,
}

impl DemoFrame {
    pub fn summary_line(&self) -> String {
        format!(
            "{}: {} source vertices -> {} prepared vertices, {} triangles, {} scene nodes, {} definitions, {} mesh uploads, {} draw instances",
            self.label,
            self.source_vertices,
            self.prepared_vertices,
            self.triangles,
            self.scene_nodes,
            self.definitions,
            self.uploads.len(),
            self.draw_instances
        )
    }
}

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error(transparent)]
    Source(#[from] GeometryPackageSourceError),
    #[error("prepared geometry package must contain at least one definition")]
    MissingDefinitions,
    #[error("prepared geometry package must contain at least one semantic element")]
    MissingElements,
    #[error("prepared geometry package must contain at least one instance")]
    MissingInstances,
    #[error("prepared geometry package references missing definition {0:?}")]
    MissingDefinition(GeometryDefinitionId),
    #[error("prepared geometry package references missing semantic element {0:?}")]
    MissingElement(SemanticElementId),
}

fn validate_prepared_package(package: &PreparedGeometryPackage) -> Result<(), RuntimeError> {
    if package.definitions.is_empty() {
        return Err(RuntimeError::MissingDefinitions);
    }
    if package.elements.is_empty() {
        return Err(RuntimeError::MissingElements);
    }
    if package.instances.is_empty() {
        return Err(RuntimeError::MissingInstances);
    }

    let definition_ids = package
        .definitions
        .iter()
        .map(|definition| definition.id)
        .collect::<HashSet<_>>();
    let element_ids = package
        .elements
        .iter()
        .map(|element| element.id.clone())
        .collect::<HashSet<_>>();

    for instance in &package.instances {
        if !definition_ids.contains(&instance.definition_id) {
            return Err(RuntimeError::MissingDefinition(instance.definition_id));
        }
        if !element_ids.contains(&instance.element_id) {
            return Err(RuntimeError::MissingElement(instance.element_id.clone()));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cc_w_types::{
        Bounds3, DefaultRenderClass, DisplayColor, ExternalId, GeometryDefinitionId,
        GeometryInstanceId, PreparedGeometryDefinition, PreparedGeometryElement,
        PreparedGeometryInstance, PreparedMesh, PreparedVertex, SemanticElementId,
    };
    use glam::{DMat4, DVec3};
    use std::collections::HashMap;

    #[derive(Debug, Default)]
    struct SyntheticGeometryPackageSource {
        packages: HashMap<String, PreparedGeometryPackage>,
    }

    impl SyntheticGeometryPackageSource {
        fn with_package(resource: &str, package: PreparedGeometryPackage) -> Self {
            let mut packages = HashMap::new();
            packages.insert(resource.to_string(), package);
            Self { packages }
        }
    }

    impl GeometryPackageSource for SyntheticGeometryPackageSource {
        fn load_prepared_package(
            &self,
            resource: &str,
        ) -> Result<PreparedGeometryPackage, GeometryPackageSourceError> {
            self.packages.get(resource).cloned().ok_or_else(|| {
                GeometryPackageSourceError::UnknownResource {
                    requested: resource.to_string(),
                    available: self.packages.keys().cloned().collect::<Vec<_>>().join(", "),
                }
            })
        }
    }

    fn triangle_package(label: &str, external_id: &str) -> PreparedGeometryPackage {
        let definition_id = GeometryDefinitionId(7);
        let element_id = SemanticElementId::new(external_id);
        let mesh = prepared_convex_polygon(vec![
            DVec3::new(-1.0, -1.0, 0.0),
            DVec3::new(1.0, -1.0, 0.0),
            DVec3::new(0.0, 1.5, 0.0),
        ]);

        PreparedGeometryPackage {
            definitions: vec![PreparedGeometryDefinition {
                id: definition_id,
                mesh: mesh.clone(),
            }],
            elements: vec![PreparedGeometryElement {
                id: element_id.clone(),
                label: label.to_string(),
                declared_entity: "DemoGeometry".to_string(),
                default_render_class: DefaultRenderClass::Physical,
                bounds: mesh.bounds,
            }],
            instances: vec![PreparedGeometryInstance {
                id: GeometryInstanceId(1),
                element_id,
                definition_id,
                transform: DMat4::IDENTITY,
                bounds: mesh.bounds,
                external_id: ExternalId::new(external_id),
                label: label.to_string(),
                display_color: None,
            }],
        }
    }

    fn mapped_triangle_package(label: &str, external_id: &str) -> PreparedGeometryPackage {
        let definition_id = GeometryDefinitionId(8);
        let left_element_id = SemanticElementId::new(external_id);
        let right_element_id = SemanticElementId::new("synthetic/mapped/right");
        let mesh = prepared_convex_polygon(vec![
            DVec3::new(-1.0, -1.0, 0.0),
            DVec3::new(1.0, -1.0, 0.0),
            DVec3::new(0.0, 1.5, 0.0),
        ]);
        let left_bounds = mesh
            .bounds
            .transformed(DMat4::from_translation(DVec3::new(-2.5, 0.0, 0.0)));
        let right_bounds = mesh
            .bounds
            .transformed(DMat4::from_translation(DVec3::new(2.5, 0.75, 0.0)));

        PreparedGeometryPackage {
            definitions: vec![PreparedGeometryDefinition {
                id: definition_id,
                mesh: mesh.clone(),
            }],
            elements: vec![
                PreparedGeometryElement {
                    id: left_element_id.clone(),
                    label: label.to_string(),
                    declared_entity: "DemoGeometry".to_string(),
                    default_render_class: DefaultRenderClass::Physical,
                    bounds: left_bounds,
                },
                PreparedGeometryElement {
                    id: right_element_id.clone(),
                    label: format!("{label} #2"),
                    declared_entity: "DemoGeometry".to_string(),
                    default_render_class: DefaultRenderClass::Physical,
                    bounds: right_bounds,
                },
            ],
            instances: vec![
                PreparedGeometryInstance {
                    id: GeometryInstanceId(1),
                    element_id: left_element_id,
                    definition_id,
                    transform: DMat4::from_translation(DVec3::new(-2.5, 0.0, 0.0)),
                    bounds: left_bounds,
                    external_id: ExternalId::new(external_id),
                    label: label.to_string(),
                    display_color: Some(DisplayColor::new(0.95, 0.56, 0.24)),
                },
                PreparedGeometryInstance {
                    id: GeometryInstanceId(2),
                    element_id: right_element_id,
                    definition_id,
                    transform: DMat4::from_translation(DVec3::new(2.5, 0.75, 0.0)),
                    bounds: right_bounds,
                    external_id: ExternalId::new("synthetic/mapped/right"),
                    label: format!("{label} #2"),
                    display_color: Some(DisplayColor::new(0.24, 0.78, 0.55)),
                },
            ],
        }
    }

    fn mixed_render_class_package() -> PreparedGeometryPackage {
        let definition_id = GeometryDefinitionId(17);
        let mesh = prepared_convex_polygon(vec![
            DVec3::new(-1.0, -1.0, 0.0),
            DVec3::new(1.0, -1.0, 0.0),
            DVec3::new(0.0, 1.5, 0.0),
        ]);
        let physical_element_id = SemanticElementId::new("synthetic/physical");
        let space_element_id = SemanticElementId::new("synthetic/space");
        let physical_bounds = mesh
            .bounds
            .transformed(DMat4::from_translation(DVec3::new(-2.0, 0.0, 0.0)));
        let space_bounds = mesh
            .bounds
            .transformed(DMat4::from_translation(DVec3::new(2.0, 0.0, 0.0)));

        PreparedGeometryPackage {
            definitions: vec![PreparedGeometryDefinition {
                id: definition_id,
                mesh: mesh.clone(),
            }],
            elements: vec![
                PreparedGeometryElement {
                    id: physical_element_id.clone(),
                    label: "Physical".to_string(),
                    declared_entity: "IfcWall".to_string(),
                    default_render_class: DefaultRenderClass::Physical,
                    bounds: physical_bounds,
                },
                PreparedGeometryElement {
                    id: space_element_id.clone(),
                    label: "Space".to_string(),
                    declared_entity: "IfcSpace".to_string(),
                    default_render_class: DefaultRenderClass::Space,
                    bounds: space_bounds,
                },
            ],
            instances: vec![
                PreparedGeometryInstance {
                    id: GeometryInstanceId(1),
                    element_id: physical_element_id,
                    definition_id,
                    transform: DMat4::from_translation(DVec3::new(-2.0, 0.0, 0.0)),
                    bounds: physical_bounds,
                    external_id: ExternalId::new("synthetic/physical/item/1"),
                    label: "Physical".to_string(),
                    display_color: None,
                },
                PreparedGeometryInstance {
                    id: GeometryInstanceId(2),
                    element_id: space_element_id,
                    definition_id,
                    transform: DMat4::from_translation(DVec3::new(2.0, 0.0, 0.0)),
                    bounds: space_bounds,
                    external_id: ExternalId::new("synthetic/space/item/1"),
                    label: "Space".to_string(),
                    display_color: None,
                },
            ],
        }
    }

    fn prepared_convex_polygon(vertices: Vec<DVec3>) -> PreparedMesh {
        let bounds = Bounds3::from_points(&vertices).expect("bounds");
        let local_origin = bounds.center();
        let indices = (1..vertices.len().saturating_sub(1))
            .flat_map(|index| [0_u32, index as u32, index as u32 + 1])
            .collect();
        let vertices = vertices
            .into_iter()
            .map(|position| {
                let local = position - local_origin;
                PreparedVertex {
                    position: [local.x as f32, local.y as f32, local.z as f32],
                    normal: [0.0, 0.0, 1.0],
                }
            })
            .collect();

        PreparedMesh {
            local_origin,
            bounds,
            vertices,
            indices,
        }
    }

    #[test]
    fn engine_projects_prepared_package_into_asset() {
        let source = SyntheticGeometryPackageSource::with_package(
            "demo/triangle",
            triangle_package("Synthetic Triangle", "synthetic/triangle"),
        );
        let engine = Engine::new(source, NullRenderBackend::default());

        let asset = engine.build_demo_asset_for("demo/triangle").expect("asset");

        assert_eq!(asset.label, "Synthetic Triangle");
        assert_eq!(asset.scene_nodes, 2);
        assert_eq!(asset.definitions, 1);
        assert_eq!(asset.source_vertices, 3);
        assert_eq!(asset.prepared_vertices, 3);
        assert_eq!(asset.triangles, 1);
        assert_eq!(asset.render_scene.draw_count(), 1);
        assert_eq!(asset.render_scene.definition_count(), 1);
        assert_eq!(
            asset.render_scene.definitions[0].mesh.indices,
            vec![0, 1, 2]
        );
        assert_eq!(asset.render_scene.definitions[0].mesh.vertex_count(), 3);
        assert_eq!(
            asset.render_scene.instances[0].model_from_object,
            DMat4::IDENTITY
        );
    }

    #[test]
    fn engine_uploads_prepared_package_mesh() {
        let source = SyntheticGeometryPackageSource::with_package(
            "demo/triangle",
            triangle_package("Synthetic Triangle", "synthetic/triangle"),
        );
        let mut engine = Engine::new(source, NullRenderBackend::default());

        let frame = engine.build_demo_frame_for("demo/triangle").expect("frame");

        assert_eq!(frame.label, "Synthetic Triangle");
        assert_eq!(frame.uploads.len(), 1);
        assert_eq!(frame.uploads[0].vertex_count, 3);
        assert_eq!(frame.uploads[0].index_count, 3);
        assert_eq!(frame.uploads[0].shader_entry, "vs_main");
        assert_eq!(frame.draw_instances, 1);
        assert_eq!(
            frame.summary_line(),
            "Synthetic Triangle: 3 source vertices -> 3 prepared vertices, 1 triangles, 2 scene nodes, 1 definitions, 1 mesh uploads, 1 draw instances"
        );
    }

    #[test]
    fn engine_builds_instanced_render_scene_for_mapped_instances() {
        let source = SyntheticGeometryPackageSource::with_package(
            "demo/mapped-triangle",
            mapped_triangle_package("Synthetic Mapped Triangle", "synthetic/mapped/left"),
        );
        let engine = Engine::new(source, NullRenderBackend::default());

        let asset = engine
            .build_demo_asset_for("demo/mapped-triangle")
            .expect("asset");

        assert_eq!(asset.label, "Synthetic Mapped Triangle");
        assert_eq!(asset.scene_nodes, 3);
        assert_eq!(asset.definitions, 1);
        assert_eq!(asset.prepared_vertices, 6);
        assert_eq!(asset.triangles, 2);
        assert!(asset.bounds.min.x < -3.4);
        assert!(asset.bounds.max.x > 3.4);
        assert!(asset.bounds.max.y > 2.2);
        assert_eq!(asset.render_scene.draw_count(), 2);
        assert_eq!(asset.render_scene.definition_count(), 1);
        assert_eq!(
            asset.render_scene.instances[0].material.color.as_rgb(),
            [0.95, 0.56, 0.24]
        );
        assert_eq!(
            asset.render_scene.instances[1].material.color.as_rgb(),
            [0.24, 0.78, 0.55]
        );
        assert_eq!(
            asset.render_scene.instances[0].definition_id,
            asset.render_scene.definitions[0].id
        );
        assert_eq!(
            asset.render_scene.instances[1].definition_id,
            asset.render_scene.definitions[0].id
        );
        assert_eq!(
            asset.render_scene.instances[0].model_from_object,
            DMat4::from_translation(DVec3::new(-2.5, 0.0, 0.0))
        );
        assert_eq!(
            asset.render_scene.instances[1].model_from_object,
            DMat4::from_translation(DVec3::new(2.5, 0.75, 0.0))
        );
    }

    #[test]
    fn runtime_scene_defaults_hide_non_physical_elements() {
        let runtime_scene =
            RuntimeSceneState::from_prepared_package(mixed_render_class_package()).expect("scene");

        assert_eq!(
            runtime_scene.visible_element_ids(),
            vec![SemanticElementId::new("synthetic/physical")]
        );
        assert_eq!(runtime_scene.compose_render_scene().draw_count(), 1);
        assert_eq!(runtime_scene.compose_scene_graph().node_count(), 2);
    }

    #[test]
    fn runtime_scene_visibility_overrides_recompose_visible_draws() {
        let left_id = SemanticElementId::new("synthetic/mapped/left");
        let right_id = SemanticElementId::new("synthetic/mapped/right");
        let mut runtime_scene = RuntimeSceneState::from_prepared_package(mapped_triangle_package(
            "Synthetic Mapped Triangle",
            "synthetic/mapped/left",
        ))
        .expect("scene");

        assert_eq!(runtime_scene.compose_render_scene().draw_count(), 2);
        assert_eq!(runtime_scene.hide_elements([&right_id]), 1);
        assert_eq!(runtime_scene.compose_render_scene().draw_count(), 1);
        assert_eq!(runtime_scene.show_elements([&right_id]), 1);
        assert_eq!(runtime_scene.compose_render_scene().draw_count(), 2);
        assert_eq!(runtime_scene.hide_elements([&left_id, &right_id]), 2);
        assert_eq!(runtime_scene.compose_render_scene().draw_count(), 0);
        assert_eq!(runtime_scene.reset_visibility([&left_id, &right_id]), 2);
        assert_eq!(runtime_scene.compose_render_scene().draw_count(), 2);
    }

    #[test]
    fn runtime_scene_tracks_selection_and_frame_bounds_by_element() {
        let left_id = SemanticElementId::new("synthetic/mapped/left");
        let right_id = SemanticElementId::new("synthetic/mapped/right");
        let mut runtime_scene = RuntimeSceneState::from_prepared_package(mapped_triangle_package(
            "Synthetic Mapped Triangle",
            "synthetic/mapped/left",
        ))
        .expect("scene");

        assert_eq!(runtime_scene.select_elements([&left_id, &right_id]), 2);
        assert_eq!(
            runtime_scene.selected_element_ids(),
            vec![left_id.clone(), right_id.clone()]
        );
        let framed = runtime_scene
            .bounds_for_elements([&left_id, &right_id])
            .expect("bounds");
        assert!(framed.min.x < -3.4);
        assert!(framed.max.x > 3.4);
        assert_eq!(runtime_scene.clear_selection(), 2);
        assert!(runtime_scene.selected_element_ids().is_empty());
    }

    #[test]
    fn engine_rejects_package_instances_without_definitions() {
        let package = PreparedGeometryPackage {
            definitions: vec![],
            elements: vec![PreparedGeometryElement {
                id: SemanticElementId::new("synthetic/missing-definition"),
                label: "Broken".to_string(),
                declared_entity: "DemoGeometry".to_string(),
                default_render_class: DefaultRenderClass::Physical,
                bounds: Bounds3::zero(),
            }],
            instances: vec![PreparedGeometryInstance {
                id: GeometryInstanceId(1),
                element_id: SemanticElementId::new("synthetic/missing-definition"),
                definition_id: GeometryDefinitionId(99),
                transform: DMat4::IDENTITY,
                bounds: Bounds3::zero(),
                external_id: ExternalId::new("synthetic/missing-definition"),
                label: "Broken".to_string(),
                display_color: None,
            }],
        };
        let source = SyntheticGeometryPackageSource::with_package("broken", package);
        let engine = Engine::new(source, NullRenderBackend::default());

        let err = engine.build_demo_asset_for("broken").expect_err("error");
        assert!(matches!(err, RuntimeError::MissingDefinitions));
    }

    #[test]
    fn engine_rejects_package_instances_without_elements() {
        let package = PreparedGeometryPackage {
            definitions: vec![PreparedGeometryDefinition {
                id: GeometryDefinitionId(1),
                mesh: prepared_convex_polygon(vec![
                    DVec3::new(-1.0, -1.0, 0.0),
                    DVec3::new(1.0, -1.0, 0.0),
                    DVec3::new(0.0, 1.5, 0.0),
                ]),
            }],
            elements: vec![],
            instances: vec![PreparedGeometryInstance {
                id: GeometryInstanceId(1),
                element_id: SemanticElementId::new("synthetic/missing-element"),
                definition_id: GeometryDefinitionId(1),
                transform: DMat4::IDENTITY,
                bounds: Bounds3::zero(),
                external_id: ExternalId::new("synthetic/missing-element/item/1"),
                label: "Broken".to_string(),
                display_color: None,
            }],
        };
        let source = SyntheticGeometryPackageSource::with_package("broken-elements", package);
        let engine = Engine::new(source, NullRenderBackend::default());

        let err = engine
            .build_demo_asset_for("broken-elements")
            .expect_err("error");
        assert!(matches!(err, RuntimeError::MissingElements));
    }

    #[test]
    fn engine_surfaces_unknown_resource_errors_from_source() {
        let engine = Engine::new(
            SyntheticGeometryPackageSource::default(),
            NullRenderBackend::default(),
        );

        let err = engine.build_demo_asset_for("missing").expect_err("error");
        assert!(matches!(
            err,
            RuntimeError::Source(GeometryPackageSourceError::UnknownResource { .. })
        ));
    }
}
