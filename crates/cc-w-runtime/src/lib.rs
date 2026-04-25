use cc_w_render::{Camera, NullRenderBackend, RenderBackend, UploadedMesh, ViewportSize};
use cc_w_scene::{MeshHandle, Scene};
use cc_w_types::{
    Bounds3, DefaultRenderClass, DisplayColor, GeometryCatalog, GeometryDefinitionBatch,
    GeometryDefinitionBatchRequest, GeometryDefinitionId, GeometryInstanceBatch,
    GeometryInstanceBatchRequest, GeometryInstanceId, GeometryPrioritizedStreamEntry,
    GeometryPrioritizedStreamPlan, GeometryStartViewRequest, GeometryStreamPlan,
    GeometryStreamPlanReason, GeometryStreamingBudget, PreparedGeometryDefinition,
    PreparedGeometryElement, PreparedGeometryInstance, PreparedGeometryPackage, PreparedMaterial,
    PreparedRenderDefinition, PreparedRenderInstance, PreparedRenderScene, ResidencyState,
    ResolvedGeometryStartView, SemanticElementId,
};
use glam::{DVec3, DVec4};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

const SELECTED_ELEMENT_MATERIAL_COLOR: DisplayColor = DisplayColor::new(1.0, 0.88, 0.18);

pub trait GeometryPackageSource {
    fn load_prepared_package(
        &self,
        resource: &str,
    ) -> Result<PreparedGeometryPackage, GeometryPackageSourceError>;
}

pub trait GeometryStreamProvider {
    fn load_catalog(&self, resource: &str) -> Result<GeometryCatalog, GeometryPackageSourceError>;

    fn load_instance_batch(
        &self,
        resource: &str,
        request: &GeometryInstanceBatchRequest,
    ) -> Result<GeometryInstanceBatch, GeometryPackageSourceError>;

    fn load_definition_batch(
        &self,
        resource: &str,
        request: &GeometryDefinitionBatchRequest,
    ) -> Result<GeometryDefinitionBatch, GeometryPackageSourceError>;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RuntimeResidencyCounts {
    pub instances: usize,
    pub definitions: usize,
}

#[derive(Clone, Debug)]
pub struct FullPackageGeometryStreamProvider<S> {
    package_source: S,
}

impl<S> FullPackageGeometryStreamProvider<S> {
    pub fn new(package_source: S) -> Self {
        Self { package_source }
    }

    pub fn package_source(&self) -> &S {
        &self.package_source
    }

    pub fn into_package_source(self) -> S {
        self.package_source
    }
}

impl<S> GeometryStreamProvider for FullPackageGeometryStreamProvider<S>
where
    S: GeometryPackageSource,
{
    fn load_catalog(&self, resource: &str) -> Result<GeometryCatalog, GeometryPackageSourceError> {
        self.package_source
            .load_prepared_package(resource)
            .map(|package| package.catalog())
    }

    fn load_instance_batch(
        &self,
        resource: &str,
        request: &GeometryInstanceBatchRequest,
    ) -> Result<GeometryInstanceBatch, GeometryPackageSourceError> {
        let package = self.package_source.load_prepared_package(resource)?;
        Ok(package.catalog().instance_batch(request))
    }

    fn load_definition_batch(
        &self,
        resource: &str,
        request: &GeometryDefinitionBatchRequest,
    ) -> Result<GeometryDefinitionBatch, GeometryPackageSourceError> {
        let package = self.package_source.load_prepared_package(resource)?;
        Ok(package.definition_batch(request))
    }
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
    pub suppressed: bool,
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
    catalog: GeometryCatalog,
    compatibility_package: PreparedGeometryPackage,
    elements_by_id: HashMap<SemanticElementId, IndexedRuntimeElement>,
    resident_instances: HashMap<GeometryInstanceId, cc_w_types::GeometryInstanceCatalogEntry>,
    resident_definitions: HashMap<GeometryDefinitionId, PreparedGeometryDefinition>,
    start_view: GeometryStartViewRequest,
    base_visible_element_ids: HashSet<SemanticElementId>,
}

impl RuntimeSceneState {
    pub fn from_prepared_package(package: PreparedGeometryPackage) -> Result<Self, RuntimeError> {
        Self::from_prepared_package_with_start_view(package, GeometryStartViewRequest::Default)
    }

    pub fn from_prepared_package_with_start_view(
        package: PreparedGeometryPackage,
        start_view: GeometryStartViewRequest,
    ) -> Result<Self, RuntimeError> {
        validate_prepared_package(&package)?;
        let catalog = package.catalog();
        let instance_batch = catalog.instance_batch(&GeometryInstanceBatchRequest::new(
            catalog
                .instances
                .iter()
                .map(|instance| instance.id)
                .collect(),
        ));
        let definition_batch = GeometryDefinitionBatch {
            definitions: package.definitions.clone(),
        };
        let mut scene = Self::from_catalog_with_start_view(catalog, start_view)?;
        scene.mark_instance_batch_resident(&instance_batch);
        scene.mark_definition_batch_resident(&definition_batch);
        Ok(scene)
    }

    pub fn from_catalog(catalog: GeometryCatalog) -> Result<Self, RuntimeError> {
        Self::from_catalog_with_start_view(catalog, GeometryStartViewRequest::Default)
    }

    pub fn from_catalog_with_start_view(
        catalog: GeometryCatalog,
        start_view: GeometryStartViewRequest,
    ) -> Result<Self, RuntimeError> {
        validate_geometry_catalog(&catalog)?;

        let mut elements_by_id = HashMap::with_capacity(catalog.elements.len());
        for (package_index, element) in catalog.elements.iter().enumerate() {
            elements_by_id.insert(
                element.id.clone(),
                IndexedRuntimeElement {
                    package_index,
                    instance_indices: Vec::new(),
                    state: RuntimeElementState::default(),
                },
            );
        }

        for (instance_index, instance) in catalog.instances.iter().enumerate() {
            if let Some(indexed) = elements_by_id.get_mut(&instance.element_id) {
                indexed.instance_indices.push(instance_index);
            }
        }

        let mut scene = Self {
            compatibility_package: compatibility_package_from_resident_stores(
                &catalog,
                &HashMap::new(),
                &HashMap::new(),
            ),
            catalog,
            elements_by_id,
            resident_instances: HashMap::new(),
            resident_definitions: HashMap::new(),
            start_view: GeometryStartViewRequest::Default,
            base_visible_element_ids: HashSet::new(),
        };
        scene.apply_start_view(start_view);
        Ok(scene)
    }

    pub fn package(&self) -> &PreparedGeometryPackage {
        &self.compatibility_package
    }

    pub fn catalog(&self) -> GeometryCatalog {
        self.catalog.clone()
    }

    pub fn start_view_request(&self) -> &GeometryStartViewRequest {
        &self.start_view
    }

    pub fn resolve_start_view(
        &self,
        request: &GeometryStartViewRequest,
    ) -> ResolvedGeometryStartView {
        match request {
            GeometryStartViewRequest::Default => ResolvedGeometryStartView {
                visible_element_ids: self
                    .catalog
                    .elements
                    .iter()
                    .filter(|element| default_visibility_for_class(element.default_render_class))
                    .map(|element| element.id.clone())
                    .collect(),
            },
            GeometryStartViewRequest::Minimal(max_elements) => ResolvedGeometryStartView {
                visible_element_ids: self
                    .catalog
                    .elements
                    .iter()
                    .filter(|element| default_visibility_for_class(element.default_render_class))
                    .take(*max_elements)
                    .map(|element| element.id.clone())
                    .collect(),
            },
            GeometryStartViewRequest::All => ResolvedGeometryStartView {
                visible_element_ids: self
                    .catalog
                    .elements
                    .iter()
                    .map(|element| element.id.clone())
                    .collect(),
            },
            GeometryStartViewRequest::Elements(ids) => {
                let mut seen = HashSet::new();
                let visible_element_ids = ids
                    .iter()
                    .filter(|id| self.elements_by_id.contains_key(*id))
                    .filter(|id| seen.insert((*id).clone()))
                    .cloned()
                    .collect();
                ResolvedGeometryStartView {
                    visible_element_ids,
                }
            }
        }
    }

    pub fn apply_start_view(
        &mut self,
        request: GeometryStartViewRequest,
    ) -> ResolvedGeometryStartView {
        let resolved = self.resolve_start_view(&request);
        self.start_view = request;
        self.base_visible_element_ids = resolved.visible_element_ids.iter().cloned().collect();
        resolved
    }

    pub fn primary_label(&self) -> String {
        self.catalog
            .elements
            .iter()
            .find(|element| self.is_element_visible_by_id(&element.id).unwrap_or(false))
            .or_else(|| self.catalog.elements.first())
            .map(|element| element.label.clone())
            .unwrap_or_else(|| "w scene".to_string())
    }

    pub fn element(&self, id: &SemanticElementId) -> Option<&PreparedGeometryElement> {
        let indexed = self.elements_by_id.get(id)?;
        self.compatibility_package
            .elements
            .get(indexed.package_index)
    }

    pub fn element_state(&self, id: &SemanticElementId) -> Option<RuntimeElementState> {
        self.elements_by_id.get(id).map(|indexed| indexed.state)
    }

    pub fn selected_element_ids(&self) -> Vec<SemanticElementId> {
        self.catalog
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
        self.catalog
            .elements
            .iter()
            .filter(|element| self.is_element_visible_by_id(&element.id).unwrap_or(false))
            .map(|element| element.id.clone())
            .collect()
    }

    pub fn base_visible_element_ids(&self) -> Vec<SemanticElementId> {
        self.catalog
            .elements
            .iter()
            .filter(|element| self.base_visible_element_ids.contains(&element.id))
            .map(|element| element.id.clone())
            .collect()
    }

    pub fn hidden_element_ids(&self) -> Vec<SemanticElementId> {
        self.element_ids_with_visibility_override(ElementVisibilityOverride::Hidden)
    }

    pub fn shown_element_ids(&self) -> Vec<SemanticElementId> {
        self.element_ids_with_visibility_override(ElementVisibilityOverride::Visible)
    }

    pub fn suppressed_element_ids(&self) -> Vec<SemanticElementId> {
        self.catalog
            .elements
            .iter()
            .filter(|element| {
                self.elements_by_id
                    .get(&element.id)
                    .is_some_and(|indexed| indexed.state.suppressed)
            })
            .map(|element| element.id.clone())
            .collect()
    }

    pub fn stream_plan_for_visible_elements(&self) -> GeometryStreamPlan {
        self.stream_plan_for_elements(self.visible_element_ids().iter())
    }

    pub fn stream_plan_for_elements<'a, I>(&self, ids: I) -> GeometryStreamPlan
    where
        I: IntoIterator<Item = &'a SemanticElementId>,
    {
        let mut seen_instances = HashSet::new();
        let mut seen_definitions = HashSet::new();
        let mut instance_ids = Vec::new();
        let mut definition_ids = Vec::new();

        for id in ids {
            let Some(indexed) = self.elements_by_id.get(id) else {
                continue;
            };
            for instance_index in &indexed.instance_indices {
                let Some(instance) = self.catalog.instances.get(*instance_index) else {
                    continue;
                };
                if seen_instances.insert(instance.id) {
                    instance_ids.push(instance.id);
                }
                if seen_definitions.insert(instance.definition_id) {
                    definition_ids.push(instance.definition_id);
                }
            }
        }

        GeometryStreamPlan {
            instance_ids,
            definition_ids,
        }
    }

    pub fn missing_stream_plan_for_visible_elements(&self) -> GeometryStreamPlan {
        let plan = self.stream_plan_for_visible_elements();
        GeometryStreamPlan {
            instance_ids: plan
                .instance_ids
                .into_iter()
                .filter(|id| !self.resident_instances.contains_key(id))
                .collect(),
            definition_ids: plan
                .definition_ids
                .into_iter()
                .filter(|id| !self.resident_definitions.contains_key(id))
                .collect(),
        }
    }

    pub fn prioritized_missing_stream_plan_for_visible_elements(
        &self,
        camera: Camera,
        viewport: ViewportSize,
        budget: GeometryStreamingBudget,
    ) -> GeometryPrioritizedStreamPlan {
        let clip_from_world = camera.clip_from_world(viewport);
        let mut entries = Vec::new();

        for element_id in self.visible_element_ids() {
            let Some(indexed) = self.elements_by_id.get(&element_id) else {
                continue;
            };
            let selected = indexed.state.selected;
            for instance_index in &indexed.instance_indices {
                let Some(instance) = self.catalog.instances.get(*instance_index) else {
                    continue;
                };
                let needs_instance = !self.resident_instances.contains_key(&instance.id);
                let needs_definition = !self
                    .resident_definitions
                    .contains_key(&instance.definition_id);
                if !needs_instance && !needs_definition {
                    continue;
                }

                let projection =
                    project_bounds_for_streaming(instance.bounds, clip_from_world, camera.eye);
                let reason = if selected {
                    GeometryStreamPlanReason::Selected
                } else if projection.intersects_viewport {
                    GeometryStreamPlanReason::InView
                } else {
                    GeometryStreamPlanReason::VisibleElement
                };
                let priority_score = streaming_priority_score(selected, &projection);

                entries.push(GeometryPrioritizedStreamEntry {
                    instance_id: instance.id,
                    element_id: instance.element_id.clone(),
                    definition_id: instance.definition_id,
                    reason,
                    priority_score,
                    projected_screen_area: projection.projected_screen_area,
                    distance_to_camera: projection.distance_to_camera,
                });
            }
        }

        entries.sort_by(|left, right| {
            right
                .priority_score
                .total_cmp(&left.priority_score)
                .then_with(|| left.instance_id.0.cmp(&right.instance_id.0))
        });

        self.apply_streaming_budget(entries, budget)
    }

    fn apply_streaming_budget(
        &self,
        entries: Vec<GeometryPrioritizedStreamEntry>,
        budget: GeometryStreamingBudget,
    ) -> GeometryPrioritizedStreamPlan {
        let mut selected_entries = Vec::new();
        let mut instance_ids = Vec::new();
        let mut definition_ids = Vec::new();
        let mut seen_instances = HashSet::new();
        let mut seen_definitions = HashSet::new();

        for entry in entries {
            let needs_instance = !self.resident_instances.contains_key(&entry.instance_id);
            let needs_definition = !self.resident_definitions.contains_key(&entry.definition_id);
            let would_add_instance = needs_instance && seen_instances.insert(entry.instance_id);
            let would_add_definition =
                needs_definition && !seen_definitions.contains(&entry.definition_id);

            if would_add_instance && instance_ids.len() >= budget.max_instances {
                continue;
            }
            if would_add_definition && definition_ids.len() >= budget.max_definitions {
                continue;
            }

            if would_add_instance {
                instance_ids.push(entry.instance_id);
            }
            if would_add_definition && seen_definitions.insert(entry.definition_id) {
                definition_ids.push(entry.definition_id);
            }
            selected_entries.push(entry);
        }

        GeometryPrioritizedStreamPlan {
            entries: selected_entries,
            instance_ids,
            definition_ids,
        }
    }

    pub fn mark_instance_batch_resident(&mut self, batch: &GeometryInstanceBatch) -> usize {
        let mut added = 0;
        for instance in &batch.instances {
            if self
                .resident_instances
                .insert(instance.id, instance.clone())
                .is_none()
            {
                added += 1;
            }
        }
        self.refresh_compatibility_package();
        added
    }

    pub fn mark_definition_batch_resident(&mut self, batch: &GeometryDefinitionBatch) -> usize {
        let mut added = 0;
        for definition in &batch.definitions {
            if self
                .resident_definitions
                .insert(definition.id, definition.clone())
                .is_none()
            {
                added += 1;
            }
        }
        self.refresh_compatibility_package();
        added
    }

    pub fn resident_instance_count(&self) -> usize {
        self.resident_instances.len()
    }

    pub fn resident_definition_count(&self) -> usize {
        self.resident_definitions.len()
    }

    pub fn residency_counts(&self) -> RuntimeResidencyCounts {
        RuntimeResidencyCounts {
            instances: self.resident_instance_count(),
            definitions: self.resident_definition_count(),
        }
    }

    pub fn stream_visible_residency_from_provider<P>(
        &mut self,
        resource: &str,
        provider: &P,
    ) -> Result<RuntimeResidencyCounts, GeometryPackageSourceError>
    where
        P: GeometryStreamProvider,
    {
        let missing = self.missing_stream_plan_for_visible_elements();
        let instance_batch = provider.load_instance_batch(
            resource,
            &GeometryInstanceBatchRequest::new(missing.instance_ids),
        )?;
        let definition_batch = provider.load_definition_batch(
            resource,
            &GeometryDefinitionBatchRequest::new(missing.definition_ids),
        )?;

        Ok(RuntimeResidencyCounts {
            instances: self.mark_instance_batch_resident(&instance_batch),
            definitions: self.mark_definition_batch_resident(&definition_batch),
        })
    }

    pub fn stream_prioritized_visible_residency_from_provider<P>(
        &mut self,
        resource: &str,
        provider: &P,
        camera: Camera,
        viewport: ViewportSize,
        budget: GeometryStreamingBudget,
    ) -> Result<RuntimeResidencyCounts, GeometryPackageSourceError>
    where
        P: GeometryStreamProvider,
    {
        let plan =
            self.prioritized_missing_stream_plan_for_visible_elements(camera, viewport, budget);
        let instance_batch = provider.load_instance_batch(
            resource,
            &GeometryInstanceBatchRequest::new(plan.instance_ids),
        )?;
        let definition_batch = provider.load_definition_batch(
            resource,
            &GeometryDefinitionBatchRequest::new(plan.definition_ids),
        )?;

        Ok(RuntimeResidencyCounts {
            instances: self.mark_instance_batch_resident(&instance_batch),
            definitions: self.mark_definition_batch_resident(&definition_batch),
        })
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

    pub fn suppress_elements<'a, I>(&mut self, ids: I) -> usize
    where
        I: IntoIterator<Item = &'a SemanticElementId>,
    {
        self.set_suppressed(ids, true)
    }

    pub fn unsuppress_elements<'a, I>(&mut self, ids: I) -> usize
    where
        I: IntoIterator<Item = &'a SemanticElementId>,
    {
        self.set_suppressed(ids, false)
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
        self.compose_resident_render_scene()
    }

    pub fn compose_resident_render_scene(&self) -> PreparedRenderScene {
        let visible_instances = self
            .catalog
            .instances
            .iter()
            .filter(|instance| {
                self.is_element_visible_by_id(&instance.element_id)
                    .unwrap_or(false)
                    && self.resident_instances.contains_key(&instance.id)
                    && self
                        .resident_definitions
                        .contains_key(&instance.definition_id)
            })
            .collect::<Vec<_>>();
        let visible_definition_ids = visible_instances
            .iter()
            .map(|instance| instance.definition_id)
            .collect::<HashSet<_>>();
        let definitions = self
            .catalog
            .definitions
            .iter()
            .filter(|definition| visible_definition_ids.contains(&definition.id))
            .filter_map(|definition| {
                self.resident_definitions
                    .get(&definition.id)
                    .map(|resident_definition| PreparedRenderDefinition {
                        id: definition.id,
                        mesh: resident_definition.mesh.clone(),
                    })
            })
            .collect::<Vec<_>>();
        let instances = visible_instances
            .into_iter()
            .filter_map(|instance| {
                self.resident_instances
                    .get(&instance.id)
                    .map(|resident_instance| PreparedRenderInstance {
                        id: resident_instance.id,
                        element_id: resident_instance.element_id.clone(),
                        definition_id: resident_instance.definition_id,
                        model_from_object: resident_instance.transform,
                        world_bounds: resident_instance.bounds,
                        material: self.material_for_instance(resident_instance),
                    })
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

        for instance in &self.catalog.instances {
            if !self
                .is_element_visible_by_id(&instance.element_id)
                .unwrap_or(false)
            {
                continue;
            }
            let residency = if self.resident_instances.contains_key(&instance.id)
                && self
                    .resident_definitions
                    .contains_key(&instance.definition_id)
            {
                ResidencyState::CpuReady
            } else {
                ResidencyState::Unloaded
            };

            scene.insert_geometry_instance(
                scene.root(),
                instance.external_id.clone(),
                instance.label.clone(),
                instance.transform,
                instance.bounds,
                instance.definition_id,
                MeshHandle(instance.definition_id.0),
                residency,
            );
        }

        scene
    }

    fn is_element_visible_by_id(&self, id: &SemanticElementId) -> Option<bool> {
        let indexed = self.elements_by_id.get(id)?;
        if indexed.state.suppressed {
            return Some(false);
        }
        Some(match indexed.state.visibility {
            ElementVisibilityOverride::Inherit => self.base_visible_element_ids.contains(id),
            ElementVisibilityOverride::Hidden => false,
            ElementVisibilityOverride::Visible => true,
        })
    }

    fn element_ids_with_visibility_override(
        &self,
        visibility: ElementVisibilityOverride,
    ) -> Vec<SemanticElementId> {
        self.catalog
            .elements
            .iter()
            .filter(|element| {
                self.elements_by_id
                    .get(&element.id)
                    .is_some_and(|indexed| indexed.state.visibility == visibility)
            })
            .map(|element| element.id.clone())
            .collect()
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

    fn set_suppressed<'a, I>(&mut self, ids: I, suppressed: bool) -> usize
    where
        I: IntoIterator<Item = &'a SemanticElementId>,
    {
        let mut changed = 0;
        for id in ids {
            let Some(indexed) = self.elements_by_id.get_mut(id) else {
                continue;
            };
            if indexed.state.suppressed != suppressed {
                indexed.state.suppressed = suppressed;
                changed += 1;
            }
        }
        changed
    }

    fn material_for_instance(
        &self,
        instance: &cc_w_types::GeometryInstanceCatalogEntry,
    ) -> PreparedMaterial {
        let selected = self
            .elements_by_id
            .get(&instance.element_id)
            .is_some_and(|indexed| indexed.state.selected);
        let color = if selected {
            SELECTED_ELEMENT_MATERIAL_COLOR
        } else {
            instance.display_color.unwrap_or_default()
        };
        PreparedMaterial::new(color)
    }

    fn refresh_compatibility_package(&mut self) {
        self.compatibility_package = compatibility_package_from_resident_stores(
            &self.catalog,
            &self.resident_instances,
            &self.resident_definitions,
        );
    }
}

fn compatibility_package_from_resident_stores(
    catalog: &GeometryCatalog,
    resident_instances: &HashMap<GeometryInstanceId, cc_w_types::GeometryInstanceCatalogEntry>,
    resident_definitions: &HashMap<GeometryDefinitionId, PreparedGeometryDefinition>,
) -> PreparedGeometryPackage {
    PreparedGeometryPackage {
        definitions: catalog
            .definitions
            .iter()
            .filter_map(|definition| resident_definitions.get(&definition.id).cloned())
            .collect(),
        elements: catalog
            .elements
            .iter()
            .map(|element| PreparedGeometryElement {
                id: element.id.clone(),
                label: element.label.clone(),
                declared_entity: element.declared_entity.clone(),
                default_render_class: element.default_render_class,
                bounds: element.bounds,
            })
            .collect(),
        instances: catalog
            .instances
            .iter()
            .filter_map(|instance| {
                resident_instances
                    .get(&instance.id)
                    .map(prepared_instance_from_catalog_entry)
            })
            .collect(),
    }
}

fn prepared_instance_from_catalog_entry(
    instance: &cc_w_types::GeometryInstanceCatalogEntry,
) -> PreparedGeometryInstance {
    PreparedGeometryInstance {
        id: instance.id,
        element_id: instance.element_id.clone(),
        definition_id: instance.definition_id,
        transform: instance.transform,
        bounds: instance.bounds,
        external_id: instance.external_id.clone(),
        label: instance.label.clone(),
        display_color: instance.display_color,
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

#[derive(Clone, Copy, Debug, PartialEq)]
struct ProjectedBoundsForStreaming {
    intersects_viewport: bool,
    projected_screen_area: f64,
    distance_to_camera: f64,
}

fn project_bounds_for_streaming(
    bounds: Bounds3,
    clip_from_world: glam::DMat4,
    camera_eye: DVec3,
) -> ProjectedBoundsForStreaming {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    let mut projected_any_corner = false;

    for corner in bounds_corners(bounds) {
        let clip = clip_from_world * DVec4::new(corner.x, corner.y, corner.z, 1.0);
        if clip.w <= f64::EPSILON {
            continue;
        }
        let ndc = clip.truncate() / clip.w;
        min_x = min_x.min(ndc.x);
        min_y = min_y.min(ndc.y);
        max_x = max_x.max(ndc.x);
        max_y = max_y.max(ndc.y);
        projected_any_corner = true;
    }

    let intersects_viewport =
        projected_any_corner && max_x >= -1.0 && min_x <= 1.0 && max_y >= -1.0 && min_y <= 1.0;
    let overlap_width = (max_x.min(1.0) - min_x.max(-1.0)).max(0.0);
    let overlap_height = (max_y.min(1.0) - min_y.max(-1.0)).max(0.0);
    let projected_screen_area = if intersects_viewport {
        overlap_width * overlap_height
    } else {
        0.0
    };

    ProjectedBoundsForStreaming {
        intersects_viewport,
        projected_screen_area,
        distance_to_camera: (bounds.center() - camera_eye).length(),
    }
}

fn streaming_priority_score(selected: bool, projection: &ProjectedBoundsForStreaming) -> f64 {
    let selected_boost = if selected { 1_000_000.0 } else { 0.0 };
    let viewport_boost = if projection.intersects_viewport {
        100_000.0
    } else {
        0.0
    };
    selected_boost
        + viewport_boost
        + (projection.projected_screen_area * 10_000.0)
        + (1.0 / (1.0 + projection.distance_to_camera.max(0.0)))
}

fn bounds_corners(bounds: Bounds3) -> [DVec3; 8] {
    [
        DVec3::new(bounds.min.x, bounds.min.y, bounds.min.z),
        DVec3::new(bounds.min.x, bounds.min.y, bounds.max.z),
        DVec3::new(bounds.min.x, bounds.max.y, bounds.min.z),
        DVec3::new(bounds.min.x, bounds.max.y, bounds.max.z),
        DVec3::new(bounds.max.x, bounds.min.y, bounds.min.z),
        DVec3::new(bounds.max.x, bounds.min.y, bounds.max.z),
        DVec3::new(bounds.max.x, bounds.max.y, bounds.min.z),
        DVec3::new(bounds.max.x, bounds.max.y, bounds.max.z),
    ]
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
        self.build_runtime_scene_for_start_view(resource, GeometryStartViewRequest::Default)
    }

    pub fn build_runtime_scene_for_start_view(
        &self,
        resource: &str,
        start_view: GeometryStartViewRequest,
    ) -> Result<RuntimeSceneState, RuntimeError> {
        let package = self
            .package_source
            .load_prepared_package(resource)
            .map_err(RuntimeError::from)?;
        RuntimeSceneState::from_prepared_package_with_start_view(package, start_view)
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

fn validate_geometry_catalog(catalog: &GeometryCatalog) -> Result<(), RuntimeError> {
    if catalog.definitions.is_empty() {
        return Err(RuntimeError::MissingDefinitions);
    }
    if catalog.elements.is_empty() {
        return Err(RuntimeError::MissingElements);
    }
    if catalog.instances.is_empty() {
        return Err(RuntimeError::MissingInstances);
    }

    let definition_ids = catalog
        .definitions
        .iter()
        .map(|definition| definition.id)
        .collect::<HashSet<_>>();
    let element_ids = catalog
        .elements
        .iter()
        .map(|element| element.id.clone())
        .collect::<HashSet<_>>();

    for instance in &catalog.instances {
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
        GeometryInstanceId, GeometryStartViewRequest, PreparedGeometryDefinition,
        PreparedGeometryElement, PreparedGeometryInstance, PreparedMesh, PreparedVertex,
        SemanticElementId,
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

    fn two_definition_streaming_package() -> PreparedGeometryPackage {
        let left_definition_id = GeometryDefinitionId(31);
        let right_definition_id = GeometryDefinitionId(32);
        let left_element_id = SemanticElementId::new("synthetic/stream/left");
        let right_element_id = SemanticElementId::new("synthetic/stream/right");
        let left_mesh = prepared_convex_polygon(vec![
            DVec3::new(-1.0, -1.0, 0.0),
            DVec3::new(1.0, -1.0, 0.0),
            DVec3::new(0.0, 1.5, 0.0),
        ]);
        let right_mesh = prepared_convex_polygon(vec![
            DVec3::new(-0.75, -0.75, 0.0),
            DVec3::new(0.75, -0.75, 0.0),
            DVec3::new(0.75, 0.75, 0.0),
            DVec3::new(-0.75, 0.75, 0.0),
        ]);
        let left_bounds = left_mesh
            .bounds
            .transformed(DMat4::from_translation(DVec3::new(-2.0, 0.0, 0.0)));
        let right_bounds = right_mesh
            .bounds
            .transformed(DMat4::from_translation(DVec3::new(2.0, 0.0, 0.0)));

        PreparedGeometryPackage {
            definitions: vec![
                PreparedGeometryDefinition {
                    id: left_definition_id,
                    mesh: left_mesh,
                },
                PreparedGeometryDefinition {
                    id: right_definition_id,
                    mesh: right_mesh,
                },
            ],
            elements: vec![
                PreparedGeometryElement {
                    id: left_element_id.clone(),
                    label: "Stream Left".to_string(),
                    declared_entity: "DemoGeometry".to_string(),
                    default_render_class: DefaultRenderClass::Physical,
                    bounds: left_bounds,
                },
                PreparedGeometryElement {
                    id: right_element_id.clone(),
                    label: "Stream Right".to_string(),
                    declared_entity: "DemoGeometry".to_string(),
                    default_render_class: DefaultRenderClass::Physical,
                    bounds: right_bounds,
                },
            ],
            instances: vec![
                PreparedGeometryInstance {
                    id: GeometryInstanceId(1),
                    element_id: left_element_id,
                    definition_id: left_definition_id,
                    transform: DMat4::from_translation(DVec3::new(-2.0, 0.0, 0.0)),
                    bounds: left_bounds,
                    external_id: ExternalId::new("synthetic/stream/left/item/1"),
                    label: "Stream Left".to_string(),
                    display_color: None,
                },
                PreparedGeometryInstance {
                    id: GeometryInstanceId(2),
                    element_id: right_element_id,
                    definition_id: right_definition_id,
                    transform: DMat4::from_translation(DVec3::new(2.0, 0.0, 0.0)),
                    bounds: right_bounds,
                    external_id: ExternalId::new("synthetic/stream/right/item/1"),
                    label: "Stream Right".to_string(),
                    display_color: None,
                },
            ],
        }
    }

    fn catalog_superset_package() -> PreparedGeometryPackage {
        let physical_definition_id = GeometryDefinitionId(41);
        let space_definition_id = GeometryDefinitionId(42);
        let physical_element_id = SemanticElementId::new("synthetic/catalog/physical");
        let space_element_id = SemanticElementId::new("synthetic/catalog/space");
        let mesh = prepared_convex_polygon(vec![
            DVec3::new(-1.0, -1.0, 0.0),
            DVec3::new(1.0, -1.0, 0.0),
            DVec3::new(0.0, 1.5, 0.0),
        ]);
        let physical_bounds = mesh
            .bounds
            .transformed(DMat4::from_translation(DVec3::new(-2.0, 0.0, 0.0)));
        let space_bounds = mesh
            .bounds
            .transformed(DMat4::from_translation(DVec3::new(2.0, 0.0, 0.0)));

        PreparedGeometryPackage {
            definitions: vec![
                PreparedGeometryDefinition {
                    id: physical_definition_id,
                    mesh: mesh.clone(),
                },
                PreparedGeometryDefinition {
                    id: space_definition_id,
                    mesh,
                },
            ],
            elements: vec![
                PreparedGeometryElement {
                    id: physical_element_id.clone(),
                    label: "Catalog Physical".to_string(),
                    declared_entity: "IfcWall".to_string(),
                    default_render_class: DefaultRenderClass::Physical,
                    bounds: physical_bounds,
                },
                PreparedGeometryElement {
                    id: space_element_id.clone(),
                    label: "Catalog Space".to_string(),
                    declared_entity: "IfcSpace".to_string(),
                    default_render_class: DefaultRenderClass::Space,
                    bounds: space_bounds,
                },
            ],
            instances: vec![
                PreparedGeometryInstance {
                    id: GeometryInstanceId(1),
                    element_id: physical_element_id,
                    definition_id: physical_definition_id,
                    transform: DMat4::from_translation(DVec3::new(-2.0, 0.0, 0.0)),
                    bounds: physical_bounds,
                    external_id: ExternalId::new("synthetic/catalog/physical/item/1"),
                    label: "Catalog Physical".to_string(),
                    display_color: None,
                },
                PreparedGeometryInstance {
                    id: GeometryInstanceId(2),
                    element_id: space_element_id,
                    definition_id: space_definition_id,
                    transform: DMat4::from_translation(DVec3::new(2.0, 0.0, 0.0)),
                    bounds: space_bounds,
                    external_id: ExternalId::new("synthetic/catalog/space/item/1"),
                    label: "Catalog Space".to_string(),
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
    fn full_package_stream_provider_slices_instances_and_definitions() {
        let source = SyntheticGeometryPackageSource::with_package(
            "demo/mapped-triangle",
            mapped_triangle_package("Synthetic Mapped Triangle", "synthetic/mapped/left"),
        );
        let provider = FullPackageGeometryStreamProvider::new(source);

        let catalog = provider
            .load_catalog("demo/mapped-triangle")
            .expect("catalog");
        assert_eq!(catalog.instances.len(), 2);
        assert_eq!(catalog.definitions.len(), 1);

        let instances = provider
            .load_instance_batch(
                "demo/mapped-triangle",
                &GeometryInstanceBatchRequest::new(vec![
                    GeometryInstanceId(2),
                    GeometryInstanceId(99),
                ]),
            )
            .expect("instances");
        assert_eq!(instances.instances.len(), 1);
        assert_eq!(instances.instances[0].id, GeometryInstanceId(2));
        assert_eq!(
            instances.instances[0].definition_id,
            GeometryDefinitionId(8)
        );

        let definitions = provider
            .load_definition_batch(
                "demo/mapped-triangle",
                &GeometryDefinitionBatchRequest::new(vec![
                    GeometryDefinitionId(8),
                    GeometryDefinitionId(99),
                ]),
            )
            .expect("definitions");
        assert_eq!(definitions.definitions.len(), 1);
        assert_eq!(definitions.definitions[0].id, GeometryDefinitionId(8));
        assert_eq!(definitions.definitions[0].mesh.vertex_count(), 3);
    }

    #[test]
    fn runtime_scene_stream_plan_deduplicates_shared_definitions() {
        let package = mapped_triangle_package("Synthetic Mapped Triangle", "synthetic/mapped/left");
        let mut runtime_scene = RuntimeSceneState::from_catalog(package.catalog()).expect("scene");

        let plan = runtime_scene.stream_plan_for_visible_elements();

        assert_eq!(
            plan.instance_ids,
            vec![GeometryInstanceId(1), GeometryInstanceId(2)]
        );
        assert_eq!(plan.definition_ids, vec![GeometryDefinitionId(8)]);
        assert_eq!(
            runtime_scene.residency_counts(),
            RuntimeResidencyCounts::default()
        );
        assert_eq!(
            runtime_scene.missing_stream_plan_for_visible_elements(),
            plan
        );

        let catalog = runtime_scene.catalog();
        let instance_batch = catalog.instance_batch(&GeometryInstanceBatchRequest::new(
            plan.instance_ids.clone(),
        ));
        let added_instances = runtime_scene.mark_instance_batch_resident(&instance_batch);
        let added_definitions =
            runtime_scene.mark_definition_batch_resident(&package.definition_batch(
                &GeometryDefinitionBatchRequest::new(plan.definition_ids.clone()),
            ));

        assert_eq!(added_instances, 2);
        assert_eq!(added_definitions, 1);
        assert_eq!(
            runtime_scene.residency_counts(),
            RuntimeResidencyCounts {
                instances: 2,
                definitions: 1,
            }
        );
        assert_eq!(
            runtime_scene.missing_stream_plan_for_visible_elements(),
            GeometryStreamPlan::default()
        );
    }

    #[test]
    fn catalog_total_geometry_can_exceed_default_visible_and_resident_geometry() {
        let package = catalog_superset_package();
        let catalog = package.catalog();
        let runtime_scene = RuntimeSceneState::from_catalog(catalog.clone()).expect("scene");
        let default_plan = runtime_scene.stream_plan_for_visible_elements();

        assert_eq!(catalog.elements.len(), 2);
        assert_eq!(catalog.instances.len(), 2);
        assert_eq!(catalog.definitions.len(), 2);
        assert_eq!(
            runtime_scene.visible_element_ids(),
            vec![SemanticElementId::new("synthetic/catalog/physical")]
        );
        assert_eq!(default_plan.instance_ids, vec![GeometryInstanceId(1)]);
        assert_eq!(default_plan.definition_ids, vec![GeometryDefinitionId(41)]);
        assert!(catalog.instances.len() > default_plan.instance_ids.len());
        assert!(catalog.definitions.len() > default_plan.definition_ids.len());
        assert_eq!(
            runtime_scene.residency_counts(),
            RuntimeResidencyCounts::default()
        );
        assert_eq!(runtime_scene.package().definitions.len(), 0);
        assert_eq!(runtime_scene.package().instances.len(), 0);
        assert_eq!(
            runtime_scene.compose_resident_render_scene().draw_count(),
            0
        );
    }

    #[test]
    fn catalog_only_runtime_draws_after_streaming_batches() {
        let package = mapped_triangle_package("Synthetic Mapped Triangle", "synthetic/mapped/left");
        let mut runtime_scene = RuntimeSceneState::from_catalog(package.catalog()).expect("scene");
        let plan = runtime_scene.stream_plan_for_visible_elements();
        let catalog = runtime_scene.catalog();
        let instance_batch = catalog.instance_batch(&GeometryInstanceBatchRequest::new(
            plan.instance_ids.clone(),
        ));

        assert_eq!(runtime_scene.compose_render_scene().draw_count(), 0);
        assert_eq!(
            runtime_scene.compose_resident_render_scene().draw_count(),
            0
        );

        assert_eq!(
            runtime_scene.mark_instance_batch_resident(&instance_batch),
            2
        );
        assert_eq!(
            runtime_scene.compose_resident_render_scene().draw_count(),
            0
        );

        assert_eq!(
            runtime_scene.mark_definition_batch_resident(&package.definition_batch(
                &GeometryDefinitionBatchRequest::new(plan.definition_ids.clone()),
            )),
            1
        );
        assert_eq!(
            runtime_scene.compose_resident_render_scene().draw_count(),
            2
        );
        assert_eq!(
            runtime_scene
                .compose_resident_render_scene()
                .definition_count(),
            1
        );
    }

    #[test]
    fn prioritized_stream_plan_prefers_selected_missing_instance_with_budget() {
        let package = two_definition_streaming_package();
        let mut runtime_scene = RuntimeSceneState::from_catalog(package.catalog()).expect("scene");
        let selected_id = SemanticElementId::new("synthetic/stream/right");
        runtime_scene.select_elements([selected_id].iter());
        let camera = Camera {
            eye: DVec3::new(-2.0, -7.0, 4.0),
            target: DVec3::new(-2.0, 0.0, 0.0),
            up: DVec3::Z,
            vertical_fov_degrees: 45.0,
            near_plane: 0.1,
            far_plane: 100.0,
        };

        let plan = runtime_scene.prioritized_missing_stream_plan_for_visible_elements(
            camera,
            ViewportSize::new(800, 600),
            GeometryStreamingBudget::new(1, 1),
        );

        assert_eq!(plan.entries.len(), 1);
        assert_eq!(plan.entries[0].instance_id, GeometryInstanceId(2));
        assert_eq!(plan.entries[0].reason, GeometryStreamPlanReason::Selected);
        assert_eq!(plan.instance_ids, vec![GeometryInstanceId(2)]);
        assert_eq!(plan.definition_ids, vec![GeometryDefinitionId(32)]);
    }

    #[test]
    fn streamed_batches_progressively_make_draws_appear() {
        let package = two_definition_streaming_package();
        let mut runtime_scene = RuntimeSceneState::from_catalog(package.catalog()).expect("scene");
        let catalog = runtime_scene.catalog();

        assert_eq!(
            runtime_scene.compose_resident_render_scene().draw_count(),
            0
        );

        let first_instance_batch =
            catalog.instance_batch(&GeometryInstanceBatchRequest::new(vec![
                GeometryInstanceId(1),
            ]));
        assert_eq!(
            runtime_scene.mark_instance_batch_resident(&first_instance_batch),
            1
        );
        assert_eq!(
            runtime_scene.compose_resident_render_scene().draw_count(),
            0
        );

        let first_definition_batch =
            package.definition_batch(&GeometryDefinitionBatchRequest::new(vec![
                GeometryDefinitionId(31),
            ]));
        assert_eq!(
            runtime_scene.mark_definition_batch_resident(&first_definition_batch),
            1
        );
        assert_eq!(
            runtime_scene.compose_resident_render_scene().draw_count(),
            1
        );

        let second_instance_batch =
            catalog.instance_batch(&GeometryInstanceBatchRequest::new(vec![
                GeometryInstanceId(2),
            ]));
        assert_eq!(
            runtime_scene.mark_instance_batch_resident(&second_instance_batch),
            1
        );
        assert_eq!(
            runtime_scene.compose_resident_render_scene().draw_count(),
            1
        );
        assert_eq!(
            runtime_scene.missing_stream_plan_for_visible_elements(),
            GeometryStreamPlan {
                instance_ids: Vec::new(),
                definition_ids: vec![GeometryDefinitionId(32)],
            }
        );

        let missing_definition_batch =
            package.definition_batch(&GeometryDefinitionBatchRequest::new(vec![
                GeometryDefinitionId(404),
            ]));
        assert_eq!(
            runtime_scene.mark_definition_batch_resident(&missing_definition_batch),
            0
        );
        assert_eq!(
            runtime_scene.compose_resident_render_scene().draw_count(),
            1
        );

        let second_definition_batch =
            package.definition_batch(&GeometryDefinitionBatchRequest::new(vec![
                GeometryDefinitionId(32),
            ]));
        assert_eq!(
            runtime_scene.mark_definition_batch_resident(&second_definition_batch),
            1
        );
        assert_eq!(
            runtime_scene.compose_resident_render_scene().draw_count(),
            2
        );
        assert_eq!(
            runtime_scene.missing_stream_plan_for_visible_elements(),
            GeometryStreamPlan::default()
        );
    }

    #[test]
    fn runtime_scene_streams_visible_residency_from_provider() {
        let package = mapped_triangle_package("Synthetic Mapped Triangle", "synthetic/mapped/left");
        let source =
            SyntheticGeometryPackageSource::with_package("demo/mapped-triangle", package.clone());
        let provider = FullPackageGeometryStreamProvider::new(source);
        let mut runtime_scene = RuntimeSceneState::from_catalog(package.catalog()).expect("scene");

        let loaded = runtime_scene
            .stream_visible_residency_from_provider("demo/mapped-triangle", &provider)
            .expect("stream visible");

        assert_eq!(
            loaded,
            RuntimeResidencyCounts {
                instances: 2,
                definitions: 1,
            }
        );
        assert_eq!(
            runtime_scene.missing_stream_plan_for_visible_elements(),
            GeometryStreamPlan::default()
        );
        assert_eq!(
            runtime_scene.compose_resident_render_scene().draw_count(),
            2
        );
    }

    #[test]
    fn runtime_scene_defaults_hide_non_physical_elements() {
        let runtime_scene =
            RuntimeSceneState::from_prepared_package(mixed_render_class_package()).expect("scene");

        let catalog = runtime_scene.catalog();
        assert_eq!(catalog.elements.len(), 2);
        assert_eq!(catalog.instances.len(), 2);
        assert_eq!(
            runtime_scene.visible_element_ids(),
            vec![SemanticElementId::new("synthetic/physical")]
        );
        assert_eq!(runtime_scene.compose_render_scene().draw_count(), 1);
        assert_eq!(runtime_scene.compose_scene_graph().node_count(), 2);
    }

    #[test]
    fn runtime_scene_start_view_all_can_show_hidden_default_classes() {
        let runtime_scene = RuntimeSceneState::from_prepared_package_with_start_view(
            mixed_render_class_package(),
            GeometryStartViewRequest::All,
        )
        .expect("scene");

        assert_eq!(
            runtime_scene.visible_element_ids(),
            vec![
                SemanticElementId::new("synthetic/physical"),
                SemanticElementId::new("synthetic/space")
            ]
        );
        assert_eq!(runtime_scene.compose_render_scene().draw_count(), 2);
    }

    #[test]
    fn runtime_scene_start_view_switch_preserves_user_visibility_overrides() {
        let physical_id = SemanticElementId::new("synthetic/physical");
        let space_id = SemanticElementId::new("synthetic/space");
        let mut runtime_scene =
            RuntimeSceneState::from_prepared_package(mixed_render_class_package()).expect("scene");

        assert_eq!(
            runtime_scene.visible_element_ids(),
            vec![physical_id.clone()]
        );
        assert_eq!(runtime_scene.hide_elements([&physical_id]), 1);
        assert_eq!(
            runtime_scene.apply_start_view(GeometryStartViewRequest::All),
            ResolvedGeometryStartView {
                visible_element_ids: vec![physical_id.clone(), space_id.clone()]
            }
        );
        assert_eq!(runtime_scene.visible_element_ids(), vec![space_id.clone()]);

        assert_eq!(runtime_scene.reset_visibility([&physical_id]), 1);
        assert_eq!(
            runtime_scene.visible_element_ids(),
            vec![physical_id, space_id]
        );
    }

    #[test]
    fn runtime_scene_explicit_show_survives_default_view_mode_switch() {
        let physical_id = SemanticElementId::new("synthetic/physical");
        let space_id = SemanticElementId::new("synthetic/space");
        let mut runtime_scene =
            RuntimeSceneState::from_prepared_package(mixed_render_class_package()).expect("scene");

        assert_eq!(runtime_scene.show_elements([&space_id]), 1);
        runtime_scene.apply_start_view(GeometryStartViewRequest::Default);

        assert_eq!(
            runtime_scene.visible_element_ids(),
            vec![physical_id, space_id.clone()]
        );
        assert_eq!(runtime_scene.shown_element_ids(), vec![space_id]);
        assert!(runtime_scene.hidden_element_ids().is_empty());
    }

    #[test]
    fn runtime_scene_suppression_is_separate_from_visibility_overrides() {
        let physical_id = SemanticElementId::new("synthetic/physical");
        let space_id = SemanticElementId::new("synthetic/space");
        let mut runtime_scene =
            RuntimeSceneState::from_prepared_package(mixed_render_class_package()).expect("scene");

        assert_eq!(runtime_scene.show_elements([&space_id]), 1);
        assert_eq!(
            runtime_scene.visible_element_ids(),
            vec![physical_id.clone(), space_id.clone()]
        );

        assert_eq!(
            runtime_scene.suppress_elements([&physical_id, &space_id]),
            2
        );
        assert!(runtime_scene.visible_element_ids().is_empty());
        assert_eq!(
            runtime_scene.suppressed_element_ids(),
            vec![physical_id.clone(), space_id.clone()]
        );
        assert_eq!(runtime_scene.shown_element_ids(), vec![space_id.clone()]);

        assert_eq!(runtime_scene.hide_elements([&physical_id]), 1);
        assert_eq!(
            runtime_scene.unsuppress_elements([&physical_id, &space_id]),
            2
        );
        assert_eq!(runtime_scene.visible_element_ids(), vec![space_id.clone()]);
        assert_eq!(runtime_scene.hidden_element_ids(), vec![physical_id]);
        assert_eq!(runtime_scene.shown_element_ids(), vec![space_id]);
        assert!(runtime_scene.suppressed_element_ids().is_empty());
    }

    #[test]
    fn runtime_scene_start_view_minimal_limits_default_visible_set() {
        let runtime_scene = RuntimeSceneState::from_prepared_package_with_start_view(
            mapped_triangle_package("Synthetic Mapped Triangle", "synthetic/mapped/left"),
            GeometryStartViewRequest::Minimal(1),
        )
        .expect("scene");

        assert_eq!(
            runtime_scene.visible_element_ids(),
            vec![SemanticElementId::new("synthetic/mapped/left")]
        );
        assert_eq!(runtime_scene.compose_render_scene().draw_count(), 1);
    }

    #[test]
    fn runtime_scene_start_view_elements_limits_initial_visible_set() {
        let space_id = SemanticElementId::new("synthetic/space");
        let runtime_scene = RuntimeSceneState::from_prepared_package_with_start_view(
            mixed_render_class_package(),
            GeometryStartViewRequest::Elements(vec![space_id.clone()]),
        )
        .expect("scene");

        assert_eq!(runtime_scene.visible_element_ids(), vec![space_id]);
        assert_eq!(runtime_scene.compose_render_scene().draw_count(), 1);
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
    fn runtime_scene_render_scene_preserves_element_ids_for_picking() {
        let left_id = SemanticElementId::new("synthetic/mapped/left");
        let right_id = SemanticElementId::new("synthetic/mapped/right");
        let mut runtime_scene = RuntimeSceneState::from_prepared_package(mapped_triangle_package(
            "Synthetic Mapped Triangle",
            "synthetic/mapped/left",
        ))
        .expect("scene");

        let render_scene = runtime_scene.compose_render_scene();
        assert_eq!(
            render_scene
                .instances
                .iter()
                .map(|instance| instance.element_id.clone())
                .collect::<Vec<_>>(),
            vec![left_id.clone(), right_id.clone()]
        );

        assert_eq!(runtime_scene.hide_elements([&right_id]), 1);
        let render_scene = runtime_scene.compose_render_scene();
        assert_eq!(render_scene.instances.len(), 1);
        assert_eq!(render_scene.instances[0].element_id, left_id);
    }

    #[test]
    fn runtime_scene_selected_elements_override_composed_material_color() {
        let left_id = SemanticElementId::new("synthetic/mapped/left");
        let right_id = SemanticElementId::new("synthetic/mapped/right");
        let mut runtime_scene = RuntimeSceneState::from_prepared_package(mapped_triangle_package(
            "Synthetic Mapped Triangle",
            "synthetic/mapped/left",
        ))
        .expect("scene");

        assert_eq!(runtime_scene.select_elements([&right_id]), 1);

        let render_scene = runtime_scene.compose_render_scene();
        assert_eq!(render_scene.draw_count(), 2);
        assert_eq!(
            render_scene
                .instances
                .iter()
                .map(|instance| instance.element_id.clone())
                .collect::<Vec<_>>(),
            vec![left_id, right_id.clone()]
        );
        assert_eq!(
            render_scene.instances[0].material.color.as_rgb(),
            [0.95, 0.56, 0.24]
        );
        assert_eq!(
            render_scene.instances[1].material.color.as_rgb(),
            SELECTED_ELEMENT_MATERIAL_COLOR.as_rgb()
        );

        assert_eq!(runtime_scene.clear_selection(), 1);
        let render_scene = runtime_scene.compose_render_scene();
        assert_eq!(
            render_scene.instances[1].material.color.as_rgb(),
            [0.24, 0.78, 0.55]
        );
    }

    #[test]
    fn runtime_scene_selected_resident_elements_override_and_clear_material_color() {
        let package = mapped_triangle_package("Synthetic Mapped Triangle", "synthetic/mapped/left");
        let mut runtime_scene = RuntimeSceneState::from_catalog(package.catalog()).expect("scene");
        let plan = runtime_scene.stream_plan_for_visible_elements();
        let catalog = runtime_scene.catalog();
        let instance_batch = catalog.instance_batch(&GeometryInstanceBatchRequest::new(
            plan.instance_ids.clone(),
        ));
        let definition_batch =
            package.definition_batch(&GeometryDefinitionBatchRequest::new(plan.definition_ids));
        let right_id = SemanticElementId::new("synthetic/mapped/right");

        runtime_scene.mark_instance_batch_resident(&instance_batch);
        runtime_scene.mark_definition_batch_resident(&definition_batch);
        assert_eq!(runtime_scene.select_elements([&right_id]), 1);

        let render_scene = runtime_scene.compose_resident_render_scene();
        assert_eq!(render_scene.draw_count(), 2);
        assert_eq!(
            render_scene.instances[1].material.color.as_rgb(),
            SELECTED_ELEMENT_MATERIAL_COLOR.as_rgb()
        );

        assert_eq!(runtime_scene.deselect_elements([&right_id]), 1);
        let render_scene = runtime_scene.compose_resident_render_scene();
        assert_eq!(
            render_scene.instances[1].material.color.as_rgb(),
            [0.24, 0.78, 0.55]
        );
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
