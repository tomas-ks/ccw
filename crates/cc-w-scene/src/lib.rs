use cc_w_types::{Bounds3, ExternalId, GeometryDefinitionId, ResidencyState};
use glam::DMat4;
use slotmap::{SlotMap, new_key_type};
use smallvec::SmallVec;

new_key_type! {
    pub struct NodeId;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MeshHandle(pub u64);

#[derive(Clone, Debug)]
pub struct SceneNode {
    pub external_id: ExternalId,
    pub label: String,
    pub parent: Option<NodeId>,
    pub children: SmallVec<[NodeId; 4]>,
    pub local_transform: DMat4,
    pub world_bounds: Bounds3,
    pub geometry_definition_id: Option<GeometryDefinitionId>,
    pub mesh_handle: Option<MeshHandle>,
    pub residency: ResidencyState,
}

#[derive(Debug)]
pub struct Scene {
    nodes: SlotMap<NodeId, SceneNode>,
    root: NodeId,
}

impl Scene {
    pub fn new(label: impl Into<String>) -> Self {
        let mut nodes = SlotMap::with_key();
        let root = nodes.insert(SceneNode {
            external_id: ExternalId::new("scene/root"),
            label: label.into(),
            parent: None,
            children: SmallVec::new(),
            local_transform: DMat4::IDENTITY,
            world_bounds: Bounds3::zero(),
            geometry_definition_id: None,
            mesh_handle: None,
            residency: ResidencyState::Resident,
        });

        Self { nodes, root }
    }

    pub fn root(&self) -> NodeId {
        self.root
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn node(&self, id: NodeId) -> Option<&SceneNode> {
        self.nodes.get(id)
    }

    pub fn insert_geometry_instance(
        &mut self,
        parent: NodeId,
        external_id: ExternalId,
        label: impl Into<String>,
        local_transform: DMat4,
        world_bounds: Bounds3,
        geometry_definition_id: GeometryDefinitionId,
        mesh_handle: MeshHandle,
        residency: ResidencyState,
    ) -> NodeId {
        let node = SceneNode {
            external_id,
            label: label.into(),
            parent: Some(parent),
            children: SmallVec::new(),
            local_transform,
            world_bounds,
            geometry_definition_id: Some(geometry_definition_id),
            mesh_handle: Some(mesh_handle),
            residency,
        };
        let node_id = self.nodes.insert(node);
        self.nodes[parent].children.push(node_id);
        node_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_tracks_root_and_child_nodes() {
        let mut scene = Scene::new("w root");
        let child = scene.insert_geometry_instance(
            scene.root(),
            ExternalId::new("demo/mesh"),
            "Demo Mesh",
            DMat4::IDENTITY,
            Bounds3::zero(),
            GeometryDefinitionId(7),
            MeshHandle(7),
            ResidencyState::GpuReady,
        );

        assert_eq!(scene.node_count(), 2);
        let root = scene.node(scene.root()).expect("root");

        assert_eq!(root.children.len(), 1);
        assert_eq!(root.children[0], child);
        assert_eq!(
            scene
                .node(child)
                .and_then(|node| node.geometry_definition_id),
            Some(GeometryDefinitionId(7))
        );
    }
}
